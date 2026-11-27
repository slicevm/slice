//! Nested Sandbox Execution Scope Guard (`SavedIsolateContextGuard`).

use super::{
    ActiveBoaHttpReq, BOA_ACTIVE_CAPS, BOA_ACTIVE_TENANT, BOA_ALLOW_PRIVATE_NETS, BOA_ALS_ACTIVE,
    BOA_ALS_SNAP, BOA_CHECKPOINT_CAP, BOA_CHECKPOINT_STATS, BOA_CUSTOM_ARGV,
    BOA_DENIED_CAPABILITIES, BOA_DET_STATE, BOA_DIRTY_SUBSYSTEMS, BOA_EGRESS_SECRETS, BOA_ENV_MAP,
    BOA_EXEC_BINS, BOA_EXEC_DIRTY, BOA_FETCH_MOCKS, BOA_FUEL_BUDGET, BOA_HTTP_REQ, BOA_HTTP_RES,
    BOA_KV_MEM_BUDGET, BOA_LAST_EXIT_CODE, BOA_LAST_OCAP_VIOLATION, BOA_NET_HOSTS,
    BOA_PROCESS_EXIT, BOA_SES_BASELINE_SIGS, BOA_SES_INTL_SIG_PAIR, BOA_STDOUT, BOA_UNCAUGHT_ERROR,
    BOA_UNHANDLED_REJECTIONS, BOA_VIOLATION, BOA_WAIT_UNTIL_COUNT, BOA_WS_UPGRADED, BoaFsCaps,
    CapturedBoaHttpRes, DIRTY_ALS, DIRTY_FETCH_MOCK, DIRTY_SOCKETS, DIRTY_WASM, DIRTY_WEBSOCKET,
    EXEC_DIRTY_ARGV, EXEC_DIRTY_CHILD_PROC, EXEC_DIRTY_EXIT, EXEC_DIRTY_REJECTIONS,
    EXEC_DIRTY_UNCAUGHT, SES_INTRINSIC_COUNT, SesIntlSigDelta, SesProtoSig,
};
use rustc_hash::{FxHashMap, FxHashSet};
use slice_sandbox::SandboxViolation;

/// RAII execution scope guard that snapshots all per-invocation thread-local isolate state
/// before a nested execution (`Slice.evalSandbox`, recursive sandbox invocation) and restores
/// every field on `Drop` (including on early return or panic), preventing any child sandbox
/// state (fuel budget, deterministic PRNG/clock, HTTP request/response, AsyncLocalStorage,
/// OCap capabilities, DLP secrets, or mock cassettes) from leaking into the parent isolate.
pub(crate) struct SavedIsolateContextGuard {
    pub(crate) saved_stdout: Vec<String>,
    pub(crate) saved_violation: Option<SandboxViolation>,
    pub(crate) saved_tenant: String,
    pub(crate) saved_caps: BoaFsCaps,
    pub(crate) saved_net_hosts: FxHashSet<String>,
    pub(crate) saved_allow_private_nets: bool,
    pub(crate) saved_egress_secrets: Vec<String>,
    pub(crate) saved_env_map: FxHashMap<String, String>,
    pub(crate) saved_exec_bins: Vec<String>,
    pub(crate) saved_last_cp_violation: Option<SandboxViolation>,
    pub(crate) saved_denied_capabilities: Vec<String>,
    pub(crate) saved_fetch_mocks: Option<FxHashMap<String, String>>,
    pub(crate) saved_ws_mocks: Option<FxHashMap<String, String>>,
    pub(crate) saved_sock_mocks: Option<FxHashMap<String, String>>,
    pub(crate) saved_kv_budget: (usize, usize),
    pub(crate) saved_fuel_budget: (u64, u64),
    pub(crate) saved_checkpoint_stats: (u32, u32, u64),
    pub(crate) saved_checkpoint_cap: (bool, u32),
    pub(crate) saved_det: Option<(u64, u64)>,
    pub(crate) saved_als_active: bool,
    pub(crate) saved_als_snap: boa_engine::JsValue,
    pub(crate) saved_http_req: Option<ActiveBoaHttpReq>,
    pub(crate) saved_http_res: Option<CapturedBoaHttpRes>,
    pub(crate) saved_ws_upgraded: bool,
    pub(crate) saved_wait_until_count: usize,
    pub(crate) saved_dirty_subsystems: u16,
    pub(crate) saved_exec_dirty: u8,
    pub(crate) saved_custom_argv: Option<Vec<String>>,
    pub(crate) saved_process_exit: Option<i32>,
    pub(crate) saved_last_exit_code: Option<i32>,
    pub(crate) saved_unhandled_rejections: Vec<boa_engine::JsObject>,
    pub(crate) saved_uncaught_error: Option<String>,
    pub(crate) saved_ev_loop: Option<crate::event_loop::SavedEventLoopSnapshot>,
    pub(crate) saved_wasm_state: Option<crate::wasm::WasmThreadState>,
    pub(crate) saved_ses_baseline_sigs: Option<[SesProtoSig; SES_INTRINSIC_COUNT]>,
    pub(crate) saved_ses_intl_pair: Option<SesIntlSigDelta>,
}

impl SavedIsolateContextGuard {
    pub(crate) fn enter() -> Self {
        let saved_exec_dirty = BOA_EXEC_DIRTY.with(|c| c.replace(0));
        let saved_stdout = BOA_STDOUT.with(|out| std::mem::take(&mut *out.borrow_mut()));
        let saved_violation = BOA_VIOLATION.with(|v| v.borrow_mut().take());
        let saved_tenant = BOA_ACTIVE_TENANT.with(|t| std::mem::take(&mut *t.borrow_mut()));
        let saved_caps = BOA_ACTIVE_CAPS.with(|c| std::mem::take(&mut *c.borrow_mut()));
        let saved_net_hosts = BOA_NET_HOSTS.with(|h| std::mem::take(&mut *h.borrow_mut()));
        let saved_allow_private_nets = BOA_ALLOW_PRIVATE_NETS.with(|p| p.replace(false));
        let saved_egress_secrets =
            BOA_EGRESS_SECRETS.with(|s| std::mem::take(&mut *s.borrow_mut()));
        let saved_env_map = BOA_ENV_MAP.with(|m| std::mem::take(&mut *m.borrow_mut()));
        let saved_exec_bins = BOA_EXEC_BINS.with(|b| std::mem::take(&mut *b.borrow_mut()));
        let (saved_last_cp_violation, saved_denied_capabilities) =
            if (saved_exec_dirty & EXEC_DIRTY_CHILD_PROC) != 0 {
                (
                    BOA_LAST_OCAP_VIOLATION.with(|v| v.borrow_mut().take()),
                    BOA_DENIED_CAPABILITIES.with(|d| std::mem::take(&mut *d.borrow_mut())),
                )
            } else {
                (None, Vec::new())
            };
        let saved_fetch_mocks = BOA_FETCH_MOCKS.with(|m| {
            let b = m.borrow();
            if b.is_empty() { None } else { Some(b.clone()) }
        });
        let saved_ws_mocks = crate::websocket::BOA_WS_MOCKS.with(|m| {
            let b = m.borrow();
            if b.is_empty() { None } else { Some(b.clone()) }
        });
        let saved_sock_mocks = crate::sockets::BOA_SOCKET_MOCKS.with(|m| {
            let b = m.borrow();
            if b.is_empty() { None } else { Some(b.clone()) }
        });
        let saved_kv_budget = BOA_KV_MEM_BUDGET.with(|m| m.get());
        let saved_fuel_budget = BOA_FUEL_BUDGET.with(|f| f.get());
        let saved_checkpoint_stats = BOA_CHECKPOINT_STATS.with(|s| s.replace((0, 0, 0)));
        let saved_checkpoint_cap = BOA_CHECKPOINT_CAP.with(|c| c.get());
        let saved_det = BOA_DET_STATE.with(|s| s.get());
        let saved_als_active = BOA_ALS_ACTIVE.with(|a| a.replace(false));
        let saved_als_snap = if saved_als_active {
            BOA_ALS_SNAP.with(|s| s.replace(boa_engine::JsValue::undefined()))
        } else {
            boa_engine::JsValue::undefined()
        };
        let saved_http_req = BOA_HTTP_REQ.with(|r| {
            let mut borrow = r.borrow_mut();
            if borrow.active {
                Some(std::mem::take(&mut *borrow))
            } else {
                None
            }
        });
        let saved_http_res = BOA_HTTP_RES.with(|res| res.borrow_mut().take());
        let saved_ws_upgraded = BOA_WS_UPGRADED.with(|w| w.replace(false));
        let saved_wait_until_count = BOA_WAIT_UNTIL_COUNT.with(|c| c.replace(0));
        let saved_dirty_subsystems = BOA_DIRTY_SUBSYSTEMS.with(|d| d.replace(0));
        let saved_custom_argv = if (saved_exec_dirty & EXEC_DIRTY_ARGV) != 0 {
            BOA_CUSTOM_ARGV.with(|a| a.borrow_mut().take())
        } else {
            None
        };
        let (saved_process_exit, saved_last_exit_code) =
            if (saved_exec_dirty & EXEC_DIRTY_EXIT) != 0 {
                (
                    BOA_PROCESS_EXIT.with(|e| e.replace(None)),
                    BOA_LAST_EXIT_CODE.with(|e| e.replace(None)),
                )
            } else {
                (None, None)
            };
        let saved_unhandled_rejections = if (saved_exec_dirty & EXEC_DIRTY_REJECTIONS) != 0 {
            BOA_UNHANDLED_REJECTIONS.with(|u| std::mem::take(&mut *u.borrow_mut()))
        } else {
            Vec::new()
        };
        let saved_uncaught_error = if (saved_exec_dirty & EXEC_DIRTY_UNCAUGHT) != 0 {
            BOA_UNCAUGHT_ERROR.with(|e| e.borrow_mut().take())
        } else {
            None
        };
        let saved_ev_loop = crate::event_loop::save_and_reset_event_loop_state();
        let saved_wasm_state = crate::wasm::take_thread_wasm_state_if_nonempty();
        let saved_ses_baseline_sigs = BOA_SES_BASELINE_SIGS.with(|s| s.replace(None));
        let saved_ses_intl_pair = BOA_SES_INTL_SIG_PAIR.with(|p| p.replace(None));

        Self {
            saved_stdout,
            saved_violation,
            saved_tenant,
            saved_caps,
            saved_net_hosts,
            saved_allow_private_nets,
            saved_egress_secrets,
            saved_env_map,
            saved_exec_bins,
            saved_last_cp_violation,
            saved_denied_capabilities,
            saved_fetch_mocks,
            saved_ws_mocks,
            saved_sock_mocks,
            saved_kv_budget,
            saved_fuel_budget,
            saved_checkpoint_stats,
            saved_checkpoint_cap,
            saved_det,
            saved_als_active,
            saved_als_snap,
            saved_http_req,
            saved_http_res,
            saved_ws_upgraded,
            saved_wait_until_count,
            saved_dirty_subsystems,
            saved_exec_dirty,
            saved_custom_argv,
            saved_process_exit,
            saved_last_exit_code,
            saved_unhandled_rejections,
            saved_uncaught_error,
            saved_ev_loop,
            saved_wasm_state,
            saved_ses_baseline_sigs,
            saved_ses_intl_pair,
        }
    }
}

impl Drop for SavedIsolateContextGuard {
    fn drop(&mut self) {
        if let Some(ev) = self.saved_ev_loop.take() {
            crate::event_loop::restore_event_loop_state(ev);
        }
        let child_dirty = BOA_DIRTY_SUBSYSTEMS.with(|d| d.get());
        if let Some(ws) = self.saved_wasm_state.take() {
            crate::wasm::restore_thread_wasm_state(ws);
        } else if (child_dirty & DIRTY_WASM) != 0 {
            crate::wasm::clear_thread_wasm_state();
        }
        let child_exec_dirty = BOA_EXEC_DIRTY.with(|c| c.get());
        let union_exec_dirty = self.saved_exec_dirty | child_exec_dirty;
        BOA_DIRTY_SUBSYSTEMS.with(|d| d.set(self.saved_dirty_subsystems));
        BOA_EXEC_DIRTY.with(|c| c.set(self.saved_exec_dirty));
        if (union_exec_dirty & EXEC_DIRTY_EXIT) != 0 {
            BOA_PROCESS_EXIT.with(|e| e.set(self.saved_process_exit));
            BOA_LAST_EXIT_CODE.with(|e| e.set(self.saved_last_exit_code));
        }
        if (union_exec_dirty & EXEC_DIRTY_REJECTIONS) != 0 {
            BOA_UNHANDLED_REJECTIONS
                .with(|u| *u.borrow_mut() = std::mem::take(&mut self.saved_unhandled_rejections));
        }
        if (union_exec_dirty & EXEC_DIRTY_UNCAUGHT) != 0 {
            BOA_UNCAUGHT_ERROR.with(|e| *e.borrow_mut() = self.saved_uncaught_error.take());
        }
        if (union_exec_dirty & EXEC_DIRTY_ARGV) != 0 {
            BOA_CUSTOM_ARGV.with(|a| *a.borrow_mut() = self.saved_custom_argv.take());
        }
        BOA_STDOUT.with(|out| *out.borrow_mut() = std::mem::take(&mut self.saved_stdout));
        BOA_VIOLATION.with(|v| *v.borrow_mut() = self.saved_violation.take());
        BOA_ACTIVE_TENANT.with(|t| *t.borrow_mut() = std::mem::take(&mut self.saved_tenant));
        BOA_ACTIVE_CAPS.with(|c| *c.borrow_mut() = std::mem::take(&mut self.saved_caps));
        BOA_NET_HOSTS.with(|h| *h.borrow_mut() = std::mem::take(&mut self.saved_net_hosts));
        BOA_ALLOW_PRIVATE_NETS.with(|p| p.set(self.saved_allow_private_nets));
        BOA_EGRESS_SECRETS
            .with(|s| *s.borrow_mut() = std::mem::take(&mut self.saved_egress_secrets));
        BOA_ENV_MAP.with(|m| *m.borrow_mut() = std::mem::take(&mut self.saved_env_map));
        BOA_EXEC_BINS.with(|b| *b.borrow_mut() = std::mem::take(&mut self.saved_exec_bins));
        if (union_exec_dirty & EXEC_DIRTY_CHILD_PROC) != 0 {
            BOA_LAST_OCAP_VIOLATION.with(|v| *v.borrow_mut() = self.saved_last_cp_violation.take());
            BOA_DENIED_CAPABILITIES.with(|d| {
                *d.borrow_mut() = std::mem::take(&mut self.saved_denied_capabilities);
            });
        }
        if let Some(fm) = self.saved_fetch_mocks.take() {
            BOA_FETCH_MOCKS.with(|m| *m.borrow_mut() = fm);
        } else if (child_dirty & DIRTY_FETCH_MOCK) != 0 {
            BOA_FETCH_MOCKS.with(|m| m.borrow_mut().clear());
        }
        if let Some(wm) = self.saved_ws_mocks.take() {
            crate::websocket::BOA_WS_MOCKS.with(|m| *m.borrow_mut() = wm);
        } else if (child_dirty & DIRTY_WEBSOCKET) != 0 {
            crate::websocket::BOA_WS_MOCKS.with(|m| m.borrow_mut().clear());
        }
        if let Some(sm) = self.saved_sock_mocks.take() {
            crate::sockets::BOA_SOCKET_MOCKS.with(|m| *m.borrow_mut() = sm);
        } else if (child_dirty & DIRTY_SOCKETS) != 0 {
            crate::sockets::BOA_SOCKET_MOCKS.with(|m| m.borrow_mut().clear());
        }
        BOA_KV_MEM_BUDGET.with(|m| m.set(self.saved_kv_budget));
        BOA_FUEL_BUDGET.with(|f| f.set(self.saved_fuel_budget));
        BOA_CHECKPOINT_STATS.with(|s| s.set(self.saved_checkpoint_stats));
        BOA_CHECKPOINT_CAP.with(|c| c.set(self.saved_checkpoint_cap));
        BOA_DET_STATE.with(|s| s.set(self.saved_det));
        BOA_ALS_ACTIVE.with(|a| a.set(self.saved_als_active));
        if self.saved_als_active || (child_dirty & DIRTY_ALS) != 0 {
            BOA_ALS_SNAP.with(|s| *s.borrow_mut() = self.saved_als_snap.clone());
        }
        if let Some(req) = self.saved_http_req.take() {
            BOA_HTTP_REQ.with(|r| *r.borrow_mut() = req);
        } else {
            BOA_HTTP_REQ.with(|r| r.borrow_mut().active = false);
        }
        BOA_HTTP_RES.with(|res| *res.borrow_mut() = self.saved_http_res.take());
        BOA_WS_UPGRADED.with(|w| w.set(self.saved_ws_upgraded));
        BOA_WAIT_UNTIL_COUNT.with(|c| c.set(self.saved_wait_until_count));
        BOA_SES_BASELINE_SIGS.with(|s| s.set(self.saved_ses_baseline_sigs));
        BOA_SES_INTL_SIG_PAIR.with(|p| p.set(self.saved_ses_intl_pair));
    }
}
