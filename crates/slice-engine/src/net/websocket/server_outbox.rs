//! Server-Side Upgraded WebSocket Outbox Queue and Egress DLP Guard.

use super::verify_ws_outbound_dlp;
use crate::boa_realm::{DIRTY_WEBSOCKET, mark_subsystem_dirty};
use crate::intrinsics::base64_decode;
use std::cell::RefCell;

#[derive(Debug, Clone)]
pub enum ServerWsOutFrame {
    Text(String),
    Binary(Vec<u8>),
    Close(u16, String),
}

thread_local! {
    pub(crate) static BOA_WS_SERVER_OUTBOX: RefCell<Vec<ServerWsOutFrame>> =
        const { RefCell::new(Vec::new()) };
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
