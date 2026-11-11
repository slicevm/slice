//! Polyglot Tiered Isolate (`SliceIsolate`), WinterTC HTTP Execution Context (`HttpRequestContext`),
//! Nested Sandbox Attenuator (`SandboxEvalOptions`), and Execution Telemetry (`ExecutionMetrics`).
//!
//! Organized into focused domain submodules:
//! - [`kv_and_sandbox`]: Tenant-isolated `Slice.kv` operations, `< 5 us` state checkpoints (`checkpoint`/`restore`),
//!   and nested OCap child sandbox attenuation (`eval_sandbox`).
//! - [`exec_bytecode`]: Top-level bytecode dispatcher (`SLICEBC\x01`, `SLICEBOA`, `SLICESNP`) and Tier-2 SIMD/native kernels.
//! - [`exec_ast`]: Tier-0 recursive-descent AST & Function VM with unboxed 64-bit `NumericOp` fast-path and recursion/memory guards.
//! - [`exec_boa`]: Full ES2024+ `boa_engine` and `BoaCAS` execution, warm realm pooling, and shared OCap/DLP/SES lifecycle guards.
//! - [`http_and_ws`]: WinterTC HTTP request execution (`execute_http_request`), post-response `Slice.waitUntil` draining,
//!   and server WebSocket frame dispatch (`execute_websocket_event`).

pub(crate) mod exec_ast;
pub(crate) mod exec_boa;
pub(crate) mod exec_bytecode;
pub(crate) mod http_and_ws;
pub(crate) mod kv_and_sandbox;
pub mod state_and_kv;

pub use self::http_and_ws::{HttpExecutionOutcome, HttpFastOutcome, HttpRequestContext};
pub use self::kv_and_sandbox::{SandboxEvalOptions, SandboxEvalOutcome};

use crate::parser_and_compiler::compile_source_to_bytecode;
use crate::state_and_kv::verify_egress_secrets_slice;
use crate::value_and_ast::{ExecutionTier, JsValue};
use anyhow::{bail, Result};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use slice_sandbox::{CapabilityToken, SandboxViolation, TenantCompartment};

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
    #[serde(default)]
    pub checkpoints_taken: u32,
    #[serde(default)]
    pub restores_performed: u32,
    #[serde(default)]
    pub restore_time_ns: u64,
    #[serde(default)]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone)]
pub(crate) enum ControlFlow {
    None(JsValue),
    Return(JsValue),
    Break,
    Continue,
}

/// Polyglot Tiered Isolate executing within a hardware- and OCap-protected Compartment.
#[repr(C)]
pub struct SliceIsolate {
    pub compartment: TenantCompartment,
    pub cas_modules_resolved: usize,
    pub current_tier: ExecutionTier,
    pub(crate) env_vars: FxHashMap<String, JsValue>,
    pub stdout: Vec<String>,
    pub deterministic_seed: Option<u64>,
    pub(crate) det_prng_state: u64,
    pub(crate) det_clock_ms: u64,
    pub egress_audit_secrets: Vec<String>,
    pub(crate) recursion_depth: u32,
    pub checkpoints_taken: u32,
    pub restores_performed: u32,
    pub restore_time_ns: u64,
    pub(crate) capture_return_json: bool,
    pub(crate) last_return_json: Option<String>,
    pub(crate) last_exec_check_ses: bool,
    pub(crate) last_exit_code: Option<i32>,
    pub(crate) process_argv: Vec<String>,
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
            current_tier: ExecutionTier::Tier1Baseline,
            env_vars: FxHashMap::default(),
            stdout: Vec::new(),
            deterministic_seed: None,
            det_prng_state: 0,
            det_clock_ms: 1_700_000_000_000,
            egress_audit_secrets: Vec::new(),
            recursion_depth: 0,
            checkpoints_taken: 0,
            restores_performed: 0,
            restore_time_ns: 0,
            capture_return_json: false,
            last_return_json: None,
            last_exec_check_ses: false,
            last_exit_code: None,
            process_argv: Vec::new(),
        }
    }

    /// Returns the explicit or implicit process exit code (`process.exit(code)`, `process.exitCode`,
    /// or `1` on uncaught exception / unhandled rejection) from the most recent execution, if set.
    #[inline]
    pub fn last_exit_code(&self) -> Option<i32> {
        self.last_exit_code
    }

    /// Enables or disables capturing the JSON representation of the last evaluated expression (`-p` / `--print`).
    #[inline]
    pub fn set_capture_return_json(&mut self, enable: bool) {
        self.capture_return_json = enable;
    }

    /// Returns the captured JSON representation of the last evaluated expression, if available.
    #[inline]
    pub fn last_return_json(&self) -> Option<&str> {
        self.last_return_json.as_deref()
    }

    /// Configures `process.argv` for CLI script execution (`["slice", script_path, ...args]`).
    #[inline]
    pub fn set_process_argv(&mut self, argv: Vec<String>) {
        self.process_argv = argv;
    }

    /// Registers a sensitive secret token to be guarded against egress exfiltration (`--audit-egress`).
    /// When no secrets are registered (default), the egress guard costs `0.00 ns`.
    /// Pre-computes both standard Base64 and URL-safe Base64 encodings at registration time
    /// so `verify_egress_secrets_slice` performs zero heap allocations on hot egress paths.
    pub fn add_egress_audit_secret(&mut self, secret: impl Into<String>) {
        let s = secret.into();
        if s.is_empty() {
            return;
        }
        let b64_std = crate::intrinsics::base64_encode(s.as_bytes(), false);
        let b64_url = crate::intrinsics::base64_encode(s.as_bytes(), true);
        if !self.egress_audit_secrets.contains(&s) {
            self.egress_audit_secrets.push(s);
        }
        if !b64_std.is_empty() && !self.egress_audit_secrets.contains(&b64_std) {
            self.egress_audit_secrets.push(b64_std);
        }
        if !b64_url.is_empty() && !self.egress_audit_secrets.contains(&b64_url) {
            self.egress_audit_secrets.push(b64_url);
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
    pub(crate) fn verify_egress_not_leaked(&self, payload: &str) -> Result<(), SandboxViolation> {
        if self.egress_audit_secrets.is_empty() || payload.is_empty() {
            return Ok(());
        }
        verify_egress_secrets_slice(
            &self.compartment.tenant_id,
            &self.egress_audit_secrets,
            payload,
        )
    }

    /// Enables or disables Chrome DevTools / Perfetto execution trace recording (`--trace=<out.json>`)
    /// on the current worker thread. When disabled (`false`, default), tracing incurs `0.00 ns` overhead.
    pub fn enable_tracing(&mut self, enabled: bool) {
        crate::boa_realm::BOA_TRACE_ENABLED.with(|c| c.set(enabled));
        if enabled {
            crate::boa_realm::BOA_TRACE_EVENTS.with(|ev| ev.borrow_mut().clear());
        }
    }

    /// Exports the execution timeline and recorded time-travel / OCap spans as a Chrome DevTools
    /// / Perfetto `traceEvents` JSON document (`chrome://tracing` / `ui.perfetto.dev`).
    pub fn export_chrome_trace_json(&self, metrics: &ExecutionMetrics) -> String {
        use crate::boa_realm::ChromeTraceEvent;
        let mut events = Vec::with_capacity(8);
        events.push(ChromeTraceEvent {
            name: "isolate.instantiate".to_string(),
            cat: "slice.lifecycle".to_string(),
            ph: "X".to_string(),
            ts: 0,
            dur: metrics.instantiate_time_ns.max(1),
            pid: 1,
            tid: 1,
            args: serde_json::json!({
                "tenant_id": metrics.tenant_id,
                "pkey_domain": metrics.pkey_domain,
                "os_stat_syscalls": metrics.os_stat_syscalls,
            }),
        });
        events.push(ChromeTraceEvent {
            name: "isolate.execute".to_string(),
            cat: "slice.vm".to_string(),
            ph: "X".to_string(),
            ts: metrics.instantiate_time_ns,
            dur: metrics.execution_time_ns.max(1),
            pid: 1,
            tid: 1,
            args: serde_json::json!({
                "tenant_id": metrics.tenant_id,
                "tier": format!("{:?}", metrics.highest_tier_reached),
                "fuel_consumed": metrics.fuel_consumed,
                "memory_used_bytes": metrics.memory_used_bytes,
                "cas_modules_resolved": metrics.cas_modules_resolved,
                "return_value": metrics.return_value,
            }),
        });
        crate::boa_realm::BOA_TRACE_EVENTS.with(|recorded| {
            for ev in recorded.borrow().iter() {
                events.push(ev.clone());
            }
        });
        let doc = serde_json::json!({
            "displayTimeUnit": "ns",
            "traceEvents": events,
        });
        serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".to_string())
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
        if self.checkpoints_taken > 0 {
            crate::state_and_kv::clear_tenant_checkpoints(&self.compartment.tenant_id);
        }
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
        self.current_tier = ExecutionTier::Tier1Baseline;
        self.env_vars.clear();
        self.stdout.clear();
        self.recursion_depth = 0;
        self.checkpoints_taken = 0;
        self.restores_performed = 0;
        self.restore_time_ns = 0;
        self.last_return_json = None;
        self.last_exec_check_ses = false;
        self.last_exit_code = None;
        if let Some(s) = self.deterministic_seed {
            self.det_prng_state = s ^ 0x9E37_79B9_7F4A_7C15;
            self.det_clock_ms = 1_700_000_000_000;
        }
    }
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
