//! 16-Way Sharded `EDGE_KV_SLAB` (`Slice.kv`) with O(1) Owner Byte Accounting & TTL Eviction.

use crate::boa_realm::BOA_ACTIVE_TENANT;
use compact_str::CompactString;
use rustc_hash::FxHashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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

pub(crate) static EDGE_KV_SLAB: OnceLock<[parking_lot::RwLock<KvShardMap>; 16]> = OnceLock::new();
pub(crate) static EDGE_KV_HAS_WRITES: AtomicBool = AtomicBool::new(false);
static EDGE_KV_OWNER_BYTES: OnceLock<parking_lot::RwLock<FxHashMap<CompactString, usize>>> =
    OnceLock::new();
pub(crate) static EDGE_KV_NONEMPTY_OWNERS: AtomicUsize = AtomicUsize::new(0);
static EDGE_KV_HAS_SUBTENANT_OWNERS: AtomicBool = AtomicBool::new(false);

#[inline]
fn kv_owner_bytes_map() -> &'static parking_lot::RwLock<FxHashMap<CompactString, usize>> {
    EDGE_KV_OWNER_BYTES.get_or_init(|| parking_lot::RwLock::new(FxHashMap::default()))
}

#[inline]
fn is_in_tenant_tree(owner: &str, tenant_id: &str) -> bool {
    owner == tenant_id
        || (!tenant_id.is_empty()
            && owner.starts_with(tenant_id)
            && owner.as_bytes().get(tenant_id.len()) == Some(&b':'))
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
                EDGE_KV_HAS_SUBTENANT_OWNERS.store(true, Ordering::Relaxed);
            }
            map.insert(CompactString::new(tenant_id), add);
            EDGE_KV_NONEMPTY_OWNERS.store(map.len(), Ordering::Relaxed);
        }
    } else {
        let sub = delta.unsigned_abs();
        let mut removed_sub = false;
        if let Some(cur) = map.get_mut(tenant_id) {
            if *cur <= sub {
                removed_sub = tenant_id.contains(':');
                map.remove(tenant_id);
                EDGE_KV_NONEMPTY_OWNERS.store(map.len(), Ordering::Relaxed);
            } else {
                *cur -= sub;
            }
        }
        if removed_sub && !map.keys().any(|k| k.contains(':')) {
            EDGE_KV_HAS_SUBTENANT_OWNERS.store(false, Ordering::Relaxed);
        }
    }
}

pub(super) fn set_kv_owner_exact_bytes(tenant_id: &str, bytes: usize) {
    let mut map = kv_owner_bytes_map().write();
    if bytes == 0 {
        let removed = map.remove(tenant_id).is_some();
        if removed {
            EDGE_KV_NONEMPTY_OWNERS.store(map.len(), Ordering::Relaxed);
            if tenant_id.contains(':') && !map.keys().any(|k| k.contains(':')) {
                EDGE_KV_HAS_SUBTENANT_OWNERS.store(false, Ordering::Relaxed);
            }
        }
    } else {
        if tenant_id.contains(':') {
            EDGE_KV_HAS_SUBTENANT_OWNERS.store(true, Ordering::Relaxed);
        }
        map.insert(CompactString::new(tenant_id), bytes);
        EDGE_KV_NONEMPTY_OWNERS.store(map.len(), Ordering::Relaxed);
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

fn check_and_evict_expired_for_write(
    t_map: &mut KvTenantMap,
    key: &str,
    new_bytes: usize,
    now: u64,
    current_used: usize,
    mem_limit: usize,
) -> Result<isize, usize> {
    let old_bytes = t_map
        .get(key)
        .map(|entry| edge_kv_entry_bytes(key, entry))
        .unwrap_or(0);
    let mut delta = new_bytes as isize - old_bytes as isize;
    if delta > 0 && current_used.saturating_add(delta as usize) > mem_limit {
        let reclaimable: usize = t_map
            .iter()
            .filter(|(k, e)| k.as_str() != key && e.expires_at_ms.is_some_and(|exp| now >= exp))
            .map(|(k, e)| edge_kv_entry_bytes(k.as_str(), e))
            .sum();
        let adjusted = delta - reclaimable as isize;
        if adjusted > 0 && current_used.saturating_add(adjusted as usize) > mem_limit {
            return Err(current_used.saturating_add(adjusted as usize));
        }
        delta = adjusted;
        if reclaimable > 0 {
            t_map.retain(|k, e| k.as_str() == key || e.expires_at_ms.is_none_or(|exp| now < exp));
        }
    }
    Ok(delta)
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
        let delta =
            check_and_evict_expired_for_write(t_map, key, new_bytes, now, current_used, mem_limit)?;
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
        EDGE_KV_HAS_WRITES.store(true, Ordering::Relaxed);
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
    EDGE_KV_HAS_WRITES.store(true, Ordering::Relaxed);
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
    if EDGE_KV_NONEMPTY_OWNERS.load(Ordering::Relaxed) == 0 {
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
    if EDGE_KV_NONEMPTY_OWNERS.load(Ordering::Relaxed) == 0 {
        return 0;
    }
    edge_kv_tenant_tree_bytes_slow(tenant_id)
}

#[inline(never)]
fn edge_kv_tenant_tree_bytes_slow(tenant_id: &str) -> usize {
    let map = kv_owner_bytes_map().read();
    if !EDGE_KV_HAS_SUBTENANT_OWNERS.load(Ordering::Relaxed) {
        return map.get(tenant_id).copied().unwrap_or(0);
    }
    let mut total = 0usize;
    for (owner, &bytes) in map.iter() {
        if is_in_tenant_tree(owner.as_str(), tenant_id) {
            total = total.saturating_add(bytes);
        }
    }
    total
}

/// Captures a snapshot of `tenant_id` (and any `{tenant_id}:*` sub-tenants) across the 16 KV shards
/// so an aborted/trapped child `Slice.evalSandbox` turn can roll back uncommitted KV writes cleanly.
pub(crate) fn snapshot_tenant_tree_kv(tenant_id: &str) -> Option<Box<[KvShardMap; 16]>> {
    if !EDGE_KV_HAS_WRITES.load(Ordering::Relaxed) {
        return None;
    }
    let shards = EDGE_KV_SLAB.get()?;
    let mut out: Box<[KvShardMap; 16]> = Box::new(std::array::from_fn(|_| FxHashMap::default()));
    let mut any = false;
    for (idx, shard) in shards.iter().enumerate() {
        let guard = shard.read();
        for (owner, t_map) in guard.iter() {
            if is_in_tenant_tree(owner.as_str(), tenant_id) && !t_map.is_empty() {
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
    if !EDGE_KV_HAS_WRITES.load(Ordering::Relaxed) && snap.is_none() {
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
            guard.retain(|owner, _| !is_in_tenant_tree(owner.as_str(), tenant_id));
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
    map.retain(|owner, _| !is_in_tenant_tree(owner.as_str(), tenant_id));
    for (owner, bytes) in restored_owners {
        map.insert(owner, bytes);
    }
    EDGE_KV_NONEMPTY_OWNERS.store(map.len(), Ordering::Relaxed);
    EDGE_KV_HAS_SUBTENANT_OWNERS.store(map.keys().any(|k| k.contains(':')), Ordering::Relaxed);
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
        let mem_delta =
            check_and_evict_expired_for_write(t_map, key, new_bytes, now, current_used, mem_limit)?;
        if let Some(entry) = t_map.get_mut(key) {
            let current = if entry.expires_at_ms.is_some_and(|exp| now >= exp) {
                0
            } else {
                entry.val.as_i64().unwrap_or(0)
            };
            let next = current.wrapping_add(delta);
            entry.val = KvValue::Int(next);
            entry.expires_at_ms = None;
            EDGE_KV_HAS_WRITES.store(true, Ordering::Relaxed);
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
        EDGE_KV_HAS_WRITES.store(true, Ordering::Relaxed);
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
    EDGE_KV_HAS_WRITES.store(true, Ordering::Relaxed);
    drop(guard);
    adjust_kv_owner_bytes(tenant_id, new_bytes as isize);
    Ok((delta, new_bytes as isize))
}
