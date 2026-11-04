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
    strip_typescript_raw, TypeDirectedHints,
};
pub use resolver::{
    bundle_entrypoint_with_deps, map_bundled_error_location, resolve_project_entrypoint,
};
pub use static_assets::{detect_static_mime_type, StaticAssetEntry, StaticAssetTable};

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

        for (specifier, _path, normalized_source, bytecode) in &self.modules {
            let is_static = specifier.starts_with("static:/");
            let is_manifest = specifier == "manifest:/slice.toml";
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
                let (source_offset, source_len, bytecode_offset) = if bytecode.len() >= 12
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
    /// Opens and memory-maps a `.cas` archive file with zero heap copies of module payloads.
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path)
            .with_context(|| format!("Failed to open CAS archive {}", path.display()))?;
        let mmap = unsafe { Mmap::map(&file)? };

        if mmap.len() < 32 || &mmap[0..8] != CAS_MAGIC {
            bail!("Invalid SLICE_CAS magic header in {}", path.display());
        }

        let version = u32::from_le_bytes(mmap[8..12].try_into()?);
        if version != CAS_VERSION {
            bail!("Unsupported SLICE_CAS version: {}", version);
        }

        let header_len = u64::from_le_bytes(mmap[16..24].try_into()?) as usize;
        let data_start_offset = u64::from_le_bytes(mmap[24..32].try_into()?) as usize;

        let header: CasIndexHeader = serde_json::from_slice(&mmap[32..32 + header_len])?;

        Ok(Self {
            mmap: Arc::new(mmap),
            header,
            data_start_offset,
        })
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
        let start = self.data_start_offset + entry.offset as usize;
        let end = start + entry.source_len as usize;
        unsafe { std::str::from_utf8_unchecked(&self.mmap[start..end]) }
    }

    /// Returns a zero-copy slice to the pre-compiled bytecode directly from the `mmap` region.
    #[inline]
    pub fn get_bytecode_slice<'a>(&'a self, entry: &CasBlobEntry) -> &'a [u8] {
        let start = self.data_start_offset + entry.bytecode_offset as usize;
        let end = start + entry.bytecode_len as usize;
        &self.mmap[start..end]
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

    /// Opens a `.cas` archive embedded at `cas_offset` inside a larger file (such as a standalone executable).
    pub fn open_from_offset(path: &Path, cas_offset: usize, cas_len: usize) -> Result<Self> {
        let file = File::open(path)
            .with_context(|| format!("Failed to open standalone binary {}", path.display()))?;
        let mmap = unsafe { Mmap::map(&file)? };

        if cas_offset + cas_len > mmap.len() || cas_len < 32 {
            bail!("Invalid embedded CAS offset/length in {}", path.display());
        }
        let slice = &mmap[cas_offset..cas_offset + cas_len];
        if &slice[0..8] != CAS_MAGIC {
            bail!("Invalid embedded SLICE_CAS magic header");
        }

        let version = u32::from_le_bytes(slice[8..12].try_into()?);
        if version != CAS_VERSION {
            bail!("Unsupported SLICE_CAS version: {}", version);
        }

        let header_len = u64::from_le_bytes(slice[16..24].try_into()?) as usize;
        let rel_data_start = u64::from_le_bytes(slice[24..32].try_into()?) as usize;
        let header: CasIndexHeader = serde_json::from_slice(&slice[32..32 + header_len])?;

        Ok(Self {
            mmap: Arc::new(mmap),
            header,
            data_start_offset: cas_offset + rel_data_start,
        })
    }

    /// Checks if the current running executable has an embedded `SLICEEXE` trailer and maps it in O(1).
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
        let cas_offset = u64::from_le_bytes(mmap[tail_start..tail_start + 8].try_into()?) as usize;
        let cas_len =
            u64::from_le_bytes(mmap[tail_start + 8..tail_start + 16].try_into()?) as usize;
        drop(mmap);
        let archive = Self::open_from_offset(&exe_path, cas_offset, cas_len)?;
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
        let scoped_pkg_dir = test_dir
            .join("node_modules")
            .join("@edge")
            .join("kit");
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
}
