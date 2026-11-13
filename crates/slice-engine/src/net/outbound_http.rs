//! Outbound `fetch()` HTTP/1.1 & TLS 1.3 (`rustls`) Engine with Keep-Alive Connection Pooling,
//! Pre-Connect DNS Resolved-IP SSRF Firewall, Zero-Trust Hop-by-Hop Redirect Guard, and Egress DLP.

use crate::boa_realm::{
    BOA_ACTIVE_TENANT, BOA_EGRESS_SECRETS, BOA_ENV_MAP, BOA_KV_MEM_BUDGET, BOA_NET_HOSTS,
    BOA_VIOLATION,
};
use crate::state_and_kv::verify_egress_secrets_slice;
use rustc_hash::FxHashMap;
use serde::Serialize;
use slice_sandbox::SandboxViolation;
use std::cell::Cell;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const MAX_REDIRECT_HOPS: usize = 5;
const MAX_IDLE_CONNS_PER_TENANT: usize = 16;
const MAX_TENANT_CONN_POOLS: usize = 256;
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
    val.bytes().all(|b| b == b'\t' || (b >= 0x20 && b != 0x7F))
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

pub(crate) type ProxyTlsStream = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;

pub(crate) enum OutboundStream {
    Plain(TcpStream),
    Tls(Box<ProxyTlsStream>),
    TunneledTls(Box<rustls::StreamOwned<rustls::ClientConnection, Box<ProxyTlsStream>>>),
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
            OutboundStream::TunneledTls(s) => {
                let _ = s.get_ref().get_ref().set_read_timeout(dur);
            }
        }
    }
}

impl Read for OutboundStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            OutboundStream::Plain(s) => s.read(buf),
            OutboundStream::Tls(s) => s.read(buf),
            OutboundStream::TunneledTls(s) => s.read(buf),
        }
    }
}

impl Write for OutboundStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            OutboundStream::Plain(s) => s.write(buf),
            OutboundStream::Tls(s) => s.write(buf),
            OutboundStream::TunneledTls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            OutboundStream::Plain(s) => s.flush(),
            OutboundStream::Tls(s) => s.flush(),
            OutboundStream::TunneledTls(s) => s.flush(),
        }
    }
}

struct PooledOutboundConn {
    proxy_key: String,
    is_https: bool,
    host_port: String,
    stream: OutboundStream,
    last_used: Instant,
}

static SHARED_OUTBOUND_CONN_POOL: OnceLock<Mutex<FxHashMap<String, Vec<PooledOutboundConn>>>> =
    OnceLock::new();
static SHARED_FETCH_SLAB_POOL: OnceLock<parking_lot::Mutex<slice_io::RegisteredBufferPool>> =
    OnceLock::new();
static FETCH_SLAB_ACQUIRE_COUNT: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

thread_local! {
    static OUTBOUND_POOL_REUSE_COUNT: Cell<u64> = const { Cell::new(0) };
}

fn shared_fetch_slab_pool() -> &'static parking_lot::Mutex<slice_io::RegisteredBufferPool> {
    SHARED_FETCH_SLAB_POOL.get_or_init(|| {
        parking_lot::Mutex::new(slice_io::RegisteredBufferPool::new(
            slice_io::FIXED_BUFFER_COUNT,
            slice_io::FIXED_BUFFER_SIZE,
        ))
    })
}

/// Returns the total number of times a fixed 64 KB slab was acquired from
/// `slice-io::RegisteredBufferPool` across outbound HTTP `fetch()` operations.
pub fn fetch_reactor_slab_acquire_count() -> u64 {
    FETCH_SLAB_ACQUIRE_COUNT.load(Ordering::Relaxed)
}

/// Returns the number of `slice-io::RegisteredBufferPool` 64 KB slabs currently in use.
/// Should return `0` when all outbound `fetch()` requests have settled (zero slab leaks).
pub fn fetch_reactor_slab_in_use_count() -> usize {
    shared_fetch_slab_pool().lock().in_use_count()
}

/// Returns the number of `slice-io::RegisteredBufferPool` 64 KB slabs currently available (`0..=64`).
pub fn fetch_reactor_slab_available_count() -> usize {
    shared_fetch_slab_pool().lock().available_count()
}

/// Resets the `slice-io::RegisteredBufferPool` slab acquisition telemetry counter.
pub fn reset_fetch_slab_telemetry() {
    FETCH_SLAB_ACQUIRE_COUNT.store(0, Ordering::Relaxed);
}

struct FetchSlabSlotGuard {
    slot_idx: Option<u16>,
}

impl FetchSlabSlotGuard {
    #[inline]
    fn acquire() -> Self {
        let slot_idx = shared_fetch_slab_pool().lock().acquire_slot();
        if slot_idx.is_some() {
            FETCH_SLAB_ACQUIRE_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        Self { slot_idx }
    }

    #[inline]
    fn read_chunk_via_slab(
        &self,
        stream: &mut OutboundStream,
        dest: &mut Vec<u8>,
        max_bytes: usize,
    ) -> std::io::Result<usize> {
        let limit = max_bytes.min(slice_io::FIXED_BUFFER_SIZE);
        if limit == 0 {
            return Ok(0);
        }
        if let Some(idx) = self.slot_idx {
            let mut stack_buf = [0u8; 16384];
            let step = limit.min(stack_buf.len());
            let n = stream.read(&mut stack_buf[..step])?;
            if n > 0 {
                let mut guard = shared_fetch_slab_pool().lock();
                if let Some(slab) = guard.buffer_mut(idx) {
                    slab[..n].copy_from_slice(&stack_buf[..n]);
                    dest.extend_from_slice(&slab[..n]);
                    return Ok(n);
                }
                drop(guard);
                dest.extend_from_slice(&stack_buf[..n]);
            }
            Ok(n)
        } else {
            let mut fallback = [0u8; 4096];
            let step = limit.min(fallback.len());
            let n = stream.read(&mut fallback[..step])?;
            if n > 0 {
                dest.extend_from_slice(&fallback[..n]);
            }
            Ok(n)
        }
    }
}

impl Drop for FetchSlabSlotGuard {
    fn drop(&mut self) {
        if let Some(idx) = self.slot_idx.take() {
            shared_fetch_slab_pool().lock().release(idx);
        }
    }
}

fn shared_conn_pool() -> &'static Mutex<FxHashMap<String, Vec<PooledOutboundConn>>> {
    SHARED_OUTBOUND_CONN_POOL.get_or_init(|| Mutex::new(FxHashMap::default()))
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

/// Clears all idle connections in the outbound Keep-Alive pool across all tenants.
pub fn clear_outbound_conn_pool() {
    if let Ok(mut map) = shared_conn_pool().lock() {
        map.clear();
    }
}

/// Clears all idle connections in the outbound Keep-Alive pool for a specific `tenant_id`
/// without disturbing warm Keep-Alive sockets belonging to other concurrent tenants.
pub fn clear_outbound_conn_pool_for_tenant(tenant_id: &str) {
    if let Ok(mut map) = shared_conn_pool().lock() {
        map.remove(tenant_id);
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

fn take_pooled_stream(
    tenant_id: &str,
    proxy_key: &str,
    is_https: bool,
    host_port: &str,
) -> Option<OutboundStream> {
    let now = Instant::now();
    if let Ok(mut map) = shared_conn_pool().lock() {
        if let Some(vec) = map.get_mut(tenant_id) {
            vec.retain(|entry| now.duration_since(entry.last_used) < IDLE_CONN_TIMEOUT);
            if let Some(pos) = vec.iter().rposition(|e| {
                e.proxy_key == proxy_key && e.is_https == is_https && e.host_port == host_port
            }) {
                let entry = vec.swap_remove(pos);
                return Some(entry.stream);
            }
        }
    }
    None
}

fn return_pooled_stream(
    tenant_id: String,
    proxy_key: String,
    is_https: bool,
    host_port: String,
    stream: OutboundStream,
) {
    let now = Instant::now();
    if let Ok(mut map) = shared_conn_pool().lock() {
        if !map.contains_key(&tenant_id) && map.len() >= MAX_TENANT_CONN_POOLS {
            map.retain(|_, vec| {
                vec.retain(|entry| now.duration_since(entry.last_used) < IDLE_CONN_TIMEOUT);
                !vec.is_empty()
            });
            if map.len() >= MAX_TENANT_CONN_POOLS {
                if let Some(oldest_tenant) = map
                    .iter()
                    .min_by_key(|(_, vec)| vec.last().map(|e| e.last_used).unwrap_or(now))
                    .map(|(k, _)| k.clone())
                {
                    map.remove(&oldest_tenant);
                }
            }
        }
        let vec = map
            .entry(tenant_id)
            .or_insert_with(|| Vec::with_capacity(4));
        vec.retain(|entry| now.duration_since(entry.last_used) < IDLE_CONN_TIMEOUT);
        if vec.len() >= MAX_IDLE_CONNS_PER_TENANT {
            vec.remove(0);
        }
        vec.push(PooledOutboundConn {
            proxy_key,
            is_https,
            host_port,
            stream,
            last_used: now,
        });
    }
}

pub(crate) fn parse_outbound_http_url(raw_url: &str) -> Option<OutboundUrlParts> {
    let trimmed = raw_url.trim();
    let (scheme, _) = trimmed.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let parsed = slice_sandbox::parse_http_authority_and_path(trimmed)?;
    let clean_host = parsed.clean_host.into_owned();
    let bracketed_host = if parsed.is_ipv6 {
        format!("[{}]", clean_host)
    } else {
        clean_host.clone()
    };
    let host_port = format!("{}:{}", bracketed_host, parsed.port);
    let host_header = if parsed.port == parsed.default_port {
        bracketed_host
    } else {
        host_port.clone()
    };
    Some(OutboundUrlParts {
        is_https: parsed.is_secure,
        hostname: clean_host,
        port: parsed.port,
        host_header,
        host_port,
        path_and_query: parsed.path_and_query.into_owned(),
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
    let scheme = if base_parts.is_https { "https" } else { "http" };
    if let Some(scheme_rel) = loc.strip_prefix("//") {
        return Some(format!("{}:{}", scheme, scheme_rel));
    }
    let origin = format!("{}://{}", scheme, base_parts.host_header);
    if loc.starts_with('/') {
        return Some(format!("{}{}", origin, loc));
    }
    let base_path = base_parts.path_and_query.split('?').next().unwrap_or("/");
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

    // 2. OCap Network Host Allowlist Verification (strict host or exact host:port match; never cross-grants ports)
    let (permitted, explicit_host_grant) = BOA_NET_HOSTS.with(|hosts_cell| {
        let hosts = hosts_cell.borrow();
        let ipv6_match = parts.hostname.contains(':')
            && (hosts.contains(&format!("[{}]", parts.hostname))
                || hosts.contains(&format!("{}:{}", parts.hostname, parts.port)));
        if hosts.contains(&parts.host_port) || hosts.contains(&parts.hostname) || ipv6_match {
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
        let msg = violation.to_string();
        BOA_VIOLATION.with(|v| *v.borrow_mut() = Some(violation));
        return Err(msg);
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
        return Err(format!(
            "No socket address resolved for {}",
            parts.host_port
        ));
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
    let slab_guard = FetchSlabSlotGuard::acquire();
    stream.write_all(req_buf)?;
    stream.flush()?;

    let effective_max_body = BOA_KV_MEM_BUDGET.with(|m| {
        let (used, limit) = m.get();
        limit.saturating_sub(used).min(MAX_OUTBOUND_BODY_BYTES)
    });

    let mut raw_resp = Vec::with_capacity(4096);

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
        let n =
            slab_guard.read_chunk_via_slab(stream, &mut raw_resp, slice_io::FIXED_BUFFER_SIZE)?;
        if n == 0 {
            if raw_resp.is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "peer closed connection before sending HTTP response headers",
                ));
            }
            break (raw_resp.len(), raw_resp.len());
        }
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
                    if raw_resp.len().saturating_sub(body_start)
                        > effective_max_body.saturating_add(65536)
                    {
                        return Err(trap_outbound_memory_exceeded(
                            raw_resp.len().saturating_sub(body_start),
                            effective_max_body,
                        ));
                    }
                    let n = slab_guard.read_chunk_via_slab(
                        stream,
                        &mut raw_resp,
                        slice_io::FIXED_BUFFER_SIZE,
                    )?;
                    if n == 0 {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "truncated chunked HTTP response body",
                        ));
                    }
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
            let n = slab_guard.read_chunk_via_slab(stream, &mut raw_resp, remaining)?;
            if n == 0 {
                peer_closed = true;
                break;
            }
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
            let prev_len = raw_resp.len();
            let n = slab_guard.read_chunk_via_slab(
                stream,
                &mut raw_resp,
                slice_io::FIXED_BUFFER_SIZE,
            )?;
            if n == 0 {
                break;
            }
            if current_body_len.saturating_add(n) > effective_max_body {
                raw_resp.truncate(prev_len);
                return Err(trap_outbound_memory_exceeded(
                    current_body_len.saturating_add(n),
                    effective_max_body,
                ));
            }
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

fn establish_proxy_connect_tunnel(
    proxy_parts: &OutboundUrlParts,
    proxy_addrs: &[SocketAddr],
    target_parts: &OutboundUrlParts,
) -> Result<OutboundStream, String> {
    if !is_valid_http_header_value(&target_parts.host_port)
        || target_parts
            .host_port
            .bytes()
            .any(|b| b <= 0x20 || b == 0x7F)
    {
        return Err(
            "InvalidConnectAuthority: target host:port contains invalid characters".to_string(),
        );
    }
    let mut proxy_stream = dial_fresh_outbound_stream(proxy_parts, proxy_addrs)?;
    let connect_req = format!(
        "CONNECT {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: Slice-NexusJS/{}\r\nProxy-Connection: keep-alive\r\n\r\n",
        target_parts.host_port,
        target_parts.host_port,
        env!("CARGO_PKG_VERSION")
    );
    proxy_stream
        .write_all(connect_req.as_bytes())
        .and_then(|()| proxy_stream.flush())
        .map_err(|e| {
            format!(
                "Proxy CONNECT write failed to {}: {}",
                proxy_parts.host_port, e
            )
        })?;

    let mut hdr_buf = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    loop {
        if hdr_buf.len() >= MAX_OUTBOUND_HEADER_BYTES {
            return Err(
                "ProxyConnectError: CONNECT response headers exceeded 64 KB limit".to_string(),
            );
        }
        let n = proxy_stream.read(&mut byte).map_err(|e| {
            format!(
                "Proxy CONNECT read failed from {}: {}",
                proxy_parts.host_port, e
            )
        })?;
        if n == 0 {
            return Err(format!(
                "ProxyConnectError: proxy {} closed connection during CONNECT handshake",
                proxy_parts.host_port
            ));
        }
        hdr_buf.push(byte[0]);
        if hdr_buf.ends_with(b"\r\n\r\n") || hdr_buf.ends_with(b"\n\n") {
            break;
        }
    }

    let hdr_str = std::str::from_utf8(&hdr_buf)
        .map_err(|_| "ProxyConnectError: invalid UTF-8 in CONNECT response".to_string())?;
    let status_line = hdr_str.lines().next().unwrap_or("");
    let status_code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if status_code != 200 {
        return Err(format!(
            "ProxyConnectError: proxy {} rejected CONNECT tunnel to {} with status {}",
            proxy_parts.host_port, target_parts.host_port, status_code
        ));
    }

    let tls_cfg = get_tls_client_config();
    let sni_host = target_parts
        .hostname
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let server_name = rustls_pki_types::ServerName::try_from(sni_host).map_err(|e| {
        format!(
            "Invalid TLS SNI hostname '{}': {}",
            target_parts.hostname, e
        )
    })?;
    let client_conn = rustls::ClientConnection::new(tls_cfg, server_name).map_err(|e| {
        format!(
            "TLS initialization failed over proxy tunnel for {}: {}",
            target_parts.host_port, e
        )
    })?;

    match proxy_stream {
        OutboundStream::Plain(tcp) => Ok(OutboundStream::Tls(Box::new(rustls::StreamOwned::new(
            client_conn,
            tcp,
        )))),
        OutboundStream::Tls(proxy_tls) => Ok(OutboundStream::TunneledTls(Box::new(
            rustls::StreamOwned::new(client_conn, proxy_tls),
        ))),
        OutboundStream::TunneledTls(_) => {
            Err("ProxyConnectError: nested proxy TLS tunnels are unsupported".to_string())
        }
    }
}

enum OutboundHopDialPlan<'a> {
    Direct {
        verified_addrs: &'a [SocketAddr],
    },
    HttpProxy {
        proxy_parts: &'a OutboundUrlParts,
        proxy_addrs: &'a [SocketAddr],
    },
    HttpsConnectProxy {
        proxy_parts: &'a OutboundUrlParts,
        proxy_addrs: &'a [SocketAddr],
    },
}

fn execute_single_http11_hop(
    target_parts: &OutboundUrlParts,
    request_target: &str,
    dial_plan: OutboundHopDialPlan<'_>,
    method: &str,
    req_headers: &[(String, String)],
    body: &str,
) -> Result<RawHttpResponse, String> {
    // N-Sec-2: Strictly validate method, request target, Host header, and all custom headers against CRLF / control char injection
    if !is_valid_http_header_name(method) {
        return Err(format!(
            "InvalidMethod: '{}' contains invalid token characters",
            method
        ));
    }
    if !is_valid_request_target(request_target) {
        return Err("InvalidRequestTarget: URL path/query contains forbidden whitespace or control characters".to_string());
    }
    if !is_valid_http_header_value(&target_parts.host_header) {
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
        request_target,
        target_parts.host_header,
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
    let proxy_key = match dial_plan {
        OutboundHopDialPlan::Direct { .. } => "",
        OutboundHopDialPlan::HttpProxy { proxy_parts, .. }
        | OutboundHopDialPlan::HttpsConnectProxy { proxy_parts, .. } => &proxy_parts.host_port,
    };

    // 1. Try tenant-scoped warm Keep-Alive stream from `SHARED_OUTBOUND_CONN_POOL` first (`N-Sec-7`)
    if let Some(mut pooled_stream) = take_pooled_stream(
        &tenant_id,
        proxy_key,
        target_parts.is_https,
        &target_parts.host_port,
    ) {
        if let Ok(resp) = send_and_read_framed_http11(&mut pooled_stream, &req_buf, method) {
            OUTBOUND_POOL_REUSE_COUNT.with(|c| c.set(c.get() + 1));
            if !resp.peer_closed {
                return_pooled_stream(
                    tenant_id,
                    proxy_key.to_string(),
                    target_parts.is_https,
                    target_parts.host_port.clone(),
                    pooled_stream,
                );
            }
            return Ok(resp);
        }
        // Stale idle connection closed by remote peer; fall through and dial fresh stream
    }

    // 2. Dial fresh TCP (+ HTTP CONNECT tunnel if HTTPS over proxy + TLS 1.3 if `target_parts.is_https`)
    let mut fresh_stream = match dial_plan {
        OutboundHopDialPlan::Direct { verified_addrs } => {
            dial_fresh_outbound_stream(target_parts, verified_addrs)?
        }
        OutboundHopDialPlan::HttpProxy {
            proxy_parts,
            proxy_addrs,
        } => dial_fresh_outbound_stream(proxy_parts, proxy_addrs)?,
        OutboundHopDialPlan::HttpsConnectProxy {
            proxy_parts,
            proxy_addrs,
        } => establish_proxy_connect_tunnel(proxy_parts, proxy_addrs, target_parts)?,
    };
    let resp = send_and_read_framed_http11(&mut fresh_stream, &req_buf, method).map_err(|e| {
        format!(
            "HTTP/1.1 exchange failed with {}: {}",
            target_parts.host_port, e
        )
    })?;
    if !resp.peer_closed {
        return_pooled_stream(
            tenant_id,
            proxy_key.to_string(),
            target_parts.is_https,
            target_parts.host_port.clone(),
            fresh_stream,
        );
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

fn is_host_bypassed_by_no_proxy(hostname: &str, no_proxy_raw: &str) -> bool {
    let host_lower = hostname.to_ascii_lowercase();
    for item in no_proxy_raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if item == "*" {
            return true;
        }
        let pat = item
            .split(':')
            .next()
            .unwrap_or(item)
            .trim_start_matches('.')
            .to_ascii_lowercase();
        if pat.is_empty() {
            continue;
        }
        if host_lower == pat || host_lower.ends_with(&format!(".{}", pat)) {
            return true;
        }
    }
    false
}

fn resolve_active_proxy_url(is_https: bool, target_hostname: &str) -> Option<String> {
    BOA_ENV_MAP.with(|m| {
        let map = m.borrow();
        if map.is_empty() {
            return None;
        }
        let no_proxy = map
            .get("NO_PROXY")
            .or_else(|| map.get("no_proxy"))
            .map(String::as_str)
            .unwrap_or("");
        if !no_proxy.is_empty() && is_host_bypassed_by_no_proxy(target_hostname, no_proxy) {
            return None;
        }
        let proxy_val = if is_https {
            map.get("HTTPS_PROXY")
                .or_else(|| map.get("https_proxy"))
                .or_else(|| map.get("HTTP_PROXY"))
                .or_else(|| map.get("http_proxy"))
        } else {
            map.get("HTTP_PROXY").or_else(|| map.get("http_proxy"))
        };
        proxy_val
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    })
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
        let resp = if let Some(proxy_url) =
            resolve_active_proxy_url(parts.is_https, &parts.hostname)
        {
            let p_parts = parse_outbound_http_url(&proxy_url)
                .ok_or_else(|| format!("Invalid proxy URL: {}", proxy_url))?;
            let proxy_grant = check_boa_outbound_fetch_ocap_and_dlp(
                &p_parts,
                &proxy_url,
                &headers_json,
                &current_body,
            )?;
            let proxy_addrs = resolve_and_verify_ssrf_socket_addrs(&p_parts, proxy_grant)?;
            if parts.is_https {
                execute_single_http11_hop(
                    &parts,
                    &parts.path_and_query,
                    OutboundHopDialPlan::HttpsConnectProxy {
                        proxy_parts: &p_parts,
                        proxy_addrs: &proxy_addrs,
                    },
                    &current_method,
                    &current_headers,
                    &current_body,
                )?
            } else {
                let abs_target = format!("http://{}{}", parts.host_header, parts.path_and_query);
                execute_single_http11_hop(
                    &parts,
                    &abs_target,
                    OutboundHopDialPlan::HttpProxy {
                        proxy_parts: &p_parts,
                        proxy_addrs: &proxy_addrs,
                    },
                    &current_method,
                    &current_headers,
                    &current_body,
                )?
            }
        } else {
            let verified_addrs = resolve_and_verify_ssrf_socket_addrs(&parts, explicit_host_grant)?;
            execute_single_http11_hop(
                &parts,
                &parts.path_and_query,
                OutboundHopDialPlan::Direct {
                    verified_addrs: &verified_addrs,
                },
                &current_method,
                &current_headers,
                &current_body,
            )?
        };

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
