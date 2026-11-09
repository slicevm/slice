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
        let forward_limit = (search_cursor + 128).min(orig_lines.len());
        for (offset, orig_line) in orig_lines[search_cursor..forward_limit].iter().enumerate() {
            if orig_line.contains(probe_raw)
                || rhs_probe.is_some_and(|rp| !rp.starts_with("require(") && orig_line.contains(rp))
            {
                found_idx = Some(search_cursor + offset);
                break;
            }
        }
        if found_idx.is_none() {
            for (idx, orig_line) in orig_lines.iter().enumerate() {
                if orig_line.contains(probe_raw)
                    || rhs_probe
                        .is_some_and(|rp| !rp.starts_with("require(") && orig_line.contains(rp))
                {
                    found_idx = Some(idx);
                    break;
                }
            }
        }
        if let Some(idx) = found_idx {
            search_cursor = idx;
            push_usize_csv(&mut out, idx + 1);
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
    let lines: Vec<&str> = bundled_js.lines().collect();

    let mut explicit_line: Option<usize> = None;
    if let Some(idx) = error_msg.find("line ") {
        let digits: String = error_msg[idx + 5..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(n) = digits.parse::<usize>() {
            explicit_line = Some(n);
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
            let mut resolved_line = lookup_embedded_line_map(active_line_map, rel_idx_0)
                .unwrap_or(rel_idx_0 + 1);
            if let Some(stmt_text) = lines.get(b_line - 1).map(|s| s.trim()) {
                if !stmt_text.is_empty() {
                    if let Ok(orig_src) = std::fs::read_to_string(active_full) {
                        if let Some((orig_idx, _)) = orig_src
                            .lines()
                            .enumerate()
                            .find(|(_, ol)| ol.contains(stmt_text))
                        {
                            resolved_line = orig_idx + 1;
                        }
                    }
                }
            }
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
                let mut resolved_line = lookup_embedded_line_map(active_line_map, rel_idx_0)
                    .unwrap_or(rel_idx_0 + 1);
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
    let mut resolved_line =
        lookup_embedded_line_map(sec.line_map, rel_idx_0).unwrap_or(rel_idx_0 + 1);
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

type ParsedRawStackFrame<'a> = (Option<&'a str>, Option<(usize, usize)>);

fn parse_raw_stack_frame(trimmed: &str) -> Option<ParsedRawStackFrame<'_>> {
    let after_at = trimmed.strip_prefix("at ")?.trim();
    let (fn_name_opt, loc_str) = if let Some((fn_part, rest)) = after_at.split_once(" (") {
        let fn_clean = fn_part.trim();
        let fn_opt = if fn_clean.is_empty()
            || fn_clean == "unknown"
            || fn_clean == "__run"
            || fn_clean == "<anonymous>"
            || fn_clean.starts_with("__slice")
        {
            None
        } else {
            Some(fn_clean)
        };
        (fn_opt, rest.trim_end_matches(')').trim())
    } else {
        (None, after_at)
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
    Some((fn_name_opt, line_col))
}

/// Translates a multi-frame ECMAScript `Error.prototype.stack` string against embedded
/// `// #slice-source: <short_label>|<full_path>|<line_map>` (or compacted `//@sm:`) markers inside `bundled_js`,
/// mapping each stack frame back to its original TypeScript/JavaScript `<short_label>:<line>:<col>`.
pub fn map_bundled_stack_trace(bundled_js: &str, raw_stack: &str) -> String {
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

    let raw_lines: Vec<&str> = raw_stack.lines().collect();
    let parsed_frames: Vec<Option<ParsedRawStackFrame<'_>>> = raw_lines
        .iter()
        .map(|l| parse_raw_stack_frame(l.trim()))
        .collect();

    let mut out = String::with_capacity(raw_stack.len() + 128);
    let mut frame_ordinal = 0usize;
    let mut prev_callee_fn: Option<&str> = None;

    for (line_idx, &raw_line) in raw_lines.iter().enumerate() {
        let Some((fn_name_opt, line_col_opt)) = parsed_frames[line_idx] else {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(raw_line);
            continue;
        };

        let col = line_col_opt.map(|(_, c)| c).unwrap_or(1);
        let mut resolved: Option<(&str, usize)> = None;

        if let Some(fn_name) = fn_name_opt {
            let bare_fn = fn_name
                .rsplit_once('.')
                .map(|(_, m)| m)
                .unwrap_or(fn_name);
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
                    if frame_ordinal == 0 {
                        for l_1 in d_line_1..=fn_end_1 {
                            let t = lines[l_1 - 1];
                            if t.contains("throw ") || t.contains("Error(") {
                                target_line_1 = l_1;
                                break;
                            }
                        }
                    } else if let Some(callee) = prev_callee_fn {
                        let bare_callee =
                            callee.rsplit_once('.').map(|(_, m)| m).unwrap_or(callee);
                        let call_pat = format!("{}(", bare_callee);
                        for l_1 in d_line_1..=fn_end_1 {
                            if lines[l_1 - 1].contains(&call_pat) {
                                target_line_1 = l_1;
                                break;
                            }
                        }
                    }
                    if target_line_1 == d_line_1 {
                        if let Some((r_line, _)) = line_col_opt {
                            if r_line >= d_line_1 && r_line <= fn_end_1 {
                                target_line_1 = r_line;
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
            if let Some(callee) = prev_callee_fn {
                let bare_callee = callee.rsplit_once('.').map(|(_, m)| m).unwrap_or(callee);
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
                            break;
                        }
                    }
                    if resolved.is_some() {
                        break;
                    }
                }
            }
            if resolved.is_none() {
                if let Some((r_line, _)) = line_col_opt {
                    if let Some(sec) = sections
                        .iter()
                        .rev()
                        .find(|s| r_line > s.marker_line_1 && r_line <= s.end_line_1)
                    {
                        let orig_line = resolve_section_line(&lines, sec, r_line);
                        resolved = Some((sec.short_label, orig_line));
                    }
                }
            }
        }

        if !out.is_empty() {
            out.push('\n');
        }
        if let Some((short_label, orig_line)) = resolved {
            if let Some(fn_name) = fn_name_opt {
                out.push_str(&format!(
                    "    at {} ({}:{}:{})",
                    fn_name, short_label, orig_line, col
                ));
            } else {
                out.push_str(&format!("    at {}:{}:{}", short_label, orig_line, col));
            }
        } else {
            out.push_str(raw_line);
        }

        prev_callee_fn = fn_name_opt;
        frame_ordinal += 1;
    }

    out
}
