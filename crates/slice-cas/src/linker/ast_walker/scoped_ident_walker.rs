//! Lexical-Shadowing-Aware Identifier Reference Edit Collectors.

use super::collect_binding_pattern_names;
use oxc::ast::ast::{
    BindingPattern, ChainElement, ClassElement, Declaration, ExportDefaultDeclarationKind,
    Expression, ForStatementInit, ForStatementLeft, Function, ObjectPropertyKind, Statement,
};
use rustc_hash::{FxHashMap, FxHashSet};

/// Walks `program` while tracking lexical shadowing and emits span replacement edits for any
/// unshadowed reference to a live imported binding in `live_map` (`local_ident -> "__live_N.exp"`).
pub fn collect_live_import_reference_edits(
    program: &oxc::ast::ast::Program<'_>,
    live_map: &FxHashMap<String, String>,
    edits: &mut Vec<(usize, usize, String)>,
) {
    if live_map.is_empty() {
        return;
    }
    let mut top_shadow = FxHashSet::default();
    bind_stmts(&program.body, &mut top_shadow);
    for stmt in &program.body {
        walk_stmt(stmt, &top_shadow, live_map, edits);
    }
}

/// Walks a single `Expression` while tracking inner lexical shadowing and emits span replacement
/// edits (including `{ x }` -> `{ x: rep }` object shorthand expansion) for any unshadowed
/// identifier matching `rename_map`.
pub fn collect_expr_identifier_edits(
    expr: &Expression<'_>,
    rename_map: &FxHashMap<String, String>,
    edits: &mut Vec<(usize, usize, String)>,
) {
    if rename_map.is_empty() {
        return;
    }
    let empty_shadow = FxHashSet::default();
    walk_expr(expr, &empty_shadow, rename_map, edits);
}

/// Walks `program` for top-level CommonJS destructured bindings (excluding the candidate
/// declaration statements themselves in `excluded_stmt_spans`) while tracking lexical shadowing
/// across other top-level declarations and nested scopes.
pub fn collect_top_level_cjs_reference_edits(
    program: &oxc::ast::ast::Program<'_>,
    excluded_stmt_spans: &FxHashSet<(usize, usize)>,
    rename_map: &FxHashMap<String, String>,
    edits: &mut Vec<(usize, usize, String)>,
) {
    use oxc::span::GetSpan;
    if rename_map.is_empty() {
        return;
    }
    let mut top_shadow = FxHashSet::default();
    for stmt in &program.body {
        let sp = stmt.span();
        if !excluded_stmt_spans.contains(&(sp.start as usize, sp.end as usize)) {
            bind_stmts(std::slice::from_ref(stmt), &mut top_shadow);
        }
    }
    for stmt in &program.body {
        let sp = stmt.span();
        if !excluded_stmt_spans.contains(&(sp.start as usize, sp.end as usize)) {
            walk_stmt(stmt, &top_shadow, rename_map, edits);
        }
    }
}

fn bind_pat(pat: &BindingPattern<'_>, shadowed: &mut FxHashSet<String>) {
    let mut names = Vec::new();
    collect_binding_pattern_names(pat, &mut names);
    for (n, _) in names {
        shadowed.insert(n);
    }
}

fn bind_stmts(stmts: &[Statement<'_>], shadowed: &mut FxHashSet<String>) {
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
    live_map: &FxHashMap<String, String>,
    edits: &mut Vec<(usize, usize, String)>,
) {
    match pat {
        BindingPattern::ObjectPattern(obj) => {
            for p in &obj.properties {
                if p.computed
                    && let Some(ke) = p.key.as_expression()
                {
                    walk_expr(ke, shadowed, live_map, edits);
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

fn walk_function_node(
    func: &Function<'_>,
    bind_fn_id: bool,
    shadowed: &FxHashSet<String>,
    live_map: &FxHashMap<String, String>,
    edits: &mut Vec<(usize, usize, String)>,
) {
    let mut fn_shadow = shadowed.clone();
    if bind_fn_id && let Some(id) = &func.id {
        fn_shadow.insert(id.name.to_string());
    }
    for p in &func.params.items {
        bind_pat(&p.pattern, &mut fn_shadow);
    }
    if let Some(rest) = &func.params.rest {
        bind_pat(&rest.rest.argument, &mut fn_shadow);
    }
    if let Some(body) = &func.body {
        bind_stmts(&body.statements, &mut fn_shadow);
    }
    for p in &func.params.items {
        walk_pat(&p.pattern, &fn_shadow, live_map, edits);
        if let Some(init) = &p.initializer {
            walk_expr(init, &fn_shadow, live_map, edits);
        }
    }
    if let Some(body) = &func.body {
        for s in &body.statements {
            walk_stmt(s, &fn_shadow, live_map, edits);
        }
    }
}

fn walk_class(
    cls: &oxc::ast::ast::Class<'_>,
    shadowed: &FxHashSet<String>,
    live_map: &FxHashMap<String, String>,
    edits: &mut Vec<(usize, usize, String)>,
) {
    for el in &cls.body.body {
        match el {
            ClassElement::MethodDefinition(md) => {
                if md.computed
                    && let Some(ke) = md.key.as_expression()
                {
                    walk_expr(ke, shadowed, live_map, edits);
                }
                walk_function_node(&md.value, false, shadowed, live_map, edits);
            }
            ClassElement::PropertyDefinition(pd) => {
                if pd.computed
                    && let Some(ke) = pd.key.as_expression()
                {
                    walk_expr(ke, shadowed, live_map, edits);
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

fn walk_expr(
    expr: &Expression<'_>,
    shadowed: &FxHashSet<String>,
    live_map: &FxHashMap<String, String>,
    edits: &mut Vec<(usize, usize, String)>,
) {
    match expr {
        Expression::Identifier(id) => {
            let name = id.name.as_str();
            if !shadowed.contains(name)
                && let Some(rep) = live_map.get(name)
            {
                edits.push((id.span.start as usize, id.span.end as usize, rep.clone()));
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
                if !shadowed.contains(name)
                    && let Some(rep) = live_map.get(name)
                {
                    edits.push((id.span.start as usize, id.span.end as usize, rep.clone()));
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
                    if !shadowed.contains(name)
                        && let Some(rep) = live_map.get(name)
                    {
                        edits.push((id.span.start as usize, id.span.end as usize, rep.clone()));
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
            walk_function_node(fe, true, shadowed, live_map, edits);
        }
        Expression::ClassExpression(ce) => walk_class(ce, shadowed, live_map, edits),
        Expression::ObjectExpression(obj) => {
            for prop in &obj.properties {
                match prop {
                    ObjectPropertyKind::ObjectProperty(p) => {
                        if p.shorthand
                            && let Expression::Identifier(id) = &p.value
                        {
                            let name = id.name.as_str();
                            if !shadowed.contains(name)
                                && let Some(rep) = live_map.get(name)
                            {
                                edits.push((
                                    p.span.start as usize,
                                    p.span.end as usize,
                                    format!("{}: {}", name, rep),
                                ));
                                continue;
                            }
                        }
                        if p.computed
                            && let Some(ke) = p.key.as_expression()
                        {
                            walk_expr(ke, shadowed, live_map, edits);
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
    live_map: &FxHashMap<String, String>,
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
            walk_function_node(fd, true, shadowed, live_map, edits);
        }
        Declaration::ClassDeclaration(cd) => walk_class(cd, shadowed, live_map, edits),
        _ => {}
    }
}

fn walk_stmt(
    stmt: &Statement<'_>,
    shadowed: &FxHashSet<String>,
    live_map: &FxHashMap<String, String>,
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
            walk_function_node(fd, true, shadowed, live_map, edits);
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
                walk_function_node(fd, true, shadowed, live_map, edits);
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
