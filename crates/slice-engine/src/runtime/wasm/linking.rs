//! Store-backed `WebAssembly.Global`, `WebAssembly.Table`, exported `WebAssembly.Instance`
//! functions, module linking (`LinkError` validation), and re-entrant Wasm call execution.

use super::memory::{
    ensure_memory_js_wrapper, register_or_find_memory, sync_all_memories_js_to_wasm,
    sync_all_memories_wasm_to_js,
};
use super::store::{
    active_boa_ctx_ptr, construct_wasm_wrapper, exec_wasm_with_store, record_wasm_fuel_violation,
    set_pending_host_js_error, take_pending_host_js_error, wasm_js_error, with_store_and_entities,
    ActiveBoaCtxGuard, ActiveCallerGuard, WasmEntities, WasmErrorKind, WasmHostExceptionMarker,
    WasmStoreData, WASM_STATE,
};
use super::values::{
    default_val_for_type, enforce_range_u32, js_val_to_wasm_ref, js_val_to_wasm_val,
    parse_ref_type, parse_val_type, wasm_ref_to_js_val, wasm_val_to_js_val, HIDDEN_FN_ID_PROP,
};
use super::{get_compiled_module, global_wasm_engine};
use crate::boa_realm::BOA_FUEL_BUDGET;
use boa_engine::object::builtins::JsArray;
use boa_engine::object::FunctionObjectBuilder;
use boa_engine::property::PropertyDescriptor;
use boa_engine::{js_string, Context, JsNativeError, JsObject, JsResult, JsValue, NativeFunction};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use wasmi::{
    AsContext, AsContextMut, Caller, Error as WasmiError, Extern, ExternType, Func, Global, Linker,
    Mutability, Ref, StoreContext, Table, TableType, TrapCode, Val, ValType,
};

pub(crate) struct ManagedWasmGlobal {
    pub(crate) global: Global,
    pub(crate) js_wrapper: Option<JsObject>,
}

pub(crate) struct ManagedWasmTable {
    pub(crate) table: Table,
    pub(crate) js_wrapper: Option<JsObject>,
}

pub(crate) struct ManagedWasmFunc {
    pub(crate) func: Func,
    pub(crate) param_types: SmallVec<[ValType; 8]>,
    pub(crate) result_types: SmallVec<[ValType; 4]>,
    pub(crate) func_name: String,
    pub(crate) js_wrapper: Option<JsObject>,
}

#[inline]
fn sync_fuel_js_to_wasm(mut ctx: impl AsContextMut<Data = WasmStoreData>) -> Result<(), ()> {
    let (consumed, limit) = BOA_FUEL_BUDGET.with(|f| f.get());
    let avail = limit.saturating_sub(consumed);
    if avail == 0 {
        return Err(());
    }
    let _ = ctx.as_context_mut().set_fuel(avail);
    Ok(())
}

#[inline]
fn sync_fuel_wasm_to_js(ctx: impl AsContext<Data = WasmStoreData>, min_step: u64) {
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

#[inline]
pub(crate) fn register_or_find_global(
    global: Global,
    js_wrapper: Option<JsObject>,
    entities: &mut WasmEntities,
) -> u32 {
    let key = format!("{:?}", global);
    if let Some(&existing_id) = entities.global_by_key.get(&key) {
        if let Some(w) = js_wrapper {
            if let Some(entry) = entities.globals.get_mut(&existing_id) {
                if entry.js_wrapper.is_none() {
                    entry.js_wrapper = Some(w);
                }
            }
        }
        return existing_id;
    }
    let id = entities.alloc_id();
    entities.global_by_key.insert(key, id);
    entities
        .globals
        .insert(id, ManagedWasmGlobal { global, js_wrapper });
    id
}

pub(crate) fn ensure_global_js_wrapper(glob_id: u32, context: &mut Context) -> JsResult<JsObject> {
    let existing = with_store_and_entities(|_store_ctx, entities| {
        entities
            .globals
            .get(&glob_id)
            .and_then(|e| e.js_wrapper.clone())
    });
    if let Some(obj) = existing {
        return Ok(obj);
    }
    let wrapper = construct_wasm_wrapper("Global", "_globId", glob_id, false, context)?;
    with_store_and_entities(|_store_ctx, entities| {
        if let Some(entry) = entities.globals.get_mut(&glob_id) {
            entry.js_wrapper = Some(wrapper.clone());
        }
    });
    Ok(wrapper)
}

#[inline]
pub(crate) fn register_or_find_table(
    table: Table,
    js_wrapper: Option<JsObject>,
    entities: &mut WasmEntities,
) -> u32 {
    let key = format!("{:?}", table);
    if let Some(&existing_id) = entities.table_by_key.get(&key) {
        if let Some(w) = js_wrapper {
            if let Some(entry) = entities.tables.get_mut(&existing_id) {
                if entry.js_wrapper.is_none() {
                    entry.js_wrapper = Some(w);
                }
            }
        }
        return existing_id;
    }
    let id = entities.alloc_id();
    entities.table_by_key.insert(key, id);
    entities
        .tables
        .insert(id, ManagedWasmTable { table, js_wrapper });
    id
}

pub(crate) fn ensure_table_js_wrapper(tbl_id: u32, context: &mut Context) -> JsResult<JsObject> {
    let existing = with_store_and_entities(|_store_ctx, entities| {
        entities
            .tables
            .get(&tbl_id)
            .and_then(|e| e.js_wrapper.clone())
    });
    if let Some(obj) = existing {
        return Ok(obj);
    }
    let wrapper = construct_wasm_wrapper("Table", "_tblId", tbl_id, false, context)?;
    with_store_and_entities(|_store_ctx, entities| {
        if let Some(entry) = entities.tables.get_mut(&tbl_id) {
            entry.js_wrapper = Some(wrapper.clone());
        }
    });
    Ok(wrapper)
}

#[inline]
pub(crate) fn register_or_find_func(
    func: Func,
    preferred_name: Option<String>,
    store_ctx: StoreContext<'_, WasmStoreData>,
    entities: &mut WasmEntities,
) -> u32 {
    let key = format!("{:?}", func);
    if let Some(&existing_id) = entities.func_by_key.get(&key) {
        return existing_id;
    }
    let f_ty = func.ty(store_ctx);
    let param_types: SmallVec<[ValType; 8]> = SmallVec::from_slice(f_ty.params());
    let result_types: SmallVec<[ValType; 4]> = SmallVec::from_slice(f_ty.results());
    let id = entities.alloc_id();
    let func_name = preferred_name.unwrap_or_else(|| id.to_string());
    entities.func_by_key.insert(key, id);
    entities.funcs.insert(
        id,
        ManagedWasmFunc {
            func,
            param_types,
            result_types,
            func_name,
            js_wrapper: None,
        },
    );
    id
}

pub(crate) fn ensure_func_js_wrapper(fn_id: u32, context: &mut Context) -> JsResult<JsObject> {
    let existing = with_store_and_entities(
        |_store_ctx, entities| -> Option<Result<JsObject, (usize, String)>> {
            let entry = entities.funcs.get(&fn_id)?;
            if let Some(ref w) = entry.js_wrapper {
                Some(Ok(w.clone()))
            } else {
                Some(Err((entry.param_types.len(), entry.func_name.clone())))
            }
        },
    );
    let (arity, func_name) = match existing {
        Some(Ok(obj)) => return Ok(obj),
        Some(Err(meta)) => meta,
        None => {
            return Err(JsNativeError::typ()
                .with_message("Invalid WebAssembly exported function handle")
                .into());
        }
    };

    let js_fn = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_copy_closure(move |_this, args, ctx| {
            wasm_call_func_by_id(fn_id, args, ctx)
        }),
    )
    .name(js_string!(func_name))
    .length(arity)
    .constructor(false)
    .build();

    let fn_obj: JsObject = js_fn.into();
    let _ = fn_obj.define_property_or_throw(
        js_string!(HIDDEN_FN_ID_PROP),
        PropertyDescriptor::builder()
            .value(JsValue::from(fn_id as f64))
            .writable(false)
            .enumerable(false)
            .configurable(false)
            .build(),
        context,
    );

    with_store_and_entities(|_store_ctx, entities| {
        if let Some(entry) = entities.funcs.get_mut(&fn_id) {
            entry.js_wrapper = Some(fn_obj.clone());
        }
    });
    Ok(fn_obj)
}

/// Allocates a real `wasmi::Global` in the isolate's shared `Store` for `new WebAssembly.Global`.
pub fn wasm_create_global(
    descriptor_val: &JsValue,
    init_val: Option<&JsValue>,
    js_this_opt: Option<&JsObject>,
    context: &mut Context,
) -> JsResult<u32> {
    let Some(desc_obj) = descriptor_val.as_object() else {
        return Err(JsNativeError::typ()
            .with_message("WebAssembly.Global(): Descriptor must be an object")
            .into());
    };
    let val_ty_prop = desc_obj.get(js_string!("value"), context)?;
    let val_ty_str = val_ty_prop.to_string(context)?.to_std_string_escaped();
    let Some(vt) = parse_val_type(&val_ty_str) else {
        return Err(JsNativeError::typ()
            .with_message(format!(
                "WebAssembly.Global(): Unsupported value type '{}'",
                val_ty_str
            ))
            .into());
    };
    let mutable = desc_obj.get(js_string!("mutable"), context)?.to_boolean();

    let wv = match init_val {
        None => default_val_for_type(vt),
        Some(v) if v.is_undefined() && vt != ValType::ExternRef => default_val_for_type(vt),
        Some(v) => js_val_to_wasm_val(v, vt, context)?,
    };

    let mutability = if mutable {
        Mutability::Var
    } else {
        Mutability::Const
    };
    let id = with_store_and_entities(|mut store_ctx, entities| {
        let g = Global::new(&mut store_ctx, wv, mutability);
        register_or_find_global(g, js_this_opt.cloned(), entities)
    });
    Ok(id)
}

/// Reads a `WebAssembly.Global` value from the shared `Store`.
pub fn wasm_get_global(glob_id: u32, context: &mut Context) -> JsResult<JsValue> {
    let val_opt = with_store_and_entities(|store_ctx, entities| {
        entities
            .globals
            .get(&glob_id)
            .map(|entry| entry.global.get(&store_ctx))
    });
    let Some(val) = val_opt else {
        return Err(JsNativeError::typ()
            .with_message("WebAssembly.Global.prototype.value accessed on invalid Global")
            .into());
    };
    wasm_val_to_js_val(&val, context)
}

/// Mutates a mutable `WebAssembly.Global` value in the shared `Store`.
pub fn wasm_set_global(glob_id: u32, new_val: &JsValue, context: &mut Context) -> JsResult<()> {
    let vt = with_store_and_entities(|store_ctx, entities| -> JsResult<ValType> {
        let Some(entry) = entities.globals.get(&glob_id) else {
            return Err(JsNativeError::typ()
                .with_message("WebAssembly.Global.prototype.value set on invalid Global")
                .into());
        };
        let ty = entry.global.ty(&store_ctx);
        if matches!(ty.mutability(), Mutability::Const) {
            return Err(JsNativeError::typ()
                .with_message(
                    "WebAssembly.Global.value: cannot change value of an immutable global",
                )
                .into());
        }
        Ok(ty.content())
    })?;

    let wv = js_val_to_wasm_val(new_val, vt, context)?;
    with_store_and_entities(|mut store_ctx, entities| -> JsResult<()> {
        let Some(entry) = entities.globals.get(&glob_id) else {
            return Err(JsNativeError::typ()
                .with_message("Invalid WebAssembly.Global")
                .into());
        };
        entry
            .global
            .set(&mut store_ctx, wv)
            .map_err(|e| JsNativeError::typ().with_message(e.to_string()).into())
    })
}

/// Allocates a real `wasmi::Table` in the isolate's shared `Store` for `new WebAssembly.Table`.
pub fn wasm_create_table(
    descriptor_val: &JsValue,
    fill_val: Option<&JsValue>,
    js_this_opt: Option<&JsObject>,
    context: &mut Context,
) -> JsResult<u32> {
    let Some(desc_obj) = descriptor_val.as_object() else {
        return Err(JsNativeError::typ()
            .with_message("WebAssembly.Table(): Descriptor must be an object")
            .into());
    };
    let elem_prop = desc_obj.get(js_string!("element"), context)?;
    let elem_str = elem_prop.to_string(context)?.to_std_string_escaped();
    let Some(rt) = parse_ref_type(&elem_str) else {
        return Err(JsNativeError::typ()
            .with_message(format!(
                "WebAssembly.Table(): Unsupported element type '{}'",
                elem_str
            ))
            .into());
    };
    let init_raw = {
        let v = desc_obj.get(js_string!("initial"), context)?;
        if v.is_undefined() {
            desc_obj.get(js_string!("minimum"), context)?
        } else {
            v
        }
    };
    if init_raw.is_undefined() {
        return Err(JsNativeError::typ()
            .with_message("WebAssembly.Table(): Property 'initial' is required")
            .into());
    }
    let initial_len = enforce_range_u32(&init_raw, "Table initial", context)?;
    let max_raw = desc_obj.get(js_string!("maximum"), context)?;
    let max_len = if max_raw.is_undefined() {
        None
    } else {
        Some(enforce_range_u32(&max_raw, "Table maximum", context)?)
    };
    if let Some(m) = max_len {
        if initial_len > m {
            return Err(JsNativeError::range()
                .with_message("WebAssembly.Table(): initial length exceeds maximum")
                .into());
        }
    }

    let init_ref = match fill_val {
        None => Ref::default_for_ty(rt),
        Some(v) => js_val_to_wasm_ref(v, rt, context)?,
    };

    with_store_and_entities(|mut store_ctx, entities| {
        let t_ty = TableType::new(rt, initial_len, max_len);
        let tbl = Table::new(&mut store_ctx, t_ty, init_ref).map_err(|e| {
            JsNativeError::range().with_message(format!("WebAssembly.Table(): {}", e))
        })?;
        let id = register_or_find_table(tbl, js_this_opt.cloned(), entities);
        Ok(id)
    })
}

/// Returns the element count of `tbl_id` (`Table.prototype.length`).
pub fn wasm_table_length(tbl_id: u32) -> JsResult<u32> {
    with_store_and_entities(|store_ctx, entities| {
        let Some(entry) = entities.tables.get(&tbl_id) else {
            return Err(JsNativeError::typ()
                .with_message("WebAssembly.Table.prototype.length accessed on invalid Table")
                .into());
        };
        Ok(entry.table.size(&store_ctx) as u32)
    })
}

/// Reads an element from `tbl_id` (`Table.prototype.get`).
pub fn wasm_table_get(tbl_id: u32, idx_val: &JsValue, context: &mut Context) -> JsResult<JsValue> {
    let idx = enforce_range_u32(idx_val, "Table.get index", context)?;
    let ref_res = with_store_and_entities(|store_ctx, entities| -> JsResult<Ref> {
        let Some(entry) = entities.tables.get(&tbl_id) else {
            return Err(JsNativeError::typ()
                .with_message("WebAssembly.Table.prototype.get called on invalid Table")
                .into());
        };
        entry.table.get(&store_ctx, u64::from(idx)).ok_or_else(|| {
            JsNativeError::range()
                .with_message("WebAssembly.Table.get(): index out of bounds")
                .into()
        })
    })?;
    wasm_ref_to_js_val(ref_res, context)
}

/// Writes an element into `tbl_id` (`Table.prototype.set`).
pub fn wasm_table_set(
    tbl_id: u32,
    idx_val: &JsValue,
    val_opt: Option<&JsValue>,
    context: &mut Context,
) -> JsResult<()> {
    let idx = enforce_range_u32(idx_val, "Table.set index", context)?;
    let rt = with_store_and_entities(|store_ctx, entities| -> JsResult<_> {
        let Some(entry) = entities.tables.get(&tbl_id) else {
            return Err(JsNativeError::typ()
                .with_message("WebAssembly.Table.prototype.set called on invalid Table")
                .into());
        };
        if u64::from(idx) >= entry.table.size(&store_ctx) {
            return Err(JsNativeError::range()
                .with_message("WebAssembly.Table.set(): index out of bounds")
                .into());
        }
        Ok(entry.table.ty(&store_ctx).element())
    })?;

    let wasm_ref = match val_opt {
        None => Ref::default_for_ty(rt),
        Some(v) => js_val_to_wasm_ref(v, rt, context)?,
    };

    with_store_and_entities(|mut store_ctx, entities| {
        let Some(entry) = entities.tables.get(&tbl_id) else {
            return Err(JsNativeError::typ()
                .with_message("Invalid WebAssembly.Table")
                .into());
        };
        entry
            .table
            .set(&mut store_ctx, u64::from(idx), wasm_ref)
            .map_err(|e| {
                JsNativeError::range()
                    .with_message(format!("WebAssembly.Table.set(): {}", e))
                    .into()
            })
    })
}

/// Grows `tbl_id` by `delta` elements (`Table.prototype.grow`).
pub fn wasm_table_grow(
    tbl_id: u32,
    delta_val: &JsValue,
    fill_opt: Option<&JsValue>,
    context: &mut Context,
) -> JsResult<u32> {
    let delta = enforce_range_u32(delta_val, "Table.grow delta", context)?;
    let rt = with_store_and_entities(|store_ctx, entities| -> JsResult<_> {
        let Some(entry) = entities.tables.get(&tbl_id) else {
            return Err(JsNativeError::typ()
                .with_message("WebAssembly.Table.prototype.grow called on invalid Table")
                .into());
        };
        Ok(entry.table.ty(&store_ctx).element())
    })?;

    let fill_ref = match fill_opt {
        None => Ref::default_for_ty(rt),
        Some(v) => js_val_to_wasm_ref(v, rt, context)?,
    };

    with_store_and_entities(|mut store_ctx, entities| {
        let Some(entry) = entities.tables.get(&tbl_id) else {
            return Err(JsNativeError::typ()
                .with_message("Invalid WebAssembly.Table")
                .into());
        };
        entry
            .table
            .grow(&mut store_ctx, u64::from(delta), fill_ref)
            .map(|old| old as u32)
            .map_err(|e| {
                JsNativeError::range()
                    .with_message(format!("WebAssembly.Table.grow(): {}", e))
                    .into()
            })
    })
}

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

fn invoke_js_host_import(
    closure_id: u32,
    res_types: &[ValType],
    caller: &mut Caller<'_, WasmStoreData>,
    params: &[Val],
    results: &mut [Val],
) -> Result<(), WasmiError> {
    let ctx_ptr = active_boa_ctx_ptr();
    if ctx_ptr.is_null() {
        return Err(WasmiError::host(WasmHostExceptionMarker));
    }

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

    let ctx = unsafe { &mut *ctx_ptr };

    // 2. Push `caller` onto `ACTIVE_CALLER_PTR` so re-entrant Wasm calls, `memory.grow`,
    //    `global.value`, `table.get/set`, and `new WebAssembly.Instance` inside `js_fn`
    //    reuse the active Store borrow without RefCell conflicts.
    let call_outcome: JsResult<()> = {
        let _caller_guard = ActiveCallerGuard::enter(caller);
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
