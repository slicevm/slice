//! Shared Artifact Host Writer, Magic-Byte MIME Sniffer, and Metadata Helpers (`#227` / `#228`).

use anyhow::{Context, Result, bail};
use serde::Serialize;
use slice_engine::{DrainReport, SkippedFile};
use std::path::{Component, Path, PathBuf};

/// Serializable metadata for a drained artifact returned in `--json` and MCP `slice_eval_ts`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtifactMetadata {
    pub(crate) name: String,
    pub(crate) size: usize,
    pub(crate) content_type: String,
    pub(crate) sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) host_path: Option<String>,
}

/// Checks magic bytes for safe raster image formats (`PNG`, `JPEG`, `GIF`, `WebP`).
/// SVG and HTML are intentionally excluded so they are never emitted as MCP `image` blocks.
#[inline]
pub(crate) fn detect_raster_image_magic(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 8 && &bytes[..8] == b"\x89PNG\r\n\x1a\n" {
        return Some("image/png");
    }
    if bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
        return Some("image/jpeg");
    }
    if bytes.len() >= 6 && (&bytes[..6] == b"GIF87a" || &bytes[..6] == b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

/// Sniffs MIME type from magic bytes first, then filename extension, then UTF-8 text heuristic.
pub(crate) fn detect_artifact_mime(path: &str, bytes: &[u8]) -> &'static str {
    if let Some(img) = detect_raster_image_magic(bytes) {
        return img;
    }
    if bytes.len() >= 5 && &bytes[..5] == b"%PDF-" {
        return "application/pdf";
    }
    if bytes.len() >= 4 && &bytes[..4] == b"PK\x03\x04" {
        return "application/zip";
    }
    if bytes.len() >= 2 && bytes[0] == 0x1F && bytes[1] == 0x8B {
        return "application/gzip";
    }

    if let Some((_, ext)) = path.rsplit_once('.') {
        match ext.to_ascii_lowercase().as_str() {
            "svg" => return "image/svg+xml",
            "html" | "htm" => return "text/html",
            "json" => return "application/json",
            "csv" => return "text/csv",
            "md" | "markdown" => return "text/markdown",
            "xml" => return "application/xml",
            "yaml" | "yml" => return "application/yaml",
            "toml" => return "application/toml",
            "txt" | "log" => return "text/plain",
            "js" | "mjs" | "cjs" => return "text/javascript",
            "ts" | "tsx" => return "text/typescript",
            "css" => return "text/css",
            _ => {}
        }
    }

    if !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok() {
        "text/plain"
    } else {
        "application/octet-stream"
    }
}

/// Returns `true` when `mime` is a text-oriented MIME type eligible for inline UTF-8 MCP `resource`
/// blocks (including `image/svg+xml`, `text/html`, `application/json`, `text/*`, etc.).
#[inline]
pub(crate) fn is_inline_text_mime(mime: &str) -> bool {
    mime.starts_with("text/")
        || matches!(
            mime,
            "application/json"
                | "application/xml"
                | "application/yaml"
                | "application/toml"
                | "image/svg+xml"
        )
}

/// Validates a relative artifact path and resolves it under `base_dir`, rejecting any path
/// traversal (`..`), absolute paths, backslashes, NUL bytes, colon/drive prefixes, or symlinks
/// along any component inside `base_dir`.
pub(crate) fn write_single_artifact_safely(
    base_dir: &Path,
    rel_path: &str,
    bytes: &[u8],
) -> Result<PathBuf> {
    if rel_path.is_empty()
        || rel_path.starts_with('/')
        || rel_path.contains('\\')
        || rel_path.contains('\0')
        || rel_path.contains(':')
    {
        bail!("Refusing unsafe artifact path: {:?}", rel_path);
    }

    let mut segments: Vec<&str> = Vec::new();
    for seg in rel_path.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            bail!(
                "Refusing non-normal artifact path segment in {:?}",
                rel_path
            );
        }
        let comp = Path::new(seg).components().next();
        if !matches!(comp, Some(Component::Normal(_))) {
            bail!("Refusing non-normal artifact path {:?}", rel_path);
        }
        segments.push(seg);
    }
    if segments.is_empty() {
        bail!("Refusing empty artifact path");
    }

    std::fs::create_dir_all(base_dir).with_context(|| {
        format!(
            "Failed to create artifacts output directory {}",
            base_dir.display()
        )
    })?;

    let mut cur = base_dir.to_path_buf();
    for &dir_seg in &segments[..segments.len() - 1] {
        cur.push(dir_seg);
        match std::fs::symlink_metadata(&cur) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    bail!(
                        "Refusing to traverse symlink directory while writing artifact: {}",
                        cur.display()
                    );
                }
                if !meta.is_dir() {
                    bail!(
                        "Artifact parent path exists and is not a directory: {}",
                        cur.display()
                    );
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&cur).with_context(|| {
                    format!("Failed to create artifact subdirectory {}", cur.display())
                })?;
            }
            Err(e) => {
                return Err(e).with_context(|| {
                    format!("Failed to inspect artifact directory {}", cur.display())
                });
            }
        }
    }

    cur.push(segments[segments.len() - 1]);
    if let Ok(meta) = std::fs::symlink_metadata(&cur) {
        if meta.file_type().is_symlink() {
            bail!(
                "Refusing to overwrite symlink while writing artifact: {}",
                cur.display()
            );
        }
        if meta.is_dir() {
            bail!(
                "Refusing to overwrite directory while writing artifact: {}",
                cur.display()
            );
        }
    }

    std::fs::write(&cur, bytes)
        .with_context(|| format!("Failed to write artifact {}", cur.display()))?;
    Ok(cur)
}

/// Writes all files from `report` into `dir` without following symlinks and returns metadata for each.
pub(crate) fn write_drained_artifacts_to_dir(
    dir: &Path,
    report: &DrainReport,
) -> Result<Vec<ArtifactMetadata>> {
    if !report.files.is_empty() {
        std::fs::create_dir_all(dir).with_context(|| {
            format!(
                "Failed to create artifacts output directory {}",
                dir.display()
            )
        })?;
    }
    let mut out = Vec::with_capacity(report.files.len());
    for file in &report.files {
        let host_path = write_single_artifact_safely(dir, &file.path, &file.bytes)?;
        let sha256 = slice_engine::intrinsics::sha256_hex(&file.bytes);
        let content_type = detect_artifact_mime(&file.path, &file.bytes).to_string();
        out.push(ArtifactMetadata {
            name: file.path.clone(),
            size: file.bytes.len(),
            content_type,
            sha256,
            host_path: Some(host_path.to_string_lossy().into_owned()),
        });
    }
    Ok(out)
}

/// Prints a concise ASCII artifact summary to stderr for `slice run --artifacts-dir`.
pub(crate) fn print_artifacts_stderr_summary(
    dir: &Path,
    written: &[ArtifactMetadata],
    skipped: &[SkippedFile],
) {
    if written.is_empty() && skipped.is_empty() {
        return;
    }
    if !written.is_empty() {
        let total_bytes: usize = written.iter().map(|a| a.size).sum();
        eprintln!(
            "[Slice Artifacts] Wrote {} file(s) ({} B) to {}",
            written.len(),
            total_bytes,
            dir.display()
        );
        for item in written {
            eprintln!(
                "  - {} ({} B, {}, sha256:{})",
                item.name,
                item.size,
                item.content_type,
                &item.sha256[..12.min(item.sha256.len())]
            );
        }
    }
    if !skipped.is_empty() {
        eprintln!("[Slice Artifacts] Skipped {} entry(ies):", skipped.len());
        for s in skipped {
            eprintln!("  - {} ({:?})", s.path, s.reason);
        }
    }
}

/// Converts a host filesystem path to a `file://` URI suitable for MCP `resource_link` blocks.
pub(crate) fn path_to_file_uri(path: &Path) -> String {
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let s = abs.to_string_lossy().replace('\\', "/");
    let stripped = s.strip_prefix("//?/").unwrap_or(&s);
    if stripped.starts_with('/') {
        format!("file://{stripped}")
    } else {
        format!("file:///{stripped}")
    }
}
