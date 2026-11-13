//! Native Rust WinterTC / WHATWG Hot Primitives (`URL`, `URL.canParse`, `URLSearchParams`,
//! `application/x-www-form-urlencoded` Codec, and Fast Pathname Normalization).
//!
//! Provides memory-safe, zero-exception Rust implementations for hot WinterTC web constructors
//! and methods, registered lazily when Prelude Shard 1 (`01_web_fetch_url.js`) is loaded.

use std::borrow::Cow;

/// Parsed components of a WHATWG `URL` instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedWhatwgUrl {
    pub href: String,
    pub origin: String,
    pub protocol: String,
    pub username: String,
    pub password: String,
    pub host: String,
    pub hostname: String,
    pub port: String,
    pub pathname: String,
    pub search: String,
    pub hash: String,
}

/// Returns `true` if `b` is an ECMAScript `uriUnescaped` ASCII byte
/// (`A-Z`, `a-z`, `0-9`, `-`, `_`, `.`, `!`, `~`, `*`, `'`, `(`, `)`).
#[inline]
pub const fn is_uri_unescaped(b: u8) -> bool {
    matches!(
        b,
        b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')'
    )
}

/// Encodes a component according to `application/x-www-form-urlencoded` rules compatible with
/// `encodeURIComponent(s)` with `' '` mapped to `'+'`.
/// Returns `Cow::Borrowed(s)` with zero allocations when all bytes are `is_uri_unescaped`.
#[inline]
pub fn usp_encode_component(s: &str) -> Cow<'_, str> {
    let bytes = s.as_bytes();
    if bytes.iter().copied().all(is_uri_unescaped) {
        return Cow::Borrowed(s);
    }
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(bytes.len() + 8);
    for &b in bytes {
        if is_uri_unescaped(b) {
            out.push(b as char);
        } else if b == b' ' {
            out.push('+');
        } else {
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0x0F) as usize] as char);
        }
    }
    Cow::Owned(out)
}

#[inline]
fn decode_hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Decodes an `application/x-www-form-urlencoded` component (`'+'` -> `' '` and `%XX` hex sequences).
/// Returns `Cow::Borrowed(s)` with zero allocations when neither `'+'` nor `'%'` is present.
#[inline]
pub fn usp_decode_component(s: &str) -> Cow<'_, str> {
    let bytes = s.as_bytes();
    if memchr::memchr2(b'+', b'%', bytes).is_none() {
        return Cow::Borrowed(s);
    }
    let mut raw = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'+' {
            raw.push(b' ');
            i += 1;
        } else if b == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (
                decode_hex_nibble(bytes[i + 1]),
                decode_hex_nibble(bytes[i + 2]),
            ) {
                raw.push((hi << 4) | lo);
                i += 3;
            } else {
                raw.push(b);
                i += 1;
            }
        } else {
            raw.push(b);
            i += 1;
        }
    }
    match String::from_utf8(raw) {
        Ok(decoded) => Cow::Owned(decoded),
        Err(_) => {
            // Match JS `try { return decodeURIComponent(r); } catch (_e) { return r; }`
            Cow::Owned(s.replace('+', " "))
        }
    }
}

/// Parses a query string (with or without leading `'?'`) into `(key, value)` pairs.
pub fn usp_parse_query(query: &str) -> Vec<(String, String)> {
    let q = query.strip_prefix('?').unwrap_or(query);
    if q.is_empty() {
        return Vec::new();
    }
    let mut pairs = Vec::with_capacity(4);
    for seg in q.split('&') {
        if seg.is_empty() {
            continue;
        }
        if let Some((k, v)) = seg.split_once('=') {
            pairs.push((
                usp_decode_component(k).into_owned(),
                usp_decode_component(v).into_owned(),
            ));
        } else {
            pairs.push((usp_decode_component(seg).into_owned(), String::new()));
        }
    }
    pairs
}

/// Serializes `(key, value)` query parameter pairs into an `application/x-www-form-urlencoded` string.
pub fn usp_stringify_pairs(pairs: &[(String, String)]) -> String {
    if pairs.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(pairs.len() * 16);
    for (idx, (k, v)) in pairs.iter().enumerate() {
        if idx > 0 {
            out.push('&');
        }
        out.push_str(&usp_encode_component(k));
        out.push('=');
        out.push_str(&usp_encode_component(v));
    }
    out
}

/// Normalizes a WHATWG URL pathname, collapsing `.` and `..` segments and duplicate slashes
/// while returning `Cow::Borrowed` when the pathname is already canonical.
#[inline]
pub fn normalize_url_pathname(p: &str) -> Cow<'_, str> {
    if p.is_empty() || p == "/" {
        return Cow::Borrowed("/");
    }
    if !p.contains("/.") && !p.contains("//") && !p.contains('\\') && p.starts_with('/') {
        return Cow::Borrowed(p);
    }
    let trailing_slash = p.len() > 1 && (p.ends_with('/') || p.ends_with('\\'));
    let mut out: smallvec::SmallVec<[&str; 16]> = smallvec::SmallVec::new();
    for seg in p.split(['/', '\\']) {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            out.pop();
        } else {
            out.push(seg);
        }
    }
    if out.is_empty() {
        return Cow::Borrowed("/");
    }
    let mut res = String::with_capacity(p.len() + 1);
    for seg in &out {
        res.push('/');
        res.push_str(seg);
    }
    if trailing_slash {
        res.push('/');
    }
    Cow::Owned(res)
}

#[inline]
fn strip_whatwg_ascii_tab_or_newline(s: &str) -> Cow<'_, str> {
    if s.bytes().any(|b| matches!(b, b'\t' | b'\n' | b'\r')) {
        Cow::Owned(
            s.chars()
                .filter(|&c| !matches!(c, '\t' | '\n' | '\r'))
                .collect(),
        )
    } else {
        Cow::Borrowed(s)
    }
}

#[inline]
fn is_valid_whatwg_hostname(hostname: &str) -> bool {
    if hostname.is_empty() {
        return false;
    }
    let bytes = hostname.as_bytes();
    if bytes[0] == b'[' {
        if bytes.last() != Some(&b']') || bytes.len() < 4 {
            return false;
        }
        return bytes[1..bytes.len() - 1]
            .iter()
            .all(|&b| b.is_ascii_hexdigit() || b == b':' || b == b'.');
    }
    // Reject WHATWG forbidden host code points: C0 controls, spaces, and authority/path delimiters
    !bytes.iter().any(|&b| {
        b <= 0x20
            || b == 0x7F
            || matches!(
                b,
                b'#' | b'%'
                    | b'/'
                    | b':'
                    | b'<'
                    | b'>'
                    | b'?'
                    | b'@'
                    | b'['
                    | b'\\'
                    | b']'
                    | b'^'
                    | b'|'
                    | b'"'
                    | b'`'
            )
    })
}

#[inline]
fn has_rfc3986_scheme(s: &str) -> bool {
    let bytes = s.as_bytes();
    let Some(&first) = bytes.first() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    for &b in &bytes[1..] {
        if b == b':' {
            return true;
        }
        if !b.is_ascii_alphanumeric() && !matches!(b, b'+' | b'-' | b'.') {
            return false;
        }
    }
    false
}

/// Resolves and parses a WHATWG `URL` from `input` and optional `base`, returning `None` if invalid.
pub fn parse_whatwg_url(input: &str, base: Option<&str>) -> Option<ParsedWhatwgUrl> {
    // WHATWG URL step 1: trim leading/trailing C0 controls and space (`<= 0x20`) and strip ASCII tab/LF/CR
    let trimmed_outer = input.trim_matches(|c: char| (c as u32) <= 0x20);
    let cleaned_cow = strip_whatwg_ascii_tab_or_newline(trimmed_outer);
    let trimmed = cleaned_cow.as_ref();

    let resolved_cow: Cow<'_, str> =
        if let Some(base_str) = base.filter(|_| !has_rfc3986_scheme(trimmed)) {
            let b = parse_whatwg_url(base_str, None)?;
            if trimmed.starts_with("//") {
                Cow::Owned(format!("{}{}", b.protocol, trimmed))
            } else if trimmed.starts_with('/') || trimmed.starts_with('\\') {
                Cow::Owned(format!("{}{}", b.origin, trimmed))
            } else if trimmed.starts_with('?') {
                Cow::Owned(format!("{}{}{}", b.origin, b.pathname, trimmed))
            } else if trimmed.starts_with('#') {
                Cow::Owned(format!("{}{}{}{}", b.origin, b.pathname, b.search, trimmed))
            } else if trimmed.is_empty() {
                Cow::Owned(b.href)
            } else {
                let dir_end = b.pathname.rfind('/').map(|i| i + 1).unwrap_or(1);
                let dir = &b.pathname[..dir_end];
                Cow::Owned(format!("{}{}{}", b.origin, dir, trimmed))
            }
        } else {
            Cow::Borrowed(trimmed)
        };

    let raw = resolved_cow.as_ref();
    let (before_hash, hash_raw) = match raw.find('#') {
        Some(hi) => (&raw[..hi], &raw[hi..]),
        None => (raw, ""),
    };
    let (before_query, search_raw) = match before_hash.find('?') {
        Some(qi) => (&before_hash[..qi], &before_hash[qi..]),
        None => (before_hash, ""),
    };

    let Some(si) = before_query.find("://") else {
        let ci = before_query.find(':')?;
        if ci == 0 {
            return None;
        }
        let scheme_part = &before_query[..ci];
        if !scheme_part.bytes().next()?.is_ascii_alphabetic()
            || !scheme_part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
        {
            return None;
        }
        let protocol = format!("{}:", scheme_part.to_ascii_lowercase());
        if matches!(
            protocol.as_str(),
            "http:" | "https:" | "ws:" | "wss:" | "file:" | "ftp:"
        ) {
            return None;
        }
        let path_part = &before_query[ci + 1..];
        if path_part.starts_with('/') || path_part.starts_with('\\') {
            return None;
        }
        let pathname = path_part.to_string();
        let search = if search_raw == "?" {
            String::new()
        } else {
            search_raw.to_string()
        };
        let hash = if hash_raw == "#" {
            String::new()
        } else {
            hash_raw.to_string()
        };
        let href = format!("{}{}{}{}", protocol, pathname, search, hash);
        return Some(ParsedWhatwgUrl {
            href,
            origin: "null".to_string(),
            protocol,
            username: String::new(),
            password: String::new(),
            host: String::new(),
            hostname: String::new(),
            port: String::new(),
            pathname,
            search,
            hash,
        });
    };
    if si == 0 {
        return None;
    }
    let scheme_part = &before_query[..si];
    if !scheme_part.bytes().next()?.is_ascii_alphabetic()
        || !scheme_part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
    {
        return None;
    }
    let protocol = format!("{}:", scheme_part.to_ascii_lowercase());
    let is_special = matches!(
        protocol.as_str(),
        "http:" | "https:" | "ws:" | "wss:" | "file:" | "ftp:"
    );
    let rest = &before_query[si + 3..];
    // For special schemes, WHATWG treats both `/` and `\` as authority-terminating path delimiters,
    // preventing `http://evil.com\@127.0.0.1/` SSRF authority confusion.
    let path_split = if is_special {
        rest.find(['/', '\\'])
    } else {
        rest.find('/')
    };
    let (auth_host, path_raw) = match path_split {
        Some(pi) => (&rest[..pi], &rest[pi..]),
        None => (rest, "/"),
    };

    let (username, password, host_port) = match auth_host.rfind('@') {
        Some(ai) => {
            let up = &auth_host[..ai];
            let hp = &auth_host[ai + 1..];
            match up.find(':') {
                Some(ci) => (up[..ci].to_string(), up[ci + 1..].to_string(), hp),
                None => (up.to_string(), String::new(), hp),
            }
        }
        None => (String::new(), String::new(), auth_host),
    };

    if host_port.is_empty() && is_special && protocol != "file:" {
        return None;
    }

    let ipv6_end = host_port.find(']');
    let (hostname, port) = match host_port.rfind(':') {
        Some(pci) if ipv6_end.is_none_or(|bracket| bracket < pci) => {
            let h = host_port[..pci].to_ascii_lowercase();
            let prt_raw = &host_port[pci + 1..];
            if !prt_raw.is_empty() && !prt_raw.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            if !prt_raw.is_empty() {
                let port_num: u32 = prt_raw.parse().ok()?;
                if port_num > 65535 {
                    return None;
                }
            }
            let prt = if (protocol == "http:" && prt_raw == "80")
                || (protocol == "https:" && prt_raw == "443")
                || (protocol == "ws:" && prt_raw == "80")
                || (protocol == "wss:" && prt_raw == "443")
            {
                String::new()
            } else {
                prt_raw.to_string()
            };
            (h, prt)
        }
        _ => (host_port.to_ascii_lowercase(), String::new()),
    };

    if (is_special || !hostname.is_empty())
        && protocol != "file:"
        && !is_valid_whatwg_hostname(&hostname)
    {
        return None;
    }

    let host = if port.is_empty() {
        hostname.clone()
    } else {
        format!("{}:{}", hostname, port)
    };
    let origin = format!("{}//{}", protocol, host);
    let pathname = normalize_url_pathname(path_raw).into_owned();
    let search = if search_raw == "?" {
        String::new()
    } else {
        search_raw.to_string()
    };
    let hash = if hash_raw == "#" {
        String::new()
    } else {
        hash_raw.to_string()
    };
    let auth = if !username.is_empty() || !password.is_empty() {
        if password.is_empty() {
            format!("{}@", username)
        } else {
            format!("{}:{}@", username, password)
        }
    } else {
        String::new()
    };
    let href = format!(
        "{}//{}{}{}{}{}",
        protocol, auth, host, pathname, search, hash
    );

    Some(ParsedWhatwgUrl {
        href,
        origin,
        protocol,
        username,
        password,
        host,
        hostname,
        port,
        pathname,
        search,
        hash,
    })
}

/// Fast zero-exception check for `URL.canParse(input, base)`.
#[inline]
pub fn can_parse_whatwg_url(input: &str, base: Option<&str>) -> bool {
    parse_whatwg_url(input, base).is_some()
}
