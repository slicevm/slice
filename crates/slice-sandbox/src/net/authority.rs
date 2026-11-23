//! WHATWG HTTP/WebSocket Authority, Hostname, and `:port` Normalization.

use super::ssrf::is_malformed_numeric_host;
use std::borrow::Cow;
use std::net::Ipv6Addr;

#[inline]
fn decode_hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[inline]
fn percent_decode_host_cow(raw: &str) -> Option<Cow<'_, str>> {
    let bytes = raw.as_bytes();
    if !bytes.contains(&b'%') {
        return Some(Cow::Borrowed(raw));
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hi = decode_hex_nibble(bytes[i + 1])?;
            let lo = decode_hex_nibble(bytes[i + 2])?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok().map(Cow::Owned)
}

/// Validates a decoded hostname (or unbracketed IPv6 literal when `is_ipv6 == true`) and returns
/// `Some(has_ascii_uppercase)` in a single pass, or `None` if invalid.
#[inline]
fn validate_and_has_uppercase_host(h: &str, is_ipv6: bool) -> Option<bool> {
    if h.is_empty() {
        return None;
    }
    let mut has_upper = false;
    for b in h.bytes() {
        if b <= 0x20
            || b == 0x7F
            || matches!(
                b,
                b'/' | b'\\' | b'?' | b'#' | b'@' | b'[' | b']' | b'<' | b'>' | b'|' | b'^' | b'`'
            )
        {
            return None;
        }
        if !is_ipv6 && b == b':' {
            return None;
        }
        if b.is_ascii_uppercase() {
            has_upper = true;
        }
    }
    if is_ipv6 && h.parse::<Ipv6Addr>().is_err() {
        return None;
    }
    Some(has_upper)
}

/// Shared authority parser returning `(clean_lowercase_host, is_ipv6, explicit_port)` across
/// [`parse_http_authority_and_path`], [`parse_net_connect_target`], and [`extract_clean_hostname_and_port`].
#[inline]
fn parse_authority_host_port(
    authority: &str,
    scheme_default_port: Option<u16>,
    allow_unbracketed_ipv6: bool,
    reject_malformed_numeric: bool,
) -> Option<(Cow<'_, str>, bool, Option<u16>)> {
    if authority.is_empty() {
        return None;
    }
    let mut explicit_port = None;
    let mut is_ipv6 = false;
    let host_part = if let Some(rest) = authority.strip_prefix('[') {
        let (ipv6_raw, after_bracket) = rest.split_once(']')?;
        is_ipv6 = true;
        if !after_bracket.is_empty() {
            let port_str = after_bracket.strip_prefix(':')?;
            if port_str.is_empty() {
                scheme_default_port?;
            } else {
                let p = port_str.parse::<u16>().ok()?;
                if p == 0 {
                    return None;
                }
                explicit_port = Some(p);
            }
        }
        match ipv6_raw.split_once('%') {
            Some((v6_clean, _zone)) => v6_clean,
            None => ipv6_raw,
        }
    } else if let Some((h, port_part)) = authority.rsplit_once(':') {
        if memchr::memchr(b':', h.as_bytes()).is_some() {
            if !allow_unbracketed_ipv6 {
                return None;
            }
            is_ipv6 = true;
            match authority.split_once('%') {
                Some((v6_clean, _zone)) => v6_clean,
                None => authority,
            }
        } else if port_part.is_empty() {
            scheme_default_port?;
            h
        } else {
            let p = port_part.parse::<u16>().ok()?;
            if p == 0 {
                return None;
            }
            explicit_port = Some(p);
            h
        }
    } else {
        authority
    };

    if is_ipv6 {
        let has_upper = validate_and_has_uppercase_host(host_part, true)?;
        let clean = if has_upper {
            Cow::Owned(host_part.to_ascii_lowercase())
        } else {
            Cow::Borrowed(host_part)
        };
        return Some((clean, true, explicit_port));
    }

    if memchr::memchr(b'%', host_part.as_bytes()).is_some() {
        let mut s = percent_decode_host_cow(host_part)?.into_owned();
        while s.ends_with('.') {
            s.pop();
        }
        validate_and_has_uppercase_host(&s, false)?;
        if reject_malformed_numeric && is_malformed_numeric_host(&s) {
            return None;
        }
        s.make_ascii_lowercase();
        return Some((Cow::Owned(s), false, explicit_port));
    }

    let no_dot = host_part.trim_end_matches('.');
    let has_upper = validate_and_has_uppercase_host(no_dot, false)?;
    if reject_malformed_numeric && is_malformed_numeric_host(no_dot) {
        return None;
    }
    let clean = if has_upper {
        Cow::Owned(no_dot.to_ascii_lowercase())
    } else {
        Cow::Borrowed(no_dot)
    };
    Some((clean, false, explicit_port))
}

/// Parsed and normalized HTTP/WebSocket URL authority and request target shared across
/// `slice-sandbox` OCap checks and `slice-engine` outbound `fetch()` / `WebSocket` clients.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedHttpUrl<'a> {
    pub is_secure: bool,
    pub clean_host: Cow<'a, str>,
    pub is_ipv6: bool,
    pub port: u16,
    pub default_port: u16,
    pub path_and_query: Cow<'a, str>,
}

/// Parses an `http://`, `https://`, `ws://`, or `wss://` URL using unified WHATWG authority
/// delimiter rules (`['/', '?', '#', '\\']`), rejecting userinfo `@`, empty hostnames, control
/// characters, and invalid ports, while percent-decoding `%XX` hostnames.
pub fn parse_http_authority_and_path(raw_url: &str) -> Option<ParsedHttpUrl<'_>> {
    let trimmed = raw_url.trim();
    if trimmed.is_empty() || trimmed.bytes().any(|b| b <= 0x20 || b == 0x7F) {
        return None;
    }
    let (scheme, after_scheme) = trimmed.split_once("://")?;
    let (is_secure, default_port) =
        if scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("wss") {
            (true, 443u16)
        } else if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("ws") {
            (false, 80u16)
        } else {
            return None;
        };

    let without_hash = after_scheme.split('#').next().unwrap_or(after_scheme);
    let (authority, path_and_query) = match without_hash.find(['/', '?', '\\']) {
        Some(idx) => {
            let auth = &without_hash[..idx];
            let delim = without_hash.as_bytes()[idx];
            let pq = match delim {
                b'/' => Cow::Borrowed(&without_hash[idx..]),
                b'?' => Cow::Owned(format!("/{}", &without_hash[idx..])),
                b'\\' => Cow::Owned(format!("/{}", &without_hash[idx + 1..])),
                _ => Cow::Borrowed("/"),
            };
            (auth, pq)
        }
        None => (without_hash, Cow::Borrowed("/")),
    };

    if authority.is_empty() || authority.contains('@') {
        return None;
    }

    let (clean_host, is_ipv6, explicit_port) =
        parse_authority_host_port(authority, Some(default_port), false, true)?;

    Some(ParsedHttpUrl {
        is_secure,
        clean_host,
        is_ipv6,
        port: explicit_port.unwrap_or(default_port),
        default_port,
        path_and_query,
    })
}

/// Single-pass parsed network connection target for [`crate::TenantCompartment::verify_net_connect`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedNetConnectTarget<'a> {
    pub clean_host: Cow<'a, str>,
    pub is_ipv6: bool,
    pub explicit_port: Option<u16>,
    pub scheme_default_port: Option<u16>,
}

/// Parses a network connection target (`host`, `host:port`, `[ipv6]`, `[ipv6]:port`, or full
/// `http(s)://` / `ws(s)://` URL) in a single pass, rejecting userinfo `@`, percent-decoded
/// delimiters/control bytes (`%40`, `%2f`, `%00`, `%0d%0a`), invalid/`:0` ports, and malformed
/// numeric IPv4 literals (`127.0.0.08`, `0169.0254.0169.0254`).
#[inline]
pub fn parse_net_connect_target(raw: &str) -> Option<ParsedNetConnectTarget<'_>> {
    let trimmed = raw.trim();
    let tb = trimmed.as_bytes();
    if tb.is_empty() {
        return None;
    }
    if memchr::memmem::find(tb, b"://").is_some() {
        let parsed = parse_http_authority_and_path(trimmed)?;
        return Some(ParsedNetConnectTarget {
            clean_host: parsed.clean_host,
            is_ipv6: parsed.is_ipv6,
            explicit_port: Some(parsed.port),
            scheme_default_port: Some(parsed.default_port),
        });
    }
    for &b in tb {
        if b <= 0x20 || b == 0x7F || matches!(b, b'@' | b'/' | b'?' | b'#' | b'\\') {
            return None;
        }
    }
    let (clean_host, is_ipv6, explicit_port) =
        parse_authority_host_port(trimmed, None, true, true)?;
    Some(ParsedNetConnectTarget {
        clean_host,
        is_ipv6,
        explicit_port,
        scheme_default_port: None,
    })
}

/// Canonicalizes a `CapabilityToken` `allow_net` grant entry (`"*"`, `host`, `host:port`, `[ipv6]`,
/// `[ipv6]:port`, or full `http(s)://` URL) into lowercase canonical form.
#[inline]
pub fn canonicalize_net_grant_entry(raw: &str) -> Option<String> {
    let (host, is_ipv6, port_opt) = parse_net_grant_parts(raw)?;
    if host == "*" {
        return Some("*".to_string());
    }
    match port_opt {
        Some(port) if is_ipv6 => Some(format!("[{}]:{}", host, port)),
        Some(port) => Some(format!("{}:{}", host, port)),
        None => Some(host),
    }
}

/// Parses an `allow_net` entry into `(canonical_host, is_ipv6, explicit_port)`.
/// For wildcard `"*"`, returns `Some(("*".to_string(), false, None))`.
#[inline]
pub(crate) fn parse_net_grant_parts(raw: &str) -> Option<(String, bool, Option<u16>)> {
    let trimmed = raw.trim();
    if trimmed == "*" {
        return Some(("*".to_string(), false, None));
    }
    let parsed = parse_net_connect_target(trimmed)?;
    let canon_host = if parsed.is_ipv6 {
        let v6: Ipv6Addr = parsed.clean_host.parse().ok()?;
        v6.to_string()
    } else {
        parsed.clean_host.into_owned()
    };
    Some((canon_host, parsed.is_ipv6, parsed.explicit_port))
}

/// Extracts `(clean_lowercase_hostname, explicit_port, scheme_default_port)` from a `host`,
/// `host:port`, `[ipv6]:port`, or full URL string.
#[inline]
pub fn extract_clean_hostname_and_port(host: &str) -> (Cow<'_, str>, Option<u16>, Option<u16>) {
    let mut trimmed = host.trim();
    if trimmed.is_empty() {
        return (Cow::Borrowed(""), None, None);
    }
    let mut scheme_default_port = None;
    if let Some(idx) = memchr::memmem::find(trimmed.as_bytes(), b"://") {
        let scheme = &trimmed[..idx];
        if scheme.bytes().any(|b| b <= 0x20 || b == 0x7F) {
            return (Cow::Borrowed(""), None, None);
        }
        if scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("wss") {
            scheme_default_port = Some(443);
        } else if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("ws") {
            scheme_default_port = Some(80);
        }
        trimmed = &trimmed[idx + 3..];
    }
    let tb = trimmed.as_bytes();
    let mut auth_end = tb.len();
    let mut at_idx = None;
    let mut i = 0usize;
    while i < tb.len() {
        let b = tb[i];
        if matches!(b, b'/' | b'?' | b'#' | b'\\') {
            auth_end = i;
            break;
        }
        if b <= 0x20 || b == 0x7F {
            return (Cow::Borrowed(""), None, scheme_default_port);
        }
        if b == b'@' {
            at_idx = Some(i);
        }
        i += 1;
    }
    let authority = match at_idx {
        Some(at) => &trimmed[at + 1..auth_end],
        None => &trimmed[..auth_end],
    };
    match parse_authority_host_port(authority, scheme_default_port, true, false) {
        Some((clean_cow, _is_ipv6, explicit_port)) => {
            (clean_cow, explicit_port, scheme_default_port)
        }
        None => (Cow::Borrowed(""), None, scheme_default_port),
    }
}

/// Extracts the lowercase hostname (without scheme, path, `:port`, trailing dot, or IPv6 brackets)
/// from a `host`, `host:port`, or URL string.
#[inline]
pub fn extract_clean_hostname(host: &str) -> String {
    extract_clean_hostname_and_port(host).0.into_owned()
}
