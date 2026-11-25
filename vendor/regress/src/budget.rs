//! Amortized step, wall-clock deadline, and cross-thread interrupt budget for `regress`.

/// Reason a regex match was aborted early by the active execution budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegressAbort {
    /// Step / fuel budget exhausted.
    Fuel,
    /// Wall-clock deadline expired.
    WallClock,
    /// Host interrupt flag was signalled.
    Interrupted,
}

#[cfg(feature = "std")]
mod imp {
    use super::RegressAbort;
    use std::cell::{Cell, RefCell};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, OnceLock};
    use std::time::Instant;

    static REGRESS_EPOCH: OnceLock<Instant> = OnceLock::new();

    #[inline]
    fn monotonic_epoch_ns() -> u64 {
        let start = REGRESS_EPOCH.get_or_init(Instant::now);
        let ns = start.elapsed().as_nanos();
        u64::try_from(ns).unwrap_or(u64::MAX)
    }

    #[derive(Clone, Copy)]
    struct ActiveBudget {
        active: bool,
        remaining_steps: u64,
        consumed_steps: u64,
        deadline_epoch_ns: u64,
        aborted: Option<RegressAbort>,
    }

    impl ActiveBudget {
        const INACTIVE: Self = Self {
            active: false,
            remaining_steps: u64::MAX,
            consumed_steps: 0,
            deadline_epoch_ns: 0,
            aborted: None,
        };
    }

    thread_local! {
        static BUDGET: Cell<ActiveBudget> = const { Cell::new(ActiveBudget::INACTIVE) };
        static INTERRUPT: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
    }

    /// Runs `f` with an active step, wall-clock (`remaining_wall_ns`), and cross-thread interrupt
    /// budget, returning `(result, consumed_steps, abort_reason)`.
    pub fn with_step_budget<R>(
        max_steps: u64,
        remaining_wall_ns: Option<u64>,
        interrupt_flag: Option<Arc<AtomicBool>>,
        f: impl FnOnce() -> R,
    ) -> (R, u64, Option<RegressAbort>) {
        let deadline_epoch_ns = match remaining_wall_ns {
            Some(0) => {
                // Already expired before entering regex execution.
                let prev = BUDGET.with(|b| {
                    b.replace(ActiveBudget {
                        active: true,
                        remaining_steps: 0,
                        consumed_steps: 0,
                        deadline_epoch_ns: 1,
                        aborted: Some(RegressAbort::WallClock),
                    })
                });
                let res = f();
                BUDGET.with(|b| b.set(prev));
                return (res, 0, Some(RegressAbort::WallClock));
            }
            Some(rem_ns) => monotonic_epoch_ns().saturating_add(rem_ns),
            None => 0,
        };

        let prev_budget = BUDGET.with(|b| {
            b.replace(ActiveBudget {
                active: true,
                remaining_steps: max_steps,
                consumed_steps: 0,
                deadline_epoch_ns,
                aborted: None,
            })
        });
        let prev_int = if interrupt_flag.is_some() {
            Some(INTERRUPT.with(|i| i.replace(interrupt_flag)))
        } else {
            None
        };

        let res = f();

        let final_budget = BUDGET.with(|b| b.replace(prev_budget));
        if let Some(prev) = prev_int {
            INTERRUPT.with(|i| *i.borrow_mut() = prev);
        }

        (res, final_budget.consumed_steps, final_budget.aborted)
    }

    /// Charges `steps` against the active thread-local budget and checks fuel, wall-clock,
    /// and interrupt limits. Returns `true` if execution must abort immediately.
    #[inline]
    pub(crate) fn charge_and_check_abort(steps: u64) -> bool {
        BUDGET.with(|cell| {
            let mut b = cell.get();
            if !b.active {
                return false;
            }
            if b.aborted.is_some() {
                return true;
            }
            b.consumed_steps = b.consumed_steps.saturating_add(steps);
            if steps >= b.remaining_steps {
                b.remaining_steps = 0;
                b.aborted = Some(RegressAbort::Fuel);
                cell.set(b);
                return true;
            }
            b.remaining_steps -= steps;

            let interrupted = INTERRUPT.with(|i| {
                i.borrow()
                    .as_ref()
                    .is_some_and(|flag| flag.load(Ordering::Acquire))
            });
            if interrupted {
                b.aborted = Some(RegressAbort::Interrupted);
                cell.set(b);
                return true;
            }

            if b.deadline_epoch_ns != 0 && monotonic_epoch_ns() >= b.deadline_epoch_ns {
                b.aborted = Some(RegressAbort::WallClock);
                cell.set(b);
                return true;
            }

            cell.set(b);
            false
        })
    }
}

#[cfg(feature = "std")]
pub use imp::with_step_budget;

#[cfg(feature = "std")]
pub(crate) use imp::charge_and_check_abort;

#[cfg(not(feature = "std"))]
#[inline(always)]
pub(crate) fn charge_and_check_abort(_steps: u64) -> bool {
    false
}
