//! Content-Verified Thread-Local `SLICESNP` Module Cache & Zero-Allocation Static Export Iterator.

use super::{
    MAX_SNP_CACHE_ENTRIES, SLICESNP_FLAG_PURE_STATIC_EXPORTS, SLICESNP_V2_VERSION, SnpCursor,
    decode_slicesnp_module,
};
use crate::value_and_ast::{BytecodeModule, SNAPSHOT_MAGIC};
use rustc_hash::{FxHashMap, FxHasher};
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

#[derive(Clone)]
struct CachedSnpEntry {
    raw: Arc<[u8]>,
    module: Arc<BytecodeModule>,
}

thread_local! {
    static SNP_LAST_HIT: RefCell<Option<CachedSnpEntry>> = const { RefCell::new(None) };
    static SNP_MODULE_CACHE: RefCell<FxHashMap<u64, CachedSnpEntry>> =
        RefCell::new(FxHashMap::default());
}

#[inline]
fn empty_bytecode_module() -> Arc<BytecodeModule> {
    static EMPTY: OnceLock<Arc<BytecodeModule>> = OnceLock::new();
    Arc::clone(EMPTY.get_or_init(|| {
        Arc::new(BytecodeModule {
            ops: Vec::new(),
            static_exports: FxHashMap::default(),
        })
    }))
}

#[inline]
fn fx_hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = FxHasher::default();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// Decodes and caches a top-level `SLICESNP` entrypoint bytecode slice, updating `SNP_LAST_HIT`
/// and verifying exact payload bytes (`entry.raw.as_ref() == &bytecode[8..]`) so equal-length
/// `mimalloc` buffer reuse at the same address never returns a stale `BytecodeModule`.
#[inline]
pub(crate) fn decode_slicesnp_cached(bytecode: &[u8]) -> Arc<BytecodeModule> {
    if bytecode.len() < 9 || &bytecode[0..8] != SNAPSHOT_MAGIC {
        return empty_bytecode_module();
    }
    let payload = &bytecode[8..];
    if let Some(hit) = SNP_LAST_HIT.with(|slot| {
        let borrow = slot.borrow();
        if let Some(entry) = borrow.as_ref()
            && entry.raw.as_ref() == payload
        {
            Some(Arc::clone(&entry.module))
        } else {
            None
        }
    }) {
        return hit;
    }

    let entry = lookup_or_insert_snp_cache(bytecode, payload);
    let module = Arc::clone(&entry.module);
    SNP_LAST_HIT.with(|slot| {
        *slot.borrow_mut() = Some(entry);
    });
    module
}

/// Decodes and caches an imported child `SLICESNP` blob inside `BytecodeOp::ImportCas` without
/// displacing the active entrypoint in `SNP_LAST_HIT`.
#[inline]
pub(crate) fn decode_slicesnp_import_cached(bytecode: &[u8]) -> Option<Arc<BytecodeModule>> {
    if bytecode.len() < 9 || &bytecode[0..8] != SNAPSHOT_MAGIC {
        return None;
    }
    let payload = &bytecode[8..];
    let key = fx_hash_bytes(payload);
    SNP_MODULE_CACHE.with(|cache| {
        if let Some(existing) = cache.borrow().get(&key)
            && existing.raw.as_ref() == payload
        {
            return Some(Arc::clone(&existing.module));
        }
        let parsed = decode_slicesnp_module(bytecode).ok()?;
        let arc_mod = Arc::new(parsed);
        let raw_arc: Arc<[u8]> = Arc::from(payload);
        let mut map = cache.borrow_mut();
        if map.len() >= MAX_SNP_CACHE_ENTRIES && !map.contains_key(&key) {
            map.clear();
        }
        map.insert(
            key,
            CachedSnpEntry {
                raw: raw_arc,
                module: Arc::clone(&arc_mod),
            },
        );
        Some(arc_mod)
    })
}

fn lookup_or_insert_snp_cache(bytecode: &[u8], payload: &[u8]) -> CachedSnpEntry {
    let key = fx_hash_bytes(payload);
    SNP_MODULE_CACHE.with(|cache| {
        if let Some(existing) = cache.borrow().get(&key)
            && existing.raw.as_ref() == payload
        {
            return existing.clone();
        }
        let arc_mod = match decode_slicesnp_module(bytecode) {
            Ok(parsed) => Arc::new(parsed),
            Err(_) => empty_bytecode_module(),
        };
        let entry = CachedSnpEntry {
            raw: Arc::from(payload),
            module: arc_mod,
        };
        let mut map = cache.borrow_mut();
        if map.len() >= MAX_SNP_CACHE_ENTRIES && !map.contains_key(&key) {
            map.clear();
        }
        map.insert(key, entry.clone());
        entry
    })
}

/// Zero-allocation iterator over `(export_name: &str, value: f64)` pairs inside a
/// `SLICESNP\x02` pure static-export snapshot (`SLICESNP_FLAG_PURE_STATIC_EXPORTS`).
pub(crate) struct PureStaticExportsIter<'a> {
    cur: SnpCursor<'a>,
    remaining_exports: usize,
}

impl<'a> Iterator for PureStaticExportsIter<'a> {
    type Item = (&'a str, f64);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining_exports == 0 {
            return None;
        }
        self.remaining_exports -= 1;
        let k = self.cur.read_str()?;
        let v = self.cur.read_f64()?;
        Some((k, v))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining_exports, Some(self.remaining_exports))
    }
}

#[inline]
pub(crate) fn visit_pure_static_exports(bytecode: &[u8]) -> Option<PureStaticExportsIter<'_>> {
    if bytecode.len() < 14
        || &bytecode[0..8] != SNAPSHOT_MAGIC
        || bytecode[8] != SLICESNP_V2_VERSION
        || (bytecode[9] & SLICESNP_FLAG_PURE_STATIC_EXPORTS) == 0
    {
        return None;
    }
    let mut cur = SnpCursor::new(&bytecode[10..]);
    let count = cur.read_u32_len()?;
    Some(PureStaticExportsIter {
        cur,
        remaining_exports: count,
    })
}
