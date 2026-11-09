//! Pure-Rust WebAssembly (`WebAssembly.*`) Execution Engine & 5-Ring Security Cage (`wasmi`).
//!
//! Provides a zero-JIT-warmup, register-machine WebAssembly runtime integrated directly with
//! Slice's `SliceIsolate` resource cage:
//! - **Speculative Process-Wide Module Cache (`GLOBAL_WASM_MODULE_CACHE`):** Validates and compiles
//!   each unique `.wasm` binary at most once per process (`O(1)` `Arc`-backed `wasmi::Module` clone).
//! - **Ring 1 — Binary & Section Quota Guard:** Enforces `MAX_WASM_BINARY_BYTES` (`8 MB`) and strict
//!   `\0asm\x01\0\0\0` header validation before translation.
//! - **Ring 2 — Unified Deterministic CPU Fuel Metering:** Shares the active `SliceIsolate`'s
//!   instruction fuel budget (`BOA_FUEL_BUDGET`), deterministically trapping infinite WASM loops
//!   with `SandboxViolation::CpuFuelExhausted`.
//! - **Ring 3 — Unified Linear Memory Cage:** Enforces per-tenant byte ceilings (`BOA_KV_MEM_BUDGET`)
//!   and `MAX_WASM_MEMORY_PAGES` (`256` pages = `16 MB`) on WASM linear memory creation and growth.
//! - **Ring 4 — Zero Ambient WASI Authority & Host Import Firewall:** Pure compute sandbox with
//!   zero ambient filesystem/network authority and fuel-metered JS host import callbacks.
//! - **Ring 5 — Bidirectional Shared Linear Memory (`JsArrayBuffer` <-> `wasmi::Memory`):**
//!   Word-aligned delta synchronization between JS `Uint8Array(memory.buffer)` views and WASM
//!   linear memory without intermediate serialization.

use crate::boa_realm::{
    mark_subsystem_dirty, BOA_ACTIVE_TENANT, BOA_FUEL_BUDGET, BOA_KV_MEM_BUDGET, BOA_VIOLATION,
    DIRTY_WASM,
};
use boa_engine::object::builtins::JsArrayBuffer;
use boa_engine::{js_string, Context, JsNativeError, JsObject, JsResult, JsValue};
use rustc_hash::FxHashMap;
use slice_sandbox::SandboxViolation;
use std::cell::{Cell, RefCell};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use wasmi::errors::HostError;
use wasmi::{
    Caller, CompilationMode, Config, Engine, Error as WasmiError, Extern, ExternType, Func,
    FuncType, Global, GlobalType, Linker, Memory, MemoryType, Module, Mutability, Store,
    StoreLimits, StoreLimitsBuilder, TrapCode, Val, ValType,
};

/// Maximum `.wasm` binary size accepted by the sandbox (`8 MB`).
pub const MAX_WASM_BINARY_BYTES: usize = 8 * 1024 * 1024;

/// Maximum WASM linear memory pages per instance (`256` pages = `16 MB`).
pub const MAX_WASM_MEMORY_PAGES: u32 = 256;

/// Standard WebAssembly linear memory page size (`64 KB`).
pub const WASM_PAGE_BYTES: usize = 65_536;

/// Maximum concurrent WASM instances retained per thread.
pub const MAX_WASM_INSTANCES_PER_THREAD: usize = 64;

static GLOBAL_WASM_ENGINE: OnceLock<Engine> = OnceLock::new();
static GLOBAL_WASM_MODULE_CACHE: OnceLock<Mutex<FxHashMap<(u64, usize), Module>>> = OnceLock::new();
static WASM_MODULE_CACHE_HITS: AtomicU64 = AtomicU64::new(0);

#[inline]
fn global_wasm_engine() -> &'static Engine {
    GLOBAL_WASM_ENGINE.get_or_init(|| {
        let mut config = Config::default();
        config.consume_fuel(true);
        config.compilation_mode(CompilationMode::LazyTranslation);
        Engine::new(&config)
    })
}

#[inline]
fn global_wasm_module_cache() -> &'static Mutex<FxHashMap<(u64, usize), Module>> {
    GLOBAL_WASM_MODULE_CACHE.get_or_init(|| Mutex::new(FxHashMap::default()))
}

/// Returns the number of process-wide `GLOBAL_WASM_MODULE_CACHE` hits.
pub fn wasm_module_cache_hits() -> u64 {
    WASM_MODULE_CACHE_HITS.load(Ordering::Relaxed)
}

/// Resets the `WASM_MODULE_CACHE_HITS` telemetry counter.
pub fn reset_wasm_telemetry() {
    WASM_MODULE_CACHE_HITS.store(0, Ordering::Relaxed);
}

#[derive(Debug)]
struct WasmHostError(String);

impl std::fmt::Display for WasmHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for WasmHostError {}
impl HostError for WasmHostError {}

struct WasmStoreData {
    limits: StoreLimits,
    last_host_error: Option<String>,
}

struct CompiledModuleEntry {
    module: Module,
    raw_bytes: Vec<u8>,
}

struct ActiveWasmInstance {
    store: Store<WasmStoreData>,
    instance: wasmi::Instance,
    memory: Option<Memory>,
    max_memory_pages: Option<u32>,
    shadow_mem: Vec<u8>,
}

struct StandaloneMemoryEntry {
    pages: u32,
    max_pages: Option<u32>,
}

#[derive(Default)]
struct WasmThreadState {
    next_id: u32,
    modules: FxHashMap<u32, CompiledModuleEntry>,
    instances: FxHashMap<u32, ActiveWasmInstance>,
    memories: FxHashMap<u32, StandaloneMemoryEntry>,
    import_funcs: FxHashMap<u32, JsObject>,
}

impl WasmThreadState {
    fn alloc_id(&mut self) -> u32 {
        mark_subsystem_dirty(DIRTY_WASM);
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.next_id
    }
}

thread_local! {
    static WASM_STATE: RefCell<WasmThreadState> = RefCell::new(WasmThreadState::default());
    static ACTIVE_BOA_CTX_PTR: Cell<*mut Context> = const { Cell::new(std::ptr::null_mut()) };
    static ACTIVE_MEM_AB_PTR: Cell<*const JsObject> = const { Cell::new(std::ptr::null()) };
}

struct ActiveBoaCtxGuard {
    prev_ctx: *mut Context,
    prev_ab: *const JsObject,
}

impl ActiveBoaCtxGuard {
    fn enter(ctx: &mut Context, mem_ab: Option<&JsObject>) -> Self {
        let prev_ctx = ACTIVE_BOA_CTX_PTR.with(|p| p.replace(ctx as *mut Context));
        let ab_ptr = mem_ab
            .map(|o| o as *const JsObject)
            .unwrap_or(std::ptr::null());
        let prev_ab = ACTIVE_MEM_AB_PTR.with(|p| p.replace(ab_ptr));
        Self { prev_ctx, prev_ab }
    }
}

impl Drop for ActiveBoaCtxGuard {
    fn drop(&mut self) {
        ACTIVE_BOA_CTX_PTR.with(|p| p.set(self.prev_ctx));
        ACTIVE_MEM_AB_PTR.with(|p| p.set(self.prev_ab));
    }
}

/// Clears transient per-request host function callbacks while preserving cached WASM modules/instances
/// if within the thread pool cap (`MAX_WASM_INSTANCES_PER_THREAD`).
pub fn clear_thread_wasm_state() {
    WASM_STATE.with(|st| {
        let mut s = st.borrow_mut();
        if s.instances.len() > MAX_WASM_INSTANCES_PER_THREAD {
            s.instances.clear();
            s.import_funcs.clear();
        }
        if s.modules.len() > MAX_WASM_INSTANCES_PER_THREAD {
            s.modules.clear();
        }
        if s.memories.len() > MAX_WASM_INSTANCES_PER_THREAD {
            s.memories.clear();
        }
    });
    ACTIVE_BOA_CTX_PTR.with(|p| p.set(std::ptr::null_mut()));
    ACTIVE_MEM_AB_PTR.with(|p| p.set(std::ptr::null()));
}

#[inline]
fn compute_wasm_key(bytes: &[u8]) -> (u64, usize) {
    let mut hasher = rustc_hash::FxHasher::default();
    bytes.hash(&mut hasher);
    (hasher.finish(), bytes.len())
}

#[cold]
#[inline(never)]
fn record_wasm_memory_violation(requested_bytes: usize, limit_bytes: usize) {
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
fn record_wasm_fuel_violation() {
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

#[inline]
fn check_wasm_memory_pages_budget(total_pages: u32) -> Result<usize, String> {
    if total_pages > MAX_WASM_MEMORY_PAGES {
        let req_bytes = (total_pages as usize).saturating_mul(WASM_PAGE_BYTES);
        let max_bytes = (MAX_WASM_MEMORY_PAGES as usize) * WASM_PAGE_BYTES;
        record_wasm_memory_violation(req_bytes, max_bytes);
        return Err(format!(
            "RangeError: WebAssembly.Memory pages ({}) exceed sandbox maximum ({} pages / {} bytes)",
            total_pages, MAX_WASM_MEMORY_PAGES, max_bytes
        ));
    }
    let req_bytes = (total_pages as usize).saturating_mul(WASM_PAGE_BYTES);
    let (used, limit) = BOA_KV_MEM_BUDGET.with(|m| m.get());
    if req_bytes > limit || used.saturating_add(total_pages as usize * 1024) > limit {
        record_wasm_memory_violation(req_bytes, limit);
        return Err(format!(
            "RangeError: WebAssembly linear memory ({} bytes) exceeds isolate memory cage ({} bytes)",
            req_bytes, limit
        ));
    }
    Ok(req_bytes)
}

fn compile_or_get_cached_module(bytes: &[u8]) -> Result<Module, String> {
    if bytes.len() > MAX_WASM_BINARY_BYTES {
        let (_, limit) = BOA_KV_MEM_BUDGET.with(|m| m.get());
        record_wasm_memory_violation(bytes.len(), limit.min(MAX_WASM_BINARY_BYTES));
        return Err(format!(
            "CompileError: WebAssembly binary size ({} bytes) exceeds maximum ({} bytes)",
            bytes.len(),
            MAX_WASM_BINARY_BYTES
        ));
    }
    if bytes.len() < 8 || &bytes[0..4] != b"\0asm" || &bytes[4..8] != b"\x01\x00\x00\x00" {
        return Err(
            "CompileError: WebAssembly.Module(): invalid Wasm magic header or version".to_string(),
        );
    }
    let key = compute_wasm_key(bytes);
    if let Ok(cache) = global_wasm_module_cache().lock() {
        if let Some(m) = cache.get(&key) {
            WASM_MODULE_CACHE_HITS.fetch_add(1, Ordering::Relaxed);
            return Ok(m.clone());
        }
    }
    let engine = global_wasm_engine();
    let module = Module::new(engine, bytes)
        .map_err(|e| format!("CompileError: WebAssembly compilation failed: {}", e))?;
    if let Ok(mut cache) = global_wasm_module_cache().lock() {
        if cache.len() >= 128 {
            cache.clear();
        }
        cache.insert(key, module.clone());
    }
    Ok(module)
}

/// Validates a `.wasm` binary buffer (`WebAssembly.validate(bytes)`).
pub fn wasm_validate_bytes(bytes: &[u8]) -> bool {
    if bytes.len() > MAX_WASM_BINARY_BYTES
        || bytes.len() < 8
        || &bytes[0..4] != b"\0asm"
        || &bytes[4..8] != b"\x01\x00\x00\x00"
    {
        return false;
    }
    let key = compute_wasm_key(bytes);
    if let Ok(cache) = global_wasm_module_cache().lock() {
        if cache.contains_key(&key) {
            return true;
        }
    }
    Module::validate(global_wasm_engine(), bytes).is_ok()
}

/// Compiles a `.wasm` binary and stores it in thread-local `WASM_STATE`, returning its `mod_id`.
pub fn wasm_compile_module(bytes: &[u8]) -> Result<u32, String> {
    let module = compile_or_get_cached_module(bytes)?;
    // Charge 64 fuel units for compilation
    let (consumed, limit) = BOA_FUEL_BUDGET.with(|f| f.get());
    if consumed.saturating_add(10) > limit {
        record_wasm_fuel_violation();
        return Err(
            "RuntimeError: CPU instruction fuel exhausted during WebAssembly.compile".to_string(),
        );
    }
    BOA_FUEL_BUDGET.with(|f| f.set((consumed.saturating_add(10), limit)));

    let mod_id = WASM_STATE.with(|st| {
        let mut s = st.borrow_mut();
        let id = s.alloc_id();
        s.modules.insert(
            id,
            CompiledModuleEntry {
                module,
                raw_bytes: bytes.to_vec(),
            },
        );
        id
    });
    Ok(mod_id)
}

fn extern_type_kind_str(ty: &ExternType) -> &'static str {
    match ty {
        ExternType::Func(_) => "function",
        ExternType::Table(_) => "table",
        ExternType::Memory(_) => "memory",
        ExternType::Global(_) => "global",
    }
}

/// Returns JSON array of `{ name, kind }` for `WebAssembly.Module.exports(module)`.
pub fn wasm_module_exports_json(mod_id: u32) -> Result<String, String> {
    WASM_STATE.with(|st| {
        let s = st.borrow();
        let entry = s
            .modules
            .get(&mod_id)
            .ok_or_else(|| "TypeError: Invalid WebAssembly.Module".to_string())?;
        let mut arr = Vec::new();
        for exp in entry.module.exports() {
            arr.push(serde_json::json!({
                "name": exp.name(),
                "kind": extern_type_kind_str(exp.ty()),
            }));
        }
        Ok(serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string()))
    })
}

/// Returns JSON array of `{ module, name, kind }` for `WebAssembly.Module.imports(module)`.
pub fn wasm_module_imports_json(mod_id: u32) -> Result<String, String> {
    WASM_STATE.with(|st| {
        let s = st.borrow();
        let entry = s
            .modules
            .get(&mod_id)
            .ok_or_else(|| "TypeError: Invalid WebAssembly.Module".to_string())?;
        let mut arr = Vec::new();
        for imp in entry.module.imports() {
            arr.push(serde_json::json!({
                "module": imp.module(),
                "name": imp.name(),
                "kind": extern_type_kind_str(imp.ty()),
            }));
        }
        Ok(serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string()))
    })
}

/// Extracts custom sections matching `section_name` from the raw `.wasm` binary.
pub fn wasm_module_custom_sections(
    mod_id: u32,
    section_name: &str,
) -> Result<Vec<Vec<u8>>, String> {
    WASM_STATE.with(|st| {
        let s = st.borrow();
        let entry = s
            .modules
            .get(&mod_id)
            .ok_or_else(|| "TypeError: Invalid WebAssembly.Module".to_string())?;
        let bytes = &entry.raw_bytes;
        let mut out = Vec::new();
        if bytes.len() < 8 {
            return Ok(out);
        }
        let mut pos = 8usize;
        while pos < bytes.len() {
            let sec_id = bytes[pos];
            pos += 1;
            let Some((sec_len, read_bytes)) = decode_leb128_u32(&bytes[pos..]) else {
                break;
            };
            pos += read_bytes;
            let sec_end = pos.saturating_add(sec_len as usize).min(bytes.len());
            if sec_id == 0 && pos < sec_end {
                let sec_slice = &bytes[pos..sec_end];
                if let Some((name_len, name_hdr_len)) = decode_leb128_u32(sec_slice) {
                    let name_start = name_hdr_len;
                    let name_end = name_start.saturating_add(name_len as usize);
                    if name_end <= sec_slice.len() {
                        if let Ok(name_str) = std::str::from_utf8(&sec_slice[name_start..name_end])
                        {
                            if name_str == section_name {
                                out.push(sec_slice[name_end..].to_vec());
                            }
                        }
                    }
                }
            }
            pos = sec_end;
        }
        Ok(out)
    })
}

fn decode_leb128_u32(slice: &[u8]) -> Option<(u32, usize)> {
    let mut result: u32 = 0;
    let mut shift = 0u32;
    for (idx, &byte) in slice.iter().take(5).enumerate() {
        result |= ((byte & 0x7F) as u32) << shift;
        if (byte & 0x80) == 0 {
            return Some((result, idx + 1));
        }
        shift += 7;
    }
    None
}

/// Allocates a standalone `WebAssembly.Memory` descriptor and checks the tenant's memory cage.
pub fn wasm_create_memory(initial_pages: u32, max_pages: Option<u32>) -> Result<u32, String> {
    if let Some(max_p) = max_pages {
        if initial_pages > max_p {
            return Err(format!(
                "RangeError: WebAssembly.Memory(): initial ({}) cannot exceed maximum ({})",
                initial_pages, max_p
            ));
        }
    }
    check_wasm_memory_pages_budget(initial_pages)?;
    let (used, limit) = BOA_KV_MEM_BUDGET.with(|m| m.get());
    BOA_KV_MEM_BUDGET.with(|m| m.set((used.saturating_add(initial_pages as usize * 1024), limit)));

    let mem_id = WASM_STATE.with(|st| {
        let mut s = st.borrow_mut();
        let id = s.alloc_id();
        s.memories.insert(
            id,
            StandaloneMemoryEntry {
                pages: initial_pages,
                max_pages,
            },
        );
        id
    });
    Ok(mem_id)
}

/// Grows a standalone `WebAssembly.Memory` or instance-bound memory by `delta_pages`.
/// Returns `Ok(previous_pages)` or `Err(...)` (or `-1` if soft max_pages reached).
pub fn wasm_grow_memory(mem_id: u32, inst_id: u32, delta_pages: u32) -> Result<i32, String> {
    if inst_id > 0 {
        return WASM_STATE.with(|st| {
            let mut s = st.borrow_mut();
            let Some(inst) = s.instances.get_mut(&inst_id) else {
                return Err("TypeError: Invalid WebAssembly.Instance memory".to_string());
            };
            let Some(mem) = inst.memory else {
                return Err("TypeError: WebAssembly.Instance has no linear memory".to_string());
            };
            let cur_pages = mem.size(&inst.store) as u32;
            let new_pages = cur_pages.saturating_add(delta_pages);
            if let Some(max_p) = inst.max_memory_pages {
                if new_pages > max_p {
                    return Err(format!(
                        "RangeError: WebAssembly.Memory.grow(): new size ({} pages) exceeds declared maximum ({} pages)",
                        new_pages, max_p
                    ));
                }
            }
            check_wasm_memory_pages_budget(new_pages)?;
            match mem.grow(&mut inst.store, delta_pages.into()) {
                Ok(prev) => {
                    let new_byte_len = (new_pages as usize) * WASM_PAGE_BYTES;
                    inst.shadow_mem.resize(new_byte_len, 0);
                    let (used, limit) = BOA_KV_MEM_BUDGET.with(|m| m.get());
                    BOA_KV_MEM_BUDGET
                        .with(|m| m.set((used.saturating_add(delta_pages as usize * 1024), limit)));
                    Ok(prev as i32)
                }
                Err(e) => Err(format!("RangeError: WebAssembly.Memory.grow() failed: {}", e)),
            }
        });
    }

    WASM_STATE.with(|st| {
        let mut s = st.borrow_mut();
        let Some(entry) = s.memories.get_mut(&mem_id) else {
            return Err("TypeError: Invalid WebAssembly.Memory".to_string());
        };
        let prev = entry.pages;
        let new_pages = prev.saturating_add(delta_pages);
        if let Some(max_p) = entry.max_pages {
            if new_pages > max_p {
                return Err(format!(
                    "RangeError: WebAssembly.Memory.grow(): new size ({} pages) exceeds declared maximum ({} pages)",
                    new_pages, max_p
                ));
            }
        }
        check_wasm_memory_pages_budget(new_pages)?;
        entry.pages = new_pages;
        let (used, limit) = BOA_KV_MEM_BUDGET.with(|m| m.get());
        BOA_KV_MEM_BUDGET
            .with(|m| m.set((used.saturating_add(delta_pages as usize * 1024), limit)));
        Ok(prev as i32)
    })
}

/// Synchronizes modified bytes from a JS `ArrayBuffer` (`ab_slice`) into WASM linear memory
/// (`wasm_slice`) using 8-byte word comparisons against `shadow_mem`.
#[inline]
fn sync_js_array_buffer_to_wasm(ab_slice: &[u8], wasm_slice: &mut [u8], shadow_mem: &mut [u8]) {
    let len = ab_slice.len().min(wasm_slice.len()).min(shadow_mem.len());
    let word_count = len / 8;
    for i in 0..word_count {
        let off = i * 8;
        let ab_w = u64::from_ne_bytes(ab_slice[off..off + 8].try_into().unwrap());
        let sh_w = u64::from_ne_bytes(shadow_mem[off..off + 8].try_into().unwrap());
        if ab_w != sh_w {
            for b in off..off + 8 {
                if ab_slice[b] != shadow_mem[b] {
                    wasm_slice[b] = ab_slice[b];
                    shadow_mem[b] = ab_slice[b];
                }
            }
        }
    }
    for b in (word_count * 8)..len {
        if ab_slice[b] != shadow_mem[b] {
            wasm_slice[b] = ab_slice[b];
            shadow_mem[b] = ab_slice[b];
        }
    }
}

/// Synchronizes modified bytes from WASM linear memory (`wasm_slice`) back into the JS `ArrayBuffer`
/// (`ab_slice`) and updates `shadow_mem`.
#[inline]
fn sync_wasm_to_js_array_buffer(wasm_slice: &[u8], ab_slice: &mut [u8], shadow_mem: &mut [u8]) {
    let len = wasm_slice.len().min(ab_slice.len()).min(shadow_mem.len());
    let word_count = len / 8;
    for i in 0..word_count {
        let off = i * 8;
        let wm_w = u64::from_ne_bytes(wasm_slice[off..off + 8].try_into().unwrap());
        let sh_w = u64::from_ne_bytes(shadow_mem[off..off + 8].try_into().unwrap());
        if wm_w != sh_w {
            ab_slice[off..off + 8].copy_from_slice(&wasm_slice[off..off + 8]);
            shadow_mem[off..off + 8].copy_from_slice(&wasm_slice[off..off + 8]);
        }
    }
    for b in (word_count * 8)..len {
        if wasm_slice[b] != shadow_mem[b] {
            ab_slice[b] = wasm_slice[b];
            shadow_mem[b] = wasm_slice[b];
        }
    }
}

fn default_val_for_type(vt: ValType) -> Val {
    Val::default_for_ty(vt)
}

fn js_val_to_wasm_val(val: &JsValue, vt: ValType, context: &mut Context) -> JsResult<Val> {
    match vt {
        ValType::I32 => {
            let n = val.to_i32(context)?;
            Ok(Val::I32(n))
        }
        ValType::I64 => {
            if let Some(bi) = val.as_bigint() {
                let s = bi.to_string();
                let parsed = s.parse::<i128>().unwrap_or(0) as i64;
                Ok(Val::I64(parsed))
            } else {
                let n = val.to_number(context)? as i64;
                Ok(Val::I64(n))
            }
        }
        ValType::F32 => {
            let n = val.to_number(context)? as f32;
            Ok(Val::F32(n.into()))
        }
        ValType::F64 => {
            let n = val.to_number(context)?;
            Ok(Val::F64(n.into()))
        }
        _ => Ok(default_val_for_type(vt)),
    }
}

fn wasm_val_to_js_val(val: &Val) -> JsValue {
    match val {
        Val::I32(v) => JsValue::from(*v),
        Val::I64(v) => JsValue::from(boa_engine::JsBigInt::from(*v)),
        Val::F32(v) => JsValue::from(f32::from(*v) as f64),
        Val::F64(v) => JsValue::from(f64::from(*v)),
        _ => JsValue::undefined(),
    }
}

/// Instantiates a compiled `mod_id` with an optional JS `importObject`, returning a JS descriptor
/// object `{ instId, exports: [...], memPages, initialMemBytes }` and copying initial linear memory
/// into `mem_ab` if provided.
pub fn wasm_instantiate_module(
    mod_id: u32,
    import_obj_opt: Option<&JsObject>,
    context: &mut Context,
) -> JsResult<JsValue> {
    let module = WASM_STATE.with(|st| st.borrow().modules.get(&mod_id).map(|e| e.module.clone()));
    let Some(module) = module else {
        return Err(JsNativeError::typ()
            .with_message("WebAssembly.Instance(): argument 0 must be a WebAssembly.Module")
            .into());
    };

    let (used_mem, mem_limit) = BOA_KV_MEM_BUDGET.with(|m| m.get());
    let max_linear_bytes = mem_limit
        .saturating_sub(used_mem)
        .min((MAX_WASM_MEMORY_PAGES as usize) * WASM_PAGE_BYTES);

    let limits = StoreLimitsBuilder::new()
        .memory_size(max_linear_bytes)
        .instances(16)
        .tables(16)
        .table_elements(65536)
        .build();

    let engine = global_wasm_engine();
    let mut store = Store::new(
        engine,
        WasmStoreData {
            limits,
            last_host_error: None,
        },
    );
    store.limiter(|data| &mut data.limits);

    let (consumed_fuel, fuel_limit) = BOA_FUEL_BUDGET.with(|f| f.get());
    let avail_fuel = fuel_limit.saturating_sub(consumed_fuel);
    if avail_fuel == 0 {
        record_wasm_fuel_violation();
        return Err(JsNativeError::error()
            .with_message(
                "RuntimeError: CPU instruction fuel exhausted before WebAssembly.instantiate",
            )
            .into());
    }
    let _ = store.set_fuel(avail_fuel);

    let mut linker = <Linker<WasmStoreData>>::new(engine);
    let mut imported_memory: Option<(Memory, Option<u32>, Option<JsObject>)> = None;

    // Resolve and bind all declared module imports from `importObject`
    for imp in module.imports() {
        let mod_name = imp.module();
        let field_name = imp.name();
        let Some(imp_root) = import_obj_opt else {
            return Err(JsNativeError::typ()
                .with_message(format!(
                    "TypeError: WebAssembly.Instance(): imports argument must be present for module requiring '{}.{}'",
                    mod_name, field_name
                ))
                .into());
        };
        let ns_val = imp_root.get(js_string!(mod_name), context)?;
        let Some(ns_obj) = ns_val.as_object() else {
            return Err(JsNativeError::error()
                .with_message(format!(
                    "LinkError: WebAssembly.Instance(): import module '{}' is not an object",
                    mod_name
                ))
                .into());
        };
        let field_val = ns_obj.get(js_string!(field_name), context)?;
        if field_val.is_undefined() {
            return Err(JsNativeError::error()
                .with_message(format!(
                    "LinkError: WebAssembly.Instance(): import '{}.{}' is undefined",
                    mod_name, field_name
                ))
                .into());
        };

        match imp.ty() {
            ExternType::Func(func_ty) => {
                let Some(fn_obj) = field_val.as_object().filter(|o| o.is_callable()) else {
                    return Err(JsNativeError::error()
                        .with_message(format!(
                            "LinkError: WebAssembly.Instance(): import '{}.{}' is not a Function",
                            mod_name, field_name
                        ))
                        .into());
                };
                let fn_id = WASM_STATE.with(|st| {
                    let mut s = st.borrow_mut();
                    let id = s.alloc_id();
                    s.import_funcs.insert(id, fn_obj.clone());
                    id
                });
                let f_ty = func_ty.clone();
                let res_types: Vec<ValType> = f_ty.results().to_vec();
                let host_func = Func::new(
                    &mut store,
                    f_ty,
                    move |mut caller: Caller<'_, WasmStoreData>,
                          params: &[Val],
                          results: &mut [Val]|
                          -> Result<(), WasmiError> {
                        let ctx_ptr = ACTIVE_BOA_CTX_PTR.with(|p| p.get());
                        if ctx_ptr.is_null() {
                            return Err(WasmiError::host(WasmHostError(
                                "Host context unavailable".to_string(),
                            )));
                        }
                        // Sync WASM memory -> JS ArrayBuffer before calling JS import (if a memory export/import exists)
                        let ab_ptr = ACTIVE_MEM_AB_PTR.with(|p| p.get());
                        if !ab_ptr.is_null() {
                            if let Some(Extern::Memory(mem)) = caller.get_export("memory") {
                                let ab_obj = unsafe { &*ab_ptr };
                                if let Ok(ab) = JsArrayBuffer::from_object(ab_obj.clone()) {
                                    if let Some(mut ab_data) = ab.data_mut() {
                                        let wm_data = mem.data(&caller);
                                        let len = ab_data.len().min(wm_data.len());
                                        ab_data[..len].copy_from_slice(&wm_data[..len]);
                                    }
                                }
                            }
                        }

                        let js_fn =
                            WASM_STATE.with(|st| st.borrow().import_funcs.get(&fn_id).cloned());
                        let Some(js_fn) = js_fn else {
                            return Err(WasmiError::host(WasmHostError(
                                "Imported JS function handle missing".to_string(),
                            )));
                        };
                        let ctx = unsafe { &mut *ctx_ptr };
                        let mut js_args = Vec::with_capacity(params.len());
                        for p in params {
                            js_args.push(wasm_val_to_js_val(p));
                        }
                        match js_fn.call(&JsValue::undefined(), &js_args, ctx) {
                            Ok(ret_val) => {
                                if !ab_ptr.is_null() {
                                    if let Some(Extern::Memory(mem)) = caller.get_export("memory") {
                                        let ab_obj = unsafe { &*ab_ptr };
                                        if let Ok(ab) = JsArrayBuffer::from_object(ab_obj.clone()) {
                                            if let Some(ab_data) = ab.data() {
                                                let wm_data = mem.data_mut(&mut caller);
                                                let len = ab_data.len().min(wm_data.len());
                                                wm_data[..len].copy_from_slice(&ab_data[..len]);
                                            }
                                        }
                                    }
                                }
                                if results.len() == 1 {
                                    match js_val_to_wasm_val(&ret_val, res_types[0], ctx) {
                                        Ok(wv) => results[0] = wv,
                                        Err(e) => {
                                            let msg = e.to_string();
                                            caller.data_mut().last_host_error = Some(msg.clone());
                                            return Err(WasmiError::host(WasmHostError(msg)));
                                        }
                                    }
                                }
                                Ok(())
                            }
                            Err(js_err) => {
                                let msg = js_err.to_string();
                                caller.data_mut().last_host_error = Some(msg.clone());
                                Err(WasmiError::host(WasmHostError(msg)))
                            }
                        }
                    },
                );
                let _ = linker.define(mod_name, field_name, host_func);
            }
            ExternType::Memory(mem_ty) => {
                let min_pages = mem_ty.minimum() as u32;
                let max_pages = mem_ty.maximum().map(|p| p as u32);
                let mut actual_pages = min_pages;
                let mut js_mem_buf_obj: Option<JsObject> = None;

                if let Some(mem_obj) = field_val.as_object() {
                    if let Ok(buf_val) = mem_obj.get(js_string!("buffer"), context) {
                        if let Some(b_obj) = buf_val.as_object() {
                            if let Ok(ab) = JsArrayBuffer::from_object(b_obj.clone()) {
                                if let Some(d) = ab.data() {
                                    let ab_pages = (d.len() / WASM_PAGE_BYTES) as u32;
                                    if ab_pages < min_pages {
                                        return Err(JsNativeError::error()
                                            .with_message(format!(
                                                "LinkError: Imported WebAssembly.Memory size ({} pages) is smaller than required minimum ({} pages)",
                                                ab_pages, min_pages
                                            ))
                                            .into());
                                    }
                                    actual_pages = ab_pages;
                                    js_mem_buf_obj = Some(b_obj.clone());
                                }
                            }
                        }
                    }
                }
                if let Err(msg) = check_wasm_memory_pages_budget(actual_pages) {
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                let eff_max = max_pages
                    .or(Some(MAX_WASM_MEMORY_PAGES))
                    .map(|m| m.max(actual_pages));
                let m_ty = MemoryType::new(actual_pages, eff_max);
                let mem = Memory::new(&mut store, m_ty).map_err(|e| {
                    JsNativeError::range().with_message(format!("RangeError: {}", e))
                })?;
                if let Some(ref b_obj) = js_mem_buf_obj {
                    if let Ok(ab) = JsArrayBuffer::from_object(b_obj.clone()) {
                        if let Some(d) = ab.data() {
                            let wm_slice = mem.data_mut(&mut store);
                            let len = d.len().min(wm_slice.len());
                            wm_slice[..len].copy_from_slice(&d[..len]);
                        }
                    }
                }
                imported_memory = Some((mem, max_pages, js_mem_buf_obj));
                let _ = linker.define(mod_name, field_name, mem);
            }
            ExternType::Global(glob_ty) => {
                let vt = glob_ty.content();
                let mut raw_val = field_val.clone();
                if let Some(g_obj) = field_val.as_object() {
                    if let Ok(v) = g_obj.get(js_string!("value"), context) {
                        if !v.is_undefined() {
                            raw_val = v;
                        }
                    }
                }
                let wv = js_val_to_wasm_val(&raw_val, vt, context)?;
                let g = Global::new(&mut store, wv, glob_ty.mutability());
                let _ = linker.define(mod_name, field_name, g);
            }
            ExternType::Table(tab_ty) => {
                let init_val = wasmi::Ref::default_for_ty(tab_ty.element());
                let tab = wasmi::Table::new(&mut store, *tab_ty, init_val).map_err(|e| {
                    JsNativeError::error().with_message(format!("LinkError: {}", e))
                })?;
                let _ = linker.define(mod_name, field_name, tab);
            }
        }
    }

    // Check declared exported memory limits before instantiation
    for exp in module.exports() {
        if let ExternType::Memory(mem_ty) = exp.ty() {
            let min_pages = mem_ty.minimum() as u32;
            if let Err(msg) = check_wasm_memory_pages_budget(min_pages) {
                return Err(JsNativeError::range().with_message(msg).into());
            }
        }
    }

    let instance = {
        let _guard = ActiveBoaCtxGuard::enter(
            context,
            imported_memory.as_ref().and_then(|(_, _, o)| o.as_ref()),
        );
        match linker.instantiate_and_start(&mut store, &module) {
            Ok(inst) => inst,
            Err(e) => {
                let left = store.get_fuel().unwrap_or(0);
                let used = avail_fuel.saturating_sub(left).max(1);
                BOA_FUEL_BUDGET.with(|f| f.set((consumed_fuel.saturating_add(used), fuel_limit)));
                let err_str = e.to_string();
                if err_str.contains("fuel") || e.as_trap_code() == Some(TrapCode::OutOfFuel) {
                    record_wasm_fuel_violation();
                }
                return Err(JsNativeError::error()
                    .with_message(format!("RuntimeError: WebAssembly start trap: {}", err_str))
                    .into());
            }
        }
    };

    let left_fuel = store.get_fuel().unwrap_or(0);
    let start_used = avail_fuel.saturating_sub(left_fuel).max(5);
    BOA_FUEL_BUDGET.with(|f| f.set((consumed_fuel.saturating_add(start_used), fuel_limit)));

    let mut active_mem: Option<Memory> = imported_memory.as_ref().map(|(m, _, _)| *m);
    let mut max_mem_pages: Option<u32> = imported_memory.as_ref().and_then(|(_, max_p, _)| *max_p);

    let mut exports_meta = Vec::new();
    for exp in instance.exports(&store) {
        let name = exp.name().to_string();
        match exp.into_extern() {
            Extern::Func(f) => {
                let f_ty = f.ty(&store);
                exports_meta.push(serde_json::json!({
                    "name": name,
                    "kind": "function",
                    "arity": f_ty.params().len(),
                }));
            }
            Extern::Memory(m) => {
                let m_ty = m.ty(&store);
                let cur_pages = m.size(&store) as u32;
                let max_p = m_ty.maximum().map(|p| p as u32);
                active_mem = Some(m);
                max_mem_pages = max_p;
                exports_meta.push(serde_json::json!({
                    "name": name,
                    "kind": "memory",
                    "pages": cur_pages,
                    "maxPages": max_p,
                }));
            }
            Extern::Global(g) => {
                let g_ty = g.ty(&store);
                let val = g.get(&store);
                let is_mut = matches!(g_ty.mutability(), Mutability::Var);
                let (v_num, v_str, is_i64) = match val {
                    Val::I32(v) => (v as f64, v.to_string(), false),
                    Val::I64(v) => (v as f64, v.to_string(), true),
                    Val::F32(v) => (f32::from(v) as f64, f32::from(v).to_string(), false),
                    Val::F64(v) => (f64::from(v), f64::from(v).to_string(), false),
                    _ => (0.0, "0".to_string(), false),
                };
                exports_meta.push(serde_json::json!({
                    "name": name,
                    "kind": "global",
                    "mutable": is_mut,
                    "isI64": is_i64,
                    "num": v_num,
                    "str": v_str,
                }));
            }
            Extern::Table(t) => {
                let sz = t.size(&store);
                exports_meta.push(serde_json::json!({
                    "name": name,
                    "kind": "table",
                    "length": sz,
                }));
            }
        }
    }

    let mut shadow_mem = Vec::new();
    let mut mem_pages = 0u32;
    if let Some(m) = active_mem {
        mem_pages = m.size(&store) as u32;
        let data = m.data(&store);
        shadow_mem.extend_from_slice(data);
        let (used, limit) = BOA_KV_MEM_BUDGET.with(|b| b.get());
        BOA_KV_MEM_BUDGET.with(|b| b.set((used.saturating_add(mem_pages as usize * 1024), limit)));
        // If memory was imported with an existing ArrayBuffer, sync any data segment initialization back
        if let Some((_, _, Some(ref b_obj))) = imported_memory {
            if let Ok(ab) = JsArrayBuffer::from_object(b_obj.clone()) {
                if let Some(mut ab_data) = ab.data_mut() {
                    let len = ab_data.len().min(data.len());
                    ab_data[..len].copy_from_slice(&data[..len]);
                }
            }
        }
    }

    let inst_id = WASM_STATE.with(|st| {
        let mut s = st.borrow_mut();
        let id = s.alloc_id();
        s.instances.insert(
            id,
            ActiveWasmInstance {
                store,
                instance,
                memory: active_mem,
                max_memory_pages: max_mem_pages,
                shadow_mem,
            },
        );
        id
    });

    let res_obj = JsObject::with_object_proto(context.intrinsics());
    let _ = res_obj.set(
        js_string!("instId"),
        JsValue::from(inst_id as f64),
        false,
        context,
    );
    let _ = res_obj.set(
        js_string!("exportsJson"),
        JsValue::from(js_string!(
            serde_json::to_string(&exports_meta).unwrap_or_else(|_| "[]".to_string())
        )),
        false,
        context,
    );
    let _ = res_obj.set(
        js_string!("memPages"),
        JsValue::from(mem_pages as f64),
        false,
        context,
    );
    Ok(res_obj.into())
}

/// Copies the initial linear memory of `inst_id` into the provided JS `ArrayBuffer` (`ab_obj`).
pub fn wasm_sync_initial_memory_to_ab(inst_id: u32, ab_obj: &JsObject) -> Result<(), String> {
    let ab = JsArrayBuffer::from_object(ab_obj.clone())
        .map_err(|e| format!("TypeError: Expected ArrayBuffer: {}", e))?;
    WASM_STATE.with(|st| {
        let s = st.borrow();
        let Some(inst) = s.instances.get(&inst_id) else {
            return Err("TypeError: Invalid WebAssembly.Instance".to_string());
        };
        if let Some(mem) = inst.memory {
            let wm_slice = mem.data(&inst.store);
            if let Some(mut ab_data) = ab.data_mut() {
                let len = ab_data.len().min(wm_slice.len());
                ab_data[..len].copy_from_slice(&wm_slice[..len]);
            }
        }
        Ok(())
    })
}

/// Invokes an exported WASM function `export_name` on `inst_id`, synchronizing linear memory
/// before and after execution and metering CPU instruction fuel against `BOA_FUEL_BUDGET`.
pub fn wasm_call_export(
    inst_id: u32,
    export_name: &str,
    js_args: &[JsValue],
    mem_ab_opt: Option<&JsObject>,
    context: &mut Context,
) -> JsResult<JsValue> {
    let (func, param_types, result_types, has_mem) = WASM_STATE.with(|st| -> JsResult<_> {
        let mut s = st.borrow_mut();
        let Some(inst) = s.instances.get_mut(&inst_id) else {
            return Err(JsNativeError::typ()
                .with_message("WebAssembly.Instance handle is invalid")
                .into());
        };
        let Some(f) = inst.instance.get_func(&inst.store, export_name) else {
            return Err(JsNativeError::typ()
                .with_message(format!(
                    "WebAssembly.Instance export '{}' is not a function",
                    export_name
                ))
                .into());
        };
        let f_ty: FuncType = f.ty(&inst.store);
        let p_types: Vec<ValType> = f_ty.params().to_vec();
        let r_types: Vec<ValType> = f_ty.results().to_vec();

        // Sync JS ArrayBuffer -> WASM linear memory prior to execution
        if let (Some(mem), Some(ab_obj)) = (inst.memory, mem_ab_opt) {
            if let Ok(ab) = JsArrayBuffer::from_object(ab_obj.clone()) {
                if let Some(ab_slice) = ab.data() {
                    let cur_len = mem.data(&inst.store).len();
                    if inst.shadow_mem.len() != cur_len {
                        inst.shadow_mem.resize(cur_len, 0);
                    }
                    let mut shadow_tmp = std::mem::take(&mut inst.shadow_mem);
                    let wm_slice = mem.data_mut(&mut inst.store);
                    sync_js_array_buffer_to_wasm(&ab_slice, wm_slice, &mut shadow_tmp);
                    inst.shadow_mem = shadow_tmp;
                }
            }
        }
        Ok((f, p_types, r_types, inst.memory.is_some()))
    })?;

    let mut wasm_params = Vec::with_capacity(param_types.len());
    for (idx, pt) in param_types.iter().enumerate() {
        let js_v = js_args.get(idx).cloned().unwrap_or_else(JsValue::undefined);
        wasm_params.push(js_val_to_wasm_val(&js_v, *pt, context)?);
    }
    let mut wasm_results: Vec<Val> = result_types
        .iter()
        .map(|rt| default_val_for_type(*rt))
        .collect();

    let (consumed_fuel, fuel_limit) = BOA_FUEL_BUDGET.with(|f| f.get());
    let avail_fuel = fuel_limit.saturating_sub(consumed_fuel);
    if avail_fuel == 0 {
        record_wasm_fuel_violation();
        return Err(JsNativeError::error()
            .with_message("RuntimeError: CPU instruction fuel exhausted before WebAssembly call")
            .into());
    }

    let call_outcome = {
        let _guard = ActiveBoaCtxGuard::enter(context, mem_ab_opt);
        let mut inst = WASM_STATE
            .with(|st| st.borrow_mut().instances.remove(&inst_id))
            .unwrap();
        inst.store.data_mut().last_host_error = None;
        let _ = inst.store.set_fuel(avail_fuel);
        let res = func.call(&mut inst.store, &wasm_params, &mut wasm_results);
        let left = inst.store.get_fuel().unwrap_or(0);
        let used = avail_fuel.saturating_sub(left).max(1);
        let host_err = inst.store.data_mut().last_host_error.take();

        if res.is_ok() && has_mem {
            if let (Some(mem), Some(ab_obj)) = (inst.memory, mem_ab_opt) {
                if let Ok(ab) = JsArrayBuffer::from_object(ab_obj.clone()) {
                    if let Some(mut ab_slice) = ab.data_mut() {
                        let cur_len = mem.data(&inst.store).len();
                        if inst.shadow_mem.len() != cur_len {
                            inst.shadow_mem.resize(cur_len, 0);
                        }
                        let mut shadow_tmp = std::mem::take(&mut inst.shadow_mem);
                        let wm_slice = mem.data(&inst.store);
                        sync_wasm_to_js_array_buffer(wm_slice, &mut ab_slice, &mut shadow_tmp);
                        inst.shadow_mem = shadow_tmp;
                    }
                }
            }
        }
        WASM_STATE.with(|st| {
            st.borrow_mut().instances.insert(inst_id, inst);
        });
        (res, used, host_err)
    };

    let (res, used_fuel, host_err) = call_outcome;
    BOA_FUEL_BUDGET.with(|f| f.set((consumed_fuel.saturating_add(used_fuel), fuel_limit)));

    if let Err(err) = res {
        if let Some(h_err) = host_err {
            return Err(JsNativeError::error().with_message(h_err).into());
        }
        let err_str = err.to_string();
        if err_str.contains("fuel") || err.as_trap_code() == Some(TrapCode::OutOfFuel) {
            record_wasm_fuel_violation();
        }
        return Err(JsNativeError::error()
            .with_message(format!("RuntimeError: WebAssembly trap: {}", err_str))
            .into());
    }

    if wasm_results.is_empty() {
        Ok(JsValue::undefined())
    } else if wasm_results.len() == 1 {
        Ok(wasm_val_to_js_val(&wasm_results[0]))
    } else {
        let arr = boa_engine::object::builtins::JsArray::from_iter(
            wasm_results.iter().map(wasm_val_to_js_val),
            context,
        );
        Ok(arr.into())
    }
}

/// Reads an exported `WebAssembly.Global` value from `inst_id`.
pub fn wasm_get_global(inst_id: u32, export_name: &str) -> JsResult<JsValue> {
    WASM_STATE.with(|st| {
        let s = st.borrow();
        let Some(inst) = s.instances.get(&inst_id) else {
            return Err(JsNativeError::typ()
                .with_message("Invalid WebAssembly.Instance")
                .into());
        };
        let Some(g) = inst.instance.get_global(&inst.store, export_name) else {
            return Err(JsNativeError::typ()
                .with_message(format!(
                    "Export '{}' is not a WebAssembly.Global",
                    export_name
                ))
                .into());
        };
        Ok(wasm_val_to_js_val(&g.get(&inst.store)))
    })
}

/// Mutates an exported `WebAssembly.Global` value on `inst_id`.
pub fn wasm_set_global(
    inst_id: u32,
    export_name: &str,
    new_val: &JsValue,
    context: &mut Context,
) -> JsResult<()> {
    let vt = WASM_STATE.with(|st| {
        let s = st.borrow();
        let inst = s
            .instances
            .get(&inst_id)
            .ok_or_else(|| JsNativeError::typ().with_message("Invalid WebAssembly.Instance"))?;
        let g = inst
            .instance
            .get_global(&inst.store, export_name)
            .ok_or_else(|| {
                JsNativeError::typ().with_message(format!(
                    "Export '{}' is not a WebAssembly.Global",
                    export_name
                ))
            })?;
        let ty: GlobalType = g.ty(&inst.store);
        if matches!(ty.mutability(), Mutability::Const) {
            return Err(JsNativeError::typ()
                .with_message("TypeError: Cannot set value of an immutable WebAssembly.Global")
                .into());
        }
        Ok::<ValType, boa_engine::JsError>(ty.content())
    })?;
    let wv = js_val_to_wasm_val(new_val, vt, context)?;
    WASM_STATE.with(|st| {
        let mut s = st.borrow_mut();
        let inst = s.instances.get_mut(&inst_id).unwrap();
        let g = inst.instance.get_global(&inst.store, export_name).unwrap();
        g.set(&mut inst.store, wv)
            .map_err(|e| JsNativeError::typ().with_message(e.to_string()).into())
    })
}
