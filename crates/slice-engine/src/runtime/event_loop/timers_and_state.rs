//! Slab Timer Wheel (`setTimeout`, `setInterval`, `setImmediate`), Monotonic & Virtual Clocks,
//! and Thread-Local Event Loop State Snapshots.

use super::MAX_ACTIVE_TIMERS;
use crate::boa_realm::{
    BOA_ACTIVE_TENANT, BOA_ALS_ACTIVE, BOA_ALS_SNAP, BOA_DET_STATE, BOA_DIRTY_SUBSYSTEMS,
    BOA_VIOLATION, DIRTY_ALS, DIRTY_EVENT_LOOP, DIRTY_SOCKETS, DIRTY_WORKERS, mark_subsystem_dirty,
};
use boa_engine::{JsObject, JsValue};
use rustc_hash::FxHashMap;
use slice_sandbox::SandboxViolation;
use std::cell::{Cell, RefCell};
use std::sync::OnceLock;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;

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
    pub(crate) delay_ms: u64,
    pub(crate) deadline_ms: u64,
    pub(crate) interval_ms: Option<u64>,
    pub(crate) is_immediate: bool,
    pub(crate) refed: bool,
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
    pub(crate) body: Vec<u8>,
    pub(crate) redirect_mode: String,
    pub(crate) resolve_fn: JsObject,
    pub(crate) reject_fn: JsObject,
    pub(crate) signal_obj: Option<JsObject>,
    pub(crate) als_snap: Option<JsValue>,
}

pub(super) struct InFlightFetchHandle {
    pub(super) url: String,
    pub(super) method: String,
    pub(super) resolve_fn: JsObject,
    pub(super) reject_fn: JsObject,
    pub(super) signal_obj: Option<JsObject>,
    pub(super) als_snap: Option<JsValue>,
}

pub(super) struct CompletedAsyncFetch {
    pub(super) id: u32,
    pub(super) result: Result<String, String>,
    pub(super) violation: Option<SandboxViolation>,
    pub(super) reused_conns: u64,
}

pub(super) struct EventLoopState {
    pub(super) next_timer_id: u32,
    pub(super) next_fetch_id: u32,
    pub(super) timers: Vec<ScheduledTimer>,
    pub(super) queued_fetches: Vec<QueuedAsyncFetch>,
    pub(super) in_flight_fetches: FxHashMap<u32, InFlightFetchHandle>,
    pub(super) fetch_tx: Sender<CompletedAsyncFetch>,
    pub(super) fetch_rx: Receiver<CompletedAsyncFetch>,
}

impl EventLoopState {
    pub(super) fn new() -> Self {
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

    pub(super) fn reset(&mut self) {
        self.timers.clear();
        self.queued_fetches.clear();
        self.in_flight_fetches.clear();
        while self.fetch_rx.try_recv().is_ok() {}
    }

    #[inline]
    pub(super) fn active_timer_count(&self) -> usize {
        self.timers.iter().filter(|t| !t.cancelled).count()
    }

    #[inline]
    pub(super) fn has_pending_work(&self) -> bool {
        !self.queued_fetches.is_empty()
            || !self.in_flight_fetches.is_empty()
            || self.timers.iter().any(|t| !t.cancelled && t.refed)
    }
}

thread_local! {
    pub(super) static EVENT_LOOP_STATE: RefCell<EventLoopState> = RefCell::new(EventLoopState::new());
    pub(super) static ACTIVE_ASYNC_ITEMS: Cell<u32> = const { Cell::new(0) };
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
pub(super) fn virtual_time_warp_add(delta_ms: u64) {
    VIRTUAL_TIME_WARPED_MS.with(|w| w.set(w.get().saturating_add(delta_ms)));
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

pub(crate) fn save_and_reset_event_loop_state() -> Option<SavedEventLoopSnapshot> {
    let active_items = ACTIVE_ASYNC_ITEMS.with(|c| c.replace(0));
    EVENT_LOOP_STATE.with(|st| {
        if active_items == 0 && !st.borrow().has_pending_work() {
            return None;
        }
        let state = std::mem::replace(&mut *st.borrow_mut(), EventLoopState::new());
        Some(SavedEventLoopSnapshot {
            state,
            active_items,
        })
    })
}

pub(crate) fn restore_event_loop_state(saved: SavedEventLoopSnapshot) {
    ACTIVE_ASYNC_ITEMS.with(|c| c.set(saved.active_items));
    EVENT_LOOP_STATE.with(|st| *st.borrow_mut() = saved.state);
}

#[inline]
pub(crate) fn clear_event_loop_state() {
    let dirty = BOA_DIRTY_SUBSYSTEMS.with(|d| d.get());
    let has_async = ACTIVE_ASYNC_ITEMS.with(|c| c.replace(0)) > 0;
    if has_async || (dirty & DIRTY_EVENT_LOOP) != 0 {
        EVENT_LOOP_STATE.with(|st| st.borrow_mut().reset());
    }
    if (dirty & DIRTY_ALS) != 0 || BOA_ALS_ACTIVE.with(|a| a.get()) {
        BOA_ALS_ACTIVE.with(|a| a.set(false));
        BOA_ALS_SNAP.with(|s| *s.borrow_mut() = JsValue::undefined());
    }
    if (dirty & DIRTY_WORKERS) != 0 {
        crate::workers::clear_thread_worker_state();
    }
    if (dirty & DIRTY_SOCKETS) != 0 {
        crate::sockets::close_all_active_sockets();
    }
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
            delay_ms,
            deadline_ms,
            interval_ms,
            is_immediate,
            refed: true,
            callback,
            args,
            als_snap,
            cancelled: false,
        });
        mark_subsystem_dirty(DIRTY_EVENT_LOOP);
        ACTIVE_ASYNC_ITEMS.with(|c| c.set(c.get().saturating_add(1)));
        Ok(id)
    })
}

pub(crate) fn set_timer_ref(timer_id: u32, refed: bool) {
    if timer_id == 0 {
        return;
    }
    EVENT_LOOP_STATE.with(|st| {
        let mut loop_st = st.borrow_mut();
        for timer in &mut loop_st.timers {
            if timer.id == timer_id && !timer.cancelled {
                timer.refed = refed;
                break;
            }
        }
        if loop_st.has_pending_work() {
            ACTIVE_ASYNC_ITEMS.with(|c| {
                if c.get() == 0 {
                    c.set(1);
                }
            });
        } else {
            ACTIVE_ASYNC_ITEMS.with(|c| c.set(0));
        }
    });
}

pub(crate) fn refresh_timer(timer_id: u32) {
    if timer_id == 0 {
        return;
    }
    let now_ms = current_event_loop_clock_ms();
    EVENT_LOOP_STATE.with(|st| {
        let mut loop_st = st.borrow_mut();
        for timer in &mut loop_st.timers {
            if timer.id == timer_id && !timer.cancelled {
                timer.deadline_ms = now_ms.saturating_add(timer.delay_ms);
                break;
            }
        }
        if loop_st.has_pending_work() {
            ACTIVE_ASYNC_ITEMS.with(|c| {
                if c.get() == 0 {
                    c.set(1);
                }
            });
        }
    });
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
