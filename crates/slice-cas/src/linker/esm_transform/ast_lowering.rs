//! Oxc AST-Driven ESM-to-CommonJS Lowering (`try_transform_esm_to_cjs_ast`).

use super::barrel_and_fallback::apply_cjs_lazy_barrel_getters;
use super::mutable_exports::get_module_mutable_exports;
use super::{
    choose_hygienic_ident, choose_hygienic_prefix, format_default_import_unwrap,
    format_namespace_import_rhs, ESM_ESMODULE_DEFINE,
};
use crate::bundler::resolve_module_specifier;
use crate::linker::{
    collect_binding_pattern_names, collect_live_import_reference_edits,
    collect_mutated_binding_names, collect_require_edits_from_program, format_exports_lhs,
    is_builtin_module, is_jsx_runtime_builtin, is_valid_js_identifier, resolve_spec_and_record,
    slice_with_edits,
};
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};

pub(super) fn try_transform_esm_to_cjs_ast(
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

    let def_var = choose_hygienic_ident(js_code, "__slice_default", "__slice", "_default");
    let imp_pfx = choose_hygienic_prefix(js_code, "__imp_", "__imp");
    let reexp_pfx = choose_hygienic_prefix(js_code, "__reexp_", "__reexp");
    let live_pfx = choose_hygienic_prefix(js_code, "__live_", "__live");

    // Detect any imported bindings (named or default) that point to mutable ESM live bindings:
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
        let has_bindable = specs.iter().any(|s| match s {
            ImportDeclarationSpecifier::ImportDefaultSpecifier(_) => true,
            ImportDeclarationSpecifier::ImportSpecifier(n) => !n.import_kind.is_type(),
            ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => false,
        });
        if !has_bindable {
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
            match s {
                ImportDeclarationSpecifier::ImportDefaultSpecifier(def) => {
                    if mut_exports.contains("default") {
                        let var_name = live_var.get_or_insert_with(|| {
                            live_counter += 1;
                            format!("{live_pfx}{live_counter}")
                        });
                        live_import_map
                            .insert(def.local.name.to_string(), format!("{var_name}.default"));
                    }
                }
                ImportDeclarationSpecifier::ImportSpecifier(named) => {
                    if named.import_kind.is_type() {
                        continue;
                    }
                    let imp_n = named.imported.name().as_str();
                    if mut_exports.contains(imp_n) {
                        let var_name = live_var.get_or_insert_with(|| {
                            live_counter += 1;
                            format!("{live_pfx}{live_counter}")
                        });
                        let access = if is_valid_js_identifier(imp_n) {
                            format!("{var_name}.{imp_n}")
                        } else {
                            format!("{var_name}[{imp_n:?}]")
                        };
                        live_import_map.insert(named.local.name.to_string(), access);
                    }
                }
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => {}
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
    let mut default_local_ident: Option<String> = None;
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
                    body_lines.push(format!("require({resolved_id:?});"));
                    continue;
                };
                if specs.is_empty() {
                    body_lines.push(format!("require({resolved_id:?});"));
                    continue;
                }
                if let Some(live_var) = live_var_by_stmt_start.get(&imp.span.start) {
                    body_lines.push(format!("const {live_var} = require({resolved_id:?});"));
                }
                let mut default_ident: Option<&str> = None;
                let mut namespace_ident: Option<&str> = None;
                let mut named_specs: Vec<(String, String)> = Vec::new();

                for s in specs {
                    match s {
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(def) => {
                            let loc_s = def.local.name.as_str();
                            if !live_import_map.contains_key(loc_s) {
                                default_ident = Some(loc_s);
                            }
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
                    let rhs = format_namespace_import_rhs(spec, &resolved_id);
                    body_lines.push(format!("const {ns} = {rhs};"));
                }
                if let Some(def_name) = default_ident {
                    req_counter += 1;
                    let tmp_var = format!("{imp_pfx}{req_counter}");
                    let unwrap_expr = format_default_import_unwrap(&tmp_var);
                    if !named_specs.is_empty() {
                        let fields = format_destructure_fields(&named_specs);
                        body_lines.push(format!(
                            "const {tmp_var} = require({resolved_id:?}); const {def_name} = {unwrap_expr}; const {{ {fields} }} = {tmp_var};"
                        ));
                    } else {
                        body_lines.push(format!(
                            "const {tmp_var} = require({resolved_id:?}); const {def_name} = {unwrap_expr};"
                        ));
                    }
                } else if !named_specs.is_empty() {
                    let fields = format_destructure_fields(&named_specs);
                    body_lines.push(format!("const {{ {fields} }} = require({resolved_id:?});"));
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
                    let rhs = format_namespace_import_rhs(spec, &resolved_id);
                    body_lines.push(format!("{lhs} = {rhs};"));
                } else {
                    let target_has_mutable = !is_builtin_module(spec)
                        && resolve_module_specifier(from_dir, spec)
                            .is_some_and(|p| !get_module_mutable_exports(&p).is_empty());
                    if target_has_mutable {
                        req_counter += 1;
                        let tmp_var = format!("{reexp_pfx}{req_counter}");
                        body_lines.push(format!(
                            "const {tmp_var} = require({resolved_id:?}); Object.keys({tmp_var}).forEach(function(__k) {{ if (__k !== \"default\" && __k !== \"__esModule\") Object.defineProperty(exports, __k, {{ enumerable: true, configurable: true, get: function() {{ return {tmp_var}[__k]; }} }}); }});"
                        ));
                    } else {
                        body_lines.push(format!(
                            "globalThis.__slice_export_star(exports, require({resolved_id:?}));"
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
                            default_local_ident = Some(id.name.to_string());
                        } else {
                            body_lines.push(format!("const {def_var} = {fn_src};"));
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
                            default_local_ident = Some(id.name.to_string());
                        } else {
                            body_lines.push(format!("const {def_var} = {cls_src};"));
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
                        body_lines.push(format!("const {def_var} = {expr_src};"));
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
                        if !vd.declare {
                            has_any_esm_export = true;
                        }
                        body_lines.push(decl_src);
                        for d in &vd.declarations {
                            collect_binding_pattern_names(&d.id, &mut deferred_exports);
                        }
                    }
                    Declaration::FunctionDeclaration(fd) => {
                        has_any_esm_export = true;
                        body_lines.push(decl_src);
                        if let Some(id) = &fd.id {
                            hoisted_exports.push((id.name.to_string(), id.name.to_string()));
                        }
                    }
                    Declaration::ClassDeclaration(cd) => {
                        has_any_esm_export = true;
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
                has_any_esm_export = true;
                for spec in &exp_named.specifiers {
                    if spec.export_kind.is_type() {
                        continue;
                    }
                    let local_n = spec.local.name().to_string();
                    let exported_n = spec.exported.name().to_string();
                    if top_level_fn_names.contains(&local_n) {
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
                has_any_esm_export = true;
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
                let tmp_var = format!("{reexp_pfx}{req_counter}");
                let mut has_runtime_spec = false;
                let mut reexp_lines = Vec::new();
                for spec in &exp_from.specifiers {
                    if spec.export_kind.is_type() {
                        continue;
                    }
                    has_runtime_spec = true;
                    let orig_n = spec.local.name();
                    let exp_n = spec.exported.name();
                    if orig_n == "default" {
                        let unwrap_expr = format!("({})", format_default_import_unwrap(&tmp_var));
                        if target_mut.contains("default") {
                            live_getter_exports.push((exp_n.to_string(), unwrap_expr));
                        } else if exp_n == "default" {
                            has_default_export = true;
                            reexp_lines.push(format!("const {def_var} = {unwrap_expr};"));
                        } else {
                            deferred_exports.push((exp_n.to_string(), unwrap_expr));
                        }
                    } else {
                        let rhs = if is_valid_js_identifier(orig_n.as_str()) {
                            format!("{tmp_var}.{orig_n}")
                        } else {
                            format!("{tmp_var}[{:?}]", orig_n.as_str())
                        };
                        if target_mut.contains(orig_n.as_str()) {
                            live_getter_exports.push((exp_n.to_string(), rhs));
                        } else {
                            deferred_exports.push((exp_n.to_string(), rhs));
                        }
                    }
                }
                if has_runtime_spec {
                    body_lines.push(format!("const {tmp_var} = require({resolved_id:?});"));
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
    let mutated_locals = if hoisted_exports.is_empty()
        && deferred_exports.is_empty()
        && default_local_ident.is_none()
    {
        FxHashSet::default()
    } else {
        collect_mutated_binding_names(&parser_ret.program)
    };

    let mut final_lines =
        Vec::with_capacity(hoisted_exports.len() + body_lines.len() + deferred_exports.len() + 4);
    if !hoisted_exports.is_empty() {
        final_lines.push(ESM_ESMODULE_DEFINE.to_string());
        for (export_name, local_name) in &hoisted_exports {
            if is_valid_js_identifier(local_name) && mutated_locals.contains(local_name) {
                final_lines.push(format!(
                    "Object.defineProperty(exports, {export_name:?}, {{ enumerable: true, configurable: true, get: function() {{ return {local_name}; }} }});"
                ));
            } else {
                let lhs = format_exports_lhs(export_name);
                final_lines.push(format!("{lhs} = {local_name};"));
            }
        }
    }
    final_lines.extend(body_lines);

    if hoisted_exports.is_empty() && (has_default_export || has_named_exports || has_any_esm_export)
    {
        final_lines.push(ESM_ESMODULE_DEFINE.to_string());
    }
    for (export_name, local_name) in &deferred_exports {
        if is_valid_js_identifier(local_name) && mutated_locals.contains(local_name) {
            final_lines.push(format!(
                "Object.defineProperty(exports, {export_name:?}, {{ enumerable: true, configurable: true, get: function() {{ return {local_name}; }} }});"
            ));
        } else {
            let lhs = format_exports_lhs(export_name);
            final_lines.push(format!("{lhs} = {local_name};"));
        }
    }
    for (export_name, getter_rhs) in &live_getter_exports {
        final_lines.push(format!(
            "Object.defineProperty(exports, {export_name:?}, {{ enumerable: true, configurable: true, get: function() {{ return {getter_rhs}; }} }});"
        ));
    }
    if has_default_export {
        if let Some(local_id) = &default_local_ident {
            if mutated_locals.contains(local_id) {
                final_lines.push(format!(
                    "Object.defineProperty(exports, \"default\", {{ enumerable: true, configurable: true, get: function() {{ return {local_id}; }} }});"
                ));
            } else {
                final_lines.push(format!("exports.default = {local_id};"));
            }
        } else {
            final_lines.push(format!("exports.default = {def_var};"));
        }
    }

    Some(final_lines.join("\n"))
}

fn format_destructure_fields(named_specs: &[(String, String)]) -> String {
    named_specs
        .iter()
        .map(|(imp_n, loc_n)| {
            if imp_n == loc_n {
                imp_n.clone()
            } else if is_valid_js_identifier(imp_n) {
                format!("{imp_n}: {loc_n}")
            } else {
                format!("{imp_n:?}: {loc_n}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}
