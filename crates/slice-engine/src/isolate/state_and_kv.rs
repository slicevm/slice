//! Sharded Edge KV Slab (`Slice.kv`), Sub-5us State Checkpoints (`Slice.checkpoint`/`Slice.restore`),
//! Deterministic SplitMix64 PRNG Re-export, and Outbound Egress Secret DLP Guard.

use crate::bindings::fs::VfsSnapshot;
use crate::boa_realm::{BOA_ACTIVE_TENANT, BOA_VFS_STATE, CHECKPOINT_SLAB, NEXT_CHECKPOINT_ID};
pub(crate) use crate::intrinsics::splitmix64_next_f64;
use compact_str::CompactString;
use rustc_hash::FxHashMap;
use slice_sandbox::SandboxViolation;

/// Maximum retained checkpoints per worker thread slab before FIFO eviction of the oldest snapshot.
pub const MAX_CHECKPOINTS_PER_SLAB: usize = 64;

#[derive(Clone, Debug)]
pub(crate) struct StateCheckpoint {
    pub(crate) owner_tenant: CompactString,
    pub(crate) vfs_snap: VfsSnapshot,
    pub(crate) tenant_kv: [Option<KvTenantMap>; 16],
    pub(crate) state_bytes: usize,
}

#[inline]
pub(crate) fn current_vfs_state_bytes() -> usize {
    BOA_VFS_STATE.with(|v| v.borrow().total_accounted_bytes())
}

#[inline]
pub(crate) fn current_vfs_bytes_if_owned(vfs_ns: &str) -> usize {
    let bytes = BOA_VFS_STATE.with(|v| v.borrow().total_accounted_bytes());
    if bytes == 0 {
        return 0;
    }
    if crate::boa_realm::BOA_VFS_OWNER_TENANT.with(|o| o.borrow().as_str() == vfs_ns) {
        bytes
    } else {
        0
    }
}

pub(crate) fn checkpoint_tenant_state(tenant_id: &str) -> u64 {
    let trace_on = crate::boa_realm::is_trace_recording_enabled();
    let t0 = if trace_on {
        Some(std::time::Instant::now())
    } else {
        None
    };
    let vfs_snap = BOA_VFS_STATE.with(|v| v.borrow().snapshot_with_fds());
    let vfs_bytes = vfs_snap.total_accounted_bytes;
    let vfs_files_count = vfs_snap.paths.len();
    let mut tenant_kv: [Option<KvTenantMap>; 16] = std::array::from_fn(|_| None);
    let mut kv_keys_count = 0usize;
    let mut kv_bytes = 0usize;
    if let Some(shards) = EDGE_KV_SLAB.get() {
        let now = current_epoch_ms();
        for (idx, shard) in shards.iter().enumerate() {
            let guard = shard.read();
            if let Some(t_map) = guard.get(tenant_id)
                && !t_map.is_empty()
            {
                kv_keys_count += t_map.len();
                for (k, entry) in t_map {
                    if entry.expires_at_ms.is_none_or(|exp| now < exp) {
                        kv_bytes = kv_bytes.saturating_add(edge_kv_entry_bytes(k.as_str(), entry));
                    }
                }
                tenant_kv[idx] = Some(t_map.clone());
            }
        }
    }
    let state_bytes = vfs_bytes.saturating_add(kv_bytes);
    let id = NEXT_CHECKPOINT_ID.with(|n| {
        let cur = n.get();
        n.set(cur + 1);
        cur
    });
    CHECKPOINT_SLAB.with(|slab| {
        let mut map = slab.borrow_mut();
        while map.len() >= MAX_CHECKPOINTS_PER_SLAB {
            let evict_id = map
                .iter()
                .filter(|(_, cp)| cp.owner_tenant.as_str() == tenant_id)
                .map(|(&k, _)| k)
                .min()
                .or_else(|| map.keys().copied().min());
            if let Some(old_id) = evict_id {
                map.remove(&old_id);
            } else {
                break;
            }
        }
        map.insert(
            id,
            StateCheckpoint {
                owner_tenant: CompactString::new(tenant_id),
                vfs_snap,
                tenant_kv,
                state_bytes,
            },
        );
    });
    if let Some(start) = t0 {
        crate::boa_realm::record_trace_event(
            "state.checkpoint",
            "slice.timetravel",
            id * 10,
            start.elapsed().as_nanos() as u64,
            serde_json::json!({
                "tenant_id": tenant_id,
                "checkpoint_id": id,
                "vfs_files": vfs_files_count,
                "kv_keys": kv_keys_count
            }),
        );
    }
    id
}

pub(crate) fn restore_tenant_state_checked(
    tenant_id: &str,
    checkpoint_id: u64,
) -> Result<bool, String> {
    let trace_on = crate::boa_realm::is_trace_recording_enabled();
    let t0 = if trace_on {
        Some(std::time::Instant::now())
    } else {
        None
    };
    let snap_opt = CHECKPOINT_SLAB.with(|slab| {
        slab.borrow()
            .get(&checkpoint_id)
            .filter(|cp| cp.owner_tenant.as_str() == tenant_id)
            .cloned()
    });
    let Some(snap) = snap_opt else {
        return Ok(false);
    };
    let pre_restore_bytes =
        current_vfs_state_bytes().saturating_add(edge_kv_tenant_bytes(tenant_id));
    let delta = snap.state_bytes as isize - pre_restore_bytes as isize;
    crate::bindings::helpers::try_adjust_isolate_memory(delta)?;

    let vfs_files_count = snap.vfs_snap.paths.len();
    BOA_VFS_STATE.with(|v| v.borrow_mut().restore_from_snapshot(snap.vfs_snap));
    let shards = EDGE_KV_SLAB
        .get_or_init(|| std::array::from_fn(|_| parking_lot::RwLock::new(FxHashMap::default())));
    let mut restored_kv_bytes = 0usize;
    let now = current_epoch_ms();
    for (idx, shard_snap) in snap.tenant_kv.into_iter().enumerate() {
        if let Some(t_map) = shard_snap {
            if !t_map.is_empty() {
                EDGE_KV_HAS_WRITES.store(true, std::sync::atomic::Ordering::Relaxed);
                for (k, entry) in &t_map {
                    if entry.expires_at_ms.is_none_or(|exp| now < exp) {
                        restored_kv_bytes = restored_kv_bytes
                            .saturating_add(edge_kv_entry_bytes(k.as_str(), entry));
                    }
                }
            }
            shards[idx]
                .write()
                .insert(CompactString::new(tenant_id), t_map);
        } else {
            let mut guard = shards[idx].write();
            if !guard.is_empty() {
                guard.remove(tenant_id);
            }
        }
    }
    set_kv_owner_exact_bytes(tenant_id, restored_kv_bytes);
    if let Some(start) = t0 {
        crate::boa_realm::record_trace_event(
            "state.restore",
            "slice.timetravel",
            checkpoint_id * 10 + 5,
            start.elapsed().as_nanos() as u64,
            serde_json::json!({
                "tenant_id": tenant_id,
                "checkpoint_id": checkpoint_id,
                "vfs_files_restored": vfs_files_count
            }),
        );
    }
    Ok(true)
}

#[inline]
pub(crate) fn restore_tenant_state(tenant_id: &str, checkpoint_id: u64) -> bool {
    restore_tenant_state_checked(tenant_id, checkpoint_id).unwrap_or(false)
}

pub(crate) fn release_tenant_checkpoint(tenant_id: &str, checkpoint_id: u64) -> bool {
    CHECKPOINT_SLAB.with(|slab| {
        let mut map = slab.borrow_mut();
        if map
            .get(&checkpoint_id)
            .is_some_and(|cp| cp.owner_tenant.as_str() == tenant_id)
        {
            map.remove(&checkpoint_id);
            true
        } else {
            false
        }
    })
}

#[inline]
pub(crate) fn clear_tenant_checkpoints(tenant_id: &str) -> usize {
    CHECKPOINT_SLAB.with(|slab| {
        if slab.borrow().is_empty() {
            return 0;
        }
        let mut map = slab.borrow_mut();
        let before = map.len();
        map.retain(|_, cp| cp.owner_tenant.as_str() != tenant_id);
        before.saturating_sub(map.len())
    })
}

/// Returns the current number of active checkpoints retained in the thread-local `CHECKPOINT_SLAB`.
pub fn checkpoint_slab_len() -> usize {
    CHECKPOINT_SLAB.with(|slab| slab.borrow().len())
}

#[inline]
fn push_unique_dlp_pattern(out: &mut Vec<String>, pat: String) {
    if !pat.is_empty() && !out.contains(&pat) {
        out.push(pat);
    }
}

#[inline]
fn hex_encode_bytes(bytes: &[u8], uppercase: bool) -> String {
    let lut: &[u8; 16] = if uppercase {
        b"0123456789ABCDEF"
    } else {
        b"0123456789abcdef"
    };
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(lut[(b >> 4) as usize] as char);
        out.push(lut[(b & 0x0f) as usize] as char);
    }
    out
}

#[inline]
fn url_encode_uri_component(bytes: &[u8], uppercase: bool, form_plus: bool) -> String {
    let lut: &[u8; 16] = if uppercase {
        b"0123456789ABCDEF"
    } else {
        b"0123456789abcdef"
    };
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        if form_plus && b == b' ' {
            out.push('+');
            continue;
        }
        let unescaped = if form_plus {
            b.is_ascii_alphanumeric() || matches!(b, b'*' | b'-' | b'.' | b'_')
        } else {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
                )
        };
        if unescaped {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(lut[(b >> 4) as usize] as char);
            out.push(lut[(b & 0x0f) as usize] as char);
        }
    }
    out
}

#[inline]
fn full_percent_encode_bytes(bytes: &[u8], uppercase: bool) -> String {
    let lut: &[u8; 16] = if uppercase {
        b"0123456789ABCDEF"
    } else {
        b"0123456789abcdef"
    };
    let mut out = String::with_capacity(bytes.len() * 3);
    for &b in bytes {
        out.push('%');
        out.push(lut[(b >> 4) as usize] as char);
        out.push(lut[(b & 0x0f) as usize] as char);
    }
    out
}

/// Expands a registered audit `secret` into its deterministic exfiltration encodings:
/// - Raw `secret`
/// - Standard Base64 (`=`-padded and unpadded) and URL-safe Base64 (`-`, `_`) at offset 0
/// - All 3 Base64 / Base64URL triplet byte-alignment (`offset = 0, 1, 2`) interior quantum
///   slices (catching `btoa(prefix + secret + suffix)` for any surrounding prefix/suffix)
/// - Lowercase and uppercase hexadecimal (`hex_lower`, `hex_upper`) for secrets `>= 3` bytes
/// - `encodeURIComponent`, `URLSearchParams` (`+` form-encoded), and full `%XX`/`%xx` percent-encoding
pub(crate) fn expand_egress_secret_variants(secret: &str, out: &mut Vec<String>) {
    if secret.is_empty() {
        return;
    }
    let bytes = secret.as_bytes();
    push_unique_dlp_pattern(out, secret.to_string());

    // Offset-0 full Base64 (padded + unpadded) and Base64URL
    let b64_std = crate::intrinsics::base64_encode(bytes, false);
    let b64_url = crate::intrinsics::base64_encode(bytes, true);
    let b64_std_unpadded = b64_std.trim_end_matches('=').to_string();
    push_unique_dlp_pattern(out, b64_std);
    push_unique_dlp_pattern(out, b64_std_unpadded);
    push_unique_dlp_pattern(out, b64_url);

    // All 3 byte-alignment (`offset = 0, 1, 2`) interior Base64 & Base64URL slices.
    // For `offset` leading pad bytes (`P = [0u8; offset] + bytes`, `total = offset + len`):
    // - `start_char`: `0` (`offset = 0`), `2` (`offset = 1`), `3` (`offset = 2`)
    // - `end_char`: `(total / 3) * 4` (drops any trailing partial quantum mixed with suffix bytes)
    if bytes.len() >= 5 {
        let mut scratch = Vec::with_capacity(bytes.len() + 2);
        for (offset, start_char) in [(0usize, 0usize), (1usize, 2usize), (2usize, 3usize)] {
            scratch.clear();
            scratch.resize(offset, 0u8);
            scratch.extend_from_slice(bytes);
            let end_char = (scratch.len() / 3) * 4;
            if end_char >= start_char + 5 {
                let enc_std = crate::intrinsics::base64_encode(&scratch, false);
                if end_char <= enc_std.len() {
                    push_unique_dlp_pattern(out, enc_std[start_char..end_char].to_string());
                }
                let enc_url = crate::intrinsics::base64_encode(&scratch, true);
                if end_char <= enc_url.len() {
                    push_unique_dlp_pattern(out, enc_url[start_char..end_char].to_string());
                }
            }
        }
    }

    // Hexadecimal encodings (`hex_lower` and `hex_upper`)
    if bytes.len() >= 3 {
        push_unique_dlp_pattern(out, hex_encode_bytes(bytes, false));
        push_unique_dlp_pattern(out, hex_encode_bytes(bytes, true));
        push_unique_dlp_pattern(out, full_percent_encode_bytes(bytes, false));
        push_unique_dlp_pattern(out, full_percent_encode_bytes(bytes, true));
    }

    // `encodeURIComponent` and `application/x-www-form-urlencoded` percent-encodings
    let uri_upper = url_encode_uri_component(bytes, true, false);
    if uri_upper != secret {
        push_unique_dlp_pattern(out, uri_upper);
        push_unique_dlp_pattern(out, url_encode_uri_component(bytes, false, false));
    }
    let form_upper = url_encode_uri_component(bytes, true, true);
    if form_upper != secret {
        push_unique_dlp_pattern(out, form_upper);
        push_unique_dlp_pattern(out, url_encode_uri_component(bytes, false, true));
    }
}

#[inline]
fn hash_dlp_pattern_slice(secrets: &[String]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = rustc_hash::FxHasher::default();
    secrets.len().hash(&mut h);
    for s in secrets {
        s.hash(&mut h);
    }
    h.finish()
}

thread_local! {
    static EGRESS_DLP_AUTOMATON_CACHE: std::cell::RefCell<
        smallvec::SmallVec<[(u64, usize, std::sync::Arc<aho_corasick::AhoCorasick>); 8]>,
    > = const { std::cell::RefCell::new(smallvec::SmallVec::new_const()) };
}

#[inline]
fn get_or_build_dlp_automaton(
    secrets: &[String],
) -> Option<std::sync::Arc<aho_corasick::AhoCorasick>> {
    if secrets.is_empty() {
        return None;
    }
    let key_hash = hash_dlp_pattern_slice(secrets);
    let key_len = secrets.len();
    EGRESS_DLP_AUTOMATON_CACHE.with(|cache_cell| {
        {
            let cache = cache_cell.borrow();
            for (h, len, ac) in cache.iter().rev() {
                if *h == key_hash && *len == key_len {
                    return Some(std::sync::Arc::clone(ac));
                }
            }
        }
        let non_empty: Vec<&[u8]> = secrets
            .iter()
            .filter(|s| !s.is_empty())
            .map(String::as_bytes)
            .collect();
        if non_empty.is_empty() {
            return None;
        }
        let ac = aho_corasick::AhoCorasick::new(&non_empty).ok()?;
        let arc = std::sync::Arc::new(ac);
        let mut cache = cache_cell.borrow_mut();
        if cache.len() >= 8 {
            cache.remove(0);
        }
        cache.push((key_hash, key_len, std::sync::Arc::clone(&arc)));
        Some(arc)
    })
}

/// Scans `payload` in a single `O(payload)` pass using a cached `aho-corasick` automaton
/// built over all registered raw/Base64/Base64URL/hex/URL-encoded audit secret patterns.
#[inline]
pub fn verify_egress_secrets_slice(
    tenant_id: &str,
    secrets: &[String],
    payload: &str,
) -> Result<(), SandboxViolation> {
    if secrets.is_empty() || payload.is_empty() {
        return Ok(());
    }
    let payload_bytes = payload.as_bytes();
    if let Some(ac) = get_or_build_dlp_automaton(secrets) {
        if ac.is_match(payload_bytes) {
            return Err(SandboxViolation::EgressSecretLeakBlocked {
                tenant_id: tenant_id.to_string(),
            });
        }
        return Ok(());
    }
    for pat in secrets {
        if !pat.is_empty() && memchr::memmem::find(payload_bytes, pat.as_bytes()).is_some() {
            return Err(SandboxViolation::EgressSecretLeakBlocked {
                tenant_id: tenant_id.to_string(),
            });
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub(crate) enum KvValue {
    Str(CompactString),
    Int(i64),
}

impl KvValue {
    #[inline]
    pub(crate) fn to_std_string(&self) -> String {
        match self {
            Self::Str(s) => s.to_string(),
            Self::Int(n) => {
                let mut buf = itoa::Buffer::new();
                buf.format(*n).to_string()
            }
        }
    }

    #[inline]
    pub(crate) fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(n) => Some(*n),
            Self::Str(s) => s.parse::<i64>().ok(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct KvEntry {
    pub(crate) val: KvValue,
    pub(crate) expires_at_ms: Option<u64>,
}

pub(crate) type KvTenantMap = FxHashMap<CompactString, KvEntry>;
pub(crate) type KvShardMap = FxHashMap<CompactString, KvTenantMap>;

pub(crate) static EDGE_KV_SLAB: std::sync::OnceLock<[parking_lot::RwLock<KvShardMap>; 16]> =
    std::sync::OnceLock::new();
pub(crate) static EDGE_KV_HAS_WRITES: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static EDGE_KV_OWNER_BYTES: std::sync::OnceLock<
    parking_lot::RwLock<FxHashMap<CompactString, usize>>,
> = std::sync::OnceLock::new();
pub(crate) static EDGE_KV_NONEMPTY_OWNERS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
static EDGE_KV_HAS_SUBTENANT_OWNERS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[inline]
fn kv_owner_bytes_map() -> &'static parking_lot::RwLock<FxHashMap<CompactString, usize>> {
    EDGE_KV_OWNER_BYTES.get_or_init(|| parking_lot::RwLock::new(FxHashMap::default()))
}

#[inline]
fn adjust_kv_owner_bytes(tenant_id: &str, delta: isize) {
    if delta == 0 {
        return;
    }
    let mut map = kv_owner_bytes_map().write();
    if delta > 0 {
        let add = delta as usize;
        if let Some(cur) = map.get_mut(tenant_id) {
            *cur = cur.saturating_add(add);
        } else {
            if tenant_id.contains(':') {
                EDGE_KV_HAS_SUBTENANT_OWNERS.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            map.insert(CompactString::new(tenant_id), add);
            EDGE_KV_NONEMPTY_OWNERS.store(map.len(), std::sync::atomic::Ordering::Relaxed);
        }
    } else {
        let sub = delta.unsigned_abs();
        let mut removed_sub = false;
        if let Some(cur) = map.get_mut(tenant_id) {
            if *cur <= sub {
                removed_sub = tenant_id.contains(':');
                map.remove(tenant_id);
                EDGE_KV_NONEMPTY_OWNERS.store(map.len(), std::sync::atomic::Ordering::Relaxed);
            } else {
                *cur -= sub;
            }
        }
        if removed_sub && !map.keys().any(|k| k.contains(':')) {
            EDGE_KV_HAS_SUBTENANT_OWNERS.store(false, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

fn set_kv_owner_exact_bytes(tenant_id: &str, bytes: usize) {
    let mut map = kv_owner_bytes_map().write();
    if bytes == 0 {
        let removed = map.remove(tenant_id).is_some();
        if removed {
            EDGE_KV_NONEMPTY_OWNERS.store(map.len(), std::sync::atomic::Ordering::Relaxed);
            if tenant_id.contains(':') && !map.keys().any(|k| k.contains(':')) {
                EDGE_KV_HAS_SUBTENANT_OWNERS.store(false, std::sync::atomic::Ordering::Relaxed);
            }
        }
    } else {
        if tenant_id.contains(':') {
            EDGE_KV_HAS_SUBTENANT_OWNERS.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        map.insert(CompactString::new(tenant_id), bytes);
        EDGE_KV_NONEMPTY_OWNERS.store(map.len(), std::sync::atomic::Ordering::Relaxed);
    }
}

#[inline]
pub(crate) fn current_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1_700_000_000_000)
}

#[inline]
pub(crate) fn edge_kv_shard(
    tenant_id: &str,
    key: &str,
) -> &'static parking_lot::RwLock<KvShardMap> {
    let shards = EDGE_KV_SLAB
        .get_or_init(|| std::array::from_fn(|_| parking_lot::RwLock::new(FxHashMap::default())));
    use std::hash::{Hash, Hasher};
    let mut hasher = rustc_hash::FxHasher::default();
    tenant_id.hash(&mut hasher);
    key.hash(&mut hasher);
    &shards[(hasher.finish() as usize) & 15]
}

pub(crate) fn edge_kv_get(tenant_id: &str, key: &str) -> Option<String> {
    let shard = edge_kv_shard(tenant_id, key);
    {
        let guard = shard.read();
        let entry = guard.get(tenant_id).and_then(|m| m.get(key))?;
        if entry
            .expires_at_ms
            .is_none_or(|exp| current_epoch_ms() < exp)
        {
            return Some(entry.val.to_std_string());
        }
    }
    let mut guard = shard.write();
    let mut freed_expired = 0usize;
    let found = if let Some(t_map) = guard.get_mut(tenant_id) {
        if let Some(entry) = t_map.get(key) {
            if let Some(exp) = entry.expires_at_ms {
                if current_epoch_ms() >= exp {
                    if let Some(removed) = t_map.remove(key) {
                        freed_expired = edge_kv_entry_bytes(key, &removed);
                    }
                    None
                } else {
                    Some(entry.val.to_std_string())
                }
            } else {
                Some(entry.val.to_std_string())
            }
        } else {
            None
        }
    } else {
        None
    };
    drop(guard);
    if freed_expired > 0 {
        adjust_kv_owner_bytes(tenant_id, -(freed_expired as isize));
        if BOA_ACTIVE_TENANT.with(|t| t.borrow().as_str() == tenant_id) {
            crate::bindings::helpers::release_isolate_memory(freed_expired);
        }
    }
    found
}

pub(crate) fn edge_kv_set(tenant_id: &str, key: &str, val: &str, ttl_ms: u64) {
    let _ = edge_kv_set_checked(tenant_id, key, val, ttl_ms, 0, usize::MAX);
}

/// Inserts or overwrites `key` in `tenant_id`'s shard under a single write lock, enforcing
/// `mem_limit` against the exact signed byte delta (`new_bytes - old_bytes`).
/// Returns `Ok(net_delta)` on success, or `Err(projected_used)` without mutating the map on OOM.
pub(crate) fn edge_kv_set_checked(
    tenant_id: &str,
    key: &str,
    val: &str,
    ttl_ms: u64,
    current_used: usize,
    mem_limit: usize,
) -> Result<isize, usize> {
    let now = current_epoch_ms();
    let expires_at_ms = if ttl_ms > 0 {
        Some(now.saturating_add(ttl_ms))
    } else {
        None
    };
    let new_bytes = key.len().saturating_add(val.len());
    let shard = edge_kv_shard(tenant_id, key);
    let mut guard = shard.write();
    if let Some(t_map) = guard.get_mut(tenant_id) {
        let old_bytes = t_map
            .get(key)
            .map(|entry| edge_kv_entry_bytes(key, entry))
            .unwrap_or(0);
        let mut delta = new_bytes as isize - old_bytes as isize;
        let mut need_reclaim_expired = false;
        if delta > 0 && current_used.saturating_add(delta as usize) > mem_limit {
            let reclaimable: usize = t_map
                .iter()
                .filter(|(k, e)| k.as_str() != key && e.expires_at_ms.is_some_and(|exp| now >= exp))
                .map(|(k, e)| edge_kv_entry_bytes(k.as_str(), e))
                .sum();
            let adjusted_delta = delta - reclaimable as isize;
            if adjusted_delta > 0
                && current_used.saturating_add(adjusted_delta as usize) > mem_limit
            {
                return Err(current_used.saturating_add(adjusted_delta as usize));
            }
            delta = adjusted_delta;
            need_reclaim_expired = reclaimable > 0;
        }
        if need_reclaim_expired {
            t_map.retain(|k, e| k.as_str() == key || e.expires_at_ms.is_none_or(|exp| now < exp));
        }
        if let Some(entry) = t_map.get_mut(key) {
            entry.val = KvValue::Str(CompactString::new(val));
            entry.expires_at_ms = expires_at_ms;
        } else {
            t_map.insert(
                CompactString::new(key),
                KvEntry {
                    val: KvValue::Str(CompactString::new(val)),
                    expires_at_ms,
                },
            );
        }
        EDGE_KV_HAS_WRITES.store(true, std::sync::atomic::Ordering::Relaxed);
        drop(guard);
        adjust_kv_owner_bytes(tenant_id, delta);
        return Ok(delta);
    }

    let projected = current_used.saturating_add(new_bytes);
    if new_bytes > 0 && projected > mem_limit {
        return Err(projected);
    }
    let mut t_map = FxHashMap::default();
    t_map.insert(
        CompactString::new(key),
        KvEntry {
            val: KvValue::Str(CompactString::new(val)),
            expires_at_ms,
        },
    );
    guard.insert(CompactString::new(tenant_id), t_map);
    EDGE_KV_HAS_WRITES.store(true, std::sync::atomic::Ordering::Relaxed);
    drop(guard);
    adjust_kv_owner_bytes(tenant_id, new_bytes as isize);
    Ok(new_bytes as isize)
}

#[inline]
pub(crate) fn edge_kv_entry_bytes(key: &str, entry: &KvEntry) -> usize {
    let val_bytes = match &entry.val {
        KvValue::Str(s) => s.len(),
        KvValue::Int(_) => 24,
    };
    key.len().saturating_add(val_bytes)
}

#[inline]
pub(crate) fn edge_kv_tenant_bytes(tenant_id: &str) -> usize {
    if EDGE_KV_NONEMPTY_OWNERS.load(std::sync::atomic::Ordering::Relaxed) == 0 {
        return 0;
    }
    kv_owner_bytes_map()
        .read()
        .get(tenant_id)
        .copied()
        .unwrap_or(0)
}

/// Computes live `Slice.kv` bytes owned by `tenant_id` AND any nested child sub-sandboxes
/// (`{tenant_id}:sub` and `{tenant_id}:sub:*`), fast-pathing in `< 1 ns` when empty and
/// `O(1)` (`~8 ns`, zero shard iteration) when no sub-tenant KV owners exist.
#[inline(always)]
pub(crate) fn edge_kv_tenant_tree_bytes(tenant_id: &str) -> usize {
    if EDGE_KV_NONEMPTY_OWNERS.load(std::sync::atomic::Ordering::Relaxed) == 0 {
        return 0;
    }
    edge_kv_tenant_tree_bytes_slow(tenant_id)
}

#[inline(never)]
fn edge_kv_tenant_tree_bytes_slow(tenant_id: &str) -> usize {
    let map = kv_owner_bytes_map().read();
    if !EDGE_KV_HAS_SUBTENANT_OWNERS.load(std::sync::atomic::Ordering::Relaxed) {
        return map.get(tenant_id).copied().unwrap_or(0);
    }
    let mut total = 0usize;
    for (owner, &bytes) in map.iter() {
        let matches_tree = owner.as_str() == tenant_id
            || (!tenant_id.is_empty()
                && owner.starts_with(tenant_id)
                && owner.as_bytes().get(tenant_id.len()) == Some(&b':'));
        if matches_tree {
            total = total.saturating_add(bytes);
        }
    }
    total
}

/// Captures a snapshot of `tenant_id` (and any `{tenant_id}:*` sub-tenants) across the 16 KV shards
/// so an aborted/trapped child `Slice.evalSandbox` turn can roll back uncommitted KV writes cleanly.
pub(crate) fn snapshot_tenant_tree_kv(tenant_id: &str) -> Option<Box<[KvShardMap; 16]>> {
    if !EDGE_KV_HAS_WRITES.load(std::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    let shards = EDGE_KV_SLAB.get()?;
    let mut out: Box<[KvShardMap; 16]> = Box::new(std::array::from_fn(|_| FxHashMap::default()));
    let mut any = false;
    for (idx, shard) in shards.iter().enumerate() {
        let guard = shard.read();
        for (owner, t_map) in guard.iter() {
            let matches_tree = owner.as_str() == tenant_id
                || (!tenant_id.is_empty()
                    && owner.starts_with(tenant_id)
                    && owner.as_bytes().get(tenant_id.len()) == Some(&b':'));
            if matches_tree && !t_map.is_empty() {
                out[idx].insert(owner.clone(), t_map.clone());
                any = true;
            }
        }
    }
    any.then_some(out)
}

/// Restores (or wipes if `snap` is `None`) `tenant_id` and `{tenant_id}:*` across all 16 KV shards.
pub(crate) fn restore_or_clear_tenant_tree_kv(
    tenant_id: &str,
    snap: Option<Box<[KvShardMap; 16]>>,
) {
    if !EDGE_KV_HAS_WRITES.load(std::sync::atomic::Ordering::Relaxed) && snap.is_none() {
        return;
    }
    let Some(shards) = EDGE_KV_SLAB.get() else {
        return;
    };
    let mut snap_arr = snap;
    let mut restored_owners: FxHashMap<CompactString, usize> = FxHashMap::default();
    let now = current_epoch_ms();
    for (idx, shard) in shards.iter().enumerate() {
        let mut guard = shard.write();
        if !guard.is_empty() {
            guard.retain(|owner, _| {
                !(owner.as_str() == tenant_id
                    || (!tenant_id.is_empty()
                        && owner.starts_with(tenant_id)
                        && owner.as_bytes().get(tenant_id.len()) == Some(&b':')))
            });
        }
        if let Some(ref mut arr) = snap_arr {
            for (owner, t_map) in arr[idx].drain() {
                let mut b = 0usize;
                for (k, entry) in &t_map {
                    if entry.expires_at_ms.is_none_or(|exp| now < exp) {
                        b = b.saturating_add(edge_kv_entry_bytes(k.as_str(), entry));
                    }
                }
                if b > 0 {
                    *restored_owners.entry(owner.clone()).or_insert(0) += b;
                }
                guard.insert(owner, t_map);
            }
        }
    }
    let mut map = kv_owner_bytes_map().write();
    map.retain(|owner, _| {
        !(owner.as_str() == tenant_id
            || (!tenant_id.is_empty()
                && owner.starts_with(tenant_id)
                && owner.as_bytes().get(tenant_id.len()) == Some(&b':')))
    });
    for (owner, bytes) in restored_owners {
        map.insert(owner, bytes);
    }
    EDGE_KV_NONEMPTY_OWNERS.store(map.len(), std::sync::atomic::Ordering::Relaxed);
    EDGE_KV_HAS_SUBTENANT_OWNERS.store(
        map.keys().any(|k| k.contains(':')),
        std::sync::atomic::Ordering::Relaxed,
    );
}

pub(crate) fn edge_kv_del_bytes(tenant_id: &str, key: &str) -> Option<usize> {
    let shard = edge_kv_shard(tenant_id, key);
    let mut guard = shard.write();
    let freed = guard
        .get_mut(tenant_id)
        .and_then(|t_map| t_map.remove(key))
        .map(|entry| edge_kv_entry_bytes(key, &entry));
    drop(guard);
    if let Some(b) = freed {
        adjust_kv_owner_bytes(tenant_id, -(b as isize));
    }
    freed
}

/// Atomically increments a 64-bit integer counter in `tenant_id`'s shard under a single write lock,
/// enforcing `mem_limit` against the exact signed byte delta (`(key.len() + 24) - old_bytes`).
pub(crate) fn edge_kv_incr_checked(
    tenant_id: &str,
    key: &str,
    delta: i64,
    current_used: usize,
    mem_limit: usize,
) -> Result<(i64, isize), usize> {
    let now = current_epoch_ms();
    let new_bytes = key.len().saturating_add(24);
    let shard = edge_kv_shard(tenant_id, key);
    let mut guard = shard.write();
    if let Some(t_map) = guard.get_mut(tenant_id) {
        let old_bytes = t_map
            .get(key)
            .map(|entry| edge_kv_entry_bytes(key, entry))
            .unwrap_or(0);
        let mut mem_delta = new_bytes as isize - old_bytes as isize;
        let mut need_reclaim_expired = false;
        if mem_delta > 0 && current_used.saturating_add(mem_delta as usize) > mem_limit {
            let reclaimable: usize = t_map
                .iter()
                .filter(|(k, e)| k.as_str() != key && e.expires_at_ms.is_some_and(|exp| now >= exp))
                .map(|(k, e)| edge_kv_entry_bytes(k.as_str(), e))
                .sum();
            let adjusted = mem_delta - reclaimable as isize;
            if adjusted > 0 && current_used.saturating_add(adjusted as usize) > mem_limit {
                return Err(current_used.saturating_add(adjusted as usize));
            }
            mem_delta = adjusted;
            need_reclaim_expired = reclaimable > 0;
        }
        if need_reclaim_expired {
            t_map.retain(|k, e| k.as_str() == key || e.expires_at_ms.is_none_or(|exp| now < exp));
        }
        if let Some(entry) = t_map.get_mut(key) {
            let current = if entry.expires_at_ms.is_some_and(|exp| now >= exp) {
                0
            } else {
                entry.val.as_i64().unwrap_or(0)
            };
            let next = current.wrapping_add(delta);
            entry.val = KvValue::Int(next);
            entry.expires_at_ms = None;
            EDGE_KV_HAS_WRITES.store(true, std::sync::atomic::Ordering::Relaxed);
            drop(guard);
            adjust_kv_owner_bytes(tenant_id, mem_delta);
            return Ok((next, mem_delta));
        }
        t_map.insert(
            CompactString::new(key),
            KvEntry {
                val: KvValue::Int(delta),
                expires_at_ms: None,
            },
        );
        EDGE_KV_HAS_WRITES.store(true, std::sync::atomic::Ordering::Relaxed);
        drop(guard);
        adjust_kv_owner_bytes(tenant_id, mem_delta);
        return Ok((delta, mem_delta));
    }

    let projected = current_used.saturating_add(new_bytes);
    if projected > mem_limit {
        return Err(projected);
    }
    let mut t_map = FxHashMap::default();
    t_map.insert(
        CompactString::new(key),
        KvEntry {
            val: KvValue::Int(delta),
            expires_at_ms: None,
        },
    );
    guard.insert(CompactString::new(tenant_id), t_map);
    EDGE_KV_HAS_WRITES.store(true, std::sync::atomic::Ordering::Relaxed);
    drop(guard);
    adjust_kv_owner_bytes(tenant_id, new_bytes as isize);
    Ok((delta, new_bytes as isize))
}
