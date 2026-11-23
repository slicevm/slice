//! Fast SHA-256 / HMAC-SHA256, Base64, UTF-8 Byte Length, and `TextEncoder.encode` Host Bindings.

use crate::bindings::helpers::{
    ascii_bytes_to_js_val, bytes_to_js_uint8_array, js_string_to_utf8_bytes, utf8_slice_to_js_val,
    with_buffer_from_bytes, with_buffer_source_bytes, with_two_buffer_source_bytes,
};
use crate::intrinsics::{
    base64_decode, base64_decode_bytes, base64_decoded_len_bytes, base64_encode, bytes_to_hex,
    hmac_sha256_bytes, hmac_sha256_hex, sha256_bytes, sha256_hex,
};

pub(super) fn register_fast_codecs_and_hash_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::builtins::typed_array::TypedArray;
    use boa_engine::object::builtins::JsArrayBuffer;
    use boa_engine::{JsValue as BoaVal, NativeFunction, js_string};

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
            if let Some(first) = args.first()
                && first.is_object()
            {
                let encoded = with_buffer_source_bytes(first, context, |bytes| {
                    if enc.eq_ignore_ascii_case("hex") {
                        sha256_hex(bytes)
                    } else {
                        encode_digest(&sha256_bytes(bytes))
                    }
                })?;
                return Ok(BoaVal::from(js_string!(encoded)));
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
                            base64_encode(&hmac_sha256_bytes(key_bytes, msg_bytes), false)
                        } else if enc.eq_ignore_ascii_case("base64url") {
                            base64_encode(&hmac_sha256_bytes(key_bytes, msg_bytes), true)
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
            Ok(BoaVal::from(js_string!(hmac_sha256_hex(
                key.as_bytes(),
                msg.as_bytes()
            ))))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_base64_encode"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let url_safe = args.get(1).and_then(|v| v.as_boolean()).unwrap_or(false);
            let first = args.first().cloned().unwrap_or_default();
            let encoded = with_buffer_from_bytes(&first, None, context, |bytes| {
                base64_encode(bytes, url_safe)
            })?;
            Ok(ascii_bytes_to_js_val(encoded.as_bytes()))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_base64_decode"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            use boa_engine::string::JsStrVariant;
            let first = args.first().cloned().unwrap_or_default();
            if let Some(js_s) = first.as_string() {
                let decoded = match js_s.as_str().variant() {
                    JsStrVariant::Latin1(bytes) => base64_decode_bytes(bytes),
                    JsStrVariant::Utf16(_) => base64_decode(&js_s.to_std_string_escaped()),
                };
                return Ok(utf8_slice_to_js_val(&decoded));
            }
            with_buffer_from_bytes(&first, Some("base64"), context, utf8_slice_to_js_val)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_buf_to_utf8"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let first = args.first().cloned().unwrap_or_default();
            if first.is_string() {
                return Ok(first);
            }
            with_buffer_from_bytes(&first, None, context, utf8_slice_to_js_val)
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
                if let Some(ta) = obj.downcast_ref::<TypedArray>()
                    && let Some(bl) = ta.fast_byte_length()
                {
                    return Ok(BoaVal::from(bl as f64));
                }
                if let Ok(ab) = JsArrayBuffer::from_object(obj.clone()) {
                    let bl = ab.data().map(|d| d.len()).unwrap_or(0);
                    return Ok(BoaVal::from(bl as f64));
                }
                if let Some(dv) = obj.downcast_ref::<DataView>()
                    && let Some(bl) = dv.with_bytes(|b| b.len())
                {
                    return Ok(BoaVal::from(bl as f64));
                }
                if let Ok(bl) = obj.get(js_string!("byteLength"), context)
                    && let Some(n) = bl.as_number()
                {
                    return Ok(BoaVal::from(n));
                }
                if let Ok(l) = obj.get(js_string!("length"), context)
                    && let Some(n) = l.as_number()
                {
                    return Ok(BoaVal::from(n));
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
            if let JsStrVariant::Latin1(bytes) = js_s.as_str().variant()
                && bytes.is_ascii()
            {
                return bytes_to_js_uint8_array(bytes, context);
            }
            let out = js_string_to_utf8_bytes(&js_s);
            bytes_to_js_uint8_array(&out, context)
        }),
    );
}
