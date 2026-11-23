//! `RSA 2048..8192` PKCS#1 / PKCS#8 / SPKI / JWK / ASN.1 DER Codecs & `RSASSA-PKCS1-v1_5` / `RSA-PSS` Signing.

/// ASN.1 DER `AlgorithmIdentifier` for `rsaEncryption` (`1.2.840.113549.1.1.1`) with `NULL` parameters.
const RSA_ENCRYPTION_ALG_ID: [u8; 15] = [
    0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01, 0x05, 0x00,
];
/// Prefix of PKCS#1 OID arc `1.2.840.113549.1.1.*` (`rsaEncryption = .1`, `id-RSASSA-PSS = .10`).
const RSA_OID_PREFIX: [u8; 10] = [0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01];

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

    let (pkcs8, (pkcs1_priv, pub_info)) =
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
