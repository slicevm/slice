//! WHATWG WebCrypto (`crypto.subtle`) and `node:crypto` Host Bindings (Digests, HMAC, AES-GCM/CBC,
//! Ed25519, X25519, ECDSA P-256/P-384, RSA PKCS#1 v1.5 / RSA-PSS, PBKDF2, HKDF, scrypt, CSPRNG).

use crate::bindings::helpers::{
    bytes_to_js_array_buffer, bytes_to_js_uint8_array, compute_webcrypto_hmac,
    consume_crypto_kdf_fuel, extract_buffer_source_bytes, normalize_webcrypto_hash_name,
    try_consume_isolate_memory, with_buffer_source_bytes, with_two_buffer_source_bytes,
};
use crate::bindings::wintertc_and_buffer::register_buffer_host_functions;
use crate::boa_realm::{BOA_ACTIVE_TENANT, BOA_DET_STATE};
use crate::intrinsics::{
    aes_cbc_decrypt, aes_cbc_decrypt_with_expanded, aes_cbc_encrypt, aes_cbc_encrypt_with_expanded,
    aes_gcm_decrypt, aes_gcm_encrypt, aes_gcm_open_with_key, aes_gcm_seal_with_key, base64_decode,
    base64_encode, constant_time_eq, ecdsa_components_to_pkcs8, ecdsa_generate_keypair,
    ecdsa_pub_to_spki, ecdsa_sign_bytes, ecdsa_verify_bytes, ed25519_keypair_from_seed,
    ed25519_pub_to_spki, ed25519_seed_to_pkcs8_v1, ed25519_sign_bytes, ed25519_verify_bytes,
    extract_ecdsa_pkcs8_components, extract_ecdsa_pub, extract_ed25519_pub, extract_ed25519_seed,
    extract_x25519_pub, extract_x25519_seed, fill_random_bytes, generate_uuid_v4,
    hkdf_derive_bytes, insert_cached_crypto_key, inspect_rsa_private_key, inspect_rsa_public_key,
    make_ring_aead_key, md5_bytes, pbkdf2_derive_bytes, rsa_private_components_to_pkcs8,
    rsa_public_components_to_info, rsa_sign_bytes_with_alg, rsa_verify_bytes_with_alg,
    scrypt_derive_bytes, sha1_bytes, sha256_bytes, sha384_bytes, sha512_bytes,
    with_cached_crypto_key, x25519_diffie_hellman, x25519_keypair_from_seed, x25519_pub_to_spki,
    x25519_seed_to_pkcs8_v1, AesExpandedKey, CachedCryptoKeyMaterial,
};

pub(crate) fn register_node_crypto_bootstrap_host_function(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsValue as BoaVal, NativeFunction};
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

pub(crate) fn register_webcrypto_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::builtins::typed_array::{TypedArray, TypedArrayKind};
    use boa_engine::object::builtins::JsArrayBuffer;
    use boa_engine::{js_string, JsNativeError, JsValue as BoaVal, NativeFunction};
    register_buffer_host_functions(ctx);
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_crypto_random_uuid"), ctx)
        .unwrap_or(false)
    {
        return;
    }

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

            // Fast path: direct native downcast to `TypedArray` (zero JS property lookups).
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
                if let Some(res) = ta.with_bytes_mut(|slice| -> Result<(), &'static str> {
                    if slice.len() > 65_536 {
                        return Err(
                            "QuotaExceededError: crypto.getRandomValues byteLength exceeds 65536 bytes",
                        );
                    }
                    BOA_DET_STATE.with(|s| {
                        if let Some((mut prng, clk)) = s.get() {
                            fill_random_bytes(slice, Some(&mut prng));
                            s.set(Some((prng, clk)));
                        } else {
                            fill_random_bytes(slice, None);
                        }
                    });
                    Ok(())
                }) {
                    res.map_err(|msg| JsNativeError::error().with_message(msg))?;
                    return Ok(val.clone());
                }
            }

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

            let buf_val = obj.get(js_string!("buffer"), context)?;
            let Some(buf_obj) = buf_val.as_object() else {
                return Err(JsNativeError::typ()
                    .with_message("crypto.getRandomValues requires a TypedArray")
                    .into());
            };
            let ab = JsArrayBuffer::from_object(buf_obj)?;
            let offset = obj
                .get(js_string!("byteOffset"), context)
                .ok()
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as usize)
                .unwrap_or(0);
            let byte_len = obj
                .get(js_string!("byteLength"), context)
                .ok()
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as usize)
                .unwrap_or(0);

            if byte_len > 65_536 {
                return Err(JsNativeError::error()
                    .with_message(
                        "QuotaExceededError: crypto.getRandomValues byteLength exceeds 65536 bytes",
                    )
                    .into());
            }

            if let Some(mut data_mut) = ab.data_mut() {
                let start = offset.min(data_mut.len());
                let end = (start + byte_len).min(data_mut.len());
                let slice = &mut data_mut[start..end];
                BOA_DET_STATE.with(|s| {
                    if let Some((mut prng, clk)) = s.get() {
                        fill_random_bytes(slice, Some(&mut prng));
                        s.set(Some((prng, clk)));
                    } else {
                        fill_random_bytes(slice, None);
                    }
                });
            }
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
            let digest = with_buffer_source_bytes(data_arg, context, |bytes| match norm_alg {
                "SHA-1" => sha1_bytes(bytes).to_vec(),
                "SHA-256" => sha256_bytes(bytes).to_vec(),
                "SHA-384" => sha384_bytes(bytes).to_vec(),
                "SHA-512" => sha512_bytes(bytes).to_vec(),
                _ => Vec::new(),
            })?;
            bytes_to_js_array_buffer(&digest, context)
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
                let tenant = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
                let cached_sig = with_buffer_source_bytes(data_arg, context, |data_bytes| {
                    with_cached_crypto_key(handle_id, &tenant, |mat| match mat {
                        CachedCryptoKeyMaterial::Hmac { ring_key, .. } => {
                            let tag = ring::hmac::sign(ring_key, data_bytes);
                            Some(tag.as_ref().to_vec())
                        }
                        _ => None,
                    })
                    .flatten()
                })?;
                if let Some(sig_bytes) = cached_sig {
                    return bytes_to_js_array_buffer(&sig_bytes, context);
                }
            }
            let mac_opt = with_two_buffer_source_bytes(
                key_arg,
                data_arg,
                context,
                |key_bytes, data_bytes| compute_webcrypto_hmac(&hash_name, key_bytes, data_bytes),
            )?;
            let Some(mac) = mac_opt else {
                return Err(JsNativeError::error()
                    .with_message(format!(
                        "NotSupportedError: Unsupported HMAC hash '{}'",
                        hash_name
                    ))
                    .into());
            };
            bytes_to_js_array_buffer(&mac, context)
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
                let tenant = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
                let cached_ok = with_two_buffer_source_bytes(
                    sig_arg,
                    data_arg,
                    context,
                    |sig_bytes, data_bytes| {
                        with_cached_crypto_key(handle_id, &tenant, |mat| match mat {
                            CachedCryptoKeyMaterial::Hmac { ring_key, .. } => {
                                Some(ring::hmac::verify(ring_key, data_bytes, sig_bytes).is_ok())
                            }
                            _ => None,
                        })
                        .flatten()
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
                    compute_webcrypto_hmac(&hash_name, &key_bytes, data_bytes)
                        .map(|expected| constant_time_eq(&expected, sig_bytes))
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
                    if a_bytes.len() != b_bytes.len() {
                        None
                    } else {
                        Some(constant_time_eq(a_bytes, b_bytes))
                    }
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
            if let Err(msg) = try_consume_isolate_memory(size) {
                return Err(JsNativeError::range().with_message(msg).into());
            }
            let mut buf = vec![0u8; size];
            BOA_DET_STATE.with(|s| {
                if let Some((mut prng, clk)) = s.get() {
                    fill_random_bytes(&mut buf, Some(&mut prng));
                    s.set(Some((prng, clk)));
                } else {
                    fill_random_bytes(&mut buf, None);
                }
            });
            bytes_to_js_uint8_array(&buf, context)
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
            let digest = with_buffer_source_bytes(data_arg, context, |bytes| match norm_alg {
                "SHA-1" => sha1_bytes(bytes).to_vec(),
                "SHA-256" => sha256_bytes(bytes).to_vec(),
                "SHA-384" => sha384_bytes(bytes).to_vec(),
                "SHA-512" => sha512_bytes(bytes).to_vec(),
                "MD5" => md5_bytes(bytes).to_vec(),
                _ => Vec::new(),
            })?;
            bytes_to_js_uint8_array(&digest, context)
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
                |key_bytes, data_bytes| compute_webcrypto_hmac(&algo, key_bytes, data_bytes),
            )?;
            let Some(mac) = mac_opt else {
                return Err(JsNativeError::error()
                    .with_message(format!("Unsupported HMAC algorithm '{}'", algo))
                    .into());
            };
            bytes_to_js_uint8_array(&mac, context)
        }),
    );

    // Phase 2: Tenant-scoped key handle cache registration (`AES-GCM`, `AES-CBC`, `HMAC`)
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
                CachedCryptoKeyMaterial::AesGcm { less_safe, raw }
            } else if kind.eq_ignore_ascii_case("AES-CBC") {
                let expanded = AesExpandedKey::new(&raw)
                    .map_err(|msg| JsNativeError::error().with_message(msg))?;
                CachedCryptoKeyMaterial::AesCbc { expanded, raw }
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
                    ring_key,
                    hash_alg: norm_hash.to_string(),
                    raw,
                }
            } else {
                return Ok(BoaVal::from(0.0));
            };
            let handle_id = insert_cached_crypto_key(tenant, material);
            Ok(BoaVal::from(handle_id as f64))
        }),
    );

    // Phase 2: Symmetric cipher encrypt/decrypt (`AES-GCM`, `AES-CBC`) with cached handle fast-path
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
            let key_bytes = match args.get(3) {
                Some(v) if !v.is_undefined() && !v.is_null() => {
                    extract_buffer_source_bytes(v, context)?
                }
                _ => Vec::new(),
            };
            let iv_bytes = match args.get(4) {
                Some(v) if !v.is_undefined() && !v.is_null() => {
                    extract_buffer_source_bytes(v, context)?
                }
                _ => Vec::new(),
            };
            let aad_bytes = match args.get(5) {
                Some(v) if !v.is_undefined() && !v.is_null() => {
                    extract_buffer_source_bytes(v, context)?
                }
                _ => Vec::new(),
            };
            let tag_bits = args
                .get(6)
                .and_then(|v| v.as_number())
                .map(|n| n as usize)
                .unwrap_or(128);
            let data_bytes = match args.get(7) {
                Some(v) if !v.is_undefined() && !v.is_null() => {
                    extract_buffer_source_bytes(v, context)?
                }
                _ => Vec::new(),
            };
            let auto_pad = args.get(8).and_then(|v| v.as_boolean()).unwrap_or(true);
            let tenant = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());

            if alg.eq_ignore_ascii_case("AES-GCM")
                || alg.eq_ignore_ascii_case("aes-128-gcm")
                || alg.eq_ignore_ascii_case("aes-256-gcm")
            {
                let res = if handle_id != 0 {
                    with_cached_crypto_key(handle_id, &tenant, |mat| match mat {
                        CachedCryptoKeyMaterial::AesGcm { less_safe, raw } => {
                            if is_encrypt {
                                Some(aes_gcm_seal_with_key(
                                    less_safe,
                                    &iv_bytes,
                                    &aad_bytes,
                                    &data_bytes,
                                    tag_bits,
                                ))
                            } else {
                                Some(aes_gcm_open_with_key(
                                    less_safe,
                                    raw,
                                    &iv_bytes,
                                    &aad_bytes,
                                    &data_bytes,
                                    tag_bits,
                                ))
                            }
                        }
                        _ => None,
                    })
                    .flatten()
                } else {
                    None
                };
                let out = match res {
                    Some(r) => r,
                    None => {
                        if is_encrypt {
                            aes_gcm_encrypt(
                                &key_bytes,
                                &iv_bytes,
                                &aad_bytes,
                                &data_bytes,
                                tag_bits,
                            )
                        } else {
                            aes_gcm_decrypt(
                                &key_bytes,
                                &iv_bytes,
                                &aad_bytes,
                                &data_bytes,
                                tag_bits,
                            )
                        }
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
                let res = if handle_id != 0 {
                    with_cached_crypto_key(handle_id, &tenant, |mat| match mat {
                        CachedCryptoKeyMaterial::AesCbc { expanded, .. } => {
                            if is_encrypt {
                                Some(aes_cbc_encrypt_with_expanded(
                                    expanded,
                                    &iv_bytes,
                                    &data_bytes,
                                    auto_pad,
                                ))
                            } else {
                                Some(aes_cbc_decrypt_with_expanded(
                                    expanded,
                                    &iv_bytes,
                                    &data_bytes,
                                    auto_pad,
                                ))
                            }
                        }
                        _ => None,
                    })
                    .flatten()
                } else {
                    None
                };
                let out = match res {
                    Some(r) => r,
                    None => {
                        if is_encrypt {
                            aes_cbc_encrypt(&key_bytes, &iv_bytes, &data_bytes, auto_pad)
                        } else {
                            aes_cbc_decrypt(&key_bytes, &iv_bytes, &data_bytes, auto_pad)
                        }
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

    // Phase 2: Asymmetric key pair generation (`Ed25519`, `ECDSA` `P-256` / `P-384`)
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
                BOA_DET_STATE.with(|s| {
                    if let Some((mut prng, clk)) = s.get() {
                        fill_random_bytes(&mut seed, Some(&mut prng));
                        s.set(Some((prng, clk)));
                    } else {
                        fill_random_bytes(&mut seed, None);
                    }
                });
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
                BOA_DET_STATE.with(|s| {
                    if let Some((mut prng, clk)) = s.get() {
                        fill_random_bytes(&mut seed, Some(&mut prng));
                        s.set(Some((prng, clk)));
                    } else {
                        fill_random_bytes(&mut seed, None);
                    }
                });
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
                        d_bytes,
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

    // Phase 2: Asymmetric key import (`raw`, `pkcs8`, `spki`, `jwk`) into `CRYPTO_KEY_HANDLE_TABLE`
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
                        let d_raw = base64_decode(d_str);
                        let seed = extract_ed25519_seed(&d_raw)
                            .map_err(|msg| JsNativeError::error().with_message(msg))?;
                        let (seed_bytes, pub_bytes) = ed25519_keypair_from_seed(seed)
                            .map_err(|msg| JsNativeError::error().with_message(msg))?;
                        let pkcs8 = ed25519_seed_to_pkcs8_v1(&seed_bytes);
                        let id = insert_cached_crypto_key(
                            tenant,
                            CachedCryptoKeyMaterial::Ed25519Private {
                                seed: seed_bytes,
                                pub_bytes,
                            },
                        );
                        let out = serde_json::json!({
                            "id": id,
                            "type": "private",
                            "d": base64_encode(&seed_bytes, true),
                            "x": base64_encode(&pub_bytes, true),
                            "pkcs8": base64_encode(&pkcs8, false),
                        });
                        return Ok(BoaVal::from(js_string!(out.to_string())));
                    } else {
                        let x_raw = base64_decode(x_str);
                        let pub_bytes = extract_ed25519_pub(&x_raw)
                            .map_err(|msg| JsNativeError::error().with_message(msg))?;
                        let spki = ed25519_pub_to_spki(&pub_bytes);
                        let id = insert_cached_crypto_key(
                            tenant,
                            CachedCryptoKeyMaterial::Ed25519Public { pub_bytes },
                        );
                        let out = serde_json::json!({
                            "id": id,
                            "type": "public",
                            "x": base64_encode(&pub_bytes, true),
                            "pubRaw": base64_encode(&pub_bytes, false),
                            "spki": base64_encode(&spki, false),
                        });
                        return Ok(BoaVal::from(js_string!(out.to_string())));
                    }
                }
                let raw = extract_buffer_source_bytes(data_arg, context)?;
                if format.eq_ignore_ascii_case("pkcs8") {
                    let seed = extract_ed25519_seed(&raw)
                        .map_err(|msg| JsNativeError::error().with_message(msg))?;
                    let (seed_bytes, pub_bytes) = ed25519_keypair_from_seed(seed)
                        .map_err(|msg| JsNativeError::error().with_message(msg))?;
                    let pkcs8 = ed25519_seed_to_pkcs8_v1(&seed_bytes);
                    let id = insert_cached_crypto_key(
                        tenant,
                        CachedCryptoKeyMaterial::Ed25519Private {
                            seed: seed_bytes,
                            pub_bytes,
                        },
                    );
                    let out = serde_json::json!({
                        "id": id,
                        "type": "private",
                        "d": base64_encode(&seed_bytes, true),
                        "x": base64_encode(&pub_bytes, true),
                        "pkcs8": base64_encode(&pkcs8, false),
                    });
                    return Ok(BoaVal::from(js_string!(out.to_string())));
                } else {
                    let pub_bytes = extract_ed25519_pub(&raw)
                        .map_err(|msg| JsNativeError::error().with_message(msg))?;
                    let spki = ed25519_pub_to_spki(&pub_bytes);
                    let id = insert_cached_crypto_key(
                        tenant,
                        CachedCryptoKeyMaterial::Ed25519Public { pub_bytes },
                    );
                    let out = serde_json::json!({
                        "id": id,
                        "type": "public",
                        "x": base64_encode(&pub_bytes, true),
                        "pubRaw": base64_encode(&pub_bytes, false),
                        "spki": base64_encode(&spki, false),
                    });
                    return Ok(BoaVal::from(js_string!(out.to_string())));
                }
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
                                d_bytes: d_bytes.clone(),
                                pub_bytes: pub_bytes.clone(),
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
                    } else {
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
                            d_bytes,
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
                } else {
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
                        let pkcs8 = x25519_seed_to_pkcs8_v1(&seed_bytes);
                        let spki = x25519_pub_to_spki(&pub_bytes);
                        let id = insert_cached_crypto_key(
                            tenant,
                            CachedCryptoKeyMaterial::X25519Private {
                                seed: seed_bytes,
                                pub_bytes,
                            },
                        );
                        let out = serde_json::json!({
                            "id": id,
                            "type": "private",
                            "crv": "X25519",
                            "d": base64_encode(&seed_bytes, true),
                            "x": base64_encode(&pub_bytes, true),
                            "pkcs8": base64_encode(&pkcs8, false),
                            "spki": base64_encode(&spki, false),
                        });
                        return Ok(BoaVal::from(js_string!(out.to_string())));
                    } else {
                        let x_raw = base64_decode(x_str);
                        if x_raw.len() != 32 {
                            return Err(JsNativeError::error()
                                .with_message("DataError: Invalid X25519 JWK public key length")
                                .into());
                        }
                        let mut pub_bytes = [0u8; 32];
                        pub_bytes.copy_from_slice(&x_raw);
                        let spki = x25519_pub_to_spki(&pub_bytes);
                        let id = insert_cached_crypto_key(
                            tenant,
                            CachedCryptoKeyMaterial::X25519Public { pub_bytes },
                        );
                        let out = serde_json::json!({
                            "id": id,
                            "type": "public",
                            "crv": "X25519",
                            "x": base64_encode(&pub_bytes, true),
                            "pubRaw": base64_encode(&pub_bytes, false),
                            "spki": base64_encode(&spki, false),
                        });
                        return Ok(BoaVal::from(js_string!(out.to_string())));
                    }
                }
                let raw = extract_buffer_source_bytes(data_arg, context)?;
                if format.eq_ignore_ascii_case("pkcs8") {
                    let seed = extract_x25519_seed(&raw)
                        .map_err(|msg| JsNativeError::error().with_message(msg))?;
                    let (seed_bytes, pub_bytes) = x25519_keypair_from_seed(seed);
                    let pkcs8 = x25519_seed_to_pkcs8_v1(&seed_bytes);
                    let spki = x25519_pub_to_spki(&pub_bytes);
                    let id = insert_cached_crypto_key(
                        tenant,
                        CachedCryptoKeyMaterial::X25519Private {
                            seed: seed_bytes,
                            pub_bytes,
                        },
                    );
                    let out = serde_json::json!({
                        "id": id,
                        "type": "private",
                        "crv": "X25519",
                        "d": base64_encode(&seed_bytes, true),
                        "x": base64_encode(&pub_bytes, true),
                        "pkcs8": base64_encode(&pkcs8, false),
                        "spki": base64_encode(&spki, false),
                    });
                    return Ok(BoaVal::from(js_string!(out.to_string())));
                } else {
                    if format.eq_ignore_ascii_case("raw") && raw.len() != 32 {
                        return Err(JsNativeError::error()
                            .with_message("DataError: X25519 raw public key must be 32 bytes")
                            .into());
                    }
                    let pub_bytes = extract_x25519_pub(&raw)
                        .map_err(|msg| JsNativeError::error().with_message(msg))?;
                    let spki = x25519_pub_to_spki(&pub_bytes);
                    let id = insert_cached_crypto_key(
                        tenant,
                        CachedCryptoKeyMaterial::X25519Public { pub_bytes },
                    );
                    let out = serde_json::json!({
                        "id": id,
                        "type": "public",
                        "crv": "X25519",
                        "x": base64_encode(&pub_bytes, true),
                        "pubRaw": base64_encode(&pub_bytes, false),
                        "spki": base64_encode(&spki, false),
                    });
                    return Ok(BoaVal::from(js_string!(out.to_string())));
                }
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
                        let pkcs8_b64 = base64_encode(&pkcs8, false);
                        let spki_b64 = base64_encode(&pub_info.spki, false);
                        let id = insert_cached_crypto_key(
                            tenant,
                            CachedCryptoKeyMaterial::RsaPrivate {
                                pkcs8,
                                spki: pub_info.spki,
                            },
                        );
                        let out = serde_json::json!({
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
                        });
                        return Ok(BoaVal::from(js_string!(out.to_string())));
                    } else {
                        let pub_info = rsa_public_components_to_info(&n_bytes, &e_bytes)
                            .map_err(|msg| JsNativeError::error().with_message(msg))?;
                        let spki_b64 = base64_encode(&pub_info.spki, false);
                        let id = insert_cached_crypto_key(
                            tenant,
                            CachedCryptoKeyMaterial::RsaPublic {
                                spki: pub_info.spki,
                            },
                        );
                        let out = serde_json::json!({
                            "id": id,
                            "type": "public",
                            "modulusLength": pub_info.modulus_bits,
                            "publicExponent": pub_info.public_exponent,
                            "n": base64_encode(&pub_info.n_be, true),
                            "e": base64_encode(&pub_info.e_be, true),
                            "pubRaw": spki_b64,
                            "spki": spki_b64,
                        });
                        return Ok(BoaVal::from(js_string!(out.to_string())));
                    }
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
                    let pkcs8_b64 = base64_encode(&pkcs8, false);
                    let spki_b64 = base64_encode(&pub_info.spki, false);
                    let id = insert_cached_crypto_key(
                        tenant,
                        CachedCryptoKeyMaterial::RsaPrivate {
                            pkcs8,
                            spki: pub_info.spki,
                        },
                    );
                    let out = serde_json::json!({
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
                    });
                    return Ok(BoaVal::from(js_string!(out.to_string())));
                } else {
                    let pub_info = inspect_rsa_public_key(&raw)
                        .map_err(|msg| JsNativeError::error().with_message(msg))?;
                    let spki_b64 = base64_encode(&pub_info.spki, false);
                    let id = insert_cached_crypto_key(
                        tenant,
                        CachedCryptoKeyMaterial::RsaPublic {
                            spki: pub_info.spki,
                        },
                    );
                    let out = serde_json::json!({
                        "id": id,
                        "type": "public",
                        "modulusLength": pub_info.modulus_bits,
                        "publicExponent": pub_info.public_exponent,
                        "n": base64_encode(&pub_info.n_be, true),
                        "e": base64_encode(&pub_info.e_be, true),
                        "pubRaw": spki_b64,
                        "spki": spki_b64,
                    });
                    return Ok(BoaVal::from(js_string!(out.to_string())));
                }
            }

            Err(JsNativeError::error()
                .with_message(format!(
                    "NotSupportedError: Unsupported import algorithm '{}'",
                    alg
                ))
                .into())
        }),
    );

    // Phase 2: Asymmetric sign (`Ed25519`, `ECDSA` `P-256` / `P-384`)
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
            let key_fallback = match args.get(3) {
                Some(v) if !v.is_undefined() && !v.is_null() => {
                    extract_buffer_source_bytes(v, context)?
                }
                _ => Vec::new(),
            };
            let msg_bytes = match args.get(4) {
                Some(v) if !v.is_undefined() && !v.is_null() => {
                    extract_buffer_source_bytes(v, context)?
                }
                _ => Vec::new(),
            };
            let asn1_der = args.get(5).and_then(|v| v.as_boolean()).unwrap_or(false);
            let tenant = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());

            if alg.eq_ignore_ascii_case("Ed25519") {
                let sig_res = if handle_id != 0 {
                    with_cached_crypto_key(handle_id, &tenant, |mat| match mat {
                        CachedCryptoKeyMaterial::Ed25519Private { seed, .. } => {
                            Some(ed25519_sign_bytes(seed, &msg_bytes).map(|s| s.to_vec()))
                        }
                        _ => None,
                    })
                    .flatten()
                } else {
                    None
                };
                let sig = match sig_res {
                    Some(r) => r,
                    None => extract_ed25519_seed(&key_fallback)
                        .and_then(|seed| ed25519_sign_bytes(&seed, &msg_bytes).map(|s| s.to_vec())),
                }
                .map_err(|msg| JsNativeError::error().with_message(msg))?;
                return bytes_to_js_array_buffer(&sig, context);
            }

            if alg.eq_ignore_ascii_case("ECDSA") || alg.eq_ignore_ascii_case("ec") {
                let sig_res = if handle_id != 0 {
                    with_cached_crypto_key(handle_id, &tenant, |mat| match mat {
                        CachedCryptoKeyMaterial::EcdsaPrivate {
                            curve: k_curve,
                            pkcs8,
                            ..
                        } => Some(ecdsa_sign_bytes(k_curve, pkcs8, &msg_bytes, asn1_der)),
                        _ => None,
                    })
                    .flatten()
                } else {
                    None
                };
                let sig = match sig_res {
                    Some(r) => r,
                    None => ecdsa_sign_bytes(&curve, &key_fallback, &msg_bytes, asn1_der),
                }
                .map_err(|msg| JsNativeError::error().with_message(msg))?;
                return bytes_to_js_array_buffer(&sig, context);
            }

            if alg.eq_ignore_ascii_case("RSA")
                || alg.eq_ignore_ascii_case("RSASSA-PKCS1-v1_5")
                || alg.eq_ignore_ascii_case("RSA-PSS")
            {
                if let Err(msg) = consume_crypto_kdf_fuel(40) {
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                let use_pss = alg.eq_ignore_ascii_case("RSA-PSS");
                let sig_res = if handle_id != 0 {
                    with_cached_crypto_key(handle_id, &tenant, |mat| match mat {
                        CachedCryptoKeyMaterial::RsaPrivate { pkcs8, .. } => {
                            Some(rsa_sign_bytes_with_alg(pkcs8, &msg_bytes, use_pss, &curve))
                        }
                        _ => None,
                    })
                    .flatten()
                } else {
                    None
                };
                let sig = match sig_res {
                    Some(r) => r,
                    None => rsa_sign_bytes_with_alg(&key_fallback, &msg_bytes, use_pss, &curve),
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

    // Phase 2: Asymmetric verify (`Ed25519`, `ECDSA` `P-256` / `P-384`)
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
            let pub_fallback = match args.get(3) {
                Some(v) if !v.is_undefined() && !v.is_null() => {
                    extract_buffer_source_bytes(v, context)?
                }
                _ => Vec::new(),
            };
            let sig_bytes = match args.get(4) {
                Some(v) if !v.is_undefined() && !v.is_null() => {
                    extract_buffer_source_bytes(v, context)?
                }
                _ => Vec::new(),
            };
            let msg_bytes = match args.get(5) {
                Some(v) if !v.is_undefined() && !v.is_null() => {
                    extract_buffer_source_bytes(v, context)?
                }
                _ => Vec::new(),
            };
            let tenant = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());

            if alg.eq_ignore_ascii_case("Ed25519") {
                let verified = if handle_id != 0 {
                    with_cached_crypto_key(handle_id, &tenant, |mat| match mat {
                        CachedCryptoKeyMaterial::Ed25519Public { pub_bytes }
                        | CachedCryptoKeyMaterial::Ed25519Private { pub_bytes, .. } => {
                            Some(ed25519_verify_bytes(pub_bytes, &msg_bytes, &sig_bytes))
                        }
                        _ => None,
                    })
                    .flatten()
                } else {
                    None
                };
                let ok = match verified {
                    Some(v) => v,
                    None => match extract_ed25519_pub(&pub_fallback) {
                        Ok(pk) => ed25519_verify_bytes(&pk, &msg_bytes, &sig_bytes),
                        Err(_) => false,
                    },
                };
                return Ok(BoaVal::from(ok));
            }

            if alg.eq_ignore_ascii_case("ECDSA") || alg.eq_ignore_ascii_case("ec") {
                let verified = if handle_id != 0 {
                    with_cached_crypto_key(handle_id, &tenant, |mat| match mat {
                        CachedCryptoKeyMaterial::EcdsaPublic {
                            curve: k_curve,
                            pub_bytes,
                        }
                        | CachedCryptoKeyMaterial::EcdsaPrivate {
                            curve: k_curve,
                            pub_bytes,
                            ..
                        } => Some(ecdsa_verify_bytes(
                            k_curve, pub_bytes, &msg_bytes, &sig_bytes,
                        )),
                        _ => None,
                    })
                    .flatten()
                } else {
                    None
                };
                let ok = match verified {
                    Some(v) => v,
                    None => match extract_ecdsa_pub(&curve, &pub_fallback) {
                        Ok(pk) => ecdsa_verify_bytes(&curve, &pk, &msg_bytes, &sig_bytes),
                        Err(_) => false,
                    },
                };
                return Ok(BoaVal::from(ok));
            }

            if alg.eq_ignore_ascii_case("RSA")
                || alg.eq_ignore_ascii_case("RSASSA-PKCS1-v1_5")
                || alg.eq_ignore_ascii_case("RSA-PSS")
            {
                if let Err(msg) = consume_crypto_kdf_fuel(15) {
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                let use_pss = alg.eq_ignore_ascii_case("RSA-PSS");
                let verified = if handle_id != 0 {
                    with_cached_crypto_key(handle_id, &tenant, |mat| match mat {
                        CachedCryptoKeyMaterial::RsaPublic { spki }
                        | CachedCryptoKeyMaterial::RsaPrivate { spki, .. } => {
                            Some(rsa_verify_bytes_with_alg(
                                spki, &msg_bytes, &sig_bytes, use_pss, &curve,
                            ))
                        }
                        _ => None,
                    })
                    .flatten()
                } else {
                    None
                };
                let ok = match verified {
                    Some(v) => v,
                    None => rsa_verify_bytes_with_alg(
                        &pub_fallback,
                        &msg_bytes,
                        &sig_bytes,
                        use_pss,
                        &curve,
                    ),
                };
                return Ok(BoaVal::from(ok));
            }

            Ok(BoaVal::from(false))
        }),
    );

    // Phase 2: Constant-time RFC 7748 X25519 Diffie-Hellman key agreement (`crypto.diffieHellman` / `subtle.deriveBits`)
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
            let priv_fallback = match args.get(2) {
                Some(v) if !v.is_undefined() && !v.is_null() => {
                    extract_buffer_source_bytes(v, context)?
                }
                _ => Vec::new(),
            };
            let pub_id = args
                .get(3)
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let pub_fallback = match args.get(4) {
                Some(v) if !v.is_undefined() && !v.is_null() => {
                    extract_buffer_source_bytes(v, context)?
                }
                _ => Vec::new(),
            };
            let tenant = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());

            let cached_seed = if priv_id != 0 {
                with_cached_crypto_key(priv_id, &tenant, |mat| match mat {
                    CachedCryptoKeyMaterial::X25519Private { seed, .. } => Some(*seed),
                    _ => None,
                })
                .flatten()
            } else {
                None
            };
            let priv_seed = match cached_seed {
                Some(s) => s,
                None => extract_x25519_seed(&priv_fallback)
                    .map_err(|msg| JsNativeError::error().with_message(msg))?,
            };

            let cached_pub = if pub_id != 0 {
                with_cached_crypto_key(pub_id, &tenant, |mat| match mat {
                    CachedCryptoKeyMaterial::X25519Public { pub_bytes }
                    | CachedCryptoKeyMaterial::X25519Private { pub_bytes, .. } => Some(*pub_bytes),
                    _ => None,
                })
                .flatten()
            } else {
                None
            };
            let peer_pub = match cached_pub {
                Some(p) => p,
                None => extract_x25519_pub(&pub_fallback)
                    .map_err(|msg| JsNativeError::error().with_message(msg))?,
            };

            let shared = x25519_diffie_hellman(&priv_seed, &peer_pub)
                .map_err(|msg| JsNativeError::error().with_message(msg))?;
            bytes_to_js_array_buffer(&shared, context)
        }),
    );

    // Phase 2: Key Derivation Functions (`PBKDF2`, `HKDF`, `scrypt`) with proportional CPU fuel accounting
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
            let secret = match args.get(2) {
                Some(v) if !v.is_undefined() && !v.is_null() => {
                    extract_buffer_source_bytes(v, context)?
                }
                _ => Vec::new(),
            };
            let salt = match args.get(3) {
                Some(v) if !v.is_undefined() && !v.is_null() => {
                    extract_buffer_source_bytes(v, context)?
                }
                _ => Vec::new(),
            };
            let info = match args.get(4) {
                Some(v) if !v.is_undefined() && !v.is_null() => {
                    extract_buffer_source_bytes(v, context)?
                }
                _ => Vec::new(),
            };
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
                if let Err(msg) = try_consume_isolate_memory(derived.len()) {
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                return bytes_to_js_array_buffer(&derived, context);
            }

            if alg.eq_ignore_ascii_case("HKDF") {
                if let Err(msg) = consume_crypto_kdf_fuel(15) {
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
                if let Err(msg) = try_consume_isolate_memory(derived.len()) {
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
                if let Err(msg) = try_consume_isolate_memory(derived.len()) {
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
