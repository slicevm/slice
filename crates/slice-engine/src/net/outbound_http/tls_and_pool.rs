//! TLS 1.3 (`rustls`) Client Config Cache, 64 KB Fetch Slab Guard, and Tenant-Scoped Keep-Alive Connection Pool.

use super::OutboundUrlParts;
use rustc_hash::FxHashMap;
use std::cell::{Cell, RefCell};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const MAX_IDLE_CONNS_PER_TENANT: usize = 16;
const MAX_TENANT_CONN_POOLS: usize = 256;
const IDLE_CONN_TIMEOUT: Duration = Duration::from_secs(30);

static DEFAULT_TLS_CLIENT_CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
static HAS_CUSTOM_ROOTS: AtomicBool = AtomicBool::new(false);
static CUSTOM_ROOT_CERTS: Mutex<(Vec<Vec<u8>>, [u8; 32])> = Mutex::new((Vec::new(), [0u8; 32]));
static CUSTOM_TLS_CLIENT_CONFIGS: OnceLock<Mutex<FxHashMap<[u8; 32], Arc<rustls::ClientConfig>>>> =
    OnceLock::new();

/// Registers an additional trusted root CA certificate (in DER format) for outbound HTTPS `fetch()`.
pub fn register_custom_root_cert_der(cert_der: Vec<u8>) {
    if let Ok(mut guard) = CUSTOM_ROOT_CERTS.lock() {
        guard.0.push(cert_der);
        let mut hasher = blake3::Hasher::new();
        for der in &guard.0 {
            hasher.update(&(der.len() as u64).to_le_bytes());
            hasher.update(der);
        }
        guard.1 = *hasher.finalize().as_bytes();
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
    let hash_key = CUSTOM_ROOT_CERTS.lock().map(|g| g.1).unwrap_or([0u8; 32]);
    let cache = CUSTOM_TLS_CLIENT_CONFIGS.get_or_init(|| Mutex::new(FxHashMap::default()));
    if let Ok(guard) = cache.lock()
        && let Some(existing) = guard.get(&hash_key)
    {
        return Arc::clone(existing);
    }
    let extra = CUSTOM_ROOT_CERTS
        .lock()
        .map(|g| g.0.clone())
        .unwrap_or_default();
    let built = build_tls_client_config(&extra);
    if let Ok(mut guard) = cache.lock() {
        if guard.len() >= 32 {
            guard.clear();
        }
        guard.insert(hash_key, Arc::clone(&built));
    }
    built
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

    pub(crate) fn set_nonblocking(&mut self, nonblocking: bool) {
        match self {
            OutboundStream::Plain(s) => {
                let _ = s.set_nonblocking(nonblocking);
            }
            OutboundStream::Tls(s) => {
                let _ = s.get_ref().set_nonblocking(nonblocking);
            }
            OutboundStream::TunneledTls(s) => {
                let _ = s.get_ref().get_ref().set_nonblocking(nonblocking);
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
static FETCH_SLAB_IN_USE: AtomicUsize = AtomicUsize::new(0);
static FETCH_SLAB_ACQUIRE_COUNT: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static OUTBOUND_POOL_REUSE_COUNT: Cell<u64> = const { Cell::new(0) };
    static FETCH_READ_BUF: RefCell<Box<[u8; slice_io::FIXED_BUFFER_SIZE]>> =
        RefCell::new(Box::new([0u8; slice_io::FIXED_BUFFER_SIZE]));
}

/// Returns the total number of times a fixed 64 KB slab slot was acquired across outbound HTTP `fetch()` operations.
pub fn fetch_reactor_slab_acquire_count() -> u64 {
    FETCH_SLAB_ACQUIRE_COUNT.load(Ordering::Relaxed)
}

/// Returns the number of 64 KB fetch slab slots currently in use.
pub fn fetch_reactor_slab_in_use_count() -> usize {
    FETCH_SLAB_IN_USE.load(Ordering::Acquire)
}

/// Returns the number of 64 KB fetch slab slots currently available (`0..=64`).
pub fn fetch_reactor_slab_available_count() -> usize {
    slice_io::FIXED_BUFFER_COUNT.saturating_sub(FETCH_SLAB_IN_USE.load(Ordering::Acquire))
}

/// Resets the fetch slab acquisition telemetry counter.
pub fn reset_fetch_slab_telemetry() {
    FETCH_SLAB_ACQUIRE_COUNT.store(0, Ordering::Relaxed);
}

pub(super) struct FetchSlabSlotGuard {
    acquired: bool,
}

impl FetchSlabSlotGuard {
    #[inline]
    pub(super) fn acquire() -> Self {
        let mut acquired = false;
        let mut cur = FETCH_SLAB_IN_USE.load(Ordering::Relaxed);
        while cur < slice_io::FIXED_BUFFER_COUNT {
            match FETCH_SLAB_IN_USE.compare_exchange_weak(
                cur,
                cur + 1,
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    acquired = true;
                    FETCH_SLAB_ACQUIRE_COUNT.fetch_add(1, Ordering::Relaxed);
                    break;
                }
                Err(actual) => cur = actual,
            }
        }
        Self { acquired }
    }

    #[inline]
    pub(super) fn read_chunk_via_slab(
        &self,
        stream: &mut OutboundStream,
        dest: &mut Vec<u8>,
        max_bytes: usize,
    ) -> std::io::Result<usize> {
        let limit = max_bytes.min(slice_io::FIXED_BUFFER_SIZE);
        if limit == 0 {
            return Ok(0);
        }
        if self.acquired {
            FETCH_READ_BUF.with(|cell| {
                if let Ok(mut buf) = cell.try_borrow_mut() {
                    let n = stream.read(&mut buf[..limit])?;
                    if n > 0 {
                        dest.extend_from_slice(&buf[..n]);
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
            })
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
        if self.acquired {
            self.acquired = false;
            FETCH_SLAB_IN_USE.fetch_sub(1, Ordering::Release);
        }
    }
}

fn shared_conn_pool() -> &'static Mutex<FxHashMap<String, Vec<PooledOutboundConn>>> {
    SHARED_OUTBOUND_CONN_POOL.get_or_init(|| Mutex::new(FxHashMap::default()))
}

/// Returns the number of times the current isolate/worker reused a warm Keep-Alive TCP/TLS stream.
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

/// Clears all idle connections in the outbound Keep-Alive pool for a specific `tenant_id`.
pub fn clear_outbound_conn_pool_for_tenant(tenant_id: &str) {
    if let Ok(mut map) = shared_conn_pool().lock() {
        map.remove(tenant_id);
    }
}

pub(super) fn take_pooled_stream(
    tenant_id: &str,
    proxy_key: &str,
    is_https: bool,
    host_port: &str,
) -> Option<OutboundStream> {
    let now = Instant::now();
    if let Ok(mut map) = shared_conn_pool().lock()
        && let Some(vec) = map.get_mut(tenant_id)
    {
        vec.retain(|entry| now.duration_since(entry.last_used) < IDLE_CONN_TIMEOUT);
        if let Some(pos) = vec.iter().rposition(|e| {
            e.proxy_key == proxy_key && e.is_https == is_https && e.host_port == host_port
        }) {
            let entry = vec.swap_remove(pos);
            return Some(entry.stream);
        }
    }
    None
}

pub(super) fn return_pooled_stream(
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
            if map.len() >= MAX_TENANT_CONN_POOLS
                && let Some(oldest_tenant) = map
                    .iter()
                    .min_by_key(|(_, vec)| vec.last().map(|e| e.last_used).unwrap_or(now))
                    .map(|(k, _)| k.clone())
            {
                map.remove(&oldest_tenant);
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
