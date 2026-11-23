//! Opt-In Hygienic Oxc AST Array Pipeline Fusion Pass (`fuse_array_pipelines_ast`).
//!
//! Note: Standard TypeScript/JavaScript compilation (`strip_typescript_raw_with_ext`) delegates
//! higher-order iteration directly to `boa_engine`'s spec-compliant `Array.prototype` and
//! `Iterator.prototype` intrinsics so arbitrary getters, `Proxy` traps, and custom receivers
//! preserve exact ECMA-262 observable semantics. Callers invoking `fuse_array_pipelines_ast`
//! explicitly receive hygienic temporary prefixes, sparse-hole guards, frozen loop bounds,
//! runtime `Array.isArray` + own-method guards, and multi-stage side-effect bailouts.

mod hazards;
mod rewriter;

use crate::linker::slice_with_edits;

fn choose_hygienic_fusion_prefix(code: &str) -> std::borrow::Cow<'static, str> {
    if memchr::memmem::find(code.as_bytes(), b"__sl_").is_none() {
        return std::borrow::Cow::Borrowed("__sl_");
    }
    let mut idx = 0usize;
    loop {
        let candidate = format!("__sl{idx}_");
        if memchr::memmem::find(code.as_bytes(), candidate.as_bytes()).is_none() {
            return std::borrow::Cow::Owned(candidate);
        }
        idx += 1;
    }
}

/// Opt-in Oxc AST Array Pipeline Fusion Pass:
/// Inspects the Oxc AST of `code` and fuses pure higher-order array expressions into a single
/// allocation-free `for` loop IIFE with collision-free temporaries, `Array.isArray` dispatch
/// fallback, initial `.length >>> 0` capture, and sparse-hole (`i in arr`) skipping:
/// - `<arr_expr>.map((<x>) => <map_body>).reduce((<acc>, <curr>) => <red_body>, <init>)`
/// - `<arr_expr>.filter((<x>) => <pred>).map((<y>) => <map_body>).reduce((<acc>, <curr>) => <red_body>, <init>)`
/// - `<arr_expr>.some((<x>) => <pred>)`
/// - `<arr_expr>.every((<x>) => <pred>)`
pub fn fuse_array_pipelines_ast(code: &str) -> String {
    let bytes = code.as_bytes();
    if memchr::memmem::find(bytes, b".reduce(").is_none()
        && memchr::memmem::find(bytes, b".some(").is_none()
        && memchr::memmem::find(bytes, b".every(").is_none()
    {
        return code.to_string();
    }

    let allocator = oxc::allocator::Allocator::default();
    let source_type = oxc::span::SourceType::mjs();
    let parser_ret = oxc::parser::Parser::new(&allocator, code, source_type).parse();
    if !parser_ret.diagnostics.is_empty() {
        return code.to_string();
    }

    let pfx = choose_hygienic_fusion_prefix(code);
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut counter = 0usize;
    for stmt in &parser_ret.program.body {
        rewriter::walk_stmt(stmt, code, &pfx, &mut counter, &mut edits);
    }

    if edits.is_empty() {
        return code.to_string();
    }

    edits.sort_by_key(|e| e.0);
    slice_with_edits(code, 0, code.len(), &edits)
}
