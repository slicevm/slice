//! Oxc TypeScript & JSX/TSX Ahead-of-Time Compiler, Type-Directed Semantic Analyzer,
//! and General-Purpose AST Array Pipeline Fusion Pass.

use crate::esm_cjs_linker::transform_module_to_cjs;
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::Path;

/// Static type-directed optimization hints extracted by Oxc (`oxc::parser` + `oxc::semantic`)
/// before TypeScript type erasure.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeDirectedHints {
    /// Variables annotated with `: number` (eligible for unboxed Tier-2 register loops).
    pub numeric_vars: FxHashSet<String>,
    /// Variables annotated with `: string`.
    pub string_vars: FxHashSet<String>,
    /// Variables annotated with `: boolean`.
    pub boolean_vars: FxHashSet<String>,
    /// Map of `InterfaceName` -> ordered `(field_name, primitive_type_tag)` pairs (`"number"`, `"string"`, `"boolean"`).
    pub interface_schemas: FxHashMap<String, Vec<(String, String)>>,
    /// Map of `variable_name` -> `InterfaceName` from `: InterfaceName` annotations.
    pub interface_vars: FxHashMap<String, String>,
    /// Total semantic symbol bindings analyzed by `oxc::semantic::SemanticBuilder`.
    pub semantic_symbol_count: usize,
}

fn classify_ts_type_ast(ty: &oxc::ast::ast::TSType<'_>) -> Option<&'static str> {
    use oxc::ast::ast::{TSLiteral, TSType};
    match ty {
        TSType::TSNumberKeyword(_) => Some("number"),
        TSType::TSBooleanKeyword(_) => Some("boolean"),
        TSType::TSStringKeyword(_) => Some("string"),
        TSType::TSLiteralType(lit) => match &lit.literal {
            TSLiteral::StringLiteral(_) | TSLiteral::TemplateLiteral(_) => Some("string"),
            TSLiteral::NumericLiteral(_) | TSLiteral::BigIntLiteral(_) => Some("number"),
            TSLiteral::BooleanLiteral(_) => Some("boolean"),
            _ => None,
        },
        TSType::TSUnionType(union) => {
            let mut common: Option<&'static str> = None;
            for member in &union.types {
                let kind = classify_ts_type_ast(member)?;
                if let Some(prev) = common {
                    if prev != kind {
                        return None;
                    }
                } else {
                    common = Some(kind);
                }
            }
            common
        }
        TSType::TSParenthesizedType(p) => classify_ts_type_ast(&p.type_annotation),
        _ => None,
    }
}

fn extract_ts_type_ref_name<'a>(ty: &'a oxc::ast::ast::TSType<'a>) -> Option<&'a str> {
    use oxc::ast::ast::{TSType, TSTypeName};
    if let TSType::TSTypeReference(tref) = ty {
        if let TSTypeName::IdentifierReference(id) = &tref.type_name {
            return Some(id.name.as_str());
        }
    }
    None
}

fn record_annotated_ident(
    ident: &str,
    ty: &oxc::ast::ast::TSType<'_>,
    hints: &mut TypeDirectedHints,
) {
    if ident.is_empty() {
        return;
    }
    if let Some(prim) = classify_ts_type_ast(ty) {
        match prim {
            "number" => {
                hints.numeric_vars.insert(ident.to_string());
            }
            "string" => {
                hints.string_vars.insert(ident.to_string());
            }
            "boolean" => {
                hints.boolean_vars.insert(ident.to_string());
            }
            _ => {}
        }
    } else if let Some(ref_name) = extract_ts_type_ref_name(ty) {
        if hints.interface_schemas.contains_key(ref_name) {
            hints
                .interface_vars
                .insert(ident.to_string(), ref_name.to_string());
        }
    }
}

/// Detects whether a source string likely contains JSX/TSX syntax (`</Tag>`, `<Tag />`, `<>`, or `@jsx`).
#[inline]
pub(crate) fn source_contains_jsx(source: &str) -> bool {
    source.contains("</")
        || source.contains("/>")
        || source.contains("<>")
        || source.contains("@jsx")
}

/// Analyzes TypeScript source using `oxc::parser::Parser` and `oxc::semantic::SemanticBuilder`
/// to extract static primitive variable types (`: number`, `: string`, `: boolean`) and flat
/// `interface` schemas via 100% Oxc AST traversal before TypeScript erasure.
pub fn analyze_typescript_semantics(source: &str) -> TypeDirectedHints {
    use oxc::ast::ast::{
        BindingPattern, ClassElement, Declaration, Expression, FormalParameters, Statement,
        TSInterfaceDeclaration, TSSignature,
    };

    let mut hints = TypeDirectedHints::default();
    let allocator = oxc::allocator::Allocator::default();
    let parse_opts = oxc::parser::ParseOptions {
        allow_return_outside_function: true,
        ..Default::default()
    };
    let mut parser_ret = oxc::parser::Parser::new(
        &allocator,
        source,
        oxc::span::SourceType::ts().with_module(true),
    )
    .with_options(parse_opts)
    .parse();
    if !parser_ret.diagnostics.is_empty() && source_contains_jsx(source) {
        let retry_ret = oxc::parser::Parser::new(
            &allocator,
            source,
            oxc::span::SourceType::tsx().with_module(true),
        )
        .with_options(parse_opts)
        .parse();
        if retry_ret.diagnostics.is_empty() {
            parser_ret = retry_ret;
        }
    }
    let semantic_ret = oxc::semantic::SemanticBuilder::new().build(&parser_ret.program);
    hints.semantic_symbol_count = semantic_ret.semantic.scoping().symbols_len();

    fn extract_interface_decl(iface: &TSInterfaceDeclaration<'_>, hints: &mut TypeDirectedHints) {
        let iface_name = iface.id.name.to_string();
        let mut fields = Vec::new();
        for sig in &iface.body.body {
            if let TSSignature::TSPropertySignature(prop) = sig {
                if let Some(fname) = prop.key.name() {
                    if let Some(ta) = &prop.type_annotation {
                        if let Some(kind) = classify_ts_type_ast(&ta.type_annotation) {
                            fields.push((fname.to_string(), kind.to_string()));
                        }
                    }
                }
            }
        }
        if !fields.is_empty() {
            hints.interface_schemas.insert(iface_name, fields);
        }
    }

    // Pass 1: Extract all `interface Name { ... }` declarations from the Oxc AST
    for stmt in &parser_ret.program.body {
        match stmt {
            Statement::TSInterfaceDeclaration(iface) => {
                extract_interface_decl(iface, &mut hints);
            }
            Statement::ExportDeclaration(exp) => {
                if let Declaration::TSInterfaceDeclaration(iface) = &exp.declaration {
                    extract_interface_decl(iface, &mut hints);
                }
            }
            _ => {}
        }
    }

    fn walk_params(params: &FormalParameters<'_>, hints: &mut TypeDirectedHints) {
        for param in &params.items {
            if let Some(ta) = &param.type_annotation {
                if let BindingPattern::BindingIdentifier(id) = &param.pattern {
                    record_annotated_ident(id.name.as_str(), &ta.type_annotation, hints);
                }
            }
        }
    }

    fn walk_expr_types(expr: &Expression<'_>, hints: &mut TypeDirectedHints) {
        match expr {
            Expression::ArrowFunctionExpression(arrow) => {
                walk_params(&arrow.params, hints);
                if let oxc::ast::ast::ArrowFunctionBody::FunctionBody(fb) = &arrow.body {
                    for s in &fb.statements {
                        walk_stmt_types(s, hints);
                    }
                } else if let Some(e) = arrow.body.as_expression() {
                    walk_expr_types(e, hints);
                }
            }
            Expression::FunctionExpression(func) => {
                walk_params(&func.params, hints);
                if let Some(body) = &func.body {
                    for s in &body.statements {
                        walk_stmt_types(s, hints);
                    }
                }
            }
            Expression::CallExpression(call) => {
                walk_expr_types(&call.callee, hints);
                for arg in &call.arguments {
                    if let Some(e) = arg.as_expression() {
                        walk_expr_types(e, hints);
                    }
                }
            }
            Expression::ParenthesizedExpression(p) => walk_expr_types(&p.expression, hints),
            Expression::AssignmentExpression(a) => walk_expr_types(&a.right, hints),
            _ => {}
        }
    }

    fn walk_stmt_types(stmt: &Statement<'_>, hints: &mut TypeDirectedHints) {
        match stmt {
            Statement::VariableDeclaration(vd) => {
                for decl in &vd.declarations {
                    if let Some(ta) = &decl.type_annotation {
                        if let BindingPattern::BindingIdentifier(id) = &decl.id {
                            record_annotated_ident(id.name.as_str(), &ta.type_annotation, hints);
                        }
                    }
                    if let Some(init) = &decl.init {
                        walk_expr_types(init, hints);
                    }
                }
            }
            Statement::FunctionDeclaration(fd) => {
                walk_params(&fd.params, hints);
                if let Some(body) = &fd.body {
                    for s in &body.statements {
                        walk_stmt_types(s, hints);
                    }
                }
            }
            Statement::ClassDeclaration(cd) => {
                for el in &cd.body.body {
                    if let ClassElement::MethodDefinition(md) = el {
                        walk_params(&md.value.params, hints);
                        if let Some(body) = &md.value.body {
                            for s in &body.statements {
                                walk_stmt_types(s, hints);
                            }
                        }
                    }
                }
            }
            Statement::ExportDeclaration(exp) => match &exp.declaration {
                Declaration::VariableDeclaration(vd) => {
                    for d in &vd.declarations {
                        if let Some(ta) = &d.type_annotation {
                            if let BindingPattern::BindingIdentifier(id) = &d.id {
                                record_annotated_ident(
                                    id.name.as_str(),
                                    &ta.type_annotation,
                                    hints,
                                );
                            }
                        }
                        if let Some(init) = &d.init {
                            walk_expr_types(init, hints);
                        }
                    }
                }
                Declaration::FunctionDeclaration(fd) => {
                    walk_params(&fd.params, hints);
                    if let Some(body) = &fd.body {
                        for s in &body.statements {
                            walk_stmt_types(s, hints);
                        }
                    }
                }
                _ => {}
            },
            Statement::BlockStatement(bs) => {
                for s in &bs.body {
                    walk_stmt_types(s, hints);
                }
            }
            Statement::IfStatement(ifs) => {
                walk_stmt_types(&ifs.consequent, hints);
                if let Some(alt) = &ifs.alternate {
                    walk_stmt_types(alt, hints);
                }
            }
            Statement::ForStatement(fs) => {
                if let Some(oxc::ast::ast::ForStatementInit::VariableDeclaration(vd)) = &fs.init {
                    for d in &vd.declarations {
                        if let Some(ta) = &d.type_annotation {
                            if let BindingPattern::BindingIdentifier(id) = &d.id {
                                record_annotated_ident(
                                    id.name.as_str(),
                                    &ta.type_annotation,
                                    hints,
                                );
                            }
                        }
                    }
                }
                walk_stmt_types(&fs.body, hints);
            }
            Statement::TryStatement(ts) => {
                for s in &ts.block.body {
                    walk_stmt_types(s, hints);
                }
                if let Some(handler) = &ts.handler {
                    for s in &handler.body.body {
                        walk_stmt_types(s, hints);
                    }
                }
            }
            Statement::ExpressionStatement(es) => {
                walk_expr_types(&es.expression, hints);
            }
            Statement::ReturnStatement(rs) => {
                if let Some(arg) = &rs.argument {
                    walk_expr_types(arg, hints);
                }
            }
            _ => {}
        }
    }

    // Pass 2: Walk variable and parameter type annotations via Oxc AST
    for stmt in &parser_ret.program.body {
        walk_stmt_types(stmt, &mut hints);
    }

    hints
}

/// General-Purpose Oxc AST Array Pipeline Fusion Pass:
/// Inspects the Oxc AST of `code` and fuses chained higher-order array expressions on **any**
/// receiver `<arr_expr>` into a single allocation-free `for` loop IIFE:
/// - `<arr_expr>.map((<x>) => <map_body>).reduce((<acc>, <curr>) => <red_body>, <init>)`
/// - `<arr_expr>.filter((<x>) => <pred>).map((<y>) => <map_body>).reduce((<acc>, <curr>) => <red_body>, <init>)`
/// - `<arr_expr>.some((<x>) => <pred>)`
/// - `<arr_expr>.every((<x>) => <pred>)`
pub fn fuse_array_pipelines_ast(code: &str) -> String {
    if !code.contains(".reduce(") && !code.contains(".some(") && !code.contains(".every(") {
        return code.to_string();
    }

    let allocator = oxc::allocator::Allocator::default();
    let source_type = oxc::span::SourceType::mjs();
    let parser_ret = oxc::parser::Parser::new(&allocator, code, source_type).parse();
    if !parser_ret.diagnostics.is_empty() {
        return code.to_string();
    }

    use oxc::ast::ast::{Argument, BindingPattern, Expression, Statement};
    use oxc::span::GetSpan;

    struct Replacement {
        start: usize,
        end: usize,
        text: String,
    }

    fn extract_single_param_arrow<'a>(
        arg: &'a Argument<'a>,
        src: &'a str,
    ) -> Option<(&'a str, &'a str)> {
        let expr = arg.as_expression()?;
        let Expression::ArrowFunctionExpression(arrow) = expr.without_parentheses() else {
            return None;
        };
        if arrow.r#async || arrow.params.items.len() != 1 || arrow.params.rest.is_some() {
            return None;
        }
        let BindingPattern::BindingIdentifier(ident) = &arrow.params.items[0].pattern else {
            return None;
        };
        let param_name = ident.name.as_str();
        let sp = arrow.body.span();
        let body_src = src.get(sp.start as usize..sp.end as usize)?.trim();
        if !body_src.starts_with('{') && !body_src.is_empty() {
            return Some((param_name, body_src));
        }
        None
    }

    fn extract_two_param_arrow<'a>(
        arg: &'a Argument<'a>,
        src: &'a str,
    ) -> Option<(&'a str, &'a str, &'a str)> {
        let expr = arg.as_expression()?;
        let Expression::ArrowFunctionExpression(arrow) = expr.without_parentheses() else {
            return None;
        };
        if arrow.r#async || arrow.params.items.len() != 2 || arrow.params.rest.is_some() {
            return None;
        }
        let BindingPattern::BindingIdentifier(id0) = &arrow.params.items[0].pattern else {
            return None;
        };
        let BindingPattern::BindingIdentifier(id1) = &arrow.params.items[1].pattern else {
            return None;
        };
        let sp = arrow.body.span();
        let body_src = src.get(sp.start as usize..sp.end as usize)?.trim();
        if !body_src.starts_with('{') && !body_src.is_empty() {
            return Some((id0.name.as_str(), id1.name.as_str(), body_src));
        }
        None
    }

    fn try_fuse_call<'a>(
        call: &'a oxc::ast::ast::CallExpression<'a>,
        src: &'a str,
        counter: &mut usize,
    ) -> Option<Replacement> {
        let Expression::StaticMemberExpression(outer_mem) = call.callee.without_parentheses()
        else {
            return None;
        };
        let outer_method = outer_mem.property.name.as_str();

        // Case 1: `<arr>.some((x) => <pred>)` or `<arr>.every((x) => <pred>)`
        if (outer_method == "some" || outer_method == "every") && call.arguments.len() == 1 {
            let (param, pred_src) = extract_single_param_arrow(&call.arguments[0], src)?;
            let arr_sp = outer_mem.object.span();
            let arr_src = &src[arr_sp.start as usize..arr_sp.end as usize];
            *counter += 1;
            let id = *counter;
            let fused = if outer_method == "some" {
                format!(
                    "(() => {{ const __sl_a{id} = {arr}; for (let __sl_i{id} = 0; __sl_i{id} < __sl_a{id}.length; __sl_i{id}++) {{ const {param} = __sl_a{id}[__sl_i{id}]; if ({pred}) return true; }} return false; }})()",
                    id = id,
                    arr = arr_src,
                    param = param,
                    pred = pred_src
                )
            } else {
                format!(
                    "(() => {{ const __sl_a{id} = {arr}; for (let __sl_i{id} = 0; __sl_i{id} < __sl_a{id}.length; __sl_i{id}++) {{ const {param} = __sl_a{id}[__sl_i{id}]; if (!({pred})) return false; }} return true; }})()",
                    id = id,
                    arr = arr_src,
                    param = param,
                    pred = pred_src
                )
            };
            return Some(Replacement {
                start: call.span.start as usize,
                end: call.span.end as usize,
                text: fused,
            });
        }

        // Case 2: `<inner>.reduce((acc, curr) => <red_body>, <init>)`
        if outer_method == "reduce" && call.arguments.len() == 2 {
            let (acc_param, curr_param, red_body) =
                extract_two_param_arrow(&call.arguments[0], src)?;
            let init_expr = call.arguments[1].as_expression()?;
            let init_sp = init_expr.span();
            let init_src = &src[init_sp.start as usize..init_sp.end as usize];

            // Check if `<inner>` is `<arr>.map((x) => <map_body>)`
            if let Expression::CallExpression(inner_call) = outer_mem.object.without_parentheses() {
                if let Expression::StaticMemberExpression(inner_mem) =
                    inner_call.callee.without_parentheses()
                {
                    if inner_mem.property.name.as_str() == "map" && inner_call.arguments.len() == 1
                    {
                        let (map_param, map_body) =
                            extract_single_param_arrow(&inner_call.arguments[0], src)?;

                        // Check if `<arr>` is `<base>.filter((f) => <filt_body>)`
                        if let Expression::CallExpression(filt_call) =
                            inner_mem.object.without_parentheses()
                        {
                            if let Expression::StaticMemberExpression(filt_mem) =
                                filt_call.callee.without_parentheses()
                            {
                                if filt_mem.property.name.as_str() == "filter"
                                    && filt_call.arguments.len() == 1
                                {
                                    if let Some((filt_param, filt_body)) =
                                        extract_single_param_arrow(&filt_call.arguments[0], src)
                                    {
                                        let base_sp = filt_mem.object.span();
                                        let base_src =
                                            &src[base_sp.start as usize..base_sp.end as usize];
                                        *counter += 1;
                                        let id = *counter;
                                        let fused = format!(
                                            "(() => {{ const __sl_a{id} = {base}; let {acc} = {init}; for (let __sl_i{id} = 0; __sl_i{id} < __sl_a{id}.length; __sl_i{id}++) {{ const {fp} = __sl_a{id}[__sl_i{id}]; if (!({fb})) continue; const {mp} = {fp}; const {cp} = ({mb}); {acc} = ({rb}); }} return {acc}; }})()",
                                            id = id,
                                            base = base_src,
                                            acc = acc_param,
                                            init = init_src,
                                            fp = filt_param,
                                            fb = filt_body,
                                            mp = map_param,
                                            cp = curr_param,
                                            mb = map_body,
                                            rb = red_body
                                        );
                                        return Some(Replacement {
                                            start: call.span.start as usize,
                                            end: call.span.end as usize,
                                            text: fused,
                                        });
                                    }
                                }
                            }
                        }

                        let arr_sp = inner_mem.object.span();
                        let arr_src = &src[arr_sp.start as usize..arr_sp.end as usize];
                        *counter += 1;
                        let id = *counter;
                        let fused = format!(
                            "(() => {{ const __sl_a{id} = {arr}; let {acc} = {init}; for (let __sl_i{id} = 0; __sl_i{id} < __sl_a{id}.length; __sl_i{id}++) {{ const {mp} = __sl_a{id}[__sl_i{id}]; const {cp} = ({mb}); {acc} = ({rb}); }} return {acc}; }})()",
                            id = id,
                            arr = arr_src,
                            acc = acc_param,
                            init = init_src,
                            mp = map_param,
                            cp = curr_param,
                            mb = map_body,
                            rb = red_body
                        );
                        return Some(Replacement {
                            start: call.span.start as usize,
                            end: call.span.end as usize,
                            text: fused,
                        });
                    }
                }
            }
        }
        None
    }

    fn walk_expr<'a>(
        expr: &'a Expression<'a>,
        src: &'a str,
        counter: &mut usize,
        out: &mut Vec<Replacement>,
    ) {
        match expr {
            Expression::CallExpression(call) => {
                if let Some(rep) = try_fuse_call(call, src, counter) {
                    out.push(rep);
                    return;
                }
                walk_expr(&call.callee, src, counter, out);
                for arg in &call.arguments {
                    if let Some(e) = arg.as_expression() {
                        walk_expr(e, src, counter, out);
                    }
                }
            }
            Expression::BinaryExpression(b) => {
                walk_expr(&b.left, src, counter, out);
                walk_expr(&b.right, src, counter, out);
            }
            Expression::LogicalExpression(l) => {
                walk_expr(&l.left, src, counter, out);
                walk_expr(&l.right, src, counter, out);
            }
            Expression::ConditionalExpression(c) => {
                walk_expr(&c.test, src, counter, out);
                walk_expr(&c.consequent, src, counter, out);
                walk_expr(&c.alternate, src, counter, out);
            }
            Expression::AssignmentExpression(a) => {
                walk_expr(&a.right, src, counter, out);
            }
            Expression::ParenthesizedExpression(p) => {
                walk_expr(&p.expression, src, counter, out);
            }
            Expression::FunctionExpression(fe) => {
                if let Some(body) = &fe.body {
                    for s in &body.statements {
                        walk_stmt(s, src, counter, out);
                    }
                }
            }
            Expression::ObjectExpression(obj) => {
                for prop in &obj.properties {
                    if let oxc::ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop {
                        walk_expr(&p.value, src, counter, out);
                    }
                }
            }
            Expression::ArrayExpression(arr) => {
                for el in &arr.elements {
                    if let Some(e) = el.as_expression() {
                        walk_expr(e, src, counter, out);
                    }
                }
            }
            _ => {}
        }
    }

    fn walk_stmt<'a>(
        stmt: &'a Statement<'a>,
        src: &'a str,
        counter: &mut usize,
        out: &mut Vec<Replacement>,
    ) {
        match stmt {
            Statement::VariableDeclaration(vd) => {
                for decl in &vd.declarations {
                    if let Some(init) = &decl.init {
                        walk_expr(init, src, counter, out);
                    }
                }
            }
            Statement::ExpressionStatement(es) => {
                walk_expr(&es.expression, src, counter, out);
            }
            Statement::ReturnStatement(rs) => {
                if let Some(arg) = &rs.argument {
                    walk_expr(arg, src, counter, out);
                }
            }
            Statement::BlockStatement(bs) => {
                for s in &bs.body {
                    walk_stmt(s, src, counter, out);
                }
            }
            Statement::IfStatement(ifs) => {
                walk_expr(&ifs.test, src, counter, out);
                walk_stmt(&ifs.consequent, src, counter, out);
                if let Some(alt) = &ifs.alternate {
                    walk_stmt(alt, src, counter, out);
                }
            }
            Statement::FunctionDeclaration(fd) => {
                if let Some(body) = &fd.body {
                    for s in &body.statements {
                        walk_stmt(s, src, counter, out);
                    }
                }
            }
            Statement::ClassDeclaration(cd) => {
                for el in &cd.body.body {
                    if let oxc::ast::ast::ClassElement::MethodDefinition(md) = el {
                        if let Some(body) = &md.value.body {
                            for s in &body.statements {
                                walk_stmt(s, src, counter, out);
                            }
                        }
                    }
                }
            }
            Statement::ExportDefaultDeclaration(edd) => {
                if let Some(expr) = edd.declaration.as_expression() {
                    walk_expr(expr, src, counter, out);
                }
            }
            _ => {}
        }
    }

    let mut replacements = Vec::new();
    let mut counter = 0usize;
    for stmt in &parser_ret.program.body {
        walk_stmt(stmt, code, &mut counter, &mut replacements);
    }

    if replacements.is_empty() {
        return code.to_string();
    }

    replacements.sort_by_key(|r| r.start);
    let mut result = String::with_capacity(code.len() + replacements.len() * 64);
    let mut cursor = 0usize;
    for r in replacements {
        if r.start < cursor {
            continue;
        }
        result.push_str(&code[cursor..r.start]);
        result.push_str(&r.text);
        cursor = r.end;
    }
    result.push_str(&code[cursor..]);
    result
}

/// Masks any top-level `return <expr>;` statements (identified via `oxc::parser::Parser` with
/// `allow_return_outside_function: true`) as `__slice_top_ret_stmt = <expr>;` so that Oxc's
/// `SemanticBuilder` and `Transformer` can erase TypeScript annotations without rejecting
/// top-level returns. Returns `Cow::Borrowed` when no top-level `return` statement exists.
fn mask_top_level_returns_for_oxc(source: &str) -> (std::borrow::Cow<'_, str>, bool) {
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

    let mut top_ret_spans: smallvec::SmallVec<[(usize, usize, bool); 4]> =
        smallvec::SmallVec::new();
    for stmt in &parsed.program.body {
        if let oxc::ast::ast::Statement::ReturnStatement(ret) = stmt {
            let start = ret.span.start as usize;
            let has_arg = ret.argument.is_some();
            if start + 6 <= source.len() && &source[start..start + 6] == "return" {
                top_ret_spans.push((start, start + 6, has_arg));
            }
        }
    }
    if top_ret_spans.is_empty() {
        return (std::borrow::Cow::Borrowed(source), false);
    }

    let mut out = String::with_capacity(source.len() + top_ret_spans.len() * 18);
    let mut cursor = 0usize;
    for (start, end, has_arg) in top_ret_spans {
        if start < cursor {
            continue;
        }
        out.push_str(&source[cursor..start]);
        if has_arg {
            out.push_str("__slice_top_ret_stmt =");
        } else {
            out.push_str("__slice_top_ret_stmt");
        }
        cursor = end;
    }
    out.push_str(&source[cursor..]);
    (std::borrow::Cow::Owned(out), true)
}

/// Production TypeScript & JSX/TSX Compiler powered by **Oxc** (The Oxidation Compiler).
///
/// Uses `oxc::allocator::Allocator`, `oxc::parser::Parser`, `oxc::semantic::SemanticBuilder`,
/// `oxc::transformer::Transformer` (with native JSX/TSX transformation enabled), and
/// `oxc::codegen::Codegen` to lower TypeScript types, interfaces, enums, generics, and JSX/TSX
/// elements into clean standard ECMAScript.
pub fn strip_typescript_raw(source: &str, is_ts: bool) -> String {
    strip_typescript_raw_with_ext(source, is_ts, false)
}

/// Extension-aware Oxc TypeScript & JSX/TSX compiler:
/// - When `is_jsx_ext` is `true` (`.tsx` or `.jsx`), parses directly as `SourceType::tsx()`.
/// - When `is_jsx_ext` is `false` (`.ts` or inline TypeScript), parses as `SourceType::ts()` FIRST
///   so TypeScript `<T>` generic arrow functions (like `const id = <T>(x: T): T => x`) are never
///   misinterpreted as JSX tags even if the file also contains HTML strings (`"<div>...</div>"`),
///   and only retries as `SourceType::tsx()` if `.ts` parsing reported syntax diagnostics.
/// - Relies directly on `oxc::codegen::Codegen` (`CommentOptions::disabled()`) without running a
///   redundant post-codegen line-by-line comment stripper.
pub fn strip_typescript_raw_with_ext(source: &str, is_ts: bool, is_jsx_ext: bool) -> String {
    if source.starts_with("var __slice_modules =") {
        return source.to_string();
    }
    let has_jsx_hint =
        is_jsx_ext || (is_ts && memchr::memmem::find(source.as_bytes(), b"@jsx").is_some());
    if !is_ts && !has_jsx_hint && memchr::memmem::find(source.as_bytes(), b"interface ").is_none() {
        return strip_js_comments(source);
    }

    // If the source contains top-level `return ` statements (e.g., inside a Tier-0/Tier-2 snippet
    // or function body fragment), temporarily mask `return` as `__slice_top_ret_stmt =` so Oxc's
    // `SemanticBuilder` and `Transformer` erase all TypeScript annotations without rejecting
    // top-level returns, then restore `return` in the generated JS.
    let (parse_source_cow, had_top_return) = if memchr::memmem::find(source.as_bytes(), b"return ")
        .is_some()
        || memchr::memmem::find(source.as_bytes(), b"return;").is_some()
    {
        mask_top_level_returns_for_oxc(source)
    } else {
        (std::borrow::Cow::Borrowed(source), false)
    };
    let parse_source = parse_source_cow.as_ref();

    let allocator = oxc::allocator::Allocator::default();
    let parse_opts = oxc::parser::ParseOptions {
        allow_return_outside_function: true,
        ..Default::default()
    };
    let primary_source_type = if has_jsx_hint {
        oxc::span::SourceType::tsx().with_module(true)
    } else {
        oxc::span::SourceType::ts().with_module(true)
    };
    let mut parser_ret = oxc::parser::Parser::new(&allocator, parse_source, primary_source_type)
        .with_options(parse_opts)
        .parse();
    let mut active_path = if has_jsx_hint {
        "module.tsx"
    } else {
        "module.ts"
    };

    // If `.ts` parsing had diagnostics, retry with `.tsx` in case JSX syntax was used
    if !parser_ret.diagnostics.is_empty() && !has_jsx_hint {
        let retry_ret = oxc::parser::Parser::new(
            &allocator,
            parse_source,
            oxc::span::SourceType::tsx().with_module(true),
        )
        .with_options(parse_opts)
        .parse();
        if retry_ret.diagnostics.is_empty() {
            parser_ret = retry_ret;
            active_path = "module.tsx";
        }
    }

    if !parser_ret.diagnostics.is_empty() {
        return strip_js_comments(source);
    }

    let mut program = parser_ret.program;
    let scoping = oxc::semantic::SemanticBuilder::new()
        .build(&program)
        .semantic
        .into_scoping();

    let mut transform_options = oxc::transformer::TransformOptions::default();
    transform_options.jsx.jsx_plugin = true;
    let transform_ret =
        oxc::transformer::Transformer::new(&allocator, Path::new(active_path), &transform_options)
            .build_with_scoping(scoping, &mut program);

    if !transform_ret.diagnostics.is_empty() {
        return strip_js_comments(source);
    }

    let mut code = oxc::codegen::Codegen::new()
        .with_options(oxc::codegen::CodegenOptions {
            comments: oxc::codegen::CommentOptions::disabled(),
            ..Default::default()
        })
        .build(&program)
        .code;

    if had_top_return {
        use cow_utils::CowUtils;
        if let std::borrow::Cow::Owned(restored) =
            code.cow_replace("__slice_top_ret_stmt = ", "return ")
        {
            code = restored;
        }
        if let std::borrow::Cow::Owned(restored) =
            code.cow_replace("__slice_top_ret_stmt;", "return;")
        {
            code = restored;
        }
    }
    if memchr::memmem::find(code.as_bytes(), b"export {").is_some() {
        use cow_utils::CowUtils;
        if let std::borrow::Cow::Owned(cleaned) = code.cow_replace("export {};", "") {
            code = cleaned;
        }
        if let std::borrow::Cow::Owned(cleaned) = code.cow_replace("export { };", "") {
            code = cleaned;
        }
    }

    code = fuse_array_pipelines_ast(&code);

    if memchr::memmem::find(code.as_bytes(), b"Buffer.").is_some() {
        crate::esm_cjs_linker::rewrite_buffer_fast_paths_ast(&code)
    } else {
        code
    }
}

fn update_backtick_state_on_line(line: &str, mut in_backtick: bool) -> bool {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    while i < len {
        let b = bytes[i];
        if b == b'\\' {
            i += 2;
            continue;
        }
        if in_backtick {
            if b == b'`' {
                in_backtick = false;
            }
            i += 1;
            continue;
        }
        if b == b'\'' || b == b'"' {
            let q = b;
            let mut j = i + 1;
            let mut found_close = None;
            while j < len {
                if bytes[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if bytes[j] == q {
                    found_close = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(close_idx) = found_close {
                i = close_idx + 1;
                continue;
            }
        } else if b == b'`' {
            in_backtick = true;
        }
        i += 1;
    }
    in_backtick
}

/// Strips JSDoc/license block comments (`/* ... */`), full-line `//` comments, and blank lines
/// at `slice pack` compile time so `boa_engine` does not spend cold-start CPU cycles lexing comments,
/// while preserving multi-line backtick template literals verbatim.
pub(crate) fn strip_js_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut in_block_comment = false;
    let mut in_backtick = false;
    for line in source.lines() {
        if in_backtick {
            out.push_str(line);
            out.push('\n');
            if line.as_bytes().contains(&b'`') {
                in_backtick = update_backtick_state_on_line(line, in_backtick);
            }
            continue;
        }
        let trimmed = line.trim();
        if in_block_comment {
            if let Some(pos) = trimmed.find("*/") {
                in_block_comment = false;
                let rem = trimmed[pos + 2..].trim();
                if !rem.is_empty() && !rem.starts_with("//") {
                    out.push_str(rem);
                    out.push('\n');
                    if rem.as_bytes().contains(&b'`') {
                        in_backtick = update_backtick_state_on_line(rem, in_backtick);
                    }
                }
            }
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if let Some(after_open) = trimmed.strip_prefix("/*") {
            if let Some(pos) = after_open.find("*/") {
                let rem = after_open[pos + 2..].trim();
                if !rem.is_empty() && !rem.starts_with("//") {
                    out.push_str(rem);
                    out.push('\n');
                    if rem.as_bytes().contains(&b'`') {
                        in_backtick = update_backtick_state_on_line(rem, in_backtick);
                    }
                }
            } else {
                in_block_comment = true;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
        if line.as_bytes().contains(&b'`') {
            in_backtick = update_backtick_state_on_line(line, in_backtick);
        }
    }
    if out.contains("Buffer.") {
        crate::esm_cjs_linker::rewrite_buffer_fast_paths_ast(&out)
    } else {
        out
    }
}

pub fn strip_typescript_and_normalize(source: &str, is_ts: bool) -> String {
    let mut js_code = strip_typescript_raw(source, is_ts);
    if ((js_code.contains("import ")
        && (js_code.contains("node:")
            || js_code.contains("slice:")
            || js_code.contains("bun:")
            || js_code.contains("cloudflare:")
            || js_code.contains("worker_threads")
            || js_code.contains("\"net\"")
            || js_code.contains("'net'")
            || js_code.contains("\"dgram\"")
            || js_code.contains("'dgram'")
            || js_code.contains("jsx-runtime")))
        || js_code.contains("export default "))
        && !js_code.contains("var __slice_modules =")
    {
        let mut dummy_deps = Vec::new();
        js_code = transform_module_to_cjs(&js_code, Path::new("."), None, &mut dummy_deps);
        if js_code.contains("exports.default = __slice_default;") {
            use std::hash::{Hash, Hasher};
            let mut hasher = rustc_hash::FxHasher::default();
            js_code.hash(&mut hasher);
            let rc_key = format!("{:016x}", hasher.finish());
            let direct_body = rewrite_cjs_require(&js_code);
            return format!(
                "var __slice_modules = {{}};//__SLICE_RC:{}\nvar __rc = globalThis.__slice_rc || (globalThis.__slice_rc = Object.create(null));\nvar __run = __rc[\"{}\"];\nif (!__run) {{\nvar __h = null;\n__run = __rc[\"{}\"] = function() {{\nif (!__h) {{\nvar exports = {{}}, module = {{ exports: exports }};\n{}\n__h = exports.default || module.exports;\n}}\nif (__h) globalThis.__slice_dispatch_fetch(__h);\n}};\n}}\n__run();\n",
                rc_key, rc_key, rc_key, direct_body
            );
        }
    }
    rewrite_cjs_require(&js_code)
}

/// Rewrites legacy CommonJS `const x = require("mod")` into synthetic deterministic ESM/CAS lookup
/// using Oxc AST `CallExpression` span replacement so `"require(...)"` inside string or template
/// literals is never modified.
pub(crate) fn rewrite_cjs_require(source: &str) -> String {
    if !source.contains("require(")
        || source.contains("var __slice_modules =")
        || source.contains("__slice_cjs_require(")
    {
        return source.to_string();
    }
    if let Some(rewritten) = crate::esm_cjs_linker::rewrite_require_ident_ast(source) {
        return rewritten;
    }
    source.replace("require(", "__slice_cjs_require(")
}
