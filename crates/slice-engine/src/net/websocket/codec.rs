//! RFC 6455 Frame Encoder/Decoder, Word-Aligned XOR Masking, Close Frame Builder, and Stream Reassembler.

use super::{
    WS_GUID, WS_OPCODE_BINARY, WS_OPCODE_CLOSE, WS_OPCODE_CONTINUATION, WS_OPCODE_PING,
    WS_OPCODE_PONG, WS_OPCODE_TEXT,
};
use crate::boa_realm::BOA_DET_STATE;
use crate::intrinsics::{base64_encode, fill_random_bytes, sha1_bytes};
use std::io::{Read, Write};

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

/// Builds and encodes a complete RFC 6455 `Close` (`0x8`) frame from `code`, `reason`, and `mask_key`.
#[inline]
pub fn encode_websocket_close_frame(code: u16, reason: &str, mask_key: Option<[u8; 4]>) -> Vec<u8> {
    let payload = build_close_frame_payload(code, reason);
    encode_websocket_frame(true, WS_OPCODE_CLOSE, mask_key, &payload)
}

pub(super) fn next_ws_mask_key() -> [u8; 4] {
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

pub(super) enum WsFrameStep {
    NeedMoreFrames,
    SendPong(Vec<u8>),
    Close(Vec<u8>),
    Message { opcode: u8, payload: Vec<u8> },
}

/// Shared RFC 6455 control-frame & fragmented continuation reassembly state machine.
pub(super) fn step_reassemble_ws_frame(
    frame: DecodedWsFrame,
    frag_opcode: &mut Option<u8>,
    frag_payload: &mut Vec<u8>,
    max_message_bytes: usize,
) -> Result<WsFrameStep, String> {
    match frame.opcode {
        WS_OPCODE_PING => Ok(WsFrameStep::SendPong(frame.payload)),
        WS_OPCODE_PONG => Ok(WsFrameStep::NeedMoreFrames),
        WS_OPCODE_CLOSE => Ok(WsFrameStep::Close(frame.payload)),
        WS_OPCODE_TEXT | WS_OPCODE_BINARY => {
            if frag_opcode.is_some() {
                return Err(
                    "WebSocketProtocolError: received new data frame while continuation was in progress"
                        .to_string(),
                );
            }
            if frame.fin {
                return Ok(WsFrameStep::Message {
                    opcode: frame.opcode,
                    payload: frame.payload,
                });
            }
            *frag_opcode = Some(frame.opcode);
            *frag_payload = frame.payload;
            Ok(WsFrameStep::NeedMoreFrames)
        }
        WS_OPCODE_CONTINUATION => {
            let Some(orig_op) = *frag_opcode else {
                return Err("WebSocketProtocolError: unexpected continuation frame".to_string());
            };
            let new_len = frag_payload
                .len()
                .checked_add(frame.payload.len())
                .ok_or_else(|| {
                    "MemoryCeilingExceeded: fragmented WebSocket message overflow".to_string()
                })?;
            if new_len > max_message_bytes {
                return Err(format!(
                    "MemoryCeilingExceeded: fragmented WebSocket message ({} bytes) exceeds limit ({} bytes)",
                    new_len, max_message_bytes
                ));
            }
            frag_payload.extend_from_slice(&frame.payload);
            if frame.fin {
                let completed = std::mem::take(frag_payload);
                *frag_opcode = None;
                Ok(WsFrameStep::Message {
                    opcode: orig_op,
                    payload: completed,
                })
            } else {
                Ok(WsFrameStep::NeedMoreFrames)
            }
        }
        other => Err(format!(
            "WebSocketProtocolError: unsupported opcode 0x{:x}",
            other
        )),
    }
}

/// Reads and reassembles the next complete WebSocket message (`TEXT`, `BINARY`, or `CLOSE`) from `stream`,
/// automatically responding to `PING` frames with `PONG` and enforcing `max_message_bytes`.
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
                match step_reassemble_ws_frame(
                    frame,
                    &mut frag_opcode,
                    &mut frag_payload,
                    max_message_bytes,
                )? {
                    WsFrameStep::NeedMoreFrames => {}
                    WsFrameStep::SendPong(ping_payload) => {
                        let mask = if is_server_side {
                            None
                        } else {
                            Some(next_ws_mask_key())
                        };
                        let pong =
                            encode_websocket_frame(true, WS_OPCODE_PONG, mask, &ping_payload);
                        let _ = stream.write_all(&pong);
                        let _ = stream.flush();
                    }
                    WsFrameStep::Close(close_payload) => {
                        return Ok(Some((WS_OPCODE_CLOSE, close_payload)));
                    }
                    WsFrameStep::Message { opcode, payload } => {
                        return Ok(Some((opcode, payload)));
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
