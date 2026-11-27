//! Outbound HTTP URL Parsing, OCap `allow_net` Host Verification, Pre-Connect DNS SSRF Firewall, and Egress DLP.

use super::OutboundUrlParts;
use crate::bindings::helpers::{active_tenant_id_or_default, record_boa_violation_once};
use crate::boa_realm::{
    BOA_ALLOW_PRIVATE_NETS, BOA_EGRESS_SECRETS, BOA_NET_HOSTS, record_boa_ocap_denial,
};
use crate::state_and_kv::verify_egress_secrets_slice;
use slice_sandbox::{
    SandboxViolation, check_net_host_grant, is_cloud_metadata_clean_host, is_cloud_metadata_ip,
    is_loopback_ip, is_private_or_loopback_clean_host, is_private_or_loopback_ip,
};
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};

pub(crate) fn parse_outbound_http_url(raw_url: &str) -> Option<OutboundUrlParts> {
    let trimmed = raw_url.trim();
    let (scheme, _) = trimmed.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let parsed = slice_sandbox::parse_http_authority_and_path(trimmed)?;
    let clean_host = parsed.clean_host.into_owned();
    let bracketed_host = if parsed.is_ipv6 {
        format!("[{}]", clean_host)
    } else {
        clean_host.clone()
    };
    let host_port = format!("{}:{}", bracketed_host, parsed.port);
    let host_header = if parsed.port == parsed.default_port {
        bracketed_host
    } else {
        host_port.clone()
    };
    Some(OutboundUrlParts {
        is_https: parsed.is_secure,
        hostname: clean_host,
        port: parsed.port,
        host_header,
        host_port,
        path_and_query: parsed.path_and_query.into_owned(),
    })
}

/// Checks OCap host allowlist, Cloud Metadata SSRF hostname shield, and Outbound Egress Secret DLP.
/// Returns `Ok(is_explicitly_allowlisted_host)` where `true` indicates the hostname/host_port was
/// explicitly named in `allow_net` (rather than only matching wildcard `"*"`).
pub(crate) fn check_boa_outbound_fetch_ocap_and_dlp(
    parts: &OutboundUrlParts,
    raw_url: &str,
    headers_json: &str,
    body: &[u8],
) -> Result<bool, String> {
    let tenant_id = active_tenant_id_or_default();

    // 1. SSRF Cloud Metadata Shield (blocks 169.254.169.254, metadata.google.internal, etc. unconditionally)
    if is_cloud_metadata_clean_host(&parts.hostname) {
        let violation = SandboxViolation::NetAccessDenied {
            tenant_id,
            host: format!("SSRF_METADATA_BLOCKED:{}", parts.host_port),
        };
        let msg = violation.to_string();
        record_boa_ocap_denial(violation);
        return Err(msg);
    }

    // 2. OCap Network Host Allowlist Verification (strict host or exact host:port match; never cross-grants ports)
    let (explicit_host_grant, wildcard_grant) = BOA_NET_HOSTS.with(|hosts_cell| {
        check_net_host_grant(&hosts_cell.borrow(), &parts.hostname, parts.port, false)
    });
    let permitted = explicit_host_grant
        || (wildcard_grant && !is_private_or_loopback_clean_host(&parts.hostname));

    if !permitted {
        let violation = SandboxViolation::NetAccessDenied {
            tenant_id,
            host: parts.host_port.clone(),
        };
        let msg = violation.to_string();
        record_boa_ocap_denial(violation);
        return Err(msg);
    }

    // 3. Outbound Egress Secret DLP Inspection (scans URL query, outbound headers, and body for raw/Base64/Base64URL secrets)
    let dlp_err = BOA_EGRESS_SECRETS.with(|sec_cell| {
        let secrets = sec_cell.borrow();
        if secrets.is_empty() {
            return None;
        }
        let body_lossy = String::from_utf8_lossy(body);
        verify_egress_secrets_slice(&tenant_id, &secrets, raw_url)
            .and_then(|()| verify_egress_secrets_slice(&tenant_id, &secrets, headers_json))
            .and_then(|()| verify_egress_secrets_slice(&tenant_id, &secrets, &body_lossy))
            .and_then(|()| {
                if !body.is_empty() && !body.is_ascii() {
                    let b64 = crate::intrinsics::base64_encode(body, false);
                    verify_egress_secrets_slice(&tenant_id, &secrets, &b64)
                } else {
                    Ok(())
                }
            })
            .err()
    });

    if let Some(violation) = dlp_err {
        let msg = violation.to_string();
        record_boa_violation_once(violation);
        return Err(msg);
    }

    Ok(explicit_host_grant)
}

/// Validates a resolved `SocketAddr` slice against the Pre-Connect Resolved-IP SSRF Firewall.
///
/// - Cloud metadata IPs (`169.254.169.254`, `168.63.129.16`, `fd00:ec2::254`, etc.) are
///   unconditionally blocked even when `allow_private_networks` or an explicit grant is active.
/// - Loopback, private, link-local, CGNAT, unspecified, broadcast, multicast, and IPv4-mapped
///   IPv6 addresses are blocked for all DNS hostnames by default (whether explicitly granted or
///   matched via wildcard `*`), unless `allow_private_networks` is enabled, the grant named an
///   explicit IP literal, or the grant named `localhost` / `*.localhost` and resolved strictly to
///   a loopback IP.
pub(crate) fn validate_resolved_ssrf_socket_addrs(
    parts: &OutboundUrlParts,
    addrs: &[SocketAddr],
    explicit_host_grant: bool,
) -> Result<(), String> {
    let tenant_id = active_tenant_id_or_default();
    let is_ip_literal = parts.hostname.parse::<IpAddr>().is_ok();
    let is_localhost_name = parts.hostname == "localhost" || parts.hostname.ends_with(".localhost");
    let allow_private_networks = BOA_ALLOW_PRIVATE_NETS.with(|p| p.get());

    for addr in addrs {
        let ip = addr.ip();
        if is_cloud_metadata_ip(ip) {
            let violation = SandboxViolation::NetAccessDenied {
                tenant_id,
                host: format!("SSRF_METADATA_IP_BLOCKED:{}:{}", parts.host_port, ip),
            };
            let msg = violation.to_string();
            record_boa_ocap_denial(violation);
            return Err(msg);
        }
        if is_private_or_loopback_ip(ip) {
            let private_permitted = allow_private_networks
                || (explicit_host_grant && is_ip_literal)
                || (explicit_host_grant && is_localhost_name && is_loopback_ip(ip));
            if !private_permitted {
                let violation = SandboxViolation::NetAccessDenied {
                    tenant_id,
                    host: format!("SSRF_PRIVATE_IP_BLOCKED:{}:{}", parts.host_port, ip),
                };
                let msg = violation.to_string();
                record_boa_ocap_denial(violation);
                return Err(msg);
            }
        }
    }
    Ok(())
}

/// Resolves DNS for `parts` and validates every resolved `SocketAddr` against the Pre-Connect
/// Resolved-IP SSRF Firewall (preventing DNS rebinding attacks to `169.254.169.254` or `127.0.0.1`).
pub(crate) fn resolve_and_verify_ssrf_socket_addrs(
    parts: &OutboundUrlParts,
    explicit_host_grant: bool,
) -> Result<Vec<SocketAddr>, String> {
    let mut addrs: Vec<SocketAddr> = (parts.hostname.as_str(), parts.port)
        .to_socket_addrs()
        .map_err(|e| format!("DNS resolution failed for {}: {}", parts.host_port, e))?
        .collect();

    if addrs.is_empty() {
        return Err(format!(
            "No socket address resolved for {}",
            parts.host_port
        ));
    }

    validate_resolved_ssrf_socket_addrs(parts, &addrs, explicit_host_grant)?;

    // Prefer IPv4 addresses ahead of IPv6 for dual-stack hostnames (such as `localhost` on Windows)
    addrs.sort_by_key(|a| if a.is_ipv4() { 0u8 } else { 1u8 });
    Ok(addrs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boa_realm::{BOA_DENIED_CAPABILITIES, BOA_LAST_OCAP_VIOLATION, BOA_VIOLATION};
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn mk_parts(host: &str, port: u16) -> OutboundUrlParts {
        OutboundUrlParts {
            is_https: false,
            hostname: host.to_string(),
            port,
            host_header: format!("{host}:{port}"),
            host_port: format!("{host}:{port}"),
            path_and_query: "/".to_string(),
        }
    }

    #[test]
    fn resolved_ip_ssrf_firewall_blocks_private_loopback_on_explicit_dns_grants() {
        BOA_ALLOW_PRIVATE_NETS.with(|p| p.set(false));
        let dns_parts = mk_parts("rebind.example.com", 8080);

        let blocked_ips: &[IpAddr] = &[
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
            IpAddr::V6("::ffff:127.0.0.1".parse().unwrap()),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
            IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(169, 254, 1, 1)),
            IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
            IpAddr::V6("fc00::1".parse().unwrap()),
            IpAddr::V6("fe80::1".parse().unwrap()),
        ];

        for &ip in blocked_ips {
            BOA_VIOLATION.with(|v| *v.borrow_mut() = None);
            BOA_LAST_OCAP_VIOLATION.with(|v| *v.borrow_mut() = None);
            BOA_DENIED_CAPABILITIES.with(|d| d.borrow_mut().clear());

            let err =
                validate_resolved_ssrf_socket_addrs(&dns_parts, &[SocketAddr::new(ip, 8080)], true)
                    .expect_err(
                        "explicit DNS grant resolving to private/loopback IP must be blocked",
                    );
            assert!(
                err.starts_with("OCap Violation: lacks outbound network authority for host 'SSRF_PRIVATE_IP_BLOCKED:"),
                "unexpected error for {ip}: {err}"
            );
            assert!(
                !err.contains("Compartment '"),
                "error must not leak compartment name: {err}"
            );
            assert!(
                BOA_VIOLATION.with(|v| v.borrow().is_none()),
                "SSRF refusal must be catchable (not fatal BOA_VIOLATION)"
            );
            assert!(
                BOA_DENIED_CAPABILITIES.with(|d| d.borrow().len() == 1),
                "SSRF refusal must record in BOA_DENIED_CAPABILITIES"
            );
        }

        // Public IP is allowed for explicitly granted DNS host
        assert!(
            validate_resolved_ssrf_socket_addrs(
                &dns_parts,
                &[SocketAddr::new(
                    IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)),
                    8080
                )],
                true,
            )
            .is_ok()
        );

        // Explicit literal IP 127.0.0.1 is allowed
        let literal_parts = mk_parts("127.0.0.1", 8080);
        assert!(
            validate_resolved_ssrf_socket_addrs(
                &literal_parts,
                &[SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080)],
                true,
            )
            .is_ok()
        );

        // Explicit localhost is allowed ONLY for loopback, NOT RFC1918
        let localhost_parts = mk_parts("localhost", 8080);
        assert!(
            validate_resolved_ssrf_socket_addrs(
                &localhost_parts,
                &[SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080)],
                true,
            )
            .is_ok()
        );
        assert!(
            validate_resolved_ssrf_socket_addrs(
                &localhost_parts,
                &[SocketAddr::new(
                    IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
                    8080
                )],
                true,
            )
            .is_err()
        );

        // Opt-in allow_private_networks permits RFC1918/loopback DNS resolution,
        // while cloud metadata remains unconditionally blocked.
        BOA_ALLOW_PRIVATE_NETS.with(|p| p.set(true));
        assert!(
            validate_resolved_ssrf_socket_addrs(
                &dns_parts,
                &[SocketAddr::new(
                    IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
                    8080
                )],
                true,
            )
            .is_ok()
        );
        let meta_err = validate_resolved_ssrf_socket_addrs(
            &dns_parts,
            &[SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),
                80,
            )],
            true,
        )
        .expect_err("cloud metadata IP must remain blocked even with allow_private_networks");
        assert!(meta_err.contains("SSRF_METADATA_IP_BLOCKED"));
        BOA_ALLOW_PRIVATE_NETS.with(|p| p.set(false));
    }
}
