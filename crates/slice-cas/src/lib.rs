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

#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

pub mod archive;
#[cfg(feature = "compiler")]
pub mod bundler;
#[cfg(feature = "compiler")]
pub mod compiler;
#[cfg(feature = "compiler")]
pub mod linker;

#[cfg(feature = "compiler")]
pub use archive::CasBuilder;
pub use archive::{
    CACHE_LINE_ALIGN, CAS_MAGIC, CAS_VERSION, CasBacking, CasBlobEntry, CasIndexHeader,
    MappedCasArchive, MappedCasInner, StaticAssetEntry, StaticAssetPayload, StaticAssetTable,
    detect_static_mime_type, embed_cas_into_executable, inspect_sliceexe_trailer,
};
#[cfg(feature = "compiler")]
pub use bundler::{
    bundle_entrypoint_with_deps, compact_js_for_boa_bytecode, extract_slice_rc_key,
    map_bundled_error_location, map_bundled_error_location_with_offset, map_bundled_stack_trace,
    map_bundled_stack_trace_with_offset, resolve_project_entrypoint,
};
#[cfg(feature = "compiler")]
pub use compiler::{
    SyntaxErrorDiagnostic, TypeDirectedHints, analyze_typescript_semantics,
    byte_offset_to_line_col, check_syntax_source, check_syntax_source_detailed,
    compile_ts_or_js_checked, compile_ts_or_js_checked_with_ast_hook, fuse_array_pipelines_ast,
    lower_unbundled_esm, needs_unbundled_esm_lowering, program_has_ts_or_jsx,
    strip_typescript_and_normalize, strip_typescript_and_normalize_with_ext, strip_typescript_raw,
    strip_typescript_raw_with_ext,
};
#[cfg(feature = "compiler")]
pub use linker::{
    has_top_level_await, is_builtin_module, is_jsx_runtime_builtin, program_has_top_level_return,
    wrap_top_level_await_script, wrap_top_level_return_script,
};

#[cfg(test)]
pub(crate) use std::path::PathBuf;

#[cfg(test)]
#[path = "../tests/mod.rs"]
mod tests;
