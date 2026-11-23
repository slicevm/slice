//! Sorted Span Edit Slicer (`slice_with_edits`) and Mutated Binding Collector.

use super::walk_ast_expressions;

pub fn slice_with_edits(
    code: &str,
    span_start: usize,
    span_end: usize,
    edits: &[(usize, usize, String)],
) -> String {
    if edits.is_empty() {
        return code[span_start..span_end].to_string();
    }
    let start_idx = edits.partition_point(|(_, e, _)| *e <= span_start);
    if start_idx >= edits.len() || edits[start_idx].0 >= span_end {
        return code[span_start..span_end].to_string();
    }
    let mut out = String::with_capacity((span_end - span_start) + 32);
    let mut cursor = span_start;
    for (s, e, rep) in &edits[start_idx..] {
        if *s >= span_end {
            break;
        }
        if *s >= cursor && *e <= span_end {
            out.push_str(&code[cursor..*s]);
            out.push_str(rep);
            cursor = *e;
        }
    }
    out.push_str(&code[cursor..span_end]);
    out
}

/// Collects all binding identifier names that are reassigned via `AssignmentExpression`
/// (`x = ...`, `x ??= ...`, `({ x } = ...)`, `[x] = ...`) or `UpdateExpression` (`x++`, `--x`)
/// anywhere in `program`. Used to identify mutable ES module live bindings.
pub fn collect_mutated_binding_names(
    program: &oxc::ast::ast::Program<'_>,
) -> rustc_hash::FxHashSet<String> {
    use oxc::ast::ast::{
        AssignmentTarget, AssignmentTargetMaybeDefault, AssignmentTargetPattern,
        AssignmentTargetProperty, Expression, SimpleAssignmentTarget,
    };

    fn collect_target(target: &AssignmentTarget<'_>, out: &mut rustc_hash::FxHashSet<String>) {
        match target {
            AssignmentTarget::AssignmentTargetIdentifier(id) => {
                out.insert(id.name.to_string());
            }
            AssignmentTarget::ObjectAssignmentTarget(obj) => {
                for prop in &obj.properties {
                    match prop {
                        AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(p) => {
                            out.insert(p.binding.name.to_string());
                        }
                        AssignmentTargetProperty::AssignmentTargetPropertyProperty(p) => {
                            collect_maybe_default(&p.binding, out);
                        }
                    }
                }
                if let Some(rest) = &obj.rest {
                    collect_target(&rest.target, out);
                }
            }
            AssignmentTarget::ArrayAssignmentTarget(arr) => {
                for el in arr.elements.iter().flatten() {
                    collect_maybe_default(el, out);
                }
                if let Some(rest) = &arr.rest {
                    collect_target(&rest.target, out);
                }
            }
            _ => {}
        }
    }

    fn collect_maybe_default(
        md: &AssignmentTargetMaybeDefault<'_>,
        out: &mut rustc_hash::FxHashSet<String>,
    ) {
        match md {
            AssignmentTargetMaybeDefault::AssignmentTargetWithDefault(def) => {
                collect_target(&def.binding, out);
            }
            AssignmentTargetMaybeDefault::AssignmentTargetIdentifier(id) => {
                out.insert(id.name.to_string());
            }
            AssignmentTargetMaybeDefault::ObjectAssignmentTarget(obj) => {
                for prop in &obj.properties {
                    match prop {
                        AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(p) => {
                            out.insert(p.binding.name.to_string());
                        }
                        AssignmentTargetProperty::AssignmentTargetPropertyProperty(p) => {
                            collect_maybe_default(&p.binding, out);
                        }
                    }
                }
                if let Some(rest) = &obj.rest {
                    collect_target(&rest.target, out);
                }
            }
            AssignmentTargetMaybeDefault::ArrayAssignmentTarget(arr) => {
                for el in arr.elements.iter().flatten() {
                    collect_maybe_default(el, out);
                }
                if let Some(rest) = &arr.rest {
                    collect_target(&rest.target, out);
                }
            }
            _ => {}
        }
    }

    let _ = AssignmentTargetPattern::ArrayAssignmentTarget;
    let mut out = rustc_hash::FxHashSet::default();
    walk_ast_expressions(program, &mut |expr| match expr {
        Expression::AssignmentExpression(assign) => {
            collect_target(&assign.left, &mut out);
        }
        Expression::UpdateExpression(upd) => {
            if let SimpleAssignmentTarget::AssignmentTargetIdentifier(id) = &upd.argument {
                out.insert(id.name.to_string());
            }
        }
        _ => {}
    });
    out
}
