//! HTTP/1.1 wire framing, header scanners, `itoa` ASCII formatters, and vectored socket writers.

use anyhow::Result;
use std::io::Write;

/// Maximum allowed inbound HTTP request body size (`4 MB`).
pub(crate) const MAX_INBOUND_BODY_BYTES: usize = 4 * 1024 * 1024;
/// Maximum pipelined static asset size eligible for write_buf coalescing (`16 KB`).
/// Static assets larger than `16 KB` (or non-pipelined responses) write directly from the
/// zero-copy `mmap` payload slice without copying into `write_buf`.
pub(crate) const STATIC_COALESCE_MAX_BYTES: usize = 16 * 1024;
/// Capacity ceiling above which `write_buf` is shrunk back to `16 KB` after flushing.
const WRITE_BUF_SHRINK_THRESHOLD_BYTES: usize = 65_536;
const WRITE_BUF_RESTING_CAPACITY_BYTES: usize = 16_384;

static CRLF2_FINDER: std::sync::LazyLock<memchr::memmem::Finder<'static>> =
    std::sync::LazyLock::new(|| memchr::memmem::Finder::new(b"\r\n\r\n"));
static LF2_FINDER: std::sync::LazyLock<memchr::memmem::Finder<'static>> =
    std::sync::LazyLock::new(|| memchr::memmem::Finder::new(b"\n\n"));

#[inline]
pub(crate) fn find_http_header_end(buf: &[u8]) -> Option<(usize, usize)> {
    find_http_header_end_from(buf, 0)
}

#[inline]
pub(crate) fn find_http_header_end_from(buf: &[u8], scan_from: usize) -> Option<(usize, usize)> {
    let start = scan_from.min(buf.len());
    if let Some(rel) = CRLF2_FINDER.find(&buf[start..]) {
        let i = start + rel;
        return Some((i, i + 4));
    }
    // Do not fall back to bare `\n\n` if the request line uses CRLF (`\r\n`) framing.
    let first_lf = memchr::memchr(b'\n', buf)?;
    if first_lf > 0 && buf[first_lf - 1] == b'\r' {
        return None;
    }
    let lf_start = scan_from.saturating_sub(1).min(buf.len());
    if let Some(rel) = LF2_FINDER.find(&buf[lf_start..]) {
        let i = lf_start + rel;
        return Some((i, i + 2));
    }
    None
}

/// Zero-allocation tenant route extractor for `/t/<tenant_id>` and `/t/<tenant_id>/<rest>`.
///
/// Slices directly from `rest` (`raw_path.strip_prefix("/t/")`) at `&rest[t_seg.len()..]` so the
/// returned sub-path always starts at the `'/'` delimiter (`char` boundary) with zero arithmetic
/// off-by-one risk across ASCII, percent-encoded, and multi-byte UTF-8 segments.
#[inline]
pub(crate) fn extract_tenant_route_prefix(raw_path: &str) -> (&str, &str) {
    if let Some(rest) = raw_path.strip_prefix("/t/") {
        if let Some((t_seg, _sub)) = rest.split_once('/') {
            if !t_seg.is_empty() {
                return (t_seg, &rest[t_seg.len()..]);
            }
        } else if !rest.is_empty() {
            return (rest, "/");
        }
    }
    ("tenant-default", raw_path)
}

#[inline]
pub(crate) fn is_chunked_transfer_encoding_only(val: &str) -> bool {
    let mut saw_chunked = false;
    for raw_tok in val.split(',') {
        let tok = raw_tok.trim();
        if tok.is_empty() || saw_chunked || !tok.eq_ignore_ascii_case("chunked") {
            return false;
        }
        saw_chunked = true;
    }
    saw_chunked
}

#[inline]
pub(crate) fn is_forbidden_custom_response_header(name: &str) -> bool {
    name.eq_ignore_ascii_case("transfer-encoding")
        || name.eq_ignore_ascii_case("content-length")
        || name.eq_ignore_ascii_case("content-type")
        || name.eq_ignore_ascii_case("connection")
        || name.eq_ignore_ascii_case("keep-alive")
        || name.eq_ignore_ascii_case("upgrade")
        || name.eq_ignore_ascii_case("trailer")
        || name.eq_ignore_ascii_case("te")
}

#[inline]
pub(crate) fn clear_and_trim_write_buf(write_buf: &mut Vec<u8>) {
    write_buf.clear();
    if write_buf.capacity() > WRITE_BUF_SHRINK_THRESHOLD_BYTES {
        write_buf.shrink_to(WRITE_BUF_RESTING_CAPACITY_BYTES);
    }
}

#[inline]
pub(crate) fn flush_pending_write_buf<S: Write>(
    stream: &mut S,
    write_buf: &mut Vec<u8>,
) -> Result<()> {
    if !write_buf.is_empty() {
        stream.write_all(write_buf)?;
        stream.flush()?;
        clear_and_trim_write_buf(write_buf);
    }
    Ok(())
}

#[inline]
pub(crate) fn contains_ascii_ignore_case(haystack: &str, needle_lower: &[u8]) -> bool {
    haystack
        .as_bytes()
        .windows(needle_lower.len())
        .any(|w| w.eq_ignore_ascii_case(needle_lower))
}

#[inline]
pub(crate) fn http_status_reason(code: u16) -> &'static str {
    match code {
        101 => "Switching Protocols",
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        408 => "Request Timeout",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        _ => "OK",
    }
}

/// Fast stack-allocated ASCII integer formatter using `itoa` (`CLI-PERF-02`).
#[inline]
pub(crate) fn push_u64_ascii(buf: &mut Vec<u8>, val: u64) {
    let mut itoa_buf = itoa::Buffer::new();
    buf.extend_from_slice(itoa_buf.format(val).as_bytes());
}

pub(crate) fn write_http_error_and_close<S: Write>(
    stream: &mut S,
    status: u16,
    json_body: &str,
) -> Result<()> {
    let mut resp = Vec::with_capacity(256 + json_body.len());
    resp.extend_from_slice(b"HTTP/1.1 ");
    push_u64_ascii(&mut resp, status as u64);
    resp.push(b' ');
    resp.extend_from_slice(http_status_reason(status).as_bytes());
    resp.extend_from_slice(b"\r\nContent-Type: application/json\r\nContent-Length: ");
    push_u64_ascii(&mut resp, json_body.len() as u64);
    resp.extend_from_slice(b"\r\nConnection: close\r\nServer: Slice-NexusJS/0.2\r\n\r\n");
    resp.extend_from_slice(json_body.as_bytes());
    let _ = stream.write_all(&resp);
    let _ = stream.flush();
    Ok(())
}

#[inline]
pub(crate) fn write_all_vectored2<W: Write>(
    stream: &mut W,
    mut first: &[u8],
    mut second: &[u8],
) -> std::io::Result<()> {
    if second.is_empty() {
        return stream.write_all(first);
    }
    if first.is_empty() {
        return stream.write_all(second);
    }
    while !first.is_empty() {
        let slices = [std::io::IoSlice::new(first), std::io::IoSlice::new(second)];
        match stream.write_vectored(&slices) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "failed to write whole vectored response",
                ));
            }
            Ok(n) => {
                if n < first.len() {
                    first = &first[n..];
                } else {
                    let rem = n - first.len();
                    first = &[];
                    second = &second[rem..];
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    if !second.is_empty() {
        stream.write_all(second)?;
    }
    Ok(())
}
