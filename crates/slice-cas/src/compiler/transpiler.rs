//! Oxc TypeScript & JSX/TSX Ahead-of-Time Transpiler (`strip_typescript_raw_with_ext`),
//! Single-Pass Syntax-Checked Bundler Stripper, and Standalone Module Normalizer.

pub use crate::compiler::comments::strip_js_comments;
use crate::compiler::diagnostics::{
    SyntaxErrorDiagnostic, blank_leading_shebang, check_syntax_source_detailed,
    default_oxc_parse_options, extract_oxc_syntax_diagnostic, parse_ts_or_tsx_with_retries,
    try_parse_sloppy_js,
};
use crate::compiler::ts_detector::program_has_ts_or_jsx;
use crate::compiler::ts_helpers::{
    SLICE_OXC_BABEL_HELPERS, finalize_standalone_cjs_envelope,
    has_standalone_builtin_import_or_cjs_export, inspect_ts_transform_flags,
    lower_auto_accessors_in_js, lower_unbundled_esm_for_boa, may_contain_ts_or_jsx,
};
use crate::linker::{
    VIRTUAL_NO_FS_DIR, rewrite_buffer_fast_paths_ast, rewrite_require_ident_ast,
    transform_module_to_cjs,
};
use std::borrow::Cow;
use std::path::Path;

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
    program: oxc::ast::ast::Program<'a>,
    active_path: &str,
    had_top_return: bool,
    fallback_source: &str,
) -> String {
    transpile_parsed_oxc_program_with_hook(
        allocator,
        program,
        active_path,
        had_top_return,
        fallback_source,
        None,
    )
}

fn transpile_parsed_oxc_program_with_hook<'a>(
    allocator: &'a oxc::allocator::Allocator,
    mut program: oxc::ast::ast::Program<'a>,
    active_path: &str,
    had_top_return: bool,
    fallback_source: &str,
    on_final_ast: Option<&mut dyn FnMut(&oxc::ast::ast::Program<'_>)>,
) -> String {
    let (needs_cjs_mode, has_accessor) = inspect_ts_transform_flags(&program);
    let scoping = oxc::semantic::SemanticBuilder::new()
        .with_enum_eval(true)
        .build(&program)
        .semantic
        .into_scoping();

    let mut transform_options = oxc::transformer::TransformOptions::default();
    transform_options.jsx.jsx_plugin = true;
    transform_options.decorator.legacy = true;
    transform_options.env.es2026.explicit_resource_management = true;
    transform_options.helper_loader.mode = oxc::transformer::HelperLoaderMode::External;
    if needs_cjs_mode {
        transform_options.env.module = oxc::transformer::Module::CommonJS;
    }

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

    if needs_cjs_mode && let Some(stripped) = code.strip_prefix("\"use strict\";\n") {
        code = stripped.to_string();
    }

    let has_babel_helpers = memchr::memmem::find(code.as_bytes(), b"babelHelpers.").is_some();
    if has_babel_helpers {
        let mut with_helpers =
            String::with_capacity(SLICE_OXC_BABEL_HELPERS.len() + code.len() + 2);
        with_helpers.push_str(SLICE_OXC_BABEL_HELPERS);
        with_helpers.push_str(&code);
        code = with_helpers;
    }

    if has_accessor {
        code = lower_auto_accessors_in_js(&code);
    }

    if had_top_return {
        use cow_utils::CowUtils;
        if let Cow::Owned(restored) = code.cow_replace("__slice_top_ret_stmt = ", "return ") {
            code = restored;
        }
        if let Cow::Owned(restored) = code.cow_replace("__slice_top_ret_stmt;", "return;") {
            code = restored;
        }
    }
    let mut had_export_empty = false;
    if memchr::memmem::find(code.as_bytes(), b"export {").is_some() {
        use cow_utils::CowUtils;
        if let Cow::Owned(cleaned) = code.cow_replace("export {};", "") {
            code = cleaned;
            had_export_empty = true;
        }
        if let Cow::Owned(cleaned) = code.cow_replace("export { };", "") {
            code = cleaned;
            had_export_empty = true;
        }
    }

    let has_buffer_dot = memchr::memmem::find(code.as_bytes(), b"Buffer.").is_some();
    if !had_top_return
        && !had_export_empty
        && !has_accessor
        && !has_babel_helpers
        && !has_buffer_dot
        && !needs_module_or_require_normalization(&code)
        && let Some(cb) = on_final_ast
    {
        cb(&program);
    }

    if has_buffer_dot {
        rewrite_buffer_fast_paths_ast(&code)
    } else {
        code
    }
}

/// Production TypeScript & JSX/TSX Compiler powered by **Oxc** (The Oxidation Compiler).
pub fn strip_typescript_raw(source: &str, is_ts: bool) -> String {
    strip_typescript_raw_with_ext(source, is_ts, false)
}

/// Extension-aware Oxc TypeScript & JSX/TSX compiler.
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
    let (parser_ret, active_source_type, active_path) =
        parse_ts_or_tsx_with_retries(&allocator, clean, has_jsx_hint);
    if !parser_ret.diagnostics.is_empty() {
        return source.to_string();
    }

    let top_ret_spans = extract_top_level_return_spans(&parser_ret.program, clean);
    if top_ret_spans.is_empty() {
        if !has_jsx_hint && !program_has_ts_or_jsx(&parser_ret.program) {
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
        .with_options(default_oxc_parse_options())
        .parse();
    if !masked_ret.diagnostics.is_empty() {
        return source.to_string();
    }
    transpile_parsed_oxc_program(&masked_alloc, masked_ret.program, active_path, true, clean)
}

/// Fused syntax-validation + TypeScript/JSX stripping pass for `bundle_entrypoint_with_deps`,
/// eliminating the redundant pre-strip `check_syntax_source` Oxc parse on every user module.
pub(crate) fn strip_typescript_checked_for_bundler(
    source: &str,
    path: &Path,
    is_ts: bool,
    is_jsx_ext: bool,
) -> Result<String, String> {
    let clean_cow = blank_leading_shebang(source);
    let clean = clean_cow.as_ref();
    let allocator = oxc::allocator::Allocator::default();
    let parse_opts = default_oxc_parse_options();
    let has_jsx_hint =
        is_jsx_ext || (is_ts && memchr::memmem::find(clean.as_bytes(), b"@jsx").is_some());

    if !has_jsx_hint && !may_contain_ts_or_jsx(clean) {
        let mjs_ret = oxc::parser::Parser::new(&allocator, clean, oxc::span::SourceType::mjs())
            .with_options(parse_opts)
            .parse();
        if mjs_ret.diagnostics.is_empty()
            && extract_top_level_return_spans(&mjs_ret.program, clean).is_empty()
            && !program_has_ts_or_jsx(&mjs_ret.program)
        {
            drop(mjs_ret);
            return Ok(
                if memchr::memmem::find(clean.as_bytes(), b"Buffer.").is_some() {
                    rewrite_buffer_fast_paths_ast(clean)
                } else {
                    clean_cow.into_owned()
                },
            );
        }
    }

    let (parser_ret, active_source_type, active_path) =
        parse_ts_or_tsx_with_retries(&allocator, clean, has_jsx_hint);
    if !parser_ret.diagnostics.is_empty() {
        if try_parse_sloppy_js(&allocator, clean).is_some() {
            return Ok(
                if memchr::memmem::find(clean.as_bytes(), b"Buffer.").is_some() {
                    rewrite_buffer_fast_paths_ast(clean)
                } else {
                    clean_cow.into_owned()
                },
            );
        }
        let diag = extract_oxc_syntax_diagnostic(clean, &parser_ret.diagnostics[0]);
        return Err(diag.to_labeled_error_string(&path.display().to_string()));
    }

    let top_ret_spans = extract_top_level_return_spans(&parser_ret.program, clean);
    if top_ret_spans.is_empty() {
        if !has_jsx_hint && !program_has_ts_or_jsx(&parser_ret.program) {
            return Ok(
                if memchr::memmem::find(clean.as_bytes(), b"Buffer.").is_some() {
                    rewrite_buffer_fast_paths_ast(clean)
                } else {
                    clean_cow.into_owned()
                },
            );
        }
        return Ok(transpile_parsed_oxc_program(
            &allocator,
            parser_ret.program,
            active_path,
            false,
            clean,
        ));
    }

    let masked_source = build_masked_top_level_returns(clean, &top_ret_spans);
    let masked_alloc = oxc::allocator::Allocator::default();
    let masked_ret = oxc::parser::Parser::new(&masked_alloc, &masked_source, active_source_type)
        .with_options(parse_opts)
        .parse();
    if !masked_ret.diagnostics.is_empty() {
        let diag = extract_oxc_syntax_diagnostic(clean, &masked_ret.diagnostics[0]);
        return Err(diag.to_labeled_error_string(&path.display().to_string()));
    }
    Ok(transpile_parsed_oxc_program(
        &masked_alloc,
        masked_ret.program,
        active_path,
        true,
        clean,
    ))
}

pub fn strip_typescript_and_normalize(source: &str, is_ts: bool) -> String {
    strip_typescript_and_normalize_with_ext(source, is_ts, false)
}

fn normalize_transpiled_module_js(js_code: String) -> String {
    if has_standalone_builtin_import_or_cjs_export(&js_code) {
        let mut dummy_deps = Vec::new();
        let cjs = transform_module_to_cjs(
            &js_code,
            Path::new(VIRTUAL_NO_FS_DIR),
            None,
            &mut dummy_deps,
        );
        return finalize_standalone_cjs_envelope(&cjs, true, rewrite_cjs_require);
    }
    if js_code.contains("require(") {
        rewrite_cjs_require(&js_code)
    } else {
        js_code
    }
}

/// Lowers any unbundled ESM `import`/`export` statements into CJS (`__slice_cjs_require` +
/// `exports` / `module.exports`) with zero host `stat()` syscalls.
pub fn lower_unbundled_esm(js_code: &str) -> String {
    lower_unbundled_esm_for_boa(js_code, rewrite_cjs_require)
}

pub fn strip_typescript_and_normalize_with_ext(
    source: &str,
    is_ts: bool,
    is_jsx_ext: bool,
) -> String {
    let js_code = strip_typescript_raw_with_ext(source, is_ts, is_jsx_ext);
    normalize_transpiled_module_js(js_code)
}

#[inline]
fn needs_module_or_require_normalization(js_code: &str) -> bool {
    let bytes = js_code.as_bytes();
    if memchr::memmem::find(bytes, b"require(").is_some() {
        return true;
    }
    has_standalone_builtin_import_or_cjs_export(js_code)
}

fn maybe_invoke_ast_hook_for_pure_js(
    clean: &str,
    program: Option<&oxc::ast::ast::Program<'_>>,
    on_final_ast: Option<&mut dyn FnMut(&oxc::ast::ast::Program<'_>)>,
) {
    if memchr::memmem::find(clean.as_bytes(), b"Buffer.").is_none()
        && !needs_module_or_require_normalization(clean)
        && let (Some(prog), Some(cb)) = (program, on_final_ast)
    {
        cb(prog);
    }
}

fn finalize_pure_js_cow(clean_cow: Cow<'_, str>) -> Cow<'_, str> {
    let has_buffer_dot = memchr::memmem::find(clean_cow.as_bytes(), b"Buffer.").is_some();
    if !has_buffer_dot && !needs_module_or_require_normalization(&clean_cow) {
        return clean_cow;
    }
    let fast_js = if has_buffer_dot {
        rewrite_buffer_fast_paths_ast(&clean_cow)
    } else {
        clean_cow.into_owned()
    };
    Cow::Owned(normalize_transpiled_module_js(fast_js))
}

/// Single-pass syntax-checked TypeScript/JSX/ECMAScript compiler for `compile_source_to_bytecode`.
pub fn compile_ts_or_js_checked(
    source: &str,
    is_ts: bool,
    is_jsx_ext: bool,
) -> Result<String, SyntaxErrorDiagnostic> {
    compile_ts_or_js_checked_with_ast_hook(source, is_ts, is_jsx_ext, |_| {}).map(Cow::into_owned)
}

/// Borrow-preserving variant of [`compile_ts_or_js_checked`] invoking `on_final_ast` with the
/// already-parsed (and, when TypeScript was lowered, post-transform) Oxc `Program`.
pub fn compile_ts_or_js_checked_with_ast_hook<'a, F>(
    source: &'a str,
    is_ts: bool,
    is_jsx_ext: bool,
    mut on_final_ast: F,
) -> Result<Cow<'a, str>, SyntaxErrorDiagnostic>
where
    F: FnMut(&oxc::ast::ast::Program<'_>),
{
    if source.starts_with("var __slice_modules =") {
        return Ok(Cow::Borrowed(source));
    }
    let clean_cow = blank_leading_shebang(source);
    let clean = clean_cow.as_ref();
    let allocator = oxc::allocator::Allocator::default();
    let parse_opts = default_oxc_parse_options();

    let has_jsx_hint =
        is_jsx_ext || (is_ts && memchr::memmem::find(clean.as_bytes(), b"@jsx").is_some());

    // Fast path for pure JS inputs without TypeScript/JSX markers: single parse, zero Codegen!
    if !has_jsx_hint && !may_contain_ts_or_jsx(clean) {
        let mjs_ret = oxc::parser::Parser::new(&allocator, clean, oxc::span::SourceType::mjs())
            .with_options(parse_opts)
            .parse();
        if mjs_ret.diagnostics.is_empty()
            && extract_top_level_return_spans(&mjs_ret.program, clean).is_empty()
            && !program_has_ts_or_jsx(&mjs_ret.program)
        {
            maybe_invoke_ast_hook_for_pure_js(
                clean,
                Some(&mjs_ret.program),
                Some(&mut on_final_ast),
            );
            drop(mjs_ret);
            return Ok(finalize_pure_js_cow(clean_cow));
        }
    }

    let (parser_ret, active_source_type, active_path) =
        parse_ts_or_tsx_with_retries(&allocator, clean, has_jsx_hint);

    if !parser_ret.diagnostics.is_empty() {
        if let Some(retry_js) = try_parse_sloppy_js(&allocator, clean) {
            let top_ok = extract_top_level_return_spans(&retry_js.program, clean).is_empty();
            maybe_invoke_ast_hook_for_pure_js(
                clean,
                top_ok.then_some(&retry_js.program),
                Some(&mut on_final_ast),
            );
            drop(retry_js);
            return Ok(finalize_pure_js_cow(clean_cow));
        }
        return Err(extract_oxc_syntax_diagnostic(
            clean,
            &parser_ret.diagnostics[0],
        ));
    }

    let top_ret_spans = extract_top_level_return_spans(&parser_ret.program, clean);
    if top_ret_spans.is_empty() && !has_jsx_hint && !program_has_ts_or_jsx(&parser_ret.program) {
        maybe_invoke_ast_hook_for_pure_js(
            clean,
            Some(&parser_ret.program),
            Some(&mut on_final_ast),
        );
        drop(parser_ret);
        return Ok(finalize_pure_js_cow(clean_cow));
    }

    let raw_transpiled = if top_ret_spans.is_empty() {
        transpile_parsed_oxc_program_with_hook(
            &allocator,
            parser_ret.program,
            active_path,
            false,
            clean,
            Some(&mut on_final_ast),
        )
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
        transpile_parsed_oxc_program_with_hook(
            &masked_alloc,
            masked_ret.program,
            active_path,
            true,
            clean,
            None,
        )
    };

    let normalized = if needs_module_or_require_normalization(&raw_transpiled) {
        normalize_transpiled_module_js(raw_transpiled)
    } else {
        raw_transpiled
    };
    if !normalized.starts_with("var __slice_modules =")
        && !normalized.starts_with("// #slice-source: ")
        && !normalized.starts_with("//@sm:")
    {
        let orig_nl = memchr::memchr_iter(b'\n', clean.as_bytes()).count();
        let out_nl = memchr::memchr_iter(b'\n', normalized.as_bytes()).count();
        if orig_nl != out_nl {
            let line_map = crate::bundler::source_map::compute_source_line_map(clean, &normalized);
            if !line_map.is_empty() {
                return Ok(Cow::Owned(format!("//@sm:||{}\n{}", line_map, normalized)));
            }
        }
    }
    Ok(Cow::Owned(normalized))
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
pub fn check_syntax_source(source: &str, path_hint: Option<&Path>) -> Result<(), String> {
    let file_label = path_hint
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<eval>".to_string());
    check_syntax_source_detailed(source, path_hint)
        .map_err(|diag| diag.to_labeled_error_string(&file_label))
}
