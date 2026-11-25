use crate::error::RuntimeLimitError;
use std::sync::OnceLock;
use std::time::Instant;

static LIMIT_EPOCH: OnceLock<Instant> = OnceLock::new();

#[inline(always)]
fn monotonic_epoch_ns() -> u64 {
    LIMIT_EPOCH.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

/// Represents the limits of different runtime operations.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeLimits {
    /// Max stack size before an error is thrown.
    stack_size: usize,

    /// Max loop iterations / fuel units before an error is thrown.
    loop_iteration: u64,

    /// Total loop iterations / fuel units executed since the limit was last configured/reset.
    loop_iterations_executed: u64,

    /// Max guest heap bytes allocated before an uncatchable `MemoryLimit` error is thrown.
    heap_bytes_limit: usize,

    /// Cumulative guest heap bytes allocated during the current execution.
    heap_bytes_allocated: usize,

    /// Peak guest heap bytes observed during the current execution.
    peak_heap_bytes: usize,

    /// Monotonic wall-clock deadline in nanoseconds since `LIMIT_EPOCH` (`0` = disabled).
    deadline_epoch_ns: u64,

    /// Max backtrace count in exception.
    backtrace_limit: usize,

    /// Max function recursion limit
    recursion: usize,
}

impl Default for RuntimeLimits {
    #[inline]
    fn default() -> Self {
        Self {
            loop_iteration: u64::MAX,
            loop_iterations_executed: 0,
            heap_bytes_limit: usize::MAX,
            heap_bytes_allocated: 0,
            peak_heap_bytes: 0,
            deadline_epoch_ns: 0,
            recursion: 512,
            backtrace_limit: 50,
            stack_size: 1024 * 10,
        }
    }
}

impl RuntimeLimits {
    /// Return the loop iteration limit.
    ///
    /// If the limit is exceeded in a loop it will throw an error.
    ///
    /// The limit value [`u64::MAX`] means that there is no limit.
    #[inline]
    #[must_use]
    pub const fn loop_iteration_limit(&self) -> u64 {
        self.loop_iteration
    }

    /// Set the loop iteration limit.
    ///
    /// If the limit is exceeded in a loop it will throw an error.
    ///
    /// Setting the limit to [`u64::MAX`] means that there is no limit.
    #[inline]
    pub fn set_loop_iteration_limit(&mut self, value: u64) {
        self.loop_iteration = value;
        self.loop_iterations_executed = 0;
    }

    /// Disable loop iteration limit.
    #[inline]
    pub fn disable_loop_iteration_limit(&mut self) {
        self.loop_iteration = u64::MAX;
        self.loop_iterations_executed = 0;
    }

    /// Returns the total number of loop iterations executed since the loop limit was set.
    #[inline]
    #[must_use]
    pub const fn loop_iterations_executed(&self) -> u64 {
        self.loop_iterations_executed
    }

    /// Increments the total number of loop iterations executed by 1.
    #[inline(always)]
    pub fn increment_loop_iterations_executed(&mut self) {
        self.loop_iterations_executed = self.loop_iterations_executed.wrapping_add(1);
    }

    /// Increments the total number of loop iterations executed by 1 and returns `false`
    /// if the configured loop iteration limit is exceeded.
    #[allow(dead_code)]
    #[inline(always)]
    pub(crate) fn check_and_increment_loop_iteration(&mut self) -> bool {
        let next = self.loop_iterations_executed.wrapping_add(1);
        if next > self.loop_iteration {
            return false;
        }
        self.loop_iterations_executed = next;
        true
    }

    /// Charges `units` of execution fuel and performs an amortized wall-clock deadline check
    /// every `4,096` fuel units. Returns `(crossed_4k_boundary)` on success so callers can
    /// also amortize an external `AtomicBool` interrupt check on the exact same boundary.
    #[inline(always)]
    pub(crate) fn charge_fuel_units(&mut self, units: u64) -> Result<bool, RuntimeLimitError> {
        let prev = self.loop_iterations_executed;
        let next = prev.saturating_add(units);
        if next > self.loop_iteration {
            self.loop_iterations_executed = self.loop_iteration;
            return Err(RuntimeLimitError::LoopIteration);
        }
        self.loop_iterations_executed = next;
        let crossed_4k = (next >> 12) != (prev >> 12);
        if crossed_4k && self.deadline_epoch_ns != 0 && monotonic_epoch_ns() >= self.deadline_epoch_ns
        {
            return Err(RuntimeLimitError::WallClock);
        }
        Ok(crossed_4k)
    }

    /// Sets the maximum guest heap bytes allowed for this execution and resets counters.
    #[inline]
    pub fn set_heap_bytes_limit(&mut self, limit: usize) {
        self.heap_bytes_limit = limit;
        self.heap_bytes_allocated = 0;
        self.peak_heap_bytes = 0;
    }

    /// Disables the guest heap byte ceiling.
    #[inline]
    pub fn disable_heap_bytes_limit(&mut self) {
        self.heap_bytes_limit = usize::MAX;
        self.heap_bytes_allocated = 0;
        self.peak_heap_bytes = 0;
    }

    /// Returns the peak guest heap bytes observed since `set_heap_bytes_limit` was called.
    #[inline]
    #[must_use]
    pub const fn peak_heap_bytes(&self) -> usize {
        self.peak_heap_bytes
    }

    /// Returns the cumulative guest heap bytes allocated since `set_heap_bytes_limit` was called.
    #[inline]
    #[must_use]
    pub const fn heap_bytes_allocated(&self) -> usize {
        self.heap_bytes_allocated
    }

    /// Charges `bytes` against the guest heap memory ceiling.
    #[inline(always)]
    pub(crate) fn charge_heap_bytes(&mut self, bytes: usize) -> Result<(), RuntimeLimitError> {
        let next = self.heap_bytes_allocated.saturating_add(bytes);
        if next > self.peak_heap_bytes {
            self.peak_heap_bytes = next;
        }
        if next > self.heap_bytes_limit {
            return Err(RuntimeLimitError::MemoryLimit {
                requested: next,
                limit: self.heap_bytes_limit,
            });
        }
        self.heap_bytes_allocated = next;
        Ok(())
    }

    /// Configures an optional wall-clock deadline in milliseconds (`None` disables the deadline).
    #[inline]
    pub fn set_wall_clock_limit_ms(&mut self, timeout_ms: Option<u64>) {
        self.deadline_epoch_ns = match timeout_ms {
            Some(ms) => monotonic_epoch_ns().saturating_add(ms.saturating_mul(1_000_000)),
            None => 0,
        };
    }

    /// Returns `true` if a wall-clock deadline is active and has already expired.
    #[inline]
    #[must_use]
    pub fn is_wall_clock_expired(&self) -> bool {
        self.deadline_epoch_ns != 0 && monotonic_epoch_ns() >= self.deadline_epoch_ns
    }

    /// Get max backtrace limit for an exception.
    ///
    /// Default is 50.
    #[inline]
    #[must_use]
    pub const fn backtrace_limit(&self) -> usize {
        self.backtrace_limit
    }

    /// Set max backtrace limit for an exception.
    #[inline]
    pub fn set_backtrace_limit(&mut self, value: usize) {
        self.backtrace_limit = value;
    }

    /// Get max stack size.
    #[inline]
    #[must_use]
    pub const fn stack_size_limit(&self) -> usize {
        self.stack_size
    }

    /// Set max stack size before an error is thrown.
    #[inline]
    pub fn set_stack_size_limit(&mut self, value: usize) {
        self.stack_size = value;
    }

    /// Get recursion limit.
    #[inline]
    #[must_use]
    pub const fn recursion_limit(&self) -> usize {
        self.recursion
    }

    /// Set recursion limit before an error is thrown.
    #[inline]
    pub fn set_recursion_limit(&mut self, value: usize) {
        self.recursion = value;
    }
}
