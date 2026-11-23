//! Bundled Source-Map Line Mapper, Error Location Translator, Multi-Frame Stack Trace Mapper,
//! and `SLICEBOA` Source Compactor Subsystem.
//!
//! Organizes source-map and bundle post-processing into focused submodules:
//! - [`line_mapper`]: [`compute_source_line_map`] linear-time anchor mapper.
//! - [`error_location`]: [`map_bundled_error_location`] and [`map_bundled_error_location_with_offset`].
//! - [`scope_recovery`]: Lexical class/function/method symbol name recovery for anonymous frames.
//! - [`stack_trace`]: [`map_bundled_stack_trace`] and [`map_bundled_stack_trace_with_offset`].

pub mod error_location;
pub mod line_mapper;
pub mod scope_recovery;
pub mod stack_trace;

pub use error_location::{map_bundled_error_location, map_bundled_error_location_with_offset};
pub use line_mapper::compute_source_line_map;
pub use stack_trace::{map_bundled_stack_trace, map_bundled_stack_trace_with_offset};

use std::path::Path;

pub fn short_source_label(path: &Path) -> String {
    let fname = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("module.ts");
    if let Some(parent_name) = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        && matches!(parent_name, "src" | "tests" | "test" | "lib" | "app")
    {
        return format!("{}/{}", parent_name, fname);
    }
    fname.to_string()
}

fn update_backtick_state_on_line(line: &str, mut in_backtick: bool) -> bool {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    while i < len {
        let b = bytes[i];
        if b == b'\\' {
            i += 2;
            continue;
        }
        if in_backtick {
            if b == b'`' {
                in_backtick = false;
            }
            i += 1;
            continue;
        }
        if b == b'\'' || b == b'"' {
            let q = b;
            let mut j = i + 1;
            let mut found_close = None;
            while j < len {
                if bytes[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if bytes[j] == q {
                    found_close = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(close_idx) = found_close {
                i = close_idx + 1;
                continue;
            }
        } else if b == b'`' {
            in_backtick = true;
        }
        i += 1;
    }
    in_backtick
}

/// Compacts a JavaScript payload before embedding it into a `SLICEBOA` bytecode buffer:
/// - Preserves the `var __slice_modules = {};//__SLICE_RC:<16-hex>` header on line 1 at exact byte offset 21.
/// - Preserves multi-line backtick template literals verbatim (`in_backtick`).
/// - Strips leading/trailing indentation whitespace, blank lines, and `// #slice-source:` / `//` comment lines
///   from the `SLICEBOA` bytecode slot while keeping the `.cas` archive's `source` slot human-readable.
pub fn compact_js_for_boa_bytecode(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut in_backtick = false;
    for (idx, line) in source.lines().enumerate() {
        if idx == 0 && line.starts_with("var __slice_modules =") {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_backtick {
            out.push_str(line);
            out.push('\n');
            if line.as_bytes().contains(&b'`') {
                in_backtick = update_backtick_state_on_line(line, in_backtick);
            }
            continue;
        }
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("// #slice-source: ") {
            if rest.bytes().filter(|&b| b == b'|').count() >= 2 {
                out.push_str("//@sm:");
                out.push_str(rest);
                out.push('\n');
            }
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        out.push_str(trimmed);
        out.push('\n');
        if trimmed.as_bytes().contains(&b'`') {
            in_backtick = update_backtick_state_on_line(trimmed, in_backtick);
        }
    }
    out
}

/// Extracts `(rc_key_u64, hex_bytes_16)` from a block-wrapped `__SLICE_RC` bundle payload
/// (`"{\nvar __slice_modules = {};//__SLICE_RC:<16-hex>..."`).
#[inline]
pub fn extract_slice_rc_key(js_code: &str) -> Option<(u64, &[u8; 16])> {
    const PREFIX: &[u8] = b"{\nvar __slice_modules = {};//__SLICE_RC:";
    let bytes = js_code.as_bytes();
    if bytes.len() >= PREFIX.len() + 16 && bytes.starts_with(PREFIX) {
        let hex_slice = &js_code[PREFIX.len()..PREFIX.len() + 16];
        let hex_arr: &[u8; 16] = bytes[PREFIX.len()..PREFIX.len() + 16].try_into().ok()?;
        let key_u64 = u64::from_str_radix(hex_slice, 16).ok()?;
        Some((key_u64, hex_arr))
    } else {
        None
    }
}

#[derive(Debug, Clone)]
pub(super) struct BundledSourceSection<'a> {
    pub(super) short_label: &'a str,
    pub(super) full_path: &'a str,
    pub(super) line_map: &'a str,
    pub(super) marker_line_1: usize,
    pub(super) end_line_1: usize,
}

pub(super) fn parse_slice_source_marker(rest: &str) -> (&str, &str, &str) {
    let mut parts = rest.splitn(3, '|');
    let short = parts.next().unwrap_or(rest).trim();
    let full = parts.next().unwrap_or(short).trim();
    let line_map = parts.next().unwrap_or("").trim();
    (short, full, line_map)
}

pub(super) fn lookup_embedded_line_map(line_map: &str, rel_idx_0: usize) -> Option<usize> {
    if line_map.is_empty() {
        return None;
    }
    line_map
        .split(',')
        .nth(rel_idx_0)
        .and_then(|s| s.trim().parse::<usize>().ok())
}

pub(super) fn count_code_lines_between(
    lines: &[&str],
    start_line_1: usize,
    target_line_1: usize,
) -> usize {
    if target_line_1 <= start_line_1 {
        return 0;
    }
    let start_idx = start_line_1;
    let end_idx = (target_line_1 - 1).min(lines.len());
    if start_idx >= end_idx {
        return 0;
    }
    lines[start_idx..end_idx]
        .iter()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with("//")
        })
        .count()
}

pub(super) fn read_bounded_source_file(path_str: &str) -> Option<String> {
    const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
    if path_str.is_empty()
        || path_str.contains('\0')
        || path_str.starts_with("\\\\.\\")
        || path_str.starts_with("\\\\?\\")
        || path_str.starts_with("//./")
        || path_str.starts_with("//?/")
    {
        return None;
    }
    let path = Path::new(path_str);
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return None;
    }
    let ext = path.extension()?.to_str()?;
    if !matches!(
        ext,
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "mts" | "cts"
    ) {
        return None;
    }
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.file_type().is_file() || meta.file_type().is_symlink() || meta.len() > MAX_SOURCE_BYTES
    {
        return None;
    }
    let canon = std::fs::canonicalize(path).ok()?;
    let allowed = std::env::current_dir()
        .ok()
        .and_then(|d| std::fs::canonicalize(d).ok())
        .is_some_and(|cwd| canon.starts_with(&cwd))
        || std::fs::canonicalize(std::env::temp_dir())
            .ok()
            .is_some_and(|tmp| canon.starts_with(&tmp));
    if !allowed {
        return None;
    }
    std::fs::read_to_string(&canon).ok()
}

pub(super) fn resolve_section_line(
    lines: &[&str],
    sec: &BundledSourceSection<'_>,
    target_line_1: usize,
) -> usize {
    let rel_idx_0 = count_code_lines_between(lines, sec.marker_line_1, target_line_1);
    if let Some(mapped) = lookup_embedded_line_map(sec.line_map, rel_idx_0) {
        return mapped;
    }
    let mut resolved_line = rel_idx_0 + 1;
    if let Some(stmt_text) = lines.get(target_line_1.saturating_sub(1)).map(|s| s.trim()) {
        let probe = stmt_text
            .strip_prefix("exports.")
            .and_then(|r| r.split_once(" = ").map(|(_, rhs)| rhs.trim()))
            .unwrap_or(stmt_text)
            .trim_end_matches(';')
            .trim();
        if !probe.is_empty()
            && probe.len() >= 4
            && let Some(orig_src) = read_bounded_source_file(sec.full_path)
            && let Some((orig_idx, _)) = orig_src
                .lines()
                .enumerate()
                .find(|(_, ol)| ol.contains(probe))
        {
            resolved_line = orig_idx + 1;
        }
    }
    resolved_line
}
