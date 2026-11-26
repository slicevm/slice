//! Post-Evaluation Event Loop Draining, SES Prototype Verification, Fuel/Memory Accounting,
//! and Return Value Coercion (`finish_boa_compartment_execution`).

use super::BoaSessionFlags;
use crate::boa_realm::{
    BOA_CHECKPOINT_STATS, BOA_CUSTOM_ARGV, BOA_DENIED_CAPABILITIES, BOA_DIRTY_SUBSYSTEMS,
    BOA_EXEC_DIRTY, BOA_FUEL_BUDGET, BOA_HTTP_REQ, BOA_HTTP_RES, BOA_KV_MEM_BUDGET,
    BOA_LAST_EXIT_CODE, BOA_LAST_OCAP_VIOLATION, BOA_PROCESS_EXIT, BOA_SES_INTL_SIG_PAIR,
    BOA_UNCAUGHT_ERROR, BOA_UNHANDLED_REJECTIONS, BOA_VIOLATION, BOA_WAIT_UNTIL_COUNT,
    DIRTY_PROCESS, EXEC_DIRTY_ARGV, EXEC_DIRTY_CHILD_PROC, EXEC_DIRTY_EXIT, EXEC_DIRTY_REJECTIONS,
    EXEC_DIRTY_UNCAUGHT, REALM_CTL_DISPATCH_HTTP_BRIDGE, REALM_CTL_RESET_MT, REALM_CTL_RESET_WU,
    REALM_CTL_SET_NEXT_TICK, SES_PROTO_NAMES, WarmBoaRealm, call_realm_ctl,
    first_ses_proto_sig_mismatch_idx, has_process_exit, has_uncaught_error, release_warm_boa_realm,
};
use crate::event_loop::{
    clear_event_loop_state, drive_event_loop_to_completion, has_pending_event_loop_work,
};
use crate::isolate::SliceIsolate;
use crate::value_and_ast::JsValue;
use slice_sandbox::SandboxViolation;

impl SliceIsolate {
    #[inline]
    pub(crate) fn finish_boa_compartment_execution(
        &mut self,
        mut realm: WarmBoaRealm,
        flags: BoaSessionFlags,
        is_http_req: bool,
        has_next_tick: bool,
        check_ses: bool,
        mut eval_res: boa_engine::JsResult<boa_engine::JsValue>,
    ) -> Result<JsValue, SandboxViolation> {
        if let Err(ref err) = eval_res
            && let Some(boa_engine::EngineError::RuntimeLimit(rl)) = err.as_engine()
        {
            let tenant_id = self.compartment.tenant_id.clone();
            let violation = match *rl {
                boa_engine::RuntimeLimitError::LoopIteration => {
                    self.compartment.fuel_consumed = self.compartment.fuel_limit;
                    Some(SandboxViolation::CpuFuelExhausted {
                        tenant_id,
                        limit: self.compartment.fuel_limit,
                    })
                }
                boa_engine::RuntimeLimitError::MemoryLimit { requested, .. } => {
                    let baseline = flags.initial_mem_used.min(4096);
                    Some(SandboxViolation::MemoryCeilingExceeded {
                        tenant_id,
                        requested: requested.saturating_add(baseline),
                        limit: self.compartment.memory_limit_bytes,
                    })
                }
                boa_engine::RuntimeLimitError::WallClock => {
                    Some(SandboxViolation::WallClockExceeded {
                        tenant_id,
                        limit_ms: self.max_wall_clock_ms.unwrap_or(0),
                    })
                }
                boa_engine::RuntimeLimitError::Interrupted => {
                    Some(SandboxViolation::ExecutionInterrupted { tenant_id })
                }
                _ => None,
            };
            if let Some(v) = violation {
                BOA_VIOLATION.with(|slot| {
                    if slot.borrow().is_none() {
                        *slot.borrow_mut() = Some(v);
                    }
                });
            }
        }

        if let Err(ref err) = eval_res
            && (BOA_EXEC_DIRTY.with(|c| c.get()) & EXEC_DIRTY_CHILD_PROC) != 0
        {
            let err_msg = err.to_string();
            let is_ocap_err = err_msg.contains("OCap Violation")
                || err_msg.contains("Slice Sandbox Violation")
                || err_msg.contains("NetAccessDenied:")
                || err_msg.contains("ERR_SLICE_");
            if is_ocap_err
                && let Some(cp_v) = BOA_LAST_OCAP_VIOLATION.with(|v| v.borrow_mut().take())
            {
                BOA_VIOLATION.with(|v| {
                    if v.borrow().is_none() {
                        *v.borrow_mut() = Some(cp_v);
                    }
                });
            }
        }

        if has_process_exit() {
            let _ = realm.ctx.run_jobs();
            if (BOA_EXEC_DIRTY.with(|c| c.get()) & EXEC_DIRTY_REJECTIONS) != 0 {
                BOA_UNHANDLED_REJECTIONS.with(|u| u.borrow_mut().clear());
            }
            eval_res = Ok(boa_engine::JsValue::undefined());
        } else if let Err(err) = eval_res {
            call_realm_ctl(&mut realm, REALM_CTL_RESET_MT, false);
            if BOA_VIOLATION.with(|v| v.borrow().is_none())
                && crate::event_loop::handle_uncaught_js_error(err, &mut realm.ctx)
            {
                eval_res = Ok(boa_engine::JsValue::undefined());
                if !has_process_exit()
                    && (!BOA_HTTP_REQ.with(|r| r.borrow().active)
                        || BOA_HTTP_RES.with(|res| res.borrow().is_none())
                        || has_pending_event_loop_work())
                {
                    drive_event_loop_to_completion(&mut realm.ctx);
                }
            } else {
                let _ = realm.ctx.run_jobs();
                if (BOA_EXEC_DIRTY.with(|c| c.get()) & EXEC_DIRTY_REJECTIONS) != 0 {
                    BOA_UNHANDLED_REJECTIONS.with(|u| u.borrow_mut().clear());
                }
                eval_res = Ok(boa_engine::JsValue::undefined());
            }
        } else if !BOA_HTTP_REQ.with(|r| r.borrow().active)
            || BOA_HTTP_RES.with(|res| res.borrow().is_none())
            || has_pending_event_loop_work()
        {
            drive_event_loop_to_completion(&mut realm.ctx);
        }

        if is_http_req
            && !has_process_exit()
            && !has_uncaught_error()
            && BOA_HTTP_RES.with(|res| res.borrow().is_none())
        {
            call_realm_ctl(&mut realm, REALM_CTL_DISPATCH_HTTP_BRIDGE, false);
            drive_event_loop_to_completion(&mut realm.ctx);
        }
        clear_event_loop_state();

        let loop_iters = realm.ctx.runtime_limits().loop_iterations_executed();
        let peak_boa_heap = realm.ctx.runtime_limits().peak_heap_bytes();
        {
            let limits = realm.ctx.runtime_limits_mut();
            limits.set_loop_iteration_limit(u64::MAX);
            limits.disable_heap_bytes_limit();
            limits.set_wall_clock_limit_ms(None);
            limits.set_recursion_limit(512);
        }
        realm.ctx.set_interrupt_flag(None);

        let mut final_val = match eval_res {
            Ok(mut val) => {
                if val.as_promise().is_some()
                    && let Ok(st_val) = realm
                        .ctx
                        .global_object()
                        .get(boa_engine::js_string!("__slice_tla_st"), &mut realm.ctx)
                {
                    let st = st_val.as_number().unwrap_or(0.0) as i32;
                    if st == 1 || st == 2 {
                        let _ = realm.ctx.global_object().set(
                            boa_engine::js_string!("__slice_tla_st"),
                            boa_engine::JsValue::from(0),
                            false,
                            &mut realm.ctx,
                        );
                        let tla_v = realm
                            .ctx
                            .global_object()
                            .get(boa_engine::js_string!("__slice_tla_v"), &mut realm.ctx)
                            .unwrap_or_default();
                        if st == 1 {
                            val = tla_v;
                        } else if !has_process_exit() {
                            let tla_err = boa_engine::JsError::from_opaque(tla_v);
                            let _ = crate::event_loop::handle_uncaught_js_error(
                                tla_err,
                                &mut realm.ctx,
                            );
                            val = boa_engine::JsValue::undefined();
                        } else {
                            val = boa_engine::JsValue::undefined();
                        }
                    }
                }
                val
            }
            Err(_) => boa_engine::JsValue::undefined(),
        };

        if (BOA_DIRTY_SUBSYSTEMS.with(|d| d.get()) & DIRTY_PROCESS) != 0 {
            crate::event_loop::finalize_and_reset_process_state(&mut realm.ctx);
        }

        self.teardown_boa_compartment_thread_locals(&mut realm, flags, has_next_tick);
        self.harvest_boa_stdout();
        let (cp_taken, cp_restored, cp_restore_ns) =
            BOA_CHECKPOINT_STATS.with(|s| s.replace((0, 0, 0)));
        self.checkpoints_taken = cp_taken;
        self.restores_performed = cp_restored;
        self.restore_time_ns = cp_restore_ns;

        let mut pending_violation = BOA_VIOLATION.with(|v| v.borrow_mut().take());
        if let Some(
            [
                (pre_str, post_str),
                (pre_num, post_num),
                (pre_date, post_date),
            ],
        ) = BOA_SES_INTL_SIG_PAIR.with(|p| p.replace(None))
        {
            if realm.ses_proto_sigs[3] == pre_str {
                realm.ses_proto_sigs[3] = post_str;
            }
            if realm.ses_proto_sigs[6] == pre_num {
                realm.ses_proto_sigs[6] = post_num;
            }
            if realm.ses_proto_sigs[12] == pre_date {
                realm.ses_proto_sigs[12] = post_date;
            }
            realm.loaded_shards_mask |= 1 << 18;
        }
        let mut ses_mutated_target = String::new();
        if let Some(mut mismatch_idx) = first_ses_proto_sig_mismatch_idx(&realm) {
            // If `Promise.prototype` (index 4) changed solely because guest code dynamically
            // invoked `process.nextTick` (activating `__wrappedThen`), unhook cleanly and re-verify.
            if mismatch_idx == 4 && !has_next_tick {
                call_realm_ctl(&mut realm, REALM_CTL_SET_NEXT_TICK, false);
                if let Some(recheck_idx) = first_ses_proto_sig_mismatch_idx(&realm) {
                    mismatch_idx = recheck_idx;
                    ses_mutated_target.push_str(SES_PROTO_NAMES[mismatch_idx]);
                }
            } else {
                ses_mutated_target.push_str(SES_PROTO_NAMES[mismatch_idx]);
            }
        } else if (check_ses || pending_violation.is_some())
            && let Some(check_fn) = realm.ses_check_fn.clone()
            && let Ok(v) = check_fn.call(&boa_engine::JsValue::undefined(), &[], &mut realm.ctx)
            && let Some(s) = v.as_string()
        {
            let st = s.to_std_string_escaped();
            if !st.is_empty() {
                ses_mutated_target = st;
            }
        }

        let final_exec_dirty = BOA_EXEC_DIRTY.with(|c| c.replace(0));
        let (exit_code_opt, uncaught_err_opt, last_ocap_violation) = if final_exec_dirty == 0 {
            self.last_exit_code = None;
            self.last_process_exited = false;
            self.denied_capabilities.clear();
            (None, None, None)
        } else {
            if (final_exec_dirty & EXEC_DIRTY_ARGV) != 0 {
                BOA_CUSTOM_ARGV.with(|a| *a.borrow_mut() = None);
            }
            let last_ocap = if (final_exec_dirty & EXEC_DIRTY_CHILD_PROC) != 0 {
                self.denied_capabilities =
                    BOA_DENIED_CAPABILITIES.with(|d| std::mem::take(&mut *d.borrow_mut()));
                BOA_LAST_OCAP_VIOLATION.with(|v| v.borrow_mut().take())
            } else {
                self.denied_capabilities.clear();
                None
            };
            let (exit_opt, process_exited) = if (final_exec_dirty & EXEC_DIRTY_EXIT) != 0 {
                // Drain BOTH cells unconditionally so calling `process.exit(code)` after setting
                // `process.exitCode = other` never leaves `BOA_LAST_EXIT_CODE` stale on the thread.
                let abrupt_exit_opt = BOA_PROCESS_EXIT.with(|e| e.replace(None));
                let graceful_exit_opt = BOA_LAST_EXIT_CODE.with(|e| e.replace(None));
                (
                    abrupt_exit_opt.or(graceful_exit_opt),
                    abrupt_exit_opt.is_some(),
                )
            } else {
                (None, false)
            };
            self.last_exit_code = exit_opt;
            self.last_process_exited = process_exited;
            let err_opt = if (final_exec_dirty & EXEC_DIRTY_UNCAUGHT) != 0 {
                BOA_UNCAUGHT_ERROR.with(|e| e.borrow_mut().take())
            } else {
                None
            };
            if (final_exec_dirty & EXEC_DIRTY_REJECTIONS) != 0 {
                BOA_UNHANDLED_REJECTIONS.with(|u| u.borrow_mut().clear());
            }
            (exit_opt, err_opt, last_ocap)
        };
        if pending_violation.is_none()
            && let Some(ref err_str) = uncaught_err_opt
            && (err_str.contains("OCap Violation")
                || err_str.contains("Slice Sandbox Violation")
                || err_str.contains("NetAccessDenied:")
                || err_str.contains("ERR_SLICE_"))
            && let Some(ocap_v) = last_ocap_violation
        {
            pending_violation = Some(ocap_v);
        }

        let wu_queued = BOA_WAIT_UNTIL_COUNT.with(|c| c.get());
        let clear_wu_and_release = |mut r: WarmBoaRealm| {
            BOA_FUEL_BUDGET.with(|f| f.set((0, u64::MAX)));
            BOA_KV_MEM_BUDGET.with(|m| m.set((0, usize::MAX)));
            if wu_queued > 0 || r.has_pending_wu {
                call_realm_ctl(&mut r, REALM_CTL_RESET_WU, false);
                r.has_pending_wu = false;
                BOA_WAIT_UNTIL_COUNT.with(|c| c.set(0));
            }
            release_warm_boa_realm(r);
        };

        if !ses_mutated_target.is_empty() {
            // Quarantine: never return a realm whose SES intrinsics were mutated back to BOA_CONTEXT_POOL.
            BOA_FUEL_BUDGET.with(|f| f.set((0, u64::MAX)));
            BOA_KV_MEM_BUDGET.with(|m| m.set((0, usize::MAX)));
            BOA_WAIT_UNTIL_COUNT.with(|c| c.set(0));
            drop(realm);
            return Err(SandboxViolation::IntrinsicMutationBlocked {
                tenant_id: self.compartment.tenant_id.clone(),
                intrinsic: ses_mutated_target,
            });
        }
        let initial_mem_used = flags.initial_mem_used;
        let baseline_mem_used = initial_mem_used.min(4096);
        let initial_net_used = initial_mem_used.saturating_sub(baseline_mem_used);
        if let Some(mut violation) = pending_violation {
            let final_net_used = BOA_KV_MEM_BUDGET.with(|m| m.replace((0, usize::MAX)).0);
            let combined_peak = final_net_used.max(peak_boa_heap);
            if combined_peak > initial_net_used {
                self.compartment.memory_used_bytes =
                    baseline_mem_used.saturating_add(combined_peak);
            }
            if let SandboxViolation::CpuFuelExhausted { ref mut limit, .. } = violation
                && *limit == 0
            {
                self.compartment.fuel_consumed = self.compartment.fuel_limit;
                *limit = self.compartment.fuel_limit;
            }
            clear_wu_and_release(realm);
            return Err(violation);
        }
        let wasm_fuel = BOA_FUEL_BUDGET.with(|f| f.replace((0, u64::MAX)).0);
        if wasm_fuel > 0
            && let Err(v) = self.compartment.consume_fuel(wasm_fuel)
        {
            clear_wu_and_release(realm);
            return Err(v);
        }
        if loop_iters > 0
            && let Err(v) = self.compartment.consume_fuel(loop_iters)
        {
            self.compartment.fuel_consumed = self.compartment.fuel_limit;
            clear_wu_and_release(realm);
            return Err(v);
        }
        let final_net_used = BOA_KV_MEM_BUDGET.with(|m| m.replace((0, usize::MAX)).0);
        let combined_used = final_net_used.max(peak_boa_heap);
        if combined_used > initial_net_used {
            let added = combined_used - initial_net_used;
            if let Err(v) = self.compartment.alloc_memory(added) {
                self.compartment.memory_used_bytes =
                    baseline_mem_used.saturating_add(combined_used);
                clear_wu_and_release(realm);
                return Err(v);
            }
        } else {
            self.compartment.memory_used_bytes = baseline_mem_used.saturating_add(combined_used);
        }

        self.last_return_json = None;
        if let Some(err_str) = uncaught_err_opt {
            clear_wu_and_release(realm);
            return Ok(JsValue::str(err_str));
        }
        if !is_http_req && (wu_queued > 0 || realm.has_pending_wu) {
            call_realm_ctl(&mut realm, REALM_CTL_RESET_WU, false);
            realm.has_pending_wu = false;
            BOA_WAIT_UNTIL_COUNT.with(|c| c.set(0));
        } else if is_http_req && wu_queued > 0 {
            realm.has_pending_wu = true;
        }
        if exit_code_opt.is_some() && final_val.as_promise().is_some() {
            final_val = boa_engine::JsValue::undefined();
        }
        let result = if final_val.is_undefined() {
            Ok(JsValue::Undefined)
        } else if final_val.is_null() {
            if self.capture_return_json {
                self.last_return_json = Some("null".to_string());
            }
            Ok(JsValue::Null)
        } else if let Some(b) = final_val.as_boolean() {
            if self.capture_return_json {
                self.last_return_json = Some(if b { "true" } else { "false" }.to_string());
            }
            Ok(JsValue::Bool(b))
        } else if let Some(n) = final_val.as_number() {
            if self.capture_return_json && n.is_finite() {
                self.last_return_json = Some(JsValue::Number(n).to_string());
            }
            Ok(JsValue::Number(n))
        } else if let Some(s) = final_val.as_string() {
            let st = s.to_std_string_escaped();
            if self.capture_return_json {
                self.last_return_json = serde_json::to_string(&st).ok();
            }
            Ok(JsValue::str(st))
        } else {
            if self.capture_return_json
                && let Some(obj) = final_val.as_object()
                && !obj.is_callable()
                && let Ok(Some(json_val)) = final_val.to_json(&mut realm.ctx)
            {
                self.last_return_json = serde_json::to_string(&json_val).ok();
            }
            if let Ok(s) = final_val.to_string(&mut realm.ctx) {
                Ok(JsValue::str(s.to_std_string_escaped()))
            } else {
                Ok(JsValue::Undefined)
            }
        };

        release_warm_boa_realm(realm);
        result
    }
}
