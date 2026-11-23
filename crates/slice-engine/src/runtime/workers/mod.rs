//! Zero-Stat MicroVM Web Workers (`new Worker()`) & Node.js `node:worker_threads` Engine.
//!
//! Implements capability-attenuated multi-threaded background isolate execution:
//! - Sub-millisecond warm isolate checkout on background worker threads (`8 MB` stack size)
//! - Strict Capability Attenuation (`build_attenuated_worker_caps`): child workers can only
//!   receive a subset (`parent & requested`) of the parent isolate's `allow_read`, `allow_write`,
//!   and `allow_net` permissions, and always inherit `BOA_EGRESS_SECRETS` DLP rules
//! - Bounded CPU instruction fuel (`fuel`) and memory ceiling (`memoryBytes`) per worker
//! - Lock-free `mpsc` bidirectional message channels integrated directly into `event_loop`

mod messaging;
mod pool;
mod spawn;

pub(crate) use messaging::{
    poll_parent_worker_events_and_dispatch, poll_worker_self_inbox_and_dispatch,
};
pub use messaging::{
    worker_post_message_to_child, worker_self_meta_json, worker_self_post_message,
    worker_self_set_listening, worker_set_child_ref, worker_terminate_child,
};
#[cfg(test)]
pub(crate) use pool::WorkerTurnUnwindGuard;
pub use pool::{prewarm_worker_pool_sync, speculative_prewarm_worker_pool};
pub use spawn::worker_spawn;
pub(crate) use spawn::worker_spawn_with_context;

use crate::bindings::fs::VfsSnapshot;
use crate::boa_realm::{BOA_ACTIVE_TENANT, BOA_FUEL_BUDGET, BOA_KV_MEM_BUDGET, BOA_VIOLATION};
use rustc_hash::FxHashMap;
use serde::Deserialize;
use slice_sandbox::{CapabilityToken, SandboxViolation};
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};

/// Maximum concurrent active workers per parent isolate.
pub const MAX_WORKERS_PER_ISOLATE: usize = 16;

/// Maximum concurrent active workers globally across the host process.
pub const MAX_GLOBAL_ACTIVE_WORKERS: usize = 32;

/// Maximum serialized message payload size (`4 MB`) for `postMessage`.
pub const MAX_WORKER_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

/// Maximum persistent warm worker OS threads retained in `IDLE_WORKER_POOL`.
pub const MAX_POOLED_WORKER_THREADS: usize = 8;

/// Maximum cached transpiled worker script bytecode payloads in `WORKER_SCRIPT_CACHE`.
pub(super) const MAX_CACHED_WORKER_SCRIPTS: usize = 256;

pub(super) static GLOBAL_ACTIVE_WORKERS: AtomicUsize = AtomicUsize::new(0);
pub(super) static TOTAL_POOLED_WORKER_THREADS: AtomicUsize = AtomicUsize::new(0);
pub(super) static WORKER_POOL_REUSE_COUNT: AtomicUsize = AtomicUsize::new(0);
pub(super) static WORKER_SCRIPT_CACHE_HITS: AtomicUsize = AtomicUsize::new(0);
static WORKER_SCRIPT_CACHE: OnceLock<Mutex<FxHashMap<u64, Arc<Vec<u8>>>>> = OnceLock::new();
static IDLE_WORKER_POOL: OnceLock<Mutex<Vec<Sender<WorkerJob>>>> = OnceLock::new();

pub(super) struct WorkerJob {
    pub(super) worker_id: u32,
    pub(super) worker_name: String,
    pub(super) child_tenant: String,
    pub(super) worker_data_json: String,
    pub(super) child_caps: CapabilityToken,
    pub(super) child_fuel: u64,
    pub(super) child_mem: usize,
    pub(super) parent_det: Option<(u64, u64)>,
    pub(super) parent_egress_secrets: Vec<String>,
    pub(super) parent_vfs_snap: VfsSnapshot,
    pub(super) parent_fetch_mocks: FxHashMap<String, String>,
    pub(super) parent_ws_mocks: FxHashMap<String, String>,
    pub(super) parent_sock_mocks: FxHashMap<String, String>,
    pub(super) inbound_rx: Receiver<WorkerInboundCmd>,
    pub(super) event_tx: Sender<WorkerOutboundEvent>,
    pub(super) terminated: Arc<AtomicBool>,
    pub(super) bytecode: Arc<Vec<u8>>,
}

pub(super) fn worker_script_cache() -> &'static Mutex<FxHashMap<u64, Arc<Vec<u8>>>> {
    WORKER_SCRIPT_CACHE.get_or_init(|| Mutex::new(FxHashMap::default()))
}

pub(super) fn idle_worker_pool() -> &'static Mutex<Vec<Sender<WorkerJob>>> {
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
    pub allow_env: Option<Vec<String>>,
    pub checkpoints: Option<bool>,
    pub max_checkpoints: Option<u32>,
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

pub(super) struct ParentWorkerHandle {
    pub(super) inbound_tx: Sender<WorkerInboundCmd>,
    pub(super) terminated: Arc<AtomicBool>,
    pub(super) finished: bool,
    pub(super) refed: bool,
    pub(super) reserved_fuel: u64,
    pub(super) reserved_mem: usize,
    pub(super) reconciled: bool,
}

impl ParentWorkerHandle {
    #[inline]
    pub(super) fn reconcile_resources(&mut self, actual_fuel_consumed: u64) {
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

pub(super) struct ParentWorkerRegistry {
    pub(super) next_worker_id: u32,
    pub(super) workers: FxHashMap<u32, ParentWorkerHandle>,
    pub(super) event_tx: Sender<WorkerOutboundEvent>,
    pub(super) event_rx: Receiver<WorkerOutboundEvent>,
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
    pub(super) fn has_active_refed_workers(&self) -> bool {
        self.workers.values().any(|w| !w.finished && w.refed)
    }
}

pub(super) struct WorkerSelfThreadEnv {
    pub(super) is_worker_thread: bool,
    pub(super) thread_id: u32,
    pub(super) name: String,
    pub(super) worker_data_json: String,
    pub(super) inbound_rx: Option<Receiver<WorkerInboundCmd>>,
    pub(super) outbound_tx: Option<Sender<WorkerOutboundEvent>>,
    pub(super) terminated: Option<Arc<AtomicBool>>,
    pub(super) self_listening: bool,
    pub(super) explicit_exit_code: Option<i32>,
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
    pub(super) static PARENT_WORKERS: RefCell<ParentWorkerRegistry> = RefCell::new(ParentWorkerRegistry::new());
    pub(super) static WORKER_SELF_ENV: RefCell<WorkerSelfThreadEnv> = RefCell::new(WorkerSelfThreadEnv::default());
    pub(super) static WORKER_ACTIVE_COUNT: Cell<usize> = const { Cell::new(0) };
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
        e.terminated
            .as_ref()
            .is_none_or(|term| !term.load(Ordering::Acquire))
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
            .is_some_and(|t| t.load(Ordering::Acquire))
    })
}

pub(super) fn charge_worker_message_budget(bytes: usize) -> Result<(), String> {
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

#[cfg(test)]
pub(crate) fn global_active_workers_for_test() -> usize {
    GLOBAL_ACTIVE_WORKERS.load(Ordering::Acquire)
}

#[cfg(test)]
pub(crate) fn inc_global_active_workers_for_test() {
    GLOBAL_ACTIVE_WORKERS.fetch_add(1, Ordering::AcqRel);
}
