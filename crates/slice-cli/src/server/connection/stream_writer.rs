//! Live Incremental HTTP/1.1 Chunked Wire Writer (`Transfer-Encoding: chunked`) for `ReadableStream`
//! and Server-Sent Events (`text/event-stream`) (`#133`).

use super::super::tls::ServerConnStream;
use super::wire::{http_status_reason, is_forbidden_custom_response_header, push_u64_ascii};
use slice_engine::LiveHttpStreamWriter;
use std::cell::RefCell;
use std::io::{ErrorKind, Write};
use std::rc::Rc;
use std::time::{Duration, Instant};

const STREAM_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

fn write_all_nonblocking_aware<W: Write>(writer: &mut W, mut buf: &[u8]) -> std::io::Result<()> {
    let deadline = Instant::now() + STREAM_WRITE_TIMEOUT;
    while !buf.is_empty() {
        match writer.write(buf) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    ErrorKind::WriteZero,
                    "peer closed during chunked stream write",
                ));
            }
            Ok(n) => buf = &buf[n..],
            Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(std::io::Error::new(
                        ErrorKind::TimedOut,
                        "chunked stream write timed out",
                    ));
                }
                std::thread::sleep(Duration::from_micros(200));
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

pub(crate) struct ServerLiveStreamWriter {
    stream: Rc<RefCell<ServerConnStream>>,
    tenant_id: String,
    keep_alive: bool,
    is_nonblocking_socket: bool,
    started: bool,
    finished: bool,
    disconnected: bool,
}

impl ServerLiveStreamWriter {
    #[inline]
    pub(crate) fn new(
        stream: Rc<RefCell<ServerConnStream>>,
        tenant_id: &str,
        keep_alive: bool,
        is_nonblocking_socket: bool,
    ) -> Self {
        Self {
            stream,
            tenant_id: tenant_id.to_string(),
            keep_alive,
            is_nonblocking_socket,
            started: false,
            finished: false,
            disconnected: false,
        }
    }
}

impl LiveHttpStreamWriter for ServerLiveStreamWriter {
    fn start_chunked(
        &mut self,
        status: u16,
        content_type: &str,
        extra_headers: &[(String, String)],
    ) -> Result<(), String> {
        if self.started {
            return Ok(());
        }
        let safe_ct = if slice_engine::is_valid_http_header_value(content_type) {
            content_type
        } else {
            "application/octet-stream"
        };
        let conn_hdr = if self.keep_alive {
            "keep-alive"
        } else {
            "close"
        };
        let mut hdr = Vec::with_capacity(256);
        hdr.extend_from_slice(b"HTTP/1.1 ");
        push_u64_ascii(&mut hdr, status as u64);
        hdr.push(b' ');
        hdr.extend_from_slice(http_status_reason(status).as_bytes());
        hdr.extend_from_slice(b"\r\nContent-Type: ");
        hdr.extend_from_slice(safe_ct.as_bytes());
        hdr.extend_from_slice(b"\r\nTransfer-Encoding: chunked\r\n");
        if safe_ct.starts_with("text/event-stream") {
            hdr.extend_from_slice(b"Cache-Control: no-cache\r\nX-Accel-Buffering: no\r\n");
        }
        hdr.extend_from_slice(b"Connection: ");
        hdr.extend_from_slice(conn_hdr.as_bytes());
        hdr.extend_from_slice(b"\r\nServer: Slice-NexusJS/0.2\r\nX-Slice-Tenant: ");
        hdr.extend_from_slice(self.tenant_id.as_bytes());
        hdr.extend_from_slice(b"\r\n");
        for (hk, hv) in extra_headers {
            if !is_forbidden_custom_response_header(hk)
                && slice_engine::is_valid_http_header_name(hk)
                && slice_engine::is_valid_http_header_value(hv)
            {
                hdr.extend_from_slice(hk.as_bytes());
                hdr.extend_from_slice(b": ");
                hdr.extend_from_slice(hv.as_bytes());
                hdr.extend_from_slice(b"\r\n");
            }
        }
        hdr.extend_from_slice(b"\r\n");

        let mut st = self.stream.borrow_mut();
        if let Err(e) = write_all_nonblocking_aware(&mut *st, &hdr).and_then(|()| st.flush()) {
            self.disconnected = true;
            return Err(format!("CLIENT_DISCONNECTED: {e}"));
        }
        self.started = true;
        Ok(())
    }

    fn write_chunk(&mut self, data: &[u8]) -> Result<(), String> {
        if data.is_empty() {
            return Ok(());
        }
        if self.disconnected {
            return Err("CLIENT_DISCONNECTED".to_string());
        }
        let hex_hdr = format!("{:x}\r\n", data.len());
        let mut st = self.stream.borrow_mut();
        if let Err(e) = write_all_nonblocking_aware(&mut *st, hex_hdr.as_bytes())
            .and_then(|()| write_all_nonblocking_aware(&mut *st, data))
            .and_then(|()| write_all_nonblocking_aware(&mut *st, b"\r\n"))
            .and_then(|()| st.flush())
        {
            self.disconnected = true;
            return Err(format!("CLIENT_DISCONNECTED: {e}"));
        }
        Ok(())
    }

    fn finish_chunked(&mut self) -> Result<(), String> {
        if self.finished || !self.started || self.disconnected {
            return Ok(());
        }
        self.finished = true;
        let mut st = self.stream.borrow_mut();
        if let Err(e) =
            write_all_nonblocking_aware(&mut *st, b"0\r\n\r\n").and_then(|()| st.flush())
        {
            self.disconnected = true;
            return Err(format!("CLIENT_DISCONNECTED: {e}"));
        }
        Ok(())
    }

    fn is_client_disconnected(&self) -> bool {
        if self.disconnected {
            return true;
        }
        let st = self.stream.borrow();
        if let ServerConnStream::Plain(ref tcp) = *st {
            if !self.is_nonblocking_socket {
                let _ = tcp.set_nonblocking(true);
            }
            let mut probe = [0u8; 1];
            let closed = match tcp.peek(&mut probe) {
                Ok(0) => true,
                Ok(_) => false,
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => false,
                Err(_) => true,
            };
            if !self.is_nonblocking_socket {
                let _ = tcp.set_nonblocking(false);
            }
            closed
        } else {
            false
        }
    }
}
