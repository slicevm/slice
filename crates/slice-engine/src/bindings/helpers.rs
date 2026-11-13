//! Shared Buffer Extraction, JS TypedArray Builders, Crypto Helpers, and Sandbox Budget Accounting.

use crate::boa_realm::{BOA_ACTIVE_TENANT, BOA_FUEL_BUDGET, BOA_KV_MEM_BUDGET, BOA_VIOLATION};
use crate::intrinsics::{
    hmac_md5_bytes, hmac_sha1_bytes, hmac_sha256_bytes, hmac_sha384_bytes, hmac_sha512_bytes,
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
        if let boa_engine::string::JsStrVariant::Latin1(bytes) = s.as_str().variant() {
            if bytes.is_ascii() {
                return Ok(f(bytes));
            }
        }
        let owned = s.to_std_string_escaped();
        return Ok(f(owned.as_bytes()));
    }
    let Some(obj) = val.as_object() else {
        return Ok(f(&[]));
    };
    if let Some(ta) = obj.downcast_ref::<TypedArray>() {
        if let Some(res) = ta.with_bytes(&mut f) {
            return Ok(res);
        }
    }
    if let Ok(ab) = JsArrayBuffer::from_object(obj.clone()) {
        if let Some(data) = ab.data() {
            return Ok(f(&data));
        }
        return Ok(f(&[]));
    }
    if let Some(dv) = obj.downcast_ref::<DataView>() {
        if let Some(res) = dv.with_bytes(&mut f) {
            return Ok(res);
        }
    }
    let fallback = extract_buffer_source_bytes(val, context)?;
    Ok(f(&fallback))
}

/// Invokes `f` with a zero-copy borrowed `&[u8]` slice when `val` is `Some` and neither
/// `undefined` nor `null`; otherwise invokes `f(&[])`.
#[inline]
pub(crate) fn with_optional_buffer_source_bytes<R>(
    val: Option<&boa_engine::JsValue>,
    context: &mut boa_engine::Context,
    f: impl FnMut(&[u8]) -> R,
) -> boa_engine::JsResult<R> {
    let mut f = f;
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

    if let (Some(obj_a), Some(obj_b)) = (a.as_object(), b.as_object()) {
        if let (Some(ta_a), Some(ta_b)) = (
            obj_a.downcast_ref::<TypedArray>(),
            obj_b.downcast_ref::<TypedArray>(),
        ) {
            if let Some(Some(res)) =
                ta_a.with_bytes(|bytes_a| ta_b.with_bytes(|bytes_b| f(bytes_a, bytes_b)))
            {
                return Ok(res);
            }
        }
    }
    let bytes_a = extract_buffer_source_bytes(a, context)?;
    with_buffer_source_bytes(b, context, |bytes_b| f(&bytes_a, bytes_b))
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
    if let Some(ta) = obj.downcast_ref::<TypedArray>() {
        if let Some(bytes) = ta.with_bytes(|slice| slice.to_vec()) {
            return Ok(bytes);
        }
    }
    if let Ok(ab) = JsArrayBuffer::from_object(obj.clone()) {
        if let Some(data) = ab.data() {
            return Ok(data.to_vec());
        }
        return Ok(Vec::new());
    }
    if let Some(dv) = obj.downcast_ref::<DataView>() {
        if let Some(bytes) = dv.with_bytes(|slice| slice.to_vec()) {
            return Ok(bytes);
        }
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
                    let end = len
                        .map(|l| (start + l).min(data.len()))
                        .unwrap_or(data.len());
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
pub(crate) fn compute_webcrypto_hmac_stack(
    hash_name: &str,
    key: &[u8],
    data: &[u8],
) -> Option<([u8; 64], usize)> {
    let mut out = [0u8; 64];
    match normalize_webcrypto_hash_name(hash_name)? {
        "SHA-1" => {
            let d = hmac_sha1_bytes(key, data);
            out[..20].copy_from_slice(&d);
            Some((out, 20))
        }
        "SHA-256" => {
            let d = hmac_sha256_bytes(key, data);
            out[..32].copy_from_slice(&d);
            Some((out, 32))
        }
        "SHA-384" => {
            let d = hmac_sha384_bytes(key, data);
            out[..48].copy_from_slice(&d);
            Some((out, 48))
        }
        "SHA-512" => {
            let d = hmac_sha512_bytes(key, data);
            Some((d, 64))
        }
        "MD5" => {
            let d = hmac_md5_bytes(key, data);
            out[..16].copy_from_slice(&d);
            Some((out, 16))
        }
        _ => None,
    }
}

#[inline]
pub(crate) fn with_active_tenant<R>(f: impl FnOnce(&str) -> R) -> R {
    BOA_ACTIVE_TENANT.with(|t| f(&t.borrow()))
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
        BOA_VIOLATION.with(|v| {
            if v.borrow().is_none() {
                *v.borrow_mut() = Some(SandboxViolation::CpuFuelExhausted { tenant_id, limit });
            }
        });
        return Err(format!(
            "RangeError: Cryptographic KDF CPU fuel budget exhausted (limit: {} units)",
            limit
        ));
    }
    Ok(())
}

/// Charges `added_bytes` against the active isolate's `BOA_KV_MEM_BUDGET` ceiling (`max_memory_bytes`).
/// If the ceiling is exceeded, records `SandboxViolation::MemoryCeilingExceeded` in `BOA_VIOLATION`
/// and returns `Err` with the violation message.
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
        let tenant_id = active_tenant_id_or_default();
        let violation = SandboxViolation::MemoryCeilingExceeded {
            tenant_id,
            requested,
            limit,
        };
        let msg = violation.to_string();
        BOA_VIOLATION.with(|v| {
            if v.borrow().is_none() {
                *v.borrow_mut() = Some(violation);
            }
        });
        return Err(format!("RangeError: {}", msg));
    }
    Ok(())
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
