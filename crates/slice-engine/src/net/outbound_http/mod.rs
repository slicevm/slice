//! Outbound `fetch()` HTTP/1.1 & TLS 1.3 (`rustls`) Engine with Keep-Alive Connection Pooling,
//! Pre-Connect DNS Resolved-IP SSRF Firewall, Zero-Trust Hop-by-Hop Redirect Guard, and Egress DLP.

mod chunked;
mod http11_wire;
mod ocap_and_ssrf;
mod redirects_and_proxy;
mod tls_and_pool;

pub use self::chunked::{ChunkedBodyDecoder, parse_chunked_body};
pub use self::http11_wire::find_http_header_end_from;
pub use self::tls_and_pool::{
    clear_outbound_conn_pool, clear_outbound_conn_pool_for_tenant,
    fetch_reactor_slab_acquire_count, fetch_reactor_slab_available_count,
    fetch_reactor_slab_in_use_count, outbound_pool_reuse_count, register_custom_root_cert_der,
    reset_fetch_slab_telemetry,
};

pub(crate) use self::ocap_and_ssrf::{
    check_boa_outbound_fetch_ocap_and_dlp, parse_outbound_http_url,
    resolve_and_verify_ssrf_socket_addrs,
};
pub(crate) use self::redirects_and_proxy::execute_outbound_fetch_with_redirects;
pub(crate) use self::tls_and_pool::{
    OutboundStream, dial_fresh_outbound_stream, get_tls_client_config, record_outbound_pool_reuses,
};

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

/// Validates an HTTP/1.1 request target (`origin-form` / `path_and_query`) per RFC 9112 Section 3.2:
/// requires non-empty visible US-ASCII (`VCHAR`, `0x21..=0x7E`), rejecting spaces, control chars,
/// obs-text (`0x80..=0xFF`), and CRLF request-line splitting payloads.
#[inline]
pub fn is_valid_request_target(target: &str) -> bool {
    !target.is_empty() && target.bytes().all(|b| b > 0x20 && b < 0x7F)
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
