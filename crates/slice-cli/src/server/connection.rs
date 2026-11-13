use super::resolve_tenant_bytecode_hotswap;
use super::tls::ServerConnStream;
use anyhow::Result;
use slice_engine::SliceIsolate;
use slice_sandbox::CapabilityToken;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::Instant;

/// Maximum allowed inbound HTTP request body size (`4 MB`).
pub(crate) const MAX_INBOUND_BODY_BYTES: usize = 4 * 1024 * 1024;
/// Maximum pipelined static asset size eligible for write_buf coalescing (`16 KB`).
/// Static assets larger than `16 KB` (or non-pipelined responses) write directly from the
/// zero-copy `mmap` payload slice without copying into `write_buf`.
const STATIC_COALESCE_MAX_BYTES: usize = 16 * 1024;
/// Capacity ceiling above which `write_buf` is shrunk back to `16 KB` after flushing.
const WRITE_BUF_SHRINK_THRESHOLD_BYTES: usize = 65_536;
const WRITE_BUF_RESTING_CAPACITY_BYTES: usize = 16_384;

static CRLF2_FINDER: std::sync::LazyLock<memchr::memmem::Finder<'static>> =
    std::sync::LazyLock::new(|| memchr::memmem::Finder::new(b"\r\n\r\n"));
static LF2_FINDER: std::sync::LazyLock<memchr::memmem::Finder<'static>> =
    std::sync::LazyLock::new(|| memchr::memmem::Finder::new(b"\n\n"));

#[inline]
pub(crate) fn find_http_header_end(buf: &[u8]) -> Option<(usize, usize)> {
    find_http_header_end_from(buf, 0)
}

#[inline]
pub(crate) fn find_http_header_end_from(buf: &[u8], scan_from: usize) -> Option<(usize, usize)> {
    let start = scan_from.min(buf.len());
    if let Some(rel) = CRLF2_FINDER.find(&buf[start..]) {
        let i = start + rel;
        return Some((i, i + 4));
    }
    // Do not fall back to bare `\n\n` if the request line uses CRLF (`\r\n`) framing.
    let first_lf = memchr::memchr(b'\n', buf)?;
    if first_lf > 0 && buf[first_lf - 1] == b'\r' {
        return None;
    }
    let lf_start = scan_from.saturating_sub(1).min(buf.len());
    if let Some(rel) = LF2_FINDER.find(&buf[lf_start..]) {
        let i = lf_start + rel;
        return Some((i, i + 2));
    }
    None
}

#[inline]
fn is_chunked_transfer_encoding_only(val: &str) -> bool {
    let mut saw_chunked = false;
    for raw_tok in val.split(',') {
        let tok = raw_tok.trim();
        if tok.is_empty() || saw_chunked || !tok.eq_ignore_ascii_case("chunked") {
            return false;
        }
        saw_chunked = true;
    }
    saw_chunked
}

#[inline]
fn is_forbidden_custom_response_header(name: &str) -> bool {
    name.eq_ignore_ascii_case("transfer-encoding")
        || name.eq_ignore_ascii_case("content-length")
        || name.eq_ignore_ascii_case("content-type")
        || name.eq_ignore_ascii_case("connection")
        || name.eq_ignore_ascii_case("keep-alive")
        || name.eq_ignore_ascii_case("upgrade")
        || name.eq_ignore_ascii_case("trailer")
        || name.eq_ignore_ascii_case("te")
}

#[inline]
fn clear_and_trim_write_buf(write_buf: &mut Vec<u8>) {
    write_buf.clear();
    if write_buf.capacity() > WRITE_BUF_SHRINK_THRESHOLD_BYTES {
        write_buf.shrink_to(WRITE_BUF_RESTING_CAPACITY_BYTES);
    }
}

#[inline]
fn flush_pending_write_buf<S: Write>(stream: &mut S, write_buf: &mut Vec<u8>) -> Result<()> {
    if !write_buf.is_empty() {
        stream.write_all(write_buf)?;
        stream.flush()?;
        clear_and_trim_write_buf(write_buf);
    }
    Ok(())
}

#[inline]
fn contains_ascii_ignore_case(haystack: &str, needle_lower: &[u8]) -> bool {
    haystack
        .as_bytes()
        .windows(needle_lower.len())
        .any(|w| w.eq_ignore_ascii_case(needle_lower))
}

#[inline]
pub(crate) fn http_status_reason(code: u16) -> &'static str {
    match code {
        101 => "Switching Protocols",
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        408 => "Request Timeout",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        _ => "OK",
    }
}

/// Fast stack-allocated ASCII integer formatter using `itoa` (`CLI-PERF-02`).
#[inline]
pub(crate) fn push_u64_ascii(buf: &mut Vec<u8>, val: u64) {
    let mut itoa_buf = itoa::Buffer::new();
    buf.extend_from_slice(itoa_buf.format(val).as_bytes());
}

/// Escapes `raw` for safe embedding inside a JSON string literal (`"..."`), preventing malformed
/// or injected JSON when formatting `SandboxViolation` messages (`CLI-SEC-01`).
#[inline]
pub(crate) fn escape_json_string_into(raw: &str, out: &mut String) {
    for ch in raw.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
}

pub(crate) fn write_http_error_and_close<S: Write>(
    stream: &mut S,
    status: u16,
    json_body: &str,
) -> Result<()> {
    let mut resp = Vec::with_capacity(256 + json_body.len());
    resp.extend_from_slice(b"HTTP/1.1 ");
    push_u64_ascii(&mut resp, status as u64);
    resp.push(b' ');
    resp.extend_from_slice(http_status_reason(status).as_bytes());
    resp.extend_from_slice(b"\r\nContent-Type: application/json\r\nContent-Length: ");
    push_u64_ascii(&mut resp, json_body.len() as u64);
    resp.extend_from_slice(b"\r\nConnection: close\r\nServer: Slice-NexusJS/0.2\r\n\r\n");
    resp.extend_from_slice(json_body.as_bytes());
    let _ = stream.write_all(&resp);
    let _ = stream.flush();
    Ok(())
}

pub(crate) fn flush_server_ws_frames<S: Write>(
    stream: &mut S,
    frames: Vec<slice_engine::ServerWsOutFrame>,
) -> Result<bool> {
    let mut should_close = false;
    for frame in frames {
        let encoded = match frame {
            slice_engine::ServerWsOutFrame::Text(ref s) => slice_engine::encode_websocket_frame(
                true,
                slice_engine::WS_OPCODE_TEXT,
                None,
                s.as_bytes(),
            ),
            slice_engine::ServerWsOutFrame::Binary(ref b) => {
                slice_engine::encode_websocket_frame(true, slice_engine::WS_OPCODE_BINARY, None, b)
            }
            slice_engine::ServerWsOutFrame::Close(code, ref reason) => {
                should_close = true;
                let payload = slice_engine::build_close_frame_payload(code, reason);
                slice_engine::encode_websocket_frame(
                    true,
                    slice_engine::WS_OPCODE_CLOSE,
                    None,
                    &payload,
                )
            }
        };
        stream.write_all(&encoded)?;
    }
    stream.flush()?;
    Ok(should_close)
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn serve_connection_keep_alive(
    tcp_stream: TcpStream,
    slot_id: u32,
    bytecode: &[u8],
    tenant_dir_opt: Option<&Path>,
    static_assets_opt: Option<&slice_cas::StaticAssetTable>,
    deterministic_seed: Option<u64>,
    caps: &CapabilityToken,
    fuel_limit: u64,
    mem_limit: usize,
    read_buf: &mut [u8],
    write_buf: &mut Vec<u8>,
    max_requests_tracker: Option<(&std::sync::atomic::AtomicUsize, usize)>,
) -> Result<()> {
    serve_connection_keep_alive_with_tls_and_secrets(
        tcp_stream,
        slot_id,
        bytecode,
        tenant_dir_opt,
        static_assets_opt,
        deterministic_seed,
        caps,
        &[],
        fuel_limit,
        mem_limit,
        read_buf,
        write_buf,
        max_requests_tracker,
        None,
    )
}

pub(crate) fn spawn_worker_isolate(
    slot_id: u32,
    deterministic_seed: Option<u64>,
    caps: &CapabilityToken,
    audit_secrets: &[String],
    fuel_limit: u64,
    mem_limit: usize,
) -> SliceIsolate {
    let mut isolate = SliceIsolate::spawn(
        "tenant-default",
        slot_id,
        caps.clone(),
        fuel_limit,
        mem_limit,
    );
    isolate.set_deterministic_seed(deterministic_seed);
    for (k, v) in &caps.allowed_env_keys {
        if slice_sandbox::is_sensitive_env_key(k) && v.len() >= 6 {
            isolate.add_egress_audit_secret(v.clone());
        }
    }
    // CLI-SEC-01: Register explicit DLP secrets (`--audit-egress`, `SLICE_AUDIT_EGRESS`, `slice.toml` `audit_egress`, `.env`)
    for sec in audit_secrets {
        if !sec.is_empty() {
            isolate.add_egress_audit_secret(sec.clone());
        }
    }
    isolate
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn serve_connection_keep_alive_with_tls_and_secrets(
    tcp_stream: TcpStream,
    slot_id: u32,
    bytecode: &[u8],
    tenant_dir_opt: Option<&Path>,
    static_assets_opt: Option<&slice_cas::StaticAssetTable>,
    deterministic_seed: Option<u64>,
    caps: &CapabilityToken,
    audit_secrets: &[String],
    fuel_limit: u64,
    mem_limit: usize,
    read_buf: &mut [u8],
    write_buf: &mut Vec<u8>,
    max_requests_tracker: Option<(&std::sync::atomic::AtomicUsize, usize)>,
    tls_server_cfg: Option<&std::sync::Arc<rustls::ServerConfig>>,
) -> Result<()> {
    let mut isolate = spawn_worker_isolate(
        slot_id,
        deterministic_seed,
        caps,
        audit_secrets,
        fuel_limit,
        mem_limit,
    );
    serve_connection_with_isolate(
        tcp_stream,
        slot_id,
        &mut isolate,
        bytecode,
        tenant_dir_opt,
        static_assets_opt,
        fuel_limit,
        mem_limit,
        read_buf,
        write_buf,
        max_requests_tracker,
        tls_server_cfg,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn serve_connection_with_isolate(
    tcp_stream: TcpStream,
    mut slot_id: u32,
    isolate: &mut SliceIsolate,
    bytecode: &[u8],
    tenant_dir_opt: Option<&Path>,
    static_assets_opt: Option<&slice_cas::StaticAssetTable>,
    fuel_limit: u64,
    mem_limit: usize,
    read_buf: &mut [u8],
    write_buf: &mut Vec<u8>,
    max_requests_tracker: Option<(&std::sync::atomic::AtomicUsize, usize)>,
    tls_server_cfg: Option<&std::sync::Arc<rustls::ServerConfig>>,
) -> Result<()> {
    let _ = tcp_stream.set_nodelay(true);
    // N-Sec-4: Defend against Slowloris socket starvation with 10s read/write socket timeouts
    let _ = tcp_stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
    let _ = tcp_stream.set_write_timeout(Some(std::time::Duration::from_secs(10)));
    let mut stream = if let Some(tls_cfg) = tls_server_cfg {
        let conn = rustls::ServerConnection::new(std::sync::Arc::clone(tls_cfg))?;
        ServerConnStream::Tls(Box::new(rustls::StreamOwned::new(conn, tcp_stream)))
    } else {
        ServerConnStream::Plain(tcp_stream)
    };

    let mut filled: usize = 0;
    let mut carry_buf: Vec<u8> = Vec::new();
    write_buf.clear();

    loop {
        if !carry_buf.is_empty() && filled < read_buf.len() {
            let pull = carry_buf.len().min(read_buf.len() - filled);
            read_buf[filled..filled + pull].copy_from_slice(&carry_buf[..pull]);
            carry_buf.drain(..pull);
            filled += pull;
            if carry_buf.is_empty() && carry_buf.capacity() > 65536 {
                carry_buf.shrink_to(0);
            }
        }

        let mut header_deadline: Option<Instant> = if filled > 0 {
            Some(Instant::now() + std::time::Duration::from_secs(10))
        } else {
            None
        };
        let mut narrowed_timeout = false;

        // 1. Read until complete HTTP headers (`\r\n\r\n`) are present in `read_buf[..filled]`
        let mut hdr_scan_cursor = 0usize;
        let (hdr_end, body_offset) = loop {
            if let Some(pos) = find_http_header_end_from(&read_buf[..filled], hdr_scan_cursor) {
                break pos;
            }
            hdr_scan_cursor = filled.saturating_sub(3);
            if !carry_buf.is_empty() && filled < read_buf.len() {
                let pull = carry_buf.len().min(read_buf.len() - filled);
                read_buf[filled..filled + pull].copy_from_slice(&carry_buf[..pull]);
                carry_buf.drain(..pull);
                filled += pull;
                continue;
            }
            if let Some(deadline) = header_deadline {
                let now = Instant::now();
                if now >= deadline {
                    let _ = flush_pending_write_buf(&mut stream, write_buf);
                    return write_http_error_and_close(
                        &mut stream,
                        408,
                        r#"{"error":"HEADER_READ_TIMEOUT"}"#,
                    );
                }
                let rem = deadline
                    .saturating_duration_since(now)
                    .max(std::time::Duration::from_millis(1));
                stream.set_read_timeout(Some(rem));
                narrowed_timeout = true;
            }
            if filled >= read_buf.len() {
                let _ = flush_pending_write_buf(&mut stream, write_buf);
                return write_http_error_and_close(
                    &mut stream,
                    431,
                    r#"{"error":"REQUEST_HEADER_FIELDS_TOO_LARGE"}"#,
                );
            }
            flush_pending_write_buf(&mut stream, write_buf)?;
            match stream.read(&mut read_buf[filled..]) {
                Ok(0) => {
                    if filled > 0 {
                        return write_http_error_and_close(
                            &mut stream,
                            400,
                            r#"{"error":"INCOMPLETE_REQUEST_HEADERS"}"#,
                        );
                    }
                    return Ok(());
                }
                Err(ref e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    if filled > 0 {
                        return write_http_error_and_close(
                            &mut stream,
                            408,
                            r#"{"error":"HEADER_READ_TIMEOUT"}"#,
                        );
                    }
                    return Ok(());
                }
                Err(_) => return Ok(()),
                Ok(n) => {
                    if header_deadline.is_none() {
                        header_deadline = Some(Instant::now() + std::time::Duration::from_secs(10));
                    }
                    filled += n;
                }
            }
        };
        if narrowed_timeout {
            stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
        }

        let req_start = Instant::now();
        let (hdr_slice, after_hdr_buf) = read_buf.split_at_mut(body_offset);
        let Ok(hdr_str) = std::str::from_utf8(&hdr_slice[..hdr_end]) else {
            let _ = flush_pending_write_buf(&mut stream, write_buf);
            return write_http_error_and_close(
                &mut stream,
                400,
                r#"{"error":"INVALID_HEADER_UTF8"}"#,
            );
        };
        let mut lines = hdr_str.lines();
        let req_line = lines.next().unwrap_or("");

        // Zero-allocation request line split: `METHOD /path?query HTTP/1.1`
        let mut parts = req_line.split_whitespace();
        let method = parts.next().unwrap_or("");
        let raw_target = parts.next().unwrap_or("");
        let version = parts.next().unwrap_or("");
        if method.is_empty()
            || raw_target.is_empty()
            || parts.next().is_some()
            || (version != "HTTP/1.1" && version != "HTTP/1.0")
            || !slice_engine::is_valid_http_header_name(method)
            || !slice_engine::is_valid_request_target(raw_target)
        {
            let _ = flush_pending_write_buf(&mut stream, write_buf);
            return write_http_error_and_close(
                &mut stream,
                400,
                r#"{"error":"MALFORMED_REQUEST_LINE"}"#,
            );
        }
        let (raw_path, query) = raw_target.split_once('?').unwrap_or((raw_target, ""));

        let mut keep_alive = version == "HTTP/1.1";
        let mut seen_content_length: Option<usize> = None;
        let mut has_transfer_encoding = false;
        let mut is_chunked_te = false;
        let mut is_ws_upgrade = false;
        let mut sec_websocket_key: &str = "";
        let mut if_none_match: &str = "";
        let mut path = raw_path;
        let mut raw_tenant_id: &str = "tenant-default";
        if let Some(rest) = raw_path.strip_prefix("/t/") {
            if let Some((t_seg, sub)) = rest.split_once('/') {
                if !t_seg.is_empty() {
                    raw_tenant_id = t_seg;
                    path = if sub.is_empty() {
                        "/"
                    } else {
                        &raw_path[2 + t_seg.len()..]
                    };
                }
            } else if !rest.is_empty() {
                raw_tenant_id = rest;
                path = "/";
            }
        }
        // N-Sec-6: Support up to 64 request headers on the stack; reject > 64 with 431
        let mut headers_stack: [(&str, &str); 64] = [("", ""); 64];
        let mut headers_len: usize = 0;

        for line in lines {
            if line.is_empty() {
                continue;
            }
            let Some((raw_k, raw_v)) = line.split_once(':') else {
                let _ = flush_pending_write_buf(&mut stream, write_buf);
                return write_http_error_and_close(
                    &mut stream,
                    400,
                    r#"{"error":"MALFORMED_HEADER_LINE"}"#,
                );
            };
            // RFC 9112 Section 5.1: Reject leading (`obs-fold`) or trailing whitespace before `:` in header names.
            let k = raw_k;
            let v = raw_v.trim();
            if !slice_engine::is_valid_http_header_name(k)
                || !slice_engine::is_valid_http_header_value(v)
            {
                let _ = flush_pending_write_buf(&mut stream, write_buf);
                return write_http_error_and_close(
                    &mut stream,
                    400,
                    r#"{"error":"INVALID_HEADER_CHARACTERS"}"#,
                );
            }
            if headers_len >= headers_stack.len() {
                let _ = flush_pending_write_buf(&mut stream, write_buf);
                return write_http_error_and_close(
                    &mut stream,
                    431,
                    r#"{"error":"TOO_MANY_REQUEST_HEADERS"}"#,
                );
            }
            if k.eq_ignore_ascii_case("x-slice-tenant") && !v.is_empty() {
                raw_tenant_id = v;
            } else if k.eq_ignore_ascii_case("connection") {
                if contains_ascii_ignore_case(v, b"close") {
                    keep_alive = false;
                } else if contains_ascii_ignore_case(v, b"keep-alive") {
                    keep_alive = true;
                }
            } else if k.eq_ignore_ascii_case("upgrade") && v.eq_ignore_ascii_case("websocket") {
                is_ws_upgrade = true;
            } else if k.eq_ignore_ascii_case("sec-websocket-key") {
                sec_websocket_key = v;
            } else if k.eq_ignore_ascii_case("transfer-encoding") {
                // N-Sec-3: Reject requests with both Transfer-Encoding and Content-Length or duplicate Transfer-Encoding
                if seen_content_length.is_some() || has_transfer_encoding {
                    let _ = flush_pending_write_buf(&mut stream, write_buf);
                    return write_http_error_and_close(
                        &mut stream,
                        400,
                        r#"{"error":"SMUGGLING_CL_TE_CONFLICT_BLOCKED"}"#,
                    );
                }
                has_transfer_encoding = true;
                if is_chunked_transfer_encoding_only(v) {
                    is_chunked_te = true;
                } else {
                    let _ = flush_pending_write_buf(&mut stream, write_buf);
                    return write_http_error_and_close(
                        &mut stream,
                        400,
                        r#"{"error":"UNSUPPORTED_TRANSFER_ENCODING"}"#,
                    );
                }
            } else if k.eq_ignore_ascii_case("content-length") {
                // N-Sec-3: Reject requests with both Transfer-Encoding and Content-Length or conflicting duplicate Content-Length
                if has_transfer_encoding {
                    let _ = flush_pending_write_buf(&mut stream, write_buf);
                    return write_http_error_and_close(
                        &mut stream,
                        400,
                        r#"{"error":"SMUGGLING_CL_TE_CONFLICT_BLOCKED"}"#,
                    );
                }
                let Ok(parsed_len) = v.parse::<usize>() else {
                    let _ = flush_pending_write_buf(&mut stream, write_buf);
                    return write_http_error_and_close(
                        &mut stream,
                        400,
                        r#"{"error":"INVALID_CONTENT_LENGTH"}"#,
                    );
                };
                if let Some(prev_len) = seen_content_length {
                    if prev_len != parsed_len {
                        let _ = flush_pending_write_buf(&mut stream, write_buf);
                        return write_http_error_and_close(
                            &mut stream,
                            400,
                            r#"{"error":"SMUGGLING_DUPLICATE_CONTENT_LENGTH_BLOCKED"}"#,
                        );
                    }
                }
                if parsed_len > MAX_INBOUND_BODY_BYTES {
                    let _ = flush_pending_write_buf(&mut stream, write_buf);
                    return write_http_error_and_close(
                        &mut stream,
                        413,
                        r#"{"error":"PAYLOAD_TOO_LARGE"}"#,
                    );
                }
                seen_content_length = Some(parsed_len);
            } else if k.eq_ignore_ascii_case("if-none-match") {
                if_none_match = v;
            }
            headers_stack[headers_len] = (k, v);
            headers_len += 1;
        }

        let sanitized_tenant = super::sanitize_tenant_id(raw_tenant_id);
        let tenant_id: &str = if sanitized_tenant.is_empty() {
            "tenant-default"
        } else {
            sanitized_tenant.as_ref()
        };

        let content_length = seen_content_length.unwrap_or(0);
        let mut body_filled = filled.saturating_sub(body_offset);
        let mut owned_body_opt: Option<String> = None;
        let body_end: usize;

        if is_chunked_te {
            let mut decoder = slice_engine::ChunkedBodyDecoder::new(MAX_INBOUND_BODY_BYTES);
            let mut pending_chunk_buf =
                Vec::with_capacity(body_filled.saturating_add(carry_buf.len()).min(16384));
            if body_filled > 0 {
                pending_chunk_buf.extend_from_slice(&after_hdr_buf[..body_filled]);
            }
            if !carry_buf.is_empty() {
                pending_chunk_buf.extend_from_slice(&carry_buf);
                carry_buf.clear();
            }
            let mut raw_wire_seen = pending_chunk_buf.len();
            let mut scratch = [0u8; 8192];
            let mut body_deadline_opt: Option<Instant> = None;
            let (decoded, leftover_start) = loop {
                match decoder.feed(&pending_chunk_buf) {
                    Ok((consumed, true)) => break (decoder.into_vec(), consumed),
                    Ok((consumed, false)) => {
                        if consumed > 0 {
                            pending_chunk_buf.drain(..consumed);
                        }
                        if raw_wire_seen > MAX_INBOUND_BODY_BYTES.saturating_add(65536) {
                            let _ = flush_pending_write_buf(&mut stream, write_buf);
                            return write_http_error_and_close(
                                &mut stream,
                                413,
                                r#"{"error":"CHUNKED_PAYLOAD_TOO_LARGE"}"#,
                            );
                        }
                        let deadline = *body_deadline_opt.get_or_insert_with(|| {
                            Instant::now() + std::time::Duration::from_secs(30)
                        });
                        if Instant::now() >= deadline {
                            let _ = flush_pending_write_buf(&mut stream, write_buf);
                            return write_http_error_and_close(
                                &mut stream,
                                408,
                                r#"{"error":"BODY_READ_TIMEOUT"}"#,
                            );
                        }
                        flush_pending_write_buf(&mut stream, write_buf)?;
                        match stream.read(&mut scratch) {
                            Ok(0) => {
                                return write_http_error_and_close(
                                    &mut stream,
                                    400,
                                    r#"{"error":"TRUNCATED_CHUNKED_BODY"}"#,
                                );
                            }
                            Err(ref e)
                                if e.kind() == std::io::ErrorKind::WouldBlock
                                    || e.kind() == std::io::ErrorKind::TimedOut =>
                            {
                                return write_http_error_and_close(
                                    &mut stream,
                                    408,
                                    r#"{"error":"BODY_READ_TIMEOUT"}"#,
                                );
                            }
                            Err(_) => {
                                return write_http_error_and_close(
                                    &mut stream,
                                    400,
                                    r#"{"error":"TRUNCATED_CHUNKED_BODY"}"#,
                                );
                            }
                            Ok(n) => {
                                raw_wire_seen = raw_wire_seen.saturating_add(n);
                                pending_chunk_buf.extend_from_slice(&scratch[..n]);
                            }
                        }
                    }
                    Err(e) => {
                        let status = if e.kind() == std::io::ErrorKind::OutOfMemory {
                            413
                        } else {
                            400
                        };
                        let _ = flush_pending_write_buf(&mut stream, write_buf);
                        return write_http_error_and_close(
                            &mut stream,
                            status,
                            r#"{"error":"INVALID_CHUNKED_BODY"}"#,
                        );
                    }
                }
            };
            // Preserve any pipelined bytes that arrived after the chunked terminator without overflowing `after_hdr_buf`.
            let leftover = pending_chunk_buf.len().saturating_sub(leftover_start);
            let fit = leftover.min(after_hdr_buf.len());
            if fit > 0 {
                after_hdr_buf[..fit]
                    .copy_from_slice(&pending_chunk_buf[leftover_start..leftover_start + fit]);
            }
            if leftover > fit {
                carry_buf.extend_from_slice(&pending_chunk_buf[leftover_start + fit..]);
            }
            filled = body_offset + fit;
            body_end = body_offset;
            owned_body_opt = Some(String::from_utf8_lossy(&decoded).into_owned());
        } else if content_length <= after_hdr_buf.len() {
            if !carry_buf.is_empty() && body_filled < after_hdr_buf.len() {
                let pull = carry_buf.len().min(after_hdr_buf.len() - body_filled);
                after_hdr_buf[body_filled..body_filled + pull].copy_from_slice(&carry_buf[..pull]);
                carry_buf.drain(..pull);
                body_filled += pull;
            }
            if body_filled < content_length {
                let body_deadline = Instant::now() + std::time::Duration::from_secs(30);
                flush_pending_write_buf(&mut stream, write_buf)?;
                while body_filled < content_length {
                    if Instant::now() >= body_deadline {
                        return write_http_error_and_close(
                            &mut stream,
                            408,
                            r#"{"error":"BODY_READ_TIMEOUT"}"#,
                        );
                    }
                    match stream.read(&mut after_hdr_buf[body_filled..]) {
                        Ok(0) => return Ok(()),
                        Err(ref e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                || e.kind() == std::io::ErrorKind::TimedOut =>
                        {
                            return write_http_error_and_close(
                                &mut stream,
                                408,
                                r#"{"error":"BODY_READ_TIMEOUT"}"#,
                            );
                        }
                        Err(_) => return Ok(()),
                        Ok(n) => body_filled += n,
                    }
                }
            }
            filled = body_offset + body_filled;
            body_end = body_offset.saturating_add(content_length).min(filled);
        } else {
            // Spillover path for bodies between ~64 KB and `MAX_INBOUND_BODY_BYTES` (4 MB)
            let mut large_body = Vec::with_capacity(content_length);
            let initial_copy = body_filled.min(content_length);
            large_body.extend_from_slice(&after_hdr_buf[..initial_copy]);
            if !carry_buf.is_empty() && large_body.len() < content_length {
                let pull = carry_buf.len().min(content_length - large_body.len());
                large_body.extend_from_slice(&carry_buf[..pull]);
                carry_buf.drain(..pull);
            }
            if large_body.len() < content_length {
                let body_deadline = Instant::now() + std::time::Duration::from_secs(30);
                flush_pending_write_buf(&mut stream, write_buf)?;
                let mut scratch = [0u8; 8192];
                while large_body.len() < content_length {
                    if Instant::now() >= body_deadline {
                        return write_http_error_and_close(
                            &mut stream,
                            408,
                            r#"{"error":"BODY_READ_TIMEOUT"}"#,
                        );
                    }
                    let rem = (content_length - large_body.len()).min(scratch.len());
                    match stream.read(&mut scratch[..rem]) {
                        Ok(0) => return Ok(()),
                        Err(ref e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                || e.kind() == std::io::ErrorKind::TimedOut =>
                        {
                            return write_http_error_and_close(
                                &mut stream,
                                408,
                                r#"{"error":"BODY_READ_TIMEOUT"}"#,
                            );
                        }
                        Err(_) => return Ok(()),
                        Ok(n) => large_body.extend_from_slice(&scratch[..n]),
                    }
                }
            }
            if body_filled > initial_copy {
                after_hdr_buf.copy_within(initial_copy..body_filled, 0);
                filled = body_offset + (body_filled - initial_copy);
            } else {
                filled = body_offset;
            }
            body_end = body_offset;
            owned_body_opt = Some(String::from_utf8_lossy(&large_body).into_owned());
        }

        let body_len = content_length.min(body_filled);
        let conn_hdr = if keep_alive { "keep-alive" } else { "close" };
        let has_more_pipelined = keep_alive
            && ((body_end < filled
                && find_http_header_end(
                    &after_hdr_buf
                        [body_end.saturating_sub(body_offset)..filled.saturating_sub(body_offset)],
                )
                .is_some())
                || find_http_header_end(&carry_buf).is_some());

        // Phase F2: Zero-Stat CAS / In-Memory Static Asset Fast-Path (`< 2 us`, 0 JS VM spinup, 0 disk syscalls)
        if !is_ws_upgrade {
            if let Some(static_table) = static_assets_opt {
                if method == "GET" || method == "HEAD" {
                    if let Some(asset) = static_table.resolve(path) {
                        let is_304 = !if_none_match.is_empty()
                            && (if_none_match == "*"
                                || if_none_match.contains(&asset.etag_quoted)
                                || if_none_match.trim_matches('"')
                                    == asset.etag_quoted.trim_matches('"'));
                        let total_us = (req_start.elapsed().as_nanos() / 1_000) as u64;
                        if is_304 {
                            write_buf.extend_from_slice(b"HTTP/1.1 304 Not Modified\r\nETag: ");
                            write_buf.extend_from_slice(asset.etag_quoted.as_bytes());
                            write_buf.extend_from_slice(
                                b"\r\nCache-Control: public, max-age=3600\r\nConnection: ",
                            );
                            write_buf.extend_from_slice(conn_hdr.as_bytes());
                            write_buf.extend_from_slice(
                                b"\r\nServer: Slice-NexusJS/0.2\r\nX-Slice-Tenant: ",
                            );
                            write_buf.extend_from_slice(tenant_id.as_bytes());
                            write_buf.extend_from_slice(
                                b"\r\nX-Slice-Static: HIT-304\r\nX-Slice-Isolate-Us: 0\r\nX-Slice-Total-Us: ",
                            );
                            push_u64_ascii(write_buf, total_us);
                            write_buf.extend_from_slice(b"\r\nX-Slice-Fuel-Used: 0\r\n\r\n");
                        } else {
                            write_buf.extend_from_slice(b"HTTP/1.1 200 OK\r\nContent-Type: ");
                            write_buf.extend_from_slice(asset.content_type.as_bytes());
                            write_buf.extend_from_slice(b"\r\nContent-Length: ");
                            push_u64_ascii(write_buf, asset.payload.len() as u64);
                            write_buf.extend_from_slice(b"\r\nETag: ");
                            write_buf.extend_from_slice(asset.etag_quoted.as_bytes());
                            write_buf.extend_from_slice(
                                b"\r\nCache-Control: public, max-age=3600\r\nConnection: ",
                            );
                            write_buf.extend_from_slice(conn_hdr.as_bytes());
                            write_buf.extend_from_slice(
                                b"\r\nServer: Slice-NexusJS/0.2\r\nX-Slice-Tenant: ",
                            );
                            write_buf.extend_from_slice(tenant_id.as_bytes());
                            write_buf.extend_from_slice(
                                b"\r\nX-Slice-Static: HIT-200\r\nX-Slice-Isolate-Us: 0\r\nX-Slice-Total-Us: ",
                            );
                            push_u64_ascii(write_buf, total_us);
                            write_buf.extend_from_slice(b"\r\nX-Slice-Fuel-Used: 0\r\n\r\n");
                            if method != "HEAD" {
                                if !has_more_pipelined
                                    || asset.payload.len() > STATIC_COALESCE_MAX_BYTES
                                    || !keep_alive
                                {
                                    stream.write_all(write_buf)?;
                                    clear_and_trim_write_buf(write_buf);
                                    for chunk in asset.payload.chunks(65536) {
                                        stream.write_all(chunk)?;
                                    }
                                } else {
                                    write_buf.extend_from_slice(&asset.payload);
                                }
                            }
                        }
                        let reached_max = if let Some((counter, max_reqs)) = max_requests_tracker {
                            let prev = counter.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                            max_reqs > 0 && prev + 1 >= max_reqs
                        } else {
                            false
                        };
                        if !has_more_pipelined
                            || write_buf.len() >= 32768
                            || !keep_alive
                            || reached_max
                        {
                            if !write_buf.is_empty() {
                                stream.write_all(write_buf)?;
                                clear_and_trim_write_buf(write_buf);
                            }
                            if !keep_alive || reached_max {
                                stream.flush()?;
                            }
                        }
                        if !keep_alive || reached_max {
                            break;
                        }
                        if filled > body_end {
                            read_buf.copy_within(body_end..filled, 0);
                            filled -= body_end;
                        } else {
                            filled = 0;
                        }
                        continue;
                    }
                }
            }
        }

        let body_str: &str = if let Some(ref owned) = owned_body_opt {
            owned.as_str()
        } else {
            std::str::from_utf8(&after_hdr_buf[..body_len]).unwrap_or("")
        };
        let req_ctx = slice_engine::HttpRequestContext {
            method,
            path,
            query,
            tenant_id,
            body: body_str,
            headers: &headers_stack[..headers_len],
        };

        slot_id = slot_id.wrapping_add(1);
        isolate.reset_for_request(tenant_id, slot_id, fuel_limit, mem_limit);

        let dynamic_bc =
            tenant_dir_opt.and_then(|td| resolve_tenant_bytecode_hotswap(td, tenant_id));
        let active_bytecode: &[u8] = dynamic_bc
            .as_deref()
            .map(|v| v.as_slice())
            .unwrap_or(bytecode);

        match isolate.execute_http_request_fast(active_bytecode, None, &req_ctx) {
            Ok(outcome) => {
                // Only enter RFC 6455 Server-Side WebSocket Upgrade when the handler explicitly upgraded (`101`).
                if is_ws_upgrade
                    && !sec_websocket_key.is_empty()
                    && outcome.ws_upgraded
                    && outcome.status_code == 101
                {
                    flush_pending_write_buf(&mut stream, write_buf)?;
                    let accept_key = slice_engine::compute_websocket_accept_key(sec_websocket_key);
                    write_buf.extend_from_slice(
                        b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ",
                    );
                    write_buf.extend_from_slice(accept_key.as_bytes());
                    write_buf
                        .extend_from_slice(b"\r\nServer: Slice-NexusJS/0.2\r\nX-Slice-Tenant: ");
                    write_buf.extend_from_slice(tenant_id.as_bytes());
                    write_buf.extend_from_slice(b"\r\n\r\n");
                    stream.write_all(write_buf)?;
                    stream.flush()?;
                    write_buf.clear();

                    if let Ok(open_frames) =
                        isolate.execute_websocket_event("open", false, &[], 0, "")
                    {
                        if flush_server_ws_frames(&mut stream, open_frames)? {
                            if let Some((counter, _)) = max_requests_tracker {
                                counter.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                            }
                            return Ok(());
                        }
                    }

                    let mut ws_pending = Vec::new();
                    if filled > body_end {
                        ws_pending.extend_from_slice(&read_buf[body_end..filled]);
                    }
                    if !carry_buf.is_empty() {
                        ws_pending.extend_from_slice(&carry_buf);
                        carry_buf.clear();
                    }
                    let effective_ws_limit = mem_limit.min(slice_engine::MAX_WS_FRAME_BYTES);
                    loop {
                        match slice_engine::read_websocket_message_from_stream(
                            &mut stream,
                            &mut ws_pending,
                            true,
                            effective_ws_limit,
                        ) {
                            Ok(Some((slice_engine::WS_OPCODE_CLOSE, payload))) => {
                                let (code, reason) =
                                    slice_engine::parse_close_frame_payload(&payload);
                                let _ = isolate.execute_websocket_event(
                                    "close",
                                    false,
                                    &[],
                                    code,
                                    &reason,
                                );
                                let reply_payload =
                                    slice_engine::build_close_frame_payload(code, &reason);
                                let close_frame = slice_engine::encode_websocket_frame(
                                    true,
                                    slice_engine::WS_OPCODE_CLOSE,
                                    None,
                                    &reply_payload,
                                );
                                let _ = stream.write_all(&close_frame);
                                let _ = stream.flush();
                                break;
                            }
                            Ok(Some((opcode, payload))) => {
                                let is_bin = opcode == slice_engine::WS_OPCODE_BINARY;
                                match isolate
                                    .execute_websocket_event("message", is_bin, &payload, 0, "")
                                {
                                    Ok(out_frames) => {
                                        if flush_server_ws_frames(&mut stream, out_frames)? {
                                            break;
                                        }
                                    }
                                    Err(_violation) => {
                                        let close_payload = slice_engine::build_close_frame_payload(
                                            1008,
                                            "Policy Violation",
                                        );
                                        let close_frame = slice_engine::encode_websocket_frame(
                                            true,
                                            slice_engine::WS_OPCODE_CLOSE,
                                            None,
                                            &close_payload,
                                        );
                                        let _ = stream.write_all(&close_frame);
                                        let _ = stream.flush();
                                        break;
                                    }
                                }
                            }
                            Ok(None) | Err(_) => break,
                        }
                    }
                    if let Some((counter, _)) = max_requests_tracker {
                        counter.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                    }
                    return Ok(());
                }

                let isolate_us = outcome.total_time_ns / 1_000;
                let total_us = (req_start.elapsed().as_nanos() / 1_000) as u64;
                let safe_ct = if slice_engine::is_valid_http_header_value(&outcome.content_type) {
                    outcome.content_type.as_ref()
                } else {
                    "application/octet-stream"
                };
                let body_bytes = outcome.body_bytes();
                let is_bodyless_status = matches!(outcome.status_code, 100..=199 | 204 | 304);
                let suppress_wire_body = method == "HEAD" || is_bodyless_status;

                // CLI-PERF-02: Zero-format-macro byte slice + `itoa` response header serialization
                write_buf.extend_from_slice(b"HTTP/1.1 ");
                push_u64_ascii(write_buf, outcome.status_code as u64);
                write_buf.push(b' ');
                write_buf.extend_from_slice(http_status_reason(outcome.status_code).as_bytes());
                write_buf.extend_from_slice(b"\r\n");
                if !is_bodyless_status {
                    write_buf.extend_from_slice(b"Content-Type: ");
                    write_buf.extend_from_slice(safe_ct.as_bytes());
                    write_buf.extend_from_slice(b"\r\n");
                    if safe_ct.starts_with("text/event-stream") {
                        write_buf.extend_from_slice(
                            b"Cache-Control: no-cache\r\nX-Accel-Buffering: no\r\n",
                        );
                    }
                    write_buf.extend_from_slice(b"Content-Length: ");
                    push_u64_ascii(write_buf, body_bytes.len() as u64);
                    write_buf.extend_from_slice(b"\r\n");
                }
                write_buf.extend_from_slice(b"Connection: ");
                write_buf.extend_from_slice(conn_hdr.as_bytes());
                write_buf.extend_from_slice(b"\r\nServer: Slice-NexusJS/0.2\r\nX-Slice-Tenant: ");
                write_buf.extend_from_slice(tenant_id.as_bytes());
                write_buf.extend_from_slice(b"\r\nX-Slice-Isolate-Us: ");
                push_u64_ascii(write_buf, isolate_us);
                write_buf.extend_from_slice(b"\r\nX-Slice-Total-Us: ");
                push_u64_ascii(write_buf, total_us);
                write_buf.extend_from_slice(b"\r\nX-Slice-Fuel-Used: ");
                push_u64_ascii(write_buf, outcome.fuel_consumed);
                write_buf.extend_from_slice(b"\r\n");

                // N-Sec-2: Validate custom response headers against CRLF/control chars and hop-by-hop framing injection
                for (hk, hv) in &outcome.response_headers {
                    if !is_forbidden_custom_response_header(hk)
                        && slice_engine::is_valid_http_header_name(hk)
                        && slice_engine::is_valid_http_header_value(hv)
                    {
                        write_buf.extend_from_slice(hk.as_bytes());
                        write_buf.extend_from_slice(b": ");
                        write_buf.extend_from_slice(hv.as_bytes());
                        write_buf.extend_from_slice(b"\r\n");
                    }
                }
                write_buf.extend_from_slice(b"\r\n");
                if !suppress_wire_body {
                    write_buf.extend_from_slice(body_bytes);
                }
                if !has_more_pipelined || write_buf.len() >= 32768 {
                    stream.write_all(write_buf)?;
                    clear_and_trim_write_buf(write_buf);
                }

                // Phase N1.3: Execute deferred `Slice.waitUntil(fn)` tasks AFTER response is prepared/sent!
                if outcome.wait_until_count > 0 {
                    let _ = isolate.drain_wait_until_tasks();
                }
            }
            Err(violation) => {
                // CLI-SEC-01: Escape quotes/backslashes/control chars in violation string for valid JSON
                let violation_raw = violation.to_string();
                let mut err_body = String::with_capacity(violation_raw.len() + 16);
                err_body.push_str(r#"{"error":""#);
                escape_json_string_into(&violation_raw, &mut err_body);
                err_body.push_str(r#""}"#);

                write_buf.extend_from_slice(
                    b"HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: ",
                );
                push_u64_ascii(write_buf, err_body.len() as u64);
                write_buf.extend_from_slice(b"\r\nConnection: ");
                write_buf.extend_from_slice(conn_hdr.as_bytes());
                write_buf.extend_from_slice(b"\r\nServer: Slice-NexusJS/0.2\r\n\r\n");
                if method != "HEAD" {
                    write_buf.extend_from_slice(err_body.as_bytes());
                }
                stream.write_all(write_buf)?;
                clear_and_trim_write_buf(write_buf);
            }
        }

        let reached_max = if let Some((counter, max_reqs)) = max_requests_tracker {
            let prev = counter.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            max_reqs > 0 && prev + 1 >= max_reqs
        } else {
            false
        };

        if !keep_alive || reached_max {
            flush_pending_write_buf(&mut stream, write_buf)?;
            break;
        }
        if filled > body_end {
            read_buf.copy_within(body_end..filled, 0);
            filled -= body_end;
        } else {
            filled = 0;
        }
    }
    flush_pending_write_buf(&mut stream, write_buf)?;
    Ok(())
}
