//! Warm `boa_engine` Realm Pool, Thread-Local Compartment State, and OCap VFS Guards.

mod hooks_and_jobs;
mod pool_and_shards;
mod saved_context;
mod tls_state;
mod trace_and_sourcemap;
mod vfs_and_ses_guards;

pub(crate) use hooks_and_jobs::{SliceHostHooks, SliceJobExecutor};
pub(crate) use pool_and_shards::{
    acquire_warm_boa_realm, acquire_warm_boa_realm_for_tenant, build_compiler_boa_context,
    call_realm_ctl, create_bare_boa_realm, create_bare_boa_realm_for_tenant, create_warm_boa_realm,
    ensure_realm_shards, eval_or_reuse_cached_script, populate_realm_cached_handles,
    release_prewarmed_boa_realm, release_warm_boa_realm, try_acquire_pooled_boa_realm,
    try_acquire_pooled_boa_realm_for_tenant,
};
pub(crate) use saved_context::SavedIsolateContextGuard;
pub(crate) use tls_state::{
    ActiveBoaHttpReq, ActiveCasArchiveGuard, BOA_ACTIVE_CAPS, BOA_ACTIVE_CAS,
    BOA_ACTIVE_SOURCE_FUSED_SPEC, BOA_ACTIVE_SOURCE_PTR_LEN, BOA_ACTIVE_TENANT, BOA_ALS_ACTIVE,
    BOA_ALS_SNAP, BOA_CHECKPOINT_CAP, BOA_CHECKPOINT_STATS, BOA_CONTEXT_POOL, BOA_CUSTOM_ARGV,
    BOA_DET_STATE, BOA_DIRTY_SUBSYSTEMS, BOA_EGRESS_SECRETS, BOA_ENV_MAP, BOA_EXEC_BINS,
    BOA_EXEC_DIRTY, BOA_FETCH_MOCKS, BOA_FUEL_BUDGET, BOA_HTTP_REQ, BOA_HTTP_RES,
    BOA_KV_MEM_BUDGET, BOA_LAST_CHILD_PROC_VIOLATION, BOA_LAST_EXIT_CODE, BOA_NET_HOSTS,
    BOA_PROCESS_EXIT, BOA_SES_BASELINE_SIGS, BOA_SES_INTL_SIG_PAIR, BOA_STDOUT, BOA_TRACE_ENABLED,
    BOA_TRACE_EVENTS, BOA_UNCAUGHT_ERROR, BOA_UNHANDLED_REJECTIONS, BOA_VFS_OWNER_TENANT,
    BOA_VFS_STATE, BOA_VIOLATION, BOA_WAIT_UNTIL_COUNT, BOA_WS_UPGRADED, BoaFsCaps,
    CHECKPOINT_SLAB, CapturedBoaHttpRes, DIRTY_ALS, DIRTY_EVENT_LOOP, DIRTY_FETCH_MOCK,
    DIRTY_PROCESS, DIRTY_SOCKETS, DIRTY_VFS, DIRTY_WASM, DIRTY_WEBSOCKET, DIRTY_WORKERS,
    EXEC_DIRTY_ARGV, EXEC_DIRTY_CHILD_PROC, EXEC_DIRTY_EXIT, EXEC_DIRTY_REJECTIONS,
    EXEC_DIRTY_UNCAUGHT, HOST_GROUP_DET, HOST_GROUP_HTTP_BRIDGE, HOST_GROUP_KV_SANDBOX,
    HOST_GROUP_SEMVER_FAST, NEXT_CHECKPOINT_ID, REALM_CTL_DISPATCH_HTTP_BRIDGE,
    REALM_CTL_RESET_FS_WATCHERS, REALM_CTL_RESET_MT, REALM_CTL_RESET_WU, REALM_CTL_SET_DET_MODE,
    REALM_CTL_SET_NEXT_TICK, REALM_CTL_SYNC_ENV, SES_INTRINSIC_COUNT, SesIntlSigDelta, SesProtoSig,
    WarmBoaRealm, has_process_exit, has_uncaught_error, mark_exec_dirty, mark_subsystem_dirty,
    set_child_proc_violation, set_last_exit_code, set_process_exit, set_uncaught_error,
};
pub use trace_and_sourcemap::ChromeTraceEvent;
pub(crate) use trace_and_sourcemap::{
    ActiveSourceSliceGuard, encode_fused_source_spec, is_trace_recording_enabled,
    map_error_with_active_source, map_stack_with_active_source, record_trace_event,
    set_active_source_fused_spec,
};
pub(crate) use vfs_and_ses_guards::{
    SES_PROTO_NAMES, capture_ses_proto_sigs, check_boa_fs_read_ocap, check_boa_fs_write_ocap,
    ensure_vfs_tenant_affinity, first_ses_proto_sig_mismatch_idx, has_any_boa_fs_read_ocap,
    has_any_boa_fs_write_ocap, has_boa_fs_read_ocap, has_boa_fs_write_ocap, norm_vfs_path,
    proto_sig, vfs_owner_namespace,
};
