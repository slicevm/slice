//! Zero-Stat Static Web Asset Table & Cryptographic BLAKE3 `ETag` Indexer (`--static`).

use crate::archive::MappedCasArchive;
use anyhow::{bail, Result};
use rustc_hash::FxHashMap;
use std::path::Path;
use std::sync::Arc;

/// Detects the canonical HTTP `Content-Type` header for a static web asset based on its file extension.
pub fn detect_static_mime_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" | "cjs" => "application/javascript; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "wasm" => "application/wasm",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "txt" | "md" => "text/plain; charset=utf-8",
        "xml" => "application/xml",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        _ => "application/octet-stream",
    }
}

/// Zero-copy or heap-backed payload for a static web asset.
#[derive(Clone, Debug)]
pub enum StaticAssetPayload {
    Owned(Arc<Vec<u8>>),
    Mapped {
        mmap: Arc<memmap2::Mmap>,
        start: usize,
        end: usize,
    },
}

impl std::ops::Deref for StaticAssetPayload {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &[u8] {
        match self {
            Self::Owned(v) => v.as_slice(),
            Self::Mapped { mmap, start, end } => &mmap[*start..*end],
        }
    }
}

/// Pre-indexed zero-stat static web asset with cryptographic BLAKE3 `ETag`.
#[derive(Clone, Debug)]
pub struct StaticAssetEntry {
    pub url_path: String,
    pub content_type: String,
    pub etag_quoted: String,
    pub payload: StaticAssetPayload,
}

/// Zero-Stat in-memory / CAS-backed static asset lookup table (`O(1)` by URL path).
#[derive(Clone, Debug, Default)]
pub struct StaticAssetTable {
    pub by_path: FxHashMap<String, StaticAssetEntry>,
}

impl StaticAssetTable {
    pub fn new() -> Self {
        Self {
            by_path: FxHashMap::default(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }

    pub fn insert_asset(
        &mut self,
        rel_url_path: &str,
        content_type: String,
        etag_hex16: &str,
        payload: Arc<Vec<u8>>,
    ) {
        self.insert_asset_payload(
            rel_url_path,
            content_type,
            etag_hex16,
            StaticAssetPayload::Owned(payload),
        );
    }

    pub fn insert_asset_payload(
        &mut self,
        rel_url_path: &str,
        content_type: String,
        etag_hex16: &str,
        payload: StaticAssetPayload,
    ) {
        let clean_path = if rel_url_path.starts_with('/') {
            rel_url_path.to_string()
        } else {
            format!("/{}", rel_url_path)
        };
        let etag_quoted = format!("\"{}\"", etag_hex16);
        let entry = StaticAssetEntry {
            url_path: clean_path.clone(),
            content_type,
            etag_quoted,
            payload,
        };
        if clean_path == "/index.html" {
            self.by_path.insert("/".to_string(), entry.clone());
        } else if let Some(dir_prefix) = clean_path.strip_suffix("/index.html") {
            if !dir_prefix.is_empty() {
                self.by_path.insert(dir_prefix.to_string(), entry.clone());
                self.by_path
                    .insert(format!("{}/", dir_prefix), entry.clone());
            }
        }
        self.by_path.insert(clean_path, entry);
    }

    /// Recursively loads a static asset directory (`--static=./public`) and pre-computes
    /// 256-bit BLAKE3 `ETag` digests for instant `304 Not Modified` negotiation.
    ///
    /// Security (`CAS-SEC-04`): Skips symbolic links via `entry.file_type()` (`lstat`) so a
    /// symlink inside `static_dir` cannot expose arbitrary host files over HTTP.
    pub fn extend_from_directory(&mut self, static_dir: &Path) -> Result<usize> {
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
                    let raw_bytes = std::fs::read(&path)?;
                    let hash_hex = blake3::hash(&raw_bytes).to_hex().to_string();
                    let mime = detect_static_mime_type(&path).to_string();
                    let url_path = format!("/{}", rel);
                    self.insert_asset(&url_path, mime, &hash_hex[..16], Arc::new(raw_bytes));
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    /// Extracts all embedded `static:/...` blobs from a memory-mapped `.cas` archive
    /// using zero-copy `mmap` slices (`StaticAssetPayload::Mapped`) with zero heap duplication.
    pub fn from_cas_archive(archive: &MappedCasArchive) -> Self {
        let mut table = Self::new();
        for (spec, hash_hex) in &archive.header.radix_resolution_map {
            if let Some(url_path) = spec.strip_prefix("static:") {
                if let Some(blob) = archive.header.blobs.get(hash_hex) {
                    let mime = archive.get_source_slice(blob).to_string();
                    let etag_prefix = if blob.hash_hex.len() >= 16 {
                        &blob.hash_hex[..16]
                    } else {
                        blob.hash_hex.as_str()
                    };
                    if let Some((mmap, start, end)) = archive.get_shared_bytecode_view(blob) {
                        table.insert_asset_payload(
                            url_path,
                            mime,
                            etag_prefix,
                            StaticAssetPayload::Mapped { mmap, start, end },
                        );
                    }
                }
            }
        }
        table
    }

    #[inline]
    pub fn resolve(&self, url_path: &str) -> Option<&StaticAssetEntry> {
        self.by_path.get(url_path)
    }
}
