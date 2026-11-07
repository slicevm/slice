//! Native Rust Host Callbacks (`__slice_*`) Registered into `boa_engine::Context`.

use crate::boa_realm::{
    check_boa_fs_read_ocap, check_boa_fs_write_ocap, norm_vfs_path, SavedIsolateContextGuard,
    BOA_ACTIVE_TENANT, BOA_ALS_ACTIVE, BOA_ALS_SNAP, BOA_DET_STATE, BOA_EGRESS_SECRETS,
    BOA_ENV_MAP, BOA_EXEC_BINS, BOA_FETCH_MOCKS, BOA_FUEL_BUDGET, BOA_HTTP_REQ, BOA_HTTP_RES,
    BOA_KV_MEM_BUDGET, BOA_LAST_CHILD_PROC_VIOLATION, BOA_STDOUT, BOA_VFS_CACHE, BOA_VFS_DIRS,
    BOA_VIOLATION, BOA_WAIT_UNTIL_COUNT,
};
use crate::event_loop::{
    cancel_timer, clear_event_loop_state, drive_event_loop_to_completion, enqueue_async_fetch,
    schedule_immediate, schedule_timer,
};
use crate::intrinsics::{
    aes_cbc_decrypt, aes_cbc_decrypt_with_expanded, aes_cbc_encrypt, aes_cbc_encrypt_with_expanded,
    aes_gcm_decrypt, aes_gcm_encrypt, aes_gcm_open_with_key, aes_gcm_seal_with_key, base64_decode,
    base64_encode, bytes_to_hex, constant_time_eq, ecdsa_components_to_pkcs8,
    ecdsa_generate_keypair, ecdsa_pub_to_spki, ecdsa_sign_bytes, ecdsa_verify_bytes,
    ed25519_keypair_from_seed, ed25519_pub_to_spki, ed25519_seed_to_pkcs8_v1, ed25519_sign_bytes,
    ed25519_verify_bytes, extract_ecdsa_pkcs8_components, extract_ecdsa_pub, extract_ed25519_pub,
    extract_ed25519_seed, fast_format_bytes, fast_format_ms, fast_parse_bytes, fast_parse_ms,
    fill_random_bytes, generate_uuid_v4, hex_decode_bytes, hkdf_derive_bytes, hmac_md5_bytes,
    hmac_sha1_bytes, hmac_sha256_bytes, hmac_sha256_hex, hmac_sha384_bytes, hmac_sha512_bytes,
    insert_cached_crypto_key, make_ring_aead_key, md5_bytes, pbkdf2_derive_bytes,
    scrypt_derive_bytes, semver_coerce_xyz, semver_parse_strict_xyz, semver_satisfies_xyz,
    sha1_bytes, sha256_bytes, sha256_hex, sha384_bytes, sha512_bytes, whatwg_atob, whatwg_btoa,
    with_cached_crypto_key, zlib_compress_bytes, zlib_decompress_bytes, AesExpandedKey,
    CachedCryptoKeyMaterial,
};
use crate::isolate::{SandboxEvalOptions, SliceIsolate};
use crate::js_preludes::eval_lazy_prelude_shard;
use crate::node_builtins::eval_lazy_builtin_module;
use crate::outbound_http::{
    check_boa_outbound_fetch_ocap_and_dlp, execute_outbound_fetch_with_redirects,
    parse_outbound_http_url,
};
use crate::sockets::{
    classify_ip_literal, register_socket_mock, tcp_socket_close, tcp_socket_connect,
    tcp_socket_read, tcp_socket_starttls, tcp_socket_write, tcp_socket_write_bytes,
    udp_socket_bind, udp_socket_close, udp_socket_send, udp_socket_send_bytes,
};
use crate::state_and_kv::{
    checkpoint_tenant_state, edge_kv_del, edge_kv_get, edge_kv_incr, edge_kv_set,
    restore_tenant_state, splitmix64_next_f64, verify_egress_secrets_slice,
};
use crate::wasm::{
    wasm_call_export, wasm_compile_module, wasm_create_memory, wasm_get_global, wasm_grow_memory,
    wasm_instantiate_module, wasm_module_custom_sections, wasm_module_exports_json,
    wasm_module_imports_json, wasm_set_global, wasm_sync_initial_memory_to_ab, wasm_validate_bytes,
};
use crate::websocket::{
    ws_client_close, ws_client_connect, ws_client_poll, ws_client_send, ws_server_queue_close,
    ws_server_queue_send, BOA_WS_MOCKS,
};
use crate::workers::{
    worker_post_message_to_child, worker_self_meta_json, worker_self_post_message,
    worker_self_set_listening, worker_set_child_ref, worker_spawn, worker_terminate_child,
};
use slice_sandbox::{CapabilityToken, SandboxViolation};

pub(crate) fn register_slice_native_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsValue as BoaVal, NativeFunction};

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_console_log"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let line = if args.len() == 1 {
                let first = &args[0];
                if let Some(s) = first.as_string() {
                    s.to_std_string_escaped()
                } else {
                    first.to_string(context)?.to_std_string_escaped()
                }
            } else if args.is_empty() {
                String::new()
            } else {
                let mut joined = String::with_capacity(64);
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        joined.push(' ');
                    }
                    if let Some(s) = arg.as_string() {
                        joined.push_str(&s.to_std_string_escaped());
                    } else {
                        joined.push_str(&arg.to_string(context)?.to_std_string_escaped());
                    }
                }
                joined
            };
            BOA_STDOUT.with(|out| out.borrow_mut().push(line));
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_load_builtin"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let id = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            eval_lazy_builtin_module(&id, context)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_lazy_prelude"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let shard_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            eval_lazy_prelude_shard(shard_id, context)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_os_platform"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let is_det = BOA_DET_STATE.with(|s| s.get().is_some());
            let plat = if is_det {
                "linux"
            } else {
                match std::env::consts::OS {
                    "windows" => "win32",
                    "macos" => "darwin",
                    other => other,
                }
            };
            Ok(BoaVal::from(js_string!(plat)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_os_arch"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let is_det = BOA_DET_STATE.with(|s| s.get().is_some());
            let arch = if is_det {
                "x64"
            } else {
                match std::env::consts::ARCH {
                    "x86_64" => "x64",
                    "aarch64" => "arm64",
                    other => other,
                }
            };
            Ok(BoaVal::from(js_string!(arch)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_sha256"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let enc = args
                .get(1)
                .and_then(|v| v.as_string())
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "hex".to_string());
            let encode_digest = |raw: &[u8]| -> String {
                if enc.eq_ignore_ascii_case("base64") {
                    base64_encode(raw, false)
                } else if enc.eq_ignore_ascii_case("base64url") {
                    base64_encode(raw, true)
                } else {
                    bytes_to_hex(raw)
                }
            };
            if let Some(first) = args.first() {
                if first.is_object() {
                    if let Ok(bytes) = extract_buffer_source_bytes(first, context) {
                        if enc.eq_ignore_ascii_case("hex") {
                            return Ok(BoaVal::from(js_string!(sha256_hex(&bytes))));
                        }
                        let digest = sha256_bytes(&bytes);
                        return Ok(BoaVal::from(js_string!(encode_digest(&digest))));
                    }
                }
            }
            let input = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if enc.eq_ignore_ascii_case("hex") {
                return Ok(BoaVal::from(js_string!(sha256_hex(input.as_bytes()))));
            }
            let digest = sha256_bytes(input.as_bytes());
            Ok(BoaVal::from(js_string!(encode_digest(&digest))))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_hmac_sha256"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let key = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let msg = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let enc = args
                .get(2)
                .and_then(|v| v.as_string())
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "hex".to_string());
            if enc.eq_ignore_ascii_case("base64") {
                let mac = hmac_sha256_bytes(key.as_bytes(), msg.as_bytes());
                return Ok(BoaVal::from(js_string!(base64_encode(&mac, false))));
            }
            if enc.eq_ignore_ascii_case("base64url") {
                let mac = hmac_sha256_bytes(key.as_bytes(), msg.as_bytes());
                return Ok(BoaVal::from(js_string!(base64_encode(&mac, true))));
            }
            let hex = hmac_sha256_hex(key.as_bytes(), msg.as_bytes());
            Ok(BoaVal::from(js_string!(hex)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_base64_encode"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let url_safe = args.get(1).and_then(|v| v.as_boolean()).unwrap_or(false);
            let Some(first) = args.first() else {
                return Ok(BoaVal::from(js_string!("")));
            };
            if first.is_object() {
                let bytes = extract_buffer_source_bytes(first, context)?;
                return Ok(BoaVal::from(js_string!(base64_encode(&bytes, url_safe))));
            }
            let input = first.to_string(context)?.to_std_string_escaped();
            let encoded = base64_encode(input.as_bytes(), url_safe);
            Ok(BoaVal::from(js_string!(encoded)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_base64_decode"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let input = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let decoded = String::from_utf8_lossy(&base64_decode(&input)).into_owned();
            Ok(BoaVal::from(js_string!(decoded)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_buf_to_utf8"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::from(js_string!("")));
            };
            if first.is_string() {
                return Ok(first.clone());
            }
            if first.is_object() {
                let bytes = extract_buffer_source_bytes(first, context)?;
                let decoded = String::from_utf8_lossy(&bytes).into_owned();
                return Ok(BoaVal::from(js_string!(decoded)));
            }
            let s = first.to_string(context)?.to_std_string_escaped();
            Ok(BoaVal::from(js_string!(s)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_utf8_byte_len"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::from(0.0));
            };
            if let Some(obj) = first.as_object() {
                if let Ok(bl) = obj.get(js_string!("byteLength"), context) {
                    if let Some(n) = bl.as_number() {
                        return Ok(BoaVal::from(n));
                    }
                }
                if let Ok(l) = obj.get(js_string!("length"), context) {
                    if let Some(n) = l.as_number() {
                        return Ok(BoaVal::from(n));
                    }
                }
            }
            let s = first.to_string(context)?.to_std_string_escaped();
            if let Some(enc_val) = args.get(1).filter(|v| !v.is_undefined() && !v.is_null()) {
                let e = enc_val.to_string(context)?.to_std_string_escaped();
                if e.eq_ignore_ascii_case("hex") {
                    return Ok(BoaVal::from((s.len() >> 1) as f64));
                }
                if e.eq_ignore_ascii_case("base64") || e.eq_ignore_ascii_case("base64url") {
                    return Ok(BoaVal::from(base64_decode(&s).len() as f64));
                }
                if e.eq_ignore_ascii_case("latin1")
                    || e.eq_ignore_ascii_case("binary")
                    || e.eq_ignore_ascii_case("ascii")
                {
                    return Ok(BoaVal::from(s.chars().count() as f64));
                }
                if e.eq_ignore_ascii_case("utf16le")
                    || e.eq_ignore_ascii_case("utf-16le")
                    || e.eq_ignore_ascii_case("ucs2")
                    || e.eq_ignore_ascii_case("ucs-2")
                {
                    return Ok(BoaVal::from((s.encode_utf16().count() * 2) as f64));
                }
            }
            Ok(BoaVal::from(s.len() as f64))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_env_get"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let key = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let found = BOA_ENV_MAP.with(|m| m.borrow().get(&key).cloned());
            match found {
                Some(v) => Ok(BoaVal::from(js_string!(v))),
                None => {
                    let tenant = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
                    BOA_VIOLATION.with(|v| {
                        *v.borrow_mut() = Some(SandboxViolation::EnvAccessDenied {
                            tenant_id: tenant,
                            key,
                        });
                    });
                    Ok(BoaVal::from(js_string!("")))
                }
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_env_json"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let json = BOA_ENV_MAP
                .with(|m| serde_json::to_string(&*m.borrow()).unwrap_or_else(|_| "{}".to_string()));
            Ok(BoaVal::from(js_string!(json)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_timer_set"),
        4,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(cb_obj) = args.first().and_then(|v| v.as_object()) else {
                return Ok(BoaVal::from(0));
            };
            let delay_ms = args
                .get(1)
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as u64)
                .unwrap_or(0);
            let is_interval = args.get(2).and_then(|v| v.as_boolean()).unwrap_or(false);
            let mut extra_args = Vec::new();
            if let Some(arr_obj) = args.get(3).and_then(|v| v.as_object()) {
                if let Ok(len_val) = arr_obj.get(js_string!("length"), context) {
                    let len = len_val.as_number().unwrap_or(0.0) as u32;
                    for i in 0..len.min(8) {
                        if let Ok(elem) = arr_obj.get(i, context) {
                            extra_args.push(elem);
                        }
                    }
                }
            }
            match schedule_timer(cb_obj.clone(), delay_ms, is_interval, extra_args) {
                Ok(id) => Ok(BoaVal::from(id as f64)),
                Err(_) => Ok(BoaVal::from(0)),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_immediate_set"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(cb_obj) = args.first().and_then(|v| v.as_object()) else {
                return Ok(BoaVal::from(0));
            };
            let mut extra_args = Vec::new();
            if let Some(arr_obj) = args.get(1).and_then(|v| v.as_object()) {
                if let Ok(len_val) = arr_obj.get(js_string!("length"), context) {
                    let len = len_val.as_number().unwrap_or(0.0) as u32;
                    for i in 0..len.min(8) {
                        if let Ok(elem) = arr_obj.get(i, context) {
                            extra_args.push(elem);
                        }
                    }
                }
            }
            match schedule_immediate(cb_obj.clone(), extra_args) {
                Ok(id) => Ok(BoaVal::from(id as f64)),
                Err(_) => Ok(BoaVal::from(0)),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_timer_clear"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as u32)
                .unwrap_or(0);
            cancel_timer(id);
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_is_active"),
        0,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            if let Some(mode) = args.first().and_then(|v| v.as_number()) {
                let (used, rem) = BOA_KV_MEM_BUDGET.with(|m| m.get());
                if mode == 1.0 {
                    return Ok(BoaVal::from(used as f64));
                } else if mode == 2.0 {
                    return Ok(BoaVal::from(rem as f64));
                }
            }
            let active = BOA_HTTP_REQ.with(|r| r.borrow().active);
            Ok(BoaVal::from(active))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_mark_wait_until"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            BOA_WAIT_UNTIL_COUNT.with(|c| c.set(c.get() + 1));
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_text_encode"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let s = match args.first() {
                None => String::new(),
                Some(v) if v.is_undefined() => String::new(),
                Some(v) => v.to_string(context)?.to_std_string_escaped(),
            };
            bytes_to_js_uint8_array(s.as_bytes(), context)
        }),
    );
}

pub(crate) fn register_deterministic_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsValue as BoaVal, NativeFunction};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_det_random"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_det_random"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let val = BOA_DET_STATE.with(|s| {
                if let Some((mut prng, clk)) = s.get() {
                    let r = splitmix64_next_f64(&mut prng);
                    s.set(Some((prng, clk)));
                    r
                } else {
                    -1.0
                }
            });
            Ok(BoaVal::from(val))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_det_now"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let val = BOA_DET_STATE.with(|s| {
                if let Some((prng, clk)) = s.get() {
                    let next_clk = clk.wrapping_add(1);
                    s.set(Some((prng, next_clk)));
                    next_clk as f64
                } else {
                    -1.0
                }
            });
            Ok(BoaVal::from(val))
        }),
    );
}

pub(crate) fn register_os_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsValue as BoaVal, NativeFunction};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_os_tmpdir"), ctx)
        .unwrap_or(false)
    {
        return;
    }
    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_os_tmpdir"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let tmp = std::env::temp_dir().to_string_lossy().replace('\\', "/");
            Ok(BoaVal::from(js_string!(tmp)))
        }),
    );
}

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

pub(crate) fn register_semver_and_fast_intrinsics_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsValue as BoaVal, NativeFunction};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_semver_valid"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_semver_valid"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::null());
            };
            if first.is_null() || first.is_undefined() {
                return Ok(BoaVal::null());
            }
            let s = if let Some(obj) = first.as_object() {
                if let Ok(v) = obj.get(js_string!("version"), context) {
                    v.to_string(context)?.to_std_string_escaped()
                } else {
                    first.to_string(context)?.to_std_string_escaped()
                }
            } else {
                first.to_string(context)?.to_std_string_escaped()
            };
            match semver_parse_strict_xyz(&s) {
                Some((maj, min, pat)) => {
                    Ok(BoaVal::from(js_string!(format!("{}.{}.{}", maj, min, pat))))
                }
                None => Ok(BoaVal::null()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_semver_coerce"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::null());
            };
            let s = first.to_string(context)?.to_std_string_escaped();
            match semver_coerce_xyz(&s) {
                Some((maj, min, pat)) => {
                    Ok(BoaVal::from(js_string!(format!("{}.{}.{}", maj, min, pat))))
                }
                None => Ok(BoaVal::null()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_semver_gt"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let a = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let b = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if let (Some(va), Some(vb)) = (semver_parse_strict_xyz(&a), semver_parse_strict_xyz(&b))
            {
                Ok(BoaVal::from(va > vb))
            } else {
                Ok(BoaVal::from(false))
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_semver_satisfies"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let ver = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let rng = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            match semver_satisfies_xyz(&ver, &rng) {
                Some(true) => Ok(BoaVal::from(1)),
                Some(false) => Ok(BoaVal::from(0)),
                None => Ok(BoaVal::from(-1)),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fast_ms"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::undefined());
            };
            if let Some(s) = first.as_string() {
                let raw = s.to_std_string_escaped();
                if let Some(ms_val) = fast_parse_ms(&raw) {
                    return Ok(BoaVal::from(ms_val));
                }
            } else if let Some(n) = first.as_number() {
                let long = args.get(1).and_then(|v| v.as_boolean()).unwrap_or(false);
                if let Some(formatted) = fast_format_ms(n, long) {
                    return Ok(BoaVal::from(js_string!(formatted)));
                }
            }
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fast_bytes_parse"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::undefined());
            };
            if let Some(n) = first.as_number() {
                return Ok(BoaVal::from(n));
            }
            if let Some(s) = first.as_string() {
                let raw = s.to_std_string_escaped();
                if let Some(b_val) = fast_parse_bytes(&raw) {
                    return Ok(BoaVal::from(b_val));
                }
            }
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fast_bytes_format"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            if args
                .get(1)
                .is_some_and(|v| !v.is_undefined() && !v.is_null())
            {
                return Ok(BoaVal::undefined());
            }
            let Some(n) = args.first().and_then(|v| v.as_number()) else {
                return Ok(BoaVal::undefined());
            };
            if let Some(formatted) = fast_format_bytes(n) {
                return Ok(BoaVal::from(js_string!(formatted)));
            }
            Ok(BoaVal::undefined())
        }),
    );
}

pub(crate) fn register_http_bridge_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsValue as BoaVal, NativeFunction};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_http_method"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_headers_all"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let json = BOA_HTTP_REQ.with(|r| {
                let borrow = r.borrow();
                serde_json::to_string(&borrow.headers[..borrow.headers_len])
                    .unwrap_or_else(|_| "[]".to_string())
            });
            Ok(BoaVal::from(js_string!(json)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_method"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let s = BOA_HTTP_REQ.with(|r| r.borrow().method.clone());
            Ok(BoaVal::from(js_string!(s)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_path"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let s = BOA_HTTP_REQ.with(|r| r.borrow().path.clone());
            Ok(BoaVal::from(js_string!(s)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_query"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let s = BOA_HTTP_REQ.with(|r| r.borrow().query.clone());
            Ok(BoaVal::from(js_string!(s)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_tenant"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let s = BOA_HTTP_REQ.with(|r| r.borrow().tenant_id.clone());
            Ok(BoaVal::from(js_string!(s)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_body"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let s = BOA_HTTP_REQ.with(|r| r.borrow().body.clone());
            Ok(BoaVal::from(js_string!(s)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_header"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let key = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let found = BOA_HTTP_REQ.with(|r| {
                let borrow = r.borrow();
                borrow.headers[..borrow.headers_len]
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(&key))
                    .map(|(_, v)| v.clone())
            });
            match found {
                Some(v) => Ok(BoaVal::from(js_string!(v))),
                None => Ok(BoaVal::null()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_respond"),
        4,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let status = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u16)
                .unwrap_or(200);
            let body = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let content_type = args
                .get(2)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "application/json".to_string());
            let headers_json = args
                .get(3)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let extra_headers: Vec<(String, String)> = if headers_json.is_empty() {
                Vec::new()
            } else {
                serde_json::from_str(&headers_json).unwrap_or_default()
            };
            BOA_HTTP_RES.with(|res| {
                *res.borrow_mut() = Some((status, body, content_type, extra_headers));
            });
            Ok(BoaVal::undefined())
        }),
    );
}

pub(crate) fn register_kv_and_sandbox_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsValue as BoaVal, NativeFunction};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_kv_get"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_clear_timers"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            clear_event_loop_state();
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_drain_event_loop"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, context| {
            drive_event_loop_to_completion(context);
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_kv_get"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let key = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let val = BOA_ACTIVE_TENANT.with(|t| edge_kv_get(&t.borrow(), &key));
            match val {
                Some(s) => Ok(BoaVal::from(js_string!(s))),
                None => Ok(BoaVal::null()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_kv_set"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let key = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let val = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let ttl_ms = args
                .get(2)
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as u64)
                .unwrap_or(0);
            let added = key.len().saturating_add(val.len());
            let allowed = BOA_KV_MEM_BUDGET.with(|m| {
                let (used, limit) = m.get();
                let next = used.saturating_add(added);
                if next > limit {
                    false
                } else {
                    m.set((next, limit));
                    true
                }
            });
            if !allowed {
                let tenant = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
                let limit = BOA_KV_MEM_BUDGET.with(|m| m.get().1);
                BOA_VIOLATION.with(|v| {
                    *v.borrow_mut() = Some(SandboxViolation::MemoryCeilingExceeded {
                        tenant_id: tenant,
                        requested: added,
                        limit,
                    });
                });
                return Ok(BoaVal::from(false));
            }
            BOA_ACTIVE_TENANT.with(|t| edge_kv_set(&t.borrow(), &key, &val, ttl_ms));
            Ok(BoaVal::from(true))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_kv_del"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let key = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let removed = BOA_ACTIVE_TENANT.with(|t| edge_kv_del(&t.borrow(), &key));
            Ok(BoaVal::from(removed))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_kv_incr"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let key = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let delta = args
                .get(1)
                .and_then(|v| v.as_number())
                .map(|n| n as i64)
                .unwrap_or(1);
            let next = BOA_ACTIVE_TENANT.with(|t| edge_kv_incr(&t.borrow(), &key, delta));
            Ok(BoaVal::from(next as f64))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_checkpoint"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let id = BOA_ACTIVE_TENANT.with(|t| checkpoint_tenant_state(&t.borrow()));
            Ok(BoaVal::from(id as f64))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_restore"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u64)
                .unwrap_or(0);
            let ok = BOA_ACTIVE_TENANT.with(|t| restore_tenant_state(&t.borrow(), id));
            Ok(BoaVal::from(ok))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_eval_sandbox"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let code = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let opts_json = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "{}".to_string());
            let opts: SandboxEvalOptions = serde_json::from_str(&opts_json).unwrap_or_default();

            let scope_guard = SavedIsolateContextGuard::enter();

            let mut parent_caps = CapabilityToken::pure_compute();
            if let Some((_, ref r_prefs, ref w_prefs)) = scope_guard.saved_caps {
                for r in r_prefs {
                    parent_caps = parent_caps.with_read_prefix(r.clone());
                }
                for w in w_prefs {
                    parent_caps = parent_caps.with_write_prefix(w.clone());
                }
            }
            for nh in &scope_guard.saved_net_hosts {
                parent_caps = parent_caps.with_net_host(nh.clone());
            }
            for (ek, ev) in &scope_guard.saved_env_map {
                parent_caps = parent_caps.with_env(ek.clone(), ev.clone());
            }
            for eb in &scope_guard.saved_exec_bins {
                parent_caps = parent_caps.with_exec_binary(eb.clone());
            }
            let parent_rem_mem = scope_guard
                .saved_kv_budget
                .1
                .saturating_sub(scope_guard.saved_kv_budget.0);
            let mut proxy_parent = SliceIsolate::spawn(
                scope_guard.saved_tenant.clone(),
                98,
                parent_caps,
                5_000_000,
                parent_rem_mem.max(65536),
            );
            for sec in &scope_guard.saved_egress_secrets {
                proxy_parent.add_egress_audit_secret(sec.clone());
            }
            let outcome = if cfg!(debug_assertions) {
                std::thread::Builder::new()
                    .name("slice-eval-sandbox".to_string())
                    .stack_size(8 * 1024 * 1024)
                    .spawn(move || proxy_parent.eval_sandbox(&code, opts))
                    .expect("failed to spawn slice-eval-sandbox thread")
                    .join()
                    .expect("slice-eval-sandbox thread panicked")
            } else {
                proxy_parent.eval_sandbox(&code, opts)
            };
            drop(scope_guard);

            let json = serde_json::to_string(&outcome).unwrap_or_else(|_| "{}".to_string());
            Ok(BoaVal::from(js_string!(json)))
        }),
    );
}

pub(crate) fn register_shard10_outbound_fetch_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsValue as BoaVal, NativeFunction};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_fetch_try_fast"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_mock_fetch"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let key = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let payload = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !key.is_empty() && !payload.is_empty() {
                BOA_FETCH_MOCKS.with(|m| {
                    m.borrow_mut().insert(key, payload);
                });
            }
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fetch_sync"),
        5,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let url = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let method = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped().to_ascii_uppercase())
                .unwrap_or_else(|| "GET".to_string());
            let headers_json = args
                .get(2)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "[]".to_string());
            let body = args
                .get(3)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let redirect_mode = args
                .get(4)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "follow".to_string());

            let Some(parts) = parse_outbound_http_url(&url) else {
                let err_json =
                    serde_json::json!({ "__error": format!("Invalid outbound URL: {}", url) })
                        .to_string();
                return Ok(BoaVal::from(js_string!(err_json)));
            };

            if let Err(err_msg) =
                check_boa_outbound_fetch_ocap_and_dlp(&parts, &url, &headers_json, &body)
            {
                let err_json = serde_json::json!({ "__error": err_msg }).to_string();
                return Ok(BoaVal::from(js_string!(err_json)));
            }

            let method_url_key = format!("{} {}", method, url);
            if let Some(mock_json) = BOA_FETCH_MOCKS.with(|m| {
                let map = m.borrow();
                map.get(&method_url_key).or_else(|| map.get(&url)).cloned()
            }) {
                return Ok(BoaVal::from(js_string!(mock_json)));
            }

            let cassette_key = format!("fetch_cassette:{}:{}", method, url);
            let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
            if let Some(cassette_json) = edge_kv_get(&tenant_id, &cassette_key) {
                return Ok(BoaVal::from(js_string!(cassette_json)));
            }

            let req_headers: Vec<(String, String)> =
                serde_json::from_str(&headers_json).unwrap_or_default();
            match execute_outbound_fetch_with_redirects(
                &url,
                &method,
                &req_headers,
                &body,
                &redirect_mode,
            ) {
                Ok(resp_json) => {
                    if BOA_DET_STATE.with(|s| s.get().is_some()) {
                        edge_kv_set(&tenant_id, &cassette_key, &resp_json, 0);
                    }
                    Ok(BoaVal::from(js_string!(resp_json)))
                }
                Err(err_msg) => {
                    let err_json = serde_json::json!({ "__error": err_msg }).to_string();
                    Ok(BoaVal::from(js_string!(err_json)))
                }
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fetch_try_fast"),
        5,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let url = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let method = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped().to_ascii_uppercase())
                .unwrap_or_else(|| "GET".to_string());
            let headers_json = args
                .get(2)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "[]".to_string());
            let body = args
                .get(3)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let _redirect_mode = args
                .get(4)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "follow".to_string());

            let Some(parts) = parse_outbound_http_url(&url) else {
                let err_json =
                    serde_json::json!({ "__error": format!("Invalid outbound URL: {}", url) })
                        .to_string();
                return Ok(BoaVal::from(js_string!(err_json)));
            };

            if let Err(err_msg) =
                check_boa_outbound_fetch_ocap_and_dlp(&parts, &url, &headers_json, &body)
            {
                let err_json = serde_json::json!({ "__error": err_msg }).to_string();
                return Ok(BoaVal::from(js_string!(err_json)));
            }

            let method_url_key = format!("{} {}", method, url);
            if let Some(mock_json) = BOA_FETCH_MOCKS.with(|m| {
                let map = m.borrow();
                map.get(&method_url_key).or_else(|| map.get(&url)).cloned()
            }) {
                return Ok(BoaVal::from(js_string!(mock_json)));
            }

            let cassette_key = format!("fetch_cassette:{}:{}", method, url);
            let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
            if let Some(cassette_json) = edge_kv_get(&tenant_id, &cassette_key) {
                return Ok(BoaVal::from(js_string!(cassette_json)));
            }

            Ok(BoaVal::from(js_string!("")))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fetch_enqueue"),
        8,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let url = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let method = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped().to_ascii_uppercase())
                .unwrap_or_else(|| "GET".to_string());
            let headers_json = args
                .get(2)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "[]".to_string());
            let body = args
                .get(3)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let redirect_mode = args
                .get(4)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "follow".to_string());
            let Some(resolve_fn) = args.get(5).and_then(|v| v.as_object()) else {
                return Ok(BoaVal::from(0));
            };
            let Some(reject_fn) = args.get(6).and_then(|v| v.as_object()) else {
                return Ok(BoaVal::from(0));
            };
            let signal_obj = args.get(7).and_then(|v| v.as_object());

            let req_headers: Vec<(String, String)> =
                serde_json::from_str(&headers_json).unwrap_or_default();
            let id = enqueue_async_fetch(
                url,
                method,
                req_headers,
                body,
                redirect_mode,
                resolve_fn.clone(),
                reject_fn.clone(),
                signal_obj,
            );
            Ok(BoaVal::from(id as f64))
        }),
    );
}

fn extract_buffer_source_bytes(
    val: &boa_engine::JsValue,
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<Vec<u8>> {
    use boa_engine::js_string;
    use boa_engine::object::builtins::JsArrayBuffer;

    if let Some(s) = val.as_string() {
        return Ok(s.to_std_string_escaped().into_bytes());
    }
    let Some(obj) = val.as_object() else {
        return Ok(Vec::new());
    };
    if let Ok(ab) = JsArrayBuffer::from_object(obj.clone()) {
        if let Some(data) = ab.data() {
            return Ok(data.to_vec());
        }
        return Ok(Vec::new());
    }
    if let Ok(buf_val) = obj.get(js_string!("buffer"), context) {
        if let Some(buf_obj) = buf_val.as_object() {
            if let Ok(ab) = JsArrayBuffer::from_object(buf_obj) {
                let offset = obj
                    .get(js_string!("byteOffset"), context)
                    .ok()
                    .and_then(|v| v.as_number())
                    .map(|n| n.max(0.0) as usize)
                    .unwrap_or(0);
                let len = obj
                    .get(js_string!("byteLength"), context)
                    .ok()
                    .and_then(|v| v.as_number())
                    .map(|n| n.max(0.0) as usize);
                if let Some(data) = ab.data() {
                    let start = offset.min(data.len());
                    let end = len
                        .map(|l| (start + l).min(data.len()))
                        .unwrap_or(data.len());
                    return Ok(data[start..end].to_vec());
                }
            }
        }
    }
    if let Ok(len_val) = obj.get(js_string!("length"), context) {
        if let Some(len) = len_val.as_number().map(|n| n.max(0.0) as usize) {
            let mut out = Vec::with_capacity(len.min(1_048_576));
            for i in 0..len.min(1_048_576) {
                let b = obj
                    .get(i as u32, context)?
                    .as_number()
                    .map(|n| (n as i64 & 0xff) as u8)
                    .unwrap_or(0);
                out.push(b);
            }
            return Ok(out);
        }
    }
    Ok(Vec::new())
}

fn bytes_to_js_array_buffer(
    bytes: &[u8],
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<boa_engine::JsValue> {
    use boa_engine::object::builtins::{AlignedVec, JsArrayBuffer};
    let block = AlignedVec::from_slice(0, bytes);
    let ab = JsArrayBuffer::from_byte_block(block, context)?;
    Ok(ab.into())
}

fn bytes_to_js_uint8_array(
    bytes: &[u8],
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<boa_engine::JsValue> {
    use boa_engine::object::builtins::{AlignedVec, JsArrayBuffer, JsUint8Array};
    let block = AlignedVec::from_slice(0, bytes);
    let ab = JsArrayBuffer::from_byte_block(block, context)?;
    let u8_arr = JsUint8Array::from_array_buffer(ab, context)?;
    Ok(u8_arr.into())
}

#[inline]
fn normalize_webcrypto_hash_name(raw: &str) -> Option<&'static str> {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("SHA-1") || trimmed.eq_ignore_ascii_case("SHA1") {
        Some("SHA-1")
    } else if trimmed.eq_ignore_ascii_case("SHA-256") || trimmed.eq_ignore_ascii_case("SHA256") {
        Some("SHA-256")
    } else if trimmed.eq_ignore_ascii_case("SHA-384") || trimmed.eq_ignore_ascii_case("SHA384") {
        Some("SHA-384")
    } else if trimmed.eq_ignore_ascii_case("SHA-512") || trimmed.eq_ignore_ascii_case("SHA512") {
        Some("SHA-512")
    } else if trimmed.eq_ignore_ascii_case("MD5") || trimmed.eq_ignore_ascii_case("MD-5") {
        Some("MD5")
    } else {
        None
    }
}

fn compute_webcrypto_hmac(hash_name: &str, key: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    match normalize_webcrypto_hash_name(hash_name)? {
        "SHA-1" => Some(hmac_sha1_bytes(key, data).to_vec()),
        "SHA-256" => Some(hmac_sha256_bytes(key, data).to_vec()),
        "SHA-384" => Some(hmac_sha384_bytes(key, data).to_vec()),
        "SHA-512" => Some(hmac_sha512_bytes(key, data).to_vec()),
        "MD5" => Some(hmac_md5_bytes(key, data).to_vec()),
        _ => None,
    }
}

/// Charges `cost` CPU fuel against the active isolate's `BOA_FUEL_BUDGET`, recording
/// `SandboxViolation::CpuFuelExhausted` and returning `Err` if the budget is exceeded.
#[inline]
fn consume_crypto_kdf_fuel(cost: u64) -> Result<(), String> {
    let (consumed, limit) = BOA_FUEL_BUDGET.with(|f| f.get());
    let next = consumed.saturating_add(cost.max(1));
    BOA_FUEL_BUDGET.with(|f| f.set((next, limit)));
    if next > limit {
        let tenant_id = BOA_ACTIVE_TENANT.with(|t| {
            let b = t.borrow();
            if b.is_empty() {
                "boa-tenant".to_string()
            } else {
                b.clone()
            }
        });
        BOA_VIOLATION.with(|v| {
            if v.borrow().is_none() {
                *v.borrow_mut() = Some(SandboxViolation::CpuFuelExhausted {
                    tenant_id,
                    limit,
                });
            }
        });
        return Err(format!(
            "RangeError: Cryptographic KDF CPU fuel budget exhausted (limit: {} units)",
            limit
        ));
    }
    Ok(())
}

pub(crate) fn register_wintertc_phase_n3_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::object::builtins::JsArrayBuffer;
    use boa_engine::{js_string, JsNativeError, JsValue as BoaVal, NativeFunction};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_btoa"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_btoa"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let s = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|js| js.to_std_string_escaped())
                .unwrap_or_default();
            match whatwg_btoa(&s) {
                Ok(encoded) => Ok(BoaVal::from(js_string!(encoded))),
                Err(msg) => Err(JsNativeError::error().with_message(msg).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_atob"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let s = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|js| js.to_std_string_escaped())
                .unwrap_or_default();
            match whatwg_atob(&s) {
                Ok(decoded) => Ok(BoaVal::from(js_string!(decoded))),
                Err(msg) => Err(JsNativeError::error().with_message(msg).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_text_encode_into"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let s = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|js| js.to_std_string_escaped())
                .unwrap_or_default();
            let Some(dest_obj) = args.get(1).and_then(|v| v.as_object()) else {
                return Err(JsNativeError::typ()
                    .with_message("TextEncoder.encodeInto requires a Uint8Array destination")
                    .into());
            };
            let buf_val = dest_obj.get(js_string!("buffer"), context)?;
            let Some(buf_obj) = buf_val.as_object() else {
                return Err(JsNativeError::typ()
                    .with_message("TextEncoder.encodeInto requires a Uint8Array destination")
                    .into());
            };
            let ab = JsArrayBuffer::from_object(buf_obj)?;
            let offset = dest_obj
                .get(js_string!("byteOffset"), context)
                .ok()
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as usize)
                .unwrap_or(0);
            let byte_len = dest_obj
                .get(js_string!("byteLength"), context)
                .ok()
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as usize)
                .unwrap_or(0);

            let mut read_utf16 = 0usize;
            let mut written_utf8 = 0usize;
            if let Some(mut data_mut) = ab.data_mut() {
                let start = offset.min(data_mut.len());
                let end = (start + byte_len).min(data_mut.len());
                let dest_slice = &mut data_mut[start..end];
                for ch in s.chars() {
                    let need = ch.len_utf8();
                    if written_utf8 + need > dest_slice.len() {
                        break;
                    }
                    ch.encode_utf8(&mut dest_slice[written_utf8..written_utf8 + need]);
                    written_utf8 += need;
                    read_utf16 += ch.len_utf16();
                }
            }

            let res_obj = boa_engine::JsObject::with_object_proto(context.intrinsics());
            let _ = res_obj.set(
                js_string!("read"),
                BoaVal::from(read_utf16 as f64),
                false,
                context,
            );
            let _ = res_obj.set(
                js_string!("written"),
                BoaVal::from(written_utf8 as f64),
                false,
                context,
            );
            Ok(res_obj.into())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_text_decode"),
        4,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::from(js_string!("")));
            };
            if first.is_undefined() || first.is_null() {
                return Ok(BoaVal::from(js_string!("")));
            }
            let bytes = extract_buffer_source_bytes(first, context)?;
            let enc = args
                .get(1)
                .and_then(|v| v.as_string())
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "utf-8".to_string());
            let fatal = args.get(2).and_then(|v| v.as_boolean()).unwrap_or(false);
            let ignore_bom = args.get(3).and_then(|v| v.as_boolean()).unwrap_or(false);

            if enc.eq_ignore_ascii_case("latin1") || enc.eq_ignore_ascii_case("iso-8859-1") {
                let mut s = String::with_capacity(bytes.len());
                for &b in &bytes {
                    s.push(b as char);
                }
                return Ok(BoaVal::from(js_string!(s)));
            }
            if enc.eq_ignore_ascii_case("ascii") || enc.eq_ignore_ascii_case("us-ascii") {
                let mut s = String::with_capacity(bytes.len());
                for &b in &bytes {
                    if b > 0x7F {
                        if fatal {
                            return Err(JsNativeError::typ()
                                .with_message("The encoded data was not valid for encoding ascii")
                                .into());
                        }
                        s.push('\u{FFFD}');
                    } else {
                        s.push(b as char);
                    }
                }
                return Ok(BoaVal::from(js_string!(s)));
            }

            let slice = if !ignore_bom && bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
                &bytes[3..]
            } else {
                &bytes[..]
            };
            if fatal {
                match std::str::from_utf8(slice) {
                    Ok(valid) => Ok(BoaVal::from(js_string!(valid))),
                    Err(_) => Err(JsNativeError::typ()
                        .with_message("The encoded data was not valid for encoding utf-8")
                        .into()),
                }
            } else {
                let decoded = String::from_utf8_lossy(slice).into_owned();
                Ok(BoaVal::from(js_string!(decoded)))
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_zlib_compress"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let bytes = match args.first() {
                Some(v) if !v.is_undefined() && !v.is_null() => {
                    extract_buffer_source_bytes(v, context)?
                }
                _ => Vec::new(),
            };
            let fmt = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "gzip".to_string());
            match zlib_compress_bytes(&bytes, &fmt) {
                Ok(compressed) => bytes_to_js_uint8_array(&compressed, context),
                Err(msg) => Err(JsNativeError::typ().with_message(msg).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_zlib_decompress"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let bytes = match args.first() {
                Some(v) if !v.is_undefined() && !v.is_null() => {
                    extract_buffer_source_bytes(v, context)?
                }
                _ => Vec::new(),
            };
            let fmt = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "gzip".to_string());
            match zlib_decompress_bytes(&bytes, &fmt) {
                Ok(decompressed) => bytes_to_js_uint8_array(&decompressed, context),
                Err(msg) => Err(JsNativeError::typ().with_message(msg).into()),
            }
        }),
    );
}

pub(crate) fn register_async_hooks_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsValue as BoaVal, NativeFunction};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_als_get"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_als_get"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            Ok(BOA_ALS_SNAP.with(|s| s.borrow().clone()))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_als_set"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let next = args.first().cloned().unwrap_or_else(BoaVal::undefined);
            BOA_ALS_ACTIVE.with(|a| a.set(true));
            let prev = BOA_ALS_SNAP.with(|s| s.replace(next));
            Ok(prev)
        }),
    );
}

const VFS_SYMLINK_PREFIX: &[u8] = b"\x00__SLICE_SYMLINK__:";

fn resolve_vfs_symlink_norm(norm: &str) -> String {
    let mut current = norm.to_string();
    for _ in 0..8 {
        let next_opt = BOA_VFS_CACHE.with(|c| {
            c.borrow().get(&current).and_then(|bytes| {
                bytes
                    .strip_prefix(VFS_SYMLINK_PREFIX)
                    .map(|target| norm_vfs_path(&String::from_utf8_lossy(target)))
            })
        });
        if let Some(next) = next_opt {
            current = next;
        } else {
            break;
        }
    }
    current
}

pub(crate) fn register_fs_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsValue as BoaVal, NativeFunction};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_fs_read_file_sync"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_read_file_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_read_ocap(&path) {
                return Ok(BoaVal::from(js_string!("")));
            }
            let norm = resolve_vfs_symlink_norm(&norm_vfs_path(&path));
            if !check_boa_fs_read_ocap(&norm) {
                return Ok(BoaVal::from(js_string!("")));
            }
            if let Some(arc_bytes) = BOA_VFS_CACHE.with(|c| c.borrow().get(&norm).cloned()) {
                let content = String::from_utf8_lossy(&arc_bytes).into_owned();
                return Ok(BoaVal::from(js_string!(content)));
            }
            let bytes = std::fs::read(&norm)
                .or_else(|_| std::fs::read(&path))
                .unwrap_or_default();
            let content = String::from_utf8_lossy(&bytes).into_owned();
            Ok(BoaVal::from(js_string!(content)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_read_bytes_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_read_ocap(&path) {
                return bytes_to_js_uint8_array(&[], context);
            }
            let norm = resolve_vfs_symlink_norm(&norm_vfs_path(&path));
            if !check_boa_fs_read_ocap(&norm) {
                return bytes_to_js_uint8_array(&[], context);
            }
            if let Some(arc_bytes) = BOA_VFS_CACHE.with(|c| c.borrow().get(&norm).cloned()) {
                return bytes_to_js_uint8_array(&arc_bytes, context);
            }
            let bytes = std::fs::read(&norm)
                .or_else(|_| std::fs::read(&path))
                .unwrap_or_default();
            bytes_to_js_uint8_array(&bytes, context)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_write_file_sync"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_write_ocap(&path) {
                return Ok(BoaVal::undefined());
            }
            let data_bytes = if let Some(val) = args.get(1) {
                if val.is_object() {
                    extract_buffer_source_bytes(val, context)?
                } else {
                    val.to_string(context)?.to_std_string_escaped().into_bytes()
                }
            } else {
                Vec::new()
            };
            let norm = norm_vfs_path(&path);
            if let Some(idx) = norm.rfind('/') {
                let parent = &norm[..idx];
                BOA_VFS_DIRS.with(|d| {
                    d.borrow_mut().insert(parent.to_string());
                });
            }
            BOA_VFS_CACHE.with(|c| {
                c.borrow_mut().insert(norm, std::sync::Arc::new(data_bytes));
            });
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_append_file_sync"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_write_ocap(&path) {
                return Ok(BoaVal::undefined());
            }
            let data_bytes = if let Some(val) = args.get(1) {
                if val.is_object() {
                    extract_buffer_source_bytes(val, context)?
                } else {
                    val.to_string(context)?.to_std_string_escaped().into_bytes()
                }
            } else {
                Vec::new()
            };
            let norm = norm_vfs_path(&path);
            BOA_VFS_CACHE.with(|c| {
                let mut map = c.borrow_mut();
                let entry = map
                    .entry(norm)
                    .or_insert_with(|| std::sync::Arc::new(std::fs::read(&path).unwrap_or_default()));
                std::sync::Arc::make_mut(entry).extend_from_slice(&data_bytes);
            });
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_exists_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_read_ocap(&path) {
                return Ok(BoaVal::from(false));
            }
            let norm = norm_vfs_path(&path);
            let exists = BOA_VFS_CACHE.with(|c| c.borrow().contains_key(&norm))
                || BOA_VFS_DIRS.with(|d| d.borrow().contains(&norm))
                || std::path::Path::new(&path).exists();
            Ok(BoaVal::from(exists))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_mkdir_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_write_ocap(&path) {
                return Ok(BoaVal::undefined());
            }
            let norm = norm_vfs_path(&path);
            BOA_VFS_DIRS.with(|d| {
                d.borrow_mut().insert(norm);
            });
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_readdir_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_read_ocap(&path) {
                return Ok(BoaVal::from(js_string!("[]")));
            }
            let norm_dir = norm_vfs_path(&path);
            let prefix = format!("{}/", norm_dir);
            let mut entries_set = std::collections::BTreeSet::new();
            BOA_VFS_CACHE.with(|c| {
                for k in c.borrow().keys() {
                    if let Some(rest) = k.strip_prefix(&prefix) {
                        if !rest.contains('/') && !rest.is_empty() {
                            entries_set.insert(rest.to_string());
                        }
                    }
                }
            });
            if let Ok(rd) = std::fs::read_dir(&path) {
                for entry in rd.flatten() {
                    entries_set.insert(entry.file_name().to_string_lossy().to_string());
                }
            }
            let entries: Vec<String> = entries_set.into_iter().collect();
            let json = serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string());
            Ok(BoaVal::from(js_string!(json)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_stat_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_read_ocap(&path) {
                return Ok(BoaVal::from(0));
            }
            let norm = norm_vfs_path(&path);
            let size = BOA_VFS_CACHE
                .with(|c| c.borrow().get(&norm).map(|v| v.len() as u64))
                .unwrap_or_else(|| std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0));
            Ok(BoaVal::from(size as f64))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_mtime_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_read_ocap(&path) || BOA_DET_STATE.with(|s| s.get().is_some()) {
                return Ok(BoaVal::from(1_700_000_000_000.0));
            }
            let mtime_ms = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as f64)
                .unwrap_or_else(|| {
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as f64)
                        .unwrap_or(1_700_000_000_000.0)
                });
            Ok(BoaVal::from(mtime_ms))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_is_dir_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_read_ocap(&path) {
                return Ok(BoaVal::from(false));
            }
            let norm = norm_vfs_path(&path);
            let prefix = format!("{}/", norm);
            let is_dir = BOA_VFS_DIRS.with(|d| d.borrow().contains(&norm))
                || BOA_VFS_CACHE.with(|c| c.borrow().keys().any(|k| k.starts_with(&prefix)))
                || std::path::Path::new(&path).is_dir();
            Ok(BoaVal::from(is_dir))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_copy_file_sync"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let src = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let dest = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_read_ocap(&src) || !check_boa_fs_write_ocap(&dest) {
                return Ok(BoaVal::undefined());
            }
            let norm_src = norm_vfs_path(&src);
            let norm_dest = norm_vfs_path(&dest);
            let content = BOA_VFS_CACHE
                .with(|c| c.borrow().get(&norm_src).cloned())
                .unwrap_or_else(|| std::sync::Arc::new(std::fs::read(&src).unwrap_or_default()));
            if let Some(idx) = norm_dest.rfind('/') {
                let parent = &norm_dest[..idx];
                BOA_VFS_DIRS.with(|d| {
                    d.borrow_mut().insert(parent.to_string());
                });
            }
            BOA_VFS_CACHE.with(|c| {
                c.borrow_mut().insert(norm_dest, content);
            });
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_rename_sync"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let old_p = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let new_p = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_read_ocap(&old_p) || !check_boa_fs_write_ocap(&new_p) {
                return Ok(BoaVal::undefined());
            }
            let norm_old = norm_vfs_path(&old_p);
            let norm_new = norm_vfs_path(&new_p);
            let content = BOA_VFS_CACHE
                .with(|c| c.borrow_mut().remove(&norm_old))
                .unwrap_or_else(|| std::sync::Arc::new(std::fs::read(&old_p).unwrap_or_default()));
            if let Some(idx) = norm_new.rfind('/') {
                let parent = &norm_new[..idx];
                BOA_VFS_DIRS.with(|d| {
                    d.borrow_mut().insert(parent.to_string());
                });
            }
            BOA_VFS_CACHE.with(|c| {
                c.borrow_mut().insert(norm_new, content);
            });
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_rm_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_write_ocap(&path) {
                return Ok(BoaVal::undefined());
            }
            let norm = norm_vfs_path(&path);
            let prefix = format!("{}/", norm);
            BOA_VFS_CACHE.with(|c| {
                let mut map = c.borrow_mut();
                map.remove(&norm);
                map.retain(|k, _| !k.starts_with(&prefix));
            });
            BOA_VFS_DIRS.with(|d| {
                let mut dirs = d.borrow_mut();
                dirs.remove(&norm);
                dirs.retain(|k| !k.starts_with(&prefix));
            });
            let p = std::path::Path::new(&path);
            if p.exists() {
                if p.is_dir() {
                    let _ = std::fs::remove_dir_all(p);
                } else {
                    let _ = std::fs::remove_file(p);
                }
            }
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_realpath_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_read_ocap(&path) {
                return Ok(BoaVal::from(js_string!("")));
            }
            let norm = resolve_vfs_symlink_norm(&norm_vfs_path(&path));
            if !check_boa_fs_read_ocap(&norm) {
                return Ok(BoaVal::from(js_string!("")));
            }
            if BOA_VFS_CACHE.with(|c| c.borrow().contains_key(&norm))
                || BOA_VFS_DIRS.with(|d| d.borrow().contains(&norm))
            {
                let resolved = if norm.starts_with('/')
                    || (norm.len() >= 2 && norm.as_bytes()[1] == b':')
                {
                    norm
                } else {
                    let cwd = std::env::current_dir()
                        .map(|p| norm_vfs_path(&p.to_string_lossy()))
                        .unwrap_or_else(|_| "/workspace".to_string());
                    format!("{}/{}", cwd.trim_end_matches('/'), norm)
                };
                return Ok(BoaVal::from(js_string!(resolved)));
            }
            if let Ok(canon) = std::fs::canonicalize(&norm).or_else(|_| std::fs::canonicalize(&path))
            {
                let mut s = canon.to_string_lossy().replace('\\', "/");
                if let Some(stripped) = s.strip_prefix("//?/") {
                    s = stripped.to_string();
                }
                return Ok(BoaVal::from(js_string!(s)));
            }
            let fallback = if norm.starts_with('/')
                || (norm.len() >= 2 && norm.as_bytes()[1] == b':')
            {
                norm
            } else {
                let cwd = std::env::current_dir()
                    .map(|p| norm_vfs_path(&p.to_string_lossy()))
                    .unwrap_or_else(|_| "/workspace".to_string());
                format!("{}/{}", cwd.trim_end_matches('/'), norm)
            };
            Ok(BoaVal::from(js_string!(fallback)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_mkdtemp_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let prefix = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "/tmp/slice-".to_string());
            let mut rnd = [0u8; 6];
            BOA_DET_STATE.with(|s| {
                if let Some((mut prng, clock)) = s.get() {
                    fill_random_bytes(&mut rnd, Some(&mut prng));
                    s.set(Some((prng, clock)));
                } else {
                    fill_random_bytes(&mut rnd, None);
                }
            });
            const ALPH: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
            let mut suffix = String::with_capacity(6);
            for &b in &rnd {
                suffix.push(ALPH[(b as usize) % ALPH.len()] as char);
            }
            let full_path = format!("{}{}", prefix, suffix);
            if !check_boa_fs_write_ocap(&full_path) {
                return Ok(BoaVal::from(js_string!("")));
            }
            let norm = norm_vfs_path(&full_path);
            BOA_VFS_DIRS.with(|d| {
                d.borrow_mut().insert(norm.clone());
            });
            Ok(BoaVal::from(js_string!(norm)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_chmod_sync"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let _ = check_boa_fs_write_ocap(&path);
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_symlink_sync"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let target = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let link_path = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_write_ocap(&link_path) {
                return Ok(BoaVal::undefined());
            }
            let norm_link = norm_vfs_path(&link_path);
            let norm_target = norm_vfs_path(&target);
            if let Some(idx) = norm_link.rfind('/') {
                let parent = &norm_link[..idx];
                BOA_VFS_DIRS.with(|d| {
                    d.borrow_mut().insert(parent.to_string());
                });
            }
            let mut payload = Vec::with_capacity(VFS_SYMLINK_PREFIX.len() + norm_target.len());
            payload.extend_from_slice(VFS_SYMLINK_PREFIX);
            payload.extend_from_slice(norm_target.as_bytes());
            BOA_VFS_CACHE.with(|c| {
                c.borrow_mut()
                    .insert(norm_link, std::sync::Arc::new(payload));
            });
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_is_symlink_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_read_ocap(&path) {
                return Ok(BoaVal::from(false));
            }
            let norm = norm_vfs_path(&path);
            let is_sym = BOA_VFS_CACHE
                .with(|c| {
                    c.borrow()
                        .get(&norm)
                        .map(|b| b.starts_with(VFS_SYMLINK_PREFIX))
                })
                .unwrap_or_else(|| {
                    std::fs::symlink_metadata(&path)
                        .map(|m| m.file_type().is_symlink())
                        .unwrap_or(false)
                });
            Ok(BoaVal::from(is_sym))
        }),
    );
}

pub(crate) fn register_buffer_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsValue as BoaVal, NativeFunction};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_bytes_to_hex"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_bytes_to_hex"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::from(js_string!("")));
            };
            let bytes = extract_buffer_source_bytes(first, context)?;
            Ok(BoaVal::from(js_string!(bytes_to_hex(&bytes))))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_base64_decode_bytes"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let s = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|js| js.to_std_string_escaped())
                .unwrap_or_default();
            let decoded = base64_decode(&s);
            bytes_to_js_uint8_array(&decoded, context)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_base64_encode_bytes"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::from(js_string!("")));
            };
            let url_safe = args.get(1).and_then(|v| v.as_boolean()).unwrap_or(false);
            let bytes = extract_buffer_source_bytes(first, context)?;
            Ok(BoaVal::from(js_string!(base64_encode(&bytes, url_safe))))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_hex_decode"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let s = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|js| js.to_std_string_escaped())
                .unwrap_or_default();
            let decoded = hex_decode_bytes(&s);
            bytes_to_js_uint8_array(&decoded, context)
        }),
    );
}

pub(crate) fn register_webcrypto_host_functions(ctx: &mut boa_engine::Context) {
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
            let bytes = extract_buffer_source_bytes(data_arg, context)?;
            let digest: Vec<u8> = match normalize_webcrypto_hash_name(&algo) {
                Some("SHA-1") => sha1_bytes(&bytes).to_vec(),
                Some("SHA-256") => sha256_bytes(&bytes).to_vec(),
                Some("SHA-384") => sha384_bytes(&bytes).to_vec(),
                Some("SHA-512") => sha512_bytes(&bytes).to_vec(),
                _ => {
                    return Err(JsNativeError::error()
                        .with_message(format!(
                            "NotSupportedError: Unsupported digest algorithm '{}'",
                            algo
                        ))
                        .into())
                }
            };
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
            let data_bytes = extract_buffer_source_bytes(data_arg, context)?;
            if handle_id != 0 {
                let tenant = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
                if let Some(Some(sig_bytes)) =
                    with_cached_crypto_key(handle_id, &tenant, |mat| match mat {
                        CachedCryptoKeyMaterial::Hmac { ring_key, .. } => {
                            let tag = ring::hmac::sign(ring_key, &data_bytes);
                            Some(tag.as_ref().to_vec())
                        }
                        _ => None,
                    })
                {
                    return bytes_to_js_array_buffer(&sig_bytes, context);
                }
            }
            let key_bytes = extract_buffer_source_bytes(key_arg, context)?;
            let Some(mac) = compute_webcrypto_hmac(&hash_name, &key_bytes, &data_bytes) else {
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
            let sig_bytes = extract_buffer_source_bytes(sig_arg, context)?;
            let data_bytes = extract_buffer_source_bytes(data_arg, context)?;
            if handle_id != 0 {
                let tenant = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
                if let Some(Some(ok)) =
                    with_cached_crypto_key(handle_id, &tenant, |mat| match mat {
                        CachedCryptoKeyMaterial::Hmac { ring_key, .. } => {
                            Some(ring::hmac::verify(ring_key, &data_bytes, &sig_bytes).is_ok())
                        }
                        _ => None,
                    })
                {
                    return Ok(BoaVal::from(ok));
                }
            }
            let key_bytes = extract_buffer_source_bytes(key_arg, context)?;
            let Some(expected) = compute_webcrypto_hmac(&hash_name, &key_bytes, &data_bytes) else {
                return Err(JsNativeError::error()
                    .with_message(format!(
                        "NotSupportedError: Unsupported HMAC hash '{}'",
                        hash_name
                    ))
                    .into());
            };
            Ok(BoaVal::from(constant_time_eq(&expected, &sig_bytes)))
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
            let a_bytes = extract_buffer_source_bytes(a_arg, context)?;
            let b_bytes = extract_buffer_source_bytes(b_arg, context)?;
            if a_bytes.len() != b_bytes.len() {
                return Err(JsNativeError::range()
                    .with_message("Input buffers must have the same byte length")
                    .into());
            }
            Ok(BoaVal::from(constant_time_eq(&a_bytes, &b_bytes)))
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
            let bytes = extract_buffer_source_bytes(data_arg, context)?;
            let digest: Vec<u8> = match normalize_webcrypto_hash_name(&algo) {
                Some("SHA-1") => sha1_bytes(&bytes).to_vec(),
                Some("SHA-256") => sha256_bytes(&bytes).to_vec(),
                Some("SHA-384") => sha384_bytes(&bytes).to_vec(),
                Some("SHA-512") => sha512_bytes(&bytes).to_vec(),
                Some("MD5") => md5_bytes(&bytes).to_vec(),
                _ => {
                    return Err(JsNativeError::error()
                        .with_message(format!("Unsupported hash algorithm '{}'", algo))
                        .into())
                }
            };
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
            let key_bytes = extract_buffer_source_bytes(key_arg, context)?;
            let data_bytes = extract_buffer_source_bytes(data_arg, context)?;
            let Some(mac) = compute_webcrypto_hmac(&algo, &key_bytes, &data_bytes) else {
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
                let less_safe =
                    make_ring_aead_key(&raw).map_err(|msg| JsNativeError::error().with_message(msg))?;
                CachedCryptoKeyMaterial::AesGcm { less_safe, raw }
            } else if kind.eq_ignore_ascii_case("AES-CBC") {
                let expanded =
                    AesExpandedKey::new(&raw).map_err(|msg| JsNativeError::error().with_message(msg))?;
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
                        CachedCryptoKeyMaterial::AesGcm { less_safe, .. } => {
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
                            aes_gcm_encrypt(&key_bytes, &iv_bytes, &aad_bytes, &data_bytes, tag_bits)
                        } else {
                            aes_gcm_decrypt(&key_bytes, &iv_bytes, &aad_bytes, &data_bytes, tag_bits)
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
                .with_message(format!("NotSupportedError: Unsupported keygen algorithm '{}'", alg))
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
                    let jwk: serde_json::Value = serde_json::from_str(&jwk_str)
                        .map_err(|_| JsNativeError::error().with_message("DataError: Invalid JWK JSON"))?;
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
                    let jwk: serde_json::Value = serde_json::from_str(&jwk_str)
                        .map_err(|_| JsNativeError::error().with_message("DataError: Invalid JWK JSON"))?;
                    let x_bytes = base64_decode(jwk.get("x").and_then(|v| v.as_str()).unwrap_or(""));
                    let y_bytes = base64_decode(jwk.get("y").and_then(|v| v.as_str()).unwrap_or(""));
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

            Err(JsNativeError::error()
                .with_message(format!("NotSupportedError: Unsupported import algorithm '{}'", alg))
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

            Err(JsNativeError::error()
                .with_message(format!("NotSupportedError: Unsupported sign algorithm '{}'", alg))
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
                        } => Some(ecdsa_verify_bytes(k_curve, pub_bytes, &msg_bytes, &sig_bytes)),
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

            Ok(BoaVal::from(false))
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
            let iterations_or_n = args
                .get(5)
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as usize)
                .unwrap_or(1);
            let byte_len = args
                .get(6)
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as usize)
                .unwrap_or(32);
            if byte_len > 1_048_576 {
                return Err(JsNativeError::range()
                    .with_message("Derived key length exceeds 1MB sandbox limit")
                    .into());
            }

            if alg.eq_ignore_ascii_case("PBKDF2") {
                if iterations_or_n == 0 {
                    return Err(JsNativeError::error()
                        .with_message("OperationError: PBKDF2 iterations must be greater than 0")
                        .into());
                }
                let fuel_cost = ((iterations_or_n as u64) / 100).max(10);
                if let Err(msg) = consume_crypto_kdf_fuel(fuel_cost) {
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                let derived = pbkdf2_derive_bytes(
                    &hash,
                    &secret,
                    &salt,
                    iterations_or_n as u32,
                    byte_len,
                )
                .map_err(|msg| JsNativeError::error().with_message(msg))?;
                return bytes_to_js_array_buffer(&derived, context);
            }

            if alg.eq_ignore_ascii_case("HKDF") {
                if let Err(msg) = consume_crypto_kdf_fuel(15) {
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                let derived = hkdf_derive_bytes(&hash, &secret, &salt, &info, byte_len)
                    .map_err(|msg| JsNativeError::error().with_message(msg))?;
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
                .map_err(|msg| JsNativeError::error().with_message(msg))?;
                return bytes_to_js_array_buffer(&derived, context);
            }

            Err(JsNativeError::error()
                .with_message(format!("NotSupportedError: Unsupported KDF '{}'", alg))
                .into())
        }),
    );
}

pub(crate) fn register_shard7_websocket_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsNativeError, JsValue as BoaVal, NativeFunction};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_ws_connect"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_ws_mock"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let url = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let spec = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "echo".to_string());
            BOA_WS_MOCKS.with(|m| {
                m.borrow_mut().insert(url, spec);
            });
            Ok(BoaVal::from(true))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_ws_connect"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let url = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let protocols = args
                .get(1)
                .filter(|v| !v.is_undefined() && !v.is_null())
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            match ws_client_connect(&url, &protocols) {
                Ok(json) => Ok(BoaVal::from(js_string!(json))),
                Err(e) => Err(JsNativeError::error().with_message(e).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_ws_send"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let is_binary = args.get(1).and_then(|v| v.as_boolean()).unwrap_or(false);
            let data = args
                .get(2)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            match ws_client_send(id, is_binary, &data) {
                Ok(json) => Ok(BoaVal::from(js_string!(json))),
                Err(e) => Err(JsNativeError::error().with_message(e).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_ws_poll"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            match ws_client_poll(id) {
                Ok(json) => Ok(BoaVal::from(js_string!(json))),
                Err(e) => Err(JsNativeError::error().with_message(e).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_ws_close"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let code = args
                .get(1)
                .and_then(|v| v.as_number())
                .map(|n| n as u16)
                .unwrap_or(1000);
            let reason = args
                .get(2)
                .filter(|v| !v.is_undefined() && !v.is_null())
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            match ws_client_close(id, code, &reason) {
                Ok(json) => Ok(BoaVal::from(js_string!(json))),
                Err(e) => Err(JsNativeError::error().with_message(e).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_ws_server_send"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let is_binary = args.first().and_then(|v| v.as_boolean()).unwrap_or(false);
            let data = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            match ws_server_queue_send(is_binary, &data) {
                Ok(()) => Ok(BoaVal::from(true)),
                Err(e) => Err(JsNativeError::error().with_message(e).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_ws_server_close"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let code = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u16)
                .unwrap_or(1000);
            let reason = args
                .get(1)
                .filter(|v| !v.is_undefined() && !v.is_null())
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            match ws_server_queue_close(code, &reason) {
                Ok(()) => Ok(BoaVal::from(true)),
                Err(e) => Err(JsNativeError::error().with_message(e).into()),
            }
        }),
    );
}

pub(crate) fn register_shard8_workers_sockets_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsNativeError, JsValue as BoaVal, NativeFunction};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_tcp_connect"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_socket_mock"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let target = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let spec = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "echo".to_string());
            register_socket_mock(&target, &spec);
            Ok(BoaVal::from(true))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_tcp_connect"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let host = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "127.0.0.1".to_string());
            let port = args
                .get(1)
                .and_then(|v| v.as_number())
                .map(|n| n as u16)
                .unwrap_or(0);
            let tls_mode = args
                .get(2)
                .filter(|v| !v.is_undefined() && !v.is_null())
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "off".to_string());
            match tcp_socket_connect(&host, port, &tls_mode) {
                Ok(json) => Ok(BoaVal::from(js_string!(json))),
                Err(e) => Err(JsNativeError::error().with_message(e).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_tcp_write"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            if let Some(is_b64) = args.get(1).and_then(|v| v.as_boolean()) {
                let data = args
                    .get(2)
                    .map(|v| v.to_string(context))
                    .transpose()?
                    .map(|s| s.to_std_string_escaped())
                    .unwrap_or_default();
                match tcp_socket_write(id, is_b64, &data) {
                    Ok(json) => Ok(BoaVal::from(js_string!(json))),
                    Err(e) => Err(JsNativeError::error().with_message(e).into()),
                }
            } else {
                let bytes = match args.get(1) {
                    Some(v) => extract_buffer_source_bytes(v, context)?,
                    None => Vec::new(),
                };
                match tcp_socket_write_bytes(id, &bytes) {
                    Ok(json) => Ok(BoaVal::from(js_string!(json))),
                    Err(e) => Err(JsNativeError::error().with_message(e).into()),
                }
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_tcp_read"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let max_bytes = args
                .get(1)
                .and_then(|v| v.as_number())
                .map(|n| n.max(1.0) as usize)
                .unwrap_or(65_536);
            let timeout_ms = args
                .get(2)
                .and_then(|v| v.as_number())
                .map(|n| n.max(1.0) as u64)
                .unwrap_or(200);
            match tcp_socket_read(id, max_bytes, timeout_ms) {
                Ok(json) => Ok(BoaVal::from(js_string!(json))),
                Err(e) => Err(JsNativeError::error().with_message(e).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_tcp_starttls"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let sni = args
                .get(1)
                .filter(|v| !v.is_undefined() && !v.is_null())
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped());
            match tcp_socket_starttls(id, sni.as_deref()) {
                Ok(json) => Ok(BoaVal::from(js_string!(json))),
                Err(e) => Err(JsNativeError::error().with_message(e).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_tcp_close"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let half_close = args.get(1).and_then(|v| v.as_boolean()).unwrap_or(false);
            match tcp_socket_close(id, half_close) {
                Ok(json) => Ok(BoaVal::from(js_string!(json))),
                Err(e) => Err(JsNativeError::error().with_message(e).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_udp_bind"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let fam = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "udp4".to_string());
            let addr = args
                .get(1)
                .filter(|v| !v.is_undefined() && !v.is_null())
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let port = args
                .get(2)
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as u16)
                .unwrap_or(0);
            match udp_socket_bind(&fam, &addr, port) {
                Ok(json) => Ok(BoaVal::from(js_string!(json))),
                Err(e) => Err(JsNativeError::error().with_message(e).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_udp_send"),
        5,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            if let Some(is_b64) = args.get(1).and_then(|v| v.as_boolean()) {
                let data = args
                    .get(2)
                    .map(|v| v.to_string(context))
                    .transpose()?
                    .map(|s| s.to_std_string_escaped())
                    .unwrap_or_default();
                let host = args
                    .get(3)
                    .map(|v| v.to_string(context))
                    .transpose()?
                    .map(|s| s.to_std_string_escaped())
                    .unwrap_or_else(|| "127.0.0.1".to_string());
                let port = args
                    .get(4)
                    .and_then(|v| v.as_number())
                    .map(|n| n as u16)
                    .unwrap_or(0);
                match udp_socket_send(id, is_b64, &data, &host, port) {
                    Ok(json) => Ok(BoaVal::from(js_string!(json))),
                    Err(e) => Err(JsNativeError::error().with_message(e).into()),
                }
            } else {
                let bytes = match args.get(1) {
                    Some(v) => extract_buffer_source_bytes(v, context)?,
                    None => Vec::new(),
                };
                let host = args
                    .get(2)
                    .map(|v| v.to_string(context))
                    .transpose()?
                    .map(|s| s.to_std_string_escaped())
                    .unwrap_or_else(|| "127.0.0.1".to_string());
                let port = args
                    .get(3)
                    .and_then(|v| v.as_number())
                    .map(|n| n as u16)
                    .unwrap_or(0);
                match udp_socket_send_bytes(id, &bytes, &host, port) {
                    Ok(json) => Ok(BoaVal::from(js_string!(json))),
                    Err(e) => Err(JsNativeError::error().with_message(e).into()),
                }
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_udp_close"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            match udp_socket_close(id) {
                Ok(json) => Ok(BoaVal::from(js_string!(json))),
                Err(e) => Err(JsNativeError::error().with_message(e).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_net_is_ip"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(v) = args.first() else {
                return Ok(BoaVal::from(0));
            };
            if !v.is_string() {
                return Ok(BoaVal::from(0));
            }
            let s = v.to_string(context)?.to_std_string_escaped();
            Ok(BoaVal::from(classify_ip_literal(&s) as f64))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_worker_spawn"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let spec = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let opts_json = args
                .get(1)
                .filter(|v| !v.is_undefined() && !v.is_null())
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "{}".to_string());
            match worker_spawn(&spec, &opts_json) {
                Ok(id) => Ok(BoaVal::from(id as f64)),
                Err(e) => Err(JsNativeError::error().with_message(e).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_worker_post_to_child"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let msg = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "null".to_string());
            match worker_post_message_to_child(id, &msg) {
                Ok(ok) => Ok(BoaVal::from(ok)),
                Err(e) => Err(JsNativeError::error().with_message(e).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_worker_terminate_child"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            Ok(BoaVal::from(worker_terminate_child(id)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_worker_set_ref"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let refed = args.get(1).and_then(|v| v.as_boolean()).unwrap_or(true);
            worker_set_child_ref(id, refed);
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_worker_self_post"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let msg = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "null".to_string());
            match worker_self_post_message(&msg) {
                Ok(ok) => Ok(BoaVal::from(ok)),
                Err(e) => Err(JsNativeError::error().with_message(e).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_worker_self_meta"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let json = worker_self_meta_json();
            Ok(BoaVal::from(js_string!(json)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_worker_self_set_listening"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let listening = args.first().and_then(|v| v.as_boolean()).unwrap_or(false);
            let exit_code = args
                .get(1)
                .filter(|v| !v.is_undefined() && !v.is_null())
                .and_then(|v| v.as_number())
                .map(|n| n as i32);
            worker_self_set_listening(listening, exit_code);
            Ok(BoaVal::undefined())
        }),
    );
}

pub(crate) fn register_shard9_wasm_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsNativeError, JsValue as BoaVal, NativeFunction};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_wasm_compile"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_validate"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::from(false));
            };
            let Ok(bytes) = extract_buffer_source_bytes(first, context) else {
                return Ok(BoaVal::from(false));
            };
            Ok(BoaVal::from(wasm_validate_bytes(&bytes)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_compile"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(first) = args.first() else {
                return Err(JsNativeError::typ()
                    .with_message("TypeError: WebAssembly.Module(): Argument 0 is required")
                    .into());
            };
            let bytes = extract_buffer_source_bytes(first, context)?;
            match wasm_compile_module(&bytes) {
                Ok(mod_id) => Ok(BoaVal::from(mod_id as f64)),
                Err(msg) => Err(JsNativeError::error().with_message(msg).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_module_exports"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let mod_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            match wasm_module_exports_json(mod_id) {
                Ok(json) => Ok(BoaVal::from(js_string!(json))),
                Err(msg) => Err(JsNativeError::error().with_message(msg).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_module_imports"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let mod_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            match wasm_module_imports_json(mod_id) {
                Ok(json) => Ok(BoaVal::from(js_string!(json))),
                Err(msg) => Err(JsNativeError::error().with_message(msg).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_module_custom_sections"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let mod_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let sec_name = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            match wasm_module_custom_sections(mod_id, &sec_name) {
                Ok(sections) => {
                    let mut items = Vec::with_capacity(sections.len());
                    for sec in &sections {
                        let ab = bytes_to_js_array_buffer(sec, context)?;
                        items.push(ab);
                    }
                    let arr = boa_engine::object::builtins::JsArray::from_iter(items, context);
                    Ok(arr.into())
                }
                Err(msg) => Err(JsNativeError::error().with_message(msg).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_memory_create"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let init_pages = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as u32)
                .unwrap_or(0);
            let max_pages = args
                .get(1)
                .filter(|v| !v.is_undefined() && !v.is_null())
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as u32);
            match wasm_create_memory(init_pages, max_pages) {
                Ok(mem_id) => Ok(BoaVal::from(mem_id as f64)),
                Err(msg) => Err(JsNativeError::error().with_message(msg).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_memory_grow"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let mem_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let inst_id = args
                .get(1)
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let delta = args
                .get(2)
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as u32)
                .unwrap_or(0);
            match wasm_grow_memory(mem_id, inst_id, delta) {
                Ok(prev_pages) => Ok(BoaVal::from(prev_pages as f64)),
                Err(msg) => Err(JsNativeError::error().with_message(msg).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_instantiate"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let mod_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let imp_obj = args
                .get(1)
                .filter(|v| !v.is_undefined() && !v.is_null())
                .and_then(|v| v.as_object());
            wasm_instantiate_module(mod_id, imp_obj.as_ref(), context)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_sync_init_mem"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let inst_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let Some(ab_obj) = args.get(1).and_then(|v| v.as_object()) else {
                return Ok(BoaVal::undefined());
            };
            match wasm_sync_initial_memory_to_ab(inst_id, &ab_obj) {
                Ok(()) => Ok(BoaVal::undefined()),
                Err(msg) => Err(JsNativeError::error().with_message(msg).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_call"),
        4,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let inst_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let exp_name = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let mut call_args = Vec::new();
            if let Some(arr_obj) = args.get(2).and_then(|v| v.as_object()) {
                let len = arr_obj
                    .get(js_string!("length"), context)
                    .ok()
                    .and_then(|v| v.as_number())
                    .map(|n| n.max(0.0) as usize)
                    .unwrap_or(0);
                call_args.reserve(len);
                for i in 0..len {
                    let elem = arr_obj
                        .get(i as u32, context)
                        .unwrap_or_else(|_| BoaVal::undefined());
                    call_args.push(elem);
                }
            }
            let mem_ab_obj = args
                .get(3)
                .filter(|v| !v.is_undefined() && !v.is_null())
                .and_then(|v| v.as_object());
            wasm_call_export(inst_id, &exp_name, &call_args, mem_ab_obj.as_ref(), context)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_global_get"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let inst_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let exp_name = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            wasm_get_global(inst_id, &exp_name)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_global_set"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let inst_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let exp_name = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let val = args.get(2).cloned().unwrap_or_else(BoaVal::undefined);
            wasm_set_global(inst_id, &exp_name, &val, context)?;
            Ok(BoaVal::undefined())
        }),
    );
}

#[derive(Default, serde::Deserialize)]
struct ChildProcessSpawnOpts {
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: Option<std::collections::BTreeMap<String, serde_json::Value>>,
    #[serde(default)]
    input: Option<String>,
    #[serde(default)]
    timeout: Option<u64>,
    #[serde(default, rename = "maxBuffer")]
    max_buffer: Option<usize>,
    #[serde(default)]
    shell: Option<serde_json::Value>,
}

fn execute_sanitized_subprocess_sync(
    file: &str,
    args: &[String],
    child_env: &std::collections::BTreeMap<String, String>,
    cwd: Option<&str>,
    stdin_input: Option<&str>,
    timeout_ms: u64,
    max_buffer: usize,
) -> Result<(i32, Vec<u8>, Vec<u8>, Option<String>), String> {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let norm_file = file.trim().replace('\\', "/");
    let base_raw = norm_file.rsplit('/').next().unwrap_or(norm_file.as_str());
    let base_cmd = base_raw
        .strip_suffix(".exe")
        .or_else(|| base_raw.strip_suffix(".EXE"))
        .unwrap_or(base_raw)
        .to_ascii_lowercase();

    // Portable built-in handling for standard POSIX utilities (`echo`, `env`, `printenv`, `true`, `false`)
    // after Ring 1 (`ExecCap`) and Ring 2 (`BOA_ENV_MAP` sanitization) have verified authority.
    match base_cmd.as_str() {
        "echo" => {
            let mut out = String::new();
            let (no_newline, slice) = if args.first().map(|s| s.as_str()) == Some("-n") {
                (true, &args[1..])
            } else {
                (false, args)
            };
            for (i, a) in slice.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                out.push_str(a);
            }
            if !no_newline {
                out.push('\n');
            }
            let out_bytes = out.into_bytes();
            if out_bytes.len() > max_buffer {
                return Err(
                    "RangeError [ERR_CHILD_PROCESS_STDIO_MAXBUFFER]: stdout maxBuffer length exceeded"
                        .to_string(),
                );
            }
            return Ok((0, out_bytes, Vec::new(), None));
        }
        "env" if args.is_empty() => {
            let mut out = String::new();
            for (k, v) in child_env {
                out.push_str(k);
                out.push('=');
                out.push_str(v);
                out.push('\n');
            }
            let out_bytes = out.into_bytes();
            if out_bytes.len() > max_buffer {
                return Err(
                    "RangeError [ERR_CHILD_PROCESS_STDIO_MAXBUFFER]: stdout maxBuffer length exceeded"
                        .to_string(),
                );
            }
            return Ok((0, out_bytes, Vec::new(), None));
        }
        "printenv" => {
            if args.is_empty() {
                let mut out = String::new();
                for (k, v) in child_env {
                    out.push_str(k);
                    out.push('=');
                    out.push_str(v);
                    out.push('\n');
                }
                let out_bytes = out.into_bytes();
                if out_bytes.len() > max_buffer {
                    return Err(
                        "RangeError [ERR_CHILD_PROCESS_STDIO_MAXBUFFER]: stdout maxBuffer length exceeded"
                            .to_string(),
                    );
                }
                return Ok((0, out_bytes, Vec::new(), None));
            }
            let key = &args[0];
            if let Some(val) = child_env.get(key) {
                let out_bytes = format!("{}\n", val).into_bytes();
                if out_bytes.len() > max_buffer {
                    return Err(
                        "RangeError [ERR_CHILD_PROCESS_STDIO_MAXBUFFER]: stdout maxBuffer length exceeded"
                            .to_string(),
                    );
                }
                return Ok((0, out_bytes, Vec::new(), None));
            }
            return Ok((1, Vec::new(), Vec::new(), None));
        }
        "true" => return Ok((0, Vec::new(), Vec::new(), None)),
        "false" => return Ok((1, Vec::new(), Vec::new(), None)),
        _ => {}
    }

    let mut cmd = Command::new(file);
    cmd.args(args);
    // Ring 2: Strictly clear host ambient environment and inject ONLY tenant-attenuated variables
    cmd.env_clear();
    cmd.envs(child_env);
    if let Some(dir) = cwd {
        if !dir.is_empty() && std::path::Path::new(dir).is_dir() {
            cmd.current_dir(dir);
        }
    }
    cmd.stdin(if stdin_input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Error: spawn {} ENOENT ({})", file, e))?;

    if let Some(input_data) = stdin_input {
        if let Some(mut stdin_pipe) = child.stdin.take() {
            let _ = stdin_pipe.write_all(input_data.as_bytes());
        }
    }

    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let stdout_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(ref mut p) = stdout_pipe {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });
    let stderr_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(ref mut p) = stderr_pipe {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });

    let deadline = Instant::now() + Duration::from_millis(timeout_ms.max(1));
    let mut timed_out = false;
    let exit_code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(1),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break 1;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(e) => {
                return Err(format!("Error waiting for subprocess {}: {}", file, e));
            }
        }
    };

    let stdout_bytes = stdout_handle.join().unwrap_or_default();
    let stderr_bytes = stderr_handle.join().unwrap_or_default();

    if timed_out {
        return Err(format!(
            "Error: spawnSync {} ETIMEDOUT (exceeded {} ms)",
            file, timeout_ms
        ));
    }

    // Ring 4: Output byte ceiling (`maxBuffer`)
    if stdout_bytes.len().saturating_add(stderr_bytes.len()) > max_buffer {
        return Err(
            "RangeError [ERR_CHILD_PROCESS_STDIO_MAXBUFFER]: stdout/stderr maxBuffer length exceeded"
                .to_string(),
        );
    }

    Ok((exit_code, stdout_bytes, stderr_bytes, None))
}

pub(crate) fn register_child_process_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsNativeError, JsValue as BoaVal, NativeFunction};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_child_process_spawn_sync"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_child_process_spawn_sync"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let file = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let args_json = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "[]".to_string());
            let opts_json = args
                .get(2)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "{}".to_string());

            let cmd_args: Vec<String> = serde_json::from_str(&args_json).unwrap_or_default();
            let opts: ChildProcessSpawnOpts = serde_json::from_str(&opts_json).unwrap_or_default();

            let tenant_id = BOA_ACTIVE_TENANT.with(|t| {
                let b = t.borrow();
                if b.is_empty() {
                    "boa-tenant".to_string()
                } else {
                    b.clone()
                }
            });

            // Ring 1: OCap `ExecCap` Allowlist Verification (Deny-All by default)
            let allowed_bins = BOA_EXEC_BINS.with(|eb| eb.borrow().clone());
            let is_allowed = !file.trim().is_empty()
                && allowed_bins
                    .iter()
                    .any(|allowed| slice_sandbox::exec_binary_matches(&file, allowed));
            if !is_allowed {
                let violation = SandboxViolation::ExecDenied {
                    tenant_id,
                    binary: file,
                };
                let msg = violation.to_string();
                BOA_LAST_CHILD_PROC_VIOLATION.with(|v| {
                    *v.borrow_mut() = Some(violation);
                });
                return Err(JsNativeError::error().with_message(msg).into());
            }

            // If `shell` option is explicitly requested as a string path or boolean `true`,
            // also verify that the shell binary itself is authorized in `allowed_exec_binaries`.
            if let Some(ref sh_val) = opts.shell {
                let shell_bin = match sh_val {
                    serde_json::Value::Bool(true) => Some("/bin/sh"),
                    serde_json::Value::String(s) if !s.is_empty() => Some(s.as_str()),
                    _ => None,
                };
                if let Some(sb) = shell_bin {
                    let shell_allowed = allowed_bins
                        .iter()
                        .any(|allowed| slice_sandbox::exec_binary_matches(sb, allowed));
                    if !shell_allowed {
                        let violation = SandboxViolation::ExecDenied {
                            tenant_id,
                            binary: sb.to_string(),
                        };
                        let msg = violation.to_string();
                        BOA_LAST_CHILD_PROC_VIOLATION.with(|v| {
                            *v.borrow_mut() = Some(violation);
                        });
                        return Err(JsNativeError::error().with_message(msg).into());
                    }
                }
            }

            // Ring 5: Base CPU Fuel charge for subprocess invocation
            if let Err(fuel_err) = consume_crypto_kdf_fuel(500) {
                return Err(JsNativeError::range().with_message(fuel_err).into());
            }

            // Ring 2: Environment Sanitization — construct child environment strictly from
            // the tenant's attenuated `BOA_ENV_MAP` (never host `std::env::vars()`).
            let tenant_env = BOA_ENV_MAP.with(|m| m.borrow().clone());
            let mut child_env = std::collections::BTreeMap::new();
            if let Some(custom_env) = opts.env {
                for (k, v) in custom_env {
                    match v {
                        serde_json::Value::String(s) => {
                            child_env.insert(k, s);
                        }
                        serde_json::Value::Number(n) => {
                            child_env.insert(k, n.to_string());
                        }
                        serde_json::Value::Bool(b) => {
                            child_env.insert(k, b.to_string());
                        }
                        _ => {}
                    }
                }
            } else {
                for (k, v) in &tenant_env {
                    child_env.insert(k.clone(), v.clone());
                }
            }

            let timeout_ms = opts.timeout.unwrap_or(30_000).min(120_000);
            let max_buffer = opts.max_buffer.unwrap_or(1024 * 1024).min(16 * 1024 * 1024);

            let (status_code, stdout_bytes, stderr_bytes, signal) =
                match execute_sanitized_subprocess_sync(
                    &file,
                    &cmd_args,
                    &child_env,
                    opts.cwd.as_deref(),
                    opts.input.as_deref(),
                    timeout_ms,
                    max_buffer,
                ) {
                    Ok(res) => res,
                    Err(err_msg) => {
                        return Err(JsNativeError::error().with_message(err_msg).into());
                    }
                };

            // Ring 5: Charge fuel & memory ceiling proportional to child stdio output
            let total_out_bytes = stdout_bytes.len().saturating_add(stderr_bytes.len());
            if total_out_bytes > 0 {
                let extra_fuel = (total_out_bytes as u64 / 64).max(1);
                if let Err(fuel_err) = consume_crypto_kdf_fuel(extra_fuel) {
                    return Err(JsNativeError::range().with_message(fuel_err).into());
                }
                let (used_mem, limit_mem) = BOA_KV_MEM_BUDGET.with(|m| m.get());
                let next_mem = used_mem.saturating_add(total_out_bytes);
                if next_mem > limit_mem {
                    let violation = SandboxViolation::MemoryCeilingExceeded {
                        tenant_id: tenant_id.clone(),
                        requested: next_mem,
                        limit: limit_mem,
                    };
                    let msg = violation.to_string();
                    BOA_VIOLATION.with(|v| {
                        if v.borrow().is_none() {
                            *v.borrow_mut() = Some(violation);
                        }
                    });
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                BOA_KV_MEM_BUDGET.with(|m| m.set((next_mem, limit_mem)));
            }

            let stdout_str = String::from_utf8_lossy(&stdout_bytes).into_owned();
            let stderr_str = String::from_utf8_lossy(&stderr_bytes).into_owned();

            // Ring 3: Output Egress DLP — scan both `stdout` and `stderr` against the tenant's
            // Egress DLP automaton (`BOA_EGRESS_SECRETS`, including `.env` Auto-DLP secrets).
            let egress_secrets = BOA_EGRESS_SECRETS.with(|s| s.borrow().clone());
            if !egress_secrets.is_empty() {
                if let Err(violation) =
                    verify_egress_secrets_slice(&tenant_id, &egress_secrets, &stdout_str)
                        .and_then(|()| {
                            verify_egress_secrets_slice(&tenant_id, &egress_secrets, &stderr_str)
                        })
                {
                    let msg = violation.to_string();
                    BOA_LAST_CHILD_PROC_VIOLATION.with(|v| {
                        *v.borrow_mut() = Some(violation);
                    });
                    return Err(JsNativeError::error().with_message(msg).into());
                }
            }

            let resp = serde_json::json!({
                "status": status_code,
                "signal": signal,
                "stdoutBase64": base64_encode(&stdout_bytes, false),
                "stderrBase64": base64_encode(&stderr_bytes, false),
                "stdout": stdout_str,
                "stderr": stderr_str,
                "pid": 4200,
            });
            Ok(BoaVal::from(js_string!(resp.to_string())))
        }),
    );
}
