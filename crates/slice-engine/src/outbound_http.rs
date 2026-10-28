//! Outbound `fetch()` HTTP/1.1 Engine with OCap Host Allowlist, Cloud Metadata SSRF Shield, and Egress DLP.

use crate::boa_realm::{BOA_ACTIVE_TENANT, BOA_EGRESS_SECRETS, BOA_NET_HOSTS, BOA_VIOLATION};
use crate::state_and_kv::verify_egress_secrets_slice;
use serde::Serialize;
use slice_sandbox::SandboxViolation;

pub(crate) struct OutboundUrlParts {
    pub(crate) hostname: String,
    pub(crate) port: u16,
    pub(crate) host_header: String,
    pub(crate) host_port: String,
    pub(crate) path_and_query: String,
}

pub(crate) fn parse_outbound_http_url(raw_url: &str) -> Option<OutboundUrlParts> {
    let trimmed = raw_url.trim();
    let without_hash = trimmed.split('#').next().unwrap_or(trimmed);
    let (default_port, rest) = if let Some(r) = without_hash.strip_prefix("http://") {
        (80u16, r)
    } else if let Some(r) = without_hash.strip_prefix("https://") {
        (443u16, r)
    } else {
        return None;
    };
    let (authority, path_and_query) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };
    let host_port_raw = match authority.rfind('@') {
        Some(idx) => &authority[idx + 1..],
        None => authority,
    };
    let (hostname, port) = if let Some(ipv6_rest) = host_port_raw.strip_prefix('[') {
        if let Some((ipv6, after_bracket)) = ipv6_rest.split_once(']') {
            let p = after_bracket
                .strip_prefix(':')
                .and_then(|ps| ps.parse::<u16>().ok())
                .unwrap_or(default_port);
            (format!("[{}]", ipv6.to_ascii_lowercase()), p)
        } else {
            return None;
        }
    } else if let Some((h, p_str)) = host_port_raw.rsplit_once(':') {
        let p = p_str.parse::<u16>().ok()?;
        (h.to_ascii_lowercase(), p)
    } else {
        (host_port_raw.to_ascii_lowercase(), default_port)
    };
    if hostname.is_empty() {
        return None;
    }
    let clean_host = slice_sandbox::extract_clean_hostname(&hostname);
    let host_port = format!("{}:{}", clean_host, port);
    let host_header = if port == default_port {
        hostname
    } else {
        format!("{}:{}", hostname, port)
    };
    Some(OutboundUrlParts {
        hostname: clean_host,
        port,
        host_header,
        host_port,
        path_and_query: path_and_query.to_string(),
    })
}

pub(crate) fn check_boa_outbound_fetch_ocap_and_dlp(
    parts: &OutboundUrlParts,
    raw_url: &str,
    headers_json: &str,
    body: &str,
) -> Result<(), String> {
    let tenant_id = BOA_ACTIVE_TENANT.with(|t| {
        let b = t.borrow();
        if b.is_empty() {
            "boa-tenant".to_string()
        } else {
            b.clone()
        }
    });

    // 1. SSRF Cloud Metadata Shield (blocks 169.254.169.254, metadata.google.internal, etc. unconditionally)
    if slice_sandbox::is_cloud_metadata_ssrf_target(&parts.hostname) {
        BOA_VIOLATION.with(|v| {
            *v.borrow_mut() = Some(SandboxViolation::NetAccessDenied {
                tenant_id,
                host: format!("SSRF_METADATA_BLOCKED:{}", parts.host_port),
            });
        });
        return Err("NetAccessDenied: SSRF Cloud Metadata Endpoint Blocked".to_string());
    }

    // 2. OCap Network Host Allowlist Verification
    let permitted = BOA_NET_HOSTS.with(|hosts_cell| {
        let hosts = hosts_cell.borrow();
        if hosts.contains(&parts.host_port)
            || hosts.contains(&parts.hostname)
            || hosts.contains(&format!("{}:443", parts.hostname))
            || hosts.contains(&format!("{}:80", parts.hostname))
        {
            return true;
        }
        if hosts.contains("*") && !slice_sandbox::is_private_or_loopback_ssrf_target(&parts.hostname)
        {
            return true;
        }
        false
    });

    if !permitted {
        BOA_VIOLATION.with(|v| {
            *v.borrow_mut() = Some(SandboxViolation::NetAccessDenied {
                tenant_id,
                host: parts.host_port.clone(),
            });
        });
        return Err(format!("NetAccessDenied: {}", parts.host_port));
    }

    // 3. Outbound Egress Secret DLP Inspection (scans URL query, outbound headers, and body for raw/Base64/Base64URL secrets)
    let dlp_err = BOA_EGRESS_SECRETS.with(|sec_cell| {
        let secrets = sec_cell.borrow();
        if secrets.is_empty() {
            return None;
        }
        verify_egress_secrets_slice(&tenant_id, &secrets, raw_url)
            .and_then(|()| verify_egress_secrets_slice(&tenant_id, &secrets, headers_json))
            .and_then(|()| verify_egress_secrets_slice(&tenant_id, &secrets, body))
            .err()
    });

    if let Some(violation) = dlp_err {
        BOA_VIOLATION.with(|v| *v.borrow_mut() = Some(violation));
        return Err("EnvAccessDenied: EGRESS_SECRET_LEAK_BLOCKED".to_string());
    }

    Ok(())
}

pub(crate) fn decode_chunked_http_body(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let mut pos = 0;
    while pos < raw.len() {
        let Some(rel_end) = raw[pos..].windows(2).position(|w| w == b"\r\n") else {
            break;
        };
        let size_line = std::str::from_utf8(&raw[pos..pos + rel_end]).unwrap_or("");
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let Ok(chunk_len) = usize::from_str_radix(size_hex, 16) else {
            break;
        };
        pos += rel_end + 2;
        if chunk_len == 0 {
            break;
        }
        let end = (pos + chunk_len).min(raw.len());
        out.extend_from_slice(&raw[pos..end]);
        pos = (end + 2).min(raw.len());
    }
    out
}

pub(crate) fn execute_outbound_http11(
    parts: &OutboundUrlParts,
    method: &str,
    req_headers: &[(String, String)],
    body: &str,
) -> Result<String, String> {
    use std::io::{Read, Write};
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Duration;

    let addr = (parts.hostname.as_str(), parts.port)
        .to_socket_addrs()
        .map_err(|e| format!("DNS resolution failed for {}: {}", parts.host_port, e))?
        .next()
        .ok_or_else(|| format!("No socket address resolved for {}", parts.host_port))?;

    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
        .map_err(|e| format!("TCP connect failed to {}: {}", parts.host_port, e))?;
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));

    let mut req_buf = Vec::with_capacity(256 + body.len());
    let _ = write!(
        &mut req_buf,
        "{} {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: Slice-NexusJS/0.2.0\r\nConnection: close\r\n",
        method, parts.path_and_query, parts.host_header
    );
    for (k, v) in req_headers {
        if !k.eq_ignore_ascii_case("host")
            && !k.eq_ignore_ascii_case("connection")
            && !k.eq_ignore_ascii_case("content-length")
        {
            let _ = write!(&mut req_buf, "{}: {}\r\n", k, v);
        }
    }
    if !body.is_empty() || matches!(method, "POST" | "PUT" | "PATCH") {
        let _ = write!(&mut req_buf, "Content-Length: {}\r\n\r\n", body.len());
        req_buf.extend_from_slice(body.as_bytes());
    } else {
        req_buf.extend_from_slice(b"\r\n");
    }

    stream
        .write_all(&req_buf)
        .map_err(|e| format!("HTTP request write failed: {}", e))?;
    let _ = stream.flush();

    let mut raw_resp = Vec::with_capacity(4096);
    stream
        .read_to_end(&mut raw_resp)
        .map_err(|e| format!("HTTP response read failed: {}", e))?;

    let (hdr_end, body_start) = raw_resp
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| (i, i + 4))
        .or_else(|| {
            raw_resp
                .windows(2)
                .position(|w| w == b"\n\n")
                .map(|i| (i, i + 2))
        })
        .unwrap_or((raw_resp.len(), raw_resp.len()));

    let header_str = std::str::from_utf8(&raw_resp[..hdr_end]).unwrap_or("");
    let mut lines = header_str.lines();
    let status_line = lines.next().unwrap_or("HTTP/1.1 200 OK");
    let mut status_parts = status_line.splitn(3, ' ');
    let _http_ver = status_parts.next();
    let status_code: u16 = status_parts
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    let status_text = status_parts.next().unwrap_or("OK").trim().to_string();

    let mut resp_headers: Vec<(String, String)> = Vec::new();
    let mut is_chunked = false;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_ascii_lowercase();
            let val = v.trim().to_string();
            if key == "transfer-encoding" && val.to_ascii_lowercase().contains("chunked") {
                is_chunked = true;
            }
            resp_headers.push((key, val));
        }
    }

    let raw_body_slice = &raw_resp[body_start..];
    let decoded_body = if is_chunked {
        String::from_utf8_lossy(&decode_chunked_http_body(raw_body_slice)).into_owned()
    } else {
        String::from_utf8_lossy(raw_body_slice).into_owned()
    };

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct WireFetchResponse {
        status: u16,
        status_text: String,
        headers: Vec<(String, String)>,
        body: String,
    }

    serde_json::to_string(&WireFetchResponse {
        status: status_code,
        status_text,
        headers: resp_headers,
        body: decoded_body,
    })
    .map_err(|e| e.to_string())
}

