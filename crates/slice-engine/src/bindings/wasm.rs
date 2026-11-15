//! Sandboxed WebAssembly (`wasmi`) Compilation, Memory, Global, Table, and Execution Host Bindings (`Shard 9`).

use crate::bindings::helpers::{bytes_to_js_array_buffer, with_buffer_source_bytes};
use crate::wasm::{
    wasm_compile_module, wasm_create_global, wasm_create_memory, wasm_create_table,
    wasm_get_global, wasm_grow_memory, wasm_instantiate_module, wasm_js_error, wasm_memory_buffer,
    wasm_module_custom_sections, wasm_module_exports_json, wasm_module_imports_json,
    wasm_set_global, wasm_table_get, wasm_table_grow, wasm_table_length, wasm_table_set,
    wasm_validate_bytes,
};

pub(crate) fn register_shard9_wasm_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{JsNativeError, JsValue as BoaVal, NativeFunction, js_string};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_wasm_compile"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_validate"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::from(false));
            };
            let Ok(valid) = with_buffer_source_bytes(first, context, wasm_validate_bytes) else {
                return Ok(BoaVal::from(false));
            };
            Ok(BoaVal::from(valid))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_compile"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(first) = args.first() else {
                return Err(JsNativeError::typ()
                    .with_message("WebAssembly.Module(): Argument 0 is required")
                    .into());
            };
            let compiled = with_buffer_source_bytes(first, context, wasm_compile_module)?;
            match compiled {
                Ok(mod_id) => Ok(BoaVal::from(mod_id as f64)),
                Err((kind, msg)) => Err(wasm_js_error(kind, msg, context)),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_module_exports"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let mod_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            match wasm_module_exports_json(mod_id) {
                Ok(json) => Ok(BoaVal::from(js_string!(json))),
                Err(msg) => Err(JsNativeError::typ().with_message(msg).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_module_imports"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let mod_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            match wasm_module_imports_json(mod_id) {
                Ok(json) => Ok(BoaVal::from(js_string!(json))),
                Err(msg) => Err(JsNativeError::typ().with_message(msg).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_module_custom_sections"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let mod_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let sec_name = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            match wasm_module_custom_sections(mod_id, &sec_name) {
                Ok(sections) => {
                    let mut items = Vec::with_capacity(sections.len());
                    for sec in &sections {
                        let ab = bytes_to_js_array_buffer(sec, context)?;
                        items.push(ab);
                    }
                    let arr = boa_engine::object::builtins::JsArray::from_iter(items, context);
                    Ok(arr.into())
                }
                Err(msg) => Err(JsNativeError::typ().with_message(msg).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_memory_create"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let desc = args.first().cloned().unwrap_or_else(BoaVal::undefined);
            let js_this = args.get(1).and_then(|v| v.as_object());
            let mem_id = wasm_create_memory(&desc, js_this.as_ref(), context)?;
            Ok(BoaVal::from(mem_id as f64))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_memory_grow"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let mem_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let delta = args.get(1).cloned().unwrap_or_else(BoaVal::undefined);
            let prev = wasm_grow_memory(mem_id, &delta, context)?;
            Ok(BoaVal::from(prev as f64))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_memory_buffer"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let mem_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            wasm_memory_buffer(mem_id, context)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_global_create"),
        4,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let desc = args.first().cloned().unwrap_or_else(BoaVal::undefined);
            let has_val = args.get(2).map(|v| v.to_boolean()).unwrap_or(false);
            let init_val = if has_val { args.get(1) } else { None };
            let js_this = args.get(3).and_then(|v| v.as_object());
            let glob_id = wasm_create_global(&desc, init_val, js_this.as_ref(), context)?;
            Ok(BoaVal::from(glob_id as f64))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_global_get"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let glob_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            wasm_get_global(glob_id, context)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_global_set"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let glob_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let val = args.get(1).cloned().unwrap_or_else(BoaVal::undefined);
            wasm_set_global(glob_id, &val, context)?;
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_table_create"),
        4,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let desc = args.first().cloned().unwrap_or_else(BoaVal::undefined);
            let has_fill = args.get(2).map(|v| v.to_boolean()).unwrap_or(false);
            let fill_val = if has_fill { args.get(1) } else { None };
            let js_this = args.get(3).and_then(|v| v.as_object());
            let tbl_id = wasm_create_table(&desc, fill_val, js_this.as_ref(), context)?;
            Ok(BoaVal::from(tbl_id as f64))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_table_length"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let tbl_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let len = wasm_table_length(tbl_id)?;
            Ok(BoaVal::from(len as f64))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_table_get"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let tbl_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let idx = args.get(1).cloned().unwrap_or_else(BoaVal::undefined);
            wasm_table_get(tbl_id, &idx, context)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_table_set"),
        4,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let tbl_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let idx = args.get(1).cloned().unwrap_or_else(BoaVal::undefined);
            let has_val = args.get(3).map(|v| v.to_boolean()).unwrap_or(false);
            let val_opt = if has_val { args.get(2) } else { None };
            wasm_table_set(tbl_id, &idx, val_opt, context)?;
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_table_grow"),
        4,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let tbl_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let delta = args.get(1).cloned().unwrap_or_else(BoaVal::undefined);
            let has_fill = args.get(3).map(|v| v.to_boolean()).unwrap_or(false);
            let fill_opt = if has_fill { args.get(2) } else { None };
            let prev = wasm_table_grow(tbl_id, &delta, fill_opt, context)?;
            Ok(BoaVal::from(prev as f64))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_instantiate"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let mod_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let imp_obj = args
                .get(1)
                .filter(|v| !v.is_undefined() && !v.is_null())
                .and_then(|v| v.as_object());
            wasm_instantiate_module(mod_id, imp_obj.as_ref(), context)
        }),
    );
}
