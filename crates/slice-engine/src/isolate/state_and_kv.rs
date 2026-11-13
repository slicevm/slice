//! Sharded Edge KV Slab (`Slice.kv`), Sub-5us State Checkpoints (`Slice.checkpoint`/`Slice.restore`),
//! Deterministic SplitMix64 PRNG Re-export, and Outbound Egress Secret DLP Guard.

use crate::boa_realm::{
    BOA_VFS_CACHE, BOA_VFS_DIRS, BOA_VFS_MTIMES, BOA_VFS_TOMBSTONES, CHECKPOINT_SLAB,
    NEXT_CHECKPOINT_ID,
};
pub(crate) use crate::intrinsics::splitmix64_next_f64;
use compact_str::CompactString;
use rustc_hash::{FxHashMap, FxHashSet};
use slice_sandbox::SandboxViolation;
use std::sync::Arc;

/// Maximum retained checkpoints per worker thread slab before FIFO eviction of the oldest snapshot.
pub const MAX_CHECKPOINTS_PER_SLAB: usize = 64;

#[derive(Clone, Debug)]
pub(crate) struct StateCheckpoint {
    pub(crate) owner_tenant: CompactString,
    pub(crate) vfs_files: FxHashMap<String, Arc<Vec<u8>>>,
    pub(crate) vfs_dirs: FxHashSet<String>,
    pub(crate) vfs_tombstones: FxHashSet<String>,
    pub(crate) vfs_mtimes: FxHashMap<String, f64>,
    pub(crate) tenant_kv: [Option<KvTenantMap>; 16],
}

pub(crate) fn checkpoint_tenant_state(tenant_id: &str) -> u64 {
    let trace_on = crate::boa_realm::is_trace_recording_enabled();
    let t0 = if trace_on {
        Some(std::time::Instant::now())
    } else {
        None
    };
    let vfs_files = BOA_VFS_CACHE.with(|c| c.borrow().clone());
    let vfs_dirs = BOA_VFS_DIRS.with(|d| d.borrow().clone());
    let vfs_tombstones = BOA_VFS_TOMBSTONES.with(|t| {
        let b = t.borrow();
        if b.is_empty() {
            FxHashSet::default()
        } else {
            b.clone()
        }
    });
    let vfs_mtimes = BOA_VFS_MTIMES.with(|m| {
        let b = m.borrow();
        if b.is_empty() {
            FxHashMap::default()
        } else {
            b.clone()
        }
    });
    let mut tenant_kv: [Option<KvTenantMap>; 16] = std::array::from_fn(|_| None);
    let mut kv_keys_count = 0usize;
    if let Some(shards) = EDGE_KV_SLAB.get() {
        for (idx, shard) in shards.iter().enumerate() {
            let guard = shard.read();
            if let Some(t_map) = guard.get(tenant_id) {
                if !t_map.is_empty() {
                    kv_keys_count += t_map.len();
                    tenant_kv[idx] = Some(t_map.clone());
                }
            }
        }
    }
    let vfs_files_count = vfs_files.len();
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
                vfs_files,
                vfs_dirs,
                vfs_tombstones,
                vfs_mtimes,
                tenant_kv,
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

pub(crate) fn restore_tenant_state(tenant_id: &str, checkpoint_id: u64) -> bool {
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
        return false;
    };
    let vfs_files_count = snap.vfs_files.len();
    BOA_VFS_CACHE.with(|c| *c.borrow_mut() = snap.vfs_files);
    BOA_VFS_DIRS.with(|d| *d.borrow_mut() = snap.vfs_dirs);
    BOA_VFS_TOMBSTONES.with(|t| *t.borrow_mut() = snap.vfs_tombstones);
    BOA_VFS_MTIMES.with(|m| *m.borrow_mut() = snap.vfs_mtimes);
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
    true
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
    if let Some(t_map) = guard.get_mut(tenant_id) {
        if let Some(entry) = t_map.get(key) {
            if let Some(exp) = entry.expires_at_ms {
                if current_epoch_ms() >= exp {
                    t_map.remove(key);
                    return None;
                }
            }
            return Some(entry.val.to_std_string());
        }
    }
    None
}

pub(crate) fn edge_kv_set(tenant_id: &str, key: &str, val: &str, ttl_ms: u64) {
    let expires_at_ms = if ttl_ms > 0 {
        Some(current_epoch_ms().saturating_add(ttl_ms))
    } else {
        None
    };
    let shard = edge_kv_shard(tenant_id, key);
    let mut guard = shard.write();
    if let Some(t_map) = guard.get_mut(tenant_id) {
        if let Some(entry) = t_map.get_mut(key) {
            entry.val = KvValue::Str(CompactString::new(val));
            entry.expires_at_ms = expires_at_ms;
            return;
        }
        t_map.insert(
            CompactString::new(key),
            KvEntry {
                val: KvValue::Str(CompactString::new(val)),
                expires_at_ms,
            },
        );
        return;
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
pub(crate) fn edge_kv_contains_live_key(tenant_id: &str, key: &str) -> bool {
    let shard = edge_kv_shard(tenant_id, key);
    let guard = shard.read();
    guard
        .get(tenant_id)
        .and_then(|m| m.get(key))
        .is_some_and(|entry| {
            entry
                .expires_at_ms
                .is_none_or(|exp| current_epoch_ms() < exp)
        })
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

pub(crate) fn edge_kv_incr(tenant_id: &str, key: &str, delta: i64) -> i64 {
    let shard = edge_kv_shard(tenant_id, key);
    let mut guard = shard.write();
    if let Some(t_map) = guard.get_mut(tenant_id) {
        if let Some(entry) = t_map.get_mut(key) {
            let current = if entry
                .expires_at_ms
                .is_some_and(|exp| current_epoch_ms() >= exp)
            {
                0
            } else {
                entry.val.as_i64().unwrap_or(0)
            };
            let next = current.wrapping_add(delta);
            entry.val = KvValue::Int(next);
            entry.expires_at_ms = None;
            return next;
        }
        let next = delta;
        t_map.insert(
            CompactString::new(key),
            KvEntry {
                val: KvValue::Int(next),
                expires_at_ms: None,
            },
        );
        return next;
    }
    let next = delta;
    let mut t_map = FxHashMap::default();
    t_map.insert(
        CompactString::new(key),
        KvEntry {
            val: KvValue::Int(next),
            expires_at_ms: None,
        },
    );
    guard.insert(CompactString::new(tenant_id), t_map);
    next
}
