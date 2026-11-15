//! Object Capabilities (OCap), Secure ECMAScript (SES), and Hardware-Assisted Memory Sandboxing
//!
//! Enforces multi-tenant isolation at the language and hardware architecture layers:
//! 1. SES `lockdown()` freezing intrinsic prototypes (`Object.prototype`, `Array.prototype`).
//! 2. Per-Tenant `Compartment` with attenuated capability tokens (`FsCap`, `NetCap`, `EnvCap`, `ExecCap`).
//! 3. Deterministic CPU instruction fuel limits & hard byte-level heap ceilings.
//! 4. Linux `x86_64` `PKRU` domain-switch scaffolding (`SYS_pkey_alloc` / `WRPKRU` in < 20 CPU
//!    cycles; `SYS_pkey_mprotect` heap slab page tagging is not yet wired, so tenant memory
//!    isolation today relies on safe-Rust bounds checking, SES lockdown, and byte-level cages).
//!
//! ## Domain Subsystems
//! - [`compartment`]: [`TenantCompartment`], [`CapabilityToken`], [`SandboxViolation`], SES [`is_frozen_intrinsic`], and [`HardwareProtectionDomain`].
//! - [`vfs`]: Lexical VFS path normalization ([`normalize_vfs_path_lexical`]), directory-boundary OCap guard ([`vfs_prefix_matches`]), and subprocess [`exec_binary_matches`].
//! - [`net`]: Multi-layer SSRF shield ([`is_cloud_metadata_ssrf_target`], [`is_private_or_loopback_ssrf_target`]) and hostname/port parser ([`extract_clean_hostname`]).
//! - [`manifest`]: [`SliceManifest`] (`slice.toml`), [`InlineDirectives`] (`// @slice-...`), and `.env` Auto-DLP credential classifier ([`parse_dotenv_content`]).

pub mod compartment;
pub mod manifest;
pub mod net;
pub mod vfs;

pub use compartment::{
    CapabilityToken, HardwareProtectionDomain, SandboxViolation, TenantCompartment,
    is_frozen_intrinsic,
};
pub use manifest::{
    InlineDirectives, SliceManifest, is_sensitive_env_key, parse_bool_setting,
    parse_dotenv_content, parse_inline_directives, parse_tier_setting,
};
pub use net::{
    ParsedHttpUrl, extract_clean_hostname, extract_clean_hostname_and_port, is_cloud_metadata_ip,
    is_cloud_metadata_ssrf_target, is_private_or_loopback_ip, is_private_or_loopback_ssrf_target,
    parse_http_authority_and_path, parse_nonstandard_ipv4,
};
pub use vfs::{
    anchor_vfs_path_to_cwd, cached_normalized_cwd, exec_binary_matches, is_abs_normalized_vfs_path,
    is_rejected_vfs_candidate, normalize_vfs_path_cow, normalize_vfs_path_lexical,
    vfs_normalized_candidate_matches_prefix, vfs_prefix_matches,
};

#[cfg(test)]
#[path = "../tests/mod.rs"]
mod tests;
