//! Shared Manifest, Directive, CLI Numeric/Byte-Size, and JSON Escaping Parsers.

/// Strips surrounding whitespace and matching outer `"` or `'` quotes if present.
#[inline]
pub(crate) fn unquote_trimmed(raw: &str) -> &str {
    let trimmed = raw.trim();
    if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        trimmed[1..trimmed.len() - 1].trim()
    } else {
        trimmed
    }
}

/// Unescapes `\n`, `\r`, `\t`, `\"`, and `\\` sequences in a double-quoted string in one pass.
#[inline]
pub(crate) fn unescape_double_quoted_str(inner: &str) -> String {
    if !inner.contains('\\') {
        return inner.to_string();
    }
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Parses an unsigned 64-bit integer while ignoring `_`, quotes, and whitespace (`SBX-PERF-03`).
#[inline]
pub(crate) fn parse_u64_ignoring_underscores(raw: &str) -> Option<u64> {
    let mut acc = 0u64;
    let mut saw_digit = false;
    for &b in raw.trim().as_bytes() {
        if b.is_ascii_digit() {
            acc = acc.checked_mul(10)?.checked_add((b - b'0') as u64)?;
            saw_digit = true;
        } else if b == b'_' || b == b'"' || b == b'\'' || b.is_ascii_whitespace() {
            continue;
        } else {
            return None;
        }
    }
    saw_digit.then_some(acc)
}

/// Strict unsigned integer parser (`u64`) allowing optional `_` digit separators (`50_000_000`)
/// and surrounding quotes/whitespace, while rejecting any non-digit characters or unit suffixes.
#[inline]
pub fn parse_strict_u64(raw: &str) -> Option<u64> {
    let s = raw.trim().trim_matches('"').trim_matches('\'').trim();
    if s.is_empty() {
        return None;
    }
    let mut acc: u64 = 0;
    let mut saw_digit = false;
    for &b in s.as_bytes() {
        if b.is_ascii_digit() {
            saw_digit = true;
            acc = acc.checked_mul(10)?.checked_add((b - b'0') as u64)?;
        } else if b == b'_' {
            if !saw_digit {
                return None;
            }
        } else {
            return None;
        }
    }
    saw_digit.then_some(acc)
}

/// Unit-aware byte-size parser supporting plain byte counts (`1048576`, `65_536`) as well as
/// case-insensitive `B`/`bytes`, `K`/`KB`/`KiB`, `M`/`MB`/`MiB`, and `G`/`GB`/`GiB` suffixes
/// (`16MB`, `64KB`, `2GiB`). Rejects malformed numbers, unknown suffixes, and `usize` overflow.
#[inline]
pub fn parse_byte_size_usize(raw: &str) -> Option<usize> {
    let s = raw.trim().trim_matches('"').trim_matches('\'').trim();
    if s.is_empty() {
        return None;
    }
    let bytes = s.as_bytes();
    let mut idx = 0;
    let mut count: u64 = 0;
    let mut saw_digit = false;
    while idx < bytes.len() {
        let b = bytes[idx];
        if b.is_ascii_digit() {
            saw_digit = true;
            count = count.checked_mul(10)?.checked_add((b - b'0') as u64)?;
            idx += 1;
        } else if b == b'_' {
            if !saw_digit {
                return None;
            }
            idx += 1;
        } else {
            break;
        }
    }
    if !saw_digit {
        return None;
    }
    let suffix = s[idx..].trim();
    let multiplier: u64 = if suffix.is_empty()
        || suffix.eq_ignore_ascii_case("b")
        || suffix.eq_ignore_ascii_case("bytes")
    {
        1
    } else if suffix.eq_ignore_ascii_case("k")
        || suffix.eq_ignore_ascii_case("kb")
        || suffix.eq_ignore_ascii_case("kib")
    {
        1024
    } else if suffix.eq_ignore_ascii_case("m")
        || suffix.eq_ignore_ascii_case("mb")
        || suffix.eq_ignore_ascii_case("mib")
    {
        1024 * 1024
    } else if suffix.eq_ignore_ascii_case("g")
        || suffix.eq_ignore_ascii_case("gb")
        || suffix.eq_ignore_ascii_case("gib")
    {
        1024 * 1024 * 1024
    } else {
        return None;
    };
    let total = count.checked_mul(multiplier)?;
    usize::try_from(total).ok()
}

/// Parses a boolean setting string (`true`/`1`/`yes`/`on` vs `false`/`0`/`no`/`off`).
#[inline]
pub fn parse_bool_setting(raw: &str) -> Option<bool> {
    let norm = raw.trim().trim_matches('"').trim_matches('\'').trim();
    if matches!(
        norm,
        "true" | "TRUE" | "1" | "yes" | "YES" | "on" | "ON" | "enabled"
    ) {
        Some(true)
    } else if matches!(
        norm,
        "false" | "FALSE" | "0" | "no" | "NO" | "off" | "OFF" | "disabled"
    ) {
        Some(false)
    } else {
        None
    }
}

/// Parses an execution tier setting (`"1"`/`"tier1"`/`"boa"` -> `Some(1)`,
/// `"0"`/`"tier0"`/`"snp"`/`"slicesnp"` -> `Some(0)`, `"auto"`/`"default"` -> `None`).
#[inline]
pub fn parse_tier_setting(raw: &str) -> Option<Option<u8>> {
    let norm = raw.trim().trim_matches('"').trim_matches('\'').trim();
    if norm.eq_ignore_ascii_case("1")
        || norm.eq_ignore_ascii_case("tier1")
        || norm.eq_ignore_ascii_case("tier-1")
        || norm.eq_ignore_ascii_case("boa")
        || norm.eq_ignore_ascii_case("baseline")
    {
        Some(Some(1))
    } else if norm.eq_ignore_ascii_case("0")
        || norm.eq_ignore_ascii_case("tier0")
        || norm.eq_ignore_ascii_case("tier-0")
        || norm.eq_ignore_ascii_case("snp")
        || norm.eq_ignore_ascii_case("slicesnp")
    {
        Some(Some(0))
    } else if norm.eq_ignore_ascii_case("auto")
        || norm.eq_ignore_ascii_case("default")
        || norm.eq_ignore_ascii_case("optimal")
    {
        Some(None)
    } else {
        None
    }
}

pub(crate) fn strip_toml_inline_comment(raw: &str) -> &str {
    let mut in_double = false;
    let mut in_single = false;
    let mut escaped = false;
    for (idx, ch) in raw.char_indices() {
        if in_double {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_double = false;
            }
            continue;
        }
        if in_single {
            if ch == '\'' {
                in_single = false;
            }
            continue;
        }
        match ch {
            '"' => in_double = true,
            '\'' => in_single = true,
            '#' => return raw[..idx].trim(),
            _ => {}
        }
    }
    raw.trim()
}

#[inline]
pub(crate) fn bracket_delta_outside_quotes(cleaned_line: &str) -> i32 {
    let mut delta = 0i32;
    let mut in_double = false;
    let mut in_single = false;
    let mut escaped = false;
    for ch in cleaned_line.chars() {
        if in_double {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_double = false;
            }
            continue;
        }
        if in_single {
            if ch == '\'' {
                in_single = false;
            }
            continue;
        }
        match ch {
            '"' => in_double = true,
            '\'' => in_single = true,
            '[' => delta += 1,
            ']' => delta -= 1,
            _ => {}
        }
    }
    delta
}

#[inline]
pub(crate) fn parse_toml_string_array(raw: &str) -> Vec<String> {
    let cleaned = strip_toml_inline_comment(raw);
    let inner = if let Some(after_open) = cleaned.strip_prefix('[') {
        let Some(closed) = after_open.strip_suffix(']') else {
            return Vec::new();
        };
        closed
    } else {
        cleaned
    };
    let mut items = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut had_quotes = false;

    for ch in inner.chars() {
        if let Some(q) = quote {
            if q == '"' && escaped {
                match ch {
                    'n' => current.push('\n'),
                    'r' => current.push('\r'),
                    't' => current.push('\t'),
                    '\\' => current.push('\\'),
                    '"' => current.push('"'),
                    other => {
                        current.push('\\');
                        current.push(other);
                    }
                }
                escaped = false;
            } else if q == '"' && ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }

        match ch {
            '"' | '\'' => {
                if !had_quotes {
                    current.clear();
                }
                quote = Some(ch);
                had_quotes = true;
            }
            ',' => {
                let val = if had_quotes {
                    current.clone()
                } else {
                    current.trim().to_string()
                };
                if !val.is_empty() {
                    items.push(val);
                }
                current.clear();
                had_quotes = false;
            }
            _ => {
                if !had_quotes {
                    current.push(ch);
                }
            }
        }
    }
    let tail = if had_quotes {
        current
    } else {
        current.trim().to_string()
    };
    if !tail.is_empty() {
        items.push(tail);
    }
    items
}

/// Escapes `raw` for safe embedding inside a JSON string literal (`"..."`), preventing
/// malformed or injected JSON when formatting `SandboxViolation` or HTTP error messages.
#[inline]
pub fn escape_json_string_into(raw: &str, out: &mut String) {
    for ch in raw.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
}

/// Formats `raw_err` as a JSON error payload `{"error":"<escaped>"}`.
#[inline]
pub fn format_json_error_body(raw_err: &str) -> String {
    let mut out = String::with_capacity(raw_err.len() + 16);
    out.push_str(r#"{"error":""#);
    escape_json_string_into(raw_err, &mut out);
    out.push_str(r#""}"#);
    out
}
