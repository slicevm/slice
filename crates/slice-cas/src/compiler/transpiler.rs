//! Oxc TypeScript & JSX/TSX Ahead-of-Time Transpiler (`strip_typescript_raw_with_ext`)
//! and Comment/Require Normalizer.

use crate::compiler::diagnostics::{
    SyntaxErrorDiagnostic, blank_leading_shebang, check_syntax_source_detailed,
    extract_oxc_syntax_diagnostic,
};
use crate::linker::{
    has_esm_default_export_marker, rewrite_buffer_fast_paths_ast, rewrite_require_ident_ast,
    transform_module_to_cjs,
};
use std::path::Path;

#[inline]
fn may_contain_ts_or_jsx(source: &str) -> bool {
    let bytes = source.as_bytes();
    if memchr::memchr3(b':', b'<', b'!', bytes).is_some() {
        return true;
    }
    for kw in [
        &b"interface "[..],
        &b"interface{"[..],
        &b"type "[..],
        &b"enum "[..],
        &b"namespace "[..],
        &b"declare "[..],
        &b" as "[..],
        &b" satisfies "[..],
        &b" implements "[..],
        &b" abstract "[..],
        &b" readonly "[..],
        &b"public "[..],
        &b"private "[..],
        &b"protected "[..],
        &b"override "[..],
    ] {
        if memchr::memmem::find(bytes, kw).is_some() {
            return true;
        }
    }
    false
}

/// Extracts any top-level `return <expr>;` spans directly from an already-parsed Oxc `Program`
/// (`allow_return_outside_function: true`). Returns an empty `SmallVec` for the 99.9%+ common case
/// where `return` statements only appear inside functions, methods, or arrow bodies.
fn extract_top_level_return_spans(
    program: &oxc::ast::ast::Program<'_>,
    source: &str,
) -> smallvec::SmallVec<[(usize, usize, bool); 4]> {
    let mut top_ret_spans: smallvec::SmallVec<[(usize, usize, bool); 4]> =
        smallvec::SmallVec::new();
    for stmt in &program.body {
        if let oxc::ast::ast::Statement::ReturnStatement(ret) = stmt {
            let start = ret.span.start as usize;
            let has_arg = ret.argument.is_some();
            if start + 6 <= source.len() && &source[start..start + 6] == "return" {
                top_ret_spans.push((start, start + 6, has_arg));
            }
        }
    }
    top_ret_spans
}

fn build_masked_top_level_returns(source: &str, top_ret_spans: &[(usize, usize, bool)]) -> String {
    let mut out = String::with_capacity(source.len() + top_ret_spans.len() * 18);
    let mut cursor = 0usize;
    for &(start, end, has_arg) in top_ret_spans {
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
    out
}

fn transpile_parsed_oxc_program<'a>(
    allocator: &'a oxc::allocator::Allocator,
    mut program: oxc::ast::ast::Program<'a>,
    active_path: &str,
    had_top_return: bool,
    fallback_source: &str,
) -> String {
    let scoping = oxc::semantic::SemanticBuilder::new()
        .build(&program)
        .semantic
        .into_scoping();

    let mut transform_options = oxc::transformer::TransformOptions::default();
    transform_options.jsx.jsx_plugin = true;
    let transform_ret =
        oxc::transformer::Transformer::new(allocator, Path::new(active_path), &transform_options)
            .build_with_scoping(scoping, &mut program);

    if !transform_ret.diagnostics.is_empty() {
        return strip_js_comments(fallback_source);
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

    if memchr::memmem::find(code.as_bytes(), b"Buffer.").is_some() {
        rewrite_buffer_fast_paths_ast(&code)
    } else {
        code
    }
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
/// - Inspects `parser_ret.program.body` directly on the primary AST pass so modules containing
///   `return` statements inside functions/methods are parsed only ONCE, masking and re-parsing
///   only when an actual top-level `ReturnStatement` is present.
/// - Relies directly on `oxc::codegen::Codegen` (`CommentOptions::disabled()`) without running a
///   redundant post-codegen line-by-line comment stripper.
pub fn strip_typescript_raw_with_ext(source: &str, is_ts: bool, is_jsx_ext: bool) -> String {
    if source.starts_with("var __slice_modules =") {
        return source.to_string();
    }
    let clean_cow = blank_leading_shebang(source);
    let clean = clean_cow.as_ref();

    let has_jsx_hint =
        is_jsx_ext || (is_ts && memchr::memmem::find(clean.as_bytes(), b"@jsx").is_some());
    if !is_ts && !has_jsx_hint && !may_contain_ts_or_jsx(clean) {
        return if memchr::memmem::find(clean.as_bytes(), b"Buffer.").is_some() {
            rewrite_buffer_fast_paths_ast(clean)
        } else {
            clean_cow.into_owned()
        };
    }

    let allocator = oxc::allocator::Allocator::default();
    let parse_opts = oxc::parser::ParseOptions {
        allow_return_outside_function: true,
        ..Default::default()
    };
    let mut active_source_type = if has_jsx_hint {
        oxc::span::SourceType::tsx().with_module(true)
    } else {
        oxc::span::SourceType::ts().with_module(true)
    };
    let mut parser_ret = oxc::parser::Parser::new(&allocator, clean, active_source_type)
        .with_options(parse_opts)
        .parse();
    let mut active_path = if has_jsx_hint {
        "module.tsx"
    } else {
        "module.ts"
    };

    // If `.ts` parsing had diagnostics, retry with `.tsx` or sloppy script mode
    if !parser_ret.diagnostics.is_empty() && !has_jsx_hint {
        let tsx_type = oxc::span::SourceType::tsx().with_module(true);
        let retry_ret = oxc::parser::Parser::new(&allocator, clean, tsx_type)
            .with_options(parse_opts)
            .parse();
        if retry_ret.diagnostics.is_empty() {
            parser_ret = retry_ret;
            active_source_type = tsx_type;
            active_path = "module.tsx";
        }
    }
    if !parser_ret.diagnostics.is_empty() {
        let script_ts = oxc::span::SourceType::ts().with_module(false);
        let retry_script = oxc::parser::Parser::new(&allocator, clean, script_ts)
            .with_options(parse_opts)
            .parse();
        if retry_script.diagnostics.is_empty() {
            parser_ret = retry_script;
            active_source_type = script_ts;
        }
    }

    // Preserve original lines verbatim when syntax diagnostics exist so downstream error
    // reporting never suffers shifted line numbers.
    if !parser_ret.diagnostics.is_empty() {
        return source.to_string();
    }

    let top_ret_spans = extract_top_level_return_spans(&parser_ret.program, clean);
    if top_ret_spans.is_empty() {
        if !has_jsx_hint && !may_contain_ts_or_jsx(clean) {
            return if memchr::memmem::find(clean.as_bytes(), b"Buffer.").is_some() {
                rewrite_buffer_fast_paths_ast(clean)
            } else {
                clean_cow.into_owned()
            };
        }
        return transpile_parsed_oxc_program(
            &allocator,
            parser_ret.program,
            active_path,
            false,
            clean,
        );
    }

    let masked_source = build_masked_top_level_returns(clean, &top_ret_spans);
    let masked_alloc = oxc::allocator::Allocator::default();
    let masked_ret = oxc::parser::Parser::new(&masked_alloc, &masked_source, active_source_type)
        .with_options(parse_opts)
        .parse();
    if !masked_ret.diagnostics.is_empty() {
        return source.to_string();
    }
    transpile_parsed_oxc_program(&masked_alloc, masked_ret.program, active_path, true, clean)
}

/// Strips JSDoc/license block comments (`/* ... */`), `//` line comments, and blank lines
/// at `slice pack` compile time so `boa_engine` does not spend cold-start CPU cycles lexing comments,
/// while preserving string literals, RegExp literals, and multi-line / nested `${...}` template literals.
pub fn strip_js_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut raw_out = Vec::with_capacity(len);
    let mut i = 0usize;

    let mut in_template = false;
    let mut brace_depth = 0usize;
    let mut tpl_brace_stack: Vec<usize> = Vec::new();
    let mut paren_stack: Vec<bool> = Vec::new();
    let mut after_cond_close_paren = false;
    let mut prev_sig_byte = 0u8;
    let mut prev_was_regex_kw = false;
    let mut prev_was_cond_kw = false;

    let is_id_start = |b: u8| b.is_ascii_alphabetic() || b == b'_' || b == b'$';
    let is_id_cont = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';

    while i < len {
        if in_template {
            let b = bytes[i];
            raw_out.push(b);
            i += 1;
            if b == b'\\' && i < len {
                raw_out.push(bytes[i]);
                i += 1;
            } else if b == b'`' {
                in_template = false;
                prev_sig_byte = b'`';
                prev_was_regex_kw = false;
                prev_was_cond_kw = false;
                after_cond_close_paren = false;
            } else if b == b'$' && i < len && bytes[i] == b'{' {
                raw_out.push(b'{');
                i += 1;
                tpl_brace_stack.push(brace_depth);
                in_template = false;
                prev_sig_byte = b'{';
                prev_was_regex_kw = false;
                prev_was_cond_kw = false;
                after_cond_close_paren = false;
            }
            continue;
        }

        let b = bytes[i];

        // Whitespace
        if b == b' ' || b == b'\t' || b == b'\r' || b == b'\n' {
            if b != b'\r' {
                raw_out.push(b);
            }
            i += 1;
            continue;
        }

        // Line comment `// ...`
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // Block comment `/* ... */`
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            let mut had_newline = false;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                if bytes[i] == b'\n' {
                    had_newline = true;
                }
                i += 1;
            }
            i = (i + 2).min(len);
            if had_newline {
                raw_out.push(b'\n');
            } else if raw_out.last().is_some_and(|&p| !p.is_ascii_whitespace())
                && bytes.get(i).is_some_and(|&n| !n.is_ascii_whitespace())
            {
                raw_out.push(b' ');
            }
            continue;
        }

        // Quoted string literal
        if b == b'\'' || b == b'"' {
            let q = b;
            raw_out.push(q);
            i += 1;
            while i < len {
                let c = bytes[i];
                raw_out.push(c);
                i += 1;
                if c == b'\\' && i < len {
                    raw_out.push(bytes[i]);
                    i += 1;
                } else if c == q {
                    break;
                }
            }
            prev_sig_byte = q;
            prev_was_regex_kw = false;
            prev_was_cond_kw = false;
            after_cond_close_paren = false;
            continue;
        }

        // Template literal opener
        if b == b'`' {
            raw_out.push(b'`');
            i += 1;
            in_template = true;
            continue;
        }

        // RegExp literal vs division
        if b == b'/' {
            let can_start_regex = prev_was_regex_kw
                || after_cond_close_paren
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
                raw_out.push(b'/');
                i += 1;
                let mut in_class = false;
                while i < len {
                    let rc = bytes[i];
                    if rc == b'\n' {
                        break;
                    }
                    raw_out.push(rc);
                    i += 1;
                    if rc == b'\\' && i < len {
                        raw_out.push(bytes[i]);
                        i += 1;
                    } else if rc == b'[' {
                        in_class = true;
                    } else if rc == b']' {
                        in_class = false;
                    } else if rc == b'/' && !in_class {
                        while i < len && bytes[i].is_ascii_alphabetic() {
                            raw_out.push(bytes[i]);
                            i += 1;
                        }
                        break;
                    }
                }
                prev_sig_byte = b'/';
                prev_was_regex_kw = false;
                prev_was_cond_kw = false;
                after_cond_close_paren = false;
                continue;
            }
        }

        if b == b'(' {
            paren_stack.push(prev_was_cond_kw);
            raw_out.push(b'(');
            prev_sig_byte = b'(';
            prev_was_regex_kw = false;
            prev_was_cond_kw = false;
            after_cond_close_paren = false;
            i += 1;
            continue;
        }
        if b == b')' {
            after_cond_close_paren = paren_stack.pop().unwrap_or(false);
            raw_out.push(b')');
            prev_sig_byte = b')';
            prev_was_regex_kw = false;
            prev_was_cond_kw = false;
            i += 1;
            continue;
        }
        if b == b'{' {
            brace_depth += 1;
            raw_out.push(b'{');
            prev_sig_byte = b'{';
            prev_was_regex_kw = false;
            prev_was_cond_kw = false;
            after_cond_close_paren = false;
            i += 1;
            continue;
        }
        if b == b'}' {
            raw_out.push(b'}');
            prev_sig_byte = b'}';
            prev_was_regex_kw = false;
            prev_was_cond_kw = false;
            after_cond_close_paren = false;
            i += 1;
            if tpl_brace_stack.last() == Some(&brace_depth) {
                tpl_brace_stack.pop();
                in_template = true;
            } else {
                brace_depth = brace_depth.saturating_sub(1);
            }
            continue;
        }

        if is_id_start(b) {
            let start = i;
            i += 1;
            while i < len && is_id_cont(bytes[i]) {
                i += 1;
            }
            raw_out.extend_from_slice(&bytes[start..i]);
            let ident = &source[start..i];
            let is_dot_prop =
                prev_sig_byte == b'.' && !(start >= 3 && &bytes[start - 3..start] == b"...");
            prev_was_regex_kw = !is_dot_prop
                && matches!(
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
                        | "do"
                        | "new"
                        | "yield"
                        | "await"
                );
            prev_was_cond_kw = !is_dot_prop && matches!(ident, "if" | "while" | "for" | "with");
            after_cond_close_paren = false;
            prev_sig_byte = bytes[i - 1];
            continue;
        }

        raw_out.push(b);
        prev_sig_byte = b;
        prev_was_regex_kw = false;
        prev_was_cond_kw = false;
        after_cond_close_paren = false;
        i += 1;
    }

    // Compact blank lines Outside template literals while preserving multi-line templates verbatim:
    let stripped_str = String::from_utf8(raw_out).unwrap_or_else(|_| source.to_string());
    let mut out = String::with_capacity(stripped_str.len());
    let mut tpl_state = false;
    let mut tpl_braces: Vec<usize> = Vec::new();
    let mut b_depth = 0usize;
    for line in stripped_str.split('\n') {
        if !tpl_state && line.trim().is_empty() {
            continue;
        }
        out.push_str(line);
        out.push('\n');
        let lb = line.as_bytes();
        let mut li = 0usize;
        while li < lb.len() {
            if tpl_state {
                if lb[li] == b'\\' {
                    li = (li + 2).min(lb.len());
                    continue;
                }
                if lb[li] == b'`' {
                    tpl_state = false;
                } else if lb[li] == b'$' && li + 1 < lb.len() && lb[li + 1] == b'{' {
                    tpl_braces.push(b_depth);
                    tpl_state = false;
                    li += 2;
                    continue;
                }
                li += 1;
                continue;
            }
            let c = lb[li];
            if c == b'\'' || c == b'"' {
                let q = c;
                li += 1;
                while li < lb.len() {
                    if lb[li] == b'\\' {
                        li = (li + 2).min(lb.len());
                        continue;
                    }
                    if lb[li] == q {
                        li += 1;
                        break;
                    }
                    li += 1;
                }
                continue;
            }
            if c == b'`' {
                tpl_state = true;
            } else if c == b'{' {
                b_depth += 1;
            } else if c == b'}' {
                if tpl_braces.last() == Some(&b_depth) {
                    tpl_braces.pop();
                    tpl_state = true;
                } else {
                    b_depth = b_depth.saturating_sub(1);
                }
            }
            li += 1;
        }
    }

    if out.contains("Buffer.") {
        rewrite_buffer_fast_paths_ast(&out)
    } else {
        out
    }
}

pub fn strip_typescript_and_normalize(source: &str, is_ts: bool) -> String {
    strip_typescript_and_normalize_with_ext(source, is_ts, false)
}

fn normalize_transpiled_module_js(mut js_code: String) -> String {
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
        if has_esm_default_export_marker(&js_code) {
            let digest = blake3::hash(js_code.as_bytes());
            let rc_u64 = u64::from_le_bytes(digest.as_bytes()[0..8].try_into().unwrap());
            let rc_key = format!("{:016x}", rc_u64);
            let direct_body = rewrite_cjs_require(&js_code);
            return format!(
                "var __slice_modules = {{}};//__SLICE_RC:{}\nvar __rc = globalThis.__slice_rc || (globalThis.__slice_rc = Object.create(null));\nvar __run = __rc[\"{}\"];\nif (!__run) {{\nvar __h = null;\n__run = __rc[\"{}\"] = function() {{\nif (!__h) {{\nvar exports = {{}}, module = {{ exports: exports }};\n{}\n__h = exports.__esModule ? exports.default : module.exports;\n}}\nif (__h) globalThis.__slice_dispatch_fetch(__h);\n}};\n}}\n__run();\n",
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

pub fn strip_typescript_and_normalize_with_ext(
    source: &str,
    is_ts: bool,
    is_jsx_ext: bool,
) -> String {
    let js_code = strip_typescript_raw_with_ext(source, is_ts, is_jsx_ext);
    normalize_transpiled_module_js(js_code)
}

/// Single-pass syntax-checked TypeScript/JSX/ECMAScript compiler for `compile_source_to_bytecode`:
/// - Returns `Err(SyntaxErrorDiagnostic)` with exact 1-based `(line, col)` in `source` when invalid.
/// - Preserves pure-ECMAScript source lines and columns verbatim without running `Codegen`.
/// - Attaches an inline `//@sm:||<line_map>` header when TypeScript lowering alters line counts so
///   semantic early errors and runtime stack frames map back to original lines.
pub fn compile_ts_or_js_checked(
    source: &str,
    is_ts: bool,
    is_jsx_ext: bool,
) -> Result<String, SyntaxErrorDiagnostic> {
    if source.starts_with("var __slice_modules =") {
        return Ok(source.to_string());
    }
    let clean_cow = blank_leading_shebang(source);
    let clean = clean_cow.as_ref();
    let allocator = oxc::allocator::Allocator::default();
    let parse_opts = oxc::parser::ParseOptions {
        allow_return_outside_function: true,
        ..Default::default()
    };

    let has_jsx_hint =
        is_jsx_ext || (is_ts && memchr::memmem::find(clean.as_bytes(), b"@jsx").is_some());

    // Fast path for pure JS inputs without TypeScript/JSX markers: single parse, zero Codegen!
    if !has_jsx_hint && !may_contain_ts_or_jsx(clean) {
        let mjs_ret = oxc::parser::Parser::new(&allocator, clean, oxc::span::SourceType::mjs())
            .with_options(parse_opts)
            .parse();
        if mjs_ret.diagnostics.is_empty()
            && extract_top_level_return_spans(&mjs_ret.program, clean).is_empty()
        {
            let fast_js = if memchr::memmem::find(clean.as_bytes(), b"Buffer.").is_some() {
                rewrite_buffer_fast_paths_ast(clean)
            } else {
                clean_cow.into_owned()
            };
            return Ok(normalize_transpiled_module_js(fast_js));
        }
    }

    let mut active_source_type = if has_jsx_hint {
        oxc::span::SourceType::tsx().with_module(true)
    } else {
        oxc::span::SourceType::ts().with_module(true)
    };
    let mut parser_ret = oxc::parser::Parser::new(&allocator, clean, active_source_type)
        .with_options(parse_opts)
        .parse();
    let mut active_path = if has_jsx_hint {
        "module.tsx"
    } else {
        "module.ts"
    };

    if !parser_ret.diagnostics.is_empty() && !has_jsx_hint {
        let tsx_type = oxc::span::SourceType::tsx().with_module(true);
        let retry_ret = oxc::parser::Parser::new(&allocator, clean, tsx_type)
            .with_options(parse_opts)
            .parse();
        if retry_ret.diagnostics.is_empty() {
            parser_ret = retry_ret;
            active_source_type = tsx_type;
            active_path = "module.tsx";
        }
    }

    if !parser_ret.diagnostics.is_empty() {
        let script_ts = oxc::span::SourceType::ts().with_module(false);
        let retry_script = oxc::parser::Parser::new(&allocator, clean, script_ts)
            .with_options(parse_opts)
            .parse();
        if retry_script.diagnostics.is_empty() {
            parser_ret = retry_script;
            active_source_type = script_ts;
        } else {
            for sloppy_js in [
                oxc::span::SourceType::cjs(),
                oxc::span::SourceType::default(),
            ] {
                let retry_js = oxc::parser::Parser::new(&allocator, clean, sloppy_js)
                    .with_options(parse_opts)
                    .parse();
                if retry_js.diagnostics.is_empty() {
                    let fast_js = if memchr::memmem::find(clean.as_bytes(), b"Buffer.").is_some() {
                        rewrite_buffer_fast_paths_ast(clean)
                    } else {
                        clean_cow.into_owned()
                    };
                    return Ok(normalize_transpiled_module_js(fast_js));
                }
            }
            return Err(extract_oxc_syntax_diagnostic(
                clean,
                &parser_ret.diagnostics[0],
            ));
        }
    }

    let top_ret_spans = extract_top_level_return_spans(&parser_ret.program, clean);
    let raw_transpiled = if top_ret_spans.is_empty() {
        transpile_parsed_oxc_program(&allocator, parser_ret.program, active_path, false, clean)
    } else {
        let masked_source = build_masked_top_level_returns(clean, &top_ret_spans);
        let masked_alloc = oxc::allocator::Allocator::default();
        let masked_ret =
            oxc::parser::Parser::new(&masked_alloc, &masked_source, active_source_type)
                .with_options(parse_opts)
                .parse();
        if !masked_ret.diagnostics.is_empty() {
            return Err(extract_oxc_syntax_diagnostic(
                clean,
                &masked_ret.diagnostics[0],
            ));
        }
        transpile_parsed_oxc_program(&masked_alloc, masked_ret.program, active_path, true, clean)
    };

    let normalized = normalize_transpiled_module_js(raw_transpiled);
    if !normalized.starts_with("var __slice_modules =")
        && !normalized.starts_with("// #slice-source: ")
        && !normalized.starts_with("//@sm:")
    {
        let orig_nl = clean.bytes().filter(|&b| b == b'\n').count();
        let out_nl = normalized.bytes().filter(|&b| b == b'\n').count();
        if orig_nl != out_nl {
            let line_map = crate::bundler::source_map::compute_source_line_map(clean, &normalized);
            if !line_map.is_empty() {
                return Ok(format!("//@sm:||{}\n{}", line_map, normalized));
            }
        }
    }
    Ok(normalized)
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
/// Returns `Ok(())` if the source has no syntax errors, or `Err(SyntaxError message)` with exact
/// 1-based `(line, col)` coordinates if invalid.
pub fn check_syntax_source(source: &str, path_hint: Option<&Path>) -> Result<(), String> {
    let file_label = path_hint
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<eval>".to_string());
    check_syntax_source_detailed(source, path_hint)
        .map_err(|diag| diag.to_labeled_error_string(&file_label))
}
