//! Linear-Time Source Line Map Builder (`compute_source_line_map`).

use rustc_hash::FxHashMap;
use smallvec::SmallVec;

#[inline]
fn push_usize_csv(out: &mut String, n: usize) {
    if !out.is_empty() {
        out.push(',');
    }
    let mut buf = itoa::Buffer::new();
    out.push_str(buf.format(n));
}

#[inline]
fn cap_utf8_prefix(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[inline]
fn normalize_line_key(s: &str) -> &str {
    s.trim()
        .trim_end_matches(';')
        .trim_end_matches('{')
        .trim_end_matches(',')
        .trim()
}

fn strip_decl_modifiers(mut s: &str) -> &str {
    loop {
        let next = s
            .strip_prefix("export ")
            .or_else(|| s.strip_prefix("default "))
            .or_else(|| s.strip_prefix("declare "))
            .or_else(|| s.strip_prefix("public "))
            .or_else(|| s.strip_prefix("private "))
            .or_else(|| s.strip_prefix("protected "))
            .or_else(|| s.strip_prefix("readonly "))
            .or_else(|| s.strip_prefix("override "))
            .or_else(|| s.strip_prefix("abstract "))
            .or_else(|| s.strip_prefix("static "))
            .or_else(|| s.strip_prefix("async "))
            .map(str::trim_start);
        match next {
            Some(rest) => s = rest,
            None => return s,
        }
    }
}

fn next_anchor_token(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start();
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let b0 = bytes[0];
    if b0 == b'/' && bytes.get(1) == Some(&b'/') {
        return None;
    }
    if b0.is_ascii_alphabetic() || b0 == b'_' || b0 == b'$' {
        let mut end = 1;
        while end < bytes.len()
            && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_' || bytes[end] == b'$')
        {
            end += 1;
        }
        return Some((&s[..end], &s[end..]));
    }
    if b0.is_ascii_digit() {
        let mut end = 1;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'.') {
            end += 1;
        }
        return Some((&s[..end], &s[end..]));
    }
    if b0 == b'"' || b0 == b'\'' || b0 == b'`' {
        let mut end = 1;
        while end < bytes.len() {
            if bytes[end] == b'\\' {
                end = (end + 2).min(bytes.len());
                continue;
            }
            if bytes[end] == b0 {
                end += 1;
                break;
            }
            end += 1;
        }
        let tok = cap_utf8_prefix(&s[..end], 48);
        return Some((tok, &s[end..]));
    }
    let ch_len = s.chars().next().map_or(1, char::len_utf8);
    Some((&s[..ch_len], &s[ch_len..]))
}

fn extract_anchor_tokens(line: &str) -> (&str, &str, &str) {
    let s = strip_decl_modifiers(line.trim_start());
    let Some((t1, r1)) = next_anchor_token(s) else {
        return ("", "", "");
    };
    let Some((t2, r2)) = next_anchor_token(r1) else {
        return (t1, "", "");
    };
    let t3 = next_anchor_token(r2).map_or("", |(t, _)| t);
    (t1, t2, t3)
}

#[inline]
fn lookup_line_bucket(
    bucket: &[u32],
    search_cursor: usize,
    allow_hoisted_wrap: bool,
) -> Option<(usize, bool)> {
    let pos = bucket.partition_point(|&idx| (idx as usize) < search_cursor);
    if pos < bucket.len() {
        Some((bucket[pos] as usize, true))
    } else if allow_hoisted_wrap && !bucket.is_empty() {
        Some((bucket[0] as usize, false))
    } else {
        None
    }
}

/// Computes a 1-based original-line CSV map for each non-empty, non-comment line in `cjs_body`
/// in strictly linear `O(|raw_src| + |cjs_body|)` worst-case time using bounded line prefixes
/// (`<= 256` bytes) and `O(1)` hash-indexed token anchors.
pub fn compute_source_line_map(raw_src: &str, cjs_body: &str) -> String {
    const MAX_LINE_SCAN_BYTES: usize = 256;
    const LOCAL_WINDOW_LINES: usize = 8;

    let orig_lines: Vec<&str> = raw_src.lines().collect();
    if orig_lines.is_empty() {
        return String::new();
    }

    let orig_heads: Vec<&str> = orig_lines
        .iter()
        .map(|l| cap_utf8_prefix(l.trim_start(), MAX_LINE_SCAN_BYTES))
        .collect();

    let cap_hint = orig_lines.len();
    let mut exact_map: FxHashMap<&str, SmallVec<[u32; 2]>> =
        FxHashMap::with_capacity_and_hasher(cap_hint, Default::default());
    let mut tok3_map: FxHashMap<(&str, &str, &str), SmallVec<[u32; 2]>> =
        FxHashMap::with_capacity_and_hasher(cap_hint, Default::default());
    let mut tok2_map: FxHashMap<(&str, &str), SmallVec<[u32; 4]>> =
        FxHashMap::with_capacity_and_hasher(cap_hint, Default::default());

    for (idx, &head) in orig_heads.iter().enumerate() {
        if head.is_empty() || head.starts_with("//") {
            continue;
        }
        let idx_u32 = idx as u32;
        let norm = normalize_line_key(head);
        if norm.len() >= 2 {
            exact_map.entry(norm).or_default().push(idx_u32);
        }
        let (t1, t2, t3) = extract_anchor_tokens(head);
        if !t1.is_empty() && !t2.is_empty() && (t1.len() >= 2 || t2.len() >= 2) {
            tok2_map.entry((t1, t2)).or_default().push(idx_u32);
            if !t3.is_empty() {
                tok3_map.entry((t1, t2, t3)).or_default().push(idx_u32);
            }
        }
    }

    let mut out = String::with_capacity((cjs_body.len() / 8).max(32));
    let mut search_cursor = 0usize;

    for raw_cjs_line in cjs_body.lines() {
        let trimmed = cap_utf8_prefix(raw_cjs_line.trim(), MAX_LINE_SCAN_BYTES);
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        let probe_raw = if let Some(rest) = trimmed.strip_prefix("const __slice") {
            if let Some((_, rhs)) = rest.split_once("_default = ") {
                rhs.trim_end_matches(';').trim()
            } else {
                trimmed.trim_end_matches(';').trim()
            }
        } else {
            trimmed.trim_end_matches(';').trim()
        };

        if probe_raw.starts_with("exports.")
            || probe_raw.starts_with("Object.defineProperty(exports,")
            || probe_raw.starts_with("globalThis.__slice_export_star(exports,")
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
            .filter(|r| r.len() >= 4 && !r.starts_with("require("));

        // 1. O(1) bounded local window (`<= 8` lines, `<= 256` bytes each) for adjacent statements.
        let win_end = (search_cursor + LOCAL_WINDOW_LINES).min(orig_heads.len());
        let mut matched: Option<(usize, bool)> = None;
        if !probe_raw.is_empty() {
            for (offset, &orig_head) in orig_heads[search_cursor..win_end].iter().enumerate() {
                if orig_head.contains(probe_raw)
                    || rhs_probe.is_some_and(|rp| orig_head.contains(rp))
                {
                    matched = Some((search_cursor + offset, true));
                    break;
                }
            }
        }

        // 2. O(1) hash-indexed exact normalized line & RHS lookup across the whole file.
        let can_wrap = probe_raw.len() >= 12 && probe_raw.bytes().any(|b| b.is_ascii_alphabetic());
        if matched.is_none() {
            let norm_probe = normalize_line_key(probe_raw);
            if let Some(bucket) = exact_map.get(norm_probe) {
                matched = lookup_line_bucket(bucket, search_cursor, can_wrap);
            }
        }
        if matched.is_none()
            && let Some(rp) = rhs_probe
        {
            let norm_rhs = normalize_line_key(rp);
            if let Some(bucket) = exact_map.get(norm_rhs) {
                matched = lookup_line_bucket(bucket, search_cursor, can_wrap);
            }
        }

        // 3. O(1) TypeScript-erasure-resilient 3-token and 2-token anchor lookup.
        if matched.is_none() {
            let (t1, t2, t3) = extract_anchor_tokens(probe_raw);
            if !t1.is_empty() && !t2.is_empty() {
                if !t3.is_empty()
                    && let Some(bucket) = tok3_map.get(&(t1, t2, t3))
                {
                    matched =
                        lookup_line_bucket(bucket, search_cursor, can_wrap && bucket.len() == 1);
                }
                if matched.is_none()
                    && let Some(bucket) = tok2_map.get(&(t1, t2))
                {
                    matched =
                        lookup_line_bucket(bucket, search_cursor, can_wrap && bucket.len() == 1);
                }
            }
        }

        if let Some((idx, advance_cursor)) = matched {
            if advance_cursor {
                search_cursor = idx;
            }
            push_usize_csv(&mut out, idx + 1);
        } else {
            let next_guess = (search_cursor + 1).min(orig_lines.len()).max(1);
            push_usize_csv(&mut out, next_guess);
        }
    }
    out
}
