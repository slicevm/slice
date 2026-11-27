//! Bounded Concurrent Outbound `fetch()` Reactor Pool (`FetchReactorPool`) & Promise Settlement.

use super::timers_and_state::{
    ACTIVE_ASYNC_ITEMS, CompletedAsyncFetch, EVENT_LOOP_STATE, InFlightFetchHandle,
    QueuedAsyncFetch,
};
use crate::boa_realm::{
    BOA_ACTIVE_TENANT, BOA_ALLOW_PRIVATE_NETS, BOA_ALS_ACTIVE, BOA_ALS_SNAP,
    BOA_DENIED_CAPABILITIES, BOA_DET_STATE, BOA_EGRESS_SECRETS, BOA_KV_MEM_BUDGET,
    BOA_LAST_OCAP_VIOLATION, BOA_NET_HOSTS, BOA_VIOLATION, DIRTY_EVENT_LOOP, mark_subsystem_dirty,
};
use crate::outbound_http::{
    execute_outbound_fetch_with_redirects, outbound_pool_reuse_count, reset_fetch_slab_telemetry,
};
use crate::state_and_kv::edge_kv_set;
use boa_engine::{Context, JsObject, JsValue, js_string};
use rustc_hash::FxHashSet;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};

/// Maximum number of reusable OS threads in the shared `FetchReactorPool`.
pub const MAX_FETCH_REACTOR_WORKERS: usize = 16;

pub(super) struct FetchReactorJob {
    pub(super) id: u32,
    pub(super) url: String,
    pub(super) method: String,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
    pub(super) redirect_mode: String,
    pub(super) tenant_id: String,
    pub(super) net_hosts: FxHashSet<String>,
    pub(super) allow_private_networks: bool,
    pub(super) egress_secrets: Vec<String>,
    pub(super) mem_budget: (usize, usize),
    pub(super) tx: Sender<CompletedAsyncFetch>,
}

pub(super) struct FetchReactorPool {
    state: parking_lot::Mutex<(VecDeque<FetchReactorJob>, usize)>,
    cv: parking_lot::Condvar,
}

static FETCH_REACTOR_POOL: OnceLock<Arc<FetchReactorPool>> = OnceLock::new();
static FETCH_REACTOR_REUSE_COUNT: AtomicUsize = AtomicUsize::new(0);
static FETCH_REACTOR_ACTIVE_THREADS: AtomicUsize = AtomicUsize::new(0);

impl FetchReactorPool {
    pub(super) fn global() -> &'static Arc<Self> {
        FETCH_REACTOR_POOL.get_or_init(|| {
            Arc::new(Self {
                state: parking_lot::Mutex::new((VecDeque::with_capacity(64), 0)),
                cv: parking_lot::Condvar::new(),
            })
        })
    }

    pub(super) fn submit(self: &Arc<Self>, job: FetchReactorJob) {
        let mut need_spawn = false;
        let mut worker_idx = 0usize;
        {
            let mut guard = self.state.lock();
            guard.0.push_back(job);
            let idle = guard.1;
            let active = FETCH_REACTOR_ACTIVE_THREADS.load(Ordering::Acquire);
            if idle == 0 && active < MAX_FETCH_REACTOR_WORKERS {
                worker_idx = FETCH_REACTOR_ACTIVE_THREADS.fetch_add(1, Ordering::AcqRel);
                if worker_idx < MAX_FETCH_REACTOR_WORKERS {
                    need_spawn = true;
                } else {
                    FETCH_REACTOR_ACTIVE_THREADS.fetch_sub(1, Ordering::AcqRel);
                    FETCH_REACTOR_REUSE_COUNT.fetch_add(1, Ordering::Relaxed);
                }
            } else {
                FETCH_REACTOR_REUSE_COUNT.fetch_add(1, Ordering::Relaxed);
            }
        }
        if need_spawn {
            let pool_clone = Arc::clone(self);
            let _ = std::thread::Builder::new()
                .name(format!("slice-fetch-reactor-{}", worker_idx))
                .stack_size(2 * 1024 * 1024)
                .spawn(move || {
                    pool_clone.worker_loop();
                });
        }
        self.cv.notify_one();
    }

    fn worker_loop(&self) {
        loop {
            let job = {
                let mut guard = self.state.lock();
                while guard.0.is_empty() {
                    guard.1 += 1;
                    self.cv.wait(&mut guard);
                    guard.1 = guard.1.saturating_sub(1);
                }
                guard.0.pop_front()
            };
            if let Some(job) = job {
                BOA_ACTIVE_TENANT.with(|t| *t.borrow_mut() = job.tenant_id);
                BOA_NET_HOSTS.with(|h| *h.borrow_mut() = job.net_hosts);
                BOA_ALLOW_PRIVATE_NETS.with(|p| p.set(job.allow_private_networks));
                BOA_EGRESS_SECRETS.with(|s| *s.borrow_mut() = job.egress_secrets);
                BOA_KV_MEM_BUDGET.with(|m| m.set(job.mem_budget));
                BOA_VIOLATION.with(|v| *v.borrow_mut() = None);
                BOA_LAST_OCAP_VIOLATION.with(|v| *v.borrow_mut() = None);
                BOA_DENIED_CAPABILITIES.with(|d| d.borrow_mut().clear());
                let reuse_before = outbound_pool_reuse_count();

                let result = execute_outbound_fetch_with_redirects(
                    &job.url,
                    &job.method,
                    &job.headers,
                    &job.body,
                    &job.redirect_mode,
                );
                let violation = BOA_VIOLATION.with(|v| v.borrow_mut().take());
                let ocap_denial = BOA_LAST_OCAP_VIOLATION.with(|v| v.borrow_mut().take());
                BOA_DENIED_CAPABILITIES.with(|d| d.borrow_mut().clear());
                let reused_conns = outbound_pool_reuse_count().saturating_sub(reuse_before);

                let _ = job.tx.send(CompletedAsyncFetch {
                    id: job.id,
                    result,
                    violation,
                    ocap_denial,
                    reused_conns,
                });
            }
        }
    }
}

/// Returns the number of times a concurrent `fetch()` reused an existing worker thread in
/// `FetchReactorPool` instead of spawning a new OS thread.
pub fn fetch_reactor_pool_reuse_count() -> u64 {
    FETCH_REACTOR_REUSE_COUNT.load(Ordering::Relaxed) as u64
}

/// Returns the number of active OS threads in `FetchReactorPool` (bounded by `MAX_FETCH_REACTOR_WORKERS = 16`).
pub fn fetch_reactor_active_threads() -> usize {
    FETCH_REACTOR_ACTIVE_THREADS.load(Ordering::Acquire)
}

/// Resets both the `FetchReactorPool` thread reuse counter and `slice-io` slab telemetry.
pub fn reset_fetch_reactor_telemetry() {
    FETCH_REACTOR_REUSE_COUNT.store(0, Ordering::Relaxed);
    reset_fetch_slab_telemetry();
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn enqueue_async_fetch(
    url: String,
    method: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    redirect_mode: String,
    resolve_fn: JsObject,
    reject_fn: JsObject,
    signal_obj: Option<JsObject>,
) -> u32 {
    EVENT_LOOP_STATE.with(|st| {
        let mut loop_st = st.borrow_mut();
        let id = loop_st.next_fetch_id;
        loop_st.next_fetch_id = loop_st.next_fetch_id.wrapping_add(1).max(1);
        let als_snap = if BOA_ALS_ACTIVE.with(|a| a.get()) {
            Some(BOA_ALS_SNAP.with(|s| s.borrow().clone()))
        } else {
            None
        };
        loop_st.queued_fetches.push(QueuedAsyncFetch {
            id,
            url,
            method,
            headers,
            body,
            redirect_mode,
            resolve_fn,
            reject_fn,
            signal_obj,
            als_snap,
        });
        mark_subsystem_dirty(DIRTY_EVENT_LOOP);
        ACTIVE_ASYNC_ITEMS.with(|c| c.set(c.get().saturating_add(1)));
        id
    })
}

#[inline]
pub(super) fn is_signal_aborted(
    signal_opt: &Option<JsObject>,
    ctx: &mut Context,
) -> Option<JsValue> {
    let sig = signal_opt.as_ref()?;
    let aborted = sig
        .get(js_string!("aborted"), ctx)
        .ok()
        .and_then(|v| v.as_boolean())
        .unwrap_or(false);
    if aborted {
        let reason = sig
            .get(js_string!("reason"), ctx)
            .ok()
            .filter(|v| !v.is_undefined())
            .unwrap_or_else(|| JsValue::from(js_string!("AbortError: The operation was aborted")));
        Some(reason)
    } else {
        None
    }
}

pub(super) fn settle_completed_fetch(
    handle: InFlightFetchHandle,
    result: Result<String, String>,
    ctx: &mut Context,
) {
    let prev_als = handle
        .als_snap
        .as_ref()
        .map(|snap| BOA_ALS_SNAP.with(|s| s.replace(snap.clone())));
    if let Some(abort_reason) = is_signal_aborted(&handle.signal_obj, ctx) {
        let _ = handle
            .reject_fn
            .call(&JsValue::undefined(), &[abort_reason], ctx);
        if let Some(prev) = prev_als {
            BOA_ALS_SNAP.with(|s| *s.borrow_mut() = prev);
        }
        return;
    }
    match result {
        Ok(resp_json) => {
            if BOA_DET_STATE.with(|s| s.get().is_some()) {
                let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
                let cassette_key = format!("fetch_cassette:{}:{}", handle.method, handle.url);
                edge_kv_set(&tenant_id, &cassette_key, &resp_json, 0);
            }
            let arg = JsValue::from(js_string!(resp_json));
            let _ = handle.resolve_fn.call(&JsValue::undefined(), &[arg], ctx);
        }
        Err(err_msg) => {
            let err_json = serde_json::json!({ "__error": err_msg }).to_string();
            let arg = JsValue::from(js_string!(err_json));
            let _ = handle.resolve_fn.call(&JsValue::undefined(), &[arg], ctx);
        }
    }
    if let Some(prev) = prev_als {
        BOA_ALS_SNAP.with(|s| *s.borrow_mut() = prev);
    }
}
