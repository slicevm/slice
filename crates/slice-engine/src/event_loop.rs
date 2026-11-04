//! Phase N-2: Deterministic Time-Travel Event Loop, Slab Timer Wheel (`setTimeout`, `setInterval`),
//! Microtask Job Queue (`queueMicrotask`), `AbortSignal` Timeout Coordinator, and Concurrent
//! Outbound `fetch()` Multiplexer (`Promise.all([fetch(...), ...])`).

use crate::boa_realm::{
    BOA_ACTIVE_TENANT, BOA_ALS_ACTIVE, BOA_ALS_SNAP, BOA_DET_STATE, BOA_EGRESS_SECRETS,
    BOA_NET_HOSTS, BOA_VIOLATION,
};
use crate::outbound_http::{
    execute_outbound_fetch_with_redirects, outbound_pool_reuse_count, record_outbound_pool_reuses,
};
use crate::state_and_kv::edge_kv_set;
use boa_engine::{js_string, Context, JsObject, JsValue};
use rustc_hash::{FxHashMap, FxHashSet};
use slice_sandbox::SandboxViolation;
use std::cell::{Cell, RefCell};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const MAX_ACTIVE_TIMERS: usize = 1024;
const MAX_EVENT_LOOP_TICKS: usize = 4096;
const MAX_CONCURRENT_FETCH_WAIT: Duration = Duration::from_secs(10);

static PROCESS_EPOCH: OnceLock<Instant> = OnceLock::new();

#[inline]
pub(crate) fn monotonic_wall_clock_ms() -> u64 {
    PROCESS_EPOCH
        .get_or_init(Instant::now)
        .elapsed()
        .as_millis() as u64
}

#[inline]
pub(crate) fn current_event_loop_clock_ms() -> u64 {
    BOA_DET_STATE.with(|s| match s.get() {
        Some((_prng, clk)) => clk,
        None => monotonic_wall_clock_ms(),
    })
}

pub(crate) struct ScheduledTimer {
    pub(crate) id: u32,
    pub(crate) deadline_ms: u64,
    pub(crate) interval_ms: Option<u64>,
    pub(crate) is_immediate: bool,
    pub(crate) callback: JsObject,
    pub(crate) args: Vec<JsValue>,
    pub(crate) als_snap: Option<JsValue>,
    pub(crate) cancelled: bool,
}

pub(crate) struct QueuedAsyncFetch {
    pub(crate) id: u32,
    pub(crate) url: String,
    pub(crate) method: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: String,
    pub(crate) redirect_mode: String,
    pub(crate) resolve_fn: JsObject,
    pub(crate) reject_fn: JsObject,
    pub(crate) signal_obj: Option<JsObject>,
    pub(crate) als_snap: Option<JsValue>,
}

struct InFlightFetchHandle {
    url: String,
    method: String,
    resolve_fn: JsObject,
    reject_fn: JsObject,
    signal_obj: Option<JsObject>,
    als_snap: Option<JsValue>,
}

struct CompletedAsyncFetch {
    id: u32,
    result: Result<String, String>,
    violation: Option<SandboxViolation>,
    reused_conns: u64,
}

struct EventLoopState {
    next_timer_id: u32,
    next_fetch_id: u32,
    timers: Vec<ScheduledTimer>,
    queued_fetches: Vec<QueuedAsyncFetch>,
    in_flight_fetches: FxHashMap<u32, InFlightFetchHandle>,
    fetch_tx: Sender<CompletedAsyncFetch>,
    fetch_rx: Receiver<CompletedAsyncFetch>,
}

impl EventLoopState {
    fn new() -> Self {
        let (fetch_tx, fetch_rx) = mpsc::channel();
        Self {
            next_timer_id: 1,
            next_fetch_id: 1,
            timers: Vec::with_capacity(16),
            queued_fetches: Vec::with_capacity(8),
            in_flight_fetches: FxHashMap::default(),
            fetch_tx,
            fetch_rx,
        }
    }

    fn reset(&mut self) {
        self.timers.clear();
        self.queued_fetches.clear();
        self.in_flight_fetches.clear();
        while self.fetch_rx.try_recv().is_ok() {}
    }

    #[inline]
    fn active_timer_count(&self) -> usize {
        self.timers.iter().filter(|t| !t.cancelled).count()
    }

    #[inline]
    fn has_pending_work(&self) -> bool {
        !self.queued_fetches.is_empty()
            || !self.in_flight_fetches.is_empty()
            || self.timers.iter().any(|t| !t.cancelled)
    }
}

thread_local! {
    static EVENT_LOOP_STATE: RefCell<EventLoopState> = RefCell::new(EventLoopState::new());
    static ACTIVE_ASYNC_ITEMS: Cell<u32> = const { Cell::new(0) };
    static VIRTUAL_TIME_WARPED_MS: Cell<u64> = const { Cell::new(0) };
}

/// Returns the total milliseconds of virtual time warped by the deterministic event loop
/// (`--deterministic` or `slice test`) on the current worker thread without wall-clock sleeping.
pub fn virtual_time_warped_ms() -> u64 {
    VIRTUAL_TIME_WARPED_MS.with(|w| w.get())
}

/// Resets the virtual time-warp telemetry counter on the current worker thread.
pub fn reset_virtual_time_warped_ms() {
    VIRTUAL_TIME_WARPED_MS.with(|w| w.set(0));
}

#[inline]
pub(crate) fn has_pending_event_loop_work() -> bool {
    if crate::workers::has_pending_worker_work() {
        return true;
    }
    if ACTIVE_ASYNC_ITEMS.with(|c| c.get() == 0) {
        return false;
    }
    EVENT_LOOP_STATE.with(|st| st.borrow().has_pending_work())
}

pub(crate) struct SavedEventLoopSnapshot {
    state: EventLoopState,
    active_items: u32,
}

pub(crate) fn save_and_reset_event_loop_state() -> SavedEventLoopSnapshot {
    let active_items = ACTIVE_ASYNC_ITEMS.with(|c| c.replace(0));
    let state = EVENT_LOOP_STATE
        .with(|st| std::mem::replace(&mut *st.borrow_mut(), EventLoopState::new()));
    SavedEventLoopSnapshot {
        state,
        active_items,
    }
}

pub(crate) fn restore_event_loop_state(saved: SavedEventLoopSnapshot) {
    ACTIVE_ASYNC_ITEMS.with(|c| c.set(saved.active_items));
    EVENT_LOOP_STATE.with(|st| *st.borrow_mut() = saved.state);
}

pub(crate) fn clear_event_loop_state() {
    ACTIVE_ASYNC_ITEMS.with(|c| c.set(0));
    BOA_ALS_ACTIVE.with(|a| a.set(false));
    BOA_ALS_SNAP.with(|s| *s.borrow_mut() = JsValue::undefined());
    EVENT_LOOP_STATE.with(|st| st.borrow_mut().reset());
    crate::workers::clear_thread_worker_state();
    crate::sockets::close_all_active_sockets();
}

pub(crate) fn schedule_timer(
    callback: JsObject,
    delay_ms: u64,
    is_interval: bool,
    args: Vec<JsValue>,
) -> Result<u32, SandboxViolation> {
    schedule_timer_internal(callback, delay_ms, is_interval, false, args)
}

pub(crate) fn schedule_immediate(
    callback: JsObject,
    args: Vec<JsValue>,
) -> Result<u32, SandboxViolation> {
    schedule_timer_internal(callback, 0, false, true, args)
}

fn schedule_timer_internal(
    callback: JsObject,
    delay_ms: u64,
    is_interval: bool,
    is_immediate: bool,
    args: Vec<JsValue>,
) -> Result<u32, SandboxViolation> {
    EVENT_LOOP_STATE.with(|st| {
        let mut loop_st = st.borrow_mut();
        if loop_st.active_timer_count() >= MAX_ACTIVE_TIMERS {
            let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
            let violation = SandboxViolation::CpuFuelExhausted {
                tenant_id,
                limit: MAX_ACTIVE_TIMERS as u64,
            };
            BOA_VIOLATION.with(|v| *v.borrow_mut() = Some(violation.clone()));
            return Err(violation);
        }
        let id = loop_st.next_timer_id;
        loop_st.next_timer_id = loop_st.next_timer_id.wrapping_add(1).max(1);
        let now_ms = current_event_loop_clock_ms();
        let deadline_ms = now_ms.saturating_add(delay_ms);
        let interval_ms = if is_interval {
            Some(delay_ms.max(1))
        } else {
            None
        };
        let als_snap = if BOA_ALS_ACTIVE.with(|a| a.get()) {
            Some(BOA_ALS_SNAP.with(|s| s.borrow().clone()))
        } else {
            None
        };
        loop_st.timers.push(ScheduledTimer {
            id,
            deadline_ms,
            interval_ms,
            is_immediate,
            callback,
            args,
            als_snap,
            cancelled: false,
        });
        ACTIVE_ASYNC_ITEMS.with(|c| c.set(c.get().saturating_add(1)));
        Ok(id)
    })
}

pub(crate) fn cancel_timer(timer_id: u32) {
    if timer_id == 0 {
        return;
    }
    EVENT_LOOP_STATE.with(|st| {
        let mut loop_st = st.borrow_mut();
        for timer in &mut loop_st.timers {
            if timer.id == timer_id && !timer.cancelled {
                timer.cancelled = true;
                break;
            }
        }
        loop_st.timers.retain(|t| !t.cancelled);
        if !loop_st.has_pending_work() {
            ACTIVE_ASYNC_ITEMS.with(|c| c.set(0));
        }
    });
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn enqueue_async_fetch(
    url: String,
    method: String,
    headers: Vec<(String, String)>,
    body: String,
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
        ACTIVE_ASYNC_ITEMS.with(|c| c.set(c.get().saturating_add(1)));
        id
    })
}

#[inline]
fn is_signal_aborted(signal_opt: &Option<JsObject>, ctx: &mut Context) -> Option<JsValue> {
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
            .unwrap_or_else(|| {
                JsValue::from(js_string!("AbortError: The operation was aborted"))
            });
        Some(reason)
    } else {
        None
    }
}

fn settle_completed_fetch(
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

/// Drives the Phase N-2 Event Loop to completion:
/// 1. Drains all ECMAScript Promise microtasks & `queueMicrotask` jobs (`ctx.run_jobs()`).
/// 2. Dispatches queued outbound `fetch()` calls (executing single un-raced fetches inline for zero
///    channel overhead, and fanning out `Promise.all([fetch(...), ...])` or timer-raced fetches across
///    concurrent worker threads sharing `SHARED_OUTBOUND_CONN_POOL`).
/// 3. Fires ready `setTimeout` / `setInterval` macrotasks in deterministic `(deadline_ms, id)` order.
/// 4. In deterministic mode (`--deterministic` or `slice test`), warps `BOA_DET_STATE` virtual time
///    forward to the next timer deadline in `0 ns` without sleeping.
pub(crate) fn drive_event_loop_to_completion(ctx: &mut Context) {
    let _ = ctx.run_jobs();
    if !has_pending_event_loop_work() {
        return;
    }

    let mut total_ticks: usize = 0;
    let mut idle_worker_polls: usize = 0;

    loop {
        if BOA_VIOLATION.with(|v| v.borrow().is_some())
            || crate::workers::is_current_worker_terminated()
        {
            clear_event_loop_state();
            return;
        }

        // 1. Drain microtasks and any non-blocking worker messages first
        let _ = ctx.run_jobs();
        let did_parent_w = crate::workers::poll_parent_worker_events_and_dispatch(ctx, None);
        let did_self_w = crate::workers::poll_worker_self_inbox_and_dispatch(ctx, None);
        if did_parent_w || did_self_w {
            idle_worker_polls = 0;
            if BOA_VIOLATION.with(|v| v.borrow().is_some())
                || crate::workers::is_current_worker_terminated()
            {
                clear_event_loop_state();
                return;
            }
        }

        // 2. Check and prune any queued or in-flight fetches whose AbortSignal has fired
        let mut aborted_callbacks: Vec<(JsObject, JsValue)> = Vec::new();
        EVENT_LOOP_STATE.with(|st| {
            let mut loop_st = st.borrow_mut();
            let mut i = 0;
            while i < loop_st.queued_fetches.len() {
                if let Some(reason) = is_signal_aborted(&loop_st.queued_fetches[i].signal_obj, ctx)
                {
                    let removed = loop_st.queued_fetches.remove(i);
                    aborted_callbacks.push((removed.reject_fn, reason));
                } else {
                    i += 1;
                }
            }
            let in_flight_ids: Vec<u32> = loop_st.in_flight_fetches.keys().copied().collect();
            for fid in in_flight_ids {
                let abort_opt = loop_st
                    .in_flight_fetches
                    .get(&fid)
                    .and_then(|h| is_signal_aborted(&h.signal_obj, ctx));
                if let Some(reason) = abort_opt {
                    if let Some(removed) = loop_st.in_flight_fetches.remove(&fid) {
                        aborted_callbacks.push((removed.reject_fn, reason));
                    }
                }
            }
        });
        if !aborted_callbacks.is_empty() {
            for (reject_fn, reason) in aborted_callbacks {
                let _ = reject_fn.call(&JsValue::undefined(), &[reason], ctx);
            }
            let _ = ctx.run_jobs();
        }

        // 3. Dispatch queued outbound `fetch()` requests (bounded by MAX_CONCURRENT_FETCHES = 16)
        const MAX_CONCURRENT_FETCHES: usize = 16;
        let (queued_batch, has_timers_or_inflight, tx_clone) = EVENT_LOOP_STATE.with(|st| {
            let mut loop_st = st.borrow_mut();
            let avail_slots =
                MAX_CONCURRENT_FETCHES.saturating_sub(loop_st.in_flight_fetches.len());
            let drain_count = loop_st.queued_fetches.len().min(avail_slots);
            let has_more_queued = loop_st.queued_fetches.len() > drain_count;
            let batch: Vec<QueuedAsyncFetch> =
                loop_st.queued_fetches.drain(..drain_count).collect();
            let has_other = has_more_queued
                || !loop_st.in_flight_fetches.is_empty()
                || loop_st.active_timer_count() > 0;
            (batch, has_other, loop_st.fetch_tx.clone())
        });

        if !queued_batch.is_empty() {
            if queued_batch.len() == 1 && !has_timers_or_inflight {
                // Fast-path: 1 fetch with no racing timers or sibling fetches -> run inline on isolate thread
                let req = queued_batch.into_iter().next().unwrap();
                let res = execute_outbound_fetch_with_redirects(
                    &req.url,
                    &req.method,
                    &req.headers,
                    &req.body,
                    &req.redirect_mode,
                );
                if BOA_VIOLATION.with(|v| v.borrow().is_some()) {
                    clear_event_loop_state();
                    return;
                }
                settle_completed_fetch(
                    InFlightFetchHandle {
                        url: req.url,
                        method: req.method,
                        resolve_fn: req.resolve_fn,
                        reject_fn: req.reject_fn,
                        signal_obj: req.signal_obj,
                        als_snap: req.als_snap,
                    },
                    res,
                    ctx,
                );
                let _ = ctx.run_jobs();
            } else {
                // Concurrent Fan-Out Path (`Promise.all([fetch(...), ...])` or timer-raced `fetch`)
                let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
                let net_hosts: FxHashSet<String> = BOA_NET_HOSTS.with(|h| h.borrow().clone());
                let egress_secrets: Vec<String> = BOA_EGRESS_SECRETS.with(|s| s.borrow().clone());

                for req in queued_batch {
                    let fid = req.id;
                    let url = req.url.clone();
                    let method = req.method.clone();
                    let headers = req.headers;
                    let body = req.body;
                    let redirect_mode = req.redirect_mode;
                    let tx = tx_clone.clone();
                    let t_id = tenant_id.clone();
                    let n_hosts = net_hosts.clone();
                    let e_secrets = egress_secrets.clone();

                    EVENT_LOOP_STATE.with(|st| {
                        st.borrow_mut().in_flight_fetches.insert(
                            fid,
                            InFlightFetchHandle {
                                url: req.url,
                                method: req.method,
                                resolve_fn: req.resolve_fn,
                                reject_fn: req.reject_fn,
                                signal_obj: req.signal_obj,
                                als_snap: req.als_snap,
                            },
                        );
                    });

                    std::thread::spawn(move || {
                        BOA_ACTIVE_TENANT.with(|t| *t.borrow_mut() = t_id);
                        BOA_NET_HOSTS.with(|h| *h.borrow_mut() = n_hosts);
                        BOA_EGRESS_SECRETS.with(|s| *s.borrow_mut() = e_secrets);
                        BOA_VIOLATION.with(|v| *v.borrow_mut() = None);
                        let reuse_before = outbound_pool_reuse_count();

                        let result = execute_outbound_fetch_with_redirects(
                            &url,
                            &method,
                            &headers,
                            &body,
                            &redirect_mode,
                        );
                        let violation = BOA_VIOLATION.with(|v| v.borrow_mut().take());
                        let reused_conns =
                            outbound_pool_reuse_count().saturating_sub(reuse_before);

                        let _ = tx.send(CompletedAsyncFetch {
                            id: fid,
                            result,
                            violation,
                            reused_conns,
                        });
                    });
                }
            }
        }

        // 4. Drain any completed concurrent fetches from the channel without blocking
        let mut completed_list: Vec<(InFlightFetchHandle, Result<String, String>)> = Vec::new();
        let mut worker_violation: Option<SandboxViolation> = None;
        EVENT_LOOP_STATE.with(|st| {
            let mut loop_st = st.borrow_mut();
            while let Ok(done) = loop_st.fetch_rx.try_recv() {
                record_outbound_pool_reuses(done.reused_conns);
                if let Some(v) = done.violation {
                    if worker_violation.is_none() {
                        worker_violation = Some(v);
                    }
                }
                if let Some(handle) = loop_st.in_flight_fetches.remove(&done.id) {
                    completed_list.push((handle, done.result));
                }
            }
        });
        if let Some(v) = worker_violation {
            BOA_VIOLATION.with(|slot| *slot.borrow_mut() = Some(v));
            clear_event_loop_state();
            return;
        }
        if !completed_list.is_empty() {
            for (handle, result) in completed_list {
                settle_completed_fetch(handle, result, ctx);
            }
            let _ = ctx.run_jobs();
            continue;
        }

        // 5. Fire all ready timers (`deadline_ms <= now_ms`) in deterministic `(deadline_ms, !is_immediate, id)` order
        // so `setImmediate` check-phase tasks always execute before `setTimeout(0)` timers scheduled at the same millisecond.
        let now_ms = current_event_loop_clock_ms();
        type ReadyTimerEntry = (u32, u64, bool, JsObject, Vec<JsValue>, Option<JsValue>);
        let mut ready_timers: Vec<ReadyTimerEntry> = Vec::new();
        EVENT_LOOP_STATE.with(|st| {
            let mut loop_st = st.borrow_mut();
            for timer in &mut loop_st.timers {
                if !timer.cancelled && timer.deadline_ms <= now_ms {
                    ready_timers.push((
                        timer.id,
                        timer.deadline_ms,
                        timer.is_immediate,
                        timer.callback.clone(),
                        timer.args.clone(),
                        timer.als_snap.clone(),
                    ));
                    if let Some(iv) = timer.interval_ms {
                        timer.deadline_ms = now_ms.saturating_add(iv);
                    } else {
                        timer.cancelled = true;
                    }
                }
            }
            loop_st.timers.retain(|t| !t.cancelled);
        });

        if !ready_timers.is_empty() {
            ready_timers.sort_by_key(|(id, deadline, is_imm, _, _, _)| (*deadline, !*is_imm, *id));
            for (_id, _deadline, _is_imm, cb, args, als_snap) in ready_timers {
                total_ticks += 1;
                if total_ticks > MAX_EVENT_LOOP_TICKS {
                    let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
                    BOA_VIOLATION.with(|v| {
                        *v.borrow_mut() = Some(SandboxViolation::CpuFuelExhausted {
                            tenant_id,
                            limit: MAX_EVENT_LOOP_TICKS as u64,
                        });
                    });
                    clear_event_loop_state();
                    return;
                }
                let prev_als = als_snap
                    .as_ref()
                    .map(|snap| BOA_ALS_SNAP.with(|s| s.replace(snap.clone())));
                let _ = cb.call(&JsValue::undefined(), &args, ctx);
                if let Some(prev) = prev_als {
                    BOA_ALS_SNAP.with(|s| *s.borrow_mut() = prev);
                }
                let _ = ctx.run_jobs();
                if BOA_VIOLATION.with(|v| v.borrow().is_some()) {
                    clear_event_loop_state();
                    return;
                }
            }
            continue;
        }

        // 6. Check if any work remains; if not, exit cleanly
        let (in_flight_count, next_timer_deadline, has_queued) = EVENT_LOOP_STATE.with(|st| {
            let loop_st = st.borrow();
            let next_dl = loop_st
                .timers
                .iter()
                .filter(|t| !t.cancelled)
                .map(|t| t.deadline_ms)
                .min();
            (
                loop_st.in_flight_fetches.len(),
                next_dl,
                !loop_st.queued_fetches.is_empty(),
            )
        });

        if has_queued {
            continue;
        }
        let has_worker_work = crate::workers::has_pending_worker_work();
        if in_flight_count == 0 && next_timer_deadline.is_none() && !has_worker_work {
            ACTIVE_ASYNC_ITEMS.with(|c| c.set(0));
            break;
        }

        // 7. Either warp virtual time (deterministic mode with 0 in-flight fetches) or wait on I/O / worker / timer
        let now_ms = current_event_loop_clock_ms();
        if in_flight_count == 0 {
            if has_worker_work {
                let wait_dur = match next_timer_deadline {
                    Some(target_ms) if target_ms > now_ms => {
                        Duration::from_millis((target_ms - now_ms).min(20))
                    }
                    Some(_) => Duration::ZERO,
                    None => Duration::from_millis(20),
                };
                let got_p =
                    crate::workers::poll_parent_worker_events_and_dispatch(ctx, Some(wait_dur));
                let got_s =
                    crate::workers::poll_worker_self_inbox_and_dispatch(ctx, Some(wait_dur));
                if got_p || got_s {
                    idle_worker_polls = 0;
                    continue;
                }
                idle_worker_polls += 1;
                if idle_worker_polls > 250 && next_timer_deadline.is_none() {
                    break;
                }
            }
            if let Some(target_ms) = next_timer_deadline {
                let is_det = BOA_DET_STATE.with(|s| {
                    if let Some((prng, clk)) = s.get() {
                        if target_ms > clk {
                            let delta = target_ms - clk;
                            VIRTUAL_TIME_WARP_ADD(delta);
                            s.set(Some((prng, target_ms)));
                        }
                        true
                    } else {
                        false
                    }
                });
                if !is_det && target_ms > now_ms && !has_worker_work {
                    std::thread::sleep(Duration::from_millis(target_ms - now_ms));
                }
                continue;
            }
        } else {
            // Wait on the concurrent fetch completion channel up to the next timer deadline
            let wait_dur = match next_timer_deadline {
                Some(target_ms) => {
                    if BOA_DET_STATE.with(|s| s.get().is_some()) {
                        Duration::from_millis(5)
                    } else if target_ms > now_ms {
                        Duration::from_millis(target_ms - now_ms).min(MAX_CONCURRENT_FETCH_WAIT)
                    } else {
                        Duration::ZERO
                    }
                }
                None => MAX_CONCURRENT_FETCH_WAIT,
            };

            let recv_opt = EVENT_LOOP_STATE
                .with(|st| st.borrow().fetch_rx.recv_timeout(wait_dur));
            match recv_opt {
                Ok(done) => {
                    record_outbound_pool_reuses(done.reused_conns);
                    if let Some(v) = done.violation {
                        BOA_VIOLATION.with(|slot| *slot.borrow_mut() = Some(v));
                        clear_event_loop_state();
                        return;
                    }
                    let handle_opt = EVENT_LOOP_STATE
                        .with(|st| st.borrow_mut().in_flight_fetches.remove(&done.id));
                    if let Some(handle) = handle_opt {
                        settle_completed_fetch(handle, done.result, ctx);
                        let _ = ctx.run_jobs();
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if let Some(target_ms) = next_timer_deadline {
                        BOA_DET_STATE.with(|s| {
                            if let Some((prng, clk)) = s.get() {
                                if target_ms > clk {
                                    let step = (target_ms - clk).min(5);
                                    VIRTUAL_TIME_WARP_ADD(step);
                                    s.set(Some((prng, clk + step)));
                                }
                            }
                        });
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    clear_event_loop_state();
                    break;
                }
            }
        }
    }
}

#[inline]
#[allow(non_snake_case)]
fn VIRTUAL_TIME_WARP_ADD(delta_ms: u64) {
    VIRTUAL_TIME_WARPED_MS.with(|w| w.set(w.get().saturating_add(delta_ms)));
}
