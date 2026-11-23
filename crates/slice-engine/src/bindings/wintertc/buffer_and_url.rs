//! Native `Buffer` Hex/Base64 Codecs and WHATWG `URL.canParse` / `URLSearchParams` Host Bindings.

use crate::bindings::helpers::{
    ascii_bytes_to_js_val, bytes_to_js_uint8_array, with_buffer_source_bytes,
};
use crate::intrinsics::{
    base64_decode, base64_decode_bytes, base64_encode, bytes_to_hex, can_parse_whatwg_url,
    hex_decode_bytes, hex_decode_raw_bytes, usp_decode_component, usp_encode_component,
};
use std::borrow::Cow;

#[inline]
fn js_str_to_cow(js: &boa_engine::JsString) -> Cow<'_, str> {
    use boa_engine::string::JsStrVariant;
    if let JsStrVariant::Latin1(bytes) = js.as_str().variant()
        && bytes.is_ascii()
        && let Ok(s) = std::str::from_utf8(bytes)
    {
        return Cow::Borrowed(s);
    }
    Cow::Owned(js.to_std_string_escaped())
}

pub(crate) fn register_buffer_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{JsValue as BoaVal, NativeFunction, js_string};
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
            let hex = with_buffer_source_bytes(first, context, bytes_to_hex)?;
            Ok(ascii_bytes_to_js_val(hex.as_bytes()))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_base64_decode_bytes"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            use boa_engine::string::JsStrVariant;
            let Some(first) = args.first() else {
                return bytes_to_js_uint8_array(&[], context);
            };
            let js = first.to_string(context)?;
            let decoded = match js.as_str().variant() {
                JsStrVariant::Latin1(bytes) => base64_decode_bytes(bytes),
                JsStrVariant::Utf16(_) => base64_decode(&js.to_std_string_escaped()),
            };
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
            let encoded =
                with_buffer_source_bytes(first, context, |bytes| base64_encode(bytes, url_safe))?;
            Ok(ascii_bytes_to_js_val(encoded.as_bytes()))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_hex_decode"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            use boa_engine::string::JsStrVariant;
            let Some(first) = args.first() else {
                return bytes_to_js_uint8_array(&[], context);
            };
            let js = first.to_string(context)?;
            let decoded = match js.as_str().variant() {
                JsStrVariant::Latin1(bytes) => hex_decode_raw_bytes(bytes),
                JsStrVariant::Utf16(_) => hex_decode_bytes(&js.to_std_string_escaped()),
            };
            bytes_to_js_uint8_array(&decoded, context)
        }),
    );
}

/// Registers native Rust `URL` and `URLSearchParams` host intrinsics into `ctx` when Shard 1 loads.
pub(crate) fn register_url_and_usp_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{JsValue as BoaVal, NativeFunction, js_string};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_url_can_parse"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_url_can_parse"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::from(false));
            };
            let js_input = first.to_string(context)?;
            let input_cow = js_str_to_cow(&js_input);
            let js_base = match args.get(1) {
                Some(v) if !v.is_undefined() => Some(v.to_string(context)?),
                _ => None,
            };
            let base_cow = js_base.as_ref().map(js_str_to_cow);
            Ok(BoaVal::from(can_parse_whatwg_url(
                input_cow.as_ref(),
                base_cow.as_deref(),
            )))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_usp_encode"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::from(js_string!("")));
            };
            let s = first.to_string(context)?.to_std_string_escaped();
            let encoded = usp_encode_component(&s);
            Ok(BoaVal::from(js_string!(encoded.as_ref())))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_usp_decode"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::from(js_string!("")));
            };
            let s = first.to_string(context)?.to_std_string_escaped();
            let decoded = usp_decode_component(&s);
            Ok(BoaVal::from(js_string!(decoded.as_ref())))
        }),
    );
}
