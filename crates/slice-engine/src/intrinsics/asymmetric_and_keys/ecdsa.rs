//! `ECDSA P-256` and `P-384` PKCS#8 / SPKI / IEEE P1363 / ASN.1 DER Codecs & Signing/Verification.

const ECDSA_P256_SPKI_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];
const ECDSA_P384_SPKI_PREFIX: [u8; 23] = [
    0x30, 0x76, 0x30, 0x10, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x05, 0x2b,
    0x81, 0x04, 0x00, 0x22, 0x03, 0x62, 0x00,
];

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
