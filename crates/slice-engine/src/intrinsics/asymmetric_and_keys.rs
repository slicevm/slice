//! Asymmetric Cryptography (`Ed25519`, `X25519`, `ECDSA P-256 / P-384`, `RSA 2048..8192`),
//! PKCS#8 / SPKI / JWK / IEEE P1363 / DER Codecs, and Tenant-Isolated `CachedCryptoKeyMaterial` Handle Table.

use super::hash_and_hmac::constant_time_eq;
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

const X25519_PKCS8_V1_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x6e, 0x04, 0x22, 0x04, 0x20,
];
const X25519_OID: [u8; 5] = [0x06, 0x03, 0x2b, 0x65, 0x6e];
const X25519_SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x6e, 0x03, 0x21, 0x00,
];

/// ASN.1 DER `AlgorithmIdentifier` for `rsaEncryption` (`1.2.840.113549.1.1.1`) with `NULL` parameters.
const RSA_ENCRYPTION_ALG_ID: [u8; 15] = [
    0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01, 0x05, 0x00,
];
/// Prefix of PKCS#1 OID arc `1.2.840.113549.1.1.*` (`rsaEncryption = .1`, `id-RSASSA-PSS = .10`).
const RSA_OID_PREFIX: [u8; 10] = [0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01];

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

/// Wraps a 32-byte X25519 private scalar into standard 48-byte PKCS#8 v1 DER.
#[inline]
pub fn x25519_seed_to_pkcs8_v1(seed: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(48);
    out.extend_from_slice(&X25519_PKCS8_V1_PREFIX);
    out.extend_from_slice(seed);
    out
}

/// Wraps a 32-byte X25519 public key into standard 44-byte SPKI DER.
#[inline]
pub fn x25519_pub_to_spki(pub_bytes: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(44);
    out.extend_from_slice(&X25519_SPKI_PREFIX);
    out.extend_from_slice(pub_bytes);
    out
}

/// Extracts a 32-byte X25519 private scalar from raw 32-byte input, 48-byte PKCS#8 v1, or 83/85-byte PKCS#8 v2 DER.
pub fn extract_x25519_seed(input: &[u8]) -> Result<[u8; 32], &'static str> {
    if input.len() == 32 {
        let mut s = [0u8; 32];
        s.copy_from_slice(input);
        return Ok(s);
    }
    if input.len() == 48 && input[..16] == X25519_PKCS8_V1_PREFIX {
        let mut s = [0u8; 32];
        s.copy_from_slice(&input[16..48]);
        return Ok(s);
    }
    if (input.len() == 83 || input.len() == 85)
        && input[7..12] == X25519_OID
        && input[12..16] == [0x04, 0x22, 0x04, 0x20]
    {
        let mut s = [0u8; 32];
        s.copy_from_slice(&input[16..48]);
        let (_, derived_pub) = x25519_keypair_from_seed(s);
        let pub_offset = input.len() - 32;
        if !constant_time_eq(&derived_pub, &input[pub_offset..]) {
            return Err("Invalid X25519 PKCS#8 v2 key or mismatched public key");
        }
        return Ok(s);
    }
    Err("Invalid X25519 private key or PKCS#8 encoding")
}

/// Extracts a 32-byte X25519 public key from raw 32-byte input or 44-byte SPKI DER.
pub fn extract_x25519_pub(input: &[u8]) -> Result<[u8; 32], &'static str> {
    if input.len() == 32 {
        let mut p = [0u8; 32];
        p.copy_from_slice(input);
        return Ok(p);
    }
    if input.len() == 44 && input[..12] == X25519_SPKI_PREFIX {
        let mut p = [0u8; 32];
        p.copy_from_slice(&input[12..44]);
        return Ok(p);
    }
    Err("Invalid X25519 public key or SPKI encoding")
}

/// Derives an RFC 7748 X25519 `(seed, public_key)` pair using constant-time Curve25519 basepoint multiplication.
pub fn x25519_keypair_from_seed(seed: [u8; 32]) -> ([u8; 32], [u8; 32]) {
    let secret = x25519_dalek::StaticSecret::from(seed);
    let public = x25519_dalek::PublicKey::from(&secret);
    (seed, public.to_bytes())
}

/// Computes an RFC 7748 X25519 shared secret (`X25519(priv_seed, peer_pub)`), rejecting all-zero
/// low-order public points in constant time per RFC 7748 Section 6.1 and WebCrypto X25519 requirements.
pub fn x25519_diffie_hellman(
    priv_seed: &[u8; 32],
    peer_pub: &[u8; 32],
) -> Result<[u8; 32], &'static str> {
    let secret = x25519_dalek::StaticSecret::from(*priv_seed);
    let peer = x25519_dalek::PublicKey::from(*peer_pub);
    let shared = secret.diffie_hellman(&peer).to_bytes();
    if constant_time_eq(&shared, &[0u8; 32]) {
        return Err("OperationError: Invalid low-order X25519 public key");
    }
    Ok(shared)
}

#[inline]
fn encode_der_len(len: usize, out: &mut Vec<u8>) {
    if len < 128 {
        out.push(len as u8);
    } else if len <= 0xff {
        out.push(0x81);
        out.push(len as u8);
    } else if len <= 0xffff {
        out.push(0x82);
        out.push((len >> 8) as u8);
        out.push((len & 0xff) as u8);
    } else {
        out.push(0x83);
        out.push((len >> 16) as u8);
        out.push(((len >> 8) & 0xff) as u8);
        out.push((len & 0xff) as u8);
    }
}

#[inline]
fn encode_der_tlv(tag: u8, payload: &[u8], out: &mut Vec<u8>) {
    out.push(tag);
    encode_der_len(payload.len(), out);
    out.extend_from_slice(payload);
}

fn encode_der_uint_be(unsigned_be: &[u8], out: &mut Vec<u8>) -> Result<(), &'static str> {
    if unsigned_be.is_empty() {
        return Err("Empty ASN.1 INTEGER component");
    }
    let mut s = unsigned_be;
    while s.len() > 1 && s[0] == 0 {
        s = &s[1..];
    }
    let need_pad = (s[0] & 0x80) != 0;
    out.push(0x02);
    encode_der_len(s.len() + usize::from(need_pad), out);
    if need_pad {
        out.push(0x00);
    }
    out.extend_from_slice(s);
    Ok(())
}

fn parse_der_tlv(input: &[u8]) -> Result<(u8, &[u8], &[u8]), &'static str> {
    if input.len() < 2 {
        return Err("Truncated ASN.1 DER element");
    }
    let tag = input[0];
    let b1 = input[1];
    let (hdr_len, content_len) = if (b1 & 0x80) == 0 {
        (2usize, b1 as usize)
    } else {
        let num_octets = (b1 & 0x7f) as usize;
        if num_octets == 0 || num_octets > 3 || input.len() < 2 + num_octets {
            return Err("Invalid ASN.1 DER length encoding");
        }
        let mut len = 0usize;
        for &b in &input[2..2 + num_octets] {
            len = (len << 8) | (b as usize);
        }
        (2 + num_octets, len)
    };
    if input.len() < hdr_len + content_len {
        return Err("ASN.1 DER content exceeds buffer bounds");
    }
    Ok((
        tag,
        &input[hdr_len..hdr_len + content_len],
        &input[hdr_len + content_len..],
    ))
}

fn parse_der_uint_be(input: &[u8]) -> Result<(&[u8], &[u8]), &'static str> {
    let (tag, val, rest) = parse_der_tlv(input)?;
    if tag != 0x02 || val.is_empty() {
        return Err("Expected non-empty ASN.1 INTEGER");
    }
    if (val[0] & 0x80) != 0 {
        return Err("Negative ASN.1 INTEGER not valid for RSA key component");
    }
    let unsigned = if val.len() > 1 && val[0] == 0x00 {
        if (val[1] & 0x80) == 0 {
            return Err("Non-minimal ASN.1 INTEGER padding");
        }
        &val[1..]
    } else {
        val
    };
    Ok((unsigned, rest))
}

/// Wraps a PKCS#1 `RSAPublicKey` DER (`SEQUENCE { INTEGER n, INTEGER e }`) into canonical
/// `SubjectPublicKeyInfo` (`SPKI`) DER for any RSA modulus size (`2048` to `8192` bits).
pub fn rsa_pkcs1_pub_to_spki(rsa_pub_der: &[u8]) -> Vec<u8> {
    let bit_string_len = 1 + rsa_pub_der.len();
    let mut inner = Vec::with_capacity(RSA_ENCRYPTION_ALG_ID.len() + 5 + bit_string_len);
    inner.extend_from_slice(&RSA_ENCRYPTION_ALG_ID);
    inner.push(0x03);
    encode_der_len(bit_string_len, &mut inner);
    inner.push(0x00);
    inner.extend_from_slice(rsa_pub_der);

    let mut spki = Vec::with_capacity(inner.len() + 5);
    encode_der_tlv(0x30, &inner, &mut spki);
    spki
}

/// Wraps a PKCS#1 `RSAPrivateKey` DER into canonical PKCS#8 v1 DER.
pub fn rsa_pkcs1_priv_to_pkcs8(rsa_priv_der: &[u8]) -> Vec<u8> {
    let mut inner = Vec::with_capacity(3 + RSA_ENCRYPTION_ALG_ID.len() + 5 + rsa_priv_der.len());
    inner.extend_from_slice(&[0x02, 0x01, 0x00]);
    inner.extend_from_slice(&RSA_ENCRYPTION_ALG_ID);
    encode_der_tlv(0x04, rsa_priv_der, &mut inner);

    let mut pkcs8 = Vec::with_capacity(inner.len() + 5);
    encode_der_tlv(0x30, &inner, &mut pkcs8);
    pkcs8
}

/// Validated RSA public key metadata and canonical DER encodings (`2048` to `8192` bits).
#[derive(Clone, Debug)]
pub struct RsaPublicKeyInfo {
    pub modulus_bits: usize,
    pub public_exponent: u64,
    pub n_be: Vec<u8>,
    pub e_be: Vec<u8>,
    pub pkcs1_pub: Vec<u8>,
    pub spki: Vec<u8>,
}

/// Validated RSA private key JWK CRT components (`n`, `e`, `d`, `p`, `q`, `dp`, `dq`, `qi`).
#[derive(Clone, Debug)]
pub struct RsaPrivateJwkInfo {
    pub d_be: Vec<u8>,
    pub p_be: Vec<u8>,
    pub q_be: Vec<u8>,
    pub dp_be: Vec<u8>,
    pub dq_be: Vec<u8>,
    pub qi_be: Vec<u8>,
}

fn parse_rsa_pkcs1_pub_der(rsa_pub_der: &[u8]) -> Result<RsaPublicKeyInfo, &'static str> {
    let (seq_tag, seq_body, trailing) = parse_der_tlv(rsa_pub_der)?;
    if seq_tag != 0x30 || !trailing.is_empty() {
        return Err("Invalid RSAPublicKey ASN.1 SEQUENCE");
    }
    let (n_be, after_n) = parse_der_uint_be(seq_body)?;
    let (e_be, after_e) = parse_der_uint_be(after_n)?;
    if !after_e.is_empty() {
        return Err("Unexpected trailing fields in RSAPublicKey");
    }
    if n_be.is_empty() || (n_be[n_be.len() - 1] & 1) == 0 {
        return Err("Invalid RSA modulus (must be odd)");
    }
    let modulus_bits = (n_be.len() - 1) * 8 + (8 - n_be[0].leading_zeros() as usize);
    if !(2048..=8192).contains(&modulus_bits) {
        return Err("RSA modulus length must be between 2048 and 8192 bits");
    }
    if e_be.is_empty() || e_be.len() > 8 {
        return Err("Unsupported RSA public exponent length");
    }
    let public_exponent = e_be.iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b));
    if public_exponent < 3 || (public_exponent & 1) == 0 {
        return Err("Invalid RSA public exponent");
    }
    let pkcs1_pub = rsa_pub_der.to_vec();
    let spki = rsa_pkcs1_pub_to_spki(&pkcs1_pub);
    Ok(RsaPublicKeyInfo {
        modulus_bits,
        public_exponent,
        n_be: n_be.to_vec(),
        e_be: e_be.to_vec(),
        pkcs1_pub,
        spki,
    })
}

/// Parses and validates an RSA public key in either `SubjectPublicKeyInfo` (`SPKI`) DER or
/// raw PKCS#1 `RSAPublicKey` DER format (`2048` to `8192` bits).
pub fn inspect_rsa_public_key(input: &[u8]) -> Result<RsaPublicKeyInfo, &'static str> {
    let (outer_tag, outer_body, outer_rest) = parse_der_tlv(input)?;
    if outer_tag != 0x30 || !outer_rest.is_empty() {
        return Err("Invalid RSA public key DER");
    }
    // Check whether `outer_body` begins with an `AlgorithmIdentifier` SEQUENCE (`SPKI`) or an `INTEGER` (`RSAPublicKey`).
    let (first_tag, first_val, after_first) = parse_der_tlv(outer_body)?;
    if first_tag == 0x30 {
        if first_val.len() < RSA_OID_PREFIX.len() + 1
            || !first_val.starts_with(&RSA_OID_PREFIX)
            || !matches!(first_val[RSA_OID_PREFIX.len()], 0x01 | 0x0a)
        {
            return Err("SPKI AlgorithmIdentifier OID is not RSA");
        }
        let (bs_tag, bs_val, after_bs) = parse_der_tlv(after_first)?;
        if bs_tag != 0x03 || !after_bs.is_empty() || bs_val.len() < 2 || bs_val[0] != 0x00 {
            return Err("Invalid SPKI BIT STRING for RSA public key");
        }
        parse_rsa_pkcs1_pub_der(&bs_val[1..])
    } else if first_tag == 0x02 {
        parse_rsa_pkcs1_pub_der(input)
    } else {
        Err("Unrecognized RSA public key DER structure")
    }
}

/// Parses and cryptographically validates an RSA private key (accepting both PKCS#8 v1 DER and
/// PKCS#1 `RSAPrivateKey` DER) via `ring::signature::RsaKeyPair`, returning `(canonical_pkcs8, pub_info, priv_jwk)`.
pub fn inspect_rsa_private_key(
    input: &[u8],
) -> Result<(Vec<u8>, RsaPublicKeyInfo, RsaPrivateJwkInfo), &'static str> {
    use ring::signature::KeyPair;

    let (pkcs8, pkcs1_priv_slice_vec) =
        if let Ok(kp) = ring::signature::RsaKeyPair::from_pkcs8(input) {
            let (_outer_tag, outer_body, _) = parse_der_tlv(input)?;
            let (_ver_tag, _ver_val, after_ver) = parse_der_tlv(outer_body)?;
            let (_alg_tag, _alg_val, after_alg) = parse_der_tlv(after_ver)?;
            let (oct_tag, oct_val, _) = parse_der_tlv(after_alg)?;
            if oct_tag != 0x04 {
                return Err("Invalid PKCS#8 privateKey OCTET STRING");
            }
            let pub_info = parse_rsa_pkcs1_pub_der(kp.public_key().as_ref())?;
            (input.to_vec(), (oct_val.to_vec(), pub_info))
        } else if let Ok(kp) = ring::signature::RsaKeyPair::from_der(input) {
            let wrapped = rsa_pkcs1_priv_to_pkcs8(input);
            ring::signature::RsaKeyPair::from_pkcs8(&wrapped)
                .map_err(|_| "Invalid RSA PKCS#1 private key")?;
            let pub_info = parse_rsa_pkcs1_pub_der(kp.public_key().as_ref())?;
            (wrapped, (input.to_vec(), pub_info))
        } else {
            return Err("Invalid RSA PKCS#8 or PKCS#1 private key");
        };

    let (pkcs1_priv, pub_info) = pkcs1_priv_vec_split(pkcs1_priv_slice_vec);
    let (seq_tag, seq_body, _) = parse_der_tlv(&pkcs1_priv)?;
    if seq_tag != 0x30 {
        return Err("Invalid RSAPrivateKey SEQUENCE");
    }
    let (_version, r1) = parse_der_uint_be(seq_body)?;
    let (_n, r2) = parse_der_uint_be(r1)?;
    let (_e, r3) = parse_der_uint_be(r2)?;
    let (d, r4) = parse_der_uint_be(r3)?;
    let (p, r5) = parse_der_uint_be(r4)?;
    let (q, r6) = parse_der_uint_be(r5)?;
    let (dp, r7) = parse_der_uint_be(r6)?;
    let (dq, r8) = parse_der_uint_be(r7)?;
    let (qi, _r9) = parse_der_uint_be(r8)?;

    Ok((
        pkcs8,
        pub_info,
        RsaPrivateJwkInfo {
            d_be: d.to_vec(),
            p_be: p.to_vec(),
            q_be: q.to_vec(),
            dp_be: dp.to_vec(),
            dq_be: dq.to_vec(),
            qi_be: qi.to_vec(),
        },
    ))
}

#[inline]
fn pkcs1_priv_vec_split(pair: (Vec<u8>, RsaPublicKeyInfo)) -> (Vec<u8>, RsaPublicKeyInfo) {
    pair
}

/// Builds and validates an `RsaPublicKeyInfo` from big-endian JWK `(n, e)` components.
pub fn rsa_public_components_to_info(
    n_be: &[u8],
    e_be: &[u8],
) -> Result<RsaPublicKeyInfo, &'static str> {
    let mut seq_inner = Vec::with_capacity(n_be.len() + e_be.len() + 10);
    encode_der_uint_be(n_be, &mut seq_inner)?;
    encode_der_uint_be(e_be, &mut seq_inner)?;
    let mut pkcs1_pub = Vec::with_capacity(seq_inner.len() + 5);
    encode_der_tlv(0x30, &seq_inner, &mut pkcs1_pub);
    parse_rsa_pkcs1_pub_der(&pkcs1_pub)
}

/// Builds and cryptographically validates an RSA PKCS#8 v1 private key from big-endian JWK CRT components
/// (`n`, `e`, `d`, `p`, `q`, `dp`, `dq`, `qi`) via `ring::signature::RsaKeyPair::from_pkcs8`.
#[allow(clippy::too_many_arguments)]
pub fn rsa_private_components_to_pkcs8(
    n_be: &[u8],
    e_be: &[u8],
    d_be: &[u8],
    p_be: &[u8],
    q_be: &[u8],
    dp_be: &[u8],
    dq_be: &[u8],
    qi_be: &[u8],
) -> Result<(Vec<u8>, RsaPublicKeyInfo), &'static str> {
    use ring::signature::KeyPair;
    let mut seq_inner = Vec::with_capacity(n_be.len() * 5 + 48);
    seq_inner.extend_from_slice(&[0x02, 0x01, 0x00]); // version = 0
    encode_der_uint_be(n_be, &mut seq_inner)?;
    encode_der_uint_be(e_be, &mut seq_inner)?;
    encode_der_uint_be(d_be, &mut seq_inner)?;
    encode_der_uint_be(p_be, &mut seq_inner)?;
    encode_der_uint_be(q_be, &mut seq_inner)?;
    encode_der_uint_be(dp_be, &mut seq_inner)?;
    encode_der_uint_be(dq_be, &mut seq_inner)?;
    encode_der_uint_be(qi_be, &mut seq_inner)?;

    let mut pkcs1_priv = Vec::with_capacity(seq_inner.len() + 5);
    encode_der_tlv(0x30, &seq_inner, &mut pkcs1_priv);
    let pkcs8 = rsa_pkcs1_priv_to_pkcs8(&pkcs1_priv);

    let kp = ring::signature::RsaKeyPair::from_pkcs8(&pkcs8)
        .map_err(|_| "DataError: Invalid or inconsistent RSA JWK private key parameters")?;
    let pub_info = parse_rsa_pkcs1_pub_der(kp.public_key().as_ref())?;
    Ok((pkcs8, pub_info))
}

/// Extracts canonical RSA `SubjectPublicKeyInfo` (`SPKI`) DER from a PKCS#8 or PKCS#1 private key.
pub fn extract_rsa_pkcs8_spki(pkcs8: &[u8]) -> Result<Vec<u8>, &'static str> {
    let (_, pub_info, _) = inspect_rsa_private_key(pkcs8)?;
    Ok(pub_info.spki)
}

/// Normalizes an RSA public key input (either `SPKI` DER or PKCS#1 `RSAPublicKey` DER) to canonical `SPKI` DER.
pub fn extract_rsa_spki(input: &[u8]) -> Result<Vec<u8>, &'static str> {
    Ok(inspect_rsa_public_key(input)?.spki)
}

/// Signs `msg` using an RSA PKCS#8 private key with either `RSASSA-PKCS1-v1_5` (`use_pss = false`)
/// or `RSA-PSS` (`use_pss = true`) over `SHA-256`, `SHA-384`, or `SHA-512`.
pub fn rsa_sign_bytes_with_alg(
    pkcs8: &[u8],
    msg: &[u8],
    use_pss: bool,
    hash_alg: &str,
) -> Result<Vec<u8>, &'static str> {
    let kp = ring::signature::RsaKeyPair::from_pkcs8(pkcs8)
        .or_else(|_| ring::signature::RsaKeyPair::from_der(pkcs8))
        .map_err(|_| "Invalid RSA PKCS#8 private key")?;
    let is_384 = hash_alg.contains("384");
    let is_512 = hash_alg.contains("512");
    let encoding: &'static dyn ring::signature::RsaEncoding = match (use_pss, is_384, is_512) {
        (false, false, false) => &ring::signature::RSA_PKCS1_SHA256,
        (false, true, false) => &ring::signature::RSA_PKCS1_SHA384,
        (false, _, true) => &ring::signature::RSA_PKCS1_SHA512,
        (true, false, false) => &ring::signature::RSA_PSS_SHA256,
        (true, true, false) => &ring::signature::RSA_PSS_SHA384,
        (true, _, true) => &ring::signature::RSA_PSS_SHA512,
    };
    let rng = ring::rand::SystemRandom::new();
    let mut sig = vec![0u8; kp.public().modulus_len()];
    kp.sign(encoding, &rng, msg, &mut sig)
        .map_err(|_| "RSA signing operation failed")?;
    Ok(sig)
}

/// Signs `msg` using an RSA PKCS#8 private key with `RSASSA-PKCS1-v1_5` `SHA-256`.
#[inline]
pub fn rsa_sign_bytes(pkcs8: &[u8], msg: &[u8]) -> Result<Vec<u8>, &'static str> {
    rsa_sign_bytes_with_alg(pkcs8, msg, false, "SHA-256")
}

/// Verifies an RSA `RSASSA-PKCS1-v1_5` or `RSA-PSS` signature (`SHA-256`, `SHA-384`, or `SHA-512`)
/// over `msg` using an RSA `SPKI` or PKCS#1 `RSAPublicKey` DER (`2048` to `8192` bits).
pub fn rsa_verify_bytes_with_alg(
    spki_or_pub: &[u8],
    msg: &[u8],
    signature: &[u8],
    use_pss: bool,
    hash_alg: &str,
) -> bool {
    let Ok(pub_info) = inspect_rsa_public_key(spki_or_pub) else {
        return false;
    };
    let is_384 = hash_alg.contains("384");
    let is_512 = hash_alg.contains("512");
    let peer = match (use_pss, is_384, is_512) {
        (false, false, false) => ring::signature::UnparsedPublicKey::new(
            &ring::signature::RSA_PKCS1_2048_8192_SHA256,
            &pub_info.pkcs1_pub,
        ),
        (false, true, false) => ring::signature::UnparsedPublicKey::new(
            &ring::signature::RSA_PKCS1_2048_8192_SHA384,
            &pub_info.pkcs1_pub,
        ),
        (false, _, true) => ring::signature::UnparsedPublicKey::new(
            &ring::signature::RSA_PKCS1_2048_8192_SHA512,
            &pub_info.pkcs1_pub,
        ),
        (true, false, false) => ring::signature::UnparsedPublicKey::new(
            &ring::signature::RSA_PSS_2048_8192_SHA256,
            &pub_info.pkcs1_pub,
        ),
        (true, true, false) => ring::signature::UnparsedPublicKey::new(
            &ring::signature::RSA_PSS_2048_8192_SHA384,
            &pub_info.pkcs1_pub,
        ),
        (true, _, true) => ring::signature::UnparsedPublicKey::new(
            &ring::signature::RSA_PSS_2048_8192_SHA512,
            &pub_info.pkcs1_pub,
        ),
    };
    peer.verify(msg, signature).is_ok()
}

/// Verifies an RSA `RSASSA-PKCS1-v1_5` `SHA-256` signature over `msg`.
#[inline]
pub fn rsa_verify_bytes(spki_or_pub: &[u8], msg: &[u8], signature: &[u8]) -> bool {
    rsa_verify_bytes_with_alg(spki_or_pub, msg, signature, false, "SHA-256")
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
    X25519Private {
        seed: [u8; 32],
        pub_bytes: [u8; 32],
    },
    X25519Public {
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
    RsaPrivate {
        pkcs8: Vec<u8>,
        spki: Vec<u8>,
    },
    RsaPublic {
        spki: Vec<u8>,
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
