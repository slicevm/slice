//! Multi-Tenant Edge KV (`Slice.kv`), Transactional State Checkpoints, and Nested `Slice.evalSandbox` Host Bindings.

mod sandbox_eval;

use crate::bindings::helpers::{
    record_boa_violation_once, record_isolate_memory_violation, release_isolate_memory,
    trip_isolate_oom_limits, try_adjust_isolate_memory_ctx,
};
use crate::boa_realm::{BOA_ACTIVE_TENANT, BOA_KV_MEM_BUDGET};
use crate::event_loop::{clear_event_loop_state, drive_event_loop_to_completion};
use crate::state_and_kv::{
    checkpoint_tenant_state, edge_kv_del_bytes, edge_kv_get, edge_kv_incr_checked,
    edge_kv_set_checked, release_tenant_checkpoint, restore_tenant_state_checked,
};
use slice_sandbox::SandboxViolation;

pub(crate) fn register_kv_and_sandbox_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{JsValue as BoaVal, NativeFunction, js_string};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_kv_get"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_clear_timers"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            clear_event_loop_state();
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_drain_event_loop"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, context| {
            drive_event_loop_to_completion(context);
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_kv_get"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let key = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let val = BOA_ACTIVE_TENANT.with(|t| edge_kv_get(&t.borrow(), &key));
            match val {
                Some(s) => Ok(BoaVal::from(js_string!(s))),
                None => Ok(BoaVal::null()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_kv_set"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let key = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let val = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let ttl_ms = args
                .get(2)
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as u64)
                .unwrap_or(0);
            let (current_used, mem_limit) = BOA_KV_MEM_BUDGET.with(|m| m.get());
            match BOA_ACTIVE_TENANT.with(|t| {
                edge_kv_set_checked(&t.borrow(), &key, &val, ttl_ms, current_used, mem_limit)
            }) {
                Ok(delta) => {
                    if let Err(msg) = try_adjust_isolate_memory_ctx(delta, context) {
                        return Err(boa_engine::JsNativeError::range().with_message(msg).into());
                    }
                    Ok(BoaVal::from(true))
                }
                Err(requested) => {
                    let msg = record_isolate_memory_violation(requested, mem_limit);
                    trip_isolate_oom_limits(context);
                    Err(boa_engine::JsNativeError::range().with_message(msg).into())
                }
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_kv_del"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let key = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let freed = BOA_ACTIVE_TENANT.with(|t| edge_kv_del_bytes(&t.borrow(), &key));
            if let Some(bytes) = freed {
                release_isolate_memory(bytes);
            }
            Ok(BoaVal::from(freed.is_some()))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_kv_incr"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let key = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let delta = args
                .get(1)
                .and_then(|v| v.as_number())
                .map(|n| n as i64)
                .unwrap_or(1);
            let (current_used, mem_limit) = BOA_KV_MEM_BUDGET.with(|m| m.get());
            match BOA_ACTIVE_TENANT
                .with(|t| edge_kv_incr_checked(&t.borrow(), &key, delta, current_used, mem_limit))
            {
                Ok((next, mem_delta)) => {
                    if let Err(msg) = try_adjust_isolate_memory_ctx(mem_delta, context) {
                        return Err(boa_engine::JsNativeError::range().with_message(msg).into());
                    }
                    Ok(BoaVal::from(next as f64))
                }
                Err(requested) => {
                    let msg = record_isolate_memory_violation(requested, mem_limit);
                    trip_isolate_oom_limits(context);
                    Err(boa_engine::JsNativeError::range().with_message(msg).into())
                }
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_checkpoint"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, context| {
            let (allow_cp, max_cp) = crate::boa_realm::BOA_CHECKPOINT_CAP.with(|c| c.get());
            let (taken, restored, restore_ns) =
                crate::boa_realm::BOA_CHECKPOINT_STATS.with(|s| s.get());
            if !allow_cp || taken >= max_cp {
                let tenant = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
                let reason = if !allow_cp {
                    "guest checkpoints are disabled for this compartment".to_string()
                } else {
                    format!(
                        "exceeded max_checkpoints limit ({} > {})",
                        taken.saturating_add(1),
                        max_cp
                    )
                };
                record_boa_violation_once(SandboxViolation::CheckpointDenied {
                    tenant_id: tenant,
                    reason,
                });
                context.runtime_limits_mut().set_loop_iteration_limit(1);
                return Err(boa_engine::JsNativeError::error()
                    .with_message("SandboxViolation: CheckpointDenied")
                    .into());
            }
            let id = BOA_ACTIVE_TENANT.with(|t| checkpoint_tenant_state(&t.borrow()));
            crate::boa_realm::BOA_CHECKPOINT_STATS.with(|s| {
                s.set((taken.saturating_add(1), restored, restore_ns));
            });
            Ok(BoaVal::from(id as f64))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_restore"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let (allow_cp, _) = crate::boa_realm::BOA_CHECKPOINT_CAP.with(|c| c.get());
            if !allow_cp {
                let tenant = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
                record_boa_violation_once(SandboxViolation::CheckpointDenied {
                    tenant_id: tenant,
                    reason: "guest checkpoints/restores are disabled for this compartment"
                        .to_string(),
                });
                context.runtime_limits_mut().set_loop_iteration_limit(1);
                return Err(boa_engine::JsNativeError::error()
                    .with_message("SandboxViolation: CheckpointDenied")
                    .into());
            }
            let id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u64)
                .unwrap_or(0);
            let t0 = std::time::Instant::now();
            let ok = match BOA_ACTIVE_TENANT.with(|t| restore_tenant_state_checked(&t.borrow(), id))
            {
                Ok(ok) => ok,
                Err(msg) => {
                    trip_isolate_oom_limits(context);
                    return Err(boa_engine::JsNativeError::range().with_message(msg).into());
                }
            };
            let elapsed_ns = (t0.elapsed().as_nanos() as u64).max(1);
            if ok {
                crate::boa_realm::BOA_CHECKPOINT_STATS.with(|s| {
                    let (taken, restored, restore_ns) = s.get();
                    s.set((
                        taken,
                        restored.saturating_add(1),
                        restore_ns.saturating_add(elapsed_ns),
                    ));
                });
            }
            Ok(BoaVal::from(ok))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_release_checkpoint"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u64)
                .unwrap_or(0);
            let ok = BOA_ACTIVE_TENANT.with(|t| release_tenant_checkpoint(&t.borrow(), id));
            Ok(BoaVal::from(ok))
        }),
    );

    sandbox_eval::register_sandbox_eval_and_syntax_host_functions(ctx);
}
