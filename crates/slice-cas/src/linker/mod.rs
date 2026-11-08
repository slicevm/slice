//! Oxc AST-Driven ESM-to-CommonJS Linker, Dead-Export Tree-Shaker,
//! and Lazy Barrel Optimizer Subsystem.
//!
//! Organizes the linker and optimizer pipeline into focused submodules:
//! - [`builtins_and_ids`]: Built-in module classification (`node:*`, `slice:*`, `bun:*`, `cloudflare:*`, JSX runtimes), canonical module IDs, and specifier resolution.
//! - [`ast_walker`]: Oxc AST statement/expression walkers, binding pattern extractors, `has_top_level_await`, and span edit slicers.
//! - [`cjs_rewrites`]: Oxc AST `require(...)`, dynamic `import(...)`, `import.meta`, and `Buffer.*` fast-path rewriters.
//! - [`esm_transform`]: `transform_module_to_cjs`, `try_transform_esm_to_cjs_ast`, and CJS lazy barrel getter lowering.
//! - [`tree_shaker`]: `UsedExports`, `ModuleRequireSummary`, `analyze_module_used_exports`, and `prune_and_minify_cjs_module_ast`.
//! - [`lazy_and_inline`]: `inline_cold_method_requires_ast` and `collect_eager_module_indices`.

pub mod ast_walker;
pub mod builtins_and_ids;
pub mod cjs_rewrites;
pub mod esm_transform;
pub mod lazy_and_inline;
pub mod tree_shaker;

pub use ast_walker::{
    collect_binding_pattern_names, has_top_level_await, slice_with_edits,
    walk_ast_call_expressions, walk_ast_expressions,
};
pub use builtins_and_ids::{
    canonical_mod_id, clear_canonical_mod_id_cache, format_exports_lhs, is_builtin_module,
    is_jsx_runtime_builtin, is_valid_js_identifier, normalize_builtin_specifier,
    resolve_spec_and_record,
};
pub use cjs_rewrites::{
    collect_require_edits_from_program, rewrite_buffer_fast_paths_ast, rewrite_cjs_requires_ast,
    rewrite_cjs_requires_in_line, rewrite_require_ident_ast,
};
pub use esm_transform::transform_module_to_cjs;
pub use lazy_and_inline::{collect_eager_module_indices, inline_cold_method_requires_ast};
pub use tree_shaker::{
    analyze_module_used_exports, analyze_module_used_exports_from_summaries,
    extract_module_require_summary, extract_referenced_identifiers,
    extract_referenced_identifiers_with_offsets, is_js_ident_continue, is_js_ident_start,
    is_js_keyword, prune_and_minify_cjs_module_ast, ModuleRequireSummary, UsedExports,
};
