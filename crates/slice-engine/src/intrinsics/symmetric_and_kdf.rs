//! Symmetric Ciphers (`AES-128/256-CBC`, `AES-128/256-GCM`) & Key Derivation Functions
//! (`PBKDF2`, `HKDF`, and RFC 7914 `scrypt`).

const MAX_KDF_OUTPUT_BYTES: usize = 64 * 1024 * 1024;
const MAX_PBKDF2_ITERATIONS: u32 = 10_000_000;

#[inline]
fn check_sandbox_kdf_memory_budget(requested_bytes: usize) -> Result<(), &'static str> {
    if requested_bytes > MAX_KDF_OUTPUT_BYTES {
        return Err("Key derivation output length exceeds sandbox memory limit");
    }
    let exceeded = crate::boa_realm::BOA_KV_MEM_BUDGET.with(|m| {
        let (used, limit) = m.get();
        if limit == 0 || limit == usize::MAX {
            None
        } else {
            let next = used.saturating_add(requested_bytes);
            if next > limit {
                Some((next, limit))
            } else {
                None
            }
        }
    });
    if let Some((requested, limit)) = exceeded {
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
                *v.borrow_mut() = Some(slice_sandbox::SandboxViolation::MemoryCeilingExceeded {
                    tenant_id,
                    requested,
                    limit,
                });
            }
        });
        return Err("Key derivation exceeds sandbox memory limit");
    }
    Ok(())
}

const AES_SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

const AES_INV_SBOX: [u8; 256] = [
    0x52, 0x09, 0x6a, 0xd5, 0x30, 0x36, 0xa5, 0x38, 0xbf, 0x40, 0xa3, 0x9e, 0x81, 0xf3, 0xd7, 0xfb,
    0x7c, 0xe3, 0x39, 0x82, 0x9b, 0x2f, 0xff, 0x87, 0x34, 0x8e, 0x43, 0x44, 0xc4, 0xde, 0xe9, 0xcb,
    0x54, 0x7b, 0x94, 0x32, 0xa6, 0xc2, 0x23, 0x3d, 0xee, 0x4c, 0x95, 0x0b, 0x42, 0xfa, 0xc3, 0x4e,
    0x08, 0x2e, 0xa1, 0x66, 0x28, 0xd9, 0x24, 0xb2, 0x76, 0x5b, 0xa2, 0x49, 0x6d, 0x8b, 0xd1, 0x25,
    0x72, 0xf8, 0xf6, 0x64, 0x86, 0x68, 0x98, 0x16, 0xd4, 0xa4, 0x5c, 0xcc, 0x5d, 0x65, 0xb6, 0x92,
    0x6c, 0x70, 0x48, 0x50, 0xfd, 0xed, 0xb9, 0xda, 0x5e, 0x15, 0x46, 0x57, 0xa7, 0x8d, 0x9d, 0x84,
    0x90, 0xd8, 0xab, 0x00, 0x8c, 0xbc, 0xd3, 0x0a, 0xf7, 0xe4, 0x58, 0x05, 0xb8, 0xb3, 0x45, 0x06,
    0xd0, 0x2c, 0x1e, 0x8f, 0xca, 0x3f, 0x0f, 0x02, 0xc1, 0xaf, 0xbd, 0x03, 0x01, 0x13, 0x8a, 0x6b,
    0x3a, 0x91, 0x11, 0x41, 0x4f, 0x67, 0xdc, 0xea, 0x97, 0xf2, 0xcf, 0xce, 0xf0, 0xb4, 0xe6, 0x73,
    0x96, 0xac, 0x74, 0x22, 0xe7, 0xad, 0x35, 0x85, 0xe2, 0xf9, 0x37, 0xe8, 0x1c, 0x75, 0xdf, 0x6e,
    0x47, 0xf1, 0x1a, 0x71, 0x1d, 0x29, 0xc5, 0x89, 0x6f, 0xb7, 0x62, 0x0e, 0xaa, 0x18, 0xbe, 0x1b,
    0xfc, 0x56, 0x3e, 0x4b, 0xc6, 0xd2, 0x79, 0x20, 0x9a, 0xdb, 0xc0, 0xfe, 0x78, 0xcd, 0x5a, 0xf4,
    0x1f, 0xdd, 0xa8, 0x33, 0x88, 0x07, 0xc7, 0x31, 0xb1, 0x12, 0x10, 0x59, 0x27, 0x80, 0xec, 0x5f,
    0x60, 0x51, 0x7f, 0xa9, 0x19, 0xb5, 0x4a, 0x0d, 0x2d, 0xe5, 0x7a, 0x9f, 0x93, 0xc9, 0x9c, 0xef,
    0xa0, 0xe0, 0x3b, 0x4d, 0xae, 0x2a, 0xf5, 0xb0, 0xc8, 0xeb, 0xbb, 0x3c, 0x83, 0x53, 0x99, 0x61,
    0x17, 0x2b, 0x04, 0x7e, 0xba, 0x77, 0xd6, 0x26, 0xe1, 0x69, 0x14, 0x63, 0x55, 0x21, 0x0c, 0x7d,
];

/// Branchless constant-time GF(2^8) multiplication by 2 (`xtime`).
#[inline(always)]
const fn xtime(a: u8) -> u8 {
    (a << 1) ^ (((a >> 7) & 1).wrapping_neg() & 0x1b)
}

/// Branchless constant-time GF(2^8) multiplication by `(9, 11, 13, 14)` for AES `InvMixColumns`.
#[inline(always)]
const fn gf_inv_muls(a: u8) -> (u8, u8, u8, u8) {
    let x2 = xtime(a);
    let x4 = xtime(x2);
    let x8 = xtime(x4);
    (x8 ^ a, x8 ^ x2 ^ a, x8 ^ x4 ^ a, x8 ^ x4 ^ x2)
}

/// Pre-expanded AES-128 (`11` round keys) or AES-256 (`15` round keys) key schedule.
#[derive(Clone)]
pub struct AesExpandedKey {
    round_keys: [[u8; 16]; 15],
    rounds: usize,
}

impl AesExpandedKey {
    pub fn new(key: &[u8]) -> Result<Self, &'static str> {
        let (nk, nr) = match key.len() {
            16 => (4usize, 10usize),
            32 => (8usize, 14usize),
            _ => return Err("AES key length must be 16 (128-bit) or 32 (256-bit) bytes"),
        };
        let total_words = 4 * (nr + 1);
        let mut w = [0u32; 60];
        for i in 0..nk {
            w[i] = u32::from_be_bytes([key[4 * i], key[4 * i + 1], key[4 * i + 2], key[4 * i + 3]]);
        }
        const RCON: [u32; 10] = [
            0x01000000, 0x02000000, 0x04000000, 0x08000000, 0x10000000, 0x20000000, 0x40000000,
            0x80000000, 0x1b000000, 0x36000000,
        ];
        let sub_word = |val: u32| -> u32 {
            let b = val.to_be_bytes();
            u32::from_be_bytes([
                AES_SBOX[b[0] as usize],
                AES_SBOX[b[1] as usize],
                AES_SBOX[b[2] as usize],
                AES_SBOX[b[3] as usize],
            ])
        };
        for i in nk..total_words {
            let mut temp = w[i - 1];
            if i % nk == 0 {
                temp = sub_word(temp.rotate_left(8)) ^ RCON[(i / nk) - 1];
            } else if nk == 8 && i % nk == 4 {
                temp = sub_word(temp);
            }
            w[i] = w[i - nk] ^ temp;
        }
        let mut round_keys = [[0u8; 16]; 15];
        for r in 0..=nr {
            for c in 0..4 {
                let bytes = w[r * 4 + c].to_be_bytes();
                round_keys[r][c * 4..c * 4 + 4].copy_from_slice(&bytes);
            }
        }
        Ok(Self {
            round_keys,
            rounds: nr,
        })
    }

    #[inline]
    #[allow(clippy::needless_range_loop)]
    pub fn encrypt_block(&self, block: &mut [u8; 16]) {
        for i in 0..16 {
            block[i] ^= self.round_keys[0][i];
        }
        for r in 1..self.rounds {
            for b in block.iter_mut() {
                *b = AES_SBOX[*b as usize];
            }
            let s = *block;
            block[1] = s[5];
            block[5] = s[9];
            block[9] = s[13];
            block[13] = s[1];
            block[2] = s[10];
            block[6] = s[14];
            block[10] = s[2];
            block[14] = s[6];
            block[3] = s[15];
            block[7] = s[3];
            block[11] = s[7];
            block[15] = s[11];
            for c in 0..4 {
                let i = c * 4;
                let (a0, a1, a2, a3) = (block[i], block[i + 1], block[i + 2], block[i + 3]);
                let t = a0 ^ a1 ^ a2 ^ a3;
                block[i] ^= t ^ xtime(a0 ^ a1);
                block[i + 1] ^= t ^ xtime(a1 ^ a2);
                block[i + 2] ^= t ^ xtime(a2 ^ a3);
                block[i + 3] ^= t ^ xtime(a3 ^ a0);
            }
            for i in 0..16 {
                block[i] ^= self.round_keys[r][i];
            }
        }
        for b in block.iter_mut() {
            *b = AES_SBOX[*b as usize];
        }
        let s = *block;
        block[1] = s[5];
        block[5] = s[9];
        block[9] = s[13];
        block[13] = s[1];
        block[2] = s[10];
        block[6] = s[14];
        block[10] = s[2];
        block[14] = s[6];
        block[3] = s[15];
        block[7] = s[3];
        block[11] = s[7];
        block[15] = s[11];
        for i in 0..16 {
            block[i] ^= self.round_keys[self.rounds][i];
        }
    }

    #[inline]
    #[allow(clippy::needless_range_loop)]
    pub fn decrypt_block(&self, block: &mut [u8; 16]) {
        for i in 0..16 {
            block[i] ^= self.round_keys[self.rounds][i];
        }
        for r in (1..self.rounds).rev() {
            let s = *block;
            block[1] = s[13];
            block[5] = s[1];
            block[9] = s[5];
            block[13] = s[9];
            block[2] = s[10];
            block[6] = s[14];
            block[10] = s[2];
            block[14] = s[6];
            block[3] = s[7];
            block[7] = s[11];
            block[11] = s[15];
            block[15] = s[3];
            for b in block.iter_mut() {
                *b = AES_INV_SBOX[*b as usize];
            }
            for i in 0..16 {
                block[i] ^= self.round_keys[r][i];
            }
            for c in 0..4 {
                let i = c * 4;
                let (a0_9, a0_11, a0_13, a0_14) = gf_inv_muls(block[i]);
                let (a1_9, a1_11, a1_13, a1_14) = gf_inv_muls(block[i + 1]);
                let (a2_9, a2_11, a2_13, a2_14) = gf_inv_muls(block[i + 2]);
                let (a3_9, a3_11, a3_13, a3_14) = gf_inv_muls(block[i + 3]);
                block[i] = a0_14 ^ a1_11 ^ a2_13 ^ a3_9;
                block[i + 1] = a0_9 ^ a1_14 ^ a2_11 ^ a3_13;
                block[i + 2] = a0_13 ^ a1_9 ^ a2_14 ^ a3_11;
                block[i + 3] = a0_11 ^ a1_13 ^ a2_9 ^ a3_14;
            }
        }
        let s = *block;
        block[1] = s[13];
        block[5] = s[1];
        block[9] = s[5];
        block[13] = s[9];
        block[2] = s[10];
        block[6] = s[14];
        block[10] = s[2];
        block[14] = s[6];
        block[3] = s[7];
        block[7] = s[11];
        block[11] = s[15];
        block[15] = s[3];
        for b in block.iter_mut() {
            *b = AES_INV_SBOX[*b as usize];
        }
        for i in 0..16 {
            block[i] ^= self.round_keys[0][i];
        }
    }
}

/// Encrypts `plaintext` with AES-128-CBC or AES-256-CBC and PKCS#7 padding (or unpadded when `auto_pad` is false).
pub fn aes_cbc_encrypt_with_expanded(
    expanded: &AesExpandedKey,
    iv: &[u8],
    plaintext: &[u8],
    auto_pad: bool,
) -> Result<Vec<u8>, &'static str> {
    if iv.len() != 16 {
        return Err("AES-CBC IV must be exactly 16 bytes");
    }
    if !auto_pad && !plaintext.len().is_multiple_of(16) {
        return Err("AES-CBC input length must be a multiple of 16 bytes when padding is disabled");
    }
    let pad_len = if auto_pad {
        16 - (plaintext.len() % 16)
    } else {
        0
    };
    let total_len = plaintext
        .len()
        .checked_add(pad_len)
        .ok_or("AES-CBC plaintext length overflow")?;
    let mut out = Vec::with_capacity(total_len);
    out.extend_from_slice(plaintext);
    if auto_pad {
        out.resize(total_len, pad_len as u8);
    }
    let mut prev = [0u8; 16];
    prev.copy_from_slice(iv);
    for chunk in out.as_chunks_mut::<16>().0 {
        let mut block = [0u8; 16];
        for i in 0..16 {
            block[i] = chunk[i] ^ prev[i];
        }
        expanded.encrypt_block(&mut block);
        *chunk = block;
        prev = block;
    }
    Ok(out)
}

/// Decrypts `ciphertext` with AES-128-CBC or AES-256-CBC and verifies PKCS#7 padding in constant-style checks.
pub fn aes_cbc_decrypt_with_expanded(
    expanded: &AesExpandedKey,
    iv: &[u8],
    ciphertext: &[u8],
    auto_pad: bool,
) -> Result<Vec<u8>, &'static str> {
    if iv.len() != 16 {
        return Err("AES-CBC IV must be exactly 16 bytes");
    }
    if ciphertext.is_empty() && auto_pad {
        return Err("AES-CBC ciphertext cannot be empty");
    }
    if !ciphertext.len().is_multiple_of(16) {
        return Err("AES-CBC ciphertext length must be a multiple of 16 bytes");
    }
    let mut out = ciphertext.to_vec();
    let mut prev = [0u8; 16];
    prev.copy_from_slice(iv);
    for chunk in out.as_chunks_mut::<16>().0 {
        let next_prev = *chunk;
        let mut block = next_prev;
        expanded.decrypt_block(&mut block);
        for i in 0..16 {
            chunk[i] = block[i] ^ prev[i];
        }
        prev = next_prev;
    }
    if auto_pad {
        let pad_byte = *out.last().ok_or("Empty decrypted buffer")? as usize;
        if pad_byte == 0 || pad_byte > 16 || pad_byte > out.len() {
            return Err("Invalid AES-CBC PKCS#7 padding");
        }
        let start = out.len() - pad_byte;
        let mut bad = 0u8;
        for &b in &out[start..] {
            bad |= std::hint::black_box(b ^ (pad_byte as u8));
        }
        if std::hint::black_box(bad) != 0 {
            return Err("Invalid AES-CBC PKCS#7 padding bytes");
        }
        out.truncate(start);
    }
    Ok(out)
}

pub fn aes_cbc_encrypt(
    key: &[u8],
    iv: &[u8],
    plaintext: &[u8],
    auto_pad: bool,
) -> Result<Vec<u8>, &'static str> {
    let expanded = AesExpandedKey::new(key)?;
    aes_cbc_encrypt_with_expanded(&expanded, iv, plaintext, auto_pad)
}

pub fn aes_cbc_decrypt(
    key: &[u8],
    iv: &[u8],
    ciphertext: &[u8],
    auto_pad: bool,
) -> Result<Vec<u8>, &'static str> {
    let expanded = AesExpandedKey::new(key)?;
    aes_cbc_decrypt_with_expanded(&expanded, iv, ciphertext, auto_pad)
}

/// Builds a `ring::aead::LessSafeKey` for 128-bit (`16`-byte) or 256-bit (`32`-byte) AES-GCM.
#[inline]
pub fn make_ring_aead_key(key: &[u8]) -> Result<ring::aead::LessSafeKey, &'static str> {
    let alg = match key.len() {
        16 => &ring::aead::AES_128_GCM,
        32 => &ring::aead::AES_256_GCM,
        _ => return Err("AES-GCM key length must be 16 (128-bit) or 32 (256-bit) bytes"),
    };
    let unbound = ring::aead::UnboundKey::new(alg, key).map_err(|_| "Invalid AES-GCM key bytes")?;
    Ok(ring::aead::LessSafeKey::new(unbound))
}

/// Encrypts `plaintext` in-place using a pre-expanded `ring::aead::LessSafeKey`, returning `ciphertext || tag`.
#[inline]
pub fn aes_gcm_seal_with_key(
    less_safe_key: &ring::aead::LessSafeKey,
    iv: &[u8],
    aad: &[u8],
    plaintext: &[u8],
    tag_bits: usize,
) -> Result<Vec<u8>, &'static str> {
    if iv.len() != 12 {
        return Err("AES-GCM IV must be 12 bytes (96 bits)");
    }
    let tag_bytes = match tag_bits {
        32 | 64 | 96 | 104 | 112 | 120 | 128 => tag_bits / 8,
        _ => return Err("AES-GCM tagLength must be 32, 64, 96, 104, 112, 120, or 128 bits"),
    };
    let nonce =
        ring::aead::Nonce::try_assume_unique_for_key(iv).map_err(|_| "Invalid AES-GCM nonce")?;
    let mut buf = Vec::with_capacity(plaintext.len() + tag_bytes);
    buf.extend_from_slice(plaintext);
    let tag = less_safe_key
        .seal_in_place_separate_tag(nonce, ring::aead::Aad::from(aad), &mut buf)
        .map_err(|_| "AES-GCM encryption failed")?;
    buf.extend_from_slice(&tag.as_ref()[..tag_bytes]);
    Ok(buf)
}

/// Constant-time 128-bit multiplication in GF(2^128) per NIST SP 800-38D Section 6.3.
#[inline]
fn ghash_mul(x: u128, y: u128) -> u128 {
    let mut z = 0u128;
    let mut v = y;
    for i in 0..128 {
        let xi = (x >> (127 - i)) & 1;
        z ^= v & xi.wrapping_neg();
        let lsb = v & 1;
        v = (v >> 1) ^ (0xe100_0000_0000_0000_0000_0000_0000_0000u128 & lsb.wrapping_neg());
    }
    z
}

#[inline]
fn ghash_compute(h: u128, aad: &[u8], ct: &[u8]) -> [u8; 16] {
    let mut y = 0u128;
    let feed = |acc: &mut u128, data: &[u8]| {
        let mut off = 0usize;
        while off + 16 <= data.len() {
            let mut blk = [0u8; 16];
            blk.copy_from_slice(&data[off..off + 16]);
            *acc = ghash_mul(*acc ^ u128::from_be_bytes(blk), h);
            off += 16;
        }
        if off < data.len() {
            let mut blk = [0u8; 16];
            blk[..data.len() - off].copy_from_slice(&data[off..]);
            *acc = ghash_mul(*acc ^ u128::from_be_bytes(blk), h);
        }
    };
    feed(&mut y, aad);
    feed(&mut y, ct);
    let mut len_blk = [0u8; 16];
    len_blk[..8].copy_from_slice(&((aad.len() as u64) * 8).to_be_bytes());
    len_blk[8..].copy_from_slice(&((ct.len() as u64) * 8).to_be_bytes());
    y = ghash_mul(y ^ u128::from_be_bytes(len_blk), h);
    y.to_be_bytes()
}

fn aes_gcm_open_truncated_tag(
    raw_key: &[u8],
    iv: &[u8],
    aad: &[u8],
    ciphertext_and_tag: &[u8],
    tag_bytes: usize,
) -> Result<Vec<u8>, &'static str> {
    if ciphertext_and_tag.len() < tag_bytes {
        return Err("AES-GCM ciphertext too short for authentication tag");
    }
    let expanded = AesExpandedKey::new(raw_key)?;
    let (ct, recv_tag) = ciphertext_and_tag.split_at(ciphertext_and_tag.len() - tag_bytes);

    let mut h_block = [0u8; 16];
    expanded.encrypt_block(&mut h_block);
    let h = u128::from_be_bytes(h_block);

    let mut j0 = [0u8; 16];
    j0[..12].copy_from_slice(iv);
    j0[15] = 1;

    let s_block = ghash_compute(h, aad, ct);
    let mut ej0 = j0;
    expanded.encrypt_block(&mut ej0);
    for i in 0..16 {
        ej0[i] ^= s_block[i];
    }

    let mut bad = 0u8;
    for i in 0..tag_bytes {
        bad |= std::hint::black_box(ej0[i] ^ recv_tag[i]);
    }
    if std::hint::black_box(bad) != 0 {
        return Err("AES-GCM authentication tag verification failed");
    }

    let mut out = Vec::with_capacity(ct.len());
    let mut ctr = 2u32;
    let mut off = 0usize;
    while off < ct.len() {
        let mut cb = j0;
        cb[12..16].copy_from_slice(&ctr.to_be_bytes());
        ctr = ctr.wrapping_add(1);
        expanded.encrypt_block(&mut cb);
        let take = (ct.len() - off).min(16);
        for i in 0..take {
            out.push(ct[off + i] ^ cb[i]);
        }
        off += take;
    }
    Ok(out)
}

/// Decrypts `ciphertext_and_tag` using a pre-expanded `ring::aead::LessSafeKey` (or NIST SP 800-38D
/// GHASH + GCTR when `tag_bits` is a valid truncated WebCrypto tag length `< 128`), returning `plaintext`.
#[inline]
pub fn aes_gcm_open_with_key(
    less_safe_key: &ring::aead::LessSafeKey,
    raw_key: &[u8],
    iv: &[u8],
    aad: &[u8],
    ciphertext_and_tag: &[u8],
    tag_bits: usize,
) -> Result<Vec<u8>, &'static str> {
    if iv.len() != 12 {
        return Err("AES-GCM IV must be 12 bytes (96 bits)");
    }
    let tag_bytes = match tag_bits {
        32 | 64 | 96 | 104 | 112 | 120 | 128 => tag_bits / 8,
        _ => return Err("AES-GCM tagLength must be 32, 64, 96, 104, 112, 120, or 128 bits"),
    };
    if tag_bytes != 16 {
        return aes_gcm_open_truncated_tag(raw_key, iv, aad, ciphertext_and_tag, tag_bytes);
    }
    if ciphertext_and_tag.len() < 16 {
        return Err("AES-GCM ciphertext too short for authentication tag");
    }
    let nonce =
        ring::aead::Nonce::try_assume_unique_for_key(iv).map_err(|_| "Invalid AES-GCM nonce")?;
    let mut buf = ciphertext_and_tag.to_vec();
    let plain = less_safe_key
        .open_in_place(nonce, ring::aead::Aad::from(aad), &mut buf)
        .map_err(|_| "AES-GCM authentication tag verification failed")?;
    let plain_len = plain.len();
    buf.truncate(plain_len);
    Ok(buf)
}

#[inline]
pub fn aes_gcm_encrypt(
    key: &[u8],
    iv: &[u8],
    aad: &[u8],
    plaintext: &[u8],
    tag_bits: usize,
) -> Result<Vec<u8>, &'static str> {
    let k = make_ring_aead_key(key)?;
    aes_gcm_seal_with_key(&k, iv, aad, plaintext, tag_bits)
}

#[inline]
pub fn aes_gcm_decrypt(
    key: &[u8],
    iv: &[u8],
    aad: &[u8],
    ciphertext_and_tag: &[u8],
    tag_bits: usize,
) -> Result<Vec<u8>, &'static str> {
    let k = make_ring_aead_key(key)?;
    aes_gcm_open_with_key(&k, key, iv, aad, ciphertext_and_tag, tag_bits)
}

/// Derives `byte_len` key bytes using `ring::pbkdf2::derive` (`SHA-1`, `SHA-256`, `SHA-384`, `SHA-512`).
pub fn pbkdf2_derive_bytes(
    hash_alg: &str,
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    byte_len: usize,
) -> Result<Vec<u8>, &'static str> {
    if byte_len == 0 {
        return Ok(Vec::new());
    }
    if iterations > MAX_PBKDF2_ITERATIONS {
        return Err("PBKDF2 iterations exceed maximum sandbox limit (10,000,000)");
    }
    check_sandbox_kdf_memory_budget(byte_len)?;
    let non_zero_iters =
        std::num::NonZeroU32::new(iterations).ok_or("PBKDF2 iterations must be > 0")?;
    let alg = if hash_alg.eq_ignore_ascii_case("SHA-256") || hash_alg.eq_ignore_ascii_case("SHA256")
    {
        ring::pbkdf2::PBKDF2_HMAC_SHA256
    } else if hash_alg.eq_ignore_ascii_case("SHA-384") || hash_alg.eq_ignore_ascii_case("SHA384") {
        ring::pbkdf2::PBKDF2_HMAC_SHA384
    } else if hash_alg.eq_ignore_ascii_case("SHA-512") || hash_alg.eq_ignore_ascii_case("SHA512") {
        ring::pbkdf2::PBKDF2_HMAC_SHA512
    } else if hash_alg.eq_ignore_ascii_case("SHA-1") || hash_alg.eq_ignore_ascii_case("SHA1") {
        ring::pbkdf2::PBKDF2_HMAC_SHA1
    } else {
        return Err("Unsupported PBKDF2 digest algorithm");
    };
    let mut out = vec![0u8; byte_len];
    ring::pbkdf2::derive(alg, non_zero_iters, salt, password, &mut out);
    Ok(out)
}

struct HkdfLen(usize);
impl ring::hkdf::KeyType for HkdfLen {
    fn len(&self) -> usize {
        self.0
    }
}

/// Derives `byte_len` key bytes using `ring::hkdf` (`SHA-1`, `SHA-256`, `SHA-384`, `SHA-512`).
pub fn hkdf_derive_bytes(
    hash_alg: &str,
    ikm: &[u8],
    salt: &[u8],
    info: &[u8],
    byte_len: usize,
) -> Result<Vec<u8>, &'static str> {
    if byte_len == 0 {
        return Ok(Vec::new());
    }
    check_sandbox_kdf_memory_budget(byte_len)?;
    let alg = if hash_alg.eq_ignore_ascii_case("SHA-256") || hash_alg.eq_ignore_ascii_case("SHA256")
    {
        ring::hkdf::HKDF_SHA256
    } else if hash_alg.eq_ignore_ascii_case("SHA-384") || hash_alg.eq_ignore_ascii_case("SHA384") {
        ring::hkdf::HKDF_SHA384
    } else if hash_alg.eq_ignore_ascii_case("SHA-512") || hash_alg.eq_ignore_ascii_case("SHA512") {
        ring::hkdf::HKDF_SHA512
    } else if hash_alg.eq_ignore_ascii_case("SHA-1") || hash_alg.eq_ignore_ascii_case("SHA1") {
        ring::hkdf::HKDF_SHA1_FOR_LEGACY_USE_ONLY
    } else {
        return Err("Unsupported HKDF digest algorithm");
    };
    let ring_salt = ring::hkdf::Salt::new(alg, salt);
    let prk = ring_salt.extract(ikm);
    let info_slices = [info];
    let okm = prk
        .expand(&info_slices, HkdfLen(byte_len))
        .map_err(|_| "HKDF output length exceeds maximum (255 * HashLen)")?;
    let mut out = vec![0u8; byte_len];
    okm.fill(&mut out).map_err(|_| "HKDF expand failed")?;
    Ok(out)
}

#[inline]
fn salsa20_8_core(b: &mut [u8; 64]) {
    let mut x = [0u32; 16];
    for (i, item) in x.iter_mut().enumerate() {
        let off = i * 4;
        *item = u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]]);
    }
    let mut w = x;
    for _ in 0..4 {
        w[4] ^= w[0].wrapping_add(w[12]).rotate_left(7);
        w[8] ^= w[4].wrapping_add(w[0]).rotate_left(9);
        w[12] ^= w[8].wrapping_add(w[4]).rotate_left(13);
        w[0] ^= w[12].wrapping_add(w[8]).rotate_left(18);
        w[9] ^= w[5].wrapping_add(w[1]).rotate_left(7);
        w[13] ^= w[9].wrapping_add(w[5]).rotate_left(9);
        w[1] ^= w[13].wrapping_add(w[9]).rotate_left(13);
        w[5] ^= w[1].wrapping_add(w[13]).rotate_left(18);
        w[14] ^= w[10].wrapping_add(w[6]).rotate_left(7);
        w[2] ^= w[14].wrapping_add(w[10]).rotate_left(9);
        w[6] ^= w[2].wrapping_add(w[14]).rotate_left(13);
        w[10] ^= w[6].wrapping_add(w[2]).rotate_left(18);
        w[3] ^= w[15].wrapping_add(w[11]).rotate_left(7);
        w[7] ^= w[3].wrapping_add(w[15]).rotate_left(9);
        w[11] ^= w[7].wrapping_add(w[3]).rotate_left(13);
        w[15] ^= w[11].wrapping_add(w[7]).rotate_left(18);

        w[1] ^= w[0].wrapping_add(w[3]).rotate_left(7);
        w[2] ^= w[1].wrapping_add(w[0]).rotate_left(9);
        w[3] ^= w[2].wrapping_add(w[1]).rotate_left(13);
        w[0] ^= w[3].wrapping_add(w[2]).rotate_left(18);
        w[6] ^= w[5].wrapping_add(w[4]).rotate_left(7);
        w[7] ^= w[6].wrapping_add(w[5]).rotate_left(9);
        w[4] ^= w[7].wrapping_add(w[6]).rotate_left(13);
        w[5] ^= w[4].wrapping_add(w[7]).rotate_left(18);
        w[11] ^= w[10].wrapping_add(w[9]).rotate_left(7);
        w[8] ^= w[11].wrapping_add(w[10]).rotate_left(9);
        w[9] ^= w[8].wrapping_add(w[11]).rotate_left(13);
        w[10] ^= w[9].wrapping_add(w[8]).rotate_left(18);
        w[12] ^= w[15].wrapping_add(w[14]).rotate_left(7);
        w[13] ^= w[12].wrapping_add(w[15]).rotate_left(9);
        w[14] ^= w[13].wrapping_add(w[12]).rotate_left(13);
        w[15] ^= w[14].wrapping_add(w[13]).rotate_left(18);
    }
    for i in 0..16 {
        let bytes = x[i].wrapping_add(w[i]).to_le_bytes();
        b[i * 4..i * 4 + 4].copy_from_slice(&bytes);
    }
}

fn scrypt_block_mix(b: &[u8], y: &mut [u8], r: usize) {
    let mut x = [0u8; 64];
    x.copy_from_slice(&b[(2 * r - 1) * 64..2 * r * 64]);
    for i in 0..(2 * r) {
        let block = &b[i * 64..(i + 1) * 64];
        for j in 0..64 {
            x[j] ^= block[j];
        }
        salsa20_8_core(&mut x);
        let dst_idx = if i % 2 == 0 { i / 2 } else { r + i / 2 };
        y[dst_idx * 64..(dst_idx + 1) * 64].copy_from_slice(&x);
    }
}

fn scrypt_ro_mix(b: &mut [u8], v: &mut [u8], y: &mut [u8], n: usize, r: usize) {
    let block_len = 128 * r;
    for i in 0..n {
        v[i * block_len..(i + 1) * block_len].copy_from_slice(b);
        scrypt_block_mix(b, y, r);
        b.copy_from_slice(y);
    }
    let mask = n - 1;
    let last_off = (2 * r - 1) * 64;
    for _ in 0..n {
        let j = (u32::from_le_bytes([
            b[last_off],
            b[last_off + 1],
            b[last_off + 2],
            b[last_off + 3],
        ]) as usize)
            & mask;
        let v_slice = &v[j * block_len..(j + 1) * block_len];
        for k in 0..block_len {
            b[k] ^= v_slice[k];
        }
        scrypt_block_mix(b, y, r);
        b.copy_from_slice(y);
    }
}

/// RFC 7914 `scrypt` key derivation function (`N`, `r`, `p`, `maxmem`).
pub fn scrypt_derive_bytes(
    password: &[u8],
    salt: &[u8],
    n: usize,
    r: usize,
    p: usize,
    dk_len: usize,
    maxmem: usize,
) -> Result<Vec<u8>, &'static str> {
    if dk_len == 0 {
        return Ok(Vec::new());
    }
    if n < 2 || !n.is_power_of_two() || r == 0 || p == 0 {
        return Err("Invalid scrypt parameters: N must be a power of two > 1, r > 0, p > 0");
    }
    let block_len = 128usize
        .checked_mul(r)
        .ok_or("scrypt block size overflow")?;
    let v_bytes = block_len
        .checked_mul(n)
        .ok_or("scrypt memory limit exceeded")?;
    let total_b = block_len
        .checked_mul(p)
        .ok_or("scrypt parallel buffer overflow")?;
    let working_set = v_bytes.saturating_add(total_b);
    if working_set > maxmem {
        return Err("Invalid scrypt parameters: memory limit exceeded (ERR_CRYPTO_SCRYPT_INVALID_PARAMETER)");
    }
    check_sandbox_kdf_memory_budget(working_set.saturating_add(dk_len))?;
    let mut b = pbkdf2_derive_bytes("SHA-256", password, salt, 1, total_b)?;
    let mut v = vec![0u8; v_bytes];
    let mut y = vec![0u8; block_len];
    for i in 0..p {
        scrypt_ro_mix(
            &mut b[i * block_len..(i + 1) * block_len],
            &mut v,
            &mut y,
            n,
            r,
        );
    }
    pbkdf2_derive_bytes("SHA-256", password, &b, 1, dk_len)
}
