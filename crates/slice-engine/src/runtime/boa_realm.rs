//! Warm `boa_engine` Realm Pool, Thread-Local Compartment State, and OCap VFS Guards.

use crate::host_bindings::register_slice_native_host_functions;
use crate::state_and_kv::StateCheckpoint;
use crate::value_and_ast::BytecodeModule;
use compact_str::CompactString;
use rustc_hash::{FxHashMap, FxHashSet};
use slice_sandbox::SandboxViolation;

pub(crate) const DIRTY_EVENT_LOOP: u16 = 1 << 0;
pub(crate) const DIRTY_ALS: u16 = 1 << 1;
pub(crate) const DIRTY_FETCH_MOCK: u16 = 1 << 2;
pub(crate) const DIRTY_WEBSOCKET: u16 = 1 << 3;
pub(crate) const DIRTY_SOCKETS: u16 = 1 << 4;
pub(crate) const DIRTY_WORKERS: u16 = 1 << 5;
pub(crate) const DIRTY_WASM: u16 = 1 << 6;
pub(crate) const DIRTY_PROCESS: u16 = 1 << 7;
pub(crate) const DIRTY_VFS: u16 = 1 << 8;

pub(crate) const EXEC_DIRTY_EXIT: u8 = 1 << 0;
pub(crate) const EXEC_DIRTY_REJECTIONS: u8 = 1 << 1;
pub(crate) const EXEC_DIRTY_UNCAUGHT: u8 = 1 << 2;
pub(crate) const EXEC_DIRTY_CHILD_PROC: u8 = 1 << 3;
pub(crate) const EXEC_DIRTY_ARGV: u8 = 1 << 4;

pub(crate) const REALM_CTL_RESET_MT: i32 = 0;
pub(crate) const REALM_CTL_SET_DET_MODE: i32 = 1;
pub(crate) const REALM_CTL_SYNC_ENV: i32 = 2;
pub(crate) const REALM_CTL_SET_NEXT_TICK: i32 = 3;
pub(crate) const REALM_CTL_RESET_WU: i32 = 4;
pub(crate) const REALM_CTL_DISPATCH_HTTP_BRIDGE: i32 = 5;

pub(crate) const HOST_GROUP_HTTP_BRIDGE: u8 = 1 << 0;
pub(crate) const HOST_GROUP_SEMVER_FAST: u8 = 1 << 1;
pub(crate) const HOST_GROUP_KV_SANDBOX: u8 = 1 << 2;
pub(crate) const HOST_GROUP_DET: u8 = 1 << 3;

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
    pub(crate) owner_tenant: CompactString,
    pub(crate) ctx: boa_engine::Context,
    pub(crate) loaded_bundles: FxHashMap<u64, boa_engine::JsObject>,
    pub(crate) loaded_bundle_flags: FxHashMap<u64, u16>,
    pub(crate) loaded_bundle_fused_specs: FxHashMap<u64, u64>,
    pub(crate) compiled_scripts: FxHashMap<u64, boa_engine::Script>,
    pub(crate) ses_check_fn: Option<boa_engine::JsObject>,
    pub(crate) ensure_shard_fn: Option<boa_engine::JsObject>,
    pub(crate) init_worker_self_fn: Option<boa_engine::JsObject>,
    pub(crate) drain_wu_fn: Option<boa_engine::JsObject>,
    pub(crate) ctl_fn: Option<boa_engine::JsObject>,
    pub(crate) ses_proto_sigs: [(u64, usize, usize); 6],
    pub(crate) loaded_shards_mask: u32,
    pub(crate) registered_host_groups: u8,
    pub(crate) has_pending_wu: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct BoaFsCaps {
    pub(crate) read_prefixes: Vec<String>,
    pub(crate) write_prefixes: Vec<String>,
}

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
    pub(crate) static BOA_VFS_TOMBSTONES: std::cell::RefCell<FxHashSet<String>> =
        std::cell::RefCell::new(FxHashSet::default());
    pub(crate) static BOA_VFS_MTIMES: std::cell::RefCell<FxHashMap<String, f64>> =
        std::cell::RefCell::new(FxHashMap::default());
    pub(crate) static BOA_VFS_OWNER_TENANT: std::cell::RefCell<CompactString> =
        const { std::cell::RefCell::new(CompactString::const_new("")) };
    pub(crate) static BOA_ACTIVE_CAPS: std::cell::RefCell<BoaFsCaps> =
        const {
            std::cell::RefCell::new(BoaFsCaps {
                read_prefixes: Vec::new(),
                write_prefixes: Vec::new(),
            })
        };
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
    pub(crate) static BOA_ACTIVE_SOURCE_FUSED_SPEC: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };
    pub(crate) static BOA_TRACE_ENABLED: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
    pub(crate) static BOA_TRACE_EVENTS: std::cell::RefCell<Vec<ChromeTraceEvent>> =
        const { std::cell::RefCell::new(Vec::new()) };
    pub(crate) static BOA_DIRTY_SUBSYSTEMS: std::cell::Cell<u16> =
        const { std::cell::Cell::new(0) };
    pub(crate) static BOA_EXEC_DIRTY: std::cell::Cell<u8> =
        const { std::cell::Cell::new(0) };
    pub(crate) static BOA_CUSTOM_ARGV: std::cell::RefCell<Option<Vec<String>>> =
        const { std::cell::RefCell::new(None) };
    pub(crate) static BOA_PROCESS_EXIT: std::cell::Cell<Option<i32>> =
        const { std::cell::Cell::new(None) };
    pub(crate) static BOA_LAST_EXIT_CODE: std::cell::Cell<Option<i32>> =
        const { std::cell::Cell::new(None) };
    pub(crate) static BOA_UNHANDLED_REJECTIONS: std::cell::RefCell<Vec<boa_engine::JsObject>> =
        const { std::cell::RefCell::new(Vec::new()) };
    pub(crate) static BOA_UNCAUGHT_ERROR: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

#[inline]
pub(crate) fn mark_exec_dirty(bit: u8) {
    BOA_EXEC_DIRTY.with(|c| c.set(c.get() | bit));
}

#[inline]
pub(crate) fn set_process_exit(code: i32) {
    mark_exec_dirty(EXEC_DIRTY_EXIT);
    BOA_PROCESS_EXIT.with(|e| e.set(Some(code)));
    BOA_LAST_EXIT_CODE.with(|e| e.set(Some(code)));
}

#[inline]
pub(crate) fn set_last_exit_code(code: i32) {
    mark_exec_dirty(EXEC_DIRTY_EXIT);
    BOA_LAST_EXIT_CODE.with(|e| e.set(Some(code)));
}

#[inline]
pub(crate) fn set_uncaught_error(err: String) {
    mark_exec_dirty(EXEC_DIRTY_UNCAUGHT);
    BOA_UNCAUGHT_ERROR.with(|e| *e.borrow_mut() = Some(err));
}

#[inline]
pub(crate) fn set_child_proc_violation(violation: SandboxViolation) {
    mark_exec_dirty(EXEC_DIRTY_CHILD_PROC);
    BOA_LAST_CHILD_PROC_VIOLATION.with(|v| *v.borrow_mut() = Some(violation));
}

#[inline]
pub(crate) fn has_process_exit() -> bool {
    (BOA_EXEC_DIRTY.with(|c| c.get()) & EXEC_DIRTY_EXIT) != 0
        && BOA_PROCESS_EXIT.with(|e| e.get().is_some())
}

#[inline]
pub(crate) fn has_uncaught_error() -> bool {
    (BOA_EXEC_DIRTY.with(|c| c.get()) & EXEC_DIRTY_UNCAUGHT) != 0
        && BOA_UNCAUGHT_ERROR.with(|e| e.borrow().is_some())
}

#[derive(Debug, Clone)]
struct AlsCapturedSnap(boa_engine::JsValue);

impl boa_engine::gc::Finalize for AlsCapturedSnap {}

unsafe impl boa_engine::gc::Trace for AlsCapturedSnap {
    unsafe fn trace(&self, tracer: &mut boa_engine::gc::Tracer) {
        unsafe { self.0.trace(tracer) }
    }
    unsafe fn trace_non_roots(&self) {
        unsafe { self.0.trace_non_roots() }
    }
    fn run_finalizer(&self) {
        self.0.run_finalizer();
    }
}

impl boa_engine::JsData for AlsCapturedSnap {}

pub(crate) struct SliceHostHooks;

impl boa_engine::context::HostHooks for SliceHostHooks {
    fn make_job_callback(
        &self,
        callback: boa_engine::object::builtins::JsFunction,
        _context: &mut boa_engine::Context,
    ) -> boa_engine::job::JobCallback {
        if BOA_ALS_ACTIVE.with(|a| a.get()) {
            let snap = BOA_ALS_SNAP.with(|s| s.borrow().clone());
            return boa_engine::job::JobCallback::new(callback, AlsCapturedSnap(snap));
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
        if let Some(captured) = job.host_defined().downcast_ref::<AlsCapturedSnap>() {
            let prev = BOA_ALS_SNAP.with(|s| s.replace(captured.0.clone()));
            let res = job.callback().call(this, args, context);
            BOA_ALS_SNAP.with(|s| *s.borrow_mut() = prev);
            return res;
        }
        let prev = BOA_ALS_SNAP.with(|s| s.replace(boa_engine::JsValue::undefined()));
        let res = job.callback().call(this, args, context);
        BOA_ALS_SNAP.with(|s| *s.borrow_mut() = prev);
        res
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

#[inline]
pub(crate) fn mark_subsystem_dirty(bit: u16) {
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
    prev_fused_spec: u64,
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
        let prev_fused_spec = BOA_ACTIVE_SOURCE_FUSED_SPEC.with(|c| c.replace(0));
        Self {
            prev,
            prev_fused_spec,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<'a> Drop for ActiveSourceSliceGuard<'a> {
    #[inline]
    fn drop(&mut self) {
        BOA_ACTIVE_SOURCE_PTR_LEN.with(|c| c.set(self.prev));
        BOA_ACTIVE_SOURCE_FUSED_SPEC.with(|c| c.set(self.prev_fused_spec));
    }
}

#[inline]
pub(crate) fn encode_fused_source_spec(shards_mask: u32, init_lazy: bool) -> u64 {
    (1u64 << 63) | ((u64::from(init_lazy)) << 32) | u64::from(shards_mask)
}

#[inline]
pub(crate) fn set_active_source_fused_spec(spec: u64) {
    BOA_ACTIVE_SOURCE_FUSED_SPEC.with(|c| c.set(spec));
}

#[cold]
fn active_source_line_offset() -> usize {
    let spec = BOA_ACTIVE_SOURCE_FUSED_SPEC.with(|c| c.get());
    if (spec & (1u64 << 63)) == 0 {
        return 0;
    }
    let shards_mask = spec as u32;
    let init_lazy = ((spec >> 32) & 1) != 0;
    crate::js_preludes::count_fused_prelude_newlines(shards_mask, init_lazy)
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
    let line_offset = active_source_line_offset();
    slice_cas::map_bundled_stack_trace_with_offset(active_src, raw_stack, line_offset)
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
    let line_offset = active_source_line_offset();
    slice_cas::map_bundled_error_location_with_offset(active_src, raw_error, line_offset)
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

#[inline]
pub(crate) fn norm_vfs_path(p: &str) -> String {
    slice_sandbox::normalize_vfs_path_lexical(p)
}

#[cold]
#[inline(never)]
pub(crate) fn record_boa_fs_violation(tenant_id: &str, path: &str) -> bool {
    BOA_VIOLATION.with(|v| {
        let mut slot = v.borrow_mut();
        if slot.is_none() {
            *slot = Some(SandboxViolation::FsReadDenied {
                tenant_id: tenant_id.to_string(),
                path: path.to_string(),
            });
        }
    });
    false
}

#[cold]
#[inline(never)]
pub(crate) fn record_boa_fs_write_violation(tenant_id: &str, path: &str) -> bool {
    BOA_VIOLATION.with(|v| {
        let mut slot = v.borrow_mut();
        if slot.is_none() {
            *slot = Some(SandboxViolation::FsWriteDenied {
                tenant_id: tenant_id.to_string(),
                path: path.to_string(),
            });
        }
    });
    false
}

#[cold]
#[inline(never)]
fn current_boa_tenant_or_default() -> String {
    BOA_ACTIVE_TENANT.with(|t| {
        let b = t.borrow();
        if b.is_empty() {
            "boa-tenant".to_string()
        } else {
            b.clone()
        }
    })
}

/// Side-effect-free predicate checking whether the active compartment currently holds OCap read
/// authority for `path` without recording a `SandboxViolation` when denied.
#[inline]
pub(crate) fn has_boa_fs_read_ocap(path: &str) -> bool {
    if slice_sandbox::is_rejected_vfs_candidate(path) {
        return false;
    }
    BOA_ACTIVE_CAPS.with(|c| {
        let caps = c.borrow();
        if caps.read_prefixes.is_empty() {
            return false;
        }
        let norm = slice_sandbox::normalize_vfs_path_cow(path);
        caps.read_prefixes
            .iter()
            .any(|prefix| slice_sandbox::vfs_normalized_candidate_matches_prefix(&norm, prefix))
    })
}

#[inline]
pub(crate) fn check_boa_fs_read_ocap(path: &str) -> bool {
    if !has_boa_fs_read_ocap(path) {
        let tenant = current_boa_tenant_or_default();
        return record_boa_fs_violation(&tenant, path);
    }
    true
}

#[inline]
pub(crate) fn check_boa_fs_write_ocap(path: &str) -> bool {
    if slice_sandbox::is_rejected_vfs_candidate(path) {
        let tenant = current_boa_tenant_or_default();
        return record_boa_fs_write_violation(&tenant, path);
    }
    let allowed = BOA_ACTIVE_CAPS.with(|c| {
        let caps = c.borrow();
        if caps.write_prefixes.is_empty() {
            return false;
        }
        let norm = slice_sandbox::normalize_vfs_path_cow(path);
        caps.write_prefixes
            .iter()
            .any(|prefix| slice_sandbox::vfs_normalized_candidate_matches_prefix(&norm, prefix))
    });
    if !allowed {
        let tenant = current_boa_tenant_or_default();
        return record_boa_fs_write_violation(&tenant, path);
    }
    mark_subsystem_dirty(DIRTY_VFS);
    true
}

#[inline]
pub(crate) fn vfs_owner_namespace(tenant_id: &str) -> &str {
    tenant_id.strip_suffix(":sub").unwrap_or(tenant_id)
}

#[inline]
pub(crate) fn ensure_vfs_tenant_affinity(vfs_ns: &str) {
    BOA_VFS_OWNER_TENANT.with(|owner| {
        let mut cur = owner.borrow_mut();
        if cur.as_str() != vfs_ns {
            BOA_VFS_CACHE.with(|c| c.borrow_mut().clear());
            BOA_VFS_DIRS.with(|d| d.borrow_mut().clear());
            BOA_VFS_TOMBSTONES.with(|t| t.borrow_mut().clear());
            BOA_VFS_MTIMES.with(|m| m.borrow_mut().clear());
            cur.clear();
            cur.push_str(vfs_ns);
        }
    });
}

#[inline]
fn proto_sig(obj: &boa_engine::JsObject) -> (u64, usize, usize) {
    let b = obj.borrow();
    (
        b.shape().id(),
        b.properties().storage_len(),
        b.properties().index_property_values().len(),
    )
}

#[inline]
pub(crate) fn capture_ses_proto_sigs(ctx: &boa_engine::Context) -> [(u64, usize, usize); 6] {
    let c = ctx.intrinsics().constructors();
    [
        proto_sig(&c.object().prototype()),
        proto_sig(&c.array().prototype()),
        proto_sig(&c.function().prototype()),
        proto_sig(&c.string().prototype()),
        proto_sig(&c.promise().prototype()),
        proto_sig(&c.typed_uint8_array().prototype()),
    ]
}

#[inline]
pub(crate) fn has_ses_proto_sig_mismatch(realm: &WarmBoaRealm) -> bool {
    capture_ses_proto_sigs(&realm.ctx) != realm.ses_proto_sigs
}

#[inline]
pub(crate) fn try_acquire_pooled_boa_realm_for_tenant(tenant_id: &str) -> Option<WarmBoaRealm> {
    BOA_CONTEXT_POOL.with(|pool| {
        let mut p = pool.borrow_mut();
        if p.is_empty() {
            return None;
        }
        if tenant_id.is_empty() {
            let idx = p.iter().rposition(|r| r.owner_tenant.is_empty())?;
            return Some(p.remove(idx));
        }
        if p.last().is_some_and(|r| r.owner_tenant == tenant_id) {
            return p.pop();
        }
        if let Some(idx) = p.iter().rposition(|r| r.owner_tenant == tenant_id) {
            return Some(p.remove(idx));
        }
        if let Some(idx) = p.iter().rposition(|r| r.owner_tenant.is_empty()) {
            let mut realm = p.remove(idx);
            realm.owner_tenant = CompactString::new(tenant_id);
            return Some(realm);
        }
        None
    })
}

#[inline]
pub(crate) fn try_acquire_pooled_boa_realm() -> Option<WarmBoaRealm> {
    try_acquire_pooled_boa_realm_for_tenant("")
}

#[inline]
pub(crate) fn acquire_warm_boa_realm_for_tenant(tenant_id: &str) -> WarmBoaRealm {
    try_acquire_pooled_boa_realm_for_tenant(tenant_id)
        .unwrap_or_else(|| create_warm_boa_realm_for_tenant(tenant_id))
}

pub(crate) fn acquire_warm_boa_realm() -> WarmBoaRealm {
    acquire_warm_boa_realm_for_tenant("")
}

pub(crate) fn release_warm_boa_realm(realm: WarmBoaRealm) {
    BOA_CONTEXT_POOL.with(|pool| {
        let mut p = pool.borrow_mut();
        if p.len() >= 8 {
            p.remove(0);
        }
        p.push(realm);
    });
}

/// Creates a bare `boa_engine::Context` with `SliceJobExecutor` and core native Rust host functions
/// registered, without evaluating `ULTRA_CORE_PRELUDE` yet (used when executing pre-fused or
/// pre-compiled `SLICEBC\x01` payloads on cold start).
pub(crate) fn create_bare_boa_realm_for_tenant(tenant_id: &str) -> WarmBoaRealm {
    let mut ctx = boa_engine::context::ContextBuilder::new()
        .job_executor(std::rc::Rc::new(SliceJobExecutor::new()))
        .host_hooks(std::rc::Rc::new(SliceHostHooks))
        .build()
        .unwrap_or_else(|_| boa_engine::Context::default());
    register_slice_native_host_functions(&mut ctx);
    let ses_proto_sigs = capture_ses_proto_sigs(&ctx);
    WarmBoaRealm {
        owner_tenant: CompactString::new(tenant_id),
        ctx,
        loaded_bundles: FxHashMap::default(),
        loaded_bundle_flags: FxHashMap::default(),
        loaded_bundle_fused_specs: FxHashMap::default(),
        compiled_scripts: FxHashMap::default(),
        ses_check_fn: None,
        ensure_shard_fn: None,
        init_worker_self_fn: None,
        drain_wu_fn: None,
        ctl_fn: None,
        ses_proto_sigs,
        loaded_shards_mask: 0,
        registered_host_groups: 0,
        has_pending_wu: false,
    }
}

#[inline]
pub(crate) fn create_bare_boa_realm() -> WarmBoaRealm {
    create_bare_boa_realm_for_tenant("")
}

/// Refreshes the cached JS function handles (`__slice_check_ses`, `__slice_ensure_shard`,
/// `__slice_drain_wu`, `__slice_ctl`) on `realm` after `ULTRA_CORE_PRELUDE` has been evaluated.
#[inline]
pub(crate) fn populate_realm_cached_handles(realm: &mut WarmBoaRealm) {
    let global = realm.ctx.global_object();
    if realm.ses_check_fn.is_none() {
        realm.ses_check_fn = global
            .get(boa_engine::js_string!("__slice_check_ses"), &mut realm.ctx)
            .ok()
            .and_then(|v| v.as_object());
        realm.ses_proto_sigs = capture_ses_proto_sigs(&realm.ctx);
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
    if realm.ctl_fn.is_none() {
        realm.ctl_fn = global
            .get(boa_engine::js_string!("__slice_ctl"), &mut realm.ctx)
            .ok()
            .and_then(|v| v.as_object());
    }
}

#[inline]
pub(crate) fn call_realm_ctl(realm: &mut WarmBoaRealm, op: i32, flag: bool) {
    if realm.ctl_fn.is_none() {
        populate_realm_cached_handles(realm);
    }
    if let Some(ctl) = realm.ctl_fn.clone() {
        let _ = ctl.call(
            &boa_engine::JsValue::undefined(),
            &[
                boa_engine::JsValue::from(op),
                boa_engine::JsValue::from(flag),
            ],
            &mut realm.ctx,
        );
    }
}

#[inline]
fn fnv1a_script_key(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ (bytes.len() as u64);
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Evaluates a scoped JS script (`{ ... }`) on a warm realm, caching its compiled `boa_engine::Script`
/// (`Gc<CodeBlock>`) so subsequent warm invocations skip lexing, parsing, scope analysis, and byte compilation.
pub(crate) fn eval_or_reuse_cached_script(
    realm: &mut WarmBoaRealm,
    scoped_js: &str,
) -> boa_engine::JsResult<boa_engine::JsValue> {
    let key = fnv1a_script_key(scoped_js.as_bytes());
    if let Some(cached) = realm.compiled_scripts.get(&key).cloned() {
        return cached.evaluate(&mut realm.ctx);
    }
    let script = boa_engine::Script::parse(
        boa_engine::Source::from_bytes(scoped_js.as_bytes()),
        None,
        &mut realm.ctx,
    )?;
    let reusable = script.can_reuse_across_runs(&mut realm.ctx);
    let res = script.evaluate(&mut realm.ctx);
    if reusable
        && res.is_ok()
        && BOA_VIOLATION.with(|v| v.borrow().is_none())
        && realm.compiled_scripts.len() < 32
    {
        realm.compiled_scripts.insert(key, script);
    }
    res
}

/// Creates a warm `boa_engine::Context` initialized with Slice's native host functions,
/// WinterTC /WHATWG globals (`console.*`, `fetch`, `URL`, `Headers`, `Request`, `Response`,
/// `ReadableStream`, `WritableStream`, `TransformStream`, `WebCrypto`, `Buffer`, `structuredClone`),
/// and microtask/macrotask schedulers (`process.nextTick`, `queueMicrotask`, `setImmediate`, `setTimeout`).
pub(crate) fn create_warm_boa_realm_for_tenant(tenant_id: &str) -> WarmBoaRealm {
    let mut realm = create_bare_boa_realm_for_tenant(tenant_id);
    let _ = crate::js_preludes::eval_ultra_core_prelude(&mut realm.ctx);
    populate_realm_cached_handles(&mut realm);
    realm
}

pub(crate) fn create_warm_boa_realm() -> WarmBoaRealm {
    create_warm_boa_realm_for_tenant("")
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
    for shard_id in crate::js_preludes::SHARD_LOAD_ORDER {
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
            if shard_id == 18 {
                realm.ses_proto_sigs = capture_ses_proto_sigs(&realm.ctx);
            }
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
    pub(crate) saved_caps: BoaFsCaps,
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
    pub(crate) saved_dirty_subsystems: u16,
    pub(crate) saved_exec_dirty: u8,
    pub(crate) saved_custom_argv: Option<Vec<String>>,
    pub(crate) saved_process_exit: Option<i32>,
    pub(crate) saved_last_exit_code: Option<i32>,
    pub(crate) saved_unhandled_rejections: Vec<boa_engine::JsObject>,
    pub(crate) saved_uncaught_error: Option<String>,
    pub(crate) saved_ev_loop: Option<crate::event_loop::SavedEventLoopSnapshot>,
    pub(crate) saved_wasm_state: Option<crate::wasm::WasmThreadState>,
}

impl SavedIsolateContextGuard {
    pub(crate) fn enter() -> Self {
        let saved_exec_dirty = BOA_EXEC_DIRTY.with(|c| c.replace(0));
        let saved_stdout = BOA_STDOUT.with(|out| std::mem::take(&mut *out.borrow_mut()));
        let saved_violation = BOA_VIOLATION.with(|v| v.borrow_mut().take());
        let saved_tenant = BOA_ACTIVE_TENANT.with(|t| std::mem::take(&mut *t.borrow_mut()));
        let saved_caps = BOA_ACTIVE_CAPS.with(|c| std::mem::take(&mut *c.borrow_mut()));
        let saved_net_hosts = BOA_NET_HOSTS.with(|h| std::mem::take(&mut *h.borrow_mut()));
        let saved_egress_secrets =
            BOA_EGRESS_SECRETS.with(|s| std::mem::take(&mut *s.borrow_mut()));
        let saved_env_map = BOA_ENV_MAP.with(|m| std::mem::take(&mut *m.borrow_mut()));
        let saved_exec_bins = BOA_EXEC_BINS.with(|b| std::mem::take(&mut *b.borrow_mut()));
        let saved_last_cp_violation = if (saved_exec_dirty & EXEC_DIRTY_CHILD_PROC) != 0 {
            BOA_LAST_CHILD_PROC_VIOLATION.with(|v| v.borrow_mut().take())
        } else {
            None
        };
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
            saved_exec_dirty,
            saved_custom_argv,
            saved_process_exit,
            saved_last_exit_code,
            saved_unhandled_rejections,
            saved_uncaught_error,
            saved_ev_loop,
            saved_wasm_state,
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
        BOA_EGRESS_SECRETS
            .with(|s| *s.borrow_mut() = std::mem::take(&mut self.saved_egress_secrets));
        BOA_ENV_MAP.with(|m| *m.borrow_mut() = std::mem::take(&mut self.saved_env_map));
        BOA_EXEC_BINS.with(|b| *b.borrow_mut() = std::mem::take(&mut self.saved_exec_bins));
        if (union_exec_dirty & EXEC_DIRTY_CHILD_PROC) != 0 {
            BOA_LAST_CHILD_PROC_VIOLATION
                .with(|v| *v.borrow_mut() = self.saved_last_cp_violation.take());
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
        BOA_WAIT_UNTIL_COUNT.with(|c| c.set(self.saved_wait_until_count));
    }
}
