//! WHATWG `btoa`/`atob`, `TextEncoder.encodeInto`, `TextDecoder`, and `zlib` Compression Bindings.

use crate::bindings::helpers::{
    ascii_bytes_to_js_val, bytes_to_js_uint8_array, consume_crypto_kdf_fuel,
    trip_isolate_oom_limits, try_consume_isolate_memory_ctx, with_buffer_source_bytes,
    with_mutable_buffer_view_bytes,
};
use crate::intrinsics::{
    base64_encode, whatwg_atob_bytes, whatwg_btoa_utf16, zlib_compress_bytes, zlib_decompress_bytes,
};

#[inline]
fn encode_js_string_utf8_into(
    js_s: &boa_engine::JsString,
    dest_slice: &mut [u8],
) -> (usize, usize) {
    use boa_engine::string::JsStrVariant;
    match js_s.as_str().variant() {
        JsStrVariant::Latin1(bytes) => {
            if bytes.is_ascii() {
                let n = bytes.len().min(dest_slice.len());
                dest_slice[..n].copy_from_slice(&bytes[..n]);
                return (n, n);
            }
            let mut read_utf16 = 0usize;
            let mut written_utf8 = 0usize;
            for &b in bytes {
                let ch = b as char;
                let need = ch.len_utf8();
                if written_utf8 + need > dest_slice.len() {
                    break;
                }
                ch.encode_utf8(&mut dest_slice[written_utf8..written_utf8 + need]);
                written_utf8 += need;
                read_utf16 += 1;
            }
            (read_utf16, written_utf8)
        }
        JsStrVariant::Utf16(units) => {
            let mut read_utf16 = 0usize;
            let mut written_utf8 = 0usize;
            for item in char::decode_utf16(units.iter().copied()) {
                let (ch, u16_units) = match item {
                    Ok(c) => (c, c.len_utf16()),
                    Err(_) => (char::REPLACEMENT_CHARACTER, 1usize),
                };
                let need = ch.len_utf8();
                if written_utf8 + need > dest_slice.len() {
                    break;
                }
                ch.encode_utf8(&mut dest_slice[written_utf8..written_utf8 + need]);
                written_utf8 += need;
                read_utf16 += u16_units;
            }
            (read_utf16, written_utf8)
        }
    }
}

pub(super) fn register_encoding_and_zlib_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{JsNativeError, JsValue as BoaVal, NativeFunction, js_string};

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_btoa"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            use boa_engine::string::JsStrVariant;
            let Some(first) = args.first() else {
                return Ok(BoaVal::from(js_string!("")));
            };
            let js_s = first.to_string(context)?;
            let res = match js_s.as_str().variant() {
                JsStrVariant::Latin1(bytes) => Ok(base64_encode(bytes, false)),
                JsStrVariant::Utf16(units) => whatwg_btoa_utf16(units),
            };
            match res {
                Ok(encoded) => Ok(ascii_bytes_to_js_val(encoded.as_bytes())),
                Err(msg) => Err(JsNativeError::error().with_message(msg).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_atob"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            use boa_engine::string::JsStrVariant;
            let Some(first) = args.first() else {
                return Ok(BoaVal::from(js_string!("")));
            };
            let js_s = first.to_string(context)?;
            let decoded_res = match js_s.as_str().variant() {
                JsStrVariant::Latin1(bytes) => whatwg_atob_bytes(bytes),
                JsStrVariant::Utf16(units) => {
                    let mut raw = Vec::with_capacity(units.len());
                    for &u in units {
                        if u > 0x00FF {
                            return Err(JsNativeError::error()
                                .with_message("InvalidCharacterError: The string to be decoded contains invalid characters.")
                                .into());
                        }
                        raw.push(u as u8);
                    }
                    whatwg_atob_bytes(&raw)
                }
            };
            match decoded_res {
                Ok(decoded) => Ok(ascii_bytes_to_js_val(&decoded)),
                Err(msg) => Err(JsNativeError::error().with_message(msg).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_text_encode_into"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let js_s = match args.first() {
                Some(v) => v.to_string(context)?,
                None => js_string!(""),
            };
            let Some(dest_obj) = args.get(1).and_then(|v| v.as_object()) else {
                return Err(JsNativeError::typ()
                    .with_message("TextEncoder.encodeInto requires a Uint8Array destination")
                    .into());
            };

            let Some((read_utf16, written_utf8)) =
                with_mutable_buffer_view_bytes(&dest_obj, false, context, |dest_slice| {
                    encode_js_string_utf8_into(&js_s, dest_slice)
                })?
            else {
                return Err(JsNativeError::typ()
                    .with_message("TextEncoder.encodeInto requires a Uint8Array destination")
                    .into());
            };

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
            let enc = args
                .get(1)
                .and_then(|v| v.as_string())
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "utf-8".to_string());
            let fatal = args.get(2).and_then(|v| v.as_boolean()).unwrap_or(false);
            let ignore_bom = args.get(3).and_then(|v| v.as_boolean()).unwrap_or(false);

            let res = with_buffer_source_bytes(
                first,
                context,
                |bytes| -> Result<String, &'static str> {
                    if enc.eq_ignore_ascii_case("latin1") || enc.eq_ignore_ascii_case("iso-8859-1")
                    {
                        let mut s = String::with_capacity(bytes.len());
                        for &b in bytes {
                            s.push(b as char);
                        }
                        return Ok(s);
                    }
                    if enc.eq_ignore_ascii_case("ascii") || enc.eq_ignore_ascii_case("us-ascii") {
                        let mut s = String::with_capacity(bytes.len());
                        for &b in bytes {
                            if b > 0x7F {
                                if fatal {
                                    return Err(
                                        "The encoded data was not valid for encoding ascii",
                                    );
                                }
                                s.push('\u{FFFD}');
                            } else {
                                s.push(b as char);
                            }
                        }
                        return Ok(s);
                    }

                    let slice = if !ignore_bom && bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
                        &bytes[3..]
                    } else {
                        bytes
                    };
                    if fatal {
                        match simdutf8::basic::from_utf8(slice) {
                            Ok(valid) => Ok(valid.to_string()),
                            Err(_) => Err("The encoded data was not valid for encoding utf-8"),
                        }
                    } else {
                        Ok(String::from_utf8_lossy(slice).into_owned())
                    }
                },
            )?;

            match res {
                Ok(decoded) => Ok(BoaVal::from(js_string!(decoded))),
                Err(msg) => Err(JsNativeError::typ().with_message(msg).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_zlib_compress"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let fmt = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "gzip".to_string());
            let compressed_res = match args.first() {
                Some(v) if !v.is_undefined() && !v.is_null() => {
                    with_buffer_source_bytes(v, context, |bytes| {
                        let fuel_cost = ((bytes.len() as u64) / 256).max(5);
                        consume_crypto_kdf_fuel(fuel_cost)?;
                        zlib_compress_bytes(bytes, &fmt).map_err(|e| e.to_string())
                    })?
                }
                _ => zlib_compress_bytes(&[], &fmt).map_err(|e| e.to_string()),
            };
            match compressed_res {
                Ok(compressed) => {
                    if let Err(msg) = try_consume_isolate_memory_ctx(compressed.len(), context) {
                        return Err(JsNativeError::range().with_message(msg).into());
                    }
                    bytes_to_js_uint8_array(&compressed, context)
                }
                Err(msg) => {
                    if msg.contains("CPU fuel budget exhausted") {
                        trip_isolate_oom_limits(context);
                        return Err(JsNativeError::range().with_message(msg).into());
                    }
                    Err(JsNativeError::typ().with_message(msg).into())
                }
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_zlib_decompress"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let fmt = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "gzip".to_string());
            let decompressed_res = match args.first() {
                Some(v) if !v.is_undefined() && !v.is_null() => {
                    with_buffer_source_bytes(v, context, |bytes| {
                        let base_fuel = ((bytes.len() as u64) / 256).max(5);
                        consume_crypto_kdf_fuel(base_fuel)?;
                        zlib_decompress_bytes(bytes, &fmt).map_err(|e| e.to_string())
                    })?
                }
                _ => zlib_decompress_bytes(&[], &fmt).map_err(|e| e.to_string()),
            };
            match decompressed_res {
                Ok(decompressed) => {
                    let out_fuel = (decompressed.len() as u64) / 512;
                    if out_fuel > 0
                        && let Err(msg) = consume_crypto_kdf_fuel(out_fuel)
                    {
                        trip_isolate_oom_limits(context);
                        return Err(JsNativeError::range().with_message(msg).into());
                    }
                    if let Err(msg) = try_consume_isolate_memory_ctx(decompressed.len(), context) {
                        return Err(JsNativeError::range().with_message(msg).into());
                    }
                    bytes_to_js_uint8_array(&decompressed, context)
                }
                Err(msg) => {
                    if msg.contains("sandbox memory limit") {
                        let _ = try_consume_isolate_memory_ctx(usize::MAX / 2, context);
                        return Err(JsNativeError::range().with_message(msg).into());
                    }
                    if msg.contains("CPU fuel budget exhausted")
                        || msg.contains("empty block limit")
                    {
                        if msg.contains("CPU fuel budget exhausted") {
                            trip_isolate_oom_limits(context);
                        }
                        return Err(JsNativeError::range().with_message(msg).into());
                    }
                    Err(JsNativeError::typ().with_message(msg).into())
                }
            }
        }),
    );
}
