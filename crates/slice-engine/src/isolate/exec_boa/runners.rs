//! Tier-1 `exec_boa_full_es_vm` (`SLICEBOA`) and `exec_boacas_vm` (`SLICEBC\x01`) Runners.

use super::{
    BoaSessionUnwindGuard, cache_loaded_rc_bundle, ensure_realm_det_funcs,
    ensure_realm_http_bridge, ensure_realm_kv_sandbox, ensure_realm_semver_fast,
    init_worker_self_realm,
};
use crate::boa_realm::{
    ActiveSourceSliceGuard, BOA_HTTP_REQ, BOA_VIOLATION, BOA_WAIT_UNTIL_COUNT, REALM_CTL_RESET_WU,
    REALM_CTL_SET_DET_MODE, REALM_CTL_SET_NEXT_TICK, REALM_CTL_SYNC_ENV,
    acquire_warm_boa_realm_for_tenant, call_realm_ctl, create_bare_boa_realm_for_tenant,
    ensure_realm_shards, eval_or_reuse_cached_script, populate_realm_cached_handles,
    try_acquire_pooled_boa_realm_for_tenant,
};
use crate::boacas::{
    BOACAS_FLAG_HAS_HTTP_BRIDGE, BOACAS_FLAG_HAS_KV_SANDBOX, BOACAS_FLAG_HAS_NEXT_TICK,
    BOACAS_FLAG_HAS_SEMVER_FAST, BOACAS_FLAG_HAS_WORKER, BOACAS_FLAG_NEEDS_SES_CHECK, BoaCasHeader,
};
use crate::isolate::SliceIsolate;
use crate::js_preludes::{detect_needed_shards, needs_lazy_global_getters};
use crate::value_and_ast::JsValue;
use slice_cas::extract_slice_rc_key;
use slice_sandbox::SandboxViolation;

impl SliceIsolate {
    /// Executes full ES2024+ JavaScript inside the production **Boa (`boa_engine`)** ECMAScript VM,
    /// reusing pre-warmed contexts from `BOA_CONTEXT_POOL`, capturing `console.log` / `print` output
    /// via zero-eval native host hooks, and enforcing SES compartment limits.
    #[inline]
    pub(crate) fn exec_boa_full_es_vm(
        &mut self,
        js_code: &str,
    ) -> Result<JsValue, SandboxViolation> {
        let _active_src_guard = ActiveSourceSliceGuard::new(js_code);
        let flags = self.setup_boa_compartment_thread_locals();
        let mut unwind_guard = BoaSessionUnwindGuard::new();
        let rc_key_opt = extract_slice_rc_key(js_code);
        let mut realm = acquire_warm_boa_realm_for_tenant(&self.compartment.tenant_id);
        if realm.has_pending_wu {
            call_realm_ctl(&mut realm, REALM_CTL_RESET_WU, false);
            realm.has_pending_wu = false;
        }

        let cached_rc_flags =
            rc_key_opt.and_then(|(k, _)| realm.loaded_bundle_flags.get(&k).copied());

        let (has_worker_kw, has_next_tick, has_http_kw, has_semver_kw, has_kv_kw, cached_ses_kw) =
            if let Some(bf) = cached_rc_flags {
                (
                    (bf & BOACAS_FLAG_HAS_WORKER) != 0,
                    (bf & BOACAS_FLAG_HAS_NEXT_TICK) != 0,
                    (bf & BOACAS_FLAG_HAS_HTTP_BRIDGE) != 0,
                    (bf & BOACAS_FLAG_HAS_SEMVER_FAST) != 0,
                    (bf & BOACAS_FLAG_HAS_KV_SANDBOX) != 0,
                    Some((bf & BOACAS_FLAG_NEEDS_SES_CHECK) != 0),
                )
            } else {
                let has_slice_dot = memchr::memmem::find(js_code.as_bytes(), b"Slice.").is_some();
                (
                    js_code.contains("Worker") || js_code.contains("worker_threads"),
                    js_code.contains("nextTick"),
                    (has_slice_dot
                        && (js_code.contains("Slice.request")
                            || js_code.contains("Slice.respond")))
                        || js_code.contains("__slice_http_"),
                    js_code.contains("__slice_semver_") || js_code.contains("__slice_fast_"),
                    has_slice_dot
                        && (js_code.contains("Slice.kv")
                            || js_code.contains("Slice.checkpoint")
                            || js_code.contains("Slice.restore")
                            || js_code.contains("Slice.releaseCheckpoint")
                            || js_code.contains("Slice.evalSandbox")),
                    None,
                )
            };

        if !flags.is_worker && has_worker_kw {
            crate::workers::speculative_prewarm_worker_pool(1);
        }

        if flags.has_env_keys {
            call_realm_ctl(&mut realm, REALM_CTL_SYNC_ENV, true);
        }
        if self.deterministic_seed.is_some() {
            ensure_realm_det_funcs(&mut realm);
            call_realm_ctl(&mut realm, REALM_CTL_SET_DET_MODE, true);
        }
        if has_next_tick {
            call_realm_ctl(&mut realm, REALM_CTL_SET_NEXT_TICK, true);
        }
        if flags.is_worker {
            init_worker_self_realm(&mut realm);
        }
        let is_http_req = BOA_HTTP_REQ.with(|r| r.borrow().active);
        if is_http_req || has_http_kw {
            ensure_realm_http_bridge(&mut realm);
        }
        if has_semver_kw {
            ensure_realm_semver_fast(&mut realm);
        }
        if has_kv_kw {
            ensure_realm_kv_sandbox(&mut realm);
        }
        let mut should_check_ses = rc_key_opt.is_none();
        let mut owned_scoped_code = String::new();
        let mut _scoped_active_src_guard: Option<ActiveSourceSliceGuard> = None;
        // Direct JsObject Invocation (`__slice_rc`):
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
                let (bflags, _, _) = crate::boacas::compute_boacas_metadata(js_code);
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
                    cache_loaded_rc_bundle(&mut realm, rc_key_u64, hex_bytes, Some(bflags), None);
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
            let clean = slice_cas::compiler::blank_leading_shebang(js_code);
            owned_scoped_code.reserve(clean.len() + 4);
            owned_scoped_code.push_str("{\n");
            owned_scoped_code.push_str(&clean);
            owned_scoped_code.push_str("\n}");
            _scoped_active_src_guard = Some(ActiveSourceSliceGuard::new(&owned_scoped_code));
            eval_or_reuse_cached_script(&mut realm, &owned_scoped_code)
        };

        let need_ses_scan = should_check_ses || BOA_WAIT_UNTIL_COUNT.with(|c| c.get()) > 0;
        let has_ses_keywords = need_ses_scan
            && cached_ses_kw.unwrap_or_else(|| {
                js_code.contains("prototype")
                    || js_code.contains("__proto__")
                    || js_code.contains("Reflect")
                    || js_code.contains("defineProperty")
                    || js_code.contains("defineProperties")
                    || js_code.contains("setPrototypeOf")
                    || js_code.contains("getPrototypeOf")
                    || js_code.contains("constructor")
                    || js_code.contains("Object[")
            });
        self.last_exec_check_ses = has_ses_keywords;
        let check_ses = should_check_ses && has_ses_keywords;

        let res = self.finish_boa_compartment_execution(
            realm,
            flags,
            is_http_req,
            has_next_tick,
            check_ses,
            eval_res,
        );
        unwind_guard.disarm();
        res
    }

    /// Executes a precompiled `SLICEBC\x01` container (`--exper-boacas`) using precomputed
    /// `BoaCasHeader` metadata (eliminating runtime substring scans and `detect_needed_shards` passes).
    #[inline]
    pub(crate) fn exec_boacas_vm(
        &mut self,
        header: &BoaCasHeader,
        precompiled_slice: &[u8],
        fallback_js_bytes: &[u8],
    ) -> Result<JsValue, SandboxViolation> {
        let _active_src_guard = ActiveSourceSliceGuard::new_bytes(fallback_js_bytes);
        let flags = self.setup_boa_compartment_thread_locals();
        let mut unwind_guard = BoaSessionUnwindGuard::new();

        if !flags.is_worker && (header.feature_flags & BOACAS_FLAG_HAS_WORKER) != 0 {
            crate::workers::speculative_prewarm_worker_pool(1);
        }

        let (mut realm, is_cold_bare_realm) =
            match try_acquire_pooled_boa_realm_for_tenant(&self.compartment.tenant_id) {
                Some(r) => (r, false),
                None => (
                    create_bare_boa_realm_for_tenant(&self.compartment.tenant_id),
                    true,
                ),
            };
        if realm.has_pending_wu {
            call_realm_ctl(&mut realm, REALM_CTL_RESET_WU, false);
            realm.has_pending_wu = false;
        }

        let has_next_tick = (header.feature_flags & BOACAS_FLAG_HAS_NEXT_TICK) != 0;
        if is_cold_bare_realm && header.approach_kind >= 1 && !precompiled_slice.is_empty() {
            if self.deterministic_seed.is_some() {
                ensure_realm_det_funcs(&mut realm);
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
                populate_realm_cached_handles(&mut realm);
            }
            if flags.has_env_keys {
                call_realm_ctl(&mut realm, REALM_CTL_SYNC_ENV, true);
            }
            if self.deterministic_seed.is_some() {
                ensure_realm_det_funcs(&mut realm);
                call_realm_ctl(&mut realm, REALM_CTL_SET_DET_MODE, true);
            }
            if has_next_tick {
                call_realm_ctl(&mut realm, REALM_CTL_SET_NEXT_TICK, true);
            }
            if flags.is_worker {
                init_worker_self_realm(&mut realm);
            }
        }
        let is_http_req = BOA_HTTP_REQ.with(|r| r.borrow().active);
        if is_http_req || (header.feature_flags & BOACAS_FLAG_HAS_HTTP_BRIDGE) != 0 {
            ensure_realm_http_bridge(&mut realm);
        }
        if (header.feature_flags & BOACAS_FLAG_HAS_SEMVER_FAST) != 0 {
            ensure_realm_semver_fast(&mut realm);
        }
        if (header.feature_flags & BOACAS_FLAG_HAS_KV_SANDBOX) != 0 {
            ensure_realm_kv_sandbox(&mut realm);
        }

        self.apply_boa_realm_runtime_limits(&mut realm);
        let (eval_res, should_check_ses) = crate::boacas::eval_boacas_on_realm(
            &mut realm,
            header,
            precompiled_slice,
            fallback_js_bytes,
            is_http_req,
            is_cold_bare_realm,
        );

        let has_ses_flag = (header.feature_flags & BOACAS_FLAG_NEEDS_SES_CHECK) != 0;
        self.last_exec_check_ses = has_ses_flag;
        let check_ses = should_check_ses && has_ses_flag;

        let res = self.finish_boa_compartment_execution(
            realm,
            flags,
            is_http_req,
            has_next_tick,
            check_ses,
            eval_res,
        );
        unwind_guard.disarm();
        res
    }
}
