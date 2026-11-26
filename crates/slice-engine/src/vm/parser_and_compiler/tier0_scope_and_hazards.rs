//! Tier-0 Identifier Denylists, Lexical Scope Shadowing Checks, and Forced-Tier-0 Hazard Visitors.

use super::parser_stmt::is_tier0_guarded_proto_target;
use oxc::ast::ast::{
    ArrowFunctionBody, AssignmentOperator, AssignmentTarget, BindingPattern, ChainExpression,
    Declaration, Expression, ForStatementInit, FormalParameters, Statement,
};
use oxc::ast_visit::{Visit, walk};

pub(super) fn params_are_simple(params: &FormalParameters<'_>) -> bool {
    if params.rest.is_some() {
        return false;
    }
    params
        .items
        .iter()
        .all(|p| matches!(p.pattern, BindingPattern::BindingIdentifier(_)))
}

pub(super) fn static_member_chain(expr: &Expression<'_>) -> Option<String> {
    match expr.get_inner_expression() {
        Expression::Identifier(id) => Some(id.name.to_string()),
        Expression::StaticMemberExpression(m) if !m.optional => {
            let prefix = static_member_chain(&m.object)?;
            Some(format!("{}.{}", prefix, m.property.name.as_str()))
        }
        Expression::ComputedMemberExpression(m) if !m.optional => {
            if let Expression::StringLiteral(s) = m.expression.get_inner_expression() {
                let prefix = static_member_chain(&m.object)?;
                Some(format!("{}.{}", prefix, s.value.display()))
            } else {
                None
            }
        }
        _ => None,
    }
}

#[inline]
pub(super) fn is_tier0_disallowed_ident(name: &str) -> bool {
    matches!(
        name,
        "arguments"
            | "globalThis"
            | "window"
            | "self"
            | "process"
            | "Buffer"
            | "WebAssembly"
            | "Symbol"
            | "Proxy"
            | "Reflect"
            | "Object"
            | "Array"
            | "String"
            | "Number"
            | "Boolean"
            | "Function"
            | "JSON"
            | "Math"
            | "Date"
            | "RegExp"
            | "Promise"
            | "Map"
            | "Set"
            | "WeakMap"
            | "WeakSet"
            | "WeakRef"
            | "FinalizationRegistry"
            | "Error"
            | "TypeError"
            | "RangeError"
            | "SyntaxError"
            | "ReferenceError"
            | "URIError"
            | "EvalError"
            | "AggregateError"
            | "DOMException"
            | "BigInt"
            | "ArrayBuffer"
            | "SharedArrayBuffer"
            | "DataView"
            | "Atomics"
            | "Uint8Array"
            | "Uint8ClampedArray"
            | "Int8Array"
            | "Uint16Array"
            | "Int16Array"
            | "Uint32Array"
            | "Int32Array"
            | "Float32Array"
            | "Float64Array"
            | "BigInt64Array"
            | "BigUint64Array"
            | "Intl"
            | "URL"
            | "URLSearchParams"
            | "URLPattern"
            | "Headers"
            | "Request"
            | "Response"
            | "TextEncoder"
            | "TextDecoder"
            | "TextEncoderStream"
            | "TextDecoderStream"
            | "AbortController"
            | "AbortSignal"
            | "Event"
            | "CustomEvent"
            | "MessageEvent"
            | "CloseEvent"
            | "ErrorEvent"
            | "EventTarget"
            | "WebSocket"
            | "Worker"
            | "MessageChannel"
            | "MessagePort"
            | "BroadcastChannel"
            | "Blob"
            | "File"
            | "FormData"
            | "ReadableStream"
            | "WritableStream"
            | "TransformStream"
            | "CompressionStream"
            | "DecompressionStream"
            | "ByteLengthQueuingStrategy"
            | "CountQueuingStrategy"
            | "navigator"
            | "location"
            | "crypto"
            | "performance"
            | "console"
            | "fs"
            | "Slice"
            | "WinterTC"
            | "Deno"
            | "Bun"
            | "require"
            | "module"
            | "exports"
            | "__dirname"
            | "__filename"
            | "fetch"
            | "setTimeout"
            | "setInterval"
            | "setImmediate"
            | "clearTimeout"
            | "clearInterval"
            | "clearImmediate"
            | "queueMicrotask"
            | "structuredClone"
            | "btoa"
            | "atob"
            | "escape"
            | "unescape"
            | "eval"
            | "parseInt"
            | "parseFloat"
            | "isNaN"
            | "isFinite"
            | "encodeURI"
            | "decodeURI"
            | "encodeURIComponent"
            | "decodeURIComponent"
            | "describe"
            | "it"
            | "expect"
            | "beforeEach"
            | "afterEach"
    )
}

#[inline]
pub(super) fn is_tier0_disallowed_target_ident(name: &str) -> bool {
    is_tier0_disallowed_ident(name)
        || matches!(
            name,
            "undefined" | "NaN" | "Infinity" | "print" | "env" | "test"
        )
}

fn params_to_scope<'a>(params: &'a FormalParameters<'a>) -> Vec<&'a str> {
    let mut scope = Vec::with_capacity(params.items.len());
    for p in &params.items {
        if let BindingPattern::BindingIdentifier(id) = &p.pattern {
            scope.push(id.name.as_str());
        }
    }
    scope
}

pub(super) fn check_scope_stmt<'a>(
    stmt: &'a Statement<'a>,
    scope: &mut Vec<&'a str>,
    block_depth: usize,
) -> bool {
    match stmt {
        Statement::VariableDeclaration(vd) => {
            if block_depth > 0 && vd.kind.is_var() {
                return false;
            }
            for decl in &vd.declarations {
                if let BindingPattern::BindingIdentifier(id) = &decl.id {
                    let name = id.name.as_str();
                    if scope.contains(&name) && (!vd.kind.is_var() || block_depth > 0) {
                        return false;
                    }
                    if !scope.contains(&name) {
                        scope.push(name);
                    }
                }
                if let Some(init) = &decl.init
                    && !check_scope_expr(init)
                {
                    return false;
                }
            }
            true
        }
        Statement::BlockStatement(b) => {
            let mark = scope.len();
            let ok = b
                .body
                .iter()
                .all(|s| check_scope_stmt(s, scope, block_depth + 1));
            scope.truncate(mark);
            ok
        }
        Statement::IfStatement(ifs) => {
            let mark = scope.len();
            let then_ok = check_scope_stmt(&ifs.consequent, scope, block_depth + 1);
            scope.truncate(mark);
            let else_ok = ifs
                .alternate
                .as_ref()
                .is_none_or(|alt| check_scope_stmt(alt, scope, block_depth + 1));
            scope.truncate(mark);
            then_ok && else_ok
        }
        Statement::WhileStatement(ws) => {
            let mark = scope.len();
            let ok = check_scope_stmt(&ws.body, scope, block_depth + 1);
            scope.truncate(mark);
            ok
        }
        Statement::ForStatement(fs) => {
            let mark = scope.len();
            let init_ok = match &fs.init {
                Some(ForStatementInit::VariableDeclaration(vd)) => {
                    if vd.kind.is_var() && block_depth > 0 {
                        false
                    } else {
                        let mut ok = true;
                        for decl in &vd.declarations {
                            if let BindingPattern::BindingIdentifier(id) = &decl.id {
                                let name = id.name.as_str();
                                if scope.contains(&name) && !vd.kind.is_var() {
                                    ok = false;
                                    break;
                                }
                                if !scope.contains(&name) {
                                    scope.push(name);
                                }
                            }
                        }
                        ok
                    }
                }
                _ => true,
            };
            let body_ok = init_ok && check_scope_stmt(&fs.body, scope, block_depth + 1);
            scope.truncate(mark);
            body_ok
        }
        Statement::FunctionDeclaration(fd) => fd.body.as_ref().is_none_or(|fb| {
            let mut fn_scope = params_to_scope(&fd.params);
            fb.statements
                .iter()
                .all(|s| check_scope_stmt(s, &mut fn_scope, 0))
        }),
        Statement::ExportDeclaration(exp) => match &exp.declaration {
            Declaration::VariableDeclaration(vd) => {
                for decl in &vd.declarations {
                    if let BindingPattern::BindingIdentifier(id) = &decl.id {
                        let name = id.name.as_str();
                        if scope.contains(&name) && !vd.kind.is_var() {
                            return false;
                        }
                        if !scope.contains(&name) {
                            scope.push(name);
                        }
                    }
                    if let Some(init) = &decl.init
                        && !check_scope_expr(init)
                    {
                        return false;
                    }
                }
                true
            }
            Declaration::FunctionDeclaration(fd) => fd.body.as_ref().is_none_or(|fb| {
                let mut fn_scope = params_to_scope(&fd.params);
                fb.statements
                    .iter()
                    .all(|s| check_scope_stmt(s, &mut fn_scope, 0))
            }),
            _ => true,
        },
        Statement::ExpressionStatement(es) => check_scope_expr(&es.expression),
        Statement::ReturnStatement(rs) => rs.argument.as_ref().is_none_or(check_scope_expr),
        _ => true,
    }
}

fn check_scope_expr<'a>(expr: &'a Expression<'a>) -> bool {
    match expr {
        Expression::ArrowFunctionExpression(arrow) => match &arrow.body {
            ArrowFunctionBody::FunctionBody(fb) => {
                let mut fn_scope = params_to_scope(&arrow.params);
                fb.statements
                    .iter()
                    .all(|s| check_scope_stmt(s, &mut fn_scope, 0))
            }
            _ => true,
        },
        Expression::FunctionExpression(func) => func.body.as_ref().is_none_or(|fb| {
            let mut fn_scope = params_to_scope(&func.params);
            fb.statements
                .iter()
                .all(|s| check_scope_stmt(s, &mut fn_scope, 0))
        }),
        _ => true,
    }
}

pub(super) fn stmt_has_top_level_guarded_proto_mutation(stmt: &Statement<'_>) -> bool {
    let Statement::ExpressionStatement(es) = stmt else {
        return false;
    };
    let Expression::AssignmentExpression(a) = es.expression.get_inner_expression() else {
        return false;
    };
    if a.operator != AssignmentOperator::Assign {
        return false;
    }
    match &a.left {
        AssignmentTarget::StaticMemberExpression(m) if !m.optional => {
            static_member_chain(&m.object)
                .is_some_and(|chain| is_tier0_guarded_proto_target(&chain))
        }
        AssignmentTarget::ComputedMemberExpression(m) if !m.optional => {
            matches!(
                m.expression.get_inner_expression(),
                Expression::StringLiteral(_)
            ) && static_member_chain(&m.object)
                .is_some_and(|chain| is_tier0_guarded_proto_target(&chain))
        }
        _ => false,
    }
}

pub(super) fn program_has_tier0_unsupported_mutation_syntax(
    program: &oxc::ast::ast::Program<'_>,
) -> bool {
    struct ForcedTier0HazardVisitor {
        found: bool,
    }

    impl<'a> Visit<'a> for ForcedTier0HazardVisitor {
        #[inline]
        fn visit_assignment_target(&mut self, it: &AssignmentTarget<'a>) {
            if self.found {
                return;
            }
            if matches!(
                it,
                AssignmentTarget::ObjectAssignmentTarget(_)
                    | AssignmentTarget::ArrayAssignmentTarget(_)
            ) {
                self.found = true;
                return;
            }
            walk::walk_assignment_target(self, it);
        }

        #[inline]
        fn visit_chain_expression(&mut self, _it: &ChainExpression<'a>) {
            self.found = true;
        }

        #[inline]
        fn visit_binding_pattern(&mut self, it: &BindingPattern<'a>) {
            if self.found {
                return;
            }
            if matches!(
                it,
                BindingPattern::ObjectPattern(_) | BindingPattern::ArrayPattern(_)
            ) {
                self.found = true;
                return;
            }
            walk::walk_binding_pattern(self, it);
        }
    }

    let mut v = ForcedTier0HazardVisitor { found: false };
    v.visit_program(program);
    v.found
}

pub(super) fn source_has_tier0_unsupported_mutation_syntax(source: &str) -> bool {
    let allocator = oxc::allocator::Allocator::default();
    let parse_opts = oxc::parser::ParseOptions {
        allow_return_outside_function: true,
        ..Default::default()
    };
    let parsed = oxc::parser::Parser::new(
        &allocator,
        source,
        oxc::span::SourceType::ts().with_module(true),
    )
    .with_options(parse_opts)
    .parse();
    if parsed.diagnostics.is_empty() {
        return program_has_tier0_unsupported_mutation_syntax(&parsed.program);
    }
    let parsed_script = oxc::parser::Parser::new(
        &allocator,
        source,
        oxc::span::SourceType::ts().with_module(false),
    )
    .with_options(parse_opts)
    .parse();
    if parsed_script.diagnostics.is_empty() {
        return program_has_tier0_unsupported_mutation_syntax(&parsed_script.program);
    }
    true
}
