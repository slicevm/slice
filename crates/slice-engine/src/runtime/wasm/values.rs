//! WebAssembly <-> JavaScript value conversions (`ToWebAssemblyValue` / `ToJSValue` / `ToBigInt64`)
//! and WebIDL descriptor range/type parsers.

use super::linking::ensure_func_js_wrapper;
use super::store::{with_store_and_entities, WasmStoreData, WASM_STATE};
use boa_engine::{js_string, Context, JsBigInt, JsNativeError, JsResult, JsValue};
use wasmi::{AsContext, ExternRef, Nullable, Ref, RefType, StoreContext, Val, ValType};

pub(crate) const HIDDEN_FN_ID_PROP: &str = "__slice_wasm_fn_id";

/// Implements WebIDL `[EnforceRange] unsigned long` conversion:
/// rejects `NaN`, `+-Infinity`, `< 0`, and `> u32::MAX` with `TypeError`.
#[inline]
pub(crate) fn enforce_range_u32(
    val: &JsValue,
    label: &str,
    context: &mut Context,
) -> JsResult<u32> {
    let n = val.to_number(context)?;
    if !n.is_finite() {
        return Err(JsNativeError::typ()
            .with_message(format!(
                "WebAssembly {}: value must be a finite number",
                label
            ))
            .into());
    }
    let trunc = n.trunc();
    if trunc < 0.0 || trunc > (u32::MAX as f64) {
        return Err(JsNativeError::typ()
            .with_message(format!(
                "WebAssembly {}: value {} is outside unsigned 32-bit range",
                label, trunc
            ))
            .into());
    }
    Ok(trunc as u32)
}

#[inline]
pub(crate) fn parse_val_type(s: &str) -> Option<ValType> {
    match s {
        "i32" => Some(ValType::I32),
        "i64" => Some(ValType::I64),
        "f32" => Some(ValType::F32),
        "f64" => Some(ValType::F64),
        "funcref" | "anyfunc" => Some(ValType::FuncRef),
        "externref" => Some(ValType::ExternRef),
        _ => None,
    }
}

#[inline]
pub(crate) fn parse_ref_type(s: &str) -> Option<RefType> {
    match s {
        "funcref" | "anyfunc" => Some(RefType::Func),
        "externref" => Some(RefType::Extern),
        _ => None,
    }
}

#[inline]
pub(crate) fn default_val_for_type(vt: ValType) -> Val {
    Val::default_for_ty(vt)
}

/// Converts a JavaScript value into a `wasmi::Ref` according to W3C WebAssembly JS-API 2.0
/// `ToWebAssemblyValue`.
pub(crate) fn js_val_to_wasm_ref(
    val: &JsValue,
    rt: RefType,
    context: &mut Context,
) -> JsResult<Ref> {
    match rt {
        RefType::Func => {
            if val.is_null() {
                return Ok(Ref::Func(Nullable::Null));
            }
            if let Some(obj) = val.as_object() {
                if let Ok(id_val) = obj.get(js_string!(HIDDEN_FN_ID_PROP), context) {
                    if let Some(fn_id) = id_val.as_number().map(|n| n as u32).filter(|&id| id > 0) {
                        let func_opt = WASM_STATE
                            .with(|st| st.borrow().entities.funcs.get(&fn_id).map(|f| f.func));
                        if let Some(func) = func_opt {
                            return Ok(Ref::Func(Nullable::Val(func)));
                        }
                    }
                }
            }
            Err(JsNativeError::typ()
                .with_message(
                    "WebAssembly funcref value must be null or an exported WebAssembly function",
                )
                .into())
        }
        RefType::Extern => {
            if val.is_null() {
                return Ok(Ref::Extern(Nullable::Null));
            }
            let ext = with_store_and_entities(|mut store_ctx, entities| {
                let slot = entities.alloc_id();
                entities.externref_values.insert(slot, val.clone());
                ExternRef::new::<u32>(&mut store_ctx, slot)
            });
            Ok(Ref::Extern(Nullable::Val(ext)))
        }
    }
}

/// Converts a JavaScript value into a `wasmi::Val` according to W3C WebAssembly JS-API 2.0
/// `ToWebAssemblyValue`:
/// - `i32`: `ToInt32(v)`
/// - `i64`: `ToBigInt64(v)` (rejects `Number`, `undefined`, `null`, `Symbol` with `TypeError`;
///   reduces arbitrary-precision `BigInt` modulo `2^64` in signed two's complement)
/// - `f32`: `ToNumber(v) as f32`
/// - `f64`: `ToNumber(v)`
/// - `funcref` / `externref`: `js_val_to_wasm_ref`
#[inline]
pub(crate) fn js_val_to_wasm_val(
    val: &JsValue,
    vt: ValType,
    context: &mut Context,
) -> JsResult<Val> {
    match vt {
        ValType::I32 => Ok(Val::I32(val.to_i32(context)?)),
        ValType::I64 => Ok(Val::I64(val.to_big_int64(context)?)),
        ValType::F32 => Ok(Val::F32((val.to_number(context)? as f32).into())),
        ValType::F64 => Ok(Val::F64(val.to_number(context)?.into())),
        ValType::FuncRef => match js_val_to_wasm_ref(val, RefType::Func, context)? {
            Ref::Func(f) => Ok(Val::FuncRef(f)),
            _ => unreachable!(),
        },
        ValType::ExternRef => match js_val_to_wasm_ref(val, RefType::Extern, context)? {
            Ref::Extern(e) => Ok(Val::ExternRef(e)),
            _ => unreachable!(),
        },
        ValType::V128 => Err(JsNativeError::typ()
            .with_message("WebAssembly v128 values cannot be converted directly from JavaScript")
            .into()),
    }
}

/// Converts a `wasmi::Ref` into a JavaScript value (`ToJSValue`).
pub(crate) fn wasm_ref_to_js_val(r: Ref, context: &mut Context) -> JsResult<JsValue> {
    match r {
        Ref::Func(Nullable::Null) | Ref::Extern(Nullable::Null) => Ok(JsValue::null()),
        Ref::Func(Nullable::Val(func)) => {
            let fn_id = with_store_and_entities(|store_ctx, entities| {
                super::linking::register_or_find_func(func, None, store_ctx.as_context(), entities)
            });
            let fn_obj = ensure_func_js_wrapper(fn_id, context)?;
            Ok(fn_obj.into())
        }
        Ref::Extern(Nullable::Val(ext)) => {
            let found = with_store_and_entities(|store_ctx, entities| {
                let sc: StoreContext<'_, WasmStoreData> = store_ctx.as_context();
                ext.data(sc)
                    .downcast_ref::<u32>()
                    .and_then(|slot| entities.externref_values.get(slot).cloned())
            });
            Ok(found.unwrap_or_else(JsValue::null))
        }
    }
}

/// Fast scalar `ToJSValue` conversion for numeric `Val`s without touching `Context` or `Store`,
/// delegating reference types (`FuncRef` / `ExternRef`) to `wasm_ref_to_js_val`.
#[inline]
pub(crate) fn wasm_val_to_js_val(val: &Val, context: &mut Context) -> JsResult<JsValue> {
    match val {
        Val::I32(v) => Ok(JsValue::from(*v)),
        Val::I64(v) => Ok(JsValue::from(JsBigInt::from(*v))),
        Val::F32(v) => Ok(JsValue::from(f32::from(*v) as f64)),
        Val::F64(v) => Ok(JsValue::from(f64::from(*v))),
        Val::FuncRef(f) => wasm_ref_to_js_val(Ref::Func(*f), context),
        Val::ExternRef(e) => wasm_ref_to_js_val(Ref::Extern(*e), context),
        Val::V128(_) => Err(JsNativeError::typ()
            .with_message("WebAssembly v128 return values cannot be converted to JavaScript")
            .into()),
    }
}
