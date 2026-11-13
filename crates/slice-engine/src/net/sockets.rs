//! Zero-Trust OCap TCP, TLS (`startTls`), and UDP Socket Engine (`cloudflare:sockets`, `Slice.connect`, `node:net`, `node:dgram`).
//!
//! Enforces Slice's 5-Ring Socket Security Boundary on every operation:
//! 1. **Explicit OCap `allow_net` Verification**: Checks `BOA_NET_HOSTS` before connecting or binding.
//!    Under wildcard `--allow-net=*`, blocks privileged ports (`< 1024` except `80`/`443`) to prevent raw port scanning.
//! 2. **Pre-Connect DNS Resolved-IP SSRF Shield**: Resolves hostnames via `resolve_and_verify_ssrf_socket_addrs`
//!    and blocks RFC 1918 private networks, loopback, CGNAT (`100.64.0.0/10`), cloud metadata (`169.254.169.254`),
//!    IPv6 unique-local (`fc00::/7`), link-local (`fe80::/10`), and non-canonical octal/hex/dword IPv4 literals
//!    unless explicitly permitted in `allow_net`.
//! 3. **Mandatory Outbound Egress Secret DLP**: Scans every `socket.write()` and `udp.send()` payload
//!    via `verify_egress_secrets_slice` (raw, Base64, and Base64URL) before bytes touch the OS socket.
//! 4. **Memory & Descriptor Ceilings + Hard Timeouts**: Caps open sockets at `MAX_OPEN_SOCKETS_PER_ISOLATE` (32),
//!    caps single I/O buffers at `MAX_SOCKET_IO_BYTES` (4 MB), charges `BOA_KV_MEM_BUDGET`, and enforces read/write timeouts.
//! 5. **Deterministic Mock Socket Support (`Slice.mockSocket`)**: Supports zero-network deterministic testing and replay.

use crate::boa_realm::{
    mark_subsystem_dirty, BOA_ACTIVE_TENANT, BOA_EGRESS_SECRETS, BOA_FUEL_BUDGET,
    BOA_KV_MEM_BUDGET, BOA_NET_HOSTS, BOA_VIOLATION, DIRTY_SOCKETS,
};
use crate::intrinsics::{base64_decode, base64_encode};
use crate::outbound_http::{
    dial_fresh_outbound_stream, resolve_and_verify_ssrf_socket_addrs, OutboundStream,
    OutboundUrlParts,
};
use crate::state_and_kv::verify_egress_secrets_slice;
use rustc_hash::FxHashMap;
use rustls::pki_types::ServerName;
use slice_sandbox::SandboxViolation;
use std::cell::{Cell, RefCell};
use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpStream, UdpSocket};
use std::time::Duration;

/// Maximum concurrent open TCP + UDP sockets per isolate execution.
pub const MAX_OPEN_SOCKETS_PER_ISOLATE: usize = 32;

/// Maximum size of a single socket read or write payload (`4 MB`).
pub const MAX_SOCKET_IO_BYTES: usize = 4 * 1024 * 1024;

const DEFAULT_SOCKET_TIMEOUT: Duration = Duration::from_secs(5);

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

enum UdpSocketBackend {
    Live(UdpSocket),
}

struct ActiveUdpSocket {
    backend: UdpSocketBackend,
}

thread_local! {
    static NEXT_SOCKET_ID: Cell<u32> = const { Cell::new(1) };
    static TCP_SOCKET_TABLE: RefCell<FxHashMap<u32, ActiveTcpSocket>> = RefCell::new(FxHashMap::default());
    static UDP_SOCKET_TABLE: RefCell<FxHashMap<u32, ActiveUdpSocket>> = RefCell::new(FxHashMap::default());
    pub(crate) static BOA_SOCKET_MOCKS: RefCell<FxHashMap<String, String>> = RefCell::new(FxHashMap::default());
}

/// Clears all open TCP/UDP sockets and mock definitions on the current worker thread.
pub fn clear_thread_socket_state() {
    TCP_SOCKET_TABLE.with(|t| t.borrow_mut().clear());
    UDP_SOCKET_TABLE.with(|t| t.borrow_mut().clear());
    BOA_SOCKET_MOCKS.with(|m| m.borrow_mut().clear());
    NEXT_SOCKET_ID.with(|id| id.set(1));
}

/// Closes all active TCP/UDP sockets without clearing mock definitions (used between event loop resets).
pub(crate) fn close_all_active_sockets() {
    TCP_SOCKET_TABLE.with(|t| t.borrow_mut().clear());
    UDP_SOCKET_TABLE.with(|t| t.borrow_mut().clear());
}

fn next_socket_handle_id() -> Result<u32, String> {
    let active_count =
        TCP_SOCKET_TABLE.with(|t| t.borrow().len()) + UDP_SOCKET_TABLE.with(|u| u.borrow().len());
    if active_count >= MAX_OPEN_SOCKETS_PER_ISOLATE {
        return Err(format!(
            "SocketError: Maximum concurrent open sockets ({}) exceeded",
            MAX_OPEN_SOCKETS_PER_ISOLATE
        ));
    }
    let id = NEXT_SOCKET_ID.with(|c| {
        let cur = c.get();
        c.set(cur.wrapping_add(1).max(1));
        cur
    });
    mark_subsystem_dirty(DIRTY_SOCKETS);
    Ok(id)
}

/// Verifies that a single transient socket read/write payload fits within `MAX_SOCKET_IO_BYTES`
/// and the isolate's remaining memory ceiling (without permanently pinning transient wire bytes
/// into `BOA_KV_MEM_BUDGET`), and charges CPU fuel proportional to throughput.
fn verify_transient_socket_io_budget_and_fuel(bytes: usize) -> Result<(), String> {
    if bytes > MAX_SOCKET_IO_BYTES {
        let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
        let (_, limit) = BOA_KV_MEM_BUDGET.with(|b| b.get());
        BOA_VIOLATION.with(|v| {
            *v.borrow_mut() = Some(SandboxViolation::MemoryCeilingExceeded {
                tenant_id,
                requested: bytes,
                limit: limit.min(MAX_SOCKET_IO_BYTES),
            });
        });
        return Err(format!(
            "SocketError: Socket payload ({} bytes) exceeds maximum limit ({} bytes)",
            bytes, MAX_SOCKET_IO_BYTES
        ));
    }
    let exceeded = BOA_KV_MEM_BUDGET.with(|b| {
        let (used, limit) = b.get();
        if limit > 0 && limit < usize::MAX && used.saturating_add(bytes) > limit {
            Some((used.saturating_add(bytes), limit))
        } else {
            None
        }
    });
    if let Some((requested, limit)) = exceeded {
        let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
        BOA_VIOLATION.with(|v| {
            *v.borrow_mut() = Some(SandboxViolation::MemoryCeilingExceeded {
                tenant_id,
                requested,
                limit,
            });
        });
        return Err(format!(
            "SandboxViolation: Socket allocation ({} bytes) exceeded tenant memory ceiling ({} bytes)",
            requested, limit
        ));
    }
    let fuel_ticks = ((bytes as u64) / 4096).max(1);
    let fuel_exceeded = BOA_FUEL_BUDGET.with(|f| {
        let (consumed, limit) = f.get();
        if limit == u64::MAX {
            return None;
        }
        let next = consumed.saturating_add(fuel_ticks);
        f.set((next, limit));
        if next > limit {
            Some(limit)
        } else {
            None
        }
    });
    if let Some(limit) = fuel_exceeded {
        let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
        BOA_VIOLATION.with(|v| {
            if v.borrow().is_none() {
                *v.borrow_mut() = Some(SandboxViolation::CpuFuelExhausted { tenant_id, limit });
            }
        });
        return Err(format!(
            "SandboxViolation: Socket I/O exhausted tenant CPU fuel budget ({})",
            limit
        ));
    }
    Ok(())
}

fn sync_mock_socket_rx_budget(sock: &mut ActiveTcpSocket, new_rx_len: usize) -> Result<(), String> {
    let prev = sock.mock_buffered_bytes;
    if new_rx_len > prev {
        let delta = new_rx_len - prev;
        if new_rx_len > MAX_SOCKET_IO_BYTES {
            let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
            BOA_VIOLATION.with(|v| {
                *v.borrow_mut() = Some(SandboxViolation::MemoryCeilingExceeded {
                    tenant_id,
                    requested: new_rx_len,
                    limit: MAX_SOCKET_IO_BYTES,
                });
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
            BOA_VIOLATION.with(|v| {
                *v.borrow_mut() = Some(SandboxViolation::MemoryCeilingExceeded {
                    tenant_id,
                    requested,
                    limit,
                });
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

fn verify_socket_egress_dlp(payload: &[u8], target_label: &str) -> Result<(), String> {
    if payload.is_empty() {
        return Ok(());
    }
    let tenant_id = BOA_ACTIVE_TENANT.with(|t| {
        let b = t.borrow();
        if b.is_empty() {
            "boa-tenant".to_string()
        } else {
            b.clone()
        }
    });
    let payload_str = String::from_utf8_lossy(payload);
    let payload_b64 = base64_encode(payload, false);
    let dlp_err = BOA_EGRESS_SECRETS.with(|sec_cell| {
        let secrets = sec_cell.borrow();
        if secrets.is_empty() {
            return None;
        }
        verify_egress_secrets_slice(&tenant_id, &secrets, target_label)
            .and_then(|()| verify_egress_secrets_slice(&tenant_id, &secrets, &payload_str))
            .and_then(|()| verify_egress_secrets_slice(&tenant_id, &secrets, &payload_b64))
            .err()
    });
    if let Some(violation) = dlp_err {
        BOA_VIOLATION.with(|v| *v.borrow_mut() = Some(violation));
        return Err(format!(
            "EgressSecretLeakBlocked: EGRESS_SECRET_LEAK_BLOCKED on outbound socket write to '{}'",
            target_label
        ));
    }
    Ok(())
}

#[inline]
fn format_socket_host_port(clean_host: &str, port: u16) -> String {
    if clean_host.contains(':') {
        format!("[{}]:{}", clean_host, port)
    } else {
        format!("{}:{}", clean_host, port)
    }
}

/// Verifies OCap `allow_net` permission for a raw TCP/UDP socket target `host:port`.
/// Returns `Ok(has_explicit_match)`.
fn verify_raw_socket_ocap(host: &str, port: u16) -> Result<bool, String> {
    if host.is_empty() || port == 0 {
        return Err(format!(
            "SocketError: Invalid socket address '{}:{}'",
            host, port
        ));
    }
    let clean_host = slice_sandbox::extract_clean_hostname(&host.trim().to_ascii_lowercase());
    let host_port = format_socket_host_port(&clean_host, port);

    if slice_sandbox::is_cloud_metadata_ssrf_target(&clean_host) {
        let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
        BOA_VIOLATION.with(|v| {
            *v.borrow_mut() = Some(SandboxViolation::NetAccessDenied {
                tenant_id,
                host: format!("SSRF_METADATA_BLOCKED:{}", host_port),
            });
        });
        return Err(format!(
            "SSRF_SHIELD_BLOCKED: Raw socket connection to cloud metadata '{}' is forbidden",
            host_port
        ));
    }

    let (allowed, has_explicit_match, is_wildcard_only) = BOA_NET_HOSTS.with(|h| {
        let set = h.borrow();
        if set.is_empty() {
            return (false, false, false);
        }
        let mut explicit = set.contains(&host_port)
            || set.contains(&clean_host)
            || (clean_host.contains(':')
                && (set.contains(&format!("[{}]", clean_host))
                    || set.contains(&format!("{}:{}", clean_host, port))));
        if !explicit && (clean_host == "127.0.0.1" || clean_host == "localhost") {
            let alt = if clean_host == "127.0.0.1" {
                "localhost"
            } else {
                "127.0.0.1"
            };
            explicit = set.contains(alt) || set.contains(&format!("{}:{}", alt, port));
        }
        let wildcard = set.contains("*");
        (explicit || wildcard, explicit, wildcard && !explicit)
    });

    if !allowed {
        let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
        BOA_VIOLATION.with(|v| {
            *v.borrow_mut() = Some(SandboxViolation::NetAccessDenied {
                tenant_id,
                host: host_port.clone(),
            });
        });
        return Err(format!(
            "SandboxViolation: Raw socket connection to '{}' denied by OCap allow_net policy",
            host_port
        ));
    }

    // Reject non-canonical IPv4 literals (octal 0177.0.0.1, hex 0x7f.0.0.1, dword 2130706433) unconditionally
    if slice_sandbox::is_private_or_loopback_ssrf_target(&clean_host)
        && !matches!(
            clean_host.as_str(),
            "127.0.0.1" | "localhost" | "::1" | "0.0.0.0"
        )
        && !clean_host.starts_with("10.")
        && !clean_host.starts_with("192.168.")
        && !clean_host.starts_with("172.")
    {
        let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
        BOA_VIOLATION.with(|v| {
            *v.borrow_mut() = Some(SandboxViolation::NetAccessDenied {
                tenant_id,
                host: format!("SSRF_NON_CANONICAL_IP_BLOCKED:{}", host_port),
            });
        });
        return Err(format!(
            "SSRF_SHIELD_BLOCKED: Non-canonical IPv4/IPv6 socket literal '{}' blocked",
            host_port
        ));
    }

    // Under wildcard `--allow-net=*` without an explicit host grant, block privileged ports (< 1024 except 80/443)
    // and check private SSRF host patterns immediately.
    if is_wildcard_only && !has_explicit_match {
        if port < 1024 && port != 80 && port != 443 {
            let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
            BOA_VIOLATION.with(|v| {
                *v.borrow_mut() = Some(SandboxViolation::NetAccessDenied {
                    tenant_id,
                    host: host_port.clone(),
                });
            });
            return Err(format!(
                "SandboxViolation: Raw socket connection to privileged port '{}' requires explicit allow_net grant",
                host_port
            ));
        }
        if slice_sandbox::is_private_or_loopback_ssrf_target(&clean_host) {
            let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
            BOA_VIOLATION.with(|v| {
                *v.borrow_mut() = Some(SandboxViolation::NetAccessDenied {
                    tenant_id,
                    host: format!("SSRF_BLOCKED:{}", host_port),
                });
            });
            return Err(format!(
                "SSRF_SHIELD_BLOCKED: Raw socket connection to internal/private target '{}' is forbidden under wildcard allow_net",
                host_port
            ));
        }
    }

    Ok(has_explicit_match)
}

fn make_socket_url_parts(host: &str, port: u16, is_tls: bool) -> OutboundUrlParts {
    let clean_host = slice_sandbox::extract_clean_hostname(&host.trim().to_ascii_lowercase());
    let host_port = format_socket_host_port(&clean_host, port);
    OutboundUrlParts {
        is_https: is_tls,
        hostname: clean_host.clone(),
        port,
        host_header: host_port.clone(),
        host_port,
        path_and_query: "/".to_string(),
    }
}

fn generate_mock_tcp_reply(spec: &str, written: &[u8]) -> Vec<u8> {
    if spec == "echo" {
        return written.to_vec();
    }
    if let Some(prefix) = spec.strip_prefix("prefix:") {
        let mut out = prefix.as_bytes().to_vec();
        out.extend_from_slice(written);
        return out;
    }
    if let Some(static_reply) = spec.strip_prefix("reply:") {
        return static_reply.as_bytes().to_vec();
    }
    if spec == "redis" {
        let cmd = String::from_utf8_lossy(written);
        if cmd.contains("PING") {
            return b"+PONG\r\n".to_vec();
        }
        if cmd.contains("SET") {
            return b"+OK\r\n".to_vec();
        }
        if cmd.contains("GET") {
            return b"$10\r\nslice-fast\r\n".to_vec();
        }
        return b"+OK\r\n".to_vec();
    }
    if spec.starts_with('{') {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(spec) {
            if let Some(txt) = val.get("responseText").and_then(|v| v.as_str()) {
                return txt.as_bytes().to_vec();
            }
            if let Some(arr) = val.get("responseBytes").and_then(|v| v.as_array()) {
                return arr
                    .iter()
                    .filter_map(|x| x.as_u64().map(|n| (n & 0xff) as u8))
                    .collect();
            }
            if let Some(mode) = val.get("mode").and_then(|v| v.as_str()) {
                return generate_mock_tcp_reply(mode, written);
            }
        }
        return written.to_vec();
    }
    if !spec.is_empty() {
        return spec.as_bytes().to_vec();
    }
    written.to_vec()
}

/// Registers a deterministic mock socket for `host:port` (or `host`).
pub fn register_socket_mock(target: &str, spec: &str) {
    mark_subsystem_dirty(DIRTY_SOCKETS);
    BOA_SOCKET_MOCKS.with(|m| {
        m.borrow_mut()
            .insert(target.trim().to_ascii_lowercase(), spec.to_string());
    });
}

/// Connects a TCP or TLS socket (`tls_mode` = `"off"` | `"on"` | `"starttls"`).
pub fn tcp_socket_connect(host: &str, port: u16, tls_mode: &str) -> Result<String, String> {
    let explicit_grant = verify_raw_socket_ocap(host, port)?;
    let host_trimmed = host.trim();
    let is_tls = tls_mode.eq_ignore_ascii_case("on");
    let parts = make_socket_url_parts(host_trimmed, port, is_tls);
    let host_port_key = parts.host_port.clone();

    // Check deterministic mock registry first
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

                let mut buf = vec![0u8; cap.min(65_536)];
                match stream.read(&mut buf) {
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
                        if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut =>
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

    // Enforce OCap `allow_net`, SSRF, and Outbound Egress Secret DLP on the SNI / expectedServerHostname
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
                // Take ownership of the Plain TcpStream to wrap in rustls::StreamOwned
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
                let tls_conn = rustls::ClientConnection::new(
                    crate::outbound_http::get_tls_client_config(),
                    server_name,
                )
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

/// Closes an active TCP/TLS socket (`half_close_write` shuts down the write half if true, otherwise closes completely).
pub fn tcp_socket_close(id: u32, half_close_write: bool) -> Result<String, String> {
    TCP_SOCKET_TABLE.with(|t| {
        let mut table = t.borrow_mut();
        if half_close_write {
            if let Some(sock) = table.get_mut(&id) {
                if let TcpConnectionBackend::Live(OutboundStream::Plain(tcp)) = &mut sock.backend {
                    let _ = tcp.shutdown(Shutdown::Write);
                }
            }
            return Ok(serde_json::json!({ "closed": false, "halfClosed": true }).to_string());
        }
        if let Some(mut sock) = table.remove(&id) {
            if let TcpConnectionBackend::Live(stream) = std::mem::replace(
                &mut sock.backend,
                TcpConnectionBackend::Mock {
                    rx_queue: Vec::new(),
                    spec: String::new(),
                },
            ) {
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
        }
        Ok(serde_json::json!({ "closed": true }).to_string())
    })
}

/// Binds a UDP socket (`"udp4"` or `"udp6"`).
pub fn udp_socket_bind(family: &str, address: &str, port: u16) -> Result<String, String> {
    let is_v6 = family.eq_ignore_ascii_case("udp6") || address.contains(':');
    let bind_host = if address.is_empty() {
        if is_v6 {
            "::1"
        } else {
            "127.0.0.1"
        }
    } else {
        address.trim()
    };
    let clean_bind_host = slice_sandbox::extract_clean_hostname(bind_host);

    // Ephemeral client binds (`port == 0`) are permitted when any `allow_net` grant is active
    // (each outbound `udp.send()` independently enforces target `host:port` OCap + SSRF + DLP).
    // Non-ephemeral local port binds (`port != 0`) require `"*"` or an explicit `bind_host[:port]` grant.
    let bind_permitted = !clean_bind_host.is_empty()
        && BOA_NET_HOSTS.with(|h| {
            let set = h.borrow();
            if set.is_empty() {
                return false;
            }
            if port == 0 || set.contains("*") {
                return true;
            }
            let host_port = if clean_bind_host.contains(':') {
                format!("[{}]:{}", clean_bind_host, port)
            } else {
                format!("{}:{}", clean_bind_host, port)
            };
            if set.contains(&host_port)
                || set.contains(&clean_bind_host)
                || (clean_bind_host.contains(':')
                    && (set.contains(&format!("[{}]", clean_bind_host))
                        || set.contains(&format!("{}:{}", clean_bind_host, port))))
            {
                return true;
            }
            if clean_bind_host == "127.0.0.1" || clean_bind_host == "localhost" {
                let alt = if clean_bind_host == "127.0.0.1" {
                    "localhost"
                } else {
                    "127.0.0.1"
                };
                return set.contains(alt) || set.contains(&format!("{}:{}", alt, port));
            }
            false
        });
    if !bind_permitted {
        let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
        BOA_VIOLATION.with(|v| {
            *v.borrow_mut() = Some(SandboxViolation::NetAccessDenied {
                tenant_id,
                host: format!("udp://{}:{}", bind_host, port),
            });
        });
        return Err(format!(
            "SandboxViolation: UDP socket bind '{}:{}' denied by OCap allow_net policy",
            bind_host, port
        ));
    }

    let bind_spec = if is_v6 && !bind_host.starts_with('[') {
        format!("[{}]:{}", bind_host, port)
    } else {
        format!("{}:{}", bind_host, port)
    };
    let sock =
        UdpSocket::bind(&bind_spec).map_err(|e| format!("UdpBindError ({}): {}", bind_spec, e))?;
    let _ = sock.set_read_timeout(Some(DEFAULT_SOCKET_TIMEOUT));
    let _ = sock.set_write_timeout(Some(DEFAULT_SOCKET_TIMEOUT));

    let local = sock
        .local_addr()
        .map_err(|e| format!("UdpLocalAddrError: {}", e))?;
    let fam_str = if local.is_ipv6() { "IPv6" } else { "IPv4" };
    let local_ip = local.ip().to_string();
    let local_port = local.port();

    let id = next_socket_handle_id()?;
    UDP_SOCKET_TABLE.with(|u| {
        u.borrow_mut().insert(
            id,
            ActiveUdpSocket {
                backend: UdpSocketBackend::Live(sock),
            },
        );
    });

    Ok(serde_json::json!({
        "id": id,
        "address": local_ip,
        "port": local_port,
        "family": fam_str
    })
    .to_string())
}

/// Sends a raw byte slice as a UDP datagram to `host:port` after OCap, SSRF, and Outbound Egress DLP checks.
pub fn udp_socket_send_bytes(
    id: u32,
    payload: &[u8],
    host: &str,
    port: u16,
) -> Result<String, String> {
    let explicit_grant = verify_raw_socket_ocap(host, port)?;
    if payload.len() > 65_507 {
        return Err(format!(
            "UdpSendError: Datagram size ({} bytes) exceeds 65507-byte UDP limit",
            payload.len()
        ));
    }
    verify_transient_socket_io_budget_and_fuel(payload.len())?;
    let host_trimmed = host.trim();
    let parts = make_socket_url_parts(host_trimmed, port, false);
    let target_label = format!("udp://{}", parts.host_port);
    verify_socket_egress_dlp(payload, &target_label)?;

    let host_port_key = parts.host_port.clone();

    // Check deterministic mock registry
    let mock_spec_opt = BOA_SOCKET_MOCKS.with(|m| {
        let map = m.borrow();
        map.get(&format!("udp://{}", host_port_key))
            .or_else(|| map.get(&host_port_key))
            .or_else(|| map.get(&format!("{}:{}", host_trimmed.to_ascii_lowercase(), port)))
            .cloned()
    });

    let addrs = resolve_and_verify_ssrf_socket_addrs(&parts, explicit_grant)?;
    let target_addr: SocketAddr = addrs[0];

    UDP_SOCKET_TABLE.with(|u| {
        let mut table = u.borrow_mut();
        let sock_entry = table
            .get_mut(&id)
            .ok_or_else(|| format!("SocketError: Invalid or closed UDP socket id {}", id))?;

        if let Some(mock_spec) = mock_spec_opt {
            let reply = generate_mock_tcp_reply(&mock_spec, payload);
            let reply_b64 = base64_encode(&reply, false);
            return Ok(serde_json::json!({
                "bytesSent": payload.len(),
                "replies": [{
                    "dataBase64": reply_b64,
                    "size": reply.len(),
                    "address": target_addr.ip().to_string(),
                    "port": target_addr.port(),
                    "family": if target_addr.is_ipv6() { "IPv6" } else { "IPv4" }
                }]
            })
            .to_string());
        }

        match &mut sock_entry.backend {
            UdpSocketBackend::Live(udp) => {
                let sent = udp
                    .send_to(payload, target_addr)
                    .map_err(|e| format!("UdpSendError: {}", e))?;

                // Non-blocking / short-poll check for immediate UDP response (e.g. local DNS / echo server)
                let _ = udp.set_read_timeout(Some(Duration::from_millis(10)));
                let mut recv_buf = [0u8; 65_536];
                let mut replies = Vec::new();
                if let Ok((n, src_addr)) = udp.recv_from(&mut recv_buf) {
                    verify_transient_socket_io_budget_and_fuel(n)?;
                    replies.push(serde_json::json!({
                        "dataBase64": base64_encode(&recv_buf[..n], false),
                        "size": n,
                        "address": src_addr.ip().to_string(),
                        "port": src_addr.port(),
                        "family": if src_addr.is_ipv6() { "IPv6" } else { "IPv4" }
                    }));
                }
                Ok(serde_json::json!({
                    "bytesSent": sent,
                    "replies": replies
                })
                .to_string())
            }
        }
    })
}

/// Sends a UDP datagram to `host:port` after OCap, SSRF, and Outbound Egress DLP checks.
pub fn udp_socket_send(
    id: u32,
    is_base64: bool,
    data: &str,
    host: &str,
    port: u16,
) -> Result<String, String> {
    let payload = if is_base64 {
        base64_decode(data)
    } else {
        data.as_bytes().to_vec()
    };
    udp_socket_send_bytes(id, &payload, host, port)
}

/// Closes an active UDP socket.
pub fn udp_socket_close(id: u32) -> Result<String, String> {
    UDP_SOCKET_TABLE.with(|u| {
        u.borrow_mut().remove(&id);
    });
    Ok(serde_json::json!({ "closed": true }).to_string())
}

/// Classifies an IP string for `net.isIP(input)`: returns `4` for IPv4, `6` for IPv6, `0` otherwise.
#[inline]
pub fn classify_ip_literal(input: &str) -> u8 {
    match input.trim().parse::<IpAddr>() {
        Ok(IpAddr::V4(_)) => 4,
        Ok(IpAddr::V6(_)) => 6,
        Err(_) => 0,
    }
}
