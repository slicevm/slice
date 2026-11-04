//! Warm `boa_engine` Realm Pool, Thread-Local Compartment State, and OCap VFS Guards.

use crate::host_bindings::register_slice_native_host_functions;
use crate::js_preludes::ULTRA_CORE_PRELUDE;
use crate::state_and_kv::StateCheckpoint;
use crate::value_and_ast::BytecodeModule;
use rustc_hash::{FxHashMap, FxHashSet};
use slice_sandbox::SandboxViolation;

#[derive(Default)]
pub(crate) struct ActiveBoaHttpReq {
    pub(crate) active: bool,
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) query: String,
    pub(crate) tenant_id: String,
    pub(crate) body: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) headers_len: usize,
}

pub(crate) struct WarmBoaRealm {
    pub(crate) ctx: boa_engine::Context,
    pub(crate) loaded_bundles: FxHashMap<u64, boa_engine::JsObject>,
    pub(crate) ses_check_fn: Option<boa_engine::JsObject>,
    pub(crate) ensure_shard_fn: Option<boa_engine::JsObject>,
    pub(crate) init_worker_self_fn: Option<boa_engine::JsObject>,
    pub(crate) loaded_shards_mask: u16,
}

pub(crate) type ActiveBoaCaps = (String, Vec<String>, Vec<String>);
pub(crate) type CapturedBoaHttpRes = (u16, String, String, Vec<(String, String)>);

thread_local! {
    pub(crate) static SNP_MODULE_CACHE: std::cell::RefCell<FxHashMap<u64, std::sync::Arc<BytecodeModule>>> =
        std::cell::RefCell::new(FxHashMap::default());
    pub(crate) static BOA_STDOUT: std::cell::RefCell<Vec<String>> =
        std::cell::RefCell::new(Vec::with_capacity(16));
    pub(crate) static BOA_CONTEXT_POOL: std::cell::RefCell<std::mem::ManuallyDrop<Vec<WarmBoaRealm>>> =
        const { std::cell::RefCell::new(std::mem::ManuallyDrop::new(Vec::new())) };
    pub(crate) static BOA_VFS_CACHE: std::cell::RefCell<FxHashMap<String, String>> =
        std::cell::RefCell::new(FxHashMap::default());
    pub(crate) static BOA_VFS_DIRS: std::cell::RefCell<FxHashSet<String>> =
        std::cell::RefCell::new(FxHashSet::default());
    pub(crate) static BOA_ACTIVE_CAPS: std::cell::RefCell<Option<ActiveBoaCaps>> =
        const { std::cell::RefCell::new(None) };
    pub(crate) static BOA_VIOLATION: std::cell::RefCell<Option<SandboxViolation>> =
        const { std::cell::RefCell::new(None) };
    pub(crate) static BOA_HTTP_REQ: std::cell::RefCell<ActiveBoaHttpReq> =
        std::cell::RefCell::new(ActiveBoaHttpReq::default());
    pub(crate) static BOA_HTTP_RES: std::cell::RefCell<Option<CapturedBoaHttpRes>> =
        const { std::cell::RefCell::new(None) };
    pub(crate) static BOA_WAIT_UNTIL_COUNT: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
    pub(crate) static BOA_ACTIVE_TENANT: std::cell::RefCell<String> =
        std::cell::RefCell::new(String::with_capacity(32));
    pub(crate) static BOA_KV_MEM_BUDGET: std::cell::Cell<(usize, usize)> =
        const { std::cell::Cell::new((0, usize::MAX)) };
    pub(crate) static BOA_FUEL_BUDGET: std::cell::Cell<(u64, u64)> =
        const { std::cell::Cell::new((0, u64::MAX)) };
    pub(crate) static BOA_DET_STATE: std::cell::Cell<Option<(u64, u64)>> =
        const { std::cell::Cell::new(None) };
    pub(crate) static BOA_NET_HOSTS: std::cell::RefCell<FxHashSet<String>> =
        std::cell::RefCell::new(FxHashSet::default());
    pub(crate) static BOA_EGRESS_SECRETS: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
    pub(crate) static BOA_ENV_MAP: std::cell::RefCell<FxHashMap<String, String>> =
        std::cell::RefCell::new(FxHashMap::default());
    pub(crate) static BOA_FETCH_MOCKS: std::cell::RefCell<FxHashMap<String, String>> =
        std::cell::RefCell::new(FxHashMap::default());
    pub(crate) static CHECKPOINT_SLAB: std::cell::RefCell<FxHashMap<u64, StateCheckpoint>> =
        std::cell::RefCell::new(FxHashMap::default());
    pub(crate) static NEXT_CHECKPOINT_ID: std::cell::Cell<u64> =
        const { std::cell::Cell::new(1) };
}

#[inline]
pub(crate) fn norm_vfs_path(p: &str) -> String {
    let mut s = if p.as_bytes().contains(&b'\\') {
        p.replace('\\', "/")
    } else {
        p.to_string()
    };
    while s.len() > 1 && s.ends_with('/') {
        s.pop();
    }
    s
}

#[cold]
#[inline(never)]
pub(crate) fn record_boa_fs_violation(tenant_id: &str, path: &str) -> bool {
    BOA_VIOLATION.with(|v| {
        *v.borrow_mut() = Some(SandboxViolation::FsReadDenied {
            tenant_id: tenant_id.to_string(),
            path: path.to_string(),
        });
    });
    false
}

#[inline]
pub(crate) fn check_boa_fs_read_ocap(path: &str) -> bool {
    if path.contains("..") {
        let tenant = BOA_ACTIVE_CAPS
            .with(|c| c.borrow().as_ref().map(|t| t.0.clone()))
            .unwrap_or_else(|| "boa-tenant".to_string());
        return record_boa_fs_violation(&tenant, path);
    }
    BOA_ACTIVE_CAPS.with(|c| {
        if let Some((tenant_id, read_prefixes, _)) = &*c.borrow() {
            if !read_prefixes.is_empty()
                && !read_prefixes.iter().any(|prefix| path.starts_with(prefix))
            {
                return record_boa_fs_violation(tenant_id, path);
            }
        }
        true
    })
}

#[inline]
pub(crate) fn check_boa_fs_write_ocap(path: &str) -> bool {
    if path.contains("..") {
        let tenant = BOA_ACTIVE_CAPS
            .with(|c| c.borrow().as_ref().map(|t| t.0.clone()))
            .unwrap_or_else(|| "boa-tenant".to_string());
        return record_boa_fs_violation(&tenant, path);
    }
    BOA_ACTIVE_CAPS.with(|c| {
        if let Some((tenant_id, _, write_prefixes)) = &*c.borrow() {
            if !write_prefixes.is_empty()
                && !write_prefixes.iter().any(|prefix| path.starts_with(prefix))
            {
                return record_boa_fs_violation(tenant_id, path);
            }
        }
        true
    })
}

pub(crate) fn acquire_warm_boa_realm() -> WarmBoaRealm {
    BOA_CONTEXT_POOL
        .with(|pool| pool.borrow_mut().pop())
        .unwrap_or_else(create_warm_boa_realm)
}

pub(crate) fn release_warm_boa_realm(realm: WarmBoaRealm) {
    BOA_CONTEXT_POOL.with(|pool| {
        let mut p = pool.borrow_mut();
        if p.len() < 8 {
            p.push(realm);
        }
    });
}

/// Creates a warm `boa_engine::Context` initialized with Slice's native host functions,
/// WinterTC /WHATWG globals (`console.*`, `fetch`, `URL`, `Headers`, `Request`, `Response`,
/// `ReadableStream`, `WritableStream`, `TransformStream`, `WebCrypto`, `Buffer`, `structuredClone`),
/// and microtask/macrotask schedulers (`process.nextTick`, `queueMicrotask`, `setImmediate`, `setTimeout`).
pub(crate) fn create_warm_boa_realm() -> WarmBoaRealm {
    let mut ctx = boa_engine::Context::default();
    register_slice_native_host_functions(&mut ctx);

    let _ = ctx.eval(boa_engine::Source::from_bytes(ULTRA_CORE_PRELUDE));
    let global = ctx.global_object();
    let ses_check_fn = global
        .get(boa_engine::js_string!("__slice_check_ses"), &mut ctx)
        .ok()
        .and_then(|v| v.as_object());
    let ensure_shard_fn = global
        .get(boa_engine::js_string!("__slice_ensure_shard"), &mut ctx)
        .ok()
        .and_then(|v| v.as_object());
    WarmBoaRealm {
        ctx,
        loaded_bundles: FxHashMap::default(),
        ses_check_fn,
        ensure_shard_fn,
        init_worker_self_fn: None,
        loaded_shards_mask: 0,
    }
}

#[inline]
pub(crate) fn ensure_realm_shards(
    realm: &mut WarmBoaRealm,
    needed_mask: u16,
    init_lazy_getters: bool,
) {
    let missing = needed_mask & !realm.loaded_shards_mask;
    if init_lazy_getters && (realm.loaded_shards_mask & 1) == 0 {
        let _ = realm
            .ctx
            .eval(boa_engine::Source::from_bytes(crate::js_preludes::LAZY_GETTERS_INIT_PRELUDE));
        realm.loaded_shards_mask |= 1;
    }
    if missing == 0 {
        return;
    }
    let lazy_getters_active = (realm.loaded_shards_mask & 1) != 0;
    for shard_id in [2u16, 1, 10, 3, 4, 5, 6, 7, 8, 9] {
        let bit = 1u16 << shard_id;
        if (missing & bit) != 0 {
            if lazy_getters_active {
                if let Some(ref ensure_fn) = realm.ensure_shard_fn {
                    let _ = ensure_fn.call(
                        &boa_engine::JsValue::undefined(),
                        &[boa_engine::JsValue::from(shard_id as i32)],
                        &mut realm.ctx,
                    );
                }
            } else {
                let _ = crate::js_preludes::eval_lazy_prelude_shard(
                    shard_id as u32,
                    &mut realm.ctx,
                );
            }
            realm.loaded_shards_mask |= bit;
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
