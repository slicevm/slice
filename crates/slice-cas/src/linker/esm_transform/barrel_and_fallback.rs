//! CommonJS Barrel Lazy-Getter Optimizer (`apply_cjs_lazy_barrel_getters`)
//! and Line-Oriented ESM Fallback Lowering.

use super::{
    choose_hygienic_ident, choose_hygienic_prefix, format_default_import_unwrap,
    format_namespace_import_rhs, ESM_ESMODULE_DEFINE,
};
use crate::linker::{
    format_exports_lhs, is_valid_js_identifier, resolve_spec_and_record, rewrite_cjs_requires_ast,
};
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};

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
    if let Some(stripped) = trimmed
        .strip_prefix('\'')
        .and_then(|r| r.strip_suffix('\''))
    {
        return Some((stripped.to_string(), '\''));
    }
    None
}

fn line_contains_ident_token(line: &str, ident: &str) -> bool {
    let bytes = line.as_bytes();
    let ibytes = ident.as_bytes();
    let ilen = ibytes.len();
    let len = bytes.len();
    if ilen == 0 || len < ilen {
        return false;
    }
    let is_cont = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
    let mut i = 0usize;
    while i < len {
        let b = bytes[i];
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            break;
        }
        if b == b'\'' || b == b'"' || b == b'`' {
            let q = b;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(len);
                    continue;
                }
                if bytes[i] == q {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if is_cont(b) {
            let start = i;
            i += 1;
            while i < len && is_cont(bytes[i]) {
                i += 1;
            }
            if &bytes[start..i] == ibytes {
                return true;
            }
            continue;
        }
        i += 1;
    }
    false
}

fn update_cjs_line_nesting(
    line: &str,
    brace_depth: &mut isize,
    bracket_depth: &mut isize,
    paren_depth: &mut isize,
) {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    while i < len {
        let b = bytes[i];
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            break;
        }
        if b == b'\'' || b == b'"' || b == b'`' {
            let q = b;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(len);
                    continue;
                }
                if bytes[i] == q {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        match b {
            b'{' => *brace_depth += 1,
            b'}' => *brace_depth -= 1,
            b'[' => *bracket_depth += 1,
            b']' => *bracket_depth -= 1,
            b'(' => *paren_depth += 1,
            b')' => *paren_depth -= 1,
            _ => {}
        }
        i += 1;
    }
}

pub(super) fn apply_cjs_lazy_barrel_getters(mut body_lines: Vec<String>) -> Vec<String> {
    if body_lines.iter().any(|l| l.trim() == "module.exports = {") {
        let mut const_req_map: FxHashMap<String, (usize, String)> = FxHashMap::default();
        for (idx, line) in body_lines.iter().enumerate() {
            let t = line.trim().trim_end_matches(';');
            if let Some(rest) = t.strip_prefix("const ") {
                if let Some((ident, rhs)) = rest.split_once('=') {
                    let ident = ident.trim();
                    let rhs = rhs.trim();
                    if is_valid_js_identifier(ident)
                        && rhs.starts_with("require(\"")
                        && rhs.ends_with("\")")
                    {
                        const_req_map.insert(ident.to_string(), (idx, rhs.to_string()));
                    }
                }
            }
        }
        let mut to_remove_indices = FxHashSet::default();
        let mut shorthand_replacements: FxHashMap<usize, String> = FxHashMap::default();

        let mut brace_depth = 0isize;
        let mut bracket_depth = 0isize;
        let mut paren_depth = 0isize;
        let mut in_module_exports_obj = false;

        for (idx, line) in body_lines.iter().enumerate() {
            let t = line.trim();
            if !in_module_exports_obj
                && brace_depth == 0
                && bracket_depth == 0
                && paren_depth == 0
                && t == "module.exports = {"
            {
                in_module_exports_obj = true;
                update_cjs_line_nesting(t, &mut brace_depth, &mut bracket_depth, &mut paren_depth);
                if brace_depth <= 0 {
                    in_module_exports_obj = false;
                }
                continue;
            }

            if in_module_exports_obj && brace_depth == 1 && bracket_depth == 0 && paren_depth == 0 {
                if let Some(ident) = t.strip_suffix(',') {
                    let ident = ident.trim();
                    if is_valid_js_identifier(ident) {
                        if let Some((decl_idx, req_expr)) = const_req_map.get(ident) {
                            let used_elsewhere =
                                body_lines
                                    .iter()
                                    .enumerate()
                                    .any(|(other_idx, other_line)| {
                                        other_idx != idx
                                            && other_idx != *decl_idx
                                            && line_contains_ident_token(other_line, ident)
                                    });
                            if !used_elsewhere {
                                to_remove_indices.insert(*decl_idx);
                                shorthand_replacements.insert(
                                    idx,
                                    format!("  get {ident}() {{ return {req_expr}; }},"),
                                );
                            }
                        }
                    }
                }
            }

            update_cjs_line_nesting(t, &mut brace_depth, &mut bracket_depth, &mut paren_depth);
            if in_module_exports_obj && brace_depth <= 0 {
                in_module_exports_obj = false;
            }
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
    body_lines
}

pub(super) fn transform_esm_to_cjs_line_fallback(
    js_code: &str,
    from_dir: &Path,
    deps_out: &mut Vec<PathBuf>,
) -> String {
    let collapsed = collapse_multiline_imports_exports(js_code);
    let def_var = choose_hygienic_ident(js_code, "__slice_default", "__slice", "_default");
    let imp_pfx = choose_hygienic_prefix(js_code, "__imp_", "__imp");
    let reexp_pfx = choose_hygienic_prefix(js_code, "__reexp_", "__reexp");
    let mut body_lines = Vec::new();
    let mut deferred_exports: Vec<(String, String)> = Vec::new();
    let mut has_default_export = false;
    let mut has_any_esm_export = false;
    let mut req_counter = 0usize;

    for raw_line in collapsed.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(after_import) = trimmed.strip_prefix("import ") {
            if after_import.starts_with("type ") {
                continue;
            }
            if let Some((spec, _)) = extract_quoted_specifier(after_import) {
                let resolved_id = resolve_spec_and_record(&spec, from_dir, deps_out);
                body_lines.push(format!("require({resolved_id:?});"));
                continue;
            }
            if let Some((clause, from_part)) = after_import.split_once(" from ") {
                if let Some((spec, _)) = extract_quoted_specifier(from_part) {
                    let resolved_id = resolve_spec_and_record(&spec, from_dir, deps_out);
                    let clause = clause.trim();
                    if let Some(ns_name) = clause.strip_prefix("* as ") {
                        let ns = ns_name.trim();
                        if is_valid_js_identifier(ns) {
                            let rhs = format_namespace_import_rhs(&spec, &resolved_id);
                            body_lines.push(format!("const {ns} = {rhs};"));
                        }
                    } else if clause.starts_with('{') && clause.ends_with('}') {
                        let inner = &clause[1..clause.len() - 1];
                        let mut fields = Vec::new();
                        for part in inner.split(',') {
                            let p = part.trim();
                            if p.is_empty() || p.starts_with("type ") {
                                continue;
                            }
                            if let Some((orig, alias)) = p.split_once(" as ") {
                                let orig = orig.trim();
                                let alias = alias.trim();
                                if is_valid_js_identifier(orig) && is_valid_js_identifier(alias) {
                                    fields.push(format!("{orig}: {alias}"));
                                }
                            } else if is_valid_js_identifier(p) {
                                fields.push(p.to_string());
                            }
                        }
                        if !fields.is_empty() {
                            body_lines.push(format!(
                                "const {{ {} }} = require({resolved_id:?});",
                                fields.join(", ")
                            ));
                        }
                    } else {
                        let def_name = clause.trim();
                        if is_valid_js_identifier(def_name) {
                            req_counter += 1;
                            let tmp = format!("{imp_pfx}{req_counter}");
                            let unwrap_expr = format_default_import_unwrap(&tmp);
                            body_lines.push(format!(
                                "const {tmp} = require({resolved_id:?}); const {def_name} = {unwrap_expr};"
                            ));
                        }
                    }
                    continue;
                }
            }
        }

        if let Some(after_export) = trimmed.strip_prefix("export ") {
            let after_export = after_export.trim();
            if after_export.starts_with("type ") || after_export.starts_with("interface ") {
                continue;
            }
            if let Some(after_star) = after_export.strip_prefix("* from ") {
                if let Some((spec, _)) = extract_quoted_specifier(after_star) {
                    has_any_esm_export = true;
                    let resolved_id = resolve_spec_and_record(&spec, from_dir, deps_out);
                    body_lines.push(format!(
                        "globalThis.__slice_export_star(exports, require({resolved_id:?}));"
                    ));
                    continue;
                }
            }
            if after_export.starts_with('{') {
                has_any_esm_export = true;
                if let Some((brace_part, from_part)) = after_export.split_once(" from ") {
                    if let Some((spec, _)) = extract_quoted_specifier(from_part) {
                        let resolved_id = resolve_spec_and_record(&spec, from_dir, deps_out);
                        let inner = brace_part
                            .trim()
                            .trim_start_matches('{')
                            .trim_end_matches('}');
                        req_counter += 1;
                        let tmp = format!("{reexp_pfx}{req_counter}");
                        body_lines.push(format!("const {tmp} = require({resolved_id:?});"));
                        for item in inner.split(',') {
                            let item = item.trim();
                            if item.is_empty() || item.starts_with("type ") {
                                continue;
                            }
                            if let Some((orig, alias)) = item.split_once(" as ") {
                                let orig = orig.trim();
                                let alias = alias.trim();
                                if orig == "default" {
                                    let lhs = format_exports_lhs(alias);
                                    let unwrap = format_default_import_unwrap(&tmp);
                                    body_lines.push(format!("{lhs} = {unwrap};"));
                                } else if is_valid_js_identifier(orig) {
                                    let lhs = format_exports_lhs(alias);
                                    body_lines.push(format!("{lhs} = {tmp}.{orig};"));
                                }
                            } else if item == "default" {
                                let unwrap = format_default_import_unwrap(&tmp);
                                body_lines.push(format!("exports.default = {unwrap};"));
                            } else if is_valid_js_identifier(item) {
                                body_lines.push(format!("exports.{item} = {tmp}.{item};"));
                            }
                        }
                        continue;
                    }
                } else if let Some(end_b) = after_export.find('}') {
                    let inner = &after_export[1..end_b];
                    for item in inner.split(',') {
                        let item = item.trim();
                        if item.is_empty() || item.starts_with("type ") {
                            continue;
                        }
                        if let Some((orig, alias)) = item.split_once(" as ") {
                            let orig = orig.trim();
                            let alias = alias.trim();
                            if is_valid_js_identifier(orig) {
                                deferred_exports.push((alias.to_string(), orig.to_string()));
                            }
                        } else if is_valid_js_identifier(item) {
                            deferred_exports.push((item.to_string(), item.to_string()));
                        }
                    }
                    continue;
                }
            }
            if let Some(def_expr) = after_export.strip_prefix("default ") {
                has_default_export = true;
                body_lines.push(format!("const {def_var} = {def_expr}"));
                continue;
            }
            for kw in ["const ", "let ", "var "] {
                if let Some(decl_rest) = after_export.strip_prefix(kw) {
                    has_any_esm_export = true;
                    if let Some(ident) = decl_rest
                        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '$')
                        .next()
                    {
                        if is_valid_js_identifier(ident) {
                            deferred_exports.push((ident.to_string(), ident.to_string()));
                        }
                    }
                    body_lines.push(rewrite_cjs_requires_ast(after_export, from_dir, deps_out));
                    break;
                }
            }
            if after_export.starts_with("const ")
                || after_export.starts_with("let ")
                || after_export.starts_with("var ")
            {
                continue;
            }
            for kw in ["async function ", "function ", "class "] {
                if let Some(decl_rest) = after_export.strip_prefix(kw) {
                    has_any_esm_export = true;
                    if let Some(ident) = decl_rest
                        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '$')
                        .next()
                    {
                        if is_valid_js_identifier(ident) {
                            deferred_exports.push((ident.to_string(), ident.to_string()));
                        }
                    }
                    body_lines.push(rewrite_cjs_requires_ast(after_export, from_dir, deps_out));
                    break;
                }
            }
            if after_export.starts_with("async function ")
                || after_export.starts_with("function ")
                || after_export.starts_with("class ")
            {
                continue;
            }
        }

        body_lines.push(rewrite_cjs_requires_ast(raw_line, from_dir, deps_out));
    }

    body_lines = apply_cjs_lazy_barrel_getters(body_lines);

    if has_default_export || !deferred_exports.is_empty() || has_any_esm_export {
        body_lines.push(ESM_ESMODULE_DEFINE.to_string());
    }
    for (export_name, local_name) in &deferred_exports {
        let lhs = format_exports_lhs(export_name);
        body_lines.push(format!("{lhs} = {local_name};"));
    }
    if has_default_export {
        body_lines.push(format!("exports.default = {def_var};"));
    }

    body_lines.join("\n")
}
