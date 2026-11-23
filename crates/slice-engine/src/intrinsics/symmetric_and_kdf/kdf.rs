//! Key Derivation Functions (`PBKDF2`, `HKDF`, and RFC 7914 `scrypt`) with Sandbox Memory Caging.

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
        return Err(
            "Invalid scrypt parameters: memory limit exceeded (ERR_CRYPTO_SCRYPT_INVALID_PARAMETER)",
        );
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
