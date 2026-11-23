//! Side-Effect, Receiver, and Scope-Hazard Classifiers for Array Pipeline Fusion.

use oxc::ast::ast::{
    Argument, ArrayExpressionElement, ChainElement, Expression, ObjectPropertyKind, PropertyKind,
    UnaryOperator,
};
use oxc::span::GetSpan;

pub(super) fn expr_contains_fusion_hazard(expr: &Expression<'_>, src: &str) -> bool {
    match expr {
        Expression::AwaitExpression(_)
        | Expression::YieldExpression(_)
        | Expression::ThisExpression(_)
        | Expression::Super(_) => true,
        Expression::Identifier(id) => id.name.as_str() == "arguments",
        Expression::ParenthesizedExpression(p) => expr_contains_fusion_hazard(&p.expression, src),
        Expression::CallExpression(c) => {
            expr_contains_fusion_hazard(&c.callee, src)
                || c.arguments.iter().any(|a| match a {
                    Argument::SpreadElement(sp) => expr_contains_fusion_hazard(&sp.argument, src),
                    other => other
                        .as_expression()
                        .is_some_and(|e| expr_contains_fusion_hazard(e, src)),
                })
        }
        Expression::NewExpression(n) => {
            expr_contains_fusion_hazard(&n.callee, src)
                || n.arguments.iter().any(|a| match a {
                    Argument::SpreadElement(sp) => expr_contains_fusion_hazard(&sp.argument, src),
                    other => other
                        .as_expression()
                        .is_some_and(|e| expr_contains_fusion_hazard(e, src)),
                })
        }
        Expression::StaticMemberExpression(m) => expr_contains_fusion_hazard(&m.object, src),
        Expression::ComputedMemberExpression(m) => {
            expr_contains_fusion_hazard(&m.object, src)
                || expr_contains_fusion_hazard(&m.expression, src)
        }
        Expression::PrivateFieldExpression(m) => expr_contains_fusion_hazard(&m.object, src),
        Expression::BinaryExpression(b) => {
            expr_contains_fusion_hazard(&b.left, src) || expr_contains_fusion_hazard(&b.right, src)
        }
        Expression::LogicalExpression(l) => {
            expr_contains_fusion_hazard(&l.left, src) || expr_contains_fusion_hazard(&l.right, src)
        }
        Expression::UnaryExpression(u) => expr_contains_fusion_hazard(&u.argument, src),
        Expression::UpdateExpression(u) => match &u.argument {
            oxc::ast::ast::SimpleAssignmentTarget::StaticMemberExpression(m) => {
                expr_contains_fusion_hazard(&m.object, src)
            }
            oxc::ast::ast::SimpleAssignmentTarget::ComputedMemberExpression(m) => {
                expr_contains_fusion_hazard(&m.object, src)
                    || expr_contains_fusion_hazard(&m.expression, src)
            }
            oxc::ast::ast::SimpleAssignmentTarget::PrivateFieldExpression(m) => {
                expr_contains_fusion_hazard(&m.object, src)
            }
            _ => false,
        },
        Expression::ConditionalExpression(c) => {
            expr_contains_fusion_hazard(&c.test, src)
                || expr_contains_fusion_hazard(&c.consequent, src)
                || expr_contains_fusion_hazard(&c.alternate, src)
        }
        Expression::AssignmentExpression(a) => {
            let left_hazard = match &a.left {
                oxc::ast::ast::AssignmentTarget::StaticMemberExpression(m) => {
                    expr_contains_fusion_hazard(&m.object, src)
                }
                oxc::ast::ast::AssignmentTarget::ComputedMemberExpression(m) => {
                    expr_contains_fusion_hazard(&m.object, src)
                        || expr_contains_fusion_hazard(&m.expression, src)
                }
                oxc::ast::ast::AssignmentTarget::PrivateFieldExpression(m) => {
                    expr_contains_fusion_hazard(&m.object, src)
                }
                _ => false,
            };
            left_hazard || expr_contains_fusion_hazard(&a.right, src)
        }
        Expression::SequenceExpression(seq) => seq
            .expressions
            .iter()
            .any(|e| expr_contains_fusion_hazard(e, src)),
        Expression::ObjectExpression(obj) => obj.properties.iter().any(|prop| match prop {
            ObjectPropertyKind::ObjectProperty(p) => {
                (p.computed
                    && p.key
                        .as_expression()
                        .is_some_and(|e| expr_contains_fusion_hazard(e, src)))
                    || expr_contains_fusion_hazard(&p.value, src)
            }
            ObjectPropertyKind::SpreadProperty(sp) => {
                expr_contains_fusion_hazard(&sp.argument, src)
            }
        }),
        Expression::ArrayExpression(arr) => arr.elements.iter().any(|el| match el {
            ArrayExpressionElement::SpreadElement(sp) => {
                expr_contains_fusion_hazard(&sp.argument, src)
            }
            other => other
                .as_expression()
                .is_some_and(|e| expr_contains_fusion_hazard(e, src)),
        }),
        Expression::TemplateLiteral(tpl) => tpl
            .expressions
            .iter()
            .any(|e| expr_contains_fusion_hazard(e, src)),
        Expression::TaggedTemplateExpression(tt) => {
            expr_contains_fusion_hazard(&tt.tag, src)
                || tt
                    .quasi
                    .expressions
                    .iter()
                    .any(|e| expr_contains_fusion_hazard(e, src))
        }
        Expression::ChainExpression(ch) => match &ch.expression {
            ChainElement::CallExpression(c) => {
                expr_contains_fusion_hazard(&c.callee, src)
                    || c.arguments.iter().any(|a| {
                        a.as_expression()
                            .is_some_and(|e| expr_contains_fusion_hazard(e, src))
                    })
            }
            ChainElement::StaticMemberExpression(m) => expr_contains_fusion_hazard(&m.object, src),
            ChainElement::ComputedMemberExpression(m) => {
                expr_contains_fusion_hazard(&m.object, src)
                    || expr_contains_fusion_hazard(&m.expression, src)
            }
            ChainElement::PrivateFieldExpression(m) => expr_contains_fusion_hazard(&m.object, src),
            ChainElement::TSNonNullExpression(n) => expr_contains_fusion_hazard(&n.expression, src),
        },
        Expression::ArrowFunctionExpression(arrow) => {
            if arrow.r#async {
                return true;
            }
            match &arrow.body {
                oxc::ast::ast::ArrowFunctionBody::FunctionBody(_) => true,
                _ => arrow
                    .body
                    .as_expression()
                    .is_some_and(|e| expr_contains_fusion_hazard(e, src)),
            }
        }
        Expression::FunctionExpression(_) | Expression::ClassExpression(_) => false,
        other => {
            let sp = other.span();
            matches!(
                src.get(sp.start as usize..sp.end as usize),
                Some("new.target" | "import.meta")
            )
        }
    }
}

/// Rejects expressions that may perform observable side effects across stages when fusing
/// multi-stage pipelines (`map`+`reduce`, `filter`+`map`+`reduce`), preventing callback or
/// `<init>` side-effect reordering relative to ECMA-262 eager stage execution.
pub(super) fn expr_has_observable_side_effect(expr: &Expression<'_>) -> bool {
    match expr {
        Expression::CallExpression(_)
        | Expression::NewExpression(_)
        | Expression::AssignmentExpression(_)
        | Expression::UpdateExpression(_)
        | Expression::TaggedTemplateExpression(_)
        | Expression::AwaitExpression(_)
        | Expression::YieldExpression(_)
        | Expression::ImportExpression(_) => true,
        Expression::UnaryExpression(u) => {
            u.operator == UnaryOperator::Delete || expr_has_observable_side_effect(&u.argument)
        }
        Expression::SequenceExpression(seq) => {
            seq.expressions.iter().any(expr_has_observable_side_effect)
        }
        Expression::ParenthesizedExpression(p) => expr_has_observable_side_effect(&p.expression),
        Expression::StaticMemberExpression(m) => expr_has_observable_side_effect(&m.object),
        Expression::ComputedMemberExpression(m) => {
            expr_has_observable_side_effect(&m.object)
                || expr_has_observable_side_effect(&m.expression)
        }
        Expression::PrivateFieldExpression(m) => expr_has_observable_side_effect(&m.object),
        Expression::BinaryExpression(b) => {
            expr_has_observable_side_effect(&b.left) || expr_has_observable_side_effect(&b.right)
        }
        Expression::LogicalExpression(l) => {
            expr_has_observable_side_effect(&l.left) || expr_has_observable_side_effect(&l.right)
        }
        Expression::ConditionalExpression(c) => {
            expr_has_observable_side_effect(&c.test)
                || expr_has_observable_side_effect(&c.consequent)
                || expr_has_observable_side_effect(&c.alternate)
        }
        Expression::ObjectExpression(obj) => obj.properties.iter().any(|prop| match prop {
            ObjectPropertyKind::ObjectProperty(p) => {
                p.kind != PropertyKind::Init
                    || (p.computed
                        && p.key
                            .as_expression()
                            .is_some_and(expr_has_observable_side_effect))
                    || expr_has_observable_side_effect(&p.value)
            }
            ObjectPropertyKind::SpreadProperty(sp) => expr_has_observable_side_effect(&sp.argument),
        }),
        Expression::ArrayExpression(arr) => arr.elements.iter().any(|el| match el {
            ArrayExpressionElement::SpreadElement(sp) => {
                expr_has_observable_side_effect(&sp.argument)
            }
            other => other
                .as_expression()
                .is_some_and(expr_has_observable_side_effect),
        }),
        Expression::TemplateLiteral(tpl) => {
            tpl.expressions.iter().any(expr_has_observable_side_effect)
        }
        Expression::ChainExpression(ch) => match &ch.expression {
            ChainElement::CallExpression(_) => true,
            ChainElement::StaticMemberExpression(m) => expr_has_observable_side_effect(&m.object),
            ChainElement::ComputedMemberExpression(m) => {
                expr_has_observable_side_effect(&m.object)
                    || expr_has_observable_side_effect(&m.expression)
            }
            ChainElement::PrivateFieldExpression(m) => expr_has_observable_side_effect(&m.object),
            ChainElement::TSNonNullExpression(n) => expr_has_observable_side_effect(&n.expression),
        },
        _ => false,
    }
}

pub(super) fn is_definitely_non_array_receiver(expr: &Expression<'_>) -> bool {
    matches!(
        expr.without_parentheses(),
        Expression::ObjectExpression(_)
            | Expression::FunctionExpression(_)
            | Expression::ArrowFunctionExpression(_)
            | Expression::ClassExpression(_)
            | Expression::StringLiteral(_)
            | Expression::NumericLiteral(_)
            | Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_)
            | Expression::BigIntLiteral(_)
            | Expression::RegExpLiteral(_)
    )
}
