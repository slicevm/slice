//! Object Capabilities (OCap), Secure ECMAScript (SES), and Hardware-Assisted Memory Sandboxing
//!
//! Enforces multi-tenant isolation at the language and hardware architecture layers:
//! 1. SES `lockdown()` freezing intrinsic prototypes (`Object.prototype`, `Array.prototype`).
//! 2. Per-Tenant `Compartment` with attenuated capability tokens (`FsCap`, `NetCap`, `EnvCap`).
//! 3. Deterministic CPU instruction fuel limits & hard byte-level heap ceilings.
//! 4. Hardware Memory Protection Keys (`pkey_alloc` / `WRPKRU` on Linux x86_64) switching page
//!    permissions in < 20 CPU clock cycles.

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
    pub allowed_net_hosts: FxHashSet<String>,
    pub allowed_env_keys: FxHashMap<String, String>,
}

impl CapabilityToken {
    /// Pure compute compartment with zero I/O or environment authority.
    #[inline]
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
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct HardwareProtectionDomain {
    pub pkey: i32,
    pub pkru_mask: u32,
    pub hardware_enforced: bool,
}

impl HardwareProtectionDomain {
    /// Allocates a hardware protection key via Linux `pkey_alloc` when supported by CPU/kernel,
    /// or assigns a virtualized protection domain tag (1..15) for deterministic domain tracking.
    #[inline]
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
                    pkru_mask: mask,
                    hardware_enforced: true,
                };
            }
        }

        let mask = !(0b11u32 << (domain_slot * 2));
        Self {
            pkey: domain_slot,
            pkru_mask: mask,
            hardware_enforced: false,
        }
    }

    /// Executes a sub-20-cycle domain switch into this tenant's protection key domain.
    #[inline(always)]
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
    #[inline]
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

/// Extracts the lowercase hostname (without scheme, path, `:port`, or IPv6 brackets) from a `host`, `host:port`, or URL string.
#[inline]
pub fn extract_clean_hostname(host: &str) -> String {
    let mut trimmed = host.trim();
    if let Some((_, after_scheme)) = trimmed.split_once("://") {
        trimmed = after_scheme;
    }
    if let Some((authority, _)) = trimmed.split_once(['/', '?', '#']) {
        trimmed = authority;
    }
    if let Some((_, after_auth)) = trimmed.rsplit_once('@') {
        trimmed = after_auth;
    }
    let without_port = if let Some(rest) = trimmed.strip_prefix('[') {
        if let Some((ipv6, _)) = rest.split_once(']') {
            ipv6
        } else {
            trimmed
        }
    } else if let Some((h, port_part)) = trimmed.rsplit_once(':') {
        if !h.contains(':') && port_part.bytes().all(|b| b.is_ascii_digit()) {
            h
        } else {
            trimmed
        }
    } else {
        trimmed
    };
    without_port.to_ascii_lowercase()
}

/// Attempts to parse non-canonical IPv4 literal strings (octal `0177.0.0.1`, hex `0x7f.0.0.1`,
/// integer/dword `2130706433`, or shorthand 2/3-part dotted quads like `127.1` or `169.254.43518`)
/// into a canonical `std::net::Ipv4Addr` so SSRF guards cannot be bypassed via alternative IP encodings.
fn parse_nonstandard_ipv4(h: &str) -> Option<std::net::Ipv4Addr> {
    if h.is_empty() {
        return None;
    }
    let parts: Vec<&str> = h.split('.').collect();
    if parts.len() > 4 {
        return None;
    }
    let mut vals = Vec::with_capacity(parts.len());
    for part in &parts {
        if part.is_empty() {
            return None;
        }
        let val = if let Some(hex) = part.strip_prefix("0x").or_else(|| part.strip_prefix("0X")) {
            u32::from_str_radix(hex, 16).ok()?
        } else if part.len() > 1
            && part.starts_with('0')
            && part.bytes().all(|b| b.is_ascii_digit())
        {
            u32::from_str_radix(part, 8).ok()?
        } else {
            part.parse::<u32>().ok()?
        };
        vals.push(val);
    }
    let ip_u32 = match vals.as_slice() {
        [a] => *a,
        [a, b] if *a <= 0xff && *b <= 0x00ff_ffff => (*a << 24) | *b,
        [a, b, c] if *a <= 0xff && *b <= 0xff && *c <= 0xffff => (*a << 24) | (*b << 16) | *c,
        [a, b, c, d] if *a <= 0xff && *b <= 0xff && *c <= 0xff && *d <= 0xff => {
            (*a << 24) | (*b << 16) | (*c << 8) | *d
        }
        _ => return None,
    };
    Some(std::net::Ipv4Addr::from(ip_u32))
}

/// Returns `true` if `host` resolves to a cloud instance metadata endpoint
/// (AWS/GCP/Azure/DigitalOcean `169.254.169.254`, AWS ECS `169.254.170.2`,
/// Alibaba `100.100.100.200`, AWS IPv6 `fd00:ec2::254`, or GCP `metadata.google.internal`).
/// These endpoints are unconditionally blocked by the Slice SSRF shield.
#[inline]
pub fn is_cloud_metadata_ssrf_target(host: &str) -> bool {
    let h = extract_clean_hostname(host);
    if matches!(
        h.as_str(),
        "169.254.169.254"
            | "169.254.170.2"
            | "100.100.100.200"
            | "fd00:ec2::254"
            | "metadata.google.internal"
            | "metadata.goog"
            | "instance-data"
    ) {
        return true;
    }
    if let Ok(ip) = h.parse::<std::net::IpAddr>() {
        if is_cloud_metadata_ip(ip) {
            return true;
        }
    }
    if let Some(v4) = parse_nonstandard_ipv4(&h) {
        if is_cloud_metadata_ip(std::net::IpAddr::V4(v4)) {
            return true;
        }
    }
    false
}

/// Returns `true` if `host` is a cloud metadata endpoint, loopback address (`localhost`, `127.0.0.0/8`, `::1`),
/// link-local (`169.254.0.0/16`, `fe80::/10`), RFC1918 private network (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`),
/// CGNAT (`100.64.0.0/10`), benchmark (`198.18.0.0/15`), IETF protocol (`192.0.0.0/24`), multicast (`224.0.0.0/4`, `ff00::/8`),
/// IPv6 Unique Local (`fc00::/7`), or any non-canonical numeric IP representation (octal, hex, dword, shorthand IPv4).
/// Wildcard `allow_net = ["*"]` never grants access to these targets; they require explicit host allowlisting.
#[inline]
pub fn is_private_or_loopback_ssrf_target(host: &str) -> bool {
    if is_cloud_metadata_ssrf_target(host) {
        return true;
    }
    let h = extract_clean_hostname(host);
    if matches!(
        h.as_str(),
        "localhost" | "0.0.0.0" | "::" | "::1" | "0:0:0:0:0:0:0:1" | "0:0:0:0:0:0:0:0"
    ) || h.ends_with(".localhost")
    {
        return true;
    }
    if let Ok(ip) = h.parse::<std::net::IpAddr>() {
        return is_private_or_loopback_ip(ip);
    }
    if parse_nonstandard_ipv4(&h).is_some() {
        // Any non-canonical numeric IPv4 literal (octal `0177.0.0.1`, hex `0x7f.0.0.1`,
        // dword `2130706433`, or shorthand `127.1`) is unconditionally blocked under wildcard allow_net.
        return true;
    }
    false
}

/// Returns `true` if `ip` is a cloud instance metadata IP address (`169.254.169.254`,
/// `169.254.170.2`, `100.100.100.200`, `fd00:ec2::254`, or their IPv4-mapped IPv6 equivalents).
#[inline]
pub fn is_cloud_metadata_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let octs = v4.octets();
            octs == [169, 254, 169, 254]
                || octs == [169, 254, 170, 2]
                || octs == [100, 100, 100, 200]
        }
        std::net::IpAddr::V6(v6) => {
            if let Some(mapped_v4) = v6.to_ipv4_mapped() {
                return is_cloud_metadata_ip(std::net::IpAddr::V4(mapped_v4));
            }
            let segs = v6.segments();
            segs == [0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254]
        }
    }
}

/// Returns `true` if `ip` is a cloud metadata IP, loopback (`127.0.0.0/8`, `::1`),
/// unspecified (`0.0.0.0`, `::`), link-local (`169.254.0.0/16`, `fe80::/10`),
/// RFC 1918 private (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`),
/// CGNAT (`100.64.0.0/10`), IETF protocol (`192.0.0.0/24`), TEST-NET (`192.0.2.0/24`,
/// `198.51.100.0/24`, `203.0.113.0/24`), benchmark (`198.18.0.0/15`), multicast (`224.0.0.0/4`, `ff00::/8`),
/// reserved (`240.0.0.0/4`), unique local (`fc00::/7`), or IPv4-mapped IPv6 equivalent.
#[inline]
pub fn is_private_or_loopback_ip(ip: std::net::IpAddr) -> bool {
    if is_cloud_metadata_ip(ip) {
        return true;
    }
    match ip {
        std::net::IpAddr::V4(v4) => {
            let octs = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || (octs[0] == 100 && (octs[1] & 0b1100_0000) == 64) // 100.64.0.0/10 CGNAT
                || (octs[0] == 192 && octs[1] == 0 && octs[2] == 0) // 192.0.0.0/24 IETF Protocol
                || (octs[0] == 198 && (octs[1] == 18 || octs[1] == 19)) // 198.18.0.0/15 Benchmark
                || octs[0] >= 240 // 240.0.0.0/4 Reserved for future use
        }
        std::net::IpAddr::V6(v6) => {
            if let Some(mapped_v4) = v6.to_ipv4_mapped() {
                return is_private_or_loopback_ip(std::net::IpAddr::V4(mapped_v4));
            }
            let segs = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (segs[0] & 0xfe00) == 0xfc00 // fc00::/7 Unique Local
                || (segs[0] & 0xffc0) == 0xfe80 // fe80::/10 Link-Local
                || (segs[0] == 0x2001 && segs[1] == 0x0db8) // 2001:db8::/32 Documentation
        }
    }
}

/// Lexically normalizes a VFS or host filesystem path into a canonical forward-slash representation:
/// - Converts Windows `\` separators to `/`.
/// - Strips Win32 extended-length prefix (`//?/` or `\\?\`).
/// - Normalizes Windows drive-letter casing to uppercase (`c:/` -> `C:/`).
/// - Resolves `.` and `..` segments lexically on a stack-allocated `SmallVec` so path traversal
///   sequences like `/tmp/../etc/passwd` resolve to `/etc/passwd` **before** any prefix check.
pub fn normalize_vfs_path_lexical(raw: &str) -> String {
    use cow_utils::CowUtils;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return ".".to_string();
    }
    let slash_cow = trimmed.cow_replace('\\', "/");
    let mut s = slash_cow.as_ref();
    if let Some(stripped) = s.strip_prefix("//?/") {
        s = stripped;
    }

    let (root_prefix, rest, is_abs) =
        if s.len() >= 2 && s.as_bytes()[0].is_ascii_alphabetic() && s.as_bytes()[1] == b':' {
            let drive = (s.as_bytes()[0].to_ascii_uppercase()) as char;
            let after_colon = &s[2..];
            let after_slash = after_colon.trim_start_matches('/');
            (Some(drive), after_slash, true)
        } else if s.starts_with('/') {
            (None, s.trim_start_matches('/'), true)
        } else {
            (None, s, false)
        };

    let mut stack: smallvec::SmallVec<[&str; 16]> = smallvec::SmallVec::new();
    for seg in rest.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            if let Some(&last) = stack.last() {
                if last != ".." {
                    stack.pop();
                    continue;
                }
            }
            if !is_abs {
                stack.push("..");
            }
        } else {
            stack.push(seg);
        }
    }

    let mut out = String::with_capacity(s.len().max(2));
    if let Some(drive) = root_prefix {
        out.push(drive);
        out.push(':');
        out.push('/');
        for (i, seg) in stack.iter().enumerate() {
            if i > 0 {
                out.push('/');
            }
            out.push_str(seg);
        }
    } else if is_abs {
        out.push('/');
        for (i, seg) in stack.iter().enumerate() {
            if i > 0 {
                out.push('/');
            }
            out.push_str(seg);
        }
    } else if stack.is_empty() {
        out.push('.');
    } else {
        for (i, seg) in stack.iter().enumerate() {
            if i > 0 {
                out.push('/');
            }
            out.push_str(seg);
        }
    }
    out
}

#[inline]
fn path_starts_with_dir_boundary(norm_candidate: &str, norm_prefix: &str) -> bool {
    if norm_prefix == "*" || norm_prefix == "/" {
        return true;
    }
    if norm_candidate == norm_prefix {
        return true;
    }
    if let Some(rem) = norm_candidate.strip_prefix(norm_prefix) {
        return norm_prefix.ends_with('/') || rem.starts_with('/');
    }
    false
}

/// Verifies whether `candidate_path` is authorized by `allowed_prefix` under OCap VFS rules:
/// 1. Rejects any raw path containing `..` traversal segments immediately (`..` defense-in-depth).
/// 2. Lexically normalizes both `candidate_path` and `allowed_prefix` via [`normalize_vfs_path_lexical`].
/// 3. Enforces strict directory-boundary matching (`/tmp` matches `/tmp` and `/tmp/foo`, but NEVER `/tmp_evil`).
/// 4. Maps `/tmp` <-> `std::env::temp_dir()` seamlessly across Windows, Linux, and macOS.
/// 5. Resolves relative capability prefixes (`"."`, `"./data"`) or relative candidate paths against
///    `std::env::current_dir()` when comparing relative and absolute paths.
pub fn vfs_prefix_matches(candidate_path: &str, allowed_prefix: &str) -> bool {
    if memchr::memmem::find(candidate_path.as_bytes(), b"..").is_some() {
        return false;
    }
    if allowed_prefix == "*" || allowed_prefix == "/" {
        return true;
    }
    let norm_cand = normalize_vfs_path_lexical(candidate_path);
    let norm_pref = normalize_vfs_path_lexical(allowed_prefix);
    if path_starts_with_dir_boundary(&norm_cand, &norm_pref) {
        return true;
    }

    // Cross-platform `/tmp` <-> `std::env::temp_dir()` equivalence
    if norm_pref == "/tmp" || norm_pref.starts_with("/tmp/") {
        let os_tmp = normalize_vfs_path_lexical(&std::env::temp_dir().to_string_lossy());
        if !os_tmp.is_empty() {
            let mapped_pref = if norm_pref == "/tmp" {
                os_tmp
            } else {
                format!("{}{}", os_tmp, &norm_pref[4..])
            };
            if path_starts_with_dir_boundary(&norm_cand, &mapped_pref) {
                return true;
            }
        }
    }

    // Relative-vs-absolute workspace resolution against `current_dir()`
    let cand_is_abs =
        norm_cand.starts_with('/') || (norm_cand.len() >= 3 && norm_cand.as_bytes()[1] == b':');
    let pref_is_abs =
        norm_pref.starts_with('/') || (norm_pref.len() >= 3 && norm_pref.as_bytes()[1] == b':');
    if cand_is_abs != pref_is_abs {
        if let Ok(cwd) = std::env::current_dir() {
            let norm_cwd = normalize_vfs_path_lexical(&cwd.to_string_lossy());
            let abs_cand = if cand_is_abs {
                norm_cand
            } else if norm_cand == "." {
                norm_cwd.clone()
            } else {
                normalize_vfs_path_lexical(&format!("{}/{}", norm_cwd, norm_cand))
            };
            let abs_pref = if pref_is_abs {
                norm_pref
            } else if norm_pref == "." {
                norm_cwd
            } else {
                normalize_vfs_path_lexical(&format!("{}/{}", norm_cwd, norm_pref))
            };
            if path_starts_with_dir_boundary(&abs_cand, &abs_pref) {
                return true;
            }
        }
    }
    false
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

    /// Consumes `ops` deterministic instruction fuel units, trapping immediately if exceeded.
    #[inline(always)]
    pub fn consume_fuel(&mut self, ops: u64) -> Result<(), SandboxViolation> {
        self.fuel_consumed = self.fuel_consumed.saturating_add(ops);
        if self.fuel_consumed > self.fuel_limit {
            return Err(self.trap_fuel_exhausted());
        }
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
        if memchr::memmem::find(path.as_bytes(), b"..").is_some() {
            return Err(self.trap_fs_read(path));
        }
        for prefix in &self.capabilities.allowed_read_prefixes {
            if vfs_prefix_matches(path, prefix) {
                return Ok(());
            }
        }
        Err(self.trap_fs_read(path))
    }

    /// Verifies attenuated VFS write capability for `path` using lexical path normalization
    /// and strict directory-boundary matching.
    #[inline]
    pub fn verify_fs_write(&self, path: &str) -> Result<(), SandboxViolation> {
        if memchr::memmem::find(path.as_bytes(), b"..").is_some() {
            return Err(self.trap_fs_write(path));
        }
        for prefix in &self.capabilities.allowed_write_prefixes {
            if vfs_prefix_matches(path, prefix) {
                return Ok(());
            }
        }
        Err(self.trap_fs_write(path))
    }

    /// Verifies attenuated outbound network capability for `host`, enforcing the SSRF shield
    /// against cloud instance metadata endpoints (`169.254.169.254`, `metadata.google.internal`, etc.)
    /// and blocking private/loopback CIDRs under wildcard `allow_net = ["*"]`.
    #[inline]
    pub fn verify_net_connect(&self, host: &str) -> Result<(), SandboxViolation> {
        if is_cloud_metadata_ssrf_target(host) {
            return Err(self.trap_net_access(host));
        }
        let clean_host = extract_clean_hostname(host);
        if self.capabilities.allowed_net_hosts.contains(host)
            || self.capabilities.allowed_net_hosts.contains(&clean_host)
            || self
                .capabilities
                .allowed_net_hosts
                .contains(&format!("{}:443", clean_host))
            || self
                .capabilities
                .allowed_net_hosts
                .contains(&format!("{}:80", clean_host))
        {
            return Ok(());
        }
        if self.capabilities.allowed_net_hosts.contains("*")
            && !is_private_or_loopback_ssrf_target(host)
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
    pub env_file: Option<String>,
    pub capabilities: CapabilityToken,
    #[serde(default)]
    pub exper_boacas: Option<bool>,
}

impl Default for SliceManifest {
    fn default() -> Self {
        Self {
            name: "slice-edge-service".to_string(),
            version: "0.1.0".to_string(),
            entrypoint: "./src/index.ts".to_string(),
            fuel_limit: 50_000_000,
            memory_limit_kb: 4096,
            env_file: None,
            capabilities: CapabilityToken::pure_compute(),
            exper_boacas: None,
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
                    ("capabilities", "allow_env") => {
                        for item in parse_toml_string_array(val) {
                            if let Some((k, v)) = item.split_once('=') {
                                manifest
                                    .capabilities
                                    .allowed_env_keys
                                    .insert(k.trim().to_string(), v.trim().to_string());
                            } else {
                                let k = item.trim();
                                let v = std::env::var(k).unwrap_or_default();
                                manifest
                                    .capabilities
                                    .allowed_env_keys
                                    .insert(k.to_string(), v);
                            }
                        }
                    }
                    ("capabilities", "env_file") => {
                        let ef = val.trim_matches('"').trim_matches('\'').trim().to_string();
                        if !ef.is_empty() {
                            manifest.env_file = Some(ef);
                        }
                    }
                    ("capabilities.env", _) => {
                        let env_val = val.trim_matches('"').trim_matches('\'').to_string();
                        manifest
                            .capabilities
                            .allowed_env_keys
                            .insert(key.to_string(), env_val);
                    }
                    ("experiments", "boacas") => {
                        let norm = val.trim_matches('"').trim_matches('\'').trim();
                        if matches!(norm, "true" | "1" | "yes" | "on") {
                            manifest.exper_boacas = Some(true);
                        } else if matches!(norm, "false" | "0" | "no" | "off") {
                            manifest.exper_boacas = Some(false);
                        }
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

/// Returns `true` if `key` matches sensitive credential naming patterns (`*_SECRET*`, `*_TOKEN*`,
/// `*_KEY*`, `*_PASSWORD*`, `*_PRIVATE*`, `*_CREDENTIAL*`) and is not a public/publishable key.
#[inline]
pub fn is_sensitive_env_key(key: &str) -> bool {
    let upper = key.trim().to_ascii_uppercase();
    if upper.contains("PUBLIC_KEY") || upper.contains("PUBLISHABLE_KEY") {
        return false;
    }
    upper.contains("SECRET")
        || upper.contains("TOKEN")
        || upper.contains("KEY")
        || upper.contains("PASSWORD")
        || upper.contains("PRIVATE")
        || upper.contains("CREDENTIAL")
}

/// Parses `.env` file content into `(env_pairs, auto_dlp_secrets)`.
/// Automatically classifies values of keys matching [`is_sensitive_env_key`] (with length >= 6)
/// as outbound Egress DLP secrets so they cannot be exfiltrated via `stdout`, HTTP responses, SSE, or `fetch()`.
pub fn parse_dotenv_content(content: &str) -> (Vec<(String, String)>, Vec<String>) {
    let mut pairs = Vec::new();
    let mut secrets = Vec::new();

    for raw_line in content.lines() {
        let mut line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(stripped) = line.strip_prefix("export ") {
            line = stripped.trim();
        }
        let Some((raw_k, raw_v)) = line.split_once('=') else {
            continue;
        };
        let key = raw_k.trim().to_string();
        if key.is_empty() {
            continue;
        }
        let mut val_part = raw_v.trim();
        let val = if (val_part.starts_with('"') && val_part.ends_with('"') && val_part.len() >= 2)
            || (val_part.starts_with('\'') && val_part.ends_with('\'') && val_part.len() >= 2)
        {
            val_part[1..val_part.len() - 1]
                .replace("\\n", "\n")
                .replace("\\\"", "\"")
        } else {
            if let Some(hash_idx) = val_part.find(" #") {
                val_part = val_part[..hash_idx].trim();
            }
            val_part.to_string()
        };

        if val.len() >= 6 && is_sensitive_env_key(&key) && !secrets.contains(&val) {
            secrets.push(val.clone());
        }
        pairs.push((key, val));
    }

    (pairs, secrets)
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
        assert!(tenant
            .verify_fs_write("/tmp/shards/../../etc/passwd")
            .is_err());
        assert_eq!(tenant.verify_env_read("NODE_ENV"), Ok("production"));
    }

    #[test]
    fn test_phase_9a_compartment_layout_and_zero_alloc_intrinsics() {
        // Verify all 9 frozen SES intrinsics match via zero-allocation compile-time trie
        for intrinsic in [
            "Object.prototype",
            "Array.prototype",
            "Function.prototype",
            "Promise.prototype",
            "String.prototype",
            "Number.prototype",
            "Boolean.prototype",
            "RegExp.prototype",
            "Error.prototype",
        ] {
            assert!(is_frozen_intrinsic(intrinsic));
        }
        assert!(!is_frozen_intrinsic("CustomClass.prototype"));

        // Verify Windows backslash normalization still works via Cow::Owned fallback
        let caps = CapabilityToken::pure_compute()
            .with_read_prefix("/app/data")
            .with_write_prefix("/tmp/out");
        let tenant = TenantCompartment::new("tenant-9a", 3, caps, 10_000, 65536);
        assert!(tenant.verify_fs_read(r"\app\data\file.json").is_ok());
        assert!(tenant.verify_fs_write(r"\tmp/out\result.txt").is_ok());
    }

    #[test]
    fn test_tag_f3_ssrf_shield_and_wildcard_private_network_blocking() {
        // 1. Wildcard `allow_net = ["*"]` allows public internet hosts, but blocks cloud metadata and RFC1918/loopback
        let wildcard_caps = CapabilityToken::pure_compute().with_net_host("*");
        let wildcard_tenant =
            TenantCompartment::new("tenant-wildcard", 4, wildcard_caps, 10_000, 65536);

        assert!(wildcard_tenant
            .verify_net_connect("api.openai.com:443")
            .is_ok());
        assert!(wildcard_tenant.verify_net_connect("api.stripe.com").is_ok());

        // Cloud instance metadata endpoints must be unconditionally blocked even under `*`
        for metadata_host in [
            "169.254.169.254",
            "169.254.169.254:80",
            "169.254.170.2:80",
            "100.100.100.200:80",
            "[fd00:ec2::254]:80",
            "metadata.google.internal",
            "metadata.google.internal:80",
            "metadata.goog:80",
        ] {
            assert!(
                matches!(
                    wildcard_tenant.verify_net_connect(metadata_host),
                    Err(SandboxViolation::NetAccessDenied { .. })
                ),
                "Expected SSRF block for metadata endpoint {}",
                metadata_host
            );
        }

        // Loopback and RFC1918 private networks must be blocked under `*`
        for private_host in [
            "localhost:8080",
            "127.0.0.1:6379",
            "0.0.0.0:3000",
            "[::1]:8080",
            "10.0.0.42:5432",
            "172.16.0.10:8080",
            "172.31.255.1:8080",
            "192.168.1.100:9000",
        ] {
            assert!(
                matches!(
                    wildcard_tenant.verify_net_connect(private_host),
                    Err(SandboxViolation::NetAccessDenied { .. })
                ),
                "Expected SSRF block for private/loopback host {}",
                private_host
            );
        }
        // Public 172.32.x.x is outside 172.16.0.0/12 and should be permitted under `*`
        assert!(wildcard_tenant.verify_net_connect("172.32.0.1:443").is_ok());

        // 2. Explicit local microservice allowlisting permits only that exact host:port
        let local_caps = CapabilityToken::pure_compute().with_net_host("127.0.0.1:18091");
        let local_tenant = TenantCompartment::new("tenant-local", 5, local_caps, 10_000, 65536);
        assert!(local_tenant.verify_net_connect("127.0.0.1:18091").is_ok());
        assert!(local_tenant.verify_net_connect("127.0.0.1:6379").is_err());
        assert!(local_tenant
            .verify_net_connect("169.254.169.254:80")
            .is_err());
    }

    #[test]
    fn test_tag_f4_dotenv_parser_and_auto_dlp_secret_classification() {
        let dotenv_raw = r#"
# Production Vault Configuration
SERVICE_REGION=us-east-1
PUBLIC_KEY_ID="pk_live_public_identifier_123"
export STRIPE_SECRET_KEY="sk_live_f4_stripe_vault_998877"
OPENAI_API_TOKEN=tok_openai_f4_secret_665544 # inline comment
DB_PASSWORD='pg_super_secret_pw_f4'
"#;
        let (pairs, secrets) = parse_dotenv_content(dotenv_raw);
        assert_eq!(pairs.len(), 5);
        assert_eq!(
            pairs[0],
            ("SERVICE_REGION".to_string(), "us-east-1".to_string())
        );
        assert_eq!(
            pairs[1],
            (
                "PUBLIC_KEY_ID".to_string(),
                "pk_live_public_identifier_123".to_string()
            )
        );
        assert_eq!(
            pairs[2],
            (
                "STRIPE_SECRET_KEY".to_string(),
                "sk_live_f4_stripe_vault_998877".to_string()
            )
        );
        assert_eq!(
            pairs[3],
            (
                "OPENAI_API_TOKEN".to_string(),
                "tok_openai_f4_secret_665544".to_string()
            )
        );
        assert_eq!(
            pairs[4],
            (
                "DB_PASSWORD".to_string(),
                "pg_super_secret_pw_f4".to_string()
            )
        );

        // Only the 3 sensitive keys (excluding SERVICE_REGION and PUBLIC_KEY_ID) should be auto-registered for DLP
        assert_eq!(
            secrets,
            vec![
                "sk_live_f4_stripe_vault_998877".to_string(),
                "tok_openai_f4_secret_665544".to_string(),
                "pg_super_secret_pw_f4".to_string(),
            ]
        );

        let toml_with_env_file = r#"
[package]
name = "f4-app"
entrypoint = "./src/index.ts"

[capabilities]
env_file = ".env.production"
"#;
        let m = SliceManifest::parse_toml(toml_with_env_file);
        assert_eq!(m.env_file.as_deref(), Some(".env.production"));
    }

    #[test]
    fn test_phase_n1_resolved_ip_ssrf_and_ipv4_mapped_ipv6_blocking() {
        use std::net::IpAddr;

        for meta_ip in [
            "169.254.169.254",
            "169.254.170.2",
            "100.100.100.200",
            "::ffff:169.254.169.254",
            "::ffff:100.100.100.200",
            "fd00:ec2::254",
        ] {
            let parsed: IpAddr = meta_ip.parse().unwrap();
            assert!(
                is_cloud_metadata_ip(parsed),
                "Expected cloud metadata classification for {}",
                meta_ip
            );
        }

        for priv_ip in [
            "127.0.0.1",
            "0.0.0.0",
            "10.0.0.42",
            "172.16.0.1",
            "172.31.255.254",
            "192.168.1.50",
            "100.64.0.1",
            "100.127.255.254",
            "169.254.1.10",
            "::1",
            "::",
            "fe80::1",
            "fc00::1",
            "fd12:3456:789a::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::ffff:192.168.1.1",
        ] {
            let parsed: IpAddr = priv_ip.parse().unwrap();
            assert!(
                is_private_or_loopback_ip(parsed),
                "Expected private/loopback classification for {}",
                priv_ip
            );
        }

        for pub_ip in [
            "93.184.216.34",
            "1.1.1.1",
            "8.8.8.8",
            "172.32.0.1",
            "100.128.0.1",
            "2606:2800:220:1:248:1893:25c8:1946",
        ] {
            let parsed: IpAddr = pub_ip.parse().unwrap();
            assert!(!is_cloud_metadata_ip(parsed));
            assert!(!is_private_or_loopback_ip(parsed));
        }
    }

    #[test]
    fn test_nsec8_noncanonical_ip_and_special_cidr_ssrf_blocking() {
        use std::net::IpAddr;

        // Octal, hex, dword, and shorthand metadata IPs must be caught as cloud metadata targets
        for meta_bypass in [
            "http://2852039166/latest/meta-data/",
            "http://0251.0376.0251.0376/",
            "http://0xa9.0xfe.0xa9.0xfe/",
            "http://169.254.43518/",
            "http://[::ffff:169.254.169.254]/",
        ] {
            assert!(
                is_cloud_metadata_ssrf_target(meta_bypass),
                "Expected cloud metadata detection for {}",
                meta_bypass
            );
        }

        // Octal, hex, dword, shorthand loopback/private, and special-use CIDRs must be blocked under wildcard allow_net
        for ssrf_bypass in [
            "http://0177.0.0.1:8080/",
            "http://0x7f.0.0.1/",
            "http://2130706433/",
            "http://127.1/",
            "http://10.1/",
            "http://sub.localhost:3000/",
            "http://[fc00::1]/",
            "http://[fe80::1]/",
            "http://100.64.0.1/",
            "http://198.18.0.1/",
            "http://192.0.0.1/",
            "http://224.0.0.1/",
            "http://[ff02::1]/",
        ] {
            assert!(
                is_private_or_loopback_ssrf_target(ssrf_bypass),
                "Expected SSRF target classification for {}",
                ssrf_bypass
            );
        }

        // Verify special-use IPs in `is_private_or_loopback_ip`
        for special_ip in [
            "192.0.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "198.19.255.254",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "240.0.0.1",
            "ff02::1",
            "2001:db8::1",
        ] {
            let parsed: IpAddr = special_ip.parse().unwrap();
            assert!(
                is_private_or_loopback_ip(parsed),
                "Expected special-use IP blocking for {}",
                special_ip
            );
        }
    }

    #[test]
    fn test_canonical_vfs_resolver_lexical_normalization_and_boundary_guards() {
        assert_eq!(
            normalize_vfs_path_lexical(r"\\?\c:\Users\slice\.\data\..\config.json"),
            "C:/Users/slice/config.json"
        );
        assert_eq!(
            normalize_vfs_path_lexical("/tmp/./sub/../etc/../../etc/passwd"),
            "/etc/passwd"
        );
        assert_eq!(normalize_vfs_path_lexical("./a/b/../c/"), "a/c");

        // 1. Directory boundary enforcement: `/tmp` allows `/tmp` and `/tmp/file.txt`,
        //    but strictly blocks `/tmp_evil` and `/tmp/../etc/passwd`.
        assert!(vfs_prefix_matches("/tmp", "/tmp"));
        assert!(vfs_prefix_matches("/tmp/sub/file.txt", "/tmp"));
        assert!(!vfs_prefix_matches("/tmp_evil/file.txt", "/tmp"));
        assert!(!vfs_prefix_matches("/tmp/../etc/passwd", "/tmp"));

        // 2. Cross-platform `/tmp` <-> `std::env::temp_dir()` equivalence
        let os_tmp_child = std::env::temp_dir().join("slice_vfs_unit_check.txt");
        assert!(vfs_prefix_matches(&os_tmp_child.to_string_lossy(), "/tmp"));

        // 3. Relative `"."` prefix matching against workspace current_dir
        if let Ok(cwd) = std::env::current_dir() {
            let abs_in_cwd = cwd.join("Cargo.toml");
            assert!(vfs_prefix_matches(&abs_in_cwd.to_string_lossy(), "."));
            assert!(vfs_prefix_matches("Cargo.toml", &cwd.to_string_lossy()));
        }
    }
}
