//! Tenant-Isolated `Slice.kv` Operations, Sub-5us State Checkpoints (`checkpoint`/`restore`),
//! and Nested OCap Child Sandbox Attenuation (`eval_sandbox`).

use crate::isolate::SliceIsolate;
use crate::state_and_kv::{
    checkpoint_tenant_state, clear_tenant_checkpoints, edge_kv_del, edge_kv_get, edge_kv_incr,
    edge_kv_set, release_tenant_checkpoint, restore_tenant_state,
};
use serde::{Deserialize, Serialize};
use slice_sandbox::{CapabilityToken, SandboxViolation};
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
    #[serde(default, alias = "netHosts")]
    pub allow_net: Vec<String>,
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
}

/// Structured result returned by `Slice.evalSandbox(code, opts)` and `slice mcp` `slice_eval_ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxEvalOutcome {
    pub ok: bool,
    pub stdout: Vec<String>,
    pub return_value: String,
    pub return_json: Option<String>,
    pub error: Option<String>,
    pub violation: Option<String>,
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
    /// isolated `Slice.kv` partition, charging byte growth against the compartment memory limit.
    #[inline]
    pub fn kv_set(&mut self, key: &str, val: &str, ttl_ms: u64) -> Result<(), SandboxViolation> {
        let added = key.len().saturating_add(val.len());
        self.compartment.alloc_memory(added)?;
        edge_kv_set(&self.compartment.tenant_id, key, val, ttl_ms);
        Ok(())
    }

    /// Deletes a key from this tenant's isolated `Slice.kv` partition.
    #[inline]
    pub fn kv_del(&self, key: &str) -> bool {
        edge_kv_del(&self.compartment.tenant_id, key)
    }

    /// Atomically increments a 64-bit integer counter in this tenant's isolated `Slice.kv` partition.
    #[inline]
    pub fn kv_incr(&mut self, key: &str, delta: i64) -> Result<i64, SandboxViolation> {
        self.compartment.alloc_memory(key.len().saturating_add(8))?;
        Ok(edge_kv_incr(&self.compartment.tenant_id, key, delta))
    }

    /// Compiles (via Oxc) and executes untrusted TypeScript/JavaScript inside a nested child
    /// `SliceIsolate` with strict capability attenuation (child permissions are intersected with
    /// this parent's `CapabilityToken`, and child fuel/memory are bounded by and deducted from
    /// this parent's remaining budget).
    pub fn eval_sandbox(&mut self, code: &str, opts: SandboxEvalOptions) -> SandboxEvalOutcome {
        let start = Instant::now();
        let parent_rem_fuel = self
            .compartment
            .fuel_limit
            .saturating_sub(self.compartment.fuel_consumed);
        let parent_net_used = self.compartment.memory_used_bytes.saturating_sub(4096);
        let parent_rem_mem = self
            .compartment
            .memory_limit_bytes
            .saturating_sub(parent_net_used);
        let child_fuel = opts.fuel.unwrap_or(parent_rem_fuel).min(parent_rem_fuel);
        let child_mem = opts
            .memory_bytes
            .unwrap_or(parent_rem_mem)
            .min(parent_rem_mem);

        // Attenuate child CapabilityToken strictly against parent's capabilities (zero escalation)
        let mut child_caps = CapabilityToken::pure_compute();
        let parent_reads = &self.compartment.capabilities.allowed_read_prefixes;
        if !opts.allow_read.is_empty() {
            for req_prefix in &opts.allow_read {
                if parent_reads
                    .iter()
                    .any(|p| slice_sandbox::vfs_prefix_matches(req_prefix, p))
                {
                    child_caps = child_caps.with_read_prefix(req_prefix.clone());
                }
            }
        } else {
            for p in parent_reads {
                child_caps = child_caps.with_read_prefix(p.clone());
            }
        }
        if child_caps.allowed_read_prefixes.is_empty() {
            child_caps = child_caps.with_read_prefix("/__slice_denied__");
        }

        let parent_writes = &self.compartment.capabilities.allowed_write_prefixes;
        if !opts.allow_write.is_empty() {
            for req_prefix in &opts.allow_write {
                if parent_writes
                    .iter()
                    .any(|p| slice_sandbox::vfs_prefix_matches(req_prefix, p))
                {
                    child_caps = child_caps.with_write_prefix(req_prefix.clone());
                }
            }
        } else {
            for p in parent_writes {
                child_caps = child_caps.with_write_prefix(p.clone());
            }
        }
        if child_caps.allowed_write_prefixes.is_empty() {
            child_caps = child_caps.with_write_prefix("/__slice_denied__");
        }

        let parent_nets = &self.compartment.capabilities.allowed_net_hosts;
        if !opts.allow_net.is_empty() {
            for req_host in &opts.allow_net {
                if !slice_sandbox::is_cloud_metadata_ssrf_target(req_host)
                    && self.compartment.verify_net_connect(req_host).is_ok()
                {
                    child_caps = child_caps.with_net_host(req_host.clone());
                }
            }
        } else {
            for h in parent_nets {
                child_caps = child_caps.with_net_host(h.clone());
            }
        }

        let parent_execs = &self.compartment.capabilities.allowed_exec_binaries;
        if !opts.allow_exec.is_empty() {
            for req_bin in &opts.allow_exec {
                if self.compartment.verify_exec_binary(req_bin).is_ok() {
                    child_caps = child_caps.with_exec_binary(req_bin.clone());
                }
            }
        } else {
            for eb in parent_execs {
                child_caps = child_caps.with_exec_binary(eb.clone());
            }
        }

        if !opts.allow_env.is_empty() {
            let parent_envs = &self.compartment.capabilities.allowed_env_keys;
            for req_key in &opts.allow_env {
                if let Some(val) = parent_envs.get(req_key) {
                    child_caps = child_caps.with_env(req_key.clone(), val.clone());
                }
            }
        }

        let parent_allow_cp = self.compartment.capabilities.allow_checkpoints;
        let child_allow_cp =
            parent_allow_cp && opts.checkpoints.unwrap_or(true) && opts.max_checkpoints != Some(0);
        let parent_rem_cp = self
            .compartment
            .capabilities
            .max_checkpoints
            .map(|m| m.saturating_sub(self.checkpoints_taken));
        let child_max_cp = match (parent_rem_cp, opts.max_checkpoints) {
            (Some(p), Some(c)) => Some(p.min(c)),
            (Some(p), None) => Some(p),
            (None, Some(c)) => Some(c),
            (None, None) => None,
        };
        child_caps = child_caps
            .with_checkpoints(child_allow_cp)
            .with_max_checkpoints(child_max_cp);

        let child_tenant = match opts
            .tenant_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(sub_id) => format!("{}:sub:{}", self.compartment.tenant_id, sub_id),
            None => format!("{}:sub", self.compartment.tenant_id),
        };
        let mut child_iso =
            SliceIsolate::spawn(child_tenant, 99, child_caps, child_fuel, child_mem);
        child_iso.capture_return_json = true;
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

        let exec_start = Instant::now();
        let exec_res = child_iso.execute_bytecode(&bc, None);
        let exec_time_ns = (exec_start.elapsed().as_nanos() as u64).max(1);
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
            Ok(metrics) => {
                let _ = self.compartment.consume_fuel(metrics.fuel_consumed.max(1));
                let net_mem_used = metrics.memory_used_bytes.saturating_sub(4096);
                if net_mem_used > 0 {
                    self.compartment.memory_used_bytes = self
                        .compartment
                        .memory_used_bytes
                        .saturating_add(net_mem_used)
                        .min(self.compartment.memory_limit_bytes);
                }
                let is_uncaught = metrics.return_value.starts_with("Uncaught ");
                let (return_value, return_json, error) = if is_uncaught {
                    ("undefined".to_string(), None, Some(metrics.return_value))
                } else {
                    (
                        metrics.return_value,
                        child_iso.last_return_json.take(),
                        None,
                    )
                };
                SandboxEvalOutcome {
                    ok: !is_uncaught,
                    stdout: metrics.stdout,
                    return_value,
                    return_json,
                    error,
                    violation: None,
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
                    stdout: safe_stdout,
                    return_value: "undefined".to_string(),
                    return_json: None,
                    error: None,
                    violation: Some(violation.to_string()),
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
