//! `process.env` OCap Accessors, Event-Loop Timer Scheduling, HTTP Context Probes, and `process` Control.

use crate::bindings::helpers::ocap_denied_js_error;
use crate::boa_realm::{
    BOA_ACTIVE_TENANT, BOA_ENV_MAP, BOA_HTTP_REQ, BOA_KV_MEM_BUDGET, BOA_WAIT_UNTIL_COUNT,
    record_boa_ocap_denial,
};
use crate::event_loop::{
    cancel_timer, refresh_timer, schedule_immediate, schedule_timer, set_timer_ref,
};
use slice_sandbox::SandboxViolation;

pub(super) fn register_env_timers_process_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{JsValue as BoaVal, NativeFunction, js_string};

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_env_get"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let key = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let found = BOA_ENV_MAP.with(|m| m.borrow().get(&key).cloned());
            match found {
                Some(v) => Ok(BoaVal::from(js_string!(v))),
                None => {
                    let tenant = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
                    let violation = SandboxViolation::EnvAccessDenied {
                        tenant_id: tenant,
                        key,
                    };
                    let msg = violation.to_string();
                    record_boa_ocap_denial(violation);
                    Err(ocap_denied_js_error("ERR_SLICE_ENV_DENIED", &msg, context))
                }
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_env_json"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let json = BOA_ENV_MAP
                .with(|m| serde_json::to_string(&*m.borrow()).unwrap_or_else(|_| "{}".to_string()));
            Ok(BoaVal::from(js_string!(json)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_timer_set"),
        4,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(cb_obj) = args.first().and_then(|v| v.as_object()) else {
                return Ok(BoaVal::from(0));
            };
            let delay_ms = args
                .get(1)
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as u64)
                .unwrap_or(0);
            let is_interval = args.get(2).and_then(|v| v.as_boolean()).unwrap_or(false);
            let mut extra_args = Vec::new();
            if let Some(arr_obj) = args.get(3).and_then(|v| v.as_object())
                && let Ok(len_val) = arr_obj.get(js_string!("length"), context)
            {
                let len = len_val.as_number().unwrap_or(0.0) as u32;
                for i in 0..len.min(8) {
                    if let Ok(elem) = arr_obj.get(i, context) {
                        extra_args.push(elem);
                    }
                }
            }
            match schedule_timer(cb_obj.clone(), delay_ms, is_interval, extra_args) {
                Ok(id) => Ok(BoaVal::from(id as f64)),
                Err(_) => Ok(BoaVal::from(0)),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_immediate_set"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(cb_obj) = args.first().and_then(|v| v.as_object()) else {
                return Ok(BoaVal::from(0));
            };
            let mut extra_args = Vec::new();
            if let Some(arr_obj) = args.get(1).and_then(|v| v.as_object())
                && let Ok(len_val) = arr_obj.get(js_string!("length"), context)
            {
                let len = len_val.as_number().unwrap_or(0.0) as u32;
                for i in 0..len.min(8) {
                    if let Ok(elem) = arr_obj.get(i, context) {
                        extra_args.push(elem);
                    }
                }
            }
            match schedule_immediate(cb_obj.clone(), extra_args) {
                Ok(id) => Ok(BoaVal::from(id as f64)),
                Err(_) => Ok(BoaVal::from(0)),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_timer_ctl"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as u32)
                .unwrap_or(0);
            let op = args
                .get(1)
                .and_then(|v| v.as_number())
                .map(|n| n as i32)
                .unwrap_or(0);
            match op {
                0 => cancel_timer(id),
                1 => set_timer_ref(id, true),
                2 => set_timer_ref(id, false),
                3 => refresh_timer(id),
                _ => {}
            }
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_is_active"),
        0,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            if let Some(mode) = args.first().and_then(|v| v.as_number()) {
                let (used, limit) = BOA_KV_MEM_BUDGET.with(|m| m.get());
                if mode == 1.0 {
                    return Ok(BoaVal::from(used as f64));
                } else if mode == 2.0 {
                    return Ok(BoaVal::from(limit.saturating_sub(used) as f64));
                }
            }
            let active = BOA_HTTP_REQ.with(|r| r.borrow().active);
            Ok(BoaVal::from(active))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_mark_wait_until"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            BOA_WAIT_UNTIL_COUNT.with(|c| c.set(c.get() + 1));
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_process_ctl"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let op = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as i32)
                .unwrap_or(0);
            if op == 3 {
                let has_fs = crate::boa_realm::BOA_ACTIVE_CAPS.with(|c| {
                    let caps = c.borrow();
                    !caps.read_prefixes.is_empty() || !caps.write_prefixes.is_empty()
                });
                let cwd = if has_fs {
                    slice_sandbox::cached_normalized_cwd()
                } else {
                    "/workspace"
                };
                return Ok(BoaVal::from(js_string!(cwd)));
            }
            crate::boa_realm::mark_subsystem_dirty(crate::boa_realm::DIRTY_PROCESS);
            if op == 0 {
                return Ok(BoaVal::undefined());
            }
            if op == 1 {
                let custom_opt = crate::boa_realm::BOA_CUSTOM_ARGV.with(|a| a.borrow().clone());
                let arr = if let Some(items) = custom_opt {
                    boa_engine::object::builtins::JsArray::from_iter(
                        items.iter().map(|s| BoaVal::from(js_string!(s.as_str()))),
                        context,
                    )
                } else {
                    boa_engine::object::builtins::JsArray::from_iter(
                        std::iter::once(BoaVal::from(js_string!("slice"))),
                        context,
                    )
                };
                return Ok(BoaVal::from(arr));
            }
            let proc_obj = context
                .global_object()
                .get(js_string!("process"), context)
                .ok()
                .and_then(|v| v.as_object());
            let mut code: i32 = match args.get(1) {
                Some(v) if !v.is_undefined() && !v.is_null() => v.to_i32(context).unwrap_or(0),
                _ => {
                    if let Some(ref p) = proc_obj {
                        p.get(js_string!("exitCode"), context)
                            .ok()
                            .filter(|v| !v.is_undefined() && !v.is_null())
                            .and_then(|v| v.to_i32(context).ok())
                            .unwrap_or(0)
                    } else {
                        0
                    }
                }
            };
            if let Some(ref p) = proc_obj {
                let _ = p.set(js_string!("exitCode"), BoaVal::from(code), false, context);
                let already_exited = p
                    .get(js_string!("_exited"), context)
                    .ok()
                    .and_then(|v| v.as_boolean())
                    .unwrap_or(false);
                if !already_exited {
                    let _ = p.set(js_string!("_exited"), BoaVal::from(true), false, context);
                    if let Ok(emit_val) = p.get(js_string!("emit"), context)
                        && let Some(emit_fn) = emit_val.as_object()
                        && emit_fn.is_callable()
                    {
                        let _ = emit_fn.call(
                            &BoaVal::from(p.clone()),
                            &[BoaVal::from(js_string!("exit")), BoaVal::from(code)],
                            context,
                        );
                    }
                    if let Some(inner_code) = crate::boa_realm::BOA_PROCESS_EXIT.with(|e| e.get()) {
                        code = inner_code;
                    } else if let Ok(ec_val) = p.get(js_string!("exitCode"), context)
                        && !ec_val.is_undefined()
                        && !ec_val.is_null()
                        && let Ok(updated) = ec_val.to_i32(context)
                    {
                        code = updated;
                    }
                }
            }
            crate::boa_realm::set_process_exit(code);
            if crate::workers::is_worker_thread() {
                crate::workers::worker_self_set_listening(false, Some(code));
            }
            Err(boa_engine::error::PanicError::new(format!("__SLICE_PROCESS_EXIT__:{code}")).into())
        }),
    );
}
