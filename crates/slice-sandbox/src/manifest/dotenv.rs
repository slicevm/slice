//! `.env` File Parser & Automatic Outbound Egress DLP Credential Classification.

use rustc_hash::FxHashSet;

#[inline]
fn contains_ascii_ignore_case(haystack: &[u8], needle_upper: &[u8]) -> bool {
    if needle_upper.is_empty() || haystack.len() < needle_upper.len() {
        return false;
    }
    let first = needle_upper[0];
    let limit = haystack.len() - needle_upper.len();
    let mut i = 0usize;
    while i <= limit {
        if haystack[i].to_ascii_uppercase() == first
            && haystack[i..i + needle_upper.len()].eq_ignore_ascii_case(needle_upper)
        {
            return true;
        }
        i += 1;
    }
    false
}

/// Returns `true` if `key` matches sensitive credential naming patterns (`*_SECRET*`, `*_TOKEN*`,
/// `*_KEY*`, `*_PASSWORD*`, `*_PRIVATE*`, `*_CREDENTIAL*`, `*_PASSWD*`, `*_AUTH*`) and is not a
/// public/publishable key (`SBX-PERF-03` zero-allocation case-insensitive scan).
#[inline]
pub fn is_sensitive_env_key(key: &str) -> bool {
    let b = key.trim().as_bytes();
    if contains_ascii_ignore_case(b, b"PUBLIC_KEY")
        || contains_ascii_ignore_case(b, b"PUBLISHABLE_KEY")
    {
        return false;
    }
    contains_ascii_ignore_case(b, b"SECRET")
        || contains_ascii_ignore_case(b, b"TOKEN")
        || contains_ascii_ignore_case(b, b"KEY")
        || contains_ascii_ignore_case(b, b"PASSWORD")
        || contains_ascii_ignore_case(b, b"PASSWD")
        || contains_ascii_ignore_case(b, b"PRIVATE")
        || contains_ascii_ignore_case(b, b"CREDENTIAL")
}

/// Unescapes double-quoted `.env` string values in a single pass (`SBX-PERF-03`).
#[inline]
fn unescape_double_quoted_dotenv(inner: &str) -> String {
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

/// Parses `.env` file content into `(env_pairs, auto_dlp_secrets)`.
/// Automatically classifies values of keys matching [`is_sensitive_env_key`] (with length >= 6)
/// as outbound Egress DLP secrets so they cannot be exfiltrated via `stdout`, HTTP responses, SSE, or `fetch()`.
pub fn parse_dotenv_content(content: &str) -> (Vec<(String, String)>, Vec<String>) {
    let mut pairs = Vec::new();
    let mut secrets = Vec::new();
    let mut seen_secrets = FxHashSet::default();

    for raw_line in content.lines() {
        let mut line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(stripped) = line.strip_prefix("export ") {
            line = stripped.trim();
        }
        let Some((raw_k, raw_v)) = line.split_once('=') else {
            continue;
        };
        let key_slice = raw_k.trim();
        if key_slice.is_empty() {
            continue;
        }
        let val_part = raw_v.trim();
        let val = if let Some(after_open) = val_part.strip_prefix('"') {
            let mut end_quote = None;
            let mut escaped = false;
            for (ci, ch) in after_open.char_indices() {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    end_quote = Some(ci);
                    break;
                }
            }
            if let Some(eq_idx) = end_quote {
                unescape_double_quoted_dotenv(&after_open[..eq_idx])
            } else {
                unescape_double_quoted_dotenv(after_open)
            }
        } else if let Some(after_open) = val_part.strip_prefix('\'') {
            // Single-quoted `.env` values are literal according to POSIX shell / dotenv rules
            if let Some(eq_idx) = after_open.find('\'') {
                after_open[..eq_idx].to_string()
            } else {
                after_open.to_string()
            }
        } else {
            let mut unquoted = val_part;
            for (bi, ch) in val_part.char_indices() {
                if ch == '#' && (bi == 0 || val_part.as_bytes()[bi - 1].is_ascii_whitespace()) {
                    unquoted = val_part[..bi].trim();
                    break;
                }
            }
            unquoted.to_string()
        };

        if val.len() >= 6 && is_sensitive_env_key(key_slice) && seen_secrets.insert(val.clone()) {
            secrets.push(val.clone());
        }
        pairs.push((key_slice.to_string(), val));
    }

    (pairs, secrets)
}
