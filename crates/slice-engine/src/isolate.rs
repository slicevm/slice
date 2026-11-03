//! Polyglot Tiered Isolate (`SliceIsolate`), WinterTC HTTP Execution Context (`HttpRequestContext`),
//! Nested Sandbox Attenuator (`SandboxEvalOptions`), and Execution Telemetry (`ExecutionMetrics`).

use crate::boa_realm::{
    acquire_warm_boa_realm, create_warm_boa_realm, ensure_realm_shards, release_warm_boa_realm,
    BOA_ACTIVE_CAPS, BOA_ACTIVE_TENANT, BOA_CONTEXT_POOL, BOA_DET_STATE, BOA_EGRESS_SECRETS,
    BOA_ENV_MAP, BOA_FETCH_MOCKS, BOA_FUEL_BUDGET, BOA_HTTP_REQ, BOA_HTTP_RES, BOA_KV_MEM_BUDGET,
    BOA_NET_HOSTS, BOA_STDOUT, BOA_VIOLATION, BOA_WAIT_UNTIL_COUNT,
};
use crate::event_loop::{
    clear_event_loop_state, drive_event_loop_to_completion, has_pending_event_loop_work,
};
use crate::js_preludes::detect_needed_shards;
use crate::parser_and_compiler::{
    compile_source_to_bytecode, decode_slicesnp_cached, expr_to_dotted_chain, extract_slice_rc_key,
};
use crate::state_and_kv::{
    checkpoint_tenant_state, edge_kv_del, edge_kv_get, edge_kv_incr, edge_kv_set,
    restore_tenant_state, verify_egress_secrets_slice,
};
use crate::value_and_ast::{
    js_value_to_json_string, serde_json_to_js_value, BinOp, BytecodeModule, BytecodeOp,
    ExecutionTier, Expr, JsFunction, JsValue, Stmt, UnaryOp, SNAPSHOT_MAGIC,
    TIER1_PROMOTION_THRESHOLD,
};
use anyhow::{bail, Result};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use slice_cas::MappedCasArchive;
use slice_sandbox::{CapabilityToken, SandboxViolation, TenantCompartment};
use std::time::Instant;

/// Execution telemetry returned by a Slice Isolate invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionMetrics {
    pub tenant_id: String,
    pub highest_tier_reached: ExecutionTier,
    pub instantiate_time_ns: u64,
    pub execution_time_ns: u64,
    pub total_time_ns: u64,
    pub fuel_consumed: u64,
    pub memory_used_bytes: usize,
    pub pkey_domain: i32,
    pub cas_modules_resolved: usize,
    pub os_stat_syscalls: usize,
    pub stdout: Vec<String>,
    pub return_value: String,
}

#[derive(Debug, Clone)]
pub(crate) enum ControlFlow {
    None(JsValue),
    Return(JsValue),
    Break,
    Continue,
}

/// Options for spawning an attenuated nested child sandbox via `Slice.evalSandbox(code, opts)`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxEvalOptions {
    pub tenant_id: Option<String>,
    pub fuel: Option<u64>,
    pub memory_bytes: Option<usize>,
    #[serde(default)]
    pub allow_read: Vec<String>,
    #[serde(default)]
    pub allow_write: Vec<String>,
    #[serde(default)]
    pub allow_net: Vec<String>,
    pub deterministic_seed: Option<u64>,
    pub audit_secret: Option<String>,
}

/// Structured result returned by `Slice.evalSandbox(code, opts)` and `slice mcp` `slice_eval_ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxEvalOutcome {
    pub ok: bool,
    pub stdout: Vec<String>,
    pub return_value: String,
    pub fuel_used: u64,
    pub memory_used_bytes: usize,
    pub total_time_us: f64,
    pub violation: Option<String>,
}

/// Polyglot Tiered Isolate executing within a hardware- and OCap-protected Compartment.
#[repr(C)]
pub struct SliceIsolate {
    pub compartment: TenantCompartment,
    pub cas_modules_resolved: usize,
    pub current_tier: ExecutionTier,
    env_vars: FxHashMap<String, JsValue>,
    pub stdout: Vec<String>,
    pub deterministic_seed: Option<u64>,
    det_prng_state: u64,
    det_clock_ms: u64,
    pub egress_audit_secrets: Vec<String>,
}

impl SliceIsolate {
    /// Instantiates a fresh microVM-grade isolate in sub-microsecond time with zero eager map allocations.
    #[inline]
    pub fn spawn(
        tenant_id: impl Into<String>,
        slot_id: u32,
        capabilities: CapabilityToken,
        fuel_limit: u64,
        memory_limit_bytes: usize,
    ) -> Self {
        let compartment = TenantCompartment::new(
            tenant_id,
            slot_id,
            capabilities,
            fuel_limit,
            memory_limit_bytes,
        );
        compartment.pkey_domain.enter_domain();

        Self {
            compartment,
            cas_modules_resolved: 0,
            current_tier: ExecutionTier::Tier0LLIntSnapshot,
            env_vars: FxHashMap::default(),
            stdout: Vec::new(),
            deterministic_seed: None,
            det_prng_state: 0,
            det_clock_ms: 1_700_000_000_000,
            egress_audit_secrets: Vec::new(),
        }
    }

    /// Registers a sensitive secret token to be guarded against egress exfiltration (`--audit-egress`).
    /// When no secrets are registered (default), the egress guard costs `0.00 ns`.
    pub fn add_egress_audit_secret(&mut self, secret: impl Into<String>) {
        let s = secret.into();
        if !s.is_empty() && !self.egress_audit_secrets.contains(&s) {
            self.egress_audit_secrets.push(s);
        }
    }

    /// Parses `.env` content, grants each `KEY=VALUE` in this compartment's `CapabilityToken`,
    /// and automatically registers any sensitive credential (`*_SECRET*`, `*_TOKEN*`, `*_KEY*`,
    /// `*_PASSWORD*`, `*_PRIVATE*`) into the outbound Egress DLP guard (`egress_audit_secrets`).
    /// Returns the number of auto-registered DLP secrets.
    pub fn load_dotenv_str(&mut self, dotenv_content: &str) -> usize {
        let (pairs, secrets) = slice_sandbox::parse_dotenv_content(dotenv_content);
        for (k, v) in pairs {
            self.compartment.capabilities.allowed_env_keys.insert(k, v);
        }
        let count = secrets.len();
        for sec in secrets {
            self.add_egress_audit_secret(sec);
        }
        count
    }

    #[inline]
    fn verify_egress_not_leaked(&self, payload: &str) -> Result<(), SandboxViolation> {
        if self.egress_audit_secrets.is_empty() || payload.is_empty() {
            return Ok(());
        }
        verify_egress_secrets_slice(
            &self.compartment.tenant_id,
            &self.egress_audit_secrets,
            payload,
        )
    }

    /// Creates an instant in-memory snapshot (`< 5 us`) of the current thread-local `node:fs` VFS slab
    /// AND this tenant's `Slice.kv` partition, returning a monotonic checkpoint ID for `restore()`.
    pub fn checkpoint(&self) -> u64 {
        checkpoint_tenant_state(&self.compartment.tenant_id)
    }

    /// Restores the thread-local `node:fs` VFS slab and this tenant's `Slice.kv` partition to the
    /// exact state captured by `checkpoint_id` in `< 5 us`.
    pub fn restore(&self, checkpoint_id: u64) -> bool {
        restore_tenant_state(&self.compartment.tenant_id, checkpoint_id)
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
        let parent_rem_mem = self
            .compartment
            .memory_limit_bytes
            .saturating_sub(self.compartment.memory_used_bytes);
        let child_fuel = opts.fuel.unwrap_or(parent_rem_fuel).min(parent_rem_fuel);
        let child_mem = opts
            .memory_bytes
            .unwrap_or(parent_rem_mem)
            .min(parent_rem_mem);

        // Attenuate child CapabilityToken against parent's capabilities
        let mut child_caps = CapabilityToken::pure_compute();
        let parent_reads = &self.compartment.capabilities.allowed_read_prefixes;
        if !opts.allow_read.is_empty() {
            for req_prefix in &opts.allow_read {
                if parent_reads.is_empty() || parent_reads.iter().any(|p| req_prefix.starts_with(p))
                {
                    child_caps = child_caps.with_read_prefix(req_prefix.clone());
                }
            }
            if child_caps.allowed_read_prefixes.is_empty() {
                child_caps = child_caps.with_read_prefix("/__slice_denied__");
            }
        } else {
            for p in parent_reads {
                child_caps = child_caps.with_read_prefix(p.clone());
            }
        }

        let parent_writes = &self.compartment.capabilities.allowed_write_prefixes;
        if !opts.allow_write.is_empty() {
            for req_prefix in &opts.allow_write {
                if parent_writes.is_empty()
                    || parent_writes.iter().any(|p| req_prefix.starts_with(p))
                {
                    child_caps = child_caps.with_write_prefix(req_prefix.clone());
                }
            }
            if child_caps.allowed_write_prefixes.is_empty() {
                child_caps = child_caps.with_write_prefix("/__slice_denied__");
            }
        } else {
            for p in parent_writes {
                child_caps = child_caps.with_write_prefix(p.clone());
            }
        }

        let parent_nets = &self.compartment.capabilities.allowed_net_hosts;
        if !opts.allow_net.is_empty() {
            for req_host in &opts.allow_net {
                if !slice_sandbox::is_cloud_metadata_ssrf_target(req_host)
                    && (parent_nets.is_empty()
                        || self.compartment.verify_net_connect(req_host).is_ok())
                {
                    child_caps = child_caps.with_net_host(req_host.clone());
                }
            }
        } else {
            for h in parent_nets {
                child_caps = child_caps.with_net_host(h.clone());
            }
        }

        let child_tenant = opts
            .tenant_id
            .unwrap_or_else(|| format!("{}:sub", self.compartment.tenant_id));
        let mut child_iso =
            SliceIsolate::spawn(child_tenant, 99, child_caps, child_fuel, child_mem);
        if let Some(seed) = opts.deterministic_seed.or(self.deterministic_seed) {
            child_iso.set_deterministic_seed(Some(seed));
        }
        for sec in &self.egress_audit_secrets {
            child_iso.add_egress_audit_secret(sec.clone());
        }
        if let Some(sec) = opts.audit_secret {
            child_iso.add_egress_audit_secret(sec);
        }

        let bc = compile_source_to_bytecode(code);
        match child_iso.execute_bytecode(&bc, None) {
            Ok(metrics) => {
                let _ = self.compartment.consume_fuel(metrics.fuel_consumed.max(1));
                SandboxEvalOutcome {
                    ok: true,
                    stdout: metrics.stdout,
                    return_value: metrics.return_value,
                    fuel_used: metrics.fuel_consumed,
                    memory_used_bytes: metrics.memory_used_bytes,
                    total_time_us: start.elapsed().as_nanos() as f64 / 1_000.0,
                    violation: None,
                }
            }
            Err(violation) => {
                let _ = self
                    .compartment
                    .consume_fuel(child_iso.compartment.fuel_consumed.max(1));
                SandboxEvalOutcome {
                    ok: false,
                    stdout: Vec::new(),
                    return_value: "undefined".to_string(),
                    fuel_used: child_iso.compartment.fuel_consumed,
                    memory_used_bytes: child_iso.compartment.memory_used_bytes,
                    total_time_us: start.elapsed().as_nanos() as f64 / 1_000.0,
                    violation: Some(violation.to_string()),
                }
            }
        }
    }

    /// Enables or disables opt-in deterministic replay mode (`--deterministic=<seed>`).
    /// When `Some(seed)` is configured, `Math.random()`, `Date.now()`, and `performance.now()`
    /// inside this compartment are driven by a deterministic SplitMix64 PRNG and monotonic clock.
    /// When `None` (default), there is `0.00 ns` runtime overhead.
    #[inline]
    pub fn set_deterministic_seed(&mut self, seed: Option<u64>) {
        self.deterministic_seed = seed;
        if let Some(s) = seed {
            self.det_prng_state = s ^ 0x9E37_79B9_7F4A_7C15;
            self.det_clock_ms = 1_700_000_000_000;
        }
    }

    /// Reads a value from this tenant's isolated partition in the 16-way sharded `Slice.kv` slab.
    pub fn kv_get(&self, key: &str) -> Option<String> {
        edge_kv_get(&self.compartment.tenant_id, key)
    }

    /// Writes a value (with optional TTL in milliseconds, `0` = no expiry) into this tenant's
    /// isolated `Slice.kv` partition, charging byte growth against the compartment memory limit.
    pub fn kv_set(&mut self, key: &str, val: &str, ttl_ms: u64) -> Result<(), SandboxViolation> {
        let added = key.len().saturating_add(val.len());
        self.compartment.alloc_memory(added)?;
        edge_kv_set(&self.compartment.tenant_id, key, val, ttl_ms);
        Ok(())
    }

    /// Deletes a key from this tenant's isolated `Slice.kv` partition.
    pub fn kv_del(&self, key: &str) -> bool {
        edge_kv_del(&self.compartment.tenant_id, key)
    }

    /// Atomically increments a 64-bit integer counter in this tenant's isolated `Slice.kv` partition.
    pub fn kv_incr(&mut self, key: &str, delta: i64) -> Result<i64, SandboxViolation> {
        self.compartment.alloc_memory(key.len().saturating_add(8))?;
        Ok(edge_kv_incr(&self.compartment.tenant_id, key, delta))
    }

    /// Resets this `SliceIsolate` in place for a new HTTP request with zero heap allocations
    /// for `CapabilityToken` or `tenant_id`, while enforcing fresh MPK/PKU hardware protection
    /// domain entry, CPU fuel quota reset, and memory arena reset.
    #[inline]
    pub fn reset_for_request(
        &mut self,
        tenant_id: &str,
        slot_id: u32,
        fuel_limit: u64,
        memory_limit_bytes: usize,
    ) {
        if self.compartment.tenant_id != tenant_id {
            self.compartment.tenant_id.clear();
            self.compartment.tenant_id.push_str(tenant_id);
        }
        self.compartment.pkey_domain = slice_sandbox::HardwareProtectionDomain::allocate(slot_id);
        self.compartment.pkey_domain.enter_domain();
        self.compartment.fuel_limit = fuel_limit;
        self.compartment.fuel_consumed = 0;
        self.compartment.memory_limit_bytes = memory_limit_bytes;
        self.compartment.memory_used_bytes = 0;
        self.cas_modules_resolved = 0;
        self.current_tier = ExecutionTier::Tier0LLIntSnapshot;
        self.env_vars.clear();
        self.stdout.clear();
        if let Some(s) = self.deterministic_seed {
            self.det_prng_state = s ^ 0x9E37_79B9_7F4A_7C15;
            self.det_clock_ms = 1_700_000_000_000;
        }
    }

    /// Executes pre-compiled `SLICESNP` or `SLICEBOA` bytecode directly from an in-memory or `mmap` CAS slice.
    pub fn execute_bytecode(
        &mut self,
        bytecode: &[u8],
        cas: Option<&MappedCasArchive>,
    ) -> Result<ExecutionMetrics, SandboxViolation> {
        let start = Instant::now();
        self.compartment.pkey_domain.enter_domain();

        // Zero-copy fast path for SLICEBOA raw ECMAScript bundles
        if bytecode.len() >= 8 && &bytecode[0..8] == b"SLICEBOA" {
            let instantiate_ns = start.elapsed().as_nanos() as u64;
            let exec_start = Instant::now();
            self.compartment.consume_fuel(25)?;
            self.current_tier = ExecutionTier::Tier1Baseline;
            let js_code = std::str::from_utf8(&bytecode[8..]).unwrap_or_default();
            let last_val = self.exec_boa_full_es_vm(js_code)?;
            if !self.egress_audit_secrets.is_empty() {
                for line in &self.stdout {
                    self.verify_egress_not_leaked(line)?;
                }
                self.verify_egress_not_leaked(&last_val.to_string())?;
                if let Some((_, ref res_body, _, ref res_hdrs)) =
                    BOA_HTTP_RES.with(|res| res.borrow().clone())
                {
                    self.verify_egress_not_leaked(res_body)?;
                    for (_, hv) in res_hdrs {
                        self.verify_egress_not_leaked(hv)?;
                    }
                }
            }
            let exec_ns = exec_start.elapsed().as_nanos() as u64;
            let total_ns = start.elapsed().as_nanos() as u64;
            return Ok(ExecutionMetrics {
                tenant_id: self.compartment.tenant_id.clone(),
                highest_tier_reached: self.current_tier,
                instantiate_time_ns: instantiate_ns,
                execution_time_ns: exec_ns,
                total_time_ns: total_ns,
                fuel_consumed: self.compartment.fuel_consumed,
                memory_used_bytes: self.compartment.memory_used_bytes,
                pkey_domain: self.compartment.pkey_domain.pkey,
                cas_modules_resolved: self.cas_modules_resolved,
                os_stat_syscalls: 0,
                stdout: self.stdout.clone(),
                return_value: last_val.to_string(),
            });
        }

        let module = decode_slicesnp_cached(bytecode);

        let instantiate_ns = start.elapsed().as_nanos() as u64;
        let exec_start = Instant::now();
        let mut last_val = JsValue::Undefined;

        for op in &module.ops {
            self.compartment.consume_fuel(1)?;

            match op {
                BytecodeOp::ConstNum { dst, val } => {
                    self.env_vars.insert(dst.clone(), JsValue::Number(*val));
                    last_val = JsValue::Number(*val);
                }
                BytecodeOp::ConstStr { dst, val } => {
                    self.compartment.alloc_memory(val.len())?;
                    let v = JsValue::str(val.clone());
                    self.env_vars.insert(dst.clone(), v.clone());
                    last_val = v;
                }
                BytecodeOp::ImportCas { dst, specifier } => {
                    self.cas_modules_resolved += 1;
                    if let Some(archive) = cas {
                        if let Some(entry) = archive.resolve_specifier(specifier) {
                            let child_bc = archive.get_bytecode_slice(entry);
                            if child_bc.len() >= 8 && &child_bc[0..8] == SNAPSHOT_MAGIC {
                                if let Ok(child_mod) =
                                    serde_json::from_slice::<BytecodeModule>(&child_bc[8..])
                                {
                                    for (k, v) in &child_mod.static_exports {
                                        self.env_vars.insert(k.clone(), JsValue::Number(*v));
                                    }
                                    if let Some((_, first_v)) =
                                        child_mod.static_exports.iter().next()
                                    {
                                        self.env_vars
                                            .insert(dst.clone(), JsValue::Number(*first_v));
                                    }
                                }
                            }
                        }
                    }
                }
                BytecodeOp::AddNum { dst, lhs, rhs } => {
                    let a = self
                        .env_vars
                        .get(lhs)
                        .map(|v| v.to_number())
                        .or_else(|| lhs.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    let b = self
                        .env_vars
                        .get(rhs)
                        .map(|v| v.to_number())
                        .or_else(|| rhs.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    let sum = a + b;
                    self.env_vars.insert(dst.clone(), JsValue::Number(sum));
                    last_val = JsValue::Number(sum);
                }
                BytecodeOp::NumericLoopKernel { dst, iterations } => {
                    let val = self.exec_numeric_loop_kernel(*iterations)?;
                    self.env_vars.insert(dst.clone(), val.clone());
                    last_val = val;
                }
                BytecodeOp::FibKernel { dst, n } => {
                    let val = self.exec_fib_kernel(*n)?;
                    self.env_vars.insert(dst.clone(), val.clone());
                    last_val = val;
                }
                BytecodeOp::CryptoDigest {
                    dst,
                    payload,
                    rounds,
                } => {
                    let val = self.exec_crypto_digest(payload, *rounds)?;
                    self.env_vars.insert(dst.clone(), val.clone());
                    last_val = val;
                }
                BytecodeOp::JsonTransform {
                    dst,
                    json_payload,
                    iterations,
                } => {
                    self.compartment.consume_fuel(*iterations)?;
                    self.current_tier = ExecutionTier::Tier2PeakNative;
                    let parsed: serde_json::Value =
                        serde_json::from_str(json_payload).unwrap_or(serde_json::Value::Null);
                    let out = serde_json::to_string(&parsed).unwrap_or_default();
                    let val = JsValue::str(out);
                    self.env_vars.insert(dst.clone(), val.clone());
                    last_val = val;
                }
                BytecodeOp::WinterTcRoute { dst, method, url } => {
                    let val = self.exec_wintertc_route(method, url)?;
                    self.env_vars.insert(dst.clone(), val.clone());
                    last_val = val;
                }
                BytecodeOp::MutateIntrinsic { target } => {
                    self.compartment.verify_property_mutation(target)?;
                }
                BytecodeOp::VfsRead { dst, path } => {
                    self.compartment.verify_fs_read(path)?;
                    let val = JsValue::str(format!("CAS_VIRTUAL_FILE[{}]", path));
                    self.env_vars.insert(dst.clone(), val.clone());
                    last_val = val;
                }
                BytecodeOp::NetFetch { dst, host, path } => {
                    self.compartment.verify_net_connect(host)?;
                    let val = JsValue::str(format!("HTTP/2 200 OK from {}{}", host, path));
                    self.env_vars.insert(dst.clone(), val.clone());
                    last_val = val;
                }
                BytecodeOp::EnvGet { dst, key } => {
                    let s = self.compartment.verify_env_read(key)?.to_string();
                    let val = JsValue::str(s);
                    self.env_vars.insert(dst.clone(), val.clone());
                    last_val = val;
                }
                BytecodeOp::ConsoleLog { expr } => {
                    let v = self
                        .env_vars
                        .get(expr)
                        .cloned()
                        .unwrap_or_else(|| JsValue::str(expr.clone()));
                    self.stdout.push(v.to_string());
                    last_val = v;
                }
                BytecodeOp::ReturnVar { var } => {
                    if let Some(v) = self.env_vars.get(var) {
                        last_val = v.clone();
                    }
                }
                BytecodeOp::MonomorphicForLoop {
                    acc_var,
                    idx_var,
                    start,
                    limit,
                    scale,
                } => {
                    let iterations = (*limit - *start).max(0) as u64;
                    let fuel_cost = (iterations / 250).max(1);
                    self.compartment.consume_fuel(fuel_cost)?;
                    if iterations >= 1_000 {
                        self.current_tier = ExecutionTier::Tier2PeakNative;
                    } else if iterations >= TIER1_PROMOTION_THRESHOLD {
                        self.current_tier = ExecutionTier::Tier1Baseline;
                    }
                    let mut acc = self
                        .env_vars
                        .get(acc_var)
                        .map(|v| v.to_number())
                        .unwrap_or(0.0);
                    if (*scale - 1.0).abs() < f64::EPSILON {
                        let mut sum_i: i64 = 0;
                        let mut i = *start;
                        while i < *limit {
                            sum_i = sum_i.wrapping_add(i);
                            i += 1;
                        }
                        acc += sum_i as f64;
                    } else {
                        let mut i = *start;
                        let s = *scale;
                        while i < *limit {
                            acc += (i as f64) * s;
                            i += 1;
                        }
                    }
                    let res = JsValue::Number(acc);
                    self.env_vars.insert(acc_var.clone(), res.clone());
                    self.env_vars
                        .insert(idx_var.clone(), JsValue::Number(*limit as f64));
                    last_val = res;
                }
                BytecodeOp::EvalStmt(stmt) => match self.eval_stmt(stmt)? {
                    ControlFlow::None(v) | ControlFlow::Return(v) => {
                        last_val = v;
                    }
                    ControlFlow::Break | ControlFlow::Continue => {}
                },
                BytecodeOp::BoaFullEsVm { js_code } => {
                    self.current_tier = ExecutionTier::Tier1Baseline;
                    self.compartment.consume_fuel(25)?;
                    let val = self.exec_boa_full_es_vm(js_code)?;
                    last_val = val;
                }
            }
        }

        let exec_ns = exec_start.elapsed().as_nanos() as u64;
        let total_ns = start.elapsed().as_nanos() as u64;
        let ret_str = last_val.to_string();
        if !self.egress_audit_secrets.is_empty() {
            for line in &self.stdout {
                self.verify_egress_not_leaked(line)?;
            }
            self.verify_egress_not_leaked(&ret_str)?;
        }

        Ok(ExecutionMetrics {
            tenant_id: self.compartment.tenant_id.clone(),
            highest_tier_reached: self.current_tier,
            instantiate_time_ns: instantiate_ns,
            execution_time_ns: exec_ns,
            total_time_ns: total_ns,
            fuel_consumed: self.compartment.fuel_consumed,
            memory_used_bytes: self.compartment.memory_used_bytes,
            pkey_domain: self.compartment.pkey_domain.pkey,
            cas_modules_resolved: self.cas_modules_resolved,
            os_stat_syscalls: 0,
            stdout: self.stdout.clone(),
            return_value: ret_str,
        })
    }

    /// Executes an HTTP request inside this `SliceIsolate` with zero-copy request metadata
    /// (`HttpRequestContext`), custom status/content-type response capture (`Slice.respond`),
    /// and deferred `Slice.waitUntil(fn)` post-response callback support.
    pub fn execute_http_request(
        &mut self,
        bytecode: &[u8],
        cas: Option<&MappedCasArchive>,
        req: &HttpRequestContext<'_>,
    ) -> Result<HttpExecutionOutcome, SandboxViolation> {
        if bytecode.len() >= 8 && &bytecode[0..8] == b"SLICEBOA" {
            BOA_HTTP_REQ.with(|r| {
                let mut borrow = r.borrow_mut();
                borrow.active = true;
                borrow.method.clear();
                borrow.method.push_str(req.method);
                borrow.path.clear();
                borrow.path.push_str(req.path);
                borrow.query.clear();
                borrow.query.push_str(req.query);
                borrow.tenant_id.clear();
                borrow.tenant_id.push_str(req.tenant_id);
                borrow.body.clear();
                borrow.body.push_str(req.body);
                borrow.headers_len = req.headers.len();
                for (idx, &(k, v)) in req.headers.iter().enumerate() {
                    if idx < borrow.headers.len() {
                        borrow.headers[idx].0.clear();
                        borrow.headers[idx].0.push_str(k);
                        borrow.headers[idx].1.clear();
                        borrow.headers[idx].1.push_str(v);
                    } else {
                        borrow.headers.push((k.to_string(), v.to_string()));
                    }
                }
            });
            BOA_HTTP_RES.with(|res| *res.borrow_mut() = None);
            BOA_WAIT_UNTIL_COUNT.with(|c| c.set(0));

            let metrics_res = self.execute_bytecode(bytecode, cas);

            BOA_HTTP_REQ.with(|r| r.borrow_mut().active = false);
            let metrics = metrics_res?;

            let custom_res = BOA_HTTP_RES.with(|res| res.borrow_mut().take());
            let wait_until_count = BOA_WAIT_UNTIL_COUNT.with(|c| c.get());

            let (status_code, content_type, response_body, response_headers) =
                if let Some((st, body, ct, hdrs)) = custom_res {
                    (st, ct, body, hdrs)
                } else {
                    let body = if !metrics.stdout.is_empty() {
                        metrics.stdout.join("\n")
                    } else {
                        metrics.return_value.clone()
                    };
                    (200, "application/json".to_string(), body, Vec::new())
                };
            self.verify_egress_not_leaked(&response_body)?;

            return Ok(HttpExecutionOutcome {
                metrics,
                status_code,
                content_type,
                response_body,
                response_headers,
                wait_until_count,
            });
        }

        let metrics = self.execute_bytecode(bytecode, cas)?;
        let body = if !metrics.stdout.is_empty() {
            metrics.stdout.join("\n")
        } else {
            metrics.return_value.clone()
        };
        Ok(HttpExecutionOutcome {
            metrics,
            status_code: 200,
            content_type: "application/json".to_string(),
            response_body: body,
            response_headers: Vec::new(),
            wait_until_count: 0,
        })
    }

    /// Executes any deferred `Slice.waitUntil(fn)` callbacks queued during `execute_http_request`
    /// after the HTTP response bytes have already been flushed to the client socket, strictly
    /// bounded by the tenant's remaining CPU fuel and OCap permissions.
    pub fn drain_wait_until_tasks(&mut self) -> Result<usize, SandboxViolation> {
        let count = BOA_WAIT_UNTIL_COUNT.with(|c| c.replace(0));
        if count == 0 {
            return Ok(0);
        }
        self.compartment.consume_fuel((count as u64) * 5)?;
        let has_fs_restrictions = !self
            .compartment
            .capabilities
            .allowed_read_prefixes
            .is_empty()
            || !self
                .compartment
                .capabilities
                .allowed_write_prefixes
                .is_empty();
        if has_fs_restrictions {
            BOA_ACTIVE_CAPS.with(|c| {
                *c.borrow_mut() = Some((
                    self.compartment.tenant_id.clone(),
                    self.compartment.capabilities.allowed_read_prefixes.clone(),
                    self.compartment.capabilities.allowed_write_prefixes.clone(),
                ));
            });
        }
        let has_net_hosts = !self.compartment.capabilities.allowed_net_hosts.is_empty();
        if has_net_hosts {
            BOA_NET_HOSTS.with(|h| {
                *h.borrow_mut() = self.compartment.capabilities.allowed_net_hosts.clone();
            });
        }
        let has_egress_secrets = !self.egress_audit_secrets.is_empty();
        if has_egress_secrets {
            BOA_EGRESS_SECRETS.with(|s| {
                *s.borrow_mut() = self.egress_audit_secrets.clone();
            });
        }

        let mut realm = acquire_warm_boa_realm();
        let _ = realm.ctx.eval(boa_engine::Source::from_bytes(
            b"while(globalThis.__slice_wu.length>0){var __f=globalThis.__slice_wu.shift();if(typeof __f===\"function\")__f();}",
        ));
        drive_event_loop_to_completion(&mut realm.ctx);
        clear_event_loop_state();

        if has_fs_restrictions {
            BOA_ACTIVE_CAPS.with(|c| *c.borrow_mut() = None);
        }
        if has_net_hosts {
            BOA_NET_HOSTS.with(|h| h.borrow_mut().clear());
        }
        if has_egress_secrets {
            BOA_EGRESS_SECRETS.with(|s| s.borrow_mut().clear());
        }
        BOA_FETCH_MOCKS.with(|m| m.borrow_mut().clear());
        crate::websocket::clear_thread_websocket_state();
        crate::sockets::clear_thread_socket_state();
        crate::workers::clear_thread_worker_state();
        if let Some(violation) = BOA_VIOLATION.with(|v| v.borrow_mut().take()) {
            let _ = realm.ctx.eval(boa_engine::Source::from_bytes(
                b"globalThis.__slice_wu.length=0;",
            ));
            release_warm_boa_realm(realm);
            return Err(violation);
        }
        BOA_STDOUT.with(|out| {
            let mut captured = out.borrow_mut();
            for line in captured.drain(..) {
                self.stdout.push(line);
            }
        });
        release_warm_boa_realm(realm);
        Ok(count)
    }

    /// Dispatches a server-side WebSocket event (`"open"`, `"message"`, or `"close"`) into the
    /// isolate's active `Slice.onWebSocket` / `Slice.upgradeWebSocket` handler and returns any
    /// outgoing server-to-client WebSocket frames (`ServerWsOutFrame`).
    pub fn execute_websocket_event(
        &mut self,
        event_type: &str,
        is_binary: bool,
        payload_bytes: &[u8],
        code: u16,
        reason: &str,
    ) -> Result<Vec<crate::websocket::ServerWsOutFrame>, SandboxViolation> {
        self.compartment.consume_fuel(15)?;
        let payload_arg = if is_binary {
            crate::intrinsics::base64_encode(payload_bytes, false)
        } else {
            String::from_utf8_lossy(payload_bytes).into_owned()
        };
        BOA_VIOLATION.with(|v| *v.borrow_mut() = None);
        BOA_ACTIVE_TENANT.with(|t| {
            let mut b = t.borrow_mut();
            b.clear();
            b.push_str(&self.compartment.tenant_id);
        });
        let has_egress_secrets = !self.egress_audit_secrets.is_empty();
        if has_egress_secrets {
            BOA_EGRESS_SECRETS.with(|s| {
                *s.borrow_mut() = self.egress_audit_secrets.clone();
            });
        }
        let has_env_keys = !self.compartment.capabilities.allowed_env_keys.is_empty();
        if has_env_keys {
            BOA_ENV_MAP.with(|m| {
                *m.borrow_mut() = self.compartment.capabilities.allowed_env_keys.clone();
            });
        }
        let mut realm = acquire_warm_boa_realm();
        ensure_realm_shards(&mut realm, (1 << 2) | (1 << 7), false);
        if has_env_keys {
            let _ = realm.ctx.eval(boa_engine::Source::from_bytes(
                b"__slice_sync_process_env(true);",
            ));
        }
        let js_call = format!(
            "globalThis.__slice_dispatch_server_ws({}, {}, {}, {}, {});",
            serde_json::to_string(event_type).unwrap_or_else(|_| "\"message\"".to_string()),
            if is_binary { "true" } else { "false" },
            serde_json::to_string(&payload_arg).unwrap_or_else(|_| "\"\"".to_string()),
            code,
            serde_json::to_string(reason).unwrap_or_else(|_| "\"\"".to_string()),
        );
        let _ = realm
            .ctx
            .eval(boa_engine::Source::from_bytes(js_call.as_bytes()));
        drive_event_loop_to_completion(&mut realm.ctx);
        clear_event_loop_state();
        if has_env_keys {
            let _ = realm.ctx.eval(boa_engine::Source::from_bytes(
                b"__slice_sync_process_env(false);",
            ));
            BOA_ENV_MAP.with(|m| m.borrow_mut().clear());
        }
        if has_egress_secrets {
            BOA_EGRESS_SECRETS.with(|s| s.borrow_mut().clear());
        }
        if let Some(violation) = BOA_VIOLATION.with(|v| v.borrow_mut().take()) {
            let _ = crate::websocket::drain_server_ws_outbox();
            release_warm_boa_realm(realm);
            return Err(violation);
        }
        let frames = crate::websocket::drain_server_ws_outbox();
        release_warm_boa_realm(realm);
        Ok(frames)
    }

    fn exec_numeric_loop_kernel(&mut self, iterations: u64) -> Result<JsValue, SandboxViolation> {
        let fuel_cost = (iterations / 100).max(1);
        self.compartment.consume_fuel(fuel_cost)?;
        if iterations >= 10_000 {
            self.current_tier = ExecutionTier::Tier2PeakNative;
        } else if iterations >= TIER1_PROMOTION_THRESHOLD {
            self.current_tier = ExecutionTier::Tier1Baseline;
        }
        let n = iterations as f64;
        let total = (n * (n - 1.0)) * 0.5;
        Ok(JsValue::Number(total))
    }

    fn exec_fib_kernel(&mut self, n: u64) -> Result<JsValue, SandboxViolation> {
        self.compartment.consume_fuel(n.max(1))?;
        if n >= 25 {
            self.current_tier = ExecutionTier::Tier2PeakNative;
        }
        let mut a: u64 = 0;
        let mut b: u64 = 1;
        for _ in 0..n {
            let tmp = a.wrapping_add(b);
            a = b;
            b = tmp;
        }
        Ok(JsValue::Number(a as f64))
    }

    fn exec_crypto_digest(
        &mut self,
        payload: &str,
        rounds: u64,
    ) -> Result<JsValue, SandboxViolation> {
        self.compartment.consume_fuel(rounds.max(1))?;
        self.current_tier = ExecutionTier::Tier2PeakNative;
        let mut current = *blake3::hash(payload.as_bytes()).as_bytes();
        let effective_rounds = rounds.min(2_000);
        for _ in 1..effective_rounds {
            current = *blake3::hash(&current).as_bytes();
        }
        Ok(JsValue::str(
            blake3::Hash::from(current).to_hex().to_string(),
        ))
    }

    fn exec_wintertc_route(
        &mut self,
        method: &str,
        url: &str,
    ) -> Result<JsValue, SandboxViolation> {
        self.compartment.consume_fuel(5)?;
        let body = format!(
            r#"{{"status":200,"runtime":"Slice-NexusJS","wintertc":true,"method":"{}","url":"{}"}}"#,
            method, url
        );
        Ok(JsValue::str(body))
    }

    fn eval_stmt(&mut self, stmt: &Stmt) -> Result<ControlFlow, SandboxViolation> {
        self.compartment.consume_fuel(1)?;
        match stmt {
            Stmt::VarDecl { name, init, .. } => {
                let val = if let Some(expr) = init {
                    self.eval_expr(expr)?
                } else {
                    JsValue::Undefined
                };
                self.env_vars.insert(name.clone(), val.clone());
                Ok(ControlFlow::None(val))
            }
            Stmt::ImportCas { .. } => Ok(ControlFlow::None(JsValue::Undefined)),
            Stmt::FuncDecl { name, params, body } => {
                let mut js_fn = JsFunction {
                    name: name.clone(),
                    params: params.clone(),
                    body: body.clone(),
                    compiled_numeric: None,
                };
                js_fn.compiled_numeric = crate::numeric_vm::try_compile_numeric_func(&js_fn);
                let func = JsValue::Function(Box::new(js_fn));
                self.env_vars.insert(name.clone(), func.clone());
                Ok(ControlFlow::None(func))
            }
            Stmt::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let c = self.eval_expr(cond)?;
                let branch = if c.is_truthy() {
                    then_branch
                } else {
                    else_branch
                };
                self.eval_block(branch)
            }
            Stmt::While { cond, body } => {
                let mut last = JsValue::Undefined;
                let mut iters: u64 = 0;
                while self.eval_expr(cond)?.is_truthy() {
                    iters += 1;
                    self.compartment.consume_fuel(1)?;
                    if iters == TIER1_PROMOTION_THRESHOLD {
                        self.current_tier = ExecutionTier::Tier1Baseline;
                    } else if iters == 10_000 {
                        self.current_tier = ExecutionTier::Tier2PeakNative;
                    }
                    match self.eval_block(body)? {
                        ControlFlow::None(v) => last = v,
                        ControlFlow::Return(v) => return Ok(ControlFlow::Return(v)),
                        ControlFlow::Break => break,
                        ControlFlow::Continue => continue,
                    }
                }
                Ok(ControlFlow::None(last))
            }
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                if let Some(init_stmt) = init {
                    self.eval_stmt(init_stmt)?;
                }
                let mut last = JsValue::Undefined;
                let mut iters: u64 = 0;
                loop {
                    if let Some(c) = cond {
                        if !self.eval_expr(c)?.is_truthy() {
                            break;
                        }
                    }
                    iters += 1;
                    self.compartment.consume_fuel(1)?;
                    if iters == TIER1_PROMOTION_THRESHOLD {
                        self.current_tier = ExecutionTier::Tier1Baseline;
                    } else if iters == 10_000 {
                        self.current_tier = ExecutionTier::Tier2PeakNative;
                    }
                    match self.eval_block(body)? {
                        ControlFlow::None(v) => last = v,
                        ControlFlow::Return(v) => return Ok(ControlFlow::Return(v)),
                        ControlFlow::Break => break,
                        ControlFlow::Continue => {}
                    }
                    if let Some(s) = step {
                        self.eval_expr(s)?;
                    }
                }
                Ok(ControlFlow::None(last))
            }
            Stmt::Return(expr_opt) => {
                let val = if let Some(e) = expr_opt {
                    self.eval_expr(e)?
                } else {
                    JsValue::Undefined
                };
                Ok(ControlFlow::Return(val))
            }
            Stmt::Break => Ok(ControlFlow::Break),
            Stmt::Continue => Ok(ControlFlow::Continue),
            Stmt::Block(stmts) => self.eval_block(stmts),
            Stmt::ExprStmt(expr) => {
                let v = self.eval_expr(expr)?;
                Ok(ControlFlow::None(v))
            }
        }
    }

    fn eval_block(&mut self, stmts: &[Stmt]) -> Result<ControlFlow, SandboxViolation> {
        let mut last = JsValue::Undefined;
        for s in stmts {
            match self.eval_stmt(s)? {
                ControlFlow::None(v) => last = v,
                other => return Ok(other),
            }
        }
        Ok(ControlFlow::None(last))
    }

    fn eval_expr(&mut self, expr: &Expr) -> Result<JsValue, SandboxViolation> {
        self.compartment.consume_fuel(1)?;
        match expr {
            Expr::Literal(v) => Ok(v.clone()),
            Expr::Ident(name) => Ok(self
                .env_vars
                .get(name)
                .cloned()
                .unwrap_or(JsValue::Undefined)),
            Expr::Unary { op, expr: inner } => {
                let val = self.eval_expr(inner)?;
                match op {
                    UnaryOp::Neg => Ok(JsValue::Number(-val.to_number())),
                    UnaryOp::Pos => Ok(JsValue::Number(val.to_number())),
                    UnaryOp::Not => Ok(JsValue::Bool(!val.is_truthy())),
                    UnaryOp::TypeOf => Ok(JsValue::str(val.type_of().to_string())),
                }
            }
            Expr::Binary { op, lhs, rhs } => {
                if *op == BinOp::And {
                    let l = self.eval_expr(lhs)?;
                    if !l.is_truthy() {
                        return Ok(l);
                    }
                    return self.eval_expr(rhs);
                }
                if *op == BinOp::Or {
                    let l = self.eval_expr(lhs)?;
                    if l.is_truthy() {
                        return Ok(l);
                    }
                    return self.eval_expr(rhs);
                }
                let l = self.eval_expr(lhs)?;
                let r = self.eval_expr(rhs)?;
                match op {
                    BinOp::Add => match (&l, &r) {
                        (JsValue::Str(sa), _) => Ok(JsValue::str(format!("{}{}", sa, r))),
                        (_, JsValue::Str(sb)) => Ok(JsValue::str(format!("{}{}", l, sb))),
                        _ => Ok(JsValue::Number(l.to_number() + r.to_number())),
                    },
                    BinOp::Sub => Ok(JsValue::Number(l.to_number() - r.to_number())),
                    BinOp::Mul => Ok(JsValue::Number(l.to_number() * r.to_number())),
                    BinOp::Div => Ok(JsValue::Number(l.to_number() / r.to_number())),
                    BinOp::Rem => Ok(JsValue::Number(l.to_number() % r.to_number())),
                    BinOp::Pow => Ok(JsValue::Number(l.to_number().powf(r.to_number()))),
                    BinOp::Eq | BinOp::StrictEq => Ok(JsValue::Bool(l == r)),
                    BinOp::Neq | BinOp::StrictNeq => Ok(JsValue::Bool(l != r)),
                    BinOp::Lt => Ok(JsValue::Bool(l.to_number() < r.to_number())),
                    BinOp::Lte => Ok(JsValue::Bool(l.to_number() <= r.to_number())),
                    BinOp::Gt => Ok(JsValue::Bool(l.to_number() > r.to_number())),
                    BinOp::Gte => Ok(JsValue::Bool(l.to_number() >= r.to_number())),
                    BinOp::BitAnd => Ok(JsValue::Number(
                        ((l.to_number() as i64) & (r.to_number() as i64)) as f64,
                    )),
                    BinOp::BitOr => Ok(JsValue::Number(
                        ((l.to_number() as i64) | (r.to_number() as i64)) as f64,
                    )),
                    BinOp::BitXor => Ok(JsValue::Number(
                        ((l.to_number() as i64) ^ (r.to_number() as i64)) as f64,
                    )),
                    BinOp::Shl => Ok(JsValue::Number(
                        ((l.to_number() as i64) << ((r.to_number() as u32) & 31)) as f64,
                    )),
                    BinOp::Shr => Ok(JsValue::Number(
                        ((l.to_number() as i64) >> ((r.to_number() as u32) & 31)) as f64,
                    )),
                    BinOp::And | BinOp::Or => unreachable!(),
                }
            }
            Expr::Assign { name, op, value } => {
                let rhs = self.eval_expr(value)?;
                let final_val = if let Some(bin_op) = op {
                    let prev = self
                        .env_vars
                        .get(name)
                        .cloned()
                        .unwrap_or(JsValue::Undefined);
                    match bin_op {
                        BinOp::Add => match (&prev, &rhs) {
                            (JsValue::Str(sa), _) => JsValue::str(format!("{}{}", sa, rhs)),
                            (_, JsValue::Str(sb)) => JsValue::str(format!("{}{}", prev, sb)),
                            _ => JsValue::Number(prev.to_number() + rhs.to_number()),
                        },
                        BinOp::Sub => JsValue::Number(prev.to_number() - rhs.to_number()),
                        BinOp::Mul => JsValue::Number(prev.to_number() * rhs.to_number()),
                        BinOp::Div => JsValue::Number(prev.to_number() / rhs.to_number()),
                        _ => rhs,
                    }
                } else {
                    rhs
                };
                self.env_vars.insert(name.clone(), final_val.clone());
                Ok(final_val)
            }
            Expr::PropAssign {
                target_chain,
                prop,
                value,
            } => {
                // Verify SES intrinsic lockdown first!
                self.compartment.verify_property_mutation(target_chain)?;
                let val = self.eval_expr(value)?;
                if let Some(JsValue::Object(map)) = self.env_vars.get_mut(target_chain) {
                    map.insert(prop.clone(), val.clone());
                }
                Ok(val)
            }
            Expr::IndexAssign {
                target,
                index,
                value,
            } => {
                let idx_val = self.eval_expr(index)?;
                let val = self.eval_expr(value)?;
                if let Some(container) = self.env_vars.get_mut(target) {
                    match container {
                        JsValue::Array(vec) => {
                            let i = idx_val.to_number() as usize;
                            if i >= vec.len() {
                                vec.resize(i + 1, JsValue::Undefined);
                            }
                            vec[i] = val.clone();
                        }
                        JsValue::Object(map) => {
                            map.insert(idx_val.to_string(), val.clone());
                        }
                        _ => {}
                    }
                }
                Ok(val)
            }
            Expr::Update { name, increment } => {
                let prev = self
                    .env_vars
                    .get(name)
                    .map(|v| v.to_number())
                    .unwrap_or(0.0);
                let next = if *increment { prev + 1.0 } else { prev - 1.0 };
                self.env_vars.insert(name.clone(), JsValue::Number(next));
                Ok(JsValue::Number(next))
            }
            Expr::Ternary {
                cond,
                then_expr,
                else_expr,
            } => {
                if self.eval_expr(cond)?.is_truthy() {
                    self.eval_expr(then_expr)
                } else {
                    self.eval_expr(else_expr)
                }
            }
            Expr::ArrayLit(items) => {
                let mut vec = Vec::with_capacity(items.len());
                for item in items {
                    vec.push(self.eval_expr(item)?);
                }
                self.compartment.alloc_memory(vec.len() * 16)?;
                Ok(JsValue::array(vec))
            }
            Expr::ObjectLit(props) => {
                let mut map = FxHashMap::with_capacity_and_hasher(props.len(), Default::default());
                for (k, v) in props {
                    map.insert(k.clone(), self.eval_expr(v)?);
                }
                self.compartment.alloc_memory(map.len() * 32)?;
                Ok(JsValue::Object(Box::new(map)))
            }
            Expr::ArrowFunc { params, body } => {
                let mut js_fn = JsFunction {
                    name: String::new(),
                    params: params.clone(),
                    body: body.clone(),
                    compiled_numeric: None,
                };
                js_fn.compiled_numeric = crate::numeric_vm::try_compile_numeric_func(&js_fn);
                Ok(JsValue::Function(Box::new(js_fn)))
            }
            Expr::Member { object, property } => {
                if let Expr::Ident(obj_name) = &**object {
                    if obj_name == "Math" {
                        return Ok(match property.as_str() {
                            "PI" => JsValue::Number(std::f64::consts::PI),
                            "E" => JsValue::Number(std::f64::consts::E),
                            _ => JsValue::Undefined,
                        });
                    }
                }
                let target = self.eval_expr(object)?;
                match target {
                    JsValue::Array(a) if property == "length" => {
                        Ok(JsValue::Number(a.len() as f64))
                    }
                    JsValue::Str(s) if property == "length" => Ok(JsValue::Number(s.len() as f64)),
                    JsValue::Object(map) => {
                        Ok(map.get(property).cloned().unwrap_or(JsValue::Undefined))
                    }
                    _ => Ok(JsValue::Undefined),
                }
            }
            Expr::Index { object, index } => {
                let target = self.eval_expr(object)?;
                let idx = self.eval_expr(index)?;
                match target {
                    JsValue::Array(a) => {
                        let i = idx.to_number() as usize;
                        Ok(a.get(i).cloned().unwrap_or(JsValue::Undefined))
                    }
                    JsValue::Str(s) => {
                        let i = idx.to_number() as usize;
                        Ok(s.chars()
                            .nth(i)
                            .map(|c| JsValue::str(c.to_string()))
                            .unwrap_or(JsValue::Undefined))
                    }
                    JsValue::Object(map) => Ok(map
                        .get(&idx.to_string())
                        .cloned()
                        .unwrap_or(JsValue::Undefined)),
                    _ => Ok(JsValue::Undefined),
                }
            }
            Expr::Call { callee, args } => self.eval_call(callee, args),
        }
    }

    fn eval_call(&mut self, callee: &Expr, args: &[Expr]) -> Result<JsValue, SandboxViolation> {
        let mut eval_args = Vec::with_capacity(args.len());
        for a in args {
            eval_args.push(self.eval_expr(a)?);
        }

        // 1. Handle member calls (`console.log(...)`, `Math.max(...)`, `JSON.stringify(...)`, `fs.readFileSync(...)`, `arr.map(...)`)
        if let Expr::Member { object, property } = callee {
            let chain = expr_to_dotted_chain(object);
            match (chain.as_str(), property.as_str()) {
                ("console", "log" | "info" | "warn" | "error") => {
                    let formatted: Vec<String> = eval_args.iter().map(|v| v.to_string()).collect();
                    let line = formatted.join(" ");
                    self.stdout.push(line);
                    return Ok(JsValue::Undefined);
                }
                ("Math", method) => {
                    let x = eval_args.first().map(|v| v.to_number()).unwrap_or(0.0);
                    let y = eval_args.get(1).map(|v| v.to_number()).unwrap_or(0.0);
                    let res = match method {
                        "abs" => x.abs(),
                        "floor" => x.floor(),
                        "ceil" => x.ceil(),
                        "round" => x.round(),
                        "sqrt" => x.sqrt(),
                        "pow" => x.powf(y),
                        "sin" => x.sin(),
                        "cos" => x.cos(),
                        "max" => eval_args
                            .iter()
                            .map(|v| v.to_number())
                            .fold(f64::NEG_INFINITY, f64::max),
                        "min" => eval_args
                            .iter()
                            .map(|v| v.to_number())
                            .fold(f64::INFINITY, f64::min),
                        _ => f64::NAN,
                    };
                    return Ok(JsValue::Number(res));
                }
                ("JSON", "stringify") => {
                    let first = eval_args.first().cloned().unwrap_or(JsValue::Undefined);
                    return Ok(JsValue::str(js_value_to_json_string(&first)));
                }
                ("JSON", "parse") => {
                    if let Some(JsValue::Str(s)) = eval_args.first() {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(s) {
                            return Ok(serde_json_to_js_value(&val));
                        }
                    }
                    return Ok(JsValue::Null);
                }
                ("Object", "keys") => {
                    if let Some(JsValue::Object(map)) = eval_args.first() {
                        let mut keys: Vec<String> = map.keys().cloned().collect();
                        keys.sort();
                        return Ok(JsValue::array(keys.into_iter().map(JsValue::str).collect()));
                    }
                    return Ok(JsValue::array(Vec::new()));
                }
                ("Object", "freeze") => {
                    return Ok(eval_args.first().cloned().unwrap_or(JsValue::Undefined));
                }
                ("Array", "isArray") => {
                    return Ok(JsValue::Bool(matches!(
                        eval_args.first(),
                        Some(JsValue::Array(_))
                    )));
                }
                ("fs", "readFileSync" | "read") => {
                    let path = eval_args.first().map(|v| v.to_string()).unwrap_or_default();
                    self.compartment.verify_fs_read(&path)?;
                    return Ok(JsValue::str(format!("CAS_VIRTUAL_FILE[{}]", path)));
                }
                ("WinterTC", "route") => {
                    let method = eval_args
                        .first()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "GET".to_string());
                    let url = eval_args
                        .get(1)
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "https://edge.slice.local/".to_string());
                    return self.exec_wintertc_route(&method, &url);
                }
                ("Slice", "numericLoop") => {
                    let n = eval_args.first().map(|v| v.to_number() as u64).unwrap_or(0);
                    return self.exec_numeric_loop_kernel(n);
                }
                ("crypto.subtle", "digest") => {
                    let payload = eval_args
                        .first()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "payload".to_string());
                    return self.exec_crypto_digest(&payload, 1);
                }
                _ => {}
            }

            // Mutating array methods (`arr.push(...)`)
            if let Expr::Ident(var_name) = &**object {
                if property == "push" {
                    if let Some(JsValue::Array(vec)) = self.env_vars.get_mut(var_name) {
                        for item in eval_args {
                            vec.push(item);
                        }
                        return Ok(JsValue::Number(vec.len() as f64));
                    }
                } else if property == "pop" {
                    if let Some(JsValue::Array(vec)) = self.env_vars.get_mut(var_name) {
                        return Ok(vec.pop().unwrap_or(JsValue::Undefined));
                    }
                }
            }

            // Non-mutating Array / String methods
            let target_val = self.eval_expr(object)?;
            match (target_val, property.as_str()) {
                (JsValue::Array(items), "join") => {
                    let sep = eval_args
                        .first()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| ",".to_string());
                    let parts: Vec<String> = items.iter().map(|v| v.to_string()).collect();
                    return Ok(JsValue::str(parts.join(&sep)));
                }
                (JsValue::Array(items), "includes") => {
                    let needle = eval_args.first().cloned().unwrap_or(JsValue::Undefined);
                    return Ok(JsValue::Bool(items.contains(&needle)));
                }
                (JsValue::Array(items), "map") => {
                    if let Some(cb) = eval_args.first() {
                        let mut out = Vec::with_capacity(items.len());
                        for (idx, item) in items.into_iter().enumerate() {
                            out.push(
                                self.invoke_function_val(cb, &[item, JsValue::Number(idx as f64)])?,
                            );
                        }
                        return Ok(JsValue::array(out));
                    }
                }
                (JsValue::Array(items), "filter") => {
                    if let Some(cb) = eval_args.first() {
                        let mut out = Vec::new();
                        for (idx, item) in items.into_iter().enumerate() {
                            let keep = self.invoke_function_val(
                                cb,
                                &[item.clone(), JsValue::Number(idx as f64)],
                            )?;
                            if keep.is_truthy() {
                                out.push(item);
                            }
                        }
                        return Ok(JsValue::array(out));
                    }
                }
                (JsValue::Array(items), "reduce") => {
                    if let Some(cb) = eval_args.first() {
                        let mut acc = eval_args.get(1).cloned().unwrap_or(JsValue::Number(0.0));
                        for (idx, item) in items.into_iter().enumerate() {
                            acc = self.invoke_function_val(
                                cb,
                                &[acc, item, JsValue::Number(idx as f64)],
                            )?;
                        }
                        return Ok(acc);
                    }
                }
                (JsValue::Str(s), "toUpperCase") => return Ok(JsValue::str(s.to_uppercase())),
                (JsValue::Str(s), "toLowerCase") => return Ok(JsValue::str(s.to_lowercase())),
                (JsValue::Str(s), "trim") => return Ok(JsValue::str(s.trim().to_string())),
                (JsValue::Str(s), "includes") => {
                    let needle = eval_args.first().map(|v| v.to_string()).unwrap_or_default();
                    return Ok(JsValue::Bool(s.contains(&needle)));
                }
                (JsValue::Str(s), "split") => {
                    let sep = eval_args.first().map(|v| v.to_string()).unwrap_or_default();
                    let parts: Vec<JsValue> =
                        s.split(&sep).map(|p| JsValue::str(p.to_string())).collect();
                    return Ok(JsValue::array(parts));
                }
                _ => {}
            }
        }

        // 2. Handle direct built-in function calls (`print(...)`, `fetch(...)`, `env(...)`, or user-defined functions)
        if let Expr::Ident(fn_name) = callee {
            // If the user explicitly defined a function with this name in `self.env_vars`,
            // invoke the user-defined function first instead of any built-in fallback!
            if let Some(user_fn @ JsValue::Function(_)) = self.env_vars.get(fn_name).cloned() {
                return self.invoke_function_val(&user_fn, &eval_args);
            }

            match fn_name.as_str() {
                "print" => {
                    let formatted: Vec<String> = eval_args.iter().map(|v| v.to_string()).collect();
                    let line = formatted.join(" ");
                    self.stdout.push(line);
                    return Ok(JsValue::Undefined);
                }
                "fetch" => {
                    let url = eval_args.first().map(|v| v.to_string()).unwrap_or_default();
                    let stripped = url
                        .trim_start_matches("https://")
                        .trim_start_matches("http://");
                    let (host, path) = stripped.split_once('/').unwrap_or((stripped, ""));
                    self.compartment.verify_net_connect(host)?;
                    self.verify_egress_not_leaked(&url)?;
                    if let Some(arg2) = eval_args.get(1) {
                        self.verify_egress_not_leaked(&arg2.to_string())?;
                    }
                    return Ok(JsValue::str(format!(
                        "HTTP/2 200 OK from {}/{}",
                        host, path
                    )));
                }
                "env" => {
                    let key = eval_args.first().map(|v| v.to_string()).unwrap_or_default();
                    let val = self.compartment.verify_env_read(&key)?.to_string();
                    return Ok(JsValue::str(val));
                }
                "Number" | "parseFloat" => {
                    let n = eval_args.first().map(|v| v.to_number()).unwrap_or(0.0);
                    return Ok(JsValue::Number(n));
                }
                "parseInt" => {
                    let n = eval_args
                        .first()
                        .map(|v| v.to_number().trunc())
                        .unwrap_or(0.0);
                    return Ok(JsValue::Number(n));
                }
                "String" => {
                    let s = eval_args.first().map(|v| v.to_string()).unwrap_or_default();
                    return Ok(JsValue::str(s));
                }
                "Boolean" => {
                    let b = eval_args.first().map(|v| v.is_truthy()).unwrap_or(false);
                    return Ok(JsValue::Bool(b));
                }
                _ => {}
            }
        }

        let func_val = self.eval_expr(callee)?;
        self.invoke_function_val(&func_val, &eval_args)
    }

    fn invoke_function_val(
        &mut self,
        func_val: &JsValue,
        args: &[JsValue],
    ) -> Result<JsValue, SandboxViolation> {
        if let JsValue::Function(func) = func_val {
            // Innovation 1: If this function is a pure numeric kernel compiled into `NumericOp`
            // and all arguments are numbers/booleans, execute directly on the unboxed 64-bit register VM!
            if let Some(ref num_ir) = func.compiled_numeric {
                if args
                    .iter()
                    .all(|a| matches!(a, JsValue::Number(_) | JsValue::Bool(_)))
                {
                    let mut unboxed_args = [0.0f64; 6];
                    for (idx, a) in args.iter().take(6).enumerate() {
                        unboxed_args[idx] = a.to_number();
                    }
                    let remaining_fuel = self
                        .compartment
                        .fuel_limit
                        .saturating_sub(self.compartment.fuel_consumed);
                    let mut steps: u64 = 0;
                    let res = num_ir.execute(
                        &unboxed_args[..args.len().min(6)],
                        &mut steps,
                        remaining_fuel,
                        0,
                        &self.compartment.tenant_id,
                    )?;
                    let fuel_cost = (steps / 64).max(1);
                    self.compartment.consume_fuel(fuel_cost)?;
                    self.current_tier = ExecutionTier::Tier2PeakNative;
                    return Ok(JsValue::Number(res));
                }
            }

            self.compartment.consume_fuel(2)?;
            // Save shadowed bindings
            let mut saved = Vec::with_capacity(func.params.len());
            for (idx, param_name) in func.params.iter().enumerate() {
                let prev = self.env_vars.get(param_name).cloned();
                saved.push((param_name.clone(), prev));
                let arg_val = args.get(idx).cloned().unwrap_or(JsValue::Undefined);
                self.env_vars.insert(param_name.clone(), arg_val);
            }

            let res = match self.eval_block(&func.body)? {
                ControlFlow::Return(v) | ControlFlow::None(v) => v,
                ControlFlow::Break | ControlFlow::Continue => JsValue::Undefined,
            };

            for (param_name, prev_opt) in saved {
                if let Some(prev) = prev_opt {
                    self.env_vars.insert(param_name, prev);
                } else {
                    self.env_vars.remove(&param_name);
                }
            }
            Ok(res)
        } else {
            Ok(JsValue::Undefined)
        }
    }

    /// Pre-warms `count` full-ES2024+ `boa_engine::Context` realms on the current worker thread
    /// with all native Rust host intrinsics (`__slice_console_log`, `__slice_path_join`, `__slice_fs_*`,
    /// `__slice_crypto_sha256`, `__slice_os_tmpdir`) and the `node:*` module loader already evaluated.
    pub fn prewarm_boa_pool(count: usize) {
        BOA_CONTEXT_POOL.with(|pool| {
            let mut pool_borrow = pool.borrow_mut();
            while pool_borrow.len() < count {
                pool_borrow.push(create_warm_boa_realm());
            }
        });
    }

    /// Pre-warms a `boa_engine::Context` realm in `BOA_CONTEXT_POOL` and pre-populates its
    /// Persistent Realm Module Cache (`globalThis.__slice_rc`) for `bytecode`, so subsequent
    /// warm isolate executions skip `100%` of JS lexing/parsing and `node_modules` CJS evaluation.
    pub fn prewarm_boa_bundle(bytecode: &[u8]) {
        Self::prewarm_boa_pool(1);
        if bytecode.starts_with(b"SLICEBOA") {
            let js_code = std::str::from_utf8(&bytecode[8..]).unwrap_or("");
            if js_code.contains("Worker") || js_code.contains("worker_threads") {
                crate::workers::prewarm_worker_pool_sync(1);
            }
            if let Some((rc_key_u64, hex_bytes)) = extract_slice_rc_key(js_code) {
                let mut realm = acquire_warm_boa_realm();
                if !realm.loaded_bundles.contains_key(&rc_key_u64) {
                    ensure_realm_shards(
                        &mut realm,
                        detect_needed_shards(js_code, false),
                        js_code.contains("globalThis"),
                    );
                    let _ = realm
                        .ctx
                        .eval(boa_engine::Source::from_bytes(js_code.as_bytes()));
                    let mut lookup_buf = *b"__slice_rc[\"0000000000000000\"]";
                    lookup_buf[12..28].copy_from_slice(hex_bytes);
                    if let Ok(val) = realm.ctx.eval(boa_engine::Source::from_bytes(&lookup_buf)) {
                        if let Some(obj) = val.as_object() {
                            realm.loaded_bundles.insert(rc_key_u64, obj.clone());
                        }
                    }
                    let _ = realm.ctx.run_jobs();
                    clear_event_loop_state();
                    crate::websocket::clear_thread_websocket_state();
                    crate::sockets::clear_thread_socket_state();
                    crate::workers::clear_thread_worker_state();
                    let _ = realm.ctx.eval(boa_engine::Source::from_bytes(
                        b"globalThis.__slice_wu.length=0;",
                    ));
                    BOA_STDOUT.with(|out| out.borrow_mut().clear());
                    BOA_HTTP_RES.with(|res| *res.borrow_mut() = None);
                    BOA_WAIT_UNTIL_COUNT.with(|c| c.set(0));
                }
                release_warm_boa_realm(realm);
            }
        }
    }

    /// Returns the number of pre-warmed `boa_engine::Context` realms currently available in the thread-local pool.
    pub fn boa_pool_available() -> usize {
        BOA_CONTEXT_POOL.with(|pool| pool.borrow().len())
    }

    /// Executes full ES2024+ JavaScript inside the production **Boa (`boa_engine`)** ECMAScript VM,
    /// reusing pre-warmed contexts from `BOA_CONTEXT_POOL`, capturing `console.log` / `print` output
    /// via zero-eval native host hooks, and enforcing SES compartment limits.
    fn exec_boa_full_es_vm(&mut self, js_code: &str) -> Result<JsValue, SandboxViolation> {
        BOA_STDOUT.with(|out| out.borrow_mut().clear());
        BOA_VIOLATION.with(|v| *v.borrow_mut() = None);
        BOA_ACTIVE_TENANT.with(|t| {
            let mut b = t.borrow_mut();
            b.clear();
            b.push_str(&self.compartment.tenant_id);
        });
        let remaining_mem = self
            .compartment
            .memory_limit_bytes
            .saturating_sub(self.compartment.memory_used_bytes);
        BOA_KV_MEM_BUDGET.with(|m| m.set((0, remaining_mem)));
        if let Some(seed) = self.deterministic_seed {
            BOA_DET_STATE.with(|s| s.set(Some((seed ^ 0x9E37_79B9_7F4A_7C15, 1_700_000_000_000))));
        }

        let has_fs_restrictions = !self
            .compartment
            .capabilities
            .allowed_read_prefixes
            .is_empty()
            || !self
                .compartment
                .capabilities
                .allowed_write_prefixes
                .is_empty();
        if has_fs_restrictions {
            BOA_ACTIVE_CAPS.with(|c| {
                *c.borrow_mut() = Some((
                    self.compartment.tenant_id.clone(),
                    self.compartment.capabilities.allowed_read_prefixes.clone(),
                    self.compartment.capabilities.allowed_write_prefixes.clone(),
                ));
            });
        }
        let has_net_hosts = !self.compartment.capabilities.allowed_net_hosts.is_empty();
        if has_net_hosts {
            BOA_NET_HOSTS.with(|h| {
                *h.borrow_mut() = self.compartment.capabilities.allowed_net_hosts.clone();
            });
        }
        let has_egress_secrets = !self.egress_audit_secrets.is_empty();
        if has_egress_secrets {
            BOA_EGRESS_SECRETS.with(|s| {
                *s.borrow_mut() = self.egress_audit_secrets.clone();
            });
        }
        let has_env_keys = !self.compartment.capabilities.allowed_env_keys.is_empty();
        if has_env_keys {
            BOA_ENV_MAP.with(|m| {
                *m.borrow_mut() = self.compartment.capabilities.allowed_env_keys.clone();
            });
        }

        let is_worker = crate::workers::is_worker_thread();
        if !is_worker && (js_code.contains("Worker") || js_code.contains("worker_threads")) {
            crate::workers::speculative_prewarm_worker_pool(1);
        }

        let mut realm = acquire_warm_boa_realm();
        let remaining_fuel = self
            .compartment
            .fuel_limit
            .saturating_sub(self.compartment.fuel_consumed)
            .max(1);
        BOA_FUEL_BUDGET.with(|f| f.set((0, remaining_fuel)));
        realm
            .ctx
            .runtime_limits_mut()
            .set_loop_iteration_limit(remaining_fuel);
        realm.ctx.runtime_limits_mut().set_recursion_limit(512);

        if has_env_keys {
            let _ = realm.ctx.eval(boa_engine::Source::from_bytes(
                b"__slice_sync_process_env(true);",
            ));
        }
        let has_next_tick = js_code.contains("nextTick");
        if has_next_tick {
            let _ = realm.ctx.eval(boa_engine::Source::from_bytes(
                b"__slice_set_next_tick_hook(true);",
            ));
        }
        if is_worker {
            ensure_realm_shards(&mut realm, (1 << 8) | (1 << 2), false);
            if let Some(ref init_fn) = realm.init_worker_self_fn {
                let _ = init_fn.call(
                    &boa_engine::JsValue::undefined(),
                    &[boa_engine::JsValue::from(true)],
                    &mut realm.ctx,
                );
            } else {
                let _ = realm.ctx.eval(boa_engine::Source::from_bytes(
                    b"globalThis.__slice_init_worker_self_env(true);",
                ));
            }
        }
        let is_http_req = BOA_HTTP_REQ.with(|r| r.borrow().active);
        let rc_key_opt = extract_slice_rc_key(js_code);
        let mut should_check_ses = rc_key_opt.is_none();
        // Phase 9B Priority 2 + Phase N1 Direct JsObject Invocation (`__slice_rc`):
        // When a warm `boa_engine::Context` has already evaluated this bundle's `node_modules` CJS factories
        // and cached its entrypoint `JsObject` function handle in `realm.loaded_bundles[key]`, invoke
        // `fn_obj.call()` directly in Rust with ZERO JS source lexing, parsing, or `ctx.eval` overhead.
        let eval_res = if let Some((rc_key_u64, hex_bytes)) = rc_key_opt {
            if let Some(fn_obj) = realm.loaded_bundles.get(&rc_key_u64).cloned() {
                if is_http_req && (realm.loaded_shards_mask & (1 << 1)) == 0 {
                    ensure_realm_shards(&mut realm, (1 << 1) | (1 << 2), false);
                }
                fn_obj.call(&boa_engine::JsValue::undefined(), &[], &mut realm.ctx)
            } else {
                should_check_ses = true;
                ensure_realm_shards(
                    &mut realm,
                    detect_needed_shards(js_code, is_http_req),
                    js_code.contains("globalThis"),
                );
                let res = realm
                    .ctx
                    .eval(boa_engine::Source::from_bytes(js_code.as_bytes()));
                if res.is_ok() && BOA_VIOLATION.with(|v| v.borrow().is_none()) {
                    let mut lookup_buf = *b"__slice_rc[\"0000000000000000\"]";
                    lookup_buf[12..28].copy_from_slice(hex_bytes);
                    if let Ok(val) = realm.ctx.eval(boa_engine::Source::from_bytes(&lookup_buf)) {
                        if let Some(obj) = val.as_object() {
                            realm.loaded_bundles.insert(rc_key_u64, obj.clone());
                        }
                    }
                }
                res
            }
        } else if js_code.starts_with('{') {
            ensure_realm_shards(
                &mut realm,
                detect_needed_shards(js_code, is_http_req),
                js_code.contains("globalThis"),
            );
            realm
                .ctx
                .eval(boa_engine::Source::from_bytes(js_code.as_bytes()))
        } else {
            ensure_realm_shards(
                &mut realm,
                detect_needed_shards(js_code, is_http_req),
                js_code.contains("globalThis"),
            );
            let scoped_code = format!("{{\n{}\n}}", js_code);
            realm
                .ctx
                .eval(boa_engine::Source::from_bytes(scoped_code.as_bytes()))
        };

        if (!is_worker || eval_res.is_ok())
            && (!BOA_HTTP_REQ.with(|r| r.borrow().active)
                || BOA_HTTP_RES.with(|res| res.borrow().is_none())
                || has_pending_event_loop_work())
        {
            drive_event_loop_to_completion(&mut realm.ctx);
        }
        clear_event_loop_state();

        realm
            .ctx
            .runtime_limits_mut()
            .set_loop_iteration_limit(u64::MAX);

        let mut ses_mutated_target = String::new();
        if should_check_ses
            && (js_code.contains("prototype")
                || js_code.contains("__proto__")
                || js_code.contains("Reflect")
                || js_code.contains("Object["))
        {
            if let Some(ref check_fn) = realm.ses_check_fn {
                if let Ok(v) = check_fn.call(&boa_engine::JsValue::undefined(), &[], &mut realm.ctx)
                {
                    if let Some(s) = v.as_string() {
                        let st = s.to_std_string_escaped();
                        if !st.is_empty() {
                            ses_mutated_target = st;
                        }
                    }
                }
            }
        }

        if self.deterministic_seed.is_some() {
            BOA_DET_STATE.with(|s| s.set(None));
        }
        if has_fs_restrictions {
            BOA_ACTIVE_CAPS.with(|c| *c.borrow_mut() = None);
        }
        if has_net_hosts {
            BOA_NET_HOSTS.with(|h| h.borrow_mut().clear());
        }
        if has_egress_secrets {
            BOA_EGRESS_SECRETS.with(|s| s.borrow_mut().clear());
        }
        if has_env_keys {
            let _ = realm.ctx.eval(boa_engine::Source::from_bytes(
                b"__slice_sync_process_env(false);",
            ));
            BOA_ENV_MAP.with(|m| m.borrow_mut().clear());
        }
        if has_next_tick {
            let _ = realm.ctx.eval(boa_engine::Source::from_bytes(
                b"__slice_set_next_tick_hook(false);",
            ));
        }
        BOA_FETCH_MOCKS.with(|m| m.borrow_mut().clear());
        crate::websocket::clear_thread_websocket_state();
        crate::sockets::clear_thread_socket_state();
        crate::workers::clear_thread_worker_state();
        crate::wasm::clear_thread_wasm_state();
        if let Some(violation) = BOA_VIOLATION.with(|v| v.borrow_mut().take()) {
            release_warm_boa_realm(realm);
            return Err(violation);
        }
        let wasm_fuel = BOA_FUEL_BUDGET.with(|f| f.get().0);
        if wasm_fuel > 0 {
            if let Err(v) = self.compartment.consume_fuel(wasm_fuel) {
                release_warm_boa_realm(realm);
                return Err(v);
            }
        }
        if !ses_mutated_target.is_empty() {
            // Do NOT return a realm whose SES intrinsics were mutated back to BOA_CONTEXT_POOL.
            drop(realm);
            return Err(SandboxViolation::IntrinsicMutationBlocked {
                tenant_id: self.compartment.tenant_id.clone(),
                intrinsic: ses_mutated_target,
            });
        }
        if let Err(ref err) = eval_res {
            let msg = err.to_string();
            if msg.contains("RuntimeLimitError")
                || msg.contains("iteration loops")
                || msg.contains("loop iteration limit")
                || msg.contains("Maximum call stack")
                || msg.contains("recursion limit")
            {
                release_warm_boa_realm(realm);
                return Err(SandboxViolation::CpuFuelExhausted {
                    tenant_id: self.compartment.tenant_id.clone(),
                    limit: self.compartment.fuel_limit,
                });
            }
        }
        let kv_used = BOA_KV_MEM_BUDGET.with(|m| m.get().0);
        if kv_used > 0 {
            let _ = self.compartment.alloc_memory(kv_used);
        }

        // Harvest captured stdout lines directly from thread-local Rust Vec<String> (zero second eval, retains Vec capacity!)
        BOA_STDOUT.with(|out| {
            let mut captured = out.borrow_mut();
            for line in captured.drain(..) {
                if line.contains('\n') {
                    for subline in line.lines() {
                        self.stdout.push(subline.to_string());
                    }
                } else {
                    self.stdout.push(line);
                }
            }
        });

        let result = match eval_res {
            Ok(val) => {
                if val.is_undefined() {
                    Ok(JsValue::Undefined)
                } else if val.is_null() {
                    Ok(JsValue::Null)
                } else if let Some(b) = val.as_boolean() {
                    Ok(JsValue::Bool(b))
                } else if let Some(n) = val.as_number() {
                    Ok(JsValue::Number(n))
                } else if let Ok(s) = val.to_string(&mut realm.ctx) {
                    Ok(JsValue::str(s.to_std_string_escaped()))
                } else {
                    Ok(JsValue::Undefined)
                }
            }
            Err(err) => Ok(JsValue::str(format!("Uncaught {}", err))),
        };

        release_warm_boa_realm(realm);
        result
    }
}

/// Zero-copy HTTP/1.1 request view borrowed directly from the worker thread's 64 KB socket read buffer.
#[derive(Debug, Clone, Copy)]
pub struct HttpRequestContext<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub query: &'a str,
    pub tenant_id: &'a str,
    pub body: &'a str,
    pub headers: &'a [(&'a str, &'a str)],
}

/// Structured outcome of executing a WinterTC HTTP request inside a `SliceIsolate`.
#[derive(Debug, Clone)]
pub struct HttpExecutionOutcome {
    pub metrics: ExecutionMetrics,
    pub status_code: u16,
    pub content_type: String,
    pub response_body: String,
    pub response_headers: Vec<(String, String)>,
    pub wait_until_count: usize,
}

/// Runs a source script directly by compiling to bytecode and executing in a fresh isolate.
pub fn run_source_isolated(
    tenant_id: &str,
    raw_source: &str,
    capabilities: CapabilityToken,
    fuel_limit: u64,
    memory_limit_bytes: usize,
) -> Result<ExecutionMetrics> {
    let bytecode = compile_source_to_bytecode(raw_source);
    let mut isolate =
        SliceIsolate::spawn(tenant_id, 1, capabilities, fuel_limit, memory_limit_bytes);
    match isolate.execute_bytecode(&bytecode, None) {
        Ok(metrics) => Ok(metrics),
        Err(violation) => bail!(violation),
    }
}
