//! Cold Method-Scoped `require` Inliner (`inline_cold_method_requires_ast`)
//! and Eager Module Reachability Analyzer (`collect_eager_module_indices`).

use crate::linker::tree_shaker::{is_any_require_call, is_js_ident_continue};
use rustc_hash::FxHashSet;

fn replace_whole_ident_in_slice(src: &str, ident: &str, replacement: &str) -> String {
    let bytes = src.as_bytes();
    let ibytes = ident.as_bytes();
    let ilen = ibytes.len();
    if ilen == 0 {
        return src.to_string();
    }
    let finder = memchr::memmem::Finder::new(ibytes);
    let mut out = String::with_capacity(src.len() + 64);
    let mut last = 0usize;
    let mut pos = 0usize;
    while let Some(rel) = finder.find(&bytes[pos..]) {
        let idx = pos + rel;
        let end = idx + ilen;
        let valid_start =
            idx == 0 || (!is_js_ident_continue(bytes[idx - 1]) && bytes[idx - 1] != b'.');
        let valid_end = end == bytes.len() || !is_js_ident_continue(bytes[end]);
        if valid_start && valid_end {
            out.push_str(&src[last..idx]);
            out.push_str(replacement);
            last = end;
        }
        pos = end;
    }
    out.push_str(&src[last..]);
    out
}

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

    // Collect cold class method body spans and non-computed class element key spans
    let mut cold_method_spans: Vec<(usize, usize)> = Vec::new();
    let mut non_computed_key_spans: Vec<(usize, usize)> = Vec::new();
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
                    if !md.computed {
                        let ksp = md.key.span();
                        non_computed_key_spans.push((ksp.start as usize, ksp.end as usize));
                    }
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
                    if !pd.computed {
                        let ksp = pd.key.span();
                        non_computed_key_spans.push((ksp.start as usize, ksp.end as usize));
                    }
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

    let mut stmt_spans_to_remove: Vec<(usize, usize)> = Vec::new();
    let mut inline_bindings: Vec<(String, String)> = Vec::new();

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
        }
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

        let decl_end = vd.span.end as usize;
        let bytes = cjs_code.as_bytes();
        let mut eligible = true;
        for b_name in &bindings {
            let b_len = b_name.len();
            let finder = memchr::memmem::Finder::new(b_name.as_bytes());
            let mut pos = decl_end;
            let mut ref_count = 0usize;
            while let Some(rel) = finder.find(&bytes[pos..]) {
                let idx = pos + rel;
                let end = idx + b_len;
                pos = end;
                let valid_start =
                    idx == 0 || (!is_js_ident_continue(bytes[idx - 1]) && bytes[idx - 1] != b'.');
                let valid_end = end == bytes.len() || !is_js_ident_continue(bytes[end]);
                if !valid_start || !valid_end {
                    continue;
                }
                if non_computed_key_spans
                    .iter()
                    .any(|&(ks, ke)| idx >= ks && end <= ke)
                {
                    continue;
                }
                let in_cold_span = cold_method_spans.iter().any(|&(s, e)| idx >= s && end <= e);
                if !in_cold_span {
                    eligible = false;
                    break;
                }
                ref_count += 1;
            }
            if !eligible || ref_count == 0 || ref_count > 2 {
                eligible = false;
                break;
            }
        }

        if eligible {
            stmt_spans_to_remove.push((vd.span.start as usize, vd.span.end as usize));
            for b_name in bindings {
                inline_bindings.push((b_name.clone(), format!("require({:?}).{}", dep_id, b_name)));
            }
        }
    }

    if stmt_spans_to_remove.is_empty() {
        return cjs_code.to_string();
    }

    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for (s, e) in &stmt_spans_to_remove {
        edits.push((*s, *e, String::new()));
    }
    for &(ms, me) in &cold_method_spans {
        let orig_slice = &cjs_code[ms..me];
        let mut updated = orig_slice.to_string();
        for (b_name, rep_expr) in &inline_bindings {
            if updated.contains(b_name.as_str()) {
                updated = replace_whole_ident_in_slice(&updated, b_name, rep_expr);
            }
        }
        if updated != orig_slice {
            edits.push((ms, me, updated));
        }
    }
    edits.sort_by_key(|e| e.0);

    let mut result = String::with_capacity(cjs_code.len() + 64);
    let mut cursor = 0usize;
    for (s, e, rep) in edits {
        if s < cursor {
            continue;
        }
        result.push_str(&cjs_code[cursor..s]);
        result.push_str(&rep);
        cursor = e;
    }
    result.push_str(&cjs_code[cursor..]);
    result
}

/// Computes the set of module indices `0..modules.len()` that are reachable from `entry_body`
/// via **eager** `require(idx)` edges (excluding `require(idx)` calls inside lazy property getters
/// `get prop() { ... }` or cold non-constructor class methods).
pub fn collect_eager_module_indices(
    entry_body: &str,
    modules: &[(String, String)],
) -> FxHashSet<usize> {
    use oxc::ast::ast::{Expression, ObjectPropertyKind, PropertyKind, Statement};
    use oxc::span::GetSpan;

    fn extract_eager_require_ids(code: &str, max_id: usize) -> Vec<usize> {
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
                    Statement::ExpressionStatement(es) => {
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
    let mut queue = extract_eager_require_ids(entry_body, max_id);

    while let Some(idx) = queue.pop() {
        if eager_set.insert(idx) {
            for child in extract_eager_require_ids(&modules[idx].1, max_id) {
                if !eager_set.contains(&child) {
                    queue.push(child);
                }
            }
        }
    }

    eager_set
}
