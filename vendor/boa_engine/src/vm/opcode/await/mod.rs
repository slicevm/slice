use super::RegisterOperand;
use crate::{
    Context, JsArgs, JsExpect, JsResult, JsValue,
    builtins::{
        Promise,
        async_generator::AsyncGenerator,
        generator::GeneratorContext,
        promise::{OperationType, PromiseCapability, PromiseState},
    },
    error::PanicError,
    job::PromiseJob,
    js_string,
    native_function::NativeFunction,
    object::FunctionObjectBuilder,
    vm::{CompletionRecord, GeneratorResumeKind, opcode::Operation},
};
use boa_gc::Gc;
use std::{cell::Cell, ops::ControlFlow};

/// `Await` implements the Opcode Operation for `Opcode::Await`
///
/// Operation:
///  - Stops the current Async function and schedules it to resume later.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Await;

impl Await {
    #[inline(always)]
    pub(super) fn operation(
        value: RegisterOperand,
        context: &mut Context,
    ) -> ControlFlow<CompletionRecord> {
        let value = context.vm.get_register(value.into());

        // 2. Let promise be ? PromiseResolve(%Promise%, value).
        let promise = if let Some(fast) = Promise::try_fast_as_intrinsic_promise(value, context) {
            fast
        } else {
            let value = value.clone();
            match Promise::promise_resolve(
                &context.intrinsics().constructors().promise().constructor(),
                value,
                context,
            ) {
                Ok(promise) => match promise.downcast::<Promise>().ok() {
                    Some(v) => v,
                    None => {
                        return context.handle_error(
                            PanicError::new("%Promise% constructor must return a `Promise` object")
                                .into(),
                        );
                    }
                },
                Err(err) => return context.handle_error(err),
            }
        };

        let return_value = context
            .vm
            .get_promise_capability()
            .ok()
            .map(|cap| JsValue::from(cap.promise))
            .unwrap_or_default();

        // Zero-GC-allocation fast path when `promise` is already settled (`Fulfilled` or `Rejected`)
        // and no host `JobCallback` continuation context (e.g. `AsyncLocalStorage`) is active:
        // neither `onFulfilled` nor `onRejected` is exposed to user code (`resultCapability` is `None`),
        // so we can enqueue a direct `PromiseJob` resuming `GeneratorContext` by value without
        // allocating `Gc<Cell>`, `TraceableClosure`, `FunctionObjectBuilder`, or a VM `CallFrame`.
        if !context.host_hooks().has_active_job_context() {
            let settled_opt = {
                let mut borrowed = promise.borrow_mut();
                let data = borrowed.data_mut();
                match data.state() {
                    PromiseState::Pending => None,
                    PromiseState::Fulfilled(v) => {
                        let val = v.clone();
                        data.mark_handled();
                        Some((val, GeneratorResumeKind::Normal, true))
                    }
                    PromiseState::Rejected(r) => {
                        let reason = r.clone();
                        let was_handled = data.is_handled();
                        data.mark_handled();
                        Some((reason, GeneratorResumeKind::Throw, was_handled))
                    }
                }
            };
            if let Some((arg, resume_kind, was_handled)) = settled_opt {
                if matches!(resume_kind, GeneratorResumeKind::Throw) && !was_handled {
                    context.host_hooks().promise_rejection_tracker(
                        &promise,
                        OperationType::Handle,
                        context,
                    );
                }
                let mut r#gen = GeneratorContext::from_current(context, None);
                let realm = context.realm().clone();
                let job = PromiseJob::with_realm(
                    move |context| {
                        struct EmptyJobCtxGuard(std::rc::Rc<dyn crate::context::HostHooks>);
                        impl Drop for EmptyJobCtxGuard {
                            #[inline]
                            fn drop(&mut self) {
                                self.0.exit_empty_job_context();
                            }
                        }
                        let _als_guard = if context.host_hooks().has_active_job_context() {
                            let hooks = context.host_hooks();
                            hooks.enter_empty_job_context();
                            Some(EmptyJobCtxGuard(hooks))
                        } else {
                            None
                        };
                        let async_generator = r#gen.async_generator_object()?;
                        r#gen.resume(Some(arg), resume_kind, context);
                        if let Some(async_generator) = async_generator {
                            async_generator
                                .downcast_mut::<AsyncGenerator>()
                                .js_expect("must be async generator")?
                                .context = Some(r#gen);
                        }
                        Ok(JsValue::undefined())
                    },
                    realm,
                );
                context.job_executor().enqueue_job(job.into(), context);
                context.vm.set_return_value(return_value);
                return context.handle_yield();
            }
        }

        let settled_kind: u8 = match promise.borrow().data().state() {
            PromiseState::Pending => 0,
            PromiseState::Fulfilled(_) => 1,
            PromiseState::Rejected(_) => 2,
        };

        let r#gen = GeneratorContext::from_current(context, None);

        let captures = Gc::new(Cell::new(Some(r#gen)));

        // 3. Let fulfilledClosure be a new Abstract Closure with parameters (value) that captures asyncContext and performs the following steps when called:
        // 4. Let onFulfilled be CreateBuiltinFunction(fulfilledClosure, 1, "", « »).
        // When `promise` is already `PromiseState::Rejected`, `perform_promise_then` immediately
        // drops `fulfill_reaction` without exposing `on_fulfilled`, so we skip allocating it.
        let on_fulfilled = if settled_kind != 2 {
            Some(
                FunctionObjectBuilder::new(
                    context.realm(),
                    NativeFunction::from_copy_closure_with_captures(
                        |_this, args, captures, context| {
                            let mut r#gen = captures.take().js_expect("should only run once")?;
                            let async_generator = r#gen.async_generator_object()?;
                            r#gen.resume(
                                Some(args.get_or_undefined(0).clone()),
                                GeneratorResumeKind::Normal,
                                context,
                            );
                            if let Some(async_generator) = async_generator {
                                async_generator
                                    .downcast_mut::<AsyncGenerator>()
                                    .js_expect("must be async generator")?
                                    .context = Some(r#gen);
                            }
                            Ok(JsValue::undefined())
                        },
                        captures.clone(),
                    ),
                )
                .name(js_string!())
                .length(1)
                .build(),
            )
        } else {
            None
        };

        // 5. Let rejectedClosure be a new Abstract Closure with parameters (reason) that captures asyncContext and performs the following steps when called:
        // 6. Let onRejected be CreateBuiltinFunction(rejectedClosure, 1, "", « »).
        // When `promise` is already `PromiseState::Fulfilled`, `perform_promise_then` immediately
        // drops `reject_reaction` without exposing `on_rejected`, so we skip allocating it.
        let on_rejected = if settled_kind != 1 {
            Some(
                FunctionObjectBuilder::new(
                    context.realm(),
                    NativeFunction::from_copy_closure_with_captures(
                        |_this, args, captures, context| {
                            let mut r#gen = captures.take().js_expect("should only run once")?;
                            let async_generator = r#gen.async_generator_object()?;
                            r#gen.resume(
                                Some(args.get_or_undefined(0).clone()),
                                GeneratorResumeKind::Throw,
                                context,
                            );
                            if let Some(async_generator) = async_generator {
                                async_generator
                                    .downcast_mut::<AsyncGenerator>()
                                    .js_expect("must be async generator")?
                                    .context = Some(r#gen);
                            }
                            Ok(JsValue::undefined())
                        },
                        captures,
                    ),
                )
                .name(js_string!())
                .length(1)
                .build(),
            )
        } else {
            None
        };

        // 7. Perform PerformPromiseThen(promise, onFulfilled, onRejected).
        Promise::perform_promise_then(&promise, on_fulfilled, on_rejected, None, context);

        context.vm.set_return_value(return_value);
        context.handle_yield()
    }
}

impl Operation for Await {
    const NAME: &'static str = "Await";
    const INSTRUCTION: &'static str = "INST - Await";
    const COST: u8 = 5;
}

/// `CreatePromiseCapability` implements the Opcode Operation for `Opcode::CreatePromiseCapability`
///
/// Operation:
///  - Create a promise capacity for an async function.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CreatePromiseCapability;

impl CreatePromiseCapability {
    #[inline(always)]
    pub(super) fn operation((): (), context: &mut Context) -> JsResult<()> {
        let promise_capability = PromiseCapability::new(
            &context.intrinsics().constructors().promise().constructor(),
            context,
        )
        .js_expect("cannot fail per spec")?;

        context.vm.set_promise_capability(promise_capability)
    }
}

impl Operation for CreatePromiseCapability {
    const NAME: &'static str = "CreatePromiseCapability";
    const INSTRUCTION: &'static str = "INST - CreatePromiseCapability";
    const COST: u8 = 8;
}
