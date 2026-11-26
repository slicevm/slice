//! General Oxc AST Expression and Call-Expression Visitors.

pub fn walk_ast_expressions<'a, F>(program: &'a oxc::ast::ast::Program<'a>, visit_expr: &mut F)
where
    F: FnMut(&'a oxc::ast::ast::Expression<'a>),
{
    use oxc::ast::ast::{
        BindingPattern, ClassElement, Declaration, ExportDefaultDeclarationKind, Expression,
        ForStatementInit, Function, ObjectPropertyKind, Statement, VariableDeclaration,
    };

    fn walk_pat<'a, F>(pat: &'a BindingPattern<'a>, visit: &mut F)
    where
        F: FnMut(&'a Expression<'a>),
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

    fn walk_var_decl<'a, F>(vd: &'a VariableDeclaration<'a>, visit: &mut F)
    where
        F: FnMut(&'a Expression<'a>),
    {
        for d in &vd.declarations {
            walk_pat(&d.id, visit);
            if let Some(init) = &d.init {
                walk_expr(init, visit);
            }
        }
    }

    fn walk_function<'a, F>(func: &'a Function<'a>, visit: &mut F)
    where
        F: FnMut(&'a Expression<'a>),
    {
        for p in &func.params.items {
            walk_pat(&p.pattern, visit);
            if let Some(init) = &p.initializer {
                walk_expr(init, visit);
            }
        }
        if let Some(body) = &func.body {
            for s in &body.statements {
                walk_stmt(s, visit);
            }
        }
    }

    fn walk_class<'a, F>(cls: &'a oxc::ast::ast::Class<'a>, visit: &mut F)
    where
        F: FnMut(&'a Expression<'a>),
    {
        for el in &cls.body.body {
            match el {
                ClassElement::MethodDefinition(md) => walk_function(&md.value, visit),
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
        F: FnMut(&'a Expression<'a>),
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
            Expression::FunctionExpression(fe) => walk_function(fe, visit),
            Expression::ClassExpression(ce) => walk_class(ce, visit),
            Expression::ObjectExpression(obj) => {
                for prop in &obj.properties {
                    match prop {
                        ObjectPropertyKind::ObjectProperty(p) => {
                            if p.key.is_computed()
                                && let Some(ke) = p.key.as_expression()
                            {
                                walk_expr(ke, visit);
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
        F: FnMut(&'a Expression<'a>),
    {
        match decl {
            Declaration::VariableDeclaration(vd) => walk_var_decl(vd, visit),
            Declaration::FunctionDeclaration(fd) => walk_function(fd, visit),
            Declaration::ClassDeclaration(cd) => walk_class(cd, visit),
            _ => {}
        }
    }

    fn walk_stmt<'a, F>(stmt: &'a Statement<'a>, visit: &mut F)
    where
        F: FnMut(&'a Expression<'a>),
    {
        match stmt {
            Statement::VariableDeclaration(vd) => walk_var_decl(vd, visit),
            Statement::FunctionDeclaration(fd) => walk_function(fd, visit),
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
                        ForStatementInit::VariableDeclaration(vd) => walk_var_decl(vd, visit),
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
                ExportDefaultDeclarationKind::FunctionDeclaration(fd) => walk_function(fd, visit),
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
