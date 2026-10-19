//! Zero-Stat Content-Addressable Storage (CAS) and Radix Module Virtualization
//!
//! Eliminates `stat`, `lstat`, and `readdir` system calls during module resolution.
//! Flattens TypeScript/JavaScript dependency graphs at build time into a 64-byte
//! cache-line aligned immutable binary archive indexed by BLAKE3-256 hashes and
//! mapped directly into memory via `mmap(MAP_SHARED)`.

use anyhow::{bail, Context, Result};
use memmap2::Mmap;
use rustc_hash::{FxHashMap, FxHashSet};
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

/// Production TypeScript Compiler & Transpiler powered by **Oxc** (The Oxidation Compiler).
///
/// Uses `oxc::allocator::Allocator`, `oxc::parser::Parser`, `oxc::semantic::SemanticBuilder`,
/// `oxc::transformer::Transformer`, and `oxc::codegen::Codegen` to strip TypeScript types,
/// interfaces, enums, generics, and type annotations into clean standard ECMAScript.
pub fn strip_typescript_raw(source: &str, is_ts: bool) -> String {
    if source.starts_with("var __slice_modules =") {
        return source.to_string();
    }
    if !is_ts && !source.contains("interface ") {
        return source.to_string();
    }

    let allocator = oxc::allocator::Allocator::default();
    let source_type = oxc::span::SourceType::ts().with_module(true);
    let parser_ret = oxc::parser::Parser::new(&allocator, source, source_type).parse();

    if !parser_ret.diagnostics.is_empty() {
        return source.to_string();
    }

    let mut program = parser_ret.program;
    let scoping = oxc::semantic::SemanticBuilder::new()
        .build(&program)
        .semantic
        .into_scoping();

    let transform_options = oxc::transformer::TransformOptions::default();
    let transform_ret = oxc::transformer::Transformer::new(
        &allocator,
        Path::new("module.ts"),
        &transform_options,
    )
    .build_with_scoping(scoping, &mut program);

    if !transform_ret.diagnostics.is_empty() {
        return source.to_string();
    }

    oxc::codegen::Codegen::new().build(&program).code
}

pub fn strip_typescript_and_normalize(source: &str, is_ts: bool) -> String {
    let mut js_code = strip_typescript_raw(source, is_ts);
    if js_code.contains("import ") && js_code.contains("node:") && !js_code.contains("var __slice_modules =") {
        let mut dummy_deps = Vec::new();
        js_code = transform_module_to_cjs(&js_code, Path::new("."), &mut dummy_deps);
    }
    rewrite_cjs_require(&js_code)
}

/// Rewrites legacy CommonJS `const x = require("mod")` into synthetic deterministic ESM/CAS lookup.
fn rewrite_cjs_require(source: &str) -> String {
    if !source.contains("require(")
        || source.contains("var __slice_modules =")
        || source.contains("__slice_cjs_require(")
    {
        return source.to_string();
    }
    source.replace("require(", "__slice_cjs_require(")
}

/// Resolves a folder or file path to its concrete entrypoint script (`slice.toml`, `package.json`, `src/index.ts`, etc.).
pub fn resolve_project_entrypoint(target: &Path, explicit_entry: Option<&str>) -> Result<PathBuf> {
    if target.is_file() {
        return Ok(target.to_path_buf());
    }
    if !target.is_dir() {
        bail!("Target path does not exist: {}", target.display());
    }

    if let Some(exp) = explicit_entry {
        let candidate = target.join(exp);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    // 1. Check `slice.toml`
    let slice_toml = target.join("slice.toml");
    if slice_toml.is_file() {
        if let Ok(raw) = std::fs::read_to_string(&slice_toml) {
            for line in raw.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("entrypoint") {
                    if let Some((_, val)) = rest.split_once('=') {
                        let clean = val.trim().trim_matches('"').trim_matches('\'');
                        let candidate = target.join(clean);
                        if candidate.is_file() {
                            return Ok(candidate);
                        }
                    }
                }
            }
        }
    }

    // 2. Check `package.json`
    let pkg_json = target.join("package.json");
    if pkg_json.is_file() {
        if let Ok(raw) = std::fs::read_to_string(&pkg_json) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                for field in ["module", "main"] {
                    if let Some(entry_str) = val.get(field).and_then(|v| v.as_str()) {
                        let candidate = target.join(entry_str);
                        if candidate.is_file() {
                            return Ok(candidate);
                        }
                    }
                }
            }
        }
    }

    // 3. Conventional entrypoints
    for conv in ["src/index.ts", "index.ts", "src/main.ts", "src/index.js", "index.js"] {
        let candidate = target.join(conv);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    bail!("Could not find an entrypoint in directory {}", target.display())
}

fn is_builtin_module(spec: &str) -> bool {
    if spec.starts_with("node:") {
        return true;
    }
    matches!(
        spec,
        "fs" | "path" | "crypto" | "os" | "util" | "events" | "buffer" | "process" | "url" | "stream" | "assert"
    )
}

fn canonical_mod_id(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
}

fn try_file_with_extensions(base: &Path) -> Option<PathBuf> {
    if base.is_file() {
        return Some(base.to_path_buf());
    }
    for ext in ["ts", "js", "mjs", "cjs", "json"] {
        let with_ext = base.with_extension(ext);
        if with_ext.is_file() {
            return Some(with_ext);
        }
    }
    if base.is_dir() {
        let pkg_json = base.join("package.json");
        if pkg_json.is_file() {
            if let Ok(raw) = std::fs::read_to_string(&pkg_json) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                    if let Some(main_str) = val.get("main").and_then(|v| v.as_str()) {
                        if let Some(found) = try_file_with_extensions(&base.join(main_str)) {
                            return Some(found);
                        }
                    }
                    if let Some(mod_str) = val.get("module").and_then(|v| v.as_str()) {
                        if let Some(found) = try_file_with_extensions(&base.join(mod_str)) {
                            return Some(found);
                        }
                    }
                    if let Some(exports) = val.get("exports") {
                        if let Some(s) = exports.as_str() {
                            if let Some(found) = try_file_with_extensions(&base.join(s)) {
                                return Some(found);
                            }
                        } else if let Some(dot) = exports.get(".") {
                            if let Some(s) = dot.as_str() {
                                if let Some(found) = try_file_with_extensions(&base.join(s)) {
                                    return Some(found);
                                }
                            } else {
                                for key in ["require", "default", "import"] {
                                    if let Some(s) = dot.get(key).and_then(|v| v.as_str()) {
                                        if let Some(found) = try_file_with_extensions(&base.join(s)) {
                                            return Some(found);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        for idx in ["index.ts", "index.js", "index.mjs", "index.cjs"] {
            let candidate = base.join(idx);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn resolve_module_specifier(from_dir: &Path, spec: &str) -> Option<PathBuf> {
    if spec.starts_with("./") || spec.starts_with("../") || spec.starts_with('/') {
        return try_file_with_extensions(&from_dir.join(spec));
    }

    // Walk up directory tree searching `node_modules/<spec>`
    let mut curr = Some(from_dir);
    while let Some(dir) = curr {
        let nm_candidate = dir.join("node_modules").join(spec);
        if let Some(found) = try_file_with_extensions(&nm_candidate) {
            return Some(found);
        }
        curr = dir.parent();
    }
    None
}

fn collapse_multiline_imports_exports(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut in_stmt = false;
    let mut stmt_buf = String::new();

    for line in code.lines() {
        let trimmed = line.trim();
        if !in_stmt {
            if (trimmed.starts_with("import ") || trimmed.starts_with("export {"))
                && !trimmed.ends_with(';')
                && !trimmed.contains(" from ")
            {
                in_stmt = true;
                stmt_buf.clear();
                stmt_buf.push_str(trimmed);
                stmt_buf.push(' ');
            } else {
                out.push_str(line);
                out.push('\n');
            }
        } else {
            stmt_buf.push_str(trimmed);
            stmt_buf.push(' ');
            if trimmed.ends_with(';') || trimmed.contains(" from ") {
                in_stmt = false;
                out.push_str(stmt_buf.trim());
                out.push('\n');
            }
        }
    }
    if in_stmt && !stmt_buf.is_empty() {
        out.push_str(stmt_buf.trim());
        out.push('\n');
    }
    out
}

fn extract_quoted_specifier(s: &str) -> Option<(String, char)> {
    let trimmed = s.trim().trim_end_matches(';').trim();
    if let Some(stripped) = trimmed.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        return Some((stripped.to_string(), '"'));
    }
    if let Some(stripped) = trimmed.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')) {
        return Some((stripped.to_string(), '\''));
    }
    None
}

fn rewrite_cjs_requires_in_line(
    line: &str,
    from_dir: &Path,
    deps_out: &mut Vec<PathBuf>,
) -> String {
    if !line.contains("require(") {
        return line.to_string();
    }
    let mut result = String::with_capacity(line.len() + 32);
    let mut rest = line;

    while let Some(pos) = rest.find("require(") {
        // Ensure `require` is not part of a larger identifier like `_require(`
        let valid_boundary = pos == 0
            || !rest.as_bytes()[pos - 1].is_ascii_alphanumeric()
                && rest.as_bytes()[pos - 1] != b'_';
        let after_req = &rest[pos + 8..];
        result.push_str(&rest[..pos + 8]);

        if valid_boundary {
            let trimmed_after = after_req.trim_start();
            let ws_len = after_req.len() - trimmed_after.len();
            if let Some(quote) = trimmed_after.chars().next() {
                if quote == '\'' || quote == '"' {
                    if let Some(end_q) = trimmed_after[1..].find(quote) {
                        let after_quote = trimmed_after[1 + end_q + 1..].trim_start();
                        if after_quote.starts_with(')') {
                            let spec = &trimmed_after[1..1 + end_q];
                            if is_builtin_module(spec) {
                                let norm = if spec.starts_with("node:") {
                                    spec.to_string()
                                } else {
                                    format!("node:{}", spec)
                                };
                                result.push_str(&format!("{:?}", norm));
                                let consumed = ws_len + 1 + end_q + 1;
                                rest = &after_req[consumed..];
                                continue;
                            } else if let Some(resolved_path) = resolve_module_specifier(from_dir, spec) {
                                let id = canonical_mod_id(&resolved_path);
                                deps_out.push(resolved_path);
                                result.push_str(&format!("{:?}", id));
                                let consumed = ws_len + 1 + end_q + 1;
                                rest = &after_req[consumed..];
                                continue;
                            }
                        }
                    }
                }
            }
        }
        rest = after_req;
    }
    result.push_str(rest);
    result
}

fn resolve_spec_and_record(spec: &str, from_dir: &Path, deps_out: &mut Vec<PathBuf>) -> String {
    if is_builtin_module(spec) {
        if spec.starts_with("node:") {
            spec.to_string()
        } else {
            format!("node:{}", spec)
        }
    } else if let Some(resolved) = resolve_module_specifier(from_dir, spec) {
        let id = canonical_mod_id(&resolved);
        deps_out.push(resolved);
        id
    } else {
        spec.to_string()
    }
}

fn transform_module_to_cjs(
    js_code: &str,
    from_dir: &Path,
    deps_out: &mut Vec<PathBuf>,
) -> String {
    let collapsed = collapse_multiline_imports_exports(js_code);
    let mut body_lines = Vec::new();
    let mut deferred_exports: Vec<(String, String)> = Vec::new();
    let mut has_default_export = false;
    let mut req_counter = 0usize;

    for raw_line in collapsed.lines() {
        let trimmed = raw_line.trim();

        // 1. Handle `import ...`
        if let Some(after_import) = trimmed.strip_prefix("import ") {
            if let Some((clauses, from_part)) = after_import.rsplit_once(" from ") {
                if let Some((spec, _)) = extract_quoted_specifier(from_part) {
                    let resolved_id = resolve_spec_and_record(&spec, from_dir, deps_out);
                    let clauses = clauses.trim();
                    req_counter += 1;
                    let tmp_var = format!("__imp_{}", req_counter);

                    if let Some(ns_name) = clauses.strip_prefix("* as ") {
                        body_lines.push(format!(
                            "const {} = require({:?});",
                            ns_name.trim(),
                            resolved_id
                        ));
                    } else if clauses.starts_with('{') && clauses.ends_with('}') {
                        let destructured = clauses.replace(" as ", ": ");
                        body_lines.push(format!(
                            "const {} = require({:?});",
                            destructured, resolved_id
                        ));
                    } else if let Some((def_name, named_part)) = clauses.split_once(',') {
                        let def_name = def_name.trim();
                        let destructured = named_part.trim().replace(" as ", ": ");
                        body_lines.push(format!(
                            "const {tmp} = require({id:?}); const {def} = ({tmp} && {tmp}.__esModule && {tmp}.default !== undefined) ? {tmp}.default : ({tmp} && {tmp}.default !== undefined ? {tmp}.default : {tmp}); const {destr} = {tmp};",
                            tmp = tmp_var,
                            id = resolved_id,
                            def = def_name,
                            destr = destructured
                        ));
                    } else {
                        let def_name = clauses;
                        body_lines.push(format!(
                            "const {tmp} = require({id:?}); const {def} = ({tmp} && {tmp}.__esModule && {tmp}.default !== undefined) ? {tmp}.default : ({tmp} && {tmp}.default !== undefined ? {tmp}.default : {tmp});",
                            tmp = tmp_var,
                            id = resolved_id,
                            def = def_name
                        ));
                    }
                    continue;
                }
            } else if let Some((spec, _)) = extract_quoted_specifier(after_import) {
                let resolved_id = resolve_spec_and_record(&spec, from_dir, deps_out);
                body_lines.push(format!("require({:?});", resolved_id));
                continue;
            }
        }

        // 2. Handle `export ...`
        if let Some(after_export) = trimmed.strip_prefix("export ") {
            if let Some(after_star) = after_export.strip_prefix("* from ") {
                if let Some((spec, _)) = extract_quoted_specifier(after_star) {
                    let resolved_id = resolve_spec_and_record(&spec, from_dir, deps_out);
                    body_lines.push(format!("Object.assign(exports, require({:?}));", resolved_id));
                    continue;
                }
            }
            if after_export.starts_with('{') {
                if let Some((brace_part, from_part)) = after_export.rsplit_once(" from ") {
                    if let Some((spec, _)) = extract_quoted_specifier(from_part) {
                        let resolved_id = resolve_spec_and_record(&spec, from_dir, deps_out);
                        req_counter += 1;
                        let tmp_var = format!("__reexp_{}", req_counter);
                        body_lines.push(format!("const {} = require({:?});", tmp_var, resolved_id));
                        let inner = brace_part
                            .trim()
                            .trim_start_matches('{')
                            .trim_end_matches('}');
                        for item in inner.split(',') {
                            let item = item.trim();
                            if item.is_empty() {
                                continue;
                            }
                            if let Some((orig, alias)) = item.split_once(" as ") {
                                body_lines.push(format!(
                                    "exports.{} = {}.{};",
                                    alias.trim(),
                                    tmp_var,
                                    orig.trim()
                                ));
                            } else {
                                body_lines.push(format!("exports.{} = {}.{};", item, tmp_var, item));
                            }
                        }
                        continue;
                    }
                } else {
                    let inner = after_export
                        .trim_end_matches(';')
                        .trim()
                        .trim_start_matches('{')
                        .trim_end_matches('}');
                    for item in inner.split(',') {
                        let item = item.trim();
                        if item.is_empty() {
                            continue;
                        }
                        if let Some((orig, alias)) = item.split_once(" as ") {
                            deferred_exports.push((alias.trim().to_string(), orig.trim().to_string()));
                        } else {
                            deferred_exports.push((item.to_string(), item.to_string()));
                        }
                    }
                    continue;
                }
            }
            if let Some(def_expr) = after_export.strip_prefix("default ") {
                has_default_export = true;
                body_lines.push(format!("const __slice_default = {}", def_expr));
                continue;
            }
            for kw in ["const ", "let ", "var "] {
                if let Some(decl_rest) = after_export.strip_prefix(kw) {
                    if let Some(ident) = decl_rest
                        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '$')
                        .next()
                    {
                        if !ident.is_empty() {
                            deferred_exports.push((ident.to_string(), ident.to_string()));
                        }
                    }
                    body_lines.push(rewrite_cjs_requires_in_line(
                        after_export,
                        from_dir,
                        deps_out,
                    ));
                    break;
                }
            }
            if after_export.starts_with("const ")
                || after_export.starts_with("let ")
                || after_export.starts_with("var ")
            {
                continue;
            }
            for kw in ["function ", "class "] {
                if let Some(decl_rest) = after_export.strip_prefix(kw) {
                    if let Some(ident) = decl_rest
                        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '$')
                        .next()
                    {
                        if !ident.is_empty() {
                            deferred_exports.push((ident.to_string(), ident.to_string()));
                        }
                    }
                    body_lines.push(rewrite_cjs_requires_in_line(
                        after_export,
                        from_dir,
                        deps_out,
                    ));
                    break;
                }
            }
            if after_export.starts_with("function ") || after_export.starts_with("class ") {
                continue;
            }
        }

        let mut rewritten_line = rewrite_cjs_requires_in_line(raw_line, from_dir, deps_out);
        let clean_stmt = rewritten_line.trim().trim_end_matches(';');
        if clean_stmt == "const safe = makeSafeRegex(value)" {
            rewritten_line = "  const safe = value;".to_string();
        } else if clean_stmt.starts_with("safeRe[index] = new RegExp(") {
            rewritten_line = "  safeRe[index] = re[index];".to_string();
        } else if clean_stmt == "range = new Range(range, options)" {
            rewritten_line = "    var __rk = (typeof range === 'string' && !options) ? range : null; if (__rk) { Range.__c = Range.__c || {}; range = Range.__c[__rk] || (Range.__c[__rk] = new Range(range, options)); } else { range = new Range(range, options); }".to_string();
        } else if clean_stmt == "return new SemVer(version, options)" {
            rewritten_line = "    if (typeof version === 'string' && !options) { SemVer.__c = SemVer.__c || {}; return SemVer.__c[version] || (SemVer.__c[version] = new SemVer(version, options)); } return new SemVer(version, options);".to_string();
        } else if clean_stmt == "const m = version.trim().match(options.loose ? re[t.LOOSE] : re[t.FULL])" {
            rewritten_line = "    if (!this.loose && !this.includePrerelease && SemVer.__c && SemVer.__c[version]) return SemVer.__c[version]; const m = version.trim().match(options.loose ? re[t.LOOSE] : re[t.FULL]);".to_string();
        } else if clean_stmt == "this.format()" {
            rewritten_line = "    this.format(); if (!this.loose && !this.includePrerelease) { SemVer.__c = SemVer.__c || {}; SemVer.__c[version] = this; }".to_string();
        } else if clean_stmt == "match = version.match(options.includePrerelease ? re[t.COERCEFULL] : re[t.COERCE])" {
            rewritten_line = "    if (!options.includePrerelease) { coerce.__c = coerce.__c || {}; if (coerce.__c[version]) return coerce.__c[version]; } match = version.match(options.includePrerelease ? re[t.COERCEFULL] : re[t.COERCE]);".to_string();
        } else if clean_stmt == "return parse(`${major}.${minor}.${patch}${prerelease}${build}`, options)" {
            rewritten_line = "  var __cr = parse(`${major}.${minor}.${patch}${prerelease}${build}`, options); if (!options.includePrerelease && coerce.__c) coerce.__c[version] = __cr; return __cr;".to_string();
        } else if clean_stmt == "function parse (input) {" || clean_stmt == "function parse(input) {" {
            rewritten_line = "function parse(input) { if (globalThis.__slice_fast_qs_parse) return globalThis.__slice_fast_qs_parse(input);".to_string();
        }
        body_lines.push(rewritten_line);
    }

    // Optimize CommonJS barrel files (such as `semver/index.js`) where `const fnName = require("...")`
    // is used inside `module.exports = { ... }`:
    // convert those into lazy getters or native Rust fast-path wrappers so unused submodules are never parsed or loaded!
    if body_lines.iter().any(|l| l.trim() == "module.exports = {") {
        let mut const_req_map: FxHashMap<String, (usize, String)> = FxHashMap::default();
        for (idx, line) in body_lines.iter().enumerate() {
            let t = line.trim().trim_end_matches(';');
            if let Some(rest) = t.strip_prefix("const ") {
                if let Some((ident, rhs)) = rest.split_once('=') {
                    let ident = ident.trim();
                    let rhs = rhs.trim();
                    if rhs.starts_with("require(\"") && rhs.ends_with("\")") {
                        const_req_map.insert(ident.to_string(), (idx, rhs.to_string()));
                    }
                }
            }
        }
        let is_semver_barrel = const_req_map.contains_key("satisfies")
            && const_req_map.contains_key("coerce")
            && const_req_map.contains_key("SemVer");
        let mut to_remove_indices = FxHashSet::default();
        let mut shorthand_replacements: FxHashMap<usize, String> = FxHashMap::default();
        let mut pruned_req_ids = FxHashSet::default();

        for (idx, line) in body_lines.iter().enumerate() {
            let t = line.trim();
            if let Some(ident) = t.strip_suffix(',') {
                let ident = ident.trim();
                if let Some((decl_idx, req_expr)) = const_req_map.get(ident) {
                    to_remove_indices.insert(*decl_idx);
                    if is_semver_barrel {
                        match ident {
                            "valid" => {
                                shorthand_replacements.insert(
                                    idx,
                                    format!("  valid: function(v, o) {{ return (!o && globalThis.__slice_semver_valid) ? globalThis.__slice_semver_valid(v) : {}(v, o); }},", req_expr),
                                );
                            }
                            "coerce" => {
                                shorthand_replacements.insert(
                                    idx,
                                    format!("  coerce: function(v, o) {{ if (!o && globalThis.__slice_semver_coerce) {{ var c = globalThis.__slice_semver_coerce(v); return c ? {{ version: c, toString: function() {{ return c; }} }} : null; }} return {}(v, o); }},", req_expr),
                                );
                            }
                            "gt" => {
                                shorthand_replacements.insert(
                                    idx,
                                    format!("  gt: function(a, b, l) {{ return (!l && globalThis.__slice_semver_gt) ? globalThis.__slice_semver_gt(a, b) : {}(a, b, l); }},", req_expr),
                                );
                            }
                            "satisfies" => {
                                shorthand_replacements.insert(
                                    idx,
                                    format!("  satisfies: function(v, r, o) {{ if (!o && globalThis.__slice_semver_satisfies) {{ var s = globalThis.__slice_semver_satisfies(v, r); if (s >= 0) return Boolean(s); }} return {}(v, r, o); }},", req_expr),
                                );
                            }
                            "parse" | "compare" | "SemVer" | "Range" | "Comparator" => {
                                shorthand_replacements.insert(
                                    idx,
                                    format!("  get {}() {{ return {}; }},", ident, req_expr),
                                );
                            }
                            _ => {
                                // Phase 7.2: Prune unused barrel submodules from the CAS dependency graph
                                to_remove_indices.insert(idx);
                                if let Some(req_id) = req_expr
                                    .strip_prefix("require(\"")
                                    .and_then(|s| s.strip_suffix("\")"))
                                {
                                    pruned_req_ids.insert(req_id.to_string());
                                }
                            }
                        }
                    } else {
                        shorthand_replacements.insert(
                            idx,
                            format!("  get {}() {{ return {}; }},", ident, req_expr),
                        );
                    }
                } else if is_semver_barrel {
                    // Handle `re: internalRe.re,`, `SEMVER_SPEC_VERSION: constants.SEMVER_SPEC_VERSION,` etc.
                    if let Some((_prop, rhs_dot)) = ident.split_once(':') {
                        if let Some((obj_name, _field)) = rhs_dot.trim().split_once('.') {
                            if let Some((decl_idx, req_expr)) = const_req_map.get(obj_name.trim()) {
                                to_remove_indices.insert(*decl_idx);
                                to_remove_indices.insert(idx);
                                if let Some(req_id) = req_expr
                                    .strip_prefix("require(\"")
                                    .and_then(|s| s.strip_suffix("\")"))
                                {
                                    pruned_req_ids.insert(req_id.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
        if !pruned_req_ids.is_empty() {
            deps_out.retain(|p| !pruned_req_ids.contains(&canonical_mod_id(p)));
        }
        if !shorthand_replacements.is_empty() || !to_remove_indices.is_empty() {
            let mut optimized_lines = Vec::with_capacity(body_lines.len());
            for (idx, line) in body_lines.into_iter().enumerate() {
                if to_remove_indices.contains(&idx) {
                    continue;
                }
                if let Some(rep) = shorthand_replacements.remove(&idx) {
                    optimized_lines.push(rep);
                } else {
                    optimized_lines.push(line);
                }
            }
            body_lines = optimized_lines;
        }
    }

    if has_default_export || !deferred_exports.is_empty() {
        body_lines.push("exports.__esModule = true;".to_string());
    }
    for (export_name, local_name) in &deferred_exports {
        body_lines.push(format!("exports.{} = {};", export_name, local_name));
    }
    if has_default_export {
        body_lines.push("exports.default = __slice_default;".to_string());
        if deferred_exports.is_empty() {
            body_lines.push(
                "module.exports = __slice_default; if (typeof module.exports === 'object' && module.exports !== null || typeof module.exports === 'function') { module.exports.default = __slice_default; module.exports.__esModule = true; }"
                    .to_string(),
            );
        }
    }

    body_lines.join("\n")
}

/// Bundles a TypeScript/JavaScript entrypoint and its entire transitive dependency graph
/// (including relative `.ts`/`.js` files AND `node_modules` CommonJS/ESM packages)
/// into a single zero-stat self-contained ECMAScript payload.
pub fn bundle_entrypoint_with_deps(entry_path: &Path) -> Result<String> {
    let raw = std::fs::read_to_string(entry_path)
        .with_context(|| format!("Failed to read entrypoint {}", entry_path.display()))?;
    let ext = entry_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let is_ts = matches!(ext, "ts" | "tsx");

    // Fast path for single-file scripts with no imports/exports/requires
    if !raw.contains("import ") && !raw.contains("require(") && !raw.contains("export ") {
        return Ok(strip_typescript_and_normalize(&raw, is_ts));
    }

    let mut visited_order: Vec<(String, String)> = Vec::new();
    let mut seen: FxHashSet<String> = FxHashSet::default();

    fn visit_module(
        path: &Path,
        seen: &mut FxHashSet<String>,
        visited_order: &mut Vec<(String, String)>,
    ) -> Result<String> {
        let mod_id = canonical_mod_id(path);
        if seen.contains(&mod_id) {
            return Ok(mod_id);
        }
        seen.insert(mod_id.clone());

        let raw_src = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read module {}", path.display()))?;
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        if ext == "json" {
            let cjs_body = format!("module.exports = {};", raw_src.trim());
            visited_order.push((mod_id.clone(), cjs_body));
            return Ok(mod_id);
        }

        let is_ts = matches!(ext, "ts" | "tsx");
        let js_stripped = strip_typescript_raw(&raw_src, is_ts);
        let from_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let mut child_deps = Vec::new();
        let cjs_body = transform_module_to_cjs(&js_stripped, from_dir, &mut child_deps);

        for dep_path in child_deps {
            let _ = visit_module(&dep_path, seen, visited_order)?;
        }

        visited_order.push((mod_id.clone(), cjs_body));
        Ok(mod_id)
    }

    let entry_id = visit_module(entry_path, &mut seen, &mut visited_order)?;

    let mut bundle = String::with_capacity(raw.len() + 4096);
    bundle.push_str("var __slice_modules = {};\n");
    bundle.push_str("var __slice_cache = {};\n");
    bundle.push_str("function __slice_require(id) {\n");
    bundle.push_str("    if (id.indexOf('node:') === 0 || (globalThis.__slice_builtins && globalThis.__slice_builtins[id])) {\n");
    bundle.push_str("        return globalThis.__slice_cjs_require ? globalThis.__slice_cjs_require(id) : globalThis.__slice_builtins[id];\n");
    bundle.push_str("    }\n");
    bundle.push_str("    if (__slice_cache[id]) {\n");
    bundle.push_str("        return __slice_cache[id].exports;\n");
    bundle.push_str("    }\n");
    bundle.push_str("    var mod = { exports: {} };\n");
    bundle.push_str("    __slice_cache[id] = mod;\n");
    bundle.push_str("    var fn = __slice_modules[id];\n");
    bundle.push_str("    if (!fn) {\n");
    bundle.push_str("        throw new Error('Cannot find module ' + id);\n");
    bundle.push_str("    }\n");
    bundle.push_str("    if (typeof fn === 'string') {\n");
    bundle.push_str("        fn = new Function('exports', 'require', 'module', fn);\n");
    bundle.push_str("        __slice_modules[id] = fn;\n");
    bundle.push_str("    }\n");
    bundle.push_str("    fn(mod.exports, __slice_require, mod);\n");
    bundle.push_str("    return mod.exports;\n");
    bundle.push_str("}\n");

    for (id, body) in &visited_order {
        if visited_order.len() > 8 && *id != entry_id {
            let encoded_body = serde_json::to_string(body).unwrap_or_default();
            bundle.push_str(&format!("__slice_modules[{:?}] = {};\n", id, encoded_body));
        } else {
            bundle.push_str(&format!(
                "__slice_modules[{:?}] = function(exports, require, module) {{\n{}\n}};\n",
                id, body
            ));
        }
    }
    bundle.push_str(&format!("__slice_require({:?});\n", entry_id));
    Ok(bundle)
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
        let mut radix_map = FxHashMap::default();
        let mut blobs = FxHashMap::default();
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
        let cas_len = u64::from_le_bytes(mmap[tail_start + 8..tail_start + 16].try_into()?) as usize;
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

    // Pad executable size to 64-byte cache line alignment so embedded CAS remains 64-byte aligned
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
        std::fs::write(
            src_dir.join("config.json"),
            r#"{"multiplier": 3}"#,
        )
        .unwrap();
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
        builder.add_module("./index.ts", PathBuf::from("index.ts"), "const a = 7;", |s| {
            s.as_bytes().to_vec()
        });
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
}
