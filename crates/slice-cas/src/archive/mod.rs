//! Zero-Stat Content-Addressed Storage (`.cas`) Binary Archive & Static Asset Subsystem.
//!
//! Organizes the binary `.cas` archive layer into focused submodules:
//! - [`builder`]: [`CasBuilder`] for packing JS/TS modules, static web assets, and `slice.toml` manifests.
//! - [`reader`]: [`MappedCasArchive`] zero-copy `mmap` reader and `SLICEEXE` standalone binary packer.
//! - [`static_assets`]: [`StaticAssetTable`], [`StaticAssetEntry`], and BLAKE3 `ETag` negotiation.

#[cfg(feature = "compiler")]
pub mod builder;
pub mod reader;
pub mod static_assets;

#[cfg(feature = "compiler")]
pub use builder::CasBuilder;
pub use reader::{
    CasBacking, MappedCasArchive, MappedCasInner, embed_cas_into_executable,
    inspect_sliceexe_trailer,
};
pub use static_assets::{
    StaticAssetEntry, StaticAssetPayload, StaticAssetTable, detect_static_mime_type,
};

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

pub const CAS_MAGIC: &[u8; 8] = b"SLICECAS";
pub const CAS_VERSION: u32 = 1;
pub const CACHE_LINE_ALIGN: usize = 64;

#[inline]
pub(super) fn strip_single_module_ext(spec: &str) -> &str {
    for ext in [".tsx", ".mts", ".cts", ".ts", ".jsx", ".mjs", ".cjs", ".js"] {
        if let Some(stem) = spec.strip_suffix(ext) {
            return stem;
        }
    }
    spec
}

/// Walks `root_dir` recursively in deterministic filename-sorted order while skipping symbolic
/// links via `entry.file_type()` (`lstat`, `CAS-SEC-04`) and any directories rejected by
/// `should_descend_dir`.
pub(super) fn walk_sorted_non_symlink_files(
    root_dir: &std::path::Path,
    mut should_descend_dir: impl FnMut(&std::ffi::OsStr) -> bool,
    mut on_file: impl FnMut(&std::fs::DirEntry, &std::path::Path) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let mut stack = vec![root_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries: Vec<std::fs::DirEntry> =
            std::fs::read_dir(&dir)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let ft = entry.file_type()?;
            if ft.is_symlink() {
                continue;
            }
            let path = entry.path();
            if ft.is_dir() {
                if should_descend_dir(&entry.file_name()) {
                    stack.push(path);
                }
            } else if ft.is_file() {
                on_file(&entry, &path)?;
            }
        }
    }
    Ok(())
}

/// Metadata entry in the CAS Radix/Index table pointing to a 64-byte aligned slice.
#[repr(C)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CasBlobEntry {
    pub offset: u64,
    pub source_len: u64,
    pub bytecode_offset: u64,
    pub bytecode_len: u64,
    pub is_typescript: bool,
    pub hash_hex: String,
    pub original_specifier: String,
}

/// Deterministic Radix / Hash resolution table embedded in the CAS header.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CasIndexHeader {
    pub version: u32,
    pub entrypoint_specifier: String,
    pub entrypoint_hash: String,
    /// Maps `(issuer_context, import_specifier)` or canonical specifier -> `hash_hex`
    pub radix_resolution_map: FxHashMap<String, String>,
    /// Maps `hash_hex` -> `CasBlobEntry`
    pub blobs: FxHashMap<String, CasBlobEntry>,
    /// Compact source-line table (`(compacted_line_1, "#slice-source payload")`) for `.cas` and `SLICEEXE` stack traces
    #[serde(default)]
    pub source_map_markers: Vec<(usize, String)>,
    /// Per-blob source-line marker table keyed by `hash_hex` for multi-module `.cas` archives
    #[serde(default)]
    pub blob_source_map_markers: FxHashMap<String, Vec<(usize, String)>>,
    /// Optional BLAKE3 hash of the project's lockfile (`package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`, `bun.lock`)
    #[serde(default)]
    pub lockfile_blake3: Option<String>,
    /// Resolved `(package_name, resolved_version)` pairs recorded from `package.json` and `node_modules/`
    #[serde(default)]
    pub resolved_dependencies: Vec<(String, String)>,
}
