//! Warm OS Worker Thread Pool (`IDLE_WORKER_POOL`), Prewarming, and Job Execution Loop.

use super::{
    GLOBAL_ACTIVE_WORKERS, MAX_POOLED_WORKER_THREADS, TOTAL_POOLED_WORKER_THREADS,
    WORKER_POOL_REUSE_COUNT, WORKER_SELF_ENV, WorkerJob, WorkerOutboundEvent,
    WorkerOutboundEventKind, WorkerSelfThreadEnv, idle_worker_pool,
};
use crate::boa_realm::{BOA_FETCH_MOCKS, BOA_VFS_OWNER_TENANT, BOA_VFS_STATE, vfs_owner_namespace};
use crate::isolate::SliceIsolate;
use crate::sockets::BOA_SOCKET_MOCKS;
use crate::websocket::BOA_WS_MOCKS;
use slice_sandbox::SandboxViolation;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

fn spawn_pooled_worker_os_thread(
    initial_job: Option<WorkerJob>,
    ready_notify: Option<Sender<()>>,
) -> Result<Sender<WorkerJob>, String> {
    let idx = TOTAL_POOLED_WORKER_THREADS.fetch_add(1, Ordering::AcqRel) + 1;
    let (job_tx, job_rx) = mpsc::channel::<WorkerJob>();
    if let Some(job) = initial_job {
        let _ = job_tx.send(job);
    }
    let tx_for_thread = job_tx.clone();
    std::thread::Builder::new()
        .name(format!("slice-worker-pool-{}", idx))
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            worker_pool_thread_loop(tx_for_thread, job_rx, ready_notify);
        })
        .map(|_| job_tx)
        .map_err(|e| {
            TOTAL_POOLED_WORKER_THREADS.fetch_sub(1, Ordering::AcqRel);
            e.to_string()
        })
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
        match spawn_pooled_worker_os_thread(None, None) {
            Ok(job_tx) => pool.push(job_tx),
            Err(_) => break,
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
            let (ready_tx, ready_rx) = mpsc::channel::<()>();
            match spawn_pooled_worker_os_thread(None, Some(ready_tx)) {
                Ok(job_tx) => {
                    pool.push(job_tx);
                    ready_rxs.push(ready_rx);
                }
                Err(_) => break,
            }
        }
    }
    for rx in ready_rxs {
        let _ = rx.recv_timeout(Duration::from_secs(2));
    }
}

pub(super) fn dispatch_worker_job(mut job: WorkerJob) -> Result<(), String> {
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
    spawn_pooled_worker_os_thread(Some(job), None).map(|_| ())
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

pub(crate) struct WorkerTurnUnwindGuard {
    pub(crate) armed: bool,
}

impl Drop for WorkerTurnUnwindGuard {
    fn drop(&mut self) {
        if self.armed {
            WORKER_SELF_ENV.with(|env| *env.borrow_mut() = WorkerSelfThreadEnv::default());
            BOA_VFS_OWNER_TENANT.with(|o| o.borrow_mut().clear());
            BOA_VFS_STATE.with(|v| v.borrow_mut().clear());
            GLOBAL_ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

fn run_pooled_worker_job(job: WorkerJob, self_job_tx: &Sender<WorkerJob>) {
    let worker_id = job.worker_id;
    let event_tx = job.event_tx;
    let terminated_clone = job.terminated;

    // Fast abort if parent already called `worker.terminate()` before job started
    if terminated_clone.load(Ordering::Acquire) {
        GLOBAL_ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
        if let Ok(mut pool) = idle_worker_pool().lock()
            && pool.len() < MAX_POOLED_WORKER_THREADS
        {
            pool.push(self_job_tx.clone());
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
    child_isolate.egress_audit_secrets = job.parent_egress_secrets;

    let mut worker_unwind_guard = WorkerTurnUnwindGuard { armed: true };
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
    worker_unwind_guard.armed = false;
    WORKER_SELF_ENV.with(|env| *env.borrow_mut() = WorkerSelfThreadEnv::default());
    BOA_VFS_OWNER_TENANT.with(|o| o.borrow_mut().clear());
    BOA_VFS_STATE.with(|v| v.borrow_mut().clear());
    GLOBAL_ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel);
    if let Ok(mut pool) = idle_worker_pool().lock()
        && pool.len() < MAX_POOLED_WORKER_THREADS
    {
        pool.push(self_job_tx.clone());
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
