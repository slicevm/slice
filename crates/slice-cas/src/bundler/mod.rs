//! Module Graph Bundler, `package.json` `"exports"` / `tsconfig.json` `"paths"` Resolver,
//! and Bundled Source-Map Error Location Translator Subsystem.
//!
//! Organizes the bundler and module resolution pipeline into focused submodules:
//! - [`resolver`]: [`resolve_project_entrypoint`], [`resolve_module_specifier`], `package.json` `"exports"`/`"imports"`, and `tsconfig.json` `"paths"`.
//! - [`graph_bundler`]: [`bundle_entrypoint_with_deps`] multi-pass dependency graph bundler and `.wasm`/`.json` module loaders.
//! - [`source_map`]: [`map_bundled_error_location`], [`map_bundled_stack_trace`], and `compute_source_line_map`.

pub mod graph_bundler;
pub mod resolver;
pub mod source_map;

pub use graph_bundler::bundle_entrypoint_with_deps;
pub use resolver::{
    resolve_module_specifier, resolve_project_entrypoint, strip_jsonc, try_file_with_extensions,
    ResolverCacheGuard,
};
pub use source_map::{
    compute_source_line_map, map_bundled_error_location, map_bundled_stack_trace,
    short_source_label,
};
