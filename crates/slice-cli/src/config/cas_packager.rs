use super::{apply_boacas_override, find_project_manifest_path};
use anyhow::{Context, Result, bail};
use slice_cas::CasBuilder;
use slice_engine::compile_source_to_bytecode;
use slice_sandbox::SliceManifest;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_CAS_SEQ: AtomicU64 = AtomicU64::new(1);

/// Generates a collision-resistant, non-predictable temporary `.cas` staging path (`CLI-SEC-02`)
/// combining `prefix`, process ID, monotonic sequence counter, and nanosecond timestamp,
/// and rejects any pre-existing symlink at that path.
pub(crate) fn unique_temp_cas_path(prefix: &str) -> Result<PathBuf> {
    let pid = std::process::id();
    let seq = TEMP_CAS_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let candidate = env::temp_dir().join(format!("{}_{}_{}_{}.cas", prefix, pid, seq, nanos));
    if let Ok(meta) = std::fs::symlink_metadata(&candidate)
        && meta.file_type().is_symlink()
    {
        bail!(
            "Refusing to write staging CAS archive to pre-existing symlink: {}",
            candidate.display()
        );
    }
    Ok(candidate)
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
    let manifest_path_opt = find_project_manifest_path(input_path);
    let manifest_toml_opt = match &manifest_path_opt {
        Some(mf) => Some(
            std::fs::read_to_string(mf)
                .with_context(|| format!("Failed to read manifest {}", mf.display()))?,
        ),
        None => None,
    };
    let mut manifest_static_dir: Option<std::path::PathBuf> = None;
    if let Some(ref raw_toml) = manifest_toml_opt {
        let m = SliceManifest::parse_toml(raw_toml);
        if let Some(b) = m.exper_boacas
            && std::env::var("SLICE_EXPER_BOACAS").is_err()
        {
            apply_boacas_override(Some(b));
        }
        if static_dir_opt.is_none()
            && let Some(sd) = m.static_dir
        {
            let base = manifest_path_opt
                .as_deref()
                .and_then(|p| p.parent())
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            let candidate = base.join(sd);
            if candidate.is_dir() {
                manifest_static_dir = Some(candidate);
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
        b.scan_project_dependencies(input_path);
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

    if let Some(static_dir) = static_dir_opt.or(manifest_static_dir.as_deref()) {
        builder.ingest_static_directory(static_dir)?;
    }

    let header = builder.write_to_file(output_path)?;
    crate::server::notify_tenant_cas_updated();
    Ok(header)
}
