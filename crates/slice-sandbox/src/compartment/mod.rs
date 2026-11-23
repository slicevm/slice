//! Object Capabilities (OCap), SES Intrinsic Lockdown, and `TenantCompartment` Isolation.

pub mod pkey;

pub use pkey::{HardwareProtectionDomain, PkruDomainGuard, current_thread_pkru};

use crate::net::{
    canonicalize_net_grant_entry, is_cloud_metadata_clean_host, is_private_or_loopback_clean_host,
    parse_net_connect_target, parse_net_grant_parts,
};
use crate::vfs::{
    exec_binary_matches, is_rejected_vfs_candidate, normalize_vfs_path_cow,
    vfs_normalized_candidate_matches_prefix, vfs_prefix_matches,
};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum SandboxViolation {
    #[error(
        "OCap Violation: Compartment '{tenant_id}' lacks filesystem read authority for path '{path}'"
    )]
    FsReadDenied { tenant_id: String, path: String },

    #[error(
        "OCap Violation: Compartment '{tenant_id}' lacks filesystem write authority for path '{path}'"
    )]
    FsWriteDenied { tenant_id: String, path: String },

    #[error(
        "OCap Violation: Compartment '{tenant_id}' lacks outbound network authority for host '{host}'"
    )]
    NetAccessDenied { tenant_id: String, host: String },

    #[error(
        "OCap Violation: Compartment '{tenant_id}' lacks environment variable authority for key '{key}'"
    )]
    EnvAccessDenied { tenant_id: String, key: String },

    #[error(
        "[Slice Sandbox Violation] Subprocess execution denied: Compartment '{tenant_id}' lacks exec authority for binary '{binary}'"
    )]
    ExecDenied { tenant_id: String, binary: String },

    #[error(
        "SES Lockdown Violation: Compartment '{tenant_id}' attempted to mutate frozen intrinsic '{intrinsic}'"
    )]
    IntrinsicMutationBlocked {
        tenant_id: String,
        intrinsic: String,
    },

    #[error(
        "Resource Cage Exhausted: Compartment '{tenant_id}' exceeded deterministic CPU instruction fuel limit ({limit} ops)"
    )]
    CpuFuelExhausted { tenant_id: String, limit: u64 },

    #[error(
        "Resource Cage Exhausted: Compartment '{tenant_id}' exceeded memory ceiling ({requested} > {limit} bytes)"
    )]
    MemoryCeilingExceeded {
        tenant_id: String,
        requested: usize,
        limit: usize,
    },

    #[error(
        "Hardware PKEY Trap: Compartment '{tenant_id}' attempted unauthorized cross-domain memory access (PKEY 0x{pkey:x})"
    )]
    HardwareMemoryDomainTrap { tenant_id: String, pkey: u32 },

    #[error(
        "OCap Violation (CapabilityDenied: checkpoints): Compartment '{tenant_id}' checkpoint denied: {reason}"
    )]
    CheckpointDenied { tenant_id: String, reason: String },

    #[error(
        "Egress DLP Violation (EGRESS_SECRET_LEAK_BLOCKED): Compartment '{tenant_id}' blocked from exfiltrating a registered audit secret"
    )]
    EgressSecretLeakBlocked { tenant_id: String },
}

fn default_allow_checkpoints() -> bool {
    true
}

/// Attenuated Capability Token granted explicitly to a TC39/SES Compartment.
/// By default, a new Compartment has ZERO ambient authority.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityToken {
    pub allowed_read_prefixes: Vec<String>,
    pub allowed_write_prefixes: Vec<String>,
    pub allowed_net_hosts: FxHashSet<String>,
    pub allowed_env_keys: FxHashMap<String, String>,
    #[serde(default)]
    pub allowed_exec_binaries: Vec<String>,
    #[serde(default = "default_allow_checkpoints")]
    pub allow_checkpoints: bool,
    #[serde(default)]
    pub max_checkpoints: Option<u32>,
}

impl Default for CapabilityToken {
    #[inline]
    fn default() -> Self {
        Self {
            allowed_read_prefixes: Vec::new(),
            allowed_write_prefixes: Vec::new(),
            allowed_net_hosts: FxHashSet::default(),
            allowed_env_keys: FxHashMap::default(),
            allowed_exec_binaries: Vec::new(),
            allow_checkpoints: true,
            max_checkpoints: None,
        }
    }
}

/// Monotonic capability narrowing request passed to [`CapabilityToken::attenuate`].
///
/// Every `Option<&[String]>` field uses `None` to mean "inherit parent grants unchanged"
/// and `Some(slice)` to mean "strictly intersect against `slice`" (`parent & requested`,
/// where `Some(&[])` yields an empty grant set / deny-all).
#[derive(Debug, Clone, Default)]
pub struct CapabilityAttenuationRequest<'a> {
    pub allow_read: Option<&'a [String]>,
    pub allow_write: Option<&'a [String]>,
    pub allow_net: Option<&'a [String]>,
    pub allow_exec: Option<&'a [String]>,
    pub allow_env: Option<&'a [String]>,
    pub allow_checkpoints: Option<bool>,
    pub max_checkpoints: Option<u32>,
    pub consumed_checkpoints: u32,
}

impl CapabilityToken {
    /// Pure compute compartment with zero I/O or environment authority.
    #[inline]
    pub fn pure_compute() -> Self {
        Self::default()
    }

    #[inline]
    pub fn push_read_prefix(&mut self, prefix: &str) {
        let trimmed = prefix.trim();
        if !trimmed.is_empty() {
            let canon = if trimmed == "*" || trimmed == "/" {
                trimmed.to_string()
            } else {
                crate::vfs::normalize_vfs_path_lexical(trimmed)
            };
            if !self.allowed_read_prefixes.contains(&canon) {
                self.allowed_read_prefixes.push(canon);
            }
        }
    }

    #[inline]
    pub fn push_write_prefix(&mut self, prefix: &str) {
        let trimmed = prefix.trim();
        if !trimmed.is_empty() {
            let canon = if trimmed == "*" || trimmed == "/" {
                trimmed.to_string()
            } else {
                crate::vfs::normalize_vfs_path_lexical(trimmed)
            };
            if !self.allowed_write_prefixes.contains(&canon) {
                self.allowed_write_prefixes.push(canon);
            }
        }
    }

    #[inline]
    pub fn insert_net_host(&mut self, host: &str) {
        if let Some(canon) = canonicalize_net_grant_entry(host) {
            self.allowed_net_hosts.insert(canon);
        }
    }

    pub fn with_read_prefix(mut self, prefix: impl Into<String>) -> Self {
        let raw = prefix.into();
        self.push_read_prefix(&raw);
        self
    }

    pub fn with_write_prefix(mut self, prefix: impl Into<String>) -> Self {
        let raw = prefix.into();
        self.push_write_prefix(&raw);
        self
    }

    pub fn with_net_host(mut self, host: impl Into<String>) -> Self {
        let raw = host.into();
        self.insert_net_host(&raw);
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.allowed_env_keys.insert(key.into(), val.into());
        self
    }

    pub fn with_exec_binary(mut self, binary: impl Into<String>) -> Self {
        let raw = binary.into();
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let owned = trimmed.to_string();
            if !self.allowed_exec_binaries.contains(&owned) {
                self.allowed_exec_binaries.push(owned);
            }
        }
        self
    }

    #[inline]
    pub fn set_checkpoints(&mut self, enabled: bool) {
        self.allow_checkpoints = enabled;
        if !enabled {
            self.max_checkpoints = Some(0);
        } else if self.max_checkpoints == Some(0) {
            self.max_checkpoints = None;
        }
    }

    #[inline]
    pub fn set_max_checkpoints(&mut self, max: Option<u32>) {
        self.max_checkpoints = max;
        if max == Some(0) {
            self.allow_checkpoints = false;
        }
    }

    #[inline]
    pub fn with_checkpoints(mut self, enabled: bool) -> Self {
        self.set_checkpoints(enabled);
        self
    }

    #[inline]
    pub fn with_max_checkpoints(mut self, max: Option<u32>) -> Self {
        self.set_max_checkpoints(max);
        self
    }

    /// Produces a monotonically attenuated child [`CapabilityToken`] (`child <= self`) across
    /// `FsRead`, `FsWrite`, `NetCap`, `ExecCap`, `EnvCap`, and `Checkpoints`.
    ///
    /// Security invariants:
    /// 1. **Strict Port-Scope Lattice**: A port-scoped parent grant (`"api.internal:443"`) NEVER
    ///    authorizes a bare all-ports child grant (`"api.internal"`). A bare child grant is only
    ///    admitted if the parent itself holds bare `"api.internal"`, or holds `"*"` AND the host
    ///    is a public non-SSRF target (`!is_private_or_loopback_clean_host`).
    /// 2. **Wildcard SSRF Containment**: Wildcard `"*"` in the parent never materializes an
    ///    explicit grant for loopback, RFC1918, link-local, non-canonical numeric IPv4, or cloud
    ///    metadata endpoints (`"127.0.0.1:6379"`, `"::1"`, `"169.254.169.254"`).
    /// 3. **VFS Traversal & Prefix Containment**: Rejects `is_rejected_vfs_candidate` (`..`, NUL,
    ///    Win32 devices/UNC) and requires every requested prefix to match an existing parent prefix.
    /// 4. **Host Env Value Integrity**: `allow_env` filters by key membership only and copies the
    ///    parent's exact value, preventing untrusted callers from spoofing `KEY=evil`.
    pub fn attenuate(&self, req: &CapabilityAttenuationRequest<'_>) -> Self {
        let mut child = Self::pure_compute();

        match req.allow_read {
            Some(requested) => {
                for r in requested {
                    if !is_rejected_vfs_candidate(r)
                        && self
                            .allowed_read_prefixes
                            .iter()
                            .any(|p| vfs_prefix_matches(r, p))
                    {
                        child.push_read_prefix(r);
                    }
                }
            }
            None => {
                for p in &self.allowed_read_prefixes {
                    child.push_read_prefix(p);
                }
            }
        }

        match req.allow_write {
            Some(requested) => {
                for w in requested {
                    if !is_rejected_vfs_candidate(w)
                        && self
                            .allowed_write_prefixes
                            .iter()
                            .any(|p| vfs_prefix_matches(w, p))
                    {
                        child.push_write_prefix(w);
                    }
                }
            }
            None => {
                for p in &self.allowed_write_prefixes {
                    child.push_write_prefix(p);
                }
            }
        }

        let mut norm_parent_nets =
            FxHashSet::with_capacity_and_hasher(self.allowed_net_hosts.len(), Default::default());
        for raw_parent in &self.allowed_net_hosts {
            if let Some(canon) = canonicalize_net_grant_entry(raw_parent) {
                norm_parent_nets.insert(canon);
            }
        }

        match req.allow_net {
            Some(requested) => {
                let parent_wildcard = norm_parent_nets.contains("*");
                for raw_req in requested {
                    let Some((canon_host, is_ipv6, port_opt)) = parse_net_grant_parts(raw_req)
                    else {
                        continue;
                    };
                    if canon_host == "*" {
                        if parent_wildcard {
                            child.allowed_net_hosts.insert("*".to_string());
                        }
                        continue;
                    }
                    if is_cloud_metadata_clean_host(&canon_host) {
                        continue;
                    }
                    let parent_has_bare = norm_parent_nets.contains(&canon_host);
                    let wildcard_allows_public =
                        parent_wildcard && !is_private_or_loopback_clean_host(&canon_host);
                    match port_opt {
                        None => {
                            if parent_has_bare || wildcard_allows_public {
                                child.allowed_net_hosts.insert(canon_host);
                            }
                        }
                        Some(port) => {
                            let formatted = if is_ipv6 {
                                format!("[{}]:{}", canon_host, port)
                            } else {
                                format!("{}:{}", canon_host, port)
                            };
                            if norm_parent_nets.contains(&formatted)
                                || parent_has_bare
                                || wildcard_allows_public
                            {
                                child.allowed_net_hosts.insert(formatted);
                            }
                        }
                    }
                }
            }
            None => {
                child.allowed_net_hosts = norm_parent_nets;
            }
        }

        match req.allow_exec {
            Some(requested) => {
                let parent_wildcard = self.allowed_exec_binaries.iter().any(|a| a.trim() == "*");
                for req_bin in requested {
                    let trimmed = req_bin.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if trimmed == "*" {
                        if parent_wildcard {
                            child = child.with_exec_binary("*");
                        }
                    } else if self
                        .allowed_exec_binaries
                        .iter()
                        .any(|allowed| exec_binary_matches(trimmed, allowed))
                    {
                        child = child.with_exec_binary(trimmed);
                    }
                }
            }
            None => {
                for eb in &self.allowed_exec_binaries {
                    child = child.with_exec_binary(eb.clone());
                }
            }
        }

        match req.allow_env {
            Some(requested) => {
                for k in requested {
                    let kt = k.trim();
                    if let Some(val) = self.allowed_env_keys.get(kt) {
                        child.allowed_env_keys.insert(kt.to_string(), val.clone());
                    }
                }
            }
            None => {
                child.allowed_env_keys = self.allowed_env_keys.clone();
            }
        }

        let parent_rem_cp = self
            .max_checkpoints
            .map(|m| m.saturating_sub(req.consumed_checkpoints));
        let child_allow_cp = self.allow_checkpoints
            && req.allow_checkpoints.unwrap_or(true)
            && req.max_checkpoints != Some(0)
            && parent_rem_cp != Some(0);
        let child_max_cp = if !child_allow_cp {
            Some(0)
        } else {
            match (parent_rem_cp, req.max_checkpoints) {
                (Some(p), Some(c)) => Some(p.min(c)),
                (Some(p), None) => Some(p),
                (None, Some(c)) => Some(c),
                (None, None) => None,
            }
        };
        child.allow_checkpoints = child_allow_cp;
        child.max_checkpoints = child_max_cp;

        child
    }
}

/// Zero-allocation static check for SES frozen intrinsics (covering intrinsic prototypes,
/// intrinsic constructors, singleton namespaces, and optional `globalThis.` prefixes).
#[inline(always)]
pub fn is_frozen_intrinsic(target: &str) -> bool {
    let clean = target.strip_prefix("globalThis.").unwrap_or(target);
    matches!(
        clean,
        "Object"
            | "Object.prototype"
            | "Array"
            | "Array.prototype"
            | "Function"
            | "Function.prototype"
            | "String"
            | "String.prototype"
            | "Number"
            | "Number.prototype"
            | "Boolean"
            | "Boolean.prototype"
            | "RegExp"
            | "RegExp.prototype"
            | "Error"
            | "Error.prototype"
            | "Promise"
            | "Promise.prototype"
            | "Map"
            | "Map.prototype"
            | "Set"
            | "Set.prototype"
            | "Date"
            | "Date.prototype"
            | "Uint8Array"
            | "Uint8Array.prototype"
            | "Reflect"
            | "JSON"
            | "Math"
    )
}

#[inline]
fn contains_host_and_port(allowed: &FxHashSet<String>, clean_host: &str, port: u16) -> bool {
    let mut buf = [0u8; 280];
    let is_ipv6 = clean_host.contains(':');
    let mut pos = 0usize;
    if is_ipv6 {
        if clean_host.len() + 8 > buf.len() {
            return allowed.contains(&format!("[{}]:{}", clean_host, port));
        }
        buf[pos] = b'[';
        pos += 1;
        buf[pos..pos + clean_host.len()].copy_from_slice(clean_host.as_bytes());
        pos += clean_host.len();
        buf[pos] = b']';
        pos += 1;
    } else {
        if clean_host.len() + 6 > buf.len() {
            return allowed.contains(&format!("{}:{}", clean_host, port));
        }
        buf[pos..pos + clean_host.len()].copy_from_slice(clean_host.as_bytes());
        pos += clean_host.len();
    }
    buf[pos] = b':';
    pos += 1;

    let mut p = port;
    let mut digits = [0u8; 5];
    let mut d_len = 0usize;
    if p == 0 {
        digits[0] = b'0';
        d_len = 1;
    } else {
        while p > 0 {
            digits[d_len] = b'0' + (p % 10) as u8;
            p /= 10;
            d_len += 1;
        }
    }
    for i in (0..d_len).rev() {
        buf[pos] = digits[i];
        pos += 1;
    }
    if let Ok(s) = core::str::from_utf8(&buf[..pos]) {
        allowed.contains(s)
    } else {
        false
    }
}

/// Isolated MicroVM-Grade Tenant Compartment.
///
/// Phase 9A Cache-Line Packing (`#[repr(C)]`):
/// Hot scalar fields checked on every instruction (`fuel_consumed`, `fuel_limit`,
/// `memory_used_bytes`, `memory_limit_bytes`, `pkey_domain`, `ses_locked_down`)
/// are packed into the first 48 bytes (single 64-byte L1 cache line), while cold
/// `tenant_id` and `capabilities` sit on the second cache line.
#[repr(C)]
#[derive(Debug, Clone)]
pub struct TenantCompartment {
    // --- Hot L1 Cache Line 0 (48 bytes) ---
    pub fuel_consumed: u64,
    pub fuel_limit: u64,
    pub memory_used_bytes: usize,
    pub memory_limit_bytes: usize,
    pub pkey_domain: HardwareProtectionDomain,
    pub ses_locked_down: bool,
    // --- Cold Cache Line 1 ---
    pub tenant_id: String,
    pub capabilities: CapabilityToken,
}

impl TenantCompartment {
    #[inline]
    pub fn new(
        tenant_id: impl Into<String>,
        slot_id: u32,
        capabilities: CapabilityToken,
        fuel_limit: u64,
        memory_limit_bytes: usize,
    ) -> Self {
        Self {
            fuel_consumed: 0,
            fuel_limit,
            memory_used_bytes: 4096, // 4 KB base compartment header frame
            memory_limit_bytes,
            pkey_domain: HardwareProtectionDomain::allocate(slot_id),
            ses_locked_down: true,
            tenant_id: tenant_id.into(),
            capabilities,
        }
    }

    #[cold]
    #[inline(never)]
    fn trap_fuel_exhausted(&self) -> SandboxViolation {
        SandboxViolation::CpuFuelExhausted {
            tenant_id: self.tenant_id.clone(),
            limit: self.fuel_limit,
        }
    }

    #[cold]
    #[inline(never)]
    fn trap_memory_exceeded(&self, requested: usize) -> SandboxViolation {
        SandboxViolation::MemoryCeilingExceeded {
            tenant_id: self.tenant_id.clone(),
            requested,
            limit: self.memory_limit_bytes,
        }
    }

    #[cold]
    #[inline(never)]
    fn trap_intrinsic_mutation(&self, target_object: &str) -> SandboxViolation {
        SandboxViolation::IntrinsicMutationBlocked {
            tenant_id: self.tenant_id.clone(),
            intrinsic: target_object.to_string(),
        }
    }

    #[cold]
    #[inline(never)]
    fn trap_fs_read(&self, path: &str) -> SandboxViolation {
        SandboxViolation::FsReadDenied {
            tenant_id: self.tenant_id.clone(),
            path: path.to_string(),
        }
    }

    #[cold]
    #[inline(never)]
    fn trap_fs_write(&self, path: &str) -> SandboxViolation {
        SandboxViolation::FsWriteDenied {
            tenant_id: self.tenant_id.clone(),
            path: path.to_string(),
        }
    }

    #[cold]
    #[inline(never)]
    fn trap_net_access(&self, host: &str) -> SandboxViolation {
        SandboxViolation::NetAccessDenied {
            tenant_id: self.tenant_id.clone(),
            host: host.to_string(),
        }
    }

    #[cold]
    #[inline(never)]
    fn trap_env_access(&self, key: &str) -> SandboxViolation {
        SandboxViolation::EnvAccessDenied {
            tenant_id: self.tenant_id.clone(),
            key: key.to_string(),
        }
    }

    #[cold]
    #[inline(never)]
    fn trap_exec_denied(&self, binary: &str) -> SandboxViolation {
        SandboxViolation::ExecDenied {
            tenant_id: self.tenant_id.clone(),
            binary: binary.to_string(),
        }
    }

    /// Consumes `ops` deterministic instruction fuel units, trapping immediately on overflow or ceiling breach.
    #[inline(always)]
    pub fn consume_fuel(&mut self, ops: u64) -> Result<(), SandboxViolation> {
        let Some(next) = self.fuel_consumed.checked_add(ops) else {
            self.fuel_consumed = self.fuel_limit;
            return Err(self.trap_fuel_exhausted());
        };
        if next > self.fuel_limit {
            self.fuel_consumed = self.fuel_limit;
            return Err(self.trap_fuel_exhausted());
        }
        self.fuel_consumed = next;
        Ok(())
    }

    /// Allocates bytes inside the tenant's isolated heap cage, trapping on overflow or ceiling breach.
    #[inline(always)]
    pub fn alloc_memory(&mut self, bytes: usize) -> Result<(), SandboxViolation> {
        let Some(next) = self.memory_used_bytes.checked_add(bytes) else {
            return Err(self.trap_memory_exceeded(usize::MAX));
        };
        if next > self.memory_limit_bytes {
            return Err(self.trap_memory_exceeded(next));
        }
        self.memory_used_bytes = next;
        Ok(())
    }

    /// Verifies whether this compartment is permitted to take another state checkpoint.
    #[inline]
    pub fn verify_checkpoint(&self, current_taken: u32) -> Result<(), SandboxViolation> {
        if !self.capabilities.allow_checkpoints {
            return Err(SandboxViolation::CheckpointDenied {
                tenant_id: self.tenant_id.clone(),
                reason: "guest checkpoints are disabled for this compartment".to_string(),
            });
        }
        if let Some(max) = self.capabilities.max_checkpoints
            && current_taken >= max
        {
            return Err(SandboxViolation::CheckpointDenied {
                tenant_id: self.tenant_id.clone(),
                reason: format!(
                    "exceeded max_checkpoints limit ({} > {})",
                    current_taken.saturating_add(1),
                    max
                ),
            });
        }
        Ok(())
    }

    /// Verifies SES `lockdown()` protection before any property mutation on a root object.
    #[inline(always)]
    pub fn verify_property_mutation(&self, target_object: &str) -> Result<(), SandboxViolation> {
        if self.ses_locked_down && is_frozen_intrinsic(target_object) {
            return Err(self.trap_intrinsic_mutation(target_object));
        }
        Ok(())
    }

    /// Verifies attenuated VFS read capability for `path` using lexical path normalization
    /// and strict directory-boundary matching.
    #[inline]
    pub fn verify_fs_read(&self, path: &str) -> Result<(), SandboxViolation> {
        if self.capabilities.allowed_read_prefixes.is_empty() || is_rejected_vfs_candidate(path) {
            return Err(self.trap_fs_read(path));
        }
        let norm_cand = normalize_vfs_path_cow(path);
        for prefix in &self.capabilities.allowed_read_prefixes {
            if vfs_normalized_candidate_matches_prefix(&norm_cand, prefix) {
                return Ok(());
            }
        }
        Err(self.trap_fs_read(path))
    }

    /// Verifies attenuated VFS write capability for `path` using lexical path normalization
    /// and strict directory-boundary matching.
    #[inline]
    pub fn verify_fs_write(&self, path: &str) -> Result<(), SandboxViolation> {
        if self.capabilities.allowed_write_prefixes.is_empty() || is_rejected_vfs_candidate(path) {
            return Err(self.trap_fs_write(path));
        }
        let norm_cand = normalize_vfs_path_cow(path);
        for prefix in &self.capabilities.allowed_write_prefixes {
            if vfs_normalized_candidate_matches_prefix(&norm_cand, prefix) {
                return Ok(());
            }
        }
        Err(self.trap_fs_write(path))
    }

    /// Verifies attenuated outbound network capability for `host`, enforcing the SSRF shield
    /// against cloud instance metadata endpoints (`169.254.169.254`, `metadata.google.internal`, etc.)
    /// and blocking private/loopback CIDRs under wildcard `allow_net = ["*"]`.
    ///
    /// Security (`SBX-SEC-02`) & Performance (`SBX-PERF-01`):
    /// - Enforces strict port matching when `host` includes an explicit `:port` (preventing
    ///   an allowlisted `host:443` from granting access to `host:6379` or `host:22`).
    /// - Performs zero heap allocations on the hot path by borrowing lowercase host slices
    ///   and formatting `host:port` candidates onto a stack buffer.
    #[inline]
    pub fn verify_net_connect(&self, host: &str) -> Result<(), SandboxViolation> {
        if self.capabilities.allowed_net_hosts.is_empty() {
            return Err(self.trap_net_access(host));
        }
        let Some(parsed) = parse_net_connect_target(host) else {
            return Err(self.trap_net_access(host));
        };
        let clean_host = parsed.clean_host.as_ref();
        if is_cloud_metadata_clean_host(clean_host) {
            return Err(self.trap_net_access(host));
        }
        if self.capabilities.allowed_net_hosts.contains(clean_host) {
            return Ok(());
        }
        if parsed.is_ipv6 && parsed.explicit_port.is_none() {
            let mut v6_buf = [0u8; 64];
            if clean_host.len() + 2 <= v6_buf.len() {
                v6_buf[0] = b'[';
                v6_buf[1..1 + clean_host.len()].copy_from_slice(clean_host.as_bytes());
                v6_buf[1 + clean_host.len()] = b']';
                if let Ok(bracketed) = core::str::from_utf8(&v6_buf[..clean_host.len() + 2])
                    && self.capabilities.allowed_net_hosts.contains(bracketed)
                {
                    return Ok(());
                }
            }
        }
        if let Some(port) = parsed.explicit_port {
            if contains_host_and_port(&self.capabilities.allowed_net_hosts, clean_host, port) {
                return Ok(());
            }
        } else if let Some(def_port) = parsed.scheme_default_port {
            if contains_host_and_port(&self.capabilities.allowed_net_hosts, clean_host, def_port) {
                return Ok(());
            }
        } else if contains_host_and_port(&self.capabilities.allowed_net_hosts, clean_host, 443)
            || contains_host_and_port(&self.capabilities.allowed_net_hosts, clean_host, 80)
        {
            return Ok(());
        }
        if self.capabilities.allowed_net_hosts.contains("*")
            && !is_private_or_loopback_clean_host(clean_host)
        {
            return Ok(());
        }
        Err(self.trap_net_access(host))
    }

    /// Verifies attenuated environment variable access for `key`.
    #[inline]
    pub fn verify_env_read(&self, key: &str) -> Result<&str, SandboxViolation> {
        self.capabilities
            .allowed_env_keys
            .get(key)
            .map(|s| s.as_str())
            .ok_or_else(|| self.trap_env_access(key))
    }

    /// Verifies attenuated subprocess execution capability (`ExecCap`) for `binary`.
    /// By default (`allowed_exec_binaries` empty), all subprocess execution is denied.
    #[inline]
    pub fn verify_exec_binary(&self, binary: &str) -> Result<(), SandboxViolation> {
        if self.capabilities.allowed_exec_binaries.is_empty() {
            return Err(self.trap_exec_denied(binary));
        }
        let trimmed = binary.trim();
        if trimmed.is_empty() || memchr::memmem::find(trimmed.as_bytes(), b"..").is_some() {
            return Err(self.trap_exec_denied(binary));
        }
        for allowed in &self.capabilities.allowed_exec_binaries {
            if exec_binary_matches(trimmed, allowed) {
                return Ok(());
            }
        }
        Err(self.trap_exec_denied(binary))
    }
}
