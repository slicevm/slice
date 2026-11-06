//! Sharded Edge KV Slab (`Slice.kv`), Sub-5us State Checkpoints (`Slice.checkpoint`/`Slice.restore`),
//! Deterministic SplitMix64 PRNG, and Outbound Egress Secret DLP Guard.

use crate::boa_realm::{BOA_VFS_CACHE, BOA_VFS_DIRS, CHECKPOINT_SLAB, NEXT_CHECKPOINT_ID};
use crate::intrinsics::base64_encode;
use rustc_hash::{FxHashMap, FxHashSet};
use slice_sandbox::SandboxViolation;

#[derive(Clone, Debug)]
pub(crate) struct StateCheckpoint {
    pub(crate) vfs_files: FxHashMap<String, Vec<u8>>,
    pub(crate) vfs_dirs: FxHashSet<String>,
    pub(crate) tenant_kv: Vec<((String, String), KvEntry)>,
}

pub(crate) fn checkpoint_tenant_state(tenant_id: &str) -> u64 {
    let vfs_files = BOA_VFS_CACHE.with(|c| c.borrow().clone());
    let vfs_dirs = BOA_VFS_DIRS.with(|d| d.borrow().clone());
    let mut tenant_kv = Vec::new();
    if let Some(shards) = EDGE_KV_SLAB.get() {
        for shard in shards {
            let guard = shard.lock().unwrap();
            for (k, v) in guard.iter() {
                if k.0 == tenant_id {
                    tenant_kv.push((k.clone(), v.clone()));
                }
            }
        }
    }
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
    id
}

pub(crate) fn restore_tenant_state(tenant_id: &str, checkpoint_id: u64) -> bool {
    let snap_opt = CHECKPOINT_SLAB.with(|slab| slab.borrow().get(&checkpoint_id).cloned());
    let Some(snap) = snap_opt else {
        return false;
    };
    BOA_VFS_CACHE.with(|c| *c.borrow_mut() = snap.vfs_files);
    BOA_VFS_DIRS.with(|d| *d.borrow_mut() = snap.vfs_dirs);
    if let Some(shards) = EDGE_KV_SLAB.get() {
        for shard in shards {
            let mut guard = shard.lock().unwrap();
            guard.retain(|k, _| k.0 != tenant_id);
        }
    }
    for ((t_id, key), entry) in snap.tenant_kv {
        let shard = edge_kv_shard(&t_id, &key);
        shard.lock().unwrap().insert((t_id, key), entry);
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
    for sec in secrets {
        if sec.is_empty() {
            continue;
        }
        let b64_std = base64_encode(sec.as_bytes(), false);
        let b64_url = base64_encode(sec.as_bytes(), true);
        if payload.contains(sec.as_str())
            || (!b64_std.is_empty() && payload.contains(&b64_std))
            || (!b64_url.is_empty() && payload.contains(&b64_url))
        {
            return Err(SandboxViolation::EnvAccessDenied {
                tenant_id: tenant_id.to_string(),
                key: "EGRESS_SECRET_LEAK_BLOCKED".to_string(),
            });
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub(crate) struct KvEntry {
    pub(crate) val: String,
    pub(crate) expires_at_ms: Option<u64>,
}

pub(crate) type KvShardMap = FxHashMap<(String, String), KvEntry>;

pub(crate) static EDGE_KV_SLAB: std::sync::OnceLock<[std::sync::Mutex<KvShardMap>; 16]> =
    std::sync::OnceLock::new();

#[inline]
pub(crate) fn current_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1_700_000_000_000)
}

#[inline]
pub(crate) fn edge_kv_shard(tenant_id: &str, key: &str) -> &'static std::sync::Mutex<KvShardMap> {
    let shards = EDGE_KV_SLAB
        .get_or_init(|| std::array::from_fn(|_| std::sync::Mutex::new(FxHashMap::default())));
    use std::hash::{Hash, Hasher};
    let mut hasher = rustc_hash::FxHasher::default();
    tenant_id.hash(&mut hasher);
    key.hash(&mut hasher);
    &shards[(hasher.finish() as usize) & 15]
}

pub(crate) fn edge_kv_get(tenant_id: &str, key: &str) -> Option<String> {
    let shard = edge_kv_shard(tenant_id, key);
    let mut guard = shard.lock().unwrap();
    let composite = (tenant_id.to_string(), key.to_string());
    if let Some(entry) = guard.get(&composite) {
        if let Some(exp) = entry.expires_at_ms {
            if current_epoch_ms() >= exp {
                guard.remove(&composite);
                return None;
            }
        }
        return Some(entry.val.clone());
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
    let mut guard = shard.lock().unwrap();
    guard.insert(
        (tenant_id.to_string(), key.to_string()),
        KvEntry {
            val: val.to_string(),
            expires_at_ms,
        },
    );
}

pub(crate) fn edge_kv_del(tenant_id: &str, key: &str) -> bool {
    let shard = edge_kv_shard(tenant_id, key);
    let mut guard = shard.lock().unwrap();
    guard
        .remove(&(tenant_id.to_string(), key.to_string()))
        .is_some()
}

pub(crate) fn edge_kv_incr(tenant_id: &str, key: &str, delta: i64) -> i64 {
    let shard = edge_kv_shard(tenant_id, key);
    let mut guard = shard.lock().unwrap();
    let composite = (tenant_id.to_string(), key.to_string());
    let now = current_epoch_ms();
    let current = guard
        .get(&composite)
        .and_then(|entry| {
            if let Some(exp) = entry.expires_at_ms {
                if now >= exp {
                    return None;
                }
            }
            entry.val.parse::<i64>().ok()
        })
        .unwrap_or(0);
    let next = current.wrapping_add(delta);
    guard.insert(
        composite,
        KvEntry {
            val: next.to_string(),
            expires_at_ms: None,
        },
    );
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
