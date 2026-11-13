//! Zero-Dependency RFC 6455 WebSocket Engine (Client `ws://`/`wss://`, Server Upgrade, Frame Codec, & DLP).
//!
//! Implements:
//! - RFC 6455 Section 4.2.2 `Sec-WebSocket-Accept` SHA-1 + Base64 handshake (`compute_websocket_accept_key`)
//! - RFC 6455 Section 5.2 wire frame encoder/decoder (`encode_websocket_frame`, `decode_websocket_frame`)
//!   with 4-byte `u32` word-aligned XOR masking/unmasking (`apply_websocket_mask_in_place`)
//! - Fragmented frame reassembly (`FIN=0` + `0x0` continuation frames), automatic `Ping` (`0x9`) -> `Pong` (`0xA`)
//!   heartbeat responses, and `Close` (`0x8`) status code/reason parsing
//! - Hard `MAX_WS_FRAME_BYTES` (4 MB) + active tenant `BOA_KV_MEM_BUDGET` ceiling enforcement
//! - Outbound Egress Secret DLP inspection on every `ws.send()` frame (raw, Base64, and Base64URL)
//! - Deterministic `Slice.mockWebSocket` support for offline replay and unit testing

use crate::boa_realm::{
    mark_subsystem_dirty, BOA_ACTIVE_TENANT, BOA_DET_STATE, BOA_EGRESS_SECRETS, BOA_KV_MEM_BUDGET,
    BOA_VIOLATION, DIRTY_WEBSOCKET,
};
use crate::intrinsics::{base64_decode, base64_encode, fill_random_bytes, sha1_bytes};
use crate::outbound_http::{
    check_boa_outbound_fetch_ocap_and_dlp, dial_fresh_outbound_stream, is_valid_http_header_value,
    is_valid_request_target, parse_outbound_http_url, resolve_and_verify_ssrf_socket_addrs,
    OutboundStream, MAX_OUTBOUND_HEADER_BYTES,
};
use crate::state_and_kv::verify_egress_secrets_slice;
use rustc_hash::FxHashMap;
use serde::Serialize;
use slice_sandbox::SandboxViolation;
use std::cell::{Cell, RefCell};
use std::io::{Read, Write};
use std::time::Duration;

/// RFC 6455 Section 1.3 handshake GUID.
pub const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Maximum size of a single WebSocket frame or reassembled message (`4 MB`).
pub const MAX_WS_FRAME_BYTES: usize = 4 * 1024 * 1024;

/// Maximum total buffered unread WebSocket frames/events per client session (`16 MB`).
pub const MAX_WS_BUFFERED_BYTES: usize = 16 * 1024 * 1024;

pub const WS_OPCODE_CONTINUATION: u8 = 0x0;
pub const WS_OPCODE_TEXT: u8 = 0x1;
pub const WS_OPCODE_BINARY: u8 = 0x2;
pub const WS_OPCODE_CLOSE: u8 = 0x8;
pub const WS_OPCODE_PING: u8 = 0x9;
pub const WS_OPCODE_PONG: u8 = 0xA;

/// Computes the RFC 6455 `Sec-WebSocket-Accept` header value for a given client `Sec-WebSocket-Key`:
/// `Base64(SHA-1(key.trim() + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"))`.
#[inline]
pub fn compute_websocket_accept_key(sec_websocket_key: &str) -> String {
    let trimmed = sec_websocket_key.trim();
    let mut input = Vec::with_capacity(trimmed.len() + WS_GUID.len());
    input.extend_from_slice(trimmed.as_bytes());
    input.extend_from_slice(WS_GUID.as_bytes());
    let digest = sha1_bytes(&input);
    base64_encode(&digest, false)
}

/// Applies RFC 6455 Section 5.3 XOR masking/unmasking in-place using 4-byte `u32` chunks.
#[inline]
pub fn apply_websocket_mask_in_place(payload: &mut [u8], mask_key: [u8; 4]) {
    if mask_key == [0, 0, 0, 0] {
        return;
    }
    let key_u32 = u32::from_ne_bytes(mask_key);
    let len = payload.len();
    let mut idx = 0;
    while idx + 4 <= len {
        let word = u32::from_ne_bytes([
            payload[idx],
            payload[idx + 1],
            payload[idx + 2],
            payload[idx + 3],
        ]) ^ key_u32;
        payload[idx..idx + 4].copy_from_slice(&word.to_ne_bytes());
        idx += 4;
    }
    for (rem_idx, byte) in payload[idx..].iter_mut().enumerate() {
        *byte ^= mask_key[rem_idx & 3];
    }
}

/// Encodes an RFC 6455 WebSocket frame.
/// - Client-to-server frames MUST provide `Some(mask_key)`.
/// - Server-to-client frames MUST pass `None` for `mask_key`.
pub fn encode_websocket_frame(
    fin: bool,
    opcode: u8,
    mask_key: Option<[u8; 4]>,
    payload: &[u8],
) -> Vec<u8> {
    let len = payload.len();
    let header_len =
        2 + if len <= 125 {
            0
        } else if len <= 65535 {
            2
        } else {
            8
        } + if mask_key.is_some() { 4 } else { 0 };

    let mut frame = Vec::with_capacity(header_len + len);
    let b0 = (if fin { 0x80 } else { 0x00 }) | (opcode & 0x0F);
    let mask_bit = if mask_key.is_some() { 0x80 } else { 0x00 };

    frame.push(b0);
    if len <= 125 {
        frame.push(mask_bit | (len as u8));
    } else if len <= 65535 {
        frame.push(mask_bit | 126);
        frame.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        frame.push(mask_bit | 127);
        frame.extend_from_slice(&(len as u64).to_be_bytes());
    }

    if let Some(mask) = mask_key {
        frame.extend_from_slice(&mask);
        let payload_start = frame.len();
        frame.extend_from_slice(payload);
        apply_websocket_mask_in_place(&mut frame[payload_start..], mask);
    } else {
        frame.extend_from_slice(payload);
    }

    frame
}

/// Decoded RFC 6455 WebSocket frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedWsFrame {
    pub fin: bool,
    pub opcode: u8,
    pub masked: bool,
    pub payload: Vec<u8>,
    pub bytes_consumed: usize,
}

/// Attempts to decode a single RFC 6455 frame from `buf`, enforcing `max_payload_bytes` before allocation.
/// Returns:
/// - `Ok(None)` if `buf` does not yet contain a complete frame.
/// - `Ok(Some(DecodedWsFrame))` when a complete frame is decoded and unmasked.
/// - `Err(String)` if the frame header violates RFC 6455 or exceeds `max_payload_bytes`.
pub fn decode_websocket_frame(
    buf: &[u8],
    max_payload_bytes: usize,
) -> Result<Option<DecodedWsFrame>, String> {
    if buf.len() < 2 {
        return Ok(None);
    }
    let b0 = buf[0];
    let b1 = buf[1];
    let fin = (b0 & 0x80) != 0;
    let rsv = b0 & 0x70;
    if rsv != 0 {
        return Err(format!(
            "WebSocketProtocolError: non-zero RSV bits (0x{:02x}) without negotiated extension",
            rsv
        ));
    }
    let opcode = b0 & 0x0F;
    let masked = (b1 & 0x80) != 0;
    let len_code = (b1 & 0x7F) as usize;

    // RFC 6455 Section 5.5: Control frames (opcode >= 0x8) MUST NOT be fragmented and MUST have payload <= 125 bytes
    if opcode >= 0x8 {
        if !fin {
            return Err(
                "WebSocketProtocolError: fragmented control frame is forbidden".to_string(),
            );
        }
        if len_code > 125 {
            return Err(
                "WebSocketProtocolError: control frame payload exceeds 125 bytes".to_string(),
            );
        }
    }

    let mut pos = 2usize;
    let payload_len = if len_code <= 125 {
        len_code
    } else if len_code == 126 {
        if buf.len() < pos + 2 {
            return Ok(None);
        }
        let l = u16::from_be_bytes([buf[pos], buf[pos + 1]]) as usize;
        pos += 2;
        l
    } else {
        if buf.len() < pos + 8 {
            return Ok(None);
        }
        let mut arr = [0u8; 8];
        arr.copy_from_slice(&buf[pos..pos + 8]);
        let l_u64 = u64::from_be_bytes(arr);
        if (l_u64 >> 63) != 0 {
            return Err(
                "WebSocketProtocolError: most significant bit of 64-bit length must be 0"
                    .to_string(),
            );
        }
        let l = usize::try_from(l_u64)
            .map_err(|_| "WebSocketFrameTooLarge: 64-bit length overflow".to_string())?;
        pos += 8;
        l
    };

    if payload_len > max_payload_bytes {
        return Err(format!(
            "MemoryCeilingExceeded: WebSocket frame payload ({} bytes) exceeds limit ({} bytes)",
            payload_len, max_payload_bytes
        ));
    }

    let mask_key = if masked {
        if buf.len() < pos + 4 {
            return Ok(None);
        }
        let k = [buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]];
        pos += 4;
        Some(k)
    } else {
        None
    };

    let frame_end = pos
        .checked_add(payload_len)
        .ok_or_else(|| "WebSocketFrameTooLarge: frame offset overflow".to_string())?;
    if buf.len() < frame_end {
        return Ok(None);
    }

    let mut payload = buf[pos..frame_end].to_vec();
    if let Some(mask) = mask_key {
        apply_websocket_mask_in_place(&mut payload, mask);
    }

    Ok(Some(DecodedWsFrame {
        fin,
        opcode,
        masked,
        payload,
        bytes_consumed: frame_end,
    }))
}

/// Parses an RFC 6455 `Close` (`0x8`) frame payload into `(status_code, reason_utf8)`.
pub fn parse_close_frame_payload(payload: &[u8]) -> (u16, String) {
    if payload.len() >= 2 {
        let code = u16::from_be_bytes([payload[0], payload[1]]);
        let reason = String::from_utf8_lossy(&payload[2..]).into_owned();
        (code, reason)
    } else {
        (1000, String::new())
    }
}

/// Encodes an RFC 6455 `Close` (`0x8`) frame payload from `code` and `reason`.
pub fn build_close_frame_payload(code: u16, reason: &str) -> Vec<u8> {
    let reason_bytes = reason.as_bytes();
    let capped_reason = &reason_bytes[..reason_bytes.len().min(123)];
    let mut payload = Vec::with_capacity(2 + capped_reason.len());
    payload.extend_from_slice(&code.to_be_bytes());
    payload.extend_from_slice(capped_reason);
    payload
}

/// Reads and reassembles the next complete WebSocket message (`TEXT`, `BINARY`, or `CLOSE`) from `stream`,
/// automatically responding to `PING` frames with `PONG` and enforcing `max_message_bytes`.
/// - When `is_server_side` is `true`, incoming client frames MUST be masked and outgoing `PONG` frames are unmasked.
/// - When `is_server_side` is `false` (client mode), outgoing `PONG` frames are masked.
pub fn read_websocket_message_from_stream<S: Read + Write>(
    stream: &mut S,
    pending_buf: &mut Vec<u8>,
    is_server_side: bool,
    max_message_bytes: usize,
) -> Result<Option<(u8, Vec<u8>)>, String> {
    let mut frag_opcode: Option<u8> = None;
    let mut frag_payload = Vec::new();
    let mut scratch = [0u8; 4096];

    loop {
        match decode_websocket_frame(pending_buf, max_message_bytes)? {
            Some(frame) => {
                pending_buf.drain(..frame.bytes_consumed);
                if is_server_side && !frame.masked {
                    return Err(
                        "WebSocketProtocolError: client-to-server frames MUST be masked"
                            .to_string(),
                    );
                }
                match frame.opcode {
                    WS_OPCODE_PING => {
                        let mask = if is_server_side {
                            None
                        } else {
                            Some(next_ws_mask_key())
                        };
                        let pong =
                            encode_websocket_frame(true, WS_OPCODE_PONG, mask, &frame.payload);
                        let _ = stream.write_all(&pong);
                        let _ = stream.flush();
                    }
                    WS_OPCODE_PONG => {
                        // Heartbeat acknowledged
                    }
                    WS_OPCODE_CLOSE => {
                        return Ok(Some((WS_OPCODE_CLOSE, frame.payload)));
                    }
                    WS_OPCODE_TEXT | WS_OPCODE_BINARY => {
                        if frag_opcode.is_some() {
                            return Err(
                                "WebSocketProtocolError: received new data frame while continuation was in progress"
                                    .to_string(),
                            );
                        }
                        if frame.fin {
                            return Ok(Some((frame.opcode, frame.payload)));
                        }
                        frag_opcode = Some(frame.opcode);
                        frag_payload = frame.payload;
                    }
                    WS_OPCODE_CONTINUATION => {
                        let Some(orig_op) = frag_opcode else {
                            return Err(
                                "WebSocketProtocolError: unexpected continuation frame".to_string()
                            );
                        };
                        let new_len = frag_payload
                            .len()
                            .checked_add(frame.payload.len())
                            .ok_or_else(|| {
                                "MemoryCeilingExceeded: fragmented WebSocket message overflow"
                                    .to_string()
                            })?;
                        if new_len > max_message_bytes {
                            return Err(format!(
                                "MemoryCeilingExceeded: fragmented WebSocket message ({} bytes) exceeds limit ({} bytes)",
                                new_len, max_message_bytes
                            ));
                        }
                        frag_payload.extend_from_slice(&frame.payload);
                        if frame.fin {
                            return Ok(Some((orig_op, frag_payload)));
                        }
                    }
                    other => {
                        return Err(format!(
                            "WebSocketProtocolError: unsupported opcode 0x{:x}",
                            other
                        ));
                    }
                }
            }
            None => {
                let n = stream
                    .read(&mut scratch)
                    .map_err(|e| format!("WebSocketReadError: {}", e))?;
                if n == 0 {
                    if pending_buf.is_empty() && frag_opcode.is_none() {
                        return Ok(None);
                    }
                    return Err("WebSocketReadError: unexpected EOF mid-frame".to_string());
                }
                if pending_buf.len().saturating_add(n) > max_message_bytes.saturating_add(16) {
                    return Err(format!(
                        "MemoryCeilingExceeded: buffered WebSocket frame exceeds {} bytes",
                        max_message_bytes
                    ));
                }
                pending_buf.extend_from_slice(&scratch[..n]);
            }
        }
    }
}

fn next_ws_mask_key() -> [u8; 4] {
    let mut mask = [0u8; 4];
    BOA_DET_STATE.with(|s| {
        if let Some((mut prng, clk)) = s.get() {
            fill_random_bytes(&mut mask, Some(&mut prng));
            s.set(Some((prng, clk)));
        } else {
            fill_random_bytes(&mut mask, None);
        }
    });
    mask
}

fn generate_sec_websocket_key() -> String {
    let mut raw = [0u8; 16];
    BOA_DET_STATE.with(|s| {
        if let Some((mut prng, clk)) = s.get() {
            fill_random_bytes(&mut raw, Some(&mut prng));
            s.set(Some((prng, clk)));
        } else {
            fill_random_bytes(&mut raw, None);
        }
    });
    base64_encode(&raw, false)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireWsEvent {
    pub event_type: String,
    pub is_binary: bool,
    pub data: String,
    pub code: u16,
    pub reason: String,
}

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

#[derive(Debug, Clone)]
pub enum ServerWsOutFrame {
    Text(String),
    Binary(Vec<u8>),
    Close(u16, String),
}

thread_local! {
    static NEXT_CLIENT_WS_ID: Cell<u32> = const { Cell::new(1) };
    static CLIENT_WEBSOCKETS: RefCell<FxHashMap<u32, ActiveClientWebSocket>> =
        RefCell::new(FxHashMap::default());
    pub(crate) static BOA_WS_MOCKS: RefCell<FxHashMap<String, String>> =
        RefCell::new(FxHashMap::default());
    pub(crate) static BOA_WS_SERVER_OUTBOX: RefCell<Vec<ServerWsOutFrame>> =
        const { RefCell::new(Vec::new()) };
}

/// Clears all client WebSocket connections and mocks on the current worker thread.
pub fn clear_thread_websocket_state() {
    CLIENT_WEBSOCKETS.with(|m| m.borrow_mut().clear());
    BOA_WS_MOCKS.with(|m| m.borrow_mut().clear());
    BOA_WS_SERVER_OUTBOX.with(|q| q.borrow_mut().clear());
}

fn current_tenant_id() -> String {
    BOA_ACTIVE_TENANT.with(|t| {
        let b = t.borrow();
        if b.is_empty() {
            "boa-tenant".to_string()
        } else {
            b.clone()
        }
    })
}

fn effective_ws_max_bytes() -> usize {
    BOA_KV_MEM_BUDGET.with(|m| {
        let (used, limit) = m.get();
        if limit > 0 && limit < usize::MAX {
            limit.saturating_sub(used).min(MAX_WS_FRAME_BYTES)
        } else {
            MAX_WS_FRAME_BYTES
        }
    })
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
    if next_buffered > MAX_WS_BUFFERED_BYTES {
        let tenant_id = current_tenant_id();
        BOA_VIOLATION.with(|v| {
            if v.borrow().is_none() {
                *v.borrow_mut() = Some(SandboxViolation::MemoryCeilingExceeded {
                    tenant_id,
                    requested: next_buffered,
                    limit: MAX_WS_BUFFERED_BYTES,
                });
            }
        });
        return Err(format!(
            "MemoryCeilingExceeded: buffered WebSocket session data ({} bytes) exceeds {} bytes",
            next_buffered, MAX_WS_BUFFERED_BYTES
        ));
    }
    let prev_buffered = ws.buffered_mem_bytes;
    if next_buffered > prev_buffered {
        let delta = next_buffered - prev_buffered;
        let exceeded = BOA_KV_MEM_BUDGET.with(|m| {
            let (used, limit) = m.get();
            if limit > 0 && limit < usize::MAX && used.saturating_add(delta) > limit {
                Some((used.saturating_add(delta), limit))
            } else {
                m.set((used.saturating_add(delta), limit));
                None
            }
        });
        if let Some((requested, limit)) = exceeded {
            let tenant_id = current_tenant_id();
            BOA_VIOLATION.with(|v| {
                if v.borrow().is_none() {
                    *v.borrow_mut() = Some(SandboxViolation::MemoryCeilingExceeded {
                        tenant_id,
                        requested,
                        limit,
                    });
                }
            });
            return Err(format!(
                "MemoryCeilingExceeded: buffered WebSocket session data ({} bytes) exceeds tenant limit ({} bytes)",
                requested, limit
            ));
        }
        ws.buffered_mem_bytes = next_buffered;
    } else if next_buffered < prev_buffered {
        let release = prev_buffered - next_buffered;
        BOA_KV_MEM_BUDGET.with(|m| {
            let (used, limit) = m.get();
            m.set((used.saturating_sub(release), limit));
        });
        ws.buffered_mem_bytes = next_buffered;
    }
    Ok(())
}

fn verify_ws_outbound_dlp(url: &str, payload_bytes: &[u8]) -> Result<(), String> {
    let dlp_err = BOA_EGRESS_SECRETS.with(|sec_cell| {
        let secrets = sec_cell.borrow();
        if secrets.is_empty() {
            return None;
        }
        let tenant_id = current_tenant_id();
        let payload_str = String::from_utf8_lossy(payload_bytes);
        let payload_b64 = base64_encode(payload_bytes, false);
        verify_egress_secrets_slice(&tenant_id, &secrets, url)
            .and_then(|()| verify_egress_secrets_slice(&tenant_id, &secrets, &payload_str))
            .and_then(|()| verify_egress_secrets_slice(&tenant_id, &secrets, &payload_b64))
            .err()
    });
    if let Some(violation) = dlp_err {
        let msg = violation.to_string();
        BOA_VIOLATION.with(|v| *v.borrow_mut() = Some(violation));
        return Err(msg);
    }
    Ok(())
}

/// Converts a `ws://` or `wss://` URL into an HTTP/HTTPS equivalent for `parse_outbound_http_url`.
fn normalize_ws_url_to_http(raw_url: &str) -> Option<String> {
    let trimmed = raw_url.trim();
    if let Some(rest) = trimmed.strip_prefix("ws://") {
        Some(format!("http://{}", rest))
    } else if let Some(rest) = trimmed.strip_prefix("wss://") {
        Some(format!("https://{}", rest))
    } else if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Some(trimmed.to_string())
    } else {
        None
    }
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

    // 1. Always enforce OCap allow_net, Cloud Metadata SSRF, and URL Egress DLP first (even for mocks)
    let explicit_host_grant =
        check_boa_outbound_fetch_ocap_and_dlp(&parts, raw_url, protocols, b"")?;

    // 2. Check if a deterministic `Slice.mockWebSocket` matches `raw_url`
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

    // 3. Resolve DNS and verify every resolved IP against the Pre-Connect SSRF Firewall
    let verified_addrs = resolve_and_verify_ssrf_socket_addrs(&parts, explicit_host_grant)?;
    let mut stream = dial_fresh_outbound_stream(&parts, &verified_addrs)?;

    // 4. Send RFC 6455 HTTP/1.1 Upgrade request
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

    // 5. Read Upgrade response headers (`\r\n\r\n`), capped at 64 KB
    let mut raw_resp = Vec::with_capacity(1024);
    let mut scratch = [0u8; 2048];
    let (hdr_end, body_start) = loop {
        if let Some(pos) = raw_resp.windows(4).position(|w| w == b"\r\n\r\n") {
            break (pos, pos + 4);
        }
        if raw_resp.len() > MAX_OUTBOUND_HEADER_BYTES {
            return Err("WebSocketHandshakeError: response headers exceeded 64 KB".to_string());
        }
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

    // Drain any frames that arrived coalesced with the 101 response
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
        // First decode any complete frames already in `pending_buf`
        loop {
            match decode_websocket_frame(pending_buf, max_bytes) {
                Ok(Some(frame)) => {
                    pending_buf.drain(..frame.bytes_consumed);
                    match frame.opcode {
                        WS_OPCODE_PING => {
                            let mask = Some(next_ws_mask_key());
                            let pong =
                                encode_websocket_frame(true, WS_OPCODE_PONG, mask, &frame.payload);
                            if nonblocking {
                                stream.set_nonblocking(false);
                            }
                            let _ = stream.write_all(&pong);
                            let _ = stream.flush();
                            if nonblocking {
                                stream.set_nonblocking(true);
                            }
                        }
                        WS_OPCODE_PONG => {}
                        WS_OPCODE_CLOSE => {
                            let (code, reason) = parse_close_frame_payload(&frame.payload);
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
                        WS_OPCODE_TEXT | WS_OPCODE_BINARY => {
                            if frame.fin && ws.frag_opcode.is_none() {
                                let is_binary = frame.opcode == WS_OPCODE_BINARY;
                                let data = if is_binary {
                                    base64_encode(&frame.payload, false)
                                } else {
                                    String::from_utf8_lossy(&frame.payload).into_owned()
                                };
                                ws.queued_events.push(WireWsEvent {
                                    event_type: "message".to_string(),
                                    is_binary,
                                    data,
                                    code: 0,
                                    reason: String::new(),
                                });
                            } else {
                                ws.frag_opcode = Some(frame.opcode);
                                ws.frag_payload = frame.payload;
                            }
                        }
                        WS_OPCODE_CONTINUATION => {
                            let Some(orig_opcode) = ws.frag_opcode else {
                                ws.closed = true;
                                ws.queued_events.push(WireWsEvent {
                                    event_type: "close".to_string(),
                                    is_binary: false,
                                    data: String::new(),
                                    code: 1002,
                                    reason: "InvalidWebSocketFrame: unexpected continuation frame"
                                        .to_string(),
                                });
                                if nonblocking {
                                    stream.set_nonblocking(false);
                                }
                                return;
                            };
                            if ws.frag_payload.len().saturating_add(frame.payload.len()) > max_bytes
                            {
                                let tenant_id = current_tenant_id();
                                BOA_VIOLATION.with(|v| {
                                    *v.borrow_mut() =
                                        Some(SandboxViolation::MemoryCeilingExceeded {
                                            tenant_id,
                                            requested: ws
                                                .frag_payload
                                                .len()
                                                .saturating_add(frame.payload.len()),
                                            limit: max_bytes,
                                        });
                                });
                                ws.closed = true;
                                if nonblocking {
                                    stream.set_nonblocking(false);
                                }
                                return;
                            }
                            ws.frag_payload.extend_from_slice(&frame.payload);
                            if frame.fin {
                                let completed = std::mem::take(&mut ws.frag_payload);
                                ws.frag_opcode = None;
                                let is_binary = orig_opcode == WS_OPCODE_BINARY;
                                let data = if is_binary {
                                    base64_encode(&completed, false)
                                } else {
                                    String::from_utf8_lossy(&completed).into_owned()
                                };
                                ws.queued_events.push(WireWsEvent {
                                    event_type: "message".to_string(),
                                    is_binary,
                                    data,
                                    code: 0,
                                    reason: String::new(),
                                });
                            }
                        }
                        _ => {}
                    }
                }
                Ok(None) => break,
                Err(err) => {
                    if err.starts_with("MemoryCeilingExceeded") {
                        let tenant_id = current_tenant_id();
                        BOA_VIOLATION.with(|v| {
                            *v.borrow_mut() = Some(SandboxViolation::MemoryCeilingExceeded {
                                tenant_id,
                                requested: max_bytes.saturating_add(1),
                                limit: max_bytes,
                            });
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
                    let tenant_id = current_tenant_id();
                    let lim = max_bytes.min(MAX_WS_BUFFERED_BYTES);
                    BOA_VIOLATION.with(|v| {
                        if v.borrow().is_none() {
                            *v.borrow_mut() = Some(SandboxViolation::MemoryCeilingExceeded {
                                tenant_id,
                                requested: pending_after,
                                limit: lim,
                            });
                        }
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
/// and RFC 6455 client-to-server 4-byte XOR masking, then returns any immediately received reply events.
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
        let tenant_id = current_tenant_id();
        BOA_VIOLATION.with(|v| {
            *v.borrow_mut() = Some(SandboxViolation::MemoryCeilingExceeded {
                tenant_id,
                requested: payload_bytes.len(),
                limit: max_bytes,
            });
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

        // Enforce Outbound Egress Secret DLP on every frame before transmitting
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
                let close_payload = build_close_frame_payload(code, reason);
                let mask = next_ws_mask_key();
                let frame =
                    encode_websocket_frame(true, WS_OPCODE_CLOSE, Some(mask), &close_payload);
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

/// Queues an outbound server-to-client WebSocket frame after enforcing Egress DLP.
pub(crate) fn ws_server_queue_send(is_binary: bool, data_str_or_b64: &str) -> Result<(), String> {
    let bytes = if is_binary {
        base64_decode(data_str_or_b64)
    } else {
        data_str_or_b64.as_bytes().to_vec()
    };
    verify_ws_outbound_dlp("ws://server-upgrade", &bytes)?;
    mark_subsystem_dirty(DIRTY_WEBSOCKET);
    BOA_WS_SERVER_OUTBOX.with(|q| {
        if is_binary {
            q.borrow_mut().push(ServerWsOutFrame::Binary(bytes));
        } else {
            q.borrow_mut()
                .push(ServerWsOutFrame::Text(data_str_or_b64.to_string()));
        }
    });
    Ok(())
}

/// Queues an outbound server-to-client `Close` frame after enforcing Egress DLP on `reason`.
pub(crate) fn ws_server_queue_close(code: u16, reason: &str) -> Result<(), String> {
    verify_ws_outbound_dlp("ws://server-upgrade", reason.as_bytes())?;
    mark_subsystem_dirty(DIRTY_WEBSOCKET);
    BOA_WS_SERVER_OUTBOX.with(|q| {
        q.borrow_mut()
            .push(ServerWsOutFrame::Close(code, reason.to_string()));
    });
    Ok(())
}

/// Drains all server-to-client WebSocket frames queued during the most recent isolate callback.
pub fn drain_server_ws_outbox() -> Vec<ServerWsOutFrame> {
    BOA_WS_SERVER_OUTBOX.with(|q| std::mem::take(&mut *q.borrow_mut()))
}
