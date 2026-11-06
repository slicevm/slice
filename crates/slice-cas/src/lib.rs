//! Zero-Stat Content-Addressable Storage (CAS) and Radix Module Virtualization
//!
//! Eliminates `stat`, `lstat`, and `readdir` system calls during module resolution.
//! Flattens TypeScript/JavaScript/JSX/TSX dependency graphs at build time into a 64-byte
//! cache-line aligned immutable binary archive indexed by BLAKE3-256 hashes and
//! mapped directly into memory via `mmap(MAP_SHARED)`.

pub(crate) mod esm_cjs_linker;
pub(crate) mod oxc_compiler;
pub(crate) mod resolver;
pub(crate) mod static_assets;

pub use oxc_compiler::{
    analyze_typescript_semantics, fuse_array_pipelines_ast, strip_typescript_and_normalize,
    strip_typescript_raw, strip_typescript_raw_with_ext, TypeDirectedHints,
};
pub use resolver::{
    bundle_entrypoint_with_deps, map_bundled_error_location, resolve_project_entrypoint,
};
pub use static_assets::{
    detect_static_mime_type, StaticAssetEntry, StaticAssetPayload, StaticAssetTable,
};

use anyhow::{bail, Context, Result};
use memmap2::Mmap;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const CAS_MAGIC: &[u8; 8] = b"SLICECAS";
pub const CAS_VERSION: u32 = 1;
pub const CACHE_LINE_ALIGN: usize = 64;

/// 32-byte BLAKE3 cryptographic hash identifying a canonicalized module blob.
pub type Blake3Hash = [u8; 32];

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
}

/// Builder for constructing a Zero-Stat Content-Addressed Storage (`.cas`) archive.
pub struct CasBuilder {
    entrypoint: String,
    modules: Vec<(String, PathBuf, String, Vec<u8>)>, // (specifier, path, stripped_source, precompiled_bytecode)
}

impl CasBuilder {
    pub fn new(entrypoint: impl Into<String>) -> Self {
        Self {
            entrypoint: entrypoint.into(),
            modules: Vec::new(),
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
    pub fn ingest_directory(
        &mut self,
        root_dir: &Path,
        bytecode_compiler_fn: &dyn Fn(&str) -> Vec<u8>,
    ) -> Result<usize> {
        let mut count = 0;
        let mut stack = vec![root_dir.to_path_buf()];

        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
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
        Ok(count)
    }

    /// Recursively ingests static web assets (`--static=./public`) into the CAS archive under
    /// `static:/<rel_path>` specifiers so an entire full-stack Web UI + API can be packed into a
    /// single `.cas` archive or compiled into a single `SLICEEXE` executable.
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
            for entry in std::fs::read_dir(&dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.is_file() {
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
                    let pre_len =
                        u32::from_le_bytes([bytecode[24], bytecode[25], bytecode[26], bytecode[27]])
                            as usize;
                    let fb_len =
                        u32::from_le_bytes([bytecode[28], bytecode[29], bytecode[30], bytecode[31]])
                            as usize;
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
                            data_payload.resize(data_payload.len() + (CACHE_LINE_ALIGN - rem_bc), 0);
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

/// Zero-Stat Memory-Mapped Content-Addressed Storage Archive (`mmap(MAP_SHARED)`).
pub struct MappedCasArchive {
    mmap: Arc<Mmap>,
    pub header: CasIndexHeader,
    data_start_offset: usize,
}

impl MappedCasArchive {
    /// Validates and constructs a `MappedCasArchive` from an already mapped `Arc<Mmap>` region
    /// spanning `cas_offset..cas_offset + cas_len`.
    ///
    /// Performs full bounds and UTF-8 validation on the CAS header and every `CasBlobEntry`
    /// once at open time so hot-path `get_source_slice` and `get_bytecode_slice` calls
    /// are guaranteed in-bounds and valid UTF-8 with zero runtime overhead.
    fn from_mmap_region(
        mmap: Arc<Mmap>,
        cas_offset: usize,
        cas_len: usize,
        display_label: &str,
    ) -> Result<Self> {
        let cas_end = cas_offset
            .checked_add(cas_len)
            .filter(|&end| end <= mmap.len() && cas_len >= 32)
            .with_context(|| format!("Invalid CAS archive offset/length in {}", display_label))?;

        let slice = &mmap[cas_offset..cas_end];
        if &slice[0..8] != CAS_MAGIC {
            bail!("Invalid SLICE_CAS magic header in {}", display_label);
        }

        let version = u32::from_le_bytes(slice[8..12].try_into()?);
        if version != CAS_VERSION {
            bail!("Unsupported SLICE_CAS version: {}", version);
        }

        let header_len = usize::try_from(u64::from_le_bytes(slice[16..24].try_into()?))
            .context("CAS header length overflow")?;
        let rel_data_start = usize::try_from(u64::from_le_bytes(slice[24..32].try_into()?))
            .context("CAS data_start_offset overflow")?;

        let header_end = 32usize
            .checked_add(header_len)
            .filter(|&end| end <= slice.len() && end <= rel_data_start)
            .with_context(|| format!("Corrupted or truncated CAS header in {}", display_label))?;

        let header: CasIndexHeader = serde_json::from_slice(&slice[32..header_end])
            .with_context(|| format!("Malformed CAS index JSON in {}", display_label))?;

        let data_start_offset = cas_offset
            .checked_add(rel_data_start)
            .filter(|&off| off <= cas_end)
            .with_context(|| {
                format!(
                    "Invalid CAS data_start_offset out of bounds in {}",
                    display_label
                )
            })?;

        for (hash_key, entry) in &header.blobs {
            let src_off = usize::try_from(entry.offset).context("CAS blob source offset overflow")?;
            let src_len =
                usize::try_from(entry.source_len).context("CAS blob source length overflow")?;
            let src_start = data_start_offset
                .checked_add(src_off)
                .context("CAS blob source start overflow")?;
            let src_end = src_start
                .checked_add(src_len)
                .filter(|&end| end <= cas_end)
                .with_context(|| {
                    format!(
                        "CAS blob '{}' source range [{}..{}] out of bounds (archive end: {})",
                        hash_key,
                        src_start,
                        src_start.saturating_add(src_len),
                        cas_end
                    )
                })?;

            if std::str::from_utf8(&mmap[src_start..src_end]).is_err() {
                bail!(
                    "CAS blob '{}' contains invalid UTF-8 in source slice",
                    hash_key
                );
            }

            let bc_off = usize::try_from(entry.bytecode_offset)
                .context("CAS blob bytecode offset overflow")?;
            let bc_len =
                usize::try_from(entry.bytecode_len).context("CAS blob bytecode length overflow")?;
            let bc_start = data_start_offset
                .checked_add(bc_off)
                .context("CAS blob bytecode start overflow")?;
            let _bc_end = bc_start
                .checked_add(bc_len)
                .filter(|&end| end <= cas_end)
                .with_context(|| {
                    format!(
                        "CAS blob '{}' bytecode range [{}..{}] out of bounds (archive end: {})",
                        hash_key,
                        bc_start,
                        bc_start.saturating_add(bc_len),
                        cas_end
                    )
                })?;
        }

        Ok(Self {
            mmap,
            header,
            data_start_offset,
        })
    }

    /// Opens and memory-maps a `.cas` archive file with zero heap copies of module payloads.
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path)
            .with_context(|| format!("Failed to open CAS archive {}", path.display()))?;
        let mmap = unsafe { Mmap::map(&file)? };
        let len = mmap.len();
        Self::from_mmap_region(Arc::new(mmap), 0, len, &path.display().to_string())
    }

    /// Resolves an import specifier in O(1) time without touching the OS filesystem.
    #[inline]
    pub fn resolve_specifier(&self, specifier: &str) -> Option<&CasBlobEntry> {
        let hash = self
            .header
            .radix_resolution_map
            .get(specifier)
            .or_else(|| {
                let trimmed = specifier.trim_start_matches("./");
                self.header.radix_resolution_map.get(trimmed)
            })?;
        self.header.blobs.get(hash)
    }

    /// Returns a zero-copy slice to the normalized JavaScript source directly from the `mmap` region.
    #[inline]
    pub fn get_source_slice<'a>(&'a self, entry: &CasBlobEntry) -> &'a str {
        let Some(start) = self.data_start_offset.checked_add(entry.offset as usize) else {
            return "";
        };
        let Some(end) = start.checked_add(entry.source_len as usize) else {
            return "";
        };
        let Some(bytes) = self.mmap.get(start..end) else {
            return "";
        };
        std::str::from_utf8(bytes).unwrap_or("")
    }

    /// Returns a zero-copy slice to the pre-compiled bytecode directly from the `mmap` region.
    #[inline]
    pub fn get_bytecode_slice<'a>(&'a self, entry: &CasBlobEntry) -> &'a [u8] {
        let Some(start) = self
            .data_start_offset
            .checked_add(entry.bytecode_offset as usize)
        else {
            return &[];
        };
        let Some(end) = start.checked_add(entry.bytecode_len as usize) else {
            return &[];
        };
        self.mmap.get(start..end).unwrap_or(&[])
    }

    /// Returns a clone of the underlying `Arc<Mmap>` and the validated `(start, end)` byte range
    /// for `entry`'s bytecode/payload so callers can share zero-copy `mmap` slices across threads.
    #[inline]
    pub fn get_shared_bytecode_view(&self, entry: &CasBlobEntry) -> Option<(Arc<Mmap>, usize, usize)> {
        let start = self
            .data_start_offset
            .checked_add(entry.bytecode_offset as usize)?;
        let end = start.checked_add(entry.bytecode_len as usize)?;
        if end <= self.mmap.len() {
            Some((Arc::clone(&self.mmap), start, end))
        } else {
            None
        }
    }

    /// Returns the entrypoint blob entry.
    #[inline]
    pub fn entrypoint_blob(&self) -> Option<&CasBlobEntry> {
        self.header.blobs.get(&self.header.entrypoint_hash)
    }

    /// Returns the embedded `slice.toml` manifest string if one was packed into this `.cas` archive.
    #[inline]
    pub fn get_manifest_toml(&self) -> Option<&str> {
        let entry = self.resolve_specifier("manifest:/slice.toml")?;
        Some(self.get_source_slice(entry))
    }

    /// Reconstructs the `// #slice-source:` annotated JS source from `self.header.source_map_markers`
    /// so `.cas` archives and standalone `SLICEEXE` binaries map uncaught errors back to exact TypeScript lines.
    pub fn reconstruct_annotated_source(&self, entry: &CasBlobEntry) -> String {
        let src = self.get_source_slice(entry);
        if self.header.source_map_markers.is_empty() || src.contains("// #slice-source: ") {
            return src.to_string();
        }
        let mut out = String::with_capacity(src.len() + self.header.source_map_markers.len() * 64);
        for (idx, line) in src.lines().enumerate() {
            let line_1 = idx + 1;
            for (m_line, marker_rest) in &self.header.source_map_markers {
                if *m_line == line_1 {
                    out.push_str("// #slice-source: ");
                    out.push_str(marker_rest);
                    out.push('\n');
                }
            }
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    /// Opens a `.cas` archive embedded at `cas_offset` inside a larger file (such as a standalone executable).
    pub fn open_from_offset(path: &Path, cas_offset: usize, cas_len: usize) -> Result<Self> {
        let file = File::open(path)
            .with_context(|| format!("Failed to open standalone binary {}", path.display()))?;
        let mmap = unsafe { Mmap::map(&file)? };
        Self::from_mmap_region(
            Arc::new(mmap),
            cas_offset,
            cas_len,
            &path.display().to_string(),
        )
    }

    /// Checks if the current running executable has an embedded `SLICEEXE` trailer and maps it in O(1)
    /// using a single `Mmap` call.
    pub fn open_embedded_self() -> Result<Option<Self>> {
        let exe_path = std::env::current_exe()?;
        let file = File::open(&exe_path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        if mmap.len() < 24 {
            return Ok(None);
        }
        let tail_start = mmap.len() - 24;
        if &mmap[tail_start + 16..tail_start + 24] != b"SLICEEXE" {
            return Ok(None);
        }
        let cas_offset = usize::try_from(u64::from_le_bytes(
            mmap[tail_start..tail_start + 8].try_into()?,
        ))
        .context("SLICEEXE cas_offset overflow")?;
        let cas_len = usize::try_from(u64::from_le_bytes(
            mmap[tail_start + 8..tail_start + 16].try_into()?,
        ))
        .context("SLICEEXE cas_len overflow")?;
        let archive = Self::from_mmap_region(
            Arc::new(mmap),
            cas_offset,
            cas_len,
            &exe_path.display().to_string(),
        )?;
        Ok(Some(archive))
    }
}

/// Embeds a compiled `.cas` archive into a copy of the `slice` runtime binary to produce a
/// zero-dependency standalone executable (`slice compile`).
pub fn embed_cas_into_executable(
    base_exe_path: &Path,
    cas_file_path: &Path,
    output_exe_path: &Path,
) -> Result<u64> {
    let mut exe_bytes = std::fs::read(base_exe_path)
        .with_context(|| format!("Failed to read base executable {}", base_exe_path.display()))?;
    let cas_bytes = std::fs::read(cas_file_path)
        .with_context(|| format!("Failed to read CAS archive {}", cas_file_path.display()))?;

    let rem = exe_bytes.len() % CACHE_LINE_ALIGN;
    if rem != 0 {
        exe_bytes.resize(exe_bytes.len() + (CACHE_LINE_ALIGN - rem), 0);
    }

    let cas_offset = exe_bytes.len() as u64;
    let cas_len = cas_bytes.len() as u64;

    exe_bytes.extend_from_slice(&cas_bytes);
    exe_bytes.extend_from_slice(&cas_offset.to_le_bytes());
    exe_bytes.extend_from_slice(&cas_len.to_le_bytes());
    exe_bytes.extend_from_slice(b"SLICEEXE");

    std::fs::write(output_exe_path, &exe_bytes).with_context(|| {
        format!(
            "Failed to write standalone executable to {}",
            output_exe_path.display()
        )
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(output_exe_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(output_exe_path, perms)?;
    }

    Ok(exe_bytes.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ts_stripping_and_cas_roundtrip() {
        let ts_code = r#"
interface User {
    id: number;
    name: string;
}
type ID = string;
const x: number = 42;
function add(a: number, b: number): number {
    return a + b;
}
"#;
        let stripped = strip_typescript_and_normalize(ts_code, true);
        assert!(!stripped.contains("interface User"));
        assert!(!stripped.contains("type ID"));
        assert!(stripped.contains("const x = 42;"));

        let temp_path = std::env::temp_dir().join("slice_test_roundtrip.cas");
        let mut builder = CasBuilder::new("./index.ts");
        builder.add_module("./index.ts", PathBuf::from("index.ts"), ts_code, |src| {
            src.as_bytes().to_vec()
        });
        builder.write_to_file(&temp_path).unwrap();

        let archive = MappedCasArchive::open(&temp_path).unwrap();
        let entry = archive.resolve_specifier("./index.ts").unwrap();
        let loaded_src = archive.get_source_slice(entry);
        assert!(loaded_src.contains("const x = 42;"));
        let _ = std::fs::remove_file(&temp_path);
    }

    #[test]
    fn test_folder_entrypoint_and_esm_cjs_json_bundler() {
        let test_dir = std::env::temp_dir().join("slice_cas_bundler_unit_test");
        let _ = std::fs::remove_dir_all(&test_dir);
        let src_dir = test_dir.join("src");
        let nm_dir = test_dir.join("node_modules").join("mini-pkg");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::create_dir_all(&nm_dir).unwrap();

        std::fs::write(
            test_dir.join("package.json"),
            r#"{"name":"test-app","main":"./src/index.ts"}"#,
        )
        .unwrap();
        std::fs::write(src_dir.join("config.json"), r#"{"multiplier": 3}"#).unwrap();
        std::fs::write(
            src_dir.join("helper.ts"),
            r#"export function scale(v: number, m: number): number { return v * m; }"#,
        )
        .unwrap();
        std::fs::write(
            nm_dir.join("package.json"),
            r#"{"name":"mini-pkg","main":"index.js"}"#,
        )
        .unwrap();
        std::fs::write(
            nm_dir.join("index.js"),
            r#"module.exports = function addBase(n) { return n + 10; };"#,
        )
        .unwrap();
        std::fs::write(
            src_dir.join("index.ts"),
            r#"
import addBase from "mini-pkg";
import { scale } from "./helper.ts";
import cfg from "./config.json";
console.log(scale(addBase(4), cfg.multiplier));
"#,
        )
        .unwrap();

        let resolved = resolve_project_entrypoint(&test_dir, None).unwrap();
        assert!(resolved.ends_with("index.ts"));

        let bundled = bundle_entrypoint_with_deps(&resolved).unwrap();
        assert!(bundled.contains("var __slice_modules = {};"));
        assert!(bundled.contains("multiplier"));

        let _ = std::fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn test_embed_cas_into_executable_roundtrip() {
        let tmp = std::env::temp_dir();
        let fake_exe = tmp.join("slice_fake_base.bin");
        let cas_path = tmp.join("slice_embed_test.cas");
        let out_exe = tmp.join("slice_standalone_out.bin");

        std::fs::write(&fake_exe, vec![0x90u8; 100]).unwrap();
        let mut builder = CasBuilder::new("./index.ts");
        builder.add_module(
            "./index.ts",
            PathBuf::from("index.ts"),
            "const a = 7;",
            |s| s.as_bytes().to_vec(),
        );
        builder.write_to_file(&cas_path).unwrap();

        let total_size = embed_cas_into_executable(&fake_exe, &cas_path, &out_exe).unwrap();
        assert!(total_size > 128);

        let raw_out = std::fs::read(&out_exe).unwrap();
        let tail = &raw_out[raw_out.len() - 24..];
        assert_eq!(&tail[16..24], b"SLICEEXE");
        let cas_offset = u64::from_le_bytes(tail[0..8].try_into().unwrap()) as usize;
        let cas_len = u64::from_le_bytes(tail[8..16].try_into().unwrap()) as usize;
        assert_eq!(cas_offset % CACHE_LINE_ALIGN, 0);

        let opened = MappedCasArchive::open_from_offset(&out_exe, cas_offset, cas_len).unwrap();
        let entry = opened.entrypoint_blob().unwrap();
        assert!(opened.get_source_slice(entry).contains("const a = 7;"));

        let _ = std::fs::remove_file(&fake_exe);
        let _ = std::fs::remove_file(&cas_path);
        let _ = std::fs::remove_file(&out_exe);
    }

    #[test]
    fn test_tag_f4_source_line_error_mapping_across_multi_file_bundle() {
        let test_dir = std::env::temp_dir().join("slice_tag_f4_sourcemap_test");
        let _ = std::fs::remove_dir_all(&test_dir);
        let src_dir = test_dir.join("src");
        std::fs::create_dir_all(&src_dir).unwrap();

        let validator_ts = r#"export interface OrderSpec {
    id: string;
    amountCents: number;
}
export function assertValidOrder(order: OrderSpec): number {
    if (order.amountCents < 0) throw new Error("Negative order amount forbidden");
    return order.amountCents;
}
"#;
        let index_ts = r#"import { assertValidOrder } from "./pricing_validator.ts";
console.log(assertValidOrder({ id: "ord-1", amountCents: -50 }));
"#;
        std::fs::write(src_dir.join("pricing_validator.ts"), validator_ts).unwrap();
        std::fs::write(src_dir.join("index.ts"), index_ts).unwrap();

        let bundled = bundle_entrypoint_with_deps(&src_dir.join("index.ts")).unwrap();
        assert!(bundled.contains("// #slice-source: src/pricing_validator.ts|"));
        assert!(bundled.contains("// #slice-source: src/index.ts|"));

        let mapped = map_bundled_error_location(
            &bundled,
            r#"Uncaught Error: "Negative order amount forbidden""#,
        );
        assert!(
            mapped.contains("(src/pricing_validator.ts:6)"),
            "Expected mapped error location to point to (src/pricing_validator.ts:6), got: {}",
            mapped
        );

        let _ = std::fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn test_phase_n0_oxc_ast_array_pipeline_fusion_and_unmodified_node_modules() {
        let raw_code = r#"
const orders = [{ price: 120, qty: 2, active: true }, { price: 50, qty: 5, active: false }, { price: 300, qty: 1, active: true }];
const totalGross = orders.map((item) => item.price * item.qty).reduce((acc, v) => acc + v, 0);
const activeRevenue = orders.filter((o) => o.active).map((o) => o.price * o.qty).reduce((sum, val) => sum + val, 0);
const hasHighValue = orders.some((entry) => entry.price >= 250);
const allPositive = orders.every((entry) => entry.qty > 0);
"#;
        let fused = fuse_array_pipelines_ast(raw_code);
        assert!(
            !fused.contains(".map(")
                && !fused.contains(".reduce(")
                && !fused.contains(".some(")
                && !fused.contains(".every("),
            "Expected all higher-order array pipelines to be fused into single-pass for loops, got:\n{}",
            fused
        );
        assert!(fused.contains("for (let __sl_i1 = 0;"));
        assert!(fused.contains("for (let __sl_i2 = 0;"));
        assert!(fused.contains("for (let __sl_i3 = 0;"));
        assert!(fused.contains("for (let __sl_i4 = 0;"));

        let test_dir = std::env::temp_dir().join("slice_phase_n0_real_node_modules_test");
        let _ = std::fs::remove_dir_all(&test_dir);
        let nm_ms_dir = test_dir.join("node_modules").join("ms");
        std::fs::create_dir_all(&nm_ms_dir).unwrap();
        std::fs::write(
            nm_ms_dir.join("package.json"),
            r#"{"name":"ms","main":"index.js"}"#,
        )
        .unwrap();
        std::fs::write(
            nm_ms_dir.join("index.js"),
            r#"module.exports = function customMsFromDisk(val) { return "disk-ms:" + val; };"#,
        )
        .unwrap();
        let entry_path = test_dir.join("index.ts");
        std::fs::write(
            &entry_path,
            r#"import ms from "ms"; console.log(ms("5m"));"#,
        )
        .unwrap();

        let bundled = bundle_entrypoint_with_deps(&entry_path).unwrap();
        assert!(
            bundled.contains("customMsFromDisk") && bundled.contains("disk-ms:"),
            "Expected bundler to preserve actual node_modules/ms/index.js source from disk, got:\n{}",
            bundled
        );

        let _ = std::fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn test_phase_n4_oxc_ast_cjs_transform_and_package_exports() {
        let test_dir = std::env::temp_dir().join("slice_phase_n4_exports_and_ast_test");
        let _ = std::fs::remove_dir_all(&test_dir);
        let src_dir = test_dir.join("src");
        let scoped_pkg_dir = test_dir.join("node_modules").join("@edge").join("kit");
        let wildcard_pkg_dir = test_dir.join("node_modules").join("mini-hono");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::create_dir_all(scoped_pkg_dir.join("dist")).unwrap();
        std::fs::create_dir_all(wildcard_pkg_dir.join("dist").join("middleware")).unwrap();

        std::fs::write(
            scoped_pkg_dir.join("package.json"),
            r#"{
  "name": "@edge/kit",
  "exports": {
    ".": {
      "types": "./dist/index.d.ts",
      "workerd": {
        "import": "./dist/edge-workerd.js",
        "default": "./dist/edge-workerd.js"
      },
      "default": "./dist/fallback.js"
    }
  }
}"#,
        )
        .unwrap();
        std::fs::write(
            scoped_pkg_dir.join("dist").join("edge-workerd.js"),
            r#"export const RUNTIME_TARGET = "workerd-edge"; export default function kitVersion() { return "kit@4.0"; }"#,
        )
        .unwrap();

        std::fs::write(
            wildcard_pkg_dir.join("package.json"),
            r#"{
  "name": "mini-hono",
  "exports": {
    ".": "./dist/index.js",
    "./*": {
      "import": "./dist/middleware/*.js",
      "default": "./dist/middleware/*.js"
    }
  }
}"#,
        )
        .unwrap();
        std::fs::write(
            wildcard_pkg_dir
                .join("dist")
                .join("middleware")
                .join("jwt.js"),
            r#"export const { ALG_HS256, ALG_HS512 } = { ALG_HS256: "HS256", ALG_HS512: "HS512" };"#,
        )
        .unwrap();

        std::fs::write(
            src_dir.join("helper.ts"),
            r#"export function normalizeRoute(p: string): string { return p.trim().toLowerCase(); }"#,
        )
        .unwrap();

        let entry_ts = src_dir.join("index.ts");
        std::fs::write(
            &entry_ts,
            r#"import kitVersion, {
    RUNTIME_TARGET as targetName
} from "@edge/kit";
import { ALG_HS256 } from "mini-hono/jwt";
import { normalizeRoute } from "./helper.js";

export const summary = `${kitVersion()}:${targetName}:${ALG_HS256}:${normalizeRoute(" /API/V1 ")}`;
"#,
        )
        .unwrap();

        let bundled = bundle_entrypoint_with_deps(&entry_ts).unwrap();
        assert!(
            bundled.contains("workerd-edge")
                && bundled.contains("ALG_HS256")
                && bundled.contains("normalizeRoute"),
            "Expected bundled output to resolve nested workerd exports, wildcard subpaths, and .js->.ts imports:\n{}",
            bundled
        );

        let _ = std::fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn test_oxc_native_jsx_tsx_compilation_and_bundling() {
        let tsx_src = r#"
import { renderToString } from "slice:jsx";

interface CardProps {
    title: string;
    items: string[];
}

export function Card({ title, items }: CardProps) {
    return (
        <section className="card" data-count={items.length}>
            <h2>{title}</h2>
            <ul>
                {items.map((it) => <li>{it}</li>)}
            </ul>
        </section>
    );
}
"#;
        let compiled = strip_typescript_and_normalize(tsx_src, true);
        assert!(
            !compiled.contains("interface CardProps") && !compiled.contains("<section"),
            "Expected JSX and TypeScript types to be compiled away by Oxc, got:\n{}",
            compiled
        );
        assert!(
            compiled.contains("slice:jsx"),
            "Expected automatic JSX runtime import to resolve to slice:jsx builtin:\n{}",
            compiled
        );
    }

    #[test]
    fn test_general_oxc_ast_dead_export_tree_shaker_and_cold_method_inliner() {
        let test_dir = std::env::temp_dir().join("slice_cas_tree_shaker_and_cold_inliner_test");
        let _ = std::fs::remove_dir_all(&test_dir);
        let src_dir = test_dir.join("src");
        let pkg_dir = test_dir.join("node_modules").join("synth-kit");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::create_dir_all(&pkg_dir).unwrap();

        std::fs::write(
            pkg_dir.join("package.json"),
            r#"{"name":"synth-kit","module":"./index.js"}"#,
        )
        .unwrap();
        std::fs::write(
            pkg_dir.join("cold_body.js"),
            r#"export function parseBody(raw) { return "parsed:" + raw; }"#,
        )
        .unwrap();
        std::fs::write(
            pkg_dir.join("unused_sub.js"),
            r#"export function neverCalledSub() { return "dead-sub-module"; }"#,
        )
        .unwrap();
        std::fs::write(
            pkg_dir.join("index.js"),
            r#"
import { parseBody } from "./cold_body.js";
import { neverCalledSub } from "./unused_sub.js";

function deadRecursiveHelper(n) {
    return n <= 0 ? neverCalledSub() : deadRecursiveHelper(n - 1);
}

export function unusedExportFn(x) {
    return deadRecursiveHelper(x);
}

export class SynthRequest {
    constructor(url) {
        this.url = url;
    }
    async parseBody() {
        return parseBody(this.url);
    }
}
"#,
        )
        .unwrap();

        let entry_ts = src_dir.join("index.ts");
        std::fs::write(
            &entry_ts,
            r#"
import { SynthRequest } from "synth-kit";
const req = new SynthRequest("/v1/ping");
console.log(req.url);
"#,
        )
        .unwrap();

        let bundled = bundle_entrypoint_with_deps(&entry_ts).unwrap();
        assert!(
            !bundled.contains("unusedExportFn")
                && !bundled.contains("deadRecursiveHelper")
                && !bundled.contains("neverCalledSub"),
            "Expected Oxc AST dead-export tree-shaker to prune unused exports, self-recursive helpers, and transitive dead modules:\n{}",
            bundled
        );
        assert!(
            bundled.contains(r#"__slice_modules[0] = ""#),
            "Expected cold-method-only dependency cold_body.js to be emitted as a lazy JSON string module:\n{}",
            bundled
        );

        let _ = std::fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn test_wave_g_oxc_ast_typescript_semantics_and_string_literal_require_safety() {
        let ts_src = r#"
export interface OrderPayload {
    readonly orderId: string;
    totalCents: number;
    settled: boolean;
    status: "pending" | "paid";
}

const msg = "do not touch require('fs') inside string literal or fake: number";
const obj = { fakeObjField: number_ref };
export function processOrder(order: OrderPayload, discount: number, active: boolean): string {
    const label: string = `template require("crypto") stays untouched`;
    const fsMod = require("fs");
    return label + typeof fsMod + order.orderId + String(discount) + String(active);
}
"#;
        let hints = analyze_typescript_semantics(ts_src);
        assert!(hints.numeric_vars.contains("discount"));
        assert!(hints.boolean_vars.contains("active"));
        assert!(hints.string_vars.contains("label"));
        assert!(
            !hints.numeric_vars.contains("fake") && !hints.numeric_vars.contains("fakeObjField"),
            "Object literal and string literal colons must not be misclassified as TypeScript type annotations"
        );
        assert!(
            !hints.interface_schemas.contains_key("JournalRecord"),
            "No hardcoded JournalRecord schema should be injected"
        );
        let schema = hints
            .interface_schemas
            .get("OrderPayload")
            .expect("OrderPayload schema");
        assert_eq!(
            schema,
            &vec![
                ("orderId".to_string(), "string".to_string()),
                ("totalCents".to_string(), "number".to_string()),
                ("settled".to_string(), "boolean".to_string()),
                ("status".to_string(), "string".to_string()),
            ]
        );
        assert_eq!(
            hints.interface_vars.get("order").map(|s| s.as_str()),
            Some("OrderPayload")
        );

        let normalized = strip_typescript_and_normalize(ts_src, true);
        assert!(
            normalized.contains("do not touch require('fs') inside string literal"),
            "String literal containing require('fs') must not be rewritten:\n{}",
            normalized
        );
        assert!(
            normalized.contains("template require(\"crypto\") stays untouched"),
            "Template literal containing require(\"crypto\") must not be rewritten:\n{}",
            normalized
        );
        assert!(
            normalized.contains("__slice_cjs_require(\"fs\")")
                || normalized.contains("__slice_cjs_require(\"node:fs\")"),
            "Actual CallExpression require(\"fs\") must be rewritten:\n{}",
            normalized
        );
    }

    #[test]
    fn test_wave_h_single_pass_ast_bundler_and_array_module_table() {
        let test_dir = std::env::temp_dir().join("slice_wave_h_single_pass_bundler_test");
        let _ = std::fs::remove_dir_all(&test_dir);
        std::fs::create_dir_all(test_dir.join("src")).unwrap();
        std::fs::create_dir_all(test_dir.join("node_modules/pkg-cjs")).unwrap();

        std::fs::write(
            test_dir.join("node_modules/pkg-cjs/package.json"),
            r#"{"name":"pkg-cjs","main":"index.js"}"#,
        )
        .unwrap();
        std::fs::write(
            test_dir.join("node_modules/pkg-cjs/sub.js"),
            r#"module.exports = { compute: function(x) { return x * 3; } };"#,
        )
        .unwrap();
        std::fs::write(
            test_dir.join("node_modules/pkg-cjs/index.js"),
            r#"const sub = require("./sub");
module.exports = {
  sub,
};"#,
        )
        .unwrap();

        let entry_path = test_dir.join("src/index.ts");
        std::fs::write(
            &entry_path,
            r#"import pkg from "pkg-cjs";
const literalNote = "require('./sub') inside string literal stays untouched";
export const answer = pkg.sub.compute(14) + ":" + literalNote;
console.log(answer);
"#,
        )
        .unwrap();

        let bundled =
            bundle_entrypoint_with_deps(&entry_path).expect("bundle_entrypoint_with_deps");
        assert!(
            bundled.contains("__slice_modules = [];"),
            "Wave H bundle must initialize dense JS Array __slice_modules = []:\n{}",
            bundled
        );
        assert!(
            bundled.contains("get sub() { return require(0); }")
                || bundled.contains("get sub(){return require(0)}"),
            "Single-pass CJS transform must preserve lazy barrel getter optimization:\n{}",
            bundled
        );
        assert!(
            bundled.contains("require('./sub') inside string literal stays untouched"),
            "Single-pass span rewriter must not touch string literals:\n{}",
            bundled
        );

        let _ = std::fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn test_wave_i_tsconfig_paths_pkg_imports_meta_dynamic_import_and_embedded_sourcemap() {
        let test_dir = std::env::temp_dir().join("slice_wave_i_dx_resolution_test");
        let _ = std::fs::remove_dir_all(&test_dir);
        std::fs::create_dir_all(test_dir.join("src/utils")).unwrap();
        std::fs::create_dir_all(test_dir.join("src/internal")).unwrap();
        std::fs::create_dir_all(test_dir.join("src/lazy")).unwrap();
        std::fs::create_dir_all(test_dir.join("config")).unwrap();

        // 1. Base tsconfig and child tsconfig with JSONC comments + trailing commas
        std::fs::write(
            test_dir.join("tsconfig.base.json"),
            r#"{
  // Base TypeScript configuration with JSONC comments
  "compilerOptions": {
    "baseUrl": ".",
    "paths": {
      "@/*": ["./src/*"],
      "~config": ["./config/app.ts"],
    },
  },
}"#,
        )
        .unwrap();
        std::fs::write(
            test_dir.join("tsconfig.json"),
            r#"{
  /* Child config extending base */
  "extends": "./tsconfig.base.json",
}"#,
        )
        .unwrap();

        // 2. package.json with Node.js subpath `"imports"` (`#...`)
        std::fs::write(
            test_dir.join("package.json"),
            r##"{
  "name": "wave-i-app",
  "imports": {
    "#internal/*": "./src/internal/*.ts"
  }
}"##,
        )
        .unwrap();

        std::fs::write(
            test_dir.join("src/utils/math.ts"),
            r#"export function doubleVal(n: number): number {
  return n * 2;
}
"#,
        )
        .unwrap();

        std::fs::write(
            test_dir.join("config/app.ts"),
            r#"export const APP_TITLE = "SliceWaveI";
"#,
        )
        .unwrap();

        std::fs::write(
            test_dir.join("src/internal/secret.ts"),
            r#"export const INTERNAL_TAG = "subpath-ok";
"#,
        )
        .unwrap();

        std::fs::write(
            test_dir.join("src/lazy/feature.ts"),
            r#"export function lazyBanner(): string {
  return "dynamic-import-ok";
}
"#,
        )
        .unwrap();

        let entry_path = test_dir.join("src/index.ts");
        std::fs::write(
            &entry_path,
            r##"import { doubleVal } from "@/utils/math";
import { APP_TITLE } from "~config";
import { INTERNAL_TAG } from "#internal/secret";

export interface UnusedTypeBlock {
  alpha: string;
  beta: number;
  gamma: boolean;
}

const metaUrl = import.meta.url;
const metaDir = import.meta.dirname;
const cjsDir = __dirname;
const dynMod = await import("@/lazy/feature");
if (doubleVal(21) === 42) {
  throw new Error("wave-i-exact-line-check");
}
console.log(APP_TITLE, INTERNAL_TAG, metaUrl, metaDir, cjsDir, dynMod.lazyBanner());
"##,
        )
        .unwrap();

        let bundled =
            bundle_entrypoint_with_deps(&entry_path).expect("Wave I bundle_entrypoint_with_deps");

        assert!(
            bundled.contains("async function()"),
            "Top-level await must wrap entrypoint runner in async function():\n{}",
            bundled
        );
        assert!(
            bundled.contains("var __filename =") && bundled.contains("__dirname ="),
            "__dirname reference must inject __filename and __dirname bindings:\n{}",
            bundled
        );
        assert!(
            bundled.contains("file:///"),
            "import.meta.url must lower to file:// URL:\n{}",
            bundled
        );

        // Delete the entire source directory from disk to prove embedded line-map works offline
        let _ = std::fs::remove_dir_all(&test_dir);

        let mapped =
            map_bundled_error_location(&bundled, "Uncaught Error: \"wave-i-exact-line-check\"");
        assert!(
            mapped.contains("(src/index.ts:16)"),
            "Embedded #slice-source line map must resolve exact TypeScript line 16 without disk access, got: {}",
            mapped
        );
    }

    #[test]
    fn test_ts_generics_with_html_strings_and_template_comment_preservation() {
        let ts_src = r#"
export const identity = <T>(val: T): T => val;
export const htmlTemplate = `<div class="panel">
  // line inside template literal starting with double slash
  /* block comment syntax inside template literal */
  <span>Hello</span>
</div>`;
console.log(identity(htmlTemplate));
"#;
        let out = strip_typescript_raw_with_ext(ts_src, true, false);
        assert!(
            out.contains("// line inside template literal starting with double slash"),
            "Template literal lines starting with // must be preserved verbatim:\n{}",
            out
        );
        assert!(
            out.contains("/* block comment syntax inside template literal */"),
            "Template literal lines starting with /* must be preserved verbatim:\n{}",
            out
        );
        assert!(
            out.contains("identity = (val) => val"),
            "TypeScript <T> generic arrow function must not be misparsed as JSX when HTML strings exist:\n{}",
            out
        );
    }

    #[test]
    fn test_corrupted_cas_bounds_and_utf8_rejection() {
        let temp_dir = std::env::temp_dir().join("slice_cas_safety_test");
        let _ = std::fs::create_dir_all(&temp_dir);

        // 1. Valid archive baseline
        let valid_path = temp_dir.join("valid.cas");
        let mut builder = CasBuilder::new("./index.js");
        builder.add_module(
            "./index.js",
            PathBuf::from("index.js"),
            "console.log('ok');",
            |src| src.as_bytes().to_vec(),
        );
        builder.write_to_file(&valid_path).unwrap();
        let valid_bytes = std::fs::read(&valid_path).unwrap();
        let opened = MappedCasArchive::open(&valid_path).unwrap();
        let entry = opened.entrypoint_blob().unwrap();
        let src_byte_pos = opened.data_start_offset + (entry.offset as usize);
        drop(opened);

        // 2. Corrupted header_len exceeding file size must return Err, not panic
        let mut bad_hdr = valid_bytes.clone();
        bad_hdr[16..24].copy_from_slice(&(999_999u64).to_le_bytes());
        let bad_hdr_path = temp_dir.join("bad_hdr.cas");
        std::fs::write(&bad_hdr_path, &bad_hdr).unwrap();
        assert!(MappedCasArchive::open(&bad_hdr_path).is_err());

        // 3. Invalid UTF-8 inside source payload must return Err on open, preventing UB
        let mut bad_utf8 = valid_bytes.clone();
        bad_utf8[src_byte_pos] = 0xFF;
        let bad_utf8_path = temp_dir.join("bad_utf8.cas");
        std::fs::write(&bad_utf8_path, &bad_utf8).unwrap();
        assert!(MappedCasArchive::open(&bad_utf8_path).is_err());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_audit_neo_phase4_zero_copy_static_mmap_and_resolver_cache() {
        let temp_dir = std::env::temp_dir().join("slice_cas_phase4_test");
        let _ = std::fs::remove_dir_all(&temp_dir);
        let static_dir = temp_dir.join("public");
        std::fs::create_dir_all(&static_dir).unwrap();
        std::fs::write(static_dir.join("index.html"), "<h1>Zero-Copy CAS Asset</h1>").unwrap();

        let cas_path = temp_dir.join("static_test.cas");
        let mut builder = CasBuilder::new("./index.js");
        builder.add_module(
            "./index.js",
            PathBuf::from("index.js"),
            "console.log('ready');",
            |src| src.as_bytes().to_vec(),
        );
        builder.ingest_static_directory(&static_dir).unwrap();
        builder.write_to_file(&cas_path).unwrap();

        let archive = MappedCasArchive::open(&cas_path).unwrap();
        let table = StaticAssetTable::from_cas_archive(&archive);
        let root_asset = table.resolve("/").expect("Expected /index.html at /");
        assert_eq!(&root_asset.payload[..], b"<h1>Zero-Copy CAS Asset</h1>");
        assert!(
            matches!(root_asset.payload, StaticAssetPayload::Mapped { .. }),
            "Expected StaticAssetTable::from_cas_archive to use zero-copy StaticAssetPayload::Mapped"
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
