//! RFC 6455 Server-Side WebSocket `101 Switching Protocols` handshake and frame event loop.

use super::super::tls::ServerConnStream;
use super::wire::flush_pending_write_buf;
use anyhow::Result;
use slice_engine::SliceIsolate;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

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
                slice_engine::encode_websocket_close_frame(code, reason, None)
            }
        };
        stream.write_all(&encoded)?;
    }
    stream.flush()?;
    Ok(should_close)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_upgraded_websocket_session(
    stream: &mut ServerConnStream,
    isolate: &mut SliceIsolate,
    sec_websocket_key: &str,
    tenant_id: &str,
    mem_limit: usize,
    trailing_buf_bytes: &[u8],
    carry_buf: &mut Vec<u8>,
    write_buf: &mut Vec<u8>,
    max_requests_tracker: Option<(&AtomicUsize, usize)>,
) -> Result<()> {
    flush_pending_write_buf(stream, write_buf)?;
    let accept_key = slice_engine::compute_websocket_accept_key(sec_websocket_key);
    write_buf.extend_from_slice(
        b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ",
    );
    write_buf.extend_from_slice(accept_key.as_bytes());
    write_buf.extend_from_slice(b"\r\nServer: Slice-NexusJS/0.2\r\nX-Slice-Tenant: ");
    write_buf.extend_from_slice(tenant_id.as_bytes());
    write_buf.extend_from_slice(b"\r\n\r\n");
    stream.write_all(write_buf)?;
    stream.flush()?;
    write_buf.clear();

    // Clear the 10s HTTP Slowloris header timeout so upgraded WebSockets stay alive while idle.
    stream.set_read_timeout(None);

    match isolate.execute_websocket_event("open", false, &[], 0, "") {
        Ok(open_frames) => {
            if flush_server_ws_frames(stream, open_frames)? {
                if let Some((counter, _)) = max_requests_tracker {
                    counter.fetch_add(1, Ordering::AcqRel);
                }
                return Ok(());
            }
        }
        Err(_violation) => {
            let close_frame =
                slice_engine::encode_websocket_close_frame(1008, "Policy Violation", None);
            let _ = stream.write_all(&close_frame);
            let _ = stream.flush();
            if let Some((counter, _)) = max_requests_tracker {
                counter.fetch_add(1, Ordering::AcqRel);
            }
            return Ok(());
        }
    }

    let mut ws_pending = Vec::new();
    if !trailing_buf_bytes.is_empty() {
        ws_pending.extend_from_slice(trailing_buf_bytes);
    }
    if !carry_buf.is_empty() {
        ws_pending.extend_from_slice(carry_buf);
        carry_buf.clear();
    }
    let effective_ws_limit = mem_limit.min(slice_engine::MAX_WS_FRAME_BYTES);
    loop {
        match slice_engine::read_websocket_message_from_stream(
            stream,
            &mut ws_pending,
            true,
            effective_ws_limit,
        ) {
            Ok(Some((slice_engine::WS_OPCODE_CLOSE, payload))) => {
                let (code, reason) = slice_engine::parse_close_frame_payload(&payload);
                let _ = isolate.execute_websocket_event("close", false, &[], code, &reason);
                let close_frame = slice_engine::encode_websocket_close_frame(code, &reason, None);
                let _ = stream.write_all(&close_frame);
                let _ = stream.flush();
                break;
            }
            Ok(Some((opcode, payload))) => {
                let is_bin = opcode == slice_engine::WS_OPCODE_BINARY;
                match isolate.execute_websocket_event("message", is_bin, &payload, 0, "") {
                    Ok(out_frames) => {
                        if flush_server_ws_frames(stream, out_frames)? {
                            break;
                        }
                    }
                    Err(_violation) => {
                        let close_frame = slice_engine::encode_websocket_close_frame(
                            1008,
                            "Policy Violation",
                            None,
                        );
                        let _ = stream.write_all(&close_frame);
                        let _ = stream.flush();
                        break;
                    }
                }
            }
            Ok(None) | Err(_) => {
                let _ = isolate.execute_websocket_event("close", false, &[], 1006, "");
                break;
            }
        }
    }
    if let Some((counter, _)) = max_requests_tracker {
        counter.fetch_add(1, Ordering::AcqRel);
    }
    Ok(())
}
