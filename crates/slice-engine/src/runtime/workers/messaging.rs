//! Bidirectional Lock-Free `mpsc` Parent <-> Worker Message & Event Dispatch.

use super::{
    PARENT_WORKERS, WORKER_ACTIVE_COUNT, WORKER_SELF_ENV, WorkerInboundCmd, WorkerOutboundEvent,
    WorkerOutboundEventKind, charge_worker_message_budget,
};
use crate::boa_realm::{BOA_ACTIVE_TENANT, BOA_FUEL_BUDGET, BOA_VIOLATION};
use crate::event_loop::is_boa_runtime_limit_error;
use boa_engine::{Context, JsValue};
use slice_sandbox::SandboxViolation;
use std::sync::atomic::Ordering;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

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
        if let Some(ref term) = e.terminated
            && term.load(Ordering::Acquire)
        {
            should_terminate = true;
            return;
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
        if !should_terminate
            && received_msgs.is_empty()
            && e.self_listening
            && let Some(dur) = wait_timeout
        {
            match rx.recv_timeout(dur) {
                Ok(WorkerInboundCmd::Message(m)) => received_msgs.push(m),
                Ok(WorkerInboundCmd::Terminate) | Err(RecvTimeoutError::Disconnected) => {
                    should_terminate = true;
                }
                Err(RecvTimeoutError::Timeout) => {}
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
        && let Some(dispatch_fn) = dispatch_val.as_object()
    {
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
                if is_boa_runtime_limit_error(&msg) {
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
            if let WorkerOutboundEventKind::Exit { fuel_consumed, .. } = ev.kind
                && let Some(h) = r.workers.get_mut(&ev.worker_id)
            {
                h.finished = true;
                h.reconcile_resources(fuel_consumed);
                reconciled_any = true;
            }
            events.push(ev);
        }
        if events.is_empty()
            && r.has_active_refed_workers()
            && let Some(dur) = wait_timeout
            && let Ok(ev) = r.event_rx.recv_timeout(dur)
        {
            if let WorkerOutboundEventKind::Exit { fuel_consumed, .. } = ev.kind
                && let Some(h) = r.workers.get_mut(&ev.worker_id)
            {
                h.finished = true;
                h.reconcile_resources(fuel_consumed);
                reconciled_any = true;
            }
            events.push(ev);
            while let Ok(ev2) = r.event_rx.try_recv() {
                if let WorkerOutboundEventKind::Exit { fuel_consumed, .. } = ev2.kind
                    && let Some(h) = r.workers.get_mut(&ev2.worker_id)
                {
                    h.finished = true;
                    h.reconcile_resources(fuel_consumed);
                    reconciled_any = true;
                }
                events.push(ev2);
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
        && let Some(dispatch_fn) = dispatch_val.as_object()
    {
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
    false
}
