//! Fast Hex, Base64, Base64URL, and Strict WHATWG `btoa` / `atob` Codecs.

const HEX_LUT: &[u8; 16] = b"0123456789abcdef";

const fn build_b64_decode_lut(allow_url_safe: bool) -> [u8; 256] {
    let mut lut = [0xFFu8; 256];
    let mut i = 0usize;
    while i < 26 {
        lut[b'A' as usize + i] = i as u8;
        lut[b'a' as usize + i] = (i + 26) as u8;
        i += 1;
    }
    let mut d = 0usize;
    while d < 10 {
        lut[b'0' as usize + d] = (d + 52) as u8;
        d += 1;
    }
    lut[b'+' as usize] = 62;
    lut[b'/' as usize] = 63;
    if allow_url_safe {
        lut[b'-' as usize] = 62;
        lut[b'_' as usize] = 63;
    }
    lut
}

const B64_LENIENT_LUT: [u8; 256] = build_b64_decode_lut(true);
const B64_STRICT_STD_LUT: [u8; 256] = build_b64_decode_lut(false);

/// Fast Lookup-Table (`HEX_LUT`) hex encoder for 32-byte digests avoiding `std::fmt::Write` overhead.
#[inline]
pub(crate) fn digest_to_hex(digest: &[u8; 32]) -> String {
    let mut hex_bytes = Vec::with_capacity(64);
    for &b in digest {
        hex_bytes.push(HEX_LUT[(b >> 4) as usize]);
        hex_bytes.push(HEX_LUT[(b & 0x0f) as usize]);
    }
    unsafe { String::from_utf8_unchecked(hex_bytes) }
}

/// Fast Lookup-Table (`HEX_LUT`) hex encoder for arbitrary byte slices.
#[inline]
pub(crate) fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut hex_bytes = Vec::with_capacity(bytes.len() * 2);
    for &b in bytes {
        hex_bytes.push(HEX_LUT[(b >> 4) as usize]);
        hex_bytes.push(HEX_LUT[(b & 0x0f) as usize]);
    }
    unsafe { String::from_utf8_unchecked(hex_bytes) }
}

/// Decodes a hexadecimal string into raw bytes (stopping at the first non-hex byte pair,
/// matching Node.js `Buffer.from(str, 'hex')` semantics).
#[inline]
pub(crate) fn hex_decode_bytes(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let pairs = bytes.len() / 2;
    let mut out = Vec::with_capacity(pairs);
    for i in 0..pairs {
        let hi = match bytes[i * 2] {
            b'0'..=b'9' => bytes[i * 2] - b'0',
            b'a'..=b'f' => bytes[i * 2] - b'a' + 10,
            b'A'..=b'F' => bytes[i * 2] - b'A' + 10,
            _ => break,
        };
        let lo = match bytes[i * 2 + 1] {
            b'0'..=b'9' => bytes[i * 2 + 1] - b'0',
            b'a'..=b'f' => bytes[i * 2 + 1] - b'a' + 10,
            b'A'..=b'F' => bytes[i * 2 + 1] - b'A' + 10,
            _ => break,
        };
        out.push((hi << 4) | lo);
    }
    out
}

pub fn base64_encode(data: &[u8], url_safe: bool) -> String {
    const STD_TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    const URL_TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let table = if url_safe { URL_TABLE } else { STD_TABLE };
    let mut out = Vec::with_capacity(data.len().div_ceil(3) * 4);
    let (chunks, rem) = data.as_chunks::<3>();
    for chunk in chunks {
        let triple = ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | (chunk[2] as u32);
        out.push(table[((triple >> 18) & 0x3f) as usize]);
        out.push(table[((triple >> 12) & 0x3f) as usize]);
        out.push(table[((triple >> 6) & 0x3f) as usize]);
        out.push(table[(triple & 0x3f) as usize]);
    }
    if !rem.is_empty() {
        let b0 = rem[0] as u32;
        let b1 = if rem.len() > 1 { rem[1] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8);
        out.push(table[((triple >> 18) & 0x3f) as usize]);
        out.push(table[((triple >> 12) & 0x3f) as usize]);
        if rem.len() > 1 {
            out.push(table[((triple >> 6) & 0x3f) as usize]);
        } else if !url_safe {
            out.push(b'=');
        }
        if !url_safe {
            out.push(b'=');
        }
    }
    unsafe { String::from_utf8_unchecked(out) }
}

#[inline]
pub fn base64_decoded_len_bytes(bytes: &[u8]) -> usize {
    let mut valid = 0usize;
    for &b in bytes {
        if B64_LENIENT_LUT[b as usize] != 0xFF {
            valid += 1;
        }
    }
    (valid * 3) >> 2
}

pub fn base64_decode(input: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity((input.len() * 3) / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &b in input.as_bytes() {
        let val = B64_LENIENT_LUT[b as usize];
        if val == 0xFF {
            continue;
        }
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    out
}

/// Strict WHATWG `btoa`: encodes a Latin1 (`0x00..=0xFF`) string to standard Base64,
/// returning `Err` if any code point exceeds `0xFF`.
pub(crate) fn whatwg_btoa(input: &str) -> Result<String, &'static str> {
    if input.is_ascii() {
        return Ok(base64_encode(input.as_bytes(), false));
    }
    let mut bytes = Vec::with_capacity(input.len());
    for ch in input.chars() {
        let code = ch as u32;
        if code > 0xFF {
            return Err("InvalidCharacterError: The string to be encoded contains characters outside of the Latin1 range.");
        }
        bytes.push(code as u8);
    }
    Ok(base64_encode(&bytes, false))
}

/// Strict WHATWG `atob`: strips ASCII whitespace, validates Base64 alphabet and padding,
/// and returns decoded Latin1 bytes (mapped 1:1 to `0x00..=0xFF` code points).
pub(crate) fn whatwg_atob(input: &str) -> Result<String, &'static str> {
    let cleaned: std::borrow::Cow<'_, [u8]> = if input
        .bytes()
        .any(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0c))
    {
        std::borrow::Cow::Owned(
            input
                .bytes()
                .filter(|&b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0c))
                .collect(),
        )
    } else {
        std::borrow::Cow::Borrowed(input.as_bytes())
    };
    if cleaned.len() % 4 == 1 {
        return Err("InvalidCharacterError: The string to be decoded is not correctly encoded.");
    }
    let mut out_chars = String::with_capacity((cleaned.len() * 3) / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    let mut pad_count = 0usize;
    for (idx, &b) in cleaned.iter().enumerate() {
        if b == b'=' {
            pad_count += 1;
            if pad_count > 2 || (idx + 1 < cleaned.len() && cleaned[idx + 1] != b'=') {
                return Err("InvalidCharacterError: Invalid padding in base64 string.");
            }
            continue;
        }
        if pad_count > 0 {
            return Err("InvalidCharacterError: Characters found after base64 padding.");
        }
        let val = B64_STRICT_STD_LUT[b as usize];
        if val == 0xFF {
            return Err(
                "InvalidCharacterError: The string to be decoded contains invalid characters.",
            );
        }
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            let byte = ((buf >> bits) & 0xff) as u8;
            out_chars.push(byte as char);
        }
    }
    Ok(out_chars)
}
