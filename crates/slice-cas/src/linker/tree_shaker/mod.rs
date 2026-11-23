//! Oxc AST Dead-Declaration & Dead-Export Tree-Shaker (`prune_and_minify_cjs_module_ast`),
//! Used-Exports Summary Analyzer (`analyze_module_used_exports_from_summaries`), and Free Identifier Scanner.
//!
//! Organizes the tree-shaking subsystem into focused submodules:
//! - [`ident_scanner`]: [`extract_referenced_identifiers`], [`extract_referenced_identifiers_with_offsets`], and JS identifier/keyword classifiers.
//! - [`used_exports`]: [`extract_module_require_summary`] and [`analyze_module_used_exports_from_summaries`].
//! - [`cjs_pruner`]: [`prune_and_minify_cjs_module_ast`] dead-export AST pruner.

pub mod cjs_pruner;
pub mod ident_scanner;
pub mod used_exports;

pub use cjs_pruner::prune_and_minify_cjs_module_ast;
pub use ident_scanner::{
    extract_referenced_identifiers, extract_referenced_identifiers_with_offsets,
    is_js_ident_continue, is_js_ident_start, is_js_keyword,
};
pub use used_exports::{
    analyze_module_used_exports_from_summaries, extract_module_require_summary,
};

use rustc_hash::{FxHashMap, FxHashSet};

#[derive(Debug, Clone)]
pub enum UsedExports {
    All,
    Unreferenced,
    Subset(FxHashSet<String>),
}

/// Cached per-module `require("<target>")` usage summary (`CAS-PERF-02`) so Pass 2 in
/// `bundle_entrypoint_with_deps` parses each module at most once instead of $O(M^2)$ times.
#[derive(Debug, Clone, Default)]
pub struct ConsumerTargetUsage {
    pub occurrences: usize,
    pub accounted: usize,
    pub forces_all: bool,
    pub used_props: FxHashSet<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ModuleRequireSummary {
    pub parse_failed: bool,
    pub targets: FxHashMap<String, ConsumerTargetUsage>,
}

pub(crate) fn is_any_require_call(expr: &oxc::ast::ast::Expression<'_>) -> Option<String> {
    use oxc::ast::ast::{Argument, Expression};
    let Expression::CallExpression(call) = expr.without_parentheses() else {
        return None;
    };
    let Expression::Identifier(callee) = call.callee.without_parentheses() else {
        return None;
    };
    if callee.name.as_str() != "require" || call.arguments.len() != 1 {
        return None;
    }
    let Some(Argument::StringLiteral(str_lit)) = call.arguments.first() else {
        return None;
    };
    Some(str_lit.value.to_string())
}

pub(super) fn expr_contains_assignment(expr: &oxc::ast::ast::Expression<'_>) -> bool {
    use oxc::ast::ast::{
        Argument, ArrayExpressionElement, ChainElement, Expression, ObjectPropertyKind,
    };
    match expr {
        Expression::AssignmentExpression(_) | Expression::UpdateExpression(_) => true,
        Expression::ParenthesizedExpression(p) => expr_contains_assignment(&p.expression),
        Expression::SequenceExpression(seq) => seq.expressions.iter().any(expr_contains_assignment),
        Expression::BinaryExpression(b) => {
            expr_contains_assignment(&b.left) || expr_contains_assignment(&b.right)
        }
        Expression::LogicalExpression(l) => {
            expr_contains_assignment(&l.left) || expr_contains_assignment(&l.right)
        }
        Expression::UnaryExpression(u) => expr_contains_assignment(&u.argument),
        Expression::ConditionalExpression(c) => {
            expr_contains_assignment(&c.test)
                || expr_contains_assignment(&c.consequent)
                || expr_contains_assignment(&c.alternate)
        }
        Expression::CallExpression(c) => {
            expr_contains_assignment(&c.callee)
                || c.arguments.iter().any(|a| match a {
                    Argument::SpreadElement(sp) => expr_contains_assignment(&sp.argument),
                    other => other.as_expression().is_some_and(expr_contains_assignment),
                })
        }
        Expression::NewExpression(n) => {
            expr_contains_assignment(&n.callee)
                || n.arguments.iter().any(|a| match a {
                    Argument::SpreadElement(sp) => expr_contains_assignment(&sp.argument),
                    other => other.as_expression().is_some_and(expr_contains_assignment),
                })
        }
        Expression::StaticMemberExpression(m) => expr_contains_assignment(&m.object),
        Expression::ComputedMemberExpression(m) => {
            expr_contains_assignment(&m.object) || expr_contains_assignment(&m.expression)
        }
        Expression::ObjectExpression(obj) => obj.properties.iter().any(|p| match p {
            ObjectPropertyKind::ObjectProperty(op) => {
                (op.computed && op.key.as_expression().is_some_and(expr_contains_assignment))
                    || expr_contains_assignment(&op.value)
            }
            ObjectPropertyKind::SpreadProperty(sp) => expr_contains_assignment(&sp.argument),
        }),
        Expression::ArrayExpression(arr) => arr.elements.iter().any(|el| match el {
            ArrayExpressionElement::SpreadElement(sp) => expr_contains_assignment(&sp.argument),
            other => other.as_expression().is_some_and(expr_contains_assignment),
        }),
        Expression::ChainExpression(ch) => match &ch.expression {
            ChainElement::CallExpression(c) => {
                expr_contains_assignment(&c.callee)
                    || c.arguments
                        .iter()
                        .any(|a| a.as_expression().is_some_and(expr_contains_assignment))
            }
            ChainElement::StaticMemberExpression(m) => expr_contains_assignment(&m.object),
            ChainElement::ComputedMemberExpression(m) => {
                expr_contains_assignment(&m.object) || expr_contains_assignment(&m.expression)
            }
            _ => false,
        },
        _ => false,
    }
}
