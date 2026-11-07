//! Sharded Edge KV Slab (`Slice.kv`), Sub-5us State Checkpoints (`Slice.checkpoint`/`Slice.restore`),
//! Deterministic SplitMix64 PRNG, and Outbound Egress Secret DLP Guard.

use crate::boa_realm::{BOA_VFS_CACHE, BOA_VFS_DIRS, CHECKPOINT_SLAB, NEXT_CHECKPOINT_ID};
use compact_str::CompactString;
use rustc_hash::{FxHashMap, FxHashSet};
use slice_sandbox::SandboxViolation;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(crate) struct StateCheckpoint {
    pub(crate) vfs_files: FxHashMap<String, Arc<Vec<u8>>>,
    pub(crate) vfs_dirs: FxHashSet<String>,
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
        slab.borrow_mut().insert(
            id,
            StateCheckpoint {
                vfs_files,
                vfs_dirs,
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
    let snap_opt = CHECKPOINT_SLAB.with(|slab| slab.borrow().get(&checkpoint_id).cloned());
    let Some(snap) = snap_opt else {
        return false;
    };
    let vfs_files_count = snap.vfs_files.len();
    BOA_VFS_CACHE.with(|c| *c.borrow_mut() = snap.vfs_files);
    BOA_VFS_DIRS.with(|d| *d.borrow_mut() = snap.vfs_dirs);
    let shards = EDGE_KV_SLAB
        .get_or_init(|| std::array::from_fn(|_| parking_lot::RwLock::new(FxHashMap::default())));
    for (idx, shard_snap) in snap.tenant_kv.into_iter().enumerate() {
        let mut guard = shards[idx].write();
        if let Some(t_map) = shard_snap {
            guard.insert(CompactString::new(tenant_id), t_map);
        } else {
            guard.remove(tenant_id);
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
            return Err(SandboxViolation::EnvAccessDenied {
                tenant_id: tenant_id.to_string(),
                key: "EGRESS_SECRET_LEAK_BLOCKED".to_string(),
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
pub(crate) fn edge_kv_shard(tenant_id: &str, key: &str) -> &'static parking_lot::RwLock<KvShardMap> {
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
        if entry.expires_at_ms.is_none_or(|exp| current_epoch_ms() < exp) {
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

pub(crate) fn edge_kv_del(tenant_id: &str, key: &str) -> bool {
    let shard = edge_kv_shard(tenant_id, key);
    let mut guard = shard.write();
    guard
        .get_mut(tenant_id)
        .and_then(|t_map| t_map.remove(key))
        .is_some()
}

pub(crate) fn edge_kv_incr(tenant_id: &str, key: &str, delta: i64) -> i64 {
    let shard = edge_kv_shard(tenant_id, key);
    let mut guard = shard.write();
    let now = current_epoch_ms();
    if let Some(t_map) = guard.get_mut(tenant_id) {
        if let Some(entry) = t_map.get_mut(key) {
            let current = if entry.expires_at_ms.is_some_and(|exp| now >= exp) {
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

#[inline]
pub(crate) fn splitmix64_next_f64(state: &mut u64) -> f64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    ((z >> 11) as f64) * (1.0 / ((1u64 << 53) as f64))
}
