//! CSPRNG (`randomUUID`, `getRandomValues`, `randomBytes`, `randomInt`), Digests, HMAC, and `timingSafeEqual`.

use super::fill_random_bytes_det;
use crate::bindings::helpers::{
    bytes_to_js_array_buffer, bytes_to_js_uint8_array, compute_webcrypto_digest_stack,
    compute_webcrypto_hmac_stack, extract_buffer_source_bytes, normalize_webcrypto_hash_name,
    try_consume_isolate_memory_ctx, with_active_tenant, with_buffer_source_bytes,
    with_mutable_buffer_view_bytes, with_two_buffer_source_bytes,
};
use crate::boa_realm::BOA_DET_STATE;
use crate::intrinsics::{
    CachedCryptoKeyMaterial, MAX_CRYPTO_RANDOM_INT_RANGE, constant_time_eq, generate_uuid_v4,
    sample_unbiased_u64_below, with_cached_crypto_key,
};

pub(super) fn register_random_and_digest_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::builtins::typed_array::{TypedArray, TypedArrayKind};
    use boa_engine::{JsNativeError, JsValue as BoaVal, NativeFunction, js_string};

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_random_uuid"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let uuid = BOA_DET_STATE.with(|s| {
                if let Some((mut prng, clk)) = s.get() {
                    let u = generate_uuid_v4(Some(&mut prng));
                    s.set(Some((prng, clk)));
                    u
                } else {
                    generate_uuid_v4(None)
                }
            });
            Ok(BoaVal::from(js_string!(uuid)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_get_random_values"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(val) = args.first() else {
                return Err(JsNativeError::typ()
                    .with_message("crypto.getRandomValues requires an ArrayBufferView")
                    .into());
            };
            let Some(obj) = val.as_object() else {
                return Err(JsNativeError::typ()
                    .with_message("crypto.getRandomValues requires an ArrayBufferView")
                    .into());
            };

            if let Some(ta) = obj.downcast_ref::<TypedArray>() {
                if matches!(
                    ta.kind(),
                    TypedArrayKind::Float32 | TypedArrayKind::Float64
                ) {
                    return Err(JsNativeError::typ()
                        .with_message(
                            "TypeMismatchError: crypto.getRandomValues requires an integer TypedArray",
                        )
                        .into());
                }
            } else {
                let ctor_name = obj
                    .get(js_string!("constructor"), context)
                    .ok()
                    .and_then(|c| c.as_object())
                    .and_then(|co| co.get(js_string!("name"), context).ok())
                    .and_then(|n| n.as_string())
                    .map(|s| s.to_std_string_escaped())
                    .unwrap_or_default();
                if !matches!(
                    ctor_name.as_str(),
                    "Int8Array"
                        | "Uint8Array"
                        | "Uint8ClampedArray"
                        | "Int16Array"
                        | "Uint16Array"
                        | "Int32Array"
                        | "Uint32Array"
                        | "BigInt64Array"
                        | "BigUint64Array"
                        | "Buffer"
                ) {
                    return Err(JsNativeError::typ()
                        .with_message(
                            "TypeMismatchError: crypto.getRandomValues requires an integer TypedArray",
                        )
                        .into());
                }
            }

            let Some(fill_res) = with_mutable_buffer_view_bytes(
                &obj,
                false,
                context,
                |slice| -> Result<(), &'static str> {
                    if slice.len() > 65_536 {
                        return Err(
                            "QuotaExceededError: crypto.getRandomValues byteLength exceeds 65536 bytes",
                        );
                    }
                    fill_random_bytes_det(slice);
                    Ok(())
                },
            )?
            else {
                return Err(JsNativeError::typ()
                    .with_message("crypto.getRandomValues requires a TypedArray")
                    .into());
            };
            fill_res.map_err(|msg| JsNativeError::error().with_message(msg))?;
            Ok(val.clone())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_digest"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let algo = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let Some(data_arg) = args.get(1) else {
                return Err(JsNativeError::typ()
                    .with_message("crypto.subtle.digest requires data argument")
                    .into());
            };
            let Some(norm_alg) = normalize_webcrypto_hash_name(&algo) else {
                return Err(JsNativeError::error()
                    .with_message(format!(
                        "NotSupportedError: Unsupported digest algorithm '{}'",
                        algo
                    ))
                    .into());
            };
            if norm_alg == "MD5" {
                return Err(JsNativeError::error()
                    .with_message(format!(
                        "NotSupportedError: Unsupported digest algorithm '{}'",
                        algo
                    ))
                    .into());
            }
            let (digest_buf, digest_len) = with_buffer_source_bytes(data_arg, context, |bytes| {
                compute_webcrypto_digest_stack(norm_alg, bytes).unwrap_or(([0u8; 64], 0))
            })?;
            bytes_to_js_array_buffer(&digest_buf[..digest_len], context)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_hmac_sign"),
        4,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let hash_name = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "SHA-256".to_string());
            let Some(key_arg) = args.get(1) else {
                return Err(JsNativeError::typ()
                    .with_message("HMAC sign requires key bytes")
                    .into());
            };
            let Some(data_arg) = args.get(2) else {
                return Err(JsNativeError::typ()
                    .with_message("HMAC sign requires data bytes")
                    .into());
            };
            let handle_id = args
                .get(3)
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            if handle_id != 0 {
                let cached_sig = with_buffer_source_bytes(data_arg, context, |data_bytes| {
                    with_active_tenant(|tenant| {
                        with_cached_crypto_key(handle_id, tenant, |mat| match mat {
                            CachedCryptoKeyMaterial::Hmac { ring_key } => {
                                let tag = ring::hmac::sign(ring_key, data_bytes);
                                let slice = tag.as_ref();
                                let mut out = [0u8; 64];
                                let len = slice.len().min(64);
                                out[..len].copy_from_slice(&slice[..len]);
                                Some((out, len))
                            }
                            _ => None,
                        })
                        .flatten()
                    })
                })?;
                if let Some((sig_buf, sig_len)) = cached_sig {
                    return bytes_to_js_array_buffer(&sig_buf[..sig_len], context);
                }
            }
            let mac_opt = with_two_buffer_source_bytes(
                key_arg,
                data_arg,
                context,
                |key_bytes, data_bytes| {
                    compute_webcrypto_hmac_stack(&hash_name, key_bytes, data_bytes)
                },
            )?;
            let Some((mac_buf, mac_len)) = mac_opt else {
                return Err(JsNativeError::error()
                    .with_message(format!(
                        "NotSupportedError: Unsupported HMAC hash '{}'",
                        hash_name
                    ))
                    .into());
            };
            bytes_to_js_array_buffer(&mac_buf[..mac_len], context)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_hmac_verify"),
        5,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let hash_name = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "SHA-256".to_string());
            let Some(key_arg) = args.get(1) else {
                return Ok(BoaVal::from(false));
            };
            let Some(sig_arg) = args.get(2) else {
                return Ok(BoaVal::from(false));
            };
            let Some(data_arg) = args.get(3) else {
                return Ok(BoaVal::from(false));
            };
            let handle_id = args
                .get(4)
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            if handle_id != 0 {
                let cached_ok = with_two_buffer_source_bytes(
                    sig_arg,
                    data_arg,
                    context,
                    |sig_bytes, data_bytes| {
                        with_active_tenant(|tenant| {
                            with_cached_crypto_key(handle_id, tenant, |mat| match mat {
                                CachedCryptoKeyMaterial::Hmac { ring_key } => Some(
                                    ring::hmac::verify(ring_key, data_bytes, sig_bytes).is_ok(),
                                ),
                                _ => None,
                            })
                            .flatten()
                        })
                    },
                )?;
                if let Some(ok) = cached_ok {
                    return Ok(BoaVal::from(ok));
                }
            }
            let key_bytes = extract_buffer_source_bytes(key_arg, context)?;
            let verified_opt = with_two_buffer_source_bytes(
                sig_arg,
                data_arg,
                context,
                |sig_bytes, data_bytes| {
                    compute_webcrypto_hmac_stack(&hash_name, &key_bytes, data_bytes)
                        .map(|(expected, len)| constant_time_eq(&expected[..len], sig_bytes))
                },
            )?;
            let Some(ok) = verified_opt else {
                return Err(JsNativeError::error()
                    .with_message(format!(
                        "NotSupportedError: Unsupported HMAC hash '{}'",
                        hash_name
                    ))
                    .into());
            };
            Ok(BoaVal::from(ok))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_timing_safe_equal"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(a_arg) = args.first() else {
                return Err(JsNativeError::typ()
                    .with_message("timingSafeEqual requires 2 arguments")
                    .into());
            };
            let Some(b_arg) = args.get(1) else {
                return Err(JsNativeError::typ()
                    .with_message("timingSafeEqual requires 2 arguments")
                    .into());
            };
            let cmp_res =
                with_two_buffer_source_bytes(a_arg, b_arg, context, |a_bytes, b_bytes| {
                    (a_bytes.len() == b_bytes.len()).then(|| constant_time_eq(a_bytes, b_bytes))
                })?;
            match cmp_res {
                Some(eq) => Ok(BoaVal::from(eq)),
                None => Err(JsNativeError::range()
                    .with_message("Input buffers must have the same byte length")
                    .into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_random_bytes"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let size = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as usize)
                .unwrap_or(0);
            if size > 1_048_576 {
                return Err(JsNativeError::range()
                    .with_message("crypto.randomBytes size exceeds 1MB sandbox limit")
                    .into());
            }
            if let Err(msg) = try_consume_isolate_memory_ctx(size, context) {
                return Err(JsNativeError::range().with_message(msg).into());
            }
            let mut buf = vec![0u8; size];
            fill_random_bytes_det(&mut buf);
            bytes_to_js_uint8_array(&buf, context)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_random_int"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let Some(range_f) = args.first().and_then(|v| v.as_number()) else {
                return Err(JsNativeError::typ()
                    .with_message("ERR_INVALID_ARG_TYPE: range must be a safe integer")
                    .into());
            };
            if !range_f.is_finite()
                || range_f.fract() != 0.0
                || !(1.0..=(MAX_CRYPTO_RANDOM_INT_RANGE as f64)).contains(&range_f)
            {
                return Err(JsNativeError::range()
                    .with_message(
                        "ERR_OUT_OF_RANGE: The value of `max - min` must be in 1..=281474976710655",
                    )
                    .into());
            }
            let range = range_f as u64;
            let sampled = BOA_DET_STATE.with(|s| {
                if let Some((mut prng, clk)) = s.get() {
                    let v = sample_unbiased_u64_below(range, Some(&mut prng));
                    s.set(Some((prng, clk)));
                    v
                } else {
                    sample_unbiased_u64_below(range, None)
                }
            });
            Ok(BoaVal::from(sampled as f64))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_digest_bytes"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let algo = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "sha256".to_string());
            let Some(data_arg) = args.get(1) else {
                return bytes_to_js_uint8_array(&[], context);
            };
            let Some(norm_alg) = normalize_webcrypto_hash_name(&algo) else {
                return Err(JsNativeError::error()
                    .with_message(format!("Unsupported hash algorithm '{}'", algo))
                    .into());
            };
            let (digest_buf, digest_len) = with_buffer_source_bytes(data_arg, context, |bytes| {
                compute_webcrypto_digest_stack(norm_alg, bytes).unwrap_or(([0u8; 64], 0))
            })?;
            bytes_to_js_uint8_array(&digest_buf[..digest_len], context)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_hmac_bytes"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let algo = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "sha256".to_string());
            let Some(key_arg) = args.get(1) else {
                return bytes_to_js_uint8_array(&[], context);
            };
            let Some(data_arg) = args.get(2) else {
                return bytes_to_js_uint8_array(&[], context);
            };
            let mac_opt = with_two_buffer_source_bytes(
                key_arg,
                data_arg,
                context,
                |key_bytes, data_bytes| compute_webcrypto_hmac_stack(&algo, key_bytes, data_bytes),
            )?;
            let Some((mac_buf, mac_len)) = mac_opt else {
                return Err(JsNativeError::error()
                    .with_message(format!("Unsupported HMAC algorithm '{}'", algo))
                    .into());
            };
            bytes_to_js_uint8_array(&mac_buf[..mac_len], context)
        }),
    );
}
