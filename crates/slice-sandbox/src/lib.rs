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
//! - [`dlp`]: Multi-encoding outbound Egress DLP scanner ([`verify_egress_secrets_slice`]).
//! - [`vfs`]: Lexical VFS path normalization ([`normalize_vfs_path_lexical`]), directory-boundary OCap guard ([`vfs_prefix_matches`]), and subprocess [`exec_binary_matches`].
//! - [`net`]: Multi-layer SSRF shield ([`is_cloud_metadata_ssrf_target`], [`is_private_or_loopback_ssrf_target`]) and hostname/port parser ([`extract_clean_hostname`]).
//! - [`manifest`]: [`SliceManifest`] (`slice.toml`), [`InlineDirectives`] (`// @slice-...`), and `.env` Auto-DLP credential classifier ([`parse_dotenv_content`]).

#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

pub mod compartment;
pub mod dlp;
pub mod manifest;
pub mod net;
pub mod vfs;

pub use compartment::{
    CapabilityAttenuationRequest, CapabilityToken, HardwareProtectionDomain, PkruDomainGuard,
    SandboxViolation, TenantCompartment, current_thread_pkru, exec_binary_matches,
    has_exec_shell_metachar, is_frozen_intrinsic,
};
pub use dlp::{expand_egress_secret_variants, verify_egress_secrets_slice};
pub use manifest::{
    InlineDirectives, MIN_SENSITIVE_SECRET_LEN, SliceManifest, escape_json_string_into,
    format_json_error_body, is_auto_dlp_env_entry, is_sensitive_env_key, parse_bool_setting,
    parse_byte_size_usize, parse_dotenv_content, parse_inline_directives, parse_strict_u64,
    parse_tier_setting,
};
pub use net::{
    ParsedHttpUrl, ParsedNetConnectTarget, canonicalize_net_grant_entry, check_net_host_grant,
    extract_clean_hostname, extract_clean_hostname_and_port, is_cloud_metadata_clean_host,
    is_cloud_metadata_ip, is_cloud_metadata_ssrf_target, is_private_or_loopback_clean_host,
    is_private_or_loopback_ip, is_private_or_loopback_ssrf_target, parse_http_authority_and_path,
    parse_net_connect_target, parse_nonstandard_ipv4,
};
pub use vfs::{
    anchor_vfs_path_to_cwd, cached_normalized_cwd, is_abs_normalized_vfs_path,
    is_rejected_vfs_candidate, normalize_vfs_path_cow, normalize_vfs_path_lexical,
    vfs_normalized_candidate_matches_prefix, vfs_path_matches_any_prefix, vfs_prefix_matches,
};

#[cfg(test)]
#[path = "../tests/mod.rs"]
mod tests;
