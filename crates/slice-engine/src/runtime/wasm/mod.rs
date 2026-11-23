//! Pure-Rust WebAssembly (`WebAssembly.*`) Execution Engine & 5-Ring Security Cage (`wasmi`).
//!
//! Provides a zero-JIT-warmup, register-machine WebAssembly runtime integrated directly with
//! Slice's `SliceIsolate` resource cage and W3C WebAssembly JS-API 2.0 semantics:
//! - **Speculative Process-Wide Module Cache (`GLOBAL_WASM_MODULE_CACHE`):** Validates and compiles
//!   each unique `.wasm` binary at most once per process (`O(1)` `Arc`-backed `wasmi::Module` clone).
//! - **Shared Per-Isolate `wasmi::Store` (`store`):** Allows `WebAssembly.Memory`,
//!   `WebAssembly.Global`, `WebAssembly.Table`, and exported `WebAssembly.Instance` functions to
//!   share live handles and canonical JS wrapper identity across multiple instances and re-entrant
//!   host import callbacks.
//! - **Ring 1 - Binary & Section Quota Guard:** Enforces `MAX_WASM_BINARY_BYTES` (`8 MiB`) and strict
//!   `\0asm\x01\0\0\0` header validation before translation.
//! - **Ring 2 - Unified Deterministic CPU Fuel Metering:** Shares the active `SliceIsolate`'s
//!   instruction fuel budget (`BOA_FUEL_BUDGET`), deterministically trapping infinite Wasm loops
//!   with `SandboxViolation::CpuFuelExhausted`.
//! - **Ring 3 - Unified Linear Memory Cage (`SliceWasmLimiter`):** Enforces per-tenant byte ceilings
//!   (`BOA_KV_MEM_BUDGET`) and `MAX_WASM_MEMORY_PAGES` (`256` pages = `16 MiB`) across both JS and
//!   native Wasm `memory.grow` instructions without double-charging imported memories.
//! - **Ring 4 - Zero Ambient WASI Authority & Host Import Firewall:** Pure compute sandbox with
//!   zero ambient filesystem/network authority, unmangled JS exception propagation, and fuel-metered
//!   host import callbacks.
//! - **Ring 5 - Coherent Linear Memory (`memory`):** Lazy `JsArrayBuffer` materialization (zero copy
//!   for pure numeric Wasm), `ResetMemoryBuffer` detachment on native/JS `memory.grow`, and
//!   unconditional word-aligned delta synchronization across normal returns, host callbacks, and traps.

mod call;
mod globals_and_tables;
mod instantiate;
mod memory;
mod store;
mod values;

use crate::boa_realm::BOA_FUEL_BUDGET;
use rustc_hash::FxHashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use wasmi::{CompilationMode, Config, Engine, ExternType, Module};

pub use self::globals_and_tables::{
    wasm_create_global, wasm_create_table, wasm_get_global, wasm_set_global, wasm_table_get,
    wasm_table_grow, wasm_table_length, wasm_table_set,
};
pub use self::instantiate::wasm_instantiate_module;
pub use self::memory::{wasm_create_memory, wasm_grow_memory, wasm_memory_buffer};
pub(crate) use self::store::{
    WasmErrorKind, WasmThreadState, restore_thread_wasm_state, take_thread_wasm_state_if_nonempty,
};
pub use self::store::{clear_thread_wasm_state, wasm_js_error};

use self::store::{record_wasm_fuel_violation, record_wasm_memory_violation};

/// Maximum `.wasm` binary size accepted by the sandbox (`8 MiB`).
pub const MAX_WASM_BINARY_BYTES: usize = 8 * 1024 * 1024;

/// Maximum Wasm linear memory pages per memory (`256` pages = `16 MiB`).
pub const MAX_WASM_MEMORY_PAGES: u32 = 256;

/// Standard WebAssembly linear memory page size (`64 KiB`).
pub const WASM_PAGE_BYTES: usize = 65_536;

/// Maximum concurrent Wasm instances retained per isolate store.
pub const MAX_WASM_INSTANCES_PER_THREAD: usize = 64;

const MAX_CACHED_WASM_MODULES: usize = 512;

static GLOBAL_WASM_ENGINE: OnceLock<Engine> = OnceLock::new();
static GLOBAL_WASM_MODULE_CACHE: OnceLock<RwLock<GlobalModuleCacheState>> = OnceLock::new();
static WASM_MODULE_CACHE_HITS: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
struct GlobalModuleCacheState {
    next_mod_id: u32,
    by_key: FxHashMap<(u64, usize), Arc<CompiledModuleEntry>>,
    by_id: FxHashMap<u32, Arc<CompiledModuleEntry>>,
}

#[inline]
pub(crate) fn global_wasm_engine() -> &'static Engine {
    GLOBAL_WASM_ENGINE.get_or_init(|| {
        let mut config = Config::default();
        config.consume_fuel(true);
        config.compilation_mode(CompilationMode::LazyTranslation);
        Engine::new(&config)
    })
}

#[inline]
fn global_wasm_module_cache() -> &'static RwLock<GlobalModuleCacheState> {
    GLOBAL_WASM_MODULE_CACHE.get_or_init(|| RwLock::new(GlobalModuleCacheState::default()))
}

/// Returns the number of process-wide `GLOBAL_WASM_MODULE_CACHE` hits.
pub fn wasm_module_cache_hits() -> u64 {
    WASM_MODULE_CACHE_HITS.load(Ordering::Relaxed)
}

/// Resets the `WASM_MODULE_CACHE_HITS` telemetry counter.
pub fn reset_wasm_telemetry() {
    WASM_MODULE_CACHE_HITS.store(0, Ordering::Relaxed);
}

pub(crate) struct CompiledModuleEntry {
    pub(crate) mod_id: u32,
    pub(crate) module: Module,
    pub(crate) raw_bytes: Box<[u8]>,
    /// Parsed Section 7 `(export_name, export_kind, item_index)` tuples for canonical identity.
    pub(crate) export_indices: Vec<(String, u8, u32)>,
}

#[inline]
pub(crate) fn get_compiled_module(mod_id: u32) -> Option<Arc<CompiledModuleEntry>> {
    let cache = global_wasm_module_cache().read().ok()?;
    cache.by_id.get(&mod_id).cloned()
}

#[inline]
fn compute_wasm_key(bytes: &[u8]) -> (u64, usize) {
    let mut hasher = rustc_hash::FxHasher::default();
    bytes.hash(&mut hasher);
    (hasher.finish(), bytes.len())
}

fn compile_or_get_cached_module_entry(
    bytes: &[u8],
) -> Result<Arc<CompiledModuleEntry>, (WasmErrorKind, String)> {
    if bytes.len() > MAX_WASM_BINARY_BYTES {
        let (_, limit) = crate::boa_realm::BOA_KV_MEM_BUDGET.with(|m| m.get());
        record_wasm_memory_violation(bytes.len(), limit.min(MAX_WASM_BINARY_BYTES));
        return Err((
            WasmErrorKind::Compile,
            format!(
                "WebAssembly binary size ({} bytes) exceeds maximum ({} bytes)",
                bytes.len(),
                MAX_WASM_BINARY_BYTES
            ),
        ));
    }
    if bytes.len() < 8 || &bytes[0..4] != b"\0asm" || &bytes[4..8] != b"\x01\x00\x00\0" {
        return Err((
            WasmErrorKind::Compile,
            "WebAssembly.Module(): invalid Wasm magic header or version".to_string(),
        ));
    }
    let key = compute_wasm_key(bytes);
    if let Ok(cache) = global_wasm_module_cache().read()
        && let Some(entry) = cache.by_key.get(&key)
        && entry.raw_bytes.as_ref() == bytes
    {
        WASM_MODULE_CACHE_HITS.fetch_add(1, Ordering::Relaxed);
        return Ok(Arc::clone(entry));
    }
    let engine = global_wasm_engine();
    let module = Module::new(engine, bytes).map_err(|e| {
        (
            WasmErrorKind::Compile,
            format!("WebAssembly compilation failed: {}", e),
        )
    })?;
    let export_indices = parse_module_export_indices(bytes);
    let raw_bytes: Box<[u8]> = bytes.into();

    let mut cache = global_wasm_module_cache()
        .write()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(existing) = cache.by_key.get(&key)
        && existing.raw_bytes.as_ref() == bytes
    {
        WASM_MODULE_CACHE_HITS.fetch_add(1, Ordering::Relaxed);
        return Ok(Arc::clone(existing));
    }
    if cache.by_id.len() >= MAX_CACHED_WASM_MODULES {
        cache.by_key.clear();
        cache.by_id.clear();
    }
    cache.next_mod_id = cache.next_mod_id.wrapping_add(1).max(1);
    let mod_id = cache.next_mod_id;
    let entry = Arc::new(CompiledModuleEntry {
        mod_id,
        module,
        raw_bytes,
        export_indices,
    });
    cache.by_key.insert(key, Arc::clone(&entry));
    cache.by_id.insert(mod_id, Arc::clone(&entry));
    Ok(entry)
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
    if let Ok(cache) = global_wasm_module_cache().read()
        && let Some(entry) = cache.by_key.get(&key)
        && entry.raw_bytes.as_ref() == bytes
    {
        return true;
    }
    Module::validate(global_wasm_engine(), bytes).is_ok()
}

/// Compiles (or fetches from `GLOBAL_WASM_MODULE_CACHE`) a stateless `WebAssembly.Module`,
/// returning its canonical `mod_id` so module handles remain valid across warm pooled HTTP turns.
pub fn wasm_compile_module(bytes: &[u8]) -> Result<u32, (WasmErrorKind, String)> {
    let entry = compile_or_get_cached_module_entry(bytes)?;
    let (consumed, limit) = BOA_FUEL_BUDGET.with(|f| f.get());
    if consumed.saturating_add(10) > limit {
        record_wasm_fuel_violation();
        return Err((
            WasmErrorKind::Runtime,
            "CPU instruction fuel exhausted during WebAssembly.compile".to_string(),
        ));
    }
    BOA_FUEL_BUDGET.with(|f| f.set((consumed.saturating_add(10), limit)));
    Ok(entry.mod_id)
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
    let entry =
        get_compiled_module(mod_id).ok_or_else(|| "Invalid WebAssembly.Module".to_string())?;
    let mut arr = Vec::new();
    for exp in entry.module.exports() {
        arr.push(serde_json::json!({
            "name": exp.name(),
            "kind": extern_type_kind_str(exp.ty()),
        }));
    }
    Ok(serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string()))
}

/// Returns JSON array of `{ module, name, kind }` for `WebAssembly.Module.imports(module)`.
pub fn wasm_module_imports_json(mod_id: u32) -> Result<String, String> {
    let entry =
        get_compiled_module(mod_id).ok_or_else(|| "Invalid WebAssembly.Module".to_string())?;
    let mut arr = Vec::new();
    for imp in entry.module.imports() {
        arr.push(serde_json::json!({
            "module": imp.module(),
            "name": imp.name(),
            "kind": extern_type_kind_str(imp.ty()),
        }));
    }
    Ok(serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string()))
}

fn for_each_wasm_section(bytes: &[u8], mut f: impl FnMut(u8, &[u8]) -> bool) {
    if bytes.len() < 8 {
        return;
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
        if pos < sec_end && !f(sec_id, &bytes[pos..sec_end]) {
            break;
        }
        pos = sec_end;
    }
}

/// Extracts custom sections matching `section_name` from the raw `.wasm` binary.
pub fn wasm_module_custom_sections(
    mod_id: u32,
    section_name: &str,
) -> Result<Vec<Vec<u8>>, String> {
    let entry =
        get_compiled_module(mod_id).ok_or_else(|| "Invalid WebAssembly.Module".to_string())?;
    let mut out = Vec::new();
    for_each_wasm_section(entry.raw_bytes.as_ref(), |sec_id, sec_slice| {
        if sec_id == 0
            && let Some((name_len, name_start)) = decode_leb128_u32(sec_slice)
        {
            let name_end = name_start.saturating_add(name_len as usize);
            if name_end <= sec_slice.len()
                && let Ok(name_str) = std::str::from_utf8(&sec_slice[name_start..name_end])
                && name_str == section_name
            {
                out.push(sec_slice[name_end..].to_vec());
            }
        }
        true
    });
    Ok(out)
}

/// Parses Section 7 (Export section) of a valid `.wasm` binary into `(name, kind, index)` entries.
fn parse_module_export_indices(bytes: &[u8]) -> Vec<(String, u8, u32)> {
    let mut exports = Vec::new();
    for_each_wasm_section(bytes, |sec_id, sec_slice| {
        if sec_id != 7 {
            return true;
        }
        let Some((count, n)) = decode_leb128_u32(sec_slice) else {
            return false;
        };
        let mut cur = &sec_slice[n..];
        exports.reserve(count as usize);
        for _ in 0..count {
            let Some((name_len, n1)) = decode_leb128_u32(cur) else {
                break;
            };
            cur = &cur[n1..];
            let nlen = name_len as usize;
            if cur.len() < nlen + 1 {
                break;
            }
            let name = String::from_utf8_lossy(&cur[..nlen]).into_owned();
            let kind = cur[nlen];
            cur = &cur[nlen + 1..];
            let Some((item_idx, n2)) = decode_leb128_u32(cur) else {
                break;
            };
            cur = &cur[n2..];
            exports.push((name, kind, item_idx));
        }
        false
    });
    exports
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
