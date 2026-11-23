//! OCap UDP Datagram Socket (`node:dgram`) Bind, Non-Blocking Send/Recv, and Close Operations.

use super::policy_and_mock::{
    BOA_SOCKET_MOCKS, generate_mock_tcp_reply, make_socket_url_parts, verify_raw_socket_ocap,
    verify_socket_egress_dlp, verify_transient_socket_io_budget_and_fuel,
};
use super::{DEFAULT_SOCKET_TIMEOUT, SOCKET_IO_BUF, next_socket_handle_id};
use crate::bindings::helpers::record_boa_violation_once;
use crate::boa_realm::{BOA_ACTIVE_TENANT, BOA_NET_HOSTS};
use crate::intrinsics::{base64_decode, base64_encode};
use crate::outbound_http::resolve_and_verify_ssrf_socket_addrs;
use rustc_hash::FxHashMap;
use slice_sandbox::{SandboxViolation, check_net_host_grant, extract_clean_hostname};
use std::cell::RefCell;
use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

enum UdpSocketBackend {
    Live(UdpSocket),
}

struct ActiveUdpSocket {
    backend: UdpSocketBackend,
}

thread_local! {
    static UDP_SOCKET_TABLE: RefCell<FxHashMap<u32, ActiveUdpSocket>> = RefCell::new(FxHashMap::default());
}

pub(super) fn clear_udp_sockets() {
    UDP_SOCKET_TABLE.with(|t| t.borrow_mut().clear());
}

pub(super) fn active_udp_socket_count() -> usize {
    UDP_SOCKET_TABLE.with(|t| t.borrow().len())
}

/// Binds a UDP socket (`"udp4"` or `"udp6"`).
pub fn udp_socket_bind(family: &str, address: &str, port: u16) -> Result<String, String> {
    let is_v6 = family.eq_ignore_ascii_case("udp6") || address.contains(':');
    let bind_host = if address.is_empty() {
        if is_v6 { "::1" } else { "127.0.0.1" }
    } else {
        address.trim()
    };
    let clean_bind_host = extract_clean_hostname(bind_host);

    let bind_permitted = !clean_bind_host.is_empty()
        && BOA_NET_HOSTS.with(|h| {
            let set = h.borrow();
            if set.is_empty() {
                return false;
            }
            if port == 0 || set.contains("*") {
                return true;
            }
            check_net_host_grant(&set, &clean_bind_host, port, true).0
        });
    if !bind_permitted {
        let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
        record_boa_violation_once(SandboxViolation::NetAccessDenied {
            tenant_id,
            host: format!("udp://{}:{}", bind_host, port),
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

                let _ = udp.set_nonblocking(true);
                let recv_res = SOCKET_IO_BUF.with(|cell| {
                    let mut recv_buf = cell.borrow_mut();
                    let mut replies = Vec::new();
                    while let Ok((n, src_addr)) = udp.recv_from(&mut recv_buf[..]) {
                        verify_transient_socket_io_budget_and_fuel(n)?;
                        replies.push(serde_json::json!({
                            "dataBase64": base64_encode(&recv_buf[..n], false),
                            "size": n,
                            "address": src_addr.ip().to_string(),
                            "port": src_addr.port(),
                            "family": if src_addr.is_ipv6() { "IPv6" } else { "IPv4" }
                        }));
                        if replies.len() >= 16 {
                            break;
                        }
                    }
                    Ok::<Vec<serde_json::Value>, String>(replies)
                });
                let _ = udp.set_nonblocking(false);
                let replies = recv_res?;
                if replies.is_empty() {
                    return Ok(format!("{{\"bytesSent\":{},\"replies\":[]}}", sent));
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

/// Polls an active UDP socket for incoming datagrams (waiting up to `timeout_ms` for the first packet,
/// or non-blocking when `timeout_ms == 0`).
pub fn udp_socket_recv(id: u32, timeout_ms: u64) -> Result<String, String> {
    UDP_SOCKET_TABLE.with(|u| {
        let mut table = u.borrow_mut();
        let Some(sock_entry) = table.get_mut(&id) else {
            return Ok("{\"replies\":[]}".to_string());
        };
        match &mut sock_entry.backend {
            UdpSocketBackend::Live(udp) => {
                let nonblocking = timeout_ms == 0;
                if nonblocking {
                    let _ = udp.set_nonblocking(true);
                } else {
                    let _ =
                        udp.set_read_timeout(Some(Duration::from_millis(timeout_ms.min(5_000))));
                }
                let recv_res = SOCKET_IO_BUF.with(|cell| {
                    let mut recv_buf = cell.borrow_mut();
                    let mut replies = Vec::new();
                    if let Ok((n, src_addr)) = udp.recv_from(&mut recv_buf[..]) {
                        verify_transient_socket_io_budget_and_fuel(n)?;
                        replies.push(serde_json::json!({
                            "dataBase64": base64_encode(&recv_buf[..n], false),
                            "size": n,
                            "address": src_addr.ip().to_string(),
                            "port": src_addr.port(),
                            "family": if src_addr.is_ipv6() { "IPv6" } else { "IPv4" }
                        }));
                        if !nonblocking {
                            let _ = udp.set_nonblocking(true);
                        }
                        while replies.len() < 16 {
                            match udp.recv_from(&mut recv_buf[..]) {
                                Ok((n2, src2)) => {
                                    verify_transient_socket_io_budget_and_fuel(n2)?;
                                    replies.push(serde_json::json!({
                                        "dataBase64": base64_encode(&recv_buf[..n2], false),
                                        "size": n2,
                                        "address": src2.ip().to_string(),
                                        "port": src2.port(),
                                        "family": if src2.is_ipv6() { "IPv6" } else { "IPv4" }
                                    }));
                                }
                                Err(_) => break,
                            }
                        }
                    }
                    Ok::<Vec<serde_json::Value>, String>(replies)
                });
                let _ = udp.set_nonblocking(false);
                let _ = udp.set_read_timeout(Some(DEFAULT_SOCKET_TIMEOUT));
                let replies = recv_res?;
                if replies.is_empty() {
                    return Ok("{\"replies\":[]}".to_string());
                }
                Ok(serde_json::json!({ "replies": replies }).to_string())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boa_realm::BOA_EGRESS_SECRETS;

    #[test]
    fn udp_unidirectional_send_burst_is_nonblocking() {
        let sink = UdpSocket::bind("127.0.0.1:0").expect("bind sink udp");
        let sink_port = sink.local_addr().expect("sink local_addr").port();

        BOA_NET_HOSTS.with(|s| {
            let mut set = s.borrow_mut();
            set.clear();
            set.insert("127.0.0.1".to_string());
        });
        BOA_EGRESS_SECRETS.with(|s| s.borrow_mut().clear());

        let bind_json = udp_socket_bind("udp4", "127.0.0.1", 0).expect("bind sender udp");
        let parsed: serde_json::Value = serde_json::from_str(&bind_json).expect("json");
        let sock_id = parsed["id"].as_u64().expect("socket id") as u32;

        let start = std::time::Instant::now();
        for _ in 0..100 {
            udp_socket_send_bytes(sock_id, b"ping", "127.0.0.1", sink_port)
                .expect("nonblocking udp send");
        }
        let elapsed = start.elapsed();
        let _ = udp_socket_close(sock_id);
        BOA_NET_HOSTS.with(|s| s.borrow_mut().clear());

        assert!(
            elapsed < Duration::from_millis(250),
            "100 unidirectional UDP sends took {:?} (expected non-blocking < 250ms)",
            elapsed
        );
    }
}
