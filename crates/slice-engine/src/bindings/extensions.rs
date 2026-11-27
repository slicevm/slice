//! Native Host Bindings for Capability-Gated Host Extensions (`Slice.ext.<name>.<method>`) (`#229`).
//!
//! Registers `__slice_ext_call(extName, methodName, arg)` and `__slice_ext_list_methods(extName)`:
//! - Enforces deny-by-default `ExtensionCap` (`BOA_ALLOWED_EXTENSIONS`) with catchable
//!   `ERR_SLICE_EXT_DENIED` (`OCap Violation: lacks extension authority for '<ext>.<method>'`).
//! - Copies arguments and results across the boundary (JSON or bytes; zero host objects enter JS).
//! - Enforces `max_arg_bytes`, `max_result_bytes`, deterministic fuel (`base + per_byte`),
//!   memory ceiling accounting, Ring 4 Egress DLP scanning, determinism guard, checkpoint guard,
//!   wall-clock deadline, and host interrupt cancellation.

use crate::bindings::helpers::{
    active_tenant_id_or_default, ocap_denied_js_error, record_boa_violation_once,
    trip_isolate_oom_limits, try_consume_isolate_memory_ctx,
};
use crate::boa_realm::{
    BOA_ACTIVE_DEADLINE, BOA_ACTIVE_EXTENSIONS, BOA_ACTIVE_INTERRUPT, BOA_ALLOWED_EXTENSIONS,
    BOA_CHECKPOINT_STATS, BOA_DET_STATE, BOA_EGRESS_SECRETS, BOA_FUEL_BUDGET,
    record_boa_ocap_denial,
};
use crate::extensions::{ExtCallCtx, ExtError, ExtFuture, ExtValue};
use crate::intrinsics::base64_encode;
use crate::state_and_kv::verify_egress_secrets_slice;
use slice_sandbox::{SandboxViolation, extension_grant_matches};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context as TaskContext, Poll, Wake, Waker};
use std::time::{Duration, Instant};

struct ThreadParkWaker(std::thread::Thread);

impl Wake for ThreadParkWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

/// Polls an async extension future on the current thread while honoring both wall-clock deadline
/// (`WallClockExceeded`) and host interrupt (`ExecutionInterrupted`).
fn poll_ext_future_bounded(
    mut fut: ExtFuture,
    deadline: Option<(Instant, u64)>,
    cancel: &Arc<AtomicBool>,
    tenant_id: &str,
) -> Result<Result<ExtValue, ExtError>, SandboxViolation> {
    let waker = Waker::from(Arc::new(ThreadParkWaker(std::thread::current())));
    let mut cx = TaskContext::from_waker(&waker);

    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(SandboxViolation::ExecutionInterrupted {
                tenant_id: tenant_id.to_string(),
            });
        }
        if let Some((dl, limit_ms)) = deadline
            && Instant::now() >= dl
        {
            cancel.store(true, Ordering::Release);
            return Err(SandboxViolation::WallClockExceeded {
                tenant_id: tenant_id.to_string(),
                limit_ms,
            });
        }

        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(res) => return Ok(res),
            Poll::Pending => {
                let sleep_dur = match deadline {
                    Some((dl, _)) => dl
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(5)),
                    None => Duration::from_millis(5),
                };
                if !sleep_dur.is_zero() {
                    std::thread::park_timeout(sleep_dur);
                }
            }
        }
    }
}

fn charge_extension_fuel(ops: u64, context: &mut boa_engine::Context) -> boa_engine::JsResult<()> {
    if ops == 0 {
        return Ok(());
    }
    let exceeded = BOA_FUEL_BUDGET.with(|f| {
        let (used, limit) = f.get();
        let next = used.saturating_add(ops);
        f.set((next, limit));
        (next > limit).then_some(limit)
    });
    if let Some(limit) = exceeded {
        let tenant_id = active_tenant_id_or_default();
        let violation = SandboxViolation::CpuFuelExhausted { tenant_id, limit };
        let msg = violation.to_string();
        record_boa_violation_once(violation);
        trip_isolate_oom_limits(context);
        return Err(boa_engine::JsNativeError::range().with_message(msg).into());
    }
    Ok(())
}

fn scan_ext_arg_dlp(
    tenant_id: &str,
    arg: &ExtValue,
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<()> {
    let has_secrets = BOA_EGRESS_SECRETS.with(|s| !s.borrow().is_empty());
    if !has_secrets {
        return Ok(());
    }
    let leaked = BOA_EGRESS_SECRETS.with(|secrets_cell| {
        let secrets = secrets_cell.borrow();
        match arg {
            ExtValue::Json(v) => {
                let s = serde_json::to_string(v).unwrap_or_default();
                verify_egress_secrets_slice(tenant_id, &secrets, &s).is_err()
            }
            ExtValue::Bytes(bytes) => {
                let lossy = String::from_utf8_lossy(bytes);
                if verify_egress_secrets_slice(tenant_id, &secrets, &lossy).is_err() {
                    return true;
                }
                if !bytes.is_empty() && !bytes.is_ascii() {
                    let b64 = base64_encode(bytes, false);
                    return verify_egress_secrets_slice(tenant_id, &secrets, &b64).is_err();
                }
                false
            }
        }
    });
    if leaked {
        let violation = SandboxViolation::EgressSecretLeakBlocked {
            tenant_id: tenant_id.to_string(),
        };
        let msg = violation.to_string();
        record_boa_violation_once(violation);
        trip_isolate_oom_limits(context);
        return Err(boa_engine::JsNativeError::error().with_message(msg).into());
    }
    Ok(())
}

fn js_val_to_ext_value(
    val: Option<&boa_engine::JsValue>,
    max_arg_bytes: usize,
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<(ExtValue, usize)> {
    use boa_engine::builtins::{dataview::DataView, typed_array::TypedArray};
    use boa_engine::object::builtins::JsArrayBuffer;

    let Some(v) = val.filter(|v| !v.is_undefined()) else {
        return Ok((ExtValue::Json(serde_json::Value::Null), 4));
    };

    // Binary fast-path: TypedArray / ArrayBuffer / DataView -> ExtValue::Bytes (pure copy).
    if let Some(obj) = v.as_object() {
        if let Some(ta) = obj.downcast_ref::<TypedArray>() {
            let mut copied: Option<Vec<u8>> = None;
            let _ = ta.with_bytes(|slice| {
                if slice.len() <= max_arg_bytes {
                    copied = Some(slice.to_vec());
                } else {
                    copied = None;
                }
            });
            if let Some(bytes) = copied {
                let len = bytes.len();
                return Ok((ExtValue::Bytes(bytes), len));
            }
            return Err(boa_engine::JsNativeError::range()
                .with_message(format!(
                    "Extension argument exceeds max_arg_bytes ({max_arg_bytes} bytes)"
                ))
                .into());
        }
        if let Ok(ab) = JsArrayBuffer::from_object(obj.clone()) {
            let bytes = ab.data().map(|d| d.to_vec()).unwrap_or_default();
            if bytes.len() > max_arg_bytes {
                return Err(boa_engine::JsNativeError::range()
                    .with_message(format!(
                        "Extension argument exceeds max_arg_bytes ({} > {} bytes)",
                        bytes.len(),
                        max_arg_bytes
                    ))
                    .into());
            }
            let len = bytes.len();
            return Ok((ExtValue::Bytes(bytes), len));
        }
        if let Some(dv) = obj.downcast_ref::<DataView>() {
            let mut copied: Option<Vec<u8>> = None;
            let _ = dv.with_bytes(|slice| {
                if slice.len() <= max_arg_bytes {
                    copied = Some(slice.to_vec());
                }
            });
            if let Some(bytes) = copied {
                let len = bytes.len();
                return Ok((ExtValue::Bytes(bytes), len));
            }
            return Err(boa_engine::JsNativeError::range()
                .with_message(format!(
                    "Extension argument exceeds max_arg_bytes ({max_arg_bytes} bytes)"
                ))
                .into());
        }
    }

    // JSON copy path: no host objects, functions, or handles cross the boundary.
    let json_val = match v.to_json(context)? {
        Some(j) => j,
        None => serde_json::Value::Null,
    };
    let encoded_len = serde_json::to_vec(&json_val)
        .map(|buf| buf.len())
        .unwrap_or(0);
    if encoded_len > max_arg_bytes {
        return Err(boa_engine::JsNativeError::range()
            .with_message(format!(
                "Extension argument exceeds max_arg_bytes ({encoded_len} > {max_arg_bytes} bytes)"
            ))
            .into());
    }
    Ok((ExtValue::Json(json_val), encoded_len))
}

fn ext_value_to_js_val(
    val: ExtValue,
    max_result_bytes: usize,
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<boa_engine::JsValue> {
    let result_len = val.approx_byte_len();
    if result_len > max_result_bytes {
        return Err(boa_engine::JsNativeError::range()
            .with_message(format!(
                "Extension result exceeds max_result_bytes ({result_len} > {max_result_bytes} bytes)"
            ))
            .into());
    }
    if result_len > 0
        && let Err(msg) = try_consume_isolate_memory_ctx(result_len, context)
    {
        return Err(boa_engine::JsNativeError::range().with_message(msg).into());
    }
    match val {
        ExtValue::Json(j) => boa_engine::JsValue::from_json(&j, context),
        ExtValue::Bytes(bytes) => {
            let arr = boa_engine::object::builtins::JsUint8Array::from_iter(bytes, context)?;
            Ok(arr.into())
        }
    }
}

fn deny_extension_ocap(
    tenant_id: String,
    target: String,
    custom_msg: Option<&str>,
    context: &mut boa_engine::Context,
) -> boa_engine::JsError {
    let violation = SandboxViolation::ExtensionDenied { tenant_id, target };
    let msg = custom_msg
        .map(str::to_owned)
        .unwrap_or_else(|| violation.to_string());
    record_boa_ocap_denial(violation);
    ocap_denied_js_error("ERR_SLICE_EXT_DENIED", msg, context)
}

pub(crate) fn register_extension_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{JsValue as BoaVal, NativeFunction, js_string};

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_ext_call"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let ext_name = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let method_name = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let target = format!("{ext_name}.{method_name}");
            let tenant_id = active_tenant_id_or_default();

            // 1. Deny-by-default OCap check (never reveals whether an ungranted extension exists).
            let authorized = BOA_ALLOWED_EXTENSIONS.with(|list| {
                list.borrow()
                    .iter()
                    .any(|grant| extension_grant_matches(&target, grant))
            });
            if !authorized {
                return Err(deny_extension_ocap(tenant_id, target, None, context));
            }

            // 2. Resolve registered extension & method descriptor.
            let ext_opt = BOA_ACTIVE_EXTENSIONS
                .with(|reg| reg.borrow().as_ref().and_then(|r| r.get(&ext_name).cloned()));
            let Some(ext) = ext_opt else {
                return Err(deny_extension_ocap(tenant_id, target, None, context));
            };
            let Some(method_spec) = ext.methods().iter().find(|m| m.name == method_name).copied()
            else {
                return Err(deny_extension_ocap(tenant_id, target, None, context));
            };

            // 3. Determinism guard: under deterministicSeed, only deterministic extensions may run.
            if BOA_DET_STATE.with(|d| d.get().is_some()) && !method_spec.deterministic {
                let msg = format!(
                    "OCap Violation: non-deterministic extension '{target}' is blocked under deterministicSeed"
                );
                return Err(deny_extension_ocap(tenant_id, target, Some(&msg), context));
            }

            // 4. Checkpoint region guard: honor allow_in_checkpoint = false.
            if !method_spec.allow_in_checkpoint && BOA_CHECKPOINT_STATS.with(|s| s.get().0 > 0) {
                let msg = format!(
                    "OCap Violation: extension '{target}' is not allowed inside a checkpointed region"
                );
                return Err(deny_extension_ocap(tenant_id, target, Some(&msg), context));
            }

            // 5. Copy & bound argument (JSON or bytes).
            let (arg_val, arg_bytes) =
                js_val_to_ext_value(args.get(2), method_spec.max_arg_bytes, context)?;

            // 6. Charge deterministic fuel (base + per-byte).
            let fuel_cost = method_spec.base_fuel_cost.saturating_add(
                (arg_bytes as u64).saturating_mul(method_spec.fuel_per_byte),
            );
            charge_extension_fuel(fuel_cost, context)?;

            // 7. Ring 4 Egress DLP scan on outgoing argument.
            scan_ext_arg_dlp(&tenant_id, &arg_val, context)?;

            // 8. Build ExtCallCtx with remaining wall-clock deadline & interrupt cancel token.
            let deadline = BOA_ACTIVE_DEADLINE.with(|d| d.get());
            let cancel = BOA_ACTIVE_INTERRUPT
                .with(|i| i.borrow().clone())
                .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
            if cancel.load(Ordering::Acquire) {
                let violation = SandboxViolation::ExecutionInterrupted { tenant_id };
                let msg = violation.to_string();
                record_boa_violation_once(violation);
                trip_isolate_oom_limits(context);
                return Err(boa_engine::JsNativeError::error().with_message(msg).into());
            }
            let remaining_ms = match deadline {
                Some((dl, limit_ms)) => {
                    let now = Instant::now();
                    if now >= dl {
                        cancel.store(true, Ordering::Release);
                        let violation = SandboxViolation::WallClockExceeded {
                            tenant_id,
                            limit_ms,
                        };
                        let msg = violation.to_string();
                        record_boa_violation_once(violation);
                        trip_isolate_oom_limits(context);
                        return Err(boa_engine::JsNativeError::error().with_message(msg).into());
                    }
                    Some(dl.saturating_duration_since(now).as_millis() as u64)
                }
                None => None,
            };

            let call_ctx = ExtCallCtx {
                tenant_id: tenant_id.clone(),
                remaining_ms,
                cancel: Arc::clone(&cancel),
            };

            // 9. Execute sync or async method.
            let call_res: Result<ExtValue, ExtError> = if method_spec.is_async {
                let fut = ext.call_async(&call_ctx, &method_name, arg_val);
                match poll_ext_future_bounded(fut, deadline, &cancel, &tenant_id) {
                    Ok(res) => res,
                    Err(violation) => {
                        let msg = violation.to_string();
                        record_boa_violation_once(violation);
                        trip_isolate_oom_limits(context);
                        return Err(boa_engine::JsNativeError::error().with_message(msg).into());
                    }
                }
            } else {
                ext.call(&call_ctx, &method_name, arg_val)
            };

            // 10. Convert result (copied + bounded + memory-charged) and wrap in Promise if async.
            let js_outcome: boa_engine::JsResult<BoaVal> = match call_res {
                Ok(val) => ext_value_to_js_val(val, method_spec.max_result_bytes, context),
                Err(ExtError::Denied(denied_target)) => {
                    Err(deny_extension_ocap(tenant_id, denied_target, None, context))
                }
                Err(err) => Err(boa_engine::JsNativeError::error()
                    .with_message(err.to_string())
                    .into()),
            };

            if method_spec.is_async {
                match js_outcome {
                    Ok(val) => {
                        Ok(boa_engine::object::builtins::JsPromise::resolve(val, context)?.into())
                    }
                    Err(err) => {
                        Ok(boa_engine::object::builtins::JsPromise::reject(err, context)?.into())
                    }
                }
            } else {
                js_outcome
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_ext_list_methods"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let ext_name = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let mut methods: Vec<String> = Vec::new();
            BOA_ACTIVE_EXTENSIONS.with(|reg| {
                if let Some(ext) = reg.borrow().as_ref().and_then(|r| r.get(&ext_name)) {
                    BOA_ALLOWED_EXTENSIONS.with(|grants| {
                        let g = grants.borrow();
                        for m in ext.methods() {
                            let target = format!("{ext_name}.{}", m.name);
                            if g.iter()
                                .any(|grant| extension_grant_matches(&target, grant))
                            {
                                methods.push(m.name.to_string());
                            }
                        }
                    });
                }
            });
            let arr = boa_engine::object::builtins::JsArray::from_iter(
                methods.into_iter().map(|m| BoaVal::from(js_string!(m))),
                context,
            );
            Ok(arr.into())
        }),
    );
}
