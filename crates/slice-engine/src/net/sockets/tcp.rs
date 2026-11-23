//! OCap TCP & TLS (`startTls`) Socket Lifecycle, Read/Write, and Mock Queue Enforcement.

use super::policy_and_mock::{
    BOA_SOCKET_MOCKS, generate_mock_tcp_reply, make_socket_url_parts, verify_raw_socket_ocap,
    verify_socket_egress_dlp, verify_transient_socket_io_budget_and_fuel,
};
use super::{MAX_SOCKET_IO_BYTES, SOCKET_IO_BUF, next_socket_handle_id};
use crate::bindings::helpers::record_boa_violation_once;
use crate::boa_realm::{BOA_ACTIVE_TENANT, BOA_KV_MEM_BUDGET};
use crate::intrinsics::{base64_decode, base64_encode};
use crate::outbound_http::{
    OutboundStream, dial_fresh_outbound_stream, get_tls_client_config,
    resolve_and_verify_ssrf_socket_addrs,
};
use rustc_hash::FxHashMap;
use rustls::pki_types::ServerName;
use slice_sandbox::SandboxViolation;
use std::cell::RefCell;
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::time::Duration;

enum TcpConnectionBackend {
    Live(OutboundStream),
    Mock { rx_queue: Vec<u8>, spec: String },
}

struct ActiveTcpSocket {
    host: String,
    port: u16,
    is_tls: bool,
    backend: TcpConnectionBackend,
    mock_buffered_bytes: usize,
}

impl Drop for ActiveTcpSocket {
    fn drop(&mut self) {
        if self.mock_buffered_bytes > 0 {
            let release = self.mock_buffered_bytes;
            self.mock_buffered_bytes = 0;
            BOA_KV_MEM_BUDGET.with(|b| {
                let (used, limit) = b.get();
                b.set((used.saturating_sub(release), limit));
            });
        }
    }
}

thread_local! {
    static TCP_SOCKET_TABLE: RefCell<FxHashMap<u32, ActiveTcpSocket>> = RefCell::new(FxHashMap::default());
}

pub(super) fn clear_tcp_sockets() {
    TCP_SOCKET_TABLE.with(|t| t.borrow_mut().clear());
}

pub(super) fn active_tcp_socket_count() -> usize {
    TCP_SOCKET_TABLE.with(|t| t.borrow().len())
}

fn sync_mock_socket_rx_budget(sock: &mut ActiveTcpSocket, new_rx_len: usize) -> Result<(), String> {
    let prev = sock.mock_buffered_bytes;
    if new_rx_len > prev {
        let delta = new_rx_len - prev;
        if new_rx_len > MAX_SOCKET_IO_BYTES {
            let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
            record_boa_violation_once(SandboxViolation::MemoryCeilingExceeded {
                tenant_id,
                requested: new_rx_len,
                limit: MAX_SOCKET_IO_BYTES,
            });
            return Err(format!(
                "SocketError: Buffered mock socket payload ({} bytes) exceeds maximum limit ({} bytes)",
                new_rx_len, MAX_SOCKET_IO_BYTES
            ));
        }
        let exceeded = BOA_KV_MEM_BUDGET.with(|b| {
            let (used, limit) = b.get();
            if limit > 0 && limit < usize::MAX && used.saturating_add(delta) > limit {
                Some((used.saturating_add(delta), limit))
            } else {
                b.set((used.saturating_add(delta), limit));
                None
            }
        });
        if let Some((requested, limit)) = exceeded {
            let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
            record_boa_violation_once(SandboxViolation::MemoryCeilingExceeded {
                tenant_id,
                requested,
                limit,
            });
            return Err(format!(
                "SandboxViolation: Socket allocation ({} bytes) exceeded tenant memory ceiling ({} bytes)",
                requested, limit
            ));
        }
        sock.mock_buffered_bytes = new_rx_len;
    } else if new_rx_len < prev {
        let release = prev - new_rx_len;
        BOA_KV_MEM_BUDGET.with(|b| {
            let (used, limit) = b.get();
            b.set((used.saturating_sub(release), limit));
        });
        sock.mock_buffered_bytes = new_rx_len;
    }
    Ok(())
}

/// Connects a TCP or TLS socket (`tls_mode` = `"off"` | `"on"` | `"starttls"`).
pub fn tcp_socket_connect(host: &str, port: u16, tls_mode: &str) -> Result<String, String> {
    let explicit_grant = verify_raw_socket_ocap(host, port)?;
    let host_trimmed = host.trim();
    let is_tls = tls_mode.eq_ignore_ascii_case("on");
    let parts = make_socket_url_parts(host_trimmed, port, is_tls);
    let host_port_key = parts.host_port.clone();

    if let Some(mock_spec) = BOA_SOCKET_MOCKS.with(|m| {
        let map = m.borrow();
        map.get(&host_port_key)
            .or_else(|| map.get(&format!("{}:{}", host_trimmed.to_ascii_lowercase(), port)))
            .or_else(|| map.get(&host_trimmed.to_ascii_lowercase()))
            .or_else(|| map.get(&parts.hostname))
            .cloned()
    }) {
        let id = next_socket_handle_id()?;
        let initial_rx = if let Some(banner) = mock_spec.strip_prefix("banner:") {
            banner.as_bytes().to_vec()
        } else {
            Vec::new()
        };
        let initial_len = initial_rx.len();
        let mut mock_sock = ActiveTcpSocket {
            host: host_trimmed.to_string(),
            port,
            is_tls,
            backend: TcpConnectionBackend::Mock {
                rx_queue: initial_rx,
                spec: mock_spec,
            },
            mock_buffered_bytes: 0,
        };
        if initial_len > 0 {
            sync_mock_socket_rx_budget(&mut mock_sock, initial_len)?;
        }
        TCP_SOCKET_TABLE.with(|t| {
            t.borrow_mut().insert(id, mock_sock);
        });
        return Ok(serde_json::json!({
            "id": id,
            "localAddress": "127.0.0.1",
            "localPort": 50000 + (id as u16),
            "remoteAddress": "127.0.0.1",
            "remotePort": port,
            "family": "IPv4",
            "tls": is_tls
        })
        .to_string());
    }

    let verified_addrs = resolve_and_verify_ssrf_socket_addrs(&parts, explicit_grant)?;
    let stream = dial_fresh_outbound_stream(&parts, &verified_addrs)?;

    let (local_addr, local_port, remote_addr, remote_port, family) = match &stream {
        OutboundStream::Plain(tcp) => extract_tcp_addrs(tcp),
        OutboundStream::Tls(tls) => extract_tcp_addrs(tls.get_ref()),
        OutboundStream::TunneledTls(tls) => extract_tcp_addrs(tls.get_ref().get_ref()),
    };

    let id = next_socket_handle_id()?;
    TCP_SOCKET_TABLE.with(|t| {
        t.borrow_mut().insert(
            id,
            ActiveTcpSocket {
                host: host_trimmed.to_string(),
                port,
                is_tls,
                backend: TcpConnectionBackend::Live(stream),
                mock_buffered_bytes: 0,
            },
        );
    });

    Ok(serde_json::json!({
        "id": id,
        "localAddress": local_addr,
        "localPort": local_port,
        "remoteAddress": remote_addr,
        "remotePort": remote_port,
        "family": family,
        "tls": is_tls
    })
    .to_string())
}

fn extract_tcp_addrs(tcp: &TcpStream) -> (String, u16, String, u16, &'static str) {
    let (l_ip, l_port) = tcp
        .local_addr()
        .map(|a| (a.ip().to_string(), a.port()))
        .unwrap_or_else(|_| ("0.0.0.0".to_string(), 0));
    let (r_ip, r_port, fam) = tcp
        .peer_addr()
        .map(|a| {
            let f = if a.is_ipv6() { "IPv6" } else { "IPv4" };
            (a.ip().to_string(), a.port(), f)
        })
        .unwrap_or_else(|_| ("0.0.0.0".to_string(), 0, "IPv4"));
    (l_ip, l_port, r_ip, r_port, fam)
}

/// Writes raw bytes to an active TCP/TLS socket after Egress DLP verification.
pub fn tcp_socket_write_bytes(id: u32, payload: &[u8]) -> Result<String, String> {
    verify_transient_socket_io_budget_and_fuel(payload.len())?;

    TCP_SOCKET_TABLE.with(|t| {
        let mut table = t.borrow_mut();
        let sock = table
            .get_mut(&id)
            .ok_or_else(|| format!("SocketError: Invalid or closed TCP socket id {}", id))?;
        let target_label = format!("{}:{}", sock.host, sock.port);
        verify_socket_egress_dlp(payload, &target_label)?;

        let new_mock_len = match &mut sock.backend {
            TcpConnectionBackend::Mock { rx_queue, spec } => {
                let reply = generate_mock_tcp_reply(spec, payload);
                let next_len = rx_queue.len().saturating_add(reply.len());
                Some((reply, next_len))
            }
            TcpConnectionBackend::Live(stream) => {
                stream
                    .write_all(payload)
                    .map_err(|e| format!("SocketWriteError: {}", e))?;
                stream
                    .flush()
                    .map_err(|e| format!("SocketFlushError: {}", e))?;
                None
            }
        };
        if let Some((reply, next_len)) = new_mock_len {
            sync_mock_socket_rx_budget(sock, next_len)?;
            if let TcpConnectionBackend::Mock { rx_queue, .. } = &mut sock.backend {
                rx_queue.extend_from_slice(&reply);
            }
        }

        Ok(serde_json::json!({
            "bytesWritten": payload.len()
        })
        .to_string())
    })
}

/// Writes bytes (decoded from Base64 or raw UTF-8) to an active TCP/TLS socket after Egress DLP verification.
pub fn tcp_socket_write(id: u32, is_base64: bool, data: &str) -> Result<String, String> {
    let payload = if is_base64 {
        base64_decode(data)
    } else {
        data.as_bytes().to_vec()
    };
    tcp_socket_write_bytes(id, &payload)
}

/// Reads available bytes from an active TCP/TLS socket, up to `max_bytes`, waiting up to `timeout_ms`.
pub fn tcp_socket_read(id: u32, max_bytes: usize, timeout_ms: u64) -> Result<String, String> {
    let cap = max_bytes.clamp(1, MAX_SOCKET_IO_BYTES);
    TCP_SOCKET_TABLE.with(|t| {
        let mut table = t.borrow_mut();
        let sock = table
            .get_mut(&id)
            .ok_or_else(|| format!("SocketError: Invalid or closed TCP socket id {}", id))?;

        match &mut sock.backend {
            TcpConnectionBackend::Mock { rx_queue, .. } => {
                if rx_queue.is_empty() {
                    return Ok(serde_json::json!({
                        "bytesRead": 0,
                        "eof": true,
                        "dataBase64": ""
                    })
                    .to_string());
                }
                let take_len = rx_queue.len().min(cap);
                verify_transient_socket_io_budget_and_fuel(take_len)?;
                let chunk: Vec<u8> = rx_queue.drain(..take_len).collect();
                let rem_len = rx_queue.len();
                let _ = sync_mock_socket_rx_budget(sock, rem_len);
                Ok(serde_json::json!({
                    "bytesRead": chunk.len(),
                    "eof": rem_len == 0,
                    "dataBase64": base64_encode(&chunk, false)
                })
                .to_string())
            }
            TcpConnectionBackend::Live(stream) => {
                let read_to = Duration::from_millis(timeout_ms.clamp(1, 5_000));
                stream.set_read_timeout(Some(read_to));

                let step = cap.min(65_536);
                SOCKET_IO_BUF.with(|cell| {
                    let mut buf = cell.borrow_mut();
                    match stream.read(&mut buf[..step]) {
                        Ok(0) => Ok(serde_json::json!({
                            "bytesRead": 0,
                            "eof": true,
                            "dataBase64": ""
                        })
                        .to_string()),
                        Ok(n) => {
                            verify_transient_socket_io_budget_and_fuel(n)?;
                            Ok(serde_json::json!({
                                "bytesRead": n,
                                "eof": false,
                                "dataBase64": base64_encode(&buf[..n], false)
                            })
                            .to_string())
                        }
                        Err(e)
                            if e.kind() == ErrorKind::WouldBlock
                                || e.kind() == ErrorKind::TimedOut =>
                        {
                            Ok(serde_json::json!({
                                "bytesRead": 0,
                                "eof": false,
                                "timedOut": true,
                                "dataBase64": ""
                            })
                            .to_string())
                        }
                        Err(e) => Err(format!("SocketReadError: {}", e)),
                    }
                })
            }
        }
    })
}

/// Upgrades a plain TCP socket (`tls_mode: "starttls"`) to TLS 1.3 in-place.
pub fn tcp_socket_starttls(id: u32, server_name_override: Option<&str>) -> Result<String, String> {
    let (sock_host, sock_port, is_tls) = TCP_SOCKET_TABLE.with(|t| {
        let table = t.borrow();
        let sock = table
            .get(&id)
            .ok_or_else(|| format!("SocketError: Invalid or closed TCP socket id {}", id))?;
        Ok::<(String, u16, bool), String>((sock.host.clone(), sock.port, sock.is_tls))
    })?;
    if is_tls {
        return Err("SocketError: Socket is already using TLS".to_string());
    }
    let sni_host = server_name_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&sock_host)
        .to_string();

    verify_raw_socket_ocap(&sni_host, sock_port)?;
    verify_socket_egress_dlp(
        sni_host.as_bytes(),
        &format!("tls://{}:{}", sni_host, sock_port),
    )?;

    TCP_SOCKET_TABLE.with(|t| {
        let mut table = t.borrow_mut();
        let sock = table
            .get_mut(&id)
            .ok_or_else(|| format!("SocketError: Invalid or closed TCP socket id {}", id))?;

        match &mut sock.backend {
            TcpConnectionBackend::Mock { .. } => {
                sock.is_tls = true;
                sock.host = sni_host;
                Ok(serde_json::json!({ "id": id, "tls": true }).to_string())
            }
            TcpConnectionBackend::Live(_) => {
                let mut removed = table.remove(&id).unwrap();
                let TcpConnectionBackend::Live(OutboundStream::Plain(tcp)) = std::mem::replace(
                    &mut removed.backend,
                    TcpConnectionBackend::Mock {
                        rx_queue: Vec::new(),
                        spec: String::new(),
                    },
                ) else {
                    return Err("SocketError: Cannot upgrade non-plain TCP stream".to_string());
                };
                let sni_trimmed = sni_host
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .to_string();
                let server_name = ServerName::try_from(sni_trimmed)
                    .map_err(|e| format!("TLS Invalid DNS ServerName '{}': {}", sni_host, e))?;
                let tls_conn = rustls::ClientConnection::new(get_tls_client_config(), server_name)
                    .map_err(|e| format!("TLS Handshake Init Error: {}", e))?;
                let tls_stream = rustls::StreamOwned::new(tls_conn, tcp);
                removed.host = sni_host;
                removed.is_tls = true;
                removed.backend =
                    TcpConnectionBackend::Live(OutboundStream::Tls(Box::new(tls_stream)));
                table.insert(id, removed);
                Ok(serde_json::json!({ "id": id, "tls": true }).to_string())
            }
        }
    })
}

/// Closes an active TCP/TLS socket (`half_close_write` shuts down the write half if true).
pub fn tcp_socket_close(id: u32, half_close_write: bool) -> Result<String, String> {
    TCP_SOCKET_TABLE.with(|t| {
        let mut table = t.borrow_mut();
        if half_close_write {
            if let Some(sock) = table.get_mut(&id)
                && let TcpConnectionBackend::Live(OutboundStream::Plain(tcp)) = &mut sock.backend
            {
                let _ = tcp.shutdown(Shutdown::Write);
            }
            return Ok(serde_json::json!({ "closed": false, "halfClosed": true }).to_string());
        }
        if let Some(mut sock) = table.remove(&id)
            && let TcpConnectionBackend::Live(stream) = std::mem::replace(
                &mut sock.backend,
                TcpConnectionBackend::Mock {
                    rx_queue: Vec::new(),
                    spec: String::new(),
                },
            )
        {
            match stream {
                OutboundStream::Plain(tcp) => {
                    let _ = tcp.shutdown(Shutdown::Both);
                }
                OutboundStream::Tls(mut tls) => {
                    let _ = tls.flush();
                    let _ = tls.get_ref().shutdown(Shutdown::Both);
                }
                OutboundStream::TunneledTls(mut tls) => {
                    let _ = tls.flush();
                    let _ = tls.get_ref().get_ref().shutdown(Shutdown::Both);
                }
            }
        }
        Ok(serde_json::json!({ "closed": true }).to_string())
    })
}
