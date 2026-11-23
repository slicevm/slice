//! WebAssembly Module Linking (`LinkError` Validation), Instantiation, and Canonical `exports`.

use super::call::{invoke_js_host_import, sync_fuel_js_to_wasm, sync_fuel_wasm_to_js};
use super::globals_and_tables::{
    ensure_func_js_wrapper, ensure_global_js_wrapper, ensure_table_js_wrapper,
    register_or_find_func, register_or_find_global, register_or_find_table,
};
use super::memory::{
    ensure_memory_js_wrapper, register_or_find_memory, sync_all_memories_js_to_wasm,
    sync_all_memories_wasm_to_js,
};
use super::store::{
    ActiveBoaCtxGuard, WasmErrorKind, WasmStoreData, exec_wasm_with_store,
    record_wasm_fuel_violation, take_pending_host_js_error, wasm_js_error, with_store_and_entities,
};
use super::values::{HIDDEN_FN_ID_PROP, js_val_to_wasm_val};
use super::{get_compiled_module, global_wasm_engine};
use boa_engine::property::PropertyDescriptor;
use boa_engine::{Context, JsNativeError, JsObject, JsResult, JsValue, js_string};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use wasmi::{
    AsContext, AsContextMut, Caller, Error as WasmiError, Extern, ExternType, Func, Global, Linker,
    Mutability, Table, TrapCode, Val, ValType,
};

/// Instantiates a compiled `mod_id` into the shared isolate `wasmi::Store`, validating all imports
/// and returning a frozen `exports` object preserving canonical `Memory`/`Global`/`Table`/`Func`
/// identity.
pub fn wasm_instantiate_module(
    mod_id: u32,
    import_obj_opt: Option<&JsObject>,
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(mod_entry) = get_compiled_module(mod_id) else {
        return Err(JsNativeError::typ()
            .with_message("WebAssembly.Instance(): Argument 0 must be a WebAssembly.Module")
            .into());
    };
    let module = &mod_entry.module;
    let export_indices = &mod_entry.export_indices;

    let mut linker = <Linker<WasmStoreData>>::new(global_wasm_engine());
    let mut imported_func_ids: Vec<u32> = Vec::new();
    let mut imported_mem_ids: Vec<u32> = Vec::new();
    let mut imported_glob_ids: Vec<u32> = Vec::new();
    let mut imported_tbl_ids: Vec<u32> = Vec::new();

    for imp in module.imports() {
        let mod_name = imp.module();
        let field_name = imp.name();
        let Some(imp_root) = import_obj_opt else {
            return Err(JsNativeError::typ()
                .with_message(format!(
                    "WebAssembly.Instance(): imports argument must be present for module requiring '{}.{}'",
                    mod_name, field_name
                ))
                .into());
        };
        let ns_val = imp_root.get(js_string!(mod_name), context)?;
        let Some(ns_obj) = ns_val.as_object() else {
            return Err(wasm_js_error(
                WasmErrorKind::Link,
                format!(
                    "WebAssembly.Instance(): import module '{}' is not an object",
                    mod_name
                ),
                context,
            ));
        };
        let field_val = ns_obj.get(js_string!(field_name), context)?;

        match imp.ty() {
            ExternType::Func(func_ty) => {
                let Some(fn_obj) = field_val.as_object().filter(|o| o.is_callable()) else {
                    return Err(wasm_js_error(
                        WasmErrorKind::Link,
                        format!(
                            "WebAssembly.Instance(): import '{}.{}' is not a Function",
                            mod_name, field_name
                        ),
                        context,
                    ));
                };

                // Fast path: if importing an already-exported Wasm function from this Store,
                // link the underlying native `wasmi::Func` directly with zero JS trampoline!
                let existing_wasm_fn = fn_obj
                    .get(js_string!(HIDDEN_FN_ID_PROP), context)
                    .ok()
                    .and_then(|v| v.as_number())
                    .map(|n| n as u32)
                    .filter(|&id| id > 0)
                    .and_then(|id| {
                        with_store_and_entities(|store_ctx, entities| {
                            let f = entities.funcs.get(&id)?.func;
                            Some((id, f, f.ty(&store_ctx)))
                        })
                    });

                if let Some((fn_id, native_func, actual_ty)) = existing_wasm_fn {
                    if &actual_ty != func_ty {
                        return Err(wasm_js_error(
                            WasmErrorKind::Link,
                            format!(
                                "WebAssembly.Instance(): imported function '{}.{}' signature mismatch",
                                mod_name, field_name
                            ),
                            context,
                        ));
                    }
                    linker
                        .define(mod_name, field_name, native_func)
                        .map_err(|e| wasm_js_error(WasmErrorKind::Link, e.to_string(), context))?;
                    imported_func_ids.push(fn_id);
                } else {
                    let f_ty = func_ty.clone();
                    let res_types: SmallVec<[ValType; 4]> = SmallVec::from_slice(f_ty.results());
                    let closure_id = with_store_and_entities(|_store_ctx, entities| {
                        let cid = entities.alloc_id();
                        entities.import_closures.insert(cid, fn_obj.clone());
                        cid
                    });
                    let (fn_id, host_func) = with_store_and_entities(|mut store_ctx, entities| {
                        let hf = Func::new(
                            &mut store_ctx,
                            f_ty,
                            move |mut caller: Caller<'_, WasmStoreData>,
                                  params: &[Val],
                                  results: &mut [Val]|
                                  -> Result<(), WasmiError> {
                                invoke_js_host_import(
                                    closure_id,
                                    &res_types,
                                    &mut caller,
                                    params,
                                    results,
                                )
                            },
                        );
                        let id = register_or_find_func(hf, None, store_ctx.as_context(), entities);
                        (id, hf)
                    });
                    linker
                        .define(mod_name, field_name, host_func)
                        .map_err(|e| wasm_js_error(WasmErrorKind::Link, e.to_string(), context))?;
                    imported_func_ids.push(fn_id);
                }
            }
            ExternType::Memory(req_mem_ty) => {
                let mem_id_opt = field_val
                    .as_object()
                    .and_then(|o| o.get(js_string!("_memId"), context).ok())
                    .and_then(|v| v.as_number())
                    .map(|n| n as u32)
                    .filter(|&id| id > 0);
                let Some(mem_id) = mem_id_opt else {
                    return Err(wasm_js_error(
                        WasmErrorKind::Link,
                        format!(
                            "WebAssembly.Instance(): import '{}.{}' is not a WebAssembly.Memory",
                            mod_name, field_name
                        ),
                        context,
                    ));
                };
                let mem = with_store_and_entities(|store_ctx, entities| -> JsResult<_> {
                    let Some(entry) = entities.memories.get(&mem_id) else {
                        return Err(wasm_js_error(
                            WasmErrorKind::Link,
                            format!(
                                "WebAssembly.Instance(): import '{}.{}' is not a valid WebAssembly.Memory",
                                mod_name, field_name
                            ),
                            context,
                        ));
                    };
                    let cur_pages = entry.memory.size(&store_ctx);
                    let actual_ty = entry.memory.ty(&store_ctx);
                    if cur_pages < req_mem_ty.minimum() {
                        return Err(wasm_js_error(
                            WasmErrorKind::Link,
                            format!(
                                "Imported WebAssembly.Memory size ({} pages) is smaller than required minimum ({} pages)",
                                cur_pages,
                                req_mem_ty.minimum()
                            ),
                            context,
                        ));
                    }
                    if let Some(req_max) = req_mem_ty.maximum() {
                        match actual_ty.maximum() {
                            Some(act_max) if act_max <= req_max => {}
                            _ => {
                                return Err(wasm_js_error(
                                    WasmErrorKind::Link,
                                    format!(
                                        "Imported WebAssembly.Memory maximum exceeds required maximum ({} pages)",
                                        req_max
                                    ),
                                    context,
                                ));
                            }
                        }
                    }
                    Ok(entry.memory)
                })?;
                linker
                    .define(mod_name, field_name, mem)
                    .map_err(|e| wasm_js_error(WasmErrorKind::Link, e.to_string(), context))?;
                imported_mem_ids.push(mem_id);
            }
            ExternType::Global(req_glob_ty) => {
                let glob_id_opt = field_val
                    .as_object()
                    .and_then(|o| o.get(js_string!("_globId"), context).ok())
                    .and_then(|v| v.as_number())
                    .map(|n| n as u32)
                    .filter(|&id| id > 0);

                if let Some(glob_id) = glob_id_opt {
                    let g = with_store_and_entities(|store_ctx, entities| -> JsResult<Global> {
                        let Some(entry) = entities.globals.get(&glob_id) else {
                            return Err(wasm_js_error(
                                WasmErrorKind::Link,
                                format!(
                                    "WebAssembly.Instance(): import '{}.{}' is not a valid WebAssembly.Global",
                                    mod_name, field_name
                                ),
                                context,
                            ));
                        };
                        let act_ty = entry.global.ty(&store_ctx);
                        if act_ty != *req_glob_ty {
                            return Err(wasm_js_error(
                                WasmErrorKind::Link,
                                format!(
                                    "WebAssembly.Instance(): imported Global '{}.{}' type/mutability mismatch",
                                    mod_name, field_name
                                ),
                                context,
                            ));
                        }
                        Ok(entry.global)
                    })?;
                    linker
                        .define(mod_name, field_name, g)
                        .map_err(|e| wasm_js_error(WasmErrorKind::Link, e.to_string(), context))?;
                    imported_glob_ids.push(glob_id);
                } else {
                    // Raw JS value import: only permitted for immutable globals (`Mutability::Const`).
                    if matches!(req_glob_ty.mutability(), Mutability::Var) {
                        return Err(wasm_js_error(
                            WasmErrorKind::Link,
                            format!(
                                "WebAssembly.Instance(): mutable Global import '{}.{}' must be a WebAssembly.Global object",
                                mod_name, field_name
                            ),
                            context,
                        ));
                    }
                    let vt = req_glob_ty.content();
                    match vt {
                        ValType::I64 => {
                            if !field_val.is_bigint() {
                                return Err(wasm_js_error(
                                    WasmErrorKind::Link,
                                    format!(
                                        "WebAssembly.Instance(): immutable i64 Global import '{}.{}' must be a BigInt",
                                        mod_name, field_name
                                    ),
                                    context,
                                ));
                            }
                        }
                        ValType::I32 | ValType::F32 | ValType::F64 => {
                            if field_val.as_number().is_none() {
                                return Err(wasm_js_error(
                                    WasmErrorKind::Link,
                                    format!(
                                        "WebAssembly.Instance(): immutable numeric Global import '{}.{}' must be a Number",
                                        mod_name, field_name
                                    ),
                                    context,
                                ));
                            }
                        }
                        ValType::V128 => {
                            return Err(wasm_js_error(
                                WasmErrorKind::Link,
                                "v128 global imports cannot be initialized from a JS primitive",
                                context,
                            ));
                        }
                        ValType::FuncRef | ValType::ExternRef => {}
                    }
                    let wv = js_val_to_wasm_val(&field_val, vt, context)?;
                    let (glob_id, g) = with_store_and_entities(|mut store_ctx, entities| {
                        let g = Global::new(&mut store_ctx, wv, Mutability::Const);
                        let id = register_or_find_global(g, None, entities);
                        (id, g)
                    });
                    linker
                        .define(mod_name, field_name, g)
                        .map_err(|e| wasm_js_error(WasmErrorKind::Link, e.to_string(), context))?;
                    imported_glob_ids.push(glob_id);
                }
            }
            ExternType::Table(req_tab_ty) => {
                let tbl_id_opt = field_val
                    .as_object()
                    .and_then(|o| o.get(js_string!("_tblId"), context).ok())
                    .and_then(|v| v.as_number())
                    .map(|n| n as u32)
                    .filter(|&id| id > 0);
                let Some(tbl_id) = tbl_id_opt else {
                    return Err(wasm_js_error(
                        WasmErrorKind::Link,
                        format!(
                            "WebAssembly.Instance(): import '{}.{}' is not a WebAssembly.Table",
                            mod_name, field_name
                        ),
                        context,
                    ));
                };
                let tbl = with_store_and_entities(|store_ctx, entities| -> JsResult<Table> {
                    let Some(entry) = entities.tables.get(&tbl_id) else {
                        return Err(wasm_js_error(
                            WasmErrorKind::Link,
                            format!(
                                "WebAssembly.Instance(): import '{}.{}' is not a valid WebAssembly.Table",
                                mod_name, field_name
                            ),
                            context,
                        ));
                    };
                    let act_ty = entry.table.ty(&store_ctx);
                    let cur_len = entry.table.size(&store_ctx);
                    if act_ty.element() != req_tab_ty.element() || cur_len < req_tab_ty.minimum() {
                        return Err(wasm_js_error(
                            WasmErrorKind::Link,
                            format!(
                                "WebAssembly.Instance(): imported Table '{}.{}' element type or size mismatch",
                                mod_name, field_name
                            ),
                            context,
                        ));
                    }
                    if let Some(req_max) = req_tab_ty.maximum() {
                        match act_ty.maximum() {
                            Some(act_max) if act_max <= req_max => {}
                            _ => {
                                return Err(wasm_js_error(
                                    WasmErrorKind::Link,
                                    format!(
                                        "WebAssembly.Instance(): imported Table '{}.{}' maximum exceeds required maximum",
                                        mod_name, field_name
                                    ),
                                    context,
                                ));
                            }
                        }
                    }
                    Ok(entry.table)
                })?;
                linker
                    .define(mod_name, field_name, tbl)
                    .map_err(|e| wasm_js_error(WasmErrorKind::Link, e.to_string(), context))?;
                imported_tbl_ids.push(tbl_id);
            }
        }
    }

    // Flush any pending JS writes on imported memories prior to active data segments and `(start)`.
    with_store_and_entities(|store_ctx, entities| {
        sync_all_memories_js_to_wasm(store_ctx, entities);
    });

    let inst_res = {
        let _boa_guard = ActiveBoaCtxGuard::enter(context);
        exec_wasm_with_store(|mut store_ctx| {
            if sync_fuel_js_to_wasm(store_ctx.as_context_mut()).is_err() {
                return Err(None);
            }
            let res = linker.instantiate_and_start(&mut store_ctx, module);
            sync_fuel_wasm_to_js(store_ctx.as_context(), 5);
            res.map_err(Some)
        })
    };

    // Unconditionally sync all materialized memories (e.g. imported memory written by data segments
    // or by a trapping `(start)` function) back to JS before handling errors!
    with_store_and_entities(|store_ctx, entities| {
        sync_all_memories_wasm_to_js(store_ctx, entities);
    });

    let instance = match inst_res {
        Ok(inst) => inst,
        Err(None) => {
            record_wasm_fuel_violation();
            return Err(wasm_js_error(
                WasmErrorKind::Runtime,
                "CPU instruction fuel exhausted before WebAssembly.instantiate",
                context,
            ));
        }
        Err(Some(e)) => {
            if let Some(js_err) = take_pending_host_js_error() {
                return Err(js_err);
            }
            let err_str = e.to_string();
            if err_str.contains("fuel") || e.as_trap_code() == Some(TrapCode::OutOfFuel) {
                record_wasm_fuel_violation();
            }
            // Memory/table allocation denial during instantiation surfaces as RangeError per spec;
            // link mismatches surface as LinkError; start/elem/data traps surface as RuntimeError.
            let kind = match e.kind() {
                wasmi::errors::ErrorKind::Linker(_) => WasmErrorKind::Link,
                wasmi::errors::ErrorKind::Memory(_) => {
                    return Err(JsNativeError::range().with_message(err_str).into());
                }
                wasmi::errors::ErrorKind::Instantiation(_)
                    if err_str.contains("incompatible") || err_str.contains("import") =>
                {
                    WasmErrorKind::Link
                }
                _ => WasmErrorKind::Runtime,
            };
            return Err(wasm_js_error(kind, err_str, context));
        }
    };

    // Seed per-instance index-space maps with imported IDs so re-exported imports and duplicate
    // export aliases resolve to the exact canonical entity ID and JS wrapper object.
    let mut local_func_ids: FxHashMap<u32, u32> = imported_func_ids
        .into_iter()
        .enumerate()
        .map(|(i, id)| (i as u32, id))
        .collect();
    let mut local_mem_ids: FxHashMap<u32, u32> = imported_mem_ids
        .into_iter()
        .enumerate()
        .map(|(i, id)| (i as u32, id))
        .collect();
    let mut local_glob_ids: FxHashMap<u32, u32> = imported_glob_ids
        .into_iter()
        .enumerate()
        .map(|(i, id)| (i as u32, id))
        .collect();
    let mut local_tbl_ids: FxHashMap<u32, u32> = imported_tbl_ids
        .into_iter()
        .enumerate()
        .map(|(i, id)| (i as u32, id))
        .collect();

    #[derive(Clone, Copy)]
    enum ResolvedExportKind {
        Func(u32),
        Mem(u32),
        Glob(u32),
        Tbl(u32),
    }

    let resolved_exports: Vec<(String, ResolvedExportKind)> =
        with_store_and_entities(|store_ctx, entities| {
            let inst_id = entities.alloc_id();
            entities.instances.insert(inst_id, instance);

            let mut out = Vec::new();
            for (exp_idx, exp) in instance.exports(&store_ctx).enumerate() {
                let name = exp.name().to_string();
                let item_idx_opt = export_indices
                    .get(exp_idx)
                    .filter(|(n, _, _)| n == &name)
                    .map(|(_, _, idx)| *idx);

                let kind = match exp.into_extern() {
                    Extern::Func(f) => {
                        let id = if let Some(idx) = item_idx_opt {
                            if let Some(&existing) = local_func_ids.get(&idx) {
                                existing
                            } else {
                                let new_id = register_or_find_func(
                                    f,
                                    Some(idx.to_string()),
                                    store_ctx.as_context(),
                                    entities,
                                );
                                local_func_ids.insert(idx, new_id);
                                new_id
                            }
                        } else {
                            register_or_find_func(f, None, store_ctx.as_context(), entities)
                        };
                        ResolvedExportKind::Func(id)
                    }
                    Extern::Memory(m) => {
                        let id = if let Some(idx) = item_idx_opt {
                            if let Some(&existing) = local_mem_ids.get(&idx) {
                                existing
                            } else {
                                let new_id = register_or_find_memory(
                                    m,
                                    None,
                                    store_ctx.as_context(),
                                    entities,
                                );
                                local_mem_ids.insert(idx, new_id);
                                new_id
                            }
                        } else {
                            register_or_find_memory(m, None, store_ctx.as_context(), entities)
                        };
                        ResolvedExportKind::Mem(id)
                    }
                    Extern::Global(g) => {
                        let id = if let Some(idx) = item_idx_opt {
                            if let Some(&existing) = local_glob_ids.get(&idx) {
                                existing
                            } else {
                                let new_id = register_or_find_global(g, None, entities);
                                local_glob_ids.insert(idx, new_id);
                                new_id
                            }
                        } else {
                            register_or_find_global(g, None, entities)
                        };
                        ResolvedExportKind::Glob(id)
                    }
                    Extern::Table(t) => {
                        let id = if let Some(idx) = item_idx_opt {
                            if let Some(&existing) = local_tbl_ids.get(&idx) {
                                existing
                            } else {
                                let new_id = register_or_find_table(t, None, entities);
                                local_tbl_ids.insert(idx, new_id);
                                new_id
                            }
                        } else {
                            register_or_find_table(t, None, entities)
                        };
                        ResolvedExportKind::Tbl(id)
                    }
                };
                out.push((name, kind));
            }
            out
        });

    // Per WebAssembly JS-API `CreateExportsObject`: exports object has `null` prototype and
    // enumerable, non-writable, non-configurable properties (frozen).
    let exports_obj = JsObject::with_null_proto();
    for (name, kind) in resolved_exports {
        let js_obj = match kind {
            ResolvedExportKind::Func(id) => ensure_func_js_wrapper(id, context)?,
            ResolvedExportKind::Mem(id) => ensure_memory_js_wrapper(id, context)?,
            ResolvedExportKind::Glob(id) => ensure_global_js_wrapper(id, context)?,
            ResolvedExportKind::Tbl(id) => ensure_table_js_wrapper(id, context)?,
        };
        let _ = exports_obj.define_property_or_throw(
            js_string!(name),
            PropertyDescriptor::builder()
                .value(JsValue::from(js_obj))
                .writable(false)
                .enumerable(true)
                .configurable(false)
                .build(),
            context,
        );
    }
    Ok(exports_obj.into())
}
