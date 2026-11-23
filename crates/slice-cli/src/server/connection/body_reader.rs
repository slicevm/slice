//! Bounded HTTP/1.1 header framing, request-line/header validation, and chunked/fixed body reader.

use super::super::tls::ServerConnStream;
use super::wire::{
    MAX_INBOUND_BODY_BYTES, contains_ascii_ignore_case, extract_tenant_route_prefix,
    find_http_header_end_from, flush_pending_write_buf, is_chunked_transfer_encoding_only,
    write_http_error_and_close,
};
use anyhow::Result;
use std::io::Read;
use std::time::{Duration, Instant};

pub(crate) struct ParsedRequestHeaders<'a> {
    pub(crate) method: &'a str,
    pub(crate) path: &'a str,
    pub(crate) query: &'a str,
    pub(crate) raw_tenant_id: &'a str,
    pub(crate) keep_alive: bool,
    pub(crate) content_length: usize,
    pub(crate) is_chunked_te: bool,
    pub(crate) is_ws_upgrade: bool,
    pub(crate) sec_websocket_key: &'a str,
    pub(crate) if_none_match: &'a str,
    pub(crate) headers_stack: [(&'a str, &'a str); 64],
    pub(crate) headers_len: usize,
}

pub(crate) struct BodyReadOutcome {
    pub(crate) body_end: usize,
    pub(crate) body_len: usize,
    pub(crate) owned_body_bytes: Option<Vec<u8>>,
}

/// Reads from `stream` into `read_buf[..filled]` until complete HTTP headers (`\r\n\r\n` or `\n\n`)
/// are present. Returns `Ok(None)` when the connection closes cleanly or an HTTP error (`400`, `408`,
/// `431`) was written and closed.
pub(crate) fn read_http_header_frame(
    stream: &mut ServerConnStream,
    read_buf: &mut [u8],
    filled: &mut usize,
    carry_buf: &mut Vec<u8>,
    write_buf: &mut Vec<u8>,
) -> Result<Option<(usize, usize)>> {
    if !carry_buf.is_empty() && *filled < read_buf.len() {
        let pull = carry_buf.len().min(read_buf.len() - *filled);
        read_buf[*filled..*filled + pull].copy_from_slice(&carry_buf[..pull]);
        carry_buf.drain(..pull);
        *filled += pull;
        if carry_buf.is_empty() && carry_buf.capacity() > 65536 {
            carry_buf.shrink_to(0);
        }
    }

    let mut header_deadline: Option<Instant> = if *filled > 0 {
        Some(Instant::now() + Duration::from_secs(10))
    } else {
        None
    };
    let mut narrowed_timeout = false;
    let mut hdr_scan_cursor = 0usize;

    let pos = loop {
        if let Some(pos) = find_http_header_end_from(&read_buf[..*filled], hdr_scan_cursor) {
            break pos;
        }
        hdr_scan_cursor = filled.saturating_sub(3);
        if !carry_buf.is_empty() && *filled < read_buf.len() {
            let pull = carry_buf.len().min(read_buf.len() - *filled);
            read_buf[*filled..*filled + pull].copy_from_slice(&carry_buf[..pull]);
            carry_buf.drain(..pull);
            *filled += pull;
            continue;
        }
        if let Some(deadline) = header_deadline {
            let now = Instant::now();
            if now >= deadline {
                let _ = flush_pending_write_buf(stream, write_buf);
                write_http_error_and_close(stream, 408, r#"{"error":"HEADER_READ_TIMEOUT"}"#)?;
                return Ok(None);
            }
            let rem = deadline
                .saturating_duration_since(now)
                .max(Duration::from_millis(1));
            stream.set_read_timeout(Some(rem));
            narrowed_timeout = true;
        }
        if *filled >= read_buf.len() {
            let _ = flush_pending_write_buf(stream, write_buf);
            write_http_error_and_close(
                stream,
                431,
                r#"{"error":"REQUEST_HEADER_FIELDS_TOO_LARGE"}"#,
            )?;
            return Ok(None);
        }
        flush_pending_write_buf(stream, write_buf)?;
        match stream.read(&mut read_buf[*filled..]) {
            Ok(0) => {
                if *filled > 0 {
                    write_http_error_and_close(
                        stream,
                        400,
                        r#"{"error":"INCOMPLETE_REQUEST_HEADERS"}"#,
                    )?;
                }
                return Ok(None);
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                if *filled > 0 {
                    write_http_error_and_close(stream, 408, r#"{"error":"HEADER_READ_TIMEOUT"}"#)?;
                }
                return Ok(None);
            }
            Err(_) => return Ok(None),
            Ok(n) => {
                if header_deadline.is_none() {
                    header_deadline = Some(Instant::now() + Duration::from_secs(10));
                }
                *filled += n;
            }
        }
    };

    if narrowed_timeout {
        stream.set_read_timeout(Some(Duration::from_secs(10)));
    }
    Ok(Some(pos))
}

/// Validates and parses the HTTP request line and up to 64 stack-allocated headers.
/// Returns `Ok(None)` when a `400`/`413`/`431` error response has been written and closed.
pub(crate) fn parse_request_headers<'a>(
    stream: &mut ServerConnStream,
    hdr_bytes: &'a [u8],
    write_buf: &mut Vec<u8>,
) -> Result<Option<ParsedRequestHeaders<'a>>> {
    let Ok(hdr_str) = std::str::from_utf8(hdr_bytes) else {
        let _ = flush_pending_write_buf(stream, write_buf);
        write_http_error_and_close(stream, 400, r#"{"error":"INVALID_HEADER_UTF8"}"#)?;
        return Ok(None);
    };
    let mut lines = hdr_str.lines();
    let req_line = lines.next().unwrap_or("");

    // Zero-allocation request line split: `METHOD /path?query HTTP/1.1`
    let mut parts = req_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let raw_target = parts.next().unwrap_or("");
    let version = parts.next().unwrap_or("");
    if method.is_empty()
        || !raw_target.starts_with('/')
        || parts.next().is_some()
        || (version != "HTTP/1.1" && version != "HTTP/1.0")
        || !slice_engine::is_valid_http_header_name(method)
        || !slice_engine::is_valid_request_target(raw_target)
    {
        let _ = flush_pending_write_buf(stream, write_buf);
        write_http_error_and_close(stream, 400, r#"{"error":"MALFORMED_REQUEST_LINE"}"#)?;
        return Ok(None);
    }
    let (raw_path, query) = raw_target.split_once('?').unwrap_or((raw_target, ""));

    let mut keep_alive = version == "HTTP/1.1";
    let mut seen_content_length: Option<usize> = None;
    let mut has_transfer_encoding = false;
    let mut is_chunked_te = false;
    let mut is_ws_upgrade = false;
    let mut sec_websocket_key: &str = "";
    let mut if_none_match: &str = "";
    let (mut raw_tenant_id, path) = extract_tenant_route_prefix(raw_path);
    // N-Sec-6: Support up to 64 request headers on the stack; reject > 64 with 431
    let mut headers_stack: [(&str, &str); 64] = [("", ""); 64];
    let mut headers_len: usize = 0;

    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((raw_k, raw_v)) = line.split_once(':') else {
            let _ = flush_pending_write_buf(stream, write_buf);
            write_http_error_and_close(stream, 400, r#"{"error":"MALFORMED_HEADER_LINE"}"#)?;
            return Ok(None);
        };
        // RFC 9112 Section 5.1: Reject leading (`obs-fold`) or trailing whitespace before `:` in header names.
        let k = raw_k;
        let v = raw_v.trim();
        if !slice_engine::is_valid_http_header_name(k)
            || !slice_engine::is_valid_http_header_value(v)
        {
            let _ = flush_pending_write_buf(stream, write_buf);
            write_http_error_and_close(stream, 400, r#"{"error":"INVALID_HEADER_CHARACTERS"}"#)?;
            return Ok(None);
        }
        if headers_len >= headers_stack.len() {
            let _ = flush_pending_write_buf(stream, write_buf);
            write_http_error_and_close(stream, 431, r#"{"error":"TOO_MANY_REQUEST_HEADERS"}"#)?;
            return Ok(None);
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
                let _ = flush_pending_write_buf(stream, write_buf);
                write_http_error_and_close(
                    stream,
                    400,
                    r#"{"error":"SMUGGLING_CL_TE_CONFLICT_BLOCKED"}"#,
                )?;
                return Ok(None);
            }
            has_transfer_encoding = true;
            if is_chunked_transfer_encoding_only(v) {
                is_chunked_te = true;
            } else {
                let _ = flush_pending_write_buf(stream, write_buf);
                write_http_error_and_close(
                    stream,
                    400,
                    r#"{"error":"UNSUPPORTED_TRANSFER_ENCODING"}"#,
                )?;
                return Ok(None);
            }
        } else if k.eq_ignore_ascii_case("content-length") {
            // N-Sec-3: Reject requests with both Transfer-Encoding and Content-Length or conflicting duplicate Content-Length
            if has_transfer_encoding {
                let _ = flush_pending_write_buf(stream, write_buf);
                write_http_error_and_close(
                    stream,
                    400,
                    r#"{"error":"SMUGGLING_CL_TE_CONFLICT_BLOCKED"}"#,
                )?;
                return Ok(None);
            }
            let Ok(parsed_len) = v.parse::<usize>() else {
                let _ = flush_pending_write_buf(stream, write_buf);
                write_http_error_and_close(stream, 400, r#"{"error":"INVALID_CONTENT_LENGTH"}"#)?;
                return Ok(None);
            };
            if let Some(prev_len) = seen_content_length
                && prev_len != parsed_len
            {
                let _ = flush_pending_write_buf(stream, write_buf);
                write_http_error_and_close(
                    stream,
                    400,
                    r#"{"error":"SMUGGLING_DUPLICATE_CONTENT_LENGTH_BLOCKED"}"#,
                )?;
                return Ok(None);
            }
            if parsed_len > MAX_INBOUND_BODY_BYTES {
                let _ = flush_pending_write_buf(stream, write_buf);
                write_http_error_and_close(stream, 413, r#"{"error":"PAYLOAD_TOO_LARGE"}"#)?;
                return Ok(None);
            }
            seen_content_length = Some(parsed_len);
        } else if k.eq_ignore_ascii_case("if-none-match") {
            if_none_match = v;
        }
        headers_stack[headers_len] = (k, v);
        headers_len += 1;
    }

    Ok(Some(ParsedRequestHeaders {
        method,
        path,
        query,
        raw_tenant_id,
        keep_alive,
        content_length: seen_content_length.unwrap_or(0),
        is_chunked_te,
        is_ws_upgrade,
        sec_websocket_key,
        if_none_match,
        headers_stack,
        headers_len,
    }))
}

/// Reads the inbound HTTP request body (chunked `Transfer-Encoding`, in-buffer `Content-Length`,
/// or spillover up to `MAX_INBOUND_BODY_BYTES`). Returns `Ok(None)` when the socket closes early
/// or an error response (`400`, `408`, `413`) has been written and closed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn read_request_body(
    stream: &mut ServerConnStream,
    after_hdr_buf: &mut [u8],
    body_offset: usize,
    filled: &mut usize,
    carry_buf: &mut Vec<u8>,
    write_buf: &mut Vec<u8>,
    is_chunked_te: bool,
    content_length: usize,
) -> Result<Option<BodyReadOutcome>> {
    let mut body_filled = filled.saturating_sub(body_offset);
    let mut owned_body_bytes: Option<Vec<u8>> = None;
    let body_end: usize;

    if is_chunked_te {
        let mut decoder = slice_engine::ChunkedBodyDecoder::new(MAX_INBOUND_BODY_BYTES);
        let mut pending_chunk_buf =
            Vec::with_capacity(body_filled.saturating_add(carry_buf.len()).min(16384));
        if body_filled > 0 {
            pending_chunk_buf.extend_from_slice(&after_hdr_buf[..body_filled]);
        }
        if !carry_buf.is_empty() {
            pending_chunk_buf.extend_from_slice(carry_buf);
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
                        let _ = flush_pending_write_buf(stream, write_buf);
                        write_http_error_and_close(
                            stream,
                            413,
                            r#"{"error":"CHUNKED_PAYLOAD_TOO_LARGE"}"#,
                        )?;
                        return Ok(None);
                    }
                    let deadline = *body_deadline_opt
                        .get_or_insert_with(|| Instant::now() + Duration::from_secs(30));
                    if Instant::now() >= deadline {
                        let _ = flush_pending_write_buf(stream, write_buf);
                        write_http_error_and_close(
                            stream,
                            408,
                            r#"{"error":"BODY_READ_TIMEOUT"}"#,
                        )?;
                        return Ok(None);
                    }
                    flush_pending_write_buf(stream, write_buf)?;
                    match stream.read(&mut scratch) {
                        Ok(0) => {
                            write_http_error_and_close(
                                stream,
                                400,
                                r#"{"error":"TRUNCATED_CHUNKED_BODY"}"#,
                            )?;
                            return Ok(None);
                        }
                        Err(ref e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                || e.kind() == std::io::ErrorKind::TimedOut =>
                        {
                            write_http_error_and_close(
                                stream,
                                408,
                                r#"{"error":"BODY_READ_TIMEOUT"}"#,
                            )?;
                            return Ok(None);
                        }
                        Err(_) => {
                            write_http_error_and_close(
                                stream,
                                400,
                                r#"{"error":"TRUNCATED_CHUNKED_BODY"}"#,
                            )?;
                            return Ok(None);
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
                    let _ = flush_pending_write_buf(stream, write_buf);
                    write_http_error_and_close(
                        stream,
                        status,
                        r#"{"error":"INVALID_CHUNKED_BODY"}"#,
                    )?;
                    return Ok(None);
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
        *filled = body_offset + fit;
        body_end = body_offset;
        owned_body_bytes = Some(decoded);
    } else if content_length <= after_hdr_buf.len() {
        if !carry_buf.is_empty() && body_filled < after_hdr_buf.len() {
            let pull = carry_buf.len().min(after_hdr_buf.len() - body_filled);
            after_hdr_buf[body_filled..body_filled + pull].copy_from_slice(&carry_buf[..pull]);
            carry_buf.drain(..pull);
            body_filled += pull;
        }
        if body_filled < content_length {
            let body_deadline = Instant::now() + Duration::from_secs(30);
            flush_pending_write_buf(stream, write_buf)?;
            while body_filled < content_length {
                if Instant::now() >= body_deadline {
                    write_http_error_and_close(stream, 408, r#"{"error":"BODY_READ_TIMEOUT"}"#)?;
                    return Ok(None);
                }
                match stream.read(&mut after_hdr_buf[body_filled..]) {
                    Ok(0) => return Ok(None),
                    Err(ref e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            || e.kind() == std::io::ErrorKind::TimedOut =>
                    {
                        write_http_error_and_close(
                            stream,
                            408,
                            r#"{"error":"BODY_READ_TIMEOUT"}"#,
                        )?;
                        return Ok(None);
                    }
                    Err(_) => return Ok(None),
                    Ok(n) => body_filled += n,
                }
            }
        }
        *filled = body_offset + body_filled;
        body_end = body_offset.saturating_add(content_length).min(*filled);
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
            let body_deadline = Instant::now() + Duration::from_secs(30);
            flush_pending_write_buf(stream, write_buf)?;
            let mut scratch = [0u8; 8192];
            while large_body.len() < content_length {
                if Instant::now() >= body_deadline {
                    write_http_error_and_close(stream, 408, r#"{"error":"BODY_READ_TIMEOUT"}"#)?;
                    return Ok(None);
                }
                let rem = (content_length - large_body.len()).min(scratch.len());
                match stream.read(&mut scratch[..rem]) {
                    Ok(0) => return Ok(None),
                    Err(ref e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            || e.kind() == std::io::ErrorKind::TimedOut =>
                    {
                        write_http_error_and_close(
                            stream,
                            408,
                            r#"{"error":"BODY_READ_TIMEOUT"}"#,
                        )?;
                        return Ok(None);
                    }
                    Err(_) => return Ok(None),
                    Ok(n) => large_body.extend_from_slice(&scratch[..n]),
                }
            }
        }
        if body_filled > initial_copy {
            after_hdr_buf.copy_within(initial_copy..body_filled, 0);
            *filled = body_offset + (body_filled - initial_copy);
        } else {
            *filled = body_offset;
        }
        body_end = body_offset;
        owned_body_bytes = Some(large_body);
    }

    Ok(Some(BodyReadOutcome {
        body_end,
        body_len: content_length.min(body_filled),
        owned_body_bytes,
    }))
}
