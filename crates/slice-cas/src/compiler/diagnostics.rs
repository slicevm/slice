//! 1-Based `(line, col)` Span Resolution and Oxc Syntax Error Diagnostics (`#139`).

use std::path::Path;

/// Structured 1-based `(line, col)` syntax diagnostic extracted directly from source text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxErrorDiagnostic {
    pub message: String,
    pub line: usize,
    pub col: usize,
}

impl SyntaxErrorDiagnostic {
    /// Formats an uncaught `SyntaxError` matching Slice's runtime error convention.
    #[inline]
    pub fn to_uncaught_error_string(&self) -> String {
        format!(
            "Uncaught SyntaxError: {} at line {}, col {}",
            self.message, self.line, self.col
        )
    }

    /// Formats a file-labeled `SyntaxError` for `slice check` and bundler diagnostics.
    #[inline]
    pub fn to_labeled_error_string(&self, file_label: &str) -> String {
        format!(
            "SyntaxError in {}: {} at line {}, col {}",
            file_label, self.message, self.line, self.col
        )
    }
}

/// Converts a 0-based UTF-8 `byte_offset` in `source` into 1-based `(line, col)` coordinates
/// following ECMAScript `LineTerminator` rules (`\r\n` as a single break, `\n`, `\r`, `\u{2028}`,
/// `\u{2029}`) and Unicode scalar value columns matching `boa_parser`.
pub fn byte_offset_to_line_col(source: &str, byte_offset: usize) -> (usize, usize) {
    let bytes = source.as_bytes();
    let mut offset = byte_offset.min(bytes.len());
    if offset > 0 && offset < bytes.len() && bytes[offset] == b'\n' && bytes[offset - 1] == b'\r' {
        offset -= 1;
    }
    while offset > 0 && !source.is_char_boundary(offset) {
        offset -= 1;
    }

    let mut line = 1usize;
    let mut line_start = 0usize;
    let mut i = 0usize;
    while i < offset {
        let b = bytes[i];
        if b == b'\n' {
            line += 1;
            i += 1;
            line_start = i;
        } else if b == b'\r' {
            line += 1;
            if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                i += 2;
            } else {
                i += 1;
            }
            line_start = i;
        } else if b == 0xE2
            && i + 2 < offset
            && bytes[i + 1] == 0x80
            && (bytes[i + 2] == 0xA8 || bytes[i + 2] == 0xA9)
        {
            line += 1;
            i += 3;
            line_start = i;
        } else {
            i += 1;
        }
    }

    let line_slice = if line_start <= offset {
        &source[line_start..offset]
    } else {
        ""
    };
    let col = if line_slice.is_ascii() {
        line_slice.len() + 1
    } else {
        line_slice.chars().count() + 1
    };
    (line, col)
}

/// Extracts a [`SyntaxErrorDiagnostic`] with 1-based `(line, col)` from an [`oxc::diagnostics::OxcDiagnostic`].
pub fn extract_oxc_syntax_diagnostic(
    source: &str,
    diag: &oxc::diagnostics::OxcDiagnostic,
) -> SyntaxErrorDiagnostic {
    let primary_label = diag
        .labels
        .iter()
        .find(|l| l.primary())
        .or_else(|| diag.labels.last());

    let byte_offset = primary_label
        .map(|l| l.offset() as usize)
        .unwrap_or(source.len());
    let (line, col) = byte_offset_to_line_col(source, byte_offset);

    let mut message = diag.message.to_string();
    if message == "Unexpected token"
        && let Some(lbl) = primary_label
    {
        let start = lbl.offset() as usize;
        let len = lbl.len() as usize;
        let end = start.saturating_add(len);
        if (1..=16).contains(&len)
            && end <= source.len()
            && source.is_char_boundary(start)
            && source.is_char_boundary(end)
        {
            let tok = &source[start..end];
            if !tok.contains(['\n', '\r']) {
                message = format!("Unexpected token '{}'", tok);
            }
        }
    }

    SyntaxErrorDiagnostic { message, line, col }
}

#[inline]
pub(crate) fn default_oxc_parse_options() -> oxc::parser::ParseOptions {
    oxc::parser::ParseOptions {
        allow_return_outside_function: true,
        ..Default::default()
    }
}

/// Parses `clean` as `.tsx` (when `has_jsx_hint`) or `.ts` module, retrying `.tsx` module
/// (when `!has_jsx_hint`) and `.ts` non-module script (`with_module(false)`) if syntax diagnostics
/// occur. Preserves the primary parse diagnostics in the returned `ParserReturn` when all retries fail.
pub(crate) fn parse_ts_or_tsx_with_retries<'a>(
    allocator: &'a oxc::allocator::Allocator,
    clean: &'a str,
    has_jsx_hint: bool,
) -> (
    oxc::parser::ParserReturn<'a>,
    oxc::span::SourceType,
    &'static str,
) {
    let parse_opts = default_oxc_parse_options();
    let active_source_type = if has_jsx_hint {
        oxc::span::SourceType::tsx().with_module(true)
    } else {
        oxc::span::SourceType::ts().with_module(true)
    };
    let active_path = if has_jsx_hint {
        "module.tsx"
    } else {
        "module.ts"
    };
    let parser_ret = oxc::parser::Parser::new(allocator, clean, active_source_type)
        .with_options(parse_opts)
        .parse();
    if parser_ret.diagnostics.is_empty() {
        return (parser_ret, active_source_type, active_path);
    }

    if !has_jsx_hint {
        let tsx_type = oxc::span::SourceType::tsx().with_module(true);
        let retry_ret = oxc::parser::Parser::new(allocator, clean, tsx_type)
            .with_options(parse_opts)
            .parse();
        if retry_ret.diagnostics.is_empty() {
            return (retry_ret, tsx_type, "module.tsx");
        }
    }

    let script_ts = oxc::span::SourceType::ts().with_module(false);
    let retry_script = oxc::parser::Parser::new(allocator, clean, script_ts)
        .with_options(parse_opts)
        .parse();
    if retry_script.diagnostics.is_empty() {
        return (retry_script, script_ts, active_path);
    }

    (parser_ret, active_source_type, active_path)
}

/// Attempts cold sloppy CommonJS (`SourceType::cjs()`) and sloppy script (`SourceType::default()`)
/// fallbacks when TS/ESM module parsing produced diagnostics.
pub(crate) fn try_parse_sloppy_js<'a>(
    allocator: &'a oxc::allocator::Allocator,
    clean: &'a str,
) -> Option<oxc::parser::ParserReturn<'a>> {
    let parse_opts = default_oxc_parse_options();
    for sloppy_js in [
        oxc::span::SourceType::cjs(),
        oxc::span::SourceType::default(),
    ] {
        let retry = oxc::parser::Parser::new(allocator, clean, sloppy_js)
            .with_options(parse_opts)
            .parse();
        if retry.diagnostics.is_empty() {
            return Some(retry);
        }
    }
    None
}

/// Validates JS/TS/JSX/TSX syntax using Oxc's parser and returns structured 1-based `(line, col)`
/// coordinates on failure.
///
/// Cold error-path fallbacks retry `.tsx`, sloppy TypeScript script mode (`with_module(false)`),
/// and CommonJS/sloppy ECMAScript script mode so valid non-module scripts never trigger false
/// syntax errors while valid module files pay only a single parse pass.
pub fn check_syntax_source_detailed(
    source: &str,
    path_hint: Option<&Path>,
) -> Result<(), SyntaxErrorDiagnostic> {
    let clean_cow = blank_leading_shebang(source);
    let clean = clean_cow.as_ref();

    let allocator = oxc::allocator::Allocator::default();
    let is_jsx_ext = path_hint
        .and_then(|p| p.extension())
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("tsx") || ext.eq_ignore_ascii_case("jsx"));
    let (parsed, _, _) = parse_ts_or_tsx_with_retries(&allocator, clean, is_jsx_ext);
    if parsed.diagnostics.is_empty() || try_parse_sloppy_js(&allocator, clean).is_some() {
        return Ok(());
    }

    Err(extract_oxc_syntax_diagnostic(clean, &parsed.diagnostics[0]))
}

/// Replaces a leading `#!...` shebang line with empty characters up to (but keeping) its line
/// terminator so line numbers `1..=N` remain 100% unchanged and `#!` never lands inside a `{ ... }`
/// block wrapper.
#[inline]
pub fn blank_leading_shebang(source: &str) -> std::borrow::Cow<'_, str> {
    if !source.starts_with("#!") {
        return std::borrow::Cow::Borrowed(source);
    }
    match memchr::memchr(b'\n', source.as_bytes()) {
        Some(nl_idx) => {
            if nl_idx > 0 && source.as_bytes()[nl_idx - 1] == b'\r' {
                std::borrow::Cow::Owned(source[nl_idx - 1..].to_string())
            } else {
                std::borrow::Cow::Owned(source[nl_idx..].to_string())
            }
        }
        None => std::borrow::Cow::Borrowed(""),
    }
}
