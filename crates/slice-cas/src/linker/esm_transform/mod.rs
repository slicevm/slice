//! Oxc AST-Driven ESM-to-CommonJS Transformer (`transform_module_to_cjs`)
//! and Barrel Lazy-Getter Optimizer.
//!
//! Decomposed into focused submodules sharing unified ESM/CJS namespace interop rules:
//! - [`mutable_exports`]: Transitive live-binding static analysis (`get_module_mutable_exports`).
//! - [`ast_lowering`]: Primary Oxc AST ESM-to-CJS module lowering (`try_transform_esm_to_cjs_ast`).
//! - [`barrel_and_fallback`]: CommonJS lazy barrel getter pass and line-oriented fallback.

mod ast_lowering;
mod barrel_and_fallback;
mod mutable_exports;

use crate::linker::{is_builtin_module, is_jsx_runtime_builtin, rewrite_cjs_requires_ast};
use std::borrow::Cow;
use std::path::{Path, PathBuf};

pub(crate) const ESM_ESMODULE_DEFINE: &str =
    "Object.defineProperty(exports, \"__esModule\", { value: true });";

/// Detects whether a lowered CJS module body exports an ES module `default` binding
/// (either as a direct `exports.default = ...` assignment or as a live getter via
/// `Object.defineProperty(exports, "default", ...)`).
#[inline]
pub fn has_esm_default_export_marker(cjs_body: &str) -> bool {
    memchr::memmem::find(cjs_body.as_bytes(), b"exports.default = ").is_some()
        || memchr::memmem::find(
            cjs_body.as_bytes(),
            b"Object.defineProperty(exports, \"default\",",
        )
        .is_some()
}

#[inline]
pub(super) fn choose_hygienic_ident(
    code: &str,
    default_ident: &'static str,
    stem: &str,
    suffix: &str,
) -> Cow<'static, str> {
    if memchr::memmem::find(code.as_bytes(), default_ident.as_bytes()).is_none() {
        return Cow::Borrowed(default_ident);
    }
    let mut idx = 0usize;
    loop {
        let candidate = format!("{stem}{idx}{suffix}");
        if memchr::memmem::find(code.as_bytes(), candidate.as_bytes()).is_none() {
            return Cow::Owned(candidate);
        }
        idx += 1;
    }
}

#[inline]
pub(super) fn choose_hygienic_prefix(
    code: &str,
    default_pfx: &'static str,
    stem: &str,
) -> Cow<'static, str> {
    if memchr::memmem::find(code.as_bytes(), default_pfx.as_bytes()).is_none() {
        return Cow::Borrowed(default_pfx);
    }
    let mut idx = 0usize;
    loop {
        let candidate = format!("{stem}{idx}_");
        if memchr::memmem::find(code.as_bytes(), candidate.as_bytes()).is_none() {
            return Cow::Owned(candidate);
        }
        idx += 1;
    }
}

#[inline]
pub(super) fn select_esm_hygienic_names(
    code: &str,
) -> (
    Cow<'static, str>,
    Cow<'static, str>,
    Cow<'static, str>,
    Cow<'static, str>,
) {
    if memchr::memmem::find(code.as_bytes(), b"__").is_none() {
        return (
            Cow::Borrowed("__slice_default"),
            Cow::Borrowed("__imp_"),
            Cow::Borrowed("__reexp_"),
            Cow::Borrowed("__live_"),
        );
    }
    (
        choose_hygienic_ident(code, "__slice_default", "__slice", "_default"),
        choose_hygienic_prefix(code, "__imp_", "__imp"),
        choose_hygienic_prefix(code, "__reexp_", "__reexp"),
        choose_hygienic_prefix(code, "__live_", "__live"),
    )
}

/// Formats standard ESM-to-CJS default-import unwrapping:
/// - When `tmp` is an ES module namespace (`__esModule === true`), always returns `tmp.default`
///   (even when `tmp.default === undefined`, `0`, `false`, `""`, or `null`).
/// - When `tmp` is a CommonJS `module.exports` value (`!__esModule`), returns `tmp` directly
///   without falsely unwrapping user-owned `{ default: ... }` properties.
#[inline]
pub(super) fn format_default_import_unwrap(tmp: &str) -> String {
    format!("({tmp} && {tmp}.__esModule) ? {tmp}.default : {tmp}")
}

/// Formats namespace import/re-export right-hand sides (`import * as ns`, `export * as ns`):
/// - Built-in Node/Slice modules already expose `__esModule: true` and `default`, so `require(...)`
///   is returned directly.
/// - User/package modules pass through `globalThis.__slice_to_esm(require(...))`, which returns
///   ESM modules in `O(1)` (`__esModule` is non-enumerable) and wraps CJS `module.exports` in a
///   clean namespace object without mutating or clobbering `default`.
#[inline]
pub(super) fn format_namespace_import_rhs(spec: &str, resolved_id: &str) -> String {
    if is_builtin_module(spec) || is_jsx_runtime_builtin(spec) {
        format!("require({resolved_id:?})")
    } else {
        format!("globalThis.__slice_to_esm(require({resolved_id:?}))")
    }
}

pub fn transform_module_to_cjs(
    js_code: &str,
    from_dir: &Path,
    module_path: Option<&Path>,
    deps_out: &mut Vec<PathBuf>,
) -> String {
    if let Some(ast_transformed) =
        ast_lowering::try_transform_esm_to_cjs_ast(js_code, from_dir, module_path, deps_out)
    {
        return ast_transformed;
    }

    if !js_code.contains("import ") && !js_code.contains("export ") {
        let rewritten_cjs = rewrite_cjs_requires_ast(js_code, from_dir, deps_out);
        if !rewritten_cjs.contains("module.exports = {") {
            return rewritten_cjs;
        }
        let body_lines: Vec<String> = rewritten_cjs
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.to_string())
            .collect();
        return barrel_and_fallback::apply_cjs_lazy_barrel_getters(body_lines).join("\n");
    }

    barrel_and_fallback::transform_esm_to_cjs_line_fallback(js_code, from_dir, deps_out)
}
