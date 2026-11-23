//! Sub-5us Tenant State Checkpoints (`Slice.checkpoint`, `Slice.restore`, `Slice.releaseCheckpoint`).

use super::edge_kv::{
    EDGE_KV_HAS_WRITES, EDGE_KV_SLAB, KvTenantMap, current_epoch_ms, edge_kv_entry_bytes,
    edge_kv_tenant_bytes, set_kv_owner_exact_bytes,
};
use crate::bindings::fs::VfsSnapshot;
use crate::boa_realm::{BOA_VFS_OWNER_TENANT, BOA_VFS_STATE, CHECKPOINT_SLAB, NEXT_CHECKPOINT_ID};
use compact_str::CompactString;
use rustc_hash::FxHashMap;

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
    if BOA_VFS_OWNER_TENANT.with(|o| o.borrow().as_str() == vfs_ns) {
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
