//! Oxc AST-Driven ESM-to-CommonJS Linker, Barrel Lazy-Getter Optimizer,
//! and Built-in Module Classifier.

use crate::resolver::resolve_module_specifier;
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};

#[inline]
pub(crate) fn is_builtin_module(spec: &str) -> bool {
    if spec.starts_with("node:")
        || spec.starts_with("slice:")
        || spec.starts_with("bun:")
        || spec.starts_with("cloudflare:")
    {
        return true;
    }
    matches!(
        spec,
        "fs" | "fs/promises"
            | "path"
            | "path/posix"
            | "path/win32"
            | "crypto"
            | "os"
            | "util"
            | "util/types"
            | "events"
            | "buffer"
            | "process"
            | "url"
            | "stream"
            | "assert"
            | "assert/strict"
            | "timers"
            | "timers/promises"
            | "worker_threads"
            | "net"
            | "dgram"
    )
}

#[inline]
pub(crate) fn is_jsx_runtime_builtin(spec: &str) -> bool {
    matches!(
        spec,
        "react"
            | "react/jsx-runtime"
            | "react/jsx-dev-runtime"
            | "preact"
            | "preact/jsx-runtime"
            | "preact/jsx-dev-runtime"
            | "hono/jsx"
            | "hono/jsx/jsx-runtime"
            | "hono/jsx/jsx-dev-runtime"
    )
}

#[inline]
pub(crate) fn normalize_builtin_specifier(spec: &str) -> String {
    if spec.starts_with("node:")
        || spec.starts_with("slice:")
        || spec.starts_with("bun:")
        || spec.starts_with("cloudflare:")
    {
        spec.to_string()
    } else if is_jsx_runtime_builtin(spec) {
        "slice:jsx".to_string()
    } else {
        format!("node:{}", spec)
    }
}

#[inline]
pub(crate) fn canonical_mod_id(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
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
    if let Some(stripped) = trimmed
        .strip_prefix('\'')
        .and_then(|r| r.strip_suffix('\''))
    {
        return Some((stripped.to_string(), '\''));
    }
    None
}

pub(crate) fn rewrite_cjs_requires_in_line(
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
                                let norm = normalize_builtin_specifier(spec);
                                result.push_str(&format!("{:?}", norm));
                                let consumed = ws_len + 1 + end_q + 1;
                                rest = &after_req[consumed..];
                                continue;
                            } else if let Some(resolved_path) =
                                resolve_module_specifier(from_dir, spec)
                            {
                                let id = canonical_mod_id(&resolved_path);
                                deps_out.push(resolved_path);
                                result.push_str(&format!("{:?}", id));
                                let consumed = ws_len + 1 + end_q + 1;
                                rest = &after_req[consumed..];
                                continue;
                            } else if is_jsx_runtime_builtin(spec) {
                                let norm = normalize_builtin_specifier(spec);
                                result.push_str(&format!("{:?}", norm));
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

pub(crate) fn resolve_spec_and_record(
    spec: &str,
    from_dir: &Path,
    deps_out: &mut Vec<PathBuf>,
) -> String {
    if is_builtin_module(spec) {
        normalize_builtin_specifier(spec)
    } else if let Some(resolved) = resolve_module_specifier(from_dir, spec) {
        let id = canonical_mod_id(&resolved);
        deps_out.push(resolved);
        id
    } else if is_jsx_runtime_builtin(spec) {
        normalize_builtin_specifier(spec)
    } else {
        spec.to_string()
    }
}

fn collect_binding_pattern_names(
    pat: &oxc::ast::ast::BindingPattern<'_>,
    out: &mut Vec<(String, String)>,
) {
    use oxc::ast::ast::BindingPattern;
    match pat {
        BindingPattern::BindingIdentifier(ident) => {
            let n = ident.name.to_string();
            out.push((n.clone(), n));
        }
        BindingPattern::ObjectPattern(obj) => {
            for prop in &obj.properties {
                collect_binding_pattern_names(&prop.value, out);
            }
            if let Some(rest) = &obj.rest {
                collect_binding_pattern_names(&rest.argument, out);
            }
        }
        BindingPattern::ArrayPattern(arr) => {
            for elem in arr.elements.iter().flatten() {
                collect_binding_pattern_names(elem, out);
            }
            if let Some(rest) = &arr.rest {
                collect_binding_pattern_names(&rest.argument, out);
            }
        }
        BindingPattern::AssignmentPattern(assign) => {
            collect_binding_pattern_names(&assign.left, out);
        }
    }
}

fn rewrite_cjs_requires_and_crypto(
    snippet: &str,
    from_dir: &Path,
    deps_out: &mut Vec<PathBuf>,
) -> String {
    let mut out = Vec::new();
    for raw_line in snippet.lines() {
        let mut rewritten = rewrite_cjs_requires_in_line(raw_line, from_dir, deps_out);
        if rewritten.contains("crypto.createHash(\"sha256\").update(")
            && rewritten.contains(").digest(\"hex\")")
        {
            rewritten = rewritten
                .replace(
                    "crypto.createHash(\"sha256\").update(",
                    "globalThis.__slice_crypto_sha256(",
                )
                .replace(").digest(\"hex\")", ")");
        }
        if rewritten.contains("crypto.createHmac(\"sha256\", ")
            && rewritten.contains(").digest(\"hex\")")
        {
            rewritten = rewritten
                .replace(
                    "crypto.createHmac(\"sha256\", ",
                    "globalThis.__slice_crypto_hmac_sha256(",
                )
                .replace(").update(", ", ")
                .replace(").digest(\"hex\")", ")");
        }
        out.push(rewritten);
    }
    out.join("\n")
}

fn try_transform_esm_to_cjs_ast(
    js_code: &str,
    from_dir: &Path,
    deps_out: &mut Vec<PathBuf>,
) -> Option<String> {
    use oxc::ast::ast::{
        Declaration, ExportDefaultDeclarationKind, ImportDeclarationSpecifier, Statement,
    };
    use oxc::span::GetSpan;

    let allocator = oxc::allocator::Allocator::default();
    let source_type = oxc::span::SourceType::ts().with_module(true);
    let parser_ret = oxc::parser::Parser::new(&allocator, js_code, source_type).parse();
    if !parser_ret.diagnostics.is_empty() {
        return None;
    }

    let has_esm_stmt = parser_ret.program.body.iter().any(|stmt| {
        matches!(
            stmt,
            Statement::ImportDeclaration(_)
                | Statement::ExportDeclaration(_)
                | Statement::ExportNamedDeclaration(_)
                | Statement::ExportFromDeclaration(_)
                | Statement::ExportDefaultDeclaration(_)
                | Statement::ExportAllDeclaration(_)
        )
    });
    if !has_esm_stmt {
        return None;
    }

    let mut body_lines = Vec::with_capacity(parser_ret.program.body.len() + 4);
    let mut deferred_exports: Vec<(String, String)> = Vec::new();
    let mut has_default_export = false;
    let mut req_counter = 0usize;

    for stmt in &parser_ret.program.body {
        match stmt {
            Statement::ImportDeclaration(imp) => {
                if imp.import_kind.is_type() {
                    continue;
                }
                let spec = imp.source.value.as_str();
                let resolved_id = resolve_spec_and_record(spec, from_dir, deps_out);
                let Some(specs) = &imp.specifiers else {
                    body_lines.push(format!("require({:?});", resolved_id));
                    continue;
                };
                if specs.is_empty() {
                    body_lines.push(format!("require({:?});", resolved_id));
                    continue;
                }
                let mut default_ident: Option<&str> = None;
                let mut namespace_ident: Option<&str> = None;
                let mut named_specs: Vec<(String, String)> = Vec::new();

                for s in specs {
                    match s {
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(def) => {
                            default_ident = Some(def.local.name.as_str());
                        }
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(ns) => {
                            namespace_ident = Some(ns.local.name.as_str());
                        }
                        ImportDeclarationSpecifier::ImportSpecifier(named) => {
                            if named.import_kind.is_type() {
                                continue;
                            }
                            named_specs.push((
                                named.imported.name().to_string(),
                                named.local.name.to_string(),
                            ));
                        }
                    }
                }

                if default_ident.is_none() && namespace_ident.is_none() && named_specs.is_empty() {
                    continue;
                }

                if let Some(ns) = namespace_ident {
                    body_lines.push(format!("const {} = require({:?});", ns, resolved_id));
                }
                if let Some(def_name) = default_ident {
                    req_counter += 1;
                    let tmp_var = format!("__imp_{}", req_counter);
                    if !named_specs.is_empty() {
                        let fields: Vec<String> = named_specs
                            .iter()
                            .map(|(imp_n, loc_n)| {
                                if imp_n == loc_n {
                                    imp_n.clone()
                                } else {
                                    format!("{}: {}", imp_n, loc_n)
                                }
                            })
                            .collect();
                        body_lines.push(format!(
                            "const {tmp} = require({id:?}); const {def} = ({tmp} && {tmp}.__esModule && {tmp}.default !== undefined) ? {tmp}.default : ({tmp} && {tmp}.default !== undefined ? {tmp}.default : {tmp}); const {{ {destr} }} = {tmp};",
                            tmp = tmp_var,
                            id = resolved_id,
                            def = def_name,
                            destr = fields.join(", ")
                        ));
                    } else {
                        body_lines.push(format!(
                            "const {tmp} = require({id:?}); const {def} = ({tmp} && {tmp}.__esModule && {tmp}.default !== undefined) ? {tmp}.default : ({tmp} && {tmp}.default !== undefined ? {tmp}.default : {tmp});",
                            tmp = tmp_var,
                            id = resolved_id,
                            def = def_name
                        ));
                    }
                } else if !named_specs.is_empty() {
                    let fields: Vec<String> = named_specs
                        .iter()
                        .map(|(imp_n, loc_n)| {
                            if imp_n == loc_n {
                                imp_n.clone()
                            } else {
                                format!("{}: {}", imp_n, loc_n)
                            }
                        })
                        .collect();
                    body_lines.push(format!(
                        "const {{ {} }} = require({:?});",
                        fields.join(", "),
                        resolved_id
                    ));
                }
            }
            Statement::ExportAllDeclaration(exp_all) => {
                if exp_all.export_kind.is_type() {
                    continue;
                }
                let spec = exp_all.source.value.as_str();
                let resolved_id = resolve_spec_and_record(spec, from_dir, deps_out);
                if let Some(exported) = &exp_all.exported {
                    body_lines.push(format!(
                        "exports.{} = require({:?});",
                        exported.name(),
                        resolved_id
                    ));
                } else {
                    body_lines.push(format!(
                        "Object.assign(exports, require({:?}));",
                        resolved_id
                    ));
                }
            }
            Statement::ExportDefaultDeclaration(exp_def) => {
                has_default_export = true;
                match &exp_def.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
                        let fn_src = rewrite_cjs_requires_and_crypto(
                            &js_code[func.span.start as usize..func.span.end as usize],
                            from_dir,
                            deps_out,
                        );
                        if let Some(id) = &func.id {
                            body_lines.push(fn_src);
                            body_lines.push(format!("const __slice_default = {};", id.name));
                        } else {
                            body_lines.push(format!("const __slice_default = {};", fn_src));
                        }
                    }
                    ExportDefaultDeclarationKind::ClassDeclaration(cls) => {
                        let cls_src = rewrite_cjs_requires_and_crypto(
                            &js_code[cls.span.start as usize..cls.span.end as usize],
                            from_dir,
                            deps_out,
                        );
                        if let Some(id) = &cls.id {
                            body_lines.push(cls_src);
                            body_lines.push(format!("const __slice_default = {};", id.name));
                        } else {
                            body_lines.push(format!("const __slice_default = {};", cls_src));
                        }
                    }
                    ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => {
                        has_default_export = false;
                    }
                    expr => {
                        let sp = expr.span();
                        let expr_src = rewrite_cjs_requires_and_crypto(
                            &js_code[sp.start as usize..sp.end as usize],
                            from_dir,
                            deps_out,
                        );
                        body_lines.push(format!("const __slice_default = {};", expr_src));
                    }
                }
            }
            Statement::ExportDeclaration(exp_decl) => {
                let decl = &exp_decl.declaration;
                let sp = decl.span();
                let decl_src = rewrite_cjs_requires_and_crypto(
                    &js_code[sp.start as usize..sp.end as usize],
                    from_dir,
                    deps_out,
                );
                match decl {
                    Declaration::VariableDeclaration(vd) => {
                        body_lines.push(decl_src);
                        for d in &vd.declarations {
                            collect_binding_pattern_names(&d.id, &mut deferred_exports);
                        }
                    }
                    Declaration::FunctionDeclaration(fd) => {
                        body_lines.push(decl_src);
                        if let Some(id) = &fd.id {
                            deferred_exports.push((id.name.to_string(), id.name.to_string()));
                        }
                    }
                    Declaration::ClassDeclaration(cd) => {
                        body_lines.push(decl_src);
                        if let Some(id) = &cd.id {
                            deferred_exports.push((id.name.to_string(), id.name.to_string()));
                        }
                    }
                    _ => {}
                }
            }
            Statement::ExportNamedDeclaration(exp_named) => {
                if exp_named.export_kind.is_type() {
                    continue;
                }
                for spec in &exp_named.specifiers {
                    if spec.export_kind.is_type() {
                        continue;
                    }
                    let local_n = spec.local.name().to_string();
                    let exported_n = spec.exported.name().to_string();
                    if exported_n == "default" {
                        has_default_export = true;
                        body_lines.push(format!("const __slice_default = {};", local_n));
                    } else {
                        deferred_exports.push((exported_n, local_n));
                    }
                }
            }
            Statement::ExportFromDeclaration(exp_from) => {
                if exp_from.export_kind.is_type() {
                    continue;
                }
                let spec_str = exp_from.source.value.as_str();
                let resolved_id = resolve_spec_and_record(spec_str, from_dir, deps_out);
                req_counter += 1;
                let tmp_var = format!("__reexp_{}", req_counter);
                let mut has_runtime_spec = false;
                let mut reexp_lines = Vec::new();
                for spec in &exp_from.specifiers {
                    if spec.export_kind.is_type() {
                        continue;
                    }
                    has_runtime_spec = true;
                    let orig_n = spec.local.name();
                    let exp_n = spec.exported.name();
                    if orig_n == "default" && exp_n == "default" {
                        has_default_export = true;
                        reexp_lines.push(format!(
                            "const __slice_default = ({tmp} && {tmp}.__esModule && {tmp}.default !== undefined) ? {tmp}.default : ({tmp} && {tmp}.default !== undefined ? {tmp}.default : {tmp});",
                            tmp = tmp_var
                        ));
                    } else if orig_n == "default" {
                        reexp_lines.push(format!(
                            "Object.defineProperty(exports, {exp:?}, {{ enumerable: true, configurable: true, get: function() {{ return ({tmp} && {tmp}.default !== undefined) ? {tmp}.default : {tmp}; }} }});",
                            exp = exp_n.as_str(),
                            tmp = tmp_var
                        ));
                    } else {
                        reexp_lines.push(format!(
                            "Object.defineProperty(exports, {exp:?}, {{ enumerable: true, configurable: true, get: function() {{ return {tmp}[{orig:?}]; }} }});",
                            exp = exp_n.as_str(),
                            tmp = tmp_var,
                            orig = orig_n.as_str()
                        ));
                    }
                }
                if has_runtime_spec {
                    body_lines.push(format!("const {} = require({:?});", tmp_var, resolved_id));
                    body_lines.extend(reexp_lines);
                }
            }
            other => {
                let sp = other.span();
                let raw_stmt = &js_code[sp.start as usize..sp.end as usize];
                body_lines.push(rewrite_cjs_requires_and_crypto(
                    raw_stmt, from_dir, deps_out,
                ));
            }
        }
    }

    let uses_crypto_obj = body_lines.iter().any(|l| l.contains("crypto."));
    if !uses_crypto_obj {
        body_lines.retain(|l| l.trim() != "const crypto = require(\"node:crypto\");");
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

    Some(body_lines.join("\n"))
}

pub(crate) fn transform_module_to_cjs(
    js_code: &str,
    from_dir: &Path,
    deps_out: &mut Vec<PathBuf>,
) -> String {
    if let Some(ast_transformed) = try_transform_esm_to_cjs_ast(js_code, from_dir, deps_out) {
        return ast_transformed;
    }

    let collapsed = collapse_multiline_imports_exports(js_code);
    let mut body_lines = Vec::new();
    let mut deferred_exports: Vec<(String, String)> = Vec::new();
    let mut has_default_export = false;

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
                body_lines.push(format!("require({:?});", resolved_id));
                continue;
            }
            if let Some((clause, from_part)) = after_import.split_once(" from ") {
                if let Some((spec, _)) = extract_quoted_specifier(from_part) {
                    let resolved_id = resolve_spec_and_record(&spec, from_dir, deps_out);
                    let clause = clause.trim();
                    if let Some(ns_name) = clause.strip_prefix("* as ") {
                        let ns = ns_name.trim();
                        body_lines.push(format!("const {} = require({:?});", ns, resolved_id));
                    } else if clause.starts_with('{') && clause.ends_with('}') {
                        let inner = &clause[1..clause.len() - 1];
                        let mut fields = Vec::new();
                        for part in inner.split(',') {
                            let p = part.trim();
                            if p.is_empty() || p.starts_with("type ") {
                                continue;
                            }
                            if let Some((orig, alias)) = p.split_once(" as ") {
                                fields.push(format!("{}: {}", orig.trim(), alias.trim()));
                            } else {
                                fields.push(p.to_string());
                            }
                        }
                        if !fields.is_empty() {
                            body_lines.push(format!(
                                "const {{ {} }} = require({:?});",
                                fields.join(", "),
                                resolved_id
                            ));
                        }
                    } else {
                        let def_name = clause.trim();
                        body_lines.push(format!(
                            "const __imp_{} = require({:?}); const {} = (__imp_{} && __imp_{}.__esModule && __imp_{}.default !== undefined) ? __imp_{}.default : __imp_{};",
                            def_name, resolved_id, def_name, def_name, def_name, def_name, def_name, def_name
                        ));
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
                    let resolved_id = resolve_spec_and_record(&spec, from_dir, deps_out);
                    body_lines.push(format!("Object.assign(exports, require({:?}));", resolved_id));
                    continue;
                }
            }
            if after_export.starts_with('{') {
                if let Some((brace_part, from_part)) = after_export.split_once(" from ") {
                    if let Some((spec, _)) = extract_quoted_specifier(from_part) {
                        let resolved_id = resolve_spec_and_record(&spec, from_dir, deps_out);
                        let inner = brace_part
                            .trim()
                            .trim_start_matches('{')
                            .trim_end_matches('}');
                        body_lines.push(format!("const __reexp = require({:?});", resolved_id));
                        for item in inner.split(',') {
                            let item = item.trim();
                            if item.is_empty() || item.starts_with("type ") {
                                continue;
                            }
                            if let Some((orig, alias)) = item.split_once(" as ") {
                                body_lines.push(format!(
                                    "exports.{} = __reexp.{};",
                                    alias.trim(),
                                    orig.trim()
                                ));
                            } else {
                                body_lines.push(format!("exports.{} = __reexp.{};", item, item));
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
                            deferred_exports
                                .push((alias.trim().to_string(), orig.trim().to_string()));
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
            for kw in ["async function ", "function ", "class "] {
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
            if after_export.starts_with("async function ")
                || after_export.starts_with("function ")
                || after_export.starts_with("class ")
            {
                continue;
            }
        }

        let mut rewritten_line = rewrite_cjs_requires_in_line(raw_line, from_dir, deps_out);
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
        body_lines.push(rewritten_line);
    }

    let uses_crypto_obj = body_lines.iter().any(|l| l.contains("crypto."));
    if !uses_crypto_obj {
        body_lines.retain(|l| l.trim() != "const crypto = require(\"node:crypto\");");
    }

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
        let mut to_remove_indices = FxHashSet::default();
        let mut shorthand_replacements: FxHashMap<usize, String> = FxHashMap::default();

        for (idx, line) in body_lines.iter().enumerate() {
            let t = line.trim();
            if let Some(ident) = t.strip_suffix(',') {
                let ident = ident.trim();
                if let Some((decl_idx, req_expr)) = const_req_map.get(ident) {
                    let used_elsewhere =
                        body_lines
                            .iter()
                            .enumerate()
                            .any(|(other_idx, other_line)| {
                                other_idx != idx
                                    && other_idx != *decl_idx
                                    && other_line.contains(ident)
                            });
                    if !used_elsewhere {
                        to_remove_indices.insert(*decl_idx);
                        shorthand_replacements.insert(
                            idx,
                            format!("  get {}() {{ return {}; }},", ident, req_expr),
                        );
                    }
                }
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

