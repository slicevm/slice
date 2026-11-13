//! Core Native Host Bindings (`__slice_console_log`, Stack Sourcemapping, Lazy Builtins/Preludes,
//! `os.platform`/`arch`, Fast SHA-256/HMAC-SHA256, Base64/UTF-8, `process.env`, Event-Loop Timers).

use crate::bindings::helpers::{
    bytes_to_js_uint8_array, consume_crypto_kdf_fuel, try_consume_isolate_memory,
    with_buffer_source_bytes, with_two_buffer_source_bytes,
};
use crate::boa_realm::{
    BOA_ACTIVE_TENANT, BOA_DET_STATE, BOA_ENV_MAP, BOA_HTTP_REQ, BOA_KV_MEM_BUDGET, BOA_STDOUT,
    BOA_VIOLATION, BOA_WAIT_UNTIL_COUNT,
};
use crate::event_loop::{
    cancel_timer, refresh_timer, schedule_immediate, schedule_timer, set_timer_ref,
};
use crate::intrinsics::{
    base64_decode, base64_decode_bytes, base64_decoded_len_bytes, base64_encode, bytes_to_hex,
    hmac_sha256_bytes, hmac_sha256_hex, sha256_bytes, sha256_hex,
};
use crate::js_preludes::eval_lazy_prelude_shard;
use crate::node_builtins::eval_lazy_builtin_module;
use slice_sandbox::SandboxViolation;

pub(crate) fn register_slice_native_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::builtins::typed_array::TypedArray;
    use boa_engine::object::builtins::JsArrayBuffer;
    use boa_engine::{js_string, JsNativeError, JsValue as BoaVal, NativeFunction};

    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_console_log"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    static PERF_ORIGIN: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_perf_now"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let origin = PERF_ORIGIN.get_or_init(std::time::Instant::now);
            let elapsed_ms = origin.elapsed().as_secs_f64() * 1000.0
                + (crate::event_loop::virtual_time_warped_ms() as f64);
            Ok(BoaVal::from(elapsed_ms))
        }),
    );

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
            if let Err(msg) = try_consume_isolate_memory(line.len().max(8)) {
                return Err(JsNativeError::range().with_message(msg).into());
            }
            if let Err(msg) = consume_crypto_kdf_fuel(((line.len() as u64) / 32).saturating_add(2))
            {
                return Err(JsNativeError::range().with_message(msg).into());
            }
            BOA_STDOUT.with(|out| out.borrow_mut().push(line));
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_charge_mem"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let bytes = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as usize)
                .unwrap_or(0);
            let fuel = args
                .get(1)
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as u64)
                .unwrap_or_else(|| ((bytes as u64) / 64).max(1));
            if let Err(msg) = try_consume_isolate_memory(bytes) {
                return Err(JsNativeError::range().with_message(msg).into());
            }
            if let Err(msg) = consume_crypto_kdf_fuel(fuel) {
                return Err(JsNativeError::range().with_message(msg).into());
            }
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_map_error_stack"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let raw = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let mapped = crate::boa_realm::map_stack_with_active_source(&raw);
            Ok(BoaVal::from(js_string!(mapped)))
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
            if shard_id == 18 {
                let pre = crate::boa_realm::proto_sig(
                    &context.intrinsics().constructors().string().prototype(),
                );
                let res = eval_lazy_prelude_shard(shard_id, context)?;
                let post = crate::boa_realm::proto_sig(
                    &context.intrinsics().constructors().string().prototype(),
                );
                crate::boa_realm::BOA_SES_INTL_SIG_PAIR.with(|p| p.set(Some((pre, post))));
                return Ok(res);
            }
            eval_lazy_prelude_shard(shard_id, context)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_capture_ses_sigs"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let mode = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as i32)
                .unwrap_or(0);
            match mode {
                0 => {
                    let sigs = crate::boa_realm::capture_ses_proto_sigs(context);
                    crate::boa_realm::BOA_SES_BASELINE_SIGS.with(|s| s.set(Some(sigs)));
                    Ok(BoaVal::undefined())
                }
                1 => {
                    let cur_str = crate::boa_realm::proto_sig(
                        &context.intrinsics().constructors().string().prototype(),
                    );
                    let matches = crate::boa_realm::BOA_SES_BASELINE_SIGS
                        .with(|s| s.get().is_some_and(|sigs| sigs[3] == cur_str));
                    Ok(BoaVal::from(matches))
                }
                2 => {
                    let cur_str = crate::boa_realm::proto_sig(
                        &context.intrinsics().constructors().string().prototype(),
                    );
                    crate::boa_realm::BOA_SES_BASELINE_SIGS.with(|s| {
                        if let Some(mut sigs) = s.get() {
                            sigs[3] = cur_str;
                            s.set(Some(sigs));
                        }
                    });
                    Ok(BoaVal::undefined())
                }
                _ => Ok(BoaVal::undefined()),
            }
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
                    let encoded = with_buffer_source_bytes(first, context, |bytes| {
                        if enc.eq_ignore_ascii_case("hex") {
                            sha256_hex(bytes)
                        } else {
                            let digest = sha256_bytes(bytes);
                            encode_digest(&digest)
                        }
                    })?;
                    return Ok(BoaVal::from(js_string!(encoded)));
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
            let enc = args
                .get(2)
                .and_then(|v| v.as_string())
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "hex".to_string());
            let Some(key_arg) = args.first() else {
                return Ok(BoaVal::from(js_string!("")));
            };
            let Some(msg_arg) = args.get(1) else {
                return Ok(BoaVal::from(js_string!("")));
            };
            if key_arg.is_object() || msg_arg.is_object() {
                let encoded = with_two_buffer_source_bytes(
                    key_arg,
                    msg_arg,
                    context,
                    |key_bytes, msg_bytes| {
                        if enc.eq_ignore_ascii_case("base64") {
                            let mac = hmac_sha256_bytes(key_bytes, msg_bytes);
                            base64_encode(&mac, false)
                        } else if enc.eq_ignore_ascii_case("base64url") {
                            let mac = hmac_sha256_bytes(key_bytes, msg_bytes);
                            base64_encode(&mac, true)
                        } else {
                            hmac_sha256_hex(key_bytes, msg_bytes)
                        }
                    },
                )?;
                return Ok(BoaVal::from(js_string!(encoded)));
            }
            let key = key_arg.to_string(context)?.to_std_string_escaped();
            let msg = msg_arg.to_string(context)?.to_std_string_escaped();
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
            use boa_engine::string::JsStrVariant;
            let url_safe = args.get(1).and_then(|v| v.as_boolean()).unwrap_or(false);
            let Some(first) = args.first() else {
                return Ok(BoaVal::from(js_string!("")));
            };
            if first.is_object() {
                let encoded = with_buffer_source_bytes(first, context, |bytes| {
                    base64_encode(bytes, url_safe)
                })?;
                return Ok(BoaVal::from(boa_engine::JsString::from(
                    boa_engine::string::JsStr::latin1(encoded.as_bytes()),
                )));
            }
            let js_s = first.to_string(context)?;
            let encoded = match js_s.as_str().variant() {
                JsStrVariant::Latin1(bytes) if bytes.is_ascii() => base64_encode(bytes, url_safe),
                _ => base64_encode(js_s.to_std_string_escaped().as_bytes(), url_safe),
            };
            Ok(BoaVal::from(boa_engine::JsString::from(
                boa_engine::string::JsStr::latin1(encoded.as_bytes()),
            )))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_base64_decode"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            use boa_engine::string::JsStrVariant;
            let Some(first) = args.first() else {
                return Ok(BoaVal::from(js_string!("")));
            };
            let js_s = first.to_string(context)?;
            let decoded = match js_s.as_str().variant() {
                JsStrVariant::Latin1(bytes) => base64_decode_bytes(bytes),
                JsStrVariant::Utf16(_) => base64_decode(&js_s.to_std_string_escaped()),
            };
            if decoded.is_ascii() {
                return Ok(BoaVal::from(boa_engine::JsString::from(
                    boa_engine::string::JsStr::latin1(&decoded),
                )));
            }
            let utf8 = String::from_utf8_lossy(&decoded);
            Ok(BoaVal::from(js_string!(utf8.as_ref())))
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
                let decoded = with_buffer_source_bytes(first, context, |bytes| {
                    if bytes.is_ascii() {
                        BoaVal::from(boa_engine::JsString::from(
                            boa_engine::string::JsStr::latin1(bytes),
                        ))
                    } else {
                        let s = String::from_utf8_lossy(bytes);
                        BoaVal::from(js_string!(s.as_ref()))
                    }
                })?;
                return Ok(decoded);
            }
            let s = first.to_string(context)?.to_std_string_escaped();
            Ok(BoaVal::from(js_string!(s)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_utf8_byte_len"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            use boa_engine::builtins::dataview::DataView;
            let Some(first) = args.first() else {
                return Ok(BoaVal::from(0));
            };
            if let Some(obj) = first.as_object() {
                if let Some(ta) = obj.downcast_ref::<TypedArray>() {
                    if let Some(bl) = ta.fast_byte_length() {
                        return Ok(BoaVal::from(bl as f64));
                    }
                }
                if let Ok(ab) = JsArrayBuffer::from_object(obj.clone()) {
                    let bl = ab.data().map(|d| d.len()).unwrap_or(0);
                    return Ok(BoaVal::from(bl as f64));
                }
                if let Some(dv) = obj.downcast_ref::<DataView>() {
                    if let Some(bl) = dv.with_bytes(|b| b.len()) {
                        return Ok(BoaVal::from(bl as f64));
                    }
                }
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
            let js_s = match first.as_string() {
                Some(s) => s,
                None => first.to_string(context)?,
            };
            // Encoding kind: 0 = utf8, 1 = latin1/ascii/binary, 2 = utf16le/ucs2, 3 = hex, 4 = base64/base64url
            let mut enc_kind: u8 = 0;
            if let Some(enc_val) = args.get(1).filter(|v| !v.is_undefined() && !v.is_null()) {
                let enc_js = match enc_val.as_string() {
                    Some(s) => s,
                    None => enc_val.to_string(context)?,
                };
                if let boa_engine::string::JsStrVariant::Latin1(eb) = enc_js.as_str().variant() {
                    if eb.eq_ignore_ascii_case(b"utf8") || eb.eq_ignore_ascii_case(b"utf-8") {
                        enc_kind = 0;
                    } else if eb.eq_ignore_ascii_case(b"latin1")
                        || eb.eq_ignore_ascii_case(b"binary")
                        || eb.eq_ignore_ascii_case(b"ascii")
                    {
                        enc_kind = 1;
                    } else if eb.eq_ignore_ascii_case(b"utf16le")
                        || eb.eq_ignore_ascii_case(b"utf-16le")
                        || eb.eq_ignore_ascii_case(b"ucs2")
                        || eb.eq_ignore_ascii_case(b"ucs-2")
                    {
                        enc_kind = 2;
                    } else if eb.eq_ignore_ascii_case(b"hex") {
                        enc_kind = 3;
                    } else if eb.eq_ignore_ascii_case(b"base64")
                        || eb.eq_ignore_ascii_case(b"base64url")
                    {
                        enc_kind = 4;
                    }
                } else {
                    let e = enc_js.to_std_string_escaped();
                    if e.eq_ignore_ascii_case("latin1")
                        || e.eq_ignore_ascii_case("binary")
                        || e.eq_ignore_ascii_case("ascii")
                    {
                        enc_kind = 1;
                    } else if e.eq_ignore_ascii_case("utf16le")
                        || e.eq_ignore_ascii_case("utf-16le")
                        || e.eq_ignore_ascii_case("ucs2")
                        || e.eq_ignore_ascii_case("ucs-2")
                    {
                        enc_kind = 2;
                    } else if e.eq_ignore_ascii_case("hex") {
                        enc_kind = 3;
                    } else if e.eq_ignore_ascii_case("base64")
                        || e.eq_ignore_ascii_case("base64url")
                    {
                        enc_kind = 4;
                    }
                }
            }

            let byte_len: usize = match (enc_kind, js_s.as_str().variant()) {
                (0, boa_engine::string::JsStrVariant::Latin1(bytes)) => {
                    // In Latin-1 (`0x00..=0xFF`), `0x00..=0x7F` encodes to 1 UTF-8 byte
                    // and `0x80..=0xFF` encodes to 2 UTF-8 bytes.
                    let high_count: usize = bytes.iter().map(|&b| usize::from(b >> 7)).sum();
                    bytes.len() + high_count
                }
                (0, boa_engine::string::JsStrVariant::Utf16(units)) => {
                    char::decode_utf16(units.iter().copied())
                        .map(|r| match r {
                            Ok(c) => c.len_utf8(),
                            Err(_) => 3,
                        })
                        .sum()
                }
                (1, _) => js_s.len(),
                (2, _) => js_s.len() * 2,
                (3, _) => js_s.len() >> 1,
                (4, boa_engine::string::JsStrVariant::Latin1(bytes)) => {
                    base64_decoded_len_bytes(bytes)
                }
                (4, boa_engine::string::JsStrVariant::Utf16(_)) => {
                    let owned = js_s.to_std_string_escaped();
                    base64_decoded_len_bytes(owned.as_bytes())
                }
                _ => js_s.len(),
            };
            Ok(BoaVal::from(byte_len as f64))
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
        js_string!("__slice_timer_ctl"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as u32)
                .unwrap_or(0);
            let op = args
                .get(1)
                .and_then(|v| v.as_number())
                .map(|n| n as i32)
                .unwrap_or(0);
            match op {
                0 => cancel_timer(id),
                1 => set_timer_ref(id, true),
                2 => set_timer_ref(id, false),
                3 => refresh_timer(id),
                _ => {}
            }
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
            use boa_engine::string::JsStrVariant;
            let Some(first) = args.first() else {
                return bytes_to_js_uint8_array(&[], context);
            };
            if first.is_undefined() {
                return bytes_to_js_uint8_array(&[], context);
            }
            let js_s = match first.as_string() {
                Some(s) => s.clone(),
                None => first.to_string(context)?,
            };
            match js_s.as_str().variant() {
                JsStrVariant::Latin1(bytes) => {
                    if bytes.is_ascii() {
                        return bytes_to_js_uint8_array(bytes, context);
                    }
                    let mut out = Vec::with_capacity(bytes.len() * 2);
                    for &b in bytes {
                        if b < 0x80 {
                            out.push(b);
                        } else {
                            out.push(0xC0 | (b >> 6));
                            out.push(0x80 | (b & 0x3F));
                        }
                    }
                    bytes_to_js_uint8_array(&out, context)
                }
                JsStrVariant::Utf16(units) => {
                    let mut out = Vec::with_capacity(units.len() * 3);
                    for item in char::decode_utf16(units.iter().copied()) {
                        let ch = item.unwrap_or(char::REPLACEMENT_CHARACTER);
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                    }
                    bytes_to_js_uint8_array(&out, context)
                }
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_process_ctl"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let op = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as i32)
                .unwrap_or(0);
            crate::boa_realm::mark_subsystem_dirty(crate::boa_realm::DIRTY_PROCESS);
            if op == 0 {
                return Ok(BoaVal::undefined());
            }
            if op == 1 {
                let custom_opt = crate::boa_realm::BOA_CUSTOM_ARGV.with(|a| a.borrow().clone());
                let arr = if let Some(items) = custom_opt {
                    boa_engine::object::builtins::JsArray::from_iter(
                        items.iter().map(|s| BoaVal::from(js_string!(s.as_str()))),
                        context,
                    )
                } else {
                    boa_engine::object::builtins::JsArray::from_iter(
                        std::iter::once(BoaVal::from(js_string!("slice"))),
                        context,
                    )
                };
                return Ok(BoaVal::from(arr));
            }
            let proc_obj = context
                .global_object()
                .get(js_string!("process"), context)
                .ok()
                .and_then(|v| v.as_object());
            let mut code: i32 = match args.get(1) {
                Some(v) if !v.is_undefined() && !v.is_null() => v.to_i32(context).unwrap_or(0),
                _ => {
                    if let Some(ref p) = proc_obj {
                        p.get(js_string!("exitCode"), context)
                            .ok()
                            .filter(|v| !v.is_undefined() && !v.is_null())
                            .and_then(|v| v.to_i32(context).ok())
                            .unwrap_or(0)
                    } else {
                        0
                    }
                }
            };
            if let Some(ref p) = proc_obj {
                let _ = p.set(js_string!("exitCode"), BoaVal::from(code), false, context);
                let already_exited = p
                    .get(js_string!("_exited"), context)
                    .ok()
                    .and_then(|v| v.as_boolean())
                    .unwrap_or(false);
                if !already_exited {
                    let _ = p.set(js_string!("_exited"), BoaVal::from(true), false, context);
                    if let Ok(emit_val) = p.get(js_string!("emit"), context) {
                        if let Some(emit_fn) = emit_val.as_object() {
                            if emit_fn.is_callable() {
                                let _ = emit_fn.call(
                                    &BoaVal::from(p.clone()),
                                    &[BoaVal::from(js_string!("exit")), BoaVal::from(code)],
                                    context,
                                );
                            }
                        }
                    }
                    if let Some(inner_code) = crate::boa_realm::BOA_PROCESS_EXIT.with(|e| e.get()) {
                        code = inner_code;
                    } else if let Ok(ec_val) = p.get(js_string!("exitCode"), context) {
                        if !ec_val.is_undefined() && !ec_val.is_null() {
                            if let Ok(updated) = ec_val.to_i32(context) {
                                code = updated;
                            }
                        }
                    }
                }
            }
            crate::boa_realm::set_process_exit(code);
            if crate::workers::is_worker_thread() {
                crate::workers::worker_self_set_listening(false, Some(code));
            }
            Err(boa_engine::error::PanicError::new(format!("__SLICE_PROCESS_EXIT__:{code}")).into())
        }),
    );
}
