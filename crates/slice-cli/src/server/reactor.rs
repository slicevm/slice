//! Share-Nothing Per-Worker Non-Blocking C10K+ HTTP/1.1 Reactor (`NetReactor`) (`#133`).
//!
//! Multiplexes thousands of concurrent Keep-Alive connections on each worker thread using
//! `slice_io::NetReactor` (`epoll` / `kqueue` / `WSAPoll`) with:
//! - Compact idle connection slots (`~128 B` metadata, zero per-connection 64 KB buffers).
//! - Optimistic zero-allocation fast path for complete/pipelined HTTP/1.1 requests.
//! - Non-blocking write backpressure queue (`pending_write`) when kernel send buffers fill.
//! - 10s Slowloris idle/header timeout sweeper.
//! - Live incremental `ReadableStream` / SSE chunked wire flushing via `ServerLiveStreamWriter`.

use super::connection::body_reader::parse_request_headers;
use super::connection::stream_writer::ServerLiveStreamWriter;
use super::connection::websocket::handle_upgraded_websocket_session;
use super::connection::wire::{
    STATIC_COALESCE_MAX_BYTES, clear_and_trim_write_buf, find_http_header_end, http_status_reason,
    is_forbidden_custom_response_header, push_u64_ascii,
};
use super::serve_cmd::{AcceptWorkerShared, wake_accept_workers};
use super::tls::ServerConnStream;
use super::{resolve_tenant_cas_hotswap, sanitize_tenant_id, spawn_worker_isolate};
use slice_engine::{LiveHttpStreamGuard, SliceIsolate};
use slice_io::{Interest, NetReactor, ReadyEvent};
use std::cell::RefCell;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

const LISTENER_TOKEN: usize = usize::MAX;
const IDLE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_REACTOR_HEADER_BYTES: usize = 64 * 1024;
const MAX_REACTOR_BODY_BYTES: usize = 4 * 1024 * 1024;

struct ParkedConn {
    stream: TcpStream,
    live_stream_rc: Option<Rc<RefCell<ServerConnStream>>>,
    partial_read: Vec<u8>,
    pending_write: Vec<u8>,
    write_offset: usize,
    close_after_write: bool,
    last_active: Instant,
    slot_id: u32,
}

impl ParkedConn {
    fn new(stream: TcpStream, slot_id: u32, now: Instant) -> Self {
        Self {
            stream,
            live_stream_rc: None,
            partial_read: Vec::new(),
            pending_write: Vec::new(),
            write_offset: 0,
            close_after_write: false,
            last_active: now,
            slot_id,
        }
    }

    #[inline]
    fn has_pending_write(&self) -> bool {
        self.write_offset < self.pending_write.len()
    }

    /// Attempts to flush `pending_write` non-blockingly. Returns `Ok(true)` if fully drained,
    /// `Ok(false)` if `WouldBlock` (still has unwritten bytes), or `Err` on fatal socket error.
    fn flush_nonblocking(&mut self) -> std::io::Result<bool> {
        while self.write_offset < self.pending_write.len() {
            match self.stream.write(&self.pending_write[self.write_offset..]) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        ErrorKind::WriteZero,
                        "peer closed during write",
                    ));
                }
                Ok(n) => {
                    self.write_offset += n;
                }
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                    return Ok(false);
                }
                Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        self.pending_write.clear();
        if self.pending_write.capacity() > 65536 {
            self.pending_write.shrink_to(4096);
        }
        self.write_offset = 0;
        Ok(true)
    }

    /// Writes `data` optimistically to the non-blocking socket, stashing any unwritten tail in
    /// `pending_write` for later `WRITABLE` readiness flush.
    fn write_or_queue(&mut self, data: &[u8]) -> std::io::Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        if self.has_pending_write() {
            self.pending_write.extend_from_slice(data);
            return Ok(());
        }
        let mut written = 0;
        while written < data.len() {
            match self.stream.write(&data[written..]) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        ErrorKind::WriteZero,
                        "peer closed during write",
                    ));
                }
                Ok(n) => written += n,
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                    self.pending_write.extend_from_slice(&data[written..]);
                    return Ok(());
                }
                Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

/// Runs a single share-nothing non-blocking `NetReactor` worker thread.
pub(crate) fn run_reactor_worker_loop(initial_slot: u32, shared: &AcceptWorkerShared) {
    let Ok(mut reactor) = NetReactor::new() else {
        return;
    };
    let _ = shared.listener.set_nonblocking(true);
    if reactor
        .register_listener(&shared.listener, LISTENER_TOKEN)
        .is_err()
    {
        return;
    }

    SliceIsolate::prewarm_boa_bundle_with_caps(&shared.bytecode, &shared.caps);
    let mut read_buf = vec![0u8; 65536];
    let mut write_buf = Vec::with_capacity(4096);
    let mut local_slot = initial_slot;
    let mut isolate = spawn_worker_isolate(
        local_slot,
        shared.deterministic_seed,
        &shared.caps,
        &shared.audit_secrets,
        shared.fuel_limit,
        shared.mem_limit,
    );

    let mut conns: Vec<Option<ParkedConn>> = Vec::with_capacity(256);
    let mut free_slots: Vec<usize> = Vec::new();
    let mut events: Vec<ReadyEvent> = Vec::with_capacity(256);
    let mut last_sweep = Instant::now();

    while !shared.shutdown_flag.load(Ordering::Acquire) {
        let poll_timeout = Duration::from_millis(25);
        if reactor.poll(&mut events, Some(poll_timeout)).is_err() {
            continue;
        }
        let now = Instant::now();

        for &ev in &events {
            if ev.token == LISTENER_TOKEN {
                // Drain non-blocking accept queue in a batch.
                for _ in 0..64 {
                    match shared.listener.accept() {
                        Ok((stream, _)) => {
                            if shared.shutdown_flag.load(Ordering::Acquire) {
                                break;
                            }
                            let _ = stream.set_nodelay(true);
                            let _ = stream.set_nonblocking(true);
                            local_slot = local_slot.wrapping_add(1);
                            let token = if let Some(slot) = free_slots.pop() {
                                conns[slot] = Some(ParkedConn::new(stream, local_slot, now));
                                slot
                            } else {
                                let slot = conns.len();
                                conns.push(Some(ParkedConn::new(stream, local_slot, now)));
                                slot
                            };
                            if let Some(ref conn) = conns[token]
                                && reactor
                                    .register_stream(&conn.stream, token, Interest::READABLE)
                                    .is_err()
                            {
                                conns[token] = None;
                                free_slots.push(token);
                            }
                        }
                        Err(ref e) if e.kind() == ErrorKind::WouldBlock => break,
                        Err(_) => break,
                    }
                }
                continue;
            }

            let token = ev.token;
            if token >= conns.len() {
                continue;
            }
            let Some(mut conn) = conns[token].take() else {
                continue;
            };
            conn.last_active = now;

            let mut keep_conn = true;

            if ev.error {
                keep_conn = false;
            }

            // 1. Drain any pending backpressure writes first.
            if keep_conn && ev.writable && conn.has_pending_write() {
                match conn.flush_nonblocking() {
                    Ok(true) => {
                        if conn.close_after_write {
                            keep_conn = false;
                        }
                    }
                    Ok(false) => {}
                    Err(_) => keep_conn = false,
                }
            }

            // 2. Process readable data (optimistic fast path for complete/pipelined HTTP/1.1 requests).
            if keep_conn && (ev.readable || ev.hangup) && !conn.close_after_write {
                let outcome = process_readable_conn(
                    &mut conn,
                    &mut isolate,
                    shared,
                    &mut read_buf,
                    &mut write_buf,
                );
                match outcome {
                    ConnStep::Keep => {
                        if conn.close_after_write && !conn.has_pending_write() {
                            keep_conn = false;
                        }
                    }
                    ConnStep::Close => {
                        keep_conn = false;
                    }
                    ConnStep::Panicked => {
                        read_buf.fill(0);
                        write_buf.clear();
                        local_slot = local_slot.wrapping_add(1);
                        isolate = spawn_worker_isolate(
                            local_slot,
                            shared.deterministic_seed,
                            &shared.caps,
                            &shared.audit_secrets,
                            shared.fuel_limit,
                            shared.mem_limit,
                        );
                        keep_conn = false;
                    }
                }
            }

            if keep_conn {
                let interest = if conn.has_pending_write() {
                    Interest::READ_WRITE
                } else {
                    Interest::READABLE
                };
                if reactor
                    .reregister_stream(&conn.stream, token, interest)
                    .is_ok()
                {
                    conns[token] = Some(conn);
                } else {
                    let _ = reactor.deregister_stream(&conn.stream);
                    free_slots.push(token);
                }
            } else {
                let _ = reactor.deregister_stream(&conn.stream);
                free_slots.push(token);
            }

            if let Some(max) = shared.max_requests_opt
                && max > 0
                && shared.req_counter.load(Ordering::Acquire) >= max
            {
                if !shared.shutdown_flag.swap(true, Ordering::AcqRel) {
                    wake_accept_workers(shared.wake_ip, shared.bound_port, shared.workers);
                }
                break;
            }
        }

        // Periodic Slowloris idle timeout sweep (every ~1s).
        if now.duration_since(last_sweep) >= Duration::from_secs(1) {
            last_sweep = now;
            for (slot, entry) in conns.iter_mut().enumerate() {
                if let Some(conn) = entry
                    && now.duration_since(conn.last_active) >= IDLE_TIMEOUT
                {
                    let _ = reactor.deregister_stream(&conn.stream);
                    *entry = None;
                    free_slots.push(slot);
                }
            }
        }
    }
}

enum ConnStep {
    Keep,
    Close,
    Panicked,
}

fn process_readable_conn(
    conn: &mut ParkedConn,
    isolate: &mut SliceIsolate,
    shared: &AcceptWorkerShared,
    read_buf: &mut Vec<u8>,
    write_buf: &mut Vec<u8>,
) -> ConnStep {
    const MAX_TOTAL_READ: usize = MAX_REACTOR_HEADER_BYTES + MAX_REACTOR_BODY_BYTES + 16384;
    let mut filled = 0usize;
    if !conn.partial_read.is_empty() {
        let need = conn.partial_read.len();
        if need > read_buf.len() {
            read_buf.resize(need.min(MAX_TOTAL_READ), 0);
        }
        let copy_len = need.min(read_buf.len());
        read_buf[..copy_len].copy_from_slice(&conn.partial_read[..copy_len]);
        filled = copy_len;
        conn.partial_read.clear();
    }

    // Drain currently available non-blocking bytes from kernel socket buffer,
    // growing `read_buf` on demand for large bodies (> 64 KB, up to 4 MB).
    let mut peer_eof = false;
    loop {
        if filled == read_buf.len() {
            if read_buf.len() >= MAX_TOTAL_READ {
                break;
            }
            let next_len = (read_buf.len() + 65536).min(MAX_TOTAL_READ);
            read_buf.resize(next_len, 0);
        }
        match conn.stream.read(&mut read_buf[filled..]) {
            Ok(0) => {
                peer_eof = true;
                break;
            }
            Ok(n) => {
                filled += n;
            }
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return ConnStep::Close,
        }
    }

    if filled == 0 {
        if read_buf.len() > 65536 {
            read_buf.truncate(65536);
            read_buf.shrink_to(65536);
        }
        return if peer_eof {
            ConnStep::Close
        } else {
            ConnStep::Keep
        };
    }

    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        dispatch_buffered_http_requests(
            conn,
            isolate,
            shared,
            read_buf.as_mut_slice(),
            &mut filled,
            write_buf,
        )
    }));

    let step = match res {
        Ok(step) => {
            if matches!(step, ConnStep::Keep) && filled > 0 {
                if filled > MAX_TOTAL_READ {
                    ConnStep::Close
                } else {
                    conn.partial_read.extend_from_slice(&read_buf[..filled]);
                    if peer_eof && !conn.has_pending_write() {
                        ConnStep::Close
                    } else {
                        step
                    }
                }
            } else if peer_eof && filled == 0 && !conn.has_pending_write() {
                ConnStep::Close
            } else {
                step
            }
        }
        Err(_) => ConnStep::Panicked,
    };
    if read_buf.len() > 65536 {
        read_buf.truncate(65536);
        read_buf.shrink_to(65536);
    }
    step
}

fn dispatch_buffered_http_requests(
    conn: &mut ParkedConn,
    isolate: &mut SliceIsolate,
    shared: &AcceptWorkerShared,
    read_buf: &mut [u8],
    filled: &mut usize,
    write_buf: &mut Vec<u8>,
) -> ConnStep {
    write_buf.clear();

    loop {
        if *filled == 0 {
            break;
        }
        let Some((hdr_end, body_offset)) = find_http_header_end(&read_buf[..*filled]) else {
            if *filled >= MAX_REACTOR_HEADER_BYTES {
                let _ = conn.write_or_queue(
                    b"HTTP/1.1 431 Request Header Fields Too Large\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
                conn.close_after_write = true;
                return ConnStep::Keep;
            }
            // Partial header: keep bytes in partial_read and wait for next EPOLLIN.
            break;
        };
        let req_start = Instant::now();

        // Parse headers using a temporary sink for any 400 error responses.
        let mut err_sink = Vec::new();
        let (hdr_slice, after_hdr_buf) = read_buf.split_at_mut(body_offset);
        let parsed = match parse_request_headers(&mut err_sink, &hdr_slice[..hdr_end], write_buf) {
            Ok(Some(p)) => p,
            Ok(None) | Err(_) => {
                if !err_sink.is_empty() {
                    let _ = conn.write_or_queue(&err_sink);
                }
                conn.close_after_write = true;
                *filled = 0;
                return ConnStep::Keep;
            }
        };

        // Determine request body slice (or wait if incomplete / chunked).
        let (raw_body_bytes, body_end): (Vec<u8>, usize) = if parsed.is_chunked_te {
            let avail = &after_hdr_buf[..filled.saturating_sub(body_offset)];
            match parse_complete_chunked_body(avail) {
                ChunkParse::Complete(decoded, consumed) => (decoded, body_offset + consumed),
                ChunkParse::Incomplete => break,
                ChunkParse::TooLarge => {
                    let _ = conn.write_or_queue(
                        b"HTTP/1.1 413 Payload Too Large\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                    conn.close_after_write = true;
                    *filled = 0;
                    return ConnStep::Keep;
                }
                ChunkParse::Invalid => {
                    let _ = conn.write_or_queue(
                        b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                    conn.close_after_write = true;
                    *filled = 0;
                    return ConnStep::Keep;
                }
            }
        } else {
            if parsed.content_length > MAX_REACTOR_BODY_BYTES {
                let _ = conn.write_or_queue(
                    b"HTTP/1.1 413 Payload Too Large\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
                conn.close_after_write = true;
                *filled = 0;
                return ConnStep::Keep;
            }
            let needed = body_offset + parsed.content_length;
            if *filled < needed {
                break;
            }
            (after_hdr_buf[..parsed.content_length].to_vec(), needed)
        };

        let keep_alive = parsed.keep_alive;
        let method = parsed.method;
        let path = parsed.path;
        let conn_hdr = if keep_alive { "keep-alive" } else { "close" };
        let sanitized_tenant = sanitize_tenant_id(parsed.raw_tenant_id);
        let tenant_id: &str = if sanitized_tenant.is_empty() {
            "tenant-default"
        } else {
            sanitized_tenant.as_ref()
        };
        let has_more_pipelined = keep_alive
            && body_end < *filled
            && find_http_header_end(
                &after_hdr_buf
                    [body_end.saturating_sub(body_offset)..filled.saturating_sub(body_offset)],
            )
            .is_some();

        // Zero-Stat Static Asset Fast-Path
        if !parsed.is_ws_upgrade
            && let Some(static_table) = shared.static_table.as_deref()
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
                    write_buf.extend_from_slice(&asset.payload);
                }
            }
            if !has_more_pipelined || write_buf.len() >= 32768 || !keep_alive {
                if conn.write_or_queue(write_buf).is_err() {
                    return ConnStep::Close;
                }
                clear_and_trim_write_buf(write_buf);
            }
            let reached_max = increment_req_counter(shared);
            if !keep_alive || reached_max {
                if !write_buf.is_empty() && conn.write_or_queue(write_buf).is_err() {
                    return ConnStep::Close;
                }
                clear_and_trim_write_buf(write_buf);
                conn.close_after_write = true;
                *filled = 0;
                return ConnStep::Keep;
            }
            shift_read_buf(read_buf, filled, body_end);
            continue;
        }

        let req_ctx = slice_engine::HttpRequestContext {
            method,
            path,
            query: parsed.query,
            tenant_id,
            body: "",
            body_bytes: Some(raw_body_bytes.as_slice()),
            headers: &parsed.headers_stack[..parsed.headers_len],
        };

        conn.slot_id = conn.slot_id.wrapping_add(1);
        isolate.reset_for_request(tenant_id, conn.slot_id, shared.fuel_limit, shared.mem_limit);

        let dynamic_cas = shared
            .tenant_dir
            .as_deref()
            .and_then(|td| resolve_tenant_cas_hotswap(td, tenant_id));
        let active_bytecode: &[u8] = dynamic_cas
            .as_ref()
            .map(|(bc, _)| bc.as_slice())
            .unwrap_or(&shared.bytecode);
        let active_cas: Option<&slice_cas::MappedCasArchive> = dynamic_cas
            .as_ref()
            .map(|(_, cas)| cas.as_ref())
            .or(shared.cas_archive.as_deref());

        // Install live incremental chunked writer for Boa handlers (cached once per connection so
        // pipelined requests incur zero extra dup/clone syscalls and never disturb O_NONBLOCK).
        let is_boa_bc = active_bytecode.len() >= 8
            && (&active_bytecode[0..8] == b"SLICEBOA"
                || &active_bytecode[0..8] == slice_engine::BOACAS_MAGIC);
        let _live_guard = if is_boa_bc && method != "HEAD" && !parsed.is_ws_upgrade {
            if conn.live_stream_rc.is_none()
                && let Ok(cloned) = conn.stream.try_clone()
            {
                conn.live_stream_rc = Some(Rc::new(RefCell::new(ServerConnStream::Plain(cloned))));
            }
            conn.live_stream_rc.as_ref().map(|rc| {
                LiveHttpStreamGuard::enter(Box::new(ServerLiveStreamWriter::new(
                    Rc::clone(rc),
                    tenant_id,
                    keep_alive,
                    true,
                )))
            })
        } else {
            None
        };

        match isolate.execute_http_request_fast(active_bytecode, active_cas, &req_ctx) {
            Ok(outcome) => {
                drop(_live_guard);
                if parsed.is_ws_upgrade
                    && !parsed.sec_websocket_key.is_empty()
                    && outcome.ws_upgraded
                    && outcome.status_code == 101
                {
                    if !write_buf.is_empty() && conn.write_or_queue(write_buf).is_err() {
                        return ConnStep::Close;
                    }
                    clear_and_trim_write_buf(write_buf);
                    if let Ok(ws_tcp) = conn.stream.try_clone() {
                        let _ = ws_tcp.set_nonblocking(false);
                        let mut ws_stream = ServerConnStream::Plain(ws_tcp);
                        let trailing = if *filled > body_end {
                            &after_hdr_buf[body_end.saturating_sub(body_offset)
                                ..filled.saturating_sub(body_offset)]
                        } else {
                            &[]
                        };
                        let mut carry = Vec::new();
                        let _ = handle_upgraded_websocket_session(
                            &mut ws_stream,
                            isolate,
                            parsed.sec_websocket_key,
                            tenant_id,
                            shared.mem_limit,
                            trailing,
                            &mut carry,
                            write_buf,
                            shared
                                .max_requests_opt
                                .map(|max| (&*shared.req_counter, max)),
                        );
                    }
                    *filled = 0;
                    return ConnStep::Close;
                }

                if !outcome.streamed_on_wire {
                    let isolate_us = outcome.total_time_ns / 1_000;
                    let total_us = (req_start.elapsed().as_nanos() / 1_000) as u64;
                    let safe_ct = if slice_engine::is_valid_http_header_value(&outcome.content_type)
                    {
                        outcome.content_type.as_ref()
                    } else {
                        "application/octet-stream"
                    };
                    let body_bytes = outcome.body_bytes();
                    let is_bodyless = matches!(outcome.status_code, 100..=199 | 204 | 304);
                    let suppress_body = method == "HEAD" || is_bodyless;

                    write_buf.extend_from_slice(b"HTTP/1.1 ");
                    push_u64_ascii(write_buf, outcome.status_code as u64);
                    write_buf.push(b' ');
                    write_buf.extend_from_slice(http_status_reason(outcome.status_code).as_bytes());
                    write_buf.extend_from_slice(b"\r\n");
                    if !is_bodyless {
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
                    write_buf
                        .extend_from_slice(b"\r\nServer: Slice-NexusJS/0.2\r\nX-Slice-Tenant: ");
                    write_buf.extend_from_slice(tenant_id.as_bytes());
                    write_buf.extend_from_slice(b"\r\nX-Slice-Isolate-Us: ");
                    push_u64_ascii(write_buf, isolate_us);
                    write_buf.extend_from_slice(b"\r\nX-Slice-Total-Us: ");
                    push_u64_ascii(write_buf, total_us);
                    write_buf.extend_from_slice(b"\r\nX-Slice-Fuel-Used: ");
                    push_u64_ascii(write_buf, outcome.fuel_consumed);
                    write_buf.extend_from_slice(b"\r\n");

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
                    if !suppress_body {
                        if !has_more_pipelined
                            || body_bytes.len() > STATIC_COALESCE_MAX_BYTES
                            || !keep_alive
                        {
                            if conn.write_or_queue(write_buf).is_err()
                                || conn.write_or_queue(body_bytes).is_err()
                            {
                                return ConnStep::Close;
                            }
                            clear_and_trim_write_buf(write_buf);
                        } else {
                            write_buf.extend_from_slice(body_bytes);
                            if write_buf.len() >= 32768 {
                                if conn.write_or_queue(write_buf).is_err() {
                                    return ConnStep::Close;
                                }
                                clear_and_trim_write_buf(write_buf);
                            }
                        }
                    } else if !has_more_pipelined || write_buf.len() >= 32768 || !keep_alive {
                        if conn.write_or_queue(write_buf).is_err() {
                            return ConnStep::Close;
                        }
                        clear_and_trim_write_buf(write_buf);
                    }
                }

                if outcome.wait_until_count > 0 {
                    let _ = isolate.drain_wait_until_tasks();
                }
            }
            Err(violation) => {
                drop(_live_guard);
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
                if conn.write_or_queue(write_buf).is_err() {
                    return ConnStep::Close;
                }
                clear_and_trim_write_buf(write_buf);
            }
        }

        let reached_max = increment_req_counter(shared);
        if !keep_alive || reached_max {
            if !write_buf.is_empty() && conn.write_or_queue(write_buf).is_err() {
                return ConnStep::Close;
            }
            clear_and_trim_write_buf(write_buf);
            conn.close_after_write = true;
            *filled = 0;
            return ConnStep::Keep;
        }
        shift_read_buf(read_buf, filled, body_end);
    }

    if !write_buf.is_empty() {
        if conn.write_or_queue(write_buf).is_err() {
            return ConnStep::Close;
        }
        clear_and_trim_write_buf(write_buf);
    }
    ConnStep::Keep
}

#[inline]
fn increment_req_counter(shared: &AcceptWorkerShared) -> bool {
    if let Some(max_reqs) = shared.max_requests_opt {
        let prev = shared.req_counter.fetch_add(1, Ordering::AcqRel);
        max_reqs > 0 && prev + 1 >= max_reqs
    } else {
        false
    }
}

#[inline]
fn shift_read_buf(read_buf: &mut [u8], filled: &mut usize, consumed: usize) {
    if *filled > consumed {
        read_buf.copy_within(consumed..*filled, 0);
        *filled -= consumed;
    } else {
        *filled = 0;
    }
}

enum ChunkParse {
    Complete(Vec<u8>, usize),
    Incomplete,
    TooLarge,
    Invalid,
}

fn parse_complete_chunked_body(buf: &[u8]) -> ChunkParse {
    let mut out = Vec::new();
    let mut pos = 0usize;
    loop {
        let Some(rel_crlf) = memchr::memmem::find(&buf[pos..], b"\r\n") else {
            return ChunkParse::Incomplete;
        };
        let size_line = &buf[pos..pos + rel_crlf];
        let hex_part = size_line.split(|&b| b == b';').next().unwrap_or(size_line);
        let Ok(hex_str) = std::str::from_utf8(hex_part) else {
            return ChunkParse::Invalid;
        };
        let Ok(chunk_len) = usize::from_str_radix(hex_str.trim(), 16) else {
            return ChunkParse::Invalid;
        };
        pos += rel_crlf + 2;
        if chunk_len == 0 {
            if pos + 2 > buf.len() {
                return ChunkParse::Incomplete;
            }
            if &buf[pos..pos + 2] == b"\r\n" {
                return ChunkParse::Complete(out, pos + 2);
            }
            let Some(trailer_end) = memchr::memmem::find(&buf[pos..], b"\r\n\r\n") else {
                return ChunkParse::Incomplete;
            };
            return ChunkParse::Complete(out, pos + trailer_end + 4);
        }
        if out.len().saturating_add(chunk_len) > MAX_REACTOR_BODY_BYTES {
            return ChunkParse::TooLarge;
        }
        if pos + chunk_len + 2 > buf.len() {
            return ChunkParse::Incomplete;
        }
        if &buf[pos + chunk_len..pos + chunk_len + 2] != b"\r\n" {
            return ChunkParse::Invalid;
        }
        out.extend_from_slice(&buf[pos..pos + chunk_len]);
        pos += chunk_len + 2;
    }
}
