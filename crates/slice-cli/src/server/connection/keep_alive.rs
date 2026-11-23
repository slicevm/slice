//! HTTP/1.1 Keep-Alive connection loop, Zero-Stat static asset fast-path, and WinterTC dispatch.

use super::super::tls::ServerConnStream;
use super::super::{resolve_tenant_cas_hotswap, sanitize_tenant_id};
use super::body_reader::{parse_request_headers, read_http_header_frame, read_request_body};
use super::websocket::handle_upgraded_websocket_session;
use super::wire::{
    STATIC_COALESCE_MAX_BYTES, clear_and_trim_write_buf, find_http_header_end,
    flush_pending_write_buf, http_status_reason, is_forbidden_custom_response_header,
    push_u64_ascii, write_all_vectored2,
};
use anyhow::Result;
use slice_engine::SliceIsolate;
use std::io::Write;
use std::net::TcpStream;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

#[allow(clippy::too_many_arguments)]
pub(crate) fn serve_connection_with_isolate(
    tcp_stream: TcpStream,
    mut slot_id: u32,
    isolate: &mut SliceIsolate,
    bytecode: &[u8],
    cas_archive_opt: Option<&slice_cas::MappedCasArchive>,
    tenant_dir_opt: Option<&Path>,
    static_assets_opt: Option<&slice_cas::StaticAssetTable>,
    fuel_limit: u64,
    mem_limit: usize,
    read_buf: &mut [u8],
    write_buf: &mut Vec<u8>,
    max_requests_tracker: Option<(&AtomicUsize, usize)>,
    tls_server_cfg: Option<&std::sync::Arc<rustls::ServerConfig>>,
) -> Result<()> {
    let _ = tcp_stream.set_nodelay(true);
    // N-Sec-4: Defend against Slowloris socket starvation with 10s read/write socket timeouts
    let _ = tcp_stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = tcp_stream.set_write_timeout(Some(Duration::from_secs(10)));
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
        let Some((hdr_end, body_offset)) = read_http_header_frame(
            &mut stream,
            read_buf,
            &mut filled,
            &mut carry_buf,
            write_buf,
        )?
        else {
            return Ok(());
        };

        let req_start = Instant::now();
        let (hdr_slice, after_hdr_buf) = read_buf.split_at_mut(body_offset);
        let Some(parsed) = parse_request_headers(&mut stream, &hdr_slice[..hdr_end], write_buf)?
        else {
            return Ok(());
        };

        let sanitized_tenant = sanitize_tenant_id(parsed.raw_tenant_id);
        let tenant_id: &str = if sanitized_tenant.is_empty() {
            "tenant-default"
        } else {
            sanitized_tenant.as_ref()
        };

        let Some(body_frame) = read_request_body(
            &mut stream,
            after_hdr_buf,
            body_offset,
            &mut filled,
            &mut carry_buf,
            write_buf,
            parsed.is_chunked_te,
            parsed.content_length,
        )?
        else {
            return Ok(());
        };

        let body_end = body_frame.body_end;
        let keep_alive = parsed.keep_alive;
        let method = parsed.method;
        let path = parsed.path;
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
        if !parsed.is_ws_upgrade
            && let Some(static_table) = static_assets_opt
            && (method == "GET" || method == "HEAD")
            && let Some(asset) = static_table.resolve(path)
        {
            let inm = parsed.if_none_match;
            let is_304 = !inm.is_empty()
                && (inm == "*"
                    || inm.contains(&asset.etag_quoted)
                    || inm.trim_matches('"') == asset.etag_quoted.trim_matches('"'));
            let total_us = (req_start.elapsed().as_nanos() / 1_000) as u64;
            if is_304 {
                write_buf.extend_from_slice(b"HTTP/1.1 304 Not Modified\r\nETag: ");
                write_buf.extend_from_slice(asset.etag_quoted.as_bytes());
                write_buf
                    .extend_from_slice(b"\r\nCache-Control: public, max-age=3600\r\nConnection: ");
                write_buf.extend_from_slice(conn_hdr.as_bytes());
                write_buf.extend_from_slice(b"\r\nServer: Slice-NexusJS/0.2\r\nX-Slice-Tenant: ");
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
                write_buf
                    .extend_from_slice(b"\r\nCache-Control: public, max-age=3600\r\nConnection: ");
                write_buf.extend_from_slice(conn_hdr.as_bytes());
                write_buf.extend_from_slice(b"\r\nServer: Slice-NexusJS/0.2\r\nX-Slice-Tenant: ");
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
                        write_all_vectored2(&mut stream, write_buf, &asset.payload)?;
                        clear_and_trim_write_buf(write_buf);
                    } else {
                        write_buf.extend_from_slice(&asset.payload);
                    }
                }
            }
            let reached_max = if let Some((counter, max_reqs)) = max_requests_tracker {
                let prev = counter.fetch_add(1, Ordering::AcqRel);
                max_reqs > 0 && prev + 1 >= max_reqs
            } else {
                false
            };
            if !has_more_pipelined || write_buf.len() >= 32768 || !keep_alive || reached_max {
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

        let raw_body_bytes: &[u8] = if let Some(ref owned) = body_frame.owned_body_bytes {
            owned.as_slice()
        } else {
            &after_hdr_buf[..body_frame.body_len]
        };
        let req_ctx = slice_engine::HttpRequestContext {
            method,
            path,
            query: parsed.query,
            tenant_id,
            body: "",
            body_bytes: Some(raw_body_bytes),
            headers: &parsed.headers_stack[..parsed.headers_len],
        };

        slot_id = slot_id.wrapping_add(1);
        isolate.reset_for_request(tenant_id, slot_id, fuel_limit, mem_limit);

        let dynamic_cas = tenant_dir_opt.and_then(|td| resolve_tenant_cas_hotswap(td, tenant_id));
        let active_bytecode: &[u8] = dynamic_cas
            .as_ref()
            .map(|(bc, _)| bc.as_slice())
            .unwrap_or(bytecode);
        let active_cas: Option<&slice_cas::MappedCasArchive> = dynamic_cas
            .as_ref()
            .map(|(_, cas)| cas.as_ref())
            .or(cas_archive_opt);

        match isolate.execute_http_request_fast(active_bytecode, active_cas, &req_ctx) {
            Ok(outcome) => {
                // Only enter RFC 6455 Server-Side WebSocket Upgrade when the handler explicitly upgraded (`101`).
                if parsed.is_ws_upgrade
                    && !parsed.sec_websocket_key.is_empty()
                    && outcome.ws_upgraded
                    && outcome.status_code == 101
                {
                    let trailing_slice = if filled > body_end {
                        &after_hdr_buf[body_end.saturating_sub(body_offset)
                            ..filled.saturating_sub(body_offset)]
                    } else {
                        &[]
                    };
                    return handle_upgraded_websocket_session(
                        &mut stream,
                        isolate,
                        parsed.sec_websocket_key,
                        tenant_id,
                        mem_limit,
                        trailing_slice,
                        &mut carry_buf,
                        write_buf,
                        max_requests_tracker,
                    );
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
                if suppress_wire_body {
                    if !has_more_pipelined || write_buf.len() >= 32768 || !keep_alive {
                        stream.write_all(write_buf)?;
                        clear_and_trim_write_buf(write_buf);
                    }
                } else if !has_more_pipelined
                    || body_bytes.len() > STATIC_COALESCE_MAX_BYTES
                    || !keep_alive
                {
                    write_all_vectored2(&mut stream, write_buf, body_bytes)?;
                    clear_and_trim_write_buf(write_buf);
                } else {
                    write_buf.extend_from_slice(body_bytes);
                    if write_buf.len() >= 32768 {
                        stream.write_all(write_buf)?;
                        clear_and_trim_write_buf(write_buf);
                    }
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
                slice_sandbox::escape_json_string_into(&violation_raw, &mut err_body);
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
            let prev = counter.fetch_add(1, Ordering::AcqRel);
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
