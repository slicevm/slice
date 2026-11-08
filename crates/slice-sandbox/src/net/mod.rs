//! Outbound Network Capability Verification & Multi-Layer SSRF Shield.

use std::borrow::Cow;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Extracts `(clean_lowercase_hostname, explicit_port, scheme_default_port)` from a `host`,
/// `host:port`, `[ipv6]:port`, or full URL string.
///
/// Security (`SBX-SEC-02`, `SBX-SEC-03`) & Performance (`SBX-PERF-01`):
/// - Strips trailing DNS root dots (`"metadata.google.internal."` -> `"metadata.google.internal"`).
/// - Strips IPv6 zone identifiers (`"[fe80::1%eth0]:80"` -> `"fe80::1"`).
/// - Returns `Cow::Borrowed` whenever the hostname slice is already lowercase ASCII (zero heap allocation).
#[inline]
pub fn extract_clean_hostname_and_port(host: &str) -> (Cow<'_, str>, Option<u16>, Option<u16>) {
    let mut trimmed = host.trim();
    let mut scheme_default_port = None;
    if let Some((scheme, after_scheme)) = trimmed.split_once("://") {
        if scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("wss") {
            scheme_default_port = Some(443);
        } else if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("ws") {
            scheme_default_port = Some(80);
        }
        trimmed = after_scheme;
    }
    if let Some((authority, _)) = trimmed.split_once(['/', '?', '#']) {
        trimmed = authority;
    }
    if let Some((_, after_auth)) = trimmed.rsplit_once('@') {
        trimmed = after_auth;
    }

    let mut explicit_port = None;
    let without_port = if let Some(rest) = trimmed.strip_prefix('[') {
        if let Some((ipv6, after_bracket)) = rest.split_once(']') {
            if let Some(port_str) = after_bracket.strip_prefix(':') {
                if let Ok(p) = port_str.parse::<u16>() {
                    explicit_port = Some(p);
                }
            }
            // Strip RFC 6874 / OS IPv6 zone identifier (`%eth0` or `%25eth0`)
            if let Some((v6_clean, _zone)) = ipv6.split_once('%') {
                v6_clean
            } else {
                ipv6
            }
        } else {
            trimmed
        }
    } else if let Some((h, port_part)) = trimmed.rsplit_once(':') {
        if !h.contains(':')
            && !port_part.is_empty()
            && port_part.bytes().all(|b| b.is_ascii_digit())
        {
            if let Ok(p) = port_part.parse::<u16>() {
                explicit_port = Some(p);
            }
            h
        } else if let Some((v6_clean, _zone)) = trimmed.split_once('%') {
            v6_clean
        } else {
            trimmed
        }
    } else {
        trimmed
    };

    let without_trailing_dot = without_port.trim_end_matches('.');
    let clean_cow = if without_trailing_dot.bytes().any(|b| b.is_ascii_uppercase()) {
        Cow::Owned(without_trailing_dot.to_ascii_lowercase())
    } else {
        Cow::Borrowed(without_trailing_dot)
    };

    (clean_cow, explicit_port, scheme_default_port)
}

/// Extracts the lowercase hostname (without scheme, path, `:port`, trailing dot, or IPv6 brackets)
/// from a `host`, `host:port`, or URL string.
#[inline]
pub fn extract_clean_hostname(host: &str) -> String {
    extract_clean_hostname_and_port(host).0.into_owned()
}

/// Attempts to parse non-canonical IPv4 literal strings (octal `0177.0.0.1`, hex `0x7f.0.0.1`,
/// integer/dword `2130706433`, or shorthand 2/3-part dotted quads like `127.1` or `169.254.43518`)
/// into a canonical `std::net::Ipv4Addr` on the stack (`SBX-PERF-01` zero heap allocation)
/// so SSRF guards cannot be bypassed via alternative IP encodings.
#[inline]
pub fn parse_nonstandard_ipv4(h: &str) -> Option<Ipv4Addr> {
    let bytes = h.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_digit() {
        return None;
    }
    let mut vals = [0u32; 4];
    let mut count = 0usize;

    for part in h.split('.') {
        if part.is_empty() || count >= 4 {
            return None;
        }
        let val = if let Some(hex) = part.strip_prefix("0x").or_else(|| part.strip_prefix("0X")) {
            u32::from_str_radix(hex, 16).ok()?
        } else if part.len() > 1
            && part.starts_with('0')
            && part.bytes().all(|b| b.is_ascii_digit())
        {
            u32::from_str_radix(part, 8).ok()?
        } else {
            part.parse::<u32>().ok()?
        };
        vals[count] = val;
        count += 1;
    }

    let ip_u32 = match count {
        1 => vals[0],
        2 if vals[0] <= 0xff && vals[1] <= 0x00ff_ffff => (vals[0] << 24) | vals[1],
        3 if vals[0] <= 0xff && vals[1] <= 0xff && vals[2] <= 0xffff => {
            (vals[0] << 24) | (vals[1] << 16) | vals[2]
        }
        4 if vals[0] <= 0xff && vals[1] <= 0xff && vals[2] <= 0xff && vals[3] <= 0xff => {
            (vals[0] << 24) | (vals[1] << 16) | (vals[2] << 8) | vals[3]
        }
        _ => return None,
    };
    Some(Ipv4Addr::from(ip_u32))
}

/// Extracts an embedded IPv4 address from IPv6 transition/tunneling mechanisms (`SBX-SEC-03`):
/// - IPv4-mapped IPv6 (`::ffff:a.b.c.d`)
/// - IPv4-compatible IPv6 (`::a.b.c.d`, excluding `::` and `::1`)
/// - RFC 6052 Well-Known NAT64 (`64:ff9b::a.b.c.d`) and RFC 8215 Local-Use NAT64 (`64:ff9b:1::a.b.c.d`)
/// - RFC 3056 6to4 (`2002:aabb:ccdd::/48`)
#[inline]
fn extract_embedded_ipv4(v6: Ipv6Addr) -> Option<Ipv4Addr> {
    if let Some(mapped) = v6.to_ipv4_mapped() {
        return Some(mapped);
    }
    let segs = v6.segments();
    // IPv4-compatible `::a.b.c.d` (excluding `::` and `::1`)
    if segs[0..6] == [0, 0, 0, 0, 0, 0] && (segs[6] != 0 || segs[7] > 1) {
        return Some(Ipv4Addr::new(
            (segs[6] >> 8) as u8,
            segs[6] as u8,
            (segs[7] >> 8) as u8,
            segs[7] as u8,
        ));
    }
    // NAT64 Well-Known `64:ff9b::/96` or Local-Use `64:ff9b:1::/48`
    if segs[0] == 0x0064
        && segs[1] == 0xff9b
        && (segs[2] == 0 || segs[2] == 1)
        && segs[3] == 0
        && segs[4] == 0
        && segs[5] == 0
    {
        return Some(Ipv4Addr::new(
            (segs[6] >> 8) as u8,
            segs[6] as u8,
            (segs[7] >> 8) as u8,
            segs[7] as u8,
        ));
    }
    // 6to4 `2002::/16` (embeds IPv4 in segs[1] and segs[2])
    if segs[0] == 0x2002 {
        return Some(Ipv4Addr::new(
            (segs[1] >> 8) as u8,
            segs[1] as u8,
            (segs[2] >> 8) as u8,
            segs[2] as u8,
        ));
    }
    None
}

/// Returns `true` if `host` resolves to a cloud instance metadata endpoint
/// (AWS/GCP/Azure/DigitalOcean `169.254.169.254`, AWS ECS `169.254.170.2`,
/// Alibaba `100.100.100.200`, AWS IPv6 `fd00:ec2::254`, or GCP `metadata.google.internal`).
/// These endpoints are unconditionally blocked by the Slice SSRF shield.
#[inline]
pub fn is_cloud_metadata_ssrf_target(host: &str) -> bool {
    let (h, _, _) = extract_clean_hostname_and_port(host);
    if matches!(
        h.as_ref(),
        "169.254.169.254"
            | "169.254.170.2"
            | "100.100.100.200"
            | "fd00:ec2::254"
            | "metadata.google.internal"
            | "metadata.goog"
            | "instance-data"
    ) || h.ends_with(".metadata.google.internal")
    {
        return true;
    }
    if let Some(v4) = parse_nonstandard_ipv4(&h) {
        if is_cloud_metadata_ip(IpAddr::V4(v4)) {
            return true;
        }
    }
    if h.contains(':') {
        if let Ok(ip) = h.parse::<IpAddr>() {
            if is_cloud_metadata_ip(ip) {
                return true;
            }
        }
    }
    false
}

/// Returns `true` if `host` is a cloud metadata endpoint, loopback address (`localhost`, `127.0.0.0/8`, `::1`),
/// link-local (`169.254.0.0/16`, `fe80::/10`), RFC1918 private network (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`),
/// CGNAT (`100.64.0.0/10`), benchmark (`198.18.0.0/15`), IETF protocol (`192.0.0.0/24`), multicast (`224.0.0.0/4`, `ff00::/8`),
/// IPv6 Unique Local (`fc00::/7`), or any non-canonical numeric IP representation (octal, hex, dword, shorthand IPv4).
/// Wildcard `allow_net = ["*"]` never grants access to these targets; they require explicit host allowlisting.
#[inline]
pub fn is_private_or_loopback_ssrf_target(host: &str) -> bool {
    let (h, _, _) = extract_clean_hostname_and_port(host);
    if matches!(
        h.as_ref(),
        "169.254.169.254"
            | "169.254.170.2"
            | "100.100.100.200"
            | "fd00:ec2::254"
            | "metadata.google.internal"
            | "metadata.goog"
            | "instance-data"
            | "localhost"
            | "0.0.0.0"
            | "::"
            | "::1"
            | "0:0:0:0:0:0:0:1"
            | "0:0:0:0:0:0:0:0"
    ) || h.ends_with(".localhost")
        || h.ends_with(".metadata.google.internal")
    {
        return true;
    }
    if let Ok(ip) = h.parse::<IpAddr>() {
        return is_private_or_loopback_ip(ip);
    }
    if parse_nonstandard_ipv4(&h).is_some() {
        // Any non-canonical numeric IPv4 literal (octal `0177.0.0.1`, hex `0x7f.0.0.1`,
        // dword `2130706433`, or shorthand `127.1`) is unconditionally blocked under wildcard allow_net.
        return true;
    }
    false
}

/// Returns `true` if `ip` is a cloud instance metadata IP address (`169.254.169.254`,
/// `169.254.170.2`, `100.100.100.200`, `fd00:ec2::254`, or their IPv6-embedded equivalents).
#[inline]
pub fn is_cloud_metadata_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let octs = v4.octets();
            octs == [169, 254, 169, 254]
                || octs == [169, 254, 170, 2]
                || octs == [100, 100, 100, 200]
        }
        IpAddr::V6(v6) => {
            if let Some(embedded_v4) = extract_embedded_ipv4(v6) {
                if is_cloud_metadata_ip(IpAddr::V4(embedded_v4)) {
                    return true;
                }
            }
            let segs = v6.segments();
            segs == [0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254]
        }
    }
}

/// Returns `true` if `ip` is a cloud metadata IP, loopback (`127.0.0.0/8`, `::1`),
/// unspecified (`0.0.0.0`, `::`), link-local (`169.254.0.0/16`, `fe80::/10`),
/// RFC 1918 private (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`),
/// CGNAT (`100.64.0.0/10`), IETF protocol (`192.0.0.0/24`), TEST-NET (`192.0.2.0/24`,
/// `198.51.100.0/24`, `203.0.113.0/24`), benchmark (`198.18.0.0/15`), multicast (`224.0.0.0/4`, `ff00::/8`),
/// reserved (`240.0.0.0/4`), unique local (`fc00::/7`), or IPv6-embedded equivalent.
#[inline]
pub fn is_private_or_loopback_ip(ip: IpAddr) -> bool {
    if is_cloud_metadata_ip(ip) {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => {
            let octs = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || (octs[0] == 100 && (octs[1] & 0b1100_0000) == 64) // 100.64.0.0/10 CGNAT
                || (octs[0] == 192 && octs[1] == 0 && octs[2] == 0) // 192.0.0.0/24 IETF Protocol
                || (octs[0] == 198 && (octs[1] == 18 || octs[1] == 19)) // 198.18.0.0/15 Benchmark
                || octs[0] >= 240 // 240.0.0.0/4 Reserved for future use
        }
        IpAddr::V6(v6) => {
            if let Some(embedded_v4) = extract_embedded_ipv4(v6) {
                return is_private_or_loopback_ip(IpAddr::V4(embedded_v4));
            }
            let segs = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (segs[0] & 0xfe00) == 0xfc00 // fc00::/7 Unique Local
                || (segs[0] & 0xffc0) == 0xfe80 // fe80::/10 Link-Local
                || (segs[0] == 0x2001 && segs[1] == 0x0db8) // 2001:db8::/32 Documentation
        }
    }
}
