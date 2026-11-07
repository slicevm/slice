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
#[path = "../tests/mod.rs"]
mod tests;
