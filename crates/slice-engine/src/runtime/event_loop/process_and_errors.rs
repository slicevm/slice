//! Uncaught Exception Formatting, `unhandledRejection` Draining, and `process` Lifecycle Events.

use super::timers_and_state::has_pending_event_loop_work;
use crate::boa_realm::{
    BOA_ACTIVE_TENANT, BOA_DIRTY_SUBSYSTEMS, BOA_EXEC_DIRTY, BOA_LAST_EXIT_CODE, BOA_PROCESS_EXIT,
    BOA_UNHANDLED_REJECTIONS, BOA_VIOLATION, DIRTY_PROCESS, EXEC_DIRTY_REJECTIONS,
    has_process_exit, has_uncaught_error, set_last_exit_code, set_uncaught_error,
};
use boa_engine::{Context, JsObject, JsResult, JsValue, js_string};
use slice_sandbox::SandboxViolation;

/// Returns `true` when a `boa_engine::JsError` message represents an internal runtime loop
/// iteration or call-stack recursion ceiling (`SandboxViolation::CpuFuelExhausted`).
#[inline]
pub(crate) fn is_boa_runtime_limit_error(msg: &str) -> bool {
    msg.contains("RuntimeLimitError")
        || msg.contains("iteration loops")
        || msg.contains("loop iteration limit")
        || msg.contains("Maximum call stack")
        || msg.contains("recursion limit")
}

#[inline]
fn get_dirty_process_obj(ctx: &mut Context) -> Option<JsObject> {
    if (BOA_DIRTY_SUBSYSTEMS.with(|d| d.get()) & DIRTY_PROCESS) == 0 {
        return None;
    }
    ctx.global_object()
        .get(js_string!("process"), ctx)
        .ok()
        .and_then(|v| v.as_object())
}

#[inline]
fn process_has_listeners(proc_obj: &JsObject, event_opt: Option<&str>, ctx: &mut Context) -> bool {
    let Some(has_fn) = proc_obj
        .get(js_string!("_hasListeners"), ctx)
        .ok()
        .and_then(|v| v.as_object())
        .filter(|f| f.is_callable())
    else {
        return false;
    };
    let this_val = JsValue::from(proc_obj.clone());
    let res = match event_opt {
        Some(ev) => has_fn.call(&this_val, &[JsValue::from(js_string!(ev))], ctx),
        None => has_fn.call(&this_val, &[], ctx),
    };
    res.ok().is_some_and(|v| v.to_boolean())
}

#[inline]
fn process_emit(
    proc_obj: &JsObject,
    args: &[JsValue],
    ctx: &mut Context,
) -> Option<JsResult<JsValue>> {
    let emit_fn = proc_obj
        .get(js_string!("emit"), ctx)
        .ok()
        .and_then(|v| v.as_object())
        .filter(|f| f.is_callable())?;
    Some(emit_fn.call(&JsValue::from(proc_obj.clone()), args, ctx))
}

pub(crate) fn format_uncaught_js_error(err: &boa_engine::JsError, ctx: &mut Context) -> String {
    let raw_err = format!("Uncaught {}", err);
    let mut mapped = crate::boa_realm::map_error_with_active_source(&raw_err);
    if let Some((first_line, rest)) = mapped.split_once('\n')
        && let Some(open) = first_line.rfind(" (unknown at ")
    {
        let loc_paren = &first_line[open + 1..];
        if rest.contains(loc_paren) {
            mapped = format!("{}\n{}", &first_line[..open], rest);
        }
    }
    if mapped == raw_err
        && let Some(opaque) = err.as_opaque()
        && let Some(obj) = opaque.as_object()
        && let Ok(stack_val) = obj.get(js_string!("stack"), ctx)
        && let Some(s) = stack_val.as_string()
    {
        let st = s.to_std_string_escaped();
        for line in st.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("at ") && !trimmed.contains("<anonymous>") {
                if let Some(open) = trimmed.rfind('(') {
                    if let Some(close) = trimmed[open + 1..].find(')') {
                        let loc = &trimmed[open + 1..open + 1 + close];
                        if loc.contains(':') && !raw_err.contains(loc) {
                            mapped = format!("{} ({})", raw_err, loc);
                            break;
                        }
                    }
                } else if let Some(loc) = trimmed.strip_prefix("at ")
                    && loc.contains(':')
                    && !loc.contains(' ')
                    && !raw_err.contains(loc)
                {
                    mapped = format!("{} ({})", raw_err, loc);
                    break;
                }
            }
        }
    }
    mapped
}

pub(crate) fn handle_uncaught_js_error(err: boa_engine::JsError, ctx: &mut Context) -> bool {
    if has_process_exit() {
        return true;
    }
    let msg = err.to_string();
    if msg.starts_with("__SLICE_PROCESS_EXIT__:") {
        return true;
    }
    if is_boa_runtime_limit_error(&msg) {
        let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
        BOA_VIOLATION.with(|v| {
            if v.borrow().is_none() {
                *v.borrow_mut() = Some(SandboxViolation::CpuFuelExhausted {
                    tenant_id,
                    limit: 0,
                });
            }
        });
        return false;
    }

    let formatted = format_uncaught_js_error(&err, ctx);
    let err_val = err
        .into_opaque(ctx)
        .unwrap_or_else(|e| JsValue::from(js_string!(e.to_string())));

    if let Some(proc_obj) = get_dirty_process_obj(ctx)
        && process_has_listeners(&proc_obj, None, ctx)
    {
        let _ = process_emit(
            &proc_obj,
            &[
                JsValue::from(js_string!("uncaughtExceptionMonitor")),
                err_val.clone(),
                JsValue::from(js_string!("uncaughtException")),
            ],
            ctx,
        );
        if has_process_exit() {
            return true;
        }
        if let Some(emit_res) = process_emit(
            &proc_obj,
            &[
                JsValue::from(js_string!("uncaughtException")),
                err_val,
                JsValue::from(js_string!("uncaughtException")),
            ],
            ctx,
        ) {
            match emit_res {
                Ok(v) if v.to_boolean() => {
                    let _ = run_jobs_and_check_rejections(ctx);
                    return !has_uncaught_error() && BOA_VIOLATION.with(|v| v.borrow().is_none());
                }
                Err(handler_err) => {
                    if has_process_exit() {
                        return true;
                    }
                    let h_formatted = format_uncaught_js_error(&handler_err, ctx);
                    set_uncaught_error(h_formatted);
                    set_last_exit_code(1);
                    return false;
                }
                Ok(_) => {}
            }
        }
    }

    set_uncaught_error(formatted);
    set_last_exit_code(1);
    false
}

pub(crate) fn drain_unhandled_rejections(ctx: &mut Context) -> bool {
    if (BOA_EXEC_DIRTY.with(|c| c.get()) & EXEC_DIRTY_REJECTIONS) == 0 {
        return true;
    }
    while BOA_UNHANDLED_REJECTIONS.with(|u| !u.borrow().is_empty()) {
        if has_process_exit()
            || has_uncaught_error()
            || BOA_VIOLATION.with(|v| v.borrow().is_some())
        {
            BOA_UNHANDLED_REJECTIONS.with(|u| u.borrow_mut().clear());
            return false;
        }
        let pending = BOA_UNHANDLED_REJECTIONS.with(|u| std::mem::take(&mut *u.borrow_mut()));
        let proc_obj = get_dirty_process_obj(ctx);
        let has_ur_listener = proc_obj
            .as_ref()
            .is_some_and(|p| process_has_listeners(p, Some("unhandledRejection"), ctx));

        for promise_obj in pending {
            let Some(reason) =
                boa_engine::object::builtins::JsPromise::from_object(promise_obj.clone())
                    .ok()
                    .and_then(|jp| match jp.state() {
                        boa_engine::builtins::promise::PromiseState::Rejected(r) => Some(r),
                        _ => None,
                    })
            else {
                continue;
            };

            if has_ur_listener
                && let Some(ref p) = proc_obj
                && let Some(emit_res) = process_emit(
                    p,
                    &[
                        JsValue::from(js_string!("unhandledRejection")),
                        reason.clone(),
                        JsValue::from(promise_obj.clone()),
                    ],
                    ctx,
                )
            {
                match emit_res {
                    Ok(v) if v.to_boolean() => {
                        if let Err(e) = ctx.run_jobs()
                            && !handle_uncaught_js_error(e, ctx)
                        {
                            return false;
                        }
                        if has_process_exit() {
                            return false;
                        }
                        continue;
                    }
                    Err(handler_err) => {
                        if !handle_uncaught_js_error(handler_err, ctx) {
                            return false;
                        }
                        if has_process_exit() {
                            return false;
                        }
                        continue;
                    }
                    Ok(_) => {}
                }
            }

            let reason_str = reason
                .to_string(ctx)
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|_| reason.display().to_string());
            set_uncaught_error(format!("Uncaught (in promise) {}", reason_str));
            set_last_exit_code(1);
            BOA_UNHANDLED_REJECTIONS.with(|u| u.borrow_mut().clear());
            return false;
        }
    }
    true
}

#[inline]
pub(crate) fn run_jobs_and_check_rejections(ctx: &mut Context) -> bool {
    if let Err(err) = ctx.run_jobs()
        && !handle_uncaught_js_error(err, ctx)
    {
        return false;
    }
    if has_process_exit() {
        return false;
    }
    drain_unhandled_rejections(ctx)
}

pub(super) fn try_emit_before_exit(ctx: &mut Context, before_exit_count: &mut usize) -> bool {
    if *before_exit_count >= 64
        || has_process_exit()
        || has_uncaught_error()
        || BOA_VIOLATION.with(|v| v.borrow().is_some())
    {
        return false;
    }
    let Some(proc_obj) = get_dirty_process_obj(ctx) else {
        return false;
    };
    if !process_has_listeners(&proc_obj, Some("beforeExit"), ctx) {
        return false;
    }
    *before_exit_count += 1;
    let code = proc_obj
        .get(js_string!("exitCode"), ctx)
        .ok()
        .filter(|v| !v.is_undefined() && !v.is_null())
        .and_then(|v| v.to_i32(ctx).ok())
        .unwrap_or(0);
    if let Some(Err(err)) = process_emit(
        &proc_obj,
        &[JsValue::from(js_string!("beforeExit")), JsValue::from(code)],
        ctx,
    ) && !handle_uncaught_js_error(err, ctx)
    {
        return false;
    }
    if !run_jobs_and_check_rejections(ctx) {
        return false;
    }
    has_pending_event_loop_work()
}

pub(crate) fn finalize_and_reset_process_state(ctx: &mut Context) {
    let Some(proc_obj) = ctx
        .global_object()
        .get(js_string!("process"), ctx)
        .ok()
        .and_then(|v| v.as_object())
    else {
        return;
    };
    let already_exited = proc_obj
        .get(js_string!("_exited"), ctx)
        .ok()
        .and_then(|v| v.as_boolean())
        .unwrap_or(false);
    if !already_exited {
        let mut code = proc_obj
            .get(js_string!("exitCode"), ctx)
            .ok()
            .filter(|v| !v.is_undefined() && !v.is_null())
            .and_then(|v| v.to_i32(ctx).ok())
            .or_else(|| BOA_LAST_EXIT_CODE.with(|e| e.get()))
            .unwrap_or(0);
        let _ = proc_obj.set(js_string!("_exited"), JsValue::from(true), false, ctx);
        let _ = process_emit(
            &proc_obj,
            &[JsValue::from(js_string!("exit")), JsValue::from(code)],
            ctx,
        );
        if let Some(inner_code) = BOA_PROCESS_EXIT.with(|e| e.get()) {
            code = inner_code;
        } else if let Ok(ec_val) = proc_obj.get(js_string!("exitCode"), ctx)
            && !ec_val.is_undefined()
            && !ec_val.is_null()
            && let Ok(updated) = ec_val.to_i32(ctx)
        {
            code = updated;
        }
        if code != 0 || BOA_LAST_EXIT_CODE.with(|e| e.get().is_some()) {
            set_last_exit_code(code);
        }
        if crate::workers::is_worker_thread() && code != 0 {
            crate::workers::worker_self_set_listening(false, Some(code));
        }
    }
    if let Some(reset_fn) = proc_obj
        .get(js_string!("_resetProcessState"), ctx)
        .ok()
        .and_then(|v| v.as_object())
        .filter(|f| f.is_callable())
    {
        let _ = reset_fn.call(&JsValue::from(proc_obj), &[], ctx);
    } else if let Some(reset_ec_fn) = proc_obj
        .get(js_string!("_resetExitCode"), ctx)
        .ok()
        .and_then(|v| v.as_object())
        .filter(|f| f.is_callable())
    {
        let _ = reset_ec_fn.call(&JsValue::from(proc_obj), &[], ctx);
    }
}
