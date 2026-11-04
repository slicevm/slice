//! Native Rust Host Callbacks (`__slice_*`) Registered into `boa_engine::Context`.

use crate::boa_realm::{
    check_boa_fs_read_ocap, check_boa_fs_write_ocap, norm_vfs_path, BOA_ACTIVE_CAPS,
    BOA_ACTIVE_TENANT, BOA_DET_STATE, BOA_EGRESS_SECRETS, BOA_ENV_MAP, BOA_FETCH_MOCKS,
    BOA_HTTP_REQ, BOA_HTTP_RES, BOA_KV_MEM_BUDGET, BOA_NET_HOSTS, BOA_STDOUT, BOA_VFS_CACHE,
    BOA_VFS_DIRS, BOA_VIOLATION, BOA_WAIT_UNTIL_COUNT,
};
use crate::event_loop::{
    cancel_timer, clear_event_loop_state, drive_event_loop_to_completion, enqueue_async_fetch,
    restore_event_loop_state, save_and_reset_event_loop_state, schedule_immediate, schedule_timer,
};
use crate::intrinsics::{
    base64_decode, base64_encode, bytes_to_hex, constant_time_eq, fast_format_bytes,
    fast_format_ms, fast_parse_bytes, fast_parse_ms, fill_random_bytes, generate_uuid_v4,
    hex_decode_bytes, hmac_sha1_bytes, hmac_sha256_bytes, hmac_sha256_hex, hmac_sha384_bytes,
    hmac_sha512_bytes, semver_coerce_xyz, semver_parse_strict_xyz, semver_satisfies_xyz,
    sha1_bytes, sha256_bytes, sha256_hex, sha384_bytes, sha512_bytes, whatwg_atob, whatwg_btoa,
};
use crate::isolate::{SandboxEvalOptions, SliceIsolate};
use crate::js_preludes::eval_lazy_prelude_shard;
use crate::node_builtins::eval_lazy_builtin_module;
use crate::outbound_http::{
    check_boa_outbound_fetch_ocap_and_dlp, execute_outbound_fetch_with_redirects,
    parse_outbound_http_url,
};
use crate::state_and_kv::{
    checkpoint_tenant_state, edge_kv_del, edge_kv_get, edge_kv_incr, edge_kv_set,
    restore_tenant_state, splitmix64_next_f64,
};
use crate::websocket::{
    ws_client_close, ws_client_connect, ws_client_poll, ws_client_send, ws_server_queue_close,
    ws_server_queue_send, BOA_WS_MOCKS,
};
use crate::sockets::{
    classify_ip_literal, register_socket_mock, tcp_socket_close, tcp_socket_connect,
    tcp_socket_read, tcp_socket_starttls, tcp_socket_write, tcp_socket_write_bytes,
    udp_socket_bind, udp_socket_close, udp_socket_send, udp_socket_send_bytes,
};
use crate::workers::{
    worker_post_message_to_child, worker_self_meta_json, worker_self_post_message,
    worker_self_set_listening, worker_set_child_ref, worker_spawn, worker_terminate_child,
};
use crate::wasm::{
    wasm_call_export, wasm_compile_module, wasm_create_memory, wasm_get_global, wasm_grow_memory,
    wasm_instantiate_module, wasm_module_custom_sections, wasm_module_exports_json,
    wasm_module_imports_json, wasm_set_global, wasm_sync_initial_memory_to_ab,
    wasm_validate_bytes,
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
        js_string!("__slice_path_join"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let mut out = String::with_capacity(64);
            for (idx, arg) in args.iter().enumerate() {
                let s = arg.to_string(context)?.to_std_string_escaped();
                if s.is_empty() {
                    continue;
                }
                let normalized = if s.contains('\\') {
                    s.replace('\\', "/")
                } else {
                    s
                };
                let mut seg = normalized.as_str();
                if !out.is_empty() {
                    seg = seg.trim_start_matches('/');
                }
                if idx + 1 < args.len() && seg.len() > 1 {
                    seg = seg.trim_end_matches('/');
                }
                if seg.is_empty() {
                    continue;
                }
                if !out.is_empty() && !out.ends_with('/') {
                    out.push('/');
                }
                out.push_str(seg);
            }
            if out.is_empty() {
                out.push('.');
            }
            Ok(BoaVal::from(js_string!(out)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_os_tmpdir"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let tmp = std::env::temp_dir().to_string_lossy().replace('\\', "/");
            Ok(BoaVal::from(js_string!(tmp)))
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
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            if let Some(first) = args.first() {
                if first.is_object() {
                    if let Ok(bytes) = extract_buffer_source_bytes(first, context) {
                        let hex = sha256_hex(&bytes);
                        return Ok(BoaVal::from(js_string!(hex)));
                    }
                }
            }
            let input = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let hex = sha256_hex(input.as_bytes());
            Ok(BoaVal::from(js_string!(hex)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_crypto_hmac_sha256"),
        2,
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
        js_string!("__slice_http_is_active"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
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

            let saved_stdout = BOA_STDOUT.with(|out| std::mem::take(&mut *out.borrow_mut()));
            let saved_violation = BOA_VIOLATION.with(|v| v.borrow_mut().take());
            let saved_tenant = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
            let saved_caps = BOA_ACTIVE_CAPS.with(|c| c.borrow_mut().take());
            let saved_net_hosts = BOA_NET_HOSTS.with(|h| std::mem::take(&mut *h.borrow_mut()));
            let saved_egress_secrets =
                BOA_EGRESS_SECRETS.with(|s| std::mem::take(&mut *s.borrow_mut()));
            let saved_fetch_mocks = BOA_FETCH_MOCKS.with(|m| m.borrow().clone());
            let saved_ws_mocks = BOA_WS_MOCKS.with(|m| m.borrow().clone());
            let saved_sock_mocks = crate::sockets::BOA_SOCKET_MOCKS.with(|m| m.borrow().clone());
            let saved_kv_budget = BOA_KV_MEM_BUDGET.with(|m| m.get());
            let saved_det = BOA_DET_STATE.with(|s| s.get());
            let saved_ev_loop = save_and_reset_event_loop_state();

            let mut parent_caps = CapabilityToken::pure_compute();
            if let Some((_, ref r_prefs, ref w_prefs)) = saved_caps {
                for r in r_prefs {
                    parent_caps = parent_caps.with_read_prefix(r.clone());
                }
                for w in w_prefs {
                    parent_caps = parent_caps.with_write_prefix(w.clone());
                }
            }
            for nh in &saved_net_hosts {
                parent_caps = parent_caps.with_net_host(nh.clone());
            }
            let parent_rem_mem = saved_kv_budget.1.saturating_sub(saved_kv_budget.0);
            let mut proxy_parent = SliceIsolate::spawn(
                saved_tenant.clone(),
                98,
                parent_caps,
                5_000_000,
                parent_rem_mem.max(65536),
            );
            for sec in &saved_egress_secrets {
                proxy_parent.add_egress_audit_secret(sec.clone());
            }
            let outcome = proxy_parent.eval_sandbox(&code, opts);

            restore_event_loop_state(saved_ev_loop);
            BOA_STDOUT.with(|out| *out.borrow_mut() = saved_stdout);
            BOA_VIOLATION.with(|v| *v.borrow_mut() = saved_violation);
            BOA_ACTIVE_TENANT.with(|t| *t.borrow_mut() = saved_tenant);
            BOA_ACTIVE_CAPS.with(|c| *c.borrow_mut() = saved_caps);
            BOA_NET_HOSTS.with(|h| *h.borrow_mut() = saved_net_hosts);
            BOA_EGRESS_SECRETS.with(|s| *s.borrow_mut() = saved_egress_secrets);
            BOA_FETCH_MOCKS.with(|m| *m.borrow_mut() = saved_fetch_mocks);
            BOA_WS_MOCKS.with(|m| *m.borrow_mut() = saved_ws_mocks);
            crate::sockets::BOA_SOCKET_MOCKS.with(|m| *m.borrow_mut() = saved_sock_mocks);
            BOA_KV_MEM_BUDGET.with(|m| m.set(saved_kv_budget));
            BOA_DET_STATE.with(|s| s.set(saved_det));

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
                    let end = len.map(|l| (start + l).min(data.len())).unwrap_or(data.len());
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
        _ => None,
    }
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
            let _ = res_obj.set(js_string!("read"), BoaVal::from(read_utf16 as f64), false, context);
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
            let norm = norm_vfs_path(&path);
            let bytes = BOA_VFS_CACHE
                .with(|c| c.borrow().get(&norm).cloned())
                .unwrap_or_else(|| std::fs::read(&path).unwrap_or_default());
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
            let norm = norm_vfs_path(&path);
            let bytes = BOA_VFS_CACHE
                .with(|c| c.borrow().get(&norm).cloned())
                .unwrap_or_else(|| std::fs::read(&path).unwrap_or_default());
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
                c.borrow_mut().insert(norm, data_bytes);
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
                    .or_insert_with(|| std::fs::read(&path).unwrap_or_default());
                entry.extend_from_slice(&data_bytes);
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
                .unwrap_or_else(|| std::fs::read(&src).unwrap_or_default());
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
                .unwrap_or_else(|| std::fs::read(&old_p).unwrap_or_default());
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
        3,
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
            let key_bytes = extract_buffer_source_bytes(key_arg, context)?;
            let data_bytes = extract_buffer_source_bytes(data_arg, context)?;
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
        4,
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
            let key_bytes = extract_buffer_source_bytes(key_arg, context)?;
            let sig_bytes = extract_buffer_source_bytes(sig_arg, context)?;
            let data_bytes = extract_buffer_source_bytes(data_arg, context)?;
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

