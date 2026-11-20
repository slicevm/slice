//! WinterTC Phase N3 (`btoa`, `atob`, `TextEncoder.encodeInto`, `TextDecoder`, `zlib`) and `Buffer` Codec Bindings.

use crate::bindings::helpers::{
    bytes_to_js_uint8_array, consume_crypto_kdf_fuel, trip_isolate_oom_limits,
    try_consume_isolate_memory_ctx, with_buffer_source_bytes,
};
use crate::boa_realm::BOA_DET_STATE;
use crate::intrinsics::{
    base64_decode, base64_decode_bytes, base64_encode, bytes_to_hex, fill_random_bytes,
    hex_decode_bytes, hex_decode_raw_bytes, whatwg_atob_bytes, whatwg_btoa_utf16,
    zlib_compress_bytes, zlib_decompress_bytes,
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

pub(crate) fn register_wintertc_phase_n3_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::builtins::typed_array::TypedArray;
    use boa_engine::object::builtins::JsArrayBuffer;
    use boa_engine::{JsNativeError, JsValue as BoaVal, NativeFunction, js_string};
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
                Ok(encoded) => Ok(BoaVal::from(boa_engine::JsString::from(
                    boa_engine::string::JsStr::latin1(encoded.as_bytes()),
                ))),
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
                Ok(decoded) => Ok(BoaVal::from(boa_engine::JsString::from(
                    boa_engine::string::JsStr::latin1(&decoded),
                ))),
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

            let mut read_utf16 = 0usize;
            let mut written_utf8 = 0usize;

            let mut handled_fast = false;
            if let Some(ta) = dest_obj.downcast_ref::<TypedArray>()
                && ta
                    .with_bytes_mut(|dest_slice| {
                        let (r, w) = encode_js_string_utf8_into(&js_s, dest_slice);
                        read_utf16 = r;
                        written_utf8 = w;
                    })
                    .is_some()
            {
                handled_fast = true;
            }

            if !handled_fast {
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

                if let Some(mut data_mut) = ab.data_mut() {
                    let start = offset.min(data_mut.len());
                    let end = (start + byte_len).min(data_mut.len());
                    let dest_slice = &mut data_mut[start..end];
                    let (r, w) = encode_js_string_utf8_into(&js_s, dest_slice);
                    read_utf16 = r;
                    written_utf8 = w;
                };
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

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_multipart_parse"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            use boa_engine::object::builtins::JsArray;

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
            Ok(BoaVal::from(boa_engine::JsString::from(
                boa_engine::string::JsStr::latin1(out.as_bytes()),
            )))
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

const MAX_MULTIPART_BOUNDARY_LEN: usize = 256;
const MAX_MULTIPART_PARTS: usize = 2_048;
const MAX_PART_HEADER_BYTES: usize = 16 * 1024;

struct ParsedMultipartPart {
    name: String,
    filename: Option<String>,
    content_type: String,
    payload: Vec<u8>,
}

#[inline]
fn trim_ascii_bytes(mut s: &[u8]) -> &[u8] {
    while let Some((&first, rest)) = s.split_first() {
        if first == b' ' || first == b'\t' {
            s = rest;
        } else {
            break;
        }
    }
    while let Some((&last, rest)) = s.split_last() {
        if last == b' ' || last == b'\t' {
            s = rest;
        } else {
            break;
        }
    }
    s
}

fn unescape_multipart_attr(raw: &[u8]) -> String {
    let s = String::from_utf8_lossy(raw);
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            out.push(bytes[i + 1]);
            i += 2;
            continue;
        }
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let pair = &bytes[i + 1..i + 3];
            if pair.eq_ignore_ascii_case(b"22") {
                out.push(b'"');
                i += 3;
                continue;
            }
            if pair.eq_ignore_ascii_case(b"0a") {
                out.push(b'\n');
                i += 3;
                continue;
            }
            if pair.eq_ignore_ascii_case(b"0d") {
                out.push(b'\r');
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_content_disposition_form_data(
    raw: &[u8],
) -> Result<(String, Option<String>), &'static str> {
    let trimmed = trim_ascii_bytes(raw);
    let semi = memchr::memchr(b';', trimmed).unwrap_or(trimmed.len());
    let disp_type = trim_ascii_bytes(&trimmed[..semi]);
    if !disp_type.eq_ignore_ascii_case(b"form-data") {
        return Err("Failed to parse multipart/form-data: Content-Disposition is not form-data");
    }

    let mut name_opt: Option<String> = None;
    let mut filename_opt: Option<String> = None;
    let mut i = if semi < trimmed.len() {
        semi + 1
    } else {
        trimmed.len()
    };

    while i < trimmed.len() {
        while i < trimmed.len() && matches!(trimmed[i], b' ' | b'\t' | b';') {
            i += 1;
        }
        if i >= trimmed.len() {
            break;
        }
        let key_start = i;
        while i < trimmed.len() && trimmed[i] != b'=' && trimmed[i] != b';' {
            i += 1;
        }
        let key = trim_ascii_bytes(&trimmed[key_start..i]);
        if i >= trimmed.len() || trimmed[i] != b'=' {
            continue;
        }
        i += 1; // skip '='
        while i < trimmed.len() && matches!(trimmed[i], b' ' | b'\t') {
            i += 1;
        }
        let val_str = if i < trimmed.len() && trimmed[i] == b'"' {
            i += 1; // skip opening '"'
            let val_start = i;
            while i < trimmed.len() {
                if trimmed[i] == b'\\' && i + 1 < trimmed.len() {
                    i += 2;
                    continue;
                }
                if trimmed[i] == b'"' {
                    break;
                }
                i += 1;
            }
            if i >= trimmed.len() || trimmed[i] != b'"' {
                return Err("Failed to parse multipart/form-data: unterminated quoted parameter");
            }
            let raw_val = &trimmed[val_start..i];
            i += 1; // skip closing '"'
            while i < trimmed.len() && trimmed[i] != b';' {
                i += 1;
            }
            unescape_multipart_attr(raw_val)
        } else {
            let val_start = i;
            while i < trimmed.len() && trimmed[i] != b';' {
                i += 1;
            }
            unescape_multipart_attr(trim_ascii_bytes(&trimmed[val_start..i]))
        };

        if key.eq_ignore_ascii_case(b"name") {
            name_opt = Some(val_str);
        } else if key.eq_ignore_ascii_case(b"filename") {
            filename_opt = Some(val_str);
        }
    }

    let Some(name) = name_opt else {
        return Err("Failed to parse multipart/form-data: missing Content-Disposition name");
    };
    Ok((name, filename_opt))
}

#[inline]
fn classify_multipart_boundary_suffix(bytes: &[u8], after_delim: usize) -> Option<(bool, usize)> {
    if bytes.get(after_delim..after_delim + 2) == Some(b"--") {
        let mut p = after_delim + 2;
        while p < bytes.len() && matches!(bytes[p], b' ' | b'\t') {
            p += 1;
        }
        if p == bytes.len() {
            return Some((true, p));
        }
        if bytes.get(p..p + 2) == Some(b"\r\n") {
            return Some((true, p + 2));
        }
        if bytes[p] == b'\n' {
            return Some((true, p + 1));
        }
        return None;
    }

    let mut p = after_delim;
    while p < bytes.len() && matches!(bytes[p], b' ' | b'\t') {
        p += 1;
    }
    if bytes.get(p..p + 2) == Some(b"\r\n") {
        return Some((false, p + 2));
    }
    if p < bytes.len() && bytes[p] == b'\n' {
        return Some((false, p + 1));
    }
    None
}

fn parse_rfc7578_multipart(
    bytes: &[u8],
    boundary: &str,
) -> Result<Vec<ParsedMultipartPart>, String> {
    if boundary.is_empty() || boundary.len() > MAX_MULTIPART_BOUNDARY_LEN {
        return Err("Failed to parse multipart/form-data: invalid boundary length".to_string());
    }

    let mut dash_boundary = Vec::with_capacity(2 + boundary.len());
    dash_boundary.extend_from_slice(b"--");
    dash_boundary.extend_from_slice(boundary.as_bytes());
    let finder = memchr::memmem::Finder::new(&dash_boundary);

    let mut search_pos = 0usize;
    let mut part_start_opt: Option<usize> = None;
    while search_pos < bytes.len() {
        let Some(rel) = finder.find(&bytes[search_pos..]) else {
            break;
        };
        let pos = search_pos + rel;
        let valid_line_start = pos == 0 || bytes[pos - 1] == b'\n';
        if valid_line_start
            && let Some((is_closing, next_pos)) =
                classify_multipart_boundary_suffix(bytes, pos + dash_boundary.len())
        {
            if is_closing {
                return Ok(Vec::new());
            }
            part_start_opt = Some(next_pos);
            break;
        }
        search_pos = pos + 1;
    }

    let Some(mut part_start) = part_start_opt else {
        return Err("Failed to parse multipart/form-data: missing boundary".to_string());
    };

    let mut parts = Vec::new();
    loop {
        if parts.len() >= MAX_MULTIPART_PARTS {
            return Err(
                "Failed to parse multipart/form-data: part count exceeds limit".to_string(),
            );
        }

        let header_limit = bytes
            .len()
            .min(part_start.saturating_add(MAX_PART_HEADER_BYTES + 4));
        let mut cursor = part_start;
        let mut disp_raw: Option<&[u8]> = None;
        let mut content_type_opt: Option<String> = None;
        let mut body_start_opt: Option<usize> = None;

        while cursor < header_limit {
            let Some(nl_rel) = memchr::memchr(b'\n', &bytes[cursor..header_limit]) else {
                return Err(
                    "Failed to parse multipart/form-data: unterminated part headers".to_string(),
                );
            };
            let line_end = cursor + nl_rel;
            let line = if line_end > cursor && bytes[line_end - 1] == b'\r' {
                &bytes[cursor..line_end - 1]
            } else {
                &bytes[cursor..line_end]
            };
            cursor = line_end + 1;

            if line.is_empty() {
                body_start_opt = Some(cursor);
                break;
            }

            let Some(colon_idx) = memchr::memchr(b':', line) else {
                return Err(
                    "Failed to parse multipart/form-data: malformed part header".to_string()
                );
            };
            let h_name = trim_ascii_bytes(&line[..colon_idx]);
            let h_val = trim_ascii_bytes(&line[colon_idx + 1..]);
            if h_name.eq_ignore_ascii_case(b"content-disposition") {
                disp_raw = Some(h_val);
            } else if h_name.eq_ignore_ascii_case(b"content-type") {
                content_type_opt = Some(String::from_utf8_lossy(h_val).into_owned());
            }
        }

        let Some(body_start) = body_start_opt else {
            return Err(
                "Failed to parse multipart/form-data: part headers exceed 16 KiB limit".to_string(),
            );
        };
        let Some(disp_bytes) = disp_raw else {
            return Err(
                "Failed to parse multipart/form-data: missing Content-Disposition header"
                    .to_string(),
            );
        };
        let (name, filename) =
            parse_content_disposition_form_data(disp_bytes).map_err(str::to_string)?;

        let mut next_search = body_start;
        let mut found_next: Option<(usize, bool, usize)> = None;
        while next_search < bytes.len() {
            let Some(rel) = finder.find(&bytes[next_search..]) else {
                break;
            };
            let delim_pos = next_search + rel;
            let preceded_by_nl = delim_pos > body_start && bytes[delim_pos - 1] == b'\n';
            if preceded_by_nl
                && let Some((is_closing, after_next)) =
                    classify_multipart_boundary_suffix(bytes, delim_pos + dash_boundary.len())
            {
                let payload_end =
                    if delim_pos >= body_start + 2 && &bytes[delim_pos - 2..delim_pos] == b"\r\n" {
                        delim_pos - 2
                    } else {
                        delim_pos - 1
                    };
                found_next = Some((payload_end, is_closing, after_next));
                break;
            }
            next_search = delim_pos + 1;
        }

        let Some((payload_end, is_closing, after_next)) = found_next else {
            return Err(
                "Failed to parse multipart/form-data: missing closing boundary".to_string(),
            );
        };

        let ct = content_type_opt.unwrap_or_else(|| "text/plain".to_string());
        parts.push(ParsedMultipartPart {
            name,
            filename,
            content_type: ct,
            payload: bytes[body_start..payload_end].to_vec(),
        });

        if is_closing {
            return Ok(parts);
        }
        part_start = after_next;
    }
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
            Ok(BoaVal::from(boa_engine::JsString::from(
                boa_engine::string::JsStr::latin1(hex.as_bytes()),
            )))
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
            Ok(BoaVal::from(boa_engine::JsString::from(
                boa_engine::string::JsStr::latin1(encoded.as_bytes()),
            )))
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
    use crate::intrinsics::{can_parse_whatwg_url, usp_decode_component, usp_encode_component};
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
            use boa_engine::string::JsStrVariant;
            let Some(first) = args.first() else {
                return Ok(BoaVal::from(false));
            };
            let js_input = first.to_string(context)?;
            let input_cow: std::borrow::Cow<'_, str> = match js_input.as_str().variant() {
                JsStrVariant::Latin1(bytes) if bytes.is_ascii() => {
                    // SAFETY: `bytes.is_ascii()` guarantees valid UTF-8.
                    std::borrow::Cow::Borrowed(unsafe { std::str::from_utf8_unchecked(bytes) })
                }
                _ => std::borrow::Cow::Owned(js_input.to_std_string_escaped()),
            };
            let js_base = match args.get(1) {
                Some(v) if !v.is_undefined() => Some(v.to_string(context)?),
                _ => None,
            };
            let base_cow: Option<std::borrow::Cow<'_, str>> = match js_base.as_ref() {
                Some(b) => match b.as_str().variant() {
                    JsStrVariant::Latin1(bytes) if bytes.is_ascii() => {
                        Some(std::borrow::Cow::Borrowed(unsafe {
                            std::str::from_utf8_unchecked(bytes)
                        }))
                    }
                    _ => Some(std::borrow::Cow::Owned(b.to_std_string_escaped())),
                },
                None => None,
            };
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
