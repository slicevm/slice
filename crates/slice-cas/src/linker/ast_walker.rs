//! Oxc AST Statement/Expression Walkers, Top-Level `await` Detector, and Span Slicer.

pub fn collect_binding_pattern_names(
    pat: &oxc::ast::ast::BindingPattern<'_>,
    out: &mut Vec<(String, String)>,
) {
    use oxc::ast::ast::BindingPattern;
    match pat {
        BindingPattern::BindingIdentifier(ident) => {
            let n = ident.name.to_string();
            out.push((n.clone(), n));
        }
        BindingPattern::ObjectPattern(obj) => {
            for prop in &obj.properties {
                collect_binding_pattern_names(&prop.value, out);
            }
            if let Some(rest) = &obj.rest {
                collect_binding_pattern_names(&rest.argument, out);
            }
        }
        BindingPattern::ArrayPattern(arr) => {
            for elem in arr.elements.iter().flatten() {
                collect_binding_pattern_names(elem, out);
            }
            if let Some(rest) = &arr.rest {
                collect_binding_pattern_names(&rest.argument, out);
            }
        }
        BindingPattern::AssignmentPattern(assign) => {
            collect_binding_pattern_names(&assign.left, out);
        }
    }
}

pub fn walk_ast_expressions<'a, F>(program: &'a oxc::ast::ast::Program<'a>, visit_expr: &mut F)
where
    F: FnMut(&'a oxc::ast::ast::Expression<'a>),
{
    use oxc::ast::ast::{
        BindingPattern, ClassElement, Declaration, ExportDefaultDeclarationKind, Expression,
        ForStatementInit, ObjectPropertyKind, Statement,
    };

    fn walk_pat<'a, F>(pat: &'a BindingPattern<'a>, visit: &mut F)
    where
        F: FnMut(&'a oxc::ast::ast::Expression<'a>),
    {
        match pat {
            BindingPattern::ObjectPattern(obj) => {
                for p in &obj.properties {
                    walk_pat(&p.value, visit);
                }
                if let Some(r) = &obj.rest {
                    walk_pat(&r.argument, visit);
                }
            }
            BindingPattern::ArrayPattern(arr) => {
                for el in arr.elements.iter().flatten() {
                    walk_pat(el, visit);
                }
                if let Some(r) = &arr.rest {
                    walk_pat(&r.argument, visit);
                }
            }
            BindingPattern::AssignmentPattern(assign) => {
                walk_pat(&assign.left, visit);
                walk_expr(&assign.right, visit);
            }
            BindingPattern::BindingIdentifier(_) => {}
        }
    }

    fn walk_class<'a, F>(cls: &'a oxc::ast::ast::Class<'a>, visit: &mut F)
    where
        F: FnMut(&'a oxc::ast::ast::Expression<'a>),
    {
        for el in &cls.body.body {
            match el {
                ClassElement::MethodDefinition(md) => {
                    for p in &md.value.params.items {
                        walk_pat(&p.pattern, visit);
                        if let Some(init) = &p.initializer {
                            walk_expr(init, visit);
                        }
                    }
                    if let Some(body) = &md.value.body {
                        for s in &body.statements {
                            walk_stmt(s, visit);
                        }
                    }
                }
                ClassElement::PropertyDefinition(pd) => {
                    if let Some(v) = &pd.value {
                        walk_expr(v, visit);
                    }
                }
                ClassElement::StaticBlock(sb) => {
                    for s in &sb.body {
                        walk_stmt(s, visit);
                    }
                }
                _ => {}
            }
        }
    }

    fn walk_expr<'a, F>(expr: &'a Expression<'a>, visit: &mut F)
    where
        F: FnMut(&'a oxc::ast::ast::Expression<'a>),
    {
        visit(expr);
        match expr {
            Expression::CallExpression(call) => {
                walk_expr(&call.callee, visit);
                for arg in &call.arguments {
                    if let Some(e) = arg.as_expression() {
                        walk_expr(e, visit);
                    }
                }
            }
            Expression::ImportExpression(imp) => {
                walk_expr(&imp.source, visit);
            }
            Expression::NewExpression(ne) => {
                walk_expr(&ne.callee, visit);
                for arg in &ne.arguments {
                    if let Some(e) = arg.as_expression() {
                        walk_expr(e, visit);
                    }
                }
            }
            Expression::StaticMemberExpression(m) => walk_expr(&m.object, visit),
            Expression::ComputedMemberExpression(m) => {
                walk_expr(&m.object, visit);
                walk_expr(&m.expression, visit);
            }
            Expression::PrivateFieldExpression(m) => walk_expr(&m.object, visit),
            Expression::BinaryExpression(b) => {
                walk_expr(&b.left, visit);
                walk_expr(&b.right, visit);
            }
            Expression::LogicalExpression(l) => {
                walk_expr(&l.left, visit);
                walk_expr(&l.right, visit);
            }
            Expression::UnaryExpression(u) => walk_expr(&u.argument, visit),
            Expression::UpdateExpression(_) => {}
            Expression::ConditionalExpression(c) => {
                walk_expr(&c.test, visit);
                walk_expr(&c.consequent, visit);
                walk_expr(&c.alternate, visit);
            }
            Expression::AssignmentExpression(a) => {
                match &a.left {
                    oxc::ast::ast::AssignmentTarget::StaticMemberExpression(m) => {
                        walk_expr(&m.object, visit);
                    }
                    oxc::ast::ast::AssignmentTarget::ComputedMemberExpression(m) => {
                        walk_expr(&m.object, visit);
                        walk_expr(&m.expression, visit);
                    }
                    oxc::ast::ast::AssignmentTarget::PrivateFieldExpression(m) => {
                        walk_expr(&m.object, visit);
                    }
                    _ => {}
                }
                walk_expr(&a.right, visit);
            }
            Expression::SequenceExpression(seq) => {
                for e in &seq.expressions {
                    walk_expr(e, visit);
                }
            }
            Expression::ParenthesizedExpression(p) => walk_expr(&p.expression, visit),
            Expression::AwaitExpression(aw) => walk_expr(&aw.argument, visit),
            Expression::YieldExpression(y) => {
                if let Some(arg) = &y.argument {
                    walk_expr(arg, visit);
                }
            }
            Expression::ArrowFunctionExpression(arrow) => {
                for p in &arrow.params.items {
                    walk_pat(&p.pattern, visit);
                    if let Some(init) = &p.initializer {
                        walk_expr(init, visit);
                    }
                }
                if let oxc::ast::ast::ArrowFunctionBody::FunctionBody(fb) = &arrow.body {
                    for s in &fb.statements {
                        walk_stmt(s, visit);
                    }
                } else if let Some(e) = arrow.body.as_expression() {
                    walk_expr(e, visit);
                }
            }
            Expression::FunctionExpression(fe) => {
                for p in &fe.params.items {
                    walk_pat(&p.pattern, visit);
                    if let Some(init) = &p.initializer {
                        walk_expr(init, visit);
                    }
                }
                if let Some(body) = &fe.body {
                    for s in &body.statements {
                        walk_stmt(s, visit);
                    }
                }
            }
            Expression::ClassExpression(ce) => walk_class(ce, visit),
            Expression::ObjectExpression(obj) => {
                for prop in &obj.properties {
                    match prop {
                        ObjectPropertyKind::ObjectProperty(p) => {
                            if p.computed {
                                if let Some(ke) = p.key.as_expression() {
                                    walk_expr(ke, visit);
                                }
                            }
                            walk_expr(&p.value, visit);
                        }
                        ObjectPropertyKind::SpreadProperty(sp) => walk_expr(&sp.argument, visit),
                    }
                }
            }
            Expression::ArrayExpression(arr) => {
                for el in &arr.elements {
                    if let Some(e) = el.as_expression() {
                        walk_expr(e, visit);
                    } else if let oxc::ast::ast::ArrayExpressionElement::SpreadElement(sp) = el {
                        walk_expr(&sp.argument, visit);
                    }
                }
            }
            Expression::TemplateLiteral(tpl) => {
                for e in &tpl.expressions {
                    walk_expr(e, visit);
                }
            }
            Expression::TaggedTemplateExpression(tt) => {
                walk_expr(&tt.tag, visit);
                for e in &tt.quasi.expressions {
                    walk_expr(e, visit);
                }
            }
            Expression::ChainExpression(ch) => match &ch.expression {
                oxc::ast::ast::ChainElement::CallExpression(call) => {
                    walk_expr(&call.callee, visit);
                    for arg in &call.arguments {
                        if let Some(e) = arg.as_expression() {
                            walk_expr(e, visit);
                        }
                    }
                }
                oxc::ast::ast::ChainElement::StaticMemberExpression(m) => {
                    walk_expr(&m.object, visit);
                }
                oxc::ast::ast::ChainElement::ComputedMemberExpression(m) => {
                    walk_expr(&m.object, visit);
                    walk_expr(&m.expression, visit);
                }
                oxc::ast::ast::ChainElement::PrivateFieldExpression(m) => {
                    walk_expr(&m.object, visit);
                }
                oxc::ast::ast::ChainElement::TSNonNullExpression(n) => {
                    walk_expr(&n.expression, visit);
                }
            },
            _ => {}
        }
    }

    fn walk_decl<'a, F>(decl: &'a Declaration<'a>, visit: &mut F)
    where
        F: FnMut(&'a oxc::ast::ast::Expression<'a>),
    {
        match decl {
            Declaration::VariableDeclaration(vd) => {
                for d in &vd.declarations {
                    walk_pat(&d.id, visit);
                    if let Some(init) = &d.init {
                        walk_expr(init, visit);
                    }
                }
            }
            Declaration::FunctionDeclaration(fd) => {
                for p in &fd.params.items {
                    walk_pat(&p.pattern, visit);
                    if let Some(init) = &p.initializer {
                        walk_expr(init, visit);
                    }
                }
                if let Some(body) = &fd.body {
                    for s in &body.statements {
                        walk_stmt(s, visit);
                    }
                }
            }
            Declaration::ClassDeclaration(cd) => walk_class(cd, visit),
            _ => {}
        }
    }

    fn walk_stmt<'a, F>(stmt: &'a Statement<'a>, visit: &mut F)
    where
        F: FnMut(&'a oxc::ast::ast::Expression<'a>),
    {
        match stmt {
            Statement::VariableDeclaration(vd) => {
                for d in &vd.declarations {
                    walk_pat(&d.id, visit);
                    if let Some(init) = &d.init {
                        walk_expr(init, visit);
                    }
                }
            }
            Statement::FunctionDeclaration(fd) => {
                for p in &fd.params.items {
                    walk_pat(&p.pattern, visit);
                    if let Some(init) = &p.initializer {
                        walk_expr(init, visit);
                    }
                }
                if let Some(body) = &fd.body {
                    for s in &body.statements {
                        walk_stmt(s, visit);
                    }
                }
            }
            Statement::ClassDeclaration(cd) => walk_class(cd, visit),
            Statement::ExpressionStatement(es) => walk_expr(&es.expression, visit),
            Statement::BlockStatement(bs) => {
                for s in &bs.body {
                    walk_stmt(s, visit);
                }
            }
            Statement::ReturnStatement(rs) => {
                if let Some(arg) = &rs.argument {
                    walk_expr(arg, visit);
                }
            }
            Statement::ThrowStatement(ts) => walk_expr(&ts.argument, visit),
            Statement::IfStatement(ifs) => {
                walk_expr(&ifs.test, visit);
                walk_stmt(&ifs.consequent, visit);
                if let Some(alt) = &ifs.alternate {
                    walk_stmt(alt, visit);
                }
            }
            Statement::SwitchStatement(sw) => {
                walk_expr(&sw.discriminant, visit);
                for c in &sw.cases {
                    if let Some(t) = &c.test {
                        walk_expr(t, visit);
                    }
                    for s in &c.consequent {
                        walk_stmt(s, visit);
                    }
                }
            }
            Statement::ForStatement(fs) => {
                if let Some(init) = &fs.init {
                    match init {
                        ForStatementInit::VariableDeclaration(vd) => {
                            for d in &vd.declarations {
                                walk_pat(&d.id, visit);
                                if let Some(e) = &d.init {
                                    walk_expr(e, visit);
                                }
                            }
                        }
                        _ => {
                            if let Some(e) = init.as_expression() {
                                walk_expr(e, visit);
                            }
                        }
                    }
                }
                if let Some(t) = &fs.test {
                    walk_expr(t, visit);
                }
                if let Some(u) = &fs.update {
                    walk_expr(u, visit);
                }
                walk_stmt(&fs.body, visit);
            }
            Statement::ForInStatement(fi) => {
                walk_expr(&fi.right, visit);
                walk_stmt(&fi.body, visit);
            }
            Statement::ForOfStatement(fo) => {
                walk_expr(&fo.right, visit);
                walk_stmt(&fo.body, visit);
            }
            Statement::WhileStatement(ws) => {
                walk_expr(&ws.test, visit);
                walk_stmt(&ws.body, visit);
            }
            Statement::DoWhileStatement(dw) => {
                walk_stmt(&dw.body, visit);
                walk_expr(&dw.test, visit);
            }
            Statement::TryStatement(ts) => {
                for s in &ts.block.body {
                    walk_stmt(s, visit);
                }
                if let Some(h) = &ts.handler {
                    for s in &h.body.body {
                        walk_stmt(s, visit);
                    }
                }
                if let Some(f) = &ts.finalizer {
                    for s in &f.body {
                        walk_stmt(s, visit);
                    }
                }
            }
            Statement::LabeledStatement(ls) => walk_stmt(&ls.body, visit),
            Statement::ExportDeclaration(exp) => walk_decl(&exp.declaration, visit),
            Statement::ExportDefaultDeclaration(exp) => match &exp.declaration {
                ExportDefaultDeclarationKind::FunctionDeclaration(fd) => {
                    for p in &fd.params.items {
                        walk_pat(&p.pattern, visit);
                        if let Some(init) = &p.initializer {
                            walk_expr(init, visit);
                        }
                    }
                    if let Some(body) = &fd.body {
                        for s in &body.statements {
                            walk_stmt(s, visit);
                        }
                    }
                }
                ExportDefaultDeclarationKind::ClassDeclaration(cd) => walk_class(cd, visit),
                other => {
                    if let Some(e) = other.as_expression() {
                        walk_expr(e, visit);
                    }
                }
            },
            _ => {}
        }
    }

    for stmt in &program.body {
        walk_stmt(stmt, visit_expr);
    }
}

pub fn walk_ast_call_expressions<'a, F>(program: &'a oxc::ast::ast::Program<'a>, visit_call: &mut F)
where
    F: FnMut(&'a oxc::ast::ast::CallExpression<'a>),
{
    use oxc::ast::ast::{ChainElement, Expression};
    walk_ast_expressions(program, &mut |expr| match expr {
        Expression::CallExpression(call) => visit_call(call),
        Expression::ChainExpression(ch) => {
            if let ChainElement::CallExpression(call) = &ch.expression {
                visit_call(call);
            }
        }
        _ => {}
    });
}

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

/// Walks `program` while tracking lexical shadowing and emits span replacement edits for any
/// unshadowed reference to a live imported binding in `live_map` (`local_ident -> "__live_N.exp"`).
pub fn collect_live_import_reference_edits(
    program: &oxc::ast::ast::Program<'_>,
    live_map: &rustc_hash::FxHashMap<String, String>,
    edits: &mut Vec<(usize, usize, String)>,
) {
    if live_map.is_empty() {
        return;
    }
    let mut top_shadow = rustc_hash::FxHashSet::default();
    ident_edit_walker::bind_stmts(&program.body, &mut top_shadow);
    for stmt in &program.body {
        ident_edit_walker::walk_stmt(stmt, &top_shadow, live_map, edits);
    }
}

/// Walks a single `Expression` while tracking inner lexical shadowing and emits span replacement
/// edits (including `{ x }` -> `{ x: rep }` object shorthand expansion) for any unshadowed
/// identifier matching `rename_map`.
pub fn collect_expr_identifier_edits(
    expr: &oxc::ast::ast::Expression<'_>,
    rename_map: &rustc_hash::FxHashMap<String, String>,
    edits: &mut Vec<(usize, usize, String)>,
) {
    if rename_map.is_empty() {
        return;
    }
    let empty_shadow = rustc_hash::FxHashSet::default();
    ident_edit_walker::walk_expr(expr, &empty_shadow, rename_map, edits);
}

/// Walks `program` for top-level CommonJS destructured bindings (excluding the candidate
/// declaration statements themselves in `excluded_stmt_spans`) while tracking lexical shadowing
/// across other top-level declarations and nested scopes.
pub fn collect_top_level_cjs_reference_edits(
    program: &oxc::ast::ast::Program<'_>,
    excluded_stmt_spans: &rustc_hash::FxHashSet<(usize, usize)>,
    rename_map: &rustc_hash::FxHashMap<String, String>,
    edits: &mut Vec<(usize, usize, String)>,
) {
    use oxc::span::GetSpan;
    if rename_map.is_empty() {
        return;
    }
    let mut top_shadow = rustc_hash::FxHashSet::default();
    for stmt in &program.body {
        let sp = stmt.span();
        if !excluded_stmt_spans.contains(&(sp.start as usize, sp.end as usize)) {
            ident_edit_walker::bind_stmts(std::slice::from_ref(stmt), &mut top_shadow);
        }
    }
    for stmt in &program.body {
        let sp = stmt.span();
        if !excluded_stmt_spans.contains(&(sp.start as usize, sp.end as usize)) {
            ident_edit_walker::walk_stmt(stmt, &top_shadow, rename_map, edits);
        }
    }
}

mod ident_edit_walker {
    use super::collect_binding_pattern_names;
    use oxc::ast::ast::{
        BindingPattern, ChainElement, ClassElement, Declaration, ExportDefaultDeclarationKind,
        Expression, ForStatementInit, ForStatementLeft, ObjectPropertyKind, Statement,
    };
    use rustc_hash::FxHashSet;

    pub(super) fn bind_pat(pat: &BindingPattern<'_>, shadowed: &mut FxHashSet<String>) {
        let mut names = Vec::new();
        collect_binding_pattern_names(pat, &mut names);
        for (n, _) in names {
            shadowed.insert(n);
        }
    }

    pub(super) fn bind_stmts(stmts: &[Statement<'_>], shadowed: &mut FxHashSet<String>) {
        for s in stmts {
            match s {
                Statement::VariableDeclaration(vd) => {
                    for d in &vd.declarations {
                        bind_pat(&d.id, shadowed);
                    }
                }
                Statement::FunctionDeclaration(fd) => {
                    if let Some(id) = &fd.id {
                        shadowed.insert(id.name.to_string());
                    }
                }
                Statement::ClassDeclaration(cd) => {
                    if let Some(id) = &cd.id {
                        shadowed.insert(id.name.to_string());
                    }
                }
                Statement::ExportDeclaration(exp) => match &exp.declaration {
                    Declaration::VariableDeclaration(vd) => {
                        for d in &vd.declarations {
                            bind_pat(&d.id, shadowed);
                        }
                    }
                    Declaration::FunctionDeclaration(fd) => {
                        if let Some(id) = &fd.id {
                            shadowed.insert(id.name.to_string());
                        }
                    }
                    Declaration::ClassDeclaration(cd) => {
                        if let Some(id) = &cd.id {
                            shadowed.insert(id.name.to_string());
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }

    fn walk_pat(
        pat: &BindingPattern<'_>,
        shadowed: &FxHashSet<String>,
        live_map: &rustc_hash::FxHashMap<String, String>,
        edits: &mut Vec<(usize, usize, String)>,
    ) {
        match pat {
            BindingPattern::ObjectPattern(obj) => {
                for p in &obj.properties {
                    if p.computed {
                        if let Some(ke) = p.key.as_expression() {
                            walk_expr(ke, shadowed, live_map, edits);
                        }
                    }
                    walk_pat(&p.value, shadowed, live_map, edits);
                }
                if let Some(r) = &obj.rest {
                    walk_pat(&r.argument, shadowed, live_map, edits);
                }
            }
            BindingPattern::ArrayPattern(arr) => {
                for el in arr.elements.iter().flatten() {
                    walk_pat(el, shadowed, live_map, edits);
                }
                if let Some(r) = &arr.rest {
                    walk_pat(&r.argument, shadowed, live_map, edits);
                }
            }
            BindingPattern::AssignmentPattern(assign) => {
                walk_pat(&assign.left, shadowed, live_map, edits);
                walk_expr(&assign.right, shadowed, live_map, edits);
            }
            BindingPattern::BindingIdentifier(_) => {}
        }
    }

    fn walk_class(
        cls: &oxc::ast::ast::Class<'_>,
        shadowed: &FxHashSet<String>,
        live_map: &rustc_hash::FxHashMap<String, String>,
        edits: &mut Vec<(usize, usize, String)>,
    ) {
        for el in &cls.body.body {
            match el {
                ClassElement::MethodDefinition(md) => {
                    if md.computed {
                        if let Some(ke) = md.key.as_expression() {
                            walk_expr(ke, shadowed, live_map, edits);
                        }
                    }
                    let mut fn_shadow = shadowed.clone();
                    for p in &md.value.params.items {
                        bind_pat(&p.pattern, &mut fn_shadow);
                    }
                    if let Some(rest) = &md.value.params.rest {
                        bind_pat(&rest.rest.argument, &mut fn_shadow);
                    }
                    if let Some(body) = &md.value.body {
                        bind_stmts(&body.statements, &mut fn_shadow);
                    }
                    for p in &md.value.params.items {
                        walk_pat(&p.pattern, &fn_shadow, live_map, edits);
                        if let Some(init) = &p.initializer {
                            walk_expr(init, &fn_shadow, live_map, edits);
                        }
                    }
                    if let Some(body) = &md.value.body {
                        for s in &body.statements {
                            walk_stmt(s, &fn_shadow, live_map, edits);
                        }
                    }
                }
                ClassElement::PropertyDefinition(pd) => {
                    if pd.computed {
                        if let Some(ke) = pd.key.as_expression() {
                            walk_expr(ke, shadowed, live_map, edits);
                        }
                    }
                    if let Some(v) = &pd.value {
                        walk_expr(v, shadowed, live_map, edits);
                    }
                }
                ClassElement::StaticBlock(sb) => {
                    let mut blk_shadow = shadowed.clone();
                    bind_stmts(&sb.body, &mut blk_shadow);
                    for s in &sb.body {
                        walk_stmt(s, &blk_shadow, live_map, edits);
                    }
                }
                _ => {}
            }
        }
    }

    pub(super) fn walk_expr(
        expr: &Expression<'_>,
        shadowed: &FxHashSet<String>,
        live_map: &rustc_hash::FxHashMap<String, String>,
        edits: &mut Vec<(usize, usize, String)>,
    ) {
        match expr {
            Expression::Identifier(id) => {
                let name = id.name.as_str();
                if !shadowed.contains(name) {
                    if let Some(rep) = live_map.get(name) {
                        edits.push((id.span.start as usize, id.span.end as usize, rep.clone()));
                    }
                }
            }
            Expression::CallExpression(call) => {
                walk_expr(&call.callee, shadowed, live_map, edits);
                for arg in &call.arguments {
                    if let Some(e) = arg.as_expression() {
                        walk_expr(e, shadowed, live_map, edits);
                    }
                }
            }
            Expression::ImportExpression(imp) => {
                walk_expr(&imp.source, shadowed, live_map, edits);
            }
            Expression::NewExpression(ne) => {
                walk_expr(&ne.callee, shadowed, live_map, edits);
                for arg in &ne.arguments {
                    if let Some(e) = arg.as_expression() {
                        walk_expr(e, shadowed, live_map, edits);
                    }
                }
            }
            Expression::StaticMemberExpression(m) => {
                walk_expr(&m.object, shadowed, live_map, edits);
            }
            Expression::ComputedMemberExpression(m) => {
                walk_expr(&m.object, shadowed, live_map, edits);
                walk_expr(&m.expression, shadowed, live_map, edits);
            }
            Expression::PrivateFieldExpression(m) => {
                walk_expr(&m.object, shadowed, live_map, edits);
            }
            Expression::BinaryExpression(b) => {
                walk_expr(&b.left, shadowed, live_map, edits);
                walk_expr(&b.right, shadowed, live_map, edits);
            }
            Expression::LogicalExpression(l) => {
                walk_expr(&l.left, shadowed, live_map, edits);
                walk_expr(&l.right, shadowed, live_map, edits);
            }
            Expression::UnaryExpression(u) => {
                walk_expr(&u.argument, shadowed, live_map, edits);
            }
            Expression::UpdateExpression(u) => {
                if let oxc::ast::ast::SimpleAssignmentTarget::AssignmentTargetIdentifier(id) =
                    &u.argument
                {
                    let name = id.name.as_str();
                    if !shadowed.contains(name) {
                        if let Some(rep) = live_map.get(name) {
                            edits.push((id.span.start as usize, id.span.end as usize, rep.clone()));
                        }
                    }
                }
            }
            Expression::ConditionalExpression(c) => {
                walk_expr(&c.test, shadowed, live_map, edits);
                walk_expr(&c.consequent, shadowed, live_map, edits);
                walk_expr(&c.alternate, shadowed, live_map, edits);
            }
            Expression::AssignmentExpression(a) => {
                match &a.left {
                    oxc::ast::ast::AssignmentTarget::AssignmentTargetIdentifier(id) => {
                        let name = id.name.as_str();
                        if !shadowed.contains(name) {
                            if let Some(rep) = live_map.get(name) {
                                edits.push((
                                    id.span.start as usize,
                                    id.span.end as usize,
                                    rep.clone(),
                                ));
                            }
                        }
                    }
                    oxc::ast::ast::AssignmentTarget::StaticMemberExpression(m) => {
                        walk_expr(&m.object, shadowed, live_map, edits);
                    }
                    oxc::ast::ast::AssignmentTarget::ComputedMemberExpression(m) => {
                        walk_expr(&m.object, shadowed, live_map, edits);
                        walk_expr(&m.expression, shadowed, live_map, edits);
                    }
                    oxc::ast::ast::AssignmentTarget::PrivateFieldExpression(m) => {
                        walk_expr(&m.object, shadowed, live_map, edits);
                    }
                    _ => {}
                }
                walk_expr(&a.right, shadowed, live_map, edits);
            }
            Expression::SequenceExpression(seq) => {
                for e in &seq.expressions {
                    walk_expr(e, shadowed, live_map, edits);
                }
            }
            Expression::ParenthesizedExpression(p) => {
                walk_expr(&p.expression, shadowed, live_map, edits);
            }
            Expression::AwaitExpression(aw) => {
                walk_expr(&aw.argument, shadowed, live_map, edits);
            }
            Expression::YieldExpression(y) => {
                if let Some(arg) = &y.argument {
                    walk_expr(arg, shadowed, live_map, edits);
                }
            }
            Expression::ArrowFunctionExpression(arrow) => {
                let mut fn_shadow = shadowed.clone();
                for p in &arrow.params.items {
                    bind_pat(&p.pattern, &mut fn_shadow);
                }
                if let Some(rest) = &arrow.params.rest {
                    bind_pat(&rest.rest.argument, &mut fn_shadow);
                }
                if let oxc::ast::ast::ArrowFunctionBody::FunctionBody(fb) = &arrow.body {
                    bind_stmts(&fb.statements, &mut fn_shadow);
                }
                for p in &arrow.params.items {
                    walk_pat(&p.pattern, &fn_shadow, live_map, edits);
                    if let Some(init) = &p.initializer {
                        walk_expr(init, &fn_shadow, live_map, edits);
                    }
                }
                if let oxc::ast::ast::ArrowFunctionBody::FunctionBody(fb) = &arrow.body {
                    for s in &fb.statements {
                        walk_stmt(s, &fn_shadow, live_map, edits);
                    }
                } else if let Some(e) = arrow.body.as_expression() {
                    walk_expr(e, &fn_shadow, live_map, edits);
                }
            }
            Expression::FunctionExpression(fe) => {
                let mut fn_shadow = shadowed.clone();
                if let Some(id) = &fe.id {
                    fn_shadow.insert(id.name.to_string());
                }
                for p in &fe.params.items {
                    bind_pat(&p.pattern, &mut fn_shadow);
                }
                if let Some(rest) = &fe.params.rest {
                    bind_pat(&rest.rest.argument, &mut fn_shadow);
                }
                if let Some(body) = &fe.body {
                    bind_stmts(&body.statements, &mut fn_shadow);
                }
                for p in &fe.params.items {
                    walk_pat(&p.pattern, &fn_shadow, live_map, edits);
                    if let Some(init) = &p.initializer {
                        walk_expr(init, &fn_shadow, live_map, edits);
                    }
                }
                if let Some(body) = &fe.body {
                    for s in &body.statements {
                        walk_stmt(s, &fn_shadow, live_map, edits);
                    }
                }
            }
            Expression::ClassExpression(ce) => walk_class(ce, shadowed, live_map, edits),
            Expression::ObjectExpression(obj) => {
                for prop in &obj.properties {
                    match prop {
                        ObjectPropertyKind::ObjectProperty(p) => {
                            if p.shorthand {
                                if let Expression::Identifier(id) = &p.value {
                                    let name = id.name.as_str();
                                    if !shadowed.contains(name) {
                                        if let Some(rep) = live_map.get(name) {
                                            edits.push((
                                                p.span.start as usize,
                                                p.span.end as usize,
                                                format!("{}: {}", name, rep),
                                            ));
                                            continue;
                                        }
                                    }
                                }
                            }
                            if p.computed {
                                if let Some(ke) = p.key.as_expression() {
                                    walk_expr(ke, shadowed, live_map, edits);
                                }
                            }
                            walk_expr(&p.value, shadowed, live_map, edits);
                        }
                        ObjectPropertyKind::SpreadProperty(sp) => {
                            walk_expr(&sp.argument, shadowed, live_map, edits);
                        }
                    }
                }
            }
            Expression::ArrayExpression(arr) => {
                for el in &arr.elements {
                    if let Some(e) = el.as_expression() {
                        walk_expr(e, shadowed, live_map, edits);
                    } else if let oxc::ast::ast::ArrayExpressionElement::SpreadElement(sp) = el {
                        walk_expr(&sp.argument, shadowed, live_map, edits);
                    }
                }
            }
            Expression::TemplateLiteral(tpl) => {
                for e in &tpl.expressions {
                    walk_expr(e, shadowed, live_map, edits);
                }
            }
            Expression::TaggedTemplateExpression(tt) => {
                walk_expr(&tt.tag, shadowed, live_map, edits);
                for e in &tt.quasi.expressions {
                    walk_expr(e, shadowed, live_map, edits);
                }
            }
            Expression::ChainExpression(ch) => match &ch.expression {
                ChainElement::CallExpression(call) => {
                    walk_expr(&call.callee, shadowed, live_map, edits);
                    for arg in &call.arguments {
                        if let Some(e) = arg.as_expression() {
                            walk_expr(e, shadowed, live_map, edits);
                        }
                    }
                }
                ChainElement::StaticMemberExpression(m) => {
                    walk_expr(&m.object, shadowed, live_map, edits);
                }
                ChainElement::ComputedMemberExpression(m) => {
                    walk_expr(&m.object, shadowed, live_map, edits);
                    walk_expr(&m.expression, shadowed, live_map, edits);
                }
                ChainElement::PrivateFieldExpression(m) => {
                    walk_expr(&m.object, shadowed, live_map, edits);
                }
                ChainElement::TSNonNullExpression(n) => {
                    walk_expr(&n.expression, shadowed, live_map, edits);
                }
            },
            _ => {}
        }
    }

    fn walk_decl(
        decl: &Declaration<'_>,
        shadowed: &FxHashSet<String>,
        live_map: &rustc_hash::FxHashMap<String, String>,
        edits: &mut Vec<(usize, usize, String)>,
    ) {
        match decl {
            Declaration::VariableDeclaration(vd) => {
                for d in &vd.declarations {
                    walk_pat(&d.id, shadowed, live_map, edits);
                    if let Some(init) = &d.init {
                        walk_expr(init, shadowed, live_map, edits);
                    }
                }
            }
            Declaration::FunctionDeclaration(fd) => {
                let mut fn_shadow = shadowed.clone();
                if let Some(id) = &fd.id {
                    fn_shadow.insert(id.name.to_string());
                }
                for p in &fd.params.items {
                    bind_pat(&p.pattern, &mut fn_shadow);
                }
                if let Some(rest) = &fd.params.rest {
                    bind_pat(&rest.rest.argument, &mut fn_shadow);
                }
                if let Some(body) = &fd.body {
                    bind_stmts(&body.statements, &mut fn_shadow);
                }
                for p in &fd.params.items {
                    walk_pat(&p.pattern, &fn_shadow, live_map, edits);
                    if let Some(init) = &p.initializer {
                        walk_expr(init, &fn_shadow, live_map, edits);
                    }
                }
                if let Some(body) = &fd.body {
                    for s in &body.statements {
                        walk_stmt(s, &fn_shadow, live_map, edits);
                    }
                }
            }
            Declaration::ClassDeclaration(cd) => walk_class(cd, shadowed, live_map, edits),
            _ => {}
        }
    }

    pub(super) fn walk_stmt(
        stmt: &Statement<'_>,
        shadowed: &FxHashSet<String>,
        live_map: &rustc_hash::FxHashMap<String, String>,
        edits: &mut Vec<(usize, usize, String)>,
    ) {
        match stmt {
            Statement::VariableDeclaration(vd) => {
                for d in &vd.declarations {
                    walk_pat(&d.id, shadowed, live_map, edits);
                    if let Some(init) = &d.init {
                        walk_expr(init, shadowed, live_map, edits);
                    }
                }
            }
            Statement::FunctionDeclaration(fd) => {
                let mut fn_shadow = shadowed.clone();
                if let Some(id) = &fd.id {
                    fn_shadow.insert(id.name.to_string());
                }
                for p in &fd.params.items {
                    bind_pat(&p.pattern, &mut fn_shadow);
                }
                if let Some(rest) = &fd.params.rest {
                    bind_pat(&rest.rest.argument, &mut fn_shadow);
                }
                if let Some(body) = &fd.body {
                    bind_stmts(&body.statements, &mut fn_shadow);
                }
                for p in &fd.params.items {
                    walk_pat(&p.pattern, &fn_shadow, live_map, edits);
                    if let Some(init) = &p.initializer {
                        walk_expr(init, &fn_shadow, live_map, edits);
                    }
                }
                if let Some(body) = &fd.body {
                    for s in &body.statements {
                        walk_stmt(s, &fn_shadow, live_map, edits);
                    }
                }
            }
            Statement::ClassDeclaration(cd) => walk_class(cd, shadowed, live_map, edits),
            Statement::ExpressionStatement(es) => {
                walk_expr(&es.expression, shadowed, live_map, edits);
            }
            Statement::BlockStatement(bs) => {
                let mut blk_shadow = shadowed.clone();
                bind_stmts(&bs.body, &mut blk_shadow);
                for s in &bs.body {
                    walk_stmt(s, &blk_shadow, live_map, edits);
                }
            }
            Statement::ReturnStatement(rs) => {
                if let Some(arg) = &rs.argument {
                    walk_expr(arg, shadowed, live_map, edits);
                }
            }
            Statement::ThrowStatement(ts) => {
                walk_expr(&ts.argument, shadowed, live_map, edits);
            }
            Statement::IfStatement(ifs) => {
                walk_expr(&ifs.test, shadowed, live_map, edits);
                walk_stmt(&ifs.consequent, shadowed, live_map, edits);
                if let Some(alt) = &ifs.alternate {
                    walk_stmt(alt, shadowed, live_map, edits);
                }
            }
            Statement::SwitchStatement(sw) => {
                walk_expr(&sw.discriminant, shadowed, live_map, edits);
                let mut sw_shadow = shadowed.clone();
                for c in &sw.cases {
                    bind_stmts(&c.consequent, &mut sw_shadow);
                }
                for c in &sw.cases {
                    if let Some(t) = &c.test {
                        walk_expr(t, &sw_shadow, live_map, edits);
                    }
                    for s in &c.consequent {
                        walk_stmt(s, &sw_shadow, live_map, edits);
                    }
                }
            }
            Statement::ForStatement(fs) => {
                let mut for_shadow = shadowed.clone();
                if let Some(ForStatementInit::VariableDeclaration(vd)) = &fs.init {
                    for d in &vd.declarations {
                        bind_pat(&d.id, &mut for_shadow);
                    }
                }
                if let Some(init) = &fs.init {
                    match init {
                        ForStatementInit::VariableDeclaration(vd) => {
                            for d in &vd.declarations {
                                walk_pat(&d.id, &for_shadow, live_map, edits);
                                if let Some(e) = &d.init {
                                    walk_expr(e, &for_shadow, live_map, edits);
                                }
                            }
                        }
                        _ => {
                            if let Some(e) = init.as_expression() {
                                walk_expr(e, &for_shadow, live_map, edits);
                            }
                        }
                    }
                }
                if let Some(t) = &fs.test {
                    walk_expr(t, &for_shadow, live_map, edits);
                }
                if let Some(u) = &fs.update {
                    walk_expr(u, &for_shadow, live_map, edits);
                }
                walk_stmt(&fs.body, &for_shadow, live_map, edits);
            }
            Statement::ForInStatement(fi) => {
                let mut for_shadow = shadowed.clone();
                if let ForStatementLeft::VariableDeclaration(vd) = &fi.left {
                    for d in &vd.declarations {
                        bind_pat(&d.id, &mut for_shadow);
                    }
                }
                walk_expr(&fi.right, shadowed, live_map, edits);
                walk_stmt(&fi.body, &for_shadow, live_map, edits);
            }
            Statement::ForOfStatement(fo) => {
                let mut for_shadow = shadowed.clone();
                if let ForStatementLeft::VariableDeclaration(vd) = &fo.left {
                    for d in &vd.declarations {
                        bind_pat(&d.id, &mut for_shadow);
                    }
                }
                walk_expr(&fo.right, shadowed, live_map, edits);
                walk_stmt(&fo.body, &for_shadow, live_map, edits);
            }
            Statement::WhileStatement(ws) => {
                walk_expr(&ws.test, shadowed, live_map, edits);
                walk_stmt(&ws.body, shadowed, live_map, edits);
            }
            Statement::DoWhileStatement(dw) => {
                walk_stmt(&dw.body, shadowed, live_map, edits);
                walk_expr(&dw.test, shadowed, live_map, edits);
            }
            Statement::TryStatement(ts) => {
                let mut try_shadow = shadowed.clone();
                bind_stmts(&ts.block.body, &mut try_shadow);
                for s in &ts.block.body {
                    walk_stmt(s, &try_shadow, live_map, edits);
                }
                if let Some(h) = &ts.handler {
                    let mut catch_shadow = shadowed.clone();
                    if let Some(p) = &h.param {
                        bind_pat(&p.pattern, &mut catch_shadow);
                    }
                    bind_stmts(&h.body.body, &mut catch_shadow);
                    if let Some(p) = &h.param {
                        walk_pat(&p.pattern, &catch_shadow, live_map, edits);
                    }
                    for s in &h.body.body {
                        walk_stmt(s, &catch_shadow, live_map, edits);
                    }
                }
                if let Some(f) = &ts.finalizer {
                    let mut fin_shadow = shadowed.clone();
                    bind_stmts(&f.body, &mut fin_shadow);
                    for s in &f.body {
                        walk_stmt(s, &fin_shadow, live_map, edits);
                    }
                }
            }
            Statement::LabeledStatement(ls) => walk_stmt(&ls.body, shadowed, live_map, edits),
            Statement::ExportDeclaration(exp) => {
                walk_decl(&exp.declaration, shadowed, live_map, edits);
            }
            Statement::ExportDefaultDeclaration(exp) => match &exp.declaration {
                ExportDefaultDeclarationKind::FunctionDeclaration(fd) => {
                    let mut fn_shadow = shadowed.clone();
                    if let Some(id) = &fd.id {
                        fn_shadow.insert(id.name.to_string());
                    }
                    for p in &fd.params.items {
                        bind_pat(&p.pattern, &mut fn_shadow);
                    }
                    if let Some(rest) = &fd.params.rest {
                        bind_pat(&rest.rest.argument, &mut fn_shadow);
                    }
                    if let Some(body) = &fd.body {
                        bind_stmts(&body.statements, &mut fn_shadow);
                    }
                    for p in &fd.params.items {
                        walk_pat(&p.pattern, &fn_shadow, live_map, edits);
                        if let Some(init) = &p.initializer {
                            walk_expr(init, &fn_shadow, live_map, edits);
                        }
                    }
                    if let Some(body) = &fd.body {
                        for s in &body.statements {
                            walk_stmt(s, &fn_shadow, live_map, edits);
                        }
                    }
                }
                ExportDefaultDeclarationKind::ClassDeclaration(cd) => {
                    walk_class(cd, shadowed, live_map, edits);
                }
                other => {
                    if let Some(e) = other.as_expression() {
                        walk_expr(e, shadowed, live_map, edits);
                    }
                }
            },
            _ => {}
        }
    }
}
