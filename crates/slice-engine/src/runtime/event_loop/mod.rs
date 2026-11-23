//! Phase N-2: Deterministic Time-Travel Event Loop, Slab Timer Wheel (`setTimeout`, `setInterval`),
//! Microtask Job Queue (`queueMicrotask`), `AbortSignal` Timeout Coordinator, and Concurrent
//! Outbound `fetch()` Multiplexer (`Promise.all([fetch(...), ...])`).

mod driver;
mod fetch_reactor;
mod process_and_errors;
mod timers_and_state;

use std::time::Duration;

pub(super) const MAX_ACTIVE_TIMERS: usize = 1024;
pub(super) const MAX_EVENT_LOOP_TICKS: usize = 4096;
pub(super) const MAX_CONCURRENT_FETCH_WAIT: Duration = Duration::from_secs(10);

pub(crate) use driver::drive_event_loop_to_completion;
pub(crate) use fetch_reactor::enqueue_async_fetch;
pub use fetch_reactor::{
    MAX_FETCH_REACTOR_WORKERS, fetch_reactor_active_threads, fetch_reactor_pool_reuse_count,
    reset_fetch_reactor_telemetry,
};
pub(crate) use process_and_errors::{
    finalize_and_reset_process_state, handle_uncaught_js_error, is_boa_runtime_limit_error,
};
pub(crate) use timers_and_state::{
    SavedEventLoopSnapshot, cancel_timer, clear_event_loop_state, has_pending_event_loop_work,
    refresh_timer, restore_event_loop_state, save_and_reset_event_loop_state, schedule_immediate,
    schedule_timer, set_timer_ref,
};
pub use timers_and_state::{reset_virtual_time_warped_ms, virtual_time_warped_ms};
