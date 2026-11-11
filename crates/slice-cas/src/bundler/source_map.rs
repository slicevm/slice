//! Bundled Source-Map Line Mapper and Stack Trace Translator (`map_bundled_error_location`, `map_bundled_stack_trace`).

use std::path::Path;

#[inline]
fn push_usize_csv(out: &mut String, n: usize) {
    if !out.is_empty() {
        out.push(',');
    }
    let mut buf = itoa::Buffer::new();
    out.push_str(buf.format(n));
}

pub fn compute_source_line_map(raw_src: &str, cjs_body: &str) -> String {
    let orig_lines: Vec<&str> = raw_src.lines().collect();
    if orig_lines.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity((cjs_body.len() / 8).max(32));
    let mut search_cursor = 0usize;

    for raw_cjs_line in cjs_body.lines() {
        let trimmed = raw_cjs_line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        let probe_raw = if let Some(rest) = trimmed.strip_prefix("const __slice_default = ") {
            rest.trim_end_matches(';').trim()
        } else {
            trimmed.trim_end_matches(';').trim()
        };

        if probe_raw.starts_with("exports.")
            || probe_raw.starts_with("module.exports = __slice_default")
            || probe_raw.starts_with("var __filename =")
        {
            let fallback_line = (search_cursor + 1).min(orig_lines.len()).max(1);
            push_usize_csv(&mut out, fallback_line);
            continue;
        }

        let rhs_probe = probe_raw
            .split_once(" = ")
            .map(|(_, r)| r.trim())
            .filter(|r| r.len() >= 4);

        let mut found_idx: Option<usize> = None;
        for (offset, orig_line) in orig_lines[search_cursor..].iter().enumerate() {
            if orig_line.contains(probe_raw)
                || rhs_probe.is_some_and(|rp| !rp.starts_with("require(") && orig_line.contains(rp))
            {
                found_idx = Some(search_cursor + offset);
                break;
            }
        }
        if let Some(idx) = found_idx {
            search_cursor = idx;
            push_usize_csv(&mut out, idx + 1);
        } else if probe_raw.len() >= 12 && probe_raw.bytes().any(|b| b.is_ascii_alphabetic()) {
            let mut fallback_idx = None;
            for (idx, orig_line) in orig_lines[..search_cursor].iter().enumerate() {
                if orig_line.contains(probe_raw)
                    || rhs_probe
                        .is_some_and(|rp| !rp.starts_with("require(") && orig_line.contains(rp))
                {
                    fallback_idx = Some(idx);
                    break;
                }
            }
            if let Some(idx) = fallback_idx {
                push_usize_csv(&mut out, idx + 1);
            } else {
                let next_guess = (search_cursor + 1).min(orig_lines.len()).max(1);
                push_usize_csv(&mut out, next_guess);
            }
        } else {
            let next_guess = (search_cursor + 1).min(orig_lines.len()).max(1);
            push_usize_csv(&mut out, next_guess);
        }
    }
    out
}

pub fn short_source_label(path: &Path) -> String {
    let fname = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("module.ts");
    if let Some(parent_name) = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
    {
        if matches!(parent_name, "src" | "tests" | "test" | "lib" | "app") {
            return format!("{}/{}", parent_name, fname);
        }
    }
    fname.to_string()
}

fn parse_slice_source_marker(rest: &str) -> (&str, &str, &str) {
    let mut parts = rest.splitn(3, '|');
    let short = parts.next().unwrap_or(rest).trim();
    let full = parts.next().unwrap_or(short).trim();
    let line_map = parts.next().unwrap_or("").trim();
    (short, full, line_map)
}

fn lookup_embedded_line_map(line_map: &str, rel_idx_0: usize) -> Option<usize> {
    if line_map.is_empty() {
        return None;
    }
    line_map
        .split(',')
        .nth(rel_idx_0)
        .and_then(|s| s.trim().parse::<usize>().ok())
}

/// Maps an uncaught JS error or bundled line reference back to its original TypeScript/JavaScript
/// source file and 1-based line number (`<short_file>:<line>`) using `// #slice-source:` markers.
pub fn map_bundled_error_location(bundled_js: &str, error_msg: &str) -> String {
    map_bundled_error_location_with_offset(bundled_js, error_msg, 0)
}

/// Maps an uncaught JS error or multi-frame stack trace back to original TypeScript/JavaScript
/// coordinates, adjusting raw runtime line numbers by `line_offset` when executing fused containers.
pub fn map_bundled_error_location_with_offset(
    bundled_js: &str,
    error_msg: &str,
    line_offset: usize,
) -> String {
    if error_msg.contains("\n    at ") || error_msg.contains("\tat ") {
        let stack_mapped = map_bundled_stack_trace_with_offset(bundled_js, error_msg, line_offset);
        if stack_mapped != error_msg || !error_msg.contains("unknown at :") {
            return stack_mapped;
        }
    }

    let lines: Vec<&str> = bundled_js.lines().collect();

    let mut explicit_line: Option<usize> = None;
    if let Some(idx) = error_msg.find("line ") {
        let digits: String = error_msg[idx + 5..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(n) = digits.parse::<usize>() {
            explicit_line = n.checked_sub(line_offset).filter(|&l| l >= 1);
        }
    }

    if let Some(b_line) = explicit_line {
        let mut active_short = "";
        let mut active_full = "";
        let mut active_line_map = "";
        let mut marker_line_idx = 0usize;
        for (idx, line) in lines.iter().enumerate() {
            let line_1 = idx + 1;
            if line_1 > b_line {
                break;
            }
            let trimmed = line.trim();
            if let Some(rest) = trimmed
                .strip_prefix("// #slice-source: ")
                .or_else(|| trimmed.strip_prefix("//@sm:"))
            {
                let (s, f, lm) = parse_slice_source_marker(rest);
                active_short = s;
                active_full = f;
                active_line_map = lm;
                marker_line_idx = line_1;
            }
        }
        if !active_short.is_empty() && b_line > marker_line_idx {
            let rel_idx_0 = count_code_lines_between(&lines, marker_line_idx, b_line);
            let resolved_line =
                if let Some(mapped) = lookup_embedded_line_map(active_line_map, rel_idx_0) {
                    mapped
                } else {
                    let mut guess = rel_idx_0 + 1;
                    if let Some(stmt_text) = lines.get(b_line - 1).map(|s| s.trim()) {
                        if !stmt_text.is_empty() {
                            if let Ok(orig_src) = std::fs::read_to_string(active_full) {
                                if let Some((orig_idx, _)) = orig_src
                                    .lines()
                                    .enumerate()
                                    .find(|(_, ol)| ol.contains(stmt_text))
                                {
                                    guess = orig_idx + 1;
                                }
                            }
                        }
                    }
                    guess
                };
            return format!("{} ({}:{})", error_msg, active_short, resolved_line);
        }
    }

    let core_msg = error_msg
        .trim()
        .trim_start_matches("Uncaught ")
        .trim_start_matches("Error: ")
        .trim_start_matches("TypeError: ")
        .trim_start_matches("RangeError: ")
        .trim_matches('"')
        .trim();

    if !core_msg.is_empty() {
        let prefix_probe = core_msg
            .split_once(':')
            .map(|(p, _)| p.trim())
            .filter(|p| p.len() >= 6);
        let mut active_short = "";
        let mut active_full = "";
        let mut active_line_map = "";
        let mut marker_line_idx = 0usize;
        for (idx, line) in lines.iter().enumerate() {
            let line_1 = idx + 1;
            let trimmed = line.trim();
            if let Some(rest) = trimmed
                .strip_prefix("// #slice-source: ")
                .or_else(|| trimmed.strip_prefix("//@sm:"))
            {
                let (s, f, lm) = parse_slice_source_marker(rest);
                active_short = s;
                active_full = f;
                active_line_map = lm;
                marker_line_idx = line_1;
                continue;
            }
            let matches_line = !active_short.is_empty()
                && (line.contains(core_msg)
                    || prefix_probe.is_some_and(|pp| {
                        (line.contains("throw ") || line.contains("Error(")) && line.contains(pp)
                    }));
            if matches_line {
                let rel_idx_0 = count_code_lines_between(&lines, marker_line_idx, line_1);
                let mut resolved_line =
                    lookup_embedded_line_map(active_line_map, rel_idx_0).unwrap_or(rel_idx_0 + 1);
                if let Ok(orig_src) = std::fs::read_to_string(active_full) {
                    if let Some((orig_idx, _)) = orig_src.lines().enumerate().find(|(_, ol)| {
                        ol.contains(core_msg)
                            || prefix_probe.is_some_and(|pp| {
                                (ol.contains("throw ") || ol.contains("Error(")) && ol.contains(pp)
                            })
                    }) {
                        resolved_line = orig_idx + 1;
                    }
                }
                return format!("{} ({}:{})", error_msg, active_short, resolved_line);
            }
        }
    }

    error_msg.to_string()
}

#[derive(Debug, Clone)]
struct BundledSourceSection<'a> {
    short_label: &'a str,
    full_path: &'a str,
    line_map: &'a str,
    marker_line_1: usize,
    end_line_1: usize,
}

fn count_code_lines_between(lines: &[&str], start_line_1: usize, target_line_1: usize) -> usize {
    if target_line_1 <= start_line_1 {
        return 0;
    }
    let start_idx = start_line_1;
    let end_idx = (target_line_1 - 1).min(lines.len());
    if start_idx >= end_idx {
        return 0;
    }
    lines[start_idx..end_idx]
        .iter()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with("//")
        })
        .count()
}

fn resolve_section_line(
    lines: &[&str],
    sec: &BundledSourceSection<'_>,
    target_line_1: usize,
) -> usize {
    let rel_idx_0 = count_code_lines_between(lines, sec.marker_line_1, target_line_1);
    if let Some(mapped) = lookup_embedded_line_map(sec.line_map, rel_idx_0) {
        return mapped;
    }
    let mut resolved_line = rel_idx_0 + 1;
    if let Some(stmt_text) = lines.get(target_line_1.saturating_sub(1)).map(|s| s.trim()) {
        let probe = stmt_text
            .strip_prefix("exports.")
            .and_then(|r| r.split_once(" = ").map(|(_, rhs)| rhs.trim()))
            .unwrap_or(stmt_text)
            .trim_end_matches(';')
            .trim();
        if !probe.is_empty() && probe.len() >= 4 {
            if let Ok(orig_src) = std::fs::read_to_string(sec.full_path) {
                if let Some((orig_idx, _)) = orig_src
                    .lines()
                    .enumerate()
                    .find(|(_, ol)| ol.contains(probe))
                {
                    resolved_line = orig_idx + 1;
                }
            }
        }
    }
    resolved_line
}

#[derive(Debug, Clone, Copy)]
struct ParsedRawStackFrame<'a> {
    raw_fn_name: Option<&'a str>,
    fn_name: Option<&'a str>,
    loc_str: &'a str,
    line_col: Option<(usize, usize)>,
}

fn parse_raw_stack_frame(trimmed: &str) -> Option<ParsedRawStackFrame<'_>> {
    let after_at = trimmed.strip_prefix("at ")?.trim();
    let (raw_fn_name, fn_name, loc_str) = if let Some((fn_part, rest)) = after_at.split_once(" (") {
        let fn_clean = fn_part.trim();
        let raw_opt = if fn_clean.is_empty() {
            None
        } else {
            Some(fn_clean)
        };
        let fn_opt = if fn_clean.is_empty()
            || fn_clean == "unknown"
            || fn_clean == "<main>"
            || fn_clean == "<anonymous>"
            || fn_clean == "__run"
            || fn_clean.starts_with("__slice")
        {
            None
        } else {
            Some(fn_clean)
        };
        (raw_opt, fn_opt, rest.trim_end_matches(')').trim())
    } else {
        (None, None, after_at)
    };

    let mut line_col = None;
    let mut rsplit = loc_str.rsplitn(3, ':');
    if let (Some(col_s), Some(line_s)) = (rsplit.next(), rsplit.next()) {
        if let (Ok(l), Ok(c)) = (
            line_s.trim().parse::<usize>(),
            col_s.trim().parse::<usize>(),
        ) {
            line_col = Some((l, c));
        }
    }
    Some(ParsedRawStackFrame {
        raw_fn_name,
        fn_name,
        loc_str,
        line_col,
    })
}

#[inline]
fn is_bundler_scaffolding_line(line: &str) -> bool {
    let t = line.trim();
    t == "__run();"
        || t.starts_with("var __slice_modules =")
        || t.starts_with("__slice_modules =")
        || t.starts_with("__slice_modules[")
        || t.starts_with("var __slice_cache =")
        || t.starts_with("function __slice_require(")
        || t.starts_with("fn(m.exports, __slice_require, m)")
        || t.starts_with("var __rc = globalThis.__slice_rc")
        || t.starts_with("var __run = __rc[")
        || t.starts_with("__run = __rc[")
        || t == "if (!__run) {"
        || t == "var __h = null;"
        || t == "if (!__h) {"
        || t == "var require = __slice_require, exports = {}, module = { exports: exports };"
        || t == "var exports = {}, module = { exports: exports };"
        || t == "__h = exports.default || module.exports;"
        || t.starts_with("if (__h) globalThis.__slice_dispatch_fetch(")
        || t.starts_with("if (__h && globalThis.__slice_active_http_server")
        || t.starts_with("if (globalThis.__slice_active_http_server) __h =")
}

#[inline]
fn is_closing_wrapper_brace(line: &str) -> bool {
    matches!(line.trim(), "}" | "};" | "});")
}

fn strip_header_unknown_location(header: &str) -> &str {
    for marker in [" (unknown at :", " (<anonymous>:"] {
        if let Some(open_idx) = header.rfind(marker) {
            let tail = &header[open_idx + marker.len()..];
            if let Some(body) = tail.strip_suffix(')') {
                if !body.is_empty() && body.bytes().all(|b| b.is_ascii_digit() || b == b':') {
                    return &header[..open_idx];
                }
            }
        }
    }
    header
}

#[inline]
fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

fn take_first_ident(s: &str) -> Option<&str> {
    let trimmed = s.trim_start();
    let len = trimmed.bytes().take_while(|&b| is_ident_char(b)).count();
    if len > 0 && !trimmed.as_bytes()[0].is_ascii_digit() {
        Some(&trimmed[..len])
    } else {
        None
    }
}

fn take_trailing_lhs_name(lhs: &str) -> Option<&str> {
    let trimmed = lhs.trim();
    let end = trimmed.len();
    let start = trimmed
        .bytes()
        .rev()
        .take_while(|&b| is_ident_char(b) || b == b'.')
        .count();
    if start == 0 {
        return None;
    }
    let cand = trimmed[end - start..].trim_matches('.');
    if cand.is_empty() || cand.as_bytes()[0].is_ascii_digit() {
        return None;
    }
    if let Some(stripped) = cand.strip_prefix("exports.") {
        if !stripped.is_empty() {
            return Some(stripped);
        }
    }
    if let Some(stripped) = cand.strip_prefix("module.exports.") {
        if !stripped.is_empty() {
            return Some(stripped);
        }
    }
    if let Some((cls, method)) = cand.split_once(".prototype.") {
        if !cls.is_empty() && !method.is_empty() {
            return Some(cand);
        }
    }
    Some(cand)
}

#[derive(Debug, Clone)]
enum LexicalScope {
    Block,
    Class(String),
    Callable(String),
    AnonymousCallable,
}

fn classify_brace_header(header: &str, enclosing_class: Option<&str>) -> LexicalScope {
    let h = header.trim();
    if h.is_empty() {
        return LexicalScope::Block;
    }

    // 1. Class declaration or assigned class expression (`class Ledger`, `export class Ledger`, `const L = class`)
    if let Some(after_cls) = h
        .strip_prefix("class ")
        .or_else(|| h.strip_prefix("export class "))
        .or_else(|| h.strip_prefix("export default class "))
    {
        if let Some(cls_name) = take_first_ident(after_cls) {
            if cls_name != "extends" {
                return LexicalScope::Class(cls_name.to_string());
            }
        }
        return LexicalScope::Class("<anonymous>".to_string());
    }
    if let Some((lhs, rhs)) = h.rsplit_once('=') {
        let rhs_t = rhs.trim();
        if !lhs.ends_with('=')
            && !lhs.ends_with('!')
            && !lhs.ends_with('<')
            && !lhs.ends_with('>')
            && (rhs_t == "class" || rhs_t.starts_with("class "))
        {
            let explicit = rhs_t
                .strip_prefix("class")
                .and_then(take_first_ident)
                .filter(|&id| id != "extends");
            if let Some(c) = explicit.or_else(|| take_trailing_lhs_name(lhs)) {
                return LexicalScope::Class(c.to_string());
            }
        }
    }

    // 2. Control-flow blocks and object literals (`if (...)`, `try`, `catch (e)`, `const x =`, `foo(`)
    if let Some(kw) = take_first_ident(h) {
        if matches!(
            kw,
            "if" | "else"
                | "for"
                | "while"
                | "do"
                | "switch"
                | "try"
                | "catch"
                | "finally"
                | "with"
                | "return"
                | "throw"
        ) {
            return LexicalScope::Block;
        }
    }
    if h.ends_with('=')
        || h.ends_with('(')
        || h.ends_with(',')
        || h.ends_with(':')
        || h.ends_with('[')
    {
        return LexicalScope::Block;
    }

    // 3. Arrow functions (`(...) =>`)
    if let Some(before_arrow) = h.strip_suffix("=>") {
        if let Some((lhs, _params)) = before_arrow.rsplit_once('=') {
            let lhs_trim = lhs.trim_end();
            if !lhs_trim.ends_with('=')
                && !lhs_trim.ends_with('!')
                && !lhs_trim.ends_with('<')
                && !lhs_trim.ends_with('>')
            {
                if let Some(name) = take_trailing_lhs_name(lhs_trim) {
                    return LexicalScope::Callable(name.replace(".prototype.", "."));
                }
            }
        }
        return LexicalScope::AnonymousCallable;
    }

    // 4. `function` declarations and expressions
    if let Some(fn_pos) = h.rfind("function") {
        let before_ok = fn_pos == 0 || !is_ident_char(h.as_bytes()[fn_pos - 1]);
        let after_fn = &h[fn_pos + 8..];
        let after_ok = after_fn.is_empty()
            || after_fn.starts_with(' ')
            || after_fn.starts_with('*')
            || after_fn.starts_with('(');
        if before_ok && after_ok {
            let after_star = after_fn.trim_start().trim_start_matches('*').trim_start();
            if let Some(explicit_fn) = take_first_ident(after_star) {
                return LexicalScope::Callable(explicit_fn.to_string());
            }
            let prefix = h[..fn_pos].trim_end().trim_end_matches("async").trim_end();
            if let Some(lhs) = prefix.strip_suffix('=') {
                if let Some(name) = take_trailing_lhs_name(lhs) {
                    return LexicalScope::Callable(name.replace(".prototype.", "."));
                }
            }
            return LexicalScope::AnonymousCallable;
        }
    }

    // 5. Concise ES6 class methods and object methods (`post(o)`, `async post(o)`, `static create()`, `get x()`)
    if h.ends_with(')') {
        let mut rest = h;
        loop {
            let next = rest
                .strip_prefix("public ")
                .or_else(|| rest.strip_prefix("private "))
                .or_else(|| rest.strip_prefix("protected "))
                .or_else(|| rest.strip_prefix("readonly "))
                .or_else(|| rest.strip_prefix("override "))
                .or_else(|| rest.strip_prefix("static "))
                .or_else(|| rest.strip_prefix("async "))
                .or_else(|| rest.strip_prefix("get "))
                .or_else(|| rest.strip_prefix("set "))
                .map(str::trim_start);
            match next {
                Some(n) => rest = n,
                None => break,
            }
        }
        rest = rest.trim_start_matches('*').trim_start();
        if let Some(method_name) = take_first_ident(rest) {
            let after_name = rest[method_name.len()..].trim_start();
            if after_name.starts_with('(') {
                if let Some(cls) = enclosing_class {
                    if cls != "<anonymous>" {
                        if method_name == "constructor" {
                            return LexicalScope::Callable(format!("new {}", cls));
                        }
                        return LexicalScope::Callable(format!("{}.{}", cls, method_name));
                    }
                }
                return LexicalScope::Callable(method_name.to_string());
            }
        }
    }

    LexicalScope::Block
}

fn recover_enclosing_symbol_name(
    lines: &[&str],
    sec: &BundledSourceSection<'_>,
    target_line_1: usize,
    target_col_1: usize,
) -> Option<String> {
    let end_l = target_line_1.min(sec.end_line_1).min(lines.len());
    if end_l <= sec.marker_line_1 {
        return None;
    }

    let mut scope_stack: Vec<LexicalScope> = Vec::with_capacity(8);
    let mut in_str: Option<u8> = None;

    for l_1 in (sec.marker_line_1 + 1)..=end_l {
        let line = lines[l_1 - 1];
        let bytes = line.as_bytes();
        let is_target_line = l_1 == target_line_1;
        let mut seg_start = 0usize;
        let mut i = 0usize;
        let mut opened_callable_on_target = false;

        while i < bytes.len() {
            if is_target_line && target_col_1 > 1 && i >= target_col_1 {
                break;
            }
            let b = bytes[i];
            if let Some(q) = in_str {
                if b == b'\\' {
                    i += 2;
                    continue;
                }
                if b == q {
                    in_str = None;
                }
                i += 1;
                continue;
            }
            if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                break;
            }
            if b == b'\'' || b == b'"' || b == b'`' {
                in_str = Some(b);
                i += 1;
                continue;
            }
            if b == b';' {
                seg_start = i + 1;
            } else if b == b'{' {
                let header = if seg_start <= i {
                    &line[seg_start..i]
                } else {
                    ""
                };
                let direct_class = match scope_stack.last() {
                    Some(LexicalScope::Class(cls)) => Some(cls.as_str()),
                    _ => None,
                };
                let scope = classify_brace_header(header, direct_class);
                if is_target_line
                    && matches!(
                        scope,
                        LexicalScope::Callable(_) | LexicalScope::AnonymousCallable
                    )
                {
                    opened_callable_on_target = true;
                }
                scope_stack.push(scope);
                seg_start = i + 1;
            } else if b == b'}' {
                if is_target_line && target_col_1 <= 1 && opened_callable_on_target {
                    break;
                }
                scope_stack.pop();
                seg_start = i + 1;
            }
            i += 1;
        }
    }

    for scope in scope_stack.iter().rev() {
        match scope {
            LexicalScope::Callable(name) => return Some(name.clone()),
            LexicalScope::AnonymousCallable => return None,
            LexicalScope::Block | LexicalScope::Class(_) => {}
        }
    }
    None
}

/// Translates a multi-frame ECMAScript `Error.prototype.stack` string against embedded
/// `// #slice-source: <short_label>|<full_path>|<line_map>` (or compacted `//@sm:`) markers inside `bundled_js`,
/// mapping each stack frame back to its original TypeScript/JavaScript `<short_label>:<line>:<col>`.
pub fn map_bundled_stack_trace(bundled_js: &str, raw_stack: &str) -> String {
    map_bundled_stack_trace_with_offset(bundled_js, raw_stack, 0)
}

/// Translates a multi-frame ECMAScript stack trace against `bundled_js`, subtracting `line_offset`
/// from raw runtime bundle line numbers when executing pre-fused bytecode containers.
pub fn map_bundled_stack_trace_with_offset(
    bundled_js: &str,
    raw_stack: &str,
    line_offset: usize,
) -> String {
    if (!bundled_js.contains("// #slice-source: ") && !bundled_js.contains("//@sm:"))
        || raw_stack.trim().is_empty()
    {
        return raw_stack.to_string();
    }

    let lines: Vec<&str> = bundled_js.lines().collect();
    let mut sections: Vec<BundledSourceSection<'_>> = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let line_1 = idx + 1;
        let trimmed = line.trim();
        if let Some(rest) = trimmed
            .strip_prefix("// #slice-source: ")
            .or_else(|| trimmed.strip_prefix("//@sm:"))
        {
            if let Some(prev) = sections.last_mut() {
                prev.end_line_1 = line_1.saturating_sub(1);
            }
            let (short_label, full_path, line_map) = parse_slice_source_marker(rest);
            sections.push(BundledSourceSection {
                short_label,
                full_path,
                line_map,
                marker_line_1: line_1,
                end_line_1: lines.len(),
            });
        }
    }

    if sections.is_empty() {
        return raw_stack.to_string();
    }

    // Clamp each section's `end_line_1` to its exact `line_map` code-line count and stop before
    // any bundler wrapper scaffolding lines (`__run()`, module factory `};`, etc.).
    for sec in &mut sections {
        let expected_code_lines = if sec.line_map.is_empty() {
            usize::MAX
        } else {
            sec.line_map.bytes().filter(|&b| b == b',').count() + 1
        };
        let max_l = sec.end_line_1.min(lines.len());
        let mut seen_code = 0usize;
        let mut clamped_end = sec.marker_line_1;
        for l_1 in (sec.marker_line_1 + 1)..=max_l {
            let raw_l = lines[l_1 - 1];
            let t = raw_l.trim();
            if is_bundler_scaffolding_line(t) {
                break;
            }
            if !t.is_empty() && !t.starts_with("//") {
                seen_code += 1;
                clamped_end = l_1;
                if seen_code >= expected_code_lines {
                    break;
                }
            } else {
                clamped_end = l_1;
            }
        }
        if clamped_end >= sec.marker_line_1 {
            sec.end_line_1 = clamped_end;
        }
    }

    let raw_lines: Vec<&str> = raw_stack.lines().collect();
    let parsed_frames: Vec<Option<ParsedRawStackFrame<'_>>> = raw_lines
        .iter()
        .map(|l| parse_raw_stack_frame(l.trim()))
        .collect();
    let has_any_frame = parsed_frames.iter().any(Option::is_some);
    let last_section_end = sections.last().map(|s| s.end_line_1).unwrap_or(0);

    let mut out = String::with_capacity(raw_stack.len() + 128);
    let mut frame_ordinal = 0usize;
    let mut prev_callee_fn: Option<String> = None;

    for (line_idx, &raw_line) in raw_lines.iter().enumerate() {
        let Some(frame) = parsed_frames[line_idx] else {
            if !out.is_empty() {
                out.push('\n');
            }
            if line_idx == 0 && has_any_frame {
                out.push_str(strip_header_unknown_location(raw_line));
            } else {
                out.push_str(raw_line);
            }
            continue;
        };

        // Omit internal Slice bundler/loader wrapper frames (`__run`, `__slice_require`, etc.).
        if let Some(raw_fn) = frame.raw_fn_name {
            if raw_fn == "__run" || raw_fn.starts_with("__slice_") {
                continue;
            }
        }

        let is_boa_bundle_loc = frame.loc_str.starts_with("unknown at :");
        let is_unmapped_loc = is_boa_bundle_loc
            || frame.loc_str.starts_with("<anonymous>:")
            || frame.loc_str == "unknown";

        // Idempotence: preserve frames that have already been mapped to original source files.
        if !is_unmapped_loc {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(raw_line);
            prev_callee_fn = frame.fn_name.map(str::to_string);
            frame_ordinal += 1;
            continue;
        }

        let col = frame.line_col.map(|(_, c)| c).unwrap_or(1);
        let adj_line_opt = frame
            .line_col
            .and_then(|(l, _)| l.checked_sub(line_offset))
            .filter(|&l| l >= 1);

        let mut resolved: Option<(&str, usize)> = None;
        let mut display_fn: Option<String> = frame.fn_name.map(str::to_string);

        // Primary exact path: Boa bundle line coordinates (`unknown at :<line>:<col>`).
        if is_boa_bundle_loc {
            if let Some(adj_line) = adj_line_opt {
                if let Some(sec) = sections
                    .iter()
                    .rev()
                    .find(|s| adj_line > s.marker_line_1 && adj_line <= s.end_line_1)
                {
                    let orig_line = resolve_section_line(&lines, sec, adj_line);
                    resolved = Some((sec.short_label, orig_line));
                    match &display_fn {
                        None => {
                            display_fn = recover_enclosing_symbol_name(&lines, sec, adj_line, col);
                        }
                        Some(existing) if !existing.contains('.') => {
                            if let Some(enc) =
                                recover_enclosing_symbol_name(&lines, sec, adj_line, col)
                            {
                                if enc.ends_with(&format!(".{}", existing)) {
                                    display_fn = Some(enc);
                                }
                            }
                        }
                        _ => {}
                    }
                } else if display_fn.is_none() {
                    let is_wrapper = adj_line > last_section_end
                        || lines.get(adj_line - 1).is_some_and(|l| {
                            is_bundler_scaffolding_line(l) || is_closing_wrapper_brace(l)
                        });
                    if is_wrapper {
                        continue;
                    }
                }
            }
        }

        // Secondary fallback path for `<anonymous>:...` / dynamic `Function` / unindexed frames.
        if resolved.is_none() {
            if let Some(fn_name) = frame.fn_name {
                let bare_fn = fn_name.rsplit_once('.').map(|(_, m)| m).unwrap_or(fn_name);
                let fn_pat_1 = format!("function {}(", bare_fn);
                let fn_pat_2 = format!("function {} (", bare_fn);
                let fn_pat_3 = format!("{} = function", bare_fn);
                let method_pat_1 = format!("{}(", bare_fn);
                let method_pat_2 = format!("async {}(", bare_fn);
                let method_pat_3 = format!("static {}(", bare_fn);

                for sec in &sections {
                    let mut decl_line_1 = None;
                    for l_1 in (sec.marker_line_1 + 1)..=sec.end_line_1.min(lines.len()) {
                        let text = lines[l_1 - 1];
                        let trimmed_l = text.trim();
                        let is_method_decl = (trimmed_l.starts_with(&method_pat_1)
                            || trimmed_l.starts_with(&method_pat_2)
                            || trimmed_l.starts_with(&method_pat_3))
                            && trimmed_l.ends_with('{');
                        if text.contains(&fn_pat_1)
                            || text.contains(&fn_pat_2)
                            || text.contains(&fn_pat_3)
                            || is_method_decl
                        {
                            decl_line_1 = Some(l_1);
                            break;
                        }
                    }
                    if let Some(d_line_1) = decl_line_1 {
                        let mut fn_end_1 = sec.end_line_1.min(lines.len());
                        let mut brace_depth = 0i32;
                        let mut seen_open = false;
                        for l_1 in d_line_1..=sec.end_line_1.min(lines.len()) {
                            for ch in lines[l_1 - 1].chars() {
                                if ch == '{' {
                                    brace_depth += 1;
                                    seen_open = true;
                                } else if ch == '}' {
                                    brace_depth -= 1;
                                }
                            }
                            if seen_open && brace_depth <= 0 {
                                fn_end_1 = l_1;
                                break;
                            }
                        }

                        let mut target_line_1 = d_line_1;
                        if let Some(adj_line) = adj_line_opt {
                            if adj_line >= d_line_1 && adj_line <= fn_end_1 {
                                target_line_1 = adj_line;
                            }
                        }
                        if target_line_1 == d_line_1 {
                            if frame_ordinal == 0 {
                                for l_1 in d_line_1..=fn_end_1 {
                                    let t = lines[l_1 - 1];
                                    if t.contains("throw ") || t.contains("Error(") {
                                        target_line_1 = l_1;
                                        break;
                                    }
                                }
                            } else if let Some(ref callee) = prev_callee_fn {
                                let bare_callee = callee
                                    .rsplit_once('.')
                                    .map(|(_, m)| m)
                                    .unwrap_or(callee.as_str());
                                let call_pat = format!("{}(", bare_callee);
                                for l_1 in d_line_1..=fn_end_1 {
                                    if lines[l_1 - 1].contains(&call_pat) {
                                        target_line_1 = l_1;
                                        break;
                                    }
                                }
                            }
                        }

                        if !fn_name.contains('.') {
                            if let Some(enc) =
                                recover_enclosing_symbol_name(&lines, sec, target_line_1, col)
                            {
                                if enc.ends_with(&format!(".{}", fn_name)) {
                                    display_fn = Some(enc);
                                }
                            }
                        }

                        let orig_line = resolve_section_line(&lines, sec, target_line_1);
                        resolved = Some((sec.short_label, orig_line));
                        break;
                    }
                }
            }
            if resolved.is_none() {
                if let Some(adj_line) = adj_line_opt {
                    if let Some(sec) = sections
                        .iter()
                        .rev()
                        .find(|s| adj_line > s.marker_line_1 && adj_line <= s.end_line_1)
                    {
                        let orig_line = resolve_section_line(&lines, sec, adj_line);
                        resolved = Some((sec.short_label, orig_line));
                        if display_fn.is_none() {
                            display_fn = recover_enclosing_symbol_name(&lines, sec, adj_line, col);
                        }
                    }
                }
            }
            if resolved.is_none() {
                if let Some(ref callee) = prev_callee_fn {
                    let bare_callee = callee
                        .rsplit_once('.')
                        .map(|(_, m)| m)
                        .unwrap_or(callee.as_str());
                    let call_pat = format!("{}(", bare_callee);
                    for sec in sections.iter().rev() {
                        for l_1 in (sec.marker_line_1 + 1)..=sec.end_line_1.min(lines.len()) {
                            let t = lines[l_1 - 1].trim();
                            if t.contains(&call_pat)
                                && !t.starts_with("function ")
                                && !(t.starts_with(&call_pat) && t.ends_with('{'))
                            {
                                let orig_line = resolve_section_line(&lines, sec, l_1);
                                resolved = Some((sec.short_label, orig_line));
                                if display_fn.is_none() {
                                    display_fn = recover_enclosing_symbol_name(&lines, sec, l_1, 1);
                                }
                                break;
                            }
                        }
                        if resolved.is_some() {
                            break;
                        }
                    }
                }
            }
        }

        if !out.is_empty() {
            out.push('\n');
        }
        if let Some((short_label, orig_line)) = resolved {
            if let Some(ref fn_str) = display_fn {
                out.push_str(&format!(
                    "    at {} ({}:{}:{})",
                    fn_str, short_label, orig_line, col
                ));
            } else {
                out.push_str(&format!("    at {}:{}:{}", short_label, orig_line, col));
            }
        } else {
            out.push_str(raw_line);
        }

        prev_callee_fn = display_fn;
        frame_ordinal += 1;
    }

    out
}
