//! Oxc AST `require(...)`, Dynamic `import(...)`, `import.meta`, and `Buffer.*` Fast-Path Rewriters.

use crate::bundler::resolve_module_specifier;
use crate::linker::{
    canonical_mod_id, is_builtin_module, is_jsx_runtime_builtin, normalize_builtin_specifier,
    resolve_spec_and_record, walk_ast_call_expressions, walk_ast_expressions,
};
use std::path::{Path, PathBuf};

pub fn rewrite_cjs_requires_in_line(
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

#[inline]
fn is_unresolved_global_ident(
    scoping: &oxc::semantic::Scoping,
    ident: &oxc::ast::ast::IdentifierReference<'_>,
) -> bool {
    ident
        .reference_id
        .get()
        .is_none_or(|ref_id| scoping.get_reference(ref_id).symbol_id().is_none())
}

/// Rewrites `require("<spec>")` `CallExpression` string literal arguments via Oxc AST span
/// replacement without modifying `"require(...)"` inside string literals, template literals,
/// comments, or lexically shadowed `require` bindings.
pub fn rewrite_cjs_requires_ast(
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
            let scoping = oxc::semantic::SemanticBuilder::new()
                .build(&retry_ret.program)
                .semantic
                .into_scoping();
            let mut edits: Vec<(usize, usize, String)> = Vec::new();
            walk_ast_call_expressions(&retry_ret.program, &mut |call| {
                let Expression::Identifier(callee) = call.callee.without_parentheses() else {
                    return;
                };
                if callee.name.as_str() != "require"
                    || call.arguments.len() != 1
                    || !is_unresolved_global_ident(&scoping, callee)
                {
                    return;
                }
                let Some(Argument::StringLiteral(str_lit)) = call.arguments.first() else {
                    return;
                };
                let spec = str_lit.value.as_str();
                let resolved_opt = if is_builtin_module(spec) {
                    Some(normalize_builtin_specifier(spec))
                } else if let Some(resolved_path) = resolve_module_specifier(from_dir, spec) {
                    let id = canonical_mod_id(&resolved_path);
                    deps_out.push(resolved_path);
                    Some(id)
                } else if is_jsx_runtime_builtin(spec) {
                    Some(normalize_builtin_specifier(spec))
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

    let scoping = oxc::semantic::SemanticBuilder::new()
        .build(&parser_ret.program)
        .semantic
        .into_scoping();
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    walk_ast_call_expressions(&parser_ret.program, &mut |call| {
        let Expression::Identifier(callee) = call.callee.without_parentheses() else {
            return;
        };
        if callee.name.as_str() != "require"
            || call.arguments.len() != 1
            || !is_unresolved_global_ident(&scoping, callee)
        {
            return;
        }
        let Some(Argument::StringLiteral(str_lit)) = call.arguments.first() else {
            return;
        };
        let spec = str_lit.value.as_str();
        let resolved_opt = if is_builtin_module(spec) {
            Some(normalize_builtin_specifier(spec))
        } else if let Some(resolved_path) = resolve_module_specifier(from_dir, spec) {
            let id = canonical_mod_id(&resolved_path);
            deps_out.push(resolved_path);
            Some(id)
        } else if is_jsx_runtime_builtin(spec) {
            Some(normalize_builtin_specifier(spec))
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

/// Rewrites unshadowed global `require(...)` callee identifiers to `__slice_cjs_require(...)`
/// using Oxc AST + `SemanticBuilder` so string literals, comments, and local `require` bindings
/// are untouched.
pub fn rewrite_require_ident_ast(source: &str) -> Option<String> {
    use oxc::ast::ast::Expression;

    let allocator = oxc::allocator::Allocator::default();
    let source_type = oxc::span::SourceType::ts().with_module(true);
    let parser_ret = oxc::parser::Parser::new(&allocator, source, source_type).parse();
    if !parser_ret.diagnostics.is_empty() {
        return None;
    }

    let scoping = oxc::semantic::SemanticBuilder::new()
        .build(&parser_ret.program)
        .semantic
        .into_scoping();
    let mut callee_spans: Vec<(usize, usize)> = Vec::new();
    walk_ast_call_expressions(&parser_ret.program, &mut |call| {
        if let Expression::Identifier(callee) = call.callee.without_parentheses() {
            if callee.name.as_str() == "require" && is_unresolved_global_ident(&scoping, callee) {
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

/// Lowers unshadowed global `Buffer.byteLength(<arg>)` and `Buffer.from(<arg>, <enc>?).toString(<to_enc>?)`
/// via Oxc AST + `SemanticBuilder` span replacement so string literals, comments, and lexically
/// shadowed `Buffer` variables/parameters are never touched.
pub fn rewrite_buffer_fast_paths_ast(code: &str) -> String {
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

    let scoping = oxc::semantic::SemanticBuilder::new()
        .build(&parser_ret.program)
        .semantic
        .into_scoping();
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
                if obj_id.name.as_str() == "Buffer" && is_unresolved_global_ident(&scoping, obj_id)
                {
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
                || !is_unresolved_global_ident(&scoping, obj_id)
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

pub fn collect_require_edits_from_program(
    code: &str,
    program: &oxc::ast::ast::Program<'_>,
    from_dir: &Path,
    module_path: Option<&Path>,
    deps_out: &mut Vec<PathBuf>,
) -> Vec<(usize, usize, String)> {
    use oxc::ast::ast::{Argument, ChainElement, Expression};
    use oxc::span::GetSpan;

    let has_req = code.contains("require(");
    if !has_req && !code.contains("import(") && !code.contains("import.meta") {
        return Vec::new();
    }

    let req_scoping = if has_req {
        Some(
            oxc::semantic::SemanticBuilder::new()
                .build(program)
                .semantic
                .into_scoping(),
        )
    } else {
        None
    };

    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    fn handle_call(
        call: &oxc::ast::ast::CallExpression<'_>,
        scoping: Option<&oxc::semantic::Scoping>,
        from_dir: &Path,
        deps_out: &mut Vec<PathBuf>,
        edits: &mut Vec<(usize, usize, String)>,
    ) {
        let Expression::Identifier(callee) = call.callee.without_parentheses() else {
            return;
        };
        if callee.name.as_str() != "require"
            || call.arguments.len() != 1
            || scoping.is_some_and(|sc| !is_unresolved_global_ident(sc, callee))
        {
            return;
        }
        let Some(Argument::StringLiteral(str_lit)) = call.arguments.first() else {
            return;
        };
        let spec = str_lit.value.as_str();
        let resolved_opt = if is_builtin_module(spec) {
            Some(normalize_builtin_specifier(spec))
        } else if let Some(resolved_path) = resolve_module_specifier(from_dir, spec) {
            let id = canonical_mod_id(&resolved_path);
            deps_out.push(resolved_path);
            Some(id)
        } else if is_jsx_runtime_builtin(spec) {
            Some(normalize_builtin_specifier(spec))
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
        Expression::CallExpression(call) => {
            handle_call(call, req_scoping.as_ref(), from_dir, deps_out, &mut edits)
        }
        Expression::ChainExpression(ch) => {
            if let ChainElement::CallExpression(call) = &ch.expression {
                handle_call(call, req_scoping.as_ref(), from_dir, deps_out, &mut edits);
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
