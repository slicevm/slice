//! Zero-Stat Content-Addressable Storage (CAS) and Radix Module Virtualization
//!
//! Eliminates `stat`, `lstat`, and `readdir` system calls during module resolution.
//! Flattens TypeScript/JavaScript dependency graphs at build time into a 64-byte
//! cache-line aligned immutable binary archive indexed by BLAKE3-256 hashes and
//! mapped directly into memory via `mmap(MAP_SHARED)`.

use anyhow::{bail, Context, Result};
use memmap2::Mmap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CasBlobEntry {
    pub hash_hex: String,
    pub offset: u64,
    pub source_len: u64,
    pub bytecode_offset: u64,
    pub bytecode_len: u64,
    pub original_specifier: String,
    pub is_typescript: bool,
}

/// Deterministic Radix / Hash resolution table embedded in the CAS header.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CasIndexHeader {
    pub version: u32,
    pub entrypoint_specifier: String,
    pub entrypoint_hash: String,
    /// Maps `(issuer_context, import_specifier)` or canonical specifier -> `hash_hex`
    pub radix_resolution_map: HashMap<String, String>,
    /// Maps `hash_hex` -> `CasBlobEntry`
    pub blobs: HashMap<String, CasBlobEntry>,
}

/// Fast zero-copy TypeScript type stripper & CommonJS synthetic ESM rewriter.
///
/// Strips `: TypeAnnotation`, `interface ... {}`, `type ... = ...;`, and `as Type`
/// constructs while preserving line structure and transforming `require("x")` calls.
pub fn strip_typescript_and_normalize(source: &str, is_ts: bool) -> String {
    if !is_ts && !source.contains("interface ") && !source.contains(": ") {
        return rewrite_cjs_require(source);
    }

    let mut out = String::with_capacity(source.len());
    let mut lines = source.lines().peekable();
    let mut in_interface_block = false;
    let mut brace_depth: i32 = 0;

    while let Some(line) = lines.next() {
        let trimmed = line.trim();

        // Skip standalone `type Foo = ...;` declarations
        if (trimmed.starts_with("type ") || trimmed.starts_with("export type "))
            && trimmed.contains('=')
            && trimmed.ends_with(';')
        {
            continue;
        }

        // Skip `interface Foo { ... }` blocks
        if trimmed.starts_with("interface ") || trimmed.starts_with("export interface ") {
            in_interface_block = true;
            brace_depth = 0;
            for ch in trimmed.chars() {
                if ch == '{' {
                    brace_depth += 1;
                } else if ch == '}' {
                    brace_depth -= 1;
                }
            }
            if brace_depth <= 0 && trimmed.contains('}') {
                in_interface_block = false;
            }
            continue;
        }

        if in_interface_block {
            for ch in trimmed.chars() {
                if ch == '{' {
                    brace_depth += 1;
                } else if ch == '}' {
                    brace_depth -= 1;
                }
            }
            if brace_depth <= 0 {
                in_interface_block = false;
            }
            continue;
        }

        let stripped_line = strip_inline_ts_types(line);
        out.push_str(&stripped_line);
        out.push('\n');
    }

    rewrite_cjs_require(&out)
}

fn strip_inline_ts_types(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut result = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let mut in_string: Option<u8> = None;

    while i < bytes.len() {
        let b = bytes[i];

        if let Some(quote) = in_string {
            result.push(b);
            if b == b'\\' && i + 1 < bytes.len() {
                i += 1;
                result.push(bytes[i]);
            } else if b == quote {
                in_string = None;
            }
            i += 1;
            continue;
        }

        if b == b'"' || b == b'\'' || b == b'`' {
            in_string = Some(b);
            result.push(b);
            i += 1;
            continue;
        }

        // Preserve comments
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            result.extend_from_slice(&bytes[i..]);
            break;
        }

        // Detect `: Type` annotations after identifiers or `)`
        if b == b':' {
            // Check if this is part of an object literal `{ key: value }` vs type annotation
            // Type annotations typically follow `const x: number =`, `function f(a: number): number`, etc.
            let prev_non_space = result.iter().rev().find(|&&c| c != b' ').copied();
            let rest = &line[i + 1..];
            let trimmed_rest = rest.trim_start();
            let looks_like_type = trimmed_rest.starts_with("number")
                || trimmed_rest.starts_with("string")
                || trimmed_rest.starts_with("boolean")
                || trimmed_rest.starts_with("any")
                || trimmed_rest.starts_with("void")
                || trimmed_rest.starts_with("unknown")
                || trimmed_rest.starts_with("never")
                || trimmed_rest.starts_with("Record<")
                || trimmed_rest.starts_with("Array<")
                || trimmed_rest.starts_with("Promise<")
                || trimmed_rest.starts_with("Uint8Array")
                || (prev_non_space == Some(b')')
                    && trimmed_rest
                        .chars()
                        .next()
                        .map(|c| c.is_ascii_alphabetic())
                        .unwrap_or(false));

            if looks_like_type {
                i += 1;
                let mut angle_depth = 0;
                while i < bytes.len() {
                    let c = bytes[i];
                    if c == b'<' {
                        angle_depth += 1;
                    } else if c == b'>' && angle_depth > 0 {
                        angle_depth -= 1;
                        i += 1;
                        continue;
                    }
                    if angle_depth == 0
                        && (c == b'='
                            || c == b','
                            || c == b')'
                            || c == b';'
                            || c == b'{'
                            || c == b'\n')
                    {
                        if c == b'=' {
                            result.push(b' ');
                            i += 1;
                            while i < bytes.len() && bytes[i] == b' ' {
                                i += 1;
                            }
                            result.push(b'=');
                            result.push(b' ');
                        }
                        break;
                    }
                    i += 1;
                }
                continue;
            }
        }

        result.push(b);
        i += 1;
    }

    String::from_utf8_lossy(&result).into_owned()
}

/// Rewrites legacy CommonJS `const x = require("mod")` into synthetic deterministic ESM/CAS lookup.
fn rewrite_cjs_require(source: &str) -> String {
    if !source.contains("require(") {
        return source.to_string();
    }
    source.replace("require(", "__slice_cjs_require(")
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
        let is_ts = spec.ends_with(".ts") || spec.ends_with(".tsx");
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
                    if matches!(ext, "js" | "ts" | "mjs" | "cjs") {
                        let rel = path
                            .strip_prefix(root_dir)
                            .unwrap_or(&path)
                            .to_string_lossy()
                            .replace('\\', "/");
                        let specifier = format!("./{}", rel);
                        let raw = std::fs::read_to_string(&path)?;
                        let is_ts = ext == "ts";
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

    /// Serializes the entire module graph into an immutable, 64-byte aligned `.cas` archive file.
    pub fn write_to_file(&self, output_path: &Path) -> Result<CasIndexHeader> {
        let mut radix_map = HashMap::new();
        let mut blobs = HashMap::new();
        let mut data_payload: Vec<u8> = Vec::new();
        let mut entry_hash = String::new();

        for (specifier, _path, normalized_source, bytecode) in &self.modules {
            let mut hasher = blake3::Hasher::new();
            hasher.update(normalized_source.as_bytes());
            hasher.update(bytecode);
            let hash = hasher.finalize();
            let hash_hex = hash.to_hex().to_string();

            // Register multiple normalized keys in the O(1) resolution table
            radix_map.insert(specifier.clone(), hash_hex.clone());
            let stripped_ext = specifier
                .trim_end_matches(".ts")
                .trim_end_matches(".js")
                .trim_end_matches(".mjs")
                .to_string();
            radix_map.insert(stripped_ext.clone(), hash_hex.clone());
            if let Some(bare) = stripped_ext.strip_prefix("./") {
                radix_map.insert(bare.to_string(), hash_hex.clone());
            }

            if *specifier == self.entrypoint
                || stripped_ext == self.entrypoint.trim_end_matches(".ts").trim_end_matches(".js")
            {
                entry_hash = hash_hex.clone();
            }

            if !blobs.contains_key(&hash_hex) {
                // Align payload offset to 64-byte cache line boundary
                let rem = data_payload.len() % CACHE_LINE_ALIGN;
                if rem != 0 {
                    data_payload.resize(data_payload.len() + (CACHE_LINE_ALIGN - rem), 0);
                }
                let source_offset = data_payload.len() as u64;
                let source_bytes = normalized_source.as_bytes();
                data_payload.extend_from_slice(source_bytes);

                let rem_bc = data_payload.len() % CACHE_LINE_ALIGN;
                if rem_bc != 0 {
                    data_payload.resize(data_payload.len() + (CACHE_LINE_ALIGN - rem_bc), 0);
                }
                let bytecode_offset = data_payload.len() as u64;
                data_payload.extend_from_slice(bytecode);

                blobs.insert(
                    hash_hex.clone(),
                    CasBlobEntry {
                        hash_hex,
                        offset: source_offset,
                        source_len: source_bytes.len() as u64,
                        bytecode_offset,
                        bytecode_len: bytecode.len() as u64,
                        original_specifier: specifier.clone(),
                        is_typescript: specifier.ends_with(".ts"),
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

        // File layout:
        // [0..8]   : CAS_MAGIC ("SLICECAS")
        // [8..12]  : CAS_VERSION (u32 LE)
        // [12..16] : Reserved (u32 LE = 0)
        // [16..24] : Header JSON length (u64 LE)
        // [24..32] : Data section start offset (u64 LE, 64-byte aligned)
        // [32..32+header_len] : Header JSON bytes
        // [padding to 64-byte alignment]
        // [data_start..] : 64-byte aligned CAS source & bytecode blobs
        let pre_data_len = 32 + header_bytes.len();
        let pad_len = (CACHE_LINE_ALIGN - (pre_data_len % CACHE_LINE_ALIGN)) % CACHE_LINE_ALIGN;
        let data_start_offset = (pre_data_len + pad_len) as u64;

        let mut file = File::create(output_path)
            .with_context(|| format!("Failed to create CAS archive at {}", output_path.display()))?;
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

/// Zero-Stat Memory-Mapped Content-Addressable Storage Archive (`mmap(MAP_SHARED)`).
///
/// Once mapped, resolving any specifier or reading any module's source/bytecode
/// executes in O(1) memory reads with **zero** `stat`, `lstat`, `open`, or `read` syscalls.
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
        let hash = self.header.radix_resolution_map.get(specifier).or_else(|| {
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
}
