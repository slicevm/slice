//! Thread-Local Compartment State, Dirty Subsystem Bitmasks, and Safe `ActiveCasArchiveGuard`.

use super::ChromeTraceEvent;
use crate::state_and_kv::StateCheckpoint;
use compact_str::CompactString;
use rustc_hash::{FxHashMap, FxHashSet};
use slice_sandbox::SandboxViolation;

pub(crate) const DIRTY_EVENT_LOOP: u16 = 1 << 0;
pub(crate) const DIRTY_ALS: u16 = 1 << 1;
pub(crate) const DIRTY_FETCH_MOCK: u16 = 1 << 2;
pub(crate) const DIRTY_WEBSOCKET: u16 = 1 << 3;
pub(crate) const DIRTY_SOCKETS: u16 = 1 << 4;
pub(crate) const DIRTY_WORKERS: u16 = 1 << 5;
pub(crate) const DIRTY_WASM: u16 = 1 << 6;
pub(crate) const DIRTY_PROCESS: u16 = 1 << 7;
pub(crate) const DIRTY_VFS: u16 = 1 << 8;

pub(crate) const EXEC_DIRTY_EXIT: u8 = 1 << 0;
pub(crate) const EXEC_DIRTY_REJECTIONS: u8 = 1 << 1;
pub(crate) const EXEC_DIRTY_UNCAUGHT: u8 = 1 << 2;
pub(crate) const EXEC_DIRTY_CHILD_PROC: u8 = 1 << 3;
pub(crate) const EXEC_DIRTY_ARGV: u8 = 1 << 4;

pub(crate) const REALM_CTL_RESET_MT: i32 = 0;
pub(crate) const REALM_CTL_SET_DET_MODE: i32 = 1;
pub(crate) const REALM_CTL_SYNC_ENV: i32 = 2;
pub(crate) const REALM_CTL_SET_NEXT_TICK: i32 = 3;
pub(crate) const REALM_CTL_RESET_WU: i32 = 4;
pub(crate) const REALM_CTL_DISPATCH_HTTP_BRIDGE: i32 = 5;
pub(crate) const REALM_CTL_RESET_FS_WATCHERS: i32 = 6;

pub(crate) const HOST_GROUP_HTTP_BRIDGE: u8 = 1 << 0;
pub(crate) const HOST_GROUP_SEMVER_FAST: u8 = 1 << 1;
pub(crate) const HOST_GROUP_KV_SANDBOX: u8 = 1 << 2;
pub(crate) const HOST_GROUP_DET: u8 = 1 << 3;

#[derive(Default, Clone)]
pub(crate) struct ActiveBoaHttpReq {
    pub(crate) active: bool,
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) query: String,
    pub(crate) tenant_id: String,
    pub(crate) body: String,
    pub(crate) body_bytes: Vec<u8>,
    pub(crate) has_raw_body_bytes: bool,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) headers_len: usize,
}

pub(crate) struct WarmBoaRealm {
    pub(crate) owner_tenant: CompactString,
    pub(crate) ctx: boa_engine::Context,
    pub(crate) loaded_bundles: FxHashMap<u64, boa_engine::JsObject>,
    pub(crate) loaded_bundle_flags: FxHashMap<u64, u16>,
    pub(crate) loaded_bundle_fused_specs: FxHashMap<u64, u64>,
    pub(crate) compiled_scripts: FxHashMap<u64, boa_engine::Script>,
    pub(crate) ses_check_fn: Option<boa_engine::JsObject>,
    pub(crate) ensure_shard_fn: Option<boa_engine::JsObject>,
    pub(crate) init_worker_self_fn: Option<boa_engine::JsObject>,
    pub(crate) drain_wu_fn: Option<boa_engine::JsObject>,
    pub(crate) ctl_fn: Option<boa_engine::JsObject>,
    pub(crate) ses_proto_sigs: [SesProtoSig; SES_INTRINSIC_COUNT],
    pub(crate) loaded_shards_mask: u32,
    pub(crate) registered_host_groups: u8,
    pub(crate) has_pending_wu: bool,
    pub(crate) virgin_prewarmed: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct BoaFsCaps {
    pub(crate) read_prefixes: Vec<String>,
    pub(crate) write_prefixes: Vec<String>,
}

pub(crate) type CapturedBoaHttpRes = (u16, String, Option<Vec<u8>>, String, Vec<(String, String)>);
pub(crate) const SES_INTRINSIC_COUNT: usize = 29;
pub(crate) type SesProtoSig = (u64, usize, u64);
pub(crate) type SesIntlSigDelta = [(SesProtoSig, SesProtoSig); 3];

pub(crate) struct ActiveCasState {
    pub(crate) archive: slice_cas::MappedCasArchive,
    pub(crate) referrer_stack: Vec<String>,
    pub(crate) module_cache: Vec<(CompactString, boa_engine::JsObject)>,
    pub(crate) resolved_count: usize,
}

/// Stack-scoped RAII guard binding `cas: Option<&MappedCasArchive>` onto `BOA_ACTIVE_CAS`
/// (`O(1)` `Arc`-cloned `MappedCasArchive`, zero raw pointers) for Tier-1
/// `__slice_cas_require(specifier)` resolution and restoring any outer state on normal return,
/// early `?` error, or panic unwind (including hiding parent `.cas` modules when nested
/// `Slice.evalSandbox` runs with `cas: None`).
pub(crate) struct ActiveCasArchiveGuard {
    prev: Option<ActiveCasState>,
    swapped: bool,
}

impl ActiveCasArchiveGuard {
    #[inline]
    pub(crate) fn enter(cas: Option<&slice_cas::MappedCasArchive>) -> Self {
        match cas {
            None => BOA_ACTIVE_CAS.with(|slot| {
                if slot.borrow().is_none() {
                    Self {
                        prev: None,
                        swapped: false,
                    }
                } else {
                    let prev = slot.borrow_mut().take();
                    Self {
                        prev,
                        swapped: true,
                    }
                }
            }),
            Some(archive) => {
                let mut referrer_stack = Vec::with_capacity(4);
                if let Some(entry) = archive.entrypoint_blob() {
                    referrer_stack.push(entry.original_specifier.clone());
                }
                let next = ActiveCasState {
                    archive: archive.clone(),
                    referrer_stack,
                    module_cache: Vec::new(),
                    resolved_count: 0,
                };
                let prev = BOA_ACTIVE_CAS.with(|slot| slot.replace(Some(next)));
                Self {
                    prev,
                    swapped: true,
                }
            }
        }
    }

    #[inline]
    pub(crate) fn finish(&mut self) -> usize {
        if !self.swapped {
            return 0;
        }
        self.swapped = false;
        let prev = self.prev.take();
        BOA_ACTIVE_CAS.with(|slot| {
            let ended = slot.replace(prev);
            ended.map_or(0, |s| s.resolved_count)
        })
    }
}

impl Drop for ActiveCasArchiveGuard {
    #[inline]
    fn drop(&mut self) {
        if self.swapped {
            let prev = self.prev.take();
            BOA_ACTIVE_CAS.with(|slot| {
                let _ = slot.replace(prev);
            });
        }
    }
}

thread_local! {
    pub(crate) static BOA_ACTIVE_CAS: std::cell::RefCell<Option<ActiveCasState>> =
        const { std::cell::RefCell::new(None) };
    pub(crate) static BOA_STDOUT: std::cell::RefCell<Vec<String>> =
        std::cell::RefCell::new(Vec::with_capacity(16));
    pub(crate) static BOA_CONTEXT_POOL: std::cell::RefCell<std::mem::ManuallyDrop<Vec<WarmBoaRealm>>> =
        const { std::cell::RefCell::new(std::mem::ManuallyDrop::new(Vec::new())) };
    pub(crate) static BOA_VFS_STATE: std::cell::RefCell<crate::bindings::fs::VfsState> =
        const {
            std::cell::RefCell::new(crate::bindings::fs::VfsState {
                inodes: Vec::new(),
                free_inodes: Vec::new(),
                paths: FxHashMap::with_hasher(rustc_hash::FxBuildHasher),
                fds: Vec::new(),
                free_fds: Vec::new(),
                open_fd_total: 0,
                total_accounted_bytes: 0,
                has_active_watchers: false,
            })
        };
    pub(crate) static BOA_VFS_OWNER_TENANT: std::cell::RefCell<CompactString> =
        const { std::cell::RefCell::new(CompactString::const_new("")) };
    pub(crate) static BOA_ACTIVE_CAPS: std::cell::RefCell<BoaFsCaps> =
        const {
            std::cell::RefCell::new(BoaFsCaps {
                read_prefixes: Vec::new(),
                write_prefixes: Vec::new(),
            })
        };
    pub(crate) static BOA_VIOLATION: std::cell::RefCell<Option<SandboxViolation>> =
        const { std::cell::RefCell::new(None) };
    pub(crate) static BOA_HTTP_REQ: std::cell::RefCell<ActiveBoaHttpReq> =
        std::cell::RefCell::new(ActiveBoaHttpReq::default());
    pub(crate) static BOA_HTTP_RES: std::cell::RefCell<Option<CapturedBoaHttpRes>> =
        const { std::cell::RefCell::new(None) };
    pub(crate) static BOA_WS_UPGRADED: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
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
    pub(crate) static BOA_EXEC_BINS: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
    pub(crate) static BOA_LAST_CHILD_PROC_VIOLATION: std::cell::RefCell<Option<SandboxViolation>> =
        const { std::cell::RefCell::new(None) };
    pub(crate) static BOA_FETCH_MOCKS: std::cell::RefCell<FxHashMap<String, String>> =
        std::cell::RefCell::new(FxHashMap::default());
    pub(crate) static CHECKPOINT_SLAB: std::cell::RefCell<FxHashMap<u64, StateCheckpoint>> =
        std::cell::RefCell::new(FxHashMap::default());
    pub(crate) static NEXT_CHECKPOINT_ID: std::cell::Cell<u64> =
        const { std::cell::Cell::new(1) };
    pub(crate) static BOA_CHECKPOINT_STATS: std::cell::Cell<(u32, u32, u64)> =
        const { std::cell::Cell::new((0, 0, 0)) };
    pub(crate) static BOA_CHECKPOINT_CAP: std::cell::Cell<(bool, u32)> =
        const { std::cell::Cell::new((true, u32::MAX)) };
    pub(crate) static BOA_ALS_ACTIVE: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
    pub(crate) static BOA_SES_BASELINE_SIGS: std::cell::Cell<Option<[SesProtoSig; SES_INTRINSIC_COUNT]>> =
        const { std::cell::Cell::new(None) };
    pub(crate) static BOA_SES_INTL_SIG_PAIR: std::cell::Cell<Option<SesIntlSigDelta>> =
        const { std::cell::Cell::new(None) };
    pub(crate) static BOA_ALS_SNAP: std::cell::RefCell<boa_engine::JsValue> =
        const { std::cell::RefCell::new(boa_engine::JsValue::undefined()) };
    pub(crate) static BOA_ACTIVE_SOURCE_PTR_LEN: std::cell::Cell<(usize, usize)> =
        const { std::cell::Cell::new((0, 0)) };
    pub(crate) static BOA_ACTIVE_SOURCE_FUSED_SPEC: std::cell::Cell<u64> =
        const { std::cell::Cell::new(0) };
    pub(crate) static BOA_TRACE_ENABLED: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
    pub(crate) static BOA_TRACE_EVENTS: std::cell::RefCell<Vec<ChromeTraceEvent>> =
        const { std::cell::RefCell::new(Vec::new()) };
    pub(crate) static BOA_DIRTY_SUBSYSTEMS: std::cell::Cell<u16> =
        const { std::cell::Cell::new(0) };
    pub(crate) static BOA_EXEC_DIRTY: std::cell::Cell<u8> =
        const { std::cell::Cell::new(0) };
    pub(crate) static BOA_CUSTOM_ARGV: std::cell::RefCell<Option<Vec<String>>> =
        const { std::cell::RefCell::new(None) };
    pub(crate) static BOA_PROCESS_EXIT: std::cell::Cell<Option<i32>> =
        const { std::cell::Cell::new(None) };
    pub(crate) static BOA_LAST_EXIT_CODE: std::cell::Cell<Option<i32>> =
        const { std::cell::Cell::new(None) };
    pub(crate) static BOA_UNHANDLED_REJECTIONS: std::cell::RefCell<Vec<boa_engine::JsObject>> =
        const { std::cell::RefCell::new(Vec::new()) };
    pub(crate) static BOA_UNCAUGHT_ERROR: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

#[inline]
pub(crate) fn mark_exec_dirty(bit: u8) {
    BOA_EXEC_DIRTY.with(|c| c.set(c.get() | bit));
}

#[inline]
pub(crate) fn set_process_exit(code: i32) {
    mark_exec_dirty(EXEC_DIRTY_EXIT);
    BOA_PROCESS_EXIT.with(|e| e.set(Some(code)));
    BOA_LAST_EXIT_CODE.with(|e| e.set(Some(code)));
}

#[inline]
pub(crate) fn set_last_exit_code(code: i32) {
    mark_exec_dirty(EXEC_DIRTY_EXIT);
    BOA_LAST_EXIT_CODE.with(|e| e.set(Some(code)));
}

#[inline]
pub(crate) fn set_uncaught_error(err: String) {
    mark_exec_dirty(EXEC_DIRTY_UNCAUGHT);
    BOA_UNCAUGHT_ERROR.with(|e| *e.borrow_mut() = Some(err));
}

#[inline]
pub(crate) fn set_child_proc_violation(violation: SandboxViolation) {
    mark_exec_dirty(EXEC_DIRTY_CHILD_PROC);
    BOA_LAST_CHILD_PROC_VIOLATION.with(|v| *v.borrow_mut() = Some(violation));
}

#[inline]
pub(crate) fn has_process_exit() -> bool {
    (BOA_EXEC_DIRTY.with(|c| c.get()) & EXEC_DIRTY_EXIT) != 0
        && BOA_PROCESS_EXIT.with(|e| e.get().is_some())
}

#[inline]
pub(crate) fn has_uncaught_error() -> bool {
    (BOA_EXEC_DIRTY.with(|c| c.get()) & EXEC_DIRTY_UNCAUGHT) != 0
        && BOA_UNCAUGHT_ERROR.with(|e| e.borrow().is_some())
}

#[inline]
pub(crate) fn mark_subsystem_dirty(bit: u16) {
    BOA_DIRTY_SUBSYSTEMS.with(|c| c.set(c.get() | bit));
}
