use crate::config_and_watch::{install_ctrlc_handler, ResolvedExecutionConfig};
use anyhow::{Context, Result};
use slice_cas::MappedCasArchive;
use slice_engine::{compile_source_to_bytecode, SliceIsolate};
use slice_sandbox::CapabilityToken;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub(crate) fn cmd_serve(args: &[String]) -> Result<()> {
    let mut exec_cfg = ResolvedExecutionConfig::from_cli_args(args)?;
    let mut smoke_test = false;
    let mut max_requests_opt: Option<usize> = None;
    let mut tenant_dir_opt: Option<PathBuf> = None;
    let mut target_opt: Option<String> = None;

    for arg in args {
        if let Some(m) = arg.strip_prefix("--max-requests=") {
            max_requests_opt = m.parse().ok();
        } else if let Some(td) = arg.strip_prefix("--tenant-dir=") {
            tenant_dir_opt = Some(PathBuf::from(td));
        } else if arg == "--smoke-test" || arg == "--once" {
            smoke_test = true;
        } else if !arg.starts_with('-') && target_opt.is_none() {
            target_opt = Some(arg.clone());
        }
    }

    // Pre-compile or map handler bytecode and extract any embedded CAS static assets
    let default_handler_src = r#"
const route_res: string = WinterTC.route("GET", "https://edge.slice.local/api/v1/status");
console.log(route_res);
"#;
    let mut static_table = slice_cas::StaticAssetTable::new();
    let bytecode = std::sync::Arc::new(if let Some(target) = &target_opt {
        let p = Path::new(target);
        if target.ends_with(".cas") {
            let archive = MappedCasArchive::open(p)?;
            if let Some(toml_str) = archive.get_manifest_toml() {
                exec_cfg.apply_manifest_toml(toml_str, p.parent());
            }
            static_table = slice_cas::StaticAssetTable::from_cas_archive(&archive);
            let entry = archive
                .entrypoint_blob()
                .context("Missing CAS entrypoint")?;
            archive.get_bytecode_slice(entry).to_vec()
        } else {
            exec_cfg.apply_target_path_manifest(p);
            let entry_file = slice_cas::resolve_project_entrypoint(p, None)?;
            if let Ok(raw_entry) = std::fs::read_to_string(&entry_file) {
                exec_cfg.apply_inline_directives(&raw_entry, entry_file.parent());
            }
            let bundled = slice_cas::bundle_entrypoint_with_deps(&entry_file)?;
            compile_source_to_bytecode(&bundled)
        }
    } else if let Ok(Some(archive)) = MappedCasArchive::open_embedded_self() {
        if let Some(toml_str) = archive.get_manifest_toml() {
            exec_cfg.apply_manifest_toml(toml_str, None);
        }
        static_table = slice_cas::StaticAssetTable::from_cas_archive(&archive);
        if let Some(entry) = archive.entrypoint_blob() {
            archive.get_bytecode_slice(entry).to_vec()
        } else {
            compile_source_to_bytecode(default_handler_src)
        }
    } else {
        compile_source_to_bytecode(default_handler_src)
    });

    let port: u16 = exec_cfg.server_port.unwrap_or(3000);
    let workers: usize = exec_cfg.server_workers.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get().clamp(1, 8))
            .unwrap_or(4)
    });
    let static_dir_opt = exec_cfg.static_dir;
    let deterministic_seed = exec_cfg.deterministic_seed;
    let caps = exec_cfg.caps;
    let fuel_limit = exec_cfg.fuel_limit;
    let mem_limit = exec_cfg.mem_limit;

    if let Some(static_dir) = &static_dir_opt {
        static_table.extend_from_directory(static_dir)?;
    }
    let static_table_arc = if static_table.is_empty() {
        None
    } else {
        Some(std::sync::Arc::new(static_table))
    };

    if smoke_test {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let bound_port = listener.local_addr()?.port();
        let bc_clone = std::sync::Arc::clone(&bytecode);
        let st_clone = static_table_arc.as_ref().map(std::sync::Arc::clone);
        let caps_clone = caps.clone();

        let server_thread = std::thread::spawn(move || -> Result<()> {
            SliceIsolate::prewarm_boa_bundle(&bc_clone);
            let mut read_buf = vec![0u8; 65536];
            let mut write_buf = Vec::with_capacity(4096);
            if let Ok((stream, _)) = listener.accept() {
                let _ = serve_connection_keep_alive(
                    stream,
                    1,
                    &bc_clone,
                    None,
                    st_clone.as_deref(),
                    deterministic_seed,
                    &caps_clone,
                    fuel_limit,
                    mem_limit,
                    &mut read_buf,
                    &mut write_buf,
                    None,
                );
            }
            Ok(())
        });

        let mut client = TcpStream::connect(("127.0.0.1", bound_port))?;
        let _ = client.set_nodelay(true);
        // Send 2 pipelined HTTP/1.1 Keep-Alive requests on a single TCP connection
        client.write_all(
            b"GET /api/v1/status?ping=1 HTTP/1.1\r\nHost: localhost\r\nX-Slice-Tenant: tenant-edge-42\r\nConnection: keep-alive\r\n\r\nGET /api/v1/status?ping=2 HTTP/1.1\r\nHost: localhost\r\nX-Slice-Tenant: tenant-edge-42\r\nConnection: close\r\n\r\n",
        )?;
        let mut resp = String::new();
        client.read_to_string(&mut resp)?;
        let _ = server_thread.join();

        println!("=========================================================================");
        println!("  SLICE WINTERTC MULTI-TENANT HTTP MICROVM SERVER SMOKE TEST (KEEP-ALIVE)");
        println!("=========================================================================");
        for line in resp.lines() {
            println!("  {}", line);
        }
        println!("=========================================================================");
        return Ok(());
    }

    install_ctrlc_handler();

    let addr = format!("0.0.0.0:{}", port);
    let listener = std::sync::Arc::new(TcpListener::bind(&addr)?);
    println!(
        "[Slice Edge Server] Listening on http://127.0.0.1:{} (Workers: {}, HTTP/1.1 Keep-Alive + Pre-Warmed __slice_rc)",
        port, workers
    );
    let _ = std::io::Write::flush(&mut std::io::stdout());

    let req_counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let tenant_dir_arc = tenant_dir_opt.map(std::sync::Arc::new);
    let mut handles = Vec::with_capacity(workers.saturating_sub(1));

    for worker_idx in 1..workers {
        let listener_clone = std::sync::Arc::clone(&listener);
        let bc_clone = std::sync::Arc::clone(&bytecode);
        let td_clone = tenant_dir_arc.as_ref().map(std::sync::Arc::clone);
        let st_clone = static_table_arc.as_ref().map(std::sync::Arc::clone);
        let caps_clone = caps.clone();
        let counter_clone = std::sync::Arc::clone(&req_counter);
        handles.push(std::thread::spawn(move || {
            SliceIsolate::prewarm_boa_bundle(&bc_clone);
            let mut read_buf = vec![0u8; 65536];
            let mut write_buf = Vec::with_capacity(4096);
            let mut local_slot = (worker_idx as u32) << 20;
            while let Ok((stream, _)) = listener_clone.accept() {
                local_slot = local_slot.wrapping_add(1);
                let _ = serve_connection_keep_alive(
                    stream,
                    local_slot,
                    &bc_clone,
                    td_clone.as_deref().map(|p| p.as_path()),
                    st_clone.as_deref(),
                    deterministic_seed,
                    &caps_clone,
                    fuel_limit,
                    mem_limit,
                    &mut read_buf,
                    &mut write_buf,
                    max_requests_opt.map(|max| (&*counter_clone, max)),
                );
            }
        }));
    }

    SliceIsolate::prewarm_boa_bundle(&bytecode);
    let mut read_buf = vec![0u8; 65536];
    let mut write_buf = Vec::with_capacity(4096);
    let mut local_slot: u32 = 0;
    while let Ok((stream, _)) = listener.accept() {
        local_slot = local_slot.wrapping_add(1);
        let _ = serve_connection_keep_alive(
            stream,
            local_slot,
            &bytecode,
            tenant_dir_arc.as_deref().map(|p| p.as_path()),
            static_table_arc.as_deref(),
            deterministic_seed,
            &caps,
            fuel_limit,
            mem_limit,
            &mut read_buf,
            &mut write_buf,
            max_requests_opt.map(|max| (&*req_counter, max)),
        );
    }
    Ok(())
}

type TenantCasCacheEntry = (u128, u64, std::sync::Arc<Vec<u8>>);

thread_local! {
    static TENANT_CAS_CACHE: std::cell::RefCell<std::collections::HashMap<String, TenantCasCacheEntry>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Resolves a per-tenant `.cas` archive from `tenant_dir` (`<tenant_id>.cas` or `default.cas`),
/// automatically hot-swapping and pre-warming the bundle when the file's modification timestamp
/// or byte length changes on disk, with zero dropped TCP connections.
pub(crate) fn resolve_tenant_bytecode_hotswap(
    tenant_dir: &Path,
    tenant_id: &str,
) -> Option<std::sync::Arc<Vec<u8>>> {
    let safe_tenant: String = tenant_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if safe_tenant.is_empty() {
        return None;
    }
    let mut candidate = tenant_dir.join(format!("{}.cas", safe_tenant));
    if !candidate.is_file() {
        candidate = tenant_dir.join("default.cas");
        if !candidate.is_file() {
            return None;
        }
    }
    let meta = std::fs::metadata(&candidate).ok()?;
    let file_len = meta.len();
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    TENANT_CAS_CACHE.with(|cache| {
        if let Some((cached_mtime, cached_len, arc_bc)) = cache.borrow().get(&safe_tenant) {
            if *cached_mtime == mtime_ns && *cached_len == file_len {
                return Some(std::sync::Arc::clone(arc_bc));
            }
        }
        let archive = MappedCasArchive::open(&candidate).ok()?;
        let entry = archive.entrypoint_blob()?;
        let bc_vec = archive.get_bytecode_slice(entry).to_vec();
        SliceIsolate::prewarm_boa_bundle(&bc_vec);
        let arc_bc = std::sync::Arc::new(bc_vec);
        cache.borrow_mut().insert(
            safe_tenant,
            (mtime_ns, file_len, std::sync::Arc::clone(&arc_bc)),
        );
        Some(arc_bc)
    })
}

#[inline]
pub(crate) fn find_http_header_end(buf: &[u8]) -> Option<(usize, usize)> {
    if let Some(i) = memchr::memmem::find(buf, b"\r\n\r\n") {
        return Some((i, i + 4));
    }
    if let Some(i) = memchr::memmem::find(buf, b"\n\n") {
        return Some((i, i + 2));
    }
    None
}

#[inline]
fn contains_ascii_ignore_case(haystack: &str, needle_lower: &[u8]) -> bool {
    haystack
        .as_bytes()
        .windows(needle_lower.len())
        .any(|w| w.eq_ignore_ascii_case(needle_lower))
}

/// Maximum allowed inbound HTTP request body size (`4 MB`).
pub(crate) const MAX_INBOUND_BODY_BYTES: usize = 4 * 1024 * 1024;

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

fn write_http_error_and_close(stream: &mut TcpStream, status: u16, json_body: &str) -> Result<()> {
    let mut resp = Vec::with_capacity(256 + json_body.len());
    let _ = write!(
        &mut resp,
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nServer: Slice-NexusJS/0.2\r\n\r\n{}",
        status,
        http_status_reason(status),
        json_body.len(),
        json_body
    );
    let _ = stream.write_all(&resp);
    let _ = stream.flush();
    Ok(())
}

fn flush_server_ws_frames(
    stream: &mut TcpStream,
    frames: Vec<slice_engine::ServerWsOutFrame>,
) -> Result<bool> {
    let mut should_close = false;
    for frame in frames {
        let encoded = match frame {
            slice_engine::ServerWsOutFrame::Text(ref s) => slice_engine::encode_websocket_frame(
                true,
                slice_engine::WS_OPCODE_TEXT,
                None,
                s.as_bytes(),
            ),
            slice_engine::ServerWsOutFrame::Binary(ref b) => {
                slice_engine::encode_websocket_frame(true, slice_engine::WS_OPCODE_BINARY, None, b)
            }
            slice_engine::ServerWsOutFrame::Close(code, ref reason) => {
                should_close = true;
                let payload = slice_engine::build_close_frame_payload(code, reason);
                slice_engine::encode_websocket_frame(
                    true,
                    slice_engine::WS_OPCODE_CLOSE,
                    None,
                    &payload,
                )
            }
        };
        stream.write_all(&encoded)?;
    }
    stream.flush()?;
    Ok(should_close)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn serve_connection_keep_alive(
    mut stream: TcpStream,
    mut slot_id: u32,
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
    let _ = stream.set_nodelay(true);
    // N-Sec-4: Defend against Slowloris socket starvation with 10s read/write socket timeouts
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(10)));

    let mut filled: usize = 0;
    write_buf.clear();
    let mut isolate = SliceIsolate::spawn(
        "tenant-default",
        slot_id,
        caps.clone(),
        fuel_limit,
        mem_limit,
    );
    isolate.set_deterministic_seed(deterministic_seed);
    for (k, v) in &caps.allowed_env_keys {
        if slice_sandbox::is_sensitive_env_key(k) && v.len() >= 6 {
            isolate.add_egress_audit_secret(v.clone());
        }
    }

    loop {
        let mut header_deadline: Option<Instant> = if filled > 0 {
            Some(Instant::now() + std::time::Duration::from_secs(5))
        } else {
            None
        };

        // 1. Read until complete HTTP headers (`\r\n\r\n`) are present in `read_buf[..filled]`
        let (hdr_end, body_offset) = loop {
            if let Some(pos) = find_http_header_end(&read_buf[..filled]) {
                break pos;
            }
            if let Some(deadline) = header_deadline {
                if Instant::now() > deadline {
                    return write_http_error_and_close(
                        &mut stream,
                        408,
                        r#"{"error":"HEADER_READ_TIMEOUT"}"#,
                    );
                }
            }
            if filled >= read_buf.len() {
                return write_http_error_and_close(
                    &mut stream,
                    431,
                    r#"{"error":"REQUEST_HEADER_FIELDS_TOO_LARGE"}"#,
                );
            }
            match stream.read(&mut read_buf[filled..]) {
                Ok(0) => return Ok(()),
                Err(ref e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    if filled > 0 {
                        return write_http_error_and_close(
                            &mut stream,
                            408,
                            r#"{"error":"HEADER_READ_TIMEOUT"}"#,
                        );
                    }
                    return Ok(());
                }
                Err(_) => return Ok(()),
                Ok(n) => {
                    if header_deadline.is_none() {
                        header_deadline = Some(Instant::now() + std::time::Duration::from_secs(5));
                    }
                    filled += n;
                }
            }
        };

        let req_start = Instant::now();
        let (hdr_slice, after_hdr_buf) = read_buf.split_at_mut(body_offset);
        let Ok(hdr_str) = std::str::from_utf8(&hdr_slice[..hdr_end]) else {
            return write_http_error_and_close(
                &mut stream,
                400,
                r#"{"error":"INVALID_HEADER_UTF8"}"#,
            );
        };
        let mut lines = hdr_str.lines();
        let req_line = lines.next().unwrap_or("");

        // Zero-allocation request line split: `METHOD /path?query HTTP/1.1`
        let mut parts = req_line.split_whitespace();
        let method = parts.next().unwrap_or("");
        let raw_target = parts.next().unwrap_or("");
        let version = parts.next().unwrap_or("HTTP/1.1");
        if method.is_empty()
            || raw_target.is_empty()
            || !slice_engine::is_valid_http_header_name(method)
            || !slice_engine::is_valid_request_target(raw_target)
        {
            return write_http_error_and_close(
                &mut stream,
                400,
                r#"{"error":"MALFORMED_REQUEST_LINE"}"#,
            );
        }
        let (raw_path, query) = raw_target.split_once('?').unwrap_or((raw_target, ""));

        let mut keep_alive = !version.ends_with("1.0");
        let mut seen_content_length: Option<usize> = None;
        let mut has_transfer_encoding = false;
        let mut is_chunked_te = false;
        let mut is_ws_upgrade = false;
        let mut sec_websocket_key: &str = "";
        let mut if_none_match: &str = "";
        let mut path = raw_path;
        let mut tenant_id: &str = "tenant-default";
        if let Some(rest) = raw_path.strip_prefix("/t/") {
            if let Some((t_seg, sub)) = rest.split_once('/') {
                if !t_seg.is_empty() {
                    tenant_id = t_seg;
                    path = if sub.is_empty() {
                        "/"
                    } else {
                        &raw_path[2 + t_seg.len()..]
                    };
                }
            } else if !rest.is_empty() {
                tenant_id = rest;
                path = "/";
            }
        }
        // N-Sec-6: Support up to 64 request headers on the stack; reject > 64 with 431
        let mut headers_stack: [(&str, &str); 64] = [("", ""); 64];
        let mut headers_len: usize = 0;

        for line in lines {
            if line.is_empty() {
                continue;
            }
            let Some((k, v)) = line.split_once(':') else {
                return write_http_error_and_close(
                    &mut stream,
                    400,
                    r#"{"error":"MALFORMED_HEADER_LINE"}"#,
                );
            };
            let k = k.trim();
            let v = v.trim();
            if !slice_engine::is_valid_http_header_name(k)
                || !slice_engine::is_valid_http_header_value(v)
            {
                return write_http_error_and_close(
                    &mut stream,
                    400,
                    r#"{"error":"INVALID_HEADER_CHARACTERS"}"#,
                );
            }
            if headers_len >= headers_stack.len() {
                return write_http_error_and_close(
                    &mut stream,
                    431,
                    r#"{"error":"TOO_MANY_REQUEST_HEADERS"}"#,
                );
            }
            if k.eq_ignore_ascii_case("x-slice-tenant") && !v.is_empty() {
                tenant_id = v;
            } else if k.eq_ignore_ascii_case("connection") {
                if contains_ascii_ignore_case(v, b"close") {
                    keep_alive = false;
                } else if contains_ascii_ignore_case(v, b"keep-alive") {
                    keep_alive = true;
                }
            } else if k.eq_ignore_ascii_case("upgrade") && v.eq_ignore_ascii_case("websocket") {
                is_ws_upgrade = true;
            } else if k.eq_ignore_ascii_case("sec-websocket-key") {
                sec_websocket_key = v;
            } else if k.eq_ignore_ascii_case("transfer-encoding") {
                // N-Sec-3: Reject requests with both Transfer-Encoding and Content-Length (CL.TE / TE.CL smuggling)
                if seen_content_length.is_some() {
                    return write_http_error_and_close(
                        &mut stream,
                        400,
                        r#"{"error":"SMUGGLING_CL_TE_CONFLICT_BLOCKED"}"#,
                    );
                }
                has_transfer_encoding = true;
                if contains_ascii_ignore_case(v, b"chunked") {
                    is_chunked_te = true;
                } else {
                    return write_http_error_and_close(
                        &mut stream,
                        400,
                        r#"{"error":"UNSUPPORTED_TRANSFER_ENCODING"}"#,
                    );
                }
            } else if k.eq_ignore_ascii_case("content-length") {
                // N-Sec-3: Reject requests with both Transfer-Encoding and Content-Length or conflicting duplicate Content-Length
                if has_transfer_encoding {
                    return write_http_error_and_close(
                        &mut stream,
                        400,
                        r#"{"error":"SMUGGLING_CL_TE_CONFLICT_BLOCKED"}"#,
                    );
                }
                let Ok(parsed_len) = v.parse::<usize>() else {
                    return write_http_error_and_close(
                        &mut stream,
                        400,
                        r#"{"error":"INVALID_CONTENT_LENGTH"}"#,
                    );
                };
                if let Some(prev_len) = seen_content_length {
                    if prev_len != parsed_len {
                        return write_http_error_and_close(
                            &mut stream,
                            400,
                            r#"{"error":"SMUGGLING_DUPLICATE_CONTENT_LENGTH_BLOCKED"}"#,
                        );
                    }
                }
                if parsed_len > MAX_INBOUND_BODY_BYTES {
                    return write_http_error_and_close(
                        &mut stream,
                        413,
                        r#"{"error":"PAYLOAD_TOO_LARGE"}"#,
                    );
                }
                seen_content_length = Some(parsed_len);
            } else if k.eq_ignore_ascii_case("if-none-match") {
                if_none_match = v;
            }
            headers_stack[headers_len] = (k, v);
            headers_len += 1;
        }

        let content_length = seen_content_length.unwrap_or(0);
        let mut body_filled = filled.saturating_sub(body_offset);
        let mut owned_body_opt: Option<String> = None;
        let body_end: usize;

        if is_chunked_te {
            let mut chunk_raw = Vec::with_capacity(body_filled.max(1024));
            chunk_raw.extend_from_slice(&after_hdr_buf[..body_filled]);
            let mut scratch = [0u8; 4096];
            let (decoded, consumed_raw) = loop {
                match slice_engine::parse_chunked_body(&chunk_raw, MAX_INBOUND_BODY_BYTES) {
                    Ok(Some(res)) => break res,
                    Ok(None) => {
                        if chunk_raw.len() > MAX_INBOUND_BODY_BYTES.saturating_add(65536) {
                            return write_http_error_and_close(
                                &mut stream,
                                413,
                                r#"{"error":"CHUNKED_PAYLOAD_TOO_LARGE"}"#,
                            );
                        }
                        match stream.read(&mut scratch) {
                            Ok(0) | Err(_) => {
                                return write_http_error_and_close(
                                    &mut stream,
                                    400,
                                    r#"{"error":"TRUNCATED_CHUNKED_BODY"}"#,
                                );
                            }
                            Ok(n) => chunk_raw.extend_from_slice(&scratch[..n]),
                        }
                    }
                    Err(e) => {
                        let status = if e.kind() == std::io::ErrorKind::OutOfMemory {
                            413
                        } else {
                            400
                        };
                        return write_http_error_and_close(
                            &mut stream,
                            status,
                            r#"{"error":"INVALID_CHUNKED_BODY"}"#,
                        );
                    }
                }
            };
            // Preserve any pipelined bytes that arrived after the chunked terminator
            let leftover = chunk_raw.len().saturating_sub(consumed_raw);
            if leftover > 0 && leftover <= after_hdr_buf.len() {
                after_hdr_buf[..leftover].copy_from_slice(&chunk_raw[consumed_raw..]);
            }
            filled = body_offset + leftover;
            body_end = body_offset;
            owned_body_opt = Some(String::from_utf8_lossy(&decoded).into_owned());
        } else if content_length <= after_hdr_buf.len() {
            while body_filled < content_length {
                match stream.read(&mut after_hdr_buf[body_filled..]) {
                    Ok(0) | Err(_) => return Ok(()),
                    Ok(n) => body_filled += n,
                }
            }
            filled = body_offset + body_filled;
            body_end = body_offset.saturating_add(content_length).min(filled);
        } else {
            // Spillover path for bodies between ~64 KB and `MAX_INBOUND_BODY_BYTES` (4 MB)
            let mut large_body = Vec::with_capacity(content_length);
            let initial_copy = body_filled.min(content_length);
            large_body.extend_from_slice(&after_hdr_buf[..initial_copy]);
            let mut scratch = [0u8; 8192];
            while large_body.len() < content_length {
                let rem = (content_length - large_body.len()).min(scratch.len());
                match stream.read(&mut scratch[..rem]) {
                    Ok(0) | Err(_) => return Ok(()),
                    Ok(n) => large_body.extend_from_slice(&scratch[..n]),
                }
            }
            filled = body_offset;
            body_end = body_offset;
            owned_body_opt = Some(String::from_utf8_lossy(&large_body).into_owned());
        }

        let body_len = content_length.min(body_filled);
        let conn_hdr = if keep_alive { "keep-alive" } else { "close" };
        let has_more_pipelined = keep_alive
            && body_end < filled
            && find_http_header_end(
                &after_hdr_buf
                    [body_end.saturating_sub(body_offset)..filled.saturating_sub(body_offset)],
            )
            .is_some();

        // Phase F2: Zero-Stat CAS / In-Memory Static Asset Fast-Path (`< 2 us`, 0 JS VM spinup, 0 disk syscalls)
        if !is_ws_upgrade {
            if let Some(static_table) = static_assets_opt {
                if method == "GET" || method == "HEAD" {
                    if let Some(asset) = static_table.resolve(path) {
                        let is_304 = !if_none_match.is_empty()
                            && (if_none_match == "*"
                                || if_none_match.contains(&asset.etag_quoted)
                                || if_none_match.trim_matches('"')
                                    == asset.etag_quoted.trim_matches('"'));
                        let total_us = req_start.elapsed().as_nanos() / 1_000;
                        if is_304 {
                            let _ = write!(
                                write_buf,
                                "HTTP/1.1 304 Not Modified\r\nETag: {}\r\nCache-Control: public, max-age=3600\r\nConnection: {}\r\nServer: Slice-NexusJS/0.2\r\nX-Slice-Tenant: {}\r\nX-Slice-Static: HIT-304\r\nX-Slice-Isolate-Us: 0\r\nX-Slice-Total-Us: {}\r\nX-Slice-Fuel-Used: 0\r\n\r\n",
                                asset.etag_quoted,
                                conn_hdr,
                                tenant_id,
                                total_us,
                            );
                        } else {
                            let _ = write!(
                                write_buf,
                                "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nETag: {}\r\nCache-Control: public, max-age=3600\r\nConnection: {}\r\nServer: Slice-NexusJS/0.2\r\nX-Slice-Tenant: {}\r\nX-Slice-Static: HIT-200\r\nX-Slice-Isolate-Us: 0\r\nX-Slice-Total-Us: {}\r\nX-Slice-Fuel-Used: 0\r\n\r\n",
                                asset.content_type,
                                asset.payload.len(),
                                asset.etag_quoted,
                                conn_hdr,
                                tenant_id,
                                total_us,
                            );
                            if method != "HEAD" {
                                write_buf.extend_from_slice(&asset.payload);
                            }
                        }
                        if !has_more_pipelined || write_buf.len() >= 32768 {
                            stream.write_all(write_buf)?;
                            write_buf.clear();
                        }
                        if let Some((counter, max_reqs)) = max_requests_tracker {
                            let prev = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            if max_reqs > 0 && prev + 1 >= max_reqs {
                                std::process::exit(0);
                            }
                        }
                        if !keep_alive {
                            break;
                        }
                        if filled > body_end {
                            read_buf.copy_within(body_end..filled, 0);
                            filled -= body_end;
                        } else {
                            filled = 0;
                        }
                        continue;
                    }
                }
            }
        }

        let body_str: &str = if let Some(ref owned) = owned_body_opt {
            owned.as_str()
        } else {
            std::str::from_utf8(&after_hdr_buf[..body_len]).unwrap_or("")
        };
        let req_ctx = slice_engine::HttpRequestContext {
            method,
            path,
            query,
            tenant_id,
            body: body_str,
            headers: &headers_stack[..headers_len],
        };

        slot_id = slot_id.wrapping_add(1);
        isolate.reset_for_request(tenant_id, slot_id, fuel_limit, mem_limit);

        let dynamic_bc =
            tenant_dir_opt.and_then(|td| resolve_tenant_bytecode_hotswap(td, tenant_id));
        let active_bytecode: &[u8] = dynamic_bc
            .as_deref()
            .map(|v| v.as_slice())
            .unwrap_or(bytecode);

        match isolate.execute_http_request(active_bytecode, None, &req_ctx) {
            Ok(outcome) => {
                // Check for RFC 6455 Server-Side WebSocket Upgrade (`Upgrade: websocket` + `Sec-WebSocket-Key`)
                if is_ws_upgrade
                    && !sec_websocket_key.is_empty()
                    && outcome.status_code != 403
                    && outcome.status_code != 400
                {
                    let accept_key = slice_engine::compute_websocket_accept_key(&sec_websocket_key);
                    write_buf.clear();
                    let _ = write!(
                        write_buf,
                        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\nServer: Slice-NexusJS/0.2\r\nX-Slice-Tenant: {}\r\n\r\n",
                        accept_key, outcome.metrics.tenant_id
                    );
                    stream.write_all(write_buf)?;
                    stream.flush()?;
                    write_buf.clear();

                    if let Ok(open_frames) =
                        isolate.execute_websocket_event("open", false, &[], 0, "")
                    {
                        if flush_server_ws_frames(&mut stream, open_frames)? {
                            return Ok(());
                        }
                    }

                    let mut ws_pending = Vec::new();
                    if filled > body_end {
                        ws_pending.extend_from_slice(&read_buf[body_end..filled]);
                    }
                    let effective_ws_limit = mem_limit.min(slice_engine::MAX_WS_FRAME_BYTES);
                    loop {
                        match slice_engine::read_websocket_message_from_stream(
                            &mut stream,
                            &mut ws_pending,
                            true,
                            effective_ws_limit,
                        ) {
                            Ok(Some((slice_engine::WS_OPCODE_CLOSE, payload))) => {
                                let (code, reason) =
                                    slice_engine::parse_close_frame_payload(&payload);
                                let _ = isolate.execute_websocket_event(
                                    "close",
                                    false,
                                    &[],
                                    code,
                                    &reason,
                                );
                                let reply_payload =
                                    slice_engine::build_close_frame_payload(code, &reason);
                                let close_frame = slice_engine::encode_websocket_frame(
                                    true,
                                    slice_engine::WS_OPCODE_CLOSE,
                                    None,
                                    &reply_payload,
                                );
                                let _ = stream.write_all(&close_frame);
                                let _ = stream.flush();
                                break;
                            }
                            Ok(Some((opcode, payload))) => {
                                let is_bin = opcode == slice_engine::WS_OPCODE_BINARY;
                                match isolate
                                    .execute_websocket_event("message", is_bin, &payload, 0, "")
                                {
                                    Ok(out_frames) => {
                                        if flush_server_ws_frames(&mut stream, out_frames)? {
                                            break;
                                        }
                                    }
                                    Err(_violation) => {
                                        let close_payload = slice_engine::build_close_frame_payload(
                                            1008,
                                            "Policy Violation",
                                        );
                                        let close_frame = slice_engine::encode_websocket_frame(
                                            true,
                                            slice_engine::WS_OPCODE_CLOSE,
                                            None,
                                            &close_payload,
                                        );
                                        let _ = stream.write_all(&close_frame);
                                        let _ = stream.flush();
                                        break;
                                    }
                                }
                            }
                            Ok(None) | Err(_) => break,
                        }
                    }
                    return Ok(());
                }

                let isolate_us = outcome.metrics.total_time_ns / 1_000;
                let total_us = req_start.elapsed().as_nanos() / 1_000;
                let safe_ct = if slice_engine::is_valid_http_header_value(&outcome.content_type) {
                    outcome.content_type.as_str()
                } else {
                    "application/octet-stream"
                };
                let extra_sse_hdr = if safe_ct.starts_with("text/event-stream") {
                    "Cache-Control: no-cache\r\nX-Accel-Buffering: no\r\n"
                } else {
                    ""
                };
                let _ = write!(
                    write_buf,
                    "HTTP/1.1 {} {}\r\nContent-Type: {}\r\n{}Content-Length: {}\r\nConnection: {}\r\nServer: Slice-NexusJS/0.2\r\nX-Slice-Tenant: {}\r\nX-Slice-Isolate-Us: {}\r\nX-Slice-Total-Us: {}\r\nX-Slice-Fuel-Used: {}\r\n",
                    outcome.status_code,
                    http_status_reason(outcome.status_code),
                    safe_ct,
                    extra_sse_hdr,
                    outcome.response_body.len(),
                    conn_hdr,
                    outcome.metrics.tenant_id,
                    isolate_us,
                    total_us,
                    outcome.metrics.fuel_consumed,
                );
                // N-Sec-2: Validate all custom response headers against CRLF / control character injection
                for (hk, hv) in &outcome.response_headers {
                    if slice_engine::is_valid_http_header_name(hk)
                        && slice_engine::is_valid_http_header_value(hv)
                    {
                        let _ = write!(write_buf, "{}: {}\r\n", hk, hv);
                    }
                }
                write_buf.extend_from_slice(b"\r\n");
                write_buf.extend_from_slice(outcome.response_body.as_bytes());
                if !has_more_pipelined || write_buf.len() >= 32768 {
                    stream.write_all(write_buf)?;
                    write_buf.clear();
                }

                // Phase N1.3: Execute deferred `Slice.waitUntil(fn)` tasks AFTER response is prepared/sent!
                if outcome.wait_until_count > 0 {
                    let _ = isolate.drain_wait_until_tasks();
                }
            }
            Err(violation) => {
                let err_body = format!(r#"{{"error":"{}"}}"#, violation);
                let _ = write!(
                    write_buf,
                    "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: {}\r\nServer: Slice-NexusJS/0.2\r\n\r\n{}",
                    err_body.len(),
                    conn_hdr,
                    err_body
                );
                stream.write_all(write_buf)?;
                write_buf.clear();
            }
        }

        if let Some((counter, max_reqs)) = max_requests_tracker {
            let prev = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if max_reqs > 0 && prev + 1 >= max_reqs {
                std::process::exit(0);
            }
        }

        if !keep_alive {
            break;
        }
        if filled > body_end {
            read_buf.copy_within(body_end..filled, 0);
            filled -= body_end;
        } else {
            filled = 0;
        }
    }
    Ok(())
}
