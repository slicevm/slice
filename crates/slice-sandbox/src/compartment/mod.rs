//! Object Capabilities (OCap), SES Intrinsic Lockdown, and `TenantCompartment` Isolation.

pub mod exec;
#[allow(unsafe_code)]
pub mod pkey;
pub mod ses;
pub mod token;
pub mod violation;

pub use exec::{exec_binary_matches, has_exec_shell_metachar};
pub use pkey::{HardwareProtectionDomain, PkruDomainGuard, current_thread_pkru};
pub use ses::is_frozen_intrinsic;
pub use token::{CapabilityAttenuationRequest, CapabilityToken, extension_grant_matches};
pub use violation::SandboxViolation;

use crate::net::{check_net_host_grant, parse_net_connect_target};
use crate::vfs::vfs_path_matches_any_prefix;

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

    #[cold]
    #[inline(never)]
    fn trap_extension_denied(&self, target: &str) -> SandboxViolation {
        SandboxViolation::ExtensionDenied {
            tenant_id: self.tenant_id.clone(),
            target: target.to_string(),
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
        if vfs_path_matches_any_prefix(path, &self.capabilities.allowed_read_prefixes)
            || vfs_path_matches_any_prefix(path, &self.capabilities.virtual_mounts)
        {
            Ok(())
        } else {
            Err(self.trap_fs_read(path))
        }
    }

    /// Verifies attenuated VFS write capability for `path` using lexical path normalization
    /// and strict directory-boundary matching.
    #[inline]
    pub fn verify_fs_write(&self, path: &str) -> Result<(), SandboxViolation> {
        if vfs_path_matches_any_prefix(path, &self.capabilities.allowed_write_prefixes)
            || vfs_path_matches_any_prefix(path, &self.capabilities.virtual_mounts)
        {
            Ok(())
        } else {
            Err(self.trap_fs_write(path))
        }
    }

    /// Verifies attenuated outbound network capability for `host`, enforcing the SSRF shield
    /// against cloud instance metadata endpoints (`169.254.169.254`, `metadata.google.internal`, etc.)
    /// and blocking private/loopback CIDRs under wildcard `allow_net = ["*"]`.
    #[inline]
    pub fn verify_net_connect(&self, host: &str) -> Result<(), SandboxViolation> {
        if self.capabilities.allowed_net_hosts.is_empty() {
            return Err(self.trap_net_access(host));
        }
        let Some(parsed) = parse_net_connect_target(host) else {
            return Err(self.trap_net_access(host));
        };
        let clean_host = parsed.clean_host.as_ref();
        let allowed = if let Some(port) = parsed.explicit_port {
            let (exp, wild) = check_net_host_grant(
                &self.capabilities.allowed_net_hosts,
                clean_host,
                port,
                false,
            );
            exp || wild
        } else if let Some(def_port) = parsed.scheme_default_port {
            let (exp, wild) = check_net_host_grant(
                &self.capabilities.allowed_net_hosts,
                clean_host,
                def_port,
                false,
            );
            exp || wild
        } else {
            let (exp443, wild443) =
                check_net_host_grant(&self.capabilities.allowed_net_hosts, clean_host, 443, false);
            exp443
                || wild443
                || check_net_host_grant(&self.capabilities.allowed_net_hosts, clean_host, 80, false)
                    .0
        };
        if allowed {
            Ok(())
        } else {
            Err(self.trap_net_access(host))
        }
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
        if trimmed.is_empty() {
            return Err(self.trap_exec_denied(binary));
        }
        for allowed in &self.capabilities.allowed_exec_binaries {
            if exec_binary_matches(trimmed, allowed) {
                return Ok(());
            }
        }
        Err(self.trap_exec_denied(binary))
    }

    /// Verifies attenuated host extension capability (`ExtensionCap`) for `target`
    /// (`"<ext>"` or `"<ext>.<method>"`). Deny-all by default.
    #[inline]
    pub fn verify_extension(&self, target: &str) -> Result<(), SandboxViolation> {
        if self.capabilities.allowed_extensions.is_empty() {
            return Err(self.trap_extension_denied(target));
        }
        let trimmed = target.trim();
        if trimmed.is_empty() {
            return Err(self.trap_extension_denied(target));
        }
        for grant in &self.capabilities.allowed_extensions {
            if extension_grant_matches(trimmed, grant) {
                return Ok(());
            }
        }
        Err(self.trap_extension_denied(target))
    }
}
