//! Tenant-Isolated `Slice.kv` Operations, Sub-5us State Checkpoints (`checkpoint`/`restore`),
//! and Nested OCap Child Sandbox Attenuation (`eval_sandbox`).

use crate::isolate::SliceIsolate;
use crate::state_and_kv::{
    checkpoint_tenant_state, clear_tenant_checkpoints, current_vfs_state_bytes, edge_kv_del_bytes,
    edge_kv_get, edge_kv_incr_checked, edge_kv_set_checked, edge_kv_tenant_bytes,
    edge_kv_tenant_tree_bytes, release_tenant_checkpoint, restore_or_clear_tenant_tree_kv,
    restore_tenant_state, snapshot_tenant_tree_kv,
};
use serde::{Deserialize, Serialize};
use slice_sandbox::SandboxViolation;
use std::time::Instant;

/// Options for spawning an attenuated nested child sandbox via `Slice.evalSandbox(code, opts)`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxEvalOptions {
    pub tenant_id: Option<String>,
    pub fuel: Option<u64>,
    #[serde(alias = "memory", alias = "maxMemoryBytes")]
    pub memory_bytes: Option<usize>,
    #[serde(default, alias = "readPrefixes", alias = "fsRead")]
    pub allow_read: Vec<String>,
    #[serde(default, alias = "writePrefixes", alias = "fsWrite")]
    pub allow_write: Vec<String>,
    #[serde(default, alias = "virtualMount")]
    pub virtual_mounts: Vec<String>,
    #[serde(default)]
    pub artifacts: bool,
    #[serde(default, alias = "netHosts")]
    pub allow_net: Vec<String>,
    #[serde(default, alias = "allow_private_networks")]
    pub allow_private_networks: Option<bool>,
    #[serde(default, alias = "execBinaries")]
    pub allow_exec: Vec<String>,
    #[serde(default, alias = "envKeys")]
    pub allow_env: Vec<String>,
    pub deterministic_seed: Option<u64>,
    pub audit_secret: Option<String>,
    #[serde(default)]
    pub audit_secrets: Vec<String>,
    #[serde(default, alias = "allowCheckpoints")]
    pub checkpoints: Option<bool>,
    #[serde(default)]
    pub max_checkpoints: Option<u32>,
    #[serde(default, alias = "wallClockMs", alias = "maxWallClockMs")]
    pub timeout_ms: Option<u64>,
}

/// Structured result returned by `Slice.evalSandbox(code, opts)` and `slice mcp` `slice_eval_ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxEvalOutcome {
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub process_exited: bool,
    pub stdout: Vec<String>,
    pub return_value: String,
    pub return_json: Option<String>,
    pub error: Option<String>,
    pub violation: Option<String>,
    #[serde(default)]
    pub denied_capabilities: Vec<String>,
    pub fuel_used: u64,
    pub memory_used_bytes: usize,
    pub peak_memory_bytes: usize,
    pub spawn_time_ns: u64,
    pub compile_time_ns: u64,
    pub exec_time_ns: u64,
    pub total_time_us: f64,
    pub checkpoints_taken: u32,
    pub restores_performed: u32,
    pub restore_time_ns: u64,
    #[serde(skip)]
    pub drained_artifacts: Option<crate::isolate::DrainReport>,
    #[serde(skip)]
    pub(crate) net_persisted_delta_bytes: isize,
}

impl SliceIsolate {
    /// Creates an instant in-memory snapshot (`< 5 us`) of the current thread-local `node:fs` VFS slab
    /// AND this tenant's `Slice.kv` partition, returning a monotonic checkpoint ID for `restore()`.
    #[inline]
    pub fn checkpoint(&self) -> u64 {
        checkpoint_tenant_state(&self.compartment.tenant_id)
    }

    /// Restores the thread-local `node:fs` VFS slab and this tenant's `Slice.kv` partition to the
    /// exact state captured by `checkpoint_id` in `< 5 us`.
    #[inline]
    pub fn restore(&self, checkpoint_id: u64) -> bool {
        restore_tenant_state(&self.compartment.tenant_id, checkpoint_id)
    }

    /// Releases a previously captured checkpoint owned by this tenant from `CHECKPOINT_SLAB`.
    #[inline]
    pub fn release_checkpoint(&self, checkpoint_id: u64) -> bool {
        release_tenant_checkpoint(&self.compartment.tenant_id, checkpoint_id)
    }

    /// Releases all checkpoints owned by this tenant from `CHECKPOINT_SLAB`.
    #[inline]
    pub fn clear_checkpoints(&self) -> usize {
        clear_tenant_checkpoints(&self.compartment.tenant_id)
    }

    /// Reads a value from this tenant's isolated partition in the 16-way sharded `Slice.kv` slab.
    #[inline]
    pub fn kv_get(&self, key: &str) -> Option<String> {
        edge_kv_get(&self.compartment.tenant_id, key)
    }

    /// Writes a value (with optional TTL in milliseconds, `0` = no expiry) into this tenant's
    /// isolated `Slice.kv` partition, charging signed byte deltas against the compartment memory limit.
    #[inline]
    pub fn kv_set(&mut self, key: &str, val: &str, ttl_ms: u64) -> Result<(), SandboxViolation> {
        let baseline = self.compartment.memory_used_bytes.min(4096);
        let live_persisted = edge_kv_tenant_tree_bytes(&self.compartment.tenant_id);
        let current_used = self
            .compartment
            .memory_used_bytes
            .max(baseline.saturating_add(live_persisted));
        self.compartment.memory_used_bytes = current_used;
        let mem_limit = self.compartment.memory_limit_bytes;
        match edge_kv_set_checked(
            &self.compartment.tenant_id,
            key,
            val,
            ttl_ms,
            current_used,
            mem_limit,
        ) {
            Ok(delta) => {
                if delta > 0 {
                    self.compartment.alloc_memory(delta as usize)?;
                } else if delta < 0 {
                    self.compartment.memory_used_bytes = self
                        .compartment
                        .memory_used_bytes
                        .saturating_sub(delta.unsigned_abs())
                        .max(baseline);
                }
                Ok(())
            }
            Err(requested) => Err(SandboxViolation::MemoryCeilingExceeded {
                tenant_id: self.compartment.tenant_id.clone(),
                requested,
                limit: mem_limit,
            }),
        }
    }

    /// Deletes a key from this tenant's isolated `Slice.kv` partition.
    #[inline]
    pub fn kv_del(&mut self, key: &str) -> bool {
        let baseline = self.compartment.memory_used_bytes.min(4096);
        if let Some(freed) = edge_kv_del_bytes(&self.compartment.tenant_id, key) {
            self.compartment.memory_used_bytes = self
                .compartment
                .memory_used_bytes
                .saturating_sub(freed)
                .max(baseline);
            true
        } else {
            false
        }
    }

    /// Atomically increments a 64-bit integer counter in this tenant's isolated `Slice.kv` partition.
    #[inline]
    pub fn kv_incr(&mut self, key: &str, delta: i64) -> Result<i64, SandboxViolation> {
        let baseline = self.compartment.memory_used_bytes.min(4096);
        let live_persisted = edge_kv_tenant_tree_bytes(&self.compartment.tenant_id);
        let current_used = self
            .compartment
            .memory_used_bytes
            .max(baseline.saturating_add(live_persisted));
        self.compartment.memory_used_bytes = current_used;
        let mem_limit = self.compartment.memory_limit_bytes;
        match edge_kv_incr_checked(
            &self.compartment.tenant_id,
            key,
            delta,
            current_used,
            mem_limit,
        ) {
            Ok((next, mem_delta)) => {
                if mem_delta > 0 {
                    self.compartment.alloc_memory(mem_delta as usize)?;
                } else if mem_delta < 0 {
                    self.compartment.memory_used_bytes = self
                        .compartment
                        .memory_used_bytes
                        .saturating_sub(mem_delta.unsigned_abs())
                        .max(baseline);
                }
                Ok(next)
            }
            Err(requested) => Err(SandboxViolation::MemoryCeilingExceeded {
                tenant_id: self.compartment.tenant_id.clone(),
                requested,
                limit: mem_limit,
            }),
        }
    }

    /// Returns the live byte footprint currently retained by this tenant in the `Slice.kv` slab.
    #[inline]
    pub fn live_kv_bytes(&self) -> usize {
        edge_kv_tenant_bytes(&self.compartment.tenant_id)
    }

    /// Synchronizes `compartment.memory_used_bytes` to the 4 KB compartment baseline plus live `Slice.kv` state.
    #[inline]
    pub fn sync_live_kv_memory(&mut self) {
        self.compartment.memory_used_bytes = 4096usize.saturating_add(self.live_kv_bytes());
    }

    /// Compiles (via Oxc) and executes untrusted TypeScript/JavaScript inside a nested child
    /// `SliceIsolate` with strict capability attenuation (child permissions are intersected with
    /// this parent's `CapabilityToken`, and child fuel/memory are bounded by and deducted from
    /// this parent's remaining budget).
    pub fn eval_sandbox(&mut self, code: &str, opts: SandboxEvalOptions) -> SandboxEvalOutcome {
        let start = Instant::now();
        let parent_vfs_ns =
            crate::boa_realm::vfs_owner_namespace(&self.compartment.tenant_id).to_string();
        crate::boa_realm::ensure_vfs_tenant_affinity(&parent_vfs_ns);

        let parent_rem_fuel = self
            .compartment
            .fuel_limit
            .saturating_sub(self.compartment.fuel_consumed);
        let parent_baseline = self.compartment.memory_used_bytes.min(4096);
        let parent_live_persisted = current_vfs_state_bytes()
            .saturating_add(edge_kv_tenant_tree_bytes(&self.compartment.tenant_id));
        let parent_effective_used = self
            .compartment
            .memory_used_bytes
            .max(parent_baseline.saturating_add(parent_live_persisted));
        self.compartment.memory_used_bytes = parent_effective_used;
        let parent_rem_mem = self
            .compartment
            .memory_limit_bytes
            .saturating_sub(parent_effective_used);
        let child_fuel = opts.fuel.unwrap_or(parent_rem_fuel).min(parent_rem_fuel);

        // Attenuate child CapabilityToken strictly against parent's capabilities via canonical OCap lattice
        let mut child_caps =
            self.compartment
                .capabilities
                .attenuate(&slice_sandbox::CapabilityAttenuationRequest {
                    allow_read: (!opts.allow_read.is_empty()).then_some(opts.allow_read.as_slice()),
                    allow_write: (!opts.allow_write.is_empty())
                        .then_some(opts.allow_write.as_slice()),
                    virtual_mounts: (!opts.virtual_mounts.is_empty())
                        .then_some(opts.virtual_mounts.as_slice()),
                    allow_net: (!opts.allow_net.is_empty()).then_some(opts.allow_net.as_slice()),
                    allow_private_networks: opts.allow_private_networks,
                    allow_exec: (!opts.allow_exec.is_empty()).then_some(opts.allow_exec.as_slice()),
                    allow_env: Some(opts.allow_env.as_slice()),
                    allow_checkpoints: opts.checkpoints,
                    max_checkpoints: opts.max_checkpoints,
                    consumed_checkpoints: self.checkpoints_taken,
                });
        if opts.artifacts {
            child_caps.push_virtual_mount("/artifacts");
        }

        let explicit_sub_tenant = opts
            .tenant_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let has_isolated_sub_tenant = explicit_sub_tenant.is_some();
        let child_tenant = match explicit_sub_tenant {
            Some(sub_id) => format!("{}:sub:{}", self.compartment.tenant_id, sub_id),
            None => format!("{}:sub", self.compartment.tenant_id),
        };
        let pre_child_kv_bytes = edge_kv_tenant_tree_bytes(&child_tenant);
        let saved_child_kv = if pre_child_kv_bytes > 0 {
            snapshot_tenant_tree_kv(&child_tenant)
        } else {
            None
        };
        let pre_shared_vfs_bytes = if !has_isolated_sub_tenant {
            current_vfs_state_bytes()
        } else {
            0
        };
        // `parent_rem_mem` already subtracts `pre_child_kv_bytes` and `pre_shared_vfs_bytes`
        // (via `parent_live_persisted`). Because `child_iso` initializes its own `initial_net_used`
        // to `pre_child_kv_bytes + child_vfs_bytes`, allow those existing bytes inside `child_mem`
        // while strictly capping net new allocations to `parent_rem_mem` (and `opts.memory_bytes`).
        let max_child_total = parent_rem_mem
            .saturating_add(pre_child_kv_bytes)
            .saturating_add(pre_shared_vfs_bytes)
            .saturating_add(4096);
        let child_mem = match opts.memory_bytes {
            Some(req) => req.min(max_child_total),
            None => max_child_total,
        };
        let mut child_iso =
            SliceIsolate::spawn(child_tenant.clone(), 99, child_caps, child_fuel, child_mem);
        child_iso.capture_return_json = true;
        let effective_timeout = match (self.max_wall_clock_ms, opts.timeout_ms) {
            (Some(p), Some(c)) => Some(p.min(c)),
            (Some(p), None) => Some(p),
            (None, Some(c)) => Some(c),
            (None, None) => None,
        };
        child_iso.set_wall_clock_limit_ms(effective_timeout);
        child_iso.interrupt_flag = self.interrupt_flag.clone();
        if let Some(seed) = opts.deterministic_seed.or(self.deterministic_seed) {
            child_iso.set_deterministic_seed(Some(seed));
        }
        child_iso.egress_audit_secrets = self.egress_audit_secrets.clone();
        if let Some(sec) = opts.audit_secret {
            child_iso.add_egress_audit_secret(sec);
        }
        for sec in opts.audit_secrets {
            child_iso.add_egress_audit_secret(sec);
        }
        let spawn_time_ns = (start.elapsed().as_nanos() as u64).max(1);

        let compile_start = Instant::now();
        let bc = crate::parser_and_compiler::compile_source_for_sandbox_eval(code);
        let compile_time_ns = (compile_start.elapsed().as_nanos() as u64).max(1);

        let saved_vfs = if has_isolated_sub_tenant {
            Some((
                crate::boa_realm::BOA_VFS_OWNER_TENANT
                    .with(|o| std::mem::take(&mut *o.borrow_mut())),
                crate::boa_realm::BOA_VFS_STATE.with(|v| {
                    let mut vfs = v.borrow_mut();
                    let snap = vfs.snapshot_with_fds();
                    vfs.clear();
                    snap
                }),
            ))
        } else if !child_iso
            .compartment
            .capabilities
            .allowed_write_prefixes
            .is_empty()
            || !child_iso.compartment.capabilities.virtual_mounts.is_empty()
        {
            Some((
                crate::boa_realm::BOA_VFS_OWNER_TENANT.with(|o| o.borrow().clone()),
                crate::boa_realm::BOA_VFS_STATE.with(|v| v.borrow().snapshot_with_fds()),
            ))
        } else {
            None
        };

        let exec_start = Instant::now();
        let exec_res = child_iso.execute_bytecode(&bc, None);
        let exec_time_ns = (exec_start.elapsed().as_nanos() as u64).max(1);

        let drained_artifacts = if opts.artifacts {
            Some(child_iso.drain_vfs("/artifacts", &crate::isolate::DrainLimits::default()))
        } else {
            None
        };

        if exec_res.is_err() {
            restore_or_clear_tenant_tree_kv(&child_tenant, saved_child_kv);
        }
        if (has_isolated_sub_tenant || exec_res.is_err())
            && let Some((owner, snap)) = saved_vfs
        {
            crate::boa_realm::BOA_VFS_OWNER_TENANT.with(|o| *o.borrow_mut() = owner);
            crate::boa_realm::BOA_VFS_STATE.with(|v| v.borrow_mut().restore_from_snapshot(snap));
        }
        if child_iso.checkpoints_taken > 0 {
            clear_tenant_checkpoints(&child_iso.compartment.tenant_id);
        }
        self.checkpoints_taken = self
            .checkpoints_taken
            .saturating_add(child_iso.checkpoints_taken);
        self.restores_performed = self
            .restores_performed
            .saturating_add(child_iso.restores_performed);
        self.restore_time_ns = self
            .restore_time_ns
            .saturating_add(child_iso.restore_time_ns);
        let outcome = match exec_res {
            Ok(mut metrics) => {
                if child_iso.denied_capabilities.len() > metrics.denied_capabilities.len() {
                    metrics.denied_capabilities.extend_from_slice(
                        &child_iso.denied_capabilities[metrics.denied_capabilities.len()..],
                    );
                }
                let _ = self.compartment.consume_fuel(metrics.fuel_consumed.max(1));
                let post_child_kv_bytes = edge_kv_tenant_tree_bytes(&child_tenant);
                let post_shared_vfs_bytes = if !has_isolated_sub_tenant {
                    current_vfs_state_bytes()
                } else {
                    0
                };
                let kv_delta = post_child_kv_bytes as isize - pre_child_kv_bytes as isize;
                let vfs_delta = post_shared_vfs_bytes as isize - pre_shared_vfs_bytes as isize;
                let persisted_delta = kv_delta.saturating_add(vfs_delta);
                if persisted_delta > 0 {
                    self.compartment.memory_used_bytes = self
                        .compartment
                        .memory_used_bytes
                        .saturating_add(persisted_delta as usize)
                        .min(self.compartment.memory_limit_bytes);
                } else if persisted_delta < 0 {
                    self.compartment.memory_used_bytes = self
                        .compartment
                        .memory_used_bytes
                        .saturating_sub(persisted_delta.unsigned_abs())
                        .max(parent_baseline);
                }
                let is_uncaught = metrics.return_value.starts_with("Uncaught ");
                let nonzero_exit = metrics.exit_code.is_some_and(|code| code != 0);
                let (return_value, return_json, error) = if is_uncaught {
                    ("undefined".to_string(), None, Some(metrics.return_value))
                } else if let Some(code) = metrics.exit_code.filter(|&c| c != 0) {
                    (
                        metrics.return_value,
                        child_iso.last_return_json.take(),
                        Some(format!("Process exited with status code {}", code)),
                    )
                } else {
                    (
                        metrics.return_value,
                        child_iso.last_return_json.take(),
                        None,
                    )
                };
                SandboxEvalOutcome {
                    ok: !is_uncaught && !nonzero_exit,
                    exit_code: metrics.exit_code,
                    process_exited: metrics.process_exited,
                    stdout: metrics.stdout,
                    return_value,
                    return_json,
                    error,
                    violation: None,
                    denied_capabilities: metrics.denied_capabilities,
                    fuel_used: metrics.fuel_consumed,
                    memory_used_bytes: metrics.memory_used_bytes,
                    peak_memory_bytes: metrics.memory_used_bytes,
                    spawn_time_ns,
                    compile_time_ns,
                    exec_time_ns,
                    total_time_us: start.elapsed().as_nanos() as f64 / 1_000.0,
                    checkpoints_taken: child_iso.checkpoints_taken,
                    restores_performed: child_iso.restores_performed,
                    restore_time_ns: child_iso.restore_time_ns,
                    drained_artifacts,
                    net_persisted_delta_bytes: persisted_delta,
                }
            }
            Err(violation) => {
                let _ = self
                    .compartment
                    .consume_fuel(child_iso.compartment.fuel_consumed.max(1));
                let raw_stdout = std::mem::take(&mut child_iso.stdout);
                let safe_stdout = if child_iso.egress_audit_secrets.is_empty() {
                    raw_stdout
                } else {
                    raw_stdout
                        .into_iter()
                        .filter(|line| child_iso.verify_egress_not_leaked(line).is_ok())
                        .collect()
                };
                SandboxEvalOutcome {
                    ok: false,
                    exit_code: None,
                    process_exited: false,
                    stdout: safe_stdout,
                    return_value: "undefined".to_string(),
                    return_json: None,
                    error: None,
                    violation: Some(violation.to_string()),
                    denied_capabilities: std::mem::take(&mut child_iso.denied_capabilities),
                    fuel_used: child_iso.compartment.fuel_consumed,
                    memory_used_bytes: child_iso.compartment.memory_used_bytes,
                    peak_memory_bytes: child_iso.compartment.memory_used_bytes,
                    spawn_time_ns,
                    compile_time_ns,
                    exec_time_ns,
                    total_time_us: start.elapsed().as_nanos() as f64 / 1_000.0,
                    checkpoints_taken: child_iso.checkpoints_taken,
                    restores_performed: child_iso.restores_performed,
                    restore_time_ns: child_iso.restore_time_ns,
                    drained_artifacts,
                    net_persisted_delta_bytes: 0,
                }
            }
        };
        if crate::boa_realm::is_trace_recording_enabled() {
            crate::boa_realm::record_trace_event(
                "sandbox.eval",
                "slice.ocap",
                100,
                start.elapsed().as_nanos() as u64,
                serde_json::json!({
                    "parent_tenant": self.compartment.tenant_id,
                    "ok": outcome.ok,
                    "fuel_used": outcome.fuel_used,
                    "memory_used_bytes": outcome.memory_used_bytes,
                    "violation": outcome.violation
                }),
            );
        }
        outcome
    }
}
