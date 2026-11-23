//! Client `ws://` / `wss://` Handshake, Mock Backend, Buffered Budget Enforcement, and Event Polling.

use super::codec::{
    WsFrameStep, compute_websocket_accept_key, decode_websocket_frame,
    encode_websocket_close_frame, encode_websocket_frame, next_ws_mask_key,
    parse_close_frame_payload, step_reassemble_ws_frame,
};
use super::{
    MAX_WS_BUFFERED_BYTES, WS_OPCODE_BINARY, WS_OPCODE_PONG, WS_OPCODE_TEXT, WireWsEvent,
    adjust_ws_buffered_budget, effective_ws_max_bytes, generate_sec_websocket_key,
    normalize_ws_url_to_http, verify_ws_outbound_dlp, ws_payload_to_event_data,
};
use crate::bindings::helpers::{active_tenant_id_or_default, record_boa_violation_once};
use crate::boa_realm::{BOA_KV_MEM_BUDGET, DIRTY_WEBSOCKET, mark_subsystem_dirty};
use crate::intrinsics::base64_decode;
use crate::outbound_http::{
    MAX_OUTBOUND_HEADER_BYTES, OutboundStream, check_boa_outbound_fetch_ocap_and_dlp,
    dial_fresh_outbound_stream, find_http_header_end_from, is_valid_http_header_value,
    is_valid_request_target, parse_outbound_http_url, resolve_and_verify_ssrf_socket_addrs,
};
use rustc_hash::FxHashMap;
use serde::Serialize;
use slice_sandbox::SandboxViolation;
use std::cell::{Cell, RefCell};
use std::io::{Read, Write};
use std::time::Duration;

enum ClientWsBackend {
    Live {
        stream: OutboundStream,
        pending_buf: Vec<u8>,
    },
    Mock {
        mode: String,
    },
}

struct ActiveClientWebSocket {
    url: String,
    backend: ClientWsBackend,
    queued_events: Vec<WireWsEvent>,
    closed: bool,
    frag_opcode: Option<u8>,
    frag_payload: Vec<u8>,
    buffered_mem_bytes: usize,
}

impl Drop for ActiveClientWebSocket {
    fn drop(&mut self) {
        if self.buffered_mem_bytes > 0 {
            let release = self.buffered_mem_bytes;
            self.buffered_mem_bytes = 0;
            BOA_KV_MEM_BUDGET.with(|m| {
                let (used, limit) = m.get();
                m.set((used.saturating_sub(release), limit));
            });
        }
    }
}

thread_local! {
    static NEXT_CLIENT_WS_ID: Cell<u32> = const { Cell::new(1) };
    static CLIENT_WEBSOCKETS: RefCell<FxHashMap<u32, ActiveClientWebSocket>> =
        RefCell::new(FxHashMap::default());
    pub(crate) static BOA_WS_MOCKS: RefCell<FxHashMap<String, String>> =
        RefCell::new(FxHashMap::default());
}

pub(super) fn clear_client_websockets() {
    CLIENT_WEBSOCKETS.with(|m| m.borrow_mut().clear());
}

fn compute_ws_buffered_bytes(ws: &ActiveClientWebSocket) -> usize {
    let pending = match &ws.backend {
        ClientWsBackend::Live { pending_buf, .. } => pending_buf.len(),
        ClientWsBackend::Mock { .. } => 0,
    };
    let events_bytes: usize = ws
        .queued_events
        .iter()
        .map(|e| {
            e.data
                .len()
                .saturating_add(e.reason.len())
                .saturating_add(32)
        })
        .sum();
    pending
        .saturating_add(ws.frag_payload.len())
        .saturating_add(events_bytes)
}

fn sync_ws_buffered_memory(ws: &mut ActiveClientWebSocket) -> Result<(), String> {
    let next_buffered = compute_ws_buffered_bytes(ws);
    adjust_ws_buffered_budget(&mut ws.buffered_mem_bytes, next_buffered)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WsConnectOutcome {
    id: u32,
    protocol: String,
    live: bool,
    events: Vec<WireWsEvent>,
}

/// Connects a client WebSocket (`ws://` or `wss://`), enforcing OCap `allow_net`, DNS Resolved-IP SSRF,
/// Outbound Egress DLP, and RFC 6455 `101 Switching Protocols` + `Sec-WebSocket-Accept` verification.
pub(crate) fn ws_client_connect(raw_url: &str, protocols: &str) -> Result<String, String> {
    let http_equiv = normalize_ws_url_to_http(raw_url).ok_or_else(|| {
        format!(
            "Invalid WebSocket URL '{}': must start with ws:// or wss://",
            raw_url
        )
    })?;
    let parts = parse_outbound_http_url(&http_equiv)
        .ok_or_else(|| format!("Invalid WebSocket URL authority: {}", raw_url))?;

    if !is_valid_request_target(&parts.path_and_query) || !is_valid_http_header_value(protocols) {
        return Err("InvalidWebSocketHandshake: CRLF or control character blocked".to_string());
    }

    let explicit_host_grant =
        check_boa_outbound_fetch_ocap_and_dlp(&parts, raw_url, protocols, b"")?;

    let matched_mock = BOA_WS_MOCKS.with(|m| {
        let map = m.borrow();
        if let Some(spec) = map.get(raw_url) {
            return Some(spec.clone());
        }
        for (pattern, spec) in map.iter() {
            if pattern == "*" || raw_url.starts_with(pattern) {
                return Some(spec.clone());
            }
        }
        None
    });

    let ws_id = NEXT_CLIENT_WS_ID.with(|c| {
        let id = c.get();
        c.set(id.wrapping_add(1).max(1));
        id
    });
    mark_subsystem_dirty(DIRTY_WEBSOCKET);

    if let Some(mock_mode) = matched_mock {
        let selected_proto = protocols.split(',').next().unwrap_or("").trim().to_string();
        CLIENT_WEBSOCKETS.with(|map| {
            map.borrow_mut().insert(
                ws_id,
                ActiveClientWebSocket {
                    url: raw_url.to_string(),
                    backend: ClientWsBackend::Mock { mode: mock_mode },
                    queued_events: Vec::new(),
                    closed: false,
                    frag_opcode: None,
                    frag_payload: Vec::new(),
                    buffered_mem_bytes: 0,
                },
            );
        });
        return serde_json::to_string(&WsConnectOutcome {
            id: ws_id,
            protocol: selected_proto,
            live: false,
            events: Vec::new(),
        })
        .map_err(|e| e.to_string());
    }

    let verified_addrs = resolve_and_verify_ssrf_socket_addrs(&parts, explicit_host_grant)?;
    let mut stream = dial_fresh_outbound_stream(&parts, &verified_addrs)?;

    let sec_key = generate_sec_websocket_key();
    let expected_accept = compute_websocket_accept_key(&sec_key);
    let mut req_buf = Vec::with_capacity(256);
    let _ = write!(
        &mut req_buf,
        "GET {} HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\n",
        parts.path_and_query, parts.host_header, sec_key
    );
    if !protocols.trim().is_empty() {
        let _ = write!(
            &mut req_buf,
            "Sec-WebSocket-Protocol: {}\r\n",
            protocols.trim()
        );
    }
    req_buf.extend_from_slice(b"\r\n");

    stream
        .write_all(&req_buf)
        .and_then(|()| stream.flush())
        .map_err(|e| format!("WebSocket handshake write failed: {}", e))?;

    let mut raw_resp = Vec::with_capacity(1024);
    let mut scratch = [0u8; 2048];
    let mut scan_cursor = 0usize;
    let (hdr_end, body_start) = loop {
        if let Some(found) = find_http_header_end_from(&raw_resp, scan_cursor)
            .map_err(|msg| format!("WebSocketHandshakeError: {}", msg))?
        {
            break found;
        }
        if raw_resp.len() > MAX_OUTBOUND_HEADER_BYTES {
            return Err("WebSocketHandshakeError: response headers exceeded 64 KB".to_string());
        }
        scan_cursor = raw_resp.len().saturating_sub(3);
        let n = stream
            .read(&mut scratch)
            .map_err(|e| format!("WebSocket handshake read failed: {}", e))?;
        if n == 0 {
            return Err(
                "WebSocketHandshakeError: peer closed connection during handshake".to_string(),
            );
        }
        raw_resp.extend_from_slice(&scratch[..n]);
    };

    let header_str = std::str::from_utf8(&raw_resp[..hdr_end]).unwrap_or("");
    let mut lines = header_str.lines();
    let status_line = lines.next().unwrap_or("");
    let status_code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if status_code != 101 {
        return Err(format!(
            "WebSocketHandshakeError: expected HTTP 101 Switching Protocols, received {}",
            status_line
        ));
    }

    let mut accept_header = String::new();
    let mut negotiated_protocol = String::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim();
            let val = v.trim();
            if key.eq_ignore_ascii_case("sec-websocket-accept") {
                accept_header = val.to_string();
            } else if key.eq_ignore_ascii_case("sec-websocket-protocol") {
                negotiated_protocol = val.to_string();
            }
        }
    }

    if accept_header != expected_accept {
        return Err(format!(
            "WebSocketHandshakeError: invalid Sec-WebSocket-Accept (expected '{}', got '{}')",
            expected_accept, accept_header
        ));
    }

    let leftover = raw_resp[body_start..].to_vec();
    let mut initial_events = Vec::new();
    let mut ws = ActiveClientWebSocket {
        url: raw_url.to_string(),
        backend: ClientWsBackend::Live {
            stream,
            pending_buf: leftover,
        },
        queued_events: Vec::new(),
        closed: false,
        frag_opcode: None,
        frag_payload: Vec::new(),
        buffered_mem_bytes: 0,
    };

    drain_available_live_frames(&mut ws, Duration::ZERO, false);
    initial_events.append(&mut ws.queued_events);
    let _ = sync_ws_buffered_memory(&mut ws);

    CLIENT_WEBSOCKETS.with(|map| {
        map.borrow_mut().insert(ws_id, ws);
    });

    serde_json::to_string(&WsConnectOutcome {
        id: ws_id,
        protocol: negotiated_protocol,
        live: true,
        events: initial_events,
    })
    .map_err(|e| e.to_string())
}

fn drain_available_live_frames(
    ws: &mut ActiveClientWebSocket,
    read_timeout: Duration,
    wait_for_close: bool,
) {
    let max_bytes = effective_ws_max_bytes();
    let ClientWsBackend::Live {
        ref mut stream,
        ref mut pending_buf,
    } = ws.backend
    else {
        return;
    };

    let nonblocking = read_timeout.is_zero();
    if nonblocking {
        stream.set_nonblocking(true);
    } else {
        stream.set_read_timeout(Some(read_timeout));
    }
    let mut read_calls = 0u32;
    let mut follow_up_timeout_set = false;
    let mut scratch = [0u8; 4096];

    loop {
        loop {
            let step_res = decode_websocket_frame(pending_buf, max_bytes).and_then(|opt| {
                let Some(frame) = opt else {
                    return Ok(None);
                };
                pending_buf.drain(..frame.bytes_consumed);
                step_reassemble_ws_frame(
                    frame,
                    &mut ws.frag_opcode,
                    &mut ws.frag_payload,
                    max_bytes,
                )
                .map(Some)
            });

            match step_res {
                Ok(Some(WsFrameStep::NeedMoreFrames)) => {}
                Ok(Some(WsFrameStep::SendPong(ping_payload))) => {
                    let mask = Some(next_ws_mask_key());
                    let pong = encode_websocket_frame(true, WS_OPCODE_PONG, mask, &ping_payload);
                    if nonblocking {
                        stream.set_nonblocking(false);
                    }
                    let _ = stream.write_all(&pong);
                    let _ = stream.flush();
                    if nonblocking {
                        stream.set_nonblocking(true);
                    }
                }
                Ok(Some(WsFrameStep::Close(close_payload))) => {
                    let (code, reason) = parse_close_frame_payload(&close_payload);
                    ws.closed = true;
                    ws.queued_events.push(WireWsEvent {
                        event_type: "close".to_string(),
                        is_binary: false,
                        data: String::new(),
                        code,
                        reason,
                    });
                    if nonblocking {
                        stream.set_nonblocking(false);
                    }
                    return;
                }
                Ok(Some(WsFrameStep::Message { opcode, payload })) => {
                    let is_binary = opcode == WS_OPCODE_BINARY;
                    let data = ws_payload_to_event_data(is_binary, payload);
                    ws.queued_events.push(WireWsEvent {
                        event_type: "message".to_string(),
                        is_binary,
                        data,
                        code: 0,
                        reason: String::new(),
                    });
                }
                Ok(None) => break,
                Err(err) => {
                    if err.starts_with("MemoryCeilingExceeded") {
                        let tenant_id = active_tenant_id_or_default();
                        record_boa_violation_once(SandboxViolation::MemoryCeilingExceeded {
                            tenant_id,
                            requested: max_bytes.saturating_add(1),
                            limit: max_bytes,
                        });
                    }
                    ws.closed = true;
                    ws.queued_events.push(WireWsEvent {
                        event_type: "close".to_string(),
                        is_binary: false,
                        data: String::new(),
                        code: 1002,
                        reason: err,
                    });
                    if nonblocking {
                        stream.set_nonblocking(false);
                    }
                    return;
                }
            }
        }

        if !wait_for_close
            && !ws.queued_events.is_empty()
            && pending_buf.is_empty()
            && ws.frag_opcode.is_none()
        {
            break;
        }

        if !nonblocking
            && read_calls > 0
            && !follow_up_timeout_set
            && read_timeout < Duration::from_millis(5)
        {
            stream.set_read_timeout(Some(Duration::from_millis(5)));
            follow_up_timeout_set = true;
        }
        read_calls = read_calls.saturating_add(1);

        match stream.read(&mut scratch) {
            Ok(0) => {
                ws.closed = true;
                ws.queued_events.push(WireWsEvent {
                    event_type: "close".to_string(),
                    is_binary: false,
                    data: String::new(),
                    code: 1000,
                    reason: String::new(),
                });
                break;
            }
            Ok(n) => {
                let pending_after = pending_buf.len().saturating_add(n);
                if pending_after > MAX_WS_BUFFERED_BYTES
                    || pending_after > max_bytes.saturating_add(65_536)
                {
                    let tenant_id = active_tenant_id_or_default();
                    let lim = max_bytes.min(MAX_WS_BUFFERED_BYTES);
                    record_boa_violation_once(SandboxViolation::MemoryCeilingExceeded {
                        tenant_id,
                        requested: pending_after,
                        limit: lim,
                    });
                    ws.closed = true;
                    ws.queued_events.push(WireWsEvent {
                        event_type: "close".to_string(),
                        is_binary: false,
                        data: String::new(),
                        code: 1009,
                        reason: "MemoryCeilingExceeded: buffered WebSocket stream exceeded limit"
                            .to_string(),
                    });
                    break;
                }
                pending_buf.extend_from_slice(&scratch[..n]);
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                break;
            }
            Err(_) => {
                break;
            }
        }
    }

    if nonblocking {
        stream.set_nonblocking(false);
    }
}

/// Sends a text or binary frame over a connected client WebSocket, enforcing Outbound Egress DLP
/// and RFC 6455 client-to-server 4-byte XOR masking.
pub(crate) fn ws_client_send(
    ws_id: u32,
    is_binary: bool,
    data_str_or_b64: &str,
) -> Result<String, String> {
    let payload_bytes = if is_binary {
        base64_decode(data_str_or_b64)
    } else {
        data_str_or_b64.as_bytes().to_vec()
    };

    let max_bytes = effective_ws_max_bytes();
    if payload_bytes.len() > max_bytes {
        let tenant_id = active_tenant_id_or_default();
        record_boa_violation_once(SandboxViolation::MemoryCeilingExceeded {
            tenant_id,
            requested: payload_bytes.len(),
            limit: max_bytes,
        });
        return Err(
            "MemoryCeilingExceeded: outbound WebSocket frame exceeds memory ceiling".to_string(),
        );
    }

    CLIENT_WEBSOCKETS.with(|map| {
        let mut borrow = map.borrow_mut();
        let ws = borrow
            .get_mut(&ws_id)
            .ok_or_else(|| format!("WebSocket session {} not found", ws_id))?;

        if ws.closed {
            return Err("WebSocket is already closed".to_string());
        }

        verify_ws_outbound_dlp(&ws.url, &payload_bytes)?;

        let url_clone = ws.url.clone();
        match ws.backend {
            ClientWsBackend::Mock { ref mode } => {
                let reply_data = if mode == "echo" || mode.is_empty() {
                    data_str_or_b64.to_string()
                } else if let Some(prefix) = mode.strip_prefix("prefix:") {
                    if is_binary {
                        data_str_or_b64.to_string()
                    } else {
                        format!("{}{}", prefix, data_str_or_b64)
                    }
                } else {
                    mode.clone()
                };
                ws.queued_events.push(WireWsEvent {
                    event_type: "message".to_string(),
                    is_binary,
                    data: reply_data,
                    code: 0,
                    reason: String::new(),
                });
            }
            ClientWsBackend::Live { ref mut stream, .. } => {
                let opcode = if is_binary {
                    WS_OPCODE_BINARY
                } else {
                    WS_OPCODE_TEXT
                };
                let mask = next_ws_mask_key();
                let frame = encode_websocket_frame(true, opcode, Some(mask), &payload_bytes);
                stream
                    .write_all(&frame)
                    .and_then(|()| stream.flush())
                    .map_err(|e| format!("WebSocket write to {} failed: {}", url_clone, e))?;
                drain_available_live_frames(ws, Duration::ZERO, false);
            }
        }

        let events: Vec<WireWsEvent> = std::mem::take(&mut ws.queued_events);
        sync_ws_buffered_memory(ws)?;
        if events.is_empty() {
            return Ok("[]".to_string());
        }
        serde_json::to_string(&events).map_err(|e| e.to_string())
    })
}

/// Polls any pending incoming frames on a client WebSocket.
pub(crate) fn ws_client_poll(ws_id: u32) -> Result<String, String> {
    CLIENT_WEBSOCKETS.with(|map| {
        let mut borrow = map.borrow_mut();
        let Some(ws) = borrow.get_mut(&ws_id) else {
            return Ok("[]".to_string());
        };
        if !ws.closed {
            drain_available_live_frames(ws, Duration::from_millis(2), false);
        }
        let events: Vec<WireWsEvent> = std::mem::take(&mut ws.queued_events);
        sync_ws_buffered_memory(ws)?;
        if events.is_empty() {
            return Ok("[]".to_string());
        }
        serde_json::to_string(&events).map_err(|e| e.to_string())
    })
}

/// Sends an RFC 6455 `Close` (`0x8`) frame and closes the client WebSocket session.
pub(crate) fn ws_client_close(ws_id: u32, code: u16, reason: &str) -> Result<String, String> {
    CLIENT_WEBSOCKETS.with(|map| {
        let mut borrow = map.borrow_mut();
        let Some(ws) = borrow.get_mut(&ws_id) else {
            return Ok("[]".to_string());
        };
        if !ws.closed {
            verify_ws_outbound_dlp(&ws.url, reason.as_bytes())?;
            if let ClientWsBackend::Live { ref mut stream, .. } = ws.backend {
                let frame = encode_websocket_close_frame(code, reason, Some(next_ws_mask_key()));
                let _ = stream.write_all(&frame);
                let _ = stream.flush();
                drain_available_live_frames(ws, Duration::from_millis(50), true);
            }
            ws.closed = true;
            if !ws.queued_events.iter().any(|e| e.event_type == "close") {
                ws.queued_events.push(WireWsEvent {
                    event_type: "close".to_string(),
                    is_binary: false,
                    data: String::new(),
                    code,
                    reason: reason.to_string(),
                });
            }
        }
        let events: Vec<WireWsEvent> = std::mem::take(&mut ws.queued_events);
        let _ = sync_ws_buffered_memory(ws);
        serde_json::to_string(&events).map_err(|e| e.to_string())
    })
}
