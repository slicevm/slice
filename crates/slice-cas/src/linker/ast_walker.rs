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
            Expression::AssignmentExpression(a) => walk_expr(&a.right, visit),
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
            Expression::ChainExpression(ch) => {
                if let oxc::ast::ast::ChainElement::CallExpression(call) = &ch.expression {
                    walk_expr(&call.callee, visit);
                    for arg in &call.arguments {
                        if let Some(e) = arg.as_expression() {
                            walk_expr(e, visit);
                        }
                    }
                }
            }
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
pub fn has_top_level_await(code: &str) -> bool {
    use oxc::ast::ast::{Declaration, Expression, ForStatementInit, ObjectPropertyKind, Statement};

    if !code.contains("await ") && !code.contains("await(") {
        return false;
    }

    let allocator = oxc::allocator::Allocator::default();
    let source_type = oxc::span::SourceType::ts().with_module(true);
    let parser_ret = oxc::parser::Parser::new(&allocator, code, source_type).parse();
    if !parser_ret.diagnostics.is_empty() {
        return false;
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

    parser_ret.program.body.iter().any(stmt_has_tla)
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
