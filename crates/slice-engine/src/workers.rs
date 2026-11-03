//! Zero-Stat MicroVM Web Workers (`new Worker()`) & Node.js `node:worker_threads` Engine.
//!
//! Implements capability-attenuated multi-threaded background isolate execution:
//! - Sub-millisecond warm isolate checkout on background worker threads (`16 MB` stack size)
//! - Strict Capability Attenuation (`attenuate_worker_capabilities`): child workers can only
//!   receive a subset (`parent ∩ requested`) of the parent isolate's `allow_read`, `allow_write`,
//!   and `allow_net` permissions, and always inherit `BOA_EGRESS_SECRETS` DLP rules
//! - Bounded CPU instruction fuel (`fuel`) and memory ceiling (`memoryBytes`) per worker
//! - Lock-free `mpsc` bidirectional message channels integrated directly into `event_loop.rs`

use crate::boa_realm::{
    BOA_ACTIVE_CAPS, BOA_ACTIVE_TENANT, BOA_DET_STATE, BOA_EGRESS_SECRETS, BOA_ENV_MAP,
    BOA_FETCH_MOCKS, BOA_KV_MEM_BUDGET, BOA_NET_HOSTS, BOA_VIOLATION, BOA_VFS_CACHE,
    BOA_VFS_DIRS, check_boa_fs_read_ocap, norm_vfs_path,
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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

/// Maximum concurrent active workers per parent isolate.
pub const MAX_WORKERS_PER_ISOLATE: usize = 16;

/// Maximum concurrent active workers globally across the host process.
pub const MAX_GLOBAL_ACTIVE_WORKERS: usize = 32;

/// Maximum serialized message payload size (`4 MB`) for `postMessage`.
pub const MAX_WORKER_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

static GLOBAL_ACTIVE_WORKERS: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerSliceAttenuationOpts {
    pub fuel: Option<u64>,
    pub memory_bytes: Option<usize>,
    pub allow_read: Option<Vec<String>>,
    pub allow_write: Option<Vec<String>>,
    pub allow_net: Option<Vec<String>>,
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
    Exit { exit_code: i32 },
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
        for (_, h) in self.workers.drain() {
            h.terminated.store(true, Ordering::Release);
            let _ = h.inbound_tx.send(WorkerInboundCmd::Terminate);
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
    let norm = norm_vfs_path(raw_path);
    if !check_boa_fs_read_ocap(&norm) {
        return Err(format!(
            "SandboxViolation: Worker script read '{}' denied by OCap read policy",
            norm
        ));
    }

    // 1. Check in-memory VFS cache (`BOA_VFS_CACHE`) first (0 disk stat syscalls)
    if let Some(vfs_src) = BOA_VFS_CACHE.with(|c| {
        let map = c.borrow();
        map.get(&norm)
            .or_else(|| map.get(raw_path))
            .or_else(|| {
                let stripped = norm.strip_prefix("./").unwrap_or(&norm);
                map.get(stripped)
            })
            .cloned()
    }) {
        return Ok(vfs_src);
    }

    // 2. Fallback to host filesystem if OCap permits
    std::fs::read_to_string(raw_path)
        .map_err(|e| format!("WorkerError: Cannot read worker module '{}': {}", raw_path, e))
}

fn build_attenuated_worker_caps(
    attenuation: Option<&WorkerSliceAttenuationOpts>,
    parent_caps_opt: &Option<(String, Vec<String>, Vec<String>)>,
    parent_net_hosts: &FxHashSet<String>,
    parent_env_map: &FxHashMap<String, String>,
) -> CapabilityToken {
    let mut child_caps = CapabilityToken::pure_compute();
    let parent_reads: &[String] = parent_caps_opt
        .as_ref()
        .map(|(_, r, _)| r.as_slice())
        .unwrap_or(&[]);
    let parent_writes: &[String] = parent_caps_opt
        .as_ref()
        .map(|(_, _, w)| w.as_slice())
        .unwrap_or(&[]);

    // 1. Attenuate read prefixes
    if let Some(req_reads) = attenuation.and_then(|a| a.allow_read.as_ref()) {
        if req_reads.is_empty() {
            child_caps = child_caps.with_read_prefix("/__slice_worker_denied_read__");
        } else {
            for req in req_reads {
                let norm_req = norm_vfs_path(req);
                if parent_reads.is_empty()
                    || parent_reads.iter().any(|p| norm_req.starts_with(p))
                {
                    child_caps = child_caps.with_read_prefix(norm_req);
                }
            }
            if child_caps.allowed_read_prefixes.is_empty() {
                child_caps = child_caps.with_read_prefix("/__slice_worker_denied_read__");
            }
        }
    } else if !parent_reads.is_empty() {
        for p in parent_reads {
            child_caps = child_caps.with_read_prefix(p.clone());
        }
    }

    // 2. Attenuate write prefixes
    if let Some(req_writes) = attenuation.and_then(|a| a.allow_write.as_ref()) {
        if req_writes.is_empty() {
            child_caps = child_caps.with_write_prefix("/__slice_worker_denied_write__");
        } else {
            for req in req_writes {
                let norm_req = norm_vfs_path(req);
                if parent_writes.is_empty()
                    || parent_writes.iter().any(|p| norm_req.starts_with(p))
                {
                    child_caps = child_caps.with_write_prefix(norm_req);
                }
            }
            if child_caps.allowed_write_prefixes.is_empty() {
                child_caps = child_caps.with_write_prefix("/__slice_worker_denied_write__");
            }
        }
    } else if !parent_writes.is_empty() {
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

    child_caps
}

/// Spawns a capability-attenuated MicroVM Worker thread and returns its `worker_id`.
pub fn worker_spawn(specifier_or_code: &str, options_json: &str) -> Result<u32, String> {
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
    let parent_caps = BOA_ACTIVE_CAPS.with(|c| c.borrow().clone());
    let parent_net_hosts = BOA_NET_HOSTS.with(|h| h.borrow().clone());
    let parent_env_map = BOA_ENV_MAP.with(|m| m.borrow().clone());
    let parent_egress_secrets = BOA_EGRESS_SECRETS.with(|s| s.borrow().clone());
    let parent_vfs_cache = BOA_VFS_CACHE.with(|c| c.borrow().clone());
    let parent_vfs_dirs = BOA_VFS_DIRS.with(|d| d.borrow().clone());
    let parent_fetch_mocks = BOA_FETCH_MOCKS.with(|m| m.borrow().clone());
    let parent_ws_mocks = BOA_WS_MOCKS.with(|m| m.borrow().clone());
    let parent_sock_mocks = BOA_SOCKET_MOCKS.with(|m| m.borrow().clone());
    let parent_det = BOA_DET_STATE.with(|s| s.get());
    let (used_mem, limit_mem) = BOA_KV_MEM_BUDGET.with(|b| b.get());
    let parent_rem_mem = limit_mem.saturating_sub(used_mem).max(65_536);

    let child_caps = build_attenuated_worker_caps(
        opts.slice.as_ref(),
        &parent_caps,
        &parent_net_hosts,
        &parent_env_map,
    );
    let child_fuel = opts
        .slice
        .as_ref()
        .and_then(|s| s.fuel)
        .unwrap_or(5_000_000)
        .clamp(100, 50_000_000);
    let child_mem = opts
        .slice
        .as_ref()
        .and_then(|s| s.memory_bytes)
        .unwrap_or(parent_rem_mem)
        .min(parent_rem_mem)
        .max(65_536);

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
            },
        );
        (id, tx)
    });
    WORKER_ACTIVE_COUNT.with(|c| c.set(c.get().saturating_add(1)));

    let worker_name = opts.name.unwrap_or_else(|| format!("worker-{}", worker_id));
    let child_tenant = format!("{}:worker-{}", parent_tenant, worker_id);

    // Pre-compile bytecode before spawning thread (ensures TypeScript/ESM/JSX stripping happens once)
    let transpiled_js = slice_cas::strip_typescript_and_normalize(&raw_script, true);
    let mut bytecode = Vec::with_capacity(12 + transpiled_js.len());
    bytecode.extend_from_slice(b"SLICEBOA{\n");
    bytecode.extend_from_slice(transpiled_js.as_bytes());
    bytecode.extend_from_slice(b"\n}");

    let builder = std::thread::Builder::new()
        .name(format!("slice-worker-{}", worker_id))
        .stack_size(8 * 1024 * 1024);

    let spawn_res = builder.spawn(move || {
        // Initialize worker thread-local state
        BOA_VFS_CACHE.with(|c| *c.borrow_mut() = parent_vfs_cache);
        BOA_VFS_DIRS.with(|d| *d.borrow_mut() = parent_vfs_dirs);
        BOA_FETCH_MOCKS.with(|m| *m.borrow_mut() = parent_fetch_mocks);
        BOA_WS_MOCKS.with(|m| *m.borrow_mut() = parent_ws_mocks);
        BOA_SOCKET_MOCKS.with(|m| *m.borrow_mut() = parent_sock_mocks);

        let tx_for_self = event_tx.clone();
        WORKER_SELF_ENV.with(|env| {
            *env.borrow_mut() = WorkerSelfThreadEnv {
                is_worker_thread: true,
                thread_id: worker_id,
                name: worker_name,
                worker_data_json,
                inbound_rx: Some(inbound_rx),
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
            child_tenant,
            100 + (worker_id % 16),
            child_caps,
            child_fuel,
            child_mem,
        );
        if let Some((prng, _clk)) = parent_det {
            child_isolate.set_deterministic_seed(Some(
                prng ^ (worker_id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15),
            ));
        }
        for sec in parent_egress_secrets {
            child_isolate.add_egress_audit_secret(sec);
        }

        let exec_res = child_isolate.execute_bytecode(&bytecode, None);
        let was_terminated = terminated_clone.load(Ordering::Acquire);
        let explicit_code = WORKER_SELF_ENV.with(|env| env.borrow().explicit_exit_code);

        if was_terminated {
            let _ = event_tx.send(WorkerOutboundEvent {
                worker_id,
                kind: WorkerOutboundEventKind::Exit {
                    exit_code: explicit_code.unwrap_or(1),
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
                            },
                        });
                    } else {
                        let _ = event_tx.send(WorkerOutboundEvent {
                            worker_id,
                            kind: WorkerOutboundEventKind::Exit {
                                exit_code: explicit_code.unwrap_or(0),
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
                        },
                    });
                }
            }
        }

        WORKER_SELF_ENV.with(|env| *env.borrow_mut() = WorkerSelfThreadEnv::default());
        GLOBAL_ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
    });

    if let Err(e) = spawn_res {
        GLOBAL_ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
        PARENT_WORKERS.with(|reg| {
            reg.borrow_mut().workers.remove(&worker_id);
        });
        return Err(format!("WorkerSpawnError: {}", e));
    }

    Ok(worker_id)
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
pub(crate) fn poll_worker_self_inbox_and_dispatch(ctx: &mut Context, wait_timeout: Option<Duration>) -> bool {
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
                if let Err(err) = call_res {
                    let msg = err.to_string();
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
    PARENT_WORKERS.with(|reg| {
        let mut r = reg.borrow_mut();
        while let Ok(ev) = r.event_rx.try_recv() {
            if matches!(ev.kind, WorkerOutboundEventKind::Exit { .. }) {
                if let Some(h) = r.workers.get_mut(&ev.worker_id) {
                    h.finished = true;
                }
            }
            events.push(ev);
        }
        if events.is_empty() && r.has_active_refed_workers() {
            if let Some(dur) = wait_timeout {
                if let Ok(ev) = r.event_rx.recv_timeout(dur) {
                    if matches!(ev.kind, WorkerOutboundEventKind::Exit { .. }) {
                        if let Some(h) = r.workers.get_mut(&ev.worker_id) {
                            h.finished = true;
                        }
                    }
                    events.push(ev);
                }
            }
        }
        r.workers.retain(|_, w| !w.finished);
        WORKER_ACTIVE_COUNT.with(|c| c.set(r.workers.len()));
    });

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
                    WorkerOutboundEventKind::Exit { exit_code } => {
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
