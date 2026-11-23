//! Single-Pass `ModuleRequireSummary` Extractor and Used-Exports Analyzer (`CAS-PERF-02`).

use super::{
    ModuleRequireSummary, UsedExports, expr_contains_assignment, is_any_require_call,
    is_js_ident_continue, is_js_ident_start,
};
use rustc_hash::FxHashSet;

fn is_to_esm_wrapped_require_call(expr: &oxc::ast::ast::Expression<'_>) -> Option<String> {
    use oxc::ast::ast::Expression;
    let Expression::CallExpression(call) = expr.without_parentheses() else {
        return None;
    };
    if call.arguments.len() != 1 {
        return None;
    }
    let is_to_esm = match call.callee.without_parentheses() {
        Expression::Identifier(id) => id.name.as_str() == "__slice_to_esm",
        Expression::StaticMemberExpression(mem) => {
            mem.property.name.as_str() == "__slice_to_esm"
                && matches!(
                    mem.object.without_parentheses(),
                    Expression::Identifier(obj) if obj.name.as_str() == "globalThis"
                )
        }
        _ => false,
    };
    if !is_to_esm {
        return None;
    }
    let inner = call.arguments.first()?.as_expression()?;
    is_any_require_call(inner)
}

fn has_ident_outside_span(code: &str, var_name: &str, allowed_span: (usize, usize)) -> bool {
    let bytes = code.as_bytes();
    let vbytes = var_name.as_bytes();
    let vlen = vbytes.len();
    if vlen == 0 {
        return false;
    }
    let finder = memchr::memmem::Finder::new(vbytes);
    let mut pos = 0usize;
    while let Some(rel) = finder.find(&bytes[pos..]) {
        let idx = pos + rel;
        let end = idx + vlen;
        pos = end;
        if idx >= allowed_span.0 && end <= allowed_span.1 {
            continue;
        }
        let valid_start =
            idx == 0 || (!is_js_ident_continue(bytes[idx - 1]) && bytes[idx - 1] != b'.');
        let valid_end = end == bytes.len() || !is_js_ident_continue(bytes[end]);
        if valid_start && valid_end {
            return true;
        }
    }
    false
}

fn try_collect_esm_default_interop_props(
    stmts: &[oxc::ast::ast::Statement<'_>],
    stmt_idx: usize,
    imp_var: &str,
    imp_stmt_start: usize,
    consumer_code: &str,
) -> Option<FxHashSet<String>> {
    use oxc::ast::ast::{BindingPattern, Expression, Statement};

    let Statement::VariableDeclaration(def_vd) = stmts.get(stmt_idx + 1)? else {
        return None;
    };
    if def_vd.declarations.len() != 1 {
        return None;
    }
    let def_decl = &def_vd.declarations[0];
    let BindingPattern::BindingIdentifier(def_ident) = &def_decl.id else {
        return None;
    };
    let def_init = def_decl.init.as_ref()?;
    let def_init_text = &consumer_code[def_vd.span.start as usize..def_vd.span.end as usize];
    if !def_init_text.contains(imp_var) || expr_contains_assignment(def_init) {
        return None;
    }

    let mut group_end = def_vd.span.end as usize;
    let mut props = FxHashSet::default();

    if let Some(Statement::VariableDeclaration(destr_vd)) = stmts.get(stmt_idx + 2)
        && destr_vd.declarations.len() == 1
    {
        let destr_decl = &destr_vd.declarations[0];
        if let Some(Expression::Identifier(id_expr)) = &destr_decl.init
            && id_expr.name.as_str() == imp_var
        {
            group_end = destr_vd.span.end as usize;
            let BindingPattern::ObjectPattern(obj_pat) = &destr_decl.id else {
                return None;
            };
            if obj_pat.rest.is_some() {
                return None;
            }
            for prop in &obj_pat.properties {
                let key_name = prop.key.name()?;
                props.insert(key_name.to_string());
            }
        }
    }

    if has_ident_outside_span(consumer_code, imp_var, (imp_stmt_start, group_end)) {
        return None;
    }

    if !collect_member_accesses_on_var(
        consumer_code,
        def_ident.name.as_str(),
        (imp_stmt_start, group_end),
        &mut props,
    ) {
        return None;
    }

    props.insert("default".to_string());
    props.insert("__esModule".to_string());
    Some(props)
}

fn collect_member_accesses_on_var(
    code: &str,
    var_name: &str,
    skip_span: (usize, usize),
    used_out: &mut FxHashSet<String>,
) -> bool {
    let bytes = code.as_bytes();
    let len = bytes.len();
    let vbytes = var_name.as_bytes();
    let vlen = vbytes.len();
    if vlen == 0 {
        return false;
    }
    let mut tpl_brace_stack: Vec<usize> = Vec::new();
    let mut i = 0usize;
    while i < len {
        let b = bytes[i];
        if b == b'/' && i + 1 < len {
            if bytes[i + 1] == b'/' {
                i += 2;
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if bytes[i + 1] == b'*' {
                i += 2;
                while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(len);
                continue;
            }
        }
        if b == b'\'' || b == b'"' {
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
        if b == b'`' || (b == b'}' && tpl_brace_stack.last() == Some(&0)) {
            if b == b'}' {
                tpl_brace_stack.pop();
            }
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(len);
                    continue;
                }
                if bytes[i] == b'`' {
                    i += 1;
                    break;
                }
                if bytes[i] == b'$' && i + 1 < len && bytes[i + 1] == b'{' {
                    tpl_brace_stack.push(0);
                    i += 2;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if b == b'{' {
            if let Some(depth) = tpl_brace_stack.last_mut() {
                *depth += 1;
            }
            i += 1;
            continue;
        }
        if b == b'}' {
            if let Some(depth) = tpl_brace_stack.last_mut() {
                *depth = depth.saturating_sub(1);
            }
            i += 1;
            continue;
        }
        if is_js_ident_start(b) {
            let start = i;
            i += 1;
            while i < len && is_js_ident_continue(bytes[i]) {
                i += 1;
            }
            let end = i;
            if end - start == vlen && &bytes[start..end] == vbytes {
                if start >= skip_span.0 && end <= skip_span.1 {
                    continue;
                }
                if start > 0 && bytes[start - 1] == b'.' {
                    continue;
                }
                let after = code[end..].trim_start();
                let Some(after_dot) = after.strip_prefix('.') else {
                    return false;
                };
                let prop_bytes = after_dot.as_bytes();
                let prop_len = prop_bytes
                    .iter()
                    .take_while(|&&pb| is_js_ident_continue(pb))
                    .count();
                if prop_len == 0 {
                    return false;
                }
                used_out.insert(after_dot[..prop_len].to_string());
            }
            continue;
        }
        i += 1;
    }
    true
}

/// Extracts a single-pass `ModuleRequireSummary` for `consumer_code` (`CAS-PERF-02`) by parsing
/// `consumer_code` once with Oxc and indexing all `require("<target>")` bindings and inline
/// `require("<target>").Prop` member accesses.
pub fn extract_module_require_summary(consumer_code: &str) -> ModuleRequireSummary {
    use oxc::ast::ast::{Argument, BindingPattern, Expression, Statement};

    let mut summary = ModuleRequireSummary::default();
    if !consumer_code.contains("require(") {
        return summary;
    }

    let allocator = oxc::allocator::Allocator::default();
    let source_type = oxc::span::SourceType::mjs().with_module(false);
    let parser_ret = oxc::parser::Parser::new(&allocator, consumer_code, source_type).parse();
    if !parser_ret.diagnostics.is_empty() {
        summary.parse_failed = true;
        return summary;
    }

    // 1. Count all `require("<str>")` calls across the entire AST
    crate::linker::walk_ast_call_expressions(&parser_ret.program, &mut |call| {
        if let Expression::Identifier(callee) = call.callee.without_parentheses()
            && callee.name.as_str() == "require"
            && call.arguments.len() == 1
            && let Some(Argument::StringLiteral(str_lit)) = call.arguments.first()
        {
            let target = str_lit.value.as_str();
            summary
                .targets
                .entry(target.to_string())
                .or_default()
                .occurrences += 1;
        }
    });

    if summary.targets.is_empty() {
        return summary;
    }

    // 2. Inspect top-level `const { ... } = require("<target>")` and `const x = require("<target>")`
    for (stmt_idx, stmt) in parser_ret.program.body.iter().enumerate() {
        let Statement::VariableDeclaration(vd) = stmt else {
            continue;
        };
        for decl in &vd.declarations {
            let Some(init) = &decl.init else {
                continue;
            };
            let (target_id, is_to_esm) = if let Some(id) = is_any_require_call(init) {
                (id, false)
            } else if let Some(id) = is_to_esm_wrapped_require_call(init) {
                (id, true)
            } else {
                continue;
            };
            let entry = summary.targets.entry(target_id).or_default();
            match &decl.id {
                BindingPattern::ObjectPattern(obj_pat) => {
                    if obj_pat.rest.is_some() {
                        entry.forces_all = true;
                    } else {
                        for prop in &obj_pat.properties {
                            if let Some(key_name) = prop.key.name() {
                                if is_to_esm && key_name == "default" {
                                    let mut def_props = FxHashSet::default();
                                    if let BindingPattern::BindingIdentifier(local_id) = &prop.value
                                        && collect_member_accesses_on_var(
                                            consumer_code,
                                            local_id.name.as_str(),
                                            (decl.span.start as usize, decl.span.end as usize),
                                            &mut def_props,
                                        )
                                    {
                                        def_props.insert("default".to_string());
                                        def_props.insert("__esModule".to_string());
                                        entry.used_props.extend(def_props);
                                    } else {
                                        entry.forces_all = true;
                                    }
                                } else {
                                    entry.used_props.insert(key_name.to_string());
                                }
                            } else {
                                entry.forces_all = true;
                            }
                        }
                        entry.accounted += 1;
                    }
                }
                BindingPattern::BindingIdentifier(ident) => {
                    let var_name = ident.name.as_str();
                    if var_name.starts_with("__imp") {
                        if vd.declarations.len() == 1 {
                            if let Some(props) = try_collect_esm_default_interop_props(
                                &parser_ret.program.body,
                                stmt_idx,
                                var_name,
                                vd.span.start as usize,
                                consumer_code,
                            ) {
                                entry.used_props.extend(props);
                                entry.accounted += 1;
                            } else {
                                entry.forces_all = true;
                            }
                        } else {
                            entry.forces_all = true;
                        }
                    } else {
                        let mut ns_props = FxHashSet::default();
                        if collect_member_accesses_on_var(
                            consumer_code,
                            var_name,
                            (decl.span.start as usize, decl.span.end as usize),
                            &mut ns_props,
                        ) && !(is_to_esm && ns_props.contains("default"))
                        {
                            entry.used_props.extend(ns_props);
                            entry.accounted += 1;
                        } else {
                            entry.forces_all = true;
                        }
                    }
                }
                _ => {
                    entry.forces_all = true;
                }
            }
        }
    }

    // 3. Also check direct `require("<target>").Prop` inline member accesses
    crate::linker::walk_ast_expressions(&parser_ret.program, &mut |expr| {
        if let Expression::StaticMemberExpression(mem) = expr
            && let Some(target_id) = is_any_require_call(&mem.object)
            && let Some(entry) = summary.targets.get_mut(&target_id)
        {
            entry
                .used_props
                .insert(mem.property.name.as_str().to_string());
            entry.accounted += 1;
        }
    });

    summary
}

/// Evaluates `UsedExports` for `target_mod_id` from precomputed `ModuleRequireSummary` entries
/// in $O(M)$ time with zero redundant Oxc AST parses (`CAS-PERF-02`).
pub fn analyze_module_used_exports_from_summaries(
    target_mod_id: &str,
    summaries: &[ModuleRequireSummary],
    skip_idx: usize,
) -> UsedExports {
    let mut total_occurrences = 0usize;
    let mut used_set = FxHashSet::default();

    for (idx, summary) in summaries.iter().enumerate() {
        if idx == skip_idx {
            continue;
        }
        if summary.parse_failed {
            return UsedExports::All;
        }
        let Some(usage) = summary.targets.get(target_mod_id) else {
            continue;
        };
        if usage.occurrences == 0 {
            continue;
        }
        total_occurrences += usage.occurrences;
        if usage.forces_all || usage.accounted != usage.occurrences {
            return UsedExports::All;
        }
        used_set.extend(usage.used_props.iter().cloned());
    }

    if total_occurrences == 0 {
        UsedExports::Unreferenced
    } else {
        UsedExports::Subset(used_set)
    }
}
