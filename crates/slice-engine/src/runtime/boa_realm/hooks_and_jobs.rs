//! `AsyncLocalStorage` Host Hooks (`SliceHostHooks`) and Promise Microtask Queue (`SliceJobExecutor`).

use super::{
    BOA_ALS_ACTIVE, BOA_ALS_SNAP, BOA_EXEC_DIRTY, BOA_UNHANDLED_REJECTIONS, EXEC_DIRTY_REJECTIONS,
    has_process_exit, mark_exec_dirty,
};

struct AlsSnapGuard {
    prev: Option<boa_engine::JsValue>,
}

impl AlsSnapGuard {
    #[inline]
    fn enter(next: boa_engine::JsValue) -> Self {
        let prev = BOA_ALS_SNAP.with(|s| s.replace(next));
        Self { prev: Some(prev) }
    }
}

impl Drop for AlsSnapGuard {
    #[inline]
    fn drop(&mut self) {
        if let Some(prev) = self.prev.take() {
            BOA_ALS_SNAP.with(|s| *s.borrow_mut() = prev);
        }
    }
}

thread_local! {
    static BOA_ALS_MASK_STACK: std::cell::RefCell<Vec<boa_engine::JsValue>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

pub(crate) struct SliceHostHooks;

impl boa_engine::context::HostHooks for SliceHostHooks {
    #[inline]
    fn has_active_job_context(&self) -> bool {
        BOA_ALS_ACTIVE.with(|a| a.get())
    }

    #[inline]
    fn enter_empty_job_context(&self) {
        let prev = BOA_ALS_SNAP.with(|s| s.replace(boa_engine::JsValue::undefined()));
        BOA_ALS_MASK_STACK.with(|st| st.borrow_mut().push(prev));
    }

    #[inline]
    fn exit_empty_job_context(&self) {
        if let Some(prev) = BOA_ALS_MASK_STACK.with(|st| st.borrow_mut().pop()) {
            BOA_ALS_SNAP.with(|s| *s.borrow_mut() = prev);
        }
    }

    fn make_job_callback(
        &self,
        callback: boa_engine::object::builtins::JsFunction,
        _context: &mut boa_engine::Context,
    ) -> boa_engine::job::JobCallback {
        if BOA_ALS_ACTIVE.with(|a| a.get()) {
            let snap = BOA_ALS_SNAP.with(|s| s.borrow().clone());
            return boa_engine::job::JobCallback::new(callback, [snap]);
        }
        boa_engine::job::JobCallback::new(callback, ())
    }

    fn call_job_callback(
        &self,
        job: &boa_engine::job::JobCallback,
        this: &boa_engine::JsValue,
        args: &[boa_engine::JsValue],
        context: &mut boa_engine::Context,
    ) -> boa_engine::JsResult<boa_engine::JsValue> {
        if !BOA_ALS_ACTIVE.with(|a| a.get()) {
            return job.callback().call(this, args, context);
        }
        let next = job
            .host_defined()
            .downcast_ref::<[boa_engine::JsValue; 1]>()
            .map_or_else(boa_engine::JsValue::undefined, |[snap]| snap.clone());
        let _guard = AlsSnapGuard::enter(next);
        job.callback().call(this, args, context)
    }

    fn promise_rejection_tracker(
        &self,
        promise: &boa_engine::object::JsObject<boa_engine::builtins::promise::Promise>,
        operation: boa_engine::builtins::promise::OperationType,
        _context: &mut boa_engine::Context,
    ) {
        match operation {
            boa_engine::builtins::promise::OperationType::Reject => {
                mark_exec_dirty(EXEC_DIRTY_REJECTIONS);
                let upcast = promise.clone().upcast();
                BOA_UNHANDLED_REJECTIONS.with(|list| {
                    let mut b = list.borrow_mut();
                    if !b.contains(&upcast) {
                        b.push(upcast);
                    }
                });
            }
            boa_engine::builtins::promise::OperationType::Handle => {
                if (BOA_EXEC_DIRTY.with(|c| c.get()) & EXEC_DIRTY_REJECTIONS) == 0 {
                    return;
                }
                BOA_UNHANDLED_REJECTIONS.with(|list| {
                    let mut b = list.borrow_mut();
                    if b.is_empty() {
                        return;
                    }
                    let upcast = promise.clone().upcast();
                    b.retain(|p| p != &upcast);
                });
            }
        }
    }
}

pub(crate) struct SliceJobExecutor {
    promise_jobs: std::cell::RefCell<std::collections::VecDeque<boa_engine::job::PromiseJob>>,
    fallback: std::rc::Rc<boa_engine::job::SimpleJobExecutor>,
}

impl SliceJobExecutor {
    pub(crate) fn new() -> Self {
        Self {
            promise_jobs: std::cell::RefCell::new(std::collections::VecDeque::with_capacity(16)),
            fallback: std::rc::Rc::new(boa_engine::job::SimpleJobExecutor::new()),
        }
    }
}

impl boa_engine::job::JobExecutor for SliceJobExecutor {
    fn enqueue_job(
        self: std::rc::Rc<Self>,
        job: boa_engine::job::Job,
        context: &mut boa_engine::Context,
    ) {
        match job {
            boa_engine::job::Job::PromiseJob(p) => {
                self.promise_jobs.borrow_mut().push_back(p);
            }
            other => {
                self.fallback.clone().enqueue_job(other, context);
            }
        }
    }

    fn run_jobs(
        self: std::rc::Rc<Self>,
        context: &mut boa_engine::Context,
    ) -> boa_engine::JsResult<()> {
        if has_process_exit() {
            self.promise_jobs.borrow_mut().clear();
            return Ok(());
        }
        loop {
            let next_job = self.promise_jobs.borrow_mut().pop_front();
            if let Some(job) = next_job {
                let res = job.call(context);
                if has_process_exit() {
                    self.promise_jobs.borrow_mut().clear();
                    return Ok(());
                }
                res?;
            } else {
                self.fallback.clone().run_jobs(context)?;
                if has_process_exit() {
                    self.promise_jobs.borrow_mut().clear();
                    return Ok(());
                }
                if self.promise_jobs.borrow().is_empty() {
                    break;
                }
            }
        }
        Ok(())
    }
}
