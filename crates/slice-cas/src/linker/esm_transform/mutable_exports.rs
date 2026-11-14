//! Transitive Live-Binding Export Analyzer (`get_module_mutable_exports`).

use crate::bundler::resolve_module_specifier;
use crate::linker::builtins_and_ids::MUTABLE_EXPORTS_CACHE;
use crate::linker::{
    collect_binding_pattern_names, collect_mutated_binding_names, is_builtin_module,
};
use rustc_hash::FxHashSet;
use std::path::Path;

pub(super) fn get_module_mutable_exports(path: &Path) -> FxHashSet<String> {
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

    let is_jsx_ext =
        matches!(ext, "tsx" | "jsx") || memchr::memmem::find(raw_src.as_bytes(), b"@jsx").is_some();

    let allocator = oxc::allocator::Allocator::default();
    let primary_source_type = if is_jsx_ext {
        oxc::span::SourceType::tsx().with_module(true)
    } else {
        oxc::span::SourceType::ts().with_module(true)
    };
    let mut parser_ret =
        oxc::parser::Parser::new(&allocator, &raw_src, primary_source_type).parse();
    if !parser_ret.diagnostics.is_empty() && !is_jsx_ext {
        let retry_ret = oxc::parser::Parser::new(
            &allocator,
            &raw_src,
            oxc::span::SourceType::tsx().with_module(true),
        )
        .parse();
        if retry_ret.diagnostics.is_empty() {
            parser_ret = retry_ret;
        }
    }
    if !parser_ret.diagnostics.is_empty() {
        return FxHashSet::default();
    }

    let mutated_locals = collect_mutated_binding_names(&parser_ret.program);
    let mut mutable_exported_names = FxHashSet::default();
    let from_dir = path.parent().unwrap_or_else(|| Path::new("."));

    use oxc::ast::ast::{Declaration, ExportDefaultDeclarationKind, Statement};
    for stmt in &parser_ret.program.body {
        match stmt {
            Statement::ExportDeclaration(exp_decl) if !mutated_locals.is_empty() => {
                match &exp_decl.declaration {
                    Declaration::VariableDeclaration(vd) if !vd.declare => {
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
                    Declaration::FunctionDeclaration(fd) => {
                        if let Some(id) = &fd.id {
                            if mutated_locals.contains(id.name.as_str()) {
                                mutable_exported_names.insert(id.name.to_string());
                            }
                        }
                    }
                    Declaration::ClassDeclaration(cd) => {
                        if let Some(id) = &cd.id {
                            if mutated_locals.contains(id.name.as_str()) {
                                mutable_exported_names.insert(id.name.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
            Statement::ExportDefaultDeclaration(exp_def) if !mutated_locals.is_empty() => {
                match &exp_def.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(fd) => {
                        if let Some(id) = &fd.id {
                            if mutated_locals.contains(id.name.as_str()) {
                                mutable_exported_names.insert("default".to_string());
                            }
                        }
                    }
                    ExportDefaultDeclarationKind::ClassDeclaration(cd) => {
                        if let Some(id) = &cd.id {
                            if mutated_locals.contains(id.name.as_str()) {
                                mutable_exported_names.insert("default".to_string());
                            }
                        }
                    }
                    _ => {}
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
                        for name in inner_mut {
                            if name != "default" {
                                mutable_exported_names.insert(name);
                            }
                        }
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
