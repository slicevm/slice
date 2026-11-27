//! Compartment Session Setup, Prewarming, Unwind Scrubbing, and Thread-Local Teardown.

use super::{
    BoaSessionFlags, cache_loaded_rc_bundle, ensure_realm_kv_sandbox, ensure_realm_semver_fast,
};
use crate::boa_realm::{
    BOA_ACTIVE_CAPS, BOA_ACTIVE_DEADLINE, BOA_ACTIVE_EXTENSIONS, BOA_ACTIVE_INTERRUPT,
    BOA_ACTIVE_TENANT, BOA_ALLOW_PRIVATE_NETS, BOA_ALLOWED_EXTENSIONS, BOA_CHECKPOINT_CAP,
    BOA_CHECKPOINT_STATS, BOA_CONTEXT_POOL, BOA_CUSTOM_ARGV, BOA_DENIED_CAPABILITIES,
    BOA_DET_STATE, BOA_DIRTY_SUBSYSTEMS, BOA_EGRESS_SECRETS, BOA_ENV_MAP, BOA_EXEC_BINS,
    BOA_EXEC_DIRTY, BOA_FETCH_MOCKS, BOA_FUEL_BUDGET, BOA_HTTP_RES, BOA_KV_MEM_BUDGET,
    BOA_LAST_EXIT_CODE, BOA_LAST_OCAP_VIOLATION, BOA_NET_HOSTS, BOA_PROCESS_EXIT,
    BOA_SES_INTL_SIG_PAIR, BOA_STDOUT, BOA_UNCAUGHT_ERROR, BOA_UNHANDLED_REJECTIONS,
    BOA_VFS_OWNER_TENANT, BOA_VFS_STATE, BOA_VIOLATION, BOA_WAIT_UNTIL_COUNT, BOA_WS_UPGRADED,
    CHECKPOINT_SLAB, DIRTY_FETCH_MOCK, DIRTY_SOCKETS, DIRTY_VFS, DIRTY_WASM, DIRTY_WEBSOCKET,
    DIRTY_WORKERS, EXEC_DIRTY_ARGV, EXEC_DIRTY_CHILD_PROC, EXEC_DIRTY_EXIT, EXEC_DIRTY_REJECTIONS,
    EXEC_DIRTY_UNCAUGHT, REALM_CTL_RESET_FS_WATCHERS, REALM_CTL_RESET_WU, REALM_CTL_SET_DET_MODE,
    REALM_CTL_SET_NEXT_TICK, REALM_CTL_SYNC_ENV, WarmBoaRealm, acquire_warm_boa_realm,
    call_realm_ctl, create_bare_boa_realm, create_warm_boa_realm, ensure_realm_shards,
    ensure_vfs_tenant_affinity, first_ses_proto_sig_mismatch_idx, mark_exec_dirty,
    release_prewarmed_boa_realm, try_acquire_pooled_boa_realm, vfs_owner_namespace,
};
use crate::boacas::{
    BOACAS_FLAG_HAS_KV_SANDBOX, BOACAS_FLAG_HAS_SEMVER_FAST, BOACAS_FLAG_HAS_WORKER,
    BOACAS_FLAG_IS_RC_BUNDLE,
};
use crate::event_loop::clear_event_loop_state;
use crate::isolate::SliceIsolate;
use crate::js_preludes::{detect_needed_shards, needs_lazy_global_getters};
use slice_cas::extract_slice_rc_key;
use slice_sandbox::CapabilityToken;

fn setup_prewarm_caps(caps: &CapabilityToken) -> (bool, bool, bool, bool) {
    let has_fs = !caps.allowed_read_prefixes.is_empty()
        || !caps.allowed_write_prefixes.is_empty()
        || !caps.virtual_mounts.is_empty();
    if has_fs {
        BOA_ACTIVE_CAPS.with(|c| {
            let mut dst = c.borrow_mut();
            dst.read_prefixes.clear();
            dst.read_prefixes
                .extend_from_slice(&caps.allowed_read_prefixes);
            dst.write_prefixes.clear();
            dst.write_prefixes
                .extend_from_slice(&caps.allowed_write_prefixes);
            dst.virtual_mounts.clear();
            dst.virtual_mounts.extend_from_slice(&caps.virtual_mounts);
        });
    }
    let has_net = !caps.allowed_net_hosts.is_empty();
    if has_net {
        BOA_NET_HOSTS.with(|h| *h.borrow_mut() = caps.allowed_net_hosts.clone());
    }
    BOA_ALLOW_PRIVATE_NETS.with(|p| p.set(caps.allow_private_networks));
    let has_env = !caps.allowed_env_keys.is_empty();
    if has_env {
        BOA_ENV_MAP.with(|m| *m.borrow_mut() = caps.allowed_env_keys.clone());
    }
    let has_exec = !caps.allowed_exec_binaries.is_empty();
    if has_exec {
        BOA_EXEC_BINS.with(|b| *b.borrow_mut() = caps.allowed_exec_binaries.clone());
    }
    (has_fs, has_net, has_env, has_exec)
}

fn scrub_after_prewarm(realm: &mut WarmBoaRealm, prewarm_flags: (bool, bool, bool, bool)) {
    let (has_fs, has_net, has_env, has_exec) = prewarm_flags;
    if has_fs {
        BOA_ACTIVE_CAPS.with(|c| {
            let mut dst = c.borrow_mut();
            dst.read_prefixes.clear();
            dst.write_prefixes.clear();
            dst.virtual_mounts.clear();
        });
    }
    if has_net {
        BOA_NET_HOSTS.with(|h| h.borrow_mut().clear());
    }
    BOA_ALLOW_PRIVATE_NETS.with(|p| p.set(false));
    if has_env {
        call_realm_ctl(realm, REALM_CTL_SYNC_ENV, false);
        BOA_ENV_MAP.with(|m| m.borrow_mut().clear());
    }
    if has_exec {
        BOA_EXEC_BINS.with(|b| b.borrow_mut().clear());
    }
    clear_event_loop_state();
    crate::websocket::clear_thread_websocket_state();
    crate::sockets::clear_thread_socket_state();
    crate::workers::clear_thread_worker_state();
    crate::wasm::clear_thread_wasm_state();
    CHECKPOINT_SLAB.with(|s| s.borrow_mut().clear());
    BOA_CHECKPOINT_STATS.with(|s| s.set((0, 0, 0)));
    BOA_DIRTY_SUBSYSTEMS.with(|d| d.set(0));
    BOA_EXEC_DIRTY.with(|c| c.set(0));
    BOA_PROCESS_EXIT.with(|e| e.set(None));
    BOA_LAST_EXIT_CODE.with(|e| e.set(None));
    BOA_UNHANDLED_REJECTIONS.with(|u| u.borrow_mut().clear());
    BOA_UNCAUGHT_ERROR.with(|e| *e.borrow_mut() = None);
    BOA_LAST_OCAP_VIOLATION.with(|v| *v.borrow_mut() = None);
    BOA_DENIED_CAPABILITIES.with(|d| d.borrow_mut().clear());
    BOA_VIOLATION.with(|v| *v.borrow_mut() = None);
    BOA_VFS_STATE.with(|v| v.borrow_mut().clear());
    BOA_VFS_OWNER_TENANT.with(|o| o.borrow_mut().clear());
    if BOA_WAIT_UNTIL_COUNT.with(|c| c.get()) > 0 {
        call_realm_ctl(realm, REALM_CTL_RESET_WU, false);
    }
    realm.has_pending_wu = false;
    BOA_STDOUT.with(|out| out.borrow_mut().clear());
    BOA_HTTP_RES.with(|res| *res.borrow_mut() = None);
    BOA_WS_UPGRADED.with(|w| w.set(false));
    BOA_WAIT_UNTIL_COUNT.with(|c| c.set(0));
    BOA_FUEL_BUDGET.with(|f| f.set((0, u64::MAX)));
    BOA_KV_MEM_BUDGET.with(|m| m.set((0, usize::MAX)));
}

#[inline]
fn scrub_common_dirty_subsystems(dirty: u16) {
    if (dirty & DIRTY_FETCH_MOCK) != 0 {
        BOA_FETCH_MOCKS.with(|m| m.borrow_mut().clear());
    }
    if (dirty & DIRTY_WEBSOCKET) != 0 {
        crate::websocket::clear_thread_websocket_state();
    }
    if (dirty & DIRTY_SOCKETS) != 0 {
        crate::sockets::clear_thread_socket_state();
    }
    if (dirty & DIRTY_WORKERS) != 0 {
        crate::workers::clear_thread_worker_state();
    }
    if (dirty & DIRTY_WASM) != 0 {
        crate::wasm::clear_thread_wasm_state();
    }
}

#[inline(never)]
#[cold]
pub(crate) fn scrub_boa_thread_locals_on_unwind() {
    BOA_STDOUT.with(|out| out.borrow_mut().clear());
    BOA_VIOLATION.with(|v| *v.borrow_mut() = None);
    BOA_SES_INTL_SIG_PAIR.with(|p| p.set(None));
    BOA_ACTIVE_TENANT.with(|t| t.borrow_mut().clear());
    BOA_DET_STATE.with(|s| s.set(None));
    BOA_FUEL_BUDGET.with(|f| f.set((0, u64::MAX)));
    BOA_KV_MEM_BUDGET.with(|m| m.set((0, usize::MAX)));
    BOA_CHECKPOINT_STATS.with(|s| s.set((0, 0, 0)));
    BOA_CHECKPOINT_CAP.with(|c| c.set((true, u32::MAX)));
    BOA_WAIT_UNTIL_COUNT.with(|c| c.set(0));
    BOA_WS_UPGRADED.with(|w| w.set(false));
    BOA_HTTP_RES.with(|res| *res.borrow_mut() = None);
    BOA_ACTIVE_CAPS.with(|c| {
        let mut caps = c.borrow_mut();
        caps.read_prefixes.clear();
        caps.write_prefixes.clear();
    });
    BOA_NET_HOSTS.with(|h| h.borrow_mut().clear());
    BOA_EGRESS_SECRETS.with(|s| s.borrow_mut().clear());
    BOA_ENV_MAP.with(|m| m.borrow_mut().clear());
    BOA_EXEC_BINS.with(|b| b.borrow_mut().clear());
    let exec_dirty = BOA_EXEC_DIRTY.with(|c| c.replace(0));
    if exec_dirty != 0 {
        BOA_PROCESS_EXIT.with(|e| e.set(None));
        BOA_LAST_EXIT_CODE.with(|e| e.set(None));
        BOA_UNHANDLED_REJECTIONS.with(|u| u.borrow_mut().clear());
        BOA_UNCAUGHT_ERROR.with(|e| *e.borrow_mut() = None);
        BOA_LAST_OCAP_VIOLATION.with(|v| *v.borrow_mut() = None);
        BOA_CUSTOM_ARGV.with(|a| *a.borrow_mut() = None);
    }
    let dirty = BOA_DIRTY_SUBSYSTEMS.with(|d| d.replace(0));
    scrub_dirty_subsystems_without_realm(dirty);
    clear_event_loop_state();
}

#[inline(never)]
#[cold]
fn scrub_dirty_subsystems_without_realm(dirty: u16) {
    scrub_common_dirty_subsystems(dirty);
    if (dirty & DIRTY_VFS) != 0 {
        BOA_VFS_STATE.with(|v| {
            let mut vfs = v.borrow_mut();
            let _ = vfs.close_all_open_fds();
            vfs.has_active_watchers = false;
        });
    }
}

pub(crate) struct BoaSessionUnwindGuard {
    pub(crate) armed: bool,
}

impl BoaSessionUnwindGuard {
    #[inline(always)]
    pub(crate) fn new() -> Self {
        Self { armed: true }
    }

    #[inline(always)]
    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for BoaSessionUnwindGuard {
    #[inline]
    fn drop(&mut self) {
        if self.armed {
            scrub_boa_thread_locals_on_unwind();
        }
    }
}

impl SliceIsolate {
    /// Pre-warms `count` full-ES2024+ `boa_engine::Context` realms on the current worker thread
    /// with all native Rust host intrinsics (`__slice_console_log`, `__slice_path_join`, `__slice_fs_*`,
    /// `__slice_crypto_sha256`, `__slice_os_tmpdir`) and the `node:*` module loader already evaluated.
    pub fn prewarm_boa_pool(count: usize) {
        BOA_CONTEXT_POOL.with(|pool| {
            let mut pool_borrow = pool.borrow_mut();
            while pool_borrow.len() < count {
                let mut realm = create_warm_boa_realm();
                realm.virgin_prewarmed = true;
                pool_borrow.push(realm);
            }
        });
    }

    /// Pre-warms a `boa_engine::Context` realm in `BOA_CONTEXT_POOL` under strict zero-authority
    /// (`CapabilityToken::pure_compute()`) containment and pre-populates its Persistent Realm Module
    /// Cache (`globalThis.__slice_rc`) for `bytecode`.
    pub fn prewarm_boa_bundle(bytecode: &[u8]) {
        Self::prewarm_boa_bundle_with_caps(bytecode, &CapabilityToken::pure_compute());
    }

    /// Pre-warms a `boa_engine::Context` realm in `BOA_CONTEXT_POOL` bounded by `caps` so top-level
    /// module initialization obeys the exact same OCap permissions as subsequent executions.
    pub fn prewarm_boa_bundle_with_caps(bytecode: &[u8], caps: &CapabilityToken) {
        if let Some((hdr, precompiled_slice, fallback_js_bytes)) =
            crate::boacas::BoaCasHeader::decode_raw(bytecode)
            && crate::boacas::is_exper_boacas_runtime_enabled()
        {
            if (hdr.feature_flags & BOACAS_FLAG_HAS_WORKER) != 0 {
                crate::workers::prewarm_worker_pool_sync(1);
            }
            if (hdr.feature_flags & BOACAS_FLAG_IS_RC_BUNDLE) != 0 {
                let (mut realm, is_cold_bare_realm) = match try_acquire_pooled_boa_realm() {
                    Some(r) => (r, false),
                    None => (create_bare_boa_realm(), true),
                };
                if !realm.loaded_bundles.contains_key(&hdr.rc_key) {
                    if (hdr.feature_flags & BOACAS_FLAG_HAS_SEMVER_FAST) != 0 {
                        ensure_realm_semver_fast(&mut realm);
                    }
                    if (hdr.feature_flags & BOACAS_FLAG_HAS_KV_SANDBOX) != 0 {
                        ensure_realm_kv_sandbox(&mut realm);
                    }
                    let pw_flags = setup_prewarm_caps(caps);
                    if pw_flags.2 {
                        call_realm_ctl(&mut realm, REALM_CTL_SYNC_ENV, true);
                    }
                    let _ = crate::boacas::eval_boacas_on_realm(
                        &mut realm,
                        &hdr,
                        precompiled_slice,
                        fallback_js_bytes,
                        false,
                        is_cold_bare_realm,
                    );
                    let _ = realm.ctx.run_jobs();
                    scrub_after_prewarm(&mut realm, pw_flags);
                }
                if first_ses_proto_sig_mismatch_idx(&realm).is_none() {
                    release_prewarmed_boa_realm(realm);
                }
                return;
            }
        }

        Self::prewarm_boa_pool(1);
        let js_slice_opt = if bytecode.starts_with(b"SLICEBOA") {
            std::str::from_utf8(&bytecode[8..]).ok()
        } else if let Some((_, _, fallback_js)) = crate::boacas::BoaCasHeader::decode(bytecode) {
            Some(fallback_js)
        } else {
            None
        };
        if let Some(js_code) = js_slice_opt {
            if js_code.contains("Worker") || js_code.contains("worker_threads") {
                crate::workers::prewarm_worker_pool_sync(1);
            }
            if let Some((rc_key_u64, hex_bytes)) = extract_slice_rc_key(js_code) {
                let mut realm = acquire_warm_boa_realm();
                if !realm.loaded_bundles.contains_key(&rc_key_u64) {
                    let (bflags, _, _) = crate::boacas::compute_boacas_metadata(js_code);
                    if (bflags & BOACAS_FLAG_HAS_SEMVER_FAST) != 0 {
                        ensure_realm_semver_fast(&mut realm);
                    }
                    if (bflags & BOACAS_FLAG_HAS_KV_SANDBOX) != 0 {
                        ensure_realm_kv_sandbox(&mut realm);
                    }
                    ensure_realm_shards(
                        &mut realm,
                        detect_needed_shards(js_code, false),
                        needs_lazy_global_getters(js_code),
                    );
                    let pw_flags = setup_prewarm_caps(caps);
                    if pw_flags.2 {
                        call_realm_ctl(&mut realm, REALM_CTL_SYNC_ENV, true);
                    }
                    let _ = realm
                        .ctx
                        .eval(boa_engine::Source::from_bytes(js_code.as_bytes()));
                    cache_loaded_rc_bundle(&mut realm, rc_key_u64, hex_bytes, Some(bflags), None);
                    let _ = realm.ctx.run_jobs();
                    scrub_after_prewarm(&mut realm, pw_flags);
                }
                if first_ses_proto_sig_mismatch_idx(&realm).is_none() {
                    release_prewarmed_boa_realm(realm);
                }
            }
        }
    }

    /// Returns the number of pre-warmed `boa_engine::Context` realms currently available in the thread-local pool.
    #[inline]
    pub fn boa_pool_available() -> usize {
        BOA_CONTEXT_POOL.with(|pool| pool.borrow().len())
    }

    #[inline]
    pub(crate) fn setup_boa_compartment_thread_locals(&self) -> BoaSessionFlags {
        BOA_STDOUT.with(|out| out.borrow_mut().clear());
        BOA_VIOLATION.with(|v| *v.borrow_mut() = None);
        BOA_SES_INTL_SIG_PAIR.with(|p| p.set(None));
        let stale_dirty = BOA_DIRTY_SUBSYSTEMS.with(|d| d.replace(0));
        if stale_dirty != 0 {
            scrub_dirty_subsystems_without_realm(stale_dirty);
        }
        let stale_exec_dirty = BOA_EXEC_DIRTY.with(|c| c.replace(0));
        if stale_exec_dirty != 0 {
            if (stale_exec_dirty & EXEC_DIRTY_EXIT) != 0 {
                BOA_PROCESS_EXIT.with(|e| e.set(None));
                BOA_LAST_EXIT_CODE.with(|e| e.set(None));
            }
            if (stale_exec_dirty & EXEC_DIRTY_REJECTIONS) != 0 {
                BOA_UNHANDLED_REJECTIONS.with(|u| u.borrow_mut().clear());
            }
            if (stale_exec_dirty & EXEC_DIRTY_UNCAUGHT) != 0 {
                BOA_UNCAUGHT_ERROR.with(|e| *e.borrow_mut() = None);
            }
            if (stale_exec_dirty & EXEC_DIRTY_CHILD_PROC) != 0 {
                BOA_LAST_OCAP_VIOLATION.with(|v| *v.borrow_mut() = None);
                BOA_DENIED_CAPABILITIES.with(|d| d.borrow_mut().clear());
            }
            if (stale_exec_dirty & EXEC_DIRTY_ARGV) != 0 {
                BOA_CUSTOM_ARGV.with(|a| *a.borrow_mut() = None);
            }
        }
        if !self.process_argv.is_empty() {
            mark_exec_dirty(EXEC_DIRTY_ARGV);
            BOA_CUSTOM_ARGV.with(|a| *a.borrow_mut() = Some(self.process_argv.clone()));
        }
        ensure_vfs_tenant_affinity(vfs_owner_namespace(&self.compartment.tenant_id));
        BOA_ACTIVE_TENANT.with(|t| {
            let mut b = t.borrow_mut();
            b.clear();
            b.push_str(&self.compartment.tenant_id);
        });
        let baseline_mem_used = self.compartment.memory_used_bytes.min(4096);
        let live_persisted_bytes = crate::state_and_kv::current_vfs_state_bytes().saturating_add(
            crate::state_and_kv::edge_kv_tenant_tree_bytes(&self.compartment.tenant_id),
        );
        let initial_net_used = self
            .compartment
            .memory_used_bytes
            .saturating_sub(baseline_mem_used)
            .max(live_persisted_bytes);
        let net_limit_mem = self
            .compartment
            .memory_limit_bytes
            .saturating_sub(baseline_mem_used);
        BOA_KV_MEM_BUDGET.with(|m| m.set((initial_net_used, net_limit_mem)));
        BOA_CHECKPOINT_STATS.with(|s| {
            s.set((
                self.checkpoints_taken,
                self.restores_performed,
                self.restore_time_ns,
            ));
        });
        let allow_cp = self.compartment.capabilities.allow_checkpoints;
        let max_cp = self
            .compartment
            .capabilities
            .max_checkpoints
            .unwrap_or(u32::MAX);
        BOA_CHECKPOINT_CAP.with(|c| c.set((allow_cp, max_cp)));
        if let Some(seed) = self.deterministic_seed {
            BOA_DET_STATE.with(|s| s.set(Some((seed ^ 0x9E37_79B9_7F4A_7C15, 1_700_000_000_000))));
        } else {
            BOA_DET_STATE.with(|s| s.set(None));
        }

        let has_fs_grants = !self
            .compartment
            .capabilities
            .allowed_read_prefixes
            .is_empty()
            || !self
                .compartment
                .capabilities
                .allowed_write_prefixes
                .is_empty()
            || !self.compartment.capabilities.virtual_mounts.is_empty();
        BOA_ACTIVE_CAPS.with(|c| {
            let mut caps = c.borrow_mut();
            caps.read_prefixes.clear();
            caps.write_prefixes.clear();
            caps.virtual_mounts.clear();
            if has_fs_grants {
                caps.read_prefixes
                    .extend_from_slice(&self.compartment.capabilities.allowed_read_prefixes);
                caps.write_prefixes
                    .extend_from_slice(&self.compartment.capabilities.allowed_write_prefixes);
                caps.virtual_mounts
                    .extend_from_slice(&self.compartment.capabilities.virtual_mounts);
            }
        });
        // Pre-seed every virtual-only mount prefix as an empty opaque VFS directory so
        // guest code can write directly (e.g. `fs.writeFileSync("/artifacts/a.txt", ...)`)
        // without calling `mkdirSync` first and with zero host-disk fall-through.
        if !self.compartment.capabilities.virtual_mounts.is_empty() {
            BOA_VFS_STATE.with(|v| {
                let mut vfs = v.borrow_mut();
                for mount in &self.compartment.capabilities.virtual_mounts {
                    let norm_mount = crate::boa_realm::norm_vfs_path(mount);
                    if !norm_mount.is_empty() && norm_mount != "/" {
                        vfs.ensure_dir_with_ancestors(&norm_mount, true);
                    }
                }
            });
        }
        let has_net_hosts = !self.compartment.capabilities.allowed_net_hosts.is_empty();
        BOA_NET_HOSTS.with(|h| {
            let mut set = h.borrow_mut();
            if !set.is_empty() {
                set.clear();
            }
            if has_net_hosts {
                set.extend(
                    self.compartment
                        .capabilities
                        .allowed_net_hosts
                        .iter()
                        .cloned(),
                );
            }
        });
        BOA_ALLOW_PRIVATE_NETS.with(|p| {
            p.set(self.compartment.capabilities.allow_private_networks);
        });
        let has_egress_secrets = !self.egress_audit_secrets.is_empty();
        BOA_EGRESS_SECRETS.with(|s| {
            let mut vec = s.borrow_mut();
            if !vec.is_empty() {
                vec.clear();
            }
            if has_egress_secrets {
                vec.extend(self.egress_audit_secrets.iter().cloned());
            }
        });
        let has_env_keys = !self.compartment.capabilities.allowed_env_keys.is_empty();
        BOA_ENV_MAP.with(|m| {
            let mut map = m.borrow_mut();
            if !map.is_empty() {
                map.clear();
            }
            if has_env_keys {
                map.extend(
                    self.compartment
                        .capabilities
                        .allowed_env_keys
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone())),
                );
            }
        });
        let has_exec_bins = !self
            .compartment
            .capabilities
            .allowed_exec_binaries
            .is_empty();
        BOA_EXEC_BINS.with(|b| {
            let mut set = b.borrow_mut();
            if !set.is_empty() {
                set.clear();
            }
            if has_exec_bins {
                set.extend(
                    self.compartment
                        .capabilities
                        .allowed_exec_binaries
                        .iter()
                        .cloned(),
                );
            }
        });

        let has_extensions = self.extensions.is_some()
            || !self.compartment.capabilities.allowed_extensions.is_empty();
        if has_extensions {
            BOA_ACTIVE_EXTENSIONS.with(|r| *r.borrow_mut() = self.extensions.clone());
            BOA_ALLOWED_EXTENSIONS.with(|e| {
                let mut vec = e.borrow_mut();
                vec.clear();
                vec.extend_from_slice(&self.compartment.capabilities.allowed_extensions);
            });
        }
        let has_deadline_or_interrupt =
            self.max_wall_clock_ms.is_some() || self.interrupt_flag.is_some();
        if has_deadline_or_interrupt {
            let dl_pair = self.max_wall_clock_ms.map(|ms| {
                (
                    std::time::Instant::now() + std::time::Duration::from_millis(ms),
                    ms,
                )
            });
            BOA_ACTIVE_DEADLINE.with(|d| d.set(dl_pair));
            BOA_ACTIVE_INTERRUPT.with(|i| *i.borrow_mut() = self.interrupt_flag.clone());
        }

        BoaSessionFlags {
            has_fs_grants,
            has_net_hosts,
            has_egress_secrets,
            has_env_keys,
            has_exec_bins,
            has_extensions,
            has_deadline_or_interrupt,
            is_worker: crate::workers::is_worker_thread(),
            initial_mem_used: baseline_mem_used.saturating_add(initial_net_used),
        }
    }

    #[inline]
    pub(crate) fn apply_boa_realm_runtime_limits(&self, realm: &mut WarmBoaRealm) {
        let remaining_fuel = self
            .compartment
            .fuel_limit
            .saturating_sub(self.compartment.fuel_consumed);
        let baseline = self.compartment.memory_used_bytes.min(4096);
        let remaining_heap = self.compartment.memory_limit_bytes.saturating_sub(baseline);
        BOA_FUEL_BUDGET.with(|f| f.set((0, remaining_fuel.max(1))));
        let limits = realm.ctx.runtime_limits_mut();
        limits.set_loop_iteration_limit(remaining_fuel);
        limits.set_heap_bytes_limit(remaining_heap);
        limits.set_wall_clock_limit_ms(self.max_wall_clock_ms);
        limits.set_recursion_limit(512);
        realm.ctx.set_interrupt_flag(self.interrupt_flag.clone());
    }

    #[inline]
    pub(crate) fn teardown_boa_compartment_thread_locals(
        &self,
        realm: &mut WarmBoaRealm,
        flags: BoaSessionFlags,
        has_next_tick: bool,
    ) {
        if self.deterministic_seed.is_some() {
            call_realm_ctl(realm, REALM_CTL_SET_DET_MODE, false);
            BOA_DET_STATE.with(|s| s.set(None));
        }
        if flags.has_fs_grants {
            BOA_ACTIVE_CAPS.with(|c| {
                let mut caps = c.borrow_mut();
                caps.read_prefixes.clear();
                if caps.read_prefixes.capacity() > 64 {
                    caps.read_prefixes.shrink_to(16);
                }
                caps.write_prefixes.clear();
                if caps.write_prefixes.capacity() > 64 {
                    caps.write_prefixes.shrink_to(16);
                }
                caps.virtual_mounts.clear();
                if caps.virtual_mounts.capacity() > 64 {
                    caps.virtual_mounts.shrink_to(16);
                }
            });
        }
        if flags.has_net_hosts {
            BOA_NET_HOSTS.with(|h| {
                let mut set = h.borrow_mut();
                set.clear();
                if set.capacity() > 64 {
                    set.shrink_to(16);
                }
            });
        }
        if flags.has_egress_secrets {
            BOA_EGRESS_SECRETS.with(|s| {
                let mut vec = s.borrow_mut();
                vec.clear();
                if vec.capacity() > 64 {
                    vec.shrink_to(16);
                }
            });
        }
        if flags.has_env_keys {
            call_realm_ctl(realm, REALM_CTL_SYNC_ENV, false);
            BOA_ENV_MAP.with(|m| {
                let mut map = m.borrow_mut();
                map.clear();
                if map.capacity() > 64 {
                    map.shrink_to(16);
                }
            });
        }
        if flags.has_exec_bins {
            BOA_EXEC_BINS.with(|b| {
                let mut set = b.borrow_mut();
                set.clear();
                if set.capacity() > 64 {
                    set.shrink_to(16);
                }
            });
        }
        if flags.has_extensions {
            BOA_ACTIVE_EXTENSIONS.with(|r| *r.borrow_mut() = None);
            BOA_ALLOWED_EXTENSIONS.with(|e| {
                let mut vec = e.borrow_mut();
                vec.clear();
                if vec.capacity() > 64 {
                    vec.shrink_to(16);
                }
            });
        }
        if flags.has_deadline_or_interrupt {
            BOA_ACTIVE_DEADLINE.with(|d| d.set(None));
            BOA_ACTIVE_INTERRUPT.with(|i| *i.borrow_mut() = None);
        }
        if has_next_tick {
            call_realm_ctl(realm, REALM_CTL_SET_NEXT_TICK, false);
        }
        let dirty = BOA_DIRTY_SUBSYSTEMS.with(|d| d.replace(0));
        scrub_common_dirty_subsystems(dirty);
        if (dirty & DIRTY_VFS) != 0 {
            let (freed_fd_bytes, had_watchers) = BOA_VFS_STATE.with(|v| {
                let mut vfs = v.borrow_mut();
                let freed = vfs.close_all_open_fds();
                let hw = std::mem::replace(&mut vfs.has_active_watchers, false);
                (freed, hw)
            });
            if freed_fd_bytes > 0 {
                crate::bindings::helpers::release_isolate_memory(freed_fd_bytes);
            }
            if had_watchers {
                call_realm_ctl(realm, REALM_CTL_RESET_FS_WATCHERS, false);
            }
        }
    }

    #[inline]
    pub(crate) fn harvest_boa_stdout(&mut self) {
        BOA_STDOUT.with(|out| {
            let mut captured = out.borrow_mut();
            if !captured.is_empty() {
                self.stdout.reserve(captured.len());
                for line in captured.drain(..) {
                    if memchr::memchr(b'\n', line.as_bytes()).is_some() {
                        for subline in line.lines() {
                            self.stdout.push(subline.to_string());
                        }
                    } else {
                        self.stdout.push(line);
                    }
                }
            }
        });
    }
}
