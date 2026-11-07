//! RFC 6455 WebSocket Client and Server Host Bindings (`Shard 7`).

use crate::websocket::{
    ws_client_close, ws_client_connect, ws_client_poll, ws_client_send, ws_server_queue_close,
    ws_server_queue_send, BOA_WS_MOCKS,
};

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
