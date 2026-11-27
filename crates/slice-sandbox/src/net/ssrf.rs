//! Multi-Layer Cloud Metadata, Private/Loopback CIDR, and Non-Canonical IP SSRF Shield.

use super::extract_clean_hostname_and_port;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Returns `true` when `h` (an unbracketed non-IPv6 hostname with trailing dots already stripped)
/// ends in a purely numeric or `0x`-hex label per WHATWG URL Standard "ends in a number checker".
#[inline]
pub(super) fn host_ends_in_number(h: &str) -> bool {
    if h.is_empty() || h.contains(':') {
        return false;
    }
    let last = h.rsplit('.').next().unwrap_or(h);
    if last.is_empty() {
        return false;
    }
    if last.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    last.strip_prefix("0x")
        .or_else(|| last.strip_prefix("0X"))
        .is_some_and(|hex| hex.is_empty() || hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Parses 4-part decimal IPv4 strings that include leading zeros with digits `8` or `9`
/// (such as `127.0.0.08` or `0169.0254.0169.0254`) so metadata/loopback classifiers detect
/// non-octal leading-zero IPv4 strings regardless of downstream resolver quirks.
#[inline]
fn parse_leading_zero_decimal_ipv4(h: &str) -> Option<Ipv4Addr> {
    let bytes = h.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_digit() {
        return None;
    }
    let mut octs = [0u8; 4];
    let mut count = 0usize;
    for part in h.split('.') {
        if part.is_empty() || count >= 4 || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let val = part.parse::<u16>().ok()?;
        if val > 255 {
            return None;
        }
        octs[count] = val as u8;
        count += 1;
    }
    if count == 4 {
        Some(Ipv4Addr::from(octs))
    } else {
        None
    }
}

/// Returns `true` when `h` ends in a numeric label (per WHATWG URL rules) yet is neither a
/// canonical `Ipv4Addr` nor a valid non-standard octal/hex/shorthand `Ipv4Addr` (for example,
/// `127.0.0.08`, `0169.0254.0169.0254`, or `999.999.999.999`).
#[inline]
pub(super) fn is_malformed_numeric_host(h: &str) -> bool {
    host_ends_in_number(h) && h.parse::<Ipv4Addr>().is_err() && parse_nonstandard_ipv4(h).is_none()
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
/// - SIIT IPv4-translated IPv6 (`::ffff:0:a.b.c.d`, RFC 2765)
/// - IPv4-compatible IPv6 (`::a.b.c.d`, excluding `::` and `::1`)
/// - RFC 6052 Well-Known NAT64 (`64:ff9b::a.b.c.d`) and RFC 8215 Local-Use NAT64 (`64:ff9b:1::/48`)
/// - RFC 3056 6to4 (`2002:aabb:ccdd::/48`)
/// - RFC 4380 Teredo (`2001:0000::/32`, XOR-obfuscated client IPv4 in `segs[6..8]`)
#[inline]
fn extract_embedded_ipv4(v6: Ipv6Addr) -> Option<Ipv4Addr> {
    if let Some(mapped) = v6.to_ipv4_mapped() {
        return Some(mapped);
    }
    let segs = v6.segments();
    // SIIT IPv4-translated `::ffff:0:a.b.c.d` (`::ffff:0:0/96`, RFC 2765)
    if segs[0..4] == [0, 0, 0, 0] && segs[4] == 0xffff && segs[5] == 0 {
        return Some(Ipv4Addr::new(
            (segs[6] >> 8) as u8,
            segs[6] as u8,
            (segs[7] >> 8) as u8,
            segs[7] as u8,
        ));
    }
    // IPv4-compatible `::a.b.c.d` (excluding `::` and `::1`)
    if segs[0..6] == [0, 0, 0, 0, 0, 0] && (segs[6] != 0 || segs[7] > 1) {
        return Some(Ipv4Addr::new(
            (segs[6] >> 8) as u8,
            segs[6] as u8,
            (segs[7] >> 8) as u8,
            segs[7] as u8,
        ));
    }
    // NAT64 Well-Known `64:ff9b::/96` or RFC 8215 Local-Use `64:ff9b:1::/48`
    if segs[0] == 0x0064
        && segs[1] == 0xff9b
        && ((segs[2] == 0 && segs[3] == 0 && segs[4] == 0 && segs[5] == 0) || segs[2] == 0x0001)
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
    // Teredo `2001:0000::/32` (client IPv4 XOR-obfuscated with 0xffff in segs[6..8])
    if segs[0] == 0x2001 && segs[1] == 0x0000 {
        let s6 = !segs[6];
        let s7 = !segs[7];
        return Some(Ipv4Addr::new(
            (s6 >> 8) as u8,
            s6 as u8,
            (s7 >> 8) as u8,
            s7 as u8,
        ));
    }
    None
}

/// Checks an already-cleaned lowercase hostname `h` (no brackets, port, or trailing dot)
/// against cloud instance metadata endpoints without re-parsing `h`.
#[inline]
pub fn is_cloud_metadata_clean_host(h: &str) -> bool {
    if h.is_empty() {
        return false;
    }
    if matches!(
        h,
        "169.254.169.254"
            | "169.254.170.2"
            | "169.254.170.23"
            | "168.63.129.16"
            | "100.100.100.200"
            | "fd00:ec2::254"
            | "fd00:ec2::23"
            | "metadata.google.internal"
            | "metadata.goog"
            | "instance-data"
    ) || h.ends_with(".metadata.google.internal")
    {
        return true;
    }
    if let Some(v4) = parse_nonstandard_ipv4(h).or_else(|| parse_leading_zero_decimal_ipv4(h))
        && is_cloud_metadata_ip(IpAddr::V4(v4))
    {
        return true;
    }
    h.contains(':') && h.parse::<IpAddr>().is_ok_and(is_cloud_metadata_ip)
}

/// Checks an already-cleaned lowercase hostname `h` (no brackets, port, or trailing dot)
/// against cloud metadata, loopback, link-local, RFC1918, CGNAT, special-use CIDRs, and
/// non-canonical/malformed numeric IPv4 representations without re-parsing `h`.
#[inline]
pub fn is_private_or_loopback_clean_host(h: &str) -> bool {
    if h.is_empty() || is_cloud_metadata_clean_host(h) {
        return true;
    }
    if matches!(
        h,
        "localhost" | "0.0.0.0" | "::" | "::1" | "0:0:0:0:0:0:0:1" | "0:0:0:0:0:0:0:0"
    ) || h.ends_with(".localhost")
    {
        return true;
    }
    if let Ok(ip) = h.parse::<IpAddr>() {
        return is_private_or_loopback_ip(ip);
    }
    // Any non-canonical or malformed numeric IPv4 literal (octal `0177.0.0.1`,
    // leading-zero non-octal `127.0.0.08`, hex `0x7f.0.0.1`, dword `2130706433`,
    // or shorthand `127.1`) is unconditionally blocked under wildcard allow_net.
    parse_nonstandard_ipv4(h).is_some() || host_ends_in_number(h)
}

/// Returns `true` if `host` resolves to a cloud instance metadata endpoint.
/// These endpoints are unconditionally blocked by the Slice SSRF shield.
#[inline]
pub fn is_cloud_metadata_ssrf_target(host: &str) -> bool {
    let (h, _, _) = extract_clean_hostname_and_port(host);
    is_cloud_metadata_clean_host(&h)
}

/// Returns `true` if `host` is a cloud metadata endpoint, loopback address, link-local,
/// RFC1918 private network, CGNAT, benchmark, IETF protocol, multicast, IPv6 Unique Local,
/// or any non-canonical/malformed numeric IP representation.
/// Wildcard `allow_net = ["*"]` never grants access to these targets.
#[inline]
pub fn is_private_or_loopback_ssrf_target(host: &str) -> bool {
    let (h, _, _) = extract_clean_hostname_and_port(host);
    is_private_or_loopback_clean_host(&h)
}

/// Returns `true` if `ip` is a cloud instance metadata IP address (`169.254.169.254`,
/// `169.254.170.2`, `169.254.170.23`, `168.63.129.16`, `100.100.100.200`, `fd00:ec2::254`,
/// `fd00:ec2::23`, or their IPv6-embedded equivalents).
#[inline]
pub fn is_cloud_metadata_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let octs = v4.octets();
            octs == [169, 254, 169, 254]
                || octs == [169, 254, 170, 2]
                || octs == [169, 254, 170, 23]
                || octs == [168, 63, 129, 16]
                || octs == [100, 100, 100, 200]
        }
        IpAddr::V6(v6) => {
            if let Some(embedded_v4) = extract_embedded_ipv4(v6)
                && is_cloud_metadata_ip(IpAddr::V4(embedded_v4))
            {
                return true;
            }
            let segs = v6.segments();
            segs == [0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254]
                || segs == [0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0023]
        }
    }
}

/// Returns `true` if `ip` is a loopback address (`127.0.0.0/8`, `::1`, or an IPv6 address
/// embedding an IPv4 loopback address such as `::ffff:127.0.0.1`).
#[inline]
pub fn is_loopback_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || extract_embedded_ipv4(v6).is_some_and(|embedded_v4| embedded_v4.is_loopback())
        }
    }
}

/// Returns `true` if `ip` is a cloud metadata IP, loopback, unspecified, link-local,
/// RFC 1918 private, CGNAT, IETF protocol, TEST-NET, benchmark, multicast, reserved,
/// unique local, deprecated site-local, or IPv6-embedded equivalent.
#[inline]
pub fn is_private_or_loopback_ip(ip: IpAddr) -> bool {
    if is_cloud_metadata_ip(ip) {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => {
            let octs = v4.octets();
            octs[0] == 0 // 0.0.0.0/8 ("This network", RFC 1122 / Linux 0.0.0.1 loopback)
                || v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || (octs[0] == 100 && (octs[1] & 0b1100_0000) == 64) // 100.64.0.0/10 CGNAT
                || (octs[0] == 192 && octs[1] == 0 && octs[2] == 0) // 192.0.0.0/24 IETF Protocol
                || (octs[0] == 192 && octs[1] == 88 && octs[2] == 99) // 192.88.99.0/24 6to4 Relay Anycast
                || (octs[0] == 198 && (octs[1] == 18 || octs[1] == 19)) // 198.18.0.0/15 Benchmark
                || octs[0] >= 240 // 240.0.0.0/4 Reserved for future use
        }
        IpAddr::V6(v6) => {
            let segs = v6.segments();
            // Unconditionally block RFC 8215 Local-Use NAT64 (`64:ff9b:1::/48`),
            // SIIT (`::ffff:0:0/96`), and RFC 4380 Teredo (`2001:0000::/32`).
            if (segs[0] == 0x0064 && segs[1] == 0xff9b && segs[2] == 0x0001)
                || (segs[0..4] == [0, 0, 0, 0] && segs[4] == 0xffff && segs[5] == 0)
                || (segs[0] == 0x2001 && segs[1] == 0x0000)
            {
                return true;
            }
            if let Some(embedded_v4) = extract_embedded_ipv4(v6) {
                return is_private_or_loopback_ip(IpAddr::V4(embedded_v4));
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (segs[0] & 0xfe00) == 0xfc00 // fc00::/7 Unique Local (RFC 4193)
                || (segs[0] & 0xffc0) == 0xfe80 // fe80::/10 Link-Local
                || (segs[0] & 0xffc0) == 0xfec0 // fec0::/10 Deprecated Site-Local (RFC 3879)
                || (segs[0] == 0x0100 && segs[1] == 0 && segs[2] == 0 && segs[3] == 0) // 100::/64 Discard-Only (RFC 6666)
                || (segs[0] == 0x2001 && segs[1] == 0x0002 && segs[2] == 0) // 2001:2::/48 Benchmarking (RFC 5180)
                || (segs[0] == 0x2001
                    && ((segs[1] & 0xfff0) == 0x0010 || (segs[1] & 0xfff0) == 0x0020)) // 2001:10::/28 & 2001:20::/28 ORCHID
                || (segs[0] == 0x2001 && segs[1] == 0x0db8) // 2001:db8::/32 Documentation (RFC 3849)
                || (segs[0] == 0x3fff && (segs[1] & 0xf000) == 0) // 3fff::/20 Documentation (RFC 9637)
        }
    }
}
