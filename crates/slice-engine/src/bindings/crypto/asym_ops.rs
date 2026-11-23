//! Asymmetric Sign, Verify (`Ed25519`, `ECDSA`, `RSA`), and `X25519` Diffie-Hellman Host Bindings.

use super::extract_optional_bytes;
use crate::bindings::helpers::{
    bytes_to_js_array_buffer, consume_crypto_kdf_fuel, trip_isolate_oom_limits, with_active_tenant,
    with_optional_buffer_source_bytes, with_two_buffer_source_bytes,
};
use crate::intrinsics::{
    CachedCryptoKeyMaterial, ecdsa_sign_bytes, ecdsa_verify_bytes, ed25519_sign_bytes,
    ed25519_verify_bytes, extract_ecdsa_pub, extract_ed25519_pub, extract_ed25519_seed,
    extract_x25519_pub, extract_x25519_seed, rsa_sign_bytes_with_alg, rsa_verify_bytes_with_alg,
    with_cached_crypto_key, x25519_diffie_hellman,
};

pub(super) fn register_asym_ops_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{JsNativeError, JsValue as BoaVal, NativeFunction, js_string};

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_asym_sign"),
        6,
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
            let handle_id = args
                .get(2)
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let asn1_der = args.get(5).and_then(|v| v.as_boolean()).unwrap_or(false);

            if alg.eq_ignore_ascii_case("Ed25519") {
                let sig_res = if handle_id != 0 {
                    with_optional_buffer_source_bytes(args.get(4), context, |msg_bytes| {
                        with_active_tenant(|tenant| {
                            with_cached_crypto_key(handle_id, tenant, |mat| match mat {
                                CachedCryptoKeyMaterial::Ed25519Private { seed, .. } => {
                                    Some(ed25519_sign_bytes(seed, msg_bytes))
                                }
                                _ => None,
                            })
                            .flatten()
                        })
                    })?
                } else {
                    None
                };
                let sig_arr = match sig_res {
                    Some(r) => r,
                    None => {
                        let key_fallback = extract_optional_bytes(args.get(3), context)?;
                        with_optional_buffer_source_bytes(args.get(4), context, |msg_bytes| {
                            extract_ed25519_seed(&key_fallback)
                                .and_then(|seed| ed25519_sign_bytes(&seed, msg_bytes))
                        })?
                    }
                }
                .map_err(|msg| JsNativeError::error().with_message(msg))?;
                return bytes_to_js_array_buffer(&sig_arr, context);
            }

            if alg.eq_ignore_ascii_case("ECDSA") || alg.eq_ignore_ascii_case("ec") {
                let sig_res = if handle_id != 0 {
                    with_optional_buffer_source_bytes(args.get(4), context, |msg_bytes| {
                        with_active_tenant(|tenant| {
                            with_cached_crypto_key(handle_id, tenant, |mat| match mat {
                                CachedCryptoKeyMaterial::EcdsaPrivate {
                                    curve: k_curve,
                                    pkcs8,
                                    ..
                                } => Some(ecdsa_sign_bytes(k_curve, pkcs8, msg_bytes, asn1_der)),
                                _ => None,
                            })
                            .flatten()
                        })
                    })?
                } else {
                    None
                };
                let sig = match sig_res {
                    Some(r) => r,
                    None => {
                        let key_fallback = extract_optional_bytes(args.get(3), context)?;
                        with_optional_buffer_source_bytes(args.get(4), context, |msg_bytes| {
                            ecdsa_sign_bytes(&curve, &key_fallback, msg_bytes, asn1_der)
                        })?
                    }
                }
                .map_err(|msg| JsNativeError::error().with_message(msg))?;
                return bytes_to_js_array_buffer(&sig, context);
            }

            if alg.eq_ignore_ascii_case("RSA")
                || alg.eq_ignore_ascii_case("RSASSA-PKCS1-v1_5")
                || alg.eq_ignore_ascii_case("RSA-PSS")
            {
                if let Err(msg) = consume_crypto_kdf_fuel(40) {
                    trip_isolate_oom_limits(context);
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                let use_pss = alg.eq_ignore_ascii_case("RSA-PSS");
                let sig_res = if handle_id != 0 {
                    with_optional_buffer_source_bytes(args.get(4), context, |msg_bytes| {
                        with_active_tenant(|tenant| {
                            with_cached_crypto_key(handle_id, tenant, |mat| match mat {
                                CachedCryptoKeyMaterial::RsaPrivate { pkcs8, .. } => {
                                    Some(rsa_sign_bytes_with_alg(pkcs8, msg_bytes, use_pss, &curve))
                                }
                                _ => None,
                            })
                            .flatten()
                        })
                    })?
                } else {
                    None
                };
                let sig = match sig_res {
                    Some(r) => r,
                    None => {
                        let key_fallback = extract_optional_bytes(args.get(3), context)?;
                        with_optional_buffer_source_bytes(args.get(4), context, |msg_bytes| {
                            rsa_sign_bytes_with_alg(&key_fallback, msg_bytes, use_pss, &curve)
                        })?
                    }
                }
                .map_err(|msg| JsNativeError::error().with_message(msg))?;
                return bytes_to_js_array_buffer(&sig, context);
            }

            Err(JsNativeError::error()
                .with_message(format!(
                    "NotSupportedError: Unsupported sign algorithm '{}'",
                    alg
                ))
                .into())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_asym_verify"),
        6,
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
            let handle_id = args
                .get(2)
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let (Some(sig_arg), Some(msg_arg)) = (args.get(4), args.get(5)) else {
                return Ok(BoaVal::from(false));
            };

            if alg.eq_ignore_ascii_case("Ed25519") {
                let verified = if handle_id != 0 {
                    with_two_buffer_source_bytes(
                        sig_arg,
                        msg_arg,
                        context,
                        |sig_bytes, msg_bytes| {
                            with_active_tenant(|tenant| {
                                with_cached_crypto_key(handle_id, tenant, |mat| match mat {
                                    CachedCryptoKeyMaterial::Ed25519Public { pub_bytes }
                                    | CachedCryptoKeyMaterial::Ed25519Private {
                                        pub_bytes, ..
                                    } => {
                                        Some(ed25519_verify_bytes(pub_bytes, msg_bytes, sig_bytes))
                                    }
                                    _ => None,
                                })
                                .flatten()
                            })
                        },
                    )?
                } else {
                    None
                };
                let ok = match verified {
                    Some(v) => v,
                    None => {
                        let pub_fallback = extract_optional_bytes(args.get(3), context)?;
                        match extract_ed25519_pub(&pub_fallback) {
                            Ok(pk) => with_two_buffer_source_bytes(
                                sig_arg,
                                msg_arg,
                                context,
                                |sig_bytes, msg_bytes| {
                                    ed25519_verify_bytes(&pk, msg_bytes, sig_bytes)
                                },
                            )?,
                            Err(_) => false,
                        }
                    }
                };
                return Ok(BoaVal::from(ok));
            }

            if alg.eq_ignore_ascii_case("ECDSA") || alg.eq_ignore_ascii_case("ec") {
                let verified = if handle_id != 0 {
                    with_two_buffer_source_bytes(
                        sig_arg,
                        msg_arg,
                        context,
                        |sig_bytes, msg_bytes| {
                            with_active_tenant(|tenant| {
                                with_cached_crypto_key(handle_id, tenant, |mat| match mat {
                                    CachedCryptoKeyMaterial::EcdsaPublic {
                                        curve: k_curve,
                                        pub_bytes,
                                    }
                                    | CachedCryptoKeyMaterial::EcdsaPrivate {
                                        curve: k_curve,
                                        pub_bytes,
                                        ..
                                    } => Some(ecdsa_verify_bytes(
                                        k_curve, pub_bytes, msg_bytes, sig_bytes,
                                    )),
                                    _ => None,
                                })
                                .flatten()
                            })
                        },
                    )?
                } else {
                    None
                };
                let ok = match verified {
                    Some(v) => v,
                    None => {
                        let pub_fallback = extract_optional_bytes(args.get(3), context)?;
                        match extract_ecdsa_pub(&curve, &pub_fallback) {
                            Ok(pk) => with_two_buffer_source_bytes(
                                sig_arg,
                                msg_arg,
                                context,
                                |sig_bytes, msg_bytes| {
                                    ecdsa_verify_bytes(&curve, &pk, msg_bytes, sig_bytes)
                                },
                            )?,
                            Err(_) => false,
                        }
                    }
                };
                return Ok(BoaVal::from(ok));
            }

            if alg.eq_ignore_ascii_case("RSA")
                || alg.eq_ignore_ascii_case("RSASSA-PKCS1-v1_5")
                || alg.eq_ignore_ascii_case("RSA-PSS")
            {
                if let Err(msg) = consume_crypto_kdf_fuel(15) {
                    trip_isolate_oom_limits(context);
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                let use_pss = alg.eq_ignore_ascii_case("RSA-PSS");
                let verified = if handle_id != 0 {
                    with_two_buffer_source_bytes(
                        sig_arg,
                        msg_arg,
                        context,
                        |sig_bytes, msg_bytes| {
                            with_active_tenant(|tenant| {
                                with_cached_crypto_key(handle_id, tenant, |mat| match mat {
                                    CachedCryptoKeyMaterial::RsaPublic { spki }
                                    | CachedCryptoKeyMaterial::RsaPrivate { spki, .. } => {
                                        Some(rsa_verify_bytes_with_alg(
                                            spki, msg_bytes, sig_bytes, use_pss, &curve,
                                        ))
                                    }
                                    _ => None,
                                })
                                .flatten()
                            })
                        },
                    )?
                } else {
                    None
                };
                let ok = match verified {
                    Some(v) => v,
                    None => {
                        let pub_fallback = extract_optional_bytes(args.get(3), context)?;
                        with_two_buffer_source_bytes(
                            sig_arg,
                            msg_arg,
                            context,
                            |sig_bytes, msg_bytes| {
                                rsa_verify_bytes_with_alg(
                                    &pub_fallback,
                                    msg_bytes,
                                    sig_bytes,
                                    use_pss,
                                    &curve,
                                )
                            },
                        )?
                    }
                };
                return Ok(BoaVal::from(ok));
            }

            Ok(BoaVal::from(false))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_ecdh"),
        5,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let alg = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !alg.eq_ignore_ascii_case("X25519") {
                return Err(JsNativeError::error()
                    .with_message(format!(
                        "NotSupportedError: Unsupported key agreement algorithm '{}'",
                        alg
                    ))
                    .into());
            }
            let priv_id = args
                .get(1)
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let pub_id = args
                .get(3)
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);

            let cached_seed = if priv_id != 0 {
                with_active_tenant(|tenant| {
                    with_cached_crypto_key(priv_id, tenant, |mat| match mat {
                        CachedCryptoKeyMaterial::X25519Private { seed, .. } => Some(*seed),
                        _ => None,
                    })
                    .flatten()
                })
            } else {
                None
            };
            let priv_seed = match cached_seed {
                Some(s) => s,
                None => {
                    let priv_fallback = extract_optional_bytes(args.get(2), context)?;
                    extract_x25519_seed(&priv_fallback)
                        .map_err(|msg| JsNativeError::error().with_message(msg))?
                }
            };

            let cached_pub = if pub_id != 0 {
                with_active_tenant(|tenant| {
                    with_cached_crypto_key(pub_id, tenant, |mat| match mat {
                        CachedCryptoKeyMaterial::X25519Public { pub_bytes }
                        | CachedCryptoKeyMaterial::X25519Private { pub_bytes, .. } => {
                            Some(*pub_bytes)
                        }
                        _ => None,
                    })
                    .flatten()
                })
            } else {
                None
            };
            let peer_pub = match cached_pub {
                Some(p) => p,
                None => {
                    let pub_fallback = extract_optional_bytes(args.get(4), context)?;
                    extract_x25519_pub(&pub_fallback)
                        .map_err(|msg| JsNativeError::error().with_message(msg))?
                }
            };

            let shared = x25519_diffie_hellman(&priv_seed, &peer_pub)
                .map_err(|msg| JsNativeError::error().with_message(msg))?;
            bytes_to_js_array_buffer(&shared, context)
        }),
    );
}
