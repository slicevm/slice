//! Multi-Tenant Edge KV (`Slice.kv`), Transactional State Checkpoints, and Nested `Slice.evalSandbox` Host Bindings.

use crate::bindings::helpers::{
    record_isolate_memory_violation, release_isolate_memory, trip_isolate_oom_limits,
    try_adjust_isolate_memory_ctx,
};
use crate::boa_realm::{
    BOA_ACTIVE_TENANT, BOA_KV_MEM_BUDGET, BOA_VIOLATION, SavedIsolateContextGuard,
};
use crate::event_loop::{clear_event_loop_state, drive_event_loop_to_completion};
use crate::isolate::{SandboxEvalOptions, SandboxEvalOutcome, SliceIsolate};
use crate::state_and_kv::{
    checkpoint_tenant_state, edge_kv_del_bytes, edge_kv_get, edge_kv_incr_checked,
    edge_kv_set_checked, release_tenant_checkpoint, restore_tenant_state_checked,
};
use slice_sandbox::{CapabilityToken, SandboxViolation};

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
                BOA_VIOLATION.with(|v| {
                    *v.borrow_mut() = Some(SandboxViolation::CheckpointDenied {
                        tenant_id: tenant,
                        reason,
                    });
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
                BOA_VIOLATION.with(|v| {
                    *v.borrow_mut() = Some(SandboxViolation::CheckpointDenied {
                        tenant_id: tenant,
                        reason: "guest checkpoints/restores are disabled for this compartment"
                            .to_string(),
                    });
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

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_eval_sandbox"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let code = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let (opts, deny_read, deny_write, deny_net, deny_exec) =
                parse_sandbox_eval_options_arg(args.get(1), context)?;

            let scope_guard = SavedIsolateContextGuard::enter();

            let mut parent_caps = CapabilityToken::pure_compute();
            if !deny_read {
                for r in &scope_guard.saved_caps.read_prefixes {
                    parent_caps = parent_caps.with_read_prefix(r.clone());
                }
            }
            if !deny_write {
                for w in &scope_guard.saved_caps.write_prefixes {
                    parent_caps = parent_caps.with_write_prefix(w.clone());
                }
            }
            if !deny_net {
                for nh in &scope_guard.saved_net_hosts {
                    parent_caps = parent_caps.with_net_host(nh.clone());
                }
            }
            for (ek, ev) in &scope_guard.saved_env_map {
                parent_caps = parent_caps.with_env(ek.clone(), ev.clone());
            }
            if !deny_exec {
                for eb in &scope_guard.saved_exec_bins {
                    parent_caps = parent_caps.with_exec_binary(eb.clone());
                }
            }
            let (parent_allow_cp, parent_max_cp) = scope_guard.saved_checkpoint_cap;
            parent_caps = parent_caps.with_checkpoints(parent_allow_cp);
            if parent_max_cp < u32::MAX {
                let rem_cp = parent_max_cp.saturating_sub(scope_guard.saved_checkpoint_stats.0);
                parent_caps = parent_caps.with_max_checkpoints(Some(rem_cp));
            }
            let parent_rem_fuel = scope_guard
                .saved_fuel_budget
                .1
                .saturating_sub(scope_guard.saved_fuel_budget.0);
            let parent_used_mem = scope_guard.saved_kv_budget.0;
            let parent_limit_mem = scope_guard.saved_kv_budget.1;
            let mut proxy_parent = SliceIsolate::spawn(
                scope_guard.saved_tenant.clone(),
                98,
                parent_caps,
                parent_rem_fuel,
                parent_limit_mem.saturating_add(4096),
            );
            proxy_parent.compartment.memory_used_bytes = parent_used_mem.saturating_add(4096);
            proxy_parent.egress_audit_secrets = scope_guard.saved_egress_secrets.clone();
            let parent_tenant_for_trace = scope_guard.saved_tenant.clone();
            let outcome = if cfg!(debug_assertions) {
                let vfs_owner = crate::boa_realm::BOA_VFS_OWNER_TENANT.with(|o| o.borrow().clone());
                let vfs_snap =
                    crate::boa_realm::BOA_VFS_STATE.with(|v| v.borrow().snapshot_with_fds());
                let (out, new_owner, new_snap) = std::thread::Builder::new()
                    .name("slice-eval-sandbox".to_string())
                    .stack_size(8 * 1024 * 1024)
                    .spawn(move || {
                        crate::boa_realm::BOA_VFS_OWNER_TENANT
                            .with(|o| *o.borrow_mut() = vfs_owner);
                        crate::boa_realm::BOA_VFS_STATE
                            .with(|v| v.borrow_mut().restore_from_snapshot(vfs_snap));
                        let res = proxy_parent.eval_sandbox(&code, opts);
                        let no =
                            crate::boa_realm::BOA_VFS_OWNER_TENANT.with(|o| o.borrow().clone());
                        let ns = crate::boa_realm::BOA_VFS_STATE
                            .with(|v| v.borrow().snapshot_with_fds());
                        (res, no, ns)
                    })
                    .expect("failed to spawn slice-eval-sandbox thread")
                    .join()
                    .expect("slice-eval-sandbox thread panicked");
                crate::boa_realm::BOA_VFS_OWNER_TENANT.with(|o| *o.borrow_mut() = new_owner);
                crate::boa_realm::BOA_VFS_STATE
                    .with(|v| v.borrow_mut().restore_from_snapshot(new_snap));
                out
            } else {
                proxy_parent.eval_sandbox(&code, opts)
            };
            drop(scope_guard);
            if outcome.net_persisted_delta_bytes != 0 {
                let _ = crate::bindings::helpers::try_adjust_isolate_memory_ctx(
                    outcome.net_persisted_delta_bytes,
                    context,
                );
            }
            if outcome.checkpoints_taken > 0 || outcome.restores_performed > 0 {
                crate::boa_realm::BOA_CHECKPOINT_STATS.with(|s| {
                    let (taken, restored, restore_ns) = s.get();
                    s.set((
                        taken.saturating_add(outcome.checkpoints_taken),
                        restored.saturating_add(outcome.restores_performed),
                        restore_ns.saturating_add(outcome.restore_time_ns),
                    ));
                });
            }
            if outcome.fuel_used > 0 {
                crate::boa_realm::BOA_FUEL_BUDGET.with(|f| {
                    let (used, limit) = f.get();
                    f.set((used.saturating_add(outcome.fuel_used), limit));
                });
            }
            if crate::boa_realm::is_trace_recording_enabled() {
                crate::boa_realm::record_trace_event(
                    "sandbox.eval",
                    "slice.ocap",
                    100,
                    (outcome.total_time_us * 1_000.0).max(1.0) as u64,
                    serde_json::json!({
                        "parent_tenant": parent_tenant_for_trace,
                        "ok": outcome.ok,
                        "fuel_used": outcome.fuel_used,
                        "memory_used_bytes": outcome.memory_used_bytes,
                        "violation": outcome.violation
                    }),
                );
            }

            Ok(sandbox_outcome_to_js_object(outcome, context))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_vm_check_syntax"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let code = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let filename = args
                .get(1)
                .filter(|v| !v.is_undefined() && !v.is_null())
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let path_opt = if filename.is_empty() {
                None
            } else {
                Some(std::path::Path::new(filename.as_str()))
            };
            match slice_cas::check_syntax_source_detailed(&code, path_opt) {
                Ok(()) => Ok(BoaVal::null()),
                Err(diag) => {
                    let msg = format!("{} at line {}, col {}", diag.message, diag.line, diag.col);
                    Ok(BoaVal::from(js_string!(msg)))
                }
            }
        }),
    );
}

fn read_js_string_array_prop(
    obj: &boa_engine::JsObject,
    keys: &[boa_engine::JsString],
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<(Vec<String>, bool)> {
    for key in keys {
        if !obj.has_own_property(key.clone(), context)? {
            continue;
        }
        let val = obj.get(key.clone(), context)?;
        if let Some(arr_obj) = val.as_object()
            && arr_obj.is_array()
        {
            let len = arr_obj
                .get(boa_engine::js_string!("length"), context)?
                .as_number()
                .map(|n| n.max(0.0) as usize)
                .unwrap_or(0);
            if len == 0 {
                return Ok((Vec::new(), true));
            }
            let mut out = Vec::with_capacity(len.min(64));
            for i in 0..len {
                let elem = arr_obj.get(i, context)?;
                if !elem.is_undefined() && !elem.is_null() {
                    out.push(elem.to_string(context)?.to_std_string_escaped());
                }
            }
            return Ok((out, false));
        }
    }
    Ok((Vec::new(), false))
}

fn parse_sandbox_eval_options_arg(
    arg: Option<&boa_engine::JsValue>,
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<(SandboxEvalOptions, bool, bool, bool, bool)> {
    use boa_engine::js_string;
    let Some(val) = arg.filter(|v| !v.is_undefined() && !v.is_null()) else {
        return Ok((SandboxEvalOptions::default(), false, false, false, false));
    };

    if let Some(obj) = val.as_object() {
        let mut opts = SandboxEvalOptions::default();

        if obj.has_own_property(js_string!("fuel"), context)? {
            let v = obj.get(js_string!("fuel"), context)?;
            if let Some(n) = v.as_number() {
                opts.fuel = Some(n.max(0.0) as u64);
            }
        }

        for mk in [
            js_string!("memoryBytes"),
            js_string!("memory"),
            js_string!("maxMemoryBytes"),
        ] {
            if obj.has_own_property(mk.clone(), context)? {
                let v = obj.get(mk, context)?;
                if let Some(n) = v.as_number() {
                    opts.memory_bytes = Some(n.max(0.0) as usize);
                    break;
                }
            }
        }

        if obj.has_own_property(js_string!("tenantId"), context)? {
            let v = obj.get(js_string!("tenantId"), context)?;
            if !v.is_undefined() && !v.is_null() {
                opts.tenant_id = Some(v.to_string(context)?.to_std_string_escaped());
            }
        }

        if obj.has_own_property(js_string!("deterministicSeed"), context)? {
            let v = obj.get(js_string!("deterministicSeed"), context)?;
            if let Some(n) = v.as_number() {
                opts.deterministic_seed = Some(n.max(0.0) as u64);
            }
        }

        if obj.has_own_property(js_string!("auditSecret"), context)? {
            let v = obj.get(js_string!("auditSecret"), context)?;
            if !v.is_undefined() && !v.is_null() {
                opts.audit_secret = Some(v.to_string(context)?.to_std_string_escaped());
            }
        }

        let (audit_secrets, _) =
            read_js_string_array_prop(&obj, &[js_string!("auditSecrets")], context)?;
        opts.audit_secrets = audit_secrets;

        for ck in [js_string!("checkpoints"), js_string!("allowCheckpoints")] {
            if obj.has_own_property(ck.clone(), context)? {
                let v = obj.get(ck, context)?;
                if let Some(b) = v.as_boolean() {
                    opts.checkpoints = Some(b);
                    break;
                }
            }
        }

        if obj.has_own_property(js_string!("maxCheckpoints"), context)? {
            let v = obj.get(js_string!("maxCheckpoints"), context)?;
            if let Some(n) = v.as_number() {
                opts.max_checkpoints = Some(n.max(0.0) as u32);
            }
        }

        let (allow_read, deny_read) = read_js_string_array_prop(
            &obj,
            &[
                js_string!("allowRead"),
                js_string!("readPrefixes"),
                js_string!("fsRead"),
            ],
            context,
        )?;
        opts.allow_read = allow_read;

        let (allow_write, deny_write) = read_js_string_array_prop(
            &obj,
            &[
                js_string!("allowWrite"),
                js_string!("writePrefixes"),
                js_string!("fsWrite"),
            ],
            context,
        )?;
        opts.allow_write = allow_write;

        let (allow_net, deny_net) = read_js_string_array_prop(
            &obj,
            &[js_string!("allowNet"), js_string!("netHosts")],
            context,
        )?;
        opts.allow_net = allow_net;

        let (allow_exec, deny_exec) = read_js_string_array_prop(
            &obj,
            &[js_string!("allowExec"), js_string!("execBinaries")],
            context,
        )?;
        opts.allow_exec = allow_exec;

        let (allow_env, _) = read_js_string_array_prop(
            &obj,
            &[js_string!("allowEnv"), js_string!("envKeys")],
            context,
        )?;
        opts.allow_env = allow_env;

        return Ok((opts, deny_read, deny_write, deny_net, deny_exec));
    }

    let opts_json = val.to_string(context)?.to_std_string_escaped();
    if opts_json == "{}" || opts_json.is_empty() {
        Ok((SandboxEvalOptions::default(), false, false, false, false))
    } else if !opts_json.contains("[]") {
        Ok((
            serde_json::from_str::<SandboxEvalOptions>(&opts_json).unwrap_or_default(),
            false,
            false,
            false,
            false,
        ))
    } else if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&opts_json) {
        let has_empty_arr = |keys: &[&str]| -> bool {
            json_val.as_object().is_some_and(|o| {
                keys.iter().any(|k| {
                    o.get(*k)
                        .and_then(|v| v.as_array())
                        .is_some_and(|a| a.is_empty())
                })
            })
        };
        let dr = has_empty_arr(&["allowRead", "readPrefixes", "fsRead"]);
        let dw = has_empty_arr(&["allowWrite", "writePrefixes", "fsWrite"]);
        let dn = has_empty_arr(&["allowNet", "netHosts"]);
        let de = has_empty_arr(&["allowExec", "execBinaries"]);
        let parsed: SandboxEvalOptions = serde_json::from_value(json_val).unwrap_or_default();
        Ok((parsed, dr, dw, dn, de))
    } else {
        Ok((SandboxEvalOptions::default(), false, false, false, false))
    }
}

fn sandbox_outcome_to_js_object(
    outcome: SandboxEvalOutcome,
    context: &mut boa_engine::Context,
) -> boa_engine::JsValue {
    use boa_engine::{
        JsValue as BoaVal, js_string, object::ObjectInitializer, property::Attribute,
    };

    let stdout_arr = boa_engine::object::builtins::JsArray::from_iter(
        outcome
            .stdout
            .into_iter()
            .map(|line| BoaVal::from(js_string!(line))),
        context,
    );
    let ret_json_val = match outcome.return_json {
        Some(s) => BoaVal::from(js_string!(s)),
        None => BoaVal::null(),
    };
    let err_val = match outcome.error {
        Some(s) => BoaVal::from(js_string!(s)),
        None => BoaVal::null(),
    };
    let viol_val = match outcome.violation {
        Some(s) => BoaVal::from(js_string!(s)),
        None => BoaVal::null(),
    };
    let attr = Attribute::WRITABLE | Attribute::ENUMERABLE | Attribute::CONFIGURABLE;

    ObjectInitializer::new(context)
        .property(js_string!("ok"), outcome.ok, attr)
        .property(js_string!("stdout"), stdout_arr, attr)
        .property(
            js_string!("returnValue"),
            js_string!(outcome.return_value),
            attr,
        )
        .property(js_string!("returnJson"), ret_json_val, attr)
        .property(js_string!("error"), err_val, attr)
        .property(js_string!("violation"), viol_val, attr)
        .property(js_string!("fuelUsed"), outcome.fuel_used as f64, attr)
        .property(
            js_string!("memoryUsedBytes"),
            outcome.memory_used_bytes as f64,
            attr,
        )
        .property(
            js_string!("peakMemoryBytes"),
            outcome.peak_memory_bytes as f64,
            attr,
        )
        .property(
            js_string!("spawnTimeNs"),
            outcome.spawn_time_ns as f64,
            attr,
        )
        .property(
            js_string!("compileTimeNs"),
            outcome.compile_time_ns as f64,
            attr,
        )
        .property(js_string!("execTimeNs"), outcome.exec_time_ns as f64, attr)
        .property(js_string!("totalTimeUs"), outcome.total_time_us, attr)
        .property(
            js_string!("checkpointsTaken"),
            f64::from(outcome.checkpoints_taken),
            attr,
        )
        .property(
            js_string!("restoresPerformed"),
            f64::from(outcome.restores_performed),
            attr,
        )
        .property(
            js_string!("restoreTimeNs"),
            outcome.restore_time_ns as f64,
            attr,
        )
        .build()
        .into()
}
