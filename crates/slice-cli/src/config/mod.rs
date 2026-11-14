pub(crate) mod cas_packager;
pub(crate) mod flags;
pub(crate) mod secrets_and_env;
pub(crate) mod watcher;

#[cfg(test)]
pub(crate) use cas_packager::build_cas_archive;
pub(crate) use cas_packager::build_cas_archive_with_static;
#[cfg(test)]
pub(crate) use flags::parse_u64_digits;
pub(crate) use flags::{
    is_known_slice_runtime_flag, match_non_empty_flag_path, match_non_empty_flag_value,
    skip_runtime_flag_with_separate_value,
};
pub(crate) use secrets_and_env::{
    apply_dotenv_file_to_caps_and_dlp, load_audit_egress_file, maybe_register_sensitive_env_dlp,
    register_audit_egress_spec,
};
#[cfg(test)]
pub(crate) use watcher::collect_watch_fingerprint;
pub(crate) use watcher::OsNativeFileWatcher;

use anyhow::{Context, Result};
use slice_sandbox::{CapabilityToken, SliceManifest};
use std::path::{Path, PathBuf};

#[cfg(windows)]
mod win_ctrlc {
    extern "system" {
        fn SetConsoleCtrlHandler(
            handler: Option<unsafe extern "system" fn(u32) -> i32>,
            add: i32,
        ) -> i32;
    }

    unsafe extern "system" fn ctrl_handler(_ctrl_type: u32) -> i32 {
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let _ = std::io::Write::flush(&mut std::io::stderr());
        std::process::exit(0);
    }

    pub fn install() {
        unsafe {
            let _ = SetConsoleCtrlHandler(Some(ctrl_handler), 1);
        }
    }
}

/// Installs a cross-platform `Ctrl+C` / console termination signal handler so long-running
/// subcommands (`slice serve`, `slice run --watch`, `slice test --watch`) flush stdio and exit
/// cleanly without leaving worker threads blocked on sockets or freezing Windows consoles.
pub(crate) fn install_ctrlc_handler() {
    #[cfg(windows)]
    win_ctrlc::install();
}

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
    pub(crate) cli_tls_cert_override: Option<PathBuf>,
    pub(crate) cli_tls_key_override: Option<PathBuf>,
    pub(crate) cli_trust_directives_override: Option<bool>,
}

fn extract_explicit_config_path(args: &[String]) -> Result<Option<PathBuf>> {
    let mut found: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--" {
            break;
        }
        if let Some(cfg_path) = match_non_empty_flag_path(args, &mut i, "--config")? {
            found = Some(cfg_path);
        }
        i += 1;
    }
    Ok(found)
}

impl ResolvedExecutionConfig {
    pub(crate) fn from_cli_args(args: &[String]) -> Result<Self> {
        Self::from_cli_args_with_ambient_cwd(args, true)
    }

    pub(crate) fn from_cli_args_with_ambient_cwd(
        args: &[String],
        load_ambient_cwd: bool,
    ) -> Result<Self> {
        let explicit_config_path = extract_explicit_config_path(args)?;
        let has_explicit_config = explicit_config_path.is_some();

        let mut caps = CapabilityToken::pure_compute();
        let mut fuel_limit: u64 = 50_000_000;
        let mut mem_limit: usize = 16 * 1024 * 1024;
        let mut audit_secrets = Vec::new();
        let mut audit_egress_specs = Vec::new();
        let mut audit_egress_files = Vec::new();
        let mut env_files = Vec::new();
        let mut warm_pool = false;
        let mut deterministic_seed = None;
        let mut server_port = None;
        let mut server_workers = None;
        let mut static_dir = None;
        let mut tls_cert = None;
        let mut tls_key = None;
        let mut trust_directives = false;
        let mut trace_output_path = std::env::var("SLICE_TRACE")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);

        let mut cli_fuel_override = None;
        let mut cli_mem_override = None;
        let mut cli_boacas_override = None;
        let mut cli_tier_override = None;
        let mut cli_warm_override = None;
        let mut cli_det_override = None;
        let mut cli_port_override = None;
        let mut cli_workers_override = None;
        let mut cli_static_override = None;
        let mut cli_tls_cert_override = None;
        let mut cli_tls_key_override = None;
        let mut cli_trust_directives_override = None;

        // 1. Apply explicit `--config <path>` or `./slice.toml` (fail-closed if present and unreadable)
        let mut loaded_manifest_path: Option<PathBuf> = None;
        let loaded_manifest_opt: Option<(String, PathBuf)> =
            if let Some(ref cfg_path) = explicit_config_path {
                let raw = std::fs::read_to_string(cfg_path)
                    .with_context(|| format!("Failed to read --config {}", cfg_path.display()))?;
                loaded_manifest_path =
                    Some(std::fs::canonicalize(cfg_path).unwrap_or_else(|_| cfg_path.clone()));
                let base = cfg_path
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf();
                Some((raw, base))
            } else if load_ambient_cwd {
                let cwd_manifest = Path::new("slice.toml");
                if cwd_manifest.exists() {
                    let raw = std::fs::read_to_string(cwd_manifest).with_context(|| {
                        format!("Failed to read manifest {}", cwd_manifest.display())
                    })?;
                    loaded_manifest_path = Some(
                        std::fs::canonicalize(cwd_manifest)
                            .unwrap_or_else(|_| cwd_manifest.to_path_buf()),
                    );
                    Some((raw, PathBuf::from(".")))
                } else {
                    None
                }
            } else {
                None
            };

        if let Some((raw, manifest_base)) = loaded_manifest_opt {
            let m = SliceManifest::parse_toml(&raw);
            fuel_limit = m.fuel_limit;
            mem_limit = m.memory_limit_kb * 1024;
            if let Some(w) = m.warm_pool {
                warm_pool = w;
            }
            if let Some(s) = m.deterministic_seed {
                deterministic_seed = Some(s);
            }
            if let Some(p) = m.server_port {
                server_port = Some(p);
            }
            if let Some(wk) = m.server_workers {
                server_workers = Some(wk);
            }
            if let Some(ref sd) = m.static_dir {
                static_dir = Some(manifest_base.join(sd));
            }
            if let Some(ref tc) = m.tls_cert {
                tls_cert = Some(manifest_base.join(tc));
            }
            if let Some(ref tk) = m.tls_key {
                tls_key = Some(manifest_base.join(tk));
            }
            if let Some(td) = m.trust_directives {
                trust_directives = td;
            }
            if let Some(b) = m.exper_boacas {
                apply_boacas_override(Some(b));
            }
            if let Some(t) = m.tier {
                apply_tier_override(Some(t));
            }
            merge_parsed_manifest_into_caps_and_dlp(
                m,
                Some(manifest_base.as_path()),
                &mut caps,
                &mut audit_secrets,
            )?;
        }

        // 2. Apply `SLICE_*` environment variables (override slice.toml, overridden by CLI flags)
        if let Ok(v) = std::env::var("SLICE_FUEL") {
            if let Some(parsed) = flags::parse_u64_digits(&v) {
                fuel_limit = parsed;
                cli_fuel_override = Some(parsed);
            }
        }
        if let Ok(v) = std::env::var("SLICE_MEMORY_KB") {
            if let Some(parsed) = flags::parse_u64_digits(&v) {
                let bytes = usize::try_from(parsed)
                    .unwrap_or(usize::MAX)
                    .saturating_mul(1024);
                mem_limit = bytes;
                cli_mem_override = Some(bytes);
            }
        }
        if let Ok(v) = std::env::var("SLICE_MEMORY") {
            if let Some(parsed) = flags::parse_u64_digits(&v) {
                let bytes = usize::try_from(parsed).unwrap_or(usize::MAX);
                mem_limit = bytes;
                cli_mem_override = Some(bytes);
            }
        }
        if let Ok(v) = std::env::var("SLICE_ALLOW_READ") {
            for item in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                caps = caps.with_read_prefix(item);
            }
        }
        if let Ok(v) = std::env::var("SLICE_ALLOW_WRITE") {
            for item in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                caps = caps.with_write_prefix(item);
            }
        }
        if let Ok(v) = std::env::var("SLICE_ALLOW_NET") {
            for item in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                caps = caps.with_net_host(item);
            }
        }
        if let Ok(v) = std::env::var("SLICE_ALLOW_EXEC") {
            for item in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                if !caps.allowed_exec_binaries.contains(&item.to_string()) {
                    caps = caps.with_exec_binary(item);
                }
            }
        }
        if let Ok(v) = std::env::var("SLICE_ALLOW_ENV") {
            for item in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                if let Some((k, val)) = item.split_once('=') {
                    let kt = k.trim();
                    let vt = val.trim();
                    maybe_register_sensitive_env_dlp(kt, vt, &mut audit_secrets);
                    caps = caps.with_env(kt, vt);
                } else {
                    let env_val = std::env::var(item).unwrap_or_default();
                    maybe_register_sensitive_env_dlp(item, &env_val, &mut audit_secrets);
                    caps = caps.with_env(item, env_val);
                }
            }
        }
        if let Ok(v) = std::env::var("SLICE_ENV_FILE") {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                env_files.push(PathBuf::from(trimmed));
            }
        }
        if let Ok(v) = std::env::var("SLICE_AUDIT_EGRESS_FILE") {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                audit_egress_files.push(PathBuf::from(trimmed));
            }
        }
        if let Ok(v) = std::env::var("SLICE_AUDIT_EGRESS") {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                audit_egress_specs.push(trimmed.to_string());
            }
        }
        if let Ok(v) = std::env::var("SLICE_CHECKPOINTS") {
            if let Some(b) = slice_sandbox::parse_bool_setting(&v) {
                caps = caps.with_checkpoints(b);
            }
        }
        if let Ok(v) = std::env::var("SLICE_MAX_CHECKPOINTS") {
            if let Some(n) = flags::parse_u64_digits(&v) {
                caps = caps.with_max_checkpoints(Some(u32::try_from(n).unwrap_or(u32::MAX)));
            }
        }
        if let Ok(v) = std::env::var("SLICE_WARM") {
            if let Some(b) = slice_sandbox::parse_bool_setting(&v) {
                warm_pool = b;
                cli_warm_override = Some(b);
            }
        }
        if let Ok(v) = std::env::var("SLICE_DETERMINISTIC_SEED") {
            if let Some(seed) = flags::parse_u64_digits(&v) {
                deterministic_seed = Some(seed);
                cli_det_override = Some(Some(seed));
            }
        }
        if let Ok(v) = std::env::var("SLICE_PORT") {
            if let Ok(p) = v.trim().parse::<u16>() {
                server_port = Some(p);
                cli_port_override = Some(p);
            }
        }
        if let Ok(v) = std::env::var("SLICE_WORKERS") {
            if let Ok(w) = v.trim().parse::<usize>() {
                let wk = w.max(1);
                server_workers = Some(wk);
                cli_workers_override = Some(wk);
            }
        }
        if let Ok(v) = std::env::var("SLICE_STATIC_DIR") {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                let p = PathBuf::from(trimmed);
                static_dir = Some(p.clone());
                cli_static_override = Some(p);
            }
        }
        if let Ok(v) = std::env::var("SLICE_TLS_CERT") {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                let p = PathBuf::from(trimmed);
                tls_cert = Some(p.clone());
                cli_tls_cert_override = Some(p);
            }
        }
        if let Ok(v) = std::env::var("SLICE_TLS_KEY") {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                let p = PathBuf::from(trimmed);
                tls_key = Some(p.clone());
                cli_tls_key_override = Some(p);
            }
        }
        if let Ok(v) =
            std::env::var("SLICE_BOACAS").or_else(|_| std::env::var("SLICE_EXPER_BOACAS"))
        {
            if let Some(b) = slice_sandbox::parse_bool_setting(&v) {
                cli_boacas_override = Some(b);
                apply_boacas_override(Some(b));
            }
        }
        if let Ok(v) = std::env::var("SLICE_TIER") {
            if let Some(t) = slice_sandbox::parse_tier_setting(&v) {
                cli_tier_override = Some(t);
                apply_tier_override(t);
            }
        }
        if let Ok(v) = std::env::var("SLICE_TRUST_DIRECTIVES") {
            if let Some(b) = slice_sandbox::parse_bool_setting(&v) {
                trust_directives = b;
                cli_trust_directives_override = Some(b);
            }
        }

        // 3. Apply explicit CLI inline flags (highest precedence, fail-closed on empty/missing file paths)
        let mut arg_idx = 0;
        while arg_idx < args.len() {
            if args[arg_idx] == "--" {
                break;
            }
            if match_non_empty_flag_path(args, &mut arg_idx, "--config")?.is_some() {
                arg_idx += 1;
                continue;
            }
            if let Some(ef) = match_non_empty_flag_path(args, &mut arg_idx, "--env-file")? {
                env_files.push(ef);
                arg_idx += 1;
                continue;
            }
            if let Some(sf) = match_non_empty_flag_path(args, &mut arg_idx, "--audit-egress-file")?
            {
                audit_egress_files.push(sf);
                arg_idx += 1;
                continue;
            }
            if let Some(sec) = match_non_empty_flag_value(args, &mut arg_idx, "--audit-egress")? {
                let sec_owned = sec.to_string();
                audit_egress_specs.push(sec_owned);
                arg_idx += 1;
                continue;
            }
            if let Some(tr) = match_non_empty_flag_path(args, &mut arg_idx, "--trace")? {
                trace_output_path = Some(tr);
                arg_idx += 1;
                continue;
            }

            let arg = &args[arg_idx];
            if let Some(prefix) = arg.strip_prefix("--allow-read=") {
                caps = caps.with_read_prefix(prefix);
            } else if let Some(prefix) = arg.strip_prefix("--allow-write=") {
                caps = caps.with_write_prefix(prefix);
            } else if let Some(host) = arg.strip_prefix("--allow-net=") {
                for h in host.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    caps = caps.with_net_host(h);
                }
            } else if let Some(bins) = arg.strip_prefix("--allow-exec=") {
                for b in bins.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    if !caps.allowed_exec_binaries.contains(&b.to_string()) {
                        caps = caps.with_exec_binary(b);
                    }
                }
            } else if let Some(kv) = arg.strip_prefix("--allow-env=") {
                if let Some((k, v)) = kv.split_once('=') {
                    maybe_register_sensitive_env_dlp(k, v, &mut audit_secrets);
                    caps = caps.with_env(k, v);
                } else {
                    let k = kv.trim();
                    let v = std::env::var(k).unwrap_or_default();
                    maybe_register_sensitive_env_dlp(k, &v, &mut audit_secrets);
                    caps = caps.with_env(k, v);
                }
            } else if arg == "--no-checkpoints" {
                caps = caps.with_checkpoints(false);
            } else if arg == "--checkpoints" {
                caps = caps.with_checkpoints(true);
            } else if let Some(cp_str) = arg.strip_prefix("--checkpoints=") {
                if let Some(b) = slice_sandbox::parse_bool_setting(cp_str) {
                    caps = caps.with_checkpoints(b);
                }
            } else if let Some(mcp_str) = arg.strip_prefix("--max-checkpoints=") {
                if let Some(n) = flags::parse_u64_digits(mcp_str) {
                    caps = caps.with_max_checkpoints(Some(u32::try_from(n).unwrap_or(u32::MAX)));
                }
            } else if let Some(f) = arg.strip_prefix("--fuel=") {
                if let Some(parsed) = flags::parse_u64_digits(f) {
                    fuel_limit = parsed;
                    cli_fuel_override = Some(parsed);
                }
            } else if let Some(m) = arg.strip_prefix("--memory=") {
                if let Some(parsed) = flags::parse_u64_digits(m) {
                    let bytes = usize::try_from(parsed).unwrap_or(usize::MAX);
                    mem_limit = bytes;
                    cli_mem_override = Some(bytes);
                }
            } else if let Some(mkb) = arg.strip_prefix("--memory-kb=") {
                if let Some(parsed) = flags::parse_u64_digits(mkb) {
                    let bytes = usize::try_from(parsed)
                        .unwrap_or(usize::MAX)
                        .saturating_mul(1024);
                    mem_limit = bytes;
                    cli_mem_override = Some(bytes);
                }
            } else if arg == "--boacas" || arg == "--exper-boacas" {
                cli_boacas_override = Some(true);
                apply_boacas_override(Some(true));
            } else if arg == "--no-boacas" || arg == "--no-exper-boacas" {
                cli_boacas_override = Some(false);
                apply_boacas_override(Some(false));
            } else if let Some(b_str) = arg
                .strip_prefix("--boacas=")
                .or_else(|| arg.strip_prefix("--exper-boacas="))
            {
                if let Some(b) = slice_sandbox::parse_bool_setting(b_str) {
                    cli_boacas_override = Some(b);
                    apply_boacas_override(Some(b));
                }
            } else if let Some(t_str) = arg.strip_prefix("--tier=") {
                if let Some(t) = slice_sandbox::parse_tier_setting(t_str) {
                    cli_tier_override = Some(t);
                    apply_tier_override(t);
                }
            } else if arg == "--warm" {
                warm_pool = true;
                cli_warm_override = Some(true);
            } else if arg == "--no-warm" {
                warm_pool = false;
                cli_warm_override = Some(false);
            } else if let Some(seed_str) = arg.strip_prefix("--deterministic=") {
                let s = seed_str.parse().unwrap_or(42);
                deterministic_seed = Some(s);
                cli_det_override = Some(Some(s));
            } else if arg == "--deterministic" {
                deterministic_seed = Some(42);
                cli_det_override = Some(Some(42));
            } else if arg == "--no-deterministic" {
                deterministic_seed = None;
                cli_det_override = Some(None);
            } else if let Some(p_str) = arg.strip_prefix("--port=") {
                if let Ok(p) = p_str.parse::<u16>() {
                    server_port = Some(p);
                    cli_port_override = Some(p);
                }
            } else if let Some(w_str) = arg.strip_prefix("--workers=") {
                if let Ok(w) = w_str.parse::<usize>() {
                    let wk = w.max(1);
                    server_workers = Some(wk);
                    cli_workers_override = Some(wk);
                }
            } else if let Some(sd_str) = arg.strip_prefix("--static=") {
                let p = PathBuf::from(sd_str);
                static_dir = Some(p.clone());
                cli_static_override = Some(p);
            } else if let Some(tc_str) = arg.strip_prefix("--tls-cert=") {
                let p = PathBuf::from(tc_str);
                tls_cert = Some(p.clone());
                cli_tls_cert_override = Some(p);
            } else if let Some(tk_str) = arg.strip_prefix("--tls-key=") {
                let p = PathBuf::from(tk_str);
                tls_key = Some(p.clone());
                cli_tls_key_override = Some(p);
            } else if arg == "--trust-directives" {
                trust_directives = true;
                cli_trust_directives_override = Some(true);
            } else if arg == "--no-trust-directives"
                || arg == "--no-directives"
                || arg == "--untrusted"
            {
                trust_directives = false;
                cli_trust_directives_override = Some(false);
            } else if let Some(td_str) = arg.strip_prefix("--trust-directives=") {
                if let Some(b) = slice_sandbox::parse_bool_setting(td_str) {
                    trust_directives = b;
                    cli_trust_directives_override = Some(b);
                }
            }
            arg_idx += 1;
        }

        for ef in &env_files {
            apply_dotenv_file_to_caps_and_dlp(ef, &mut caps, &mut audit_secrets)?;
        }
        for sf in &audit_egress_files {
            load_audit_egress_file(sf, &mut audit_secrets)?;
        }
        let cli_audit_base = explicit_config_path
            .as_deref()
            .and_then(|p| p.parent().filter(|d| !d.as_os_str().is_empty()))
            .or_else(|| {
                if load_ambient_cwd {
                    Some(Path::new("."))
                } else {
                    None
                }
            });
        for spec in &audit_egress_specs {
            register_audit_egress_spec(spec, cli_audit_base, &caps, &mut audit_secrets)?;
        }

        Ok(Self {
            caps,
            fuel_limit,
            mem_limit,
            audit_secrets,
            warm_pool,
            deterministic_seed,
            server_port,
            server_workers,
            static_dir,
            tls_cert,
            tls_key,
            trust_directives,
            trace_output_path,
            has_explicit_config,
            loaded_manifest_path,
            cli_fuel_override,
            cli_mem_override,
            cli_boacas_override,
            cli_tier_override,
            cli_warm_override,
            cli_det_override,
            cli_port_override,
            cli_workers_override,
            cli_static_override,
            cli_tls_cert_override,
            cli_tls_key_override,
            cli_trust_directives_override,
        })
    }

    /// Merges a target project's `slice.toml` or embedded `.cas` manifest TOML into this
    /// configuration, updating settings unless already overridden by `--config`, `SLICE_*` env vars, or CLI flags.
    pub(crate) fn apply_manifest_toml(
        &mut self,
        raw_toml: &str,
        base_dir: Option<&Path>,
    ) -> Result<()> {
        let m = SliceManifest::parse_toml(raw_toml);
        if !self.has_explicit_config {
            if self.cli_fuel_override.is_none() {
                self.fuel_limit = m.fuel_limit;
            }
            if self.cli_mem_override.is_none() {
                self.mem_limit = m.memory_limit_kb * 1024;
            }
            if self.cli_boacas_override.is_none() {
                if let Some(b) = m.exper_boacas {
                    apply_boacas_override(Some(b));
                }
            }
            if self.cli_tier_override.is_none() {
                if let Some(t) = m.tier {
                    apply_tier_override(Some(t));
                }
            }
            if self.cli_warm_override.is_none() {
                if let Some(w) = m.warm_pool {
                    self.warm_pool = w;
                }
            }
            if self.cli_det_override.is_none() {
                if let Some(s) = m.deterministic_seed {
                    self.deterministic_seed = Some(s);
                }
            }
            if self.cli_port_override.is_none() {
                if let Some(p) = m.server_port {
                    self.server_port = Some(p);
                }
            }
            if self.cli_workers_override.is_none() {
                if let Some(w) = m.server_workers {
                    self.server_workers = Some(w);
                }
            }
            if let Some(dir) = base_dir {
                if self.cli_static_override.is_none() {
                    if let Some(ref sd) = m.static_dir {
                        self.static_dir = Some(dir.join(sd));
                    }
                }
                if self.cli_tls_cert_override.is_none() {
                    if let Some(ref tc) = m.tls_cert {
                        self.tls_cert = Some(dir.join(tc));
                    }
                }
                if self.cli_tls_key_override.is_none() {
                    if let Some(ref tk) = m.tls_key {
                        self.tls_key = Some(dir.join(tk));
                    }
                }
            }
            if self.cli_trust_directives_override.is_none() {
                if let Some(td) = m.trust_directives {
                    self.trust_directives = td;
                }
            }
        }
        merge_parsed_manifest_into_caps_and_dlp(
            m,
            base_dir,
            &mut self.caps,
            &mut self.audit_secrets,
        )
    }

    /// Applies top-of-file inline source directives (`// @slice-...`) from an entrypoint script,
    /// overriding `slice.toml` defaults while respecting `SLICE_*` env vars and CLI flags.
    pub(crate) fn apply_inline_directives(
        &mut self,
        source: &str,
        base_dir: Option<&Path>,
    ) -> Result<()> {
        let trimmed = source.trim_start_matches('\u{feff}').trim_start();
        if !trimmed.starts_with("//")
            && !trimmed.starts_with("/*")
            && !trimmed.starts_with("#!")
            && !trimmed.starts_with("\"use strict\"")
            && !trimmed.starts_with("'use strict'")
        {
            return Ok(());
        }
        let dirs = slice_sandbox::parse_inline_directives(source);
        if self.cli_fuel_override.is_none() {
            if let Some(f) = dirs.fuel_limit {
                if self.trust_directives {
                    self.fuel_limit = f;
                } else {
                    self.fuel_limit = self.fuel_limit.min(f);
                }
            }
        }
        if self.cli_mem_override.is_none() {
            if let Some(m) = dirs.memory_limit_bytes {
                if self.trust_directives {
                    self.mem_limit = m;
                } else {
                    self.mem_limit = self.mem_limit.min(m);
                }
            }
        }
        if self.cli_boacas_override.is_none() {
            if let Some(b) = dirs.boacas {
                apply_boacas_override(Some(b));
            }
        }
        if self.cli_tier_override.is_none() {
            if let Some(t) = dirs.tier {
                apply_tier_override(t);
            }
        }
        if self.cli_warm_override.is_none() {
            if let Some(w) = dirs.warm {
                self.warm_pool = w;
            }
        }
        if self.cli_det_override.is_none() {
            if let Some(s) = dirs.deterministic_seed {
                self.deterministic_seed = Some(s);
            }
        }
        if self.cli_port_override.is_none() {
            if let Some(p) = dirs.port {
                self.server_port = Some(p);
            }
        }
        if self.cli_workers_override.is_none() {
            if let Some(w) = dirs.workers {
                self.server_workers = Some(w);
            }
        }
        for sec in dirs.audit_egress {
            register_audit_egress_spec(&sec, base_dir, &self.caps, &mut self.audit_secrets)?;
        }

        if self.trust_directives {
            for r in dirs.allow_read {
                if !self.caps.allowed_read_prefixes.contains(&r) {
                    self.caps.allowed_read_prefixes.push(r);
                }
            }
            for w in dirs.allow_write {
                if !self.caps.allowed_write_prefixes.contains(&w) {
                    self.caps.allowed_write_prefixes.push(w);
                }
            }
            for nh in dirs.allow_net {
                self.caps.allowed_net_hosts.insert(nh);
            }
            for eb in dirs.allow_exec {
                if !self.caps.allowed_exec_binaries.contains(&eb) {
                    self.caps.allowed_exec_binaries.push(eb);
                }
            }
            for (k, v) in dirs.allow_env {
                maybe_register_sensitive_env_dlp(&k, &v, &mut self.audit_secrets);
                self.caps.allowed_env_keys.entry(k).or_insert(v);
            }
            if let Some(ef) = dirs.env_file {
                let env_path = match base_dir {
                    Some(dir) => dir.join(&ef),
                    None => PathBuf::from(&ef),
                };
                apply_dotenv_file_to_caps_and_dlp(
                    &env_path,
                    &mut self.caps,
                    &mut self.audit_secrets,
                )?;
            }
            if self.cli_tls_cert_override.is_none() {
                if let Some(tc) = dirs.tls_cert {
                    let p = match base_dir {
                        Some(dir) => dir.join(&tc),
                        None => PathBuf::from(&tc),
                    };
                    self.tls_cert = Some(p);
                }
            }
            if self.cli_tls_key_override.is_none() {
                if let Some(tk) = dirs.tls_key {
                    let p = match base_dir {
                        Some(dir) => dir.join(&tk),
                        None => PathBuf::from(&tk),
                    };
                    self.tls_key = Some(p);
                }
            }
        } else {
            // Untrusted mode (default): monotonic capability attenuation only (never widens host caps)
            if !dirs.allow_read.is_empty() && !self.caps.allowed_read_prefixes.is_empty() {
                let narrowed: Vec<String> = dirs
                    .allow_read
                    .into_iter()
                    .filter(|r| {
                        self.caps
                            .allowed_read_prefixes
                            .iter()
                            .any(|p| slice_sandbox::vfs_prefix_matches(r, p))
                    })
                    .collect();
                if !narrowed.is_empty() {
                    self.caps.allowed_read_prefixes = narrowed;
                }
            }
            if !dirs.allow_write.is_empty() && !self.caps.allowed_write_prefixes.is_empty() {
                let narrowed: Vec<String> = dirs
                    .allow_write
                    .into_iter()
                    .filter(|w| {
                        self.caps
                            .allowed_write_prefixes
                            .iter()
                            .any(|p| slice_sandbox::vfs_prefix_matches(w, p))
                    })
                    .collect();
                if !narrowed.is_empty() {
                    self.caps.allowed_write_prefixes = narrowed;
                }
            }
            if !dirs.allow_net.is_empty() && !self.caps.allowed_net_hosts.is_empty() {
                let narrowed: Vec<String> = dirs
                    .allow_net
                    .into_iter()
                    .filter(|nh| {
                        !slice_sandbox::is_cloud_metadata_ssrf_target(nh)
                            && (self.caps.allowed_net_hosts.contains(nh)
                                || self.caps.allowed_net_hosts.contains("*"))
                    })
                    .collect();
                if !narrowed.is_empty() {
                    self.caps.allowed_net_hosts.clear();
                    for nh in narrowed {
                        self.caps.allowed_net_hosts.insert(nh);
                    }
                }
            }
            if !dirs.allow_exec.is_empty() && !self.caps.allowed_exec_binaries.is_empty() {
                let narrowed: Vec<String> = dirs
                    .allow_exec
                    .into_iter()
                    .filter(|eb| {
                        self.caps
                            .allowed_exec_binaries
                            .iter()
                            .any(|allowed| slice_sandbox::exec_binary_matches(eb, allowed))
                    })
                    .collect();
                if !narrowed.is_empty() {
                    self.caps.allowed_exec_binaries = narrowed;
                }
            }
        }
        Ok(())
    }

    /// Discovers and applies `slice.toml` for `target_path` (if one exists in `target_path`,
    /// its parent directory, or its grandparent directory), skipping a duplicate disk read and
    /// TOML parse when the discovered manifest is identical to the ambient `./slice.toml` already loaded.
    pub(crate) fn apply_target_path_manifest(&mut self, target_path: &Path) -> Result<()> {
        if self.has_explicit_config {
            return Ok(());
        }
        if let Some(manifest_file) = find_project_manifest_path(target_path) {
            let canon =
                std::fs::canonicalize(&manifest_file).unwrap_or_else(|_| manifest_file.clone());
            if self.loaded_manifest_path.as_deref() == Some(canon.as_path()) {
                return Ok(());
            }
            let raw = std::fs::read_to_string(&manifest_file)
                .with_context(|| format!("Failed to read manifest {}", manifest_file.display()))?;
            self.loaded_manifest_path = Some(canon);
            self.apply_manifest_toml(&raw, manifest_file.parent())?;
        }
        Ok(())
    }
}

pub(crate) fn load_manifest_or_default() -> Result<(CapabilityToken, u64, usize)> {
    let manifest_path = Path::new("slice.toml");
    if manifest_path.exists() {
        let raw = std::fs::read_to_string(manifest_path)
            .with_context(|| format!("Failed to read manifest {}", manifest_path.display()))?;
        let m = SliceManifest::parse_toml(&raw);
        return Ok((m.capabilities, m.fuel_limit, m.memory_limit_kb * 1024));
    }
    Ok((
        CapabilityToken::pure_compute(),
        50_000_000,
        16 * 1024 * 1024,
    ))
}

#[cfg(test)]
pub(crate) fn merge_manifest_toml_into_caps(raw_toml: &str, caps: &mut CapabilityToken) {
    let mut dummy_secrets = Vec::new();
    merge_manifest_toml_into_caps_and_dlp(raw_toml, None, caps, &mut dummy_secrets)
        .expect("in-memory manifest TOML merge should not fail");
}

#[cfg(test)]
pub(crate) fn merge_manifest_toml_into_caps_and_dlp(
    raw_toml: &str,
    base_dir: Option<&Path>,
    caps: &mut CapabilityToken,
    audit_secrets: &mut Vec<String>,
) -> Result<()> {
    let m = SliceManifest::parse_toml(raw_toml);
    merge_parsed_manifest_into_caps_and_dlp(m, base_dir, caps, audit_secrets)
}

pub(crate) fn merge_parsed_manifest_into_caps_and_dlp(
    m: SliceManifest,
    base_dir: Option<&Path>,
    caps: &mut CapabilityToken,
    audit_secrets: &mut Vec<String>,
) -> Result<()> {
    for r in m.capabilities.allowed_read_prefixes {
        if !caps.allowed_read_prefixes.contains(&r) {
            caps.allowed_read_prefixes.push(r);
        }
    }
    for w in m.capabilities.allowed_write_prefixes {
        if !caps.allowed_write_prefixes.contains(&w) {
            caps.allowed_write_prefixes.push(w);
        }
    }
    for nh in m.capabilities.allowed_net_hosts {
        caps.allowed_net_hosts.insert(nh);
    }
    for eb in m.capabilities.allowed_exec_binaries {
        if !caps.allowed_exec_binaries.contains(&eb) {
            caps.allowed_exec_binaries.push(eb);
        }
    }
    if !m.capabilities.allow_checkpoints {
        caps.allow_checkpoints = false;
        caps.max_checkpoints = Some(0);
    }
    if let Some(max_cp) = m.capabilities.max_checkpoints {
        caps.max_checkpoints = Some(match caps.max_checkpoints {
            Some(existing) => existing.min(max_cp),
            None => max_cp,
        });
        if max_cp == 0 {
            caps.allow_checkpoints = false;
        }
    }
    for (k, v) in m.capabilities.allowed_env_keys {
        maybe_register_sensitive_env_dlp(&k, &v, audit_secrets);
        caps.allowed_env_keys.entry(k).or_insert(v);
    }
    for sec in m.audit_egress {
        register_audit_egress_spec(&sec, base_dir, caps, audit_secrets)?;
    }
    if let (Some(env_file_rel), Some(dir)) = (m.env_file, base_dir) {
        let env_path = dir.join(&env_file_rel);
        apply_dotenv_file_to_caps_and_dlp(&env_path, caps, audit_secrets)?;
    }
    Ok(())
}

pub(crate) fn find_project_manifest_path(input_path: &Path) -> Option<PathBuf> {
    if input_path.is_dir() {
        let candidate = input_path.join("slice.toml");
        if candidate.exists() {
            return Some(candidate);
        }
    } else if let Some(parent) = input_path.parent() {
        let candidate = parent.join("slice.toml");
        if candidate.exists() {
            return Some(candidate);
        }
        if let Some(grandparent) = parent.parent() {
            let candidate2 = grandparent.join("slice.toml");
            if candidate2.exists() {
                return Some(candidate2);
            }
        }
    }
    None
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
