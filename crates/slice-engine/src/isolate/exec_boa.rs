//! Full ES2024+ `boa_engine` and Precompiled `BoaCAS` (`SLICEBC\x01`) Execution,
//! Warm Realm Pool Management, and Shared OCap/DLP/SES Compartment Lifecycle Guards.

use crate::boa_realm::{
    acquire_warm_boa_realm, call_realm_ctl, create_bare_boa_realm, create_warm_boa_realm,
    ensure_realm_shards, eval_or_reuse_cached_script, has_process_exit, has_uncaught_error,
    mark_exec_dirty, release_warm_boa_realm, try_acquire_pooled_boa_realm, WarmBoaRealm,
    BOA_ACTIVE_CAPS, BOA_ACTIVE_TENANT, BOA_CHECKPOINT_STATS, BOA_CONTEXT_POOL, BOA_CUSTOM_ARGV,
    BOA_DET_STATE, BOA_DIRTY_SUBSYSTEMS, BOA_EGRESS_SECRETS, BOA_ENV_MAP, BOA_EXEC_BINS,
    BOA_EXEC_DIRTY, BOA_FETCH_MOCKS, BOA_FUEL_BUDGET, BOA_HTTP_REQ, BOA_HTTP_RES,
    BOA_KV_MEM_BUDGET, BOA_LAST_CHILD_PROC_VIOLATION, BOA_LAST_EXIT_CODE, BOA_NET_HOSTS,
    BOA_PROCESS_EXIT, BOA_STDOUT, BOA_UNCAUGHT_ERROR, BOA_UNHANDLED_REJECTIONS, BOA_VIOLATION,
    BOA_WAIT_UNTIL_COUNT, CHECKPOINT_SLAB, DIRTY_FETCH_MOCK, DIRTY_SOCKETS, DIRTY_WASM,
    DIRTY_WEBSOCKET, DIRTY_WORKERS, EXEC_DIRTY_ARGV, EXEC_DIRTY_CHILD_PROC, EXEC_DIRTY_EXIT,
    EXEC_DIRTY_REJECTIONS, EXEC_DIRTY_UNCAUGHT, REALM_CTL_DISPATCH_HTTP_BRIDGE, REALM_CTL_RESET_MT,
    REALM_CTL_RESET_WU, REALM_CTL_SET_DET_MODE, REALM_CTL_SET_NEXT_TICK, REALM_CTL_SYNC_ENV,
};
use crate::boacas::{
    BOACAS_FLAG_HAS_HTTP_BRIDGE, BOACAS_FLAG_HAS_KV_SANDBOX, BOACAS_FLAG_HAS_NEXT_TICK,
    BOACAS_FLAG_HAS_SEMVER_FAST, BOACAS_FLAG_HAS_WORKER, BOACAS_FLAG_NEEDS_SES_CHECK,
};
use crate::event_loop::{
    clear_event_loop_state, drive_event_loop_to_completion, has_pending_event_loop_work,
};
use crate::isolate::SliceIsolate;
use crate::js_preludes::{detect_needed_shards, needs_lazy_global_getters};
use crate::parser_and_compiler::extract_slice_rc_key;
use crate::value_and_ast::JsValue;
use slice_sandbox::SandboxViolation;

#[derive(Clone, Copy)]
pub(crate) struct BoaSessionFlags {
    pub(crate) has_fs_restrictions: bool,
    pub(crate) has_net_hosts: bool,
    pub(crate) has_egress_secrets: bool,
    pub(crate) has_env_keys: bool,
    pub(crate) has_exec_bins: bool,
    pub(crate) is_worker: bool,
}

impl SliceIsolate {
    /// Pre-warms `count` full-ES2024+ `boa_engine::Context` realms on the current worker thread
    /// with all native Rust host intrinsics (`__slice_console_log`, `__slice_path_join`, `__slice_fs_*`,
    /// `__slice_crypto_sha256`, `__slice_os_tmpdir`) and the `node:*` module loader already evaluated.
    pub fn prewarm_boa_pool(count: usize) {
        BOA_CONTEXT_POOL.with(|pool| {
            let mut pool_borrow = pool.borrow_mut();
            while pool_borrow.len() < count {
                pool_borrow.push(create_warm_boa_realm());
            }
        });
    }

    /// Pre-warms a `boa_engine::Context` realm in `BOA_CONTEXT_POOL` and pre-populates its
    /// Persistent Realm Module Cache (`globalThis.__slice_rc`) for `bytecode`, so subsequent
    /// warm isolate executions skip `100%` of JS lexing/parsing and `node_modules` CJS evaluation.
    pub fn prewarm_boa_bundle(bytecode: &[u8]) {
        if let Some((hdr, precompiled_slice, fallback_js)) =
            crate::boacas::BoaCasHeader::decode(bytecode)
        {
            if crate::boacas::is_exper_boacas_runtime_enabled() {
                if (hdr.feature_flags & crate::boacas::BOACAS_FLAG_HAS_WORKER) != 0 {
                    crate::workers::prewarm_worker_pool_sync(1);
                }
                if (hdr.feature_flags & crate::boacas::BOACAS_FLAG_IS_RC_BUNDLE) != 0 {
                    let (mut realm, is_cold_bare_realm) = match try_acquire_pooled_boa_realm() {
                        Some(r) => (r, false),
                        None => (create_bare_boa_realm(), true),
                    };
                    if !realm.loaded_bundles.contains_key(&hdr.rc_key) {
                        if (hdr.feature_flags & crate::boacas::BOACAS_FLAG_HAS_SEMVER_FAST) != 0 {
                            crate::host_bindings::register_semver_and_fast_intrinsics_host_functions(
                                &mut realm.ctx,
                            );
                        }
                        if (hdr.feature_flags & crate::boacas::BOACAS_FLAG_HAS_KV_SANDBOX) != 0 {
                            crate::host_bindings::register_kv_and_sandbox_host_functions(
                                &mut realm.ctx,
                            );
                        }
                        let total_end = crate::boacas::BOACAS_HEADER_LEN
                            + (hdr.precompiled_len as usize)
                            + (hdr.fallback_js_len as usize);
                        let contiguous_payload =
                            &bytecode[crate::boacas::BOACAS_HEADER_LEN..total_end];
                        let _ = crate::boacas::eval_boacas_on_realm(
                            &mut realm,
                            &hdr,
                            precompiled_slice,
                            fallback_js,
                            contiguous_payload,
                            false,
                            is_cold_bare_realm,
                        );
                        let _ = realm.ctx.run_jobs();
                        clear_event_loop_state();
                        crate::websocket::clear_thread_websocket_state();
                        crate::sockets::clear_thread_socket_state();
                        crate::workers::clear_thread_worker_state();
                        CHECKPOINT_SLAB.with(|s| s.borrow_mut().clear());
                        BOA_CHECKPOINT_STATS.with(|s| s.set((0, 0, 0)));
                        BOA_DIRTY_SUBSYSTEMS.with(|d| d.set(0));
                        if BOA_WAIT_UNTIL_COUNT.with(|c| c.get()) > 0 {
                            call_realm_ctl(&mut realm, REALM_CTL_RESET_WU, false);
                        }
                        BOA_STDOUT.with(|out| out.borrow_mut().clear());
                        BOA_HTTP_RES.with(|res| *res.borrow_mut() = None);
                        BOA_WAIT_UNTIL_COUNT.with(|c| c.set(0));
                    }
                    release_warm_boa_realm(realm);
                    return;
                }
            }
        }

        Self::prewarm_boa_pool(1);
        let js_slice_opt = if bytecode.starts_with(b"SLICEBOA") {
            std::str::from_utf8(&bytecode[8..]).ok()
        } else if let Some((_, _, fallback_js)) = crate::boacas::BoaCasHeader::decode(bytecode) {
            Some(fallback_js)
        } else {
            None
        };
        if let Some(js_code) = js_slice_opt {
            if js_code.contains("Worker") || js_code.contains("worker_threads") {
                crate::workers::prewarm_worker_pool_sync(1);
            }
            if let Some((rc_key_u64, hex_bytes)) = extract_slice_rc_key(js_code) {
                let mut realm = acquire_warm_boa_realm();
                if !realm.loaded_bundles.contains_key(&rc_key_u64) {
                    if js_code.contains("__slice_semver_") || js_code.contains("__slice_fast_") {
                        crate::host_bindings::register_semver_and_fast_intrinsics_host_functions(
                            &mut realm.ctx,
                        );
                    }
                    if js_code.contains("Slice.kv")
                        || js_code.contains("Slice.checkpoint")
                        || js_code.contains("Slice.restore")
                        || js_code.contains("Slice.releaseCheckpoint")
                        || js_code.contains("Slice.evalSandbox")
                    {
                        crate::host_bindings::register_kv_and_sandbox_host_functions(
                            &mut realm.ctx,
                        );
                    }
                    ensure_realm_shards(
                        &mut realm,
                        detect_needed_shards(js_code, false),
                        needs_lazy_global_getters(js_code),
                    );
                    let _ = realm
                        .ctx
                        .eval(boa_engine::Source::from_bytes(js_code.as_bytes()));
                    if let Ok(hex_str) = std::str::from_utf8(hex_bytes) {
                        let global = realm.ctx.global_object();
                        if let Ok(rc_val) =
                            global.get(boa_engine::js_string!("__slice_rc"), &mut realm.ctx)
                        {
                            if let Some(rc_obj) = rc_val.as_object() {
                                if let Ok(val) =
                                    rc_obj.get(boa_engine::js_string!(hex_str), &mut realm.ctx)
                                {
                                    if let Some(obj) = val.as_object() {
                                        realm.loaded_bundles.insert(rc_key_u64, obj.clone());
                                    }
                                }
                            }
                        }
                    }
                    let _ = realm.ctx.run_jobs();
                    clear_event_loop_state();
                    crate::websocket::clear_thread_websocket_state();
                    crate::sockets::clear_thread_socket_state();
                    crate::workers::clear_thread_worker_state();
                    CHECKPOINT_SLAB.with(|s| s.borrow_mut().clear());
                    BOA_CHECKPOINT_STATS.with(|s| s.set((0, 0, 0)));
                    BOA_DIRTY_SUBSYSTEMS.with(|d| d.set(0));
                    if BOA_WAIT_UNTIL_COUNT.with(|c| c.get()) > 0 {
                        call_realm_ctl(&mut realm, REALM_CTL_RESET_WU, false);
                    }
                    BOA_STDOUT.with(|out| out.borrow_mut().clear());
                    BOA_HTTP_RES.with(|res| *res.borrow_mut() = None);
                    BOA_WAIT_UNTIL_COUNT.with(|c| c.set(0));
                }
                release_warm_boa_realm(realm);
            }
        }
    }

    /// Returns the number of pre-warmed `boa_engine::Context` realms currently available in the thread-local pool.
    #[inline]
    pub fn boa_pool_available() -> usize {
        BOA_CONTEXT_POOL.with(|pool| pool.borrow().len())
    }

    #[inline]
    pub(crate) fn setup_boa_compartment_thread_locals(&self) -> BoaSessionFlags {
        BOA_STDOUT.with(|out| out.borrow_mut().clear());
        BOA_VIOLATION.with(|v| *v.borrow_mut() = None);
        BOA_DIRTY_SUBSYSTEMS.with(|d| d.set(0));
        let stale_exec_dirty = BOA_EXEC_DIRTY.with(|c| c.replace(0));
        if stale_exec_dirty != 0 {
            if (stale_exec_dirty & EXEC_DIRTY_EXIT) != 0 {
                BOA_PROCESS_EXIT.with(|e| e.set(None));
                BOA_LAST_EXIT_CODE.with(|e| e.set(None));
            }
            if (stale_exec_dirty & EXEC_DIRTY_REJECTIONS) != 0 {
                BOA_UNHANDLED_REJECTIONS.with(|u| u.borrow_mut().clear());
            }
            if (stale_exec_dirty & EXEC_DIRTY_UNCAUGHT) != 0 {
                BOA_UNCAUGHT_ERROR.with(|e| *e.borrow_mut() = None);
            }
            if (stale_exec_dirty & EXEC_DIRTY_CHILD_PROC) != 0 {
                BOA_LAST_CHILD_PROC_VIOLATION.with(|v| *v.borrow_mut() = None);
            }
            if (stale_exec_dirty & EXEC_DIRTY_ARGV) != 0 {
                BOA_CUSTOM_ARGV.with(|a| *a.borrow_mut() = None);
            }
        }
        if !self.process_argv.is_empty() {
            mark_exec_dirty(EXEC_DIRTY_ARGV);
            BOA_CUSTOM_ARGV.with(|a| *a.borrow_mut() = Some(self.process_argv.clone()));
        }
        BOA_ACTIVE_TENANT.with(|t| {
            let mut b = t.borrow_mut();
            b.clear();
            b.push_str(&self.compartment.tenant_id);
        });
        let remaining_mem = self
            .compartment
            .memory_limit_bytes
            .saturating_sub(self.compartment.memory_used_bytes);
        BOA_KV_MEM_BUDGET.with(|m| m.set((0, remaining_mem)));
        crate::boa_realm::BOA_CHECKPOINT_STATS.with(|s| {
            s.set((
                self.checkpoints_taken,
                self.restores_performed,
                self.restore_time_ns,
            ));
        });
        let allow_cp = self.compartment.capabilities.allow_checkpoints;
        let max_cp = self
            .compartment
            .capabilities
            .max_checkpoints
            .unwrap_or(u32::MAX);
        crate::boa_realm::BOA_CHECKPOINT_CAP.with(|c| c.set((allow_cp, max_cp)));
        if let Some(seed) = self.deterministic_seed {
            BOA_DET_STATE.with(|s| s.set(Some((seed ^ 0x9E37_79B9_7F4A_7C15, 1_700_000_000_000))));
        }

        let has_fs_restrictions = !self
            .compartment
            .capabilities
            .allowed_read_prefixes
            .is_empty()
            || !self
                .compartment
                .capabilities
                .allowed_write_prefixes
                .is_empty();
        if has_fs_restrictions {
            let read_prefs = if self
                .compartment
                .capabilities
                .allowed_read_prefixes
                .is_empty()
            {
                vec!["/__slice_denied__".to_string()]
            } else {
                self.compartment.capabilities.allowed_read_prefixes.clone()
            };
            let write_prefs = if self
                .compartment
                .capabilities
                .allowed_write_prefixes
                .is_empty()
            {
                vec!["/__slice_denied__".to_string()]
            } else {
                self.compartment.capabilities.allowed_write_prefixes.clone()
            };
            BOA_ACTIVE_CAPS.with(|c| {
                *c.borrow_mut() =
                    Some((self.compartment.tenant_id.clone(), read_prefs, write_prefs));
            });
        }
        let has_net_hosts = !self.compartment.capabilities.allowed_net_hosts.is_empty();
        if has_net_hosts {
            BOA_NET_HOSTS.with(|h| {
                *h.borrow_mut() = self.compartment.capabilities.allowed_net_hosts.clone();
            });
        }
        let has_egress_secrets = !self.egress_audit_secrets.is_empty();
        if has_egress_secrets {
            BOA_EGRESS_SECRETS.with(|s| {
                *s.borrow_mut() = self.egress_audit_secrets.clone();
            });
        }
        let has_env_keys = !self.compartment.capabilities.allowed_env_keys.is_empty();
        if has_env_keys {
            BOA_ENV_MAP.with(|m| {
                *m.borrow_mut() = self.compartment.capabilities.allowed_env_keys.clone();
            });
        }
        let has_exec_bins = !self
            .compartment
            .capabilities
            .allowed_exec_binaries
            .is_empty();
        if has_exec_bins {
            BOA_EXEC_BINS.with(|b| {
                *b.borrow_mut() = self.compartment.capabilities.allowed_exec_binaries.clone();
            });
        }

        BoaSessionFlags {
            has_fs_restrictions,
            has_net_hosts,
            has_egress_secrets,
            has_env_keys,
            has_exec_bins,
            is_worker: crate::workers::is_worker_thread(),
        }
    }

    #[inline]
    pub(crate) fn apply_boa_realm_runtime_limits(&self, realm: &mut WarmBoaRealm) {
        let remaining_fuel = self
            .compartment
            .fuel_limit
            .saturating_sub(self.compartment.fuel_consumed);
        BOA_FUEL_BUDGET.with(|f| f.set((0, remaining_fuel.max(1))));
        realm
            .ctx
            .runtime_limits_mut()
            .set_loop_iteration_limit(remaining_fuel);
        realm.ctx.runtime_limits_mut().set_recursion_limit(512);
    }

    #[inline]
    pub(crate) fn teardown_boa_compartment_thread_locals(
        &self,
        realm: &mut WarmBoaRealm,
        flags: BoaSessionFlags,
        has_next_tick: bool,
    ) {
        if self.deterministic_seed.is_some() {
            call_realm_ctl(realm, REALM_CTL_SET_DET_MODE, false);
            BOA_DET_STATE.with(|s| s.set(None));
        }
        if flags.has_fs_restrictions {
            BOA_ACTIVE_CAPS.with(|c| *c.borrow_mut() = None);
        }
        if flags.has_net_hosts {
            BOA_NET_HOSTS.with(|h| h.borrow_mut().clear());
        }
        if flags.has_egress_secrets {
            BOA_EGRESS_SECRETS.with(|s| s.borrow_mut().clear());
        }
        if flags.has_env_keys {
            call_realm_ctl(realm, REALM_CTL_SYNC_ENV, false);
            BOA_ENV_MAP.with(|m| m.borrow_mut().clear());
        }
        if flags.has_exec_bins {
            BOA_EXEC_BINS.with(|b| b.borrow_mut().clear());
        }
        if has_next_tick {
            call_realm_ctl(realm, REALM_CTL_SET_NEXT_TICK, false);
        }
        let dirty = BOA_DIRTY_SUBSYSTEMS.with(|d| d.replace(0));
        if (dirty & DIRTY_FETCH_MOCK) != 0 {
            BOA_FETCH_MOCKS.with(|m| m.borrow_mut().clear());
        }
        if (dirty & DIRTY_WEBSOCKET) != 0 {
            crate::websocket::clear_thread_websocket_state();
        }
        if (dirty & DIRTY_SOCKETS) != 0 {
            crate::sockets::clear_thread_socket_state();
        }
        if (dirty & DIRTY_WORKERS) != 0 {
            crate::workers::clear_thread_worker_state();
        }
        if (dirty & DIRTY_WASM) != 0 {
            crate::wasm::clear_thread_wasm_state();
        }
    }

    #[inline]
    pub(crate) fn harvest_boa_stdout(&mut self) {
        BOA_STDOUT.with(|out| {
            let mut captured = out.borrow_mut();
            if !captured.is_empty() {
                self.stdout.reserve(captured.len());
                for line in captured.drain(..) {
                    if memchr::memchr(b'\n', line.as_bytes()).is_some() {
                        for subline in line.lines() {
                            self.stdout.push(subline.to_string());
                        }
                    } else {
                        self.stdout.push(line);
                    }
                }
            }
        });
    }

    pub(crate) fn finish_boa_compartment_execution(
        &mut self,
        mut realm: WarmBoaRealm,
        flags: BoaSessionFlags,
        is_http_req: bool,
        has_next_tick: bool,
        check_ses: bool,
        mut eval_res: boa_engine::JsResult<boa_engine::JsValue>,
    ) -> Result<JsValue, SandboxViolation> {
        if eval_res.is_err() && (BOA_EXEC_DIRTY.with(|c| c.get()) & EXEC_DIRTY_CHILD_PROC) != 0 {
            if let Some(cp_v) = BOA_LAST_CHILD_PROC_VIOLATION.with(|v| v.borrow_mut().take()) {
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
        realm
            .ctx
            .runtime_limits_mut()
            .set_loop_iteration_limit(u64::MAX);

        let mut ses_mutated_target = String::new();
        if check_ses {
            if let Some(ref check_fn) = realm.ses_check_fn {
                if let Ok(v) = check_fn.call(&boa_engine::JsValue::undefined(), &[], &mut realm.ctx)
                {
                    if let Some(s) = v.as_string() {
                        let st = s.to_std_string_escaped();
                        if !st.is_empty() {
                            ses_mutated_target = st;
                        }
                    }
                }
            }
        }

        let mut final_val = match eval_res {
            Ok(mut val) => {
                if val.as_promise().is_some() {
                    if let Ok(st_val) = realm
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
                }
                val
            }
            Err(_) => boa_engine::JsValue::undefined(),
        };

        if (BOA_DIRTY_SUBSYSTEMS.with(|d| d.get()) & crate::boa_realm::DIRTY_PROCESS) != 0 {
            crate::event_loop::finalize_and_reset_process_state(&mut realm.ctx);
        }

        self.teardown_boa_compartment_thread_locals(&mut realm, flags, has_next_tick);
        self.harvest_boa_stdout();
        let (cp_taken, cp_restored, cp_restore_ns) =
            BOA_CHECKPOINT_STATS.with(|s| s.replace((0, 0, 0)));
        self.checkpoints_taken = cp_taken;
        self.restores_performed = cp_restored;
        self.restore_time_ns = cp_restore_ns;

        let final_exec_dirty = BOA_EXEC_DIRTY.with(|c| c.replace(0));
        let (exit_code_opt, uncaught_err_opt) = if final_exec_dirty == 0 {
            self.last_exit_code = None;
            (None, None)
        } else {
            if (final_exec_dirty & EXEC_DIRTY_ARGV) != 0 {
                BOA_CUSTOM_ARGV.with(|a| *a.borrow_mut() = None);
            }
            if (final_exec_dirty & EXEC_DIRTY_CHILD_PROC) != 0 {
                BOA_LAST_CHILD_PROC_VIOLATION.with(|v| *v.borrow_mut() = None);
            }
            let exit_opt = if (final_exec_dirty & EXEC_DIRTY_EXIT) != 0 {
                BOA_PROCESS_EXIT
                    .with(|e| e.replace(None))
                    .or_else(|| BOA_LAST_EXIT_CODE.with(|e| e.replace(None)))
            } else {
                None
            };
            self.last_exit_code = exit_opt;
            let err_opt = if (final_exec_dirty & EXEC_DIRTY_UNCAUGHT) != 0 {
                BOA_UNCAUGHT_ERROR.with(|e| e.borrow_mut().take())
            } else {
                None
            };
            if (final_exec_dirty & EXEC_DIRTY_REJECTIONS) != 0 {
                BOA_UNHANDLED_REJECTIONS.with(|u| u.borrow_mut().clear());
            }
            (exit_opt, err_opt)
        };

        if let Some(mut violation) = BOA_VIOLATION.with(|v| v.borrow_mut().take()) {
            if let SandboxViolation::CpuFuelExhausted { ref mut limit, .. } = violation {
                if *limit == 0 {
                    self.compartment.fuel_consumed = self.compartment.fuel_limit;
                    *limit = self.compartment.fuel_limit;
                }
            }
            release_warm_boa_realm(realm);
            return Err(violation);
        }
        let wasm_fuel = BOA_FUEL_BUDGET.with(|f| f.get().0);
        if wasm_fuel > 0 {
            if let Err(v) = self.compartment.consume_fuel(wasm_fuel) {
                release_warm_boa_realm(realm);
                return Err(v);
            }
        }
        if loop_iters > 0 {
            if let Err(v) = self.compartment.consume_fuel(loop_iters) {
                self.compartment.fuel_consumed = self.compartment.fuel_limit;
                release_warm_boa_realm(realm);
                return Err(v);
            }
        }
        if !ses_mutated_target.is_empty() {
            // Do NOT return a realm whose SES intrinsics were mutated back to BOA_CONTEXT_POOL.
            drop(realm);
            return Err(SandboxViolation::IntrinsicMutationBlocked {
                tenant_id: self.compartment.tenant_id.clone(),
                intrinsic: ses_mutated_target,
            });
        }
        let kv_used = BOA_KV_MEM_BUDGET.with(|m| m.get().0);
        if kv_used > 0 {
            if let Err(v) = self.compartment.alloc_memory(kv_used) {
                self.compartment.memory_used_bytes =
                    self.compartment.memory_used_bytes.saturating_add(kv_used);
                release_warm_boa_realm(realm);
                return Err(v);
            }
        }

        self.last_return_json = None;
        if let Some(err_str) = uncaught_err_opt {
            release_warm_boa_realm(realm);
            return Ok(JsValue::str(err_str));
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
            if self.capture_return_json {
                if let Some(obj) = final_val.as_object() {
                    if !obj.is_callable() {
                        if let Ok(Some(json_val)) = final_val.to_json(&mut realm.ctx) {
                            self.last_return_json = serde_json::to_string(&json_val).ok();
                        }
                    }
                }
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

    /// Executes full ES2024+ JavaScript inside the production **Boa (`boa_engine`)** ECMAScript VM,
    /// reusing pre-warmed contexts from `BOA_CONTEXT_POOL`, capturing `console.log` / `print` output
    /// via zero-eval native host hooks, and enforcing SES compartment limits.
    pub(crate) fn exec_boa_full_es_vm(
        &mut self,
        js_code: &str,
    ) -> Result<JsValue, SandboxViolation> {
        let _active_src_guard = crate::boa_realm::ActiveSourceSliceGuard::new(js_code);
        let flags = self.setup_boa_compartment_thread_locals();

        if !flags.is_worker && (js_code.contains("Worker") || js_code.contains("worker_threads")) {
            crate::workers::speculative_prewarm_worker_pool(1);
        }

        let mut realm = acquire_warm_boa_realm();

        if flags.has_env_keys {
            call_realm_ctl(&mut realm, REALM_CTL_SYNC_ENV, true);
        }
        if self.deterministic_seed.is_some() {
            crate::host_bindings::register_deterministic_host_functions(&mut realm.ctx);
            call_realm_ctl(&mut realm, REALM_CTL_SET_DET_MODE, true);
        }
        let has_next_tick = js_code.contains("nextTick");
        if has_next_tick {
            call_realm_ctl(&mut realm, REALM_CTL_SET_NEXT_TICK, true);
        }
        if flags.is_worker {
            ensure_realm_shards(&mut realm, (1 << 8) | (1 << 13) | (1 << 2), false);
            if let Some(ref init_fn) = realm.init_worker_self_fn {
                let _ = init_fn.call(
                    &boa_engine::JsValue::undefined(),
                    &[boa_engine::JsValue::from(true)],
                    &mut realm.ctx,
                );
            } else {
                let _ = realm.ctx.eval(boa_engine::Source::from_bytes(
                    b"globalThis.__slice_init_worker_self_env(true);",
                ));
            }
        }
        let is_http_req = BOA_HTTP_REQ.with(|r| r.borrow().active);
        let has_slice_dot = memchr::memmem::find(js_code.as_bytes(), b"Slice.").is_some();
        if is_http_req
            || (has_slice_dot
                && (js_code.contains("Slice.request") || js_code.contains("Slice.respond")))
            || js_code.contains("__slice_http_")
        {
            crate::host_bindings::register_http_bridge_host_functions(&mut realm.ctx);
        }
        if js_code.contains("__slice_semver_") || js_code.contains("__slice_fast_") {
            crate::host_bindings::register_semver_and_fast_intrinsics_host_functions(
                &mut realm.ctx,
            );
        }
        if has_slice_dot
            && (js_code.contains("Slice.kv")
                || js_code.contains("Slice.checkpoint")
                || js_code.contains("Slice.restore")
                || js_code.contains("Slice.releaseCheckpoint")
                || js_code.contains("Slice.evalSandbox"))
        {
            crate::host_bindings::register_kv_and_sandbox_host_functions(&mut realm.ctx);
        }
        let rc_key_opt = extract_slice_rc_key(js_code);
        let mut should_check_ses = rc_key_opt.is_none();
        // Phase 9B Priority 2 + Phase N1 Direct JsObject Invocation (`__slice_rc`):
        // When a warm `boa_engine::Context` has already evaluated this bundle's `node_modules` CJS factories
        // and cached its entrypoint `JsObject` function handle in `realm.loaded_bundles[key]`, invoke
        // `fn_obj.call()` directly in Rust with ZERO JS source lexing, parsing, or `ctx.eval` overhead.
        let eval_res = if let Some((rc_key_u64, hex_bytes)) = rc_key_opt {
            if let Some(fn_obj) = realm.loaded_bundles.get(&rc_key_u64).cloned() {
                if is_http_req && (realm.loaded_shards_mask & (1 << 11)) == 0 {
                    ensure_realm_shards(&mut realm, 1 << 11, false);
                }
                self.apply_boa_realm_runtime_limits(&mut realm);
                fn_obj.call(&boa_engine::JsValue::undefined(), &[], &mut realm.ctx)
            } else {
                should_check_ses = true;
                ensure_realm_shards(
                    &mut realm,
                    detect_needed_shards(js_code, is_http_req),
                    needs_lazy_global_getters(js_code),
                );
                self.apply_boa_realm_runtime_limits(&mut realm);
                let res = realm
                    .ctx
                    .eval(boa_engine::Source::from_bytes(js_code.as_bytes()));
                if res.is_ok() && BOA_VIOLATION.with(|v| v.borrow().is_none()) {
                    if let Ok(hex_str) = std::str::from_utf8(hex_bytes) {
                        let global = realm.ctx.global_object();
                        if let Ok(rc_val) =
                            global.get(boa_engine::js_string!("__slice_rc"), &mut realm.ctx)
                        {
                            if let Some(rc_obj) = rc_val.as_object() {
                                if let Ok(val) =
                                    rc_obj.get(boa_engine::js_string!(hex_str), &mut realm.ctx)
                                {
                                    if let Some(obj) = val.as_object() {
                                        realm.loaded_bundles.insert(rc_key_u64, obj.clone());
                                    }
                                }
                            }
                        }
                    }
                }
                res
            }
        } else if js_code.starts_with('{') {
            ensure_realm_shards(
                &mut realm,
                detect_needed_shards(js_code, is_http_req),
                needs_lazy_global_getters(js_code),
            );
            self.apply_boa_realm_runtime_limits(&mut realm);
            eval_or_reuse_cached_script(&mut realm, js_code)
        } else {
            ensure_realm_shards(
                &mut realm,
                detect_needed_shards(js_code, is_http_req),
                needs_lazy_global_getters(js_code),
            );
            self.apply_boa_realm_runtime_limits(&mut realm);
            let scoped_code = if js_code.starts_with("//") {
                format!("{{\n{}\n}}", js_code)
            } else {
                format!("{{{}\n}}", js_code)
            };
            eval_or_reuse_cached_script(&mut realm, &scoped_code)
        };

        let need_ses_scan = should_check_ses || BOA_WAIT_UNTIL_COUNT.with(|c| c.get()) > 0;
        let has_ses_keywords = need_ses_scan
            && (js_code.contains("prototype")
                || js_code.contains("__proto__")
                || js_code.contains("Reflect")
                || js_code.contains("Object["));
        self.last_exec_check_ses = has_ses_keywords;
        let check_ses = should_check_ses && has_ses_keywords;

        self.finish_boa_compartment_execution(
            realm,
            flags,
            is_http_req,
            has_next_tick,
            check_ses,
            eval_res,
        )
    }

    /// Executes a precompiled `SLICEBC\x01` container (`--exper-boacas`) using precomputed
    /// `BoaCasHeader` metadata (eliminating all 18 runtime substring scans and runtime
    /// `detect_needed_shards` Aho-Corasick passes).
    pub(crate) fn exec_boacas_vm(
        &mut self,
        header: &crate::boacas::BoaCasHeader,
        precompiled_slice: &[u8],
        fallback_js: &str,
        contiguous_payload: &[u8],
    ) -> Result<JsValue, SandboxViolation> {
        let _active_src_guard = crate::boa_realm::ActiveSourceSliceGuard::new(fallback_js);
        let flags = self.setup_boa_compartment_thread_locals();

        if !flags.is_worker && (header.feature_flags & BOACAS_FLAG_HAS_WORKER) != 0 {
            crate::workers::speculative_prewarm_worker_pool(1);
        }

        let (mut realm, is_cold_bare_realm) = match try_acquire_pooled_boa_realm() {
            Some(r) => (r, false),
            None => (create_bare_boa_realm(), true),
        };

        let has_next_tick = (header.feature_flags & BOACAS_FLAG_HAS_NEXT_TICK) != 0;
        if is_cold_bare_realm && header.approach_kind >= 1 && !precompiled_slice.is_empty() {
            if self.deterministic_seed.is_some() {
                crate::host_bindings::register_deterministic_host_functions(&mut realm.ctx);
            }
            let mut boot_flags: i32 = 0;
            if flags.has_env_keys {
                boot_flags |= 1;
            }
            if self.deterministic_seed.is_some() {
                boot_flags |= 2;
            }
            if flags.is_worker {
                boot_flags |= 4;
                crate::js_preludes::register_shard_host_functions_for_mask(
                    (1 << 8) | (1 << 13) | (1 << 2),
                    &mut realm.ctx,
                );
            }
            if boot_flags != 0 {
                let _ = realm.ctx.global_object().set(
                    boa_engine::js_string!("__slice_bf"),
                    boa_engine::JsValue::from(boot_flags),
                    false,
                    &mut realm.ctx,
                );
            }
        } else {
            if is_cold_bare_realm {
                let _ = crate::js_preludes::eval_ultra_core_prelude(&mut realm.ctx);
                crate::boa_realm::populate_realm_cached_handles(&mut realm);
            }
            if flags.has_env_keys {
                call_realm_ctl(&mut realm, REALM_CTL_SYNC_ENV, true);
            }
            if self.deterministic_seed.is_some() {
                crate::host_bindings::register_deterministic_host_functions(&mut realm.ctx);
                call_realm_ctl(&mut realm, REALM_CTL_SET_DET_MODE, true);
            }
            if has_next_tick {
                call_realm_ctl(&mut realm, REALM_CTL_SET_NEXT_TICK, true);
            }
            if flags.is_worker {
                ensure_realm_shards(&mut realm, (1 << 8) | (1 << 13) | (1 << 2), false);
                if let Some(ref init_fn) = realm.init_worker_self_fn {
                    let _ = init_fn.call(
                        &boa_engine::JsValue::undefined(),
                        &[boa_engine::JsValue::from(true)],
                        &mut realm.ctx,
                    );
                } else {
                    let _ = realm.ctx.eval(boa_engine::Source::from_bytes(
                        b"globalThis.__slice_init_worker_self_env(true);",
                    ));
                }
            }
        }
        let is_http_req = BOA_HTTP_REQ.with(|r| r.borrow().active);
        if is_http_req || (header.feature_flags & BOACAS_FLAG_HAS_HTTP_BRIDGE) != 0 {
            crate::host_bindings::register_http_bridge_host_functions(&mut realm.ctx);
        }
        if (header.feature_flags & BOACAS_FLAG_HAS_SEMVER_FAST) != 0 {
            crate::host_bindings::register_semver_and_fast_intrinsics_host_functions(
                &mut realm.ctx,
            );
        }
        if (header.feature_flags & BOACAS_FLAG_HAS_KV_SANDBOX) != 0 {
            crate::host_bindings::register_kv_and_sandbox_host_functions(&mut realm.ctx);
        }

        self.apply_boa_realm_runtime_limits(&mut realm);
        let (eval_res, should_check_ses) = crate::boacas::eval_boacas_on_realm(
            &mut realm,
            header,
            precompiled_slice,
            fallback_js,
            contiguous_payload,
            is_http_req,
            is_cold_bare_realm,
        );

        let has_ses_flag = (header.feature_flags & BOACAS_FLAG_NEEDS_SES_CHECK) != 0;
        self.last_exec_check_ses = has_ses_flag;
        let check_ses = should_check_ses && has_ses_flag;

        self.finish_boa_compartment_execution(
            realm,
            flags,
            is_http_req,
            has_next_tick,
            check_ses,
            eval_res,
        )
    }
}
