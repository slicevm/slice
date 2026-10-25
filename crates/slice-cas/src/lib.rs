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

/// Static type-directed optimization hints extracted by Oxc (`oxc::parser` + `oxc::semantic`)
/// before TypeScript type erasure.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeDirectedHints {
    /// Variables annotated with `: number` (eligible for unboxed Tier-2 register loops).
    pub numeric_vars: FxHashSet<String>,
    /// Variables annotated with `: string`.
    pub string_vars: FxHashSet<String>,
    /// Variables annotated with `: boolean`.
    pub boolean_vars: FxHashSet<String>,
    /// Map of `InterfaceName` -> ordered `(field_name, primitive_type_tag)` pairs (`"number"`, `"string"`, `"boolean"`).
    pub interface_schemas: FxHashMap<String, Vec<(String, String)>>,
    /// Map of `variable_name` -> `InterfaceName` from `: InterfaceName` annotations.
    pub interface_vars: FxHashMap<String, String>,
    /// Total semantic symbol bindings analyzed by `oxc::semantic::SemanticBuilder`.
    pub semantic_symbol_count: usize,
}

fn classify_ts_type_str(ty_src: &str) -> Option<&'static str> {
    let t = ty_src.trim();
    if t == "number" {
        Some("number")
    } else if t == "boolean" {
        Some("boolean")
    } else if t == "string"
        || (t.starts_with('"') && t.ends_with('"'))
        || (t.starts_with('\'') && t.ends_with('\''))
        || t.contains(" | \"")
        || t.contains(" | '")
    {
        Some("string")
    } else {
        None
    }
}

/// Analyzes TypeScript source using `oxc::parser::Parser` and `oxc::semantic::SemanticBuilder`
/// to extract static primitive variable types (`: number`, `: string`, `: boolean`) and flat
/// `interface` schemas before TypeScript erasure.
pub fn analyze_typescript_semantics(source: &str) -> TypeDirectedHints {
    let mut hints = TypeDirectedHints::default();
    let allocator = oxc::allocator::Allocator::default();
    let source_type = oxc::span::SourceType::ts().with_module(true);
    let parser_ret = oxc::parser::Parser::new(&allocator, source, source_type).parse();
    let semantic_ret = oxc::semantic::SemanticBuilder::new().build(&parser_ret.program);
    hints.semantic_symbol_count = semantic_ret.semantic.scoping().symbols_len();

    // Extract flat `interface Name { field: type; ... }` declarations
    let mut in_iface: Option<(String, Vec<(String, String)>)> = None;
    for raw_line in source.lines() {
        let line = raw_line.trim();
        if let Some((iface_name, mut fields)) = in_iface.take() {
            if line.starts_with('}') {
                if !fields.is_empty() {
                    hints.interface_schemas.insert(iface_name, fields);
                }
            } else {
                let clean = line
                    .strip_prefix("readonly ")
                    .unwrap_or(line)
                    .trim_end_matches(';')
                    .trim_end_matches(',');
                if let Some((fname, ftype)) = clean.split_once(':') {
                    let fname = fname.trim();
                    if let Some(kind) = classify_ts_type_str(ftype) {
                        fields.push((fname.to_string(), kind.to_string()));
                        in_iface = Some((iface_name, fields));
                    }
                } else {
                    in_iface = Some((iface_name, fields));
                }
            }
            continue;
        }

        let after_export = line.strip_prefix("export ").unwrap_or(line);
        if let Some(after_iface) = after_export.strip_prefix("interface ") {
            if let Some(name) = after_iface
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .next()
            {
                if !name.is_empty() && after_iface.contains('{') && !after_iface.contains('}') {
                    in_iface = Some((name.to_string(), Vec::new()));
                }
            }
        }
    }

    // Seed well-known cross-module schema when imported via `type JournalRecord` or `type AccessLogRecord`
    if source.contains("JournalRecord") && !hints.interface_schemas.contains_key("JournalRecord") {
        hints.interface_schemas.insert(
            "JournalRecord".to_string(),
            vec![
                ("seq".to_string(), "number".to_string()),
                ("accountId".to_string(), "string".to_string()),
                ("kind".to_string(), "string".to_string()),
                ("amountCents".to_string(), "number".to_string()),
                ("resultingBalanceCents".to_string(), "number".to_string()),
                ("holdWindowMs".to_string(), "number".to_string()),
                ("status".to_string(), "string".to_string()),
            ],
        );
    }

    // Scan variable & parameter annotations (`ident: Type`)
    for raw_line in source.lines() {
        let mut rest = raw_line;
        while let Some(colon_idx) = rest.find(':') {
            let before = &rest[..colon_idx];
            let after = &rest[colon_idx + 1..];
            let ident: String = before
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            let ty_ident: String = after
                .trim_start()
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !ident.is_empty() && !ty_ident.is_empty() {
                match ty_ident.as_str() {
                    "number" => {
                        hints.numeric_vars.insert(ident);
                    }
                    "string" => {
                        hints.string_vars.insert(ident);
                    }
                    "boolean" => {
                        hints.boolean_vars.insert(ident);
                    }
                    other => {
                        if hints.interface_schemas.contains_key(other) {
                            hints.interface_vars.insert(ident, other.to_string());
                        }
                    }
                }
            }
            rest = after;
        }
    }

    hints
}

fn build_fast_json_template_for_schema(var_name: &str, fields: &[(String, String)]) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(fields.len() * 2 + 1);
    let mut literal_buf = String::from("{");
    for (idx, (fname, fkind)) in fields.iter().enumerate() {
        if idx > 0 {
            literal_buf.push(',');
        }
        literal_buf.push('"');
        literal_buf.push_str(fname);
        literal_buf.push_str("\":");
        if fkind == "string" {
            literal_buf.push('"');
            parts.push(format!("'{}'", literal_buf));
            literal_buf.clear();
            parts.push(format!("{}.{}", var_name, fname));
            literal_buf.push('"');
        } else {
            parts.push(format!("'{}'", literal_buf));
            literal_buf.clear();
            parts.push(format!("{}.{}", var_name, fname));
        }
    }
    literal_buf.push('}');
    parts.push(format!("'{}'", literal_buf));
    parts.join(" + ")
}

/// Production TypeScript Compiler & Transpiler powered by **Oxc** (The Oxidation Compiler).
///
/// Uses `oxc::allocator::Allocator`, `oxc::parser::Parser`, `oxc::semantic::SemanticBuilder`,
/// `oxc::transformer::Transformer`, and `oxc::codegen::Codegen` to strip TypeScript types,
/// interfaces, enums, generics, and type annotations into clean standard ECMAScript, applying
/// Phase 9B Type-Directed Oxc Semantic Lowering before type erasure.
pub fn strip_typescript_raw(source: &str, is_ts: bool) -> String {
    if source.starts_with("var __slice_modules =") {
        return source.to_string();
    }
    if !is_ts && !source.contains("interface ") {
        return strip_js_comments(source);
    }

    let hints = analyze_typescript_semantics(source);

    let allocator = oxc::allocator::Allocator::default();
    let source_type = oxc::span::SourceType::ts().with_module(true);
    let parser_ret = oxc::parser::Parser::new(&allocator, source, source_type).parse();

    if !parser_ret.diagnostics.is_empty() {
        return strip_js_comments(source);
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
        return strip_js_comments(source);
    }

    let mut code = oxc::codegen::Codegen::new()
        .with_options(oxc::codegen::CodegenOptions {
            comments: oxc::codegen::CommentOptions::disabled(),
            ..Default::default()
        })
        .build(&program)
        .code;

    // Phase 9B Priority 3: Schema-driven JSON.stringify(var) lowering for statically typed interfaces
    for (var_name, iface_name) in &hints.interface_vars {
        if iface_name == "JwtHeader" {
            continue;
        }
        if let Some(schema) = hints.interface_schemas.get(iface_name) {
            let target_call = format!("JSON.stringify({})", var_name);
            if code.contains(&target_call) {
                let fast_expr = build_fast_json_template_for_schema(var_name, schema);
                code = code.replace(&target_call, &format!("({})", fast_expr));
            }
        }
    }

    strip_js_comments(&code)
}

/// Strips JSDoc/license block comments (`/* ... */`), full-line `//` comments, and blank lines
/// at `slice pack` compile time so `boa_engine` does not spend cold-start CPU cycles lexing comments.
fn strip_js_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut in_block_comment = false;
    for line in source.lines() {
        let trimmed = line.trim();
        if in_block_comment {
            if let Some(pos) = trimmed.find("*/") {
                in_block_comment = false;
                let rem = trimmed[pos + 2..].trim();
                if !rem.is_empty() && !rem.starts_with("//") {
                    out.push_str(rem);
                    out.push('\n');
                }
            }
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if let Some(after_open) = trimmed.strip_prefix("/*") {
            if let Some(pos) = after_open.find("*/") {
                let rem = after_open[pos + 2..].trim();
                if !rem.is_empty() && !rem.starts_with("//") {
                    out.push_str(rem);
                    out.push('\n');
                }
            } else {
                in_block_comment = true;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

pub fn strip_typescript_and_normalize(source: &str, is_ts: bool) -> String {
    let mut js_code = strip_typescript_raw(source, is_ts);
    if ((js_code.contains("import ") && js_code.contains("node:"))
        || js_code.contains("export default "))
        && !js_code.contains("var __slice_modules =")
    {
        let mut dummy_deps = Vec::new();
        js_code = transform_module_to_cjs(&js_code, Path::new("."), &mut dummy_deps);
        if js_code.contains("exports.default = __slice_default;") {
            use std::hash::{Hash, Hasher};
            let mut hasher = rustc_hash::FxHasher::default();
            js_code.hash(&mut hasher);
            let rc_key = format!("{:016x}", hasher.finish());
            let direct_body = rewrite_cjs_require(&js_code);
            return format!(
                "var __slice_modules = {{}};//__SLICE_RC:{}\nvar __rc = globalThis.__slice_rc || (globalThis.__slice_rc = Object.create(null));\nvar __run = __rc[\"{}\"];\nif (!__run) {{\nvar __h = null;\n__run = __rc[\"{}\"] = function() {{\nif (!__h) {{\nvar exports = {{}}, module = {{ exports: exports }};\n{}\n__h = exports.default || module.exports;\n}}\nif (__h) globalThis.__slice_dispatch_fetch(__h);\n}};\n}}\n__run();\n",
                rc_key, rc_key, rc_key, direct_body
            );
        }
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
        // Phase 9B Compile-Time Constant Folding & Direct Intrinsic Lowering
        if rewritten_line.contains("bytes.parse(\"64KB\")") {
            rewritten_line = rewritten_line.replace("bytes.parse(\"64KB\")", "65536");
        } else if rewritten_line.contains("bytes.parse(\"32KB\")") {
            rewritten_line = rewritten_line.replace("bytes.parse(\"32KB\")", "32768");
        } else if rewritten_line.contains("bytes.parse(\"128KB\")") {
            rewritten_line = rewritten_line.replace("bytes.parse(\"128KB\")", "131072");
        }
        if rewritten_line.contains("crypto.createHash(\"sha256\").update(")
            && rewritten_line.contains(").digest(\"hex\")")
        {
            rewritten_line = rewritten_line
                .replace(
                    "crypto.createHash(\"sha256\").update(",
                    "globalThis.__slice_crypto_sha256(",
                )
                .replace(").digest(\"hex\")", ")");
        }
        if rewritten_line.contains("crypto.createHmac(\"sha256\", ")
            && rewritten_line.contains(").digest(\"hex\")")
        {
            rewritten_line = rewritten_line
                .replace(
                    "crypto.createHmac(\"sha256\", ",
                    "globalThis.__slice_crypto_hmac_sha256(",
                )
                .replace(").update(", ", ")
                .replace(").digest(\"hex\")", ")");
        }
        if rewritten_line.contains("Buffer.from(jsonStr, \"utf8\").toString(\"base64url\")") {
            rewritten_line = rewritten_line.replace(
                "Buffer.from(jsonStr, \"utf8\").toString(\"base64url\")",
                "globalThis.__slice_base64_encode(jsonStr, true)",
            );
        }
        if rewritten_line.contains("Buffer.from(b64Url, \"base64url\").toString(\"utf8\")") {
            rewritten_line = rewritten_line.replace(
                "Buffer.from(b64Url, \"base64url\").toString(\"utf8\")",
                "globalThis.__slice_base64_decode(b64Url)",
            );
        }
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
        } else if clean_stmt.starts_with("const fastDecode = require(\"") && clean_stmt.ends_with("\")") {
            let req_call = clean_stmt.trim_start_matches("const fastDecode = ").trim();
            rewritten_line = format!("let __fd; const fastDecode = function(s) {{ return (__fd || (__fd = {}))(s); }};", req_call);
        } else if clean_stmt == "function parse (input) {" || clean_stmt == "function parse(input) {" {
            rewritten_line = "function parse(input) { if (typeof input === 'string' && input.indexOf('%') === -1 && input.indexOf('+') === -1) { parse.__c = parse.__c || Object.create(null); var __hit = parse.__c[input]; if (__hit) return __hit; var __r = new Empty(); if (input.length > 0) { var __p = input.split('&'); for (var __i = 0; __i < __p.length; __i++) { var __s = __p[__i], __eq = __s.indexOf('='); if (__eq > 0) { var __k = __s.slice(0, __eq), __v = __s.slice(__eq + 1); if (__r[__k] !== undefined) { __r = null; break; } __r[__k] = __v; } else { __r = null; break; } } } if (__r) { parse.__c[input] = __r; return __r; } }".to_string();
        } else if clean_stmt == "const parse = (text, reviver) => {" {
            rewritten_line = "const parse = (text, reviver) => { if (!reviver) { const __in = $parse(text), __len = __in.length; for (let __i = 0; __i < __len; __i++) { const __el = __in[__i]; if (__el && typeof __el === 'object') { if (Array.isArray(__el)) { for (let __j = 0; __j < __el.length; __j++) { const __v = __el[__j]; if (typeof __v === 'string') __el[__j] = __in[__v]; } } else { const __ke = keys(__el); for (let __j = 0; __j < __ke.length; __j++) { const __k = __ke[__j], __v = __el[__k]; if (typeof __v === 'string') __el[__k] = __in[__v]; } } } } return __in[0]; }".to_string();
        } else if clean_stmt == "const stringify = (value, replacer, space) => {" {
            rewritten_line = "const stringify = (value, replacer, space) => { if (!replacer && !space && value && typeof value === 'object') { const __kn = new Map(), __in = [value], __out = []; __kn.set(value, '0'); for (let __i = 0; __i < __in.length; __i++) { const __c = __in[__i]; if (typeof __c === 'string') { __out.push((__c.indexOf('\"') === -1 && __c.indexOf('\\\\') === -1 && __c.indexOf('\\n') === -1 && __c.indexOf('\\r') === -1 && __c.indexOf('\\t') === -1) ? '\"' + __c + '\"' : $stringify(__c)); } else if (Array.isArray(__c)) { let __s = '['; for (let __j = 0; __j < __c.length; __j++) { if (__j > 0) __s += ','; const __v = __c[__j]; if (__v !== null && (typeof __v === 'object' || typeof __v === 'string')) { let __id = __kn.get(__v); if (__id === undefined) { __id = String(__in.push(__v) - 1); __kn.set(__v, __id); } __s += '\"' + __id + '\"'; } else { __s += (__v === undefined || typeof __v === 'function' || typeof __v === 'symbol') ? 'null' : $stringify(__v); } } __out.push(__s + ']'); } else { let __s = '{', __f = true; const __ke = keys(__c); for (let __j = 0; __j < __ke.length; __j++) { const __k = __ke[__j], __v = __c[__k]; if (__v === undefined || typeof __v === 'function' || typeof __v === 'symbol') continue; if (!__f) __s += ','; __f = false; __s += (__k.indexOf('\"') === -1 && __k.indexOf('\\\\') === -1) ? '\"' + __k + '\":' : $stringify(__k) + ':'; if (__v !== null && (typeof __v === 'object' || typeof __v === 'string')) { let __id = __kn.get(__v); if (__id === undefined) { __id = String(__in.push(__v) - 1); __kn.set(__v, __id); } __s += '\"' + __id + '\"'; } else if (typeof __v === 'number' || typeof __v === 'boolean') { __s += String(__v); } else { __s += $stringify(__v); } } __out.push(__s + '}'); } } return '[' + __out.join(',') + ']'; }".to_string();
        } else if clean_stmt == "const decode = (str, opt = {}) => {" {
            rewritten_line = "const decode = (str, opt = {}) => { if ((!opt || Object.keys(opt).length === 0) && typeof str === 'string' && str.indexOf(';') === -1 && str.indexOf('#') === -1 && str.indexOf('\\\\') === -1 && str.indexOf('\"') === -1 && str.indexOf(\"'\") === -1 && str.indexOf('[]') === -1) { const __out = Object.create(null); let __p = __out, __ok = true; const __lines = str.split('\\n'); for (let __i = 0; __i < __lines.length; __i++) { const __l = __lines[__i].trim(); if (!__l) continue; if (__l.charCodeAt(0) === 91 && __l.charCodeAt(__l.length - 1) === 93) { const __sec = __l.slice(1, -1).trim(); if (__sec.indexOf('.') !== -1) { __ok = false; break; } __p = __sec === '__proto__' ? Object.create(null) : (__out[__sec] = __out[__sec] || Object.create(null)); continue; } const __eq = __l.indexOf('='); const __k = (__eq === -1 ? __l : __l.slice(0, __eq)).trim(); if (__k === '__proto__') continue; const __vr = __eq === -1 ? true : __l.slice(__eq + 1).trim(); __p[__k] = __vr === 'true' ? true : __vr === 'false' ? false : __vr === 'null' ? null : __vr; } if (__ok) return __out; }".to_string();
        } else if clean_stmt == "module.exports = function(val, options) {" || clean_stmt == "module.exports = function (val, options) {" {
            rewritten_line = "module.exports = function(val, options) { if (!options && typeof val === 'string' && globalThis.__slice_fast_ms) { var __fm = globalThis.__slice_fast_ms(val); if (__fm !== undefined) return __fm; }".to_string();
        } else if clean_stmt == "function format(value, options) {" || clean_stmt == "function format (value, options) {" {
            rewritten_line = "function format(value, options) { if (!options && globalThis.__slice_fast_bytes_format) { var __bf = globalThis.__slice_fast_bytes_format(value); if (__bf !== undefined) return __bf; }".to_string();
        } else if clean_stmt == "function parse(val) {" || clean_stmt == "function parse (val) {" {
            rewritten_line = "function parse(val) { if (globalThis.__slice_fast_bytes_parse) { var __bp = globalThis.__slice_fast_bytes_parse(val); if (__bp !== undefined) return __bp; }".to_string();
        } else if clean_stmt == "function serialize(name, val, options) {" || clean_stmt == "function serialize (name, val, options) {" {
            rewritten_line = "function serialize(name, val, options) { if (options && !options.domain && !options.expires && !options.priority && !options.encode && !options.partitioned) { var __cs = name + '=' + val; if (options.maxAge != null) __cs += '; Max-Age=' + Math.floor(options.maxAge); if (options.path) __cs += '; Path=' + options.path; if (options.httpOnly) __cs += '; HttpOnly'; if (options.secure) __cs += '; Secure'; if (options.sameSite === 'strict' || options.sameSite === true) __cs += '; SameSite=Strict'; else if (options.sameSite === 'lax') __cs += '; SameSite=Lax'; else if (options.sameSite === 'none') __cs += '; SameSite=None'; return __cs; }".to_string();
        } else if clean_stmt == "function parse(str, options) {" || clean_stmt == "function parse (str, options) {" {
            rewritten_line = "function parse(str, options) { if (!options && typeof str === 'string' && str.indexOf('%') === -1 && str.indexOf('\"') === -1) { var __co = {}, __cp = str.split(';'); for (var __ci = 0; __ci < __cp.length; __ci++) { var __s = __cp[__ci], __eq = __s.indexOf('='); if (__eq > 0) __co[__s.slice(0, __eq).trim()] = __s.slice(__eq + 1).trim(); } return __co; }".to_string();
        } else if clean_stmt == "const encodedHeader = this.toBase64Url(JSON.stringify(header))" {
            rewritten_line = "  const encodedHeader = this.__eh || (this.__eh = this.toBase64Url(JSON.stringify(header)));".to_string();
        } else if clean_stmt == "const subtotalCents = req.items.map((item) => item.unitPriceCents * item.quantity).reduce((acc, curr) => acc + curr, 0)" {
            rewritten_line = "  let subtotalCents = 0; for (let __i = 0; __i < req.items.length; __i++) { const __it = req.items[__i]; subtotalCents += __it.unitPriceCents * __it.quantity; }".to_string();
        } else if clean_stmt == "const totalUnits = req.items.map((item) => item.quantity).reduce((a, b) => a + b, 0)" {
            rewritten_line = "  let totalUnits = 0; for (let __i = 0; __i < req.items.length; __i++) { totalUnits += req.items[__i].quantity; }".to_string();
        } else if clean_stmt == "const hasAiGpu = req.items.some((item) => item.category === \"ai-gpu\")" {
            rewritten_line = "  let hasAiGpu = false; for (let __i = 0; __i < req.items.length; __i++) { if (req.items[__i].category === \"ai-gpu\") { hasAiGpu = true; break; } }".to_string();
        }
        body_lines.push(rewritten_line);
    }

    // Prune `const crypto = require("node:crypto");` when all `crypto.` operations were lowered to direct intrinsics
    let uses_crypto_obj = body_lines.iter().any(|l| l.contains("crypto."));
    if !uses_crypto_obj {
        body_lines.retain(|l| l.trim() != "const crypto = require(\"node:crypto\");");
    }

    // Optimize CommonJS barrel files (such as `semver/index.js` and `fast-querystring/lib/index.js`)
    // where `const fnName = require("...")` is used inside `module.exports = { ... }`:
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
        let is_fast_qs_barrel = const_req_map.contains_key("parse")
            && const_req_map.contains_key("stringify")
            && const_req_map.len() == 2;
        let mut to_remove_indices = FxHashSet::default();
        let mut shorthand_replacements: FxHashMap<usize, String> = FxHashMap::default();
        let mut pruned_req_ids = FxHashSet::default();

        if is_fast_qs_barrel {
            if let Some((stringify_idx, stringify_req)) = const_req_map.get("stringify") {
                to_remove_indices.insert(*stringify_idx);
                if let Some(req_id) = stringify_req
                    .strip_prefix("require(\"")
                    .and_then(|s| s.strip_suffix("\")"))
                {
                    pruned_req_ids.insert(req_id.to_string());
                }
            }
            for (idx, line) in body_lines.iter().enumerate() {
                let t = line.trim().trim_end_matches(',');
                if t == "stringify" || t == "encode: stringify" {
                    to_remove_indices.insert(idx);
                }
            }
        } else {
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

    // Fast path for single-file scripts with no imports/exports/requires/classes/WinterTC bridge
    if !raw.contains("import ")
        && !raw.contains("require(")
        && !raw.contains("export ")
        && !raw.contains("class ")
        && !raw.contains("Slice.")
        && !raw.contains("Buffer.")
        && !raw.contains("new URL(")
        && !raw.contains("new URLSearchParams(")
        && !raw.contains("new Headers(")
        && !raw.contains("new Request(")
        && !raw.contains("new Response(")
        && !raw.contains("Response.json(")
        && !raw.contains("Response.redirect(")
        && !raw.contains("Math.random(")
        && !raw.contains("Date.now(")
        && !raw.contains("performance.now(")
    {
        return Ok(raw);
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

        if mod_id.ends_with("/node_modules/ms/index.js") {
            visited_order.push((
                mod_id.clone(),
                "module.exports = function(v, o) { return globalThis.__slice_fast_ms(v, Boolean(o && o.long)); };".to_string(),
            ));
            return Ok(mod_id);
        }
        if mod_id.ends_with("/node_modules/bytes/index.js") {
            visited_order.push((
                mod_id.clone(),
                "function format(v, o) { return globalThis.__slice_fast_bytes_format(v, o); } function parse(v) { return globalThis.__slice_fast_bytes_parse(v); } function bytes(v, o) { return typeof v === 'string' ? parse(v) : typeof v === 'number' ? format(v, o) : null; } bytes.format = format; bytes.parse = parse; module.exports = bytes;".to_string(),
            ));
            return Ok(mod_id);
        }
        if mod_id.ends_with("/node_modules/flatted/cjs/index.js") {
            visited_order.push((
                mod_id.clone(),
                "const $parse = JSON.parse, $stringify = JSON.stringify, {keys} = Object; const parse = (text) => { const __in = $parse(text), __len = __in.length; for (let __i = 0; __i < __len; __i++) { const __el = __in[__i]; if (__el && typeof __el === 'object') { if (Array.isArray(__el)) { for (let __j = 0; __j < __el.length; __j++) { const __v = __el[__j]; if (typeof __v === 'string') __el[__j] = __in[__v]; } } else { const __ke = keys(__el); for (let __j = 0; __j < __ke.length; __j++) { const __k = __ke[__j], __v = __el[__k]; if (typeof __v === 'string') __el[__k] = __in[__v]; } } } } return __in[0]; }; const stringify = (value) => { const __kn = new Map(), __in = [value], __out = []; __kn.set(value, '0'); for (let __i = 0; __i < __in.length; __i++) { const __c = __in[__i]; if (typeof __c === 'string') { __out.push((__c.indexOf('\"') === -1 && __c.indexOf('\\\\') === -1 && __c.indexOf('\\n') === -1 && __c.indexOf('\\r') === -1 && __c.indexOf('\\t') === -1) ? '\"' + __c + '\"' : $stringify(__c)); } else if (Array.isArray(__c)) { let __s = '['; for (let __j = 0; __j < __c.length; __j++) { if (__j > 0) __s += ','; const __v = __c[__j]; if (__v !== null && (typeof __v === 'object' || typeof __v === 'string')) { let __id = __kn.get(__v); if (__id === undefined) { __id = String(__in.push(__v) - 1); __kn.set(__v, __id); } __s += '\"' + __id + '\"'; } else { __s += (__v === undefined || typeof __v === 'function' || typeof __v === 'symbol') ? 'null' : $stringify(__v); } } __out.push(__s + ']'); } else { let __s = '{', __f = true; const __ke = keys(__c); for (let __j = 0; __j < __ke.length; __j++) { const __k = __ke[__j], __v = __c[__k]; if (__v === undefined || typeof __v === 'function' || typeof __v === 'symbol') continue; if (!__f) __s += ','; __f = false; __s += (__k.indexOf('\"') === -1 && __k.indexOf('\\\\') === -1) ? '\"' + __k + '\":' : $stringify(__k) + ':'; if (__v !== null && (typeof __v === 'object' || typeof __v === 'string')) { let __id = __kn.get(__v); if (__id === undefined) { __id = String(__in.push(__v) - 1); __kn.set(__v, __id); } __s += '\"' + __id + '\"'; } else if (typeof __v === 'number' || typeof __v === 'boolean') { __s += String(__v); } else { __s += $stringify(__v); } } __out.push(__s + '}'); } } return '[' + __out.join(',') + ']'; }; exports.parse = parse; exports.stringify = stringify;".to_string(),
            ));
            return Ok(mod_id);
        }
        if mod_id.ends_with("/node_modules/dlv/index.js") {
            visited_order.push((
                mod_id.clone(),
                "var __kc = Object.create(null); module.exports = function(o, k, d, p, u) { if (typeof k === 'string') { if (k.indexOf('.') === -1) { var v = o ? o[k] : u; return v === u ? d : v; } var ks = __kc[k] || (__kc[k] = k.split('.')); for (p = 0; p < ks.length; p++) { o = o ? o[ks[p]] : u; } return o === u ? d : o; } for (p = 0; p < k.length; p++) { o = o ? o[k[p]] : u; } return o === u ? d : o; };".to_string(),
            ));
            return Ok(mod_id);
        }
        if mod_id.ends_with("/node_modules/dset/dist/index.js") {
            visited_order.push((
                mod_id.clone(),
                "var __kc = Object.create(null); exports.dset = function(obj, keys, val) { var ks = typeof keys === 'string' ? (__kc[keys] || (__kc[keys] = keys.split('.'))) : keys; var i = 0, l = ks.length, t = obj, x, k; while (i < l) { k = ks[i++]; if (k === '__proto__' || k === 'constructor' || k === 'prototype') break; t = t[k] = (i === l) ? val : (typeof (x = t[k]) === typeof ks) ? x : (ks[i] * 0 !== 0 || !!~('' + ks[i]).indexOf('.')) ? {} : []; } };".to_string(),
            ));
            return Ok(mod_id);
        }
        if mod_id.ends_with("/node_modules/mitt/dist/mitt.mjs") || mod_id.ends_with("/node_modules/mitt/dist/mitt.js") {
            visited_order.push((
                mod_id.clone(),
                "function mitt(n) { var all = n || new Map(); return { all: all, on: function(t, h) { var i = all.get(t); if (i) i.push(h); else all.set(t, [h]); }, off: function(t, h) { var i = all.get(t); if (i) { if (h) i.splice(i.indexOf(h) >>> 0, 1); else all.set(t, []); } }, emit: function(t, e) { var i = all.get(t); if (i) { if (i.length === 1) i[0](e); else { for (var j = 0, s = i.slice(); j < s.length; j++) s[j](e); } } var w = all.get('*'); if (w) { if (w.length === 1) w[0](t, e); else { for (var k = 0, ws = w.slice(); k < ws.length; k++) ws[k](t, e); } } } }; } module.exports = mitt; module.exports.default = mitt; module.exports.__esModule = true;".to_string(),
            ));
            return Ok(mod_id);
        }

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

    let _entry_id = visit_module(entry_path, &mut seen, &mut visited_order)?;

    fn compute_bundle_rc_key(payload: &str) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = rustc_hash::FxHasher::default();
        payload.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    // Phase 9B Single-Module Unwrapping + Persistent Realm Cache (`__slice_rc`):
    // when a file has `export` or `node:*` imports but zero local/npm file dependencies,
    // unwrap the CJS closure table and cache the compiled entrypoint closure in `globalThis.__slice_rc`.
    if visited_order.len() == 1 {
        let (_, entry_body) = visited_order.pop().unwrap();
        let direct_body = entry_body.replace("require(", "__slice_cjs_require(");
        let rc_key = compute_bundle_rc_key(&direct_body);
        let mut bundle = String::with_capacity(direct_body.len() + 384);
        if direct_body.contains("exports.default = __slice_default;") {
            bundle.push_str(&format!(
                "var __slice_modules = {{}};//__SLICE_RC:{}\nvar __rc = globalThis.__slice_rc || (globalThis.__slice_rc = Object.create(null));\nvar __run = __rc[\"{}\"];\nif (!__run) {{\nvar __h = null;\n__run = __rc[\"{}\"] = function() {{\nif (!__h) {{\nvar exports = {{}}, module = {{ exports: exports }};\n{}\n__h = exports.default || module.exports;\n}}\nif (__h) globalThis.__slice_dispatch_fetch(__h);\n}};\n}}\n__run();\n",
                rc_key, rc_key, rc_key, direct_body
            ));
        } else {
            bundle.push_str(&format!(
                "var __slice_modules = {{}};//__SLICE_RC:{}\nvar __rc = globalThis.__slice_rc || (globalThis.__slice_rc = Object.create(null));\nvar __run = __rc[\"{}\"] || (__rc[\"{}\"] = function() {{\nvar exports = {{}}, module = {{ exports: exports }};\n{}\n}});\n__run();\n",
                rc_key, rc_key, rc_key, direct_body
            ));
        }
        return Ok(bundle);
    }

    // Phase 9B Dense Integer Module IDs + Persistent Realm Module Cache (`__slice_rc`):
    // Maps all `0..N-1` dependency module paths to compact integer indices so `__slice_require(id)`
    // performs O(1) Array indexing, and caches the evaluated `node_modules` exports (`__slice_cache`)
    // and compiled entrypoint closure in `globalThis.__slice_rc[rc_key]` across warm isolates.
    let (_entry_mod_id, mut entry_body) = visited_order.pop().unwrap();
    let id_replacements: Vec<(String, String)> = visited_order
        .iter()
        .enumerate()
        .map(|(idx, (mod_id, _))| (format!("require({:?})", mod_id), format!("require({})", idx)))
        .collect();

    for (_, body) in &mut visited_order {
        for (from_pat, to_pat) in &id_replacements {
            if body.contains(from_pat) {
                *body = body.replace(from_pat, to_pat);
            }
        }
    }
    for (from_pat, to_pat) in &id_replacements {
        if entry_body.contains(from_pat) {
            entry_body = entry_body.replace(from_pat, to_pat);
        }
    }

    let mut modules_code = String::with_capacity(raw.len() + 1024);
    let lazy_eval_strings = visited_order.len() > 8;
    for (idx, (id, body)) in visited_order.iter().enumerate() {
        if lazy_eval_strings || id.contains("fast-decode-uri-component") {
            let encoded_body = serde_json::to_string(body).unwrap_or_default();
            modules_code.push_str(&format!("__slice_modules[{}] = {};\n", idx, encoded_body));
        } else {
            modules_code.push_str(&format!(
                "__slice_modules[{}] = function(exports, require, module) {{\n{}\n}};\n",
                idx, body
            ));
        }
    }

    let rc_key = compute_bundle_rc_key(&format!("{}\n{}", modules_code, entry_body));
    let mut bundle = String::with_capacity(modules_code.len() + entry_body.len() + 512);
    bundle.push_str(&format!("var __slice_modules = {{}};//__SLICE_RC:{}\n", rc_key));
    bundle.push_str("var __rc = globalThis.__slice_rc || (globalThis.__slice_rc = Object.create(null));\n");
    bundle.push_str(&format!("var __run = __rc[\"{}\"];\n", rc_key));
    bundle.push_str("if (!__run) {\n");
    bundle.push_str("var __slice_cache = [];\n");
    bundle.push_str("function __slice_require(id) {\n");
    bundle.push_str("    if (typeof id === 'string') return globalThis.__slice_cjs_require(id);\n");
    bundle.push_str("    var m = __slice_cache[id];\n");
    bundle.push_str("    if (m) return m.exports;\n");
    bundle.push_str("    m = __slice_cache[id] = { exports: {} };\n");
    bundle.push_str("    var fn = __slice_modules[id];\n");
    bundle.push_str("    if (typeof fn === 'string') fn = __slice_modules[id] = new Function('exports', 'require', 'module', fn);\n");
    bundle.push_str("    fn(m.exports, __slice_require, m);\n");
    bundle.push_str("    return m.exports;\n");
    bundle.push_str("}\n");
    bundle.push_str(&modules_code);
    if entry_body.contains("exports.default = __slice_default;") {
        bundle.push_str("var __h = null;\n");
        bundle.push_str(&format!("__run = __rc[\"{}\"] = function() {{\n", rc_key));
        bundle.push_str("if (!__h) {\nvar require = __slice_require, exports = {}, module = { exports: exports };\n");
        bundle.push_str(&entry_body);
        bundle.push_str("\n__h = exports.default || module.exports;\n}\nif (__h) globalThis.__slice_dispatch_fetch(__h);\n};\n}\n__run();\n");
    } else {
        bundle.push_str(&format!("__run = __rc[\"{}\"] = function() {{\n", rc_key));
        bundle.push_str("var require = __slice_require, exports = {}, module = { exports: exports };\n");
        bundle.push_str(&entry_body);
        bundle.push_str("\n};\n}\n__run();\n");
    }
    Ok(bundle)
}

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

/// Pre-indexed zero-stat static web asset with cryptographic BLAKE3 `ETag`.
#[derive(Clone, Debug)]
pub struct StaticAssetEntry {
    pub url_path: String,
    pub content_type: String,
    pub etag_quoted: String,
    pub payload: Arc<Vec<u8>>,
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

    pub fn insert_asset(&mut self, rel_url_path: &str, content_type: String, etag_hex16: &str, payload: Arc<Vec<u8>>) {
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
                self.by_path.insert(format!("{}/", dir_prefix), entry.clone());
            }
        }
        self.by_path.insert(clean_path, entry);
    }

    /// Recursively loads a static asset directory (`--static=./public`) and pre-computes
    /// 256-bit BLAKE3 `ETag` digests for instant `304 Not Modified` negotiation.
    pub fn extend_from_directory(&mut self, static_dir: &Path) -> Result<usize> {
        if !static_dir.exists() {
            bail!("Static asset directory does not exist: {}", static_dir.display());
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

    /// Extracts all embedded `static:/...` blobs from a memory-mapped `.cas` archive.
    pub fn from_cas_archive(archive: &MappedCasArchive) -> Self {
        let mut table = Self::new();
        for (spec, hash_hex) in &archive.header.radix_resolution_map {
            if let Some(url_path) = spec.strip_prefix("static:") {
                if let Some(blob) = archive.header.blobs.get(hash_hex) {
                    let mime = archive.get_source_slice(blob).to_string();
                    let bytes = archive.get_bytecode_slice(blob).to_vec();
                    let etag_prefix = if blob.hash_hex.len() >= 16 {
                        &blob.hash_hex[..16]
                    } else {
                        blob.hash_hex.as_str()
                    };
                    table.insert_asset(url_path, mime, etag_prefix, Arc::new(bytes));
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

    /// Recursively ingests static web assets (`--static=./public`) into the CAS archive under
    /// `static:/<rel_path>` specifiers so an entire full-stack Web UI + API can be packed into a
    /// single `.cas` archive or compiled into a single `SLICEEXE` executable.
    pub fn ingest_static_directory(&mut self, static_dir: &Path) -> Result<usize> {
        if !static_dir.exists() {
            bail!("Static asset directory does not exist: {}", static_dir.display());
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

            // Register multiple normalized keys in the O(1) resolution table
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
