//! Environment variable (`SLICE_*`) and CLI argument resolution for `ResolvedExecutionConfig`.

use super::{
    ResolvedExecutionConfig, apply_boacas_override, apply_dotenv_file_with_precedence,
    apply_tier_override, load_audit_egress_file, match_non_empty_flag_path,
    match_non_empty_flag_value, maybe_register_sensitive_env_dlp, parse_byte_size_usize,
    parse_strict_u64, register_audit_egress_spec,
};
use anyhow::{Context, Result};
use slice_sandbox::CapabilityToken;
use std::path::{Path, PathBuf};

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
    fn default_with_trace(trace_output_path: Option<PathBuf>) -> Self {
        Self {
            caps: CapabilityToken::pure_compute(),
            fuel_limit: 50_000_000,
            mem_limit: 16 * 1024 * 1024,
            audit_secrets: Vec::new(),
            warm_pool: false,
            deterministic_seed: None,
            server_port: None,
            server_workers: None,
            static_dir: None,
            tls_cert: None,
            tls_key: None,
            trust_directives: false,
            trace_output_path,
            has_explicit_config: false,
            loaded_manifest_path: None,
            cli_fuel_override: None,
            cli_mem_override: None,
            cli_boacas_override: None,
            cli_tier_override: None,
            cli_warm_override: None,
            cli_det_override: None,
            cli_port_override: None,
            cli_workers_override: None,
            cli_static_override: None,
            cli_tls_cert_override: None,
            cli_tls_key_override: None,
            cli_trust_directives_override: None,
            cli_checkpoints_override: None,
            cli_max_checkpoints_override: None,
        }
    }

    pub(crate) fn from_cli_args(args: &[String]) -> Result<Self> {
        Self::from_cli_args_with_ambient_cwd(args, true)
    }

    pub(crate) fn from_cli_args_with_ambient_cwd(
        args: &[String],
        load_ambient_cwd: bool,
    ) -> Result<Self> {
        let explicit_config_path = extract_explicit_config_path(args)?;
        let has_explicit_config = explicit_config_path.is_some();
        let trace_output_path = std::env::var("SLICE_TRACE")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);

        let mut cfg = Self::default_with_trace(trace_output_path);
        let mut audit_egress_specs = Vec::new();
        let mut audit_egress_files = Vec::new();
        let mut env_files = Vec::new();

        // 1. Apply explicit `--config <path>` or `./slice.toml` via shared `apply_manifest_toml`
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
            cfg.apply_manifest_toml(&raw, Some(manifest_base.as_path()))?;
        }
        cfg.has_explicit_config = has_explicit_config;
        cfg.loaded_manifest_path = loaded_manifest_path;

        // 2. Apply `SLICE_*` environment variables cached once per process via `OnceLock`
        //    (eliminates 22 per-invocation `getenv` locks and heap allocations).
        static CACHED_SLICE_ENV_VARS: std::sync::OnceLock<Vec<(String, String)>> =
            std::sync::OnceLock::new();
        let slice_env_vars = CACHED_SLICE_ENV_VARS.get_or_init(|| {
            let mut out = Vec::new();
            for (os_k, os_v) in std::env::vars_os() {
                let Some(k_str) = os_k.to_str() else {
                    continue;
                };
                let is_slice_var = if cfg!(windows) {
                    k_str.len() >= 6 && k_str.as_bytes()[..6].eq_ignore_ascii_case(b"SLICE_")
                } else {
                    k_str.starts_with("SLICE_")
                };
                if is_slice_var && let Ok(v_str) = os_v.into_string() {
                    let norm_k = if cfg!(windows) {
                        k_str.to_ascii_uppercase()
                    } else {
                        k_str.to_string()
                    };
                    out.push((norm_k, v_str));
                }
            }
            out
        });

        if !slice_env_vars.is_empty() {
            let find_slice_env = |name: &str| -> Option<&str> {
                slice_env_vars
                    .iter()
                    .rev()
                    .find(|(k, _)| k == name)
                    .map(|(_, v)| v.as_str())
            };

            if let Some(v) = find_slice_env("SLICE_FUEL")
                && !v.trim().is_empty()
            {
                let parsed = parse_strict_u64(v)
                    .with_context(|| format!("Invalid SLICE_FUEL integer value: {:?}", v.trim()))?;
                cfg.fuel_limit = parsed;
                cfg.cli_fuel_override = Some(parsed);
            }
            if let Some(v) = find_slice_env("SLICE_MEMORY_KB")
                && !v.trim().is_empty()
            {
                let parsed = parse_strict_u64(v).with_context(|| {
                    format!("Invalid SLICE_MEMORY_KB integer value: {:?}", v.trim())
                })?;
                let bytes = usize::try_from(parsed)
                    .unwrap_or(usize::MAX)
                    .saturating_mul(1024);
                cfg.mem_limit = bytes;
                cfg.cli_mem_override = Some(bytes);
            }
            if let Some(v) = find_slice_env("SLICE_MEMORY_MB")
                && !v.trim().is_empty()
            {
                let parsed = parse_strict_u64(v).with_context(|| {
                    format!("Invalid SLICE_MEMORY_MB integer value: {:?}", v.trim())
                })?;
                let bytes = usize::try_from(parsed)
                    .unwrap_or(usize::MAX)
                    .saturating_mul(1024 * 1024);
                cfg.mem_limit = bytes;
                cfg.cli_mem_override = Some(bytes);
            }
            if let Some(v) = find_slice_env("SLICE_MEMORY")
                && !v.trim().is_empty()
            {
                let bytes = parse_byte_size_usize(v).with_context(|| {
                    format!(
                        "Invalid SLICE_MEMORY byte size {:?} (expected bytes or KB/KiB/MB/MiB/GB/GiB)",
                        v.trim()
                    )
                })?;
                cfg.mem_limit = bytes;
                cfg.cli_mem_override = Some(bytes);
            }
            if let Some(v) = find_slice_env("SLICE_ALLOW_READ") {
                for item in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    cfg.caps = cfg.caps.with_read_prefix(item);
                }
            }
            if let Some(v) = find_slice_env("SLICE_ALLOW_WRITE") {
                for item in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    cfg.caps = cfg.caps.with_write_prefix(item);
                }
            }
            if let Some(v) = find_slice_env("SLICE_ALLOW_NET") {
                for item in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    cfg.caps = cfg.caps.with_net_host(item);
                }
            }
            if let Some(v) = find_slice_env("SLICE_ALLOW_EXEC") {
                for item in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    if !cfg.caps.allowed_exec_binaries.contains(&item.to_string()) {
                        cfg.caps = cfg.caps.with_exec_binary(item);
                    }
                }
            }
            if let Some(v) = find_slice_env("SLICE_ENV_FILE") {
                let trimmed = v.trim();
                if !trimmed.is_empty() {
                    apply_dotenv_file_with_precedence(
                        Path::new(trimmed),
                        &mut cfg.caps,
                        &mut cfg.audit_secrets,
                        true,
                    )?;
                }
            }
            if let Some(v) = find_slice_env("SLICE_ALLOW_ENV") {
                for item in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    if let Some((k, val)) = item.split_once('=') {
                        let kt = k.trim();
                        let vt = val.trim();
                        maybe_register_sensitive_env_dlp(kt, vt, &mut cfg.audit_secrets);
                        cfg.caps = cfg.caps.with_env(kt, vt);
                    } else {
                        let env_val = std::env::var(item).unwrap_or_default();
                        maybe_register_sensitive_env_dlp(item, &env_val, &mut cfg.audit_secrets);
                        cfg.caps = cfg.caps.with_env(item, env_val);
                    }
                }
            }
            if let Some(v) = find_slice_env("SLICE_AUDIT_EGRESS_FILE") {
                let trimmed = v.trim();
                if !trimmed.is_empty() {
                    audit_egress_files.push(PathBuf::from(trimmed));
                }
            }
            if let Some(v) = find_slice_env("SLICE_AUDIT_EGRESS") {
                let trimmed = v.trim();
                if !trimmed.is_empty() {
                    audit_egress_specs.push(trimmed.to_string());
                }
            }
            if let Some(v) = find_slice_env("SLICE_CHECKPOINTS")
                && let Some(b) = slice_sandbox::parse_bool_setting(v)
            {
                cfg.caps = cfg.caps.with_checkpoints(b);
                cfg.cli_checkpoints_override = Some(b);
            }
            if let Some(v) = find_slice_env("SLICE_MAX_CHECKPOINTS")
                && !v.trim().is_empty()
            {
                let n = parse_strict_u64(v).with_context(|| {
                    format!(
                        "Invalid SLICE_MAX_CHECKPOINTS integer value: {:?}",
                        v.trim()
                    )
                })?;
                let max_cp = u32::try_from(n).unwrap_or(u32::MAX);
                cfg.caps = cfg.caps.with_max_checkpoints(Some(max_cp));
                cfg.cli_max_checkpoints_override = Some(max_cp);
            }
            if let Some(v) = find_slice_env("SLICE_WARM")
                && let Some(b) = slice_sandbox::parse_bool_setting(v)
            {
                cfg.warm_pool = b;
                cfg.cli_warm_override = Some(b);
            }
            if let Some(v) = find_slice_env("SLICE_DETERMINISTIC_SEED")
                && !v.trim().is_empty()
            {
                let seed = parse_strict_u64(v).with_context(|| {
                    format!(
                        "Invalid SLICE_DETERMINISTIC_SEED integer value: {:?}",
                        v.trim()
                    )
                })?;
                cfg.deterministic_seed = Some(seed);
                cfg.cli_det_override = Some(Some(seed));
            }
            if let Some(v) = find_slice_env("SLICE_PORT")
                && let Ok(p) = v.trim().parse::<u16>()
            {
                cfg.server_port = Some(p);
                cfg.cli_port_override = Some(p);
            }
            if let Some(v) = find_slice_env("SLICE_WORKERS")
                && let Ok(w) = v.trim().parse::<usize>()
            {
                let wk = w.max(1);
                cfg.server_workers = Some(wk);
                cfg.cli_workers_override = Some(wk);
            }
            if let Some(v) = find_slice_env("SLICE_STATIC_DIR") {
                let trimmed = v.trim();
                if !trimmed.is_empty() {
                    let p = PathBuf::from(trimmed);
                    cfg.static_dir = Some(p.clone());
                    cfg.cli_static_override = Some(p);
                }
            }
            if let Some(v) = find_slice_env("SLICE_TLS_CERT") {
                let trimmed = v.trim();
                if !trimmed.is_empty() {
                    let p = PathBuf::from(trimmed);
                    cfg.tls_cert = Some(p.clone());
                    cfg.cli_tls_cert_override = Some(p);
                }
            }
            if let Some(v) = find_slice_env("SLICE_TLS_KEY") {
                let trimmed = v.trim();
                if !trimmed.is_empty() {
                    let p = PathBuf::from(trimmed);
                    cfg.tls_key = Some(p.clone());
                    cfg.cli_tls_key_override = Some(p);
                }
            }
            if let Some(v) =
                find_slice_env("SLICE_BOACAS").or_else(|| find_slice_env("SLICE_EXPER_BOACAS"))
                && let Some(b) = slice_sandbox::parse_bool_setting(v)
            {
                cfg.cli_boacas_override = Some(b);
                apply_boacas_override(Some(b));
            }
            if let Some(v) = find_slice_env("SLICE_TIER")
                && let Some(t) = slice_sandbox::parse_tier_setting(v)
            {
                cfg.cli_tier_override = Some(t);
                apply_tier_override(t);
            }
            if let Some(v) = find_slice_env("SLICE_TRUST_DIRECTIVES")
                && let Some(b) = slice_sandbox::parse_bool_setting(v)
            {
                cfg.trust_directives = b;
                cfg.cli_trust_directives_override = Some(b);
            }
        }

        // 3. Apply explicit CLI inline flags (highest precedence, fail-closed on empty/missing file paths)
        let mut cli_allow_env_overrides: Vec<(String, String)> = Vec::new();
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
                audit_egress_specs.push(sec.to_string());
                arg_idx += 1;
                continue;
            }
            if let Some(tr) = match_non_empty_flag_path(args, &mut arg_idx, "--trace")? {
                cfg.trace_output_path = Some(tr);
                arg_idx += 1;
                continue;
            }
            if let Some(prefixes) = match_non_empty_flag_value(args, &mut arg_idx, "--allow-read")?
            {
                for p in prefixes.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    cfg.caps = cfg.caps.with_read_prefix(p);
                }
                arg_idx += 1;
                continue;
            }
            if let Some(prefixes) = match_non_empty_flag_value(args, &mut arg_idx, "--allow-write")?
            {
                for p in prefixes.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    cfg.caps = cfg.caps.with_write_prefix(p);
                }
                arg_idx += 1;
                continue;
            }
            if let Some(hosts) = match_non_empty_flag_value(args, &mut arg_idx, "--allow-net")? {
                for h in hosts.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    cfg.caps = cfg.caps.with_net_host(h);
                }
                arg_idx += 1;
                continue;
            }
            if let Some(bins) = match_non_empty_flag_value(args, &mut arg_idx, "--allow-exec")? {
                for b in bins.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    cfg.caps = cfg.caps.with_exec_binary(b);
                }
                arg_idx += 1;
                continue;
            }
            if let Some(kvs) = match_non_empty_flag_value(args, &mut arg_idx, "--allow-env")? {
                for item in kvs.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    if let Some((k, v)) = item.split_once('=') {
                        cli_allow_env_overrides.push((k.trim().to_string(), v.to_string()));
                    } else {
                        let v = std::env::var(item).unwrap_or_default();
                        cli_allow_env_overrides.push((item.to_string(), v));
                    }
                }
                arg_idx += 1;
                continue;
            }
            if let Some(mcp_str) =
                match_non_empty_flag_value(args, &mut arg_idx, "--max-checkpoints")?
            {
                let n = parse_strict_u64(mcp_str)
                    .with_context(|| format!("Invalid --max-checkpoints value: {:?}", mcp_str))?;
                let max_cp = u32::try_from(n).unwrap_or(u32::MAX);
                cfg.caps = cfg.caps.with_max_checkpoints(Some(max_cp));
                cfg.cli_max_checkpoints_override = Some(max_cp);
                arg_idx += 1;
                continue;
            }
            if let Some(f_str) = match_non_empty_flag_value(args, &mut arg_idx, "--fuel")? {
                let parsed = parse_strict_u64(f_str)
                    .with_context(|| format!("Invalid --fuel integer value: {:?}", f_str))?;
                cfg.fuel_limit = parsed;
                cfg.cli_fuel_override = Some(parsed);
                arg_idx += 1;
                continue;
            }
            if let Some(m_str) = match_non_empty_flag_value(args, &mut arg_idx, "--memory")? {
                let bytes = parse_byte_size_usize(m_str).with_context(|| {
                    format!(
                        "Invalid --memory byte size {:?} (expected integer bytes or KB/KiB/MB/MiB/GB/GiB suffix)",
                        m_str
                    )
                })?;
                cfg.mem_limit = bytes;
                cfg.cli_mem_override = Some(bytes);
                arg_idx += 1;
                continue;
            }
            if let Some(mkb_str) = match_non_empty_flag_value(args, &mut arg_idx, "--memory-kb")? {
                let parsed = parse_strict_u64(mkb_str)
                    .with_context(|| format!("Invalid --memory-kb integer value: {:?}", mkb_str))?;
                let bytes = usize::try_from(parsed)
                    .unwrap_or(usize::MAX)
                    .saturating_mul(1024);
                cfg.mem_limit = bytes;
                cfg.cli_mem_override = Some(bytes);
                arg_idx += 1;
                continue;
            }
            if let Some(mmb_str) = match_non_empty_flag_value(args, &mut arg_idx, "--memory-mb")? {
                let parsed = parse_strict_u64(mmb_str)
                    .with_context(|| format!("Invalid --memory-mb integer value: {:?}", mmb_str))?;
                let bytes = usize::try_from(parsed)
                    .unwrap_or(usize::MAX)
                    .saturating_mul(1024 * 1024);
                cfg.mem_limit = bytes;
                cfg.cli_mem_override = Some(bytes);
                arg_idx += 1;
                continue;
            }
            if let Some(t_str) = match_non_empty_flag_value(args, &mut arg_idx, "--tier")? {
                let t = slice_sandbox::parse_tier_setting(t_str).with_context(|| {
                    format!(
                        "Invalid --tier value {:?} (expected 0/tier0, 1/tier1, or 2/tier2)",
                        t_str
                    )
                })?;
                cfg.cli_tier_override = Some(t);
                apply_tier_override(t);
                arg_idx += 1;
                continue;
            }
            if let Some(p_str) = match_non_empty_flag_value(args, &mut arg_idx, "--port")? {
                let p = p_str.parse::<u16>().with_context(|| {
                    format!("Invalid --port value {:?} (expected 0..=65535)", p_str)
                })?;
                cfg.server_port = Some(p);
                cfg.cli_port_override = Some(p);
                arg_idx += 1;
                continue;
            }
            if match_non_empty_flag_value(args, &mut arg_idx, "--host")?.is_some() {
                arg_idx += 1;
                continue;
            }
            if let Some(w_str) = match_non_empty_flag_value(args, &mut arg_idx, "--workers")? {
                let w = parse_strict_u64(w_str)
                    .and_then(|n| usize::try_from(n).ok())
                    .filter(|&n| n >= 1)
                    .with_context(|| {
                        format!("Invalid --workers count {:?} (expected >= 1)", w_str)
                    })?;
                cfg.server_workers = Some(w);
                cfg.cli_workers_override = Some(w);
                arg_idx += 1;
                continue;
            }
            if let Some(sd) = match_non_empty_flag_path(args, &mut arg_idx, "--static")? {
                cfg.static_dir = Some(sd.clone());
                cfg.cli_static_override = Some(sd);
                arg_idx += 1;
                continue;
            }
            if let Some(tc) = match_non_empty_flag_path(args, &mut arg_idx, "--tls-cert")? {
                cfg.tls_cert = Some(tc.clone());
                cfg.cli_tls_cert_override = Some(tc);
                arg_idx += 1;
                continue;
            }
            if let Some(tk) = match_non_empty_flag_path(args, &mut arg_idx, "--tls-key")? {
                cfg.tls_key = Some(tk.clone());
                cfg.cli_tls_key_override = Some(tk);
                arg_idx += 1;
                continue;
            }

            let arg = &args[arg_idx];
            if arg == "--no-checkpoints" {
                cfg.caps = cfg.caps.with_checkpoints(false);
                cfg.cli_checkpoints_override = Some(false);
            } else if arg == "--checkpoints" {
                cfg.caps = cfg.caps.with_checkpoints(true);
                cfg.cli_checkpoints_override = Some(true);
            } else if let Some(cp_str) = arg.strip_prefix("--checkpoints=") {
                let b = slice_sandbox::parse_bool_setting(cp_str).with_context(|| {
                    format!("Invalid --checkpoints boolean value: {:?}", cp_str)
                })?;
                cfg.caps = cfg.caps.with_checkpoints(b);
                cfg.cli_checkpoints_override = Some(b);
            } else if arg == "--boacas" || arg == "--exper-boacas" {
                cfg.cli_boacas_override = Some(true);
                apply_boacas_override(Some(true));
            } else if arg == "--no-boacas" || arg == "--no-exper-boacas" {
                cfg.cli_boacas_override = Some(false);
                apply_boacas_override(Some(false));
            } else if let Some(b_str) = arg
                .strip_prefix("--boacas=")
                .or_else(|| arg.strip_prefix("--exper-boacas="))
            {
                let b = slice_sandbox::parse_bool_setting(b_str)
                    .with_context(|| format!("Invalid --boacas boolean value: {:?}", b_str))?;
                cfg.cli_boacas_override = Some(b);
                apply_boacas_override(Some(b));
            } else if arg == "--warm" {
                cfg.warm_pool = true;
                cfg.cli_warm_override = Some(true);
            } else if arg == "--no-warm" {
                cfg.warm_pool = false;
                cfg.cli_warm_override = Some(false);
            } else if let Some(seed_str) = arg.strip_prefix("--deterministic=") {
                let s = parse_strict_u64(seed_str).with_context(|| {
                    format!("Invalid --deterministic seed integer: {:?}", seed_str)
                })?;
                cfg.deterministic_seed = Some(s);
                cfg.cli_det_override = Some(Some(s));
            } else if arg == "--deterministic" {
                cfg.deterministic_seed = Some(42);
                cfg.cli_det_override = Some(Some(42));
            } else if arg == "--no-deterministic" {
                cfg.deterministic_seed = None;
                cfg.cli_det_override = Some(None);
            } else if arg == "--trust-directives" {
                cfg.trust_directives = true;
                cfg.cli_trust_directives_override = Some(true);
            } else if arg == "--no-trust-directives"
                || arg == "--no-directives"
                || arg == "--untrusted"
            {
                cfg.trust_directives = false;
                cfg.cli_trust_directives_override = Some(false);
            } else if let Some(td_str) = arg.strip_prefix("--trust-directives=") {
                let b = slice_sandbox::parse_bool_setting(td_str).with_context(|| {
                    format!("Invalid --trust-directives boolean value: {:?}", td_str)
                })?;
                cfg.trust_directives = b;
                cfg.cli_trust_directives_override = Some(b);
            }
            arg_idx += 1;
        }

        for ef in &env_files {
            apply_dotenv_file_with_precedence(ef, &mut cfg.caps, &mut cfg.audit_secrets, true)?;
        }
        for (k, v) in cli_allow_env_overrides {
            maybe_register_sensitive_env_dlp(&k, &v, &mut cfg.audit_secrets);
            cfg.caps = cfg.caps.with_env(k, v);
        }
        for sf in &audit_egress_files {
            load_audit_egress_file(sf, &mut cfg.audit_secrets)?;
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
            register_audit_egress_spec(spec, cli_audit_base, &cfg.caps, &mut cfg.audit_secrets)?;
        }

        Ok(cfg)
    }
}
