//! Zero-Stat MicroVM Web Workers (`new Worker()`) & Node.js `node:worker_threads` Engine.
//!
//! Implements capability-attenuated multi-threaded background isolate execution:
//! - Sub-millisecond warm isolate checkout on background worker threads (`16 MB` stack size)
//! - Strict Capability Attenuation (`attenuate_worker_capabilities`): child workers can only
//!   receive a subset (`parent ∩ requested`) of the parent isolate's `allow_read`, `allow_write`,
//!   and `allow_net` permissions, and always inherit `BOA_EGRESS_SECRETS` DLP rules
//! - Bounded CPU instruction fuel (`fuel`) and memory ceiling (`memoryBytes`) per worker
//! - Lock-free `mpsc` bidirectional message channels integrated directly into `event_loop.rs`

use crate::bindings::fs::VfsSnapshot;
use crate::boa_realm::{
    check_boa_fs_read_ocap, mark_subsystem_dirty, norm_vfs_path, vfs_owner_namespace, BoaFsCaps,
    BOA_ACTIVE_CAPS, BOA_ACTIVE_TENANT, BOA_DET_STATE, BOA_EGRESS_SECRETS, BOA_ENV_MAP,
    BOA_EXEC_BINS, BOA_FETCH_MOCKS, BOA_FUEL_BUDGET, BOA_KV_MEM_BUDGET, BOA_NET_HOSTS,
    BOA_VFS_OWNER_TENANT, BOA_VFS_STATE, BOA_VIOLATION, DIRTY_WORKERS,
};
use crate::intrinsics::base64_decode;
use crate::isolate::SliceIsolate;
use crate::sockets::BOA_SOCKET_MOCKS;
use crate::websocket::BOA_WS_MOCKS;
use boa_engine::{Context, JsValue, Source};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Deserialize;
use slice_sandbox::{CapabilityToken, SandboxViolation};
use std::cell::{Cell, RefCell};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// Maximum concurrent active workers per parent isolate.
pub const MAX_WORKERS_PER_ISOLATE: usize = 16;

/// Maximum concurrent active workers globally across the host process.
pub const MAX_GLOBAL_ACTIVE_WORKERS: usize = 32;

/// Maximum serialized message payload size (`4 MB`) for `postMessage`.
pub const MAX_WORKER_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

/// Maximum persistent warm worker OS threads retained in `IDLE_WORKER_POOL`.
pub const MAX_POOLED_WORKER_THREADS: usize = 8;

/// Maximum cached transpiled worker script bytecode payloads in `WORKER_SCRIPT_CACHE`.
const MAX_CACHED_WORKER_SCRIPTS: usize = 256;

static GLOBAL_ACTIVE_WORKERS: AtomicUsize = AtomicUsize::new(0);
static TOTAL_POOLED_WORKER_THREADS: AtomicUsize = AtomicUsize::new(0);
static WORKER_POOL_REUSE_COUNT: AtomicUsize = AtomicUsize::new(0);
static WORKER_SCRIPT_CACHE_HITS: AtomicUsize = AtomicUsize::new(0);
static WORKER_SCRIPT_CACHE: OnceLock<Mutex<FxHashMap<u64, Arc<Vec<u8>>>>> = OnceLock::new();
static IDLE_WORKER_POOL: OnceLock<Mutex<Vec<Sender<WorkerJob>>>> = OnceLock::new();

struct WorkerJob {
    worker_id: u32,
    worker_name: String,
    child_tenant: String,
    worker_data_json: String,
    child_caps: CapabilityToken,
    child_fuel: u64,
    child_mem: usize,
    parent_det: Option<(u64, u64)>,
    parent_egress_secrets: Vec<String>,
    parent_vfs_snap: VfsSnapshot,
    parent_fetch_mocks: FxHashMap<String, String>,
    parent_ws_mocks: FxHashMap<String, String>,
    parent_sock_mocks: FxHashMap<String, String>,
    inbound_rx: Receiver<WorkerInboundCmd>,
    event_tx: Sender<WorkerOutboundEvent>,
    terminated: Arc<AtomicBool>,
    bytecode: Arc<Vec<u8>>,
}

fn worker_script_cache() -> &'static Mutex<FxHashMap<u64, Arc<Vec<u8>>>> {
    WORKER_SCRIPT_CACHE.get_or_init(|| Mutex::new(FxHashMap::default()))
}

fn idle_worker_pool() -> &'static Mutex<Vec<Sender<WorkerJob>>> {
    IDLE_WORKER_POOL.get_or_init(|| Mutex::new(Vec::with_capacity(MAX_POOLED_WORKER_THREADS)))
}

/// Returns the number of times `new Worker()` reused a warm worker thread from `IDLE_WORKER_POOL`
/// instead of spawning a new OS thread and cold-initializing a `boa_engine::Context`.
pub fn worker_pool_reuse_count() -> usize {
    WORKER_POOL_REUSE_COUNT.load(Ordering::Acquire)
}

/// Returns the number of times `new Worker()` hit the pre-transpiled `WORKER_SCRIPT_CACHE`
/// instead of re-running Oxc TypeScript stripping.
pub fn worker_script_cache_hits() -> usize {
    WORKER_SCRIPT_CACHE_HITS.load(Ordering::Acquire)
}

/// Resets worker pool telemetry counters (for deterministic testing).
pub fn reset_worker_pool_telemetry() {
    WORKER_POOL_REUSE_COUNT.store(0, Ordering::Release);
    WORKER_SCRIPT_CACHE_HITS.store(0, Ordering::Release);
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerSliceAttenuationOpts {
    pub fuel: Option<u64>,
    pub memory_bytes: Option<usize>,
    pub allow_read: Option<Vec<String>>,
    pub allow_write: Option<Vec<String>>,
    pub allow_net: Option<Vec<String>>,
    pub allow_exec: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerSpawnOptions {
    #[serde(default)]
    pub eval: bool,
    pub worker_data_json: Option<String>,
    pub name: Option<String>,
    pub slice: Option<WorkerSliceAttenuationOpts>,
}

pub(crate) enum WorkerInboundCmd {
    Message(String),
    Terminate,
}

#[derive(Debug)]
pub(crate) enum WorkerOutboundEventKind {
    Online,
    Message(String),
    Error { message: String, is_violation: bool },
    Exit { exit_code: i32, fuel_consumed: u64 },
}

#[derive(Debug)]
pub(crate) struct WorkerOutboundEvent {
    pub(crate) worker_id: u32,
    pub(crate) kind: WorkerOutboundEventKind,
}

struct ParentWorkerHandle {
    inbound_tx: Sender<WorkerInboundCmd>,
    terminated: Arc<AtomicBool>,
    finished: bool,
    refed: bool,
    reserved_fuel: u64,
    reserved_mem: usize,
    reconciled: bool,
}

impl ParentWorkerHandle {
    #[inline]
    fn reconcile_resources(&mut self, actual_fuel_consumed: u64) {
        if self.reconciled {
            return;
        }
        self.reconciled = true;
        let refund_fuel = self.reserved_fuel.saturating_sub(actual_fuel_consumed);
        if refund_fuel > 0 {
            BOA_FUEL_BUDGET.with(|f| {
                let (used, limit) = f.get();
                f.set((used.saturating_sub(refund_fuel), limit));
            });
        }
        if self.reserved_mem > 0 {
            BOA_KV_MEM_BUDGET.with(|b| {
                let (used, limit) = b.get();
                b.set((used.saturating_sub(self.reserved_mem), limit));
            });
        }
    }
}

struct ParentWorkerRegistry {
    next_worker_id: u32,
    workers: FxHashMap<u32, ParentWorkerHandle>,
    event_tx: Sender<WorkerOutboundEvent>,
    event_rx: Receiver<WorkerOutboundEvent>,
}

impl ParentWorkerRegistry {
    fn new() -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        Self {
            next_worker_id: 1,
            workers: FxHashMap::default(),
            event_tx,
            event_rx,
        }
    }

    fn reset(&mut self) {
        self.next_worker_id = 1;
        for (_, mut h) in self.workers.drain() {
            h.terminated.store(true, Ordering::Release);
            let _ = h.inbound_tx.send(WorkerInboundCmd::Terminate);
            h.reconcile_resources(h.reserved_fuel.min(1_000));
        }
        while self.event_rx.try_recv().is_ok() {}
    }

    #[inline]
    fn has_active_refed_workers(&self) -> bool {
        self.workers.values().any(|w| !w.finished && w.refed)
    }
}

struct WorkerSelfThreadEnv {
    is_worker_thread: bool,
    thread_id: u32,
    name: String,
    worker_data_json: String,
    inbound_rx: Option<Receiver<WorkerInboundCmd>>,
    outbound_tx: Option<Sender<WorkerOutboundEvent>>,
    terminated: Option<Arc<AtomicBool>>,
    self_listening: bool,
    explicit_exit_code: Option<i32>,
}

impl Default for WorkerSelfThreadEnv {
    fn default() -> Self {
        Self {
            is_worker_thread: false,
            thread_id: 0,
            name: String::new(),
            worker_data_json: "undefined".to_string(),
            inbound_rx: None,
            outbound_tx: None,
            terminated: None,
            self_listening: false,
            explicit_exit_code: None,
        }
    }
}

thread_local! {
    static PARENT_WORKERS: RefCell<ParentWorkerRegistry> = RefCell::new(ParentWorkerRegistry::new());
    static WORKER_SELF_ENV: RefCell<WorkerSelfThreadEnv> = RefCell::new(WorkerSelfThreadEnv::default());
    static WORKER_ACTIVE_COUNT: Cell<usize> = const { Cell::new(0) };
}

/// Resets all active workers spawned by the current parent thread.
pub fn clear_thread_worker_state() {
    PARENT_WORKERS.with(|reg| reg.borrow_mut().reset());
    WORKER_ACTIVE_COUNT.with(|c| c.set(0));
}

/// Returns `true` if the current isolate has active `ref`ed child workers or unread worker events,
/// OR if the current isolate IS a worker thread that is actively listening for parent messages.
#[inline]
pub(crate) fn has_pending_worker_work() -> bool {
    let has_parent_workers = if WORKER_ACTIVE_COUNT.with(|c| c.get() > 0) {
        PARENT_WORKERS.with(|reg| reg.borrow().has_active_refed_workers())
    } else {
        false
    };
    if has_parent_workers {
        return true;
    }
    WORKER_SELF_ENV.with(|env| {
        let e = env.borrow();
        if !e.is_worker_thread || !e.self_listening {
            return false;
        }
        if let Some(ref term) = e.terminated {
            if term.load(Ordering::Acquire) {
                return false;
            }
        }
        true
    })
}

/// Returns `true` if the current OS thread is executing a spawned child worker isolate.
#[inline]
pub(crate) fn is_worker_thread() -> bool {
    WORKER_SELF_ENV.with(|env| env.borrow().is_worker_thread)
}

/// Returns `true` if the current worker thread has been terminated by its parent (`worker.terminate()`).
#[inline]
pub(crate) fn is_current_worker_terminated() -> bool {
    WORKER_SELF_ENV.with(|env| {
        let e = env.borrow();
        e.terminated
            .as_ref()
            .map(|t| t.load(Ordering::Acquire))
            .unwrap_or(false)
    })
}

fn charge_worker_message_budget(bytes: usize) -> Result<(), String> {
    if bytes > MAX_WORKER_MESSAGE_BYTES {
        return Err(format!(
            "DataCloneError: Worker postMessage payload ({} bytes) exceeds maximum limit ({} bytes)",
            bytes, MAX_WORKER_MESSAGE_BYTES
        ));
    }
    let exceeded = BOA_KV_MEM_BUDGET.with(|b| {
        let (used, limit) = b.get();
        if limit > 0 && used.saturating_add(bytes) > limit {
            Some((used.saturating_add(bytes), limit))
        } else {
            b.set((used.saturating_add(bytes), limit));
            None
        }
    });
    if let Some((requested, limit)) = exceeded {
        let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
        BOA_VIOLATION.with(|v| {
            *v.borrow_mut() = Some(SandboxViolation::MemoryCeilingExceeded {
                tenant_id,
                requested,
                limit,
            });
        });
        return Err(format!(
            "SandboxViolation: Worker postMessage allocation ({} bytes) exceeded tenant memory ceiling ({} bytes)",
            requested, limit
        ));
    }
    Ok(())
}

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
            } else {
                return Ok(body.to_string());
            }
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

    // 1. Check in-memory `VfsState` first (0 disk stat syscalls, explicit file-kind check)
    match crate::bindings::fs::read_vfs_file_bytes_resolved(raw_path) {
        Ok(Some(vfs_src)) => return Ok(String::from_utf8_lossy(&vfs_src).into_owned()),
        Ok(None) => {}
        Err(()) => {
            return Err(format!(
                "WorkerError: Cannot read worker module '{}': symlink loop or OCap denied",
                raw_path
            ));
        }
    }

    if crate::bindings::fs::is_vfs_tombstoned(&norm)
        || crate::bindings::fs::is_host_unc_blocked(raw_path)
    {
        return Err(format!(
            "WorkerError: Cannot read worker module '{}': file removed or blocked in VFS",
            raw_path
        ));
    }

    // 2. Fallback to host filesystem if OCap permits
    std::fs::read_to_string(&norm).map_err(|e| {
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
    let mut child_caps = CapabilityToken::pure_compute();
    let parent_reads = parent_caps.read_prefixes.as_slice();
    let parent_writes = parent_caps.write_prefixes.as_slice();

    // 1. Attenuate read prefixes strictly (`parent ∩ requested`; empty allow-list = deny-all)
    if let Some(req_reads) = attenuation.and_then(|a| a.allow_read.as_ref()) {
        for req in req_reads {
            if !slice_sandbox::is_rejected_vfs_candidate(req)
                && parent_reads
                    .iter()
                    .any(|p| slice_sandbox::vfs_prefix_matches(req, p))
            {
                child_caps = child_caps.with_read_prefix(norm_vfs_path(req));
            }
        }
    } else {
        for p in parent_reads {
            child_caps = child_caps.with_read_prefix(p.clone());
        }
    }

    // 2. Attenuate write prefixes strictly (`parent ∩ requested`; empty allow-list = deny-all)
    if let Some(req_writes) = attenuation.and_then(|a| a.allow_write.as_ref()) {
        for req in req_writes {
            if !slice_sandbox::is_rejected_vfs_candidate(req)
                && parent_writes
                    .iter()
                    .any(|p| slice_sandbox::vfs_prefix_matches(req, p))
            {
                child_caps = child_caps.with_write_prefix(norm_vfs_path(req));
            }
        }
    } else {
        for p in parent_writes {
            child_caps = child_caps.with_write_prefix(p.clone());
        }
    }

    // 3. Attenuate network hosts
    if let Some(req_nets) = attenuation.and_then(|a| a.allow_net.as_ref()) {
        for req_host in req_nets {
            let h_lower = req_host.trim().to_ascii_lowercase();
            if !slice_sandbox::is_cloud_metadata_ssrf_target(&h_lower)
                && (parent_net_hosts.contains(&h_lower) || parent_net_hosts.contains("*"))
            {
                child_caps = child_caps.with_net_host(h_lower);
            }
        }
    } else {
        for h in parent_net_hosts {
            child_caps = child_caps.with_net_host(h.clone());
        }
    }

    // 4. Inherit allowed environment variables
    for (k, v) in parent_env_map {
        child_caps = child_caps.with_env(k.clone(), v.clone());
    }

    // 5. Attenuate allowed exec binaries against parent's `BOA_EXEC_BINS`
    let parent_execs = BOA_EXEC_BINS.with(|eb_cell| eb_cell.borrow().clone());
    if let Some(req_execs) = attenuation.and_then(|a| a.allow_exec.as_ref()) {
        for req_bin in req_execs {
            if parent_execs
                .iter()
                .any(|allowed| slice_sandbox::exec_binary_matches(req_bin, allowed))
            {
                child_caps = child_caps.with_exec_binary(req_bin.clone());
            }
        }
    } else {
        for eb in &parent_execs {
            child_caps = child_caps.with_exec_binary(eb.clone());
        }
    }

    child_caps
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

    if let Ok(cache) = worker_script_cache().lock() {
        if let Some(existing) = cache.get(&script_hash) {
            WORKER_SCRIPT_CACHE_HITS.fetch_add(1, Ordering::Relaxed);
            return Arc::clone(existing);
        }
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

/// Speculatively spawns up to `target_threads` background worker threads into `IDLE_WORKER_POOL`
/// if none have been spawned yet, allowing background worker `WarmBoaRealm` initialization to run
/// concurrently on another CPU core while the main isolate is booting.
pub fn speculative_prewarm_worker_pool(target_threads: usize) {
    let target = target_threads.min(MAX_POOLED_WORKER_THREADS);
    if TOTAL_POOLED_WORKER_THREADS.load(Ordering::Acquire) >= target {
        return;
    }
    let Ok(mut pool) = idle_worker_pool().lock() else {
        return;
    };
    while TOTAL_POOLED_WORKER_THREADS.load(Ordering::Acquire) < target {
        let idx = TOTAL_POOLED_WORKER_THREADS.fetch_add(1, Ordering::AcqRel) + 1;
        let (job_tx, job_rx) = mpsc::channel::<WorkerJob>();
        pool.push(job_tx.clone());
        let builder = std::thread::Builder::new()
            .name(format!("slice-worker-pool-{}", idx))
            .stack_size(8 * 1024 * 1024);
        if builder
            .spawn(move || {
                worker_pool_thread_loop(job_tx, job_rx, None);
            })
            .is_err()
        {
            TOTAL_POOLED_WORKER_THREADS.fetch_sub(1, Ordering::AcqRel);
            let _ = pool.pop();
            break;
        }
    }
}

/// Synchronously warms `target_threads` background worker threads (waiting until their
/// `WarmBoaRealm` and prelude shards `2` and `8` are fully initialized). Used by `--warm`.
pub fn prewarm_worker_pool_sync(target_threads: usize) {
    let target = target_threads.min(MAX_POOLED_WORKER_THREADS);
    if TOTAL_POOLED_WORKER_THREADS.load(Ordering::Acquire) >= target {
        return;
    }
    let mut ready_rxs = Vec::new();
    if let Ok(mut pool) = idle_worker_pool().lock() {
        while TOTAL_POOLED_WORKER_THREADS.load(Ordering::Acquire) < target {
            let idx = TOTAL_POOLED_WORKER_THREADS.fetch_add(1, Ordering::AcqRel) + 1;
            let (job_tx, job_rx) = mpsc::channel::<WorkerJob>();
            let (ready_tx, ready_rx) = mpsc::channel::<()>();
            pool.push(job_tx.clone());
            let builder = std::thread::Builder::new()
                .name(format!("slice-worker-pool-{}", idx))
                .stack_size(8 * 1024 * 1024);
            if builder
                .spawn(move || {
                    worker_pool_thread_loop(job_tx, job_rx, Some(ready_tx));
                })
                .is_ok()
            {
                ready_rxs.push(ready_rx);
            } else {
                TOTAL_POOLED_WORKER_THREADS.fetch_sub(1, Ordering::AcqRel);
                let _ = pool.pop();
                break;
            }
        }
    }
    for rx in ready_rxs {
        let _ = rx.recv_timeout(Duration::from_secs(2));
    }
}

fn dispatch_worker_job(mut job: WorkerJob) -> Result<(), String> {
    loop {
        let candidate_tx = idle_worker_pool().lock().ok().and_then(|mut p| p.pop());
        let Some(tx) = candidate_tx else {
            break;
        };
        match tx.send(job) {
            Ok(()) => {
                WORKER_POOL_REUSE_COUNT.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
            Err(sent_err) => {
                job = sent_err.0;
            }
        }
    }

    let idx = TOTAL_POOLED_WORKER_THREADS.fetch_add(1, Ordering::AcqRel) + 1;
    let (job_tx, job_rx) = mpsc::channel::<WorkerJob>();
    let _ = job_tx.send(job);
    let builder = std::thread::Builder::new()
        .name(format!("slice-worker-pool-{}", idx))
        .stack_size(8 * 1024 * 1024);
    builder
        .spawn(move || {
            worker_pool_thread_loop(job_tx, job_rx, None);
        })
        .map(|_| ())
        .map_err(|e| {
            TOTAL_POOLED_WORKER_THREADS.fetch_sub(1, Ordering::AcqRel);
            e.to_string()
        })
}

fn worker_pool_thread_loop(
    self_job_tx: Sender<WorkerJob>,
    job_rx: Receiver<WorkerJob>,
    ready_notify: Option<Sender<()>>,
) {
    // Eagerly warm this worker thread's `WarmBoaRealm` + Shards 2 & 8 before waiting on `job_rx`
    {
        let mut realm = crate::boa_realm::acquire_warm_boa_realm();
        crate::boa_realm::ensure_realm_shards(&mut realm, (1 << 8) | (1 << 2), true);
        crate::boa_realm::release_warm_boa_realm(realm);
    }
    if let Some(r_tx) = ready_notify {
        let _ = r_tx.send(());
    }

    while let Ok(job) = job_rx.recv() {
        run_pooled_worker_job(job, &self_job_tx);
    }
}

fn run_pooled_worker_job(job: WorkerJob, self_job_tx: &Sender<WorkerJob>) {
    let worker_id = job.worker_id;
    let event_tx = job.event_tx;
    let terminated_clone = job.terminated;

    // Fast abort if parent already called `worker.terminate()` before job started
    if terminated_clone.load(Ordering::Acquire) {
        GLOBAL_ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
        if let Ok(mut pool) = idle_worker_pool().lock() {
            if pool.len() < MAX_POOLED_WORKER_THREADS {
                pool.push(self_job_tx.clone());
            }
        }
        let _ = event_tx.send(WorkerOutboundEvent {
            worker_id,
            kind: WorkerOutboundEventKind::Exit {
                exit_code: 1,
                fuel_consumed: 0,
            },
        });
        return;
    }

    BOA_VFS_OWNER_TENANT.with(|o| {
        let mut cur = o.borrow_mut();
        cur.clear();
        cur.push_str(vfs_owner_namespace(&job.child_tenant));
    });
    BOA_VFS_STATE.with(|v| v.borrow_mut().restore_from_snapshot(job.parent_vfs_snap));
    BOA_FETCH_MOCKS.with(|m| *m.borrow_mut() = job.parent_fetch_mocks);
    BOA_WS_MOCKS.with(|m| *m.borrow_mut() = job.parent_ws_mocks);
    BOA_SOCKET_MOCKS.with(|m| *m.borrow_mut() = job.parent_sock_mocks);

    let tx_for_self = event_tx.clone();
    WORKER_SELF_ENV.with(|env| {
        *env.borrow_mut() = WorkerSelfThreadEnv {
            is_worker_thread: true,
            thread_id: worker_id,
            name: job.worker_name,
            worker_data_json: job.worker_data_json,
            inbound_rx: Some(job.inbound_rx),
            outbound_tx: Some(tx_for_self),
            terminated: Some(Arc::clone(&terminated_clone)),
            self_listening: false,
            explicit_exit_code: None,
        };
    });

    let _ = event_tx.send(WorkerOutboundEvent {
        worker_id,
        kind: WorkerOutboundEventKind::Online,
    });

    let mut child_isolate = SliceIsolate::spawn(
        job.child_tenant,
        100 + (worker_id % 16),
        job.child_caps,
        job.child_fuel,
        job.child_mem,
    );
    if let Some((prng, _clk)) = job.parent_det {
        child_isolate.set_deterministic_seed(Some(
            prng ^ (worker_id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15),
        ));
    }
    for sec in job.parent_egress_secrets {
        child_isolate.add_egress_audit_secret(sec);
    }

    let child_fuel_cap = job.child_fuel;
    let exec_res = child_isolate.execute_bytecode(&job.bytecode, None);
    let was_terminated = terminated_clone.load(Ordering::Acquire);
    let explicit_code = WORKER_SELF_ENV
        .with(|env| env.borrow().explicit_exit_code)
        .or_else(|| child_isolate.last_exit_code());
    let actual_fuel_consumed = match &exec_res {
        Ok(m) => m.fuel_consumed.min(child_fuel_cap).max(1),
        Err(SandboxViolation::CpuFuelExhausted { .. }) => child_fuel_cap,
        Err(_) => child_isolate
            .compartment
            .fuel_consumed
            .min(child_fuel_cap)
            .max(1),
    };

    // Scrub thread-local worker state and return this thread to `IDLE_WORKER_POOL`
    // BEFORE sending `Exit` to the parent isolate so sequential `await worker` loops
    // immediately reuse this warm thread without a race condition!
    WORKER_SELF_ENV.with(|env| *env.borrow_mut() = WorkerSelfThreadEnv::default());
    BOA_VFS_OWNER_TENANT.with(|o| o.borrow_mut().clear());
    BOA_VFS_STATE.with(|v| v.borrow_mut().clear());
    GLOBAL_ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
    if let Ok(mut pool) = idle_worker_pool().lock() {
        if pool.len() < MAX_POOLED_WORKER_THREADS {
            pool.push(self_job_tx.clone());
        }
    }

    if was_terminated {
        let _ = event_tx.send(WorkerOutboundEvent {
            worker_id,
            kind: WorkerOutboundEventKind::Exit {
                exit_code: explicit_code.unwrap_or(1),
                fuel_consumed: actual_fuel_consumed,
            },
        });
    } else {
        match exec_res {
            Ok(metrics) => {
                if metrics.return_value.starts_with("Uncaught ") {
                    let _ = event_tx.send(WorkerOutboundEvent {
                        worker_id,
                        kind: WorkerOutboundEventKind::Error {
                            message: metrics.return_value,
                            is_violation: false,
                        },
                    });
                    let _ = event_tx.send(WorkerOutboundEvent {
                        worker_id,
                        kind: WorkerOutboundEventKind::Exit {
                            exit_code: explicit_code.unwrap_or(1),
                            fuel_consumed: actual_fuel_consumed,
                        },
                    });
                } else {
                    let _ = event_tx.send(WorkerOutboundEvent {
                        worker_id,
                        kind: WorkerOutboundEventKind::Exit {
                            exit_code: explicit_code.unwrap_or(0),
                            fuel_consumed: actual_fuel_consumed,
                        },
                    });
                }
            }
            Err(violation) => {
                let _ = event_tx.send(WorkerOutboundEvent {
                    worker_id,
                    kind: WorkerOutboundEventKind::Error {
                        message: violation.to_string(),
                        is_violation: true,
                    },
                });
                let _ = event_tx.send(WorkerOutboundEvent {
                    worker_id,
                    kind: WorkerOutboundEventKind::Exit {
                        exit_code: explicit_code.unwrap_or(1),
                        fuel_consumed: actual_fuel_consumed,
                    },
                });
            }
        }
    }
}

/// Posts a JSON-serialized message from the parent isolate to `worker_id`.
pub fn worker_post_message_to_child(worker_id: u32, msg_json: &str) -> Result<bool, String> {
    charge_worker_message_budget(msg_json.len())?;
    PARENT_WORKERS.with(|reg| {
        let r = reg.borrow();
        let Some(handle) = r.workers.get(&worker_id) else {
            return Ok(false);
        };
        if handle.finished || handle.terminated.load(Ordering::Acquire) {
            return Ok(false);
        }
        Ok(handle
            .inbound_tx
            .send(WorkerInboundCmd::Message(msg_json.to_string()))
            .is_ok())
    })
}

/// Terminates `worker_id` from the parent isolate (`worker.terminate()`).
pub fn worker_terminate_child(worker_id: u32) -> bool {
    PARENT_WORKERS.with(|reg| {
        let mut r = reg.borrow_mut();
        let Some(handle) = r.workers.get_mut(&worker_id) else {
            return false;
        };
        handle.terminated.store(true, Ordering::Release);
        let _ = handle.inbound_tx.send(WorkerInboundCmd::Terminate);
        true
    })
}

/// Sets `ref` / `unref` status on `worker_id` in the parent isolate.
pub fn worker_set_child_ref(worker_id: u32, refed: bool) {
    PARENT_WORKERS.with(|reg| {
        let mut r = reg.borrow_mut();
        if let Some(handle) = r.workers.get_mut(&worker_id) {
            handle.refed = refed;
        }
    });
}

/// Posts a JSON-serialized message from inside a worker isolate back to its parent (`parentPort.postMessage` / `self.postMessage`).
pub fn worker_self_post_message(msg_json: &str) -> Result<bool, String> {
    charge_worker_message_budget(msg_json.len())?;
    WORKER_SELF_ENV.with(|env| {
        let e = env.borrow();
        if !e.is_worker_thread {
            return Ok(false);
        }
        let Some(ref tx) = e.outbound_tx else {
            return Ok(false);
        };
        Ok(tx
            .send(WorkerOutboundEvent {
                worker_id: e.thread_id,
                kind: WorkerOutboundEventKind::Message(msg_json.to_string()),
            })
            .is_ok())
    })
}

/// Returns metadata JSON (`{ isMainThread, threadId, name, workerDataJson }`) for the current thread.
pub fn worker_self_meta_json() -> String {
    WORKER_SELF_ENV.with(|env| {
        let e = env.borrow();
        serde_json::json!({
            "isMainThread": !e.is_worker_thread,
            "threadId": e.thread_id,
            "name": e.name,
            "workerDataJson": e.worker_data_json,
        })
        .to_string()
    })
}

/// Updates whether the current worker isolate is actively listening for incoming messages from `parentPort` / `self.onmessage`.
pub fn worker_self_set_listening(listening: bool, exit_code: Option<i32>) {
    WORKER_SELF_ENV.with(|env| {
        let mut e = env.borrow_mut();
        e.self_listening = listening;
        if let Some(code) = exit_code {
            e.explicit_exit_code = Some(code);
            if let Some(ref term) = e.terminated {
                term.store(true, Ordering::Release);
            }
        }
    });
}

/// Polls any pending messages sent from the parent isolate to this worker isolate, and dispatches them into JS.
/// Returns `true` if any message was dispatched.
pub(crate) fn poll_worker_self_inbox_and_dispatch(
    ctx: &mut Context,
    wait_timeout: Option<Duration>,
) -> bool {
    let is_worker = WORKER_SELF_ENV.with(|env| env.borrow().is_worker_thread);
    if !is_worker {
        return false;
    }

    let mut received_msgs: Vec<String> = Vec::new();
    let mut should_terminate = false;

    WORKER_SELF_ENV.with(|env| {
        let mut e = env.borrow_mut();
        if let Some(ref term) = e.terminated {
            if term.load(Ordering::Acquire) {
                should_terminate = true;
                return;
            }
        }
        let Some(ref rx) = e.inbound_rx else {
            return;
        };
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                WorkerInboundCmd::Message(m) => received_msgs.push(m),
                WorkerInboundCmd::Terminate => {
                    should_terminate = true;
                    break;
                }
            }
        }
        if !should_terminate && received_msgs.is_empty() && e.self_listening {
            if let Some(dur) = wait_timeout {
                match rx.recv_timeout(dur) {
                    Ok(WorkerInboundCmd::Message(m)) => received_msgs.push(m),
                    Ok(WorkerInboundCmd::Terminate) | Err(RecvTimeoutError::Disconnected) => {
                        should_terminate = true;
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                }
            }
        }
        if should_terminate {
            e.self_listening = false;
            if let Some(ref term) = e.terminated {
                term.store(true, Ordering::Release);
            }
        }
    });

    if should_terminate || received_msgs.is_empty() {
        return false;
    }

    if let Ok(dispatch_val) = ctx
        .global_object()
        .get(boa_engine::js_string!("__slice_worker_self_deliver"), ctx)
    {
        if let Some(dispatch_fn) = dispatch_val.as_object() {
            for msg_json in received_msgs {
                let arg = JsValue::from(boa_engine::js_string!(msg_json));
                let call_res = dispatch_fn.call(&JsValue::undefined(), &[arg], ctx);
                let _ = ctx.run_jobs();
                if crate::boa_realm::has_process_exit() {
                    return true;
                }
                if let Err(err) = call_res {
                    let msg = err.to_string();
                    if msg.starts_with("__SLICE_PROCESS_EXIT__:") {
                        return true;
                    }
                    if msg.contains("RuntimeLimitError")
                        || msg.contains("iteration loops")
                        || msg.contains("loop iteration limit")
                        || msg.contains("Maximum call stack")
                        || msg.contains("recursion limit")
                    {
                        let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
                        BOA_VIOLATION.with(|v| {
                            if v.borrow().is_none() {
                                *v.borrow_mut() = Some(SandboxViolation::CpuFuelExhausted {
                                    tenant_id,
                                    limit: 0,
                                });
                            }
                        });
                    } else if BOA_VIOLATION.with(|v| v.borrow().is_none()) {
                        WORKER_SELF_ENV.with(|env| {
                            let mut e = env.borrow_mut();
                            e.self_listening = false;
                            e.explicit_exit_code = Some(1);
                            if let Some(ref tx) = e.outbound_tx {
                                let _ = tx.send(WorkerOutboundEvent {
                                    worker_id: e.thread_id,
                                    kind: WorkerOutboundEventKind::Error {
                                        message: format!("Uncaught {}", msg),
                                        is_violation: false,
                                    },
                                });
                            }
                        });
                        return true;
                    }
                }
                if BOA_VIOLATION.with(|v| v.borrow().is_some()) {
                    WORKER_SELF_ENV.with(|env| {
                        let mut e = env.borrow_mut();
                        e.self_listening = false;
                        e.explicit_exit_code = Some(1);
                    });
                    return true;
                }
            }
            return true;
        }
    }
    false
}

/// Polls any outbound events (`Online`, `Message`, `Error`, `Exit`) from child workers on the parent thread
/// and dispatches them into JS. Returns `true` if at least one event was processed.
pub(crate) fn poll_parent_worker_events_and_dispatch(
    ctx: &mut Context,
    wait_timeout: Option<Duration>,
) -> bool {
    if WORKER_ACTIVE_COUNT.with(|c| c.get() == 0) {
        return false;
    }

    let mut events: Vec<WorkerOutboundEvent> = Vec::new();
    let mut reconciled_any = false;
    PARENT_WORKERS.with(|reg| {
        let mut r = reg.borrow_mut();
        while let Ok(ev) = r.event_rx.try_recv() {
            if let WorkerOutboundEventKind::Exit { fuel_consumed, .. } = ev.kind {
                if let Some(h) = r.workers.get_mut(&ev.worker_id) {
                    h.finished = true;
                    h.reconcile_resources(fuel_consumed);
                    reconciled_any = true;
                }
            }
            events.push(ev);
        }
        if events.is_empty() && r.has_active_refed_workers() {
            if let Some(dur) = wait_timeout {
                if let Ok(ev) = r.event_rx.recv_timeout(dur) {
                    if let WorkerOutboundEventKind::Exit { fuel_consumed, .. } = ev.kind {
                        if let Some(h) = r.workers.get_mut(&ev.worker_id) {
                            h.finished = true;
                            h.reconcile_resources(fuel_consumed);
                            reconciled_any = true;
                        }
                    }
                    events.push(ev);
                    while let Ok(ev2) = r.event_rx.try_recv() {
                        if let WorkerOutboundEventKind::Exit { fuel_consumed, .. } = ev2.kind {
                            if let Some(h) = r.workers.get_mut(&ev2.worker_id) {
                                h.finished = true;
                                h.reconcile_resources(fuel_consumed);
                                reconciled_any = true;
                            }
                        }
                        events.push(ev2);
                    }
                }
            }
        }
        r.workers.retain(|_, w| !w.finished);
        WORKER_ACTIVE_COUNT.with(|c| c.set(r.workers.len()));
    });

    if reconciled_any && BOA_VIOLATION.with(|v| v.borrow().is_none()) {
        let (consumed, limit) = BOA_FUEL_BUDGET.with(|f| f.get());
        let cur_iters = ctx.runtime_limits().loop_iterations_executed();
        let next_cap = limit
            .saturating_sub(consumed)
            .max(cur_iters.saturating_add(1));
        ctx.runtime_limits_mut().set_loop_iteration_limit(next_cap);
    }

    if events.is_empty() {
        return false;
    }

    if let Ok(dispatch_val) = ctx
        .global_object()
        .get(boa_engine::js_string!("__slice_worker_host_deliver"), ctx)
    {
        if let Some(dispatch_fn) = dispatch_val.as_object() {
            for ev in events {
                let (ev_type, payload_str, code_num) = match ev.kind {
                    WorkerOutboundEventKind::Online => ("online", String::new(), 0.0),
                    WorkerOutboundEventKind::Message(m) => ("message", m, 0.0),
                    WorkerOutboundEventKind::Error {
                        message,
                        is_violation,
                    } => (
                        "error",
                        if is_violation && !message.starts_with("SandboxViolation") {
                            format!("SandboxViolation: {}", message)
                        } else {
                            message
                        },
                        1.0,
                    ),
                    WorkerOutboundEventKind::Exit { exit_code, .. } => {
                        ("exit", String::new(), exit_code as f64)
                    }
                };
                let args = [
                    JsValue::from(ev.worker_id as f64),
                    JsValue::from(boa_engine::js_string!(ev_type)),
                    JsValue::from(boa_engine::js_string!(payload_str)),
                    JsValue::from(code_num),
                ];
                let _ = dispatch_fn.call(&JsValue::undefined(), &args, ctx);
                let _ = ctx.run_jobs();
            }
            return true;
        }
    }
    let _ = Source::from_bytes(b"");
    false
}
