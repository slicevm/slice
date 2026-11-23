//! Re-Entrant Wasm <-> JS Function Execution, Host Import Trampoline, and Fuel Synchronization.

use super::memory::{sync_all_memories_js_to_wasm, sync_all_memories_wasm_to_js};
use super::store::{
    ACTIVE_BOA_CTX_PTR, ActiveBoaCtxGuard, ActiveCallerGuard, WASM_STATE, WasmErrorKind,
    WasmHostExceptionMarker, WasmStoreData, exec_wasm_with_store, record_wasm_fuel_violation,
    set_pending_host_js_error, take_pending_host_js_error, wasm_js_error, with_store_and_entities,
};
use super::values::{default_val_for_type, js_val_to_wasm_val, wasm_val_to_js_val};
use crate::boa_realm::BOA_FUEL_BUDGET;
use boa_engine::object::builtins::JsArray;
use boa_engine::{Context, JsNativeError, JsResult, JsValue, js_string};
use smallvec::SmallVec;
use wasmi::{AsContext, AsContextMut, Caller, Error as WasmiError, TrapCode, Val, ValType};

#[inline]
pub(super) fn sync_fuel_js_to_wasm(
    mut ctx: impl AsContextMut<Data = WasmStoreData>,
) -> Result<(), ()> {
    let (consumed, limit) = BOA_FUEL_BUDGET.with(|f| f.get());
    let avail = limit.saturating_sub(consumed);
    if avail == 0 {
        return Err(());
    }
    let _ = ctx.as_context_mut().set_fuel(avail);
    Ok(())
}

#[inline]
pub(super) fn sync_fuel_wasm_to_js(ctx: impl AsContext<Data = WasmStoreData>, min_step: u64) {
    let left = ctx.as_context().get_fuel().unwrap_or(0);
    BOA_FUEL_BUDGET.with(|f| {
        let (prev_consumed, limit) = f.get();
        let exact_consumed = limit.saturating_sub(left);
        let next_consumed = exact_consumed
            .max(prev_consumed.saturating_add(min_step))
            .min(limit);
        f.set((next_consumed, limit));
    });
}

pub(super) fn invoke_js_host_import(
    closure_id: u32,
    res_types: &[ValType],
    caller: &mut Caller<'_, WasmStoreData>,
    params: &[Val],
    results: &mut [Val],
) -> Result<(), WasmiError> {
    let ctx_ptr = ACTIVE_BOA_CTX_PTR.with(|p| p.replace(std::ptr::null_mut()));
    if ctx_ptr.is_null() {
        return Err(WasmiError::host(WasmHostExceptionMarker));
    }
    struct RestoreBoaCtxPtr(*mut Context);
    impl Drop for RestoreBoaCtxPtr {
        #[inline]
        fn drop(&mut self) {
            ACTIVE_BOA_CTX_PTR.with(|p| p.set(self.0));
        }
    }
    let _restore_ctx = RestoreBoaCtxPtr(ctx_ptr);

    // 1. Transition Wasm -> JS: sync fuel and flush any Wasm memory writes / native memory.grow
    //    into materialized JS ArrayBuffers before invoking JavaScript.
    sync_fuel_wasm_to_js(caller.as_context(), 0);
    let js_fn_opt = WASM_STATE.with(|st| {
        let mut s = st.borrow_mut();
        sync_all_memories_wasm_to_js(caller.as_context_mut(), &mut s.entities);
        s.entities.import_closures.get(&closure_id).cloned()
    });
    let Some(js_fn) = js_fn_opt else {
        return Err(WasmiError::host(WasmHostExceptionMarker));
    };

    // 2. Push `caller` onto `ACTIVE_CALLER_PTR` so re-entrant Wasm calls, `memory.grow`,
    //    `global.value`, `table.get/set`, and `new WebAssembly.Instance` inside `js_fn`
    //    reuse the active Store borrow without RefCell conflicts.
    let call_outcome: JsResult<()> = {
        let _caller_guard = ActiveCallerGuard::enter(caller);
        // SAFETY:
        // 1. `ctx_ptr` was installed by `ActiveBoaCtxGuard::enter` (`!Send + !Sync`,
        //    `PhantomData<*mut &'a mut Context>`) on this thread from a live exclusive
        //    `&mut boa_engine::Context` in the enclosing `wasm_instantiate_module` or
        //    `wasm_call_func_by_id` frame.
        // 2. Atomically checking out `ACTIVE_BOA_CTX_PTR` via `replace(null_mut())` above ensures
        //    no other code can materialize a second `&mut Context` from `ACTIVE_BOA_CTX_PTR`; any
        //    nested JS -> Wasm re-entry borrows `ctx` exclusively via `js_fn.call(..., ctx)` and
        //    pushes its own scoped `ActiveBoaCtxGuard` derived from that inner borrow.
        // 3. `ctx` is strictly confined to this block and dropped before `_restore_ctx` restores
        //    `ctx_ptr` for subsequent host calls in the parent Wasm frame.
        #[allow(unsafe_code)]
        let ctx = unsafe { &mut *ctx_ptr };
        let mut js_args: SmallVec<[JsValue; 8]> = SmallVec::with_capacity(params.len());
        let mut param_err = None;
        for p in params {
            match wasm_val_to_js_val(p, ctx) {
                Ok(jv) => js_args.push(jv),
                Err(e) => {
                    param_err = Some(e);
                    break;
                }
            }
        }
        if let Some(e) = param_err {
            Err(e)
        } else {
            match js_fn.call(&JsValue::undefined(), &js_args, ctx) {
                Ok(ret_val) => coerce_host_return_values(&ret_val, res_types, results, ctx),
                Err(js_err) => Err(js_err),
            }
        }
    };

    // 3. Unconditionally flush any JS ArrayBuffer writes back into Wasm linear memory (even if
    //    `js_fn` or return coercion threw an exception!), then transition fuel JS -> Wasm.
    WASM_STATE.with(|st| {
        let mut s = st.borrow_mut();
        sync_all_memories_js_to_wasm(caller.as_context_mut(), &mut s.entities);
    });
    let (consumed, limit) = BOA_FUEL_BUDGET.with(|f| f.get());
    let _ = caller.set_fuel(limit.saturating_sub(consumed));

    match call_outcome {
        Ok(()) => Ok(()),
        Err(js_err) => {
            set_pending_host_js_error(js_err);
            Err(WasmiError::host(WasmHostExceptionMarker))
        }
    }
}

#[inline]
fn coerce_host_return_values(
    ret_val: &JsValue,
    res_types: &[ValType],
    results: &mut [Val],
    ctx: &mut Context,
) -> JsResult<()> {
    match res_types.len() {
        0 => Ok(()),
        1 => {
            results[0] = js_val_to_wasm_val(ret_val, res_types[0], ctx)?;
            Ok(())
        }
        n => {
            let Some(obj) = ret_val.as_object() else {
                return Err(JsNativeError::typ()
                    .with_message("WebAssembly multi-value host return must be an iterable object")
                    .into());
            };
            let len_val = obj.get(js_string!("length"), ctx)?;
            let len = len_val.to_length(ctx)? as usize;
            if len != n {
                return Err(JsNativeError::typ()
                    .with_message(format!(
                        "WebAssembly multi-value host return expected {} values, got {}",
                        n, len
                    ))
                    .into());
            }
            for i in 0..n {
                let elem = obj.get(i as u32, ctx)?;
                results[i] = js_val_to_wasm_val(&elem, res_types[i], ctx)?;
            }
            Ok(())
        }
    }
}

/// Invokes a registered `wasmi::Func` by `fn_id` directly from its native Boa `JsFunction`
/// wrapper with zero intermediate JS array/string allocations.
pub(crate) fn wasm_call_func_by_id(
    fn_id: u32,
    js_args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let meta_opt = WASM_STATE.with(|st| {
        let s = st.borrow();
        let entry = s.entities.funcs.get(&fn_id)?;
        Some((
            entry.func,
            entry.param_types.clone(),
            entry.result_types.clone(),
        ))
    });
    let Some((func, param_types, result_types)) = meta_opt else {
        return Err(JsNativeError::typ()
            .with_message("WebAssembly exported function handle is invalid")
            .into());
    };

    // 1. Coerce all JS arguments BEFORE syncing JS ArrayBuffers -> Wasm linear memory so any
    //    side effects inside user `valueOf()` / `Symbol.toPrimitive` methods are captured!
    let mut wasm_params: SmallVec<[Val; 8]> = SmallVec::with_capacity(param_types.len());
    for (idx, pt) in param_types.iter().enumerate() {
        let js_v = js_args.get(idx).cloned().unwrap_or_else(JsValue::undefined);
        wasm_params.push(js_val_to_wasm_val(&js_v, *pt, context)?);
    }
    let mut wasm_results: SmallVec<[Val; 4]> = result_types
        .iter()
        .map(|rt| default_val_for_type(*rt))
        .collect();

    // 2. Sync materialized JS ArrayBuffers -> Wasm linear memory.
    with_store_and_entities(|store_ctx, entities| {
        sync_all_memories_js_to_wasm(store_ctx, entities);
    });

    // 3. Execute Wasm function with fuel metering and re-entrant context guard.
    let call_res = {
        let _boa_guard = ActiveBoaCtxGuard::enter(context);
        exec_wasm_with_store(|mut store_ctx| {
            if sync_fuel_js_to_wasm(store_ctx.as_context_mut()).is_err() {
                return Err(None);
            }
            let res = func.call(&mut store_ctx, &wasm_params, &mut wasm_results);
            sync_fuel_wasm_to_js(store_ctx.as_context(), 1);
            res.map_err(Some)
        })
    };

    // 4. UNCONDITIONALLY sync Wasm linear memory -> materialized JS ArrayBuffers (and detach stale
    //    buffers on native `memory.grow`) on BOTH `Ok` and `Err` (traps / host exceptions)!
    with_store_and_entities(|store_ctx, entities| {
        sync_all_memories_wasm_to_js(store_ctx, entities);
    });

    match call_res {
        Ok(()) => {}
        Err(None) => {
            record_wasm_fuel_violation();
            return Err(wasm_js_error(
                WasmErrorKind::Runtime,
                "CPU instruction fuel exhausted before WebAssembly call",
                context,
            ));
        }
        Err(Some(err)) => {
            if let Some(js_err) = take_pending_host_js_error() {
                return Err(js_err);
            }
            let err_str = err.to_string();
            if err_str.contains("fuel") || err.as_trap_code() == Some(TrapCode::OutOfFuel) {
                record_wasm_fuel_violation();
            }
            return Err(wasm_js_error(
                WasmErrorKind::Runtime,
                format!("WebAssembly trap: {}", err_str),
                context,
            ));
        }
    }

    match wasm_results.len() {
        0 => Ok(JsValue::undefined()),
        1 => wasm_val_to_js_val(&wasm_results[0], context),
        _ => {
            let mut out = Vec::with_capacity(wasm_results.len());
            for v in &wasm_results {
                out.push(wasm_val_to_js_val(v, context)?);
            }
            Ok(JsArray::from_iter(out, context).into())
        }
    }
}
