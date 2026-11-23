//! Per-tenant `.cas` hot-swap cache, directory watcher registry, and tenant ID sanitizer.

use rustc_hash::{FxHashMap, FxHashSet};
use slice_cas::MappedCasArchive;
use slice_engine::SliceIsolate;
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_TENANT_CACHE_ENTRIES: usize = 1024;
const TENANT_STAT_TTL: std::time::Duration = std::time::Duration::from_millis(250);

static TENANT_FS_GENERATION: AtomicU64 = AtomicU64::new(1);

struct TenantDirWatchRegistry {
    watched_dirs: FxHashSet<PathBuf>,
    watcher: Option<notify::RecommendedWatcher>,
}

static TENANT_DIR_WATCHERS: std::sync::OnceLock<std::sync::Mutex<TenantDirWatchRegistry>> =
    std::sync::OnceLock::new();

#[inline]
pub(crate) fn notify_tenant_cas_updated() {
    TENANT_FS_GENERATION.fetch_add(1, Ordering::Release);
}

fn ensure_tenant_dir_watched(tenant_dir: &Path) {
    use notify::{RecursiveMode, Watcher};
    let reg = TENANT_DIR_WATCHERS.get_or_init(|| {
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res
                && !matches!(ev.kind, notify::EventKind::Access(_))
            {
                TENANT_FS_GENERATION.fetch_add(1, Ordering::Release);
            }
        })
        .ok();
        std::sync::Mutex::new(TenantDirWatchRegistry {
            watched_dirs: FxHashSet::default(),
            watcher,
        })
    });
    if let Ok(mut guard) = reg.lock()
        && !guard.watched_dirs.contains(tenant_dir)
        && guard.watched_dirs.len() < 64
    {
        if let Some(ref mut w) = guard.watcher {
            let _ = w.watch(tenant_dir, RecursiveMode::NonRecursive);
        }
        guard.watched_dirs.insert(tenant_dir.to_path_buf());
    }
}

pub(crate) type TenantBundlePair = (Arc<Vec<u8>>, Arc<MappedCasArchive>);

pub(crate) struct TenantHotSwapEntry {
    pub(crate) tenant_dir: PathBuf,
    pub(crate) resolved_path: PathBuf,
    pub(crate) mtime_ns: u128,
    pub(crate) file_len: u64,
    pub(crate) verified_gen: u64,
    pub(crate) last_stat_check: std::time::Instant,
    pub(crate) bytecode: Arc<Vec<u8>>,
    pub(crate) archive: Arc<MappedCasArchive>,
}

struct CompiledCasBundleEntry {
    mtime_ns: u128,
    file_len: u64,
    bytecode: Arc<Vec<u8>>,
    archive: Arc<MappedCasArchive>,
}

struct TenantDirSnapshot {
    dedicated_stems: FxHashSet<String>,
    default_bundle: Option<TenantBundlePair>,
    verified_gen: u64,
    last_stat_check: std::time::Instant,
}

#[derive(Default)]
struct TenantCasCacheState {
    by_tenant: FxHashMap<String, TenantHotSwapEntry>,
    by_canonical_path: FxHashMap<PathBuf, CompiledCasBundleEntry>,
    by_dir: FxHashMap<PathBuf, TenantDirSnapshot>,
    watched_dirs: FxHashSet<PathBuf>,
}

thread_local! {
    static TENANT_CAS_CACHE: std::cell::RefCell<TenantCasCacheState> =
        std::cell::RefCell::new(TenantCasCacheState::default());
}

#[cfg(test)]
pub(crate) fn tenant_cas_cache_len() -> (usize, usize) {
    TENANT_CAS_CACHE.with(|c| {
        let borrow = c.borrow();
        (borrow.by_tenant.len(), borrow.by_canonical_path.len())
    })
}

#[cfg(test)]
pub(crate) fn current_tenant_fs_generation() -> u64 {
    TENANT_FS_GENERATION.load(Ordering::Acquire)
}

#[cfg(test)]
pub(crate) fn clear_tenant_cas_cache() {
    TENANT_CAS_CACHE.with(|c| {
        let mut borrow = c.borrow_mut();
        borrow.by_tenant.clear();
        borrow.by_canonical_path.clear();
        borrow.by_dir.clear();
        borrow.watched_dirs.clear();
    });
}

#[inline]
fn is_clean_tenant_id(s: &str) -> bool {
    if s.is_empty() || s.len() > 128 {
        return false;
    }
    let bytes = s.as_bytes();
    if bytes[0] == b'.' || bytes[bytes.len() - 1] == b'.' {
        return false;
    }
    let mut prev_dot = false;
    for &b in bytes {
        if b == b'.' {
            if prev_dot {
                return false;
            }
            prev_dot = true;
        } else if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
            prev_dot = false;
        } else {
            return false;
        }
    }
    true
}

/// Sanitizes `tenant_id` without heap allocation when already valid (`[A-Za-z0-9_-]` plus single
/// interior `.`, max 128 bytes). Strips `:` (preventing `<tenant>:<key>` KV collisions), path
/// separators, and `..` traversal sequences.
#[inline]
pub(crate) fn sanitize_tenant_id(tenant_id: &str) -> Cow<'_, str> {
    if is_clean_tenant_id(tenant_id) {
        return Cow::Borrowed(tenant_id);
    }
    let mut out = String::with_capacity(tenant_id.len().min(128));
    let mut prev_dot = false;
    for ch in tenant_id.chars() {
        if out.len() >= 128 {
            break;
        }
        if ch == '.' {
            if !out.is_empty() && !prev_dot {
                out.push('.');
                prev_dot = true;
            }
        } else if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
            prev_dot = false;
        }
    }
    while out.ends_with('.') {
        out.pop();
    }
    Cow::Owned(out)
}

/// Resolves a per-tenant `.cas` archive from `tenant_dir` (`<tenant_id>.cas` or `default.cas`),
/// automatically hot-swapping and pre-warming the bundle when the file's modification timestamp
/// or byte length changes on disk, deduplicating `default.cas` across unknown tenant headers,
/// and bounding cache entries at `1,024`.
#[cfg(test)]
pub(crate) fn resolve_tenant_bytecode_hotswap(
    tenant_dir: &Path,
    tenant_id: &str,
) -> Option<Arc<Vec<u8>>> {
    resolve_tenant_cas_hotswap(tenant_dir, tenant_id).map(|(bc, _)| bc)
}

fn load_or_reuse_canonical_cas_bundle(
    state: &mut TenantCasCacheState,
    candidate: &Path,
    meta: &std::fs::Metadata,
) -> Option<TenantBundlePair> {
    let file_len = meta.len();
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let canonical_path =
        std::fs::canonicalize(candidate).unwrap_or_else(|_| candidate.to_path_buf());

    if let Some(bundle) = state.by_canonical_path.get(&canonical_path)
        && bundle.mtime_ns == mtime_ns
        && bundle.file_len == file_len
    {
        return Some((Arc::clone(&bundle.bytecode), Arc::clone(&bundle.archive)));
    }

    let archive = MappedCasArchive::open_detached(&canonical_path).ok()?;
    let entry_blob = archive.entrypoint_blob()?;
    let bc_vec = archive.get_bytecode_slice(entry_blob).to_vec();
    SliceIsolate::prewarm_boa_bundle(&bc_vec);
    let arc_bc = Arc::new(bc_vec);
    let arc_cas = Arc::new(archive);

    if state.by_canonical_path.len() >= MAX_TENANT_CACHE_ENTRIES
        && !state.by_canonical_path.contains_key(&canonical_path)
    {
        state.by_canonical_path.clear();
    }
    state.by_canonical_path.insert(
        canonical_path,
        CompiledCasBundleEntry {
            mtime_ns,
            file_len,
            bytecode: Arc::clone(&arc_bc),
            archive: Arc::clone(&arc_cas),
        },
    );
    Some((arc_bc, arc_cas))
}

pub(crate) fn resolve_tenant_cas_hotswap(
    tenant_dir: &Path,
    tenant_id: &str,
) -> Option<TenantBundlePair> {
    let safe_tenant = sanitize_tenant_id(tenant_id);
    if safe_tenant.is_empty() {
        return None;
    }

    TENANT_CAS_CACHE.with(|cache| {
        let cur_gen = TENANT_FS_GENERATION.load(Ordering::Acquire);
        let mut stale_last_known_good: Option<TenantBundlePair> = None;

        // 1. Fast path: dedicated `<tenant_id>.cas` OR `default.cas` / negative directory snapshot
        {
            let mut borrow = cache.borrow_mut();
            if let Some(entry) = borrow.by_tenant.get_mut(safe_tenant.as_ref())
                && entry.tenant_dir == tenant_dir
            {
                if entry.verified_gen == cur_gen
                    && entry.last_stat_check.elapsed() < TENANT_STAT_TTL
                {
                    return Some((Arc::clone(&entry.bytecode), Arc::clone(&entry.archive)));
                }
                match std::fs::metadata(&entry.resolved_path) {
                    Ok(meta) if meta.is_file() => {
                        let file_len = meta.len();
                        let mtime_ns = meta
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_nanos())
                            .unwrap_or(0);
                        if entry.mtime_ns == mtime_ns && entry.file_len == file_len {
                            entry.verified_gen = cur_gen;
                            entry.last_stat_check = std::time::Instant::now();
                            return Some((Arc::clone(&entry.bytecode), Arc::clone(&entry.archive)));
                        }
                        stale_last_known_good =
                            Some((Arc::clone(&entry.bytecode), Arc::clone(&entry.archive)));
                    }
                    _ => {
                        borrow.by_tenant.remove(safe_tenant.as_ref());
                    }
                }
            } else if let Some(dir_snap) = borrow.by_dir.get(tenant_dir)
                && dir_snap.verified_gen == cur_gen
                && dir_snap.last_stat_check.elapsed() < TENANT_STAT_TTL
                && !dir_snap.dedicated_stems.contains(safe_tenant.as_ref())
            {
                return dir_snap
                    .default_bundle
                    .as_ref()
                    .map(|(bc, cas)| (Arc::clone(bc), Arc::clone(cas)));
            }
        }

        // 2. Register OS directory watcher once per worker thread (`0` global Mutex locks on steady state)
        let needs_watch_reg = !cache.borrow().watched_dirs.contains(tenant_dir);
        if needs_watch_reg {
            ensure_tenant_dir_watched(tenant_dir);
            let mut borrow = cache.borrow_mut();
            if borrow.watched_dirs.len() < 64 {
                borrow.watched_dirs.insert(tenant_dir.to_path_buf());
            }
        }
        let cur_gen = TENANT_FS_GENERATION.load(Ordering::Acquire);

        // 3. Dedicated `<tenant_id>.cas` check (retaining last-known-good Arc on transient mid-copy open failure)
        let dedicated_candidate = tenant_dir.join(format!("{}.cas", safe_tenant));
        if let Ok(meta) = std::fs::metadata(&dedicated_candidate)
            && meta.is_file()
        {
            let file_len = meta.len();
            let mtime_ns = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let canonical_path = std::fs::canonicalize(&dedicated_candidate)
                .unwrap_or_else(|_| dedicated_candidate.clone());
            let mut borrow = cache.borrow_mut();
            let Some((arc_bc, arc_cas)) =
                load_or_reuse_canonical_cas_bundle(&mut borrow, &canonical_path, &meta)
            else {
                return stale_last_known_good;
            };
            if borrow.by_tenant.len() >= MAX_TENANT_CACHE_ENTRIES
                && !borrow.by_tenant.contains_key(safe_tenant.as_ref())
            {
                borrow.by_tenant.clear();
            }
            borrow.by_tenant.insert(
                safe_tenant.into_owned(),
                TenantHotSwapEntry {
                    tenant_dir: tenant_dir.to_path_buf(),
                    resolved_path: canonical_path,
                    mtime_ns,
                    file_len,
                    verified_gen: cur_gen,
                    last_stat_check: std::time::Instant::now(),
                    bytecode: Arc::clone(&arc_bc),
                    archive: Arc::clone(&arc_cas),
                },
            );
            return Some((arc_bc, arc_cas));
        }

        // 4. Refresh directory snapshot (`dedicated_stems` + `default.cas`) so both `default.cas`
        //    and negative lookups remain O(1) lock-free within `TENANT_STAT_TTL` with 0 `by_tenant` growth.
        let mut dedicated_stems = FxHashSet::default();
        if let Ok(rd) = std::fs::read_dir(tenant_dir) {
            for entry in rd.flatten() {
                if let Ok(ft) = entry.file_type()
                    && (ft.is_file() || ft.is_symlink())
                {
                    let fname = entry.file_name();
                    if let Some(s) = fname.to_str()
                        && let Some(stem) = s.strip_suffix(".cas")
                        && stem != "default"
                        && dedicated_stems.len() < MAX_TENANT_CACHE_ENTRIES
                    {
                        dedicated_stems.insert(stem.to_string());
                    }
                }
            }
        }

        let default_candidate = tenant_dir.join("default.cas");
        let default_meta_opt = std::fs::metadata(&default_candidate)
            .ok()
            .filter(std::fs::Metadata::is_file);
        let mut borrow = cache.borrow_mut();
        let prev_default = borrow
            .by_dir
            .get(tenant_dir)
            .and_then(|snap| snap.default_bundle.clone());
        let mut transient_default_err = false;
        let default_bundle = match default_meta_opt {
            Some(ref dmeta) => {
                if let Some(pair) =
                    load_or_reuse_canonical_cas_bundle(&mut borrow, &default_candidate, dmeta)
                {
                    Some(pair)
                } else {
                    transient_default_err = true;
                    prev_default
                }
            }
            None => None,
        };

        let result_pair = default_bundle
            .as_ref()
            .map(|(bc, cas)| (Arc::clone(bc), Arc::clone(cas)));

        if !transient_default_err {
            if borrow.by_dir.len() >= 64 && !borrow.by_dir.contains_key(tenant_dir) {
                borrow.by_dir.clear();
            }
            borrow.by_dir.insert(
                tenant_dir.to_path_buf(),
                TenantDirSnapshot {
                    dedicated_stems,
                    default_bundle,
                    verified_gen: cur_gen,
                    last_stat_check: std::time::Instant::now(),
                },
            );
        }

        result_pair
    })
}
