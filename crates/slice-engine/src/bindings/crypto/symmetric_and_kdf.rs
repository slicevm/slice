//! Symmetric Ciphers (`AES-GCM`, `AES-CBC`), Key Handle Registration, and KDFs (`PBKDF2`, `HKDF`, `scrypt`).

use super::extract_optional_bytes;
use crate::bindings::helpers::{
    bytes_to_js_array_buffer, consume_crypto_kdf_fuel, extract_buffer_source_bytes,
    normalize_webcrypto_hash_name, trip_isolate_oom_limits, try_consume_isolate_memory_ctx,
    with_active_tenant, with_optional_buffer_source_bytes,
};
use crate::boa_realm::BOA_ACTIVE_TENANT;
use crate::intrinsics::{
    AesExpandedKey, CachedCryptoKeyMaterial, aes_cbc_decrypt, aes_cbc_decrypt_with_expanded,
    aes_cbc_encrypt, aes_cbc_encrypt_with_expanded, aes_gcm_decrypt, aes_gcm_encrypt,
    aes_gcm_open_with_key, aes_gcm_seal_with_key, hkdf_derive_bytes, insert_cached_crypto_key,
    make_ring_aead_key, pbkdf2_derive_bytes, scrypt_derive_bytes, with_cached_crypto_key,
};

pub(super) fn register_symmetric_and_kdf_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{JsNativeError, JsValue as BoaVal, NativeFunction, js_string};

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_key_register"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let kind = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let sub = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let Some(raw_arg) = args.get(2) else {
                return Ok(BoaVal::from(0.0));
            };
            let raw = extract_buffer_source_bytes(raw_arg, context)?;
            let tenant = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
            let material = if kind.eq_ignore_ascii_case("AES-GCM") {
                let less_safe = make_ring_aead_key(&raw)
                    .map_err(|msg| JsNativeError::error().with_message(msg))?;
                CachedCryptoKeyMaterial::AesGcm {
                    less_safe: Box::new(less_safe),
                    raw,
                }
            } else if kind.eq_ignore_ascii_case("AES-CBC") {
                let expanded = AesExpandedKey::new(&raw)
                    .map_err(|msg| JsNativeError::error().with_message(msg))?;
                CachedCryptoKeyMaterial::AesCbc {
                    expanded: Box::new(expanded),
                }
            } else if kind.eq_ignore_ascii_case("HMAC") {
                let norm_hash = normalize_webcrypto_hash_name(&sub).unwrap_or("SHA-256");
                let ring_alg = match norm_hash {
                    "SHA-1" => ring::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
                    "SHA-384" => ring::hmac::HMAC_SHA384,
                    "SHA-512" => ring::hmac::HMAC_SHA512,
                    _ => ring::hmac::HMAC_SHA256,
                };
                let ring_key = ring::hmac::Key::new(ring_alg, &raw);
                CachedCryptoKeyMaterial::Hmac {
                    ring_key: Box::new(ring_key),
                }
            } else {
                return Ok(BoaVal::from(0.0));
            };
            let handle_id = insert_cached_crypto_key(tenant, material);
            Ok(BoaVal::from(handle_id as f64))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_cipher"),
        8,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let alg = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let is_encrypt = args.get(1).and_then(|v| v.as_boolean()).unwrap_or(true);
            let handle_id = args
                .get(2)
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let iv_bytes = extract_optional_bytes(args.get(4), context)?;
            let aad_bytes = extract_optional_bytes(args.get(5), context)?;
            let tag_bits = args
                .get(6)
                .and_then(|v| v.as_number())
                .map(|n| n as usize)
                .unwrap_or(128);
            let auto_pad = args.get(8).and_then(|v| v.as_boolean()).unwrap_or(true);

            if alg.eq_ignore_ascii_case("AES-GCM")
                || alg.eq_ignore_ascii_case("aes-128-gcm")
                || alg.eq_ignore_ascii_case("aes-256-gcm")
            {
                let cached_res = if handle_id != 0 {
                    with_optional_buffer_source_bytes(args.get(7), context, |data_bytes| {
                        with_active_tenant(|tenant| {
                            with_cached_crypto_key(handle_id, tenant, |mat| match mat {
                                CachedCryptoKeyMaterial::AesGcm { less_safe, raw } => {
                                    if is_encrypt {
                                        Some(aes_gcm_seal_with_key(
                                            less_safe, &iv_bytes, &aad_bytes, data_bytes, tag_bits,
                                        ))
                                    } else {
                                        Some(aes_gcm_open_with_key(
                                            less_safe, raw, &iv_bytes, &aad_bytes, data_bytes,
                                            tag_bits,
                                        ))
                                    }
                                }
                                _ => None,
                            })
                            .flatten()
                        })
                    })?
                } else {
                    None
                };
                let out = match cached_res {
                    Some(r) => r,
                    None => {
                        let key_bytes = extract_optional_bytes(args.get(3), context)?;
                        with_optional_buffer_source_bytes(args.get(7), context, |data_bytes| {
                            if is_encrypt {
                                aes_gcm_encrypt(
                                    &key_bytes, &iv_bytes, &aad_bytes, data_bytes, tag_bits,
                                )
                            } else {
                                aes_gcm_decrypt(
                                    &key_bytes, &iv_bytes, &aad_bytes, data_bytes, tag_bits,
                                )
                            }
                        })?
                    }
                }
                .map_err(|msg| {
                    JsNativeError::error().with_message(format!("OperationError: {}", msg))
                })?;
                return bytes_to_js_array_buffer(&out, context);
            }

            if alg.eq_ignore_ascii_case("AES-CBC")
                || alg.eq_ignore_ascii_case("aes-128-cbc")
                || alg.eq_ignore_ascii_case("aes-256-cbc")
            {
                let cached_res = if handle_id != 0 {
                    with_optional_buffer_source_bytes(args.get(7), context, |data_bytes| {
                        with_active_tenant(|tenant| {
                            with_cached_crypto_key(handle_id, tenant, |mat| match mat {
                                CachedCryptoKeyMaterial::AesCbc { expanded } => {
                                    if is_encrypt {
                                        Some(aes_cbc_encrypt_with_expanded(
                                            expanded, &iv_bytes, data_bytes, auto_pad,
                                        ))
                                    } else {
                                        Some(aes_cbc_decrypt_with_expanded(
                                            expanded, &iv_bytes, data_bytes, auto_pad,
                                        ))
                                    }
                                }
                                _ => None,
                            })
                            .flatten()
                        })
                    })?
                } else {
                    None
                };
                let out = match cached_res {
                    Some(r) => r,
                    None => {
                        let key_bytes = extract_optional_bytes(args.get(3), context)?;
                        with_optional_buffer_source_bytes(args.get(7), context, |data_bytes| {
                            if is_encrypt {
                                aes_cbc_encrypt(&key_bytes, &iv_bytes, data_bytes, auto_pad)
                            } else {
                                aes_cbc_decrypt(&key_bytes, &iv_bytes, data_bytes, auto_pad)
                            }
                        })?
                    }
                }
                .map_err(|msg| {
                    JsNativeError::error().with_message(format!("OperationError: {}", msg))
                })?;
                return bytes_to_js_array_buffer(&out, context);
            }

            Err(JsNativeError::error()
                .with_message(format!("NotSupportedError: Unsupported cipher '{}'", alg))
                .into())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_kdf"),
        8,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let alg = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let hash = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "SHA-256".to_string());
            let secret = extract_optional_bytes(args.get(2), context)?;
            let salt = extract_optional_bytes(args.get(3), context)?;
            let info = extract_optional_bytes(args.get(4), context)?;
            let raw_iters_or_n = args.get(5).and_then(|v| v.as_number()).unwrap_or(1.0);
            if !raw_iters_or_n.is_finite() || raw_iters_or_n < 0.0 || raw_iters_or_n.fract() != 0.0
            {
                return Err(JsNativeError::range()
                    .with_message("OperationError: KDF iterations/cost must be a non-negative integer")
                    .into());
            }
            let iterations_or_n = if raw_iters_or_n > (usize::MAX as f64) {
                return Err(JsNativeError::range()
                    .with_message("OperationError: KDF iterations/cost exceeds supported range")
                    .into());
            } else {
                raw_iters_or_n as usize
            };
            let raw_byte_len = args.get(6).and_then(|v| v.as_number()).unwrap_or(32.0);
            if !raw_byte_len.is_finite() || raw_byte_len < 0.0 || raw_byte_len.fract() != 0.0 {
                return Err(JsNativeError::range()
                    .with_message("OperationError: Derived key length must be a non-negative integer")
                    .into());
            }
            if raw_byte_len > 1_048_576.0 {
                return Err(JsNativeError::range()
                    .with_message("Derived key length exceeds 1MB sandbox limit")
                    .into());
            }
            let byte_len = raw_byte_len as usize;

            if alg.eq_ignore_ascii_case("PBKDF2") {
                if iterations_or_n == 0 {
                    return Err(JsNativeError::error()
                        .with_message("OperationError: PBKDF2 iterations must be greater than 0")
                        .into());
                }
                let Ok(iters_u32) = u32::try_from(iterations_or_n) else {
                    return Err(JsNativeError::range()
                        .with_message("OperationError: PBKDF2 iterations exceed unsigned 32-bit range")
                        .into());
                };
                let blocks = byte_len.div_ceil(32).max(1) as u64;
                let fuel_cost = ((iters_u32 as u64).saturating_mul(blocks) / 100).max(10);
                if let Err(msg) = consume_crypto_kdf_fuel(fuel_cost) {
                    trip_isolate_oom_limits(context);
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                if iters_u32 > 10_000_000 {
                    return Err(JsNativeError::range()
                        .with_message("OperationError: PBKDF2 iterations exceed maximum sandbox limit (10,000,000)")
                        .into());
                }
                let derived = pbkdf2_derive_bytes(&hash, &secret, &salt, iters_u32, byte_len)
                    .map_err(|msg| {
                        if msg.contains("sandbox memory limit") || msg.contains("maximum") {
                            JsNativeError::range().with_message(msg)
                        } else {
                            JsNativeError::error().with_message(msg)
                        }
                    })?;
                if let Err(msg) = try_consume_isolate_memory_ctx(derived.len(), context) {
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                return bytes_to_js_array_buffer(&derived, context);
            }

            if alg.eq_ignore_ascii_case("HKDF") {
                if let Err(msg) = consume_crypto_kdf_fuel(15) {
                    trip_isolate_oom_limits(context);
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                let derived =
                    hkdf_derive_bytes(&hash, &secret, &salt, &info, byte_len).map_err(|msg| {
                        if msg.contains("sandbox memory limit")
                            || msg.contains("output length exceeds maximum")
                        {
                            JsNativeError::range().with_message(format!("OperationError: {}", msg))
                        } else {
                            JsNativeError::error().with_message(msg)
                        }
                    })?;
                if let Err(msg) = try_consume_isolate_memory_ctx(derived.len(), context) {
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                return bytes_to_js_array_buffer(&derived, context);
            }

            if alg.eq_ignore_ascii_case("SCRYPT") {
                let r_val = args
                    .get(7)
                    .and_then(|v| v.as_number())
                    .map(|n| n.max(1.0) as usize)
                    .unwrap_or(8);
                let p_val = args
                    .get(8)
                    .and_then(|v| v.as_number())
                    .map(|n| n.max(1.0) as usize)
                    .unwrap_or(1);
                let maxmem = args
                    .get(9)
                    .and_then(|v| v.as_number())
                    .map(|n| n.max(0.0) as usize)
                    .unwrap_or(32 * 1024 * 1024);
                let fuel_cost = (iterations_or_n as u64)
                    .saturating_mul(r_val as u64)
                    .saturating_mul(p_val as u64)
                    / 16;
                if let Err(msg) = consume_crypto_kdf_fuel(fuel_cost.max(50)) {
                    trip_isolate_oom_limits(context);
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                let derived = scrypt_derive_bytes(
                    &secret,
                    &salt,
                    iterations_or_n,
                    r_val,
                    p_val,
                    byte_len,
                    maxmem,
                )
                .map_err(|msg| {
                    if msg.contains("sandbox memory limit") {
                        JsNativeError::range().with_message(msg)
                    } else {
                        JsNativeError::error().with_message(msg)
                    }
                })?;
                if let Err(msg) = try_consume_isolate_memory_ctx(derived.len(), context) {
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                return bytes_to_js_array_buffer(&derived, context);
            }

            Err(JsNativeError::error()
                .with_message(format!("NotSupportedError: Unsupported KDF '{}'", alg))
                .into())
        }),
    );
}
