//! 5-Ring Raw Socket OCap & SSRF Policy Guard, Transient Memory/Fuel Accounting, DLP, and Mock Registry.

use super::MAX_SOCKET_IO_BYTES;
use crate::bindings::helpers::{active_tenant_id_or_default, record_boa_violation_once};
use crate::boa_realm::{
    BOA_ACTIVE_TENANT, BOA_EGRESS_SECRETS, BOA_FUEL_BUDGET, BOA_KV_MEM_BUDGET, BOA_NET_HOSTS,
    DIRTY_SOCKETS, mark_subsystem_dirty,
};
use crate::intrinsics::base64_encode;
use crate::outbound_http::OutboundUrlParts;
use crate::state_and_kv::verify_egress_secrets_slice;
use rustc_hash::FxHashMap;
use slice_sandbox::{
    SandboxViolation, check_net_host_grant, extract_clean_hostname, is_cloud_metadata_clean_host,
    is_private_or_loopback_clean_host,
};
use std::cell::RefCell;

thread_local! {
    pub(crate) static BOA_SOCKET_MOCKS: RefCell<FxHashMap<String, String>> = RefCell::new(FxHashMap::default());
}

/// Registers a deterministic mock socket for `host:port` (or `host`).
pub fn register_socket_mock(target: &str, spec: &str) {
    mark_subsystem_dirty(DIRTY_SOCKETS);
    BOA_SOCKET_MOCKS.with(|m| {
        m.borrow_mut()
            .insert(target.trim().to_ascii_lowercase(), spec.to_string());
    });
}

/// Verifies that a single transient socket read/write payload fits within `MAX_SOCKET_IO_BYTES`
/// and the isolate's remaining memory ceiling, and charges CPU fuel proportional to throughput.
pub(super) fn verify_transient_socket_io_budget_and_fuel(bytes: usize) -> Result<(), String> {
    if bytes > MAX_SOCKET_IO_BYTES {
        let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
        let (_, limit) = BOA_KV_MEM_BUDGET.with(|b| b.get());
        record_boa_violation_once(SandboxViolation::MemoryCeilingExceeded {
            tenant_id,
            requested: bytes,
            limit: limit.min(MAX_SOCKET_IO_BYTES),
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
    let fuel_ticks = ((bytes as u64) / 4096).max(1);
    let fuel_exceeded = BOA_FUEL_BUDGET.with(|f| {
        let (consumed, limit) = f.get();
        if limit == u64::MAX {
            return None;
        }
        let next = consumed.saturating_add(fuel_ticks);
        f.set((next, limit));
        (next > limit).then_some(limit)
    });
    if let Some(limit) = fuel_exceeded {
        let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
        record_boa_violation_once(SandboxViolation::CpuFuelExhausted { tenant_id, limit });
        return Err(format!(
            "SandboxViolation: Socket I/O exhausted tenant CPU fuel budget ({})",
            limit
        ));
    }
    Ok(())
}

pub(super) fn verify_socket_egress_dlp(payload: &[u8], target_label: &str) -> Result<(), String> {
    if payload.is_empty() {
        return Ok(());
    }
    let dlp_err = BOA_EGRESS_SECRETS.with(|sec_cell| {
        let secrets = sec_cell.borrow();
        if secrets.is_empty() {
            return None;
        }
        let tenant_id = active_tenant_id_or_default();
        let payload_str = String::from_utf8_lossy(payload);
        let payload_b64 = base64_encode(payload, false);
        verify_egress_secrets_slice(&tenant_id, &secrets, target_label)
            .and_then(|()| verify_egress_secrets_slice(&tenant_id, &secrets, &payload_str))
            .and_then(|()| verify_egress_secrets_slice(&tenant_id, &secrets, &payload_b64))
            .err()
    });
    if let Some(violation) = dlp_err {
        record_boa_violation_once(violation);
        return Err(format!(
            "EgressSecretLeakBlocked: EGRESS_SECRET_LEAK_BLOCKED on outbound socket write to '{}'",
            target_label
        ));
    }
    Ok(())
}

#[inline]
pub(super) fn format_socket_host_port(clean_host: &str, port: u16) -> String {
    if clean_host.contains(':') {
        format!("[{}]:{}", clean_host, port)
    } else {
        format!("{}:{}", clean_host, port)
    }
}

/// Verifies OCap `allow_net` permission for a raw TCP/UDP socket target `host:port`.
/// Returns `Ok(has_explicit_match)`.
pub(super) fn verify_raw_socket_ocap(host: &str, port: u16) -> Result<bool, String> {
    if host.is_empty() || port == 0 {
        return Err(format!(
            "SocketError: Invalid socket address '{}:{}'",
            host, port
        ));
    }
    let clean_host = extract_clean_hostname(&host.trim().to_ascii_lowercase());
    if clean_host.is_empty() {
        let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
        record_boa_violation_once(SandboxViolation::NetAccessDenied {
            tenant_id,
            host: format!("{}:{}", host.trim(), port),
        });
        return Err(format!(
            "SocketError: Invalid or forbidden socket hostname '{}:{}'",
            host, port
        ));
    }
    let host_port = format_socket_host_port(&clean_host, port);

    if is_cloud_metadata_clean_host(&clean_host) {
        let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
        record_boa_violation_once(SandboxViolation::NetAccessDenied {
            tenant_id,
            host: format!("SSRF_METADATA_BLOCKED:{}", host_port),
        });
        return Err(format!(
            "SSRF_SHIELD_BLOCKED: Raw socket connection to cloud metadata '{}' is forbidden",
            host_port
        ));
    }

    let (has_explicit_match, wildcard_public_match, has_wildcard_rule) = BOA_NET_HOSTS.with(|h| {
        let set = h.borrow();
        let (explicit, wildcard) = check_net_host_grant(&set, &clean_host, port, true);
        (explicit, wildcard, set.contains("*"))
    });

    if !has_explicit_match && !has_wildcard_rule {
        let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
        record_boa_violation_once(SandboxViolation::NetAccessDenied {
            tenant_id,
            host: host_port.clone(),
        });
        return Err(format!(
            "SandboxViolation: Raw socket connection to '{}' denied by OCap allow_net policy",
            host_port
        ));
    }

    // Reject non-canonical IPv4 literals (octal 0177.0.0.1, hex 0x7f.0.0.1, dword 2130706433) unconditionally
    if is_private_or_loopback_clean_host(&clean_host)
        && !matches!(
            clean_host.as_str(),
            "127.0.0.1" | "localhost" | "::1" | "0.0.0.0"
        )
        && !clean_host.starts_with("10.")
        && !clean_host.starts_with("192.168.")
        && !clean_host.starts_with("172.")
    {
        let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
        record_boa_violation_once(SandboxViolation::NetAccessDenied {
            tenant_id,
            host: format!("SSRF_NON_CANONICAL_IP_BLOCKED:{}", host_port),
        });
        return Err(format!(
            "SSRF_SHIELD_BLOCKED: Non-canonical IPv4/IPv6 socket literal '{}' blocked",
            host_port
        ));
    }

    // Under wildcard `--allow-net=*` without an explicit host grant, block privileged ports (< 1024 except 80/443)
    // and check private SSRF host patterns immediately.
    if !has_explicit_match {
        if port < 1024 && port != 80 && port != 443 {
            let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
            record_boa_violation_once(SandboxViolation::NetAccessDenied {
                tenant_id,
                host: host_port.clone(),
            });
            return Err(format!(
                "SandboxViolation: Raw socket connection to privileged port '{}' requires explicit allow_net grant",
                host_port
            ));
        }
        if !wildcard_public_match {
            let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
            record_boa_violation_once(SandboxViolation::NetAccessDenied {
                tenant_id,
                host: format!("SSRF_BLOCKED:{}", host_port),
            });
            return Err(format!(
                "SSRF_SHIELD_BLOCKED: Raw socket connection to internal/private target '{}' is forbidden under wildcard allow_net",
                host_port
            ));
        }
    }

    Ok(has_explicit_match)
}

pub(super) fn make_socket_url_parts(host: &str, port: u16, is_tls: bool) -> OutboundUrlParts {
    let clean_host = extract_clean_hostname(&host.trim().to_ascii_lowercase());
    let host_port = format_socket_host_port(&clean_host, port);
    OutboundUrlParts {
        is_https: is_tls,
        hostname: clean_host,
        port,
        host_header: host_port.clone(),
        host_port,
        path_and_query: "/".to_string(),
    }
}

pub(super) fn generate_mock_tcp_reply(spec: &str, written: &[u8]) -> Vec<u8> {
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
