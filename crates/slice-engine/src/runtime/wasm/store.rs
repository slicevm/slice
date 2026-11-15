//! Shared per-isolate `wasmi::Store`, `SliceWasmLimiter`, re-entrant execution guards,
//! and JavaScript WebAssembly error/wrapper constructors.

use super::linking::{ManagedWasmFunc, ManagedWasmGlobal, ManagedWasmTable};
use super::memory::ManagedWasmMemory;
use super::{
    MAX_WASM_INSTANCES_PER_THREAD, MAX_WASM_MEMORY_PAGES, WASM_PAGE_BYTES, global_wasm_engine,
};
use crate::boa_realm::{
    BOA_ACTIVE_TENANT, BOA_FUEL_BUDGET, BOA_KV_MEM_BUDGET, BOA_VIOLATION, DIRTY_WASM,
    mark_subsystem_dirty,
};
use boa_engine::{Context, JsError, JsNativeError, JsObject, JsResult, JsValue, js_string};
use rustc_hash::FxHashMap;
use slice_sandbox::SandboxViolation;
use std::cell::{Cell, RefCell};
use wasmi::errors::{HostError, MemoryError, TableError};
use wasmi::{AsContextMut, Caller, ResourceLimiter, Store, StoreContextMut};
use wasmi_core::LimiterError;

/// Classification of WebAssembly specification errors constructed in JavaScript.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum WasmErrorKind {
    /// `WebAssembly.CompileError`
    Compile,
    /// `WebAssembly.LinkError`
    Link,
    /// `WebAssembly.RuntimeError`
    Runtime,
}

impl WasmErrorKind {
    #[inline]
    const fn ctor_name(self) -> &'static str {
        match self {
            Self::Compile => "CompileError",
            Self::Link => "LinkError",
            Self::Runtime => "RuntimeError",
        }
    }
}

/// Constructs an authentic `WebAssembly.CompileError`, `WebAssembly.LinkError`, or
/// `WebAssembly.RuntimeError` instance in the active Boa `Context`.
pub fn wasm_js_error(
    kind: WasmErrorKind,
    message: impl AsRef<str>,
    context: &mut Context,
) -> JsError {
    let msg = message.as_ref();
    if let Ok(wasm_ns) = context
        .global_object()
        .get(js_string!("WebAssembly"), context)
        && let Some(ns_obj) = wasm_ns.as_object()
        && let Ok(ctor_val) = ns_obj.get(js_string!(kind.ctor_name()), context)
        && let Some(ctor_obj) = ctor_val.as_object()
        && let Ok(err_obj) = ctor_obj.construct(&[JsValue::from(js_string!(msg))], None, context)
    {
        return JsError::from_opaque(err_obj.into());
    }
    JsNativeError::error()
        .with_message(format!("{}: {}", kind.ctor_name(), msg))
        .into()
}

/// Allocates a canonical JS wrapper (`WebAssembly.Memory`, `WebAssembly.Global`, or
/// `WebAssembly.Table`) inheriting from `WebAssembly[ctor_name].prototype`.
pub(crate) fn construct_wasm_wrapper(
    ctor_name: &str,
    id_prop: &str,
    id: u32,
    init_buf_null: bool,
    context: &mut Context,
) -> JsResult<JsObject> {
    let mut proto_opt = None;
    if let Ok(wasm_ns) = context
        .global_object()
        .get(js_string!("WebAssembly"), context)
        && let Some(ns_obj) = wasm_ns.as_object()
        && let Ok(ctor_val) = ns_obj.get(js_string!(ctor_name), context)
        && let Some(ctor_obj) = ctor_val.as_object()
        && let Ok(proto_val) = ctor_obj.get(js_string!("prototype"), context)
    {
        proto_opt = proto_val.as_object();
    }
    let obj = JsObject::with_object_proto(context.intrinsics());
    if let Some(proto) = proto_opt {
        obj.set_prototype(Some(proto));
    }
    let _ = obj.set(
        js_string!(id_prop),
        JsValue::from(id as f64),
        false,
        context,
    );
    if init_buf_null {
        let _ = obj.set(js_string!("_buf"), JsValue::null(), false, context);
    }
    Ok(obj)
}

/// Marker `HostError` used solely to unwind `wasmi` frames when a JS host import throws;
/// the exact unmangled `boa_engine::JsError` is carried in `PENDING_HOST_JS_ERROR`.
#[derive(Debug)]
pub(crate) struct WasmHostExceptionMarker;

impl std::fmt::Display for WasmHostExceptionMarker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WebAssembly host import threw an exception")
    }
}

impl std::error::Error for WasmHostExceptionMarker {}
impl HostError for WasmHostExceptionMarker {}

/// Enforces Slice's 5-Ring linear memory and table limits across all memories/tables in the
/// isolate's shared `wasmi::Store`.
#[derive(Default)]
pub(crate) struct SliceWasmLimiter {
    last_charged_delta: usize,
}

impl ResourceLimiter for SliceWasmLimiter {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> Result<bool, LimiterError> {
        self.last_charged_delta = 0;
        if maximum.is_some_and(|max| desired > max) {
            return Ok(false);
        }
        let max_sandbox_bytes = (MAX_WASM_MEMORY_PAGES as usize) * WASM_PAGE_BYTES;
        if desired > max_sandbox_bytes {
            record_wasm_memory_violation(desired, max_sandbox_bytes);
            return Err(LimiterError::ResourceLimiterDeniedAllocation);
        }
        let delta_bytes = desired.saturating_sub(current);
        let (used, limit) = BOA_KV_MEM_BUDGET.with(|m| m.get());
        if desired > limit || used.saturating_add(delta_bytes) > limit {
            record_wasm_memory_violation(desired.max(used.saturating_add(delta_bytes)), limit);
            return Err(LimiterError::ResourceLimiterDeniedAllocation);
        }
        if delta_bytes > 0 {
            BOA_KV_MEM_BUDGET.with(|m| m.set((used.saturating_add(delta_bytes), limit)));
            self.last_charged_delta = delta_bytes;
        }
        Ok(true)
    }

    fn memory_grow_failed(&mut self, _error: &MemoryError) -> Result<(), LimiterError> {
        if self.last_charged_delta > 0 {
            let delta = self.last_charged_delta;
            self.last_charged_delta = 0;
            let (used, limit) = BOA_KV_MEM_BUDGET.with(|m| m.get());
            BOA_KV_MEM_BUDGET.with(|m| m.set((used.saturating_sub(delta), limit)));
        }
        Ok(())
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> Result<bool, LimiterError> {
        Ok(desired <= 65_536 && maximum.is_none_or(|max| desired <= max))
    }

    fn table_grow_failed(&mut self, _error: &TableError) -> Result<(), LimiterError> {
        Ok(())
    }

    fn instances(&self) -> usize {
        MAX_WASM_INSTANCES_PER_THREAD
    }

    fn tables(&self) -> usize {
        64
    }

    fn memories(&self) -> usize {
        64
    }
}

#[derive(Default)]
pub(crate) struct WasmStoreData {
    pub(crate) limiter: SliceWasmLimiter,
}

#[derive(Default)]
pub(crate) struct WasmEntities {
    pub(crate) next_id: u32,
    pub(crate) memories: FxHashMap<u32, ManagedWasmMemory>,
    pub(crate) memory_ids: Vec<u32>,
    pub(crate) memory_by_key: FxHashMap<String, u32>,
    pub(crate) globals: FxHashMap<u32, ManagedWasmGlobal>,
    pub(crate) global_by_key: FxHashMap<String, u32>,
    pub(crate) tables: FxHashMap<u32, ManagedWasmTable>,
    pub(crate) table_by_key: FxHashMap<String, u32>,
    pub(crate) funcs: FxHashMap<u32, ManagedWasmFunc>,
    pub(crate) func_by_key: FxHashMap<String, u32>,
    pub(crate) import_closures: FxHashMap<u32, JsObject>,
    pub(crate) externref_values: FxHashMap<u32, JsValue>,
    pub(crate) instances: FxHashMap<u32, wasmi::Instance>,
}

impl WasmEntities {
    #[inline]
    pub(crate) fn alloc_id(&mut self) -> u32 {
        mark_subsystem_dirty(DIRTY_WASM);
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.next_id
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.memories.is_empty()
            && self.globals.is_empty()
            && self.tables.is_empty()
            && self.funcs.is_empty()
            && self.import_closures.is_empty()
            && self.externref_values.is_empty()
            && self.instances.is_empty()
    }
}

#[derive(Default)]
pub(crate) struct WasmThreadState {
    pub(crate) root_store: Option<Store<WasmStoreData>>,
    pub(crate) entities: WasmEntities,
    saved_caller_ptr: usize,
    saved_boa_ctx_ptr: usize,
    saved_pending_err: Option<JsError>,
}

impl WasmThreadState {
    #[inline]
    fn ensure_root_store(&mut self) -> &mut Store<WasmStoreData> {
        self.root_store.get_or_insert_with(|| {
            let mut store = Store::new(global_wasm_engine(), WasmStoreData::default());
            store.limiter(|data| &mut data.limiter);
            store
        })
    }

    #[inline]
    fn is_completely_idle(&self) -> bool {
        self.root_store.is_none() && self.entities.is_empty()
    }
}

thread_local! {
    pub(crate) static WASM_STATE: RefCell<WasmThreadState> = RefCell::new(WasmThreadState::default());
    static ACTIVE_BOA_CTX_PTR: Cell<*mut Context> = const { Cell::new(std::ptr::null_mut()) };
    static ACTIVE_CALLER_PTR: Cell<*mut Caller<'static, WasmStoreData>> = const { Cell::new(std::ptr::null_mut()) };
    static PENDING_HOST_JS_ERROR: RefCell<Option<JsError>> = const { RefCell::new(None) };
}

pub(crate) struct ActiveBoaCtxGuard {
    prev_ctx: *mut Context,
}

impl ActiveBoaCtxGuard {
    #[inline]
    pub(crate) fn enter(ctx: &mut Context) -> Self {
        let prev_ctx = ACTIVE_BOA_CTX_PTR.with(|p| p.replace(ctx as *mut Context));
        Self { prev_ctx }
    }
}

impl Drop for ActiveBoaCtxGuard {
    #[inline]
    fn drop(&mut self) {
        ACTIVE_BOA_CTX_PTR.with(|p| p.set(self.prev_ctx));
    }
}

#[inline]
pub(crate) fn active_boa_ctx_ptr() -> *mut Context {
    ACTIVE_BOA_CTX_PTR.with(|p| p.get())
}

pub(crate) struct ActiveCallerGuard {
    prev_caller: *mut Caller<'static, WasmStoreData>,
}

impl ActiveCallerGuard {
    #[inline]
    pub(crate) fn enter(caller: &mut Caller<'_, WasmStoreData>) -> Self {
        let raw =
            (caller as *mut Caller<'_, WasmStoreData>).cast::<Caller<'static, WasmStoreData>>();
        let prev_caller = ACTIVE_CALLER_PTR.with(|p| p.replace(raw));
        Self { prev_caller }
    }
}

impl Drop for ActiveCallerGuard {
    #[inline]
    fn drop(&mut self) {
        ACTIVE_CALLER_PTR.with(|p| p.set(self.prev_caller));
    }
}

#[inline]
pub(crate) fn set_pending_host_js_error(err: JsError) {
    PENDING_HOST_JS_ERROR.with(|slot| {
        *slot.borrow_mut() = Some(err);
    });
}

#[inline]
pub(crate) fn take_pending_host_js_error() -> Option<JsError> {
    PENDING_HOST_JS_ERROR.with(|slot| slot.borrow_mut().take())
}

/// Borrows both the active `StoreContextMut` (either the re-entrant host `Caller` or the
/// top-level `root_store`) and `WasmEntities` simultaneously without holding `WASM_STATE` across
/// Wasm bytecode execution.
pub(crate) fn with_store_and_entities<R>(
    f: impl FnOnce(StoreContextMut<'_, WasmStoreData>, &mut WasmEntities) -> R,
) -> R {
    let caller_ptr = ACTIVE_CALLER_PTR.with(|p| p.replace(std::ptr::null_mut()));
    if !caller_ptr.is_null() {
        struct RestoreCaller(*mut Caller<'static, WasmStoreData>);
        impl Drop for RestoreCaller {
            fn drop(&mut self) {
                ACTIVE_CALLER_PTR.with(|p| p.set(self.0));
            }
        }
        let _restore = RestoreCaller(caller_ptr);
        let caller = unsafe { &mut *caller_ptr };
        return WASM_STATE.with(|st| {
            let mut s = st.borrow_mut();
            f(caller.as_context_mut(), &mut s.entities)
        });
    }

    WASM_STATE.with(|st| {
        let mut s = st.borrow_mut();
        s.ensure_root_store();
        let WasmThreadState {
            root_store,
            entities,
            ..
        } = &mut *s;
        let store = root_store.as_mut().expect("root_store initialized");
        f(store.as_context_mut(), entities)
    })
}

struct RootStoreCheckoutGuard {
    store: Option<Store<WasmStoreData>>,
}

impl RootStoreCheckoutGuard {
    fn checkout() -> Self {
        let store = WASM_STATE.with(|st| {
            let mut s = st.borrow_mut();
            s.ensure_root_store();
            s.root_store.take()
        });
        Self { store }
    }

    fn as_mut(&mut self) -> &mut Store<WasmStoreData> {
        self.store.as_mut().expect("checked out root_store")
    }
}

impl Drop for RootStoreCheckoutGuard {
    fn drop(&mut self) {
        if let Some(store) = self.store.take() {
            WASM_STATE.with(|st| {
                let mut s = st.borrow_mut();
                if s.root_store.is_none() {
                    s.root_store = Some(store);
                }
            });
        }
    }
}

/// Executes Wasm bytecode (`Func::call` or `Linker::instantiate_and_start`) with `WASM_STATE`
/// completely unborrowed so host import callbacks can freely enter `with_store_and_entities` or
/// nested `exec_wasm_with_store`.
pub(crate) fn exec_wasm_with_store<R>(
    f: impl FnOnce(StoreContextMut<'_, WasmStoreData>) -> R,
) -> R {
    let caller_ptr = ACTIVE_CALLER_PTR.with(|p| p.replace(std::ptr::null_mut()));
    if !caller_ptr.is_null() {
        struct RestoreCaller(*mut Caller<'static, WasmStoreData>);
        impl Drop for RestoreCaller {
            fn drop(&mut self) {
                ACTIVE_CALLER_PTR.with(|p| p.set(self.0));
            }
        }
        let _restore = RestoreCaller(caller_ptr);
        let caller = unsafe { &mut *caller_ptr };
        return f(caller.as_context_mut());
    }

    let mut guard = RootStoreCheckoutGuard::checkout();
    f(guard.as_mut().as_context_mut())
}

/// Clears all per-turn Wasm instances, linear memories, globals, tables, functions, and imported
/// JS closures from thread-local state while preserving compiled `wasmi::Module` artifacts in
/// `GLOBAL_WASM_MODULE_CACHE`.
pub fn clear_thread_wasm_state() {
    WASM_STATE.with(|st| {
        *st.borrow_mut() = WasmThreadState::default();
    });
    ACTIVE_BOA_CTX_PTR.with(|p| p.set(std::ptr::null_mut()));
    ACTIVE_CALLER_PTR.with(|p| p.set(std::ptr::null_mut()));
    PENDING_HOST_JS_ERROR.with(|slot| {
        *slot.borrow_mut() = None;
    });
}

#[inline]
pub(crate) fn take_thread_wasm_state_if_nonempty() -> Option<WasmThreadState> {
    let caller_ptr = ACTIVE_CALLER_PTR.with(|p| p.replace(std::ptr::null_mut()));
    let boa_ptr = ACTIVE_BOA_CTX_PTR.with(|p| p.replace(std::ptr::null_mut()));
    let pending_err = PENDING_HOST_JS_ERROR.with(|slot| slot.borrow_mut().take());
    WASM_STATE.with(|st| {
        let mut s = st.borrow_mut();
        if s.is_completely_idle()
            && caller_ptr.is_null()
            && boa_ptr.is_null()
            && pending_err.is_none()
        {
            None
        } else {
            let mut taken = std::mem::take(&mut *s);
            taken.saved_caller_ptr = caller_ptr as usize;
            taken.saved_boa_ctx_ptr = boa_ptr as usize;
            taken.saved_pending_err = pending_err;
            Some(taken)
        }
    })
}

#[inline]
pub(crate) fn restore_thread_wasm_state(mut saved: WasmThreadState) {
    let caller_ptr = saved.saved_caller_ptr as *mut Caller<'static, WasmStoreData>;
    let boa_ptr = saved.saved_boa_ctx_ptr as *mut Context;
    let pending_err = saved.saved_pending_err.take();
    saved.saved_caller_ptr = 0;
    saved.saved_boa_ctx_ptr = 0;
    WASM_STATE.with(|st| {
        *st.borrow_mut() = saved;
    });
    ACTIVE_CALLER_PTR.with(|p| p.set(caller_ptr));
    ACTIVE_BOA_CTX_PTR.with(|p| p.set(boa_ptr));
    PENDING_HOST_JS_ERROR.with(|slot| {
        *slot.borrow_mut() = pending_err;
    });
}

#[cold]
#[inline(never)]
pub(crate) fn record_wasm_memory_violation(requested_bytes: usize, limit_bytes: usize) {
    let tenant_id = BOA_ACTIVE_TENANT.with(|t| {
        let b = t.borrow();
        if b.is_empty() {
            "wasm-tenant".to_string()
        } else {
            b.clone()
        }
    });
    BOA_VIOLATION.with(|v| {
        if v.borrow().is_none() {
            *v.borrow_mut() = Some(SandboxViolation::MemoryCeilingExceeded {
                tenant_id,
                requested: requested_bytes,
                limit: limit_bytes,
            });
        }
    });
}

#[cold]
#[inline(never)]
pub(crate) fn record_wasm_fuel_violation() {
    let tenant_id = BOA_ACTIVE_TENANT.with(|t| {
        let b = t.borrow();
        if b.is_empty() {
            "wasm-tenant".to_string()
        } else {
            b.clone()
        }
    });
    let limit = BOA_FUEL_BUDGET.with(|f| f.get().1);
    BOA_VIOLATION.with(|v| {
        if v.borrow().is_none() {
            *v.borrow_mut() = Some(SandboxViolation::CpuFuelExhausted { tenant_id, limit });
        }
    });
}
