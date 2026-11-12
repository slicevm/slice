//! Multi-Tenant Edge KV (`Slice.kv`), Transactional State Checkpoints, and Nested `Slice.evalSandbox` Host Bindings.

use crate::boa_realm::{
    SavedIsolateContextGuard, BOA_ACTIVE_TENANT, BOA_KV_MEM_BUDGET, BOA_VIOLATION,
};
use crate::event_loop::{clear_event_loop_state, drive_event_loop_to_completion};
use crate::isolate::{SandboxEvalOptions, SliceIsolate};
use crate::state_and_kv::{
    checkpoint_tenant_state, edge_kv_del, edge_kv_get, edge_kv_incr, edge_kv_set,
    release_tenant_checkpoint, restore_tenant_state,
};
use slice_sandbox::{CapabilityToken, SandboxViolation};

pub(crate) fn register_kv_and_sandbox_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsValue as BoaVal, NativeFunction};
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
            let added = key.len().saturating_add(val.len());
            let allowed = BOA_KV_MEM_BUDGET.with(|m| {
                let (used, limit) = m.get();
                let next = used.saturating_add(added);
                if next > limit {
                    false
                } else {
                    m.set((next, limit));
                    true
                }
            });
            if !allowed {
                let tenant = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
                let limit = BOA_KV_MEM_BUDGET.with(|m| m.get().1);
                BOA_VIOLATION.with(|v| {
                    *v.borrow_mut() = Some(SandboxViolation::MemoryCeilingExceeded {
                        tenant_id: tenant,
                        requested: added,
                        limit,
                    });
                });
                return Ok(BoaVal::from(false));
            }
            BOA_ACTIVE_TENANT.with(|t| edge_kv_set(&t.borrow(), &key, &val, ttl_ms));
            Ok(BoaVal::from(true))
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
            let removed = BOA_ACTIVE_TENANT.with(|t| edge_kv_del(&t.borrow(), &key));
            Ok(BoaVal::from(removed))
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
            let next = BOA_ACTIVE_TENANT.with(|t| edge_kv_incr(&t.borrow(), &key, delta));
            Ok(BoaVal::from(next as f64))
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
            let ok = BOA_ACTIVE_TENANT.with(|t| restore_tenant_state(&t.borrow(), id));
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
            let opts_json = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "{}".to_string());
            let (opts, deny_read, deny_write, deny_net, deny_exec) = if opts_json == "{}" {
                (SandboxEvalOptions::default(), false, false, false, false)
            } else if let Ok(val) = serde_json::from_str::<serde_json::Value>(&opts_json) {
                let has_empty_arr = |keys: &[&str]| -> bool {
                    val.as_object().is_some_and(|obj| {
                        keys.iter().any(|k| {
                            obj.get(*k)
                                .and_then(|v| v.as_array())
                                .is_some_and(|a| a.is_empty())
                        })
                    })
                };
                let dr = has_empty_arr(&["allowRead", "readPrefixes", "fsRead"]);
                let dw = has_empty_arr(&["allowWrite", "writePrefixes", "fsWrite"]);
                let dn = has_empty_arr(&["allowNet", "netHosts"]);
                let de = has_empty_arr(&["allowExec", "execBinaries"]);
                let parsed: SandboxEvalOptions = serde_json::from_value(val).unwrap_or_default();
                (parsed, dr, dw, dn, de)
            } else {
                (SandboxEvalOptions::default(), false, false, false, false)
            };

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
            let parent_rem_mem = scope_guard
                .saved_kv_budget
                .1
                .saturating_sub(scope_guard.saved_kv_budget.0);
            let mut proxy_parent = SliceIsolate::spawn(
                scope_guard.saved_tenant.clone(),
                98,
                parent_caps,
                parent_rem_fuel,
                parent_rem_mem.saturating_add(4096),
            );
            proxy_parent.egress_audit_secrets = scope_guard.saved_egress_secrets.clone();
            let parent_tenant_for_trace = scope_guard.saved_tenant.clone();
            let outcome = if cfg!(debug_assertions) {
                let vfs_owner = crate::boa_realm::BOA_VFS_OWNER_TENANT.with(|o| o.borrow().clone());
                let vfs_files = crate::boa_realm::BOA_VFS_CACHE.with(|c| c.borrow().clone());
                let vfs_dirs = crate::boa_realm::BOA_VFS_DIRS.with(|d| d.borrow().clone());
                let vfs_tombs = crate::boa_realm::BOA_VFS_TOMBSTONES.with(|t| t.borrow().clone());
                let vfs_mtimes = crate::boa_realm::BOA_VFS_MTIMES.with(|m| m.borrow().clone());
                let (out, new_owner, new_files, new_dirs, new_tombs, new_mtimes) =
                    std::thread::Builder::new()
                        .name("slice-eval-sandbox".to_string())
                        .stack_size(8 * 1024 * 1024)
                        .spawn(move || {
                            crate::boa_realm::BOA_VFS_OWNER_TENANT
                                .with(|o| *o.borrow_mut() = vfs_owner);
                            crate::boa_realm::BOA_VFS_CACHE.with(|c| *c.borrow_mut() = vfs_files);
                            crate::boa_realm::BOA_VFS_DIRS.with(|d| *d.borrow_mut() = vfs_dirs);
                            crate::boa_realm::BOA_VFS_TOMBSTONES
                                .with(|t| *t.borrow_mut() = vfs_tombs);
                            crate::boa_realm::BOA_VFS_MTIMES.with(|m| *m.borrow_mut() = vfs_mtimes);
                            let res = proxy_parent.eval_sandbox(&code, opts);
                            let no =
                                crate::boa_realm::BOA_VFS_OWNER_TENANT.with(|o| o.borrow().clone());
                            let nf = crate::boa_realm::BOA_VFS_CACHE.with(|c| c.borrow().clone());
                            let nd = crate::boa_realm::BOA_VFS_DIRS.with(|d| d.borrow().clone());
                            let nt =
                                crate::boa_realm::BOA_VFS_TOMBSTONES.with(|t| t.borrow().clone());
                            let nm = crate::boa_realm::BOA_VFS_MTIMES.with(|m| m.borrow().clone());
                            (res, no, nf, nd, nt, nm)
                        })
                        .expect("failed to spawn slice-eval-sandbox thread")
                        .join()
                        .expect("slice-eval-sandbox thread panicked");
                crate::boa_realm::BOA_VFS_OWNER_TENANT.with(|o| *o.borrow_mut() = new_owner);
                crate::boa_realm::BOA_VFS_CACHE.with(|c| *c.borrow_mut() = new_files);
                crate::boa_realm::BOA_VFS_DIRS.with(|d| *d.borrow_mut() = new_dirs);
                crate::boa_realm::BOA_VFS_TOMBSTONES.with(|t| *t.borrow_mut() = new_tombs);
                crate::boa_realm::BOA_VFS_MTIMES.with(|m| *m.borrow_mut() = new_mtimes);
                out
            } else {
                proxy_parent.eval_sandbox(&code, opts)
            };
            drop(scope_guard);
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

            let json = serde_json::to_string(&outcome).unwrap_or_else(|_| "{}".to_string());
            Ok(BoaVal::from(js_string!(json)))
        }),
    );
}
