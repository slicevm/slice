use anyhow::{Context, Result};
use slice_cas::CasBuilder;
use slice_engine::compile_source_to_bytecode;
use slice_sandbox::{CapabilityToken, SliceManifest};
use std::path::{Path, PathBuf};

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
                    "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "json" | "toml" | "env"
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

    if let Some(manifest_file) = find_project_manifest_path(input_path) {
        if let Ok(raw_toml) = std::fs::read_to_string(&manifest_file) {
            builder.embed_manifest_toml(&raw_toml);
        }
    }

    if let Some(static_dir) = static_dir_opt {
        builder.ingest_static_directory(static_dir)?;
    }

    builder.write_to_file(output_path)
}
