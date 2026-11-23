//! WHATWG WebCrypto (`crypto.subtle`) and `node:crypto` Host Bindings (Digests, HMAC, AES-GCM/CBC,
//! Ed25519, X25519, ECDSA P-256/P-384, RSA PKCS#1 v1.5 / RSA-PSS, PBKDF2, HKDF, scrypt, CSPRNG).

mod asym_import;
mod asym_ops;
mod random_and_digest;
mod symmetric_and_kdf;

use crate::bindings::helpers::extract_buffer_source_bytes;
use crate::bindings::wintertc::register_buffer_host_functions;
use crate::boa_realm::{BOA_ACTIVE_TENANT, BOA_DET_STATE};
use crate::intrinsics::{
    CachedCryptoKeyMaterial, base64_encode, ecdsa_generate_keypair, ecdsa_pub_to_spki,
    ed25519_keypair_from_seed, ed25519_pub_to_spki, ed25519_seed_to_pkcs8_v1, fill_random_bytes,
    insert_cached_crypto_key, x25519_keypair_from_seed, x25519_pub_to_spki,
    x25519_seed_to_pkcs8_v1,
};

#[inline]
pub(super) fn fill_random_bytes_det(dest: &mut [u8]) {
    BOA_DET_STATE.with(|s| {
        if let Some((mut prng, clk)) = s.get() {
            fill_random_bytes(dest, Some(&mut prng));
            s.set(Some((prng, clk)));
        } else {
            fill_random_bytes(dest, None);
        }
    });
}

#[inline]
pub(super) fn extract_optional_bytes(
    arg: Option<&boa_engine::JsValue>,
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<Vec<u8>> {
    match arg {
        Some(v) if !v.is_undefined() && !v.is_null() => extract_buffer_source_bytes(v, context),
        _ => Ok(Vec::new()),
    }
}

pub(crate) fn register_node_crypto_bootstrap_host_function(ctx: &mut boa_engine::Context) {
    use boa_engine::{JsValue as BoaVal, NativeFunction, js_string};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_init_crypto_ext"), ctx)
        .unwrap_or(false)
    {
        return;
    }
    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_init_crypto_ext"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, context| {
            register_webcrypto_host_functions(context);
            Ok(BoaVal::undefined())
        }),
    );
}

fn register_asym_keygen_host_function(ctx: &mut boa_engine::Context) {
    use boa_engine::{JsNativeError, JsValue as BoaVal, NativeFunction, js_string};

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_asym_keygen"),
        2,
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
            let tenant = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());

            if alg.eq_ignore_ascii_case("Ed25519") {
                let mut seed = [0u8; 32];
                fill_random_bytes_det(&mut seed);
                let (seed_bytes, pub_bytes) = ed25519_keypair_from_seed(seed)
                    .map_err(|msg| JsNativeError::error().with_message(msg))?;
                let pkcs8 = ed25519_seed_to_pkcs8_v1(&seed_bytes);
                let spki = ed25519_pub_to_spki(&pub_bytes);
                let priv_id = insert_cached_crypto_key(
                    tenant.clone(),
                    CachedCryptoKeyMaterial::Ed25519Private {
                        seed: seed_bytes,
                        pub_bytes,
                    },
                );
                let pub_id = insert_cached_crypto_key(
                    tenant,
                    CachedCryptoKeyMaterial::Ed25519Public { pub_bytes },
                );
                let res = serde_json::json!({
                    "privId": priv_id,
                    "pubId": pub_id,
                    "d": base64_encode(&seed_bytes, true),
                    "x": base64_encode(&pub_bytes, true),
                    "pubRaw": base64_encode(&pub_bytes, false),
                    "pkcs8": base64_encode(&pkcs8, false),
                    "spki": base64_encode(&spki, false),
                });
                return Ok(BoaVal::from(js_string!(res.to_string())));
            }

            if alg.eq_ignore_ascii_case("X25519") {
                let mut seed = [0u8; 32];
                fill_random_bytes_det(&mut seed);
                let (seed_bytes, pub_bytes) = x25519_keypair_from_seed(seed);
                let pkcs8 = x25519_seed_to_pkcs8_v1(&seed_bytes);
                let spki = x25519_pub_to_spki(&pub_bytes);
                let priv_id = insert_cached_crypto_key(
                    tenant.clone(),
                    CachedCryptoKeyMaterial::X25519Private {
                        seed: seed_bytes,
                        pub_bytes,
                    },
                );
                let pub_id = insert_cached_crypto_key(
                    tenant,
                    CachedCryptoKeyMaterial::X25519Public { pub_bytes },
                );
                let res = serde_json::json!({
                    "privId": priv_id,
                    "pubId": pub_id,
                    "crv": "X25519",
                    "d": base64_encode(&seed_bytes, true),
                    "x": base64_encode(&pub_bytes, true),
                    "pubRaw": base64_encode(&pub_bytes, false),
                    "pkcs8": base64_encode(&pkcs8, false),
                    "spki": base64_encode(&spki, false),
                });
                return Ok(BoaVal::from(js_string!(res.to_string())));
            }

            if alg.eq_ignore_ascii_case("RSA")
                || alg.eq_ignore_ascii_case("RSASSA-PKCS1-v1_5")
                || alg.eq_ignore_ascii_case("RSA-PSS")
                || alg.eq_ignore_ascii_case("RSA-OAEP")
            {
                return Err(JsNativeError::error()
                    .with_message(
                        "NotSupportedError: Runtime RSA key generation is disabled in the edge sandbox; import an existing RSA key or generate Ed25519, X25519, or ECDSA P-256/P-384",
                    )
                    .into());
            }

            if alg.eq_ignore_ascii_case("ECDSA") || alg.eq_ignore_ascii_case("ec") {
                let norm_curve = if curve.eq_ignore_ascii_case("P-384")
                    || curve.eq_ignore_ascii_case("secp384r1")
                {
                    "P-384"
                } else if curve.eq_ignore_ascii_case("P-256")
                    || curve.eq_ignore_ascii_case("prime256v1")
                {
                    "P-256"
                } else {
                    return Err(JsNativeError::error()
                        .with_message(format!(
                            "NotSupportedError: Unsupported ECDSA namedCurve '{}'",
                            curve
                        ))
                        .into());
                };
                let (pkcs8, d_bytes, pub_bytes) = ecdsa_generate_keypair(norm_curve)
                    .map_err(|msg| JsNativeError::error().with_message(msg))?;
                let spki = ecdsa_pub_to_spki(norm_curve, &pub_bytes)
                    .map_err(|msg| JsNativeError::error().with_message(msg))?;
                let coord_len = (pub_bytes.len() - 1) / 2;
                let x_b64u = base64_encode(&pub_bytes[1..1 + coord_len], true);
                let y_b64u = base64_encode(&pub_bytes[1 + coord_len..], true);
                let d_b64u = base64_encode(&d_bytes, true);
                let pub_raw_b64 = base64_encode(&pub_bytes, false);
                let pkcs8_b64 = base64_encode(&pkcs8, false);
                let spki_b64 = base64_encode(&spki, false);
                let priv_id = insert_cached_crypto_key(
                    tenant.clone(),
                    CachedCryptoKeyMaterial::EcdsaPrivate {
                        curve: norm_curve.to_string(),
                        pkcs8,
                        pub_bytes: pub_bytes.clone(),
                    },
                );
                let pub_id = insert_cached_crypto_key(
                    tenant,
                    CachedCryptoKeyMaterial::EcdsaPublic {
                        curve: norm_curve.to_string(),
                        pub_bytes,
                    },
                );
                let res = serde_json::json!({
                    "privId": priv_id,
                    "pubId": pub_id,
                    "crv": norm_curve,
                    "x": x_b64u,
                    "y": y_b64u,
                    "d": d_b64u,
                    "pubRaw": pub_raw_b64,
                    "pkcs8": pkcs8_b64,
                    "spki": spki_b64,
                });
                return Ok(BoaVal::from(js_string!(res.to_string())));
            }

            Err(JsNativeError::error()
                .with_message(format!(
                    "NotSupportedError: Unsupported keygen algorithm '{}'",
                    alg
                ))
                .into())
        }),
    );
}

pub(crate) fn register_webcrypto_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::js_string;
    register_buffer_host_functions(ctx);
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_crypto_random_uuid"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    random_and_digest::register_random_and_digest_host_functions(ctx);
    symmetric_and_kdf::register_symmetric_and_kdf_host_functions(ctx);
    register_asym_keygen_host_function(ctx);
    asym_import::register_asym_import_host_function(ctx);
    asym_ops::register_asym_ops_host_functions(ctx);
}
