pub(crate) mod cas_packager;
pub(crate) mod ctrlc;
pub(crate) mod env_and_cli;
pub(crate) mod flags;
pub(crate) mod manifest_merge;
pub(crate) mod secrets_and_env;
pub(crate) mod watcher;

#[cfg(test)]
pub(crate) use cas_packager::build_cas_archive;
pub(crate) use cas_packager::{build_cas_archive_with_static, unique_temp_cas_path};
pub(crate) use ctrlc::install_ctrlc_handler;
#[cfg(any(test, target_os = "linux", target_os = "macos"))]
pub(crate) use flags::parse_u64_digits;
pub(crate) use flags::{
    is_known_slice_runtime_flag, match_non_empty_flag_path, match_non_empty_flag_value,
    parse_byte_size_usize, parse_strict_u64, skip_runtime_flag_with_separate_value,
};
pub(crate) use manifest_merge::find_project_manifest_path;
#[cfg(test)]
pub(crate) use manifest_merge::merge_manifest_toml_into_caps;
pub(crate) use secrets_and_env::{
    apply_dotenv_file_to_caps_and_dlp, apply_dotenv_file_with_precedence, load_audit_egress_file,
    maybe_register_sensitive_env_dlp, register_audit_egress_spec,
};
pub(crate) use watcher::OsNativeFileWatcher;
#[cfg(test)]
pub(crate) use watcher::collect_watch_fingerprint;

use slice_sandbox::CapabilityToken;
use std::path::PathBuf;

/// Unified execution configuration (`CapabilityToken`, `fuel_limit`, `mem_limit`, `audit_secrets`,
/// `warm_pool`, `deterministic_seed`, `server_port`, `server_workers`, `static_dir`, `tier`, `boacas`)
/// resolved across:
/// 1. Built-in Optimal Defaults (`Tier 1`, `boacas = true`, `fuel = 50_000_000`, `mem = 16 MB`)
/// 2. `./slice.toml`, target subdirectory `slice.toml`, or `.cas` embedded manifest
/// 3. Top-of-file inline source directives (`// @slice-...`)
/// 4. `SLICE_*` environment variables
/// 5. Explicit CLI inline flags (`--tier=`, `--boacas`, `--fuel=`, `--memory=`, `--allow-*`, etc.)
#[derive(Debug, Clone)]
pub(crate) struct ResolvedExecutionConfig {
    pub(crate) caps: CapabilityToken,
    pub(crate) fuel_limit: u64,
    pub(crate) mem_limit: usize,
    pub(crate) audit_secrets: Vec<String>,
    pub(crate) warm_pool: bool,
    pub(crate) deterministic_seed: Option<u64>,
    pub(crate) server_port: Option<u16>,
    pub(crate) server_workers: Option<usize>,
    pub(crate) static_dir: Option<PathBuf>,
    pub(crate) artifacts_dir: Option<PathBuf>,
    pub(crate) tls_cert: Option<PathBuf>,
    pub(crate) tls_key: Option<PathBuf>,
    pub(crate) trust_directives: bool,
    pub(crate) trace_output_path: Option<PathBuf>,
    pub(crate) has_explicit_config: bool,
    pub(crate) loaded_manifest_path: Option<PathBuf>,
    pub(crate) cli_fuel_override: Option<u64>,
    pub(crate) cli_mem_override: Option<usize>,
    pub(crate) cli_boacas_override: Option<bool>,
    pub(crate) cli_tier_override: Option<Option<u8>>,
    pub(crate) cli_warm_override: Option<bool>,
    pub(crate) cli_det_override: Option<Option<u64>>,
    pub(crate) cli_port_override: Option<u16>,
    pub(crate) cli_workers_override: Option<usize>,
    pub(crate) cli_static_override: Option<PathBuf>,
    pub(crate) cli_artifacts_dir_override: Option<PathBuf>,
    pub(crate) cli_tls_cert_override: Option<PathBuf>,
    pub(crate) cli_tls_key_override: Option<PathBuf>,
    pub(crate) cli_trust_directives_override: Option<bool>,
    pub(crate) cli_checkpoints_override: Option<bool>,
    pub(crate) cli_max_checkpoints_override: Option<u32>,
}

#[inline]
pub(crate) fn apply_boacas_override(enabled: Option<bool>) {
    #[cfg(test)]
    slice_engine::set_thread_exper_boacas_override(enabled);
    #[cfg(not(test))]
    slice_engine::set_exper_boacas_override(enabled);
}

#[inline]
pub(crate) fn apply_tier_override(tier: Option<u8>) {
    #[cfg(test)]
    slice_engine::set_thread_tier_override(tier);
    #[cfg(not(test))]
    slice_engine::set_tier_override(tier);
}
