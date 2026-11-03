//! Outbound `fetch()` HTTP/1.1 & TLS 1.3 (`rustls`) Engine with Keep-Alive Connection Pooling,
//! Pre-Connect DNS Resolved-IP SSRF Firewall, Zero-Trust Hop-by-Hop Redirect Guard, and Egress DLP.

use crate::boa_realm::{
    BOA_ACTIVE_TENANT, BOA_EGRESS_SECRETS, BOA_KV_MEM_BUDGET, BOA_NET_HOSTS, BOA_VIOLATION,
};
use crate::state_and_kv::verify_egress_secrets_slice;
use serde::Serialize;
use slice_sandbox::SandboxViolation;
use std::cell::Cell;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const MAX_REDIRECT_HOPS: usize = 5;
const MAX_IDLE_CONNS_GLOBAL: usize = 32;
const IDLE_CONN_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum allowed size of outbound HTTP response headers (`64 KB`).
pub const MAX_OUTBOUND_HEADER_BYTES: usize = 64 * 1024;
/// Maximum allowed size of an outbound HTTP response body (`16 MB`), further bounded by active tenant memory budget.
pub const MAX_OUTBOUND_BODY_BYTES: usize = 16 * 1024 * 1024;

/// Compile-time lookup table for RFC 9110 Section 5.6.2 `tchar` token characters.
const HTTP_TCHAR_TABLE: [bool; 256] = {
    let mut t = [false; 256];
    let mut c = 0usize;
    while c < 256 {
        let b = c as u8;
        t[c] = matches!(
            b,
            b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+' | b'-' | b'.' | b'^' | b'_'
                | b'`' | b'|' | b'~' | b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z'
        );
        c += 1;
    }
    t
};

/// Validates an HTTP header field name against RFC 9110 `tchar` rules (blocks `\r`, `\n`, `\0`, `:`, spaces, etc.).
#[inline]
pub fn is_valid_http_header_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| HTTP_TCHAR_TABLE[b as usize])
}

/// Validates an HTTP header field value (blocks `\r`, `\n`, `\0`, and ASCII control characters except HTAB `\t`).
#[inline]
pub fn is_valid_http_header_value(val: &str) -> bool {
    val.bytes()
        .all(|b| b == b'\t' || (b >= 0x20 && b != 0x7F))
}

/// Validates an HTTP request target (`path_and_query`) to prevent request line splitting via spaces or control chars.
#[inline]
pub fn is_valid_request_target(target: &str) -> bool {
    !target.is_empty() && target.bytes().all(|b| b > 0x20 && b != 0x7F)
}

#[derive(Debug, Clone)]
pub(crate) struct OutboundUrlParts {
    pub(crate) is_https: bool,
    pub(crate) hostname: String,
    pub(crate) port: u16,
    pub(crate) host_header: String,
    pub(crate) host_port: String,
    pub(crate) path_and_query: String,
}

static DEFAULT_TLS_CLIENT_CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
static HAS_CUSTOM_ROOTS: AtomicBool = AtomicBool::new(false);
static CUSTOM_ROOT_CERTS: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

/// Registers an additional trusted root CA certificate (in DER format) for outbound HTTPS `fetch()`.
/// Used by enterprise internal PKI deployments and by the offline local TLS 1.3 test suite.
pub fn register_custom_root_cert_der(cert_der: Vec<u8>) {
    if let Ok(mut guard) = CUSTOM_ROOT_CERTS.lock() {
        guard.push(cert_der);
        HAS_CUSTOM_ROOTS.store(true, Ordering::Release);
    }
}

fn build_tls_client_config(extra_ders: &[Vec<u8>]) -> Arc<rustls::ClientConfig> {
    let mut root_store = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    for der in extra_ders {
        let cert = rustls_pki_types::CertificateDer::from(der.clone());
        let _ = root_store.add(cert);
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut cfg = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("valid TLS protocol versions")
        .with_root_certificates(root_store)
        .with_no_client_auth();
    cfg.resumption = rustls::client::Resumption::in_memory_sessions(256);
    Arc::new(cfg)
}

#[inline]
pub(crate) fn get_tls_client_config() -> Arc<rustls::ClientConfig> {
    if !HAS_CUSTOM_ROOTS.load(Ordering::Acquire) {
        return Arc::clone(DEFAULT_TLS_CLIENT_CONFIG.get_or_init(|| build_tls_client_config(&[])));
    }
    let extra = CUSTOM_ROOT_CERTS
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    build_tls_client_config(&extra)
}

pub(crate) enum OutboundStream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl OutboundStream {
    pub(crate) fn set_read_timeout(&mut self, dur: Option<Duration>) {
        match self {
            OutboundStream::Plain(s) => {
                let _ = s.set_read_timeout(dur);
            }
            OutboundStream::Tls(s) => {
                let _ = s.get_ref().set_read_timeout(dur);
            }
        }
    }
}

impl Read for OutboundStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            OutboundStream::Plain(s) => s.read(buf),
            OutboundStream::Tls(s) => s.read(buf),
        }
    }
}

impl Write for OutboundStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            OutboundStream::Plain(s) => s.write(buf),
            OutboundStream::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            OutboundStream::Plain(s) => s.flush(),
            OutboundStream::Tls(s) => s.flush(),
        }
    }
}

struct PooledOutboundConn {
    tenant_id: String,
    is_https: bool,
    host_port: String,
    stream: OutboundStream,
    last_used: Instant,
}

static SHARED_OUTBOUND_CONN_POOL: OnceLock<Mutex<Vec<PooledOutboundConn>>> = OnceLock::new();

thread_local! {
    static OUTBOUND_POOL_REUSE_COUNT: Cell<u64> = const { Cell::new(0) };
}

fn shared_conn_pool() -> &'static Mutex<Vec<PooledOutboundConn>> {
    SHARED_OUTBOUND_CONN_POOL.get_or_init(|| Mutex::new(Vec::with_capacity(MAX_IDLE_CONNS_GLOBAL)))
}

/// Returns the number of times the current isolate/worker reused a warm Keep-Alive TCP/TLS stream
/// from `SHARED_OUTBOUND_CONN_POOL` instead of performing a new TCP + TLS handshake.
pub fn outbound_pool_reuse_count() -> u64 {
    OUTBOUND_POOL_REUSE_COUNT.with(|c| c.get())
}

pub(crate) fn record_outbound_pool_reuses(delta: u64) {
    if delta > 0 {
        OUTBOUND_POOL_REUSE_COUNT.with(|c| c.set(c.get() + delta));
    }
}

/// Clears all idle connections in the outbound Keep-Alive pool.
pub fn clear_outbound_conn_pool() {
    if let Ok(mut vec) = shared_conn_pool().lock() {
        vec.clear();
    }
}

fn current_tenant_id() -> String {
    BOA_ACTIVE_TENANT.with(|t| {
        let b = t.borrow();
        if b.is_empty() {
            "boa-tenant".to_string()
        } else {
            b.clone()
        }
    })
}

fn take_pooled_stream(tenant_id: &str, is_https: bool, host_port: &str) -> Option<OutboundStream> {
    let now = Instant::now();
    if let Ok(mut vec) = shared_conn_pool().lock() {
        vec.retain(|entry| now.duration_since(entry.last_used) < IDLE_CONN_TIMEOUT);
        if let Some(pos) = vec.iter().rposition(|e| {
            e.tenant_id == tenant_id && e.is_https == is_https && e.host_port == host_port
        }) {
            let entry = vec.swap_remove(pos);
            return Some(entry.stream);
        }
    }
    None
}

fn return_pooled_stream(
    tenant_id: String,
    is_https: bool,
    host_port: String,
    stream: OutboundStream,
) {
    let now = Instant::now();
    if let Ok(mut vec) = shared_conn_pool().lock() {
        vec.retain(|entry| now.duration_since(entry.last_used) < IDLE_CONN_TIMEOUT);
        if vec.len() >= MAX_IDLE_CONNS_GLOBAL {
            vec.remove(0);
        }
        vec.push(PooledOutboundConn {
            tenant_id,
            is_https,
            host_port,
            stream,
            last_used: now,
        });
    }
}

pub(crate) fn parse_outbound_http_url(raw_url: &str) -> Option<OutboundUrlParts> {
    let trimmed = raw_url.trim();
    let without_hash = trimmed.split('#').next().unwrap_or(trimmed);
    let (is_https, default_port, rest) = if let Some(r) = without_hash.strip_prefix("http://") {
        (false, 80u16, r)
    } else if let Some(r) = without_hash.strip_prefix("https://") {
        (true, 443u16, r)
    } else {
        return None;
    };
    let (authority, path_and_query) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };
    let host_port_raw = match authority.rfind('@') {
        Some(idx) => &authority[idx + 1..],
        None => authority,
    };
    let (hostname, port) = if let Some(ipv6_rest) = host_port_raw.strip_prefix('[') {
        let (ipv6, after_bracket) = ipv6_rest.split_once(']')?;
        let p = after_bracket
            .strip_prefix(':')
            .and_then(|ps| ps.parse::<u16>().ok())
            .unwrap_or(default_port);
        (format!("[{}]", ipv6.to_ascii_lowercase()), p)
    } else if let Some((h, p_str)) = host_port_raw.rsplit_once(':') {
        let p = p_str.parse::<u16>().ok()?;
        (h.to_ascii_lowercase(), p)
    } else {
        (host_port_raw.to_ascii_lowercase(), default_port)
    };
    if hostname.is_empty() {
        return None;
    }
    let clean_host = slice_sandbox::extract_clean_hostname(&hostname);
    let host_port = format!("{}:{}", clean_host, port);
    let host_header = if port == default_port {
        hostname
    } else {
        format!("{}:{}", hostname, port)
    };
    Some(OutboundUrlParts {
        is_https,
        hostname: clean_host,
        port,
        host_header,
        host_port,
        path_and_query: path_and_query.to_string(),
    })
}

fn resolve_redirect_url(
    base_url: &str,
    base_parts: &OutboundUrlParts,
    location: &str,
) -> Option<String> {
    let loc = location.trim();
    if loc.is_empty() {
        return None;
    }
    if loc.starts_with("http://") || loc.starts_with("https://") {
        return Some(loc.to_string());
    }
    let scheme = if base_parts.is_https {
        "https"
    } else {
        "http"
    };
    if let Some(scheme_rel) = loc.strip_prefix("//") {
        return Some(format!("{}:{}", scheme, scheme_rel));
    }
    let origin = format!("{}://{}", scheme, base_parts.host_header);
    if loc.starts_with('/') {
        return Some(format!("{}{}", origin, loc));
    }
    let base_path = base_parts
        .path_and_query
        .split('?')
        .next()
        .unwrap_or("/");
    let dir = match base_path.rfind('/') {
        Some(idx) => &base_path[..=idx],
        None => "/",
    };
    let _ = base_url;
    Some(format!("{}{}{}", origin, dir, loc))
}

/// Checks OCap host allowlist, Cloud Metadata SSRF hostname shield, and Outbound Egress Secret DLP.
/// Returns `Ok(is_explicitly_allowlisted_host)` where `true` indicates the hostname/host_port was
/// explicitly named in `allow_net` (rather than only matching wildcard `"*"`).
pub(crate) fn check_boa_outbound_fetch_ocap_and_dlp(
    parts: &OutboundUrlParts,
    raw_url: &str,
    headers_json: &str,
    body: &str,
) -> Result<bool, String> {
    let tenant_id = BOA_ACTIVE_TENANT.with(|t| {
        let b = t.borrow();
        if b.is_empty() {
            "boa-tenant".to_string()
        } else {
            b.clone()
        }
    });

    // 1. SSRF Cloud Metadata Shield (blocks 169.254.169.254, metadata.google.internal, etc. unconditionally)
    if slice_sandbox::is_cloud_metadata_ssrf_target(&parts.hostname) {
        BOA_VIOLATION.with(|v| {
            *v.borrow_mut() = Some(SandboxViolation::NetAccessDenied {
                tenant_id,
                host: format!("SSRF_METADATA_BLOCKED:{}", parts.host_port),
            });
        });
        return Err("NetAccessDenied: SSRF Cloud Metadata Endpoint Blocked".to_string());
    }

    // 2. OCap Network Host Allowlist Verification
    let (permitted, explicit_host_grant) = BOA_NET_HOSTS.with(|hosts_cell| {
        let hosts = hosts_cell.borrow();
        if hosts.contains(&parts.host_port)
            || hosts.contains(&parts.hostname)
            || hosts.contains(&format!("{}:443", parts.hostname))
            || hosts.contains(&format!("{}:80", parts.hostname))
        {
            return (true, true);
        }
        if hosts.contains("*")
            && !slice_sandbox::is_private_or_loopback_ssrf_target(&parts.hostname)
        {
            return (true, false);
        }
        (false, false)
    });

    if !permitted {
        BOA_VIOLATION.with(|v| {
            *v.borrow_mut() = Some(SandboxViolation::NetAccessDenied {
                tenant_id,
                host: parts.host_port.clone(),
            });
        });
        return Err(format!("NetAccessDenied: {}", parts.host_port));
    }

    // 3. Outbound Egress Secret DLP Inspection (scans URL query, outbound headers, and body for raw/Base64/Base64URL secrets)
    let dlp_err = BOA_EGRESS_SECRETS.with(|sec_cell| {
        let secrets = sec_cell.borrow();
        if secrets.is_empty() {
            return None;
        }
        verify_egress_secrets_slice(&tenant_id, &secrets, raw_url)
            .and_then(|()| verify_egress_secrets_slice(&tenant_id, &secrets, headers_json))
            .and_then(|()| verify_egress_secrets_slice(&tenant_id, &secrets, body))
            .err()
    });

    if let Some(violation) = dlp_err {
        BOA_VIOLATION.with(|v| *v.borrow_mut() = Some(violation));
        return Err("EnvAccessDenied: EGRESS_SECRET_LEAK_BLOCKED".to_string());
    }

    Ok(explicit_host_grant)
}

/// Resolves DNS for `parts` and validates every resolved `SocketAddr` against the Pre-Connect
/// Resolved-IP SSRF Firewall (preventing DNS rebinding attacks to `169.254.169.254` or `127.0.0.1`).
pub(crate) fn resolve_and_verify_ssrf_socket_addrs(
    parts: &OutboundUrlParts,
    explicit_host_grant: bool,
) -> Result<Vec<SocketAddr>, String> {
    let tenant_id = current_tenant_id();

    let mut addrs: Vec<SocketAddr> = (parts.hostname.as_str(), parts.port)
        .to_socket_addrs()
        .map_err(|e| format!("DNS resolution failed for {}: {}", parts.host_port, e))?
        .collect();

    if addrs.is_empty() {
        return Err(format!("No socket address resolved for {}", parts.host_port));
    }

    for addr in &addrs {
        let ip = addr.ip();
        if slice_sandbox::is_cloud_metadata_ip(ip) {
            BOA_VIOLATION.with(|v| {
                *v.borrow_mut() = Some(SandboxViolation::NetAccessDenied {
                    tenant_id,
                    host: format!("SSRF_METADATA_IP_BLOCKED:{}:{}", parts.host_port, ip),
                });
            });
            return Err(format!(
                "NetAccessDenied: SSRF Cloud Metadata IP Blocked ({})",
                ip
            ));
        }
        if !explicit_host_grant && slice_sandbox::is_private_or_loopback_ip(ip) {
            BOA_VIOLATION.with(|v| {
                *v.borrow_mut() = Some(SandboxViolation::NetAccessDenied {
                    tenant_id,
                    host: format!("SSRF_PRIVATE_IP_BLOCKED:{}:{}", parts.host_port, ip),
                });
            });
            return Err(format!(
                "NetAccessDenied: DNS Rebinding / Private IP Blocked under wildcard allow_net ({})",
                ip
            ));
        }
    }

    // Prefer IPv4 addresses ahead of IPv6 for dual-stack hostnames (such as `localhost` on Windows)
    addrs.sort_by_key(|a| if a.is_ipv4() { 0u8 } else { 1u8 });
    Ok(addrs)
}

/// Strictly parses an HTTP/1.1 `Transfer-Encoding: chunked` body with checked arithmetic (`N-Sec-5`),
/// strict hex length validation (`1..=16` hex digits), and bounded decoded payload size.
/// Returns:
/// - `Ok(None)` if `raw` does not yet contain the complete terminating `0\r\n...\r\n` chunk.
/// - `Ok(Some((decoded_bytes, consumed_raw_bytes)))` when the complete chunked stream is decoded.
/// - `Err(io::Error)` (`InvalidData` or `OutOfMemory`) on malformed chunk headers or size overflow.
pub fn parse_chunked_body(
    raw: &[u8],
    max_decoded_bytes: usize,
) -> Result<Option<(Vec<u8>, usize)>, std::io::Error> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < raw.len() {
        let Some(rel_end) = raw[pos..].windows(2).position(|w| w == b"\r\n") else {
            if raw.len() - pos > 1024 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "chunk header line exceeds 1024 bytes without CRLF",
                ));
            }
            return Ok(None);
        };
        let size_line = std::str::from_utf8(&raw[pos..pos + rel_end]).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid UTF-8 in chunk size line",
            )
        })?;
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        if size_hex.is_empty()
            || size_hex.len() > 16
            || !size_hex.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "malformed hexadecimal chunk size",
            ));
        }
        let chunk_len = usize::from_str_radix(size_hex, 16).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "chunk size overflow")
        })?;
        let data_start = pos
            .checked_add(rel_end)
            .and_then(|v| v.checked_add(2))
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "chunk offset overflow")
            })?;

        if chunk_len == 0 {
            // Final `0\r\n` chunk; wait for trailing `\r\n` (or optional trailers + `\r\n\r\n`)
            if raw.len() < data_start.saturating_add(2) {
                return Ok(None);
            }
            if &raw[data_start..data_start + 2] == b"\r\n" {
                return Ok(Some((out, data_start + 2)));
            }
            if let Some(trailer_end) = raw[data_start..].windows(4).position(|w| w == b"\r\n\r\n") {
                return Ok(Some((out, data_start + trailer_end + 4)));
            }
            return Ok(None);
        }

        let new_decoded_len = out.len().checked_add(chunk_len).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "decoded chunked total overflow",
            )
        })?;
        if new_decoded_len > max_decoded_bytes {
            return Err(std::io::Error::new(
                std::io::ErrorKind::OutOfMemory,
                "chunked HTTP body exceeded maximum memory budget",
            ));
        }

        let data_end = data_start.checked_add(chunk_len).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "chunk data_end overflow")
        })?;
        let frame_end = data_end.checked_add(2).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "chunk frame_end overflow")
        })?;

        if raw.len() < frame_end {
            return Ok(None);
        }
        if &raw[data_end..frame_end] != b"\r\n" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "missing CRLF after chunk payload",
            ));
        }

        out.extend_from_slice(&raw[data_start..data_end]);
        pos = frame_end;
    }
    Ok(None)
}

pub(crate) fn dial_fresh_outbound_stream(
    parts: &OutboundUrlParts,
    verified_addrs: &[SocketAddr],
) -> Result<OutboundStream, String> {
    let mut last_err = None;
    let mut tcp_opt = None;
    for addr in verified_addrs {
        match TcpStream::connect_timeout(addr, Duration::from_secs(5)) {
            Ok(s) => {
                tcp_opt = Some(s);
                break;
            }
            Err(e) => last_err = Some(e),
        }
    }
    let tcp = tcp_opt.ok_or_else(|| {
        format!(
            "TCP connect failed to {}: {}",
            parts.host_port,
            last_err
                .map(|e| e.to_string())
                .unwrap_or_else(|| "no addresses".to_string())
        )
    })?;
    let _ = tcp.set_nodelay(true);
    let _ = tcp.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = tcp.set_write_timeout(Some(Duration::from_secs(5)));

    if !parts.is_https {
        return Ok(OutboundStream::Plain(tcp));
    }

    let tls_cfg = get_tls_client_config();
    let sni_host = parts
        .hostname
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let server_name = rustls_pki_types::ServerName::try_from(sni_host)
        .map_err(|e| format!("Invalid TLS SNI hostname '{}': {}", parts.hostname, e))?;
    let client_conn = rustls::ClientConnection::new(tls_cfg, server_name)
        .map_err(|e| format!("TLS initialization failed for {}: {}", parts.host_port, e))?;
    Ok(OutboundStream::Tls(Box::new(rustls::StreamOwned::new(
        client_conn,
        tcp,
    ))))
}

struct RawHttpResponse {
    status: u16,
    status_text: String,
    headers: Vec<(String, String)>,
    body: String,
    peer_closed: bool,
}

fn find_header_delimiter(buf: &[u8]) -> Option<(usize, usize)> {
    buf.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| (i, i + 4))
        .or_else(|| {
            buf.windows(2)
                .position(|w| w == b"\n\n")
                .map(|i| (i, i + 2))
        })
}

fn trap_outbound_memory_exceeded(requested: usize, limit: usize) -> std::io::Error {
    let tenant_id = current_tenant_id();
    BOA_VIOLATION.with(|v| {
        *v.borrow_mut() = Some(SandboxViolation::MemoryCeilingExceeded {
            tenant_id,
            requested,
            limit,
        });
    });
    std::io::Error::new(
        std::io::ErrorKind::OutOfMemory,
        format!(
            "MemoryCeilingExceeded: outbound HTTP response ({} bytes) exceeded budget ({} bytes)",
            requested, limit
        ),
    )
}

fn send_and_read_framed_http11(
    stream: &mut OutboundStream,
    req_buf: &[u8],
    method: &str,
) -> Result<RawHttpResponse, std::io::Error> {
    stream.write_all(req_buf)?;
    stream.flush()?;

    let effective_max_body = BOA_KV_MEM_BUDGET.with(|m| {
        let (used, limit) = m.get();
        limit.saturating_sub(used).min(MAX_OUTBOUND_BODY_BYTES)
    });

    let mut raw_resp = Vec::with_capacity(4096);
    let mut scratch = [0u8; 4096];

    // 1. Read until end of HTTP response headers (`\r\n\r\n`), capped at `MAX_OUTBOUND_HEADER_BYTES` (64 KB)
    let (hdr_end, body_start) = loop {
        if let Some(found) = find_header_delimiter(&raw_resp) {
            if found.0 > MAX_OUTBOUND_HEADER_BYTES {
                return Err(trap_outbound_memory_exceeded(
                    found.0,
                    MAX_OUTBOUND_HEADER_BYTES,
                ));
            }
            break found;
        }
        if raw_resp.len() > MAX_OUTBOUND_HEADER_BYTES {
            return Err(trap_outbound_memory_exceeded(
                raw_resp.len(),
                MAX_OUTBOUND_HEADER_BYTES,
            ));
        }
        let n = stream.read(&mut scratch)?;
        if n == 0 {
            if raw_resp.is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "peer closed connection before sending HTTP response headers",
                ));
            }
            break (raw_resp.len(), raw_resp.len());
        }
        raw_resp.extend_from_slice(&scratch[..n]);
    };

    let header_str = std::str::from_utf8(&raw_resp[..hdr_end]).unwrap_or("");
    let mut lines = header_str.lines();
    let status_line = lines.next().unwrap_or("HTTP/1.1 200 OK");
    let mut status_parts = status_line.splitn(3, ' ');
    let http_ver = status_parts.next().unwrap_or("HTTP/1.1");
    let status_code: u16 = status_parts
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    let status_text = status_parts.next().unwrap_or("OK").trim().to_string();

    let mut resp_headers: Vec<(String, String)> = Vec::new();
    let mut is_chunked = false;
    let mut content_length: Option<usize> = None;
    let mut peer_closed = http_ver.ends_with("1.0");

    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_ascii_lowercase();
            let val = v.trim().to_string();
            let val_lower = val.to_ascii_lowercase();
            if key == "transfer-encoding" && val_lower.contains("chunked") {
                is_chunked = true;
            } else if key == "content-length" {
                if let Ok(len) = val.trim().parse::<usize>() {
                    content_length = Some(len);
                }
            } else if key == "connection" {
                if val_lower.contains("close") {
                    peer_closed = true;
                } else if val_lower.contains("keep-alive") {
                    peer_closed = false;
                }
            }
            resp_headers.push((key, val));
        }
    }

    // 2. Frame the response body with strict `effective_max_body` ceiling enforcement (`N-Sec-1` & `N-Sec-5`)
    let no_body_status = method == "HEAD" || matches!(status_code, 100..=199 | 204 | 304);
    let decoded_body = if no_body_status {
        String::new()
    } else if is_chunked {
        loop {
            match parse_chunked_body(&raw_resp[body_start..], effective_max_body) {
                Ok(Some((decoded, _consumed))) => {
                    break String::from_utf8_lossy(&decoded).into_owned();
                }
                Ok(None) => {
                    if raw_resp.len().saturating_sub(body_start) > effective_max_body.saturating_add(65536) {
                        return Err(trap_outbound_memory_exceeded(
                            raw_resp.len().saturating_sub(body_start),
                            effective_max_body,
                        ));
                    }
                    let n = stream.read(&mut scratch)?;
                    if n == 0 {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "truncated chunked HTTP response body",
                        ));
                    }
                    raw_resp.extend_from_slice(&scratch[..n]);
                }
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::OutOfMemory {
                        return Err(trap_outbound_memory_exceeded(
                            effective_max_body.saturating_add(1),
                            effective_max_body,
                        ));
                    }
                    return Err(e);
                }
            }
        }
    } else if let Some(expected_len) = content_length {
        if expected_len > effective_max_body {
            return Err(trap_outbound_memory_exceeded(
                expected_len,
                effective_max_body,
            ));
        }
        while raw_resp.len().saturating_sub(body_start) < expected_len {
            let remaining = expected_len - (raw_resp.len() - body_start);
            let to_read = remaining.min(scratch.len());
            let n = stream.read(&mut scratch[..to_read])?;
            if n == 0 {
                peer_closed = true;
                break;
            }
            raw_resp.extend_from_slice(&scratch[..n]);
        }
        let end_idx = (body_start + expected_len).min(raw_resp.len());
        String::from_utf8_lossy(&raw_resp[body_start..end_idx]).into_owned()
    } else {
        peer_closed = true;
        loop {
            let current_body_len = raw_resp.len().saturating_sub(body_start);
            if current_body_len > effective_max_body {
                return Err(trap_outbound_memory_exceeded(
                    current_body_len,
                    effective_max_body,
                ));
            }
            let n = stream.read(&mut scratch)?;
            if n == 0 {
                break;
            }
            if current_body_len.saturating_add(n) > effective_max_body {
                return Err(trap_outbound_memory_exceeded(
                    current_body_len.saturating_add(n),
                    effective_max_body,
                ));
            }
            raw_resp.extend_from_slice(&scratch[..n]);
        }
        String::from_utf8_lossy(&raw_resp[body_start..]).into_owned()
    };

    Ok(RawHttpResponse {
        status: status_code,
        status_text,
        headers: resp_headers,
        body: decoded_body,
        peer_closed,
    })
}

fn execute_single_http11_hop(
    parts: &OutboundUrlParts,
    verified_addrs: &[SocketAddr],
    method: &str,
    req_headers: &[(String, String)],
    body: &str,
) -> Result<RawHttpResponse, String> {
    // N-Sec-2: Strictly validate method, request target, Host header, and all custom headers against CRLF / control char injection
    if !is_valid_http_header_name(method) {
        return Err(format!("InvalidMethod: '{}' contains invalid token characters", method));
    }
    if !is_valid_request_target(&parts.path_and_query) {
        return Err("InvalidRequestTarget: URL path/query contains forbidden whitespace or control characters".to_string());
    }
    if !is_valid_http_header_value(&parts.host_header) {
        return Err("InvalidHostHeader: Host contains forbidden control characters".to_string());
    }
    for (k, v) in req_headers {
        if !is_valid_http_header_name(k) || !is_valid_http_header_value(v) {
            return Err(format!(
                "InvalidHeader: CRLF or invalid token character blocked in header '{}'",
                k
            ));
        }
    }

    let mut req_buf = Vec::with_capacity(256 + body.len());
    let _ = write!(
        &mut req_buf,
        "{} {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: Slice-NexusJS/{}\r\nConnection: keep-alive\r\n",
        method,
        parts.path_and_query,
        parts.host_header,
        env!("CARGO_PKG_VERSION")
    );
    for (k, v) in req_headers {
        if !k.eq_ignore_ascii_case("host")
            && !k.eq_ignore_ascii_case("connection")
            && !k.eq_ignore_ascii_case("content-length")
        {
            let _ = write!(&mut req_buf, "{}: {}\r\n", k, v);
        }
    }
    if !body.is_empty() || matches!(method, "POST" | "PUT" | "PATCH") {
        let _ = write!(&mut req_buf, "Content-Length: {}\r\n\r\n", body.len());
        req_buf.extend_from_slice(body.as_bytes());
    } else {
        req_buf.extend_from_slice(b"\r\n");
    }

    let tenant_id = current_tenant_id();

    // 1. Try tenant-scoped warm Keep-Alive stream from `SHARED_OUTBOUND_CONN_POOL` first (`N-Sec-7`)
    if let Some(mut pooled_stream) = take_pooled_stream(&tenant_id, parts.is_https, &parts.host_port) {
        if let Ok(resp) = send_and_read_framed_http11(&mut pooled_stream, &req_buf, method) {
            OUTBOUND_POOL_REUSE_COUNT.with(|c| c.set(c.get() + 1));
            if !resp.peer_closed {
                return_pooled_stream(tenant_id, parts.is_https, parts.host_port.clone(), pooled_stream);
            }
            return Ok(resp);
        }
        // Stale idle connection closed by remote peer; fall through and dial fresh stream
    }

    // 2. Dial fresh TCP (+ TLS 1.3 if `parts.is_https`) stream to the pin-verified SocketAddrs
    let mut fresh_stream = dial_fresh_outbound_stream(parts, verified_addrs)?;
    let resp = send_and_read_framed_http11(&mut fresh_stream, &req_buf, method)
        .map_err(|e| format!("HTTP/1.1 exchange failed with {}: {}", parts.host_port, e))?;
    if !resp.peer_closed {
        return_pooled_stream(tenant_id, parts.is_https, parts.host_port.clone(), fresh_stream);
    }
    Ok(resp)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireFetchResponse {
    status: u16,
    status_text: String,
    headers: Vec<(String, String)>,
    body: String,
    final_url: String,
    redirected: bool,
}

/// Executes an outbound HTTP/1.1 or HTTPS (TLS 1.3/1.2) `fetch()` request with:
/// - Zero-Trust Hop-by-Hop Redirect Guard (`301`/`302`/`303`/`307`/`308`, up to 5 hops)
/// - Per-hop OCap host allowlist + Pre-Connect DNS Resolved-IP SSRF Firewall + Egress DLP
/// - Cross-origin redirect credential stripping (`Authorization`, `Cookie`, `Proxy-Authorization`)
/// - Thread-local Keep-Alive connection pooling (`OUTBOUND_CONN_POOL`)
pub(crate) fn execute_outbound_fetch_with_redirects(
    initial_url: &str,
    initial_method: &str,
    initial_headers: &[(String, String)],
    initial_body: &str,
    redirect_mode: &str,
) -> Result<String, String> {
    let mut current_url = initial_url.to_string();
    let mut current_method = initial_method.to_string();
    let mut current_headers: Vec<(String, String)> = initial_headers.to_vec();
    let mut current_body = initial_body.to_string();
    let mut redirected = false;

    for hop in 0..=MAX_REDIRECT_HOPS {
        let parts = parse_outbound_http_url(&current_url)
            .ok_or_else(|| format!("Invalid outbound URL: {}", current_url))?;
        let headers_json =
            serde_json::to_string(&current_headers).unwrap_or_else(|_| "[]".to_string());
        let explicit_host_grant = check_boa_outbound_fetch_ocap_and_dlp(
            &parts,
            &current_url,
            &headers_json,
            &current_body,
        )?;
        let verified_addrs = resolve_and_verify_ssrf_socket_addrs(&parts, explicit_host_grant)?;

        let resp = execute_single_http11_hop(
            &parts,
            &verified_addrs,
            &current_method,
            &current_headers,
            &current_body,
        )?;

        let is_redirect = matches!(resp.status, 301 | 302 | 303 | 307 | 308);
        if is_redirect {
            let location_opt = resp
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("location"))
                .map(|(_, v)| v.clone());

            if let Some(location) = location_opt {
                if redirect_mode == "manual" {
                    return serde_json::to_string(&WireFetchResponse {
                        status: resp.status,
                        status_text: resp.status_text,
                        headers: resp.headers,
                        body: resp.body,
                        final_url: current_url,
                        redirected,
                    })
                    .map_err(|e| e.to_string());
                }
                if redirect_mode == "error" {
                    return Err(format!(
                        "RedirectError: fetch redirect mode is 'error' (received {} -> {})",
                        resp.status, location
                    ));
                }
                if hop == MAX_REDIRECT_HOPS {
                    return Err(format!(
                        "RedirectError: maximum redirect hops ({}) exceeded",
                        MAX_REDIRECT_HOPS
                    ));
                }

                let next_url = resolve_redirect_url(&current_url, &parts, &location)
                    .ok_or_else(|| format!("Invalid redirect Location header: {}", location))?;
                let next_parts = parse_outbound_http_url(&next_url)
                    .ok_or_else(|| format!("Invalid redirect target URL: {}", next_url))?;

                // Strip sensitive credentials when redirecting across origins (scheme, host, or port)
                if next_parts.is_https != parts.is_https || next_parts.host_port != parts.host_port
                {
                    current_headers.retain(|(k, _)| {
                        !k.eq_ignore_ascii_case("authorization")
                            && !k.eq_ignore_ascii_case("cookie")
                            && !k.eq_ignore_ascii_case("proxy-authorization")
                    });
                }

                // Per WHATWG Fetch specification: 303 or (301/302 with POST) rewrites method to GET and drops body
                if resp.status == 303
                    || ((resp.status == 301 || resp.status == 302) && current_method == "POST")
                {
                    current_method = "GET".to_string();
                    current_body.clear();
                    current_headers.retain(|(k, _)| {
                        !k.eq_ignore_ascii_case("content-type")
                            && !k.eq_ignore_ascii_case("content-length")
                    });
                }

                current_url = next_url;
                redirected = true;
                continue;
            }
        }

        return serde_json::to_string(&WireFetchResponse {
            status: resp.status,
            status_text: resp.status_text,
            headers: resp.headers,
            body: resp.body,
            final_url: current_url,
            redirected,
        })
        .map_err(|e| e.to_string());
    }

    Err("RedirectError: redirect loop terminated".to_string())
}
