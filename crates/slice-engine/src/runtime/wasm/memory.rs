//! Store-backed `WebAssembly.Memory`, `ResetMemoryBuffer` detachment, and unconditional
//! word-aligned delta synchronization between `wasmi::Memory` and `JsArrayBuffer`.

use super::WASM_PAGE_BYTES;
use super::store::{WasmEntities, WasmStoreData, construct_wasm_wrapper, with_store_and_entities};
use super::values::enforce_range_u32;
use boa_engine::object::builtins::{AlignedVec, JsArrayBuffer};
use boa_engine::{Context, JsNativeError, JsObject, JsResult, JsValue, js_string};
use wasmi::{AsContext, AsContextMut, Memory, MemoryType, StoreContextMut};

const WASM_MEMORY_DETACH_KEY: &str = "WebAssembly.Memory";
const MAX_SPEC_WASM32_PAGES: u32 = 65_536;

pub(crate) struct ManagedWasmMemory {
    pub(crate) memory: Memory,
    pub(crate) ab_obj: Option<JsObject>,
    pub(crate) shadow_mem: Vec<u8>,
    pub(crate) synced_byte_len: usize,
    pub(crate) js_wrapper: Option<JsObject>,
}

#[inline]
fn detach_memory_ab(ab_obj: &JsObject) {
    if let Ok(ab) = JsArrayBuffer::from_object(ab_obj.clone()) {
        let key = JsValue::from(js_string!(WASM_MEMORY_DETACH_KEY));
        let _ = ab.detach(&key);
    }
}

#[inline]
pub(crate) fn register_or_find_memory(
    memory: Memory,
    js_wrapper: Option<JsObject>,
    ctx: impl AsContext<Data = WasmStoreData>,
    entities: &mut WasmEntities,
) -> u32 {
    let key = format!("{:?}", memory);
    if let Some(&existing_id) = entities.memory_by_key.get(&key) {
        if let Some(w) = js_wrapper
            && let Some(entry) = entities.memories.get_mut(&existing_id)
            && entry.js_wrapper.is_none()
        {
            entry.js_wrapper = Some(w);
        }
        return existing_id;
    }
    let byte_len = memory.data_size(&ctx);
    let id = entities.alloc_id();
    entities.memory_by_key.insert(key, id);
    entities.memory_ids.push(id);
    entities.memories.insert(
        id,
        ManagedWasmMemory {
            memory,
            ab_obj: None,
            shadow_mem: Vec::new(),
            synced_byte_len: byte_len,
            js_wrapper,
        },
    );
    id
}

pub(crate) fn ensure_memory_js_wrapper(mem_id: u32, context: &mut Context) -> JsResult<JsObject> {
    let existing = with_store_and_entities(|_store_ctx, entities| {
        entities
            .memories
            .get(&mem_id)
            .and_then(|e| e.js_wrapper.clone())
    });
    if let Some(obj) = existing {
        return Ok(obj);
    }
    let wrapper = construct_wasm_wrapper("Memory", "_memId", mem_id, true, context)?;
    with_store_and_entities(|_store_ctx, entities| {
        if let Some(entry) = entities.memories.get_mut(&mem_id) {
            entry.js_wrapper = Some(wrapper.clone());
        }
    });
    Ok(wrapper)
}

/// Allocates a real `wasmi::Memory` in the isolate's shared `Store` for `new WebAssembly.Memory`.
pub fn wasm_create_memory(
    descriptor_val: &JsValue,
    js_this_opt: Option<&JsObject>,
    context: &mut Context,
) -> JsResult<u32> {
    let Some(desc_obj) = descriptor_val.as_object() else {
        return Err(JsNativeError::typ()
            .with_message("WebAssembly.Memory(): Descriptor must be an object")
            .into());
    };
    let init_raw = {
        let v = desc_obj.get(js_string!("initial"), context)?;
        if v.is_undefined() {
            desc_obj.get(js_string!("minimum"), context)?
        } else {
            v
        }
    };
    if init_raw.is_undefined() {
        return Err(JsNativeError::typ()
            .with_message("WebAssembly.Memory(): Property 'initial' is required")
            .into());
    }
    let initial_pages = enforce_range_u32(&init_raw, "Memory initial", context)?;
    let max_raw = desc_obj.get(js_string!("maximum"), context)?;
    let max_pages = if max_raw.is_undefined() {
        None
    } else {
        Some(enforce_range_u32(&max_raw, "Memory maximum", context)?)
    };

    if initial_pages > MAX_SPEC_WASM32_PAGES {
        return Err(JsNativeError::range()
            .with_message(format!(
                "WebAssembly.Memory(): initial size ({}) exceeds 65536 pages",
                initial_pages
            ))
            .into());
    }
    if let Some(max_p) = max_pages {
        if max_p > MAX_SPEC_WASM32_PAGES {
            return Err(JsNativeError::range()
                .with_message(format!(
                    "WebAssembly.Memory(): maximum size ({}) exceeds 65536 pages",
                    max_p
                ))
                .into());
        }
        if initial_pages > max_p {
            return Err(JsNativeError::range()
                .with_message(format!(
                    "WebAssembly.Memory(): initial ({}) cannot exceed maximum ({})",
                    initial_pages, max_p
                ))
                .into());
        }
    }

    with_store_and_entities(|mut store_ctx, entities| {
        let m_ty = MemoryType::new(initial_pages, max_pages);
        let mem = Memory::new(&mut store_ctx, m_ty).map_err(|e| {
            JsNativeError::range().with_message(format!("WebAssembly.Memory(): {}", e))
        })?;
        let id = register_or_find_memory(mem, js_this_opt.cloned(), store_ctx, entities);
        Ok(id)
    })
}

/// Grows a `WebAssembly.Memory` by `delta` pages (`Memory.prototype.grow`), flushing any pending
/// JS writes first and detaching the previous `ArrayBuffer` (`ResetMemoryBuffer`) even on `delta == 0`.
pub fn wasm_grow_memory(mem_id: u32, delta_val: &JsValue, context: &mut Context) -> JsResult<u32> {
    let delta_pages = enforce_range_u32(delta_val, "Memory.grow delta", context)?;
    with_store_and_entities(|mut store_ctx, entities| {
        let Some(entry) = entities.memories.get_mut(&mem_id) else {
            return Err(JsNativeError::typ()
                .with_message("WebAssembly.Memory.prototype.grow called on incompatible receiver")
                .into());
        };
        // Flush any pending writes from the current JS ArrayBuffer into wasmi::Memory before growing.
        sync_single_memory_js_to_wasm(entry, store_ctx.as_context_mut());

        match entry.memory.grow(&mut store_ctx, u64::from(delta_pages)) {
            Ok(prev_pages) => {
                // WebAssembly JS-API ResetMemoryBuffer: always detach old buffer on grow (including delta == 0).
                if let Some(old_ab) = entry.ab_obj.take() {
                    detach_memory_ab(&old_ab);
                }
                entry.shadow_mem.clear();
                entry.synced_byte_len =
                    ((prev_pages as usize) + (delta_pages as usize)) * WASM_PAGE_BYTES;
                Ok(prev_pages as u32)
            }
            Err(e) => Err(JsNativeError::range()
                .with_message(format!("WebAssembly.Memory.grow(): {}", e))
                .into()),
        }
    })
}

/// Returns (or lazily materializes) the fixed-length `JsArrayBuffer` backing `mem_id`,
/// tagging `[[ArrayBufferDetachKey]]` with `"WebAssembly.Memory"`.
pub fn wasm_memory_buffer(mem_id: u32, context: &mut Context) -> JsResult<JsValue> {
    let existing = with_store_and_entities(|_store_ctx, entities| {
        entities.memories.get(&mem_id).and_then(|entry| {
            let ab_obj = entry.ab_obj.as_ref()?;
            let ab = JsArrayBuffer::from_object(ab_obj.clone()).ok()?;
            if ab.data().is_some() {
                Some(ab_obj.clone())
            } else {
                None
            }
        })
    });
    if let Some(ab_obj) = existing {
        return Ok(ab_obj.into());
    }

    let aligned_opt = with_store_and_entities(|store_ctx, entities| -> Option<AlignedVec<u8>> {
        let entry = entities.memories.get_mut(&mem_id)?;
        let wm_slice = entry.memory.data(&store_ctx);
        entry.shadow_mem.clear();
        entry.shadow_mem.extend_from_slice(wm_slice);
        entry.synced_byte_len = wm_slice.len();
        Some(AlignedVec::from_slice(0, wm_slice))
    });

    let Some(aligned) = aligned_opt else {
        return Err(JsNativeError::typ()
            .with_message("WebAssembly.Memory.prototype.buffer accessed on invalid Memory")
            .into());
    };

    let detach_key = JsValue::from(js_string!(WASM_MEMORY_DETACH_KEY));
    let ab = JsArrayBuffer::from_byte_block_with_detach_key(aligned, detach_key, context)?;
    let ab_obj: JsObject = ab.into();
    with_store_and_entities(|_store_ctx, entities| {
        if let Some(entry) = entities.memories.get_mut(&mem_id) {
            entry.ab_obj = Some(ab_obj.clone());
        }
    });
    Ok(ab_obj.into())
}

#[inline]
fn sync_single_memory_js_to_wasm(
    entry: &mut ManagedWasmMemory,
    mut store_ctx: StoreContextMut<'_, WasmStoreData>,
) {
    let Some(ref ab_obj) = entry.ab_obj else {
        return;
    };
    let Ok(ab) = JsArrayBuffer::from_object(ab_obj.clone()) else {
        entry.ab_obj = None;
        entry.shadow_mem.clear();
        return;
    };
    let Some(ab_slice) = ab.data() else {
        entry.ab_obj = None;
        entry.shadow_mem.clear();
        return;
    };
    let cur_len = entry.memory.data_size(&store_ctx);
    if entry.shadow_mem.len() != cur_len {
        entry.shadow_mem.resize(cur_len, 0);
    }
    let wm_slice = entry.memory.data_mut(&mut store_ctx);
    sync_js_array_buffer_to_wasm(&ab_slice, wm_slice, &mut entry.shadow_mem);
}

#[inline]
fn sync_single_memory_wasm_to_js(
    entry: &mut ManagedWasmMemory,
    store_ctx: StoreContextMut<'_, WasmStoreData>,
) {
    let cur_len = entry.memory.data_size(&store_ctx);
    let Some(ab_obj) = entry.ab_obj.clone() else {
        entry.synced_byte_len = cur_len;
        return;
    };

    // If native Wasm `memory.grow` increased linear memory size, detach the stale ArrayBuffer
    // (`ResetMemoryBuffer`) so the next `memory.buffer` read materializes the grown buffer.
    if cur_len != entry.synced_byte_len {
        detach_memory_ab(&ab_obj);
        entry.ab_obj = None;
        entry.shadow_mem.clear();
        entry.synced_byte_len = cur_len;
        return;
    }

    let Ok(ab) = JsArrayBuffer::from_object(ab_obj) else {
        entry.ab_obj = None;
        entry.shadow_mem.clear();
        return;
    };
    let Some(mut ab_slice) = ab.data_mut() else {
        entry.ab_obj = None;
        entry.shadow_mem.clear();
        return;
    };
    if ab_slice.len() != cur_len {
        drop(ab_slice);
        detach_memory_ab(entry.ab_obj.as_ref().unwrap());
        entry.ab_obj = None;
        entry.shadow_mem.clear();
        entry.synced_byte_len = cur_len;
        return;
    }
    if entry.shadow_mem.len() != cur_len {
        entry.shadow_mem.resize(cur_len, 0);
    }
    let wm_slice = entry.memory.data(&store_ctx);
    sync_wasm_to_js_array_buffer(wm_slice, &mut ab_slice, &mut entry.shadow_mem);
}

/// Synchronizes all materialized JS `ArrayBuffer` mirrors into their backing `wasmi::Memory`
/// handles prior to entering Wasm execution. Pure numeric workloads (`ab_obj == None`) copy 0 bytes.
#[inline]
pub(crate) fn sync_all_memories_js_to_wasm(
    mut store_ctx: StoreContextMut<'_, WasmStoreData>,
    entities: &mut WasmEntities,
) {
    if entities.memory_ids.is_empty() {
        return;
    }
    for i in 0..entities.memory_ids.len() {
        let mem_id = entities.memory_ids[i];
        if let Some(entry) = entities.memories.get_mut(&mem_id)
            && entry.ab_obj.is_some()
        {
            sync_single_memory_js_to_wasm(entry, store_ctx.as_context_mut());
        }
    }
}

/// Unconditionally synchronizes all backing `wasmi::Memory` handles back into their materialized
/// JS `ArrayBuffer` mirrors (detaching stale buffers if native `memory.grow` occurred).
#[inline]
pub(crate) fn sync_all_memories_wasm_to_js(
    mut store_ctx: StoreContextMut<'_, WasmStoreData>,
    entities: &mut WasmEntities,
) {
    if entities.memory_ids.is_empty() {
        return;
    }
    for i in 0..entities.memory_ids.len() {
        let mem_id = entities.memory_ids[i];
        if let Some(entry) = entities.memories.get_mut(&mem_id) {
            sync_single_memory_wasm_to_js(entry, store_ctx.as_context_mut());
        }
    }
}

/// Synchronizes modified bytes from a JS `ArrayBuffer` (`ab_slice`) into Wasm linear memory
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

/// Synchronizes modified bytes from Wasm linear memory (`wasm_slice`) back into the JS `ArrayBuffer`
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
