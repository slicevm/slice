use crate::config_and_watch::{
    find_project_manifest_path, load_manifest_or_default, merge_manifest_toml_into_caps,
};
use anyhow::{Context, Result};
use slice_cas::MappedCasArchive;
use slice_engine::{compile_source_to_bytecode, SliceIsolate};
use slice_sandbox::CapabilityToken;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub(crate) fn cmd_serve(args: &[String]) -> Result<()> {
    let mut port: u16 = 3000;
    let mut smoke_test = false;
    let mut max_requests_opt: Option<usize> = None;
    let mut tenant_dir_opt: Option<PathBuf> = None;
    let mut static_dir_opt: Option<PathBuf> = None;
    let mut deterministic_seed: Option<u64> = None;
    let mut workers: usize = std::thread::available_parallelism()
        .map(|n| n.get().clamp(1, 8))
        .unwrap_or(4);
    let mut target_opt: Option<String> = None;

    for arg in args {
        if let Some(p) = arg.strip_prefix("--port=") {
            port = p.parse().unwrap_or(3000);
        } else if let Some(w) = arg.strip_prefix("--workers=") {
            workers = w.parse().unwrap_or(4).max(1);
        } else if let Some(m) = arg.strip_prefix("--max-requests=") {
            max_requests_opt = m.parse().ok();
        } else if let Some(td) = arg.strip_prefix("--tenant-dir=") {
            tenant_dir_opt = Some(PathBuf::from(td));
        } else if let Some(sd) = arg.strip_prefix("--static=") {
            static_dir_opt = Some(PathBuf::from(sd));
        } else if let Some(seed_str) = arg.strip_prefix("--deterministic=") {
            deterministic_seed = Some(seed_str.parse().unwrap_or(42));
        } else if arg == "--deterministic" {
            deterministic_seed = Some(42);
        } else if arg == "--smoke-test" || arg == "--once" {
            smoke_test = true;
        } else if !arg.starts_with('-') && target_opt.is_none() {
            target_opt = Some(arg.clone());
        }
    }

    let (mut caps, fuel_limit, mem_limit) = load_manifest_or_default();
    for arg in args {
        if let Some(host) = arg.strip_prefix("--allow-net=") {
            for h in host.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                caps = caps.with_net_host(h);
            }
        } else if let Some(prefix) = arg.strip_prefix("--allow-read=") {
            caps = caps.with_read_prefix(prefix);
        } else if let Some(prefix) = arg.strip_prefix("--allow-write=") {
            caps = caps.with_write_prefix(prefix);
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
                merge_manifest_toml_into_caps(toml_str, &mut caps);
            }
            static_table = slice_cas::StaticAssetTable::from_cas_archive(&archive);
            let entry = archive
                .entrypoint_blob()
                .context("Missing CAS entrypoint")?;
            archive.get_bytecode_slice(entry).to_vec()
        } else {
            if let Some(manifest_file) = find_project_manifest_path(p) {
                if let Ok(raw) = std::fs::read_to_string(&manifest_file) {
                    merge_manifest_toml_into_caps(&raw, &mut caps);
                }
            }
            let entry_file = slice_cas::resolve_project_entrypoint(p, None)?;
            let bundled = slice_cas::bundle_entrypoint_with_deps(&entry_file)?;
            compile_source_to_bytecode(&bundled)
        }
    } else if let Ok(Some(archive)) = MappedCasArchive::open_embedded_self() {
        if let Some(toml_str) = archive.get_manifest_toml() {
            merge_manifest_toml_into_caps(toml_str, &mut caps);
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

    let addr = format!("0.0.0.0:{}", port);
    let listener = std::sync::Arc::new(TcpListener::bind(&addr)?);
    println!(
        "[Slice Edge Server] Listening on http://127.0.0.1:{} (Workers: {}, HTTP/1.1 Keep-Alive + Pre-Warmed __slice_rc)",
        port, workers
    );

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
    if buf.len() < 4 {
        return None;
    }
    for i in 0..=buf.len() - 4 {
        if buf[i] == b'\r' && buf[i + 1] == b'\n' && buf[i + 2] == b'\r' && buf[i + 3] == b'\n' {
            return Some((i, i + 4));
        }
    }
    for i in 0..=buf.len().saturating_sub(2) {
        if buf[i] == b'\n' && buf[i + 1] == b'\n' {
            return Some((i, i + 2));
        }
    }
    None
}

#[inline]
pub(crate) fn http_status_reason(code: u16) -> &'static str {
    match code {
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
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        _ => "OK",
    }
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

    loop {
        // 1. Read until complete HTTP headers (`\r\n\r\n`) are present in `read_buf[..filled]`
        let (hdr_end, body_offset) = loop {
            if let Some(pos) = find_http_header_end(&read_buf[..filled]) {
                break pos;
            }
            if filled >= read_buf.len() {
                return Ok(());
            }
            match stream.read(&mut read_buf[filled..]) {
                Ok(0) | Err(_) => return Ok(()),
                Ok(n) => filled += n,
            }
        };

        let req_start = Instant::now();
        let (hdr_slice, after_hdr_buf) = read_buf.split_at_mut(body_offset);
        let hdr_str = std::str::from_utf8(&hdr_slice[..hdr_end]).unwrap_or("");
        let mut lines = hdr_str.lines();
        let req_line = lines.next().unwrap_or("");

        // Zero-allocation request line split: `METHOD /path?query HTTP/1.1`
        let mut parts = req_line.split_whitespace();
        let method = parts.next().unwrap_or("GET");
        let raw_target = parts.next().unwrap_or("/");
        let version = parts.next().unwrap_or("HTTP/1.1");
        let (raw_path, query) = raw_target.split_once('?').unwrap_or((raw_target, ""));

        let mut keep_alive = !version.ends_with("1.0");
        let mut content_length: usize = 0;
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
        let mut headers_stack: [(&str, &str); 16] = [("", ""); 16];
        let mut headers_len: usize = 0;

        for line in lines {
            if let Some((k, v)) = line.split_once(':') {
                let k = k.trim();
                let v = v.trim();
                if k.eq_ignore_ascii_case("x-slice-tenant") && !v.is_empty() {
                    tenant_id = v;
                } else if k.eq_ignore_ascii_case("connection") {
                    if v.eq_ignore_ascii_case("close") {
                        keep_alive = false;
                    } else if v.eq_ignore_ascii_case("keep-alive") {
                        keep_alive = true;
                    }
                } else if k.eq_ignore_ascii_case("content-length") {
                    content_length = v.parse().unwrap_or(0);
                } else if k.eq_ignore_ascii_case("if-none-match") {
                    if_none_match = v;
                }
                if headers_len < headers_stack.len() {
                    headers_stack[headers_len] = (k, v);
                    headers_len += 1;
                }
            }
        }

        let mut body_filled = filled.saturating_sub(body_offset);
        while body_filled < content_length && content_length <= after_hdr_buf.len() {
            match stream.read(&mut after_hdr_buf[body_filled..]) {
                Ok(0) | Err(_) => return Ok(()),
                Ok(n) => body_filled += n,
            }
        }
        filled = body_offset + body_filled;

        let body_end = body_offset.saturating_add(content_length).min(filled);
        let body_len = content_length.min(body_filled);
        let conn_hdr = if keep_alive { "keep-alive" } else { "close" };
        let has_more_pipelined =
            keep_alive && find_http_header_end(&after_hdr_buf[body_len..body_filled]).is_some();

        // Phase F2: Zero-Stat CAS / In-Memory Static Asset Fast-Path (`< 2 us`, 0 JS VM spinup, 0 disk syscalls)
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

        let body_str = std::str::from_utf8(&after_hdr_buf[..body_len]).unwrap_or("");
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
                let isolate_us = outcome.metrics.total_time_ns / 1_000;
                let total_us = req_start.elapsed().as_nanos() / 1_000;
                let extra_sse_hdr = if outcome.content_type.starts_with("text/event-stream") {
                    "Cache-Control: no-cache\r\nX-Accel-Buffering: no\r\n"
                } else {
                    ""
                };
                let _ = write!(
                    write_buf,
                    "HTTP/1.1 {} {}\r\nContent-Type: {}\r\n{}Content-Length: {}\r\nConnection: {}\r\nServer: Slice-NexusJS/0.2\r\nX-Slice-Tenant: {}\r\nX-Slice-Isolate-Us: {}\r\nX-Slice-Total-Us: {}\r\nX-Slice-Fuel-Used: {}\r\n",
                    outcome.status_code,
                    http_status_reason(outcome.status_code),
                    outcome.content_type,
                    extra_sse_hdr,
                    outcome.response_body.len(),
                    conn_hdr,
                    outcome.metrics.tenant_id,
                    isolate_us,
                    total_us,
                    outcome.metrics.fuel_consumed,
                );
                for (hk, hv) in &outcome.response_headers {
                    let _ = write!(write_buf, "{}: {}\r\n", hk, hv);
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
