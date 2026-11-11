//! Oxc TypeScript & JSX/TSX Ahead-of-Time Transpiler (`strip_typescript_raw_with_ext`)
//! and Comment/Require Normalizer.

use crate::compiler::fuse_array_pipelines_ast;
use crate::linker::{
    rewrite_buffer_fast_paths_ast, rewrite_require_ident_ast, transform_module_to_cjs,
};
use std::path::Path;

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

    if code.contains(".reduce(") || code.contains(".some(") || code.contains(".every(") {
        code = fuse_array_pipelines_ast(&code);
    }

    if memchr::memmem::find(code.as_bytes(), b"Buffer.").is_some() {
        rewrite_buffer_fast_paths_ast(&code)
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
pub fn strip_js_comments(source: &str) -> String {
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
        rewrite_buffer_fast_paths_ast(&out)
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
    if js_code.contains("require(") {
        rewrite_cjs_require(&js_code)
    } else {
        js_code
    }
}

/// Rewrites legacy CommonJS `const x = require("mod")` into synthetic deterministic ESM/CAS lookup
/// using Oxc AST `CallExpression` span replacement so `"require(...)"` inside string or template
/// literals is never modified.
pub fn rewrite_cjs_require(source: &str) -> String {
    if !source.contains("require(")
        || source.contains("var __slice_modules =")
        || source.contains("__slice_cjs_require(")
    {
        return source.to_string();
    }
    if let Some(rewritten) = rewrite_require_ident_ast(source) {
        return rewritten;
    }
    source.replace("require(", "__slice_cjs_require(")
}

/// Validates JS/TS/JSX/TSX syntax using Oxc's parser (`oxc::parser::Parser`).
/// Returns `Ok(())` if the source has no syntax errors, or `Err(SyntaxError message)` if invalid.
pub fn check_syntax_source(source: &str, path_hint: Option<&Path>) -> Result<(), String> {
    let allocator = oxc::allocator::Allocator::default();
    let parse_opts = oxc::parser::ParseOptions {
        allow_return_outside_function: true,
        ..Default::default()
    };
    let is_jsx_ext = path_hint
        .and_then(|p| p.extension())
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("tsx") || ext.eq_ignore_ascii_case("jsx"));
    let src_type = if is_jsx_ext {
        oxc::span::SourceType::tsx().with_module(true)
    } else {
        oxc::span::SourceType::ts().with_module(true)
    };
    let parsed = oxc::parser::Parser::new(&allocator, source, src_type)
        .with_options(parse_opts)
        .parse();
    if parsed.diagnostics.is_empty() {
        return Ok(());
    }
    if !is_jsx_ext {
        let retry_tsx = oxc::parser::Parser::new(
            &allocator,
            source,
            oxc::span::SourceType::tsx().with_module(true),
        )
        .with_options(parse_opts)
        .parse();
        if retry_tsx.diagnostics.is_empty() {
            return Ok(());
        }
    }
    let first_err = &parsed.diagnostics[0];
    let file_label = path_hint
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<eval>".to_string());
    Err(format!("SyntaxError in {}: {}", file_label, first_err))
}
