//! Outbound Network Capability Verification, Authority Normalization, and Multi-Layer SSRF Shield.

mod authority;
mod ssrf;

pub(crate) use authority::parse_net_grant_parts;
pub use authority::{
    ParsedHttpUrl, ParsedNetConnectTarget, canonicalize_net_grant_entry, extract_clean_hostname,
    extract_clean_hostname_and_port, parse_http_authority_and_path, parse_net_connect_target,
};
pub use ssrf::{
    is_cloud_metadata_clean_host, is_cloud_metadata_ip, is_cloud_metadata_ssrf_target,
    is_private_or_loopback_clean_host, is_private_or_loopback_ip,
    is_private_or_loopback_ssrf_target, parse_nonstandard_ipv4,
};

use rustc_hash::FxHashSet;

#[inline]
fn write_port_digits(buf: &mut [u8], mut pos: usize, mut port: u16) -> usize {
    let mut digits = [0u8; 5];
    let mut d_len = 0usize;
    if port == 0 {
        digits[0] = b'0';
        d_len = 1;
    } else {
        while port > 0 {
            digits[d_len] = b'0' + (port % 10) as u8;
            port /= 10;
            d_len += 1;
        }
    }
    for i in (0..d_len).rev() {
        buf[pos] = digits[i];
        pos += 1;
    }
    pos
}

#[inline]
fn contains_formatted_host_port(
    allowed_hosts: &FxHashSet<String>,
    clean_host: &str,
    port: u16,
    bracket_ipv6: bool,
) -> bool {
    let mut buf = [0u8; 280];
    let needed = clean_host.len() + if bracket_ipv6 { 8 } else { 6 };
    if needed > buf.len() {
        let heap = if bracket_ipv6 {
            format!("[{}]:{}", clean_host, port)
        } else {
            format!("{}:{}", clean_host, port)
        };
        return allowed_hosts.contains(&heap);
    }
    let mut pos = 0usize;
    if bracket_ipv6 {
        buf[pos] = b'[';
        pos += 1;
    }
    buf[pos..pos + clean_host.len()].copy_from_slice(clean_host.as_bytes());
    pos += clean_host.len();
    if bracket_ipv6 {
        buf[pos] = b']';
        pos += 1;
    }
    buf[pos] = b':';
    pos += 1;
    pos = write_port_digits(&mut buf, pos, port);
    core::str::from_utf8(&buf[..pos]).is_ok_and(|s| allowed_hosts.contains(s))
}

#[inline]
fn contains_bracketed_bare_ipv6(allowed_hosts: &FxHashSet<String>, clean_host: &str) -> bool {
    let mut buf = [0u8; 96];
    if clean_host.len() + 2 > buf.len() {
        return allowed_hosts.contains(&format!("[{}]", clean_host));
    }
    buf[0] = b'[';
    buf[1..1 + clean_host.len()].copy_from_slice(clean_host.as_bytes());
    buf[1 + clean_host.len()] = b']';
    core::str::from_utf8(&buf[..clean_host.len() + 2]).is_ok_and(|s| allowed_hosts.contains(s))
}

#[inline]
fn matches_explicit_host_or_port(
    allowed_hosts: &FxHashSet<String>,
    clean_host: &str,
    port: u16,
) -> bool {
    if allowed_hosts.contains(clean_host) {
        return true;
    }
    let is_ipv6 = memchr::memchr(b':', clean_host.as_bytes()).is_some();
    if is_ipv6 {
        contains_formatted_host_port(allowed_hosts, clean_host, port, true)
            || contains_bracketed_bare_ipv6(allowed_hosts, clean_host)
            || contains_formatted_host_port(allowed_hosts, clean_host, port, false)
    } else {
        contains_formatted_host_port(allowed_hosts, clean_host, port, false)
    }
}

/// Checks whether `(clean_host, port)` is granted by `allowed_hosts`, returning
/// `(explicit_allowed, wildcard_allowed)` using zero-allocation stack `host:port` formatting.
///
/// - Supports bare `host`, `host:port`, IPv6 `[host]:port` / `[host]`, and optional bidirectional
///   `localhost` <-> `127.0.0.1` aliasing (`allow_localhost_alias`).
/// - Cloud metadata targets (`is_cloud_metadata_clean_host`) always return `(false, false)`.
/// - `wildcard_allowed` is `true` only when `!explicit_allowed`, `allowed_hosts` contains `"*"`,
///   and `!is_private_or_loopback_clean_host(clean_host)`.
#[inline]
pub fn check_net_host_grant(
    allowed_hosts: &FxHashSet<String>,
    clean_host: &str,
    port: u16,
    allow_localhost_alias: bool,
) -> (bool, bool) {
    if allowed_hosts.is_empty() || clean_host.is_empty() || is_cloud_metadata_clean_host(clean_host)
    {
        return (false, false);
    }
    let mut explicit = matches_explicit_host_or_port(allowed_hosts, clean_host, port);
    if !explicit
        && allow_localhost_alias
        && (clean_host == "127.0.0.1" || clean_host == "localhost")
    {
        let alt = if clean_host == "127.0.0.1" {
            "localhost"
        } else {
            "127.0.0.1"
        };
        explicit = matches_explicit_host_or_port(allowed_hosts, alt, port);
    }
    if explicit {
        return (true, false);
    }
    let wildcard = allowed_hosts.contains("*") && !is_private_or_loopback_clean_host(clean_host);
    (false, wildcard)
}
