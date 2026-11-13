//! Cold Method-Scoped `require` Inliner (`inline_cold_method_requires_ast`)
//! and Eager Module Reachability Analyzer (`collect_eager_module_indices`).

use crate::linker::tree_shaker::is_any_require_call;
use crate::linker::{
    collect_mutated_binding_names, collect_top_level_cjs_reference_edits, slice_with_edits,
};
use rustc_hash::{FxHashMap, FxHashSet};

/// General-Purpose Cold Method-Scoped `require` Inliner (`inline_cold_method_requires_ast`):
/// If a `/node_modules/` module has a top-level `const { a, b } = require("<dep_id>");` where
/// `<dep_id>` is only required once in the bundle and its bindings (`a`, `b`) are never referenced
/// at module top-level, in `constructor`, or in hot-path router/request methods, inlines
/// `require("<dep_id>").a` directly into the cold method body and removes the eager top-level `require`.
pub fn inline_cold_method_requires_ast(
    cjs_code: &str,
    single_consumer_deps: &FxHashSet<String>,
) -> String {
    use oxc::ast::ast::{
        BindingPattern, ClassElement, Expression, MethodDefinitionKind, Statement,
    };
    use oxc::span::GetSpan;

    if single_consumer_deps.is_empty() {
        return cjs_code.to_string();
    }

    let allocator = oxc::allocator::Allocator::default();
    let source_type = oxc::span::SourceType::mjs().with_module(false);
    let parser_ret = oxc::parser::Parser::new(&allocator, cjs_code, source_type).parse();
    if !parser_ret.diagnostics.is_empty() {
        return cjs_code.to_string();
    }

    // Collect cold class method body spans
    let mut cold_method_spans: Vec<(usize, usize)> = Vec::new();
    let is_hot_method_name = |name: &str| {
        matches!(
            name,
            "constructor"
                | "fetch"
                | "match"
                | "add"
                | "insert"
                | "lookup"
                | "search"
                | "get"
                | "set"
                | "header"
                | "json"
                | "text"
                | "body"
                | "newResponse"
                | "param"
                | "query"
                | "queries"
                | "req"
                | "res"
                | "url"
                | "method"
                | "status"
        )
    };

    let mut collect_from_class = |cls: &oxc::ast::ast::Class<'_>| {
        for el in &cls.body.body {
            match el {
                ClassElement::MethodDefinition(md) => {
                    if md.kind == MethodDefinitionKind::Constructor {
                        continue;
                    }
                    let mname = md.key.name().unwrap_or_default();
                    if is_hot_method_name(&mname) {
                        continue;
                    }
                    if let Some(body) = &md.value.body {
                        cold_method_spans.push((body.span.start as usize, body.span.end as usize));
                    }
                }
                ClassElement::PropertyDefinition(pd) => {
                    let pname = pd.key.name().unwrap_or_default();
                    if is_hot_method_name(&pname) {
                        continue;
                    }
                    if let Some(Expression::ArrowFunctionExpression(arrow)) = &pd.value {
                        let sp = arrow.body.span();
                        cold_method_spans.push((sp.start as usize, sp.end as usize));
                    }
                }
                _ => {}
            }
        }
    };

    for stmt in &parser_ret.program.body {
        match stmt {
            Statement::ClassDeclaration(cd) => collect_from_class(cd),
            Statement::VariableDeclaration(vd) => {
                for decl in &vd.declarations {
                    if let Some(Expression::ClassExpression(ce)) = &decl.init {
                        collect_from_class(ce);
                    }
                }
            }
            _ => {}
        }
    }

    if cold_method_spans.is_empty() {
        return cjs_code.to_string();
    }

    struct CandidateDecl {
        stmt_span: (usize, usize),
        dep_id: String,
        bindings: Vec<String>,
    }

    let mut candidates: Vec<CandidateDecl> = Vec::new();
    let mut probe_map: FxHashMap<String, String> = FxHashMap::default();

    for stmt in &parser_ret.program.body {
        let Statement::VariableDeclaration(vd) = stmt else {
            continue;
        };
        if vd.declarations.len() != 1 {
            continue;
        }
        let decl = &vd.declarations[0];
        let Some(init) = &decl.init else {
            continue;
        };
        let Some(dep_id) = is_any_require_call(init) else {
            continue;
        };
        if !single_consumer_deps.contains(&dep_id) {
            continue;
        };
        let BindingPattern::ObjectPattern(obj_pat) = &decl.id else {
            continue;
        };
        if obj_pat.rest.is_some() || obj_pat.properties.is_empty() {
            continue;
        }
        let mut bindings = Vec::new();
        let mut all_simple = true;
        for prop in &obj_pat.properties {
            let Some(k) = prop.key.name() else {
                all_simple = false;
                break;
            };
            let BindingPattern::BindingIdentifier(loc) = &prop.value else {
                all_simple = false;
                break;
            };
            if k != loc.name.as_str() {
                all_simple = false;
                break;
            }
            bindings.push(loc.name.to_string());
        }
        if !all_simple {
            continue;
        }
        for b_name in &bindings {
            probe_map.insert(b_name.clone(), format!("require({:?}).{}", dep_id, b_name));
        }
        candidates.push(CandidateDecl {
            stmt_span: (vd.span.start as usize, vd.span.end as usize),
            dep_id,
            bindings,
        });
    }

    if candidates.is_empty() {
        return cjs_code.to_string();
    }

    let candidate_spans: FxHashSet<(usize, usize)> =
        candidates.iter().map(|c| c.stmt_span).collect();
    let mutated = collect_mutated_binding_names(&parser_ret.program);
    let mut all_ref_edits: Vec<(usize, usize, String)> = Vec::new();
    collect_top_level_cjs_reference_edits(
        &parser_ret.program,
        &candidate_spans,
        &probe_map,
        &mut all_ref_edits,
    );

    let mut final_edits: Vec<(usize, usize, String)> = Vec::new();
    for cand in &candidates {
        let mut decl_edits: Vec<(usize, usize, String)> = Vec::new();
        let mut eligible = true;
        for b_name in &cand.bindings {
            if mutated.contains(b_name) {
                eligible = false;
                break;
            }
            let expected_rep = format!("require({:?}).{}", cand.dep_id, b_name);
            let expected_shorthand = format!("{}: {}", b_name, expected_rep);
            let mut ref_count = 0usize;
            for (rs, re, rep) in &all_ref_edits {
                if *rep == expected_rep || *rep == expected_shorthand {
                    let in_cold_span = cold_method_spans
                        .iter()
                        .any(|&(cs, ce)| *rs >= cs && *re <= ce);
                    if !in_cold_span {
                        eligible = false;
                        break;
                    }
                    ref_count += 1;
                    decl_edits.push((*rs, *re, rep.clone()));
                }
            }
            if !eligible || ref_count == 0 || ref_count > 2 {
                eligible = false;
                break;
            }
        }
        if eligible {
            final_edits.push((cand.stmt_span.0, cand.stmt_span.1, String::new()));
            final_edits.extend(decl_edits);
        }
    }

    if final_edits.is_empty() {
        return cjs_code.to_string();
    }

    final_edits.sort_by_key(|e| e.0);
    slice_with_edits(cjs_code, 0, cjs_code.len(), &final_edits)
}

/// Computes the set of module indices `0..modules.len()` that are reachable from `entry_body`
/// via **eager** `require(idx)` edges (excluding `require(idx)` calls inside unpruned lazy
/// property getters `get prop() { ... }` or cold non-constructor class methods).
pub fn collect_eager_module_indices(
    entry_body: &str,
    modules: &[(String, String)],
    subset_pruned: &[bool],
) -> FxHashSet<usize> {
    use oxc::ast::ast::{Expression, ObjectPropertyKind, PropertyKind, Statement};
    use oxc::span::GetSpan;

    fn extract_eager_require_ids(
        code: &str,
        max_id: usize,
        keep_export_getters_eager: bool,
    ) -> Vec<usize> {
        let allocator = oxc::allocator::Allocator::default();
        let source_type = oxc::span::SourceType::mjs().with_module(false);
        let parser_ret = oxc::parser::Parser::new(&allocator, code, source_type).parse();

        let mut deferred_spans: Vec<(usize, usize)> = Vec::new();
        if parser_ret.diagnostics.is_empty() {
            fn collect_class_deferred(
                cls: &oxc::ast::ast::Class<'_>,
                deferred_spans: &mut Vec<(usize, usize)>,
            ) {
                use oxc::ast::ast::{ClassElement, Expression, MethodDefinitionKind};
                use oxc::span::GetSpan;
                for el in &cls.body.body {
                    match el {
                        ClassElement::MethodDefinition(md) => {
                            if md.kind != MethodDefinitionKind::Constructor {
                                if let Some(b) = &md.value.body {
                                    deferred_spans
                                        .push((b.span.start as usize, b.span.end as usize));
                                }
                            }
                        }
                        ClassElement::PropertyDefinition(pd) => {
                            if let Some(Expression::ArrowFunctionExpression(arrow)) = &pd.value {
                                let sp = arrow.body.span();
                                deferred_spans.push((sp.start as usize, sp.end as usize));
                            }
                        }
                        _ => {}
                    }
                }
            }

            for stmt in &parser_ret.program.body {
                match stmt {
                    Statement::ClassDeclaration(cd) => {
                        collect_class_deferred(cd, &mut deferred_spans)
                    }
                    Statement::FunctionDeclaration(fd) => {
                        if let (Some(id), Some(body)) = (&fd.id, &fd.body) {
                            let call_pat = format!("{}(", id.name);
                            let start = fd.span.start as usize;
                            let end = fd.span.end as usize;
                            if !code[..start].contains(&call_pat)
                                && !code[end..].contains(&call_pat)
                            {
                                deferred_spans
                                    .push((body.span.start as usize, body.span.end as usize));
                            }
                        }
                    }
                    Statement::VariableDeclaration(vd) => {
                        for decl in &vd.declarations {
                            if let Some(Expression::ClassExpression(ce)) = &decl.init {
                                collect_class_deferred(ce, &mut deferred_spans);
                            } else if matches!(
                                &decl.init,
                                Some(
                                    Expression::FunctionExpression(_)
                                        | Expression::ArrowFunctionExpression(_)
                                )
                            ) {
                                if let oxc::ast::ast::BindingPattern::BindingIdentifier(id) =
                                    &decl.id
                                {
                                    let call_pat = format!("{}(", id.name);
                                    let start = decl.span.start as usize;
                                    let end = decl.span.end as usize;
                                    if !code[..start].contains(&call_pat)
                                        && !code[end..].contains(&call_pat)
                                    {
                                        deferred_spans.push((start, end));
                                    }
                                }
                            }
                        }
                    }
                    Statement::ExpressionStatement(es) if !keep_export_getters_eager => {
                        if let Expression::AssignmentExpression(assign) =
                            es.expression.without_parentheses()
                        {
                            if let Expression::ObjectExpression(obj) =
                                assign.right.without_parentheses()
                            {
                                for prop in &obj.properties {
                                    if let ObjectPropertyKind::ObjectProperty(p) = prop {
                                        if p.kind == PropertyKind::Get {
                                            let sp = p.value.span();
                                            deferred_spans
                                                .push((sp.start as usize, sp.end as usize));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        let mut out = Vec::new();
        let bytes = code.as_bytes();
        let finder = memchr::memmem::Finder::new(b"require(");
        let mut pos = 0usize;
        while let Some(rel) = finder.find(&bytes[pos..]) {
            let idx = pos + rel;
            let after_start = idx + 8;
            pos = after_start;
            let in_deferred = deferred_spans.iter().any(|&(s, e)| idx >= s && idx < e);
            if in_deferred {
                continue;
            }
            let after_bytes = &bytes[after_start..];
            let digit_len = after_bytes
                .iter()
                .take_while(|&&b| b.is_ascii_digit())
                .count();
            if digit_len > 0 && after_bytes.get(digit_len) == Some(&b')') {
                if let Ok(id) = code[after_start..after_start + digit_len].parse::<usize>() {
                    if id < max_id {
                        out.push(id);
                    }
                }
            }
        }
        out
    }

    let max_id = modules.len();
    let mut eager_set = FxHashSet::default();
    let mut queue = extract_eager_require_ids(entry_body, max_id, false);

    while let Some(idx) = queue.pop() {
        if eager_set.insert(idx) {
            let keep_getters = subset_pruned.get(idx).copied().unwrap_or(false);
            for child in extract_eager_require_ids(&modules[idx].1, max_id, keep_getters) {
                if !eager_set.contains(&child) {
                    queue.push(child);
                }
            }
        }
    }

    eager_set
}
