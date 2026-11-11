//! Zero-Stat Content-Addressable Storage (CAS) and Radix Module Virtualization
//!
//! Eliminates `stat`, `lstat`, and `readdir` system calls during module resolution.
//! Flattens TypeScript/JavaScript/JSX/TSX dependency graphs at build time into a 64-byte
//! cache-line aligned immutable binary archive indexed by BLAKE3-256 hashes and
//! mapped directly into memory via `mmap(MAP_SHARED)`.
//!
//! ## Domain Subsystems
//! - [`archive`]: Binary `.cas` archive builder ([`CasBuilder`]), zero-copy `mmap` reader ([`MappedCasArchive`]), and [`StaticAssetTable`].
//! - [`compiler`]: Oxc AST TypeScript transpiler, type-directed semantic extraction ([`TypeDirectedHints`]), and array pipeline loop fusion.
//! - [`linker`]: Zero-Overhead ESM/CJS AST linker, exported-symbol tree-shaker, and demand-driven lazy `require()` thunks.
//! - [`bundler`]: Node/Deno/TypeScript module specifier resolver, dependency graph bundler, and stack-trace source mapping.

pub mod archive;
pub mod bundler;
pub mod compiler;
pub mod linker;

// Backward-compatible module aliases for internal/external callers
pub use archive::static_assets;
pub use bundler as resolver;
pub use compiler as oxc_compiler;
pub use linker as esm_cjs_linker;

pub use archive::{
    detect_static_mime_type, embed_cas_into_executable, Blake3Hash, CasBlobEntry, CasBuilder,
    CasIndexHeader, MappedCasArchive, StaticAssetEntry, StaticAssetPayload, StaticAssetTable,
    CACHE_LINE_ALIGN, CAS_MAGIC, CAS_VERSION,
};
pub use bundler::{
    bundle_entrypoint_with_deps, map_bundled_error_location,
    map_bundled_error_location_with_offset, map_bundled_stack_trace,
    map_bundled_stack_trace_with_offset, resolve_project_entrypoint,
};
pub use compiler::{
    analyze_typescript_semantics, check_syntax_source, fuse_array_pipelines_ast,
    strip_typescript_and_normalize, strip_typescript_raw, strip_typescript_raw_with_ext,
    TypeDirectedHints,
};
pub use linker::{has_top_level_await, wrap_top_level_await_script};

#[cfg(test)]
pub(crate) use std::path::PathBuf;

#[cfg(test)]
#[path = "../tests/mod.rs"]
mod tests;
