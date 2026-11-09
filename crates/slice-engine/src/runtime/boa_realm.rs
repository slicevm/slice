//! Warm `boa_engine` Realm Pool, Thread-Local Compartment State, and OCap VFS Guards.

use crate::host_bindings::register_slice_native_host_functions;
use crate::state_and_kv::StateCheckpoint;
use crate::value_and_ast::BytecodeModule;
use rustc_hash::{FxHashMap, FxHashSet};
use slice_sandbox::SandboxViolation;

pub(crate) const DIRTY_EVENT_LOOP: u8 = 1 << 0;
pub(crate) const DIRTY_ALS: u8 = 1 << 1;
pub(crate) const DIRTY_FETCH_MOCK: u8 = 1 << 2;
pub(crate) const DIRTY_WEBSOCKET: u8 = 1 << 3;
pub(crate) const DIRTY_SOCKETS: u8 = 1 << 4;
pub(crate) const DIRTY_WORKERS: u8 = 1 << 5;
pub(crate) const DIRTY_WASM: u8 = 1 << 6;

#[derive(Default, Clone)]
pub(crate) struct ActiveBoaHttpReq {
    pub(crate) active: bool,
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) query: String,
    pub(crate) tenant_id: String,
    pub(crate) body: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) headers_len: usize,
}

pub(crate) struct WarmBoaRealm {
    pub(crate) ctx: boa_engine::Context,
    pub(crate) loaded_bundles: FxHashMap<u64, boa_engine::JsObject>,
    pub(crate) ses_check_fn: Option<boa_engine::JsObject>,
    pub(crate) ensure_shard_fn: Option<boa_engine::JsObject>,
    pub(crate) init_worker_self_fn: Option<boa_engine::JsObject>,
    pub(crate) drain_wu_fn: Option<boa_engine::JsObject>,
    pub(crate) loaded_shards_mask: u32,
}

pub(crate) type ActiveBoaCaps = (String, Vec<String>, Vec<String>);
pub(crate) type CapturedBoaHttpRes = (u16, String, String, Vec<(String, String)>);

thread_local! {
    pub(crate) static SNP_MODULE_CACHE: std::cell::RefCell<FxHashMap<u64, std::sync::Arc<BytecodeModule>>> =
        std::cell::RefCell::new(FxHashMap::default());
    pub(crate) static BOA_STDOUT: std::cell::RefCell<Vec<String>> =
        std::cell::RefCell::new(Vec::with_capacity(16));
    pub(crate) static BOA_CONTEXT_POOL: std::cell::RefCell<std::mem::ManuallyDrop<Vec<WarmBoaRealm>>> =
        const { std::cell::RefCell::new(std::mem::ManuallyDrop::new(Vec::new())) };
    pub(crate) static BOA_VFS_CACHE: std::cell::RefCell<FxHashMap<String, std::sync::Arc<Vec<u8>>>> =
        std::cell::RefCell::new(FxHashMap::default());
    pub(crate) static BOA_VFS_DIRS: std::cell::RefCell<FxHashSet<String>> =
        std::cell::RefCell::new(FxHashSet::default());
    pub(crate) static BOA_ACTIVE_CAPS: std::cell::RefCell<Option<ActiveBoaCaps>> =
        const { std::cell::RefCell::new(None) };
    pub(crate) static BOA_VIOLATION: std::cell::RefCell<Option<SandboxViolation>> =
        const { std::cell::RefCell::new(None) };
    pub(crate) static BOA_HTTP_REQ: std::cell::RefCell<ActiveBoaHttpReq> =
        std::cell::RefCell::new(ActiveBoaHttpReq::default());
    pub(crate) static BOA_HTTP_RES: std::cell::RefCell<Option<CapturedBoaHttpRes>> =
        const { std::cell::RefCell::new(None) };
    pub(crate) static BOA_WAIT_UNTIL_COUNT: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
    pub(crate) static BOA_ACTIVE_TENANT: std::cell::RefCell<String> =
        std::cell::RefCell::new(String::with_capacity(32));
    pub(crate) static BOA_KV_MEM_BUDGET: std::cell::Cell<(usize, usize)> =
        const { std::cell::Cell::new((0, usize::MAX)) };
    pub(crate) static BOA_FUEL_BUDGET: std::cell::Cell<(u64, u64)> =
        const { std::cell::Cell::new((0, u64::MAX)) };
    pub(crate) static BOA_DET_STATE: std::cell::Cell<Option<(u64, u64)>> =
        const { std::cell::Cell::new(None) };
    pub(crate) static BOA_NET_HOSTS: std::cell::RefCell<FxHashSet<String>> =
        std::cell::RefCell::new(FxHashSet::default());
    pub(crate) static BOA_EGRESS_SECRETS: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
    pub(crate) static BOA_ENV_MAP: std::cell::RefCell<FxHashMap<String, String>> =
        std::cell::RefCell::new(FxHashMap::default());
    pub(crate) static BOA_EXEC_BINS: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
    pub(crate) static BOA_LAST_CHILD_PROC_VIOLATION: std::cell::RefCell<Option<SandboxViolation>> =
        const { std::cell::RefCell::new(None) };
    pub(crate) static BOA_FETCH_MOCKS: std::cell::RefCell<FxHashMap<String, String>> =
        std::cell::RefCell::new(FxHashMap::default());
    pub(crate) static CHECKPOINT_SLAB: std::cell::RefCell<FxHashMap<u64, StateCheckpoint>> =
        std::cell::RefCell::new(FxHashMap::default());
    pub(crate) static NEXT_CHECKPOINT_ID: std::cell::Cell<u64> =
        const { std::cell::Cell::new(1) };
    pub(crate) static BOA_CHECKPOINT_STATS: std::cell::Cell<(u32, u32, u64)> =
        const { std::cell::Cell::new((0, 0, 0)) };
    pub(crate) static BOA_CHECKPOINT_CAP: std::cell::Cell<(bool, u32)> =
        const { std::cell::Cell::new((true, u32::MAX)) };
    pub(crate) static BOA_ALS_ACTIVE: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
    pub(crate) static BOA_ALS_SNAP: std::cell::RefCell<boa_engine::JsValue> =
        const { std::cell::RefCell::new(boa_engine::JsValue::undefined()) };
    pub(crate) static BOA_ACTIVE_SOURCE_PTR_LEN: std::cell::Cell<(usize, usize)> =
        const { std::cell::Cell::new((0, 0)) };
    pub(crate) static BOA_TRACE_ENABLED: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
    pub(crate) static BOA_TRACE_EVENTS: std::cell::RefCell<Vec<ChromeTraceEvent>> =
        const { std::cell::RefCell::new(Vec::new()) };
    pub(crate) static BOA_DIRTY_SUBSYSTEMS: std::cell::Cell<u8> =
        const { std::cell::Cell::new(0) };
}

#[inline]
pub(crate) fn mark_subsystem_dirty(bit: u8) {
    BOA_DIRTY_SUBSYSTEMS.with(|c| c.set(c.get() | bit));
}

/// Chrome DevTools / Perfetto Trace Event (`ph: "X"` complete duration event).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChromeTraceEvent {
    pub name: String,
    pub cat: String,
    pub ph: String,
    pub ts: u64,
    pub dur: u64,
    pub pid: u32,
    pub tid: u32,
    pub args: serde_json::Value,
}

#[inline]
pub(crate) fn is_trace_recording_enabled() -> bool {
    BOA_TRACE_ENABLED.with(|c| c.get())
}

pub(crate) fn record_trace_event(
    name: impl Into<String>,
    cat: impl Into<String>,
    ts: u64,
    dur: u64,
    args: serde_json::Value,
) {
    BOA_TRACE_EVENTS.with(|ev| {
        ev.borrow_mut().push(ChromeTraceEvent {
            name: name.into(),
            cat: cat.into(),
            ph: "X".to_string(),
            ts,
            dur: dur.max(1),
            pid: 1,
            tid: 1,
            args,
        });
    });
}

/// Zero-allocation RAII guard that binds the currently executing JS bundle slice `(ptr, len)`
/// to `BOA_ACTIVE_SOURCE_PTR_LEN` so lazy `Error.prototype.stack` accesses can translate frames
/// without copying or allocating the source string on the hot execution path.
pub(crate) struct ActiveSourceSliceGuard<'a> {
    prev: (usize, usize),
    _marker: std::marker::PhantomData<&'a str>,
}

impl<'a> ActiveSourceSliceGuard<'a> {
    #[inline]
    pub(crate) fn new(src: &'a str) -> Self {
        let prev = BOA_ACTIVE_SOURCE_PTR_LEN.with(|c| {
            let p = c.get();
            c.set((src.as_ptr() as usize, src.len()));
            p
        });
        Self {
            prev,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<'a> Drop for ActiveSourceSliceGuard<'a> {
    #[inline]
    fn drop(&mut self) {
        BOA_ACTIVE_SOURCE_PTR_LEN.with(|c| c.set(self.prev));
    }
}

pub(crate) fn map_stack_with_active_source(raw_stack: &str) -> String {
    let (ptr, len) = BOA_ACTIVE_SOURCE_PTR_LEN.with(|c| c.get());
    if ptr == 0 || len == 0 {
        return raw_stack.to_string();
    }
    // SAFETY: `ActiveSourceSliceGuard` guarantees `(ptr, len)` points to a valid live `&str`
    // on the current thread's call stack and restores `prev` before the `&str` lifetime ends.
    let active_src = unsafe {
        let bytes = std::slice::from_raw_parts(ptr as *const u8, len);
        std::str::from_utf8_unchecked(bytes)
    };
    slice_cas::map_bundled_stack_trace(active_src, raw_stack)
}

pub(crate) fn map_error_with_active_source(raw_error: &str) -> String {
    let (ptr, len) = BOA_ACTIVE_SOURCE_PTR_LEN.with(|c| c.get());
    if ptr == 0 || len == 0 {
        return raw_error.to_string();
    }
    // SAFETY: `ActiveSourceSliceGuard` guarantees `(ptr, len)` points to a valid live `&str`
    // on the current thread's call stack and restores `prev` before the `&str` lifetime ends.
    let active_src = unsafe {
        let bytes = std::slice::from_raw_parts(ptr as *const u8, len);
        std::str::from_utf8_unchecked(bytes)
    };
    slice_cas::map_bundled_error_location(active_src, raw_error)
}

pub(crate) struct SliceJobExecutor {
    promise_jobs: std::cell::RefCell<
        std::collections::VecDeque<(boa_engine::job::PromiseJob, Option<boa_engine::JsValue>)>,
    >,
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
                let snap = if BOA_ALS_ACTIVE.with(|a| a.get()) {
                    Some(BOA_ALS_SNAP.with(|s| s.borrow().clone()))
                } else {
                    None
                };
                self.promise_jobs.borrow_mut().push_back((p, snap));
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
        loop {
            let next_job = self.promise_jobs.borrow_mut().pop_front();
            if let Some((job, snap_opt)) = next_job {
                if let Some(snap) = snap_opt {
                    let prev = BOA_ALS_SNAP.with(|s| s.replace(snap));
                    let res = job.call(context);
                    BOA_ALS_SNAP.with(|s| *s.borrow_mut() = prev);
                    res?;
                } else {
                    job.call(context)?;
                }
            } else {
                self.fallback.clone().run_jobs(context)?;
                if self.promise_jobs.borrow().is_empty() {
                    break;
                }
            }
        }
        Ok(())
    }
}

#[inline]
pub(crate) fn norm_vfs_path(p: &str) -> String {
    slice_sandbox::normalize_vfs_path_lexical(p)
}

#[cold]
#[inline(never)]
pub(crate) fn record_boa_fs_violation(tenant_id: &str, path: &str) -> bool {
    BOA_VIOLATION.with(|v| {
        *v.borrow_mut() = Some(SandboxViolation::FsReadDenied {
            tenant_id: tenant_id.to_string(),
            path: path.to_string(),
        });
    });
    false
}

#[cold]
#[inline(never)]
pub(crate) fn record_boa_fs_write_violation(tenant_id: &str, path: &str) -> bool {
    BOA_VIOLATION.with(|v| {
        *v.borrow_mut() = Some(SandboxViolation::FsWriteDenied {
            tenant_id: tenant_id.to_string(),
            path: path.to_string(),
        });
    });
    false
}

#[inline]
pub(crate) fn check_boa_fs_read_ocap(path: &str) -> bool {
    if memchr::memmem::find(path.as_bytes(), b"..").is_some() {
        let tenant = BOA_ACTIVE_CAPS
            .with(|c| c.borrow().as_ref().map(|t| t.0.clone()))
            .unwrap_or_else(|| "boa-tenant".to_string());
        return record_boa_fs_violation(&tenant, path);
    }
    BOA_ACTIVE_CAPS.with(|c| {
        if let Some((tenant_id, read_prefixes, _)) = &*c.borrow() {
            if !read_prefixes.is_empty()
                && !read_prefixes
                    .iter()
                    .any(|prefix| slice_sandbox::vfs_prefix_matches(path, prefix))
            {
                return record_boa_fs_violation(tenant_id, path);
            }
        }
        true
    })
}

#[inline]
pub(crate) fn check_boa_fs_write_ocap(path: &str) -> bool {
    if memchr::memmem::find(path.as_bytes(), b"..").is_some() {
        let tenant = BOA_ACTIVE_CAPS
            .with(|c| c.borrow().as_ref().map(|t| t.0.clone()))
            .unwrap_or_else(|| "boa-tenant".to_string());
        return record_boa_fs_write_violation(&tenant, path);
    }
    BOA_ACTIVE_CAPS.with(|c| {
        if let Some((tenant_id, _, write_prefixes)) = &*c.borrow() {
            if !write_prefixes.is_empty()
                && !write_prefixes
                    .iter()
                    .any(|prefix| slice_sandbox::vfs_prefix_matches(path, prefix))
            {
                return record_boa_fs_write_violation(tenant_id, path);
            }
        }
        true
    })
}

#[inline]
pub(crate) fn try_acquire_pooled_boa_realm() -> Option<WarmBoaRealm> {
    BOA_CONTEXT_POOL.with(|pool| pool.borrow_mut().pop())
}

pub(crate) fn acquire_warm_boa_realm() -> WarmBoaRealm {
    try_acquire_pooled_boa_realm().unwrap_or_else(create_warm_boa_realm)
}

pub(crate) fn release_warm_boa_realm(realm: WarmBoaRealm) {
    BOA_CONTEXT_POOL.with(|pool| {
        let mut p = pool.borrow_mut();
        if p.len() < 8 {
            p.push(realm);
        }
    });
}

/// Creates a bare `boa_engine::Context` with `SliceJobExecutor` and core native Rust host functions
/// registered, without evaluating `ULTRA_CORE_PRELUDE` yet (used when executing pre-fused or
/// pre-compiled `SLICEBC\x01` payloads on cold start).
pub(crate) fn create_bare_boa_realm() -> WarmBoaRealm {
    let mut ctx = boa_engine::context::ContextBuilder::new()
        .job_executor(std::rc::Rc::new(SliceJobExecutor::new()))
        .build()
        .unwrap_or_else(|_| boa_engine::Context::default());
    register_slice_native_host_functions(&mut ctx);
    WarmBoaRealm {
        ctx,
        loaded_bundles: FxHashMap::default(),
        ses_check_fn: None,
        ensure_shard_fn: None,
        init_worker_self_fn: None,
        drain_wu_fn: None,
        loaded_shards_mask: 0,
    }
}

/// Refreshes the cached JS function handles (`__slice_check_ses`, `__slice_ensure_shard`, `__slice_drain_wu`)
/// on `realm` after `ULTRA_CORE_PRELUDE` has been evaluated.
#[inline]
pub(crate) fn populate_realm_cached_handles(realm: &mut WarmBoaRealm) {
    let global = realm.ctx.global_object();
    if realm.ses_check_fn.is_none() {
        realm.ses_check_fn = global
            .get(boa_engine::js_string!("__slice_check_ses"), &mut realm.ctx)
            .ok()
            .and_then(|v| v.as_object());
    }
    if realm.ensure_shard_fn.is_none() {
        realm.ensure_shard_fn = global
            .get(
                boa_engine::js_string!("__slice_ensure_shard"),
                &mut realm.ctx,
            )
            .ok()
            .and_then(|v| v.as_object());
    }
    if realm.drain_wu_fn.is_none() {
        realm.drain_wu_fn = global
            .get(boa_engine::js_string!("__slice_drain_wu"), &mut realm.ctx)
            .ok()
            .and_then(|v| v.as_object());
    }
}

/// Creates a warm `boa_engine::Context` initialized with Slice's native host functions,
/// WinterTC /WHATWG globals (`console.*`, `fetch`, `URL`, `Headers`, `Request`, `Response`,
/// `ReadableStream`, `WritableStream`, `TransformStream`, `WebCrypto`, `Buffer`, `structuredClone`),
/// and microtask/macrotask schedulers (`process.nextTick`, `queueMicrotask`, `setImmediate`, `setTimeout`).
pub(crate) fn create_warm_boa_realm() -> WarmBoaRealm {
    let mut realm = create_bare_boa_realm();
    let _ = crate::js_preludes::eval_ultra_core_prelude(&mut realm.ctx);
    populate_realm_cached_handles(&mut realm);
    realm
}

#[inline]
pub(crate) fn ensure_realm_shards(
    realm: &mut WarmBoaRealm,
    needed_mask: u32,
    init_lazy_getters: bool,
) {
    let missing = needed_mask & !realm.loaded_shards_mask;
    if init_lazy_getters && (realm.loaded_shards_mask & 1) == 0 {
        let _ = crate::js_preludes::eval_lazy_getters_init_prelude(&mut realm.ctx);
        realm.loaded_shards_mask |= 1;
    }
    if missing == 0 {
        return;
    }
    let lazy_getters_active = (realm.loaded_shards_mask & 1) != 0;
    for shard_id in [
        2u32, 13, 1, 11, 12, 10, 3, 14, 15, 4, 16, 5, 17, 6, 7, 8, 9, 18,
    ] {
        let bit = 1u32 << shard_id;
        if (missing & bit) != 0 {
            if lazy_getters_active {
                if let Some(ref ensure_fn) = realm.ensure_shard_fn {
                    let _ = ensure_fn.call(
                        &boa_engine::JsValue::undefined(),
                        &[boa_engine::JsValue::from(shard_id as i32)],
                        &mut realm.ctx,
                    );
                }
            } else {
                let _ = crate::js_preludes::eval_lazy_prelude_shard(shard_id, &mut realm.ctx);
            }
            realm.loaded_shards_mask |= bit;
            if shard_id == 8 && realm.init_worker_self_fn.is_none() {
                realm.init_worker_self_fn = realm
                    .ctx
                    .global_object()
                    .get(
                        boa_engine::js_string!("__slice_init_worker_self_env"),
                        &mut realm.ctx,
                    )
                    .ok()
                    .and_then(|v| v.as_object());
            }
        }
    }
}

/// RAII execution scope guard that snapshots all per-invocation thread-local isolate state
/// before a nested execution (`Slice.evalSandbox`, recursive sandbox invocation) and restores
/// every field on `Drop` (including on early return or panic), preventing any child sandbox
/// state (fuel budget, deterministic PRNG/clock, HTTP request/response, AsyncLocalStorage,
/// OCap capabilities, DLP secrets, or mock cassettes) from leaking into the parent isolate.
pub(crate) struct SavedIsolateContextGuard {
    pub(crate) saved_stdout: Vec<String>,
    pub(crate) saved_violation: Option<SandboxViolation>,
    pub(crate) saved_tenant: String,
    pub(crate) saved_caps: Option<ActiveBoaCaps>,
    pub(crate) saved_net_hosts: FxHashSet<String>,
    pub(crate) saved_egress_secrets: Vec<String>,
    pub(crate) saved_env_map: FxHashMap<String, String>,
    pub(crate) saved_exec_bins: Vec<String>,
    pub(crate) saved_last_cp_violation: Option<SandboxViolation>,
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
    pub(crate) saved_wait_until_count: usize,
    pub(crate) saved_dirty_subsystems: u8,
    pub(crate) saved_ev_loop: Option<crate::event_loop::SavedEventLoopSnapshot>,
}

impl SavedIsolateContextGuard {
    pub(crate) fn enter() -> Self {
        let saved_stdout = BOA_STDOUT.with(|out| std::mem::take(&mut *out.borrow_mut()));
        let saved_violation = BOA_VIOLATION.with(|v| v.borrow_mut().take());
        let saved_tenant = BOA_ACTIVE_TENANT.with(|t| std::mem::take(&mut *t.borrow_mut()));
        let saved_caps = BOA_ACTIVE_CAPS.with(|c| c.borrow_mut().take());
        let saved_net_hosts = BOA_NET_HOSTS.with(|h| std::mem::take(&mut *h.borrow_mut()));
        let saved_egress_secrets =
            BOA_EGRESS_SECRETS.with(|s| std::mem::take(&mut *s.borrow_mut()));
        let saved_env_map = BOA_ENV_MAP.with(|m| std::mem::take(&mut *m.borrow_mut()));
        let saved_exec_bins = BOA_EXEC_BINS.with(|b| std::mem::take(&mut *b.borrow_mut()));
        let saved_last_cp_violation = BOA_LAST_CHILD_PROC_VIOLATION.with(|v| v.borrow_mut().take());
        let saved_fetch_mocks = BOA_FETCH_MOCKS.with(|m| {
            let b = m.borrow();
            if b.is_empty() {
                None
            } else {
                Some(b.clone())
            }
        });
        let saved_ws_mocks = crate::websocket::BOA_WS_MOCKS.with(|m| {
            let b = m.borrow();
            if b.is_empty() {
                None
            } else {
                Some(b.clone())
            }
        });
        let saved_sock_mocks = crate::sockets::BOA_SOCKET_MOCKS.with(|m| {
            let b = m.borrow();
            if b.is_empty() {
                None
            } else {
                Some(b.clone())
            }
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
        let saved_wait_until_count = BOA_WAIT_UNTIL_COUNT.with(|c| c.replace(0));
        let saved_dirty_subsystems = BOA_DIRTY_SUBSYSTEMS.with(|d| d.replace(0));
        let saved_ev_loop = crate::event_loop::save_and_reset_event_loop_state();

        Self {
            saved_stdout,
            saved_violation,
            saved_tenant,
            saved_caps,
            saved_net_hosts,
            saved_egress_secrets,
            saved_env_map,
            saved_exec_bins,
            saved_last_cp_violation,
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
            saved_wait_until_count,
            saved_dirty_subsystems,
            saved_ev_loop,
        }
    }
}

impl Drop for SavedIsolateContextGuard {
    fn drop(&mut self) {
        if let Some(ev) = self.saved_ev_loop.take() {
            crate::event_loop::restore_event_loop_state(ev);
        }
        let child_dirty = BOA_DIRTY_SUBSYSTEMS.with(|d| d.get());
        BOA_DIRTY_SUBSYSTEMS.with(|d| d.set(self.saved_dirty_subsystems));
        BOA_STDOUT.with(|out| *out.borrow_mut() = std::mem::take(&mut self.saved_stdout));
        BOA_VIOLATION.with(|v| *v.borrow_mut() = self.saved_violation.take());
        BOA_ACTIVE_TENANT.with(|t| *t.borrow_mut() = std::mem::take(&mut self.saved_tenant));
        BOA_ACTIVE_CAPS.with(|c| *c.borrow_mut() = self.saved_caps.take());
        BOA_NET_HOSTS.with(|h| *h.borrow_mut() = std::mem::take(&mut self.saved_net_hosts));
        BOA_EGRESS_SECRETS
            .with(|s| *s.borrow_mut() = std::mem::take(&mut self.saved_egress_secrets));
        BOA_ENV_MAP.with(|m| *m.borrow_mut() = std::mem::take(&mut self.saved_env_map));
        BOA_EXEC_BINS.with(|b| *b.borrow_mut() = std::mem::take(&mut self.saved_exec_bins));
        BOA_LAST_CHILD_PROC_VIOLATION
            .with(|v| *v.borrow_mut() = self.saved_last_cp_violation.take());
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
        BOA_WAIT_UNTIL_COUNT.with(|c| c.set(self.saved_wait_until_count));
    }
}
