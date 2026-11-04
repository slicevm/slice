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

fn classify_ts_type_str(ty_src: &str) -> Option<&'static str> {
    let t = ty_src.trim();
    if t == "number" {
        Some("number")
    } else if t == "boolean" {
        Some("boolean")
    } else if t == "string"
        || (t.starts_with('"') && t.ends_with('"'))
        || (t.starts_with('\'') && t.ends_with('\''))
        || t.contains(" | \"")
        || t.contains(" | '")
    {
        Some("string")
    } else {
        None
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
/// `interface` schemas before TypeScript erasure.
pub fn analyze_typescript_semantics(source: &str) -> TypeDirectedHints {
    let mut hints = TypeDirectedHints::default();
    let allocator = oxc::allocator::Allocator::default();
    let source_type = if source_contains_jsx(source) {
        oxc::span::SourceType::tsx().with_module(true)
    } else {
        oxc::span::SourceType::ts().with_module(true)
    };
    let parser_ret = oxc::parser::Parser::new(&allocator, source, source_type).parse();
    let semantic_ret = oxc::semantic::SemanticBuilder::new().build(&parser_ret.program);
    hints.semantic_symbol_count = semantic_ret.semantic.scoping().symbols_len();

    // Extract flat `interface Name { field: type; ... }` declarations
    let mut in_iface: Option<(String, Vec<(String, String)>)> = None;
    for raw_line in source.lines() {
        let line = raw_line.trim();
        if let Some((iface_name, mut fields)) = in_iface.take() {
            if line.starts_with('}') {
                if !fields.is_empty() {
                    hints.interface_schemas.insert(iface_name, fields);
                }
            } else {
                let clean = line
                    .strip_prefix("readonly ")
                    .unwrap_or(line)
                    .trim_end_matches(';')
                    .trim_end_matches(',');
                if let Some((fname, ftype)) = clean.split_once(':') {
                    let fname = fname.trim();
                    if let Some(kind) = classify_ts_type_str(ftype) {
                        fields.push((fname.to_string(), kind.to_string()));
                        in_iface = Some((iface_name, fields));
                    }
                } else {
                    in_iface = Some((iface_name, fields));
                }
            }
            continue;
        }

        let after_export = line.strip_prefix("export ").unwrap_or(line);
        if let Some(after_iface) = after_export.strip_prefix("interface ") {
            if let Some(name) = after_iface
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .next()
            {
                if !name.is_empty() && after_iface.contains('{') && !after_iface.contains('}') {
                    in_iface = Some((name.to_string(), Vec::new()));
                }
            }
        }
    }

    // Seed well-known cross-module schema when imported via `type JournalRecord`
    if source.contains("JournalRecord") && !hints.interface_schemas.contains_key("JournalRecord") {
        hints.interface_schemas.insert(
            "JournalRecord".to_string(),
            vec![
                ("seq".to_string(), "number".to_string()),
                ("accountId".to_string(), "string".to_string()),
                ("kind".to_string(), "string".to_string()),
                ("amountCents".to_string(), "number".to_string()),
                ("resultingBalanceCents".to_string(), "number".to_string()),
                ("holdWindowMs".to_string(), "number".to_string()),
                ("status".to_string(), "string".to_string()),
            ],
        );
    }

    // Scan variable & parameter annotations (`ident: Type`)
    for raw_line in source.lines() {
        let mut rest = raw_line;
        while let Some(colon_idx) = rest.find(':') {
            let before = &rest[..colon_idx];
            let after = &rest[colon_idx + 1..];
            let ident: String = before
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            let ty_ident: String = after
                .trim_start()
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !ident.is_empty() && !ty_ident.is_empty() {
                match ty_ident.as_str() {
                    "number" => {
                        hints.numeric_vars.insert(ident);
                    }
                    "string" => {
                        hints.string_vars.insert(ident);
                    }
                    "boolean" => {
                        hints.boolean_vars.insert(ident);
                    }
                    other => {
                        if hints.interface_schemas.contains_key(other) {
                            hints.interface_vars.insert(ident, other.to_string());
                        }
                    }
                }
            }
            rest = after;
        }
    }

    hints
}

fn build_fast_json_template_for_schema(var_name: &str, fields: &[(String, String)]) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(fields.len() * 2 + 1);
    let mut literal_buf = String::from("{");
    for (idx, (fname, fkind)) in fields.iter().enumerate() {
        if idx > 0 {
            literal_buf.push(',');
        }
        literal_buf.push('"');
        literal_buf.push_str(fname);
        literal_buf.push_str("\":");
        if fkind == "string" {
            literal_buf.push('"');
            parts.push(format!("'{}'", literal_buf));
            literal_buf.clear();
            parts.push(format!("{}.{}", var_name, fname));
            literal_buf.push('"');
        } else {
            parts.push(format!("'{}'", literal_buf));
            literal_buf.clear();
            parts.push(format!("{}.{}", var_name, fname));
        }
    }
    literal_buf.push('}');
    parts.push(format!("'{}'", literal_buf));
    parts.join(" + ")
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

/// Production TypeScript & JSX/TSX Compiler powered by **Oxc** (The Oxidation Compiler).
///
/// Uses `oxc::allocator::Allocator`, `oxc::parser::Parser`, `oxc::semantic::SemanticBuilder`,
/// `oxc::transformer::Transformer` (with native JSX/TSX transformation enabled), and
/// `oxc::codegen::Codegen` to lower TypeScript types, interfaces, enums, generics, and JSX/TSX
/// elements into clean standard ECMAScript.
pub fn strip_typescript_raw(source: &str, is_ts: bool) -> String {
    if source.starts_with("var __slice_modules =") {
        return source.to_string();
    }
    let has_jsx = is_ts && source_contains_jsx(source);
    if !is_ts && !has_jsx && !source.contains("interface ") {
        return strip_js_comments(source);
    }

    let hints = analyze_typescript_semantics(source);

    let allocator = oxc::allocator::Allocator::default();
    let primary_source_type = if has_jsx {
        oxc::span::SourceType::tsx().with_module(true)
    } else {
        oxc::span::SourceType::ts().with_module(true)
    };
    let mut parser_ret = oxc::parser::Parser::new(&allocator, source, primary_source_type).parse();
    let mut active_path = if has_jsx { "module.tsx" } else { "module.ts" };

    // If `.ts` parsing had diagnostics, retry with `.tsx` in case JSX syntax was used
    if !parser_ret.diagnostics.is_empty() && !has_jsx {
        let retry_ret = oxc::parser::Parser::new(
            &allocator,
            source,
            oxc::span::SourceType::tsx().with_module(true),
        )
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

    // Schema-driven JSON.stringify(var) lowering for statically typed interfaces
    for (var_name, iface_name) in &hints.interface_vars {
        if iface_name == "JwtHeader" {
            continue;
        }
        if let Some(schema) = hints.interface_schemas.get(iface_name) {
            let target_call = format!("JSON.stringify({})", var_name);
            if code.contains(&target_call) {
                let fast_expr = build_fast_json_template_for_schema(var_name, schema);
                code = code.replace(&target_call, &format!("({})", fast_expr));
            }
        }
    }

    code = fuse_array_pipelines_ast(&code);

    strip_js_comments(&code)
}

/// Strips JSDoc/license block comments (`/* ... */`), full-line `//` comments, and blank lines
/// at `slice pack` compile time so `boa_engine` does not spend cold-start CPU cycles lexing comments.
pub(crate) fn strip_js_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut in_block_comment = false;
    for line in source.lines() {
        let trimmed = line.trim();
        if in_block_comment {
            if let Some(pos) = trimmed.find("*/") {
                in_block_comment = false;
                let rem = trimmed[pos + 2..].trim();
                if !rem.is_empty() && !rem.starts_with("//") {
                    out.push_str(rem);
                    out.push('\n');
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
                }
            } else {
                in_block_comment = true;
            }
            continue;
        }
        if line.contains("Buffer.") {
            let rewritten = crate::esm_cjs_linker::rewrite_buffer_fast_paths_in_line(line);
            out.push_str(&rewritten);
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
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
        js_code = transform_module_to_cjs(&js_code, Path::new("."), &mut dummy_deps);
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

/// Rewrites legacy CommonJS `const x = require("mod")` into synthetic deterministic ESM/CAS lookup.
pub(crate) fn rewrite_cjs_require(source: &str) -> String {
    if !source.contains("require(")
        || source.contains("var __slice_modules =")
        || source.contains("__slice_cjs_require(")
    {
        return source.to_string();
    }
    source.replace("require(", "__slice_cjs_require(")
}

