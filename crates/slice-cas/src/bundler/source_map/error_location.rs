//! Uncaught JS Error Location Translator (`map_bundled_error_location`).

use super::{
    count_code_lines_between, lookup_embedded_line_map, map_bundled_stack_trace_with_offset,
    parse_slice_source_marker, read_bounded_source_file,
};

/// Maps an uncaught JS error or bundled line reference back to its original TypeScript/JavaScript
/// source file and 1-based line number (`<short_file>:<line>`) using `// #slice-source:` markers.
pub fn map_bundled_error_location(bundled_js: &str, error_msg: &str) -> String {
    map_bundled_error_location_with_offset(bundled_js, error_msg, 0)
}

fn rewrite_error_line_token(error_msg: &str, new_line: usize) -> String {
    if let Some(idx) = error_msg.find("line ") {
        let digit_start = idx + 5;
        let digit_len = error_msg[digit_start..]
            .bytes()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if digit_len > 0 {
            let mut out = String::with_capacity(error_msg.len() + 8);
            out.push_str(&error_msg[..digit_start]);
            let mut buf = itoa::Buffer::new();
            out.push_str(buf.format(new_line));
            out.push_str(&error_msg[digit_start + digit_len..]);
            return out;
        }
    }
    error_msg.to_string()
}

pub(super) fn subtract_offset_in_unbundled_stack(raw_stack: &str, offset: usize) -> String {
    if offset == 0 || raw_stack.trim().is_empty() {
        return raw_stack.to_string();
    }
    let mut out = String::with_capacity(raw_stack.len());
    let bytes = raw_stack.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let matched_prefix = if raw_stack[i..].starts_with("unknown at :") {
            Some("unknown at :")
        } else if raw_stack[i..].starts_with("<anonymous>:") {
            Some("<anonymous>:")
        } else {
            None
        };
        if let Some(pfx) = matched_prefix {
            let num_start = i + pfx.len();
            let digit_len = raw_stack[num_start..]
                .bytes()
                .take_while(|b| b.is_ascii_digit())
                .count();
            if digit_len > 0
                && raw_stack.as_bytes().get(num_start + digit_len) == Some(&b':')
                && let Ok(raw_l) = raw_stack[num_start..num_start + digit_len].parse::<usize>()
            {
                let adj_l = raw_l.saturating_sub(offset).max(1);
                out.push_str(pfx);
                let mut buf = itoa::Buffer::new();
                out.push_str(buf.format(adj_l));
                i = num_start + digit_len;
                continue;
            }
        }
        let ch = raw_stack[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
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

    let mut raw_parsed_line: Option<usize> = None;
    let mut explicit_line: Option<usize> = None;
    if let Some(idx) = error_msg.find("line ") {
        let digits: String = error_msg[idx + 5..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(n) = digits.parse::<usize>() {
            raw_parsed_line = Some(n);
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
        if (!active_short.is_empty() || !active_line_map.is_empty()) && b_line > marker_line_idx {
            let rel_idx_0 = count_code_lines_between(&lines, marker_line_idx, b_line);
            let resolved_line =
                if let Some(mapped) = lookup_embedded_line_map(active_line_map, rel_idx_0) {
                    mapped
                } else {
                    let mut guess = rel_idx_0 + 1;
                    if !active_full.is_empty()
                        && let Some(stmt_text) = lines.get(b_line - 1).map(|s| s.trim())
                        && !stmt_text.is_empty()
                        && let Some(orig_src) = read_bounded_source_file(active_full)
                        && let Some((orig_idx, _)) = orig_src
                            .lines()
                            .enumerate()
                            .find(|(_, ol)| ol.contains(stmt_text))
                    {
                        guess = orig_idx + 1;
                    }
                    guess
                };
            let rewritten = rewrite_error_line_token(error_msg, resolved_line);
            if active_short.is_empty() {
                return rewritten;
            }
            return format!("{} ({}:{})", rewritten, active_short, resolved_line);
        }

        if let Some(raw_n) = raw_parsed_line {
            let total_offset = line_offset + usize::from(bundled_js.starts_with("{\n"));
            if total_offset > 0 {
                let adj = raw_n.saturating_sub(total_offset).max(1);
                return rewrite_error_line_token(error_msg, adj);
            }
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
                let resolved_line = if let Some(mapped) =
                    lookup_embedded_line_map(active_line_map, rel_idx_0)
                {
                    mapped
                } else {
                    let mut guess = rel_idx_0 + 1;
                    if let Some(orig_src) = read_bounded_source_file(active_full)
                        && let Some((orig_idx, _)) = orig_src.lines().enumerate().find(|(_, ol)| {
                            ol.contains(core_msg)
                                || prefix_probe.is_some_and(|pp| {
                                    (ol.contains("throw ") || ol.contains("Error("))
                                        && ol.contains(pp)
                                })
                        })
                    {
                        guess = orig_idx + 1;
                    }
                    guess
                };
                return format!("{} ({}:{})", error_msg, active_short, resolved_line);
            }
        }
    }

    error_msg.to_string()
}
