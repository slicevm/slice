//! Asymmetric Cryptography (`Ed25519`, `ECDSA P-256 / P-384`), PKCS#8 / SPKI / IEEE P1363 / DER
//! Codecs, and Tenant-Isolated `CachedCryptoKeyMaterial` Handle Table.

use super::symmetric_and_kdf::AesExpandedKey;

const ED25519_PKCS8_V1_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];
const ED25519_PKCS8_V2_PREFIX: [u8; 16] = [
    0x30, 0x53, 0x02, 0x01, 0x01, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];
const ED25519_OID: [u8; 5] = [0x06, 0x03, 0x2b, 0x65, 0x70];
const ED25519_SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

const ECDSA_P256_SPKI_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];
const ECDSA_P384_SPKI_PREFIX: [u8; 23] = [
    0x30, 0x76, 0x30, 0x10, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x05, 0x2b,
    0x81, 0x04, 0x00, 0x22, 0x03, 0x62, 0x00,
];

/// Wraps a 32-byte Ed25519 private seed and 32-byte public key into PKCS#8 v2 DER (accepted by `ring` and WebCrypto).
pub fn ed25519_seed_and_pub_to_pkcs8_v2(seed: &[u8; 32], pub_bytes: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(85);
    out.extend_from_slice(&ED25519_PKCS8_V2_PREFIX);
    out.extend_from_slice(seed);
    out.extend_from_slice(&[0xa1, 0x23, 0x03, 0x21, 0x00]);
    out.extend_from_slice(pub_bytes);
    out
}

/// Wraps a 32-byte Ed25519 private seed into standard 48-byte PKCS#8 v1 DER for `exportKey("pkcs8")`.
#[inline]
pub fn ed25519_seed_to_pkcs8_v1(seed: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(48);
    out.extend_from_slice(&ED25519_PKCS8_V1_PREFIX);
    out.extend_from_slice(seed);
    out
}

/// Wraps a 32-byte Ed25519 public key into standard 44-byte SPKI DER for `exportKey("spki")`.
#[inline]
pub fn ed25519_pub_to_spki(pub_bytes: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(44);
    out.extend_from_slice(&ED25519_SPKI_PREFIX);
    out.extend_from_slice(pub_bytes);
    out
}

/// Extracts a 32-byte Ed25519 seed from raw 32-byte input, 48-byte PKCS#8 v1, or 83/85-byte PKCS#8 v2.
/// Strictly verifies the Ed25519 OID (`1.3.101.112`) and validates any embedded PKCS#8 v2 public key.
pub fn extract_ed25519_seed(input: &[u8]) -> Result<[u8; 32], &'static str> {
    if input.len() == 32 {
        let mut s = [0u8; 32];
        s.copy_from_slice(input);
        return Ok(s);
    }
    if input.len() == 48 && input[..16] == ED25519_PKCS8_V1_PREFIX {
        let mut s = [0u8; 32];
        s.copy_from_slice(&input[16..48]);
        return Ok(s);
    }
    if (input.len() == 83 || input.len() == 85)
        && input[7..12] == ED25519_OID
        && input[12..16] == [0x04, 0x22, 0x04, 0x20]
    {
        // Verify that the PKCS#8 v2 structure and embedded public key match the private seed
        ring::signature::Ed25519KeyPair::from_pkcs8(input)
            .map_err(|_| "Invalid Ed25519 PKCS#8 v2 key or mismatched public key")?;
        let mut s = [0u8; 32];
        s.copy_from_slice(&input[16..48]);
        return Ok(s);
    }
    Err("Invalid Ed25519 private key or PKCS#8 encoding")
}

/// Extracts a 32-byte Ed25519 public key from raw 32-byte input or 44-byte SPKI DER.
pub fn extract_ed25519_pub(input: &[u8]) -> Result<[u8; 32], &'static str> {
    if input.len() == 32 {
        let mut p = [0u8; 32];
        p.copy_from_slice(input);
        return Ok(p);
    }
    if input.len() == 44 && input[..12] == ED25519_SPKI_PREFIX {
        let mut p = [0u8; 32];
        p.copy_from_slice(&input[12..44]);
        return Ok(p);
    }
    Err("Invalid Ed25519 public key or SPKI encoding")
}

/// Derives an Ed25519 `(seed, public_key)` pair from a 32-byte seed using `ring::signature::Ed25519KeyPair`.
pub fn ed25519_keypair_from_seed(seed: [u8; 32]) -> Result<([u8; 32], [u8; 32]), &'static str> {
    use ring::signature::KeyPair;
    let kp = ring::signature::Ed25519KeyPair::from_seed_unchecked(&seed)
        .map_err(|_| "Failed to derive Ed25519 key pair from seed")?;
    let mut pub_bytes = [0u8; 32];
    pub_bytes.copy_from_slice(kp.public_key().as_ref());
    Ok((seed, pub_bytes))
}

/// Signs `msg` with a 32-byte Ed25519 seed, returning the 64-byte signature.
#[inline]
pub fn ed25519_sign_bytes(seed: &[u8; 32], msg: &[u8]) -> Result<[u8; 64], &'static str> {
    let kp = ring::signature::Ed25519KeyPair::from_seed_unchecked(seed)
        .map_err(|_| "Invalid Ed25519 seed")?;
    let sig = kp.sign(msg);
    let mut out = [0u8; 64];
    out.copy_from_slice(sig.as_ref());
    Ok(out)
}

/// Verifies a 64-byte Ed25519 `signature` over `msg` using a 32-byte `pub_bytes`.
#[inline]
pub fn ed25519_verify_bytes(pub_bytes: &[u8], msg: &[u8], signature: &[u8]) -> bool {
    let peer = ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, pub_bytes);
    peer.verify(msg, signature).is_ok()
}

/// Wraps an ECDSA uncompressed public key (`0x04 || X || Y`) and private scalar `d` into PKCS#8 v1 DER
/// accepted by `ring::signature::EcdsaKeyPair::from_pkcs8`.
pub fn ecdsa_components_to_pkcs8(
    curve: &str,
    d: &[u8],
    pub_uncompressed: &[u8],
) -> Result<Vec<u8>, &'static str> {
    if curve.eq_ignore_ascii_case("P-256") || curve.eq_ignore_ascii_case("prime256v1") {
        if d.len() != 32 || pub_uncompressed.len() != 65 || pub_uncompressed[0] != 0x04 {
            return Err("Invalid P-256 key component lengths");
        }
        let mut out = Vec::with_capacity(138);
        out.extend_from_slice(&[
            0x30, 0x81, 0x87, 0x02, 0x01, 0x00, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce,
            0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x04,
            0x6d, 0x30, 0x6b, 0x02, 0x01, 0x01, 0x04, 0x20,
        ]);
        out.extend_from_slice(d);
        out.extend_from_slice(&[0xa1, 0x44, 0x03, 0x42, 0x00]);
        out.extend_from_slice(pub_uncompressed);
        Ok(out)
    } else if curve.eq_ignore_ascii_case("P-384") || curve.eq_ignore_ascii_case("secp384r1") {
        if d.len() != 48 || pub_uncompressed.len() != 97 || pub_uncompressed[0] != 0x04 {
            return Err("Invalid P-384 key component lengths");
        }
        let mut out = Vec::with_capacity(185);
        out.extend_from_slice(&[
            0x30, 0x81, 0xb6, 0x02, 0x01, 0x00, 0x30, 0x10, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce,
            0x3d, 0x02, 0x01, 0x06, 0x05, 0x2b, 0x81, 0x04, 0x00, 0x22, 0x04, 0x81, 0x9e, 0x30,
            0x81, 0x9b, 0x02, 0x01, 0x01, 0x04, 0x30,
        ]);
        out.extend_from_slice(d);
        out.extend_from_slice(&[0xa1, 0x64, 0x03, 0x62, 0x00]);
        out.extend_from_slice(pub_uncompressed);
        Ok(out)
    } else {
        Err("Unsupported ECDSA curve (expected P-256 or P-384)")
    }
}

/// Extracts `(d_bytes, pub_uncompressed_bytes)` from a P-256 or P-384 PKCS#8 v1 DER buffer.
pub fn extract_ecdsa_pkcs8_components(
    curve: &str,
    pkcs8: &[u8],
) -> Result<(Vec<u8>, Vec<u8>), &'static str> {
    let (scalar_len, point_len) =
        if curve.eq_ignore_ascii_case("P-256") || curve.eq_ignore_ascii_case("prime256v1") {
            (32usize, 65usize)
        } else if curve.eq_ignore_ascii_case("P-384") || curve.eq_ignore_ascii_case("secp384r1") {
            (48usize, 97usize)
        } else {
            return Err("Unsupported ECDSA curve");
        };
    // Validate with `ring` first and obtain public key directly from `KeyPair::public_key()`
    use ring::signature::KeyPair;
    let rng = ring::rand::SystemRandom::new();
    let alg = if scalar_len == 32 {
        &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING
    } else {
        &ring::signature::ECDSA_P384_SHA384_FIXED_SIGNING
    };
    let kp = ring::signature::EcdsaKeyPair::from_pkcs8(alg, pkcs8, &rng)
        .map_err(|_| "Invalid ECDSA PKCS#8 private key")?;
    let pub_bytes = kp.public_key().as_ref().to_vec();
    if pub_bytes.len() != point_len {
        return Err("Unexpected ECDSA public key length");
    }
    // Locate the `[0x04, scalar_len]` OCTET STRING tag inside PKCS#8
    let marker = [0x02, 0x01, 0x01, 0x04, scalar_len as u8];
    if let Some(pos) = memchr::memmem::find(pkcs8, &marker) {
        let start = pos + marker.len();
        if start + scalar_len <= pkcs8.len() {
            return Ok((pkcs8[start..start + scalar_len].to_vec(), pub_bytes));
        }
    }
    Err("Failed to locate ECDSA private scalar in PKCS#8")
}

/// Wraps an uncompressed ECDSA public point (`0x04 || X || Y`) into SPKI DER.
pub fn ecdsa_pub_to_spki(curve: &str, pub_uncompressed: &[u8]) -> Result<Vec<u8>, &'static str> {
    if curve.eq_ignore_ascii_case("P-256") || curve.eq_ignore_ascii_case("prime256v1") {
        if pub_uncompressed.len() != 65 {
            return Err("Invalid P-256 uncompressed public point length");
        }
        let mut out = Vec::with_capacity(91);
        out.extend_from_slice(&ECDSA_P256_SPKI_PREFIX);
        out.extend_from_slice(pub_uncompressed);
        Ok(out)
    } else if curve.eq_ignore_ascii_case("P-384") || curve.eq_ignore_ascii_case("secp384r1") {
        if pub_uncompressed.len() != 97 {
            return Err("Invalid P-384 uncompressed public point length");
        }
        let mut out = Vec::with_capacity(120);
        out.extend_from_slice(&ECDSA_P384_SPKI_PREFIX);
        out.extend_from_slice(pub_uncompressed);
        Ok(out)
    } else {
        Err("Unsupported ECDSA curve")
    }
}

/// Extracts an uncompressed ECDSA public point (`0x04 || X || Y`) from raw or SPKI DER.
pub fn extract_ecdsa_pub(curve: &str, input: &[u8]) -> Result<Vec<u8>, &'static str> {
    if curve.eq_ignore_ascii_case("P-256") || curve.eq_ignore_ascii_case("prime256v1") {
        if input.len() == 65 && input[0] == 0x04 {
            return Ok(input.to_vec());
        }
        if input.len() == 91 && input[..26] == ECDSA_P256_SPKI_PREFIX {
            return Ok(input[26..91].to_vec());
        }
        Err("Invalid P-256 public key or SPKI DER")
    } else if curve.eq_ignore_ascii_case("P-384") || curve.eq_ignore_ascii_case("secp384r1") {
        if input.len() == 97 && input[0] == 0x04 {
            return Ok(input.to_vec());
        }
        if input.len() == 120 && input[..23] == ECDSA_P384_SPKI_PREFIX {
            return Ok(input[23..120].to_vec());
        }
        Err("Invalid P-384 public key or SPKI DER")
    } else {
        Err("Unsupported ECDSA curve")
    }
}

pub type EcdsaGeneratedKeypair = (Vec<u8>, Vec<u8>, Vec<u8>);

/// Generates a new ECDSA (`P-256` or `P-384`) keypair, returning `(pkcs8_bytes, d_bytes, pub_uncompressed_bytes)`.
pub fn ecdsa_generate_keypair(curve: &str) -> Result<EcdsaGeneratedKeypair, &'static str> {
    let rng = ring::rand::SystemRandom::new();
    let alg = if curve.eq_ignore_ascii_case("P-256") || curve.eq_ignore_ascii_case("prime256v1") {
        &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING
    } else if curve.eq_ignore_ascii_case("P-384") || curve.eq_ignore_ascii_case("secp384r1") {
        &ring::signature::ECDSA_P384_SHA384_FIXED_SIGNING
    } else {
        return Err("Unsupported ECDSA curve (expected P-256 or P-384)");
    };
    let doc = ring::signature::EcdsaKeyPair::generate_pkcs8(alg, &rng)
        .map_err(|_| "Failed to generate ECDSA key pair")?;
    let pkcs8 = doc.as_ref().to_vec();
    let (d, pub_bytes) = extract_ecdsa_pkcs8_components(curve, &pkcs8)?;
    Ok((pkcs8, d, pub_bytes))
}

/// Converts a fixed IEEE P1363 `(r || s)` ECDSA signature into ASN.1 DER `SEQUENCE { INTEGER r, INTEGER s }`.
pub fn ecdsa_fixed_to_der(fixed: &[u8]) -> Result<Vec<u8>, &'static str> {
    if fixed.len() != 64 && fixed.len() != 96 {
        return Err("Invalid fixed ECDSA signature length");
    }
    let half = fixed.len() / 2;
    let encode_int = |slice: &[u8]| -> Vec<u8> {
        let mut s = slice;
        while s.len() > 1 && s[0] == 0 {
            s = &s[1..];
        }
        let mut out = Vec::with_capacity(s.len() + 3);
        out.push(0x02);
        if (s[0] & 0x80) != 0 {
            out.push((s.len() + 1) as u8);
            out.push(0x00);
        } else {
            out.push(s.len() as u8);
        }
        out.extend_from_slice(s);
        out
    };
    let r_der = encode_int(&fixed[..half]);
    let s_der = encode_int(&fixed[half..]);
    let seq_len = r_der.len() + s_der.len();
    let mut out = Vec::with_capacity(seq_len + 3);
    out.push(0x30);
    if seq_len >= 128 {
        out.push(0x81);
    }
    out.push(seq_len as u8);
    out.extend_from_slice(&r_der);
    out.extend_from_slice(&s_der);
    Ok(out)
}

/// Signs `msg` using an ECDSA PKCS#8 key (`P-256` or `P-384`), returning IEEE P1363 (`64`/`96` bytes)
/// when `asn1_der` is `false` or ASN.1 DER when `asn1_der` is `true`.
pub fn ecdsa_sign_bytes(
    curve: &str,
    pkcs8: &[u8],
    msg: &[u8],
    asn1_der: bool,
) -> Result<Vec<u8>, &'static str> {
    let rng = ring::rand::SystemRandom::new();
    let alg = if curve.eq_ignore_ascii_case("P-256") || curve.eq_ignore_ascii_case("prime256v1") {
        &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING
    } else if curve.eq_ignore_ascii_case("P-384") || curve.eq_ignore_ascii_case("secp384r1") {
        &ring::signature::ECDSA_P384_SHA384_FIXED_SIGNING
    } else {
        return Err("Unsupported ECDSA curve");
    };
    let kp = ring::signature::EcdsaKeyPair::from_pkcs8(alg, pkcs8, &rng)
        .map_err(|_| "Invalid ECDSA PKCS#8 key")?;
    let sig = kp
        .sign(&rng, msg)
        .map_err(|_| "ECDSA signing operation failed")?;
    if asn1_der {
        ecdsa_fixed_to_der(sig.as_ref())
    } else {
        Ok(sig.as_ref().to_vec())
    }
}

/// Verifies an ECDSA (`P-256` or `P-384`) signature over `msg` (accepting both IEEE P1363 fixed and ASN.1 DER formats).
pub fn ecdsa_verify_bytes(
    curve: &str,
    pub_uncompressed: &[u8],
    msg: &[u8],
    signature: &[u8],
) -> bool {
    let is_p256 = curve.eq_ignore_ascii_case("P-256") || curve.eq_ignore_ascii_case("prime256v1");
    let is_p384 = curve.eq_ignore_ascii_case("P-384") || curve.eq_ignore_ascii_case("secp384r1");
    if is_p256 {
        if signature.len() == 64 {
            let peer = ring::signature::UnparsedPublicKey::new(
                &ring::signature::ECDSA_P256_SHA256_FIXED,
                pub_uncompressed,
            );
            if peer.verify(msg, signature).is_ok() {
                return true;
            }
        }
        let peer_asn1 = ring::signature::UnparsedPublicKey::new(
            &ring::signature::ECDSA_P256_SHA256_ASN1,
            pub_uncompressed,
        );
        peer_asn1.verify(msg, signature).is_ok()
    } else if is_p384 {
        if signature.len() == 96 {
            let peer = ring::signature::UnparsedPublicKey::new(
                &ring::signature::ECDSA_P384_SHA384_FIXED,
                pub_uncompressed,
            );
            if peer.verify(msg, signature).is_ok() {
                return true;
            }
        }
        let peer_asn1 = ring::signature::UnparsedPublicKey::new(
            &ring::signature::ECDSA_P384_SHA384_ASN1,
            pub_uncompressed,
        );
        peer_asn1.verify(msg, signature).is_ok()
    } else {
        false
    }
}

/// Cached pre-expanded cryptographic key entry stored in the isolate-local / tenant-scoped slot table.
#[allow(dead_code, clippy::large_enum_variant)]
pub(crate) enum CachedCryptoKeyMaterial {
    AesGcm {
        less_safe: ring::aead::LessSafeKey,
        raw: Vec<u8>,
    },
    AesCbc {
        expanded: AesExpandedKey,
        raw: Vec<u8>,
    },
    Hmac {
        ring_key: ring::hmac::Key,
        hash_alg: String,
        raw: Vec<u8>,
    },
    Ed25519Private {
        seed: [u8; 32],
        pub_bytes: [u8; 32],
    },
    Ed25519Public {
        pub_bytes: [u8; 32],
    },
    EcdsaPrivate {
        curve: String,
        pkcs8: Vec<u8>,
        d_bytes: Vec<u8>,
        pub_bytes: Vec<u8>,
    },
    EcdsaPublic {
        curve: String,
        pub_bytes: Vec<u8>,
    },
}

pub(crate) struct TenantCachedCryptoKey {
    pub(crate) tenant_id: String,
    pub(crate) material: CachedCryptoKeyMaterial,
}

thread_local! {
    static NEXT_CRYPTO_KEY_HANDLE: std::cell::Cell<u32> = const { std::cell::Cell::new(1) };
    static CRYPTO_KEY_HANDLE_TABLE: std::cell::RefCell<rustc_hash::FxHashMap<u32, TenantCachedCryptoKey>> =
        std::cell::RefCell::new(rustc_hash::FxHashMap::default());
}

/// Inserts a pre-expanded `CachedCryptoKeyMaterial` into the tenant-scoped handle table, returning its opaque `u32` ID.
pub(crate) fn insert_cached_crypto_key(
    tenant_id: String,
    material: CachedCryptoKeyMaterial,
) -> u32 {
    let id = NEXT_CRYPTO_KEY_HANDLE.with(|c| {
        let cur = c.get();
        c.set(cur.wrapping_add(1).max(1));
        cur
    });
    CRYPTO_KEY_HANDLE_TABLE.with(|table| {
        let mut borrow = table.borrow_mut();
        if borrow.len() >= 1024 {
            borrow.retain(|_, v| v.tenant_id == tenant_id);
            if borrow.len() >= 1024 {
                borrow.clear();
            }
        }
        borrow.insert(
            id,
            TenantCachedCryptoKey {
                tenant_id,
                material,
            },
        );
    });
    id
}

/// Executes `f` with a reference to the cached `CachedCryptoKeyMaterial` for `handle_id` ONLY if its
/// `tenant_id` matches `expected_tenant`, enforcing strict multi-tenant key isolation.
#[inline]
pub(crate) fn with_cached_crypto_key<R>(
    handle_id: u32,
    expected_tenant: &str,
    f: impl FnOnce(&CachedCryptoKeyMaterial) -> R,
) -> Option<R> {
    if handle_id == 0 {
        return None;
    }
    CRYPTO_KEY_HANDLE_TABLE.with(|table| {
        let borrow = table.borrow();
        let entry = borrow.get(&handle_id)?;
        if entry.tenant_id != expected_tenant {
            return None;
        }
        Some(f(&entry.material))
    })
}

/// Clears all cached cryptographic key handles belonging to `tenant_id` (or all handles when `tenant_id` is empty).
#[allow(dead_code)]
pub fn clear_cached_crypto_keys_for_tenant(tenant_id: &str) {
    CRYPTO_KEY_HANDLE_TABLE.with(|table| {
        let mut borrow = table.borrow_mut();
        if tenant_id.is_empty() {
            borrow.clear();
        } else {
            borrow.retain(|_, v| v.tenant_id != tenant_id);
        }
    });
}
