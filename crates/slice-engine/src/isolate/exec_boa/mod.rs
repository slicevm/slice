//! Full ES2024+ `boa_engine` and Precompiled `BoaCAS` (`SLICEBC\x01`) Execution,
//! Warm Realm Pool Management, and Shared OCap/DLP/SES Compartment Lifecycle Guards.

mod finalize;
mod runners;
mod session_lifecycle;

pub(crate) use session_lifecycle::BoaSessionUnwindGuard;

use crate::boa_realm::{
    HOST_GROUP_DET, HOST_GROUP_HTTP_BRIDGE, HOST_GROUP_KV_SANDBOX, HOST_GROUP_SEMVER_FAST,
    WarmBoaRealm, ensure_realm_shards,
};

#[derive(Clone, Copy)]
pub(crate) struct BoaSessionFlags {
    pub(crate) has_fs_grants: bool,
    pub(crate) has_net_hosts: bool,
    pub(crate) has_egress_secrets: bool,
    pub(crate) has_env_keys: bool,
    pub(crate) has_exec_bins: bool,
    pub(crate) is_worker: bool,
    pub(crate) initial_mem_used: usize,
}

#[inline]
pub(crate) fn ensure_realm_http_bridge(realm: &mut WarmBoaRealm) {
    if (realm.registered_host_groups & HOST_GROUP_HTTP_BRIDGE) == 0 {
        crate::host_bindings::register_http_bridge_host_functions(&mut realm.ctx);
        realm.registered_host_groups |= HOST_GROUP_HTTP_BRIDGE;
    }
}

#[inline]
pub(crate) fn ensure_realm_semver_fast(realm: &mut WarmBoaRealm) {
    if (realm.registered_host_groups & HOST_GROUP_SEMVER_FAST) == 0 {
        crate::host_bindings::register_semver_and_fast_intrinsics_host_functions(&mut realm.ctx);
        realm.registered_host_groups |= HOST_GROUP_SEMVER_FAST;
    }
}

#[inline]
pub(crate) fn ensure_realm_kv_sandbox(realm: &mut WarmBoaRealm) {
    if (realm.registered_host_groups & HOST_GROUP_KV_SANDBOX) == 0 {
        crate::host_bindings::register_kv_and_sandbox_host_functions(&mut realm.ctx);
        realm.registered_host_groups |= HOST_GROUP_KV_SANDBOX;
    }
}

#[inline]
pub(crate) fn ensure_realm_det_funcs(realm: &mut WarmBoaRealm) {
    if (realm.registered_host_groups & HOST_GROUP_DET) == 0 {
        crate::host_bindings::register_deterministic_host_functions(&mut realm.ctx);
        realm.registered_host_groups |= HOST_GROUP_DET;
    }
}

/// Caches the evaluated `globalThis.__slice_rc[hex]` entrypoint `JsObject` handle (plus optional
/// `bflags` and `fused_spec` metadata) onto `realm` so subsequent warm invocations call it directly.
pub(crate) fn cache_loaded_rc_bundle(
    realm: &mut WarmBoaRealm,
    rc_key: u64,
    hex_bytes: &[u8; 16],
    bflags_opt: Option<u16>,
    fused_spec_opt: Option<u64>,
) {
    let Ok(hex_str) = std::str::from_utf8(hex_bytes) else {
        return;
    };
    let global = realm.ctx.global_object();
    if let Ok(rc_val) = global.get(boa_engine::js_string!("__slice_rc"), &mut realm.ctx)
        && let Some(rc_obj) = rc_val.as_object()
        && let Ok(val) = rc_obj.get(boa_engine::js_string!(hex_str), &mut realm.ctx)
        && let Some(obj) = val.as_object()
    {
        realm.loaded_bundles.insert(rc_key, obj.clone());
        if let Some(bflags) = bflags_opt {
            realm.loaded_bundle_flags.insert(rc_key, bflags);
        }
        if let Some(fused_spec) = fused_spec_opt {
            realm.loaded_bundle_fused_specs.insert(rc_key, fused_spec);
        }
    }
}

/// Formats `rc_key` as a 16-byte lowercase hex key on the stack and caches its `__slice_rc` handle.
#[inline]
pub(crate) fn cache_loaded_rc_bundle_u64(
    realm: &mut WarmBoaRealm,
    rc_key: u64,
    fused_spec_opt: Option<u64>,
) {
    let mut hex_buf = [0u8; 16];
    let _ = std::io::Write::write_fmt(&mut (&mut hex_buf[..]), format_args!("{:016x}", rc_key));
    cache_loaded_rc_bundle(realm, rc_key, &hex_buf, None, fused_spec_opt);
}

#[inline]
pub(super) fn init_worker_self_realm(realm: &mut WarmBoaRealm) {
    ensure_realm_shards(realm, (1 << 8) | (1 << 13) | (1 << 2), false);
    if let Some(ref init_fn) = realm.init_worker_self_fn {
        let _ = init_fn.call(
            &boa_engine::JsValue::undefined(),
            &[boa_engine::JsValue::from(true)],
            &mut realm.ctx,
        );
    } else {
        let _ = realm.ctx.eval(boa_engine::Source::from_bytes(
            b"globalThis.__slice_init_worker_self_env(true);",
        ));
    }
}
