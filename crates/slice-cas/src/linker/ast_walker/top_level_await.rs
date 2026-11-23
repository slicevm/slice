//! Top-Level `await` Detector and Async IIFE Script Wrapper (`has_top_level_await`, `wrap_top_level_await_script`).

/// Detects via Oxc AST whether `code` contains any top-level `await` expression or `for await`
/// loop outside nested function/method scopes.
#[inline]
pub fn has_top_level_await(code: &str) -> bool {
    detect_top_level_await_and_last_expr_span(code).is_some()
}

/// Single-pass Oxc AST detector that returns `Some(last_expr_span)` if `code` contains any
/// top-level `await` expression or `for await` loop outside nested functions, where
/// `last_expr_span` is `Some((start, end))` when the final top-level statement is an
/// `ExpressionStatement`.
fn detect_top_level_await_and_last_expr_span(code: &str) -> Option<Option<(usize, usize)>> {
    use oxc::ast::ast::{Declaration, Expression, ForStatementInit, ObjectPropertyKind, Statement};

    let bytes = code.as_bytes();
    if memchr::memmem::find(bytes, b"await ").is_none()
        && memchr::memmem::find(bytes, b"await(").is_none()
    {
        return None;
    }

    let allocator = oxc::allocator::Allocator::default();
    let parse_opts = oxc::parser::ParseOptions {
        allow_return_outside_function: true,
        ..Default::default()
    };
    let source_type = oxc::span::SourceType::ts().with_module(true);
    let parser_ret = oxc::parser::Parser::new(&allocator, code, source_type)
        .with_options(parse_opts)
        .parse();
    if !parser_ret.diagnostics.is_empty() {
        return None;
    }

    fn expr_has_tla(expr: &Expression<'_>) -> bool {
        match expr {
            Expression::AwaitExpression(_) => true,
            Expression::CallExpression(c) => {
                expr_has_tla(&c.callee)
                    || c.arguments
                        .iter()
                        .any(|a| a.as_expression().is_some_and(expr_has_tla))
            }
            Expression::NewExpression(n) => {
                expr_has_tla(&n.callee)
                    || n.arguments
                        .iter()
                        .any(|a| a.as_expression().is_some_and(expr_has_tla))
            }
            Expression::StaticMemberExpression(m) => expr_has_tla(&m.object),
            Expression::ComputedMemberExpression(m) => {
                expr_has_tla(&m.object) || expr_has_tla(&m.expression)
            }
            Expression::BinaryExpression(b) => expr_has_tla(&b.left) || expr_has_tla(&b.right),
            Expression::LogicalExpression(l) => expr_has_tla(&l.left) || expr_has_tla(&l.right),
            Expression::UnaryExpression(u) => expr_has_tla(&u.argument),
            Expression::ConditionalExpression(c) => {
                expr_has_tla(&c.test) || expr_has_tla(&c.consequent) || expr_has_tla(&c.alternate)
            }
            Expression::AssignmentExpression(a) => expr_has_tla(&a.right),
            Expression::SequenceExpression(s) => s.expressions.iter().any(expr_has_tla),
            Expression::ParenthesizedExpression(p) => expr_has_tla(&p.expression),
            Expression::ArrayExpression(arr) => arr.elements.iter().any(|el| {
                if let Some(e) = el.as_expression() {
                    expr_has_tla(e)
                } else if let oxc::ast::ast::ArrayExpressionElement::SpreadElement(sp) = el {
                    expr_has_tla(&sp.argument)
                } else {
                    false
                }
            }),
            Expression::ObjectExpression(obj) => obj.properties.iter().any(|prop| match prop {
                ObjectPropertyKind::ObjectProperty(p) => {
                    (p.computed && p.key.as_expression().is_some_and(expr_has_tla))
                        || expr_has_tla(&p.value)
                }
                ObjectPropertyKind::SpreadProperty(sp) => expr_has_tla(&sp.argument),
            }),
            Expression::TemplateLiteral(t) => t.expressions.iter().any(expr_has_tla),
            _ => false,
        }
    }

    fn stmt_has_tla(stmt: &Statement<'_>) -> bool {
        match stmt {
            Statement::VariableDeclaration(vd) => vd
                .declarations
                .iter()
                .any(|d| d.init.as_ref().is_some_and(expr_has_tla)),
            Statement::ExpressionStatement(es) => expr_has_tla(&es.expression),
            Statement::BlockStatement(bs) => bs.body.iter().any(stmt_has_tla),
            Statement::IfStatement(ifs) => {
                expr_has_tla(&ifs.test)
                    || stmt_has_tla(&ifs.consequent)
                    || ifs.alternate.as_ref().is_some_and(stmt_has_tla)
            }
            Statement::ForOfStatement(fo) => {
                fo.r#await || expr_has_tla(&fo.right) || stmt_has_tla(&fo.body)
            }
            Statement::ForStatement(fs) => {
                let init_tla = fs.init.as_ref().is_some_and(|i| match i {
                    ForStatementInit::VariableDeclaration(vd) => vd
                        .declarations
                        .iter()
                        .any(|d| d.init.as_ref().is_some_and(expr_has_tla)),
                    _ => i.as_expression().is_some_and(expr_has_tla),
                });
                init_tla
                    || fs.test.as_ref().is_some_and(expr_has_tla)
                    || fs.update.as_ref().is_some_and(expr_has_tla)
                    || stmt_has_tla(&fs.body)
            }
            Statement::WhileStatement(ws) => expr_has_tla(&ws.test) || stmt_has_tla(&ws.body),
            Statement::TryStatement(ts) => {
                ts.block.body.iter().any(stmt_has_tla)
                    || ts
                        .handler
                        .as_ref()
                        .is_some_and(|h| h.body.body.iter().any(stmt_has_tla))
                    || ts
                        .finalizer
                        .as_ref()
                        .is_some_and(|f| f.body.iter().any(stmt_has_tla))
            }
            Statement::ExportDeclaration(exp) => match &exp.declaration {
                Declaration::VariableDeclaration(vd) => vd
                    .declarations
                    .iter()
                    .any(|d| d.init.as_ref().is_some_and(expr_has_tla)),
                _ => false,
            },
            _ => false,
        }
    }

    if !parser_ret.program.body.iter().any(stmt_has_tla) {
        return None;
    }

    let last_expr_span =
        if let Some(Statement::ExpressionStatement(es)) = parser_ret.program.body.last() {
            Some((es.span.start as usize, es.span.end as usize))
        } else {
            None
        };
    Some(last_expr_span)
}

/// When `code` contains a top-level `await`, wraps it into an async IIFE that preserves
/// the completion value of a trailing top-level `ExpressionStatement` (returning it via
/// `globalThis.__slice_tla_v` / `globalThis.__slice_tla_st`) so `boa_engine` can evaluate
/// top-level `await` in `Slice.evalSandbox`, `slice mcp` `slice_eval_ts`, and single-file scripts.
pub fn wrap_top_level_await_script(code: &str) -> Option<String> {
    let last_expr_span = detect_top_level_await_and_last_expr_span(code)?;
    let mut out = String::with_capacity(code.len() + 220);
    out.push_str(
        "globalThis.__slice_tla_st = 0;\nglobalThis.__slice_tla_v = undefined;\n(async function() {\n",
    );
    if let Some((start, end)) = last_expr_span {
        if start <= end && end <= code.len() {
            out.push_str(&code[..start]);
            out.push_str("return ");
            out.push_str(&code[start..end]);
            out.push_str(&code[end..]);
        } else {
            out.push_str(code);
        }
    } else {
        out.push_str(code);
    }
    out.push_str(
        "\n})().then(function(__v) { globalThis.__slice_tla_st = 1; globalThis.__slice_tla_v = __v; }, function(__e) { globalThis.__slice_tla_st = 2; globalThis.__slice_tla_v = __e; });\n",
    );
    Some(out)
}
