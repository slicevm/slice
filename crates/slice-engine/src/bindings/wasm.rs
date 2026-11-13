//! Sandboxed WebAssembly (`wasmi`) Compilation, Memory, and Execution Host Bindings (`Shard 9`).

use crate::bindings::helpers::{bytes_to_js_array_buffer, with_buffer_source_bytes};
use crate::wasm::{
    wasm_call_export, wasm_compile_module, wasm_create_memory, wasm_get_global, wasm_grow_memory,
    wasm_instantiate_module, wasm_materialize_memory_ab, wasm_module_custom_sections,
    wasm_module_exports_json, wasm_module_imports_json, wasm_set_global,
    wasm_sync_initial_memory_to_ab, wasm_validate_bytes,
};

pub(crate) fn register_shard9_wasm_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsNativeError, JsValue as BoaVal, NativeFunction};
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
                    .with_message("TypeError: WebAssembly.Module(): Argument 0 is required")
                    .into());
            };
            let compiled = with_buffer_source_bytes(first, context, wasm_compile_module)?;
            match compiled {
                Ok(mod_id) => Ok(BoaVal::from(mod_id as f64)),
                Err(msg) => Err(JsNativeError::error().with_message(msg).into()),
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
                Err(msg) => Err(JsNativeError::error().with_message(msg).into()),
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
                Err(msg) => Err(JsNativeError::error().with_message(msg).into()),
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
                Err(msg) => Err(JsNativeError::error().with_message(msg).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_memory_create"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let init_pages = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as u32)
                .unwrap_or(0);
            let max_pages = args
                .get(1)
                .filter(|v| !v.is_undefined() && !v.is_null())
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as u32);
            match wasm_create_memory(init_pages, max_pages) {
                Ok(mem_id) => Ok(BoaVal::from(mem_id as f64)),
                Err(msg) => Err(JsNativeError::error().with_message(msg).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_memory_grow"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let mem_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let inst_id = args
                .get(1)
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let delta = args
                .get(2)
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as u32)
                .unwrap_or(0);
            match wasm_grow_memory(mem_id, inst_id, delta) {
                Ok(prev_pages) => Ok(BoaVal::from(prev_pages as f64)),
                Err(msg) => Err(JsNativeError::error().with_message(msg).into()),
            }
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

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_sync_init_mem"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let inst_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let Some(ab_obj) = args.get(1).and_then(|v| v.as_object()) else {
                return wasm_materialize_memory_ab(inst_id, context);
            };
            match wasm_sync_initial_memory_to_ab(inst_id, &ab_obj) {
                Ok(()) => Ok(BoaVal::undefined()),
                Err(msg) => Err(JsNativeError::error().with_message(msg).into()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_call"),
        4,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let inst_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let exp_name = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let mut call_args = Vec::new();
            if let Some(arr_obj) = args.get(2).and_then(|v| v.as_object()) {
                let len = arr_obj
                    .get(js_string!("length"), context)
                    .ok()
                    .and_then(|v| v.as_number())
                    .map(|n| n.max(0.0) as usize)
                    .unwrap_or(0);
                call_args.reserve(len);
                for i in 0..len {
                    let elem = arr_obj
                        .get(i as u32, context)
                        .unwrap_or_else(|_| BoaVal::undefined());
                    call_args.push(elem);
                }
            }
            let mem_ab_obj = args
                .get(3)
                .filter(|v| !v.is_undefined() && !v.is_null())
                .and_then(|v| v.as_object());
            wasm_call_export(inst_id, &exp_name, &call_args, mem_ab_obj.as_ref(), context)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_global_get"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let inst_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let exp_name = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            wasm_get_global(inst_id, &exp_name)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_wasm_global_set"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let inst_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            let exp_name = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let val = args.get(2).cloned().unwrap_or_else(BoaVal::undefined);
            wasm_set_global(inst_id, &exp_name, &val, context)?;
            Ok(BoaVal::undefined())
        }),
    );
}
