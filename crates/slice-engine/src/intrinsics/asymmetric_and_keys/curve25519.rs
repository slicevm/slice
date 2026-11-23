//! `Ed25519` Signing/Verification and `X25519` Diffie-Hellman PKCS#8 / SPKI Key Codecs.

use crate::intrinsics::hash_and_hmac::constant_time_eq;

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
