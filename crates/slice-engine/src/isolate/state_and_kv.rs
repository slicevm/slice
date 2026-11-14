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
fn current_vfs_state_bytes() -> usize {
    BOA_VFS_STATE.with(|v| v.borrow().total_accounted_bytes())
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
            if let Some(t_map) = guard.get(tenant_id) {
                if !t_map.is_empty() {
                    kv_keys_count += t_map.len();
                    for (k, entry) in t_map {
                        if entry.expires_at_ms.is_none_or(|exp| now < exp) {
                            kv_bytes =
                                kv_bytes.saturating_add(edge_kv_entry_bytes(k.as_str(), entry));
                        }
                    }
                    tenant_kv[idx] = Some(t_map.clone());
                }
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
    for (idx, shard_snap) in snap.tenant_kv.into_iter().enumerate() {
        if let Some(t_map) = shard_snap {
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

#[cold]
#[inline(never)]
pub(crate) fn verify_egress_secrets_slice(
    tenant_id: &str,
    secrets: &[String],
    payload: &str,
) -> Result<(), SandboxViolation> {
    let payload_bytes = payload.as_bytes();
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
    if freed_expired > 0 && BOA_ACTIVE_TENANT.with(|t| t.borrow().as_str() == tenant_id) {
        crate::bindings::helpers::release_isolate_memory(freed_expired);
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

pub(crate) fn edge_kv_tenant_bytes(tenant_id: &str) -> usize {
    let Some(shards) = EDGE_KV_SLAB.get() else {
        return 0;
    };
    let now = current_epoch_ms();
    let mut total = 0usize;
    for shard in shards {
        let guard = shard.read();
        if let Some(t_map) = guard.get(tenant_id) {
            for (k, entry) in t_map {
                if entry.expires_at_ms.is_none_or(|exp| now < exp) {
                    total = total.saturating_add(edge_kv_entry_bytes(k.as_str(), entry));
                }
            }
        }
    }
    total
}

pub(crate) fn edge_kv_del_bytes(tenant_id: &str, key: &str) -> Option<usize> {
    let shard = edge_kv_shard(tenant_id, key);
    let mut guard = shard.write();
    guard
        .get_mut(tenant_id)
        .and_then(|t_map| t_map.remove(key))
        .map(|entry| edge_kv_entry_bytes(key, &entry))
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
            return Ok((next, mem_delta));
        }
        t_map.insert(
            CompactString::new(key),
            KvEntry {
                val: KvValue::Int(delta),
                expires_at_ms: None,
            },
        );
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
    Ok((delta, new_bytes as isize))
}
