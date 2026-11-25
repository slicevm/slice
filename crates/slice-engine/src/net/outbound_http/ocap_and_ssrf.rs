//! Outbound HTTP URL Parsing, OCap `allow_net` Host Verification, Pre-Connect DNS SSRF Firewall, and Egress DLP.

use super::OutboundUrlParts;
use crate::bindings::helpers::{active_tenant_id_or_default, record_boa_violation_once};
use crate::boa_realm::{BOA_EGRESS_SECRETS, BOA_NET_HOSTS, record_boa_ocap_denial};
use crate::state_and_kv::verify_egress_secrets_slice;
use slice_sandbox::{
    SandboxViolation, check_net_host_grant, is_cloud_metadata_clean_host, is_cloud_metadata_ip,
    is_private_or_loopback_clean_host, is_private_or_loopback_ip,
};
use std::net::{SocketAddr, ToSocketAddrs};

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
        record_boa_violation_once(SandboxViolation::NetAccessDenied {
            tenant_id,
            host: format!("SSRF_METADATA_BLOCKED:{}", parts.host_port),
        });
        return Err("NetAccessDenied: SSRF Cloud Metadata Endpoint Blocked".to_string());
    }

    // 2. OCap Network Host Allowlist Verification (strict host or exact host:port match; never cross-grants ports)
    let (explicit_host_grant, wildcard_grant) = BOA_NET_HOSTS.with(|hosts_cell| {
        check_net_host_grant(&hosts_cell.borrow(), &parts.hostname, parts.port, false)
    });
    let permitted = explicit_host_grant
        || (wildcard_grant && !is_private_or_loopback_clean_host(&parts.hostname));

    if !permitted {
        record_boa_ocap_denial(SandboxViolation::NetAccessDenied {
            tenant_id,
            host: parts.host_port.clone(),
        });
        return Err(format!("NetAccessDenied: {}", parts.host_port));
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

/// Resolves DNS for `parts` and validates every resolved `SocketAddr` against the Pre-Connect
/// Resolved-IP SSRF Firewall (preventing DNS rebinding attacks to `169.254.169.254` or `127.0.0.1`).
pub(crate) fn resolve_and_verify_ssrf_socket_addrs(
    parts: &OutboundUrlParts,
    explicit_host_grant: bool,
) -> Result<Vec<SocketAddr>, String> {
    let tenant_id = active_tenant_id_or_default();

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

    for addr in &addrs {
        let ip = addr.ip();
        if is_cloud_metadata_ip(ip) {
            record_boa_violation_once(SandboxViolation::NetAccessDenied {
                tenant_id,
                host: format!("SSRF_METADATA_IP_BLOCKED:{}:{}", parts.host_port, ip),
            });
            return Err(format!(
                "NetAccessDenied: SSRF Cloud Metadata IP Blocked ({})",
                ip
            ));
        }
        if !explicit_host_grant && is_private_or_loopback_ip(ip) {
            record_boa_violation_once(SandboxViolation::NetAccessDenied {
                tenant_id,
                host: format!("SSRF_PRIVATE_IP_BLOCKED:{}:{}", parts.host_port, ip),
            });
            return Err(format!(
                "NetAccessDenied: DNS Rebinding / Private IP Blocked under wildcard allow_net ({})",
                ip
            ));
        }
    }

    // Prefer IPv4 addresses ahead of IPv6 for dual-stack hostnames (such as `localhost` on Windows)
    addrs.sort_by_key(|a| if a.is_ipv4() { 0u8 } else { 1u8 });
    Ok(addrs)
}
