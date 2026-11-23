//! HTTP/1.1 Wire Framing, SIMD Header Delimiter Scanner, Proxy `CONNECT` Tunneling, and Single-Hop Exchange.

use super::chunked::ChunkedBodyDecoder;
use super::tls_and_pool::{
    FetchSlabSlotGuard, OutboundStream, dial_fresh_outbound_stream, get_tls_client_config,
    record_outbound_pool_reuses, return_pooled_stream, take_pooled_stream,
};
use super::{
    MAX_OUTBOUND_BODY_BYTES, MAX_OUTBOUND_HEADER_BYTES, OutboundUrlParts,
    is_valid_http_header_name, is_valid_http_header_value, is_valid_request_target,
};
use crate::bindings::helpers::{active_tenant_id_or_default, record_boa_violation_once};
use crate::boa_realm::BOA_KV_MEM_BUDGET;
use slice_sandbox::SandboxViolation;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::sync::LazyLock;

static CRLF2_FINDER: LazyLock<memchr::memmem::Finder<'static>> =
    LazyLock::new(|| memchr::memmem::Finder::new(b"\r\n\r\n"));
static LF2_FINDER: LazyLock<memchr::memmem::Finder<'static>> =
    LazyLock::new(|| memchr::memmem::Finder::new(b"\n\n"));

/// Scans `buf` (resuming from `search_from` where callers pass `filled.saturating_sub(3)`)
/// using SIMD `memchr::memmem` finders for the end of an HTTP/1.x header block.
///
/// - Returns `Ok(Some((hdr_end, body_start)))` on complete `\r\n\r\n` or pure-LF `\n\n` headers.
/// - Returns `Ok(None)` when the header terminator has not arrived yet.
/// - Returns `Err` if CRLF framing is mixed with a bare `\n\n` terminator (RFC 9112 smuggling guard).
#[inline]
pub fn find_http_header_end_from(
    buf: &[u8],
    search_from: usize,
) -> Result<Option<(usize, usize)>, &'static str> {
    let Some(first_lf) = memchr::memchr(b'\n', buf) else {
        return Ok(None);
    };
    let uses_crlf = first_lf > 0 && buf[first_lf - 1] == b'\r';
    let start = search_from.min(buf.len());
    let lf_start = search_from.saturating_sub(1).min(buf.len());

    if uses_crlf {
        if let Some(rel) = CRLF2_FINDER.find(&buf[start..]) {
            let hdr_end = start + rel;
            if LF2_FINDER.find(&buf[..hdr_end]).is_some() {
                return Err("mixed CRLF and bare LF header framing");
            }
            return Ok(Some((hdr_end, hdr_end + 4)));
        }
        if LF2_FINDER.find(&buf[lf_start..]).is_some() {
            return Err("mixed CRLF and bare LF header framing");
        }
        Ok(None)
    } else {
        if let Some(rel) = LF2_FINDER.find(&buf[lf_start..]) {
            let hdr_end = lf_start + rel;
            if CRLF2_FINDER.find(&buf[..hdr_end]).is_some() {
                return Err("mixed bare LF and CRLF header framing");
            }
            return Ok(Some((hdr_end, hdr_end + 2)));
        }
        if CRLF2_FINDER.find(&buf[start..]).is_some() {
            return Err("mixed bare LF and CRLF header framing");
        }
        Ok(None)
    }
}

pub(super) struct RawHttpResponse {
    pub(super) status: u16,
    pub(super) status_text: String,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
    pub(super) peer_closed: bool,
}

fn trap_outbound_memory_exceeded(requested: usize, limit: usize) -> std::io::Error {
    let tenant_id = active_tenant_id_or_default();
    record_boa_violation_once(SandboxViolation::MemoryCeilingExceeded {
        tenant_id,
        requested,
        limit,
    });
    std::io::Error::new(
        std::io::ErrorKind::OutOfMemory,
        format!(
            "MemoryCeilingExceeded: outbound HTTP response ({} bytes) exceeded budget ({} bytes)",
            requested, limit
        ),
    )
}

fn send_and_read_framed_http11(
    stream: &mut OutboundStream,
    req_buf: &[u8],
    method: &str,
) -> Result<RawHttpResponse, std::io::Error> {
    let slab_guard = FetchSlabSlotGuard::acquire();
    stream.write_all(req_buf)?;
    stream.flush()?;

    let effective_max_body = BOA_KV_MEM_BUDGET.with(|m| {
        let (used, limit) = m.get();
        limit.saturating_sub(used).min(MAX_OUTBOUND_BODY_BYTES)
    });

    let mut raw_resp = Vec::with_capacity(4096);
    let mut info_hops = 0usize;

    let (
        body_start,
        status_code,
        status_text,
        resp_headers,
        is_chunked,
        content_length,
        mut peer_closed,
    ) = loop {
        let mut scan_cursor = 0usize;
        let (hdr_end, body_start) = loop {
            let found_opt = find_http_header_end_from(&raw_resp, scan_cursor)
                .map_err(|msg| std::io::Error::new(std::io::ErrorKind::InvalidData, msg))?;
            if let Some(found) = found_opt {
                if found.0 > MAX_OUTBOUND_HEADER_BYTES {
                    return Err(trap_outbound_memory_exceeded(
                        found.0,
                        MAX_OUTBOUND_HEADER_BYTES,
                    ));
                }
                break found;
            }
            if raw_resp.len() > MAX_OUTBOUND_HEADER_BYTES {
                return Err(trap_outbound_memory_exceeded(
                    raw_resp.len(),
                    MAX_OUTBOUND_HEADER_BYTES,
                ));
            }
            scan_cursor = raw_resp.len().saturating_sub(3);
            let n = slab_guard.read_chunk_via_slab(
                stream,
                &mut raw_resp,
                slice_io::FIXED_BUFFER_SIZE,
            )?;
            if n == 0 {
                if raw_resp.is_empty() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "peer closed connection before sending HTTP response headers",
                    ));
                }
                break (raw_resp.len(), raw_resp.len());
            }
        };

        let header_str = std::str::from_utf8(&raw_resp[..hdr_end]).unwrap_or("");
        let mut lines = header_str.lines();
        let status_line = lines.next().unwrap_or("HTTP/1.1 200 OK");
        let mut status_parts = status_line.splitn(3, ' ');
        let http_ver = status_parts.next().unwrap_or("HTTP/1.1");
        let status_code: u16 = status_parts
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(200);
        let status_text = status_parts.next().unwrap_or("OK").trim().to_string();

        // Skip RFC 9110 interim `1xx` responses (`100 Continue`, `102 Processing`, `103 Early Hints`),
        // except `101 Switching Protocols`.
        if matches!(status_code, 100..=199) && status_code != 101 {
            info_hops += 1;
            if info_hops > 8 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "too many 1xx informational HTTP responses",
                ));
            }
            raw_resp.drain(..body_start);
            continue;
        }

        let mut resp_headers: Vec<(String, String)> = Vec::new();
        let mut is_chunked = false;
        let mut content_length: Option<usize> = None;
        let mut peer_closed = http_ver.ends_with("1.0");

        for line in lines {
            if let Some((k, v)) = line.split_once(':') {
                let key = k.trim().to_ascii_lowercase();
                let val = v.trim().to_string();
                let val_lower = val.to_ascii_lowercase();
                if key == "transfer-encoding" && val_lower.contains("chunked") {
                    is_chunked = true;
                } else if key == "content-length" {
                    if let Ok(len) = val.trim().parse::<usize>() {
                        content_length = Some(len);
                    }
                } else if key == "connection" {
                    if val_lower.contains("close") {
                        peer_closed = true;
                    } else if val_lower.contains("keep-alive") {
                        peer_closed = false;
                    }
                }
                resp_headers.push((key, val));
            }
        }

        break (
            body_start,
            status_code,
            status_text,
            resp_headers,
            is_chunked,
            content_length,
            peer_closed,
        );
    };

    let no_body_status = method == "HEAD" || matches!(status_code, 100..=199 | 204 | 304);
    let decoded_body = if no_body_status {
        if raw_resp.len() > body_start {
            peer_closed = true;
        }
        Vec::new()
    } else if is_chunked {
        let mut decoder = ChunkedBodyDecoder::new(effective_max_body);
        let mut cursor = body_start;
        loop {
            let (consumed, done) = match decoder.feed(&raw_resp[cursor..]) {
                Ok(v) => v,
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::OutOfMemory {
                        return Err(trap_outbound_memory_exceeded(
                            effective_max_body.saturating_add(1),
                            effective_max_body,
                        ));
                    }
                    return Err(e);
                }
            };
            cursor += consumed;
            if done {
                if cursor < raw_resp.len() {
                    peer_closed = true;
                }
                break decoder.into_vec();
            }
            if cursor >= 16384 {
                raw_resp.drain(..cursor);
                cursor = 0;
            }
            let n = slab_guard.read_chunk_via_slab(
                stream,
                &mut raw_resp,
                slice_io::FIXED_BUFFER_SIZE,
            )?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "truncated chunked HTTP response body",
                ));
            }
        }
    } else if let Some(expected_len) = content_length {
        if expected_len > effective_max_body {
            return Err(trap_outbound_memory_exceeded(
                expected_len,
                effective_max_body,
            ));
        }
        while raw_resp.len().saturating_sub(body_start) < expected_len {
            let remaining = expected_len - (raw_resp.len() - body_start);
            let n = slab_guard.read_chunk_via_slab(stream, &mut raw_resp, remaining)?;
            if n == 0 {
                peer_closed = true;
                break;
            }
        }
        let end_idx = (body_start + expected_len).min(raw_resp.len());
        if raw_resp.len() > end_idx {
            peer_closed = true;
        }
        raw_resp.truncate(end_idx);
        if body_start > 0 {
            raw_resp.drain(..body_start);
        }
        raw_resp
    } else {
        peer_closed = true;
        loop {
            let current_body_len = raw_resp.len().saturating_sub(body_start);
            if current_body_len > effective_max_body {
                return Err(trap_outbound_memory_exceeded(
                    current_body_len,
                    effective_max_body,
                ));
            }
            let prev_len = raw_resp.len();
            let n = slab_guard.read_chunk_via_slab(
                stream,
                &mut raw_resp,
                slice_io::FIXED_BUFFER_SIZE,
            )?;
            if n == 0 {
                break;
            }
            if current_body_len.saturating_add(n) > effective_max_body {
                raw_resp.truncate(prev_len);
                return Err(trap_outbound_memory_exceeded(
                    current_body_len.saturating_add(n),
                    effective_max_body,
                ));
            }
        }
        if body_start > 0 {
            raw_resp.drain(..body_start);
        }
        raw_resp
    };

    Ok(RawHttpResponse {
        status: status_code,
        status_text,
        headers: resp_headers,
        body: decoded_body,
        peer_closed,
    })
}

fn establish_proxy_connect_tunnel(
    proxy_parts: &OutboundUrlParts,
    proxy_addrs: &[SocketAddr],
    target_parts: &OutboundUrlParts,
) -> Result<OutboundStream, String> {
    if !is_valid_http_header_value(&target_parts.host_port)
        || target_parts
            .host_port
            .bytes()
            .any(|b| b <= 0x20 || b == 0x7F)
    {
        return Err(
            "InvalidConnectAuthority: target host:port contains invalid characters".to_string(),
        );
    }
    let mut proxy_stream = dial_fresh_outbound_stream(proxy_parts, proxy_addrs)?;
    let connect_req = format!(
        "CONNECT {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: Slice-NexusJS/{}\r\nProxy-Connection: keep-alive\r\n\r\n",
        target_parts.host_port,
        target_parts.host_port,
        env!("CARGO_PKG_VERSION")
    );
    proxy_stream
        .write_all(connect_req.as_bytes())
        .and_then(|()| proxy_stream.flush())
        .map_err(|e| {
            format!(
                "Proxy CONNECT write failed to {}: {}",
                proxy_parts.host_port, e
            )
        })?;

    let mut hdr_buf = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    loop {
        if hdr_buf.len() >= MAX_OUTBOUND_HEADER_BYTES {
            return Err(
                "ProxyConnectError: CONNECT response headers exceeded 64 KB limit".to_string(),
            );
        }
        let n = proxy_stream.read(&mut byte).map_err(|e| {
            format!(
                "Proxy CONNECT read failed from {}: {}",
                proxy_parts.host_port, e
            )
        })?;
        if n == 0 {
            return Err(format!(
                "ProxyConnectError: proxy {} closed connection during CONNECT handshake",
                proxy_parts.host_port
            ));
        }
        hdr_buf.push(byte[0]);
        let scan_from = hdr_buf.len().saturating_sub(4);
        if find_http_header_end_from(&hdr_buf, scan_from)
            .map_err(|msg| format!("ProxyConnectError: {}", msg))?
            .is_some()
        {
            break;
        }
    }

    let hdr_str = std::str::from_utf8(&hdr_buf)
        .map_err(|_| "ProxyConnectError: invalid UTF-8 in CONNECT response".to_string())?;
    let status_line = hdr_str.lines().next().unwrap_or("");
    let status_code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if status_code != 200 {
        return Err(format!(
            "ProxyConnectError: proxy {} rejected CONNECT tunnel to {} with status {}",
            proxy_parts.host_port, target_parts.host_port, status_code
        ));
    }

    let tls_cfg = get_tls_client_config();
    let sni_host = target_parts
        .hostname
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let server_name = rustls_pki_types::ServerName::try_from(sni_host).map_err(|e| {
        format!(
            "Invalid TLS SNI hostname '{}': {}",
            target_parts.hostname, e
        )
    })?;
    let client_conn = rustls::ClientConnection::new(tls_cfg, server_name).map_err(|e| {
        format!(
            "TLS initialization failed over proxy tunnel for {}: {}",
            target_parts.host_port, e
        )
    })?;

    match proxy_stream {
        OutboundStream::Plain(tcp) => Ok(OutboundStream::Tls(Box::new(rustls::StreamOwned::new(
            client_conn,
            tcp,
        )))),
        OutboundStream::Tls(proxy_tls) => Ok(OutboundStream::TunneledTls(Box::new(
            rustls::StreamOwned::new(client_conn, proxy_tls),
        ))),
        OutboundStream::TunneledTls(_) => {
            Err("ProxyConnectError: nested proxy TLS tunnels are unsupported".to_string())
        }
    }
}

pub(super) enum OutboundHopDialPlan<'a> {
    Direct {
        verified_addrs: &'a [SocketAddr],
    },
    HttpProxy {
        proxy_parts: &'a OutboundUrlParts,
        proxy_addrs: &'a [SocketAddr],
    },
    HttpsConnectProxy {
        proxy_parts: &'a OutboundUrlParts,
        proxy_addrs: &'a [SocketAddr],
    },
}

pub(super) fn execute_single_http11_hop(
    target_parts: &OutboundUrlParts,
    request_target: &str,
    dial_plan: OutboundHopDialPlan<'_>,
    method: &str,
    req_headers: &[(String, String)],
    body: &[u8],
) -> Result<RawHttpResponse, String> {
    if !is_valid_http_header_name(method) {
        return Err(format!(
            "InvalidMethod: '{}' contains invalid token characters",
            method
        ));
    }
    if !is_valid_request_target(request_target) {
        return Err(
            "InvalidRequestTarget: URL path/query contains forbidden whitespace or control characters"
                .to_string(),
        );
    }
    if !is_valid_http_header_value(&target_parts.host_header) {
        return Err("InvalidHostHeader: Host contains forbidden control characters".to_string());
    }
    for (k, v) in req_headers {
        if !is_valid_http_header_name(k) || !is_valid_http_header_value(v) {
            return Err(format!(
                "InvalidHeader: CRLF or invalid token character blocked in header '{}'",
                k
            ));
        }
    }

    let mut req_buf = Vec::with_capacity(256 + body.len());
    let _ = write!(
        &mut req_buf,
        "{} {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: Slice-NexusJS/{}\r\nConnection: keep-alive\r\n",
        method,
        request_target,
        target_parts.host_header,
        env!("CARGO_PKG_VERSION")
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
        req_buf.extend_from_slice(body);
    } else {
        req_buf.extend_from_slice(b"\r\n");
    }

    let tenant_id = active_tenant_id_or_default();
    let proxy_key = match dial_plan {
        OutboundHopDialPlan::Direct { .. } => "",
        OutboundHopDialPlan::HttpProxy { proxy_parts, .. }
        | OutboundHopDialPlan::HttpsConnectProxy { proxy_parts, .. } => &proxy_parts.host_port,
    };

    if let Some(mut pooled_stream) = take_pooled_stream(
        &tenant_id,
        proxy_key,
        target_parts.is_https,
        &target_parts.host_port,
    ) && let Ok(resp) = send_and_read_framed_http11(&mut pooled_stream, &req_buf, method)
    {
        record_outbound_pool_reuses(1);
        if !resp.peer_closed {
            return_pooled_stream(
                tenant_id,
                proxy_key.to_string(),
                target_parts.is_https,
                target_parts.host_port.clone(),
                pooled_stream,
            );
        }
        return Ok(resp);
    }

    let mut fresh_stream = match dial_plan {
        OutboundHopDialPlan::Direct { verified_addrs } => {
            dial_fresh_outbound_stream(target_parts, verified_addrs)?
        }
        OutboundHopDialPlan::HttpProxy {
            proxy_parts,
            proxy_addrs,
        } => dial_fresh_outbound_stream(proxy_parts, proxy_addrs)?,
        OutboundHopDialPlan::HttpsConnectProxy {
            proxy_parts,
            proxy_addrs,
        } => establish_proxy_connect_tunnel(proxy_parts, proxy_addrs, target_parts)?,
    };
    let resp = send_and_read_framed_http11(&mut fresh_stream, &req_buf, method).map_err(|e| {
        format!(
            "HTTP/1.1 exchange failed with {}: {}",
            target_parts.host_port, e
        )
    })?;
    if !resp.peer_closed {
        return_pooled_stream(
            tenant_id,
            proxy_key.to_string(),
            target_parts.is_https,
            target_parts.host_port.clone(),
            fresh_stream,
        );
    }
    Ok(resp)
}
