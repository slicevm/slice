use anyhow::{Context, Result};
use slice_cas::CasBuilder;
use slice_engine::compile_source_to_bytecode;
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

impl ResolvedExecutionConfig {
    pub(crate) fn from_cli_args(args: &[String]) -> Result<Self> {
        let (mut caps, mut fuel_limit, mut mem_limit) = load_manifest_or_default();
        let mut audit_secrets = Vec::new();
        let mut env_files = Vec::new();
        let mut warm_pool = false;
        let mut deterministic_seed = None;
        let mut server_port = None;
        let mut server_workers = None;
        let mut static_dir = None;
        let mut tls_cert = None;
        let mut tls_key = None;
        let mut trust_directives = false;

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

        // 1. Apply `./slice.toml` if present in cwd
        let cwd_manifest = Path::new("slice.toml");
        if cwd_manifest.exists() {
            if let Ok(raw) = std::fs::read_to_string(cwd_manifest) {
                let m = SliceManifest::parse_toml(&raw);
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
                if let Some(sd) = m.static_dir {
                    static_dir = Some(PathBuf::from(sd));
                }
                if let Some(tc) = m.tls_cert {
                    tls_cert = Some(PathBuf::from(tc));
                }
                if let Some(tk) = m.tls_key {
                    tls_key = Some(PathBuf::from(tk));
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
                merge_manifest_toml_into_caps_and_dlp(
                    &raw,
                    Some(Path::new(".")),
                    &mut caps,
                    &mut audit_secrets,
                );
            }
        }

        // 2. Apply `SLICE_*` environment variables (override slice.toml, overridden by CLI flags)
        if let Ok(v) = std::env::var("SLICE_FUEL") {
            let digits: String = v.chars().filter(|c| c.is_ascii_digit()).collect();
            if let Ok(parsed) = digits.parse::<u64>() {
                fuel_limit = parsed;
                cli_fuel_override = Some(parsed);
            }
        }
        if let Ok(v) = std::env::var("SLICE_MEMORY_KB") {
            let digits: String = v.chars().filter(|c| c.is_ascii_digit()).collect();
            if let Ok(parsed) = digits.parse::<usize>() {
                let bytes = parsed.saturating_mul(1024);
                mem_limit = bytes;
                cli_mem_override = Some(bytes);
            }
        }
        if let Ok(v) = std::env::var("SLICE_MEMORY") {
            let digits: String = v.chars().filter(|c| c.is_ascii_digit()).collect();
            if let Ok(parsed) = digits.parse::<usize>() {
                mem_limit = parsed;
                cli_mem_override = Some(parsed);
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
        if let Ok(v) = std::env::var("SLICE_ALLOW_ENV") {
            for item in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                if let Some((k, val)) = item.split_once('=') {
                    if val.len() >= 6
                        && slice_sandbox::is_sensitive_env_key(k)
                        && !audit_secrets.contains(&val.to_string())
                    {
                        audit_secrets.push(val.to_string());
                    }
                    caps = caps.with_env(k.trim(), val.trim());
                } else {
                    let env_val = std::env::var(item).unwrap_or_default();
                    if env_val.len() >= 6
                        && slice_sandbox::is_sensitive_env_key(item)
                        && !audit_secrets.contains(&env_val)
                    {
                        audit_secrets.push(env_val.clone());
                    }
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
        if let Ok(v) = std::env::var("SLICE_AUDIT_EGRESS") {
            for sec in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                if !audit_secrets.contains(&sec.to_string()) {
                    audit_secrets.push(sec.to_string());
                }
            }
        }
        if let Ok(v) = std::env::var("SLICE_WARM") {
            if let Some(b) = slice_sandbox::parse_bool_setting(&v) {
                warm_pool = b;
                cli_warm_override = Some(b);
            }
        }
        if let Ok(v) = std::env::var("SLICE_DETERMINISTIC_SEED") {
            let digits: String = v.chars().filter(|c| c.is_ascii_digit()).collect();
            if let Ok(seed) = digits.parse::<u64>() {
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

        // 3. Apply explicit CLI inline flags (highest precedence)
        for arg in args {
            if let Some(prefix) = arg.strip_prefix("--allow-read=") {
                caps = caps.with_read_prefix(prefix);
            } else if let Some(prefix) = arg.strip_prefix("--allow-write=") {
                caps = caps.with_write_prefix(prefix);
            } else if let Some(host) = arg.strip_prefix("--allow-net=") {
                for h in host.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    caps = caps.with_net_host(h);
                }
            } else if let Some(kv) = arg.strip_prefix("--allow-env=") {
                if let Some((k, v)) = kv.split_once('=') {
                    if v.len() >= 6
                        && slice_sandbox::is_sensitive_env_key(k)
                        && !audit_secrets.contains(&v.to_string())
                    {
                        audit_secrets.push(v.to_string());
                    }
                    caps = caps.with_env(k, v);
                } else {
                    let k = kv.trim();
                    let v = std::env::var(k).unwrap_or_default();
                    if v.len() >= 6
                        && slice_sandbox::is_sensitive_env_key(k)
                        && !audit_secrets.contains(&v)
                    {
                        audit_secrets.push(v.clone());
                    }
                    caps = caps.with_env(k, v);
                }
            } else if let Some(ef) = arg.strip_prefix("--env-file=") {
                if !ef.is_empty() {
                    env_files.push(PathBuf::from(ef));
                }
            } else if let Some(sec) = arg.strip_prefix("--audit-egress=") {
                if !sec.is_empty() && !audit_secrets.contains(&sec.to_string()) {
                    audit_secrets.push(sec.to_string());
                }
            } else if let Some(f) = arg.strip_prefix("--fuel=") {
                let digits: String = f.chars().filter(|c| c.is_ascii_digit()).collect();
                if let Ok(parsed) = digits.parse::<u64>() {
                    fuel_limit = parsed;
                    cli_fuel_override = Some(parsed);
                }
            } else if let Some(m) = arg.strip_prefix("--memory=") {
                let digits: String = m.chars().filter(|c| c.is_ascii_digit()).collect();
                if let Ok(parsed) = digits.parse::<usize>() {
                    mem_limit = parsed;
                    cli_mem_override = Some(parsed);
                }
            } else if let Some(mkb) = arg.strip_prefix("--memory-kb=") {
                let digits: String = mkb.chars().filter(|c| c.is_ascii_digit()).collect();
                if let Ok(parsed) = digits.parse::<usize>() {
                    let bytes = parsed.saturating_mul(1024);
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
        }

        for ef in &env_files {
            apply_dotenv_file_to_caps_and_dlp(ef, &mut caps, &mut audit_secrets)?;
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
    /// configuration, updating settings unless already overridden by `SLICE_*` env vars or CLI flags.
    pub(crate) fn apply_manifest_toml(&mut self, raw_toml: &str, base_dir: Option<&Path>) {
        let m = SliceManifest::parse_toml(raw_toml);
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
        if self.cli_static_override.is_none() {
            if let Some(sd) = m.static_dir {
                let p = match base_dir {
                    Some(dir) => dir.join(&sd),
                    None => PathBuf::from(&sd),
                };
                self.static_dir = Some(p);
            }
        }
        if self.cli_tls_cert_override.is_none() {
            if let Some(tc) = m.tls_cert {
                let p = match base_dir {
                    Some(dir) => dir.join(&tc),
                    None => PathBuf::from(&tc),
                };
                self.tls_cert = Some(p);
            }
        }
        if self.cli_tls_key_override.is_none() {
            if let Some(tk) = m.tls_key {
                let p = match base_dir {
                    Some(dir) => dir.join(&tk),
                    None => PathBuf::from(&tk),
                };
                self.tls_key = Some(p);
            }
        }
        if self.cli_trust_directives_override.is_none() {
            if let Some(td) = m.trust_directives {
                self.trust_directives = td;
            }
        }
        merge_manifest_toml_into_caps_and_dlp(
            raw_toml,
            base_dir,
            &mut self.caps,
            &mut self.audit_secrets,
        );
    }

    /// Applies top-of-file inline source directives (`// @slice-...`) from an entrypoint script,
    /// overriding `slice.toml` defaults while respecting `SLICE_*` env vars and CLI flags.
    ///
    /// Security Boundary (`P33`):
    /// - Safe tuning hints (`tier`, `boacas`, `warm`, `deterministic`, `port`, `workers`),
    ///   extra DLP secrets (`audit-egress`), and monotonic self-attenuation (lowering `fuel`/`memory`
    ///   or narrowing paths already granted by the host) are always active.
    /// - Privilege-widening directives (`allow-read`, `allow-write`, `allow-net`, `allow-env`,
    ///   `env-file`, or raising `fuel`/`memory` above the host ceiling) require explicit host opt-in
    ///   via `--trust-directives` (`SLICE_TRUST_DIRECTIVES=1` or `trust_directives = true` in `slice.toml`).
    pub(crate) fn apply_inline_directives(&mut self, source: &str, base_dir: Option<&Path>) {
        if !source.starts_with("//") && !source.starts_with("#!") {
            return;
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
            if !sec.is_empty() && !self.audit_secrets.contains(&sec) {
                self.audit_secrets.push(sec);
            }
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
            for (k, v) in dirs.allow_env {
                if v.len() >= 6
                    && slice_sandbox::is_sensitive_env_key(&k)
                    && !self.audit_secrets.contains(&v)
                {
                    self.audit_secrets.push(v.clone());
                }
                self.caps.allowed_env_keys.entry(k).or_insert(v);
            }
            if let Some(ef) = dirs.env_file {
                let env_path = match base_dir {
                    Some(dir) => dir.join(&ef),
                    None => PathBuf::from(&ef),
                };
                let _ = apply_dotenv_file_to_caps_and_dlp(
                    &env_path,
                    &mut self.caps,
                    &mut self.audit_secrets,
                );
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
        }
    }

    /// Discovers and applies `slice.toml` for `target_path` (if one exists in `target_path`,
    /// its parent directory, or its grandparent directory).
    pub(crate) fn apply_target_path_manifest(&mut self, target_path: &Path) {
        if let Some(manifest_file) = find_project_manifest_path(target_path) {
            if let Ok(raw) = std::fs::read_to_string(&manifest_file) {
                self.apply_manifest_toml(&raw, manifest_file.parent());
            }
        }
    }
}

pub(crate) fn load_manifest_or_default() -> (CapabilityToken, u64, usize) {
    let manifest_path = Path::new("slice.toml");
    if manifest_path.exists() {
        if let Ok(raw) = std::fs::read_to_string(manifest_path) {
            let m = SliceManifest::parse_toml(&raw);
            return (m.capabilities, m.fuel_limit, m.memory_limit_kb * 1024);
        }
    }
    (
        CapabilityToken::pure_compute(),
        50_000_000,
        16 * 1024 * 1024,
    )
}

#[cfg(test)]
pub(crate) fn merge_manifest_toml_into_caps(raw_toml: &str, caps: &mut CapabilityToken) {
    let mut dummy_secrets = Vec::new();
    merge_manifest_toml_into_caps_and_dlp(raw_toml, None, caps, &mut dummy_secrets);
}

pub(crate) fn merge_manifest_toml_into_caps_and_dlp(
    raw_toml: &str,
    base_dir: Option<&Path>,
    caps: &mut CapabilityToken,
    audit_secrets: &mut Vec<String>,
) {
    let m = SliceManifest::parse_toml(raw_toml);
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
    for (k, v) in m.capabilities.allowed_env_keys {
        if v.len() >= 6 && slice_sandbox::is_sensitive_env_key(&k) && !audit_secrets.contains(&v) {
            audit_secrets.push(v.clone());
        }
        caps.allowed_env_keys.entry(k).or_insert(v);
    }
    for sec in m.audit_egress {
        if !sec.is_empty() && !audit_secrets.contains(&sec) {
            audit_secrets.push(sec);
        }
    }
    if let Some(env_file_rel) = m.env_file {
        let env_path = match base_dir {
            Some(dir) => dir.join(&env_file_rel),
            None => PathBuf::from(&env_file_rel),
        };
        let _ = apply_dotenv_file_to_caps_and_dlp(&env_path, caps, audit_secrets);
    }
}

pub(crate) fn apply_dotenv_file_to_caps_and_dlp(
    dotenv_path: &Path,
    caps: &mut CapabilityToken,
    audit_secrets: &mut Vec<String>,
) -> Result<usize> {
    let raw = std::fs::read_to_string(dotenv_path)
        .with_context(|| format!("Failed to read --env-file {}", dotenv_path.display()))?;
    let (pairs, secrets) = slice_sandbox::parse_dotenv_content(&raw);
    for (k, v) in pairs {
        caps.allowed_env_keys.insert(k, v);
    }
    let count = secrets.len();
    for sec in secrets {
        if !audit_secrets.contains(&sec) {
            audit_secrets.push(sec);
        }
    }
    Ok(count)
}

pub(crate) fn collect_watch_fingerprint(target: &Path) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    fn visit_watch(dir: &Path, hasher: &mut DefaultHasher) {
        if dir.is_file() {
            if let Ok(meta) = std::fs::metadata(dir) {
                dir.to_string_lossy().hash(hasher);
                meta.len().hash(hasher);
                if let Ok(m) = meta.modified() {
                    if let Ok(d) = m.duration_since(std::time::UNIX_EPOCH) {
                        d.as_nanos().hash(hasher);
                    }
                }
            }
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or_default();
            if matches!(name, "node_modules" | "target" | ".git" | ".archive") {
                continue;
            }
            if p.is_dir() {
                visit_watch(&p, hasher);
            } else {
                let ext = p.extension().and_then(|s| s.to_str()).unwrap_or_default();
                if matches!(
                    ext,
                    "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "json" | "toml" | "env" | "wasm"
                ) || name.starts_with(".env")
                {
                    visit_watch(&p, hasher);
                }
            }
        }
    }
    visit_watch(target, &mut hasher);
    hasher.finish()
}

/// OS-native filesystem watcher backed by `notify` (`inotify` on Linux, `ReadDirectoryChangesW` on Windows,
/// `FSEvents`/`kqueue` on macOS) with a 15ms debounce window and automatic fingerprint fallback.
pub(crate) struct OsNativeFileWatcher {
    target: PathBuf,
    prev_fp: u64,
    _watcher: Option<notify::RecommendedWatcher>,
    rx: std::sync::mpsc::Receiver<notify::Event>,
}

impl OsNativeFileWatcher {
    pub(crate) fn new(target: &Path) -> Self {
        use notify::{RecursiveMode, Watcher};
        let prev_fp = collect_watch_fingerprint(target);
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher_opt =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if let Ok(ev) = res {
                    let _ = tx.send(ev);
                }
            })
            .ok();
        if let Some(ref mut w) = watcher_opt {
            let watch_root = if target.is_dir() {
                target
            } else {
                target.parent().unwrap_or(target)
            };
            let _ = w.watch(watch_root, RecursiveMode::Recursive);
        }
        Self {
            target: target.to_path_buf(),
            prev_fp,
            _watcher: watcher_opt,
            rx,
        }
    }

    /// Blocks on the OS kernel event channel (with zero polling CPU when idle) or until `watch_once`
    /// deadline expires, debouncing rapid save bursts (15 ms) and confirming fingerprint change.
    pub(crate) fn wait_for_change(&mut self, watch_once: bool) -> bool {
        use std::time::{Duration, Instant};
        let wait_start = Instant::now();
        let timeout = if watch_once {
            Duration::from_millis(50)
        } else {
            Duration::from_millis(500)
        };
        loop {
            let _ = self.rx.recv_timeout(timeout);
            // Drain any coalesced save events within a 15ms debounce window
            let debounce_deadline = Instant::now() + Duration::from_millis(15);
            while self
                .rx
                .recv_timeout(debounce_deadline.saturating_duration_since(Instant::now()))
                .is_ok()
            {}
            let cur_fp = collect_watch_fingerprint(&self.target);
            if cur_fp != self.prev_fp || (watch_once && wait_start.elapsed().as_millis() >= 250) {
                self.prev_fp = cur_fp;
                return true;
            }
        }
    }
}

pub(crate) fn find_project_manifest_path(input_path: &Path) -> Option<PathBuf> {
    if input_path.is_dir() {
        let candidate = input_path.join("slice.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
    } else if let Some(parent) = input_path.parent() {
        let candidate = parent.join("slice.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        if let Some(grandparent) = parent.parent() {
            let candidate2 = grandparent.join("slice.toml");
            if candidate2.is_file() {
                return Some(candidate2);
            }
        }
    }
    None
}

#[cfg(test)]
pub(crate) fn build_cas_archive(
    input_path: &Path,
    output_path: &Path,
    entrypoint: &str,
) -> Result<slice_cas::CasIndexHeader> {
    build_cas_archive_with_static(input_path, output_path, entrypoint, None)
}

#[inline]
fn apply_boacas_override(enabled: Option<bool>) {
    #[cfg(test)]
    slice_engine::set_thread_exper_boacas_override(enabled);
    #[cfg(not(test))]
    slice_engine::set_exper_boacas_override(enabled);
}

#[inline]
fn apply_tier_override(tier: Option<u8>) {
    #[cfg(test)]
    slice_engine::set_thread_tier_override(tier);
    #[cfg(not(test))]
    slice_engine::set_tier_override(tier);
}

pub(crate) fn build_cas_archive_with_static(
    input_path: &Path,
    output_path: &Path,
    entrypoint: &str,
    static_dir_opt: Option<&Path>,
) -> Result<slice_cas::CasIndexHeader> {
    let manifest_toml_opt = find_project_manifest_path(input_path)
        .and_then(|mf| std::fs::read_to_string(&mf).ok());
    if let Some(ref raw_toml) = manifest_toml_opt {
        let m = SliceManifest::parse_toml(raw_toml);
        if let Some(b) = m.exper_boacas {
            if std::env::var("SLICE_EXPER_BOACAS").is_err() {
                apply_boacas_override(Some(b));
            }
        }
    }

    let is_folder_app = input_path.is_dir()
        && (input_path.join("package.json").is_file()
            || input_path.join("slice.toml").is_file()
            || input_path.join("src").is_dir());

    let mut builder = if input_path.is_file() || is_folder_app {
        let entry_file = slice_cas::resolve_project_entrypoint(input_path, Some(entrypoint))?;
        let file_name = entry_file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let spec = format!("./{}", file_name);
        let mut b = CasBuilder::new(&spec);
        let bundled = slice_cas::bundle_entrypoint_with_deps(&entry_file)?;
        b.add_module(spec, entry_file, &bundled, compile_source_to_bytecode);
        b
    } else {
        let mut b = CasBuilder::new(entrypoint);
        b.ingest_directory(input_path, &compile_source_to_bytecode)?;
        b
    };

    if let Some(ref raw_toml) = manifest_toml_opt {
        builder.embed_manifest_toml(raw_toml);
    }

    if let Some(static_dir) = static_dir_opt {
        builder.ingest_static_directory(static_dir)?;
    }

    builder.write_to_file(output_path)
}
