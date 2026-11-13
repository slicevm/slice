//! Oxc AST-Driven ESM-to-CommonJS Transformer (`transform_module_to_cjs`)
//! and Barrel Lazy-Getter Optimizer (`apply_cjs_lazy_barrel_getters`).

use crate::bundler::resolve_module_specifier;
use crate::compiler::strip_typescript_raw_with_ext;
use crate::linker::builtins_and_ids::MUTABLE_EXPORTS_CACHE;
use crate::linker::{
    collect_binding_pattern_names, collect_live_import_reference_edits,
    collect_mutated_binding_names, collect_require_edits_from_program, format_exports_lhs,
    is_builtin_module, is_jsx_runtime_builtin, is_valid_js_identifier, resolve_spec_and_record,
    rewrite_cjs_requires_ast, slice_with_edits,
};
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};

fn get_module_mutable_exports(path: &Path) -> FxHashSet<String> {
    if let Some(cached) = MUTABLE_EXPORTS_CACHE.with(|c| c.borrow().get(path).cloned()) {
        return cached;
    }
    // Insert empty sentinel first to break any cyclic `export ... from` chains
    MUTABLE_EXPORTS_CACHE.with(|c| {
        c.borrow_mut()
            .insert(path.to_path_buf(), FxHashSet::default());
    });

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if matches!(ext, "json" | "wasm") {
        return FxHashSet::default();
    }
    let Ok(raw_src) = std::fs::read_to_string(path) else {
        return FxHashSet::default();
    };
    if !raw_src.contains("export") {
        return FxHashSet::default();
    }

    let is_ts = matches!(ext, "ts" | "tsx" | "jsx" | "mts" | "cts");
    let is_jsx_ext = matches!(ext, "tsx" | "jsx");
    let js_code = strip_typescript_raw_with_ext(&raw_src, is_ts, is_jsx_ext);

    let allocator = oxc::allocator::Allocator::default();
    let source_type = oxc::span::SourceType::ts().with_module(true);
    let parser_ret = oxc::parser::Parser::new(&allocator, &js_code, source_type).parse();
    if !parser_ret.diagnostics.is_empty() {
        return FxHashSet::default();
    }

    let mutated_locals = collect_mutated_binding_names(&parser_ret.program);
    let mut mutable_exported_names = FxHashSet::default();
    let from_dir = path.parent().unwrap_or_else(|| Path::new("."));

    use oxc::ast::ast::{Declaration, Statement};
    for stmt in &parser_ret.program.body {
        match stmt {
            Statement::ExportDeclaration(exp_decl) if !mutated_locals.is_empty() => {
                if let Declaration::VariableDeclaration(vd) = &exp_decl.declaration {
                    let mut pairs = Vec::new();
                    for d in &vd.declarations {
                        collect_binding_pattern_names(&d.id, &mut pairs);
                    }
                    for (exp_n, loc_n) in pairs {
                        if mutated_locals.contains(&loc_n) {
                            mutable_exported_names.insert(exp_n);
                        }
                    }
                }
            }
            Statement::ExportNamedDeclaration(exp_named)
                if !mutated_locals.is_empty() && !exp_named.export_kind.is_type() =>
            {
                for spec in &exp_named.specifiers {
                    if spec.export_kind.is_type() {
                        continue;
                    }
                    let loc_n = spec.local.name().as_str();
                    if mutated_locals.contains(loc_n) {
                        mutable_exported_names.insert(spec.exported.name().to_string());
                    }
                }
            }
            Statement::ExportFromDeclaration(exp_from) if !exp_from.export_kind.is_type() => {
                let spec_str = exp_from.source.value.as_str();
                if !is_builtin_module(spec_str) {
                    if let Some(inner_path) = resolve_module_specifier(from_dir, spec_str) {
                        let inner_mut = get_module_mutable_exports(&inner_path);
                        if !inner_mut.is_empty() {
                            for spec in &exp_from.specifiers {
                                if !spec.export_kind.is_type()
                                    && inner_mut.contains(spec.local.name().as_str())
                                {
                                    mutable_exported_names.insert(spec.exported.name().to_string());
                                }
                            }
                        }
                    }
                }
            }
            Statement::ExportAllDeclaration(exp_all)
                if !exp_all.export_kind.is_type() && exp_all.exported.is_none() =>
            {
                let spec_str = exp_all.source.value.as_str();
                if !is_builtin_module(spec_str) {
                    if let Some(inner_path) = resolve_module_specifier(from_dir, spec_str) {
                        let inner_mut = get_module_mutable_exports(&inner_path);
                        mutable_exported_names.extend(inner_mut);
                    }
                }
            }
            _ => {}
        }
    }

    MUTABLE_EXPORTS_CACHE.with(|c| {
        c.borrow_mut()
            .insert(path.to_path_buf(), mutable_exported_names.clone());
    });
    mutable_exported_names
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

fn apply_cjs_lazy_barrel_getters(mut body_lines: Vec<String>) -> Vec<String> {
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
                                    format!("  get {}() {{ return {}; }},", ident, req_expr),
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

fn try_transform_esm_to_cjs_ast(
    js_code: &str,
    from_dir: &Path,
    module_path: Option<&Path>,
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

    // Single-pass `require(...)`, dynamic `import(...)`, and `import.meta` AST edit collection:
    let mut req_edits = collect_require_edits_from_program(
        js_code,
        &parser_ret.program,
        from_dir,
        module_path,
        deps_out,
    );

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
        let rewritten_cjs = slice_with_edits(js_code, 0, js_code.len(), &req_edits);
        if !rewritten_cjs.contains("module.exports = {") {
            return Some(rewritten_cjs);
        }
        let body_lines: Vec<String> = rewritten_cjs
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.to_string())
            .collect();
        return Some(apply_cjs_lazy_barrel_getters(body_lines).join("\n"));
    }

    // Detect any imported bindings that point to mutable ESM live bindings in their source module:
    let mut live_import_map: FxHashMap<String, String> = FxHashMap::default();
    let mut live_var_by_stmt_start: FxHashMap<u32, String> = FxHashMap::default();
    let mut live_counter = 0usize;
    for stmt in &parser_ret.program.body {
        let Statement::ImportDeclaration(imp) = stmt else {
            continue;
        };
        if imp.import_kind.is_type() {
            continue;
        }
        let Some(specs) = &imp.specifiers else {
            continue;
        };
        let has_named = specs
            .iter()
            .any(|s| matches!(s, ImportDeclarationSpecifier::ImportSpecifier(n) if !n.import_kind.is_type()));
        if !has_named {
            continue;
        }
        let spec = imp.source.value.as_str();
        if is_builtin_module(spec) || is_jsx_runtime_builtin(spec) {
            continue;
        }
        let Some(dep_path) = resolve_module_specifier(from_dir, spec) else {
            continue;
        };
        let mut_exports = get_module_mutable_exports(&dep_path);
        if mut_exports.is_empty() {
            continue;
        }
        let mut live_var: Option<String> = None;
        for s in specs {
            if let ImportDeclarationSpecifier::ImportSpecifier(named) = s {
                if named.import_kind.is_type() {
                    continue;
                }
                let imp_n = named.imported.name().as_str();
                if mut_exports.contains(imp_n) {
                    let var_name = live_var.get_or_insert_with(|| {
                        live_counter += 1;
                        format!("__live_{}", live_counter)
                    });
                    let access = if is_valid_js_identifier(imp_n) {
                        format!("{}.{}", var_name, imp_n)
                    } else {
                        format!("{}[{:?}]", var_name, imp_n)
                    };
                    live_import_map.insert(named.local.name.to_string(), access);
                }
            }
        }
        if let Some(var_name) = live_var {
            live_var_by_stmt_start.insert(imp.span.start, var_name);
        }
    }

    if !live_import_map.is_empty() {
        collect_live_import_reference_edits(&parser_ret.program, &live_import_map, &mut req_edits);
        req_edits.sort_by_key(|e| e.0);
    }

    let has_crypto_spec_import = parser_ret.program.body.iter().any(|stmt| {
        if let Statement::ImportDeclaration(imp) = stmt {
            !imp.import_kind.is_type()
                && matches!(imp.source.value.as_str(), "crypto" | "node:crypto")
                && imp.specifiers.as_ref().is_some_and(|s| !s.is_empty())
        } else {
            false
        }
    });
    let crypto_scoping = if has_crypto_spec_import {
        Some(
            oxc::semantic::SemanticBuilder::new()
                .build(&parser_ret.program)
                .semantic
                .into_scoping(),
        )
    } else {
        None
    };

    let mut top_level_fn_names: FxHashSet<String> = FxHashSet::default();
    for stmt in &parser_ret.program.body {
        match stmt {
            Statement::FunctionDeclaration(fd) => {
                if let Some(id) = &fd.id {
                    top_level_fn_names.insert(id.name.to_string());
                }
            }
            Statement::ExportDeclaration(exp_decl) => {
                if let Declaration::FunctionDeclaration(fd) = &exp_decl.declaration {
                    if let Some(id) = &fd.id {
                        top_level_fn_names.insert(id.name.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    let mut body_lines = Vec::with_capacity(parser_ret.program.body.len() + 4);
    let mut hoisted_exports: Vec<(String, String)> = Vec::new();
    let mut deferred_exports: Vec<(String, String)> = Vec::new();
    let mut live_getter_exports: Vec<(String, String)> = Vec::new();
    let mut has_default_export = false;
    let mut has_any_esm_export = false;
    let mut req_counter = 0usize;

    for stmt in &parser_ret.program.body {
        match stmt {
            Statement::ImportDeclaration(imp) => {
                if imp.import_kind.is_type() {
                    continue;
                }
                let spec = imp.source.value.as_str();
                if matches!(spec, "crypto" | "node:crypto") {
                    if let (Some(scoping), Some(specs)) = (&crypto_scoping, &imp.specifiers) {
                        if !specs.is_empty() {
                            let any_used = specs.iter().any(|s| match s {
                                ImportDeclarationSpecifier::ImportDefaultSpecifier(d) => {
                                    d.local.symbol_id.get().is_none_or(|id| {
                                        !scoping.get_resolved_reference_ids(id).is_empty()
                                    })
                                }
                                ImportDeclarationSpecifier::ImportNamespaceSpecifier(n) => {
                                    n.local.symbol_id.get().is_none_or(|id| {
                                        !scoping.get_resolved_reference_ids(id).is_empty()
                                    })
                                }
                                ImportDeclarationSpecifier::ImportSpecifier(n) => {
                                    !n.import_kind.is_type()
                                        && n.local.symbol_id.get().is_none_or(|id| {
                                            !scoping.get_resolved_reference_ids(id).is_empty()
                                        })
                                }
                            });
                            if !any_used {
                                continue;
                            }
                        }
                    }
                }
                let resolved_id = resolve_spec_and_record(spec, from_dir, deps_out);
                let Some(specs) = &imp.specifiers else {
                    body_lines.push(format!("require({:?});", resolved_id));
                    continue;
                };
                if specs.is_empty() {
                    body_lines.push(format!("require({:?});", resolved_id));
                    continue;
                }
                if let Some(live_var) = live_var_by_stmt_start.get(&imp.span.start) {
                    body_lines.push(format!("const {} = require({:?});", live_var, resolved_id));
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
                            let loc_s = named.local.name.as_str();
                            if live_import_map.contains_key(loc_s) {
                                continue;
                            }
                            named_specs
                                .push((named.imported.name().to_string(), loc_s.to_string()));
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
                                } else if is_valid_js_identifier(imp_n) {
                                    format!("{}: {}", imp_n, loc_n)
                                } else {
                                    format!("{:?}: {}", imp_n, loc_n)
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
                            } else if is_valid_js_identifier(imp_n) {
                                format!("{}: {}", imp_n, loc_n)
                            } else {
                                format!("{:?}: {}", imp_n, loc_n)
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
                has_any_esm_export = true;
                let spec = exp_all.source.value.as_str();
                let resolved_id = resolve_spec_and_record(spec, from_dir, deps_out);
                if let Some(exported) = &exp_all.exported {
                    let lhs = format_exports_lhs(exported.name().as_str());
                    body_lines.push(format!("{} = require({:?});", lhs, resolved_id));
                } else {
                    let target_has_mutable = !is_builtin_module(spec)
                        && resolve_module_specifier(from_dir, spec)
                            .is_some_and(|p| !get_module_mutable_exports(&p).is_empty());
                    if target_has_mutable {
                        req_counter += 1;
                        let tmp_var = format!("__reexp_{}", req_counter);
                        body_lines.push(format!(
                            "const {tmp} = require({id:?}); Object.keys({tmp}).forEach(function(__k) {{ if (__k !== \"default\" && __k !== \"__esModule\") Object.defineProperty(exports, __k, {{ enumerable: true, configurable: true, get: function() {{ return {tmp}[__k]; }} }}); }});",
                            tmp = tmp_var,
                            id = resolved_id
                        ));
                    } else {
                        body_lines.push(format!(
                            "Object.assign(exports, require({:?}));",
                            resolved_id
                        ));
                    }
                }
            }
            Statement::ExportDefaultDeclaration(exp_def) => {
                has_default_export = true;
                match &exp_def.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
                        let fn_src = slice_with_edits(
                            js_code,
                            func.span.start as usize,
                            func.span.end as usize,
                            &req_edits,
                        );
                        if let Some(id) = &func.id {
                            body_lines.push(fn_src);
                            body_lines.push(format!("const __slice_default = {};", id.name));
                        } else {
                            body_lines.push(format!("const __slice_default = {};", fn_src));
                        }
                    }
                    ExportDefaultDeclarationKind::ClassDeclaration(cls) => {
                        let cls_src = slice_with_edits(
                            js_code,
                            cls.span.start as usize,
                            cls.span.end as usize,
                            &req_edits,
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
                        let expr_src = slice_with_edits(
                            js_code,
                            sp.start as usize,
                            sp.end as usize,
                            &req_edits,
                        );
                        body_lines.push(format!("const __slice_default = {};", expr_src));
                    }
                }
            }
            Statement::ExportDeclaration(exp_decl) => {
                let decl = &exp_decl.declaration;
                let sp = decl.span();
                let decl_src =
                    slice_with_edits(js_code, sp.start as usize, sp.end as usize, &req_edits);
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
                            hoisted_exports.push((id.name.to_string(), id.name.to_string()));
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
                    if exported_n == "default"
                        && exp_named.specifiers.len() == 1
                        && deferred_exports.is_empty()
                        && hoisted_exports.is_empty()
                        && live_getter_exports.is_empty()
                    {
                        has_default_export = true;
                        body_lines.push(format!("const __slice_default = {};", local_n));
                    } else if top_level_fn_names.contains(&local_n) {
                        hoisted_exports.push((exported_n, local_n));
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
                let target_mut = if !is_builtin_module(spec_str) {
                    resolve_module_specifier(from_dir, spec_str)
                        .map(|p| get_module_mutable_exports(&p))
                        .unwrap_or_default()
                } else {
                    FxHashSet::default()
                };
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
                        deferred_exports.push((
                            exp_n.to_string(),
                            format!(
                                "(({tmp} && {tmp}.default !== undefined) ? {tmp}.default : {tmp})",
                                tmp = tmp_var
                            ),
                        ));
                    } else {
                        let rhs = if is_valid_js_identifier(orig_n.as_str()) {
                            format!("{}.{}", tmp_var, orig_n)
                        } else {
                            format!("{}[{:?}]", tmp_var, orig_n.as_str())
                        };
                        if target_mut.contains(orig_n.as_str()) {
                            live_getter_exports.push((exp_n.to_string(), rhs));
                        } else {
                            deferred_exports.push((exp_n.to_string(), rhs));
                        }
                    }
                }
                if has_runtime_spec {
                    body_lines.push(format!("const {} = require({:?});", tmp_var, resolved_id));
                    body_lines.extend(reexp_lines);
                }
            }
            other => {
                let sp = other.span();
                body_lines.push(slice_with_edits(
                    js_code,
                    sp.start as usize,
                    sp.end as usize,
                    &req_edits,
                ));
            }
        }
    }

    let has_named_exports = !hoisted_exports.is_empty()
        || !deferred_exports.is_empty()
        || !live_getter_exports.is_empty();
    let mutated_locals = if hoisted_exports.is_empty() && deferred_exports.is_empty() {
        FxHashSet::default()
    } else {
        collect_mutated_binding_names(&parser_ret.program)
    };

    let mut final_lines =
        Vec::with_capacity(hoisted_exports.len() + body_lines.len() + deferred_exports.len() + 4);
    if !hoisted_exports.is_empty() {
        final_lines.push("exports.__esModule = true;".to_string());
        for (export_name, local_name) in &hoisted_exports {
            if is_valid_js_identifier(local_name) && mutated_locals.contains(local_name) {
                final_lines.push(format!(
                    "Object.defineProperty(exports, {:?}, {{ enumerable: true, configurable: true, get: function() {{ return {}; }} }});",
                    export_name, local_name
                ));
            } else {
                let lhs = format_exports_lhs(export_name);
                final_lines.push(format!("{} = {};", lhs, local_name));
            }
        }
    }
    final_lines.extend(body_lines);

    if hoisted_exports.is_empty() && (has_default_export || has_named_exports || has_any_esm_export)
    {
        final_lines.push("exports.__esModule = true;".to_string());
    }
    for (export_name, local_name) in &deferred_exports {
        if is_valid_js_identifier(local_name) && mutated_locals.contains(local_name) {
            final_lines.push(format!(
                "Object.defineProperty(exports, {:?}, {{ enumerable: true, configurable: true, get: function() {{ return {}; }} }});",
                export_name, local_name
            ));
        } else {
            let lhs = format_exports_lhs(export_name);
            final_lines.push(format!("{} = {};", lhs, local_name));
        }
    }
    for (export_name, getter_rhs) in &live_getter_exports {
        final_lines.push(format!(
            "Object.defineProperty(exports, {:?}, {{ enumerable: true, configurable: true, get: function() {{ return {}; }} }});",
            export_name, getter_rhs
        ));
    }
    if has_default_export {
        final_lines.push("exports.default = __slice_default;".to_string());
        if !has_named_exports && !has_any_esm_export {
            final_lines.push(
                "module.exports = __slice_default; if (typeof module.exports === 'object' && module.exports !== null || typeof module.exports === 'function') { module.exports.default = __slice_default; module.exports.__esModule = true; }"
                    .to_string(),
            );
        }
    }

    Some(final_lines.join("\n"))
}

pub fn transform_module_to_cjs(
    js_code: &str,
    from_dir: &Path,
    module_path: Option<&Path>,
    deps_out: &mut Vec<PathBuf>,
) -> String {
    if let Some(ast_transformed) =
        try_transform_esm_to_cjs_ast(js_code, from_dir, module_path, deps_out)
    {
        return ast_transformed;
    }

    // Fallback CommonJS module path if top-level parse failed
    if !js_code.contains("import ") && !js_code.contains("export ") {
        let rewritten_cjs = rewrite_cjs_requires_ast(js_code, from_dir, deps_out);
        if !rewritten_cjs.contains("module.exports = {") {
            return rewritten_cjs;
        }
        let body_lines: Vec<String> = rewritten_cjs
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.to_string())
            .collect();
        return apply_cjs_lazy_barrel_getters(body_lines).join("\n");
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
                        if is_valid_js_identifier(ns) {
                            body_lines.push(format!("const {} = require({:?});", ns, resolved_id));
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
                                    fields.push(format!("{}: {}", orig, alias));
                                }
                            } else if is_valid_js_identifier(p) {
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
                        if is_valid_js_identifier(def_name) {
                            body_lines.push(format!(
                                "const __imp_{} = require({:?}); const {} = (__imp_{} && __imp_{}.__esModule && __imp_{}.default !== undefined) ? __imp_{}.default : __imp_{};",
                                def_name, resolved_id, def_name, def_name, def_name, def_name, def_name, def_name
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
                    let resolved_id = resolve_spec_and_record(&spec, from_dir, deps_out);
                    body_lines.push(format!(
                        "Object.assign(exports, require({:?}));",
                        resolved_id
                    ));
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
                                let orig = orig.trim();
                                let alias = alias.trim();
                                if is_valid_js_identifier(orig) {
                                    let lhs = format_exports_lhs(alias);
                                    body_lines.push(format!("{} = __reexp.{};", lhs, orig));
                                }
                            } else if is_valid_js_identifier(item) {
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
                body_lines.push(format!("const __slice_default = {}", def_expr));
                continue;
            }
            for kw in ["const ", "let ", "var "] {
                if let Some(decl_rest) = after_export.strip_prefix(kw) {
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

    if has_default_export || !deferred_exports.is_empty() {
        body_lines.push("exports.__esModule = true;".to_string());
    }
    for (export_name, local_name) in &deferred_exports {
        let lhs = format_exports_lhs(export_name);
        body_lines.push(format!("{} = {};", lhs, local_name));
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
