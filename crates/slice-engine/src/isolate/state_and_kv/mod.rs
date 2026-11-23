//! Sharded Edge KV Slab (`Slice.kv`), Sub-5us State Checkpoints (`Slice.checkpoint`/`Slice.restore`),
//! Deterministic SplitMix64 PRNG Re-export, and Outbound Egress Secret DLP Guard.

mod checkpoints;
mod edge_kv;

pub use checkpoints::{MAX_CHECKPOINTS_PER_SLAB, checkpoint_slab_len};
pub(crate) use checkpoints::{
    StateCheckpoint, checkpoint_tenant_state, clear_tenant_checkpoints, current_vfs_bytes_if_owned,
    current_vfs_state_bytes, release_tenant_checkpoint, restore_tenant_state,
    restore_tenant_state_checked,
};
pub(crate) use edge_kv::{
    edge_kv_del_bytes, edge_kv_get, edge_kv_incr_checked, edge_kv_set, edge_kv_set_checked,
    edge_kv_tenant_bytes, edge_kv_tenant_tree_bytes, restore_or_clear_tenant_tree_kv,
    snapshot_tenant_tree_kv,
};
pub(crate) use slice_sandbox::expand_egress_secret_variants;
pub use slice_sandbox::verify_egress_secrets_slice;
