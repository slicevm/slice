//! Object Capabilities (OCap), SES Intrinsic Lockdown, and `TenantCompartment` Isolation.

pub mod pkey;

pub use pkey::HardwareProtectionDomain;

use crate::net::{
    extract_clean_hostname_and_port, is_cloud_metadata_ssrf_target,
    is_private_or_loopback_ssrf_target,
};
use crate::vfs::{
    exec_binary_matches, is_rejected_vfs_candidate, normalize_vfs_path_cow,
    vfs_normalized_candidate_matches_prefix,
};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum SandboxViolation {
    #[error("OCap Violation: Compartment '{tenant_id}' lacks filesystem read authority for path '{path}'")]
    FsReadDenied { tenant_id: String, path: String },

    #[error("OCap Violation: Compartment '{tenant_id}' lacks filesystem write authority for path '{path}'")]
    FsWriteDenied { tenant_id: String, path: String },

    #[error("OCap Violation: Compartment '{tenant_id}' lacks outbound network authority for host '{host}'")]
    NetAccessDenied { tenant_id: String, host: String },

    #[error("OCap Violation: Compartment '{tenant_id}' lacks environment variable authority for key '{key}'")]
    EnvAccessDenied { tenant_id: String, key: String },

    #[error("[Slice Sandbox Violation] Subprocess execution denied: Compartment '{tenant_id}' lacks exec authority for binary '{binary}'")]
    ExecDenied { tenant_id: String, binary: String },

    #[error("SES Lockdown Violation: Compartment '{tenant_id}' attempted to mutate frozen intrinsic '{intrinsic}'")]
    IntrinsicMutationBlocked {
        tenant_id: String,
        intrinsic: String,
    },

    #[error("Resource Cage Exhausted: Compartment '{tenant_id}' exceeded deterministic CPU instruction fuel limit ({limit} ops)")]
    CpuFuelExhausted { tenant_id: String, limit: u64 },

    #[error("Resource Cage Exhausted: Compartment '{tenant_id}' exceeded memory ceiling ({requested} > {limit} bytes)")]
    MemoryCeilingExceeded {
        tenant_id: String,
        requested: usize,
        limit: usize,
    },

    #[error("Hardware PKEY Trap: Compartment '{tenant_id}' attempted unauthorized cross-domain memory access (PKEY 0x{pkey:x})")]
    HardwareMemoryDomainTrap { tenant_id: String, pkey: u32 },

    #[error("OCap Violation (CapabilityDenied: checkpoints): Compartment '{tenant_id}' checkpoint denied: {reason}")]
    CheckpointDenied { tenant_id: String, reason: String },

    #[error("Egress DLP Violation (EGRESS_SECRET_LEAK_BLOCKED): Compartment '{tenant_id}' blocked from exfiltrating a registered audit secret")]
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

impl CapabilityToken {
    /// Pure compute compartment with zero I/O or environment authority.
    #[inline]
    pub fn pure_compute() -> Self {
        Self::default()
    }

    pub fn with_read_prefix(mut self, prefix: impl Into<String>) -> Self {
        let raw = prefix.into();
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let canon = if trimmed == "*" || trimmed == "/" {
                trimmed.to_string()
            } else {
                crate::vfs::normalize_vfs_path_lexical(trimmed)
            };
            self.allowed_read_prefixes.push(canon);
        }
        self
    }

    pub fn with_write_prefix(mut self, prefix: impl Into<String>) -> Self {
        let raw = prefix.into();
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let canon = if trimmed == "*" || trimmed == "/" {
                trimmed.to_string()
            } else {
                crate::vfs::normalize_vfs_path_lexical(trimmed)
            };
            self.allowed_write_prefixes.push(canon);
        }
        self
    }

    pub fn with_net_host(mut self, host: impl Into<String>) -> Self {
        self.allowed_net_hosts.insert(host.into());
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.allowed_env_keys.insert(key.into(), val.into());
        self
    }

    pub fn with_exec_binary(mut self, binary: impl Into<String>) -> Self {
        self.allowed_exec_binaries.push(binary.into());
        self
    }

    pub fn with_checkpoints(mut self, enabled: bool) -> Self {
        self.allow_checkpoints = enabled;
        if !enabled {
            self.max_checkpoints = Some(0);
        } else if self.max_checkpoints == Some(0) {
            self.max_checkpoints = None;
        }
        self
    }

    pub fn with_max_checkpoints(mut self, max: Option<u32>) -> Self {
        self.max_checkpoints = max;
        if max == Some(0) {
            self.allow_checkpoints = false;
        }
        self
    }
}

/// Zero-allocation static check for SES frozen intrinsics.
#[inline(always)]
pub fn is_frozen_intrinsic(target: &str) -> bool {
    matches!(
        target,
        "Object.prototype"
            | "Array.prototype"
            | "Function.prototype"
            | "String.prototype"
            | "Number.prototype"
            | "Boolean.prototype"
            | "RegExp.prototype"
            | "Error.prototype"
            | "Promise.prototype"
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

    /// Consumes `ops` deterministic instruction fuel units, trapping immediately if exceeded.
    #[inline(always)]
    pub fn consume_fuel(&mut self, ops: u64) -> Result<(), SandboxViolation> {
        let next = self.fuel_consumed.saturating_add(ops);
        if next > self.fuel_limit {
            self.fuel_consumed = self.fuel_limit;
            return Err(self.trap_fuel_exhausted());
        }
        self.fuel_consumed = next;
        Ok(())
    }

    /// Allocates bytes inside the tenant's isolated heap cage, trapping if exceeding the cap.
    #[inline(always)]
    pub fn alloc_memory(&mut self, bytes: usize) -> Result<(), SandboxViolation> {
        let next = self.memory_used_bytes.saturating_add(bytes);
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
        if let Some(max) = self.capabilities.max_checkpoints {
            if current_taken >= max {
                return Err(SandboxViolation::CheckpointDenied {
                    tenant_id: self.tenant_id.clone(),
                    reason: format!(
                        "exceeded max_checkpoints limit ({} > {})",
                        current_taken.saturating_add(1),
                        max
                    ),
                });
            }
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
        let trimmed = host.trim();
        if trimmed.is_empty() || trimmed.bytes().any(|b| b <= 0x20 || b == 0x7F) {
            return Err(self.trap_net_access(host));
        }

        if trimmed.contains("://") {
            let Some(parsed) = crate::net::parse_http_authority_and_path(trimmed) else {
                return Err(self.trap_net_access(host));
            };
            let clean_host = parsed.clean_host.as_ref();
            if is_cloud_metadata_ssrf_target(clean_host) {
                return Err(self.trap_net_access(host));
            }
            if self.capabilities.allowed_net_hosts.contains(clean_host)
                || contains_host_and_port(
                    &self.capabilities.allowed_net_hosts,
                    clean_host,
                    parsed.port,
                )
            {
                return Ok(());
            }
            if self.capabilities.allowed_net_hosts.contains("*")
                && !is_private_or_loopback_ssrf_target(clean_host)
            {
                return Ok(());
            }
            return Err(self.trap_net_access(host));
        }

        if trimmed.contains(['@', '/', '?', '#', '\\']) {
            return Err(self.trap_net_access(host));
        }
        if is_cloud_metadata_ssrf_target(trimmed) {
            return Err(self.trap_net_access(host));
        }
        let (clean_host, explicit_port, scheme_default_port) =
            extract_clean_hostname_and_port(trimmed);
        if clean_host.is_empty() {
            return Err(self.trap_net_access(host));
        }
        if self.capabilities.allowed_net_hosts.contains(trimmed)
            || self
                .capabilities
                .allowed_net_hosts
                .contains(clean_host.as_ref())
        {
            return Ok(());
        }
        if let Some(port) = explicit_port {
            if contains_host_and_port(&self.capabilities.allowed_net_hosts, &clean_host, port) {
                return Ok(());
            }
        } else if let Some(def_port) = scheme_default_port {
            if contains_host_and_port(&self.capabilities.allowed_net_hosts, &clean_host, def_port) {
                return Ok(());
            }
        } else if contains_host_and_port(&self.capabilities.allowed_net_hosts, &clean_host, 443)
            || contains_host_and_port(&self.capabilities.allowed_net_hosts, &clean_host, 80)
        {
            return Ok(());
        }
        if self.capabilities.allowed_net_hosts.contains("*")
            && !is_private_or_loopback_ssrf_target(&clean_host)
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
