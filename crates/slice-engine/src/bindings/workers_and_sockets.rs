//! Web `Worker` Threads (`worker_threads`) and OCap TCP/TLS/UDP Sockets (`Shard 8`) Host Bindings.

use crate::bindings::helpers::with_buffer_source_bytes;
use crate::sockets::{
    classify_ip_literal, register_socket_mock, tcp_socket_close, tcp_socket_connect,
    tcp_socket_read, tcp_socket_starttls, tcp_socket_write, tcp_socket_write_bytes,
    udp_socket_bind, udp_socket_close, udp_socket_send, udp_socket_send_bytes,
};
use crate::workers::{
    worker_post_message_to_child, worker_self_meta_json, worker_self_post_message,
    worker_self_set_listening, worker_set_child_ref, worker_spawn, worker_terminate_child,
};

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
                let res = match args.get(1) {
                    Some(v) => with_buffer_source_bytes(v, context, |bytes| {
                        tcp_socket_write_bytes(id, bytes)
                    })?,
                    None => tcp_socket_write_bytes(id, &[]),
                };
                match res {
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
                let res = match args.get(1) {
                    Some(v) => with_buffer_source_bytes(v, context, |bytes| {
                        udp_socket_send_bytes(id, bytes, &host, port)
                    })?,
                    None => udp_socket_send_bytes(id, &[], &host, port),
                };
                match res {
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
