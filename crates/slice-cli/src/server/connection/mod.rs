//! Multi-tenant HTTP/1.1 keep-alive, chunked body decoding, SSE, and RFC 6455 WebSocket server.

pub(crate) mod body_reader;
pub(crate) mod keep_alive;
pub(crate) mod websocket;
pub(crate) mod wire;

pub(crate) use keep_alive::serve_connection_with_isolate;
#[cfg(test)]
pub(crate) use slice_sandbox::escape_json_string_into;
#[cfg(test)]
pub(crate) use wire::{
    extract_tenant_route_prefix, find_http_header_end_from, push_u64_ascii, write_all_vectored2,
};

#[cfg(test)]
use anyhow::Result;
use slice_engine::SliceIsolate;
use slice_sandbox::CapabilityToken;
#[cfg(test)]
use std::net::TcpStream;
#[cfg(test)]
use std::path::Path;

pub(crate) fn spawn_worker_isolate(
    slot_id: u32,
    deterministic_seed: Option<u64>,
    caps: &CapabilityToken,
    audit_secrets: &[String],
    fuel_limit: u64,
    mem_limit: usize,
) -> SliceIsolate {
    let mut isolate = SliceIsolate::spawn(
        "tenant-default",
        slot_id,
        caps.clone(),
        fuel_limit,
        mem_limit,
    );
    isolate.set_deterministic_seed(deterministic_seed);
    for (k, v) in &caps.allowed_env_keys {
        if slice_sandbox::is_auto_dlp_env_entry(k, v) {
            isolate.add_egress_audit_secret(v.clone());
        }
    }
    // CLI-SEC-01: Register explicit DLP secrets (`--audit-egress`, `SLICE_AUDIT_EGRESS`, `slice.toml` `audit_egress`, `.env`)
    for sec in audit_secrets {
        if !sec.is_empty() {
            isolate.add_egress_audit_secret(sec.clone());
        }
    }
    isolate
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn serve_connection_keep_alive(
    tcp_stream: TcpStream,
    slot_id: u32,
    bytecode: &[u8],
    tenant_dir_opt: Option<&Path>,
    static_assets_opt: Option<&slice_cas::StaticAssetTable>,
    deterministic_seed: Option<u64>,
    caps: &CapabilityToken,
    fuel_limit: u64,
    mem_limit: usize,
    read_buf: &mut [u8],
    write_buf: &mut Vec<u8>,
    max_requests_tracker: Option<(&std::sync::atomic::AtomicUsize, usize)>,
) -> Result<()> {
    serve_connection_keep_alive_with_tls_and_secrets(
        tcp_stream,
        slot_id,
        bytecode,
        tenant_dir_opt,
        static_assets_opt,
        deterministic_seed,
        caps,
        &[],
        fuel_limit,
        mem_limit,
        read_buf,
        write_buf,
        max_requests_tracker,
        None,
    )
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn serve_connection_keep_alive_with_tls_and_secrets(
    tcp_stream: TcpStream,
    slot_id: u32,
    bytecode: &[u8],
    tenant_dir_opt: Option<&Path>,
    static_assets_opt: Option<&slice_cas::StaticAssetTable>,
    deterministic_seed: Option<u64>,
    caps: &CapabilityToken,
    audit_secrets: &[String],
    fuel_limit: u64,
    mem_limit: usize,
    read_buf: &mut [u8],
    write_buf: &mut Vec<u8>,
    max_requests_tracker: Option<(&std::sync::atomic::AtomicUsize, usize)>,
    tls_server_cfg: Option<&std::sync::Arc<rustls::ServerConfig>>,
) -> Result<()> {
    let mut isolate = spawn_worker_isolate(
        slot_id,
        deterministic_seed,
        caps,
        audit_secrets,
        fuel_limit,
        mem_limit,
    );
    serve_connection_with_isolate(
        tcp_stream,
        slot_id,
        &mut isolate,
        bytecode,
        None,
        tenant_dir_opt,
        static_assets_opt,
        fuel_limit,
        mem_limit,
        read_buf,
        write_buf,
        max_requests_tracker,
        tls_server_cfg,
    )
}
