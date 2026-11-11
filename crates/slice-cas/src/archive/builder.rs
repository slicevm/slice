//! Zero-Stat Content-Addressed Storage (`.cas`) Builder (`CasBuilder`).

use crate::archive::{
    detect_static_mime_type, CasBlobEntry, CasIndexHeader, CACHE_LINE_ALIGN, CAS_MAGIC, CAS_VERSION,
};
use crate::compiler::strip_typescript_and_normalize;
use anyhow::{bail, Context, Result};
use rustc_hash::FxHashMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Builder for constructing a Zero-Stat Content-Addressed Storage (`.cas`) archive.
pub struct CasBuilder {
    entrypoint: String,
    modules: Vec<(String, PathBuf, String, Vec<u8>)>, // (specifier, path, stripped_source, precompiled_bytecode)
    lockfile_blake3: Option<String>,
    resolved_dependencies: Vec<(String, String)>,
}

impl CasBuilder {
    pub fn new(entrypoint: impl Into<String>) -> Self {
        Self {
            entrypoint: entrypoint.into(),
            modules: Vec::new(),
            lockfile_blake3: None,
            resolved_dependencies: Vec::new(),
        }
    }

    /// Scans `project_path` (or its ancestor directories) for `package.json` and lockfiles
    /// (`package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`, `bun.lock`, `bun.lockb`), recording
    /// deterministic lockfile BLAKE3 provenance and resolved `node_modules/<pkg>/package.json` versions.
    pub fn scan_project_dependencies(&mut self, project_path: &Path) {
        let start_dir = if project_path.is_dir() {
            project_path
        } else {
            project_path.parent().unwrap_or(project_path)
        };
        let mut cur = Some(start_dir);
        while let Some(dir) = cur {
            let pkg_json = dir.join("package.json");
            if pkg_json.is_file() {
                for lock_name in [
                    "package-lock.json",
                    "pnpm-lock.yaml",
                    "yarn.lock",
                    "bun.lock",
                    "bun.lockb",
                ] {
                    let lock_path = dir.join(lock_name);
                    if lock_path.is_file() {
                        if let Ok(bytes) = std::fs::read(&lock_path) {
                            self.lockfile_blake3 = Some(blake3::hash(&bytes).to_hex().to_string());
                            break;
                        }
                    }
                }
                if let Ok(raw) = std::fs::read_to_string(&pkg_json) {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                        let mut deps = Vec::new();
                        for section in ["dependencies", "devDependencies"] {
                            if let Some(obj) = val.get(section).and_then(|v| v.as_object()) {
                                for (name, spec_val) in obj {
                                    let declared = spec_val.as_str().unwrap_or("*").to_string();
                                    let resolved_pkg =
                                        dir.join("node_modules").join(name).join("package.json");
                                    let version = std::fs::read_to_string(&resolved_pkg)
                                        .ok()
                                        .and_then(|s| {
                                            serde_json::from_str::<serde_json::Value>(&s).ok()
                                        })
                                        .and_then(|v| {
                                            v.get("version")
                                                .and_then(|ver| ver.as_str())
                                                .map(|s| s.to_string())
                                        })
                                        .unwrap_or(declared);
                                    if !deps.iter().any(|(n, _)| n == name) {
                                        deps.push((name.clone(), version));
                                    }
                                }
                            }
                        }
                        deps.sort_by(|a, b| a.0.cmp(&b.0));
                        self.resolved_dependencies = deps;
                    }
                }
                break;
            }
            cur = dir.parent();
        }
    }

    /// Adds a source module and optional pre-compiled bytecode payload to the CAS builder.
    pub fn add_module(
        &mut self,
        specifier: impl Into<String>,
        path: PathBuf,
        raw_source: &str,
        bytecode_compiler_fn: impl FnOnce(&str) -> Vec<u8>,
    ) {
        let spec = specifier.into();
        let is_ts = spec.ends_with(".ts") || spec.ends_with(".tsx") || spec.ends_with(".jsx");
        let normalized = strip_typescript_and_normalize(raw_source, is_ts);
        let bytecode = bytecode_compiler_fn(&normalized);
        self.modules.push((spec, path, normalized, bytecode));
    }

    /// Recursively ingests a directory or module tree into the CAS builder.
    ///
    /// Security (`CAS-SEC-04`): Uses `entry.file_type()` (`lstat`) to skip symbolic links
    /// (preventing symlink-cycle infinite loops and arbitrary host file inclusion), skips
    /// `.git`/`node_modules`/`target`, and sorts entries deterministically by filename.
    pub fn ingest_directory(
        &mut self,
        root_dir: &Path,
        bytecode_compiler_fn: &dyn Fn(&str) -> Vec<u8>,
    ) -> Result<usize> {
        self.scan_project_dependencies(root_dir);
        let mut count = 0;
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
                    let dir_name = entry.file_name();
                    if dir_name == ".git" || dir_name == "node_modules" || dir_name == "target" {
                        continue;
                    }
                    stack.push(path);
                } else if ft.is_file() {
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        if matches!(ext, "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs") {
                            let rel = path
                                .strip_prefix(root_dir)
                                .unwrap_or(&path)
                                .to_string_lossy()
                                .replace('\\', "/");
                            let specifier = format!("./{}", rel);
                            let raw = std::fs::read_to_string(&path)?;
                            let is_ts = matches!(ext, "ts" | "tsx" | "jsx");
                            let normalized = strip_typescript_and_normalize(&raw, is_ts);
                            let bytecode = bytecode_compiler_fn(&normalized);
                            self.modules.push((specifier, path, normalized, bytecode));
                            count += 1;
                        }
                    }
                }
            }
        }
        Ok(count)
    }

    /// Recursively ingests static web assets (`--static=./public`) into the CAS archive under
    /// `static:/<rel_path>` specifiers so an entire full-stack Web UI + API can be packed into a
    /// single `.cas` archive or compiled into a single `SLICEEXE` executable.
    ///
    /// Security (`CAS-SEC-04`): Skips symbolic links via `entry.file_type()` (`lstat`) so a
    /// symlink inside `static_dir` cannot pack host files outside `static_dir`.
    pub fn ingest_static_directory(&mut self, static_dir: &Path) -> Result<usize> {
        if !static_dir.exists() {
            bail!(
                "Static asset directory does not exist: {}",
                static_dir.display()
            );
        }
        let mut count = 0;
        let mut stack = vec![static_dir.to_path_buf()];
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
                    stack.push(path);
                } else if ft.is_file() {
                    let rel = path
                        .strip_prefix(static_dir)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .replace('\\', "/");
                    let specifier = format!("static:/{}", rel);
                    let raw_bytes = std::fs::read(&path)?;
                    let mime = detect_static_mime_type(&path).to_string();
                    self.modules.push((specifier, path, mime, raw_bytes));
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    /// Embeds a `slice.toml` declarative OCap manifest directly into the `.cas` archive under
    /// `"manifest:/slice.toml"` so `.cas` archives and standalone `SLICEEXE` binaries enforce
    /// their declared `allow_net`, `allow_read`, `allow_write`, and `[limits]` policies automatically.
    pub fn embed_manifest_toml(&mut self, toml_content: &str) {
        self.modules.push((
            "manifest:/slice.toml".to_string(),
            PathBuf::from("slice.toml"),
            toml_content.to_string(),
            toml_content.as_bytes().to_vec(),
        ));
    }

    /// Serializes the entire module graph into an immutable, 64-byte aligned `.cas` archive file.
    pub fn write_to_file(&self, output_path: &Path) -> Result<CasIndexHeader> {
        let mut radix_map = FxHashMap::default();
        let mut blobs = FxHashMap::default();
        let mut data_payload: Vec<u8> = Vec::new();
        let mut entry_hash = String::new();
        let mut source_map_markers: Vec<(usize, String)> = Vec::new();

        for (specifier, _path, normalized_source, bytecode) in &self.modules {
            let is_static = specifier.starts_with("static:/");
            let is_manifest = specifier == "manifest:/slice.toml";
            if !is_static && !is_manifest && normalized_source.contains("// #slice-source: ") {
                let mut compacted_line_1 = 1usize;
                for (idx, line) in normalized_source.lines().enumerate() {
                    if idx == 0 && line.starts_with("var __slice_modules =") {
                        compacted_line_1 += 1;
                        continue;
                    }
                    let trimmed = line.trim();
                    if let Some(rest) = trimmed.strip_prefix("// #slice-source: ") {
                        source_map_markers.push((compacted_line_1, rest.to_string()));
                        continue;
                    }
                    if trimmed.is_empty() || trimmed.starts_with("//") {
                        continue;
                    }
                    compacted_line_1 += 1;
                }
            }
            let hash_hex = if is_static || is_manifest {
                blake3::hash(bytecode).to_hex().to_string()
            } else {
                let mut hasher = blake3::Hasher::new();
                hasher.update(normalized_source.as_bytes());
                hasher.update(bytecode);
                hasher.finalize().to_hex().to_string()
            };

            radix_map.insert(specifier.clone(), hash_hex.clone());
            if is_static {
                if specifier == "static:/index.html" {
                    radix_map.insert("static:/".to_string(), hash_hex.clone());
                } else if let Some(dir_prefix) = specifier.strip_suffix("/index.html") {
                    radix_map.insert(dir_prefix.to_string(), hash_hex.clone());
                    radix_map.insert(format!("{}/", dir_prefix), hash_hex.clone());
                }
            } else if !is_manifest {
                let stripped_ext = specifier
                    .trim_end_matches(".tsx")
                    .trim_end_matches(".ts")
                    .trim_end_matches(".jsx")
                    .trim_end_matches(".js")
                    .trim_end_matches(".mjs")
                    .to_string();
                radix_map.insert(stripped_ext.clone(), hash_hex.clone());
                if let Some(bare) = stripped_ext.strip_prefix("./") {
                    radix_map.insert(bare.to_string(), hash_hex.clone());
                }

                if *specifier == self.entrypoint
                    || stripped_ext
                        == self
                            .entrypoint
                            .trim_end_matches(".tsx")
                            .trim_end_matches(".ts")
                            .trim_end_matches(".jsx")
                            .trim_end_matches(".js")
                {
                    entry_hash = hash_hex.clone();
                }
            }

            if !blobs.contains_key(&hash_hex) {
                let rem = data_payload.len() % CACHE_LINE_ALIGN;
                if rem != 0 {
                    data_payload.resize(data_payload.len() + (CACHE_LINE_ALIGN - rem), 0);
                }
                let source_bytes = normalized_source.as_bytes();
                let (source_offset, source_len, bytecode_offset) = if bytecode.len() >= 36
                    && &bytecode[0..8] == b"SLICEBC\x01"
                {
                    let pre_len = u32::from_le_bytes([
                        bytecode[24],
                        bytecode[25],
                        bytecode[26],
                        bytecode[27],
                    ]) as usize;
                    let fb_len = u32::from_le_bytes([
                        bytecode[28],
                        bytecode[29],
                        bytecode[30],
                        bytecode[31],
                    ]) as usize;
                    if 32usize.saturating_add(pre_len).saturating_add(fb_len) == bytecode.len()
                        && fb_len >= 4
                    {
                        let bc_off = data_payload.len() as u64;
                        data_payload.extend_from_slice(bytecode);
                        (
                            bc_off + 32 + (pre_len as u64) + 2,
                            (fb_len - 4) as u64,
                            bc_off,
                        )
                    } else {
                        let src_off = data_payload.len() as u64;
                        data_payload.extend_from_slice(source_bytes);
                        let rem_bc = data_payload.len() % CACHE_LINE_ALIGN;
                        if rem_bc != 0 {
                            data_payload
                                .resize(data_payload.len() + (CACHE_LINE_ALIGN - rem_bc), 0);
                        }
                        let bc_off = data_payload.len() as u64;
                        data_payload.extend_from_slice(bytecode);
                        (src_off, source_bytes.len() as u64, bc_off)
                    }
                } else if bytecode.len() >= 12
                    && &bytecode[0..10] == b"SLICEBOA{\n"
                    && &bytecode[bytecode.len() - 2..] == b"\n}"
                {
                    let bc_off = data_payload.len() as u64;
                    data_payload.extend_from_slice(bytecode);
                    (bc_off + 10, (bytecode.len() - 12) as u64, bc_off)
                } else if bytecode.len() == source_bytes.len() + 8
                    && &bytecode[0..8] == b"SLICEBOA"
                    && &bytecode[8..] == source_bytes
                {
                    let bc_off = data_payload.len() as u64;
                    data_payload.extend_from_slice(bytecode);
                    (bc_off + 8, source_bytes.len() as u64, bc_off)
                } else {
                    let src_off = data_payload.len() as u64;
                    data_payload.extend_from_slice(source_bytes);
                    let rem_bc = data_payload.len() % CACHE_LINE_ALIGN;
                    if rem_bc != 0 {
                        data_payload.resize(data_payload.len() + (CACHE_LINE_ALIGN - rem_bc), 0);
                    }
                    let bc_off = data_payload.len() as u64;
                    data_payload.extend_from_slice(bytecode);
                    (src_off, source_bytes.len() as u64, bc_off)
                };

                blobs.insert(
                    hash_hex.clone(),
                    CasBlobEntry {
                        hash_hex,
                        offset: source_offset,
                        source_len,
                        bytecode_offset,
                        bytecode_len: bytecode.len() as u64,
                        original_specifier: specifier.clone(),
                        is_typescript: specifier.ends_with(".ts") || specifier.ends_with(".tsx"),
                    },
                );
            }
        }

        if entry_hash.is_empty() {
            if let Some((_, first_hash)) = radix_map.iter().next() {
                entry_hash = first_hash.clone();
            }
        }

        let header = CasIndexHeader {
            version: CAS_VERSION,
            entrypoint_specifier: self.entrypoint.clone(),
            entrypoint_hash: entry_hash,
            radix_resolution_map: radix_map,
            blobs,
            source_map_markers,
            lockfile_blake3: self.lockfile_blake3.clone(),
            resolved_dependencies: self.resolved_dependencies.clone(),
        };

        let header_bytes = serde_json::to_vec(&header)?;
        let header_len = header_bytes.len() as u64;

        let pre_data_len = 32 + header_bytes.len();
        let pad_len = (CACHE_LINE_ALIGN - (pre_data_len % CACHE_LINE_ALIGN)) % CACHE_LINE_ALIGN;
        let data_start_offset = (pre_data_len + pad_len) as u64;

        let mut file = File::create(output_path).with_context(|| {
            format!("Failed to create CAS archive at {}", output_path.display())
        })?;
        file.write_all(CAS_MAGIC)?;
        file.write_all(&CAS_VERSION.to_le_bytes())?;
        file.write_all(&0u32.to_le_bytes())?;
        file.write_all(&header_len.to_le_bytes())?;
        file.write_all(&data_start_offset.to_le_bytes())?;
        file.write_all(&header_bytes)?;
        if pad_len > 0 {
            file.write_all(&vec![0u8; pad_len])?;
        }
        file.write_all(&data_payload)?;
        file.flush()?;

        Ok(header)
    }
}
