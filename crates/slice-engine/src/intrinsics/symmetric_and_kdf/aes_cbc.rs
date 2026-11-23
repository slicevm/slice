//! Branchless `AES-128-CBC` and `AES-256-CBC` Block Cipher & PKCS#7 Padding.

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
