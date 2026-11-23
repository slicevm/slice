//! `AES-128-GCM` and `AES-256-GCM` AEAD (`ring` Fast-Path + NIST SP 800-38D Truncated WebCrypto Tag Open).

use super::aes_cbc::AesExpandedKey;

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
