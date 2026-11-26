//! Oxc AST Dead-Declaration & Dead-Export Tree-Shaker (`prune_and_minify_cjs_module_ast`).

use super::{expr_contains_assignment, extract_referenced_identifiers, is_any_require_call};
use crate::linker::collect_binding_pattern_names;
use rustc_hash::{FxHashMap, FxHashSet};

fn pattern_has_live(pat: &oxc::ast::ast::BindingPattern<'_>, live: &FxHashSet<String>) -> bool {
    let mut names = Vec::new();
    collect_binding_pattern_names(pat, &mut names);
    names.iter().any(|(n, _)| live.contains(n))
}

fn match_exports_define_property_stmt(
    stmt: &oxc::ast::ast::Statement<'_>,
    src: &str,
) -> Option<(String, FxHashSet<String>)> {
    use oxc::ast::ast::{Argument, Expression, Statement};
    use oxc::span::GetSpan;
    let Statement::ExpressionStatement(es) = stmt else {
        return None;
    };
    let Expression::CallExpression(call) = es.expression.without_parentheses() else {
        return None;
    };
    if call.arguments.len() != 3 {
        return None;
    }
    let Expression::StaticMemberExpression(mem) = call.callee.without_parentheses() else {
        return None;
    };
    if mem.property.name.as_str() != "defineProperty" {
        return None;
    }
    let Expression::Identifier(obj_id) = mem.object.without_parentheses() else {
        return None;
    };
    if obj_id.name.as_str() != "Object" {
        return None;
    }
    let first_expr = call.arguments[0].as_expression()?;
    if !matches!(
        first_expr.without_parentheses(),
        Expression::Identifier(id) if id.name.as_str() == "exports"
    ) {
        return None;
    }
    let Argument::StringLiteral(key_lit) = &call.arguments[1] else {
        return None;
    };
    let desc_expr = call.arguments[2].as_expression()?;
    let desc_sp = desc_expr.span();
    let desc_src = &src[desc_sp.start as usize..desc_sp.end as usize];
    Some((
        key_lit.value.display().to_string(),
        extract_referenced_identifiers(desc_src),
    ))
}

fn match_cjs_export_stmt(
    stmt: &oxc::ast::ast::Statement<'_>,
    src: &str,
) -> Option<(String, FxHashSet<String>)> {
    match_exports_assignment_stmt(stmt, src)
        .or_else(|| match_exports_define_property_stmt(stmt, src))
}

fn match_exports_assignment_stmt(
    stmt: &oxc::ast::ast::Statement<'_>,
    src: &str,
) -> Option<(String, FxHashSet<String>)> {
    use oxc::ast::ast::Statement;
    use oxc::span::GetSpan;
    let Statement::ExpressionStatement(_) = stmt else {
        return None;
    };
    let sp = stmt.span();
    let text = src[sp.start as usize..sp.end as usize].trim();
    let after_exp = text.strip_prefix("exports.")?;
    let eq_idx = after_exp.find('=')?;
    if after_exp[eq_idx + 1..].starts_with('=') {
        return None;
    }
    let exp_name = after_exp[..eq_idx].trim();
    if exp_name.is_empty()
        || !exp_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
    {
        return None;
    }
    let rhs = after_exp[eq_idx + 1..].trim().trim_end_matches(';').trim();
    if rhs.contains("exports.") {
        return None;
    }
    Some((exp_name.to_string(), extract_referenced_identifiers(rhs)))
}

/// General-Purpose Oxc AST Dead-Declaration & Dead-Export Tree-Shaker (`prune_and_minify_cjs_module_ast`):
/// - Computes the transitive closure of top-level declarations (`function`, `class`, and per-`VariableDeclarator`
///   `const`/`let`/`var` bindings) reachable from `used_exports` and top-level side effects.
/// - Prunes unreferenced `exports.<Unused> = ...` assignments, unreferenced top-level functions/classes/variables,
///   and unused destructured properties in `const { a, b } = require("...")`.
/// - Emits clean, comment-free (and optionally minified) JavaScript via `oxc::codegen::Codegen`.
pub fn prune_and_minify_cjs_module_ast(
    cjs_code: &str,
    used_exports: Option<&FxHashSet<String>>,
    minify: bool,
) -> String {
    use oxc::ast::ast::{BindingPattern, Statement};
    use oxc::span::GetSpan;

    let allocator = oxc::allocator::Allocator::default();
    let source_type = oxc::span::SourceType::mjs().with_module(false);
    let parser_ret = oxc::parser::Parser::new(&allocator, cjs_code, source_type).parse();
    if !parser_ret.diagnostics.is_empty() {
        return cjs_code.to_string();
    }

    let mut program = parser_ret.program;

    if let Some(used_set) = used_exports {
        let has_named_export_stmts = program.body.iter().any(|s| {
            match_cjs_export_stmt(s, cjs_code).is_some_and(|(name, _)| name != "__esModule")
        });

        let mut pruned_module_exports_obj_span: Option<(u32, Vec<String>)> = None;
        if !has_named_export_stmts && cjs_code.matches("exports").count() == 1 {
            for stmt in program.body.iter_mut() {
                let Statement::ExpressionStatement(es) = stmt else {
                    continue;
                };
                let oxc::ast::ast::Expression::AssignmentExpression(assign) =
                    es.expression.get_inner_expression_mut()
                else {
                    continue;
                };
                if assign.operator != oxc::ast::ast::AssignmentOperator::Assign {
                    continue;
                }
                let lhs_sp = assign.left.span();
                if cjs_code[lhs_sp.start as usize..lhs_sp.end as usize].trim() != "module.exports" {
                    continue;
                }
                let oxc::ast::ast::Expression::ObjectExpression(obj) = &mut assign.right else {
                    continue;
                };
                let obj_text = &cjs_code[obj.span.start as usize..obj.span.end as usize];
                if obj_text.contains("this.") {
                    break;
                }
                let all_static_named = obj.properties.iter().all(|p| match p {
                    oxc::ast::ast::ObjectPropertyKind::ObjectProperty(op) => {
                        !op.key.is_computed() && op.key.name().is_some()
                    }
                    oxc::ast::ast::ObjectPropertyKind::SpreadProperty(_) => false,
                });
                if !all_static_named {
                    break;
                }
                let before_len = obj.properties.len();
                obj.properties.retain(|p| {
                    let oxc::ast::ast::ObjectPropertyKind::ObjectProperty(op) = p else {
                        return true;
                    };
                    let Some(k_name) = op.key.name() else {
                        return true;
                    };
                    if k_name.as_str().is_some_and(|s| used_set.contains(s)) {
                        return true;
                    }
                    let is_pure_droppable = op.kind == oxc::ast::ast::PropertyKind::Get
                        || matches!(
                            op.value.without_parentheses(),
                            oxc::ast::ast::Expression::Identifier(_)
                                | oxc::ast::ast::Expression::FunctionExpression(_)
                                | oxc::ast::ast::Expression::ArrowFunctionExpression(_)
                                | oxc::ast::ast::Expression::ClassExpression(_)
                                | oxc::ast::ast::Expression::NumericLiteral(_)
                                | oxc::ast::ast::Expression::StringLiteral(_)
                                | oxc::ast::ast::Expression::BooleanLiteral(_)
                                | oxc::ast::ast::Expression::NullLiteral(_)
                        );
                    !is_pure_droppable
                });
                if obj.properties.len() != before_len {
                    let mut kept_refs = Vec::new();
                    for p in &obj.properties {
                        let p_sp = p.span();
                        let p_txt = &cjs_code[p_sp.start as usize..p_sp.end as usize];
                        kept_refs.extend(extract_referenced_identifiers(p_txt));
                    }
                    pruned_module_exports_obj_span = Some((es.span.start, kept_refs));
                }
                break;
            }
        }

        if has_named_export_stmts || pruned_module_exports_obj_span.is_some() {
            // Step 1: Remove unused `exports.<Unused> = ...`, `Object.defineProperty(exports, "<Unused>", ...)`, and legacy `module.exports = __slice_default`
            if has_named_export_stmts {
                program.body.retain(|stmt| {
                    if let Some((exp_name, _)) = match_cjs_export_stmt(stmt, cjs_code) {
                        exp_name == "__esModule" || used_set.contains(&exp_name)
                    } else {
                        let sp = stmt.span();
                        let text = cjs_code[sp.start as usize..sp.end as usize].trim();
                        !text.starts_with("module.exports = __slice_default;")
                            || used_set.contains("default")
                    }
                });
            }

            // Step 2: Iterative Oxc Semantic Scope-Resolved Zero-Reference Pruning.
            for _ in 0..8 {
                let scoping = oxc::semantic::SemanticBuilder::new()
                    .build(&program)
                    .semantic
                    .into_scoping();
                let root_scope = scoping.root_scope_id();
                let mut referenced_root_syms: FxHashSet<String> = FxHashSet::default();
                for sym_id in scoping.symbol_ids() {
                    if scoping.symbol_scope_id(sym_id) == root_scope
                        && !scoping.get_resolved_reference_ids(sym_id).is_empty()
                    {
                        referenced_root_syms.insert(scoping.symbol_name(sym_id).to_string());
                    }
                }

                let mut changed = false;
                for stmt in program.body.iter_mut() {
                    if let Statement::VariableDeclaration(vd) = stmt {
                        for decl in vd.declarations.iter_mut() {
                            let is_req = decl
                                .init
                                .as_ref()
                                .is_some_and(|e| is_any_require_call(e).is_some());
                            if is_req
                                && let BindingPattern::ObjectPattern(obj_pat) = &mut decl.id
                                && obj_pat.rest.is_none()
                            {
                                let before_len = obj_pat.properties.len();
                                obj_pat.properties.retain(|prop| {
                                    pattern_has_live(&prop.value, &referenced_root_syms)
                                });
                                if obj_pat.properties.len() != before_len {
                                    changed = true;
                                }
                            }
                        }
                        let before_decls = vd.declarations.len();
                        vd.declarations.retain(|decl| {
                            pattern_has_live(&decl.id, &referenced_root_syms)
                                || decl.init.as_ref().is_some_and(expr_contains_assignment)
                        });
                        if vd.declarations.len() != before_decls {
                            changed = true;
                        }
                    }
                }

                let before_stmts = program.body.len();
                program.body.retain(|stmt| match stmt {
                    Statement::FunctionDeclaration(fd) => fd
                        .id
                        .as_ref()
                        .is_none_or(|id| referenced_root_syms.contains(id.name.as_str())),
                    Statement::ClassDeclaration(cd) => cd
                        .id
                        .as_ref()
                        .is_none_or(|id| referenced_root_syms.contains(id.name.as_str())),
                    Statement::VariableDeclaration(vd) => !vd.declarations.is_empty(),
                    _ => true,
                });
                if program.body.len() != before_stmts {
                    changed = true;
                }
                if !changed {
                    break;
                }
            }

            // Step 3: Transitive reachability closure to prune self-recursive dead functions
            let mut decl_dep_map: FxHashMap<String, Vec<String>> = FxHashMap::default();
            for stmt in &program.body {
                match stmt {
                    Statement::FunctionDeclaration(fd) => {
                        if let Some(id) = &fd.id {
                            let name = id.name.to_string();
                            let refs = extract_referenced_identifiers(
                                &cjs_code[fd.span.start as usize..fd.span.end as usize],
                            );
                            decl_dep_map.entry(name).or_default().extend(refs);
                        }
                    }
                    Statement::ClassDeclaration(cd) => {
                        if let Some(id) = &cd.id {
                            let name = id.name.to_string();
                            let refs = extract_referenced_identifiers(
                                &cjs_code[cd.span.start as usize..cd.span.end as usize],
                            );
                            decl_dep_map.entry(name).or_default().extend(refs);
                        }
                    }
                    Statement::VariableDeclaration(vd) => {
                        for decl in &vd.declarations {
                            let mut bound = Vec::new();
                            collect_binding_pattern_names(&decl.id, &mut bound);
                            let is_destructured_req = matches!(&decl.id, BindingPattern::ObjectPattern(obj) if obj.rest.is_none())
                                && decl
                                    .init
                                    .as_ref()
                                    .is_some_and(|e| is_any_require_call(e).is_some());
                            let refs = if is_destructured_req {
                                FxHashSet::default()
                            } else {
                                extract_referenced_identifiers(
                                    &cjs_code[decl.span.start as usize..decl.span.end as usize],
                                )
                            };
                            for (b_name, _) in bound {
                                decl_dep_map
                                    .entry(b_name)
                                    .or_default()
                                    .extend(refs.iter().cloned());
                            }
                        }
                    }
                    _ => {}
                }
            }

            let mut worklist: Vec<String> = Vec::new();
            for stmt in &program.body {
                match stmt {
                    Statement::FunctionDeclaration(_) | Statement::ClassDeclaration(_) => {}
                    Statement::VariableDeclaration(vd) => {
                        for decl in &vd.declarations {
                            if decl.init.as_ref().is_some_and(expr_contains_assignment) {
                                let sp = decl.span();
                                let text = cjs_code[sp.start as usize..sp.end as usize].trim();
                                worklist.extend(extract_referenced_identifiers(text));
                            }
                        }
                    }
                    _ => {
                        let sp = stmt.span();
                        if let Some((obj_stmt_start, ref kept_refs)) =
                            pruned_module_exports_obj_span
                            && sp.start == obj_stmt_start
                        {
                            worklist.extend(kept_refs.iter().cloned());
                            continue;
                        }
                        let text = cjs_code[sp.start as usize..sp.end as usize].trim();
                        worklist.extend(extract_referenced_identifiers(text));
                    }
                }
            }

            let mut live_idents: FxHashSet<String> = FxHashSet::default();
            while let Some(ident) = worklist.pop() {
                if live_idents.insert(ident.clone())
                    && let Some(deps) = decl_dep_map.get(&ident)
                {
                    for d in deps {
                        if !live_idents.contains(d) {
                            worklist.push(d.clone());
                        }
                    }
                }
            }

            for stmt in program.body.iter_mut() {
                if let Statement::VariableDeclaration(vd) = stmt {
                    for decl in vd.declarations.iter_mut() {
                        let is_req = decl
                            .init
                            .as_ref()
                            .is_some_and(|e| is_any_require_call(e).is_some());
                        if is_req
                            && let BindingPattern::ObjectPattern(obj_pat) = &mut decl.id
                            && obj_pat.rest.is_none()
                        {
                            obj_pat
                                .properties
                                .retain(|prop| pattern_has_live(&prop.value, &live_idents));
                        }
                    }
                    vd.declarations.retain(|decl| {
                        pattern_has_live(&decl.id, &live_idents)
                            || decl.init.as_ref().is_some_and(expr_contains_assignment)
                    });
                }
            }

            program.body.retain(|stmt| match stmt {
                Statement::FunctionDeclaration(fd) => fd
                    .id
                    .as_ref()
                    .is_none_or(|id| live_idents.contains(id.name.as_str())),
                Statement::ClassDeclaration(cd) => cd
                    .id
                    .as_ref()
                    .is_none_or(|id| live_idents.contains(id.name.as_str())),
                Statement::VariableDeclaration(vd) => !vd.declarations.is_empty(),
                _ => {
                    if let Some((exp_name, _)) = match_cjs_export_stmt(stmt, cjs_code) {
                        exp_name == "__esModule" || used_set.contains(&exp_name)
                    } else {
                        let sp = stmt.span();
                        let text = cjs_code[sp.start as usize..sp.end as usize].trim();
                        !text.starts_with("module.exports = __slice_default;")
                            || used_set.contains("default")
                    }
                }
            });
        }
    }

    oxc::codegen::Codegen::new()
        .with_options(oxc::codegen::CodegenOptions {
            minify,
            comments: oxc::codegen::CommentOptions::disabled(),
            ..Default::default()
        })
        .build(&program)
        .code
}
