//! Oxc TypeScript & JSX/TSX Ahead-of-Time Compiler Subsystem.
//!
//! Organizes the Oxc-based compilation pipeline into focused submodules:
//! - [`semantics`]: [`TypeDirectedHints`] and [`analyze_typescript_semantics`] type-directed analyzer.
//! - [`pipeline_fusion`]: [`fuse_array_pipelines_ast`] higher-order array pipeline fusion pass.
//! - [`transpiler`]: [`strip_typescript_raw`], [`strip_typescript_raw_with_ext`], and [`strip_typescript_and_normalize`].

pub mod diagnostics;
pub mod pipeline_fusion;
pub mod semantics;
pub mod transpiler;

pub use diagnostics::{
    SyntaxErrorDiagnostic, blank_leading_shebang, byte_offset_to_line_col,
    check_syntax_source_detailed, extract_oxc_syntax_diagnostic,
};
pub use pipeline_fusion::fuse_array_pipelines_ast;
pub use semantics::{TypeDirectedHints, analyze_typescript_semantics, source_contains_jsx};
pub use transpiler::{
    check_syntax_source, compile_ts_or_js_checked, rewrite_cjs_require, strip_js_comments,
    strip_typescript_and_normalize, strip_typescript_and_normalize_with_ext, strip_typescript_raw,
    strip_typescript_raw_with_ext,
};
