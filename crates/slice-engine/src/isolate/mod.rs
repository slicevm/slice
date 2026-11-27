//! Polyglot Tiered Isolate (`SliceIsolate`), WinterTC HTTP Execution Context (`HttpRequestContext`),
//! Nested Sandbox Attenuator (`SandboxEvalOptions`), and Execution Telemetry (`ExecutionMetrics`).
//!
//! Organized into focused domain submodules:
//! - `kv_and_sandbox`: Tenant-isolated `Slice.kv` operations, `< 5 us` state checkpoints (`checkpoint`/`restore`),
//!   and nested OCap child sandbox attenuation (`eval_sandbox`).
//! - `exec_bytecode`: Top-level bytecode dispatcher (`SLICEBC\x01`, `SLICEBOA`, `SLICESNP`) and Tier-2 SIMD/native kernels.
//! - `exec_ast`: Tier-0 recursive-descent AST & Function VM with unboxed 64-bit `NumericOp` fast-path and recursion/memory guards.
//! - `exec_boa`: Full ES2024+ `boa_engine` and `BoaCAS` execution, warm realm pooling, and shared OCap/DLP/SES lifecycle guards.
//! - `http_and_ws`: WinterTC HTTP request execution (`execute_http_request`), post-response `Slice.waitUntil` draining,
//!   and server WebSocket frame dispatch (`execute_websocket_event`).

pub(crate) mod exec_ast;
pub(crate) mod exec_boa;
pub(crate) mod exec_bytecode;
pub(crate) mod exec_llint;
pub(crate) mod http_and_ws;
pub(crate) mod kv_and_sandbox;
pub mod state_and_kv;
pub(crate) mod vfs_drain;

pub use self::http_and_ws::{HttpExecutionOutcome, HttpFastOutcome, HttpRequestContext};
pub use self::kv_and_sandbox::{SandboxEvalOptions, SandboxEvalOutcome};
pub use self::vfs_drain::{DrainLimits, DrainReport, DrainedFile, SkipReason, SkippedFile};

use crate::parser_and_compiler::compile_source_to_bytecode;
use crate::state_and_kv::verify_egress_secrets_slice;
use crate::value_and_ast::{ExecutionTier, JsValue};
use anyhow::{Result, bail};
use compact_str::CompactString;
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
    /// Display string of the script's completion value. Note that object formatting depends on
    /// execution tier (e.g. `{ a: 1 }` on Tier-0 vs JS `ToString` `[object Object]` on Tier-1);
    /// use [`SliceIsolate::set_capture_return_json`] + [`SliceIsolate::last_return_json`] when
    /// reading structured JSON values across tiers.
    pub return_value: String,
    #[serde(default)]
    pub checkpoints_taken: u32,
    #[serde(default)]
    pub restores_performed: u32,
    #[serde(default)]
    pub restore_time_ns: u64,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub process_exited: bool,
    #[serde(default)]
    pub denied_capabilities: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum ControlFlow {
    None(JsValue),
    Return(JsValue),
    Break,
    Continue,
}

/// Thread-safe handle allowing an embedder to interrupt a running [`SliceIsolate`] from another thread.
#[derive(Debug, Clone)]
pub struct IsolateInterruptHandle {
    flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl IsolateInterruptHandle {
    /// Signals the associated [`SliceIsolate`] to abort execution with `SandboxViolation::ExecutionInterrupted`.
    #[inline]
    pub fn interrupt(&self) {
        self.flag.store(true, std::sync::atomic::Ordering::Release);
    }

    /// Returns `true` if `interrupt()` has been called on this handle.
    #[inline]
    pub fn is_interrupted(&self) -> bool {
        self.flag.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Clears a previously set interrupt signal before reusing the isolate.
    #[inline]
    pub fn reset(&self) {
        self.flag.store(false, std::sync::atomic::Ordering::Release);
    }
}

/// Polyglot Tiered Isolate executing within a hardware- and OCap-protected Compartment.
#[repr(C)]
pub struct SliceIsolate {
    pub compartment: TenantCompartment,
    pub cas_modules_resolved: usize,
    pub current_tier: ExecutionTier,
    pub(crate) env_vars: FxHashMap<CompactString, JsValue>,
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
    pub(crate) last_process_exited: bool,
    pub(crate) pending_tier0_throw: Option<String>,
    pub(crate) process_argv: Vec<String>,
    pub max_wall_clock_ms: Option<u64>,
    pub(crate) active_wall_deadline: Option<std::time::Instant>,
    pub(crate) interrupt_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    pub(crate) extensions: Option<crate::extensions::ExtensionRegistry>,
    pub denied_capabilities: Vec<String>,
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
            last_process_exited: false,
            pending_tier0_throw: None,
            process_argv: Vec::new(),
            max_wall_clock_ms: None,
            active_wall_deadline: None,
            interrupt_flag: None,
            extensions: None,
            denied_capabilities: Vec::new(),
        }
    }

    /// Instantiates a fresh microVM-grade isolate with a shared [`crate::extensions::ExtensionRegistry`]
    /// (`Arc`-cloned in `O(1)` with zero per-boot JS evaluation).
    #[inline]
    pub fn spawn_with_extensions(
        tenant_id: impl Into<String>,
        slot_id: u32,
        capabilities: CapabilityToken,
        fuel_limit: u64,
        memory_limit_bytes: usize,
        extensions: crate::extensions::ExtensionRegistry,
    ) -> Self {
        let mut iso = Self::spawn(
            tenant_id,
            slot_id,
            capabilities,
            fuel_limit,
            memory_limit_bytes,
        );
        iso.extensions = Some(extensions);
        iso
    }

    /// Attaches or clears a shared host [`crate::extensions::ExtensionRegistry`] on this isolate.
    #[inline]
    pub fn set_extensions(&mut self, extensions: Option<crate::extensions::ExtensionRegistry>) {
        self.extensions = extensions;
    }

    /// Amortized wall-clock deadline and cross-thread interrupt check for Tier-0 loops.
    /// Costs a single bitwise AND on 1,023 out of 1,024 iterations.
    #[inline(always)]
    pub(crate) fn check_tier0_deadline_and_interrupt(
        &self,
        iters: u64,
    ) -> Result<(), SandboxViolation> {
        if (iters & 0x03FF) != 0 {
            return Ok(());
        }
        if let Some(ref flag) = self.interrupt_flag
            && flag.load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(SandboxViolation::ExecutionInterrupted {
                tenant_id: self.compartment.tenant_id.clone(),
            });
        }
        if let Some(deadline) = self.active_wall_deadline
            && std::time::Instant::now() >= deadline
        {
            return Err(SandboxViolation::WallClockExceeded {
                tenant_id: self.compartment.tenant_id.clone(),
                limit_ms: self.max_wall_clock_ms.unwrap_or(0),
            });
        }
        Ok(())
    }

    /// Returns the audit log of denied OCap capability attempts observed during the most recent
    /// execution (including denials caught by guest `try/catch` or `.catch()`).
    #[inline]
    pub fn denied_capabilities(&self) -> &[String] {
        &self.denied_capabilities
    }

    /// Configures an optional wall-clock execution deadline in milliseconds (`None` = disabled).
    /// When exceeded, execution aborts with `SandboxViolation::WallClockExceeded`.
    #[inline]
    pub fn set_wall_clock_limit_ms(&mut self, timeout_ms: Option<u64>) {
        self.max_wall_clock_ms = timeout_ms;
    }

    /// Returns a thread-safe [`IsolateInterruptHandle`] that can abort this isolate's execution
    /// from another thread with `SandboxViolation::ExecutionInterrupted`.
    #[inline]
    pub fn interrupt_handle(&mut self) -> IsolateInterruptHandle {
        let flag = self
            .interrupt_flag
            .get_or_insert_with(|| std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)));
        IsolateInterruptHandle {
            flag: std::sync::Arc::clone(flag),
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

    /// Returns the canonical JSON representation of the last evaluated expression, if captured
    /// (`set_capture_return_json(true)`). Prefer this over `ExecutionMetrics::return_value` when
    /// reading structured object/array results consistently across Tier-0 and Tier-1.
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
    /// Pre-expands raw, hex, URL/percent-encoded, padded/unpadded Base64 & Base64URL, and all 3
    /// Base64 triplet-alignment slices at registration time so `verify_egress_secrets_slice` scans
    /// payloads in a single `O(payload)` pass via a cached `aho-corasick` automaton.
    pub fn add_egress_audit_secret(&mut self, secret: impl Into<String>) {
        let s = secret.into();
        if s.is_empty() {
            return;
        }
        crate::state_and_kv::expand_egress_secret_variants(&s, &mut self.egress_audit_secrets);
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
    /// for `CapabilityToken` or `tenant_id`, rebinding its `HardwareProtectionDomain` (`PKRU`
    /// register switch on PKU-capable Linux `x86_64` hosts; virtual slot tag elsewhere) and
    /// resetting CPU fuel and memory counters.
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
        self.compartment.pkey_domain.rebind_slot(slot_id);
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
        self.last_process_exited = false;
        self.denied_capabilities.clear();
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
