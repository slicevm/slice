//! Self-Contained RFC 1951 (DEFLATE), RFC 1950 (Zlib), & RFC 1952 (Gzip) Compression Engine
//! with Compile-Time CRC-32 Lookup Table, O(1) 9-Bit Primary Huffman Decode LUT, and Sandbox Memory Caging.

pub(crate) mod deflate;
pub(crate) mod inflate;

pub(crate) use deflate::deflate_raw_compress;
pub(crate) use inflate::inflate_raw_decompress;

pub(super) const MAX_DECOMPRESSED_BYTES: usize = 64 * 1024 * 1024;

pub(super) const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
pub(super) const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
pub(super) const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
pub(super) const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

const fn build_crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut c = i as u32;
        let mut bit = 0;
        while bit < 8 {
            c = if (c & 1) != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            bit += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

const CRC32_TABLE: [u32; 256] = build_crc32_table();

#[inline]
pub fn adler32_checksum(data: &[u8]) -> u32 {
    const MOD_ADLER: u32 = 65521;
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for chunk in data.chunks(5552) {
        for &byte in chunk {
            a += byte as u32;
            b += a;
        }
        a %= MOD_ADLER;
        b %= MOD_ADLER;
    }
    (b << 16) | a
}

#[inline]
pub fn crc32_checksum(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        let idx = ((crc as u8) ^ byte) as usize;
        crc = (crc >> 8) ^ CRC32_TABLE[idx];
    }
    !crc
}

#[inline]
pub(super) fn effective_compression_mem_budget() -> usize {
    crate::boa_realm::BOA_KV_MEM_BUDGET.with(|m| {
        let (used, limit) = m.get();
        if limit > 0 && limit < usize::MAX {
            limit.saturating_sub(used).min(MAX_DECOMPRESSED_BYTES)
        } else {
            MAX_DECOMPRESSED_BYTES
        }
    })
}

#[inline]
pub(super) fn charge_inflate_fuel(ticks: u64) -> Result<(), &'static str> {
    let exceeded = crate::boa_realm::BOA_FUEL_BUDGET.with(|f| {
        let (consumed, limit) = f.get();
        if limit == u64::MAX {
            return None;
        }
        let next = consumed.saturating_add(ticks);
        f.set((next, limit));
        if next > limit { Some(limit) } else { None }
    });
    if let Some(limit) = exceeded {
        let tenant_id = crate::boa_realm::BOA_ACTIVE_TENANT.with(|t| {
            let b = t.borrow();
            if b.is_empty() {
                "boa-tenant".to_string()
            } else {
                b.clone()
            }
        });
        crate::boa_realm::BOA_VIOLATION.with(|v| {
            if v.borrow().is_none() {
                *v.borrow_mut() =
                    Some(slice_sandbox::SandboxViolation::CpuFuelExhausted { tenant_id, limit });
            }
        });
        return Err("CPU fuel budget exhausted during DEFLATE decompression");
    }
    Ok(())
}

/// Compresses `input` using `"gzip"` (RFC 1952), `"deflate"` (RFC 1950 zlib), or `"deflate-raw"` (RFC 1951).
pub fn zlib_compress_bytes(input: &[u8], format: &str) -> Result<Vec<u8>, &'static str> {
    if input.len() > effective_compression_mem_budget() {
        return Err("Compression input exceeds sandbox memory limit");
    }
    let raw = deflate_raw_compress(input);
    match format {
        "deflate-raw" => Ok(raw),
        "deflate" => {
            let mut out = Vec::with_capacity(raw.len() + 6);
            out.extend_from_slice(&[0x78, 0x9C]);
            out.extend_from_slice(&raw);
            let adler = adler32_checksum(input);
            out.extend_from_slice(&adler.to_be_bytes());
            Ok(out)
        }
        "gzip" => {
            let mut out = Vec::with_capacity(raw.len() + 18);
            out.extend_from_slice(&[0x1F, 0x8B, 0x08, 0x00, 0, 0, 0, 0, 0x00, 0xFF]);
            out.extend_from_slice(&raw);
            let crc = crc32_checksum(input);
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(input.len() as u32).to_le_bytes());
            Ok(out)
        }
        _ => Err("Unsupported compression format"),
    }
}

/// Decompresses `input` using `"gzip"` (RFC 1952), `"deflate"` (RFC 1950 zlib), or `"deflate-raw"` (RFC 1951).
pub fn zlib_decompress_bytes(input: &[u8], format: &str) -> Result<Vec<u8>, &'static str> {
    match format {
        "deflate-raw" => inflate_raw_decompress(input),
        "deflate" => {
            if input.len() < 6 {
                return Err("Truncated zlib stream");
            }
            let cmf = input[0];
            let flg = input[1];
            if (cmf & 0x0F) != 8 || !((cmf as u16) << 8 | (flg as u16)).is_multiple_of(31) {
                return Err("Invalid zlib header");
            }
            let start = if (flg & 0x20) != 0 { 6 } else { 2 };
            if input.len() < start + 4 {
                return Err("Truncated zlib stream");
            }
            let end = input.len() - 4;
            let out = inflate_raw_decompress(&input[start..end])?;
            let expected_adler =
                u32::from_be_bytes([input[end], input[end + 1], input[end + 2], input[end + 3]]);
            if adler32_checksum(&out) != expected_adler {
                return Err("Zlib Adler-32 checksum mismatch");
            }
            Ok(out)
        }
        "gzip" => {
            if input.len() < 18 {
                return Err("Truncated gzip stream");
            }
            if input[0] != 0x1F || input[1] != 0x8B || input[2] != 0x08 {
                return Err("Invalid gzip header magic");
            }
            let flg = input[3];
            let end = input.len() - 8;
            let mut pos = 10usize;
            if (flg & 0x04) != 0 {
                if pos + 2 > end {
                    return Err("Truncated gzip FEXTRA header");
                }
                let xlen = u16::from_le_bytes([input[pos], input[pos + 1]]) as usize;
                pos = pos
                    .checked_add(2 + xlen)
                    .ok_or("Truncated gzip FEXTRA header")?;
                if pos > end {
                    return Err("Truncated gzip FEXTRA header");
                }
            }
            if (flg & 0x08) != 0 {
                let nul_off = input
                    .get(pos..end)
                    .and_then(|s| memchr::memchr(0, s))
                    .ok_or("Truncated gzip FNAME header")?;
                pos += nul_off + 1;
            }
            if (flg & 0x10) != 0 {
                let nul_off = input
                    .get(pos..end)
                    .and_then(|s| memchr::memchr(0, s))
                    .ok_or("Truncated gzip FCOMMENT header")?;
                pos += nul_off + 1;
            }
            if (flg & 0x02) != 0 {
                if pos + 2 > end {
                    return Err("Truncated gzip FHCRC header");
                }
                pos += 2;
            }
            if pos > end {
                return Err("Truncated gzip trailer");
            }
            let out = inflate_raw_decompress(&input[pos..end])?;
            let expected_crc =
                u32::from_le_bytes([input[end], input[end + 1], input[end + 2], input[end + 3]]);
            let expected_isize = u32::from_le_bytes([
                input[end + 4],
                input[end + 5],
                input[end + 6],
                input[end + 7],
            ]);
            if crc32_checksum(&out) != expected_crc || (out.len() as u32) != expected_isize {
                return Err("Gzip CRC-32 or ISIZE trailer mismatch");
            }
            Ok(out)
        }
        _ => Err("Unsupported decompression format"),
    }
}
