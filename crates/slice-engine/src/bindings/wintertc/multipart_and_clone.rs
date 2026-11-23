//! RFC 7578 `multipart/form-data`, Byte Subsequence Search, and `structuredClone` Inspection Bindings.

use crate::bindings::helpers::{
    ascii_bytes_to_js_val, bytes_to_js_uint8_array, consume_crypto_kdf_fuel,
    trip_isolate_oom_limits, try_consume_isolate_memory_ctx, with_buffer_source_bytes,
};
use crate::boa_realm::BOA_DET_STATE;
use crate::intrinsics::multipart::parse_rfc7578_multipart;
use crate::intrinsics::{bytes_to_hex, fill_random_bytes};

pub(super) fn register_multipart_and_clone_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::builtins::typed_array::TypedArray;
    use boa_engine::object::builtins::JsArray;
    use boa_engine::{JsNativeError, JsValue as BoaVal, NativeFunction, js_string};

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_multipart_parse"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(buf_arg) = args.first() else {
                return Err(JsNativeError::typ()
                    .with_message("Failed to parse multipart/form-data: missing body")
                    .into());
            };
            let boundary = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();

            let parsed_res = with_buffer_source_bytes(buf_arg, context, |bytes| {
                let fuel_cost = ((bytes.len() as u64) / 1024).max(2);
                consume_crypto_kdf_fuel(fuel_cost)?;
                parse_rfc7578_multipart(bytes, &boundary)
            })?;

            let parts = match parsed_res {
                Ok(p) => p,
                Err(msg) => {
                    if msg.contains("CPU fuel budget exhausted") {
                        trip_isolate_oom_limits(context);
                        return Err(JsNativeError::range().with_message(msg).into());
                    }
                    return Err(JsNativeError::typ().with_message(msg).into());
                }
            };

            let mut out_items = Vec::with_capacity(parts.len());
            for part in parts {
                let entry = if let Some(filename) = part.filename {
                    if !part.payload.is_empty()
                        && let Err(msg) =
                            try_consume_isolate_memory_ctx(part.payload.len(), context)
                    {
                        return Err(JsNativeError::range().with_message(msg).into());
                    }
                    let u8_val = bytes_to_js_uint8_array(&part.payload, context)?;
                    JsArray::from_iter(
                        [
                            BoaVal::from(js_string!(part.name)),
                            BoaVal::from(true),
                            BoaVal::from(js_string!(filename)),
                            BoaVal::from(js_string!(part.content_type)),
                            u8_val,
                        ],
                        context,
                    )
                } else {
                    let text = String::from_utf8_lossy(&part.payload);
                    JsArray::from_iter(
                        [
                            BoaVal::from(js_string!(part.name)),
                            BoaVal::from(false),
                            BoaVal::from(js_string!(text.as_ref())),
                        ],
                        context,
                    )
                };
                out_items.push(BoaVal::from(entry));
            }
            Ok(BoaVal::from(JsArray::from_iter(out_items, context)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_multipart_boundary"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let mut rnd = [0u8; 16];
            BOA_DET_STATE.with(|s| {
                if let Some((mut prng, clk)) = s.get() {
                    fill_random_bytes(&mut rnd, Some(&mut prng));
                    s.set(Some((prng, clk)));
                } else {
                    fill_random_bytes(&mut rnd, None);
                }
            });
            let mut out = String::with_capacity(53);
            out.push_str("----SliceFormBoundary");
            out.push_str(&bytes_to_hex(&rnd));
            Ok(ascii_bytes_to_js_val(out.as_bytes()))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_u8_contains"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(buf_arg) = args.first() else {
                return Ok(BoaVal::from(false));
            };
            let needle = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if needle.is_empty() {
                return Ok(BoaVal::from(true));
            }
            let found = with_buffer_source_bytes(buf_arg, context, |bytes| {
                memchr::memmem::find(bytes, needle.as_bytes()).is_some()
            })?;
            Ok(BoaVal::from(found))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_sc_inspect"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let Some(val) = args.first() else {
                return Ok(BoaVal::from(0));
            };
            let Some(obj) = val.as_object() else {
                return Ok(BoaVal::from(0));
            };
            if obj.is::<boa_engine::builtins::object::OrdinaryObject>() || obj.is_array() {
                return Ok(BoaVal::from(0));
            }
            if let Some(ta) = obj.downcast_ref::<TypedArray>() {
                if ta.is_out_of_bounds_now() {
                    return Ok(BoaVal::from(-1));
                }
                return Ok(BoaVal::from(if ta.is_auto_length() { 2 } else { 1 }));
            }
            if let Some(dv) = obj.downcast_ref::<boa_engine::builtins::dataview::DataView>() {
                if dv.is_out_of_bounds_now() {
                    return Ok(BoaVal::from(-1));
                }
                return Ok(BoaVal::from(if dv.is_auto_byte_length() { 2 } else { 1 }));
            }
            if let Some(ab) = obj.downcast_ref::<boa_engine::builtins::array_buffer::ArrayBuffer>()
            {
                if ab.has_detach_key() {
                    return Ok(BoaVal::from(-3));
                }
                return Ok(BoaVal::from(0));
            }
            if obj.is::<boa_engine::JsBigInt>() {
                return Ok(BoaVal::from(3));
            }
            if obj.is::<bool>() {
                return Ok(BoaVal::from(4));
            }
            if obj.is::<f64>() {
                return Ok(BoaVal::from(5));
            }
            if obj.is::<boa_engine::JsString>() {
                return Ok(BoaVal::from(6));
            }
            if obj.is::<boa_engine::builtins::proxy::Proxy>()
                || obj.is::<boa_engine::JsSymbol>()
                || obj.is::<boa_engine::builtins::generator::Generator>()
                || obj.is::<boa_engine::builtins::async_generator::AsyncGenerator>()
            {
                return Ok(BoaVal::from(-2));
            }
            Ok(BoaVal::from(0))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_sc_clone_sab"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            use boa_engine::object::builtins::JsSharedArrayBuffer;
            let Some(obj) = args.first().and_then(|v| v.as_object()) else {
                return Err(JsNativeError::typ()
                    .with_message("Expected SharedArrayBuffer")
                    .into());
            };
            let sab = JsSharedArrayBuffer::from_object(obj.clone())?;
            let cloned = JsSharedArrayBuffer::from_buffer(sab.inner(), context);
            Ok(BoaVal::from(cloned))
        }),
    );
}
