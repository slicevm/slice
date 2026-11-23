//! AST-Verified Tier-0 (`SLICESNP`) Compatibility Gate over `oxc::ast::ast::Program`.

use super::tier0_scope_and_hazards::{
    check_scope_stmt, is_tier0_disallowed_ident, is_tier0_disallowed_target_ident,
    params_are_simple, static_member_chain, stmt_has_top_level_guarded_proto_mutation,
};
use oxc::ast::ast::{
    Argument, ArrayExpressionElement, ArrowFunctionBody, AssignmentOperator, AssignmentTarget,
    BinaryOperator, BindingPattern, Declaration, Expression, ForStatementInit, LogicalOperator,
    ObjectPropertyKind, PropertyKey, SimpleAssignmentTarget, Statement, UnaryOperator,
};

/// Maximum ASCII source byte size eligible for Tier-0 `SLICESNP\x02` compilation (`128 KiB`,
/// accommodating multi-hundred-module `.cas` entrypoints while bounding lexer work).
const MAX_TIER0_SLICESNP_SOURCE_BYTES: usize = 131_072;

/// AST-verified Tier-0 (`SLICESNP`) compatibility analyzer using `oxc::parser::Parser`.
///
/// Instead of maintaining a fragile ~95-substring denylist (`uses_full_es_vm`), Slice defaults
/// full ECMAScript/TypeScript execution to `SLICEBOA` (Boa ES2024 VM) and routes a single-file
/// script to the lightweight Tier-0/Tier-2 `SLICESNP` register/AST engine **only** when 100% of
/// statements, expressions, operators, and built-in calls in the Oxc AST belong to the exact
/// subset supported by `SLICESNP` (`eval_stmt` / `eval_expr` / `NumericFuncIr`).
pub fn is_tier0_slicesnp_compatible_ast(source: &str) -> bool {
    analyze_tier0_slicesnp_ast(source).0
}

#[inline]
pub(super) fn can_attempt_tier0_slicesnp(source: &str) -> bool {
    let src_bytes = source.as_bytes();
    source.is_ascii()
        && src_bytes.len() <= MAX_TIER0_SLICESNP_SOURCE_BYTES
        && memchr::memmem::find(src_bytes, b"var __slice_modules =").is_none()
        && memchr::memmem::find(src_bytes, b"__slice_").is_none()
        && memchr::memmem::find(src_bytes, b"node:").is_none()
        && memchr::memmem::find(src_bytes, b"cloudflare:").is_none()
        && memchr::memmem::find(src_bytes, b"slice:").is_none()
        && memchr::memmem::find(src_bytes, b"new ").is_none()
        && memchr::memmem::find(src_bytes, b"try").is_none()
        && memchr::memmem::find(src_bytes, b"async").is_none()
        && memchr::memmem::find(src_bytes, b"await").is_none()
        && memchr::memmem::find(src_bytes, b"class ").is_none()
}

/// Single-pass Oxc AST check returning `(is_tier0_compatible, has_guarded_proto_mutation)`.
pub(super) fn analyze_tier0_slicesnp_ast(source: &str) -> (bool, bool) {
    if !can_attempt_tier0_slicesnp(source) {
        return (false, false);
    }

    let allocator = oxc::allocator::Allocator::default();
    let parse_opts = oxc::parser::ParseOptions {
        allow_return_outside_function: true,
        ..Default::default()
    };
    let source_type = oxc::span::SourceType::ts().with_module(true);
    let parsed = oxc::parser::Parser::new(&allocator, source, source_type)
        .with_options(parse_opts)
        .parse();
    let program = if !parsed.diagnostics.is_empty() {
        let parsed_script = oxc::parser::Parser::new(
            &allocator,
            source,
            oxc::span::SourceType::ts().with_module(false),
        )
        .with_options(parse_opts)
        .parse();
        if !parsed_script.diagnostics.is_empty() || parsed_script.program.body.is_empty() {
            return (false, false);
        }
        parsed_script.program
    } else {
        if parsed.program.body.is_empty() {
            return (false, false);
        }
        parsed.program
    };

    analyze_tier0_slicesnp_program(&program)
}

pub(super) fn analyze_tier0_slicesnp_program(program: &oxc::ast::ast::Program<'_>) -> (bool, bool) {
    if program.body.is_empty() {
        return (false, false);
    }

    let mut top_scope = Vec::new();
    let compatible = program.body.iter().all(|stmt| is_supported_stmt(stmt, 0))
        && program
            .body
            .iter()
            .all(|stmt| check_scope_stmt(stmt, &mut top_scope, 0));
    let has_proto_mutation = compatible
        && program
            .body
            .iter()
            .any(stmt_has_top_level_guarded_proto_mutation);
    (compatible, has_proto_mutation)
}

fn is_supported_call(
    callee: &Expression<'_>,
    args: &[Argument<'_>],
    in_function_depth: usize,
) -> bool {
    for arg in args {
        let Some(arg_expr) = arg.as_expression() else {
            return false;
        };
        if !is_supported_expr(arg_expr, in_function_depth) {
            return false;
        }
    }

    match callee.get_inner_expression() {
        Expression::Identifier(id) => {
            let name = id.name.as_str();
            match name {
                "print" | "fetch" | "env" | "Number" | "String" | "Boolean" | "isNaN"
                | "isFinite" => true,
                "parseInt" | "parseFloat" => args.len() == 1,
                "test" => false,
                _ => !is_tier0_disallowed_ident(name),
            }
        }
        Expression::StaticMemberExpression(member) if !member.optional => {
            let prop = member.property.name.as_str();
            if let Some(chain) = static_member_chain(&member.object) {
                match chain.as_str() {
                    "console" => {
                        return matches!(prop, "log" | "info" | "warn" | "error");
                    }
                    "Math" => {
                        return matches!(
                            prop,
                            "abs"
                                | "floor"
                                | "ceil"
                                | "round"
                                | "trunc"
                                | "sign"
                                | "sqrt"
                                | "pow"
                                | "sin"
                                | "cos"
                                | "max"
                                | "min"
                                | "imul"
                                | "clz32"
                                | "fround"
                        );
                    }
                    "JSON" => {
                        return matches!(prop, "stringify" | "parse") && args.len() == 1;
                    }
                    "Object" => {
                        return matches!(prop, "keys" | "freeze") && args.len() == 1;
                    }
                    "Array" => {
                        return prop == "isArray" && args.len() == 1;
                    }
                    "fs" => {
                        return matches!(prop, "readFileSync" | "read") && args.len() == 1;
                    }
                    "WinterTC" => {
                        return prop == "route";
                    }
                    "Slice" => {
                        return prop == "numericLoop" && args.len() == 1;
                    }
                    "crypto.subtle" => {
                        return prop == "digest";
                    }
                    _ if is_tier0_disallowed_target_ident(
                        chain.split('.').next().unwrap_or(chain.as_str()),
                    ) =>
                    {
                        return false;
                    }
                    _ => {}
                }
            }
            if matches!(prop, "push" | "pop") {
                return matches!(
                    member.object.get_inner_expression(),
                    Expression::Identifier(id) if !is_tier0_disallowed_target_ident(id.name.as_str())
                );
            }
            if matches!(
                prop,
                "join"
                    | "includes"
                    | "map"
                    | "filter"
                    | "reduce"
                    | "toUpperCase"
                    | "toLowerCase"
                    | "trim"
                    | "split"
            ) {
                return is_supported_expr(&member.object, in_function_depth);
            }
            false
        }
        _ => false,
    }
}

fn is_supported_expr(expr: &Expression<'_>, in_function_depth: usize) -> bool {
    match expr.get_inner_expression() {
        Expression::NumericLiteral(_)
        | Expression::StringLiteral(_)
        | Expression::BooleanLiteral(_)
        | Expression::NullLiteral(_) => true,
        Expression::TemplateLiteral(tpl) => tpl.expressions.is_empty() && tpl.quasis.len() == 1,
        Expression::Identifier(id) => !is_tier0_disallowed_ident(id.name.as_str()),
        Expression::UnaryExpression(u) => {
            matches!(
                u.operator,
                UnaryOperator::UnaryNegation
                    | UnaryOperator::UnaryPlus
                    | UnaryOperator::LogicalNot
                    | UnaryOperator::BitwiseNot
                    | UnaryOperator::Typeof
            ) && is_supported_expr(&u.argument, in_function_depth)
        }
        Expression::UpdateExpression(u) => {
            matches!(
                &u.argument,
                SimpleAssignmentTarget::AssignmentTargetIdentifier(id)
                    if !is_tier0_disallowed_target_ident(id.name.as_str())
            )
        }
        Expression::BinaryExpression(b) => {
            matches!(
                b.operator,
                BinaryOperator::Addition
                    | BinaryOperator::Subtraction
                    | BinaryOperator::Multiplication
                    | BinaryOperator::Division
                    | BinaryOperator::Remainder
                    | BinaryOperator::Exponential
                    | BinaryOperator::Equality
                    | BinaryOperator::Inequality
                    | BinaryOperator::StrictEquality
                    | BinaryOperator::StrictInequality
                    | BinaryOperator::LessThan
                    | BinaryOperator::LessEqualThan
                    | BinaryOperator::GreaterThan
                    | BinaryOperator::GreaterEqualThan
                    | BinaryOperator::BitwiseAnd
                    | BinaryOperator::BitwiseOR
                    | BinaryOperator::BitwiseXOR
                    | BinaryOperator::ShiftLeft
                    | BinaryOperator::ShiftRight
                    | BinaryOperator::ShiftRightZeroFill
            ) && is_supported_expr(&b.left, in_function_depth)
                && is_supported_expr(&b.right, in_function_depth)
        }
        Expression::LogicalExpression(l) => {
            matches!(l.operator, LogicalOperator::And | LogicalOperator::Or)
                && is_supported_expr(&l.left, in_function_depth)
                && is_supported_expr(&l.right, in_function_depth)
        }
        Expression::ConditionalExpression(c) => {
            is_supported_expr(&c.test, in_function_depth)
                && is_supported_expr(&c.consequent, in_function_depth)
                && is_supported_expr(&c.alternate, in_function_depth)
        }
        Expression::AssignmentExpression(a) => {
            if !is_supported_expr(&a.right, in_function_depth) {
                return false;
            }
            let op_ok = matches!(
                a.operator,
                AssignmentOperator::Assign
                    | AssignmentOperator::Addition
                    | AssignmentOperator::Subtraction
                    | AssignmentOperator::Multiplication
                    | AssignmentOperator::Division
                    | AssignmentOperator::Remainder
            );
            if !op_ok {
                return false;
            }
            match &a.left {
                AssignmentTarget::AssignmentTargetIdentifier(id) => {
                    !is_tier0_disallowed_target_ident(id.name.as_str())
                }
                AssignmentTarget::StaticMemberExpression(m) if !m.optional => {
                    if a.operator != AssignmentOperator::Assign {
                        return false;
                    }
                    if let Some(chain) = static_member_chain(&m.object)
                        && (slice_sandbox::is_frozen_intrinsic(&chain)
                            || slice_sandbox::is_frozen_intrinsic(&format!(
                                "{}.{}",
                                chain, m.property.name
                            )))
                    {
                        return true;
                    }
                    matches!(
                        m.object.get_inner_expression(),
                        Expression::Identifier(id)
                            if !is_tier0_disallowed_target_ident(id.name.as_str())
                    )
                }
                AssignmentTarget::ComputedMemberExpression(m) if !m.optional => {
                    if a.operator == AssignmentOperator::Assign
                        && let Expression::StringLiteral(prop_lit) =
                            m.expression.get_inner_expression()
                        && let Some(chain) = static_member_chain(&m.object)
                        && (slice_sandbox::is_frozen_intrinsic(&chain)
                            || slice_sandbox::is_frozen_intrinsic(&format!(
                                "{}.{}",
                                chain, prop_lit.value
                            )))
                    {
                        return true;
                    }
                    let inner_idx = m.expression.get_inner_expression();
                    let non_array_idx = matches!(
                        inner_idx,
                        Expression::UnaryExpression(u) if u.operator == UnaryOperator::UnaryNegation
                    ) || matches!(
                        inner_idx,
                        Expression::Identifier(id)
                            if matches!(id.name.as_str(), "NaN" | "Infinity" | "undefined")
                    ) || matches!(
                        inner_idx,
                        Expression::NumericLiteral(nl) if nl.value < 0.0 || nl.value.fract() != 0.0
                    );
                    !non_array_idx
                        && a.operator == AssignmentOperator::Assign
                        && matches!(
                            m.object.get_inner_expression(),
                            Expression::Identifier(id)
                                if !is_tier0_disallowed_target_ident(id.name.as_str())
                        )
                        && is_supported_expr(&m.expression, in_function_depth)
                }
                _ => false,
            }
        }
        Expression::ArrayExpression(arr) => arr.elements.iter().all(|el| match el {
            ArrayExpressionElement::SpreadElement(_) | ArrayExpressionElement::Elision(_) => false,
            _ => el
                .as_expression()
                .is_some_and(|e| is_supported_expr(e, in_function_depth)),
        }),
        Expression::ObjectExpression(obj) => obj.properties.iter().all(|prop| match prop {
            ObjectPropertyKind::ObjectProperty(p) => {
                if p.method || p.computed {
                    return false;
                }
                let key_ok = matches!(
                    p.key,
                    PropertyKey::StaticIdentifier(_)
                        | PropertyKey::StringLiteral(_)
                        | PropertyKey::NumericLiteral(_)
                );
                key_ok && is_supported_expr(&p.value, in_function_depth)
            }
            ObjectPropertyKind::SpreadProperty(_) => false,
        }),
        Expression::ArrowFunctionExpression(arrow) => {
            // Disallow nested closures (arrow inside function/arrow) since Tier-0 has flat env_vars
            if arrow.r#async || in_function_depth > 0 || !params_are_simple(&arrow.params) {
                return false;
            }
            match &arrow.body {
                ArrowFunctionBody::FunctionBody(fb) => fb
                    .statements
                    .iter()
                    .all(|s| is_supported_stmt(s, in_function_depth + 1)),
                _ => arrow
                    .body
                    .as_expression()
                    .is_some_and(|e| is_supported_expr(e, in_function_depth + 1)),
            }
        }
        Expression::FunctionExpression(func) => {
            if func.r#async
                || func.generator
                || in_function_depth > 0
                || !params_are_simple(&func.params)
            {
                return false;
            }
            func.body.as_ref().is_some_and(|fb| {
                fb.statements
                    .iter()
                    .all(|s| is_supported_stmt(s, in_function_depth + 1))
            })
        }
        Expression::StaticMemberExpression(m) => {
            if m.optional {
                return false;
            }
            if let Expression::Identifier(obj_id) = &m.object
                && obj_id.name.as_str() == "Math"
            {
                return matches!(
                    m.property.name.as_str(),
                    "PI" | "E" | "SQRT2" | "LN2" | "LN10"
                );
            }
            is_supported_expr(&m.object, in_function_depth)
        }
        Expression::ComputedMemberExpression(m) => {
            !m.optional
                && is_supported_expr(&m.object, in_function_depth)
                && is_supported_expr(&m.expression, in_function_depth)
        }
        Expression::CallExpression(call) => {
            !call.optional && is_supported_call(&call.callee, &call.arguments, in_function_depth)
        }
        _ => false,
    }
}

fn is_supported_var_decl(
    vd: &oxc::ast::ast::VariableDeclaration<'_>,
    in_function_depth: usize,
) -> bool {
    if vd.declarations.len() != 1 {
        return false;
    }
    let decl = &vd.declarations[0];
    let BindingPattern::BindingIdentifier(id) = &decl.id else {
        return false;
    };
    if is_tier0_disallowed_target_ident(id.name.as_str()) {
        return false;
    }
    decl.init
        .as_ref()
        .is_none_or(|init| is_supported_expr(init, in_function_depth))
}

fn is_supported_stmt(stmt: &Statement<'_>, in_function_depth: usize) -> bool {
    match stmt {
        Statement::EmptyStatement(_)
        | Statement::BreakStatement(_)
        | Statement::ContinueStatement(_)
        | Statement::TSInterfaceDeclaration(_)
        | Statement::TSTypeAliasDeclaration(_) => true,
        Statement::BlockStatement(b) => b
            .body
            .iter()
            .all(|s| is_supported_stmt(s, in_function_depth)),
        Statement::VariableDeclaration(vd) => is_supported_var_decl(vd, in_function_depth),
        Statement::FunctionDeclaration(fd) => {
            if fd.r#async || fd.generator || in_function_depth > 0 || !params_are_simple(&fd.params)
            {
                return false;
            }
            fd.body.as_ref().is_some_and(|fb| {
                fb.statements
                    .iter()
                    .all(|s| is_supported_stmt(s, in_function_depth + 1))
            })
        }
        Statement::ExpressionStatement(es) => is_supported_expr(&es.expression, in_function_depth),
        Statement::IfStatement(ifs) => {
            is_supported_expr(&ifs.test, in_function_depth)
                && is_supported_stmt(&ifs.consequent, in_function_depth)
                && ifs
                    .alternate
                    .as_ref()
                    .is_none_or(|alt| is_supported_stmt(alt, in_function_depth))
        }
        Statement::WhileStatement(ws) => {
            (in_function_depth > 0
                || matches!(
                    ws.test.without_parentheses(),
                    Expression::BooleanLiteral(b) if b.value
                ))
                && is_supported_expr(&ws.test, in_function_depth)
                && is_supported_stmt(&ws.body, in_function_depth)
        }
        Statement::ForStatement(fs) => {
            let init_ok = match &fs.init {
                None => true,
                Some(ForStatementInit::VariableDeclaration(vd)) => {
                    is_supported_var_decl(vd, in_function_depth)
                }
                Some(other) => other
                    .as_expression()
                    .is_some_and(|e| is_supported_expr(e, in_function_depth)),
            };
            let test_ok = fs
                .test
                .as_ref()
                .is_none_or(|t| is_supported_expr(t, in_function_depth));
            let update_ok = fs
                .update
                .as_ref()
                .is_none_or(|u| is_supported_expr(u, in_function_depth));
            init_ok && test_ok && update_ok && is_supported_stmt(&fs.body, in_function_depth)
        }
        Statement::ReturnStatement(rs) => rs
            .argument
            .as_ref()
            .is_none_or(|arg| is_supported_expr(arg, in_function_depth)),
        Statement::ImportDeclaration(imp) => {
            in_function_depth == 0
                && !slice_cas::is_builtin_module(imp.source.value.as_str())
                && !slice_cas::is_jsx_runtime_builtin(imp.source.value.as_str())
                && imp.specifiers.as_ref().is_none_or(|specs| {
                    specs.iter().all(|s| match s {
                        oxc::ast::ast::ImportDeclarationSpecifier::ImportSpecifier(named) => {
                            named.imported.name() == named.local.name
                        }
                        oxc::ast::ast::ImportDeclarationSpecifier::ImportDefaultSpecifier(_) => {
                            true
                        }
                        oxc::ast::ast::ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => {
                            false
                        }
                    })
                })
        }
        Statement::ExportDeclaration(exp) => match &exp.declaration {
            Declaration::VariableDeclaration(vd) => is_supported_var_decl(vd, in_function_depth),
            Declaration::FunctionDeclaration(fd) => {
                !fd.r#async
                    && !fd.generator
                    && in_function_depth == 0
                    && params_are_simple(&fd.params)
                    && fd.body.as_ref().is_some_and(|fb| {
                        fb.statements
                            .iter()
                            .all(|s| is_supported_stmt(s, in_function_depth + 1))
                    })
            }
            Declaration::TSInterfaceDeclaration(_) | Declaration::TSTypeAliasDeclaration(_) => true,
            _ => false,
        },
        _ => false,
    }
}
