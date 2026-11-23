//! Capability-Attenuated MicroVM Worker Script Resolution, Compilation, and Spawn Budgeting.

use super::pool::dispatch_worker_job;
use super::{
    GLOBAL_ACTIVE_WORKERS, MAX_CACHED_WORKER_SCRIPTS, MAX_GLOBAL_ACTIVE_WORKERS,
    MAX_WORKERS_PER_ISOLATE, PARENT_WORKERS, ParentWorkerHandle, WORKER_ACTIVE_COUNT,
    WORKER_SCRIPT_CACHE_HITS, WorkerJob, WorkerSliceAttenuationOpts, WorkerSpawnOptions,
    charge_worker_message_budget, worker_script_cache,
};
use crate::boa_realm::{
    BOA_ACTIVE_CAPS, BOA_ACTIVE_TENANT, BOA_DET_STATE, BOA_EGRESS_SECRETS, BOA_ENV_MAP,
    BOA_EXEC_BINS, BOA_FETCH_MOCKS, BOA_FUEL_BUDGET, BOA_KV_MEM_BUDGET, BOA_NET_HOSTS,
    BOA_VFS_STATE, BOA_VIOLATION, BoaFsCaps, DIRTY_WORKERS, check_boa_fs_read_ocap,
    mark_subsystem_dirty, norm_vfs_path,
};
use crate::intrinsics::base64_decode;
use crate::sockets::BOA_SOCKET_MOCKS;
use crate::websocket::BOA_WS_MOCKS;
use boa_engine::Context;
use rustc_hash::{FxHashMap, FxHashSet};
use slice_sandbox::{CapabilityToken, SandboxViolation};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

fn resolve_worker_script_source(specifier_or_code: &str, is_eval: bool) -> Result<String, String> {
    if is_eval {
        return Ok(specifier_or_code.to_string());
    }
    let trimmed = specifier_or_code.trim();
    if let Some(data_rest) = trimmed.strip_prefix("data:") {
        if let Some((meta, body)) = data_rest.split_once(',') {
            if meta.ends_with(";base64") {
                let bytes = base64_decode(body);
                return Ok(String::from_utf8_lossy(&bytes).into_owned());
            }
            return Ok(body.to_string());
        }
        return Err("WorkerError: Invalid data: URL for Worker script".to_string());
    }

    let raw_path = trimmed.strip_prefix("file://").unwrap_or(trimmed);
    if !check_boa_fs_read_ocap(raw_path) {
        return Err(format!(
            "SandboxViolation: Worker script read '{}' denied by OCap read policy",
            raw_path
        ));
    }
    let norm = norm_vfs_path(raw_path);
    if norm != raw_path && !check_boa_fs_read_ocap(&norm) {
        return Err(format!(
            "SandboxViolation: Worker script read '{}' denied by OCap read policy",
            norm
        ));
    }

    // 1. Resolve VFS + host symlinks with per-hop OCap containment
    let resolved_norm = match crate::bindings::fs::resolve_vfs_or_host_script(raw_path) {
        Ok((_, Some(vfs_src))) => return Ok(String::from_utf8_lossy(&vfs_src).into_owned()),
        Ok((resolved_norm, None)) => {
            if !check_boa_fs_read_ocap(&resolved_norm) {
                return Err(format!(
                    "SandboxViolation: Worker script read '{}' denied by OCap read policy",
                    resolved_norm
                ));
            }
            resolved_norm
        }
        Err(crate::bindings::fs::SymlinkResolveError::ReadOcapDenied(denied)) => {
            let _ = check_boa_fs_read_ocap(&denied);
            return Err(format!(
                "SandboxViolation: Worker script read '{}' denied by OCap read policy",
                denied
            ));
        }
        Err(crate::bindings::fs::SymlinkResolveError::Loop) => {
            return Err(format!(
                "WorkerError: Cannot read worker module '{}': ELOOP too many symbolic links",
                raw_path
            ));
        }
    };

    if crate::bindings::fs::is_vfs_tombstoned(&resolved_norm)
        || crate::bindings::fs::is_host_unc_blocked(raw_path)
    {
        return Err(format!(
            "WorkerError: Cannot read worker module '{}': file removed or blocked in VFS",
            raw_path
        ));
    }

    // 2. Fallback to host filesystem if OCap permits
    std::fs::read_to_string(&resolved_norm).map_err(|e| {
        format!(
            "WorkerError: Cannot read worker module '{}': {}",
            raw_path, e
        )
    })
}

fn build_attenuated_worker_caps(
    attenuation: Option<&WorkerSliceAttenuationOpts>,
    parent_caps: &BoaFsCaps,
    parent_net_hosts: &FxHashSet<String>,
    parent_env_map: &FxHashMap<String, String>,
) -> CapabilityToken {
    let parent_execs = BOA_EXEC_BINS.with(|eb_cell| eb_cell.borrow().clone());
    let (parent_allow_cp, parent_max_cp_raw) =
        crate::boa_realm::BOA_CHECKPOINT_CAP.with(|c| c.get());
    let consumed_cp = crate::boa_realm::BOA_CHECKPOINT_STATS.with(|s| s.get().0);
    let parent_token = CapabilityToken {
        allowed_read_prefixes: parent_caps.read_prefixes.clone(),
        allowed_write_prefixes: parent_caps.write_prefixes.clone(),
        allowed_net_hosts: parent_net_hosts.clone(),
        allowed_env_keys: parent_env_map.clone(),
        allowed_exec_binaries: parent_execs,
        allow_checkpoints: parent_allow_cp,
        max_checkpoints: (parent_max_cp_raw < u32::MAX).then_some(parent_max_cp_raw),
    };
    parent_token.attenuate(&slice_sandbox::CapabilityAttenuationRequest {
        allow_read: attenuation.and_then(|a| a.allow_read.as_deref()),
        allow_write: attenuation.and_then(|a| a.allow_write.as_deref()),
        allow_net: attenuation.and_then(|a| a.allow_net.as_deref()),
        allow_exec: attenuation.and_then(|a| a.allow_exec.as_deref()),
        allow_env: attenuation.and_then(|a| a.allow_env.as_deref()),
        allow_checkpoints: attenuation.and_then(|a| a.checkpoints),
        max_checkpoints: attenuation.and_then(|a| a.max_checkpoints),
        consumed_checkpoints: consumed_cp,
    })
}

/// Spawns a capability-attenuated MicroVM Worker thread and returns its `worker_id`.
pub fn worker_spawn(specifier_or_code: &str, options_json: &str) -> Result<u32, String> {
    worker_spawn_with_context(specifier_or_code, options_json, None)
}

/// Spawns a capability-attenuated MicroVM Worker thread while reserving CPU fuel and memory
/// directly against the active parent `Context` and isolate budgets.
pub(crate) fn worker_spawn_with_context(
    specifier_or_code: &str,
    options_json: &str,
    ctx_opt: Option<&mut Context>,
) -> Result<u32, String> {
    mark_subsystem_dirty(DIRTY_WORKERS);
    let opts: WorkerSpawnOptions = serde_json::from_str(options_json).unwrap_or_default();
    let active_in_isolate = PARENT_WORKERS.with(|reg| {
        reg.borrow()
            .workers
            .values()
            .filter(|w| !w.finished)
            .count()
    });
    if active_in_isolate >= MAX_WORKERS_PER_ISOLATE {
        return Err(format!(
            "RangeError: Maximum active MicroVM workers per isolate ({}) exceeded",
            MAX_WORKERS_PER_ISOLATE
        ));
    }
    let prev_global = GLOBAL_ACTIVE_WORKERS.fetch_add(1, Ordering::AcqRel);
    if prev_global >= MAX_GLOBAL_ACTIVE_WORKERS {
        GLOBAL_ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
        return Err(format!(
            "RangeError: Global MicroVM worker pool limit ({}) reached",
            MAX_GLOBAL_ACTIVE_WORKERS
        ));
    }

    let raw_script = match resolve_worker_script_source(specifier_or_code, opts.eval) {
        Ok(s) => s,
        Err(e) => {
            GLOBAL_ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
            return Err(e);
        }
    };

    let worker_data_json = opts
        .worker_data_json
        .unwrap_or_else(|| "undefined".to_string());
    if let Err(e) = charge_worker_message_budget(worker_data_json.len()) {
        GLOBAL_ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
        return Err(e);
    }

    // Snapshot parent compartment state & attenuate capabilities
    let parent_tenant = BOA_ACTIVE_TENANT.with(|t| {
        let b = t.borrow();
        if b.is_empty() {
            "boa-tenant".to_string()
        } else {
            b.clone()
        }
    });

    // Enforce parent CPU fuel ceiling and reserve child's fuel slice upfront
    let loop_iters = ctx_opt
        .as_ref()
        .map(|c| c.runtime_limits().loop_iterations_executed())
        .unwrap_or(0);
    let (consumed_fuel, limit_fuel) = BOA_FUEL_BUDGET.with(|f| f.get());
    let total_parent_fuel = consumed_fuel.saturating_add(loop_iters);
    let avail_fuel_for_child = limit_fuel
        .saturating_sub(total_parent_fuel)
        .saturating_sub(64);
    if avail_fuel_for_child < 100 {
        GLOBAL_ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
        BOA_VIOLATION.with(|v| {
            if v.borrow().is_none() {
                *v.borrow_mut() = Some(SandboxViolation::CpuFuelExhausted {
                    tenant_id: parent_tenant.clone(),
                    limit: limit_fuel,
                });
            }
        });
        if let Some(ctx) = ctx_opt {
            ctx.runtime_limits_mut().set_loop_iteration_limit(1);
        }
        return Err(format!(
            "RangeError: Parent isolate CPU fuel budget ({}) exhausted before Worker spawn",
            limit_fuel
        ));
    }

    // Enforce parent memory ceiling and reserve child's memory slice upfront
    let (used_mem, limit_mem) = BOA_KV_MEM_BUDGET.with(|b| b.get());
    let avail_mem_for_child = limit_mem.saturating_sub(used_mem).saturating_sub(65_536);
    if avail_mem_for_child < 65_536 {
        GLOBAL_ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
        let requested = used_mem.saturating_add(131_072);
        BOA_VIOLATION.with(|v| {
            if v.borrow().is_none() {
                *v.borrow_mut() = Some(SandboxViolation::MemoryCeilingExceeded {
                    tenant_id: parent_tenant.clone(),
                    requested,
                    limit: limit_mem,
                });
            }
        });
        return Err(format!(
            "RangeError: Parent isolate memory cage ({} bytes) exhausted before Worker spawn",
            limit_mem
        ));
    }

    let parent_caps = BOA_ACTIVE_CAPS.with(|c| c.borrow().clone());
    let parent_net_hosts = BOA_NET_HOSTS.with(|h| h.borrow().clone());
    let parent_env_map = BOA_ENV_MAP.with(|m| m.borrow().clone());
    let parent_egress_secrets = BOA_EGRESS_SECRETS.with(|s| s.borrow().clone());
    let parent_vfs_snap = BOA_VFS_STATE.with(|v| v.borrow().snapshot_without_fds());
    let parent_fetch_mocks = BOA_FETCH_MOCKS.with(|m| m.borrow().clone());
    let parent_ws_mocks = BOA_WS_MOCKS.with(|m| m.borrow().clone());
    let parent_sock_mocks = BOA_SOCKET_MOCKS.with(|m| m.borrow().clone());
    let parent_det = BOA_DET_STATE.with(|s| s.get());

    let child_caps = build_attenuated_worker_caps(
        opts.slice.as_ref(),
        &parent_caps,
        &parent_net_hosts,
        &parent_env_map,
    );
    let child_fuel = match opts.slice.as_ref().and_then(|s| s.fuel) {
        Some(req_fuel) => req_fuel.clamp(100, 50_000_000).min(avail_fuel_for_child),
        None => (avail_fuel_for_child / 2)
            .clamp(100, 5_000_000)
            .min(avail_fuel_for_child),
    };
    let child_mem = match opts.slice.as_ref().and_then(|s| s.memory_bytes) {
        Some(req_mem) => req_mem.clamp(65_536, avail_mem_for_child),
        None => (avail_mem_for_child / 4)
            .clamp(65_536, 4 * 1024 * 1024)
            .min(avail_mem_for_child),
    };

    let new_consumed_fuel = consumed_fuel.saturating_add(child_fuel);
    BOA_FUEL_BUDGET.with(|f| f.set((new_consumed_fuel, limit_fuel)));
    BOA_KV_MEM_BUDGET.with(|b| b.set((used_mem.saturating_add(child_mem), limit_mem)));
    if let Some(ctx) = ctx_opt {
        let parent_loop_cap = limit_fuel
            .saturating_sub(new_consumed_fuel)
            .max(loop_iters.saturating_add(1));
        ctx.runtime_limits_mut()
            .set_loop_iteration_limit(parent_loop_cap);
    }

    let (inbound_tx, inbound_rx) = mpsc::channel();
    let terminated = Arc::new(AtomicBool::new(false));
    let terminated_clone = Arc::clone(&terminated);

    let (worker_id, event_tx) = PARENT_WORKERS.with(|reg| {
        let mut r = reg.borrow_mut();
        let id = r.next_worker_id;
        r.next_worker_id = r.next_worker_id.wrapping_add(1).max(1);
        let tx = r.event_tx.clone();
        r.workers.insert(
            id,
            ParentWorkerHandle {
                inbound_tx,
                terminated,
                finished: false,
                refed: true,
                reserved_fuel: child_fuel,
                reserved_mem: child_mem,
                reconciled: false,
            },
        );
        (id, tx)
    });
    WORKER_ACTIVE_COUNT.with(|c| c.set(c.get().saturating_add(1)));

    let worker_name = opts.name.unwrap_or_else(|| format!("worker-{}", worker_id));
    let child_tenant = format!("{}:worker", parent_tenant);
    let bytecode = compile_or_get_worker_bytecode(&raw_script);

    let job = WorkerJob {
        worker_id,
        worker_name,
        child_tenant,
        worker_data_json,
        child_caps,
        child_fuel,
        child_mem,
        parent_det,
        parent_egress_secrets,
        parent_vfs_snap,
        parent_fetch_mocks,
        parent_ws_mocks,
        parent_sock_mocks,
        inbound_rx,
        event_tx,
        terminated: terminated_clone,
        bytecode,
    };

    if let Err(e) = dispatch_worker_job(job) {
        GLOBAL_ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
        PARENT_WORKERS.with(|reg| {
            let mut r = reg.borrow_mut();
            if let Some(mut removed) = r.workers.remove(&worker_id) {
                removed.reconcile_resources(0);
            }
            WORKER_ACTIVE_COUNT.with(|c| c.set(r.workers.len()));
        });
        return Err(format!("WorkerSpawnError: {}", e));
    }

    Ok(worker_id)
}

fn compile_or_get_worker_bytecode(raw_script: &str) -> Arc<Vec<u8>> {
    let mut hasher = rustc_hash::FxHasher::default();
    raw_script.hash(&mut hasher);
    let script_hash = hasher.finish();

    if let Ok(cache) = worker_script_cache().lock()
        && let Some(existing) = cache.get(&script_hash)
    {
        WORKER_SCRIPT_CACHE_HITS.fetch_add(1, Ordering::Relaxed);
        return Arc::clone(existing);
    }

    let transpiled_js = slice_cas::strip_typescript_and_normalize(raw_script, true);
    let compacted_js = crate::parser_and_compiler::compact_js_for_boa_bytecode(&transpiled_js);
    let rc_key = format!("{:016x}", script_hash);
    let wrapped = if compacted_js.starts_with("var __slice_modules = {};//__SLICE_RC:") {
        format!("SLICEBOA{{\n{}\n}}", compacted_js.trim_end())
    } else {
        format!(
            "SLICEBOA{{\nvar __slice_modules = {{}};//__SLICE_RC:{}\nvar __rc = globalThis.__slice_rc || (globalThis.__slice_rc = Object.create(null));\nvar __run = __rc[\"{}\"] || (__rc[\"{}\"] = function() {{\nvar exports = {{}}, module = {{ exports: exports }};\n{}\n}});\n__run();\n}}",
            rc_key,
            rc_key,
            rc_key,
            compacted_js.trim_end()
        )
    };
    let arc_bc = Arc::new(wrapped.into_bytes());

    if let Ok(mut cache) = worker_script_cache().lock() {
        if cache.len() >= MAX_CACHED_WORKER_SCRIPTS {
            cache.clear();
        }
        cache.insert(script_hash, Arc::clone(&arc_bc));
    }
    arc_bc
}
