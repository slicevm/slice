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

/// Unified execution configuration (`CapabilityToken`, `fuel_limit`, `mem_limit`, `audit_secrets`)
/// resolved across defaults, `./slice.toml`, target subdirectory `slice.toml`, `.cas` embedded
/// manifests, `--env-file`, and explicit CLI flags (`--fuel=`, `--memory=`, `--allow-*`).
#[derive(Debug, Clone)]
pub(crate) struct ResolvedExecutionConfig {
    pub(crate) caps: CapabilityToken,
    pub(crate) fuel_limit: u64,
    pub(crate) mem_limit: usize,
    pub(crate) audit_secrets: Vec<String>,
    pub(crate) cli_fuel_override: Option<u64>,
    pub(crate) cli_mem_override: Option<usize>,
    pub(crate) cli_boacas_override: Option<bool>,
}

impl ResolvedExecutionConfig {
    pub(crate) fn from_cli_args(args: &[String]) -> Result<Self> {
        let (mut caps, mut fuel_limit, mut mem_limit) = load_manifest_or_default();
        let mut audit_secrets = Vec::new();
        let mut env_files = Vec::new();
        let mut cli_fuel_override = None;
        let mut cli_mem_override = None;
        let mut cli_boacas_override = None;

        // Also capture DLP secrets from `./slice.toml` if present in cwd
        let cwd_manifest = Path::new("slice.toml");
        if cwd_manifest.exists() {
            if let Ok(raw) = std::fs::read_to_string(cwd_manifest) {
                merge_manifest_toml_into_caps_and_dlp(
                    &raw,
                    Some(Path::new(".")),
                    &mut caps,
                    &mut audit_secrets,
                );
            }
        }

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
                if let Ok(parsed) = f.parse::<u64>() {
                    fuel_limit = parsed;
                    cli_fuel_override = Some(parsed);
                }
            } else if let Some(m) = arg.strip_prefix("--memory=") {
                if let Ok(parsed) = m.parse::<usize>() {
                    mem_limit = parsed;
                    cli_mem_override = Some(parsed);
                }
            } else if arg == "--exper-boacas" {
                cli_boacas_override = Some(true);
                slice_engine::set_exper_boacas_override(Some(true));
            } else if arg == "--no-exper-boacas" {
                cli_boacas_override = Some(false);
                slice_engine::set_exper_boacas_override(Some(false));
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
            cli_fuel_override,
            cli_mem_override,
            cli_boacas_override,
        })
    }

    /// Merges a target project's `slice.toml` or embedded `.cas` manifest TOML into this
    /// configuration, updating `fuel_limit` and `mem_limit` from `[limits]` unless overridden
    /// on the command line via `--fuel=` or `--memory=`.
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
                slice_engine::set_exper_boacas_override(Some(b));
            }
        }
        merge_manifest_toml_into_caps_and_dlp(
            raw_toml,
            base_dir,
            &mut self.caps,
            &mut self.audit_secrets,
        );
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
                slice_engine::set_exper_boacas_override(Some(b));
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
