//! Multi-Frame ECMAScript Stack Trace Translator (`map_bundled_stack_trace`).

use super::error_location::subtract_offset_in_unbundled_stack;
use super::scope_recovery::recover_enclosing_symbol_name;
use super::{BundledSourceSection, parse_slice_source_marker, resolve_section_line};

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
    if let (Some(col_s), Some(line_s)) = (rsplit.next(), rsplit.next())
        && let (Ok(l), Ok(c)) = (
            line_s.trim().parse::<usize>(),
            col_s.trim().parse::<usize>(),
        )
    {
        line_col = Some((l, c));
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
            if let Some(body) = tail.strip_suffix(')')
                && !body.is_empty()
                && body.bytes().all(|b| b.is_ascii_digit() || b == b':')
            {
                return &header[..open_idx];
            }
        }
    }
    header
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
    if raw_stack.trim().is_empty() {
        return raw_stack.to_string();
    }
    if !bundled_js.contains("// #slice-source: ") && !bundled_js.contains("//@sm:") {
        let total_offset = line_offset + usize::from(bundled_js.starts_with("{\n"));
        return subtract_offset_in_unbundled_stack(raw_stack, total_offset);
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
        if let Some(raw_fn) = frame.raw_fn_name
            && (raw_fn == "__run" || raw_fn.starts_with("__slice_"))
        {
            continue;
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
        if is_boa_bundle_loc && let Some(adj_line) = adj_line_opt {
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
                        if let Some(enc) = recover_enclosing_symbol_name(&lines, sec, adj_line, col)
                            && enc.ends_with(&format!(".{}", existing))
                        {
                            display_fn = Some(enc);
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
                        if let Some(adj_line) = adj_line_opt
                            && adj_line >= d_line_1
                            && adj_line <= fn_end_1
                        {
                            target_line_1 = adj_line;
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

                        if !fn_name.contains('.')
                            && let Some(enc) =
                                recover_enclosing_symbol_name(&lines, sec, target_line_1, col)
                            && enc.ends_with(&format!(".{}", fn_name))
                        {
                            display_fn = Some(enc);
                        }

                        let orig_line = resolve_section_line(&lines, sec, target_line_1);
                        resolved = Some((sec.short_label, orig_line));
                        break;
                    }
                }
            }
            if resolved.is_none()
                && let Some(adj_line) = adj_line_opt
                && let Some(sec) = sections
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
            if resolved.is_none()
                && let Some(ref callee) = prev_callee_fn
            {
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

        if !out.is_empty() {
            out.push('\n');
        }
        if let Some((short_label, orig_line)) = resolved {
            if short_label.is_empty() {
                let raw_fn_disp = frame.raw_fn_name.unwrap_or("<main>");
                out.push_str(&format!(
                    "    at {} (unknown at :{}:{})",
                    raw_fn_disp, orig_line, col
                ));
            } else if let Some(ref fn_str) = display_fn {
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
