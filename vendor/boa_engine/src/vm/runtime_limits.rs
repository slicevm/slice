/// Represents the limits of different runtime operations.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeLimits {
    /// Max stack size before an error is thrown.
    stack_size: usize,

    /// Max loop iterations before an error is thrown.
    loop_iteration: u64,

    /// Total loop iterations executed since the limit was last configured/reset.
    loop_iterations_executed: u64,

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
    #[inline(always)]
    pub(crate) fn check_and_increment_loop_iteration(&mut self) -> bool {
        let next = self.loop_iterations_executed.wrapping_add(1);
        if next > self.loop_iteration {
            return false;
        }
        self.loop_iterations_executed = next;
        true
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
