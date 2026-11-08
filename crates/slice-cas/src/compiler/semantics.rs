//! Oxc Type-Directed Semantic Analyzer (`analyze_typescript_semantics`).

use rustc_hash::{FxHashMap, FxHashSet};

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
pub fn source_contains_jsx(source: &str) -> bool {
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
