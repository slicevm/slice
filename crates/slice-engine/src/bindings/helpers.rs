//! Shared Buffer Extraction, JS TypedArray Builders, Crypto Helpers, and Sandbox Budget Accounting.

use crate::boa_realm::{BOA_ACTIVE_TENANT, BOA_FUEL_BUDGET, BOA_KV_MEM_BUDGET, BOA_VIOLATION};
use crate::intrinsics::{
    hmac_md5_bytes, hmac_sha1_bytes, hmac_sha256_bytes, hmac_sha384_bytes, hmac_sha512_bytes,
    md5_bytes, sha1_bytes, sha256_bytes, sha384_bytes, sha512_bytes,
};
use slice_sandbox::SandboxViolation;

/// Invokes `f` with a zero-copy borrowed `&[u8]` slice when `val` is backed by a `TypedArray`
/// (`Uint8Array`, `Buffer`, etc.), `ArrayBuffer`, or `DataView`, falling back to an owned `Vec<u8>`
/// for strings or generic array-like objects.
#[inline]
pub(crate) fn with_buffer_source_bytes<R>(
    val: &boa_engine::JsValue,
    context: &mut boa_engine::Context,
    mut f: impl FnMut(&[u8]) -> R,
) -> boa_engine::JsResult<R> {
    use boa_engine::builtins::{dataview::DataView, typed_array::TypedArray};
    use boa_engine::object::builtins::JsArrayBuffer;

    if let Some(s) = val.as_string() {
        if let boa_engine::string::JsStrVariant::Latin1(bytes) = s.as_str().variant()
            && bytes.is_ascii()
        {
            return Ok(f(bytes));
        }
        let owned = s.to_std_string_escaped();
        return Ok(f(owned.as_bytes()));
    }
    let Some(obj) = val.as_object() else {
        return Ok(f(&[]));
    };
    if let Some(ta) = obj.downcast_ref::<TypedArray>()
        && let Some(res) = ta.with_bytes(&mut f)
    {
        return Ok(res);
    }
    if let Ok(ab) = JsArrayBuffer::from_object(obj.clone()) {
        if let Some(data) = ab.data() {
            return Ok(f(&data));
        }
        return Ok(f(&[]));
    }
    if let Some(dv) = obj.downcast_ref::<DataView>()
        && let Some(res) = dv.with_bytes(&mut f)
    {
        return Ok(res);
    }
    let fallback = extract_buffer_source_bytes(val, context)?;
    Ok(f(&fallback))
}

/// Fast-path helper for `Buffer.from(val, enc?).toString(...)` compile-time lowerings:
/// borrows `&[u8]` directly for primitive strings, `Uint8Array`/`Buffer`, and `ArrayBuffer`,
/// and delegates all other inputs (boxed `String`, `Symbol.toPrimitive`, `valueOf()`, non-byte
/// `TypedArray`s, `DataView`, arrays, invalid primitives) to `Buffer.from(val, enc?)` so exact
/// Node.js `Buffer.from` element conversion, unboxing, and `TypeError` checks apply (`#119`).
pub(crate) fn with_buffer_from_bytes<R>(
    val: &boa_engine::JsValue,
    enc_opt: Option<&str>,
    context: &mut boa_engine::Context,
    mut f: impl FnMut(&[u8]) -> R,
) -> boa_engine::JsResult<R> {
    use boa_engine::builtins::typed_array::TypedArray;
    use boa_engine::js_string;
    use boa_engine::object::builtins::{JsArrayBuffer, JsUint8Array};

    if enc_opt.is_none() || matches!(enc_opt, Some("utf8" | "utf-8")) {
        if let Some(s) = val.as_string() {
            if let boa_engine::string::JsStrVariant::Latin1(bytes) = s.as_str().variant()
                && bytes.is_ascii()
            {
                return Ok(f(bytes));
            }
            let owned = s.to_std_string_escaped();
            return Ok(f(owned.as_bytes()));
        }
        if let Some(obj) = val.as_object() {
            if JsUint8Array::from_object(obj.clone()).is_ok()
                && let Some(ta) = obj.downcast_ref::<TypedArray>()
                && let Some(res) = ta.with_bytes(&mut f)
            {
                return Ok(res);
            }
            if enc_opt.is_none()
                && let Ok(ab) = JsArrayBuffer::from_object(obj.clone())
            {
                if let Some(data) = ab.data() {
                    return Ok(f(&data));
                }
                return Ok(f(&[]));
            }
        }
    }

    let global = context.global_object();
    let mut buf_ctor = global.get(js_string!("Buffer"), context)?;
    if !buf_ctor.is_callable() {
        if let Ok(ensure_fn) = global.get(js_string!("__slice_ensure_shard"), context)
            && let Some(ensure_obj) = ensure_fn.as_callable()
        {
            let _ = ensure_obj.call(&boa_engine::JsValue::undefined(), &[5.into()], context)?;
        }
        buf_ctor = global.get(js_string!("Buffer"), context)?;
    }
    let Some(buf_obj) = buf_ctor.as_object() else {
        return Err(boa_engine::JsNativeError::typ()
            .with_message("Buffer is not available")
            .into());
    };
    let from_val = buf_obj.get(js_string!("from"), context)?;
    let Some(from_fn) = from_val.as_callable() else {
        return Err(boa_engine::JsNativeError::typ()
            .with_message("Buffer.from is not callable")
            .into());
    };
    let constructed = match enc_opt {
        Some(enc) => from_fn.call(
            &buf_ctor,
            &[val.clone(), boa_engine::JsValue::from(js_string!(enc))],
            context,
        )?,
        None => from_fn.call(&buf_ctor, std::slice::from_ref(val), context)?,
    };
    if let Some(c_obj) = constructed.as_object()
        && let Some(ta) = c_obj.downcast_ref::<TypedArray>()
        && let Some(res) = ta.with_bytes(&mut f)
    {
        return Ok(res);
    }
    Ok(f(&[]))
}

/// Invokes `f` with a zero-copy borrowed `&[u8]` slice when `val` is `Some` and neither
/// `undefined` nor `null`; otherwise invokes `f(&[])`.
#[inline]
pub(crate) fn with_optional_buffer_source_bytes<R>(
    val: Option<&boa_engine::JsValue>,
    context: &mut boa_engine::Context,
    mut f: impl FnMut(&[u8]) -> R,
) -> boa_engine::JsResult<R> {
    match val {
        Some(v) if !v.is_undefined() && !v.is_null() => with_buffer_source_bytes(v, context, f),
        _ => Ok(f(&[])),
    }
}

/// Invokes `f` with two zero-copy borrowed `&[u8]` slices simultaneously.
#[inline]
pub(crate) fn with_two_buffer_source_bytes<R>(
    a: &boa_engine::JsValue,
    b: &boa_engine::JsValue,
    context: &mut boa_engine::Context,
    mut f: impl FnMut(&[u8], &[u8]) -> R,
) -> boa_engine::JsResult<R> {
    use boa_engine::builtins::typed_array::TypedArray;

    if let (Some(obj_a), Some(obj_b)) = (a.as_object(), b.as_object())
        && let (Some(ta_a), Some(ta_b)) = (
            obj_a.downcast_ref::<TypedArray>(),
            obj_b.downcast_ref::<TypedArray>(),
        )
        && let Some(Some(res)) =
            ta_a.with_bytes(|bytes_a| ta_b.with_bytes(|bytes_b| f(bytes_a, bytes_b)))
    {
        return Ok(res);
    }
    let bytes_a = extract_buffer_source_bytes(a, context)?;
    with_buffer_source_bytes(b, context, |bytes_b| f(&bytes_a, bytes_b))
}

/// Invokes `f` with a mutable `&mut [u8]` slice backing a `TypedArray`, `ArrayBuffer` (when
/// `allow_raw_array_buffer` is `true`), or `ArrayBufferView`-like JS object (`{ buffer, byteOffset, byteLength }`).
pub(crate) fn with_mutable_buffer_view_bytes<R>(
    obj: &boa_engine::JsObject,
    allow_raw_array_buffer: bool,
    context: &mut boa_engine::Context,
    mut f: impl FnMut(&mut [u8]) -> R,
) -> boa_engine::JsResult<Option<R>> {
    use boa_engine::builtins::typed_array::TypedArray;
    use boa_engine::js_string;
    use boa_engine::object::builtins::JsArrayBuffer;

    if let Some(ta) = obj.downcast_ref::<TypedArray>()
        && let Some(res) = ta.with_bytes_mut(&mut f)
    {
        return Ok(Some(res));
    }
    if allow_raw_array_buffer && let Ok(ab) = JsArrayBuffer::from_object(obj.clone()) {
        return Ok(Some(match ab.data_mut() {
            Some(mut d) => f(&mut d),
            None => f(&mut []),
        }));
    }
    let buf_val = obj.get(js_string!("buffer"), context)?;
    let Some(buf_obj) = buf_val.as_object() else {
        return Ok(None);
    };
    let Ok(ab) = JsArrayBuffer::from_object(buf_obj) else {
        return Ok(None);
    };
    let view_off = obj
        .get(js_string!("byteOffset"), context)
        .ok()
        .and_then(|v| v.as_number())
        .map(|n| n.max(0.0) as usize)
        .unwrap_or(0);
    let view_len = obj
        .get(js_string!("byteLength"), context)
        .ok()
        .and_then(|v| v.as_number())
        .map(|n| n.max(0.0) as usize);
    let Some(mut data_mut) = ab.data_mut() else {
        return Ok(Some(f(&mut [])));
    };
    let start = view_off.min(data_mut.len());
    let end = view_len
        .map(|l| start.saturating_add(l).min(data_mut.len()))
        .unwrap_or(data_mut.len());
    Ok(Some(f(&mut data_mut[start..end])))
}

/// Copies `src` directly into `dest_val` starting at `dest_offset` with zero intermediate heap
/// allocations when `dest_val` is a `TypedArray`, `Buffer`, `ArrayBuffer`, or `ArrayBufferView`.
pub(crate) fn copy_into_js_buffer_view(
    dest_val: &boa_engine::JsValue,
    dest_offset: usize,
    src: &[u8],
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<usize> {
    if src.is_empty() {
        return Ok(0);
    }
    let Some(obj) = dest_val.as_object() else {
        return Ok(0);
    };
    Ok(
        with_mutable_buffer_view_bytes(&obj, true, context, |dest_slice| {
            if dest_offset >= dest_slice.len() {
                return 0;
            }
            let avail = src.len().min(dest_slice.len() - dest_offset);
            dest_slice[dest_offset..dest_offset + avail].copy_from_slice(&src[..avail]);
            avail
        })?
        .unwrap_or(0),
    )
}

/// Extracts an owned `Vec<u8>` from a JS `TypedArray`, `Buffer`, `ArrayBuffer`, `DataView`, string,
/// or array-like object using direct native downcast before falling back to JS property access.
pub(crate) fn extract_buffer_source_bytes(
    val: &boa_engine::JsValue,
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<Vec<u8>> {
    use boa_engine::builtins::{dataview::DataView, typed_array::TypedArray};
    use boa_engine::js_string;
    use boa_engine::object::builtins::JsArrayBuffer;

    if let Some(s) = val.as_string() {
        return Ok(s.to_std_string_escaped().into_bytes());
    }
    let Some(obj) = val.as_object() else {
        return Ok(Vec::new());
    };
    if let Some(ta) = obj.downcast_ref::<TypedArray>()
        && let Some(bytes) = ta.with_bytes(|slice| slice.to_vec())
    {
        return Ok(bytes);
    }
    if let Ok(ab) = JsArrayBuffer::from_object(obj.clone()) {
        return Ok(ab.data().map(|d| d.to_vec()).unwrap_or_default());
    }
    if let Some(dv) = obj.downcast_ref::<DataView>()
        && let Some(bytes) = dv.with_bytes(|slice| slice.to_vec())
    {
        return Ok(bytes);
    }
    if let Ok(buf_val) = obj.get(js_string!("buffer"), context)
        && let Some(buf_obj) = buf_val.as_object()
        && let Ok(ab) = JsArrayBuffer::from_object(buf_obj)
    {
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
    if let Ok(len_val) = obj.get(js_string!("length"), context)
        && let Some(len) = len_val.as_number().map(|n| n.max(0.0) as usize)
    {
        let capped = len.min(1_048_576);
        let mut out = Vec::with_capacity(capped);
        for i in 0..capped {
            let b = obj
                .get(i as u32, context)?
                .as_number()
                .map(|n| (n as i64 & 0xff) as u8)
                .unwrap_or(0);
            out.push(b);
        }
        return Ok(out);
    }
    Ok(Vec::new())
}

/// Encodes a `boa_engine::JsString` (Latin-1 or UTF-16, replacing lone surrogates with U+FFFD)
/// into an owned UTF-8 `Vec<u8>`.
pub(crate) fn js_string_to_utf8_bytes(js_s: &boa_engine::JsString) -> Vec<u8> {
    use boa_engine::string::JsStrVariant;
    match js_s.as_str().variant() {
        JsStrVariant::Latin1(bytes) => {
            if bytes.is_ascii() {
                return bytes.to_vec();
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
            out
        }
        JsStrVariant::Utf16(units) => {
            let mut out = Vec::with_capacity(units.len() * 3);
            for item in char::decode_utf16(units.iter().copied()) {
                let ch = item.unwrap_or(char::REPLACEMENT_CHARACTER);
                let mut buf = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            }
            out
        }
    }
}

#[inline]
pub(crate) fn ascii_bytes_to_js_val(bytes: &[u8]) -> boa_engine::JsValue {
    boa_engine::JsValue::from(boa_engine::JsString::from(
        boa_engine::string::JsStr::latin1(bytes),
    ))
}

#[inline]
pub(crate) fn utf8_slice_to_js_val(bytes: &[u8]) -> boa_engine::JsValue {
    if bytes.is_ascii() {
        return ascii_bytes_to_js_val(bytes);
    }
    let content = String::from_utf8_lossy(bytes);
    boa_engine::JsValue::from(boa_engine::js_string!(content.as_ref()))
}

#[inline]
pub(crate) fn bytes_to_js_array_buffer(
    bytes: &[u8],
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<boa_engine::JsValue> {
    use boa_engine::object::builtins::{AlignedVec, JsArrayBuffer};
    let block = AlignedVec::from_slice(0, bytes);
    let ab = JsArrayBuffer::from_byte_block(block, context)?;
    Ok(ab.into())
}

#[inline]
pub(crate) fn bytes_to_js_uint8_array(
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
pub(crate) fn normalize_webcrypto_hash_name(raw: &str) -> Option<&'static str> {
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

#[inline]
pub(crate) fn compute_webcrypto_digest_stack(
    norm_alg: &str,
    bytes: &[u8],
) -> Option<([u8; 64], usize)> {
    let mut out = [0u8; 64];
    match norm_alg {
        "SHA-1" => {
            out[..20].copy_from_slice(&sha1_bytes(bytes));
            Some((out, 20))
        }
        "SHA-256" => {
            out[..32].copy_from_slice(&sha256_bytes(bytes));
            Some((out, 32))
        }
        "SHA-384" => {
            out[..48].copy_from_slice(&sha384_bytes(bytes));
            Some((out, 48))
        }
        "SHA-512" => Some((sha512_bytes(bytes), 64)),
        "MD5" => {
            out[..16].copy_from_slice(&md5_bytes(bytes));
            Some((out, 16))
        }
        _ => None,
    }
}

#[inline]
pub(crate) fn compute_webcrypto_hmac_stack(
    hash_name: &str,
    key: &[u8],
    data: &[u8],
) -> Option<([u8; 64], usize)> {
    let mut out = [0u8; 64];
    match normalize_webcrypto_hash_name(hash_name)? {
        "SHA-1" => {
            out[..20].copy_from_slice(&hmac_sha1_bytes(key, data));
            Some((out, 20))
        }
        "SHA-256" => {
            out[..32].copy_from_slice(&hmac_sha256_bytes(key, data));
            Some((out, 32))
        }
        "SHA-384" => {
            out[..48].copy_from_slice(&hmac_sha384_bytes(key, data));
            Some((out, 48))
        }
        "SHA-512" => Some((hmac_sha512_bytes(key, data), 64)),
        "MD5" => {
            out[..16].copy_from_slice(&hmac_md5_bytes(key, data));
            Some((out, 16))
        }
        _ => None,
    }
}

#[inline]
pub(crate) fn with_active_tenant<R>(f: impl FnOnce(&str) -> R) -> R {
    BOA_ACTIVE_TENANT.with(|t| f(&t.borrow()))
}

/// Records `violation` in `BOA_VIOLATION`, preserving the first violation of the turn.
#[inline]
pub(crate) fn record_boa_violation_once(violation: SandboxViolation) {
    BOA_VIOLATION.with(|v| {
        if v.borrow().is_none() {
            *v.borrow_mut() = Some(violation);
        }
    });
}

/// Charges `cost` CPU fuel against the active isolate's `BOA_FUEL_BUDGET`, recording
/// `SandboxViolation::CpuFuelExhausted` and returning `Err` if the budget is exceeded.
#[inline]
pub(crate) fn consume_crypto_kdf_fuel(cost: u64) -> Result<(), String> {
    let (consumed, limit) = BOA_FUEL_BUDGET.with(|f| f.get());
    let next = consumed.saturating_add(cost.max(1));
    BOA_FUEL_BUDGET.with(|f| f.set((next, limit)));
    if next > limit {
        let tenant_id = active_tenant_id_or_default();
        record_boa_violation_once(SandboxViolation::CpuFuelExhausted { tenant_id, limit });
        return Err(format!(
            "RangeError: Cryptographic KDF CPU fuel budget exhausted (limit: {} units)",
            limit
        ));
    }
    Ok(())
}

/// Trips Boa's loop-iteration and recursion runtime limits to `1` so a guest script cannot
/// swallow a fatal sandbox budget violation (`MemoryCeilingExceeded` or `CpuFuelExhausted`)
/// inside `try { ... } catch {}` and continue looping.
#[inline]
pub(crate) fn trip_isolate_oom_limits(context: &mut boa_engine::Context) {
    context.runtime_limits_mut().set_loop_iteration_limit(1);
    context.runtime_limits_mut().set_recursion_limit(1);
}

/// Records `SandboxViolation::MemoryCeilingExceeded` in `BOA_VIOLATION` (preserving the first
/// violation of the turn) and returns the formatted `RangeError` message.
#[cold]
#[inline(never)]
pub(crate) fn record_isolate_memory_violation(requested: usize, limit: usize) -> String {
    let tenant_id = active_tenant_id_or_default();
    let violation = SandboxViolation::MemoryCeilingExceeded {
        tenant_id,
        requested,
        limit,
    };
    let msg = violation.to_string();
    record_boa_violation_once(violation);
    format!("RangeError: {}", msg)
}

/// Releases `released_bytes` from `BOA_KV_MEM_BUDGET` (`net_used` clamped at `0`).
#[inline]
pub(crate) fn release_isolate_memory(released_bytes: usize) {
    if released_bytes == 0 {
        return;
    }
    BOA_KV_MEM_BUDGET.with(|m| {
        let (used, limit) = m.get();
        if limit == usize::MAX {
            return;
        }
        m.set((used.saturating_sub(released_bytes), limit));
    });
}

/// Applies a signed `delta_bytes` adjustment to `BOA_KV_MEM_BUDGET`.
#[inline]
pub(crate) fn try_adjust_isolate_memory(delta_bytes: isize) -> Result<(), String> {
    if delta_bytes == 0 {
        return Ok(());
    }
    if delta_bytes < 0 {
        release_isolate_memory(delta_bytes.unsigned_abs());
        return Ok(());
    }
    try_consume_isolate_memory(delta_bytes as usize)
}

/// Context-aware signed memory adjustment that also trips `trip_isolate_oom_limits(context)` on OOM.
#[inline]
pub(crate) fn try_adjust_isolate_memory_ctx(
    delta_bytes: isize,
    context: &mut boa_engine::Context,
) -> Result<(), String> {
    let res = try_adjust_isolate_memory(delta_bytes);
    if res.is_err() {
        trip_isolate_oom_limits(context);
    }
    res
}

/// Charges `added_bytes` against the active isolate's `BOA_KV_MEM_BUDGET` ceiling (`max_memory_bytes`).
#[inline]
pub(crate) fn try_consume_isolate_memory(added_bytes: usize) -> Result<(), String> {
    if added_bytes == 0 {
        return Ok(());
    }
    let exceeded = BOA_KV_MEM_BUDGET.with(|m| {
        let (used, limit) = m.get();
        let next = used.saturating_add(added_bytes);
        if next > limit {
            Some((next, limit))
        } else {
            m.set((next, limit));
            None
        }
    });
    if let Some((requested, limit)) = exceeded {
        return Err(record_isolate_memory_violation(requested, limit));
    }
    Ok(())
}

/// Charges `added_bytes` against `BOA_KV_MEM_BUDGET` and trips `trip_isolate_oom_limits(context)` on OOM.
#[inline]
pub(crate) fn try_consume_isolate_memory_ctx(
    added_bytes: usize,
    context: &mut boa_engine::Context,
) -> Result<(), String> {
    let res = try_consume_isolate_memory(added_bytes);
    if res.is_err() {
        trip_isolate_oom_limits(context);
    }
    res
}

#[inline]
pub(crate) fn active_tenant_id_or_default() -> String {
    BOA_ACTIVE_TENANT.with(|t| {
        let b = t.borrow();
        if b.is_empty() {
            "boa-tenant".to_string()
        } else {
            b.clone()
        }
    })
}
