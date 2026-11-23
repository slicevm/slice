//! Warm `boa_engine` Realm Pool, `IdleModuleLoader`, Cached Script Evaluation, and Shard Loading.

use super::{
    BOA_CONTEXT_POOL, BOA_SES_BASELINE_SIGS, BOA_SES_INTL_SIG_PAIR, BOA_VIOLATION, SliceHostHooks,
    SliceJobExecutor, WarmBoaRealm, capture_ses_proto_sigs, proto_sig,
};
use crate::host_bindings::register_slice_native_host_functions;
use compact_str::CompactString;
use rustc_hash::FxHashMap;

#[inline]
pub(crate) fn try_acquire_pooled_boa_realm_for_tenant(tenant_id: &str) -> Option<WarmBoaRealm> {
    BOA_CONTEXT_POOL.with(|pool| {
        let mut p = pool.borrow_mut();
        if p.is_empty() {
            return None;
        }
        if tenant_id.is_empty() {
            let idx = p.iter().rposition(|r| r.owner_tenant.is_empty())?;
            return Some(p.remove(idx));
        }
        if p.last().is_some_and(|r| r.owner_tenant == tenant_id) {
            return p.pop();
        }
        if let Some(idx) = p.iter().rposition(|r| r.owner_tenant == tenant_id) {
            return Some(p.remove(idx));
        }
        // Only adopt an unnamed (`""`) realm if it is a virgin prewarmed realm that has never
        // executed untrusted guest code for an unnamed isolate.
        if let Some(idx) = p
            .iter()
            .rposition(|r| r.owner_tenant.is_empty() && r.virgin_prewarmed)
        {
            let mut realm = p.remove(idx);
            realm.owner_tenant = CompactString::new(tenant_id);
            realm.virgin_prewarmed = false;
            return Some(realm);
        }
        None
    })
}

#[inline]
pub(crate) fn try_acquire_pooled_boa_realm() -> Option<WarmBoaRealm> {
    try_acquire_pooled_boa_realm_for_tenant("")
}

#[inline]
pub(crate) fn acquire_warm_boa_realm_for_tenant(tenant_id: &str) -> WarmBoaRealm {
    try_acquire_pooled_boa_realm_for_tenant(tenant_id)
        .unwrap_or_else(|| create_warm_boa_realm_for_tenant(tenant_id))
}

#[inline]
pub(crate) fn acquire_warm_boa_realm() -> WarmBoaRealm {
    acquire_warm_boa_realm_for_tenant("")
}

#[inline]
pub(crate) fn release_warm_boa_realm(mut realm: WarmBoaRealm) {
    realm.virgin_prewarmed = false;
    push_realm_to_pool(realm);
}

#[inline]
pub(crate) fn release_prewarmed_boa_realm(mut realm: WarmBoaRealm) {
    realm.virgin_prewarmed = true;
    push_realm_to_pool(realm);
}

#[inline]
fn push_realm_to_pool(realm: WarmBoaRealm) {
    BOA_CONTEXT_POOL.with(|pool| {
        let mut p = pool.borrow_mut();
        if p.len() >= 8 {
            p.remove(0);
        }
        p.push(realm);
    });
}

thread_local! {
    static IDLE_MODULE_LOADER: std::rc::Rc<boa_engine::module::IdleModuleLoader> =
        std::rc::Rc::new(boa_engine::module::IdleModuleLoader);
}

/// Returns a zero-syscall thread-local `IdleModuleLoader` so `boa_engine::ContextBuilder::build`
/// never falls back to `SimpleModuleLoader::new(".")` (`Path::canonicalize(".")` host `realpath`).
#[inline]
pub(crate) fn idle_module_loader() -> std::rc::Rc<boa_engine::module::IdleModuleLoader> {
    IDLE_MODULE_LOADER.with(std::rc::Rc::clone)
}

/// Builds a minimal `boa_engine::Context` wired with `IdleModuleLoader` (zero filesystem syscalls).
#[inline]
pub(crate) fn build_compiler_boa_context() -> boa_engine::Context {
    boa_engine::context::ContextBuilder::new()
        .module_loader(idle_module_loader())
        .build()
        .unwrap_or_else(|_| boa_engine::Context::default())
}

/// Creates a bare `boa_engine::Context` with `SliceJobExecutor` and core native Rust host functions
/// registered, without evaluating `ULTRA_CORE_PRELUDE` yet (used when executing pre-fused or
/// pre-compiled `SLICEBC\x01` payloads on cold start).
pub(crate) fn create_bare_boa_realm_for_tenant(tenant_id: &str) -> WarmBoaRealm {
    let mut ctx = boa_engine::context::ContextBuilder::new()
        .module_loader(idle_module_loader())
        .job_executor(std::rc::Rc::new(SliceJobExecutor::new()))
        .host_hooks(std::rc::Rc::new(SliceHostHooks))
        .build()
        .unwrap_or_else(|_| build_compiler_boa_context());
    register_slice_native_host_functions(&mut ctx);
    BOA_SES_BASELINE_SIGS.with(|s| s.set(None));
    let ses_proto_sigs = capture_ses_proto_sigs(&ctx);
    WarmBoaRealm {
        owner_tenant: CompactString::new(tenant_id),
        ctx,
        loaded_bundles: FxHashMap::default(),
        loaded_bundle_flags: FxHashMap::default(),
        loaded_bundle_fused_specs: FxHashMap::default(),
        compiled_scripts: FxHashMap::default(),
        ses_check_fn: None,
        ensure_shard_fn: None,
        init_worker_self_fn: None,
        drain_wu_fn: None,
        ctl_fn: None,
        ses_proto_sigs,
        loaded_shards_mask: 0,
        registered_host_groups: 0,
        has_pending_wu: false,
        virgin_prewarmed: false,
    }
}

#[inline]
pub(crate) fn create_bare_boa_realm() -> WarmBoaRealm {
    create_bare_boa_realm_for_tenant("")
}

/// Refreshes the cached JS function handles (`__slice_check_ses`, `__slice_ensure_shard`,
/// `__slice_drain_wu`, `__slice_ctl`) on `realm` after `ULTRA_CORE_PRELUDE` has been evaluated.
#[inline]
pub(crate) fn populate_realm_cached_handles(realm: &mut WarmBoaRealm) {
    let global = realm.ctx.global_object();
    if realm.ses_check_fn.is_none() {
        realm.ses_check_fn = global
            .get(boa_engine::js_string!("__slice_check_ses"), &mut realm.ctx)
            .ok()
            .and_then(|v| v.as_object());
        realm.ses_proto_sigs = BOA_SES_BASELINE_SIGS
            .with(|s| s.replace(None))
            .unwrap_or_else(|| capture_ses_proto_sigs(&realm.ctx));
    }
    if realm.ensure_shard_fn.is_none() {
        realm.ensure_shard_fn = global
            .get(
                boa_engine::js_string!("__slice_ensure_shard"),
                &mut realm.ctx,
            )
            .ok()
            .and_then(|v| v.as_object());
    }
    if realm.drain_wu_fn.is_none() {
        realm.drain_wu_fn = global
            .get(boa_engine::js_string!("__slice_drain_wu"), &mut realm.ctx)
            .ok()
            .and_then(|v| v.as_object());
    }
    if realm.ctl_fn.is_none() {
        realm.ctl_fn = global
            .get(boa_engine::js_string!("__slice_ctl"), &mut realm.ctx)
            .ok()
            .and_then(|v| v.as_object());
    }
}

#[inline]
pub(crate) fn call_realm_ctl(realm: &mut WarmBoaRealm, op: i32, flag: bool) {
    if realm.ctl_fn.is_none() {
        populate_realm_cached_handles(realm);
    }
    if let Some(ctl) = realm.ctl_fn.clone() {
        let _ = ctl.call(
            &boa_engine::JsValue::undefined(),
            &[
                boa_engine::JsValue::from(op),
                boa_engine::JsValue::from(flag),
            ],
            &mut realm.ctx,
        );
    }
}

#[inline]
fn fnv1a_script_key(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ (bytes.len() as u64);
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Evaluates a scoped JS script (`{ ... }`) on a warm realm, caching its compiled `boa_engine::Script`
/// (`Gc<CodeBlock>`) so subsequent warm invocations skip lexing, parsing, scope analysis, and byte compilation.
#[inline]
pub(crate) fn eval_or_reuse_cached_script(
    realm: &mut WarmBoaRealm,
    scoped_js: &str,
) -> boa_engine::JsResult<boa_engine::JsValue> {
    let key = fnv1a_script_key(scoped_js.as_bytes());
    if let Some(cached) = realm.compiled_scripts.get(&key).cloned() {
        return cached.evaluate(&mut realm.ctx);
    }
    let script = boa_engine::Script::parse(
        boa_engine::Source::from_bytes(scoped_js.as_bytes()),
        None,
        &mut realm.ctx,
    )?;
    let reusable = script.can_reuse_across_runs(&mut realm.ctx);
    let res = script.evaluate(&mut realm.ctx);
    if reusable
        && res.is_ok()
        && BOA_VIOLATION.with(|v| v.borrow().is_none())
        && realm.compiled_scripts.len() < 32
    {
        realm.compiled_scripts.insert(key, script);
    }
    res
}

/// Creates a warm `boa_engine::Context` initialized with Slice's native host functions,
/// WinterTC / WHATWG globals (`console.*`, `fetch`, `URL`, `Headers`, `Request`, `Response`,
/// `ReadableStream`, `WritableStream`, `TransformStream`, `WebCrypto`, `Buffer`, `structuredClone`),
/// and microtask/macrotask schedulers (`process.nextTick`, `queueMicrotask`, `setImmediate`, `setTimeout`).
pub(crate) fn create_warm_boa_realm_for_tenant(tenant_id: &str) -> WarmBoaRealm {
    let mut realm = create_bare_boa_realm_for_tenant(tenant_id);
    let _ = crate::js_preludes::eval_ultra_core_prelude(&mut realm.ctx);
    populate_realm_cached_handles(&mut realm);
    realm
}

pub(crate) fn create_warm_boa_realm() -> WarmBoaRealm {
    create_warm_boa_realm_for_tenant("")
}

#[inline]
pub(crate) fn ensure_realm_shards(
    realm: &mut WarmBoaRealm,
    needed_mask: u32,
    init_lazy_getters: bool,
) {
    let needed_mask = crate::js_preludes::expand_shard_prerequisites(needed_mask);
    let missing = needed_mask & !realm.loaded_shards_mask;
    if init_lazy_getters && (realm.loaded_shards_mask & 1) == 0 {
        let _ = crate::js_preludes::eval_lazy_getters_init_prelude(&mut realm.ctx);
        realm.loaded_shards_mask |= 1;
    }
    if missing == 0 {
        return;
    }
    let lazy_getters_active = (realm.loaded_shards_mask & 1) != 0;
    for shard_id in crate::js_preludes::SHARD_LOAD_ORDER {
        let bit = 1u32 << shard_id;
        if (missing & bit) != 0 {
            let intl_clean_before = if shard_id == 18 {
                let c = realm.ctx.intrinsics().constructors();
                (
                    proto_sig(&c.string().prototype()) == realm.ses_proto_sigs[3],
                    proto_sig(&c.number().prototype()) == realm.ses_proto_sigs[6],
                    proto_sig(&c.date().prototype()) == realm.ses_proto_sigs[12],
                )
            } else {
                (false, false, false)
            };
            if lazy_getters_active {
                if let Some(ref ensure_fn) = realm.ensure_shard_fn {
                    let _ = ensure_fn.call(
                        &boa_engine::JsValue::undefined(),
                        &[boa_engine::JsValue::from(shard_id as i32)],
                        &mut realm.ctx,
                    );
                }
            } else {
                let _ = crate::js_preludes::eval_lazy_prelude_shard(shard_id, &mut realm.ctx);
            }
            realm.loaded_shards_mask |= bit;
            if shard_id == 18 {
                BOA_SES_INTL_SIG_PAIR.with(|p| p.set(None));
                let c = realm.ctx.intrinsics().constructors();
                if intl_clean_before.0 {
                    realm.ses_proto_sigs[3] = proto_sig(&c.string().prototype());
                }
                if intl_clean_before.1 {
                    realm.ses_proto_sigs[6] = proto_sig(&c.number().prototype());
                }
                if intl_clean_before.2 {
                    realm.ses_proto_sigs[12] = proto_sig(&c.date().prototype());
                }
            }
            if shard_id == 8 && realm.init_worker_self_fn.is_none() {
                realm.init_worker_self_fn = realm
                    .ctx
                    .global_object()
                    .get(
                        boa_engine::js_string!("__slice_init_worker_self_env"),
                        &mut realm.ctx,
                    )
                    .ok()
                    .and_then(|v| v.as_object());
            }
        }
    }
}
