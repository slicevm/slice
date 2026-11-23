//! Store-backed `WebAssembly.Global`, `WebAssembly.Table`, and canonical `wasmi::Func` JS wrappers.

use super::call::wasm_call_func_by_id;
use super::store::{WasmEntities, WasmStoreData, construct_wasm_wrapper, with_store_and_entities};
use super::values::{
    HIDDEN_FN_ID_PROP, default_val_for_type, enforce_range_u32, js_val_to_wasm_ref,
    js_val_to_wasm_val, parse_ref_type, parse_val_type, wasm_ref_to_js_val, wasm_val_to_js_val,
};
use boa_engine::object::FunctionObjectBuilder;
use boa_engine::property::PropertyDescriptor;
use boa_engine::{Context, JsNativeError, JsObject, JsResult, JsValue, NativeFunction, js_string};
use smallvec::SmallVec;
use wasmi::{Func, Global, Mutability, Ref, StoreContext, Table, TableType, ValType};

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
pub(crate) fn register_or_find_global(
    global: Global,
    js_wrapper: Option<JsObject>,
    entities: &mut WasmEntities,
) -> u32 {
    let key = format!("{:?}", global);
    if let Some(&existing_id) = entities.global_by_key.get(&key) {
        if let Some(w) = js_wrapper
            && let Some(entry) = entities.globals.get_mut(&existing_id)
            && entry.js_wrapper.is_none()
        {
            entry.js_wrapper = Some(w);
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
        if let Some(w) = js_wrapper
            && let Some(entry) = entities.tables.get_mut(&existing_id)
            && entry.js_wrapper.is_none()
        {
            entry.js_wrapper = Some(w);
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
    };
    let initial_len = enforce_range_u32(&init_raw, "Table initial", context)?;
    let max_raw = desc_obj.get(js_string!("maximum"), context)?;
    let max_len = if max_raw.is_undefined() {
        None
    } else {
        Some(enforce_range_u32(&max_raw, "Table maximum", context)?)
    };
    if let Some(m) = max_len
        && initial_len > m
    {
        return Err(JsNativeError::range()
            .with_message("WebAssembly.Table(): initial length exceeds maximum")
            .into());
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
