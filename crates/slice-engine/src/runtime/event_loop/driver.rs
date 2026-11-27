//! Phase N-2 Event Loop Driver (`drive_event_loop_to_completion`).

use super::fetch_reactor::{
    FetchReactorJob, FetchReactorPool, is_signal_aborted, settle_completed_fetch,
};
use super::process_and_errors::{
    handle_uncaught_js_error, run_jobs_and_check_rejections, try_emit_before_exit,
};
use super::timers_and_state::{
    ACTIVE_ASYNC_ITEMS, EVENT_LOOP_STATE, InFlightFetchHandle, QueuedAsyncFetch,
    clear_event_loop_state, current_event_loop_clock_ms, has_pending_event_loop_work,
    virtual_time_warp_add,
};
use super::{MAX_CONCURRENT_FETCH_WAIT, MAX_EVENT_LOOP_TICKS};
use crate::boa_realm::{
    BOA_ACTIVE_TENANT, BOA_ALS_ACTIVE, BOA_ALS_SNAP, BOA_DET_STATE, BOA_EGRESS_SECRETS,
    BOA_KV_MEM_BUDGET, BOA_NET_HOSTS, BOA_VIOLATION, has_process_exit, has_uncaught_error,
    record_boa_ocap_denial,
};
use crate::outbound_http::{execute_outbound_fetch_with_redirects, record_outbound_pool_reuses};
use boa_engine::{Context, JsObject, JsValue};
use rustc_hash::FxHashSet;
use slice_sandbox::SandboxViolation;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

/// Drives the Phase N-2 Event Loop to completion:
/// 1. Drains all ECMAScript Promise microtasks & `queueMicrotask` jobs (`ctx.run_jobs()`).
/// 2. Dispatches queued outbound `fetch()` calls (executing single un-raced fetches inline for zero
///    channel overhead, and fanning out `Promise.all([fetch(...), ...])` or timer-raced fetches across
///    concurrent worker threads sharing `SHARED_OUTBOUND_CONN_POOL`).
/// 3. Fires ready `setTimeout` / `setInterval` macrotasks in deterministic `(deadline_ms, id)` order.
/// 4. In deterministic mode (`--deterministic` or `slice test`), warps `BOA_DET_STATE` virtual time
///    forward to the next timer deadline in `0 ns` without sleeping.
pub(crate) fn drive_event_loop_to_completion(ctx: &mut Context) {
    let mut before_exit_count: usize = 0;
    if !run_jobs_and_check_rejections(ctx) {
        clear_event_loop_state();
        return;
    }
    if !has_pending_event_loop_work() && !try_emit_before_exit(ctx, &mut before_exit_count) {
        return;
    }

    let mut total_ticks: usize = 0;

    loop {
        if BOA_VIOLATION.with(|v| v.borrow().is_some())
            || has_process_exit()
            || has_uncaught_error()
            || crate::workers::is_current_worker_terminated()
        {
            clear_event_loop_state();
            return;
        }

        // 1. Drain microtasks and any non-blocking worker messages first
        if !run_jobs_and_check_rejections(ctx) {
            clear_event_loop_state();
            return;
        }
        let did_parent_w = crate::workers::poll_parent_worker_events_and_dispatch(ctx, None);
        let did_self_w = crate::workers::poll_worker_self_inbox_and_dispatch(ctx, None);
        if (did_parent_w || did_self_w)
            && (BOA_VIOLATION.with(|v| v.borrow().is_some())
                || has_process_exit()
                || has_uncaught_error()
                || crate::workers::is_current_worker_terminated())
        {
            clear_event_loop_state();
            return;
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
            if !loop_st.in_flight_fetches.is_empty()
                && loop_st
                    .in_flight_fetches
                    .values()
                    .any(|h| h.signal_obj.is_some())
            {
                let in_flight_ids: Vec<u32> = loop_st.in_flight_fetches.keys().copied().collect();
                for fid in in_flight_ids {
                    let abort_opt = loop_st
                        .in_flight_fetches
                        .get(&fid)
                        .and_then(|h| is_signal_aborted(&h.signal_obj, ctx));
                    if let Some(reason) = abort_opt
                        && let Some(removed) = loop_st.in_flight_fetches.remove(&fid)
                    {
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

        // 3. Dispatch queued outbound `fetch()` requests (bounded by MAX_CONCURRENT_FETCHES = 64 slabs)
        const MAX_CONCURRENT_FETCHES: usize = 64;
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
                if !run_jobs_and_check_rejections(ctx) {
                    clear_event_loop_state();
                    return;
                }
            } else {
                // Concurrent Fan-Out Path (`Promise.all([fetch(...), ...])` or timer-raced `fetch`)
                // Dispatches onto the bounded `FetchReactorPool` (max 16 reusable worker threads)
                // backed by `slice-io::RegisteredBufferPool` 64 KB slabs.
                let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
                let net_hosts: FxHashSet<String> = BOA_NET_HOSTS.with(|h| h.borrow().clone());
                let allow_private_networks =
                    crate::boa_realm::BOA_ALLOW_PRIVATE_NETS.with(|p| p.get());
                let egress_secrets: Vec<String> = BOA_EGRESS_SECRETS.with(|s| s.borrow().clone());
                let mem_budget = BOA_KV_MEM_BUDGET.with(|m| m.get());
                let pool = FetchReactorPool::global();

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

                    pool.submit(FetchReactorJob {
                        id: fid,
                        url,
                        method,
                        headers,
                        body,
                        redirect_mode,
                        tenant_id: t_id,
                        net_hosts: n_hosts,
                        allow_private_networks,
                        egress_secrets: e_secrets,
                        mem_budget,
                        tx,
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
                if let Some(ocap_v) = done.ocap_denial {
                    record_boa_ocap_denial(ocap_v);
                }
                if let Some(v) = done.violation
                    && worker_violation.is_none()
                {
                    worker_violation = Some(v);
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
            if !run_jobs_and_check_rejections(ctx) {
                clear_event_loop_state();
                return;
            }
            continue;
        }

        // 5. Fire all ready timers (`deadline_ms <= now_ms`) in deterministic `(deadline_ms, !is_immediate, id)` order
        // so `setImmediate` check-phase tasks always execute before `setTimeout(0)` timers scheduled at the same millisecond,
        // re-verifying liveness right before each callback so `clearTimeout` / `clearInterval` / `timer.refresh()` inside an
        // earlier callback in the same tick prevents cancelled or postponed timers from firing.
        let now_ms = current_event_loop_clock_ms();
        let mut ready_keys: Vec<(u64, bool, u32)> = Vec::new();
        EVENT_LOOP_STATE.with(|st| {
            let loop_st = st.borrow();
            for timer in &loop_st.timers {
                if !timer.cancelled && timer.deadline_ms <= now_ms {
                    ready_keys.push((timer.deadline_ms, !timer.is_immediate, timer.id));
                }
            }
        });

        if !ready_keys.is_empty() {
            ready_keys.sort_unstable();
            for (_, _, timer_id) in ready_keys {
                let next_ready = EVENT_LOOP_STATE.with(|st| {
                    let mut loop_st = st.borrow_mut();
                    let mut extracted = None;
                    for timer in &mut loop_st.timers {
                        if timer.id == timer_id && !timer.cancelled && timer.deadline_ms <= now_ms {
                            extracted = Some((
                                timer.callback.clone(),
                                timer.args.clone(),
                                timer.als_snap.clone(),
                            ));
                            if let Some(iv) = timer.interval_ms {
                                timer.deadline_ms = now_ms.saturating_add(iv);
                            } else {
                                timer.cancelled = true;
                            }
                            break;
                        }
                    }
                    loop_st.timers.retain(|t| !t.cancelled);
                    if !loop_st.has_pending_work() {
                        ACTIVE_ASYNC_ITEMS.with(|c| c.set(0));
                    }
                    extracted
                });
                let Some((cb, args, als_snap)) = next_ready else {
                    continue;
                };
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
                let prev_als = if let Some(ref snap) = als_snap {
                    Some(BOA_ALS_SNAP.with(|s| s.replace(snap.clone())))
                } else if BOA_ALS_ACTIVE.with(|a| a.get()) {
                    Some(BOA_ALS_SNAP.with(|s| s.replace(JsValue::undefined())))
                } else {
                    None
                };
                let call_res = cb.call(&JsValue::undefined(), &args, ctx);
                if let Some(prev) = prev_als {
                    BOA_ALS_SNAP.with(|s| *s.borrow_mut() = prev);
                }
                if let Err(err) = call_res
                    && !handle_uncaught_js_error(err, ctx)
                {
                    clear_event_loop_state();
                    return;
                }
                if !run_jobs_and_check_rejections(ctx) {
                    clear_event_loop_state();
                    return;
                }
                if BOA_VIOLATION.with(|v| v.borrow().is_some())
                    || has_process_exit()
                    || has_uncaught_error()
                {
                    clear_event_loop_state();
                    return;
                }
            }
            continue;
        }

        // 6. Check if any work remains; if not, exit cleanly
        let (in_flight_count, has_refed_timers, next_timer_deadline, has_queued) = EVENT_LOOP_STATE
            .with(|st| {
                let loop_st = st.borrow();
                let has_refed = loop_st.timers.iter().any(|t| !t.cancelled && t.refed);
                let next_dl = loop_st
                    .timers
                    .iter()
                    .filter(|t| !t.cancelled)
                    .map(|t| t.deadline_ms)
                    .min();
                (
                    loop_st.in_flight_fetches.len(),
                    has_refed,
                    next_dl,
                    !loop_st.queued_fetches.is_empty(),
                )
            });

        if has_queued {
            continue;
        }
        let has_worker_work = crate::workers::has_pending_worker_work();
        if in_flight_count == 0 && !has_refed_timers && !has_worker_work {
            if try_emit_before_exit(ctx, &mut before_exit_count) {
                continue;
            }
            EVENT_LOOP_STATE.with(|st| st.borrow_mut().timers.clear());
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
                    continue;
                }
            }
            if let Some(target_ms) = next_timer_deadline {
                let is_det = BOA_DET_STATE.with(|s| {
                    if let Some((prng, clk)) = s.get() {
                        if target_ms > clk {
                            let delta = target_ms - clk;
                            virtual_time_warp_add(delta);
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

            let recv_opt = EVENT_LOOP_STATE.with(|st| st.borrow().fetch_rx.recv_timeout(wait_dur));
            match recv_opt {
                Ok(done) => {
                    record_outbound_pool_reuses(done.reused_conns);
                    if let Some(ocap_v) = done.ocap_denial {
                        record_boa_ocap_denial(ocap_v);
                    }
                    if let Some(v) = done.violation {
                        BOA_VIOLATION.with(|slot| *slot.borrow_mut() = Some(v));
                        clear_event_loop_state();
                        return;
                    }
                    let handle_opt = EVENT_LOOP_STATE
                        .with(|st| st.borrow_mut().in_flight_fetches.remove(&done.id));
                    if let Some(handle) = handle_opt {
                        settle_completed_fetch(handle, done.result, ctx);
                        if !run_jobs_and_check_rejections(ctx) {
                            clear_event_loop_state();
                            return;
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if let Some(target_ms) = next_timer_deadline {
                        BOA_DET_STATE.with(|s| {
                            if let Some((prng, clk)) = s.get()
                                && target_ms > clk
                            {
                                let step = (target_ms - clk).min(5);
                                virtual_time_warp_add(step);
                                s.set(Some((prng, clk + step)));
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
