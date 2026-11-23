//! Asymmetric Key Import (`Ed25519`, `X25519`, `ECDSA`, `RSA` across `raw`, `pkcs8`, `spki`, `jwk`).

use crate::bindings::helpers::extract_buffer_source_bytes;
use crate::boa_realm::BOA_ACTIVE_TENANT;
use crate::intrinsics::{
    CachedCryptoKeyMaterial, base64_decode, base64_encode, constant_time_eq,
    ecdsa_components_to_pkcs8, ecdsa_pub_to_spki, ed25519_keypair_from_seed, ed25519_pub_to_spki,
    ed25519_seed_to_pkcs8_v1, extract_ecdsa_pkcs8_components, extract_ecdsa_pub,
    extract_ed25519_pub, extract_ed25519_seed, extract_x25519_pub, extract_x25519_seed,
    insert_cached_crypto_key, inspect_rsa_private_key, inspect_rsa_public_key,
    rsa_private_components_to_pkcs8, rsa_public_components_to_info, x25519_keypair_from_seed,
    x25519_pub_to_spki, x25519_seed_to_pkcs8_v1,
};

fn build_rsa_private_import_json(
    tenant: String,
    pkcs8: Vec<u8>,
    pub_info: crate::intrinsics::RsaPublicKeyInfo,
    priv_jwk: crate::intrinsics::RsaPrivateJwkInfo,
) -> serde_json::Value {
    let pkcs8_b64 = base64_encode(&pkcs8, false);
    let spki_b64 = base64_encode(&pub_info.spki, false);
    let id = insert_cached_crypto_key(
        tenant,
        CachedCryptoKeyMaterial::RsaPrivate {
            pkcs8,
            spki: pub_info.spki,
        },
    );
    serde_json::json!({
        "id": id,
        "type": "private",
        "modulusLength": pub_info.modulus_bits,
        "publicExponent": pub_info.public_exponent,
        "n": base64_encode(&pub_info.n_be, true),
        "e": base64_encode(&pub_info.e_be, true),
        "d": base64_encode(&priv_jwk.d_be, true),
        "p": base64_encode(&priv_jwk.p_be, true),
        "q": base64_encode(&priv_jwk.q_be, true),
        "dp": base64_encode(&priv_jwk.dp_be, true),
        "dq": base64_encode(&priv_jwk.dq_be, true),
        "qi": base64_encode(&priv_jwk.qi_be, true),
        "pkcs8": pkcs8_b64,
        "spki": spki_b64,
    })
}

fn build_rsa_public_import_json(
    tenant: String,
    pub_info: crate::intrinsics::RsaPublicKeyInfo,
) -> serde_json::Value {
    let spki_b64 = base64_encode(&pub_info.spki, false);
    let id = insert_cached_crypto_key(
        tenant,
        CachedCryptoKeyMaterial::RsaPublic {
            spki: pub_info.spki,
        },
    );
    serde_json::json!({
        "id": id,
        "type": "public",
        "modulusLength": pub_info.modulus_bits,
        "publicExponent": pub_info.public_exponent,
        "n": base64_encode(&pub_info.n_be, true),
        "e": base64_encode(&pub_info.e_be, true),
        "pubRaw": spki_b64,
        "spki": spki_b64,
    })
}

fn build_ed25519_private_json(
    tenant: String,
    seed_bytes: [u8; 32],
    pub_bytes: [u8; 32],
) -> serde_json::Value {
    let pkcs8 = ed25519_seed_to_pkcs8_v1(&seed_bytes);
    let id = insert_cached_crypto_key(
        tenant,
        CachedCryptoKeyMaterial::Ed25519Private {
            seed: seed_bytes,
            pub_bytes,
        },
    );
    serde_json::json!({
        "id": id,
        "type": "private",
        "d": base64_encode(&seed_bytes, true),
        "x": base64_encode(&pub_bytes, true),
        "pkcs8": base64_encode(&pkcs8, false),
    })
}

fn build_ed25519_public_json(tenant: String, pub_bytes: [u8; 32]) -> serde_json::Value {
    let spki = ed25519_pub_to_spki(&pub_bytes);
    let id = insert_cached_crypto_key(tenant, CachedCryptoKeyMaterial::Ed25519Public { pub_bytes });
    serde_json::json!({
        "id": id,
        "type": "public",
        "x": base64_encode(&pub_bytes, true),
        "pubRaw": base64_encode(&pub_bytes, false),
        "spki": base64_encode(&spki, false),
    })
}

fn build_x25519_private_json(
    tenant: String,
    seed_bytes: [u8; 32],
    pub_bytes: [u8; 32],
) -> serde_json::Value {
    let pkcs8 = x25519_seed_to_pkcs8_v1(&seed_bytes);
    let spki = x25519_pub_to_spki(&pub_bytes);
    let id = insert_cached_crypto_key(
        tenant,
        CachedCryptoKeyMaterial::X25519Private {
            seed: seed_bytes,
            pub_bytes,
        },
    );
    serde_json::json!({
        "id": id,
        "type": "private",
        "crv": "X25519",
        "d": base64_encode(&seed_bytes, true),
        "x": base64_encode(&pub_bytes, true),
        "pkcs8": base64_encode(&pkcs8, false),
        "spki": base64_encode(&spki, false),
    })
}

fn build_x25519_public_json(tenant: String, pub_bytes: [u8; 32]) -> serde_json::Value {
    let spki = x25519_pub_to_spki(&pub_bytes);
    let id = insert_cached_crypto_key(tenant, CachedCryptoKeyMaterial::X25519Public { pub_bytes });
    serde_json::json!({
        "id": id,
        "type": "public",
        "crv": "X25519",
        "x": base64_encode(&pub_bytes, true),
        "pubRaw": base64_encode(&pub_bytes, false),
        "spki": base64_encode(&spki, false),
    })
}

pub(super) fn register_asym_import_host_function(ctx: &mut boa_engine::Context) {
    use boa_engine::{JsNativeError, JsValue as BoaVal, NativeFunction, js_string};

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_asym_import"),
        4,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let alg = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let curve = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "P-256".to_string());
            let format = args
                .get(2)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "raw".to_string());
            let Some(data_arg) = args.get(3) else {
                return Err(JsNativeError::typ()
                    .with_message("importKey requires keyData")
                    .into());
            };
            let tenant = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());

            if alg.eq_ignore_ascii_case("Ed25519") {
                if format.eq_ignore_ascii_case("jwk") {
                    let jwk_str = data_arg.to_string(context)?.to_std_string_escaped();
                    let jwk: serde_json::Value = serde_json::from_str(&jwk_str).map_err(|_| {
                        JsNativeError::error().with_message("DataError: Invalid JWK JSON")
                    })?;
                    let x_str = jwk.get("x").and_then(|v| v.as_str()).unwrap_or("");
                    if let Some(d_str) = jwk.get("d").and_then(|v| v.as_str()) {
                        let seed = extract_ed25519_seed(&base64_decode(d_str))
                            .map_err(|msg| JsNativeError::error().with_message(msg))?;
                        let (seed_bytes, pub_bytes) = ed25519_keypair_from_seed(seed)
                            .map_err(|msg| JsNativeError::error().with_message(msg))?;
                        let out = build_ed25519_private_json(tenant, seed_bytes, pub_bytes);
                        return Ok(BoaVal::from(js_string!(out.to_string())));
                    }
                    let pub_bytes = extract_ed25519_pub(&base64_decode(x_str))
                        .map_err(|msg| JsNativeError::error().with_message(msg))?;
                    let out = build_ed25519_public_json(tenant, pub_bytes);
                    return Ok(BoaVal::from(js_string!(out.to_string())));
                }
                let raw = extract_buffer_source_bytes(data_arg, context)?;
                if format.eq_ignore_ascii_case("pkcs8") {
                    let seed = extract_ed25519_seed(&raw)
                        .map_err(|msg| JsNativeError::error().with_message(msg))?;
                    let (seed_bytes, pub_bytes) = ed25519_keypair_from_seed(seed)
                        .map_err(|msg| JsNativeError::error().with_message(msg))?;
                    let out = build_ed25519_private_json(tenant, seed_bytes, pub_bytes);
                    return Ok(BoaVal::from(js_string!(out.to_string())));
                }
                let pub_bytes = extract_ed25519_pub(&raw)
                    .map_err(|msg| JsNativeError::error().with_message(msg))?;
                let out = build_ed25519_public_json(tenant, pub_bytes);
                return Ok(BoaVal::from(js_string!(out.to_string())));
            }

            if alg.eq_ignore_ascii_case("ECDSA") || alg.eq_ignore_ascii_case("ec") {
                let norm_curve = if curve.eq_ignore_ascii_case("P-384")
                    || curve.eq_ignore_ascii_case("secp384r1")
                {
                    "P-384"
                } else {
                    "P-256"
                };
                if format.eq_ignore_ascii_case("jwk") {
                    let jwk_str = data_arg.to_string(context)?.to_std_string_escaped();
                    let jwk: serde_json::Value = serde_json::from_str(&jwk_str).map_err(|_| {
                        JsNativeError::error().with_message("DataError: Invalid JWK JSON")
                    })?;
                    let x_bytes =
                        base64_decode(jwk.get("x").and_then(|v| v.as_str()).unwrap_or(""));
                    let y_bytes =
                        base64_decode(jwk.get("y").and_then(|v| v.as_str()).unwrap_or(""));
                    let mut pub_bytes = Vec::with_capacity(1 + x_bytes.len() + y_bytes.len());
                    pub_bytes.push(0x04);
                    pub_bytes.extend_from_slice(&x_bytes);
                    pub_bytes.extend_from_slice(&y_bytes);
                    if let Some(d_str) = jwk.get("d").and_then(|v| v.as_str()) {
                        let d_bytes = base64_decode(d_str);
                        let pkcs8 = ecdsa_components_to_pkcs8(norm_curve, &d_bytes, &pub_bytes)
                            .map_err(|msg| JsNativeError::error().with_message(msg))?;
                        let id = insert_cached_crypto_key(
                            tenant,
                            CachedCryptoKeyMaterial::EcdsaPrivate {
                                curve: norm_curve.to_string(),
                                pkcs8: pkcs8.clone(),
                                pub_bytes,
                            },
                        );
                        let out = serde_json::json!({
                            "id": id,
                            "type": "private",
                            "crv": norm_curve,
                            "x": base64_encode(&x_bytes, true),
                            "y": base64_encode(&y_bytes, true),
                            "d": base64_encode(&d_bytes, true),
                            "pkcs8": base64_encode(&pkcs8, false),
                        });
                        return Ok(BoaVal::from(js_string!(out.to_string())));
                    }
                    let spki = ecdsa_pub_to_spki(norm_curve, &pub_bytes)
                        .map_err(|msg| JsNativeError::error().with_message(msg))?;
                    let id = insert_cached_crypto_key(
                        tenant,
                        CachedCryptoKeyMaterial::EcdsaPublic {
                            curve: norm_curve.to_string(),
                            pub_bytes: pub_bytes.clone(),
                        },
                    );
                    let out = serde_json::json!({
                        "id": id,
                        "type": "public",
                        "crv": norm_curve,
                        "x": base64_encode(&x_bytes, true),
                        "y": base64_encode(&y_bytes, true),
                        "pubRaw": base64_encode(&pub_bytes, false),
                        "spki": base64_encode(&spki, false),
                    });
                    return Ok(BoaVal::from(js_string!(out.to_string())));
                }
                let raw = extract_buffer_source_bytes(data_arg, context)?;
                if format.eq_ignore_ascii_case("pkcs8") {
                    let (d_bytes, pub_bytes) = extract_ecdsa_pkcs8_components(norm_curve, &raw)
                        .map_err(|msg| JsNativeError::error().with_message(msg))?;
                    let coord_len = (pub_bytes.len() - 1) / 2;
                    let x_b64u = base64_encode(&pub_bytes[1..1 + coord_len], true);
                    let y_b64u = base64_encode(&pub_bytes[1 + coord_len..], true);
                    let d_b64u = base64_encode(&d_bytes, true);
                    let pkcs8_b64 = base64_encode(&raw, false);
                    let id = insert_cached_crypto_key(
                        tenant,
                        CachedCryptoKeyMaterial::EcdsaPrivate {
                            curve: norm_curve.to_string(),
                            pkcs8: raw,
                            pub_bytes,
                        },
                    );
                    let out = serde_json::json!({
                        "id": id,
                        "type": "private",
                        "crv": norm_curve,
                        "x": x_b64u,
                        "y": y_b64u,
                        "d": d_b64u,
                        "pkcs8": pkcs8_b64,
                    });
                    return Ok(BoaVal::from(js_string!(out.to_string())));
                }
                let pub_bytes = extract_ecdsa_pub(norm_curve, &raw)
                    .map_err(|msg| JsNativeError::error().with_message(msg))?;
                let spki = ecdsa_pub_to_spki(norm_curve, &pub_bytes)
                    .map_err(|msg| JsNativeError::error().with_message(msg))?;
                let coord_len = (pub_bytes.len() - 1) / 2;
                let x_b64u = base64_encode(&pub_bytes[1..1 + coord_len], true);
                let y_b64u = base64_encode(&pub_bytes[1 + coord_len..], true);
                let pub_raw_b64 = base64_encode(&pub_bytes, false);
                let spki_b64 = base64_encode(&spki, false);
                let id = insert_cached_crypto_key(
                    tenant,
                    CachedCryptoKeyMaterial::EcdsaPublic {
                        curve: norm_curve.to_string(),
                        pub_bytes,
                    },
                );
                let out = serde_json::json!({
                    "id": id,
                    "type": "public",
                    "crv": norm_curve,
                    "x": x_b64u,
                    "y": y_b64u,
                    "pubRaw": pub_raw_b64,
                    "spki": spki_b64,
                });
                return Ok(BoaVal::from(js_string!(out.to_string())));
            }

            if alg.eq_ignore_ascii_case("X25519") {
                if format.eq_ignore_ascii_case("jwk") {
                    let jwk_str = data_arg.to_string(context)?.to_std_string_escaped();
                    let jwk: serde_json::Value = serde_json::from_str(&jwk_str).map_err(|_| {
                        JsNativeError::error().with_message("DataError: Invalid JWK JSON")
                    })?;
                    if jwk.get("kty").and_then(|v| v.as_str()) != Some("OKP")
                        || jwk.get("crv").and_then(|v| v.as_str()) != Some("X25519")
                    {
                        return Err(JsNativeError::error()
                            .with_message("DataError: JWK kty/crv must be OKP/X25519")
                            .into());
                    }
                    let x_str = jwk.get("x").and_then(|v| v.as_str()).unwrap_or("");
                    if let Some(d_str) = jwk.get("d").and_then(|v| v.as_str()) {
                        let d_raw = base64_decode(d_str);
                        if d_raw.len() != 32 {
                            return Err(JsNativeError::error()
                                .with_message("DataError: Invalid X25519 JWK private scalar length")
                                .into());
                        }
                        let mut seed = [0u8; 32];
                        seed.copy_from_slice(&d_raw);
                        let (seed_bytes, pub_bytes) = x25519_keypair_from_seed(seed);
                        if !x_str.is_empty() {
                            let x_raw = base64_decode(x_str);
                            if x_raw.len() != 32 || !constant_time_eq(&pub_bytes, &x_raw) {
                                return Err(JsNativeError::error()
                                    .with_message(
                                        "DataError: Mismatched X25519 JWK public/private components",
                                    )
                                    .into());
                            }
                        }
                        let out = build_x25519_private_json(tenant, seed_bytes, pub_bytes);
                        return Ok(BoaVal::from(js_string!(out.to_string())));
                    }
                    let x_raw = base64_decode(x_str);
                    if x_raw.len() != 32 {
                        return Err(JsNativeError::error()
                            .with_message("DataError: Invalid X25519 JWK public key length")
                            .into());
                    }
                    let mut pub_bytes = [0u8; 32];
                    pub_bytes.copy_from_slice(&x_raw);
                    let out = build_x25519_public_json(tenant, pub_bytes);
                    return Ok(BoaVal::from(js_string!(out.to_string())));
                }
                let raw = extract_buffer_source_bytes(data_arg, context)?;
                if format.eq_ignore_ascii_case("pkcs8") {
                    let seed = extract_x25519_seed(&raw)
                        .map_err(|msg| JsNativeError::error().with_message(msg))?;
                    let (seed_bytes, pub_bytes) = x25519_keypair_from_seed(seed);
                    let out = build_x25519_private_json(tenant, seed_bytes, pub_bytes);
                    return Ok(BoaVal::from(js_string!(out.to_string())));
                }
                if format.eq_ignore_ascii_case("raw") && raw.len() != 32 {
                    return Err(JsNativeError::error()
                        .with_message("DataError: X25519 raw public key must be 32 bytes")
                        .into());
                }
                let pub_bytes = extract_x25519_pub(&raw)
                    .map_err(|msg| JsNativeError::error().with_message(msg))?;
                let out = build_x25519_public_json(tenant, pub_bytes);
                return Ok(BoaVal::from(js_string!(out.to_string())));
            }

            if alg.eq_ignore_ascii_case("RSA")
                || alg.eq_ignore_ascii_case("RSASSA-PKCS1-v1_5")
                || alg.eq_ignore_ascii_case("RSA-PSS")
            {
                if format.eq_ignore_ascii_case("jwk") {
                    let jwk_str = data_arg.to_string(context)?.to_std_string_escaped();
                    let jwk: serde_json::Value = serde_json::from_str(&jwk_str).map_err(|_| {
                        JsNativeError::error().with_message("DataError: Invalid JWK JSON")
                    })?;
                    if jwk.get("kty").and_then(|v| v.as_str()) != Some("RSA") {
                        return Err(JsNativeError::error()
                            .with_message("DataError: JWK kty must be RSA")
                            .into());
                    }
                    let n_bytes =
                        base64_decode(jwk.get("n").and_then(|v| v.as_str()).unwrap_or(""));
                    let e_bytes =
                        base64_decode(jwk.get("e").and_then(|v| v.as_str()).unwrap_or(""));
                    if let Some(d_str) = jwk.get("d").and_then(|v| v.as_str()) {
                        let d_bytes = base64_decode(d_str);
                        let p_bytes =
                            base64_decode(jwk.get("p").and_then(|v| v.as_str()).unwrap_or(""));
                        let q_bytes =
                            base64_decode(jwk.get("q").and_then(|v| v.as_str()).unwrap_or(""));
                        let dp_bytes =
                            base64_decode(jwk.get("dp").and_then(|v| v.as_str()).unwrap_or(""));
                        let dq_bytes =
                            base64_decode(jwk.get("dq").and_then(|v| v.as_str()).unwrap_or(""));
                        let qi_bytes =
                            base64_decode(jwk.get("qi").and_then(|v| v.as_str()).unwrap_or(""));
                        let (pkcs8, pub_info) = rsa_private_components_to_pkcs8(
                            &n_bytes, &e_bytes, &d_bytes, &p_bytes, &q_bytes, &dp_bytes, &dq_bytes,
                            &qi_bytes,
                        )
                        .map_err(|msg| JsNativeError::error().with_message(msg))?;
                        let (_, _, priv_jwk) = inspect_rsa_private_key(&pkcs8)
                            .map_err(|msg| JsNativeError::error().with_message(msg))?;
                        let out = build_rsa_private_import_json(tenant, pkcs8, pub_info, priv_jwk);
                        return Ok(BoaVal::from(js_string!(out.to_string())));
                    }
                    let pub_info = rsa_public_components_to_info(&n_bytes, &e_bytes)
                        .map_err(|msg| JsNativeError::error().with_message(msg))?;
                    let out = build_rsa_public_import_json(tenant, pub_info);
                    return Ok(BoaVal::from(js_string!(out.to_string())));
                }
                if format.eq_ignore_ascii_case("raw") {
                    return Err(JsNativeError::error()
                        .with_message("NotSupportedError: RSA keys do not support 'raw' format")
                        .into());
                }
                let raw = extract_buffer_source_bytes(data_arg, context)?;
                if format.eq_ignore_ascii_case("pkcs8") {
                    let (pkcs8, pub_info, priv_jwk) = inspect_rsa_private_key(&raw)
                        .map_err(|msg| JsNativeError::error().with_message(msg))?;
                    let out = build_rsa_private_import_json(tenant, pkcs8, pub_info, priv_jwk);
                    return Ok(BoaVal::from(js_string!(out.to_string())));
                }
                let pub_info = inspect_rsa_public_key(&raw)
                    .map_err(|msg| JsNativeError::error().with_message(msg))?;
                let out = build_rsa_public_import_json(tenant, pub_info);
                return Ok(BoaVal::from(js_string!(out.to_string())));
            }

            Err(JsNativeError::error()
                .with_message(format!(
                    "NotSupportedError: Unsupported import algorithm '{}'",
                    alg
                ))
                .into())
        }),
    );
}
