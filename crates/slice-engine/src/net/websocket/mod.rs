//! Zero-Dependency RFC 6455 WebSocket Engine (Client `ws://`/`wss://`, Server Upgrade, Frame Codec, & DLP).

mod client;
mod codec;
mod server_outbox;

pub use self::codec::{
    DecodedWsFrame, apply_websocket_mask_in_place, build_close_frame_payload,
    compute_websocket_accept_key, decode_websocket_frame, encode_websocket_close_frame,
    encode_websocket_frame, parse_close_frame_payload, read_websocket_message_from_stream,
};
pub use self::server_outbox::{ServerWsOutFrame, drain_server_ws_outbox};

pub(crate) use self::client::{
    BOA_WS_MOCKS, ws_client_close, ws_client_connect, ws_client_poll, ws_client_send,
};
pub(crate) use self::server_outbox::{
    BOA_WS_SERVER_OUTBOX, ws_server_queue_close, ws_server_queue_send,
};

use crate::bindings::helpers::{active_tenant_id_or_default, record_boa_violation_once};
use crate::boa_realm::{BOA_DET_STATE, BOA_EGRESS_SECRETS, BOA_KV_MEM_BUDGET};
use crate::intrinsics::{base64_encode, fill_random_bytes};
use crate::state_and_kv::verify_egress_secrets_slice;
use serde::Serialize;
use slice_sandbox::SandboxViolation;

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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireWsEvent {
    pub event_type: String,
    pub is_binary: bool,
    pub data: String,
    pub code: u16,
    pub reason: String,
}

/// Clears all client WebSocket connections, mocks, and server outbox frames on the current worker thread.
pub fn clear_thread_websocket_state() {
    client::clear_client_websockets();
    BOA_WS_MOCKS.with(|m| m.borrow_mut().clear());
    BOA_WS_SERVER_OUTBOX.with(|q| q.borrow_mut().clear());
}

pub(super) fn generate_sec_websocket_key() -> String {
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

pub(super) fn effective_ws_max_bytes() -> usize {
    BOA_KV_MEM_BUDGET.with(|m| {
        let (used, limit) = m.get();
        if limit > 0 && limit < usize::MAX {
            limit.saturating_sub(used).min(MAX_WS_FRAME_BYTES)
        } else {
            MAX_WS_FRAME_BYTES
        }
    })
}

pub(super) fn adjust_ws_buffered_budget(
    prev_buffered: &mut usize,
    next_buffered: usize,
) -> Result<(), String> {
    if next_buffered > MAX_WS_BUFFERED_BYTES {
        let tenant_id = active_tenant_id_or_default();
        record_boa_violation_once(SandboxViolation::MemoryCeilingExceeded {
            tenant_id,
            requested: next_buffered,
            limit: MAX_WS_BUFFERED_BYTES,
        });
        return Err(format!(
            "MemoryCeilingExceeded: buffered WebSocket session data ({} bytes) exceeds {} bytes",
            next_buffered, MAX_WS_BUFFERED_BYTES
        ));
    }
    if next_buffered > *prev_buffered {
        let delta = next_buffered - *prev_buffered;
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
            let tenant_id = active_tenant_id_or_default();
            record_boa_violation_once(SandboxViolation::MemoryCeilingExceeded {
                tenant_id,
                requested,
                limit,
            });
            return Err(format!(
                "MemoryCeilingExceeded: buffered WebSocket session data ({} bytes) exceeds tenant limit ({} bytes)",
                requested, limit
            ));
        }
        *prev_buffered = next_buffered;
    } else if next_buffered < *prev_buffered {
        let release = *prev_buffered - next_buffered;
        BOA_KV_MEM_BUDGET.with(|m| {
            let (used, limit) = m.get();
            m.set((used.saturating_sub(release), limit));
        });
        *prev_buffered = next_buffered;
    }
    Ok(())
}

pub(super) fn verify_ws_outbound_dlp(url: &str, payload_bytes: &[u8]) -> Result<(), String> {
    let dlp_err = BOA_EGRESS_SECRETS.with(|sec_cell| {
        let secrets = sec_cell.borrow();
        if secrets.is_empty() {
            return None;
        }
        let tenant_id = active_tenant_id_or_default();
        let payload_str = String::from_utf8_lossy(payload_bytes);
        let payload_b64 = base64_encode(payload_bytes, false);
        verify_egress_secrets_slice(&tenant_id, &secrets, url)
            .and_then(|()| verify_egress_secrets_slice(&tenant_id, &secrets, &payload_str))
            .and_then(|()| verify_egress_secrets_slice(&tenant_id, &secrets, &payload_b64))
            .err()
    });
    if let Some(violation) = dlp_err {
        let msg = violation.to_string();
        record_boa_violation_once(violation);
        return Err(msg);
    }
    Ok(())
}

pub(super) fn normalize_ws_url_to_http(raw_url: &str) -> Option<String> {
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

#[inline]
pub(super) fn ws_payload_to_event_data(is_binary: bool, payload: Vec<u8>) -> String {
    if is_binary {
        base64_encode(&payload, false)
    } else {
        String::from_utf8(payload)
            .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
    }
}
