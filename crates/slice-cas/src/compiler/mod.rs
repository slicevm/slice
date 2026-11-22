//! Oxc TypeScript & JSX/TSX Ahead-of-Time Compiler Subsystem.
//!
//! Organizes the Oxc-based compilation pipeline into focused submodules:
//! - [`semantics`]: [`TypeDirectedHints`] and [`analyze_typescript_semantics`] type-directed analyzer.
//! - [`pipeline_fusion`]: [`fuse_array_pipelines_ast`] higher-order array pipeline fusion pass.
//! - [`transpiler`]: [`strip_typescript_raw`], [`strip_typescript_raw_with_ext`], and [`strip_typescript_and_normalize`].

pub mod comments;
pub mod diagnostics;
pub mod pipeline_fusion;
pub mod semantics;
pub mod transpiler;
pub mod ts_detector;
pub mod ts_helpers;

pub use comments::strip_js_comments;
pub use diagnostics::{
    SyntaxErrorDiagnostic, blank_leading_shebang, byte_offset_to_line_col,
    check_syntax_source_detailed, extract_oxc_syntax_diagnostic,
};
pub use pipeline_fusion::fuse_array_pipelines_ast;
pub use semantics::{TypeDirectedHints, analyze_typescript_semantics, source_contains_jsx};
pub(crate) use transpiler::strip_typescript_checked_for_bundler;
pub use transpiler::{
    check_syntax_source, compile_ts_or_js_checked, compile_ts_or_js_checked_with_ast_hook,
    lower_unbundled_esm, rewrite_cjs_require, strip_typescript_and_normalize,
    strip_typescript_and_normalize_with_ext, strip_typescript_raw, strip_typescript_raw_with_ext,
};
pub use ts_detector::program_has_ts_or_jsx;
pub use ts_helpers::needs_unbundled_esm_lowering;
