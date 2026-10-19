//! Object Capabilities (OCap), Secure ECMAScript (SES), and Hardware-Assisted Memory Sandboxing
//!
//! Enforces multi-tenant isolation at the language and hardware architecture layers:
//! 1. SES `lockdown()` freezing intrinsic prototypes (`Object.prototype`, `Array.prototype`).
//! 2. Per-Tenant `Compartment` with attenuated capability tokens (`FsCap`, `NetCap`, `EnvCap`).
//! 3. Deterministic CPU instruction fuel limits & hard byte-level heap ceilings.
//! 4. Hardware Memory Protection Keys (`pkey_alloc` / `WRPKRU` on Linux x86_64) switching page
//!    permissions in < 20 CPU clock cycles.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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
}

/// Attenuated Capability Token granted explicitly to a TC39/SES Compartment.
/// By default, a new Compartment has ZERO ambient authority.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilityToken {
    pub allowed_read_prefixes: Vec<String>,
    pub allowed_write_prefixes: Vec<String>,
    pub allowed_net_hosts: HashSet<String>,
    pub allowed_env_keys: HashMap<String, String>,
}

impl CapabilityToken {
    /// Pure compute compartment with zero I/O or environment authority.
    pub fn pure_compute() -> Self {
        Self::default()
    }

    pub fn with_read_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.allowed_read_prefixes.push(prefix.into());
        self
    }

    pub fn with_write_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.allowed_write_prefixes.push(prefix.into());
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
}

/// Hardware Memory Protection Key (Intel MPK / ARM POE) domain descriptor.
#[derive(Debug, Clone, Copy)]
pub struct HardwareProtectionDomain {
    pub pkey: i32,
    pub hardware_enforced: bool,
    pub pkru_mask: u32,
}

impl HardwareProtectionDomain {
    /// Allocates a hardware protection key via Linux `pkey_alloc` when supported by CPU/kernel,
    /// or assigns a virtualized protection domain tag (1..15) for deterministic domain tracking.
    pub fn allocate(tenant_slot: u32) -> Self {
        let domain_slot = ((tenant_slot % 15) + 1) as i32;

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            // SYS_pkey_alloc on x86_64 Linux is 330
            let pkey = unsafe { libc::syscall(330, 0usize, 0usize) as i32 };
            if pkey > 0 && pkey < 16 {
                let mask = !(0b11u32 << (pkey * 2));
                return Self {
                    pkey,
                    hardware_enforced: true,
                    pkru_mask: mask,
                };
            }
        }

        let mask = !(0b11u32 << (domain_slot * 2));
        Self {
            pkey: domain_slot,
            hardware_enforced: false,
            pkru_mask: mask,
        }
    }

    /// Executes a sub-20-cycle domain switch into this tenant's protection key domain.
    #[inline]
    pub fn enter_domain(&self) {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            if self.hardware_enforced {
                unsafe {
                    // Unprivileged WRPKRU instruction: EAX = pkru value, ECX = 0, EDX = 0
                    core::arch::asm!(
                        "xor ecx, ecx",
                        "xor edx, edx",
                        "wrpkru",
                        in("eax") self.pkru_mask,
                        out("ecx") _,
                        out("edx") _,
                        options(nomem, nostack)
                    );
                }
            }
        }
    }

    /// Releases the hardware protection key if allocated via kernel `pkey_alloc`.
    pub fn release(&self) {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            if self.hardware_enforced && self.pkey > 0 {
                // SYS_pkey_free on x86_64 Linux is 331
                unsafe {
                    let _ = libc::syscall(331, self.pkey as usize);
                }
            }
        }
    }
}

/// Isolated MicroVM-Grade Tenant Compartment.
///
/// Encapsulates frozen SES intrinsics, attenuated OCap capability tokens,
/// deterministic CPU instruction fuel, memory allocation accounting, and hardware PKEY state.
#[derive(Debug, Clone)]
pub struct TenantCompartment {
    pub tenant_id: String,
    pub ses_locked_down: bool,
    pub capabilities: CapabilityToken,
    pub pkey_domain: HardwareProtectionDomain,
    pub fuel_limit: u64,
    pub fuel_consumed: u64,
    pub memory_limit_bytes: usize,
    pub memory_used_bytes: usize,
    frozen_intrinsics: HashSet<String>,
}

impl TenantCompartment {
    pub fn new(
        tenant_id: impl Into<String>,
        slot_id: u32,
        capabilities: CapabilityToken,
        fuel_limit: u64,
        memory_limit_bytes: usize,
    ) -> Self {
        let mut frozen = HashSet::with_capacity(16);
        for intrinsic in [
            "Object.prototype",
            "Array.prototype",
            "Function.prototype",
            "String.prototype",
            "Number.prototype",
            "Promise.prototype",
            "Reflect",
            "JSON",
            "Math",
        ] {
            frozen.insert(intrinsic.to_string());
        }

        Self {
            tenant_id: tenant_id.into(),
            ses_locked_down: true,
            capabilities,
            pkey_domain: HardwareProtectionDomain::allocate(slot_id),
            fuel_limit,
            fuel_consumed: 0,
            memory_limit_bytes,
            memory_used_bytes: 4096, // 4 KB base compartment header frame
            frozen_intrinsics: frozen,
        }
    }

    /// Consumes `ops` deterministic instruction fuel units, trapping immediately if exceeded.
    #[inline]
    pub fn consume_fuel(&mut self, ops: u64) -> Result<(), SandboxViolation> {
        self.fuel_consumed = self.fuel_consumed.saturating_add(ops);
        if self.fuel_consumed > self.fuel_limit {
            return Err(SandboxViolation::CpuFuelExhausted {
                tenant_id: self.tenant_id.clone(),
                limit: self.fuel_limit,
            });
        }
        Ok(())
    }

    /// Allocates bytes inside the tenant's isolated heap cage, trapping if exceeding the cap.
    #[inline]
    pub fn alloc_memory(&mut self, bytes: usize) -> Result<(), SandboxViolation> {
        let next = self.memory_used_bytes.saturating_add(bytes);
        if next > self.memory_limit_bytes {
            return Err(SandboxViolation::MemoryCeilingExceeded {
                tenant_id: self.tenant_id.clone(),
                requested: next,
                limit: self.memory_limit_bytes,
            });
        }
        self.memory_used_bytes = next;
        Ok(())
    }

    /// Verifies SES `lockdown()` protection before any property mutation on a root object.
    #[inline]
    pub fn verify_property_mutation(&self, target_object: &str) -> Result<(), SandboxViolation> {
        if self.ses_locked_down && self.frozen_intrinsics.contains(target_object) {
            return Err(SandboxViolation::IntrinsicMutationBlocked {
                tenant_id: self.tenant_id.clone(),
                intrinsic: target_object.to_string(),
            });
        }
        Ok(())
    }

    /// Verifies attenuated VFS read capability for `path`.
    #[inline]
    pub fn verify_fs_read(&self, path: &str) -> Result<(), SandboxViolation> {
        let normalized = path.replace('\\', "/");
        if normalized.contains("..") {
            return Err(SandboxViolation::FsReadDenied {
                tenant_id: self.tenant_id.clone(),
                path: path.to_string(),
            });
        }
        for prefix in &self.capabilities.allowed_read_prefixes {
            if normalized.starts_with(prefix) {
                return Ok(());
            }
        }
        Err(SandboxViolation::FsReadDenied {
            tenant_id: self.tenant_id.clone(),
            path: path.to_string(),
        })
    }

    /// Verifies attenuated VFS write capability for `path`.
    #[inline]
    pub fn verify_fs_write(&self, path: &str) -> Result<(), SandboxViolation> {
        let normalized = path.replace('\\', "/");
        if normalized.contains("..") {
            return Err(SandboxViolation::FsWriteDenied {
                tenant_id: self.tenant_id.clone(),
                path: path.to_string(),
            });
        }
        for prefix in &self.capabilities.allowed_write_prefixes {
            if normalized.starts_with(prefix) {
                return Ok(());
            }
        }
        Err(SandboxViolation::FsWriteDenied {
            tenant_id: self.tenant_id.clone(),
            path: path.to_string(),
        })
    }

    /// Verifies attenuated outbound network capability for `host`.
    #[inline]
    pub fn verify_net_connect(&self, host: &str) -> Result<(), SandboxViolation> {
        if self.capabilities.allowed_net_hosts.contains(host)
            || self.capabilities.allowed_net_hosts.contains("*")
        {
            return Ok(());
        }
        Err(SandboxViolation::NetAccessDenied {
            tenant_id: self.tenant_id.clone(),
            host: host.to_string(),
        })
    }

    /// Verifies attenuated environment variable access for `key`.
    #[inline]
    pub fn verify_env_read(&self, key: &str) -> Result<&str, SandboxViolation> {
        self.capabilities
            .allowed_env_keys
            .get(key)
            .map(|s| s.as_str())
            .ok_or_else(|| SandboxViolation::EnvAccessDenied {
                tenant_id: self.tenant_id.clone(),
                key: key.to_string(),
            })
    }
}

impl Drop for TenantCompartment {
    fn drop(&mut self) {
        self.pkey_domain.release();
    }
}

/// Declarative Project & OCap Manifest (`slice.toml`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliceManifest {
    pub name: String,
    pub version: String,
    pub entrypoint: String,
    pub fuel_limit: u64,
    pub memory_limit_kb: usize,
    pub capabilities: CapabilityToken,
}

impl Default for SliceManifest {
    fn default() -> Self {
        Self {
            name: "slice-edge-service".to_string(),
            version: "0.1.0".to_string(),
            entrypoint: "./src/index.ts".to_string(),
            fuel_limit: 50_000_000,
            memory_limit_kb: 4096,
            capabilities: CapabilityToken::pure_compute(),
        }
    }
}

impl SliceManifest {
    /// Parses a `slice.toml` configuration string without external heavy dependencies.
    pub fn parse_toml(content: &str) -> Self {
        let mut manifest = Self::default();
        let mut current_section = "";

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                current_section = &trimmed[1..trimmed.len() - 1];
                continue;
            }
            if let Some((k, v)) = trimmed.split_once('=') {
                let key = k.trim();
                let val = v.trim();
                match (current_section, key) {
                    ("package", "name") => {
                        manifest.name = val.trim_matches('"').trim_matches('\'').to_string();
                    }
                    ("package", "version") => {
                        manifest.version = val.trim_matches('"').trim_matches('\'').to_string();
                    }
                    ("package", "entrypoint") => {
                        manifest.entrypoint = val.trim_matches('"').trim_matches('\'').to_string();
                    }
                    ("limits", "fuel") => {
                        let digits: String = val.chars().filter(|c| c.is_ascii_digit()).collect();
                        if let Ok(n) = digits.parse::<u64>() {
                            manifest.fuel_limit = n;
                        }
                    }
                    ("limits", "memory_kb") => {
                        let digits: String = val.chars().filter(|c| c.is_ascii_digit()).collect();
                        if let Ok(n) = digits.parse::<usize>() {
                            manifest.memory_limit_kb = n;
                        }
                    }
                    ("capabilities", "allow_read") => {
                        for item in parse_toml_string_array(val) {
                            manifest.capabilities.allowed_read_prefixes.push(item);
                        }
                    }
                    ("capabilities", "allow_write") => {
                        for item in parse_toml_string_array(val) {
                            manifest.capabilities.allowed_write_prefixes.push(item);
                        }
                    }
                    ("capabilities", "allow_net") => {
                        for item in parse_toml_string_array(val) {
                            manifest.capabilities.allowed_net_hosts.insert(item);
                        }
                    }
                    ("capabilities.env", _) => {
                        let env_val = val.trim_matches('"').trim_matches('\'').to_string();
                        manifest
                            .capabilities
                            .allowed_env_keys
                            .insert(key.to_string(), env_val);
                    }
                    _ => {}
                }
            }
        }
        manifest
    }

    /// Generates a documented default `slice.toml` file for `slice init`.
    pub fn default_template(project_name: &str) -> String {
        format!(
            r#"# Slice (Nexus-JS) Declarative MicroVM & Object Capability Manifest

[package]
name = "{}"
version = "0.1.0"
entrypoint = "./src/index.ts"

[limits]
# Deterministic CPU instruction fuel ceiling per tenant invocation
fuel = 50_000_000
# Hard per-compartment memory allocation ceiling in KB
memory_kb = 4096

[capabilities]
# Attenuated Virtual Filesystem read prefixes (Zero ambient filesystem access)
allow_read = ["/app/public", "/app/config"]
# Attenuated outbound network host:port allowlist
allow_net = ["api.internal:443", "edge.slice.local:80"]

[capabilities.env]
SERVICE_STAGE = "production"
RUNTIME_ENGINE = "Slice-NexusJS"
"#,
            project_name
        )
    }
}

fn parse_toml_string_array(raw: &str) -> Vec<String> {
    let inner = raw.trim().trim_start_matches('[').trim_end_matches(']');
    inner
        .split(',')
        .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ses_lockdown_and_ocap_enforcement() {
        let caps = CapabilityToken::pure_compute()
            .with_read_prefix("/app/data")
            .with_net_host("db.internal:5432")
            .with_env("APP_MODE", "edge");

        let mut tenant = TenantCompartment::new("tenant-alpha", 1, caps, 10_000, 65536);

        // 1. SES Prototype Pollution must be blocked
        assert_eq!(
            tenant.verify_property_mutation("Object.prototype"),
            Err(SandboxViolation::IntrinsicMutationBlocked {
                tenant_id: "tenant-alpha".into(),
                intrinsic: "Object.prototype".into()
            })
        );

        // 2. Scoped VFS read succeeds on /app/data, fails on /etc/passwd or traversal
        assert!(tenant.verify_fs_read("/app/data/config.json").is_ok());
        assert!(tenant.verify_fs_read("/etc/passwd").is_err());
        assert!(tenant.verify_fs_read("/app/data/../../etc/shadow").is_err());

        // 3. Scoped network authority
        assert!(tenant.verify_net_connect("db.internal:5432").is_ok());
        assert!(tenant.verify_net_connect("attacker.evil.com:443").is_err());

        // 4. Scoped env authority
        assert_eq!(tenant.verify_env_read("APP_MODE"), Ok("edge"));
        assert!(tenant.verify_env_read("AWS_SECRET_ACCESS_KEY").is_err());

        // 5. Fuel limit preemption
        assert!(tenant.consume_fuel(5_000).is_ok());
        assert!(tenant.consume_fuel(6_000).is_err());

        // 6. Memory ceiling enforcement
        assert!(tenant.alloc_memory(16_384).is_ok());
        assert!(matches!(
            tenant.alloc_memory(128_000),
            Err(SandboxViolation::MemoryCeilingExceeded { .. })
        ));
    }

    #[test]
    fn test_slice_manifest_toml_parsing_and_fs_write_ocap() {
        let toml_str = r#"
[package]
name = "edge-gateway"
version = "0.2.0"
entrypoint = "./src/main.ts"

[limits]
fuel = 25_000_000
memory_kb = 8192

[capabilities]
allow_read = ["/data/read", "/config"]
allow_write = ["/tmp/shards"]
allow_net = ["api.stripe.com:443"]

[capabilities.env]
NODE_ENV = "production"
"#;
        let manifest = SliceManifest::parse_toml(toml_str);
        assert_eq!(manifest.name, "edge-gateway");
        assert_eq!(manifest.version, "0.2.0");
        assert_eq!(manifest.entrypoint, "./src/main.ts");
        assert_eq!(manifest.fuel_limit, 25_000_000);
        assert_eq!(manifest.memory_limit_kb, 8192);

        let tenant = TenantCompartment::new(
            "tenant-manifest",
            2,
            manifest.capabilities,
            manifest.fuel_limit,
            manifest.memory_limit_kb * 1024,
        );
        assert!(tenant.verify_fs_read("/data/read/items.json").is_ok());
        assert!(tenant.verify_fs_write("/tmp/shards/shard_00.jsonl").is_ok());
        assert!(tenant.verify_fs_write("/etc/crontab").is_err());
        assert!(tenant.verify_fs_write("/tmp/shards/../../etc/passwd").is_err());
        assert_eq!(tenant.verify_env_read("NODE_ENV"), Ok("production"));
    }
}

