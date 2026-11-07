//! Oxc AST-Driven ESM-to-CommonJS Linker, Barrel Lazy-Getter Optimizer,
//! and Built-in Module Classifier.

use crate::resolver::resolve_module_specifier;
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};

#[inline]
pub(crate) fn is_builtin_module(spec: &str) -> bool {
    if spec.starts_with("node:")
        || spec.starts_with("slice:")
        || spec.starts_with("bun:")
        || spec.starts_with("cloudflare:")
    {
        return true;
    }
    matches!(
        spec,
        "fs" | "fs/promises"
            | "child_process"
            | "http"
            | "https"
            | "path"
            | "path/posix"
            | "path/win32"
            | "crypto"
            | "os"
            | "util"
            | "util/types"
            | "events"
            | "buffer"
            | "process"
            | "url"
            | "stream"
            | "stream/promises"
            | "stream/web"
            | "assert"
            | "assert/strict"
            | "timers"
            | "timers/promises"
            | "worker_threads"
            | "net"
            | "dgram"
            | "async_hooks"
            | "zlib"
            | "perf_hooks"
            | "tty"
            | "module"
            | "vm"
            | "string_decoder"
            | "querystring"
    )
}

#[inline]
pub(crate) fn is_jsx_runtime_builtin(spec: &str) -> bool {
    matches!(
        spec,
        "react"
            | "react/jsx-runtime"
            | "react/jsx-dev-runtime"
            | "preact"
            | "preact/jsx-runtime"
            | "preact/jsx-dev-runtime"
            | "hono/jsx"
            | "hono/jsx/jsx-runtime"
            | "hono/jsx/jsx-dev-runtime"
    )
}

#[inline]
pub(crate) fn normalize_builtin_specifier(spec: &str) -> String {
    if spec.starts_with("node:")
        || spec.starts_with("slice:")
        || spec.starts_with("bun:")
        || spec.starts_with("cloudflare:")
    {
        spec.to_string()
    } else if is_jsx_runtime_builtin(spec) {
        "slice:jsx".to_string()
    } else {
        format!("node:{}", spec)
    }
}

thread_local! {
    static CANONICAL_MOD_ID_CACHE: std::cell::RefCell<FxHashMap<PathBuf, String>> =
        std::cell::RefCell::new(FxHashMap::default());
}

pub(crate) fn clear_canonical_mod_id_cache() {
    CANONICAL_MOD_ID_CACHE.with(|c| c.borrow_mut().clear());
}

#[inline]
pub(crate) fn canonical_mod_id(path: &Path) -> String {
    CANONICAL_MOD_ID_CACHE.with(|cache| {
        if let Some(cached) = cache.borrow().get(path) {
            return cached.clone();
        }
        let s = std::fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .replace('\\', "/");
        let id = s.strip_prefix("//?/").map(|r| r.to_string()).unwrap_or(s);
        cache.borrow_mut().insert(path.to_path_buf(), id.clone());
        id
    })
}

fn collapse_multiline_imports_exports(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut in_stmt = false;
    let mut stmt_buf = String::new();

    for line in code.lines() {
        let trimmed = line.trim();
        if !in_stmt {
            if (trimmed.starts_with("import ") || trimmed.starts_with("export {"))
                && !trimmed.ends_with(';')
                && !trimmed.contains(" from ")
            {
                in_stmt = true;
                stmt_buf.clear();
                stmt_buf.push_str(trimmed);
                stmt_buf.push(' ');
            } else {
                out.push_str(line);
                out.push('\n');
            }
        } else {
            stmt_buf.push_str(trimmed);
            stmt_buf.push(' ');
            if trimmed.ends_with(';') || trimmed.contains(" from ") {
                in_stmt = false;
                out.push_str(stmt_buf.trim());
                out.push('\n');
            }
        }
    }
    if in_stmt && !stmt_buf.is_empty() {
        out.push_str(stmt_buf.trim());
        out.push('\n');
    }
    out
}

fn extract_quoted_specifier(s: &str) -> Option<(String, char)> {
    let trimmed = s.trim().trim_end_matches(';').trim();
    if let Some(stripped) = trimmed.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        return Some((stripped.to_string(), '"'));
    }
    if let Some(stripped) = trimmed
        .strip_prefix('\'')
        .and_then(|r| r.strip_suffix('\''))
    {
        return Some((stripped.to_string(), '\''));
    }
    None
}

pub(crate) fn rewrite_cjs_requires_in_line(
    line: &str,
    from_dir: &Path,
    deps_out: &mut Vec<PathBuf>,
) -> String {
    if !line.contains("require(") {
        return line.to_string();
    }
    let mut result = String::with_capacity(line.len() + 32);
    let mut rest = line;

    while let Some(pos) = rest.find("require(") {
        let valid_boundary = pos == 0
            || !rest.as_bytes()[pos - 1].is_ascii_alphanumeric()
                && rest.as_bytes()[pos - 1] != b'_';
        let after_req = &rest[pos + 8..];
        result.push_str(&rest[..pos + 8]);

        if valid_boundary {
            let trimmed_after = after_req.trim_start();
            let ws_len = after_req.len() - trimmed_after.len();
            if let Some(quote) = trimmed_after.chars().next() {
                if quote == '\'' || quote == '"' {
                    if let Some(end_q) = trimmed_after[1..].find(quote) {
                        let after_quote = trimmed_after[1 + end_q + 1..].trim_start();
                        if after_quote.starts_with(')') {
                            let spec = &trimmed_after[1..1 + end_q];
                            if is_builtin_module(spec) {
                                let norm = normalize_builtin_specifier(spec);
                                result.push_str(&format!("{:?}", norm));
                                let consumed = ws_len + 1 + end_q + 1;
                                rest = &after_req[consumed..];
                                continue;
                            } else if let Some(resolved_path) =
                                resolve_module_specifier(from_dir, spec)
                            {
                                let id = canonical_mod_id(&resolved_path);
                                deps_out.push(resolved_path);
                                result.push_str(&format!("{:?}", id));
                                let consumed = ws_len + 1 + end_q + 1;
                                rest = &after_req[consumed..];
                                continue;
                            } else if is_jsx_runtime_builtin(spec) {
                                let norm = normalize_builtin_specifier(spec);
                                result.push_str(&format!("{:?}", norm));
                                let consumed = ws_len + 1 + end_q + 1;
                                rest = &after_req[consumed..];
                                continue;
                            }
                        }
                    }
                }
            }
        }
        rest = after_req;
    }
    result.push_str(rest);
    result
}

pub(crate) fn resolve_spec_and_record(
    spec: &str,
    from_dir: &Path,
    deps_out: &mut Vec<PathBuf>,
) -> String {
    if is_builtin_module(spec) {
        normalize_builtin_specifier(spec)
    } else if let Some(resolved) = resolve_module_specifier(from_dir, spec) {
        let id = canonical_mod_id(&resolved);
        deps_out.push(resolved);
        id
    } else if is_jsx_runtime_builtin(spec) {
        normalize_builtin_specifier(spec)
    } else {
        spec.to_string()
    }
}

fn collect_binding_pattern_names(
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

fn walk_ast_expressions<'a, F>(program: &'a oxc::ast::ast::Program<'a>, visit_expr: &mut F)
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

fn walk_ast_call_expressions<'a, F>(program: &'a oxc::ast::ast::Program<'a>, visit_call: &mut F)
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

/// Rewrites `require("<spec>")` `CallExpression` string literal arguments via Oxc AST span
/// replacement without modifying `"require(...)"` inside string literals, template literals, or comments.
pub(crate) fn rewrite_cjs_requires_ast(
    snippet: &str,
    from_dir: &Path,
    deps_out: &mut Vec<PathBuf>,
) -> String {
    use oxc::ast::ast::{Argument, Expression};

    if !snippet.contains("require(") {
        return snippet.to_string();
    }

    let allocator = oxc::allocator::Allocator::default();
    let source_type = oxc::span::SourceType::ts().with_module(true);
    let parser_ret = oxc::parser::Parser::new(&allocator, snippet, source_type).parse();
    if !parser_ret.diagnostics.is_empty() {
        let wrapped = format!("({});", snippet);
        let retry_ret = oxc::parser::Parser::new(&allocator, &wrapped, source_type).parse();
        if retry_ret.diagnostics.is_empty() {
            let mut edits: Vec<(usize, usize, String)> = Vec::new();
            walk_ast_call_expressions(&retry_ret.program, &mut |call| {
                let Expression::Identifier(callee) = call.callee.without_parentheses() else {
                    return;
                };
                if callee.name.as_str() != "require" || call.arguments.len() != 1 {
                    return;
                }
                let Some(Argument::StringLiteral(str_lit)) = call.arguments.first() else {
                    return;
                };
                let spec = str_lit.value.as_str();
                let resolved_opt = if is_builtin_module(spec) || is_jsx_runtime_builtin(spec) {
                    Some(normalize_builtin_specifier(spec))
                } else if let Some(resolved_path) = resolve_module_specifier(from_dir, spec) {
                    let id = canonical_mod_id(&resolved_path);
                    deps_out.push(resolved_path);
                    Some(id)
                } else {
                    None
                };
                if let Some(resolved) = resolved_opt {
                    let start = (str_lit.span.start as usize).saturating_sub(1);
                    let end = (str_lit.span.end as usize).saturating_sub(1);
                    edits.push((start, end, format!("{:?}", resolved)));
                }
            });
            if edits.is_empty() {
                return snippet.to_string();
            }
            edits.sort_by_key(|e| e.0);
            let mut out = String::with_capacity(snippet.len() + edits.len() * 32);
            let mut cursor = 0usize;
            for (s, e, rep) in edits {
                if s < cursor || e > snippet.len() {
                    continue;
                }
                out.push_str(&snippet[cursor..s]);
                out.push_str(&rep);
                cursor = e;
            }
            out.push_str(&snippet[cursor..]);
            return out;
        }
        return snippet
            .lines()
            .map(|l| rewrite_cjs_requires_in_line(l, from_dir, deps_out))
            .collect::<Vec<_>>()
            .join("\n");
    }

    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    walk_ast_call_expressions(&parser_ret.program, &mut |call| {
        let Expression::Identifier(callee) = call.callee.without_parentheses() else {
            return;
        };
        if callee.name.as_str() != "require" || call.arguments.len() != 1 {
            return;
        }
        let Some(Argument::StringLiteral(str_lit)) = call.arguments.first() else {
            return;
        };
        let spec = str_lit.value.as_str();
        let resolved_opt = if is_builtin_module(spec) || is_jsx_runtime_builtin(spec) {
            Some(normalize_builtin_specifier(spec))
        } else if let Some(resolved_path) = resolve_module_specifier(from_dir, spec) {
            let id = canonical_mod_id(&resolved_path);
            deps_out.push(resolved_path);
            Some(id)
        } else {
            None
        };
        if let Some(resolved) = resolved_opt {
            edits.push((
                str_lit.span.start as usize,
                str_lit.span.end as usize,
                format!("{:?}", resolved),
            ));
        }
    });

    if edits.is_empty() {
        return snippet.to_string();
    }

    edits.sort_by_key(|e| e.0);
    let mut out = String::with_capacity(snippet.len() + edits.len() * 32);
    let mut cursor = 0usize;
    for (s, e, rep) in edits {
        if s < cursor || e > snippet.len() {
            continue;
        }
        out.push_str(&snippet[cursor..s]);
        out.push_str(&rep);
        cursor = e;
    }
    out.push_str(&snippet[cursor..]);
    out
}

/// Rewrites `require(...)` callee identifiers to `__slice_cjs_require(...)` using Oxc AST
/// `CallExpression` spans so string literals, template literals, and comments are untouched.
pub(crate) fn rewrite_require_ident_ast(source: &str) -> Option<String> {
    use oxc::ast::ast::Expression;

    let allocator = oxc::allocator::Allocator::default();
    let source_type = oxc::span::SourceType::ts().with_module(true);
    let parser_ret = oxc::parser::Parser::new(&allocator, source, source_type).parse();
    if !parser_ret.diagnostics.is_empty() {
        return None;
    }

    let mut callee_spans: Vec<(usize, usize)> = Vec::new();
    walk_ast_call_expressions(&parser_ret.program, &mut |call| {
        if let Expression::Identifier(callee) = call.callee.without_parentheses() {
            if callee.name.as_str() == "require" {
                callee_spans.push((callee.span.start as usize, callee.span.end as usize));
            }
        }
    });

    if callee_spans.is_empty() {
        return Some(source.to_string());
    }

    callee_spans.sort_by_key(|s| s.0);
    let mut out = String::with_capacity(source.len() + callee_spans.len() * 12);
    let mut cursor = 0usize;
    for (s, e) in callee_spans {
        if s < cursor || e > source.len() {
            continue;
        }
        out.push_str(&source[cursor..s]);
        out.push_str("__slice_cjs_require");
        cursor = e;
    }
    out.push_str(&source[cursor..]);
    Some(out)
}

/// Lowers `Buffer.byteLength(<arg>)` and `Buffer.from(<arg>, <enc>?).toString(<to_enc>?)`
/// via Oxc AST `CallExpression` span replacement so string literals and comments are never touched.
pub(crate) fn rewrite_buffer_fast_paths_ast(code: &str) -> String {
    use oxc::ast::ast::{Argument, Expression};
    use oxc::span::GetSpan;

    if !code.contains("Buffer.") {
        return code.to_string();
    }

    let allocator = oxc::allocator::Allocator::default();
    let source_type = oxc::span::SourceType::ts().with_module(true);
    let parser_ret = oxc::parser::Parser::new(&allocator, code, source_type).parse();
    if !parser_ret.diagnostics.is_empty() {
        return code.to_string();
    }

    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    walk_ast_call_expressions(&parser_ret.program, &mut |call| {
        let Expression::StaticMemberExpression(outer_mem) = call.callee.without_parentheses()
        else {
            return;
        };
        let prop_name = outer_mem.property.name.as_str();

        // 1. `Buffer.byteLength(<arg>)`
        if prop_name == "byteLength" && call.arguments.len() == 1 {
            if let Expression::Identifier(obj_id) = outer_mem.object.without_parentheses() {
                if obj_id.name.as_str() == "Buffer" {
                    if let Some(arg_expr) = call.arguments[0].as_expression() {
                        let sp = arg_expr.span();
                        let arg_src = &code[sp.start as usize..sp.end as usize];
                        edits.push((
                            call.span.start as usize,
                            call.span.end as usize,
                            format!("globalThis.__slice_utf8_byte_len({})", arg_src),
                        ));
                    }
                }
            }
            return;
        }

        // 2. `Buffer.from(<arg>, <from_enc>?).toString(<to_enc>?)`
        if prop_name == "toString" && call.arguments.len() <= 1 {
            let Expression::CallExpression(inner_call) = outer_mem.object.without_parentheses()
            else {
                return;
            };
            let Expression::StaticMemberExpression(inner_mem) =
                inner_call.callee.without_parentheses()
            else {
                return;
            };
            let Expression::Identifier(obj_id) = inner_mem.object.without_parentheses() else {
                return;
            };
            if obj_id.name.as_str() != "Buffer"
                || inner_mem.property.name.as_str() != "from"
                || inner_call.arguments.is_empty()
                || inner_call.arguments.len() > 2
            {
                return;
            }
            let Some(first_expr) = inner_call.arguments[0].as_expression() else {
                return;
            };
            let fsp = first_expr.span();
            let first_src = &code[fsp.start as usize..fsp.end as usize];

            let from_enc = if inner_call.arguments.len() == 1 {
                Some("utf8")
            } else if let Argument::StringLiteral(sl) = &inner_call.arguments[1] {
                match sl.value.as_str() {
                    "utf8" | "utf-8" => Some("utf8"),
                    "base64" | "base64url" => Some("base64"),
                    _ => None,
                }
            } else {
                None
            };

            let to_enc = if call.arguments.is_empty() {
                Some("utf8")
            } else if let Argument::StringLiteral(sl) = &call.arguments[0] {
                Some(sl.value.as_str())
            } else {
                None
            };

            let rep_opt = match (from_enc, to_enc) {
                (Some("utf8"), Some("base64")) => Some(format!(
                    "globalThis.__slice_base64_encode({}, false)",
                    first_src
                )),
                (Some("utf8"), Some("base64url")) => Some(format!(
                    "globalThis.__slice_base64_encode({}, true)",
                    first_src
                )),
                (Some("utf8"), Some("utf8" | "utf-8")) => {
                    Some(format!("globalThis.__slice_buf_to_utf8({})", first_src))
                }
                (Some("base64"), Some("utf8" | "utf-8")) => {
                    Some(format!("globalThis.__slice_base64_decode({})", first_src))
                }
                _ => None,
            };

            if let Some(rep) = rep_opt {
                edits.push((call.span.start as usize, call.span.end as usize, rep));
            }
        }
    });

    if edits.is_empty() {
        return code.to_string();
    }

    edits.sort_by_key(|e| e.0);
    let mut out = String::with_capacity(code.len() + edits.len() * 32);
    let mut cursor = 0usize;
    for (s, e, rep) in edits {
        if s < cursor || e > code.len() {
            continue;
        }
        out.push_str(&code[cursor..s]);
        out.push_str(&rep);
        cursor = e;
    }
    out.push_str(&code[cursor..]);
    out
}

fn collect_require_edits_from_program(
    code: &str,
    program: &oxc::ast::ast::Program<'_>,
    from_dir: &Path,
    module_path: Option<&Path>,
    deps_out: &mut Vec<PathBuf>,
) -> Vec<(usize, usize, String)> {
    use oxc::ast::ast::{Argument, ChainElement, Expression};
    use oxc::span::GetSpan;

    if !code.contains("require(") && !code.contains("import(") && !code.contains("import.meta") {
        return Vec::new();
    }

    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    fn handle_call(
        call: &oxc::ast::ast::CallExpression<'_>,
        from_dir: &Path,
        deps_out: &mut Vec<PathBuf>,
        edits: &mut Vec<(usize, usize, String)>,
    ) {
        let Expression::Identifier(callee) = call.callee.without_parentheses() else {
            return;
        };
        if callee.name.as_str() != "require" || call.arguments.len() != 1 {
            return;
        };
        let Some(Argument::StringLiteral(str_lit)) = call.arguments.first() else {
            return;
        };
        let spec = str_lit.value.as_str();
        let resolved_opt = if is_builtin_module(spec) || is_jsx_runtime_builtin(spec) {
            Some(normalize_builtin_specifier(spec))
        } else if let Some(resolved_path) = resolve_module_specifier(from_dir, spec) {
            let id = canonical_mod_id(&resolved_path);
            deps_out.push(resolved_path);
            Some(id)
        } else {
            None
        };
        if let Some(resolved) = resolved_opt {
            edits.push((
                str_lit.span.start as usize,
                str_lit.span.end as usize,
                format!("{:?}", resolved),
            ));
        }
    }

    walk_ast_expressions(program, &mut |expr| match expr {
        Expression::CallExpression(call) => handle_call(call, from_dir, deps_out, &mut edits),
        Expression::ChainExpression(ch) => {
            if let ChainElement::CallExpression(call) = &ch.expression {
                handle_call(call, from_dir, deps_out, &mut edits);
            }
        }
        Expression::ImportExpression(imp_expr) => {
            if let Expression::StringLiteral(str_lit) = imp_expr.source.without_parentheses() {
                let spec = str_lit.value.as_str();
                let resolved_id = resolve_spec_and_record(spec, from_dir, deps_out);
                edits.push((
                    imp_expr.span.start as usize,
                    imp_expr.span.end as usize,
                    format!(
                        "Promise.resolve().then(function() {{ var __m = require({:?}); return (__m && __m.__esModule) ? __m : Object.assign({{ default: __m }}, __m); }})",
                        resolved_id
                    ),
                ));
            }
        }
        other
            if !matches!(
                other,
                Expression::StringLiteral(_)
                    | Expression::TemplateLiteral(_)
                    | Expression::ParenthesizedExpression(_)
            ) =>
        {
            let sp = other.span();
            let s = sp.start as usize;
            let e = sp.end as usize;
            if e.saturating_sub(s) == 11 && code.get(s..e) == Some("import.meta") {
                let dir_str = canonical_mod_id(from_dir);
                let file_str = module_path
                    .map(canonical_mod_id)
                    .unwrap_or_else(|| format!("{}/index.ts", dir_str));
                let file_url = if file_str.starts_with('/') {
                    format!("file://{}", file_str)
                } else {
                    format!("file:///{}", file_str)
                };
                edits.push((
                    s,
                    e,
                    format!(
                        "({{ url: {:?}, dirname: {:?}, filename: {:?}, env: (globalThis.process ? globalThis.process.env : {{}}) }})",
                        file_url, dir_str, file_str
                    ),
                ));
            }
        }
        _ => {}
    });
    edits.sort_by_key(|e| e.0);
    edits
}

/// Detects via Oxc AST whether `code` contains any top-level `await` expression or `for await`
/// loop outside nested function/method scopes.
pub(crate) fn has_top_level_await(code: &str) -> bool {
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

fn slice_with_edits(
    code: &str,
    span_start: usize,
    span_end: usize,
    edits: &[(usize, usize, String)],
) -> String {
    if edits.is_empty() {
        return code[span_start..span_end].to_string();
    }
    let mut out = String::with_capacity((span_end - span_start) + 32);
    let mut cursor = span_start;
    for (s, e, rep) in edits {
        if *e <= span_start {
            continue;
        }
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

fn apply_cjs_lazy_barrel_getters(mut body_lines: Vec<String>) -> Vec<String> {
    if body_lines.iter().any(|l| l.trim() == "module.exports = {") {
        let mut const_req_map: FxHashMap<String, (usize, String)> = FxHashMap::default();
        for (idx, line) in body_lines.iter().enumerate() {
            let t = line.trim().trim_end_matches(';');
            if let Some(rest) = t.strip_prefix("const ") {
                if let Some((ident, rhs)) = rest.split_once('=') {
                    let ident = ident.trim();
                    let rhs = rhs.trim();
                    if rhs.starts_with("require(\"") && rhs.ends_with("\")") {
                        const_req_map.insert(ident.to_string(), (idx, rhs.to_string()));
                    }
                }
            }
        }
        let mut to_remove_indices = FxHashSet::default();
        let mut shorthand_replacements: FxHashMap<usize, String> = FxHashMap::default();

        for (idx, line) in body_lines.iter().enumerate() {
            let t = line.trim();
            if let Some(ident) = t.strip_suffix(',') {
                let ident = ident.trim();
                if let Some((decl_idx, req_expr)) = const_req_map.get(ident) {
                    let used_elsewhere =
                        body_lines
                            .iter()
                            .enumerate()
                            .any(|(other_idx, other_line)| {
                                other_idx != idx
                                    && other_idx != *decl_idx
                                    && other_line.contains(ident)
                            });
                    if !used_elsewhere {
                        to_remove_indices.insert(*decl_idx);
                        shorthand_replacements.insert(
                            idx,
                            format!("  get {}() {{ return {}; }},", ident, req_expr),
                        );
                    }
                }
            }
        }
        if !shorthand_replacements.is_empty() || !to_remove_indices.is_empty() {
            let mut optimized_lines = Vec::with_capacity(body_lines.len());
            for (idx, line) in body_lines.into_iter().enumerate() {
                if to_remove_indices.contains(&idx) {
                    continue;
                }
                if let Some(rep) = shorthand_replacements.remove(&idx) {
                    optimized_lines.push(rep);
                } else {
                    optimized_lines.push(line);
                }
            }
            body_lines = optimized_lines;
        }
    }
    body_lines
}

fn try_transform_esm_to_cjs_ast(
    js_code: &str,
    from_dir: &Path,
    module_path: Option<&Path>,
    deps_out: &mut Vec<PathBuf>,
) -> Option<String> {
    use oxc::ast::ast::{
        Declaration, ExportDefaultDeclarationKind, ImportDeclarationSpecifier, Statement,
    };
    use oxc::span::GetSpan;

    let allocator = oxc::allocator::Allocator::default();
    let source_type = oxc::span::SourceType::ts().with_module(true);
    let parser_ret = oxc::parser::Parser::new(&allocator, js_code, source_type).parse();
    if !parser_ret.diagnostics.is_empty() {
        return None;
    }

    // Single-pass `require(...)`, dynamic `import(...)`, and `import.meta` AST edit collection:
    let req_edits = collect_require_edits_from_program(
        js_code,
        &parser_ret.program,
        from_dir,
        module_path,
        deps_out,
    );

    let has_esm_stmt = parser_ret.program.body.iter().any(|stmt| {
        matches!(
            stmt,
            Statement::ImportDeclaration(_)
                | Statement::ExportDeclaration(_)
                | Statement::ExportNamedDeclaration(_)
                | Statement::ExportFromDeclaration(_)
                | Statement::ExportDefaultDeclaration(_)
                | Statement::ExportAllDeclaration(_)
        )
    });
    if !has_esm_stmt {
        let rewritten_cjs = slice_with_edits(js_code, 0, js_code.len(), &req_edits);
        let body_lines: Vec<String> = rewritten_cjs
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.to_string())
            .collect();
        return Some(apply_cjs_lazy_barrel_getters(body_lines).join("\n"));
    }

    let mut body_lines = Vec::with_capacity(parser_ret.program.body.len() + 4);
    let mut deferred_exports: Vec<(String, String)> = Vec::new();
    let mut has_default_export = false;
    let mut req_counter = 0usize;

    for stmt in &parser_ret.program.body {
        match stmt {
            Statement::ImportDeclaration(imp) => {
                if imp.import_kind.is_type() {
                    continue;
                }
                let spec = imp.source.value.as_str();
                let resolved_id = resolve_spec_and_record(spec, from_dir, deps_out);
                let Some(specs) = &imp.specifiers else {
                    body_lines.push(format!("require({:?});", resolved_id));
                    continue;
                };
                if specs.is_empty() {
                    body_lines.push(format!("require({:?});", resolved_id));
                    continue;
                }
                let mut default_ident: Option<&str> = None;
                let mut namespace_ident: Option<&str> = None;
                let mut named_specs: Vec<(String, String)> = Vec::new();

                for s in specs {
                    match s {
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(def) => {
                            default_ident = Some(def.local.name.as_str());
                        }
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(ns) => {
                            namespace_ident = Some(ns.local.name.as_str());
                        }
                        ImportDeclarationSpecifier::ImportSpecifier(named) => {
                            if named.import_kind.is_type() {
                                continue;
                            }
                            named_specs.push((
                                named.imported.name().to_string(),
                                named.local.name.to_string(),
                            ));
                        }
                    }
                }

                if default_ident.is_none() && namespace_ident.is_none() && named_specs.is_empty() {
                    continue;
                }

                if let Some(ns) = namespace_ident {
                    body_lines.push(format!("const {} = require({:?});", ns, resolved_id));
                }
                if let Some(def_name) = default_ident {
                    req_counter += 1;
                    let tmp_var = format!("__imp_{}", req_counter);
                    if !named_specs.is_empty() {
                        let fields: Vec<String> = named_specs
                            .iter()
                            .map(|(imp_n, loc_n)| {
                                if imp_n == loc_n {
                                    imp_n.clone()
                                } else {
                                    format!("{}: {}", imp_n, loc_n)
                                }
                            })
                            .collect();
                        body_lines.push(format!(
                            "const {tmp} = require({id:?}); const {def} = ({tmp} && {tmp}.__esModule && {tmp}.default !== undefined) ? {tmp}.default : ({tmp} && {tmp}.default !== undefined ? {tmp}.default : {tmp}); const {{ {destr} }} = {tmp};",
                            tmp = tmp_var,
                            id = resolved_id,
                            def = def_name,
                            destr = fields.join(", ")
                        ));
                    } else {
                        body_lines.push(format!(
                            "const {tmp} = require({id:?}); const {def} = ({tmp} && {tmp}.__esModule && {tmp}.default !== undefined) ? {tmp}.default : ({tmp} && {tmp}.default !== undefined ? {tmp}.default : {tmp});",
                            tmp = tmp_var,
                            id = resolved_id,
                            def = def_name
                        ));
                    }
                } else if !named_specs.is_empty() {
                    let fields: Vec<String> = named_specs
                        .iter()
                        .map(|(imp_n, loc_n)| {
                            if imp_n == loc_n {
                                imp_n.clone()
                            } else {
                                format!("{}: {}", imp_n, loc_n)
                            }
                        })
                        .collect();
                    body_lines.push(format!(
                        "const {{ {} }} = require({:?});",
                        fields.join(", "),
                        resolved_id
                    ));
                }
            }
            Statement::ExportAllDeclaration(exp_all) => {
                if exp_all.export_kind.is_type() {
                    continue;
                }
                let spec = exp_all.source.value.as_str();
                let resolved_id = resolve_spec_and_record(spec, from_dir, deps_out);
                if let Some(exported) = &exp_all.exported {
                    body_lines.push(format!(
                        "exports.{} = require({:?});",
                        exported.name(),
                        resolved_id
                    ));
                } else {
                    body_lines.push(format!(
                        "Object.assign(exports, require({:?}));",
                        resolved_id
                    ));
                }
            }
            Statement::ExportDefaultDeclaration(exp_def) => {
                has_default_export = true;
                match &exp_def.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
                        let fn_src = slice_with_edits(
                            js_code,
                            func.span.start as usize,
                            func.span.end as usize,
                            &req_edits,
                        );
                        if let Some(id) = &func.id {
                            body_lines.push(fn_src);
                            body_lines.push(format!("const __slice_default = {};", id.name));
                        } else {
                            body_lines.push(format!("const __slice_default = {};", fn_src));
                        }
                    }
                    ExportDefaultDeclarationKind::ClassDeclaration(cls) => {
                        let cls_src = slice_with_edits(
                            js_code,
                            cls.span.start as usize,
                            cls.span.end as usize,
                            &req_edits,
                        );
                        if let Some(id) = &cls.id {
                            body_lines.push(cls_src);
                            body_lines.push(format!("const __slice_default = {};", id.name));
                        } else {
                            body_lines.push(format!("const __slice_default = {};", cls_src));
                        }
                    }
                    ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => {
                        has_default_export = false;
                    }
                    expr => {
                        let sp = expr.span();
                        let expr_src = slice_with_edits(
                            js_code,
                            sp.start as usize,
                            sp.end as usize,
                            &req_edits,
                        );
                        body_lines.push(format!("const __slice_default = {};", expr_src));
                    }
                }
            }
            Statement::ExportDeclaration(exp_decl) => {
                let decl = &exp_decl.declaration;
                let sp = decl.span();
                let decl_src =
                    slice_with_edits(js_code, sp.start as usize, sp.end as usize, &req_edits);
                match decl {
                    Declaration::VariableDeclaration(vd) => {
                        body_lines.push(decl_src);
                        for d in &vd.declarations {
                            collect_binding_pattern_names(&d.id, &mut deferred_exports);
                        }
                    }
                    Declaration::FunctionDeclaration(fd) => {
                        body_lines.push(decl_src);
                        if let Some(id) = &fd.id {
                            deferred_exports.push((id.name.to_string(), id.name.to_string()));
                        }
                    }
                    Declaration::ClassDeclaration(cd) => {
                        body_lines.push(decl_src);
                        if let Some(id) = &cd.id {
                            deferred_exports.push((id.name.to_string(), id.name.to_string()));
                        }
                    }
                    _ => {}
                }
            }
            Statement::ExportNamedDeclaration(exp_named) => {
                if exp_named.export_kind.is_type() {
                    continue;
                }
                for spec in &exp_named.specifiers {
                    if spec.export_kind.is_type() {
                        continue;
                    }
                    let local_n = spec.local.name().to_string();
                    let exported_n = spec.exported.name().to_string();
                    if exported_n == "default"
                        && exp_named.specifiers.len() == 1
                        && deferred_exports.is_empty()
                    {
                        has_default_export = true;
                        body_lines.push(format!("const __slice_default = {};", local_n));
                    } else {
                        deferred_exports.push((exported_n, local_n));
                    }
                }
            }
            Statement::ExportFromDeclaration(exp_from) => {
                if exp_from.export_kind.is_type() {
                    continue;
                }
                let spec_str = exp_from.source.value.as_str();
                let resolved_id = resolve_spec_and_record(spec_str, from_dir, deps_out);
                req_counter += 1;
                let tmp_var = format!("__reexp_{}", req_counter);
                let mut has_runtime_spec = false;
                let mut reexp_lines = Vec::new();
                for spec in &exp_from.specifiers {
                    if spec.export_kind.is_type() {
                        continue;
                    }
                    has_runtime_spec = true;
                    let orig_n = spec.local.name();
                    let exp_n = spec.exported.name();
                    if orig_n == "default" && exp_n == "default" {
                        has_default_export = true;
                        reexp_lines.push(format!(
                            "const __slice_default = ({tmp} && {tmp}.__esModule && {tmp}.default !== undefined) ? {tmp}.default : ({tmp} && {tmp}.default !== undefined ? {tmp}.default : {tmp});",
                            tmp = tmp_var
                        ));
                    } else if orig_n == "default" {
                        deferred_exports.push((
                            exp_n.to_string(),
                            format!(
                                "(({tmp} && {tmp}.default !== undefined) ? {tmp}.default : {tmp})",
                                tmp = tmp_var
                            ),
                        ));
                    } else {
                        deferred_exports
                            .push((exp_n.to_string(), format!("{}.{}", tmp_var, orig_n)));
                    }
                }
                if has_runtime_spec {
                    body_lines.push(format!("const {} = require({:?});", tmp_var, resolved_id));
                    body_lines.extend(reexp_lines);
                }
            }
            other => {
                let sp = other.span();
                body_lines.push(slice_with_edits(
                    js_code,
                    sp.start as usize,
                    sp.end as usize,
                    &req_edits,
                ));
            }
        }
    }

    let uses_crypto_obj = body_lines.iter().any(|l| {
        !l.contains("require(\"node:crypto\")")
            && (l.contains("crypto.")
                || l.contains("createHash(")
                || l.contains("createHmac(")
                || l.contains("randomBytes(")
                || l.contains("randomUUID(")
                || l.contains("getRandomValues(")
                || l.contains("timingSafeEqual("))
    });
    if !uses_crypto_obj {
        body_lines.retain(|l| !l.contains("require(\"node:crypto\")"));
    }

    if has_default_export || !deferred_exports.is_empty() {
        body_lines.push("exports.__esModule = true;".to_string());
    }
    for (export_name, local_name) in &deferred_exports {
        body_lines.push(format!("exports.{} = {};", export_name, local_name));
    }
    if has_default_export {
        body_lines.push("exports.default = __slice_default;".to_string());
        if deferred_exports.is_empty() {
            body_lines.push(
                "module.exports = __slice_default; if (typeof module.exports === 'object' && module.exports !== null || typeof module.exports === 'function') { module.exports.default = __slice_default; module.exports.__esModule = true; }"
                    .to_string(),
            );
        }
    }

    Some(body_lines.join("\n"))
}

pub(crate) fn transform_module_to_cjs(
    js_code: &str,
    from_dir: &Path,
    module_path: Option<&Path>,
    deps_out: &mut Vec<PathBuf>,
) -> String {
    if let Some(ast_transformed) =
        try_transform_esm_to_cjs_ast(js_code, from_dir, module_path, deps_out)
    {
        return ast_transformed;
    }

    // Fallback CommonJS module path if top-level parse failed
    if !js_code.contains("import ") && !js_code.contains("export ") {
        let rewritten_cjs = rewrite_cjs_requires_ast(js_code, from_dir, deps_out);
        let body_lines: Vec<String> = rewritten_cjs
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.to_string())
            .collect();
        return apply_cjs_lazy_barrel_getters(body_lines).join("\n");
    }

    let collapsed = collapse_multiline_imports_exports(js_code);
    let mut body_lines = Vec::new();
    let mut deferred_exports: Vec<(String, String)> = Vec::new();
    let mut has_default_export = false;

    for raw_line in collapsed.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(after_import) = trimmed.strip_prefix("import ") {
            if after_import.starts_with("type ") {
                continue;
            }
            if let Some((spec, _)) = extract_quoted_specifier(after_import) {
                let resolved_id = resolve_spec_and_record(&spec, from_dir, deps_out);
                body_lines.push(format!("require({:?});", resolved_id));
                continue;
            }
            if let Some((clause, from_part)) = after_import.split_once(" from ") {
                if let Some((spec, _)) = extract_quoted_specifier(from_part) {
                    let resolved_id = resolve_spec_and_record(&spec, from_dir, deps_out);
                    let clause = clause.trim();
                    if let Some(ns_name) = clause.strip_prefix("* as ") {
                        let ns = ns_name.trim();
                        body_lines.push(format!("const {} = require({:?});", ns, resolved_id));
                    } else if clause.starts_with('{') && clause.ends_with('}') {
                        let inner = &clause[1..clause.len() - 1];
                        let mut fields = Vec::new();
                        for part in inner.split(',') {
                            let p = part.trim();
                            if p.is_empty() || p.starts_with("type ") {
                                continue;
                            }
                            if let Some((orig, alias)) = p.split_once(" as ") {
                                fields.push(format!("{}: {}", orig.trim(), alias.trim()));
                            } else {
                                fields.push(p.to_string());
                            }
                        }
                        if !fields.is_empty() {
                            body_lines.push(format!(
                                "const {{ {} }} = require({:?});",
                                fields.join(", "),
                                resolved_id
                            ));
                        }
                    } else {
                        let def_name = clause.trim();
                        body_lines.push(format!(
                            "const __imp_{} = require({:?}); const {} = (__imp_{} && __imp_{}.__esModule && __imp_{}.default !== undefined) ? __imp_{}.default : __imp_{};",
                            def_name, resolved_id, def_name, def_name, def_name, def_name, def_name, def_name
                        ));
                    }
                    continue;
                }
            }
        }

        if let Some(after_export) = trimmed.strip_prefix("export ") {
            let after_export = after_export.trim();
            if after_export.starts_with("type ") || after_export.starts_with("interface ") {
                continue;
            }
            if let Some(after_star) = after_export.strip_prefix("* from ") {
                if let Some((spec, _)) = extract_quoted_specifier(after_star) {
                    let resolved_id = resolve_spec_and_record(&spec, from_dir, deps_out);
                    body_lines.push(format!(
                        "Object.assign(exports, require({:?}));",
                        resolved_id
                    ));
                    continue;
                }
            }
            if after_export.starts_with('{') {
                if let Some((brace_part, from_part)) = after_export.split_once(" from ") {
                    if let Some((spec, _)) = extract_quoted_specifier(from_part) {
                        let resolved_id = resolve_spec_and_record(&spec, from_dir, deps_out);
                        let inner = brace_part
                            .trim()
                            .trim_start_matches('{')
                            .trim_end_matches('}');
                        body_lines.push(format!("const __reexp = require({:?});", resolved_id));
                        for item in inner.split(',') {
                            let item = item.trim();
                            if item.is_empty() || item.starts_with("type ") {
                                continue;
                            }
                            if let Some((orig, alias)) = item.split_once(" as ") {
                                body_lines.push(format!(
                                    "exports.{} = __reexp.{};",
                                    alias.trim(),
                                    orig.trim()
                                ));
                            } else {
                                body_lines.push(format!("exports.{} = __reexp.{};", item, item));
                            }
                        }
                        continue;
                    }
                } else if let Some(end_b) = after_export.find('}') {
                    let inner = &after_export[1..end_b];
                    for item in inner.split(',') {
                        let item = item.trim();
                        if item.is_empty() || item.starts_with("type ") {
                            continue;
                        }
                        if let Some((orig, alias)) = item.split_once(" as ") {
                            deferred_exports
                                .push((alias.trim().to_string(), orig.trim().to_string()));
                        } else {
                            deferred_exports.push((item.to_string(), item.to_string()));
                        }
                    }
                    continue;
                }
            }
            if let Some(def_expr) = after_export.strip_prefix("default ") {
                has_default_export = true;
                body_lines.push(format!("const __slice_default = {}", def_expr));
                continue;
            }
            for kw in ["const ", "let ", "var "] {
                if let Some(decl_rest) = after_export.strip_prefix(kw) {
                    if let Some(ident) = decl_rest
                        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '$')
                        .next()
                    {
                        if !ident.is_empty() {
                            deferred_exports.push((ident.to_string(), ident.to_string()));
                        }
                    }
                    body_lines.push(rewrite_cjs_requires_ast(after_export, from_dir, deps_out));
                    break;
                }
            }
            if after_export.starts_with("const ")
                || after_export.starts_with("let ")
                || after_export.starts_with("var ")
            {
                continue;
            }
            for kw in ["async function ", "function ", "class "] {
                if let Some(decl_rest) = after_export.strip_prefix(kw) {
                    if let Some(ident) = decl_rest
                        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '$')
                        .next()
                    {
                        if !ident.is_empty() {
                            deferred_exports.push((ident.to_string(), ident.to_string()));
                        }
                    }
                    body_lines.push(rewrite_cjs_requires_ast(after_export, from_dir, deps_out));
                    break;
                }
            }
            if after_export.starts_with("async function ")
                || after_export.starts_with("function ")
                || after_export.starts_with("class ")
            {
                continue;
            }
        }

        body_lines.push(rewrite_cjs_requires_ast(raw_line, from_dir, deps_out));
    }

    let uses_crypto_obj = body_lines.iter().any(|l| {
        !l.contains("require(\"node:crypto\")")
            && (l.contains("crypto.")
                || l.contains("createHash(")
                || l.contains("createHmac(")
                || l.contains("randomBytes(")
                || l.contains("randomUUID(")
                || l.contains("getRandomValues(")
                || l.contains("timingSafeEqual("))
    });
    if !uses_crypto_obj {
        body_lines.retain(|l| !l.contains("require(\"node:crypto\")"));
    }

    if body_lines.iter().any(|l| l.trim() == "module.exports = {") {
        let mut const_req_map: FxHashMap<String, (usize, String)> = FxHashMap::default();
        for (idx, line) in body_lines.iter().enumerate() {
            let t = line.trim().trim_end_matches(';');
            if let Some(rest) = t.strip_prefix("const ") {
                if let Some((ident, rhs)) = rest.split_once('=') {
                    let ident = ident.trim();
                    let rhs = rhs.trim();
                    if rhs.starts_with("require(\"") && rhs.ends_with("\")") {
                        const_req_map.insert(ident.to_string(), (idx, rhs.to_string()));
                    }
                }
            }
        }
        let mut to_remove_indices = FxHashSet::default();
        let mut shorthand_replacements: FxHashMap<usize, String> = FxHashMap::default();

        for (idx, line) in body_lines.iter().enumerate() {
            let t = line.trim();
            if let Some(ident) = t.strip_suffix(',') {
                let ident = ident.trim();
                if let Some((decl_idx, req_expr)) = const_req_map.get(ident) {
                    let used_elsewhere =
                        body_lines
                            .iter()
                            .enumerate()
                            .any(|(other_idx, other_line)| {
                                other_idx != idx
                                    && other_idx != *decl_idx
                                    && other_line.contains(ident)
                            });
                    if !used_elsewhere {
                        to_remove_indices.insert(*decl_idx);
                        shorthand_replacements.insert(
                            idx,
                            format!("  get {}() {{ return {}; }},", ident, req_expr),
                        );
                    }
                }
            }
        }
        if !shorthand_replacements.is_empty() || !to_remove_indices.is_empty() {
            let mut optimized_lines = Vec::with_capacity(body_lines.len());
            for (idx, line) in body_lines.into_iter().enumerate() {
                if to_remove_indices.contains(&idx) {
                    continue;
                }
                if let Some(rep) = shorthand_replacements.remove(&idx) {
                    optimized_lines.push(rep);
                } else {
                    optimized_lines.push(line);
                }
            }
            body_lines = optimized_lines;
        }
    }

    if has_default_export || !deferred_exports.is_empty() {
        body_lines.push("exports.__esModule = true;".to_string());
    }
    for (export_name, local_name) in &deferred_exports {
        body_lines.push(format!("exports.{} = {};", export_name, local_name));
    }
    if has_default_export {
        body_lines.push("exports.default = __slice_default;".to_string());
        if deferred_exports.is_empty() {
            body_lines.push(
                "module.exports = __slice_default; if (typeof module.exports === 'object' && module.exports !== null || typeof module.exports === 'function') { module.exports.default = __slice_default; module.exports.__esModule = true; }"
                    .to_string(),
            );
        }
    }

    body_lines.join("\n")
}

#[derive(Debug, Clone)]
pub(crate) enum UsedExports {
    All,
    Unreferenced,
    Subset(FxHashSet<String>),
}

#[inline]
fn is_js_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$'
}

#[inline]
fn is_js_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

#[inline]
fn is_js_keyword(s: &str) -> bool {
    matches!(
        s,
        "const"
            | "let"
            | "var"
            | "function"
            | "return"
            | "if"
            | "else"
            | "for"
            | "while"
            | "do"
            | "switch"
            | "case"
            | "default"
            | "break"
            | "continue"
            | "try"
            | "catch"
            | "finally"
            | "throw"
            | "new"
            | "class"
            | "extends"
            | "super"
            | "this"
            | "typeof"
            | "instanceof"
            | "in"
            | "void"
            | "delete"
            | "await"
            | "yield"
            | "true"
            | "false"
            | "null"
            | "undefined"
    )
}

/// Scans `snippet` for all referenced free identifier names, skipping comments, string literals,
/// static template literal segments (while scanning `${...}` interpolations), RegExp literals,
/// and `.prop` member access names.
pub(crate) fn extract_referenced_identifiers_with_offsets(
    snippet: &str,
    base_offset: u32,
) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    let bytes = snippet.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let mut prev_sig_byte = 0u8;
    let mut prev_was_regex_kw = false;
    let mut in_template = false;
    let mut brace_depth = 0usize;
    let mut tpl_brace_stack: Vec<usize> = Vec::new();

    while i < len {
        if in_template {
            let b = bytes[i];
            if b == b'\\' {
                i = (i + 2).min(len);
                continue;
            }
            if b == b'`' {
                in_template = false;
                prev_sig_byte = b'`';
                prev_was_regex_kw = false;
                i += 1;
                continue;
            }
            if b == b'$' && i + 1 < len && bytes[i + 1] == b'{' {
                tpl_brace_stack.push(brace_depth);
                in_template = false;
                prev_sig_byte = b'{';
                prev_was_regex_kw = false;
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }

        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Line comment
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // Block comment
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(len);
            continue;
        }

        // RegExp literal
        if b == b'/' {
            let can_start_regex = prev_was_regex_kw
                || prev_sig_byte == 0
                || matches!(
                    prev_sig_byte,
                    b'(' | b','
                        | b'='
                        | b':'
                        | b'['
                        | b'!'
                        | b'&'
                        | b'|'
                        | b'?'
                        | b'{'
                        | b'}'
                        | b';'
                        | b'+'
                        | b'-'
                        | b'*'
                        | b'%'
                        | b'^'
                        | b'~'
                        | b'<'
                        | b'>'
                );
            if can_start_regex {
                i += 1;
                let mut in_class = false;
                while i < len {
                    let rb = bytes[i];
                    if rb == b'\\' {
                        i = (i + 2).min(len);
                        continue;
                    }
                    if rb == b'[' {
                        in_class = true;
                    } else if rb == b']' {
                        in_class = false;
                    } else if rb == b'/' && !in_class {
                        i += 1;
                        while i < len && bytes[i].is_ascii_alphabetic() {
                            i += 1;
                        }
                        break;
                    } else if rb == b'\n' {
                        break;
                    }
                    i += 1;
                }
                prev_sig_byte = b'/';
                prev_was_regex_kw = false;
                continue;
            }
        }

        // Quote literal
        if b == b'\'' || b == b'"' {
            let q = b;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(len);
                    continue;
                }
                if bytes[i] == q {
                    i += 1;
                    break;
                }
                i += 1;
            }
            prev_sig_byte = q;
            prev_was_regex_kw = false;
            continue;
        }

        if b == b'`' {
            in_template = true;
            i += 1;
            continue;
        }

        if b == b'{' {
            brace_depth += 1;
            prev_sig_byte = b'{';
            prev_was_regex_kw = false;
            i += 1;
            continue;
        }

        if b == b'}' {
            if tpl_brace_stack.last() == Some(&brace_depth) {
                tpl_brace_stack.pop();
                in_template = true;
            } else {
                brace_depth = brace_depth.saturating_sub(1);
            }
            prev_sig_byte = b'}';
            prev_was_regex_kw = false;
            i += 1;
            continue;
        }

        if is_js_ident_start(b) {
            let start = i;
            i += 1;
            while i < len && is_js_ident_continue(bytes[i]) {
                i += 1;
            }
            let ident = &snippet[start..i];
            prev_was_regex_kw = matches!(
                ident,
                "return"
                    | "typeof"
                    | "void"
                    | "delete"
                    | "in"
                    | "of"
                    | "instanceof"
                    | "case"
                    | "throw"
                    | "else"
                    | "yield"
                    | "await"
            );
            if (prev_sig_byte != b'.' || (start >= 3 && &bytes[start - 3..start] == b"..."))
                && !is_js_keyword(ident)
            {
                out.push((ident.to_string(), base_offset + start as u32));
            }
            prev_sig_byte = bytes[i - 1];
            continue;
        }

        prev_sig_byte = b;
        prev_was_regex_kw = false;
        i += 1;
    }

    out
}

pub(crate) fn extract_referenced_identifiers(snippet: &str) -> FxHashSet<String> {
    extract_referenced_identifiers_with_offsets(snippet, 0)
        .into_iter()
        .map(|(n, _)| n)
        .collect()
}

fn is_require_call_for_target(expr: &oxc::ast::ast::Expression<'_>, target_mod_id: &str) -> bool {
    use oxc::ast::ast::{Argument, Expression};
    let Expression::CallExpression(call) = expr.without_parentheses() else {
        return false;
    };
    let Expression::Identifier(callee) = call.callee.without_parentheses() else {
        return false;
    };
    if callee.name.as_str() != "require" || call.arguments.len() != 1 {
        return false;
    }
    let Some(Argument::StringLiteral(str_lit)) = call.arguments.first() else {
        return false;
    };
    str_lit.value.as_str() == target_mod_id
}

fn collect_member_accesses_on_var(
    code: &str,
    var_name: &str,
    skip_span: (usize, usize),
    used_out: &mut FxHashSet<String>,
) -> bool {
    let bytes = code.as_bytes();
    let vbytes = var_name.as_bytes();
    let vlen = vbytes.len();
    let mut pos = 0usize;
    while let Some(rel) = code[pos..].find(var_name) {
        let idx = pos + rel;
        let end = idx + vlen;
        pos = end;
        if idx >= skip_span.0 && end <= skip_span.1 {
            continue;
        }
        let valid_start =
            idx == 0 || (!is_js_ident_continue(bytes[idx - 1]) && bytes[idx - 1] != b'.');
        let valid_end = end == bytes.len() || !is_js_ident_continue(bytes[end]);
        if !valid_start || !valid_end {
            continue;
        }
        let after = code[end..].trim_start();
        let Some(after_dot) = after.strip_prefix('.') else {
            return false;
        };
        let prop: String = after_dot
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
            .collect();
        if prop.is_empty() {
            return false;
        }
        used_out.insert(prop);
    }
    true
}

/// Inspects all other modules in the bundle to determine whether `target_mod_id` is:
/// - `UsedExports::Unreferenced` (0 `require(target_mod_id)` calls remain in the bundle),
/// - `UsedExports::Subset(names)` (every consumer accesses a statically known set of named exports), or
/// - `UsedExports::All` (consumed via default import, namespace value, or dynamic access).
pub(crate) fn analyze_module_used_exports(
    target_mod_id: &str,
    other_modules: &[&str],
) -> UsedExports {
    use oxc::ast::ast::{BindingPattern, Statement};

    let mut total_occurrences = 0usize;
    let mut used_set = FxHashSet::default();

    for &consumer_code in other_modules {
        if !consumer_code.contains(target_mod_id) {
            continue;
        }
        let occurrences_in_consumer = consumer_code.matches(target_mod_id).count();
        total_occurrences += occurrences_in_consumer;

        let allocator = oxc::allocator::Allocator::default();
        let source_type = oxc::span::SourceType::mjs().with_module(false);
        let parser_ret = oxc::parser::Parser::new(&allocator, consumer_code, source_type).parse();
        if !parser_ret.diagnostics.is_empty() {
            return UsedExports::All;
        }

        let mut accounted = 0usize;
        for stmt in &parser_ret.program.body {
            let Statement::VariableDeclaration(vd) = stmt else {
                continue;
            };
            for decl in &vd.declarations {
                let Some(init) = &decl.init else {
                    continue;
                };
                if !is_require_call_for_target(init, target_mod_id) {
                    continue;
                }
                match &decl.id {
                    BindingPattern::ObjectPattern(obj_pat) => {
                        if obj_pat.rest.is_some() {
                            return UsedExports::All;
                        }
                        for prop in &obj_pat.properties {
                            let Some(key_name) = prop.key.name() else {
                                return UsedExports::All;
                            };
                            used_set.insert(key_name.to_string());
                        }
                        accounted += 1;
                    }
                    BindingPattern::BindingIdentifier(ident) => {
                        let var_name = ident.name.as_str();
                        if var_name.starts_with("__imp_") {
                            return UsedExports::All;
                        }
                        if !collect_member_accesses_on_var(
                            consumer_code,
                            var_name,
                            (decl.span.start as usize, decl.span.end as usize),
                            &mut used_set,
                        ) {
                            return UsedExports::All;
                        }
                        accounted += 1;
                    }
                    _ => return UsedExports::All,
                }
            }
        }

        // Also check direct `require("target_mod_id").Prop` inline member accesses
        for pat in [
            format!("require({:?}).", target_mod_id),
            format!("require('{}').", target_mod_id),
        ] {
            let mut search_pos = 0usize;
            while let Some(rel) = consumer_code[search_pos..].find(&pat) {
                let after_dot = &consumer_code[search_pos + rel + pat.len()..];
                let prop: String = after_dot
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
                    .collect();
                if !prop.is_empty() {
                    used_set.insert(prop);
                    accounted += 1;
                }
                search_pos += rel + pat.len();
            }
        }

        if accounted != occurrences_in_consumer {
            return UsedExports::All;
        }
    }

    if total_occurrences == 0 {
        UsedExports::Unreferenced
    } else {
        UsedExports::Subset(used_set)
    }
}

fn pattern_bound_names(pat: &oxc::ast::ast::BindingPattern<'_>, out: &mut Vec<String>) {
    use oxc::ast::ast::BindingPattern;
    match pat {
        BindingPattern::BindingIdentifier(id) => out.push(id.name.to_string()),
        BindingPattern::ObjectPattern(obj) => {
            for p in &obj.properties {
                pattern_bound_names(&p.value, out);
            }
            if let Some(r) = &obj.rest {
                pattern_bound_names(&r.argument, out);
            }
        }
        BindingPattern::ArrayPattern(arr) => {
            for el in arr.elements.iter().flatten() {
                pattern_bound_names(el, out);
            }
            if let Some(r) = &arr.rest {
                pattern_bound_names(&r.argument, out);
            }
        }
        BindingPattern::AssignmentPattern(assign) => {
            pattern_bound_names(&assign.left, out);
        }
    }
}

fn pattern_has_live(pat: &oxc::ast::ast::BindingPattern<'_>, live: &FxHashSet<String>) -> bool {
    let mut names = Vec::new();
    pattern_bound_names(pat, &mut names);
    names.iter().any(|n| live.contains(n))
}

fn is_any_require_call(expr: &oxc::ast::ast::Expression<'_>) -> Option<String> {
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

fn match_exports_assignment_stmt(
    stmt: &oxc::ast::ast::Statement<'_>,
    src: &str,
) -> Option<(String, FxHashSet<String>)> {
    use oxc::ast::ast::Statement;
    use oxc::span::GetSpan;
    let Statement::ExpressionStatement(_) = stmt else {
        return None;
    };
    let sp = stmt.span();
    let text = src[sp.start as usize..sp.end as usize].trim();
    let after_exp = text.strip_prefix("exports.")?;
    let eq_idx = after_exp.find('=')?;
    if after_exp[eq_idx + 1..].starts_with('=') {
        return None;
    }
    let exp_name = after_exp[..eq_idx].trim();
    if exp_name.is_empty()
        || !exp_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
    {
        return None;
    }
    let rhs = after_exp[eq_idx + 1..].trim().trim_end_matches(';').trim();
    if rhs.contains("exports.") {
        return None;
    }
    Some((exp_name.to_string(), extract_referenced_identifiers(rhs)))
}

/// General-Purpose Oxc AST Dead-Declaration & Dead-Export Tree-Shaker (`prune_and_minify_cjs_module_ast`):
/// - Computes the transitive closure of top-level declarations (`function`, `class`, and per-`VariableDeclarator`
///   `const`/`let`/`var` bindings) reachable from `used_exports` and top-level side effects.
/// - Prunes unreferenced `exports.<Unused> = ...` assignments, unreferenced top-level functions/classes/variables,
///   and unused destructured properties in `const { a, b } = require("...")`.
/// - Emits clean, comment-free (and optionally minified) JavaScript via `oxc::codegen::Codegen`.
pub(crate) fn prune_and_minify_cjs_module_ast(
    cjs_code: &str,
    used_exports: Option<&FxHashSet<String>>,
    minify: bool,
) -> String {
    use oxc::ast::ast::{BindingPattern, Statement};
    use oxc::span::GetSpan;

    let allocator = oxc::allocator::Allocator::default();
    let source_type = oxc::span::SourceType::mjs().with_module(false);
    let parser_ret = oxc::parser::Parser::new(&allocator, cjs_code, source_type).parse();
    if !parser_ret.diagnostics.is_empty() {
        return cjs_code.to_string();
    }

    let mut program = parser_ret.program;

    if let Some(used_set) = used_exports {
        let has_named_export_stmts = program.body.iter().any(|s| {
            match_exports_assignment_stmt(s, cjs_code).is_some_and(|(name, _)| name != "__esModule")
        });

        if has_named_export_stmts {
            // Step 1: Remove unused `exports.<Unused> = ...` and unused `module.exports = __slice_default`
            program.body.retain(|stmt| {
                if let Some((exp_name, _)) = match_exports_assignment_stmt(stmt, cjs_code) {
                    exp_name == "__esModule" || used_set.contains(&exp_name)
                } else {
                    let sp = stmt.span();
                    let text = cjs_code[sp.start as usize..sp.end as usize].trim();
                    !text.starts_with("module.exports = __slice_default;")
                        || used_set.contains("default")
                }
            });

            // Step 2: Iterative Oxc Semantic Scope-Resolved Zero-Reference Pruning.
            // `scoping.get_resolved_reference_ids(sym_id).is_empty()` uses Oxc's full ECMAScript
            // lexical scope resolver, so shadowed single-letter local variables (`e`, `t`, `s`, etc.)
            // inside live functions never false-keep dead top-level declarators.
            for _ in 0..8 {
                let scoping = oxc::semantic::SemanticBuilder::new()
                    .build(&program)
                    .semantic
                    .into_scoping();
                let root_scope = scoping.root_scope_id();
                let mut referenced_root_syms: FxHashSet<String> = FxHashSet::default();
                for sym_id in scoping.symbol_ids() {
                    if scoping.symbol_scope_id(sym_id) == root_scope
                        && !scoping.get_resolved_reference_ids(sym_id).is_empty()
                    {
                        referenced_root_syms.insert(scoping.symbol_name(sym_id).to_string());
                    }
                }

                let mut changed = false;
                for stmt in program.body.iter_mut() {
                    if let Statement::VariableDeclaration(vd) = stmt {
                        for decl in vd.declarations.iter_mut() {
                            let is_req = decl
                                .init
                                .as_ref()
                                .is_some_and(|e| is_any_require_call(e).is_some());
                            if is_req {
                                if let BindingPattern::ObjectPattern(obj_pat) = &mut decl.id {
                                    if obj_pat.rest.is_none() {
                                        let before_len = obj_pat.properties.len();
                                        obj_pat.properties.retain(|prop| {
                                            pattern_has_live(&prop.value, &referenced_root_syms)
                                        });
                                        if obj_pat.properties.len() != before_len {
                                            changed = true;
                                        }
                                    }
                                }
                            }
                        }
                        let before_decls = vd.declarations.len();
                        vd.declarations
                            .retain(|decl| pattern_has_live(&decl.id, &referenced_root_syms));
                        if vd.declarations.len() != before_decls {
                            changed = true;
                        }
                    }
                }

                let before_stmts = program.body.len();
                program.body.retain(|stmt| match stmt {
                    Statement::FunctionDeclaration(fd) => fd
                        .id
                        .as_ref()
                        .is_none_or(|id| referenced_root_syms.contains(id.name.as_str())),
                    Statement::ClassDeclaration(cd) => cd
                        .id
                        .as_ref()
                        .is_none_or(|id| referenced_root_syms.contains(id.name.as_str())),
                    Statement::VariableDeclaration(vd) => !vd.declarations.is_empty(),
                    _ => true,
                });
                if program.body.len() != before_stmts {
                    changed = true;
                }
                if !changed {
                    break;
                }
            }

            // Step 3: Transitive reachability closure to prune self-recursive dead functions
            // (e.g. `_matchRoutes`, `_routerNodeToTable`, `_diff`, `_toHashedObject`).
            let mut decl_dep_map: FxHashMap<String, Vec<String>> = FxHashMap::default();
            for stmt in &program.body {
                match stmt {
                    Statement::FunctionDeclaration(fd) => {
                        if let Some(id) = &fd.id {
                            let name = id.name.to_string();
                            let refs = extract_referenced_identifiers(
                                &cjs_code[fd.span.start as usize..fd.span.end as usize],
                            );
                            decl_dep_map.entry(name).or_default().extend(refs);
                        }
                    }
                    Statement::ClassDeclaration(cd) => {
                        if let Some(id) = &cd.id {
                            let name = id.name.to_string();
                            let refs = extract_referenced_identifiers(
                                &cjs_code[cd.span.start as usize..cd.span.end as usize],
                            );
                            decl_dep_map.entry(name).or_default().extend(refs);
                        }
                    }
                    Statement::VariableDeclaration(vd) => {
                        for decl in &vd.declarations {
                            let mut bound = Vec::new();
                            pattern_bound_names(&decl.id, &mut bound);
                            let is_destructured_req = matches!(&decl.id, BindingPattern::ObjectPattern(obj) if obj.rest.is_none())
                                && decl
                                    .init
                                    .as_ref()
                                    .is_some_and(|e| is_any_require_call(e).is_some());
                            let refs = if is_destructured_req {
                                FxHashSet::default()
                            } else {
                                extract_referenced_identifiers(
                                    &cjs_code[decl.span.start as usize..decl.span.end as usize],
                                )
                            };
                            for b_name in bound {
                                decl_dep_map
                                    .entry(b_name)
                                    .or_default()
                                    .extend(refs.iter().cloned());
                            }
                        }
                    }
                    _ => {}
                }
            }

            let mut worklist: Vec<String> = Vec::new();
            for stmt in &program.body {
                match stmt {
                    Statement::FunctionDeclaration(_)
                    | Statement::ClassDeclaration(_)
                    | Statement::VariableDeclaration(_) => {}
                    _ => {
                        let sp = stmt.span();
                        let text = cjs_code[sp.start as usize..sp.end as usize].trim();
                        worklist.extend(extract_referenced_identifiers(text));
                    }
                }
            }

            let mut live_idents: FxHashSet<String> = FxHashSet::default();
            while let Some(ident) = worklist.pop() {
                if live_idents.insert(ident.clone()) {
                    if let Some(deps) = decl_dep_map.get(&ident) {
                        for d in deps {
                            if !live_idents.contains(d) {
                                worklist.push(d.clone());
                            }
                        }
                    }
                }
            }

            for stmt in program.body.iter_mut() {
                if let Statement::VariableDeclaration(vd) = stmt {
                    for decl in vd.declarations.iter_mut() {
                        let is_req = decl
                            .init
                            .as_ref()
                            .is_some_and(|e| is_any_require_call(e).is_some());
                        if is_req {
                            if let BindingPattern::ObjectPattern(obj_pat) = &mut decl.id {
                                if obj_pat.rest.is_none() {
                                    obj_pat
                                        .properties
                                        .retain(|prop| pattern_has_live(&prop.value, &live_idents));
                                }
                            }
                        }
                    }
                    vd.declarations
                        .retain(|decl| pattern_has_live(&decl.id, &live_idents));
                }
            }

            program.body.retain(|stmt| match stmt {
                Statement::FunctionDeclaration(fd) => fd
                    .id
                    .as_ref()
                    .is_none_or(|id| live_idents.contains(id.name.as_str())),
                Statement::ClassDeclaration(cd) => cd
                    .id
                    .as_ref()
                    .is_none_or(|id| live_idents.contains(id.name.as_str())),
                Statement::VariableDeclaration(vd) => !vd.declarations.is_empty(),
                _ => {
                    if let Some((exp_name, _)) = match_exports_assignment_stmt(stmt, cjs_code) {
                        exp_name == "__esModule" || used_set.contains(&exp_name)
                    } else {
                        let sp = stmt.span();
                        let text = cjs_code[sp.start as usize..sp.end as usize].trim();
                        !text.starts_with("module.exports = __slice_default;")
                            || used_set.contains("default")
                    }
                }
            });
        }
    }

    oxc::codegen::Codegen::new()
        .with_options(oxc::codegen::CodegenOptions {
            minify,
            comments: oxc::codegen::CommentOptions::disabled(),
            ..Default::default()
        })
        .build(&program)
        .code
}

fn replace_whole_ident_in_slice(src: &str, ident: &str, replacement: &str) -> String {
    let bytes = src.as_bytes();
    let ibytes = ident.as_bytes();
    let ilen = ibytes.len();
    let mut out = String::with_capacity(src.len() + 64);
    let mut last = 0usize;
    let mut pos = 0usize;
    while let Some(rel) = src[pos..].find(ident) {
        let idx = pos + rel;
        let end = idx + ilen;
        let valid_start =
            idx == 0 || (!is_js_ident_continue(bytes[idx - 1]) && bytes[idx - 1] != b'.');
        let valid_end = end == bytes.len() || !is_js_ident_continue(bytes[end]);
        if valid_start && valid_end {
            out.push_str(&src[last..idx]);
            out.push_str(replacement);
            last = end;
        }
        pos = end;
    }
    out.push_str(&src[last..]);
    out
}

/// General-Purpose Cold Method-Scoped `require` Inliner (`inline_cold_method_requires_ast`):
/// If a `/node_modules/` module has a top-level `const { a, b } = require("<dep_id>");` where
/// `<dep_id>` is only required once in the bundle and its bindings (`a`, `b`) are never referenced
/// at module top-level, in `constructor`, or in hot-path router/request methods, inlines
/// `require("<dep_id>").a` directly into the cold method body and removes the eager top-level `require`.
pub(crate) fn inline_cold_method_requires_ast(
    cjs_code: &str,
    single_consumer_deps: &FxHashSet<String>,
) -> String {
    use oxc::ast::ast::{
        BindingPattern, ClassElement, Expression, MethodDefinitionKind, Statement,
    };
    use oxc::span::GetSpan;

    if single_consumer_deps.is_empty() {
        return cjs_code.to_string();
    }

    let allocator = oxc::allocator::Allocator::default();
    let source_type = oxc::span::SourceType::mjs().with_module(false);
    let parser_ret = oxc::parser::Parser::new(&allocator, cjs_code, source_type).parse();
    if !parser_ret.diagnostics.is_empty() {
        return cjs_code.to_string();
    }

    // Collect cold class method body spans and non-computed class element key spans
    let mut cold_method_spans: Vec<(usize, usize)> = Vec::new();
    let mut non_computed_key_spans: Vec<(usize, usize)> = Vec::new();
    let is_hot_method_name = |name: &str| {
        matches!(
            name,
            "constructor"
                | "fetch"
                | "match"
                | "add"
                | "insert"
                | "lookup"
                | "search"
                | "get"
                | "set"
                | "header"
                | "json"
                | "text"
                | "body"
                | "newResponse"
                | "param"
                | "query"
                | "queries"
                | "req"
                | "res"
                | "url"
                | "method"
                | "status"
        )
    };

    let mut collect_from_class = |cls: &oxc::ast::ast::Class<'_>| {
        for el in &cls.body.body {
            match el {
                ClassElement::MethodDefinition(md) => {
                    if !md.computed {
                        let ksp = md.key.span();
                        non_computed_key_spans.push((ksp.start as usize, ksp.end as usize));
                    }
                    if md.kind == MethodDefinitionKind::Constructor {
                        continue;
                    }
                    let mname = md.key.name().unwrap_or_default();
                    if is_hot_method_name(&mname) {
                        continue;
                    }
                    if let Some(body) = &md.value.body {
                        cold_method_spans.push((body.span.start as usize, body.span.end as usize));
                    }
                }
                ClassElement::PropertyDefinition(pd) => {
                    if !pd.computed {
                        let ksp = pd.key.span();
                        non_computed_key_spans.push((ksp.start as usize, ksp.end as usize));
                    }
                    let pname = pd.key.name().unwrap_or_default();
                    if is_hot_method_name(&pname) {
                        continue;
                    }
                    if let Some(Expression::ArrowFunctionExpression(arrow)) = &pd.value {
                        let sp = arrow.body.span();
                        cold_method_spans.push((sp.start as usize, sp.end as usize));
                    }
                }
                _ => {}
            }
        }
    };

    for stmt in &parser_ret.program.body {
        match stmt {
            Statement::ClassDeclaration(cd) => collect_from_class(cd),
            Statement::VariableDeclaration(vd) => {
                for decl in &vd.declarations {
                    if let Some(Expression::ClassExpression(ce)) = &decl.init {
                        collect_from_class(ce);
                    }
                }
            }
            _ => {}
        }
    }

    if cold_method_spans.is_empty() {
        return cjs_code.to_string();
    }

    let mut stmt_spans_to_remove: Vec<(usize, usize)> = Vec::new();
    let mut inline_bindings: Vec<(String, String)> = Vec::new();

    for stmt in &parser_ret.program.body {
        let Statement::VariableDeclaration(vd) = stmt else {
            continue;
        };
        if vd.declarations.len() != 1 {
            continue;
        }
        let decl = &vd.declarations[0];
        let Some(init) = &decl.init else {
            continue;
        };
        let Some(dep_id) = is_any_require_call(init) else {
            continue;
        };
        if !single_consumer_deps.contains(&dep_id) {
            continue;
        }
        let BindingPattern::ObjectPattern(obj_pat) = &decl.id else {
            continue;
        };
        if obj_pat.rest.is_some() || obj_pat.properties.is_empty() {
            continue;
        }
        let mut bindings = Vec::new();
        let mut all_simple = true;
        for prop in &obj_pat.properties {
            let Some(k) = prop.key.name() else {
                all_simple = false;
                break;
            };
            let BindingPattern::BindingIdentifier(loc) = &prop.value else {
                all_simple = false;
                break;
            };
            if k != loc.name.as_str() {
                all_simple = false;
                break;
            }
            bindings.push(loc.name.to_string());
        }
        if !all_simple {
            continue;
        }

        // Verify that every reference to each binding after `vd.span.end` is inside `cold_method_spans`
        let decl_end = vd.span.end as usize;
        let bytes = cjs_code.as_bytes();
        let mut eligible = true;
        for b_name in &bindings {
            let b_len = b_name.len();
            let mut pos = decl_end;
            let mut ref_count = 0usize;
            while let Some(rel) = cjs_code[pos..].find(b_name.as_str()) {
                let idx = pos + rel;
                let end = idx + b_len;
                pos = end;
                let valid_start =
                    idx == 0 || (!is_js_ident_continue(bytes[idx - 1]) && bytes[idx - 1] != b'.');
                let valid_end = end == bytes.len() || !is_js_ident_continue(bytes[end]);
                if !valid_start || !valid_end {
                    continue;
                }
                if non_computed_key_spans
                    .iter()
                    .any(|&(ks, ke)| idx >= ks && end <= ke)
                {
                    continue;
                }
                let in_cold_span = cold_method_spans.iter().any(|&(s, e)| idx >= s && end <= e);
                if !in_cold_span {
                    eligible = false;
                    break;
                }
                ref_count += 1;
            }
            if !eligible || ref_count == 0 || ref_count > 2 {
                eligible = false;
                break;
            }
        }

        if eligible {
            stmt_spans_to_remove.push((vd.span.start as usize, vd.span.end as usize));
            for b_name in bindings {
                inline_bindings.push((b_name.clone(), format!("require({:?}).{}", dep_id, b_name)));
            }
        }
    }

    if stmt_spans_to_remove.is_empty() {
        return cjs_code.to_string();
    }

    // Apply replacements inside cold_method_spans and remove the top-level require statements
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for (s, e) in &stmt_spans_to_remove {
        edits.push((*s, *e, String::new()));
    }
    for &(ms, me) in &cold_method_spans {
        let orig_slice = &cjs_code[ms..me];
        let mut updated = orig_slice.to_string();
        for (b_name, rep_expr) in &inline_bindings {
            if updated.contains(b_name.as_str()) {
                updated = replace_whole_ident_in_slice(&updated, b_name, rep_expr);
            }
        }
        if updated != orig_slice {
            edits.push((ms, me, updated));
        }
    }
    edits.sort_by_key(|e| e.0);

    let mut result = String::with_capacity(cjs_code.len() + 64);
    let mut cursor = 0usize;
    for (s, e, rep) in edits {
        if s < cursor {
            continue;
        }
        result.push_str(&cjs_code[cursor..s]);
        result.push_str(&rep);
        cursor = e;
    }
    result.push_str(&cjs_code[cursor..]);
    result
}

/// Computes the set of module indices `0..modules.len()` that are reachable from `entry_body`
/// via **eager** `require(idx)` edges (excluding `require(idx)` calls inside lazy property getters
/// `get prop() { ... }` or cold non-constructor class methods).
pub(crate) fn collect_eager_module_indices(
    entry_body: &str,
    modules: &[(String, String)],
) -> FxHashSet<usize> {
    use oxc::ast::ast::{Expression, ObjectPropertyKind, PropertyKind, Statement};
    use oxc::span::GetSpan;

    fn extract_eager_require_ids(code: &str, max_id: usize) -> Vec<usize> {
        let allocator = oxc::allocator::Allocator::default();
        let source_type = oxc::span::SourceType::mjs().with_module(false);
        let parser_ret = oxc::parser::Parser::new(&allocator, code, source_type).parse();

        let mut deferred_spans: Vec<(usize, usize)> = Vec::new();
        if parser_ret.diagnostics.is_empty() {
            fn collect_class_deferred(
                cls: &oxc::ast::ast::Class<'_>,
                deferred_spans: &mut Vec<(usize, usize)>,
            ) {
                use oxc::ast::ast::{ClassElement, Expression, MethodDefinitionKind};
                use oxc::span::GetSpan;
                for el in &cls.body.body {
                    match el {
                        ClassElement::MethodDefinition(md) => {
                            if md.kind != MethodDefinitionKind::Constructor {
                                if let Some(b) = &md.value.body {
                                    deferred_spans
                                        .push((b.span.start as usize, b.span.end as usize));
                                }
                            }
                        }
                        ClassElement::PropertyDefinition(pd) => {
                            if let Some(Expression::ArrowFunctionExpression(arrow)) = &pd.value {
                                let sp = arrow.body.span();
                                deferred_spans.push((sp.start as usize, sp.end as usize));
                            }
                        }
                        _ => {}
                    }
                }
            }

            for stmt in &parser_ret.program.body {
                match stmt {
                    Statement::ClassDeclaration(cd) => {
                        collect_class_deferred(cd, &mut deferred_spans)
                    }
                    Statement::FunctionDeclaration(fd) => {
                        if let (Some(id), Some(body)) = (&fd.id, &fd.body) {
                            let call_pat = format!("{}(", id.name);
                            let start = fd.span.start as usize;
                            let end = fd.span.end as usize;
                            if !code[..start].contains(&call_pat)
                                && !code[end..].contains(&call_pat)
                            {
                                deferred_spans.push((body.span.start as usize, body.span.end as usize));
                            }
                        }
                    }
                    Statement::VariableDeclaration(vd) => {
                        for decl in &vd.declarations {
                            if let Some(Expression::ClassExpression(ce)) = &decl.init {
                                collect_class_deferred(ce, &mut deferred_spans);
                            } else if matches!(
                                &decl.init,
                                Some(
                                    Expression::FunctionExpression(_)
                                        | Expression::ArrowFunctionExpression(_)
                                )
                            ) {
                                if let oxc::ast::ast::BindingPattern::BindingIdentifier(id) =
                                    &decl.id
                                {
                                    let call_pat = format!("{}(", id.name);
                                    let start = decl.span.start as usize;
                                    let end = decl.span.end as usize;
                                    if !code[..start].contains(&call_pat)
                                        && !code[end..].contains(&call_pat)
                                    {
                                        deferred_spans.push((start, end));
                                    }
                                }
                            }
                        }
                    }
                    Statement::ExpressionStatement(es) => {
                        if let Expression::AssignmentExpression(assign) =
                            es.expression.without_parentheses()
                        {
                            if let Expression::ObjectExpression(obj) =
                                assign.right.without_parentheses()
                            {
                                for prop in &obj.properties {
                                    if let ObjectPropertyKind::ObjectProperty(p) = prop {
                                        if p.kind == PropertyKind::Get {
                                            let sp = p.value.span();
                                            deferred_spans
                                                .push((sp.start as usize, sp.end as usize));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        let mut out = Vec::new();
        let mut pos = 0usize;
        while let Some(rel) = code[pos..].find("require(") {
            let idx = pos + rel;
            let after = &code[idx + 8..];
            pos = idx + 8;
            let in_deferred = deferred_spans.iter().any(|&(s, e)| idx >= s && idx < e);
            if in_deferred {
                continue;
            }
            let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() && after[digits.len()..].starts_with(')') {
                if let Ok(id) = digits.parse::<usize>() {
                    if id < max_id {
                        out.push(id);
                    }
                }
            }
        }
        out
    }

    let max_id = modules.len();
    let mut eager_set = FxHashSet::default();
    let mut queue = extract_eager_require_ids(entry_body, max_id);

    while let Some(idx) = queue.pop() {
        if eager_set.insert(idx) {
            for child in extract_eager_require_ids(&modules[idx].1, max_id) {
                if !eager_set.contains(&child) {
                    queue.push(child);
                }
            }
        }
    }

    eager_set
}
