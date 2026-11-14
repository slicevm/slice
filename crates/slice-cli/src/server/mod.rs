pub(crate) mod connection;
pub(crate) mod tls;

#[cfg(test)]
pub(crate) use connection::serve_connection_keep_alive;
pub(crate) use connection::{
    serve_connection_keep_alive_with_tls_and_secrets, serve_connection_with_isolate,
    spawn_worker_isolate,
};
pub(crate) use tls::load_tls_server_config;

use crate::config::{install_ctrlc_handler, ResolvedExecutionConfig};
use anyhow::{Context, Result};
use rustc_hash::FxHashMap;
use slice_cas::MappedCasArchive;
use slice_engine::{compile_source_to_bytecode, SliceIsolate};
use std::borrow::Cow;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};

pub(crate) fn cmd_serve(args: &[String]) -> Result<()> {
    let mut smoke_test = false;
    let mut max_requests_opt: Option<usize> = None;
    let mut tenant_dir_opt: Option<PathBuf> = None;
    let mut bind_host = String::from("0.0.0.0");
    let mut target_opt: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--config" {
            i += 2;
            continue;
        }
        if arg == "--host" {
            if let Some(h) = args.get(i + 1) {
                if !h.trim().is_empty() {
                    bind_host = h.trim().to_string();
                }
            }
            i += 2;
            continue;
        }
        if let Some(h) = arg.strip_prefix("--host=") {
            if !h.trim().is_empty() {
                bind_host = h.trim().to_string();
            }
        } else if let Some(m) = arg.strip_prefix("--max-requests=") {
            max_requests_opt = m.parse().ok();
        } else if let Some(td) = arg.strip_prefix("--tenant-dir=") {
            tenant_dir_opt = Some(PathBuf::from(td));
        } else if arg == "--smoke-test" || arg == "--once" {
            smoke_test = true;
        } else if !arg.starts_with('-') && target_opt.is_none() {
            target_opt = Some(arg.clone());
        }
        i += 1;
    }

    let embedded_archive = if target_opt.is_none() {
        MappedCasArchive::open_embedded_self().ok().flatten()
    } else {
        None
    };
    let is_cas_archive_mode =
        target_opt.as_deref().is_some_and(|t| t.ends_with(".cas")) || embedded_archive.is_some();
    let load_ambient_cwd = !is_cas_archive_mode;
    let mut exec_cfg =
        ResolvedExecutionConfig::from_cli_args_with_ambient_cwd(args, load_ambient_cwd)?;

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
                exec_cfg.apply_manifest_toml(toml_str, None);
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
    } else if let Some(archive) = embedded_archive {
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
    let has_cli_static_override = exec_cfg.cli_static_override.is_some();
    let static_dir_opt = exec_cfg.static_dir;
    let tls_cert_opt = exec_cfg.tls_cert;
    let tls_key_opt = exec_cfg.tls_key;
    let deterministic_seed = exec_cfg.deterministic_seed;
    let caps = exec_cfg.caps;
    let fuel_limit = exec_cfg.fuel_limit;
    let mem_limit = exec_cfg.mem_limit;
    let audit_secrets = std::sync::Arc::new(exec_cfg.audit_secrets);

    let tls_server_cfg = match (&tls_cert_opt, &tls_key_opt) {
        (Some(cert_path), Some(key_path)) => Some(load_tls_server_config(cert_path, key_path)?),
        (Some(_), None) => anyhow::bail!("--tls-cert specified without --tls-key"),
        (None, Some(_)) => anyhow::bail!("--tls-key specified without --tls-cert"),
        (None, None) => None,
    };

    if let Some(static_dir) = &static_dir_opt {
        if !is_cas_archive_mode || has_cli_static_override || static_dir.is_dir() {
            static_table.extend_from_directory(static_dir)?;
        }
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
        let secrets_clone = std::sync::Arc::clone(&audit_secrets);

        let server_thread = std::thread::spawn(move || -> Result<()> {
            SliceIsolate::prewarm_boa_bundle_with_caps(&bc_clone, &caps_clone);
            let mut read_buf = vec![0u8; 65536];
            let mut write_buf = Vec::with_capacity(4096);
            if let Ok((stream, _)) = listener.accept() {
                let _ = serve_connection_keep_alive_with_tls_and_secrets(
                    stream,
                    1,
                    &bc_clone,
                    None,
                    st_clone.as_deref(),
                    deterministic_seed,
                    &caps_clone,
                    &secrets_clone,
                    fuel_limit,
                    mem_limit,
                    &mut read_buf,
                    &mut write_buf,
                    None,
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

    let listener = std::sync::Arc::new(TcpListener::bind((bind_host.as_str(), port))?);
    let bound_addr = listener.local_addr()?;
    let bound_port = bound_addr.port();
    let wake_ip = if bound_addr.ip().is_unspecified() {
        if bound_addr.is_ipv6() {
            std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)
        } else {
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
        }
    } else {
        bound_addr.ip()
    };
    let scheme = if tls_server_cfg.is_some() {
        "https"
    } else {
        "http"
    };
    println!(
        "[Slice Edge Server] Listening on {}://{}:{} (Workers: {}, HTTP/1.1 Keep-Alive + Pre-Warmed __slice_rc)",
        scheme, wake_ip, bound_port, workers
    );
    let _ = std::io::Write::flush(&mut std::io::stdout());

    let req_counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let shutdown_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let tenant_dir_arc = tenant_dir_opt.map(std::sync::Arc::new);
    let mut handles = Vec::with_capacity(workers.saturating_sub(1));

    for worker_idx in 1..workers {
        let listener_clone = std::sync::Arc::clone(&listener);
        let bc_clone = std::sync::Arc::clone(&bytecode);
        let td_clone = tenant_dir_arc.as_ref().map(std::sync::Arc::clone);
        let st_clone = static_table_arc.as_ref().map(std::sync::Arc::clone);
        let tls_clone = tls_server_cfg.as_ref().map(std::sync::Arc::clone);
        let caps_clone = caps.clone();
        let secrets_clone = std::sync::Arc::clone(&audit_secrets);
        let counter_clone = std::sync::Arc::clone(&req_counter);
        let shutdown_clone = std::sync::Arc::clone(&shutdown_flag);
        handles.push(std::thread::spawn(move || {
            SliceIsolate::prewarm_boa_bundle_with_caps(&bc_clone, &caps_clone);
            let mut read_buf = vec![0u8; 65536];
            let mut write_buf = Vec::with_capacity(4096);
            let mut local_slot = (worker_idx as u32) << 20;
            let mut isolate = spawn_worker_isolate(
                local_slot,
                deterministic_seed,
                &caps_clone,
                &secrets_clone,
                fuel_limit,
                mem_limit,
            );
            while let Ok((stream, _)) = listener_clone.accept() {
                if shutdown_clone.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                local_slot = local_slot.wrapping_add(1);
                let conn_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    serve_connection_with_isolate(
                        stream,
                        local_slot,
                        &mut isolate,
                        &bc_clone,
                        td_clone.as_deref().map(|p| p.as_path()),
                        st_clone.as_deref(),
                        fuel_limit,
                        mem_limit,
                        &mut read_buf,
                        &mut write_buf,
                        max_requests_opt.map(|max| (&*counter_clone, max)),
                        tls_clone.as_ref(),
                    )
                }));
                if conn_res.is_err() {
                    read_buf.fill(0);
                    write_buf.clear();
                    isolate = spawn_worker_isolate(
                        local_slot,
                        deterministic_seed,
                        &caps_clone,
                        &secrets_clone,
                        fuel_limit,
                        mem_limit,
                    );
                }
                if let Some(max) = max_requests_opt {
                    if max > 0 && counter_clone.load(std::sync::atomic::Ordering::Acquire) >= max {
                        if !shutdown_clone.swap(true, std::sync::atomic::Ordering::AcqRel) {
                            wake_accept_workers(wake_ip, bound_port, workers);
                        }
                        break;
                    }
                }
            }
        }));
    }

    SliceIsolate::prewarm_boa_bundle_with_caps(&bytecode, &caps);
    let mut read_buf = vec![0u8; 65536];
    let mut write_buf = Vec::with_capacity(4096);
    let mut local_slot: u32 = 0;
    let mut isolate = spawn_worker_isolate(
        local_slot,
        deterministic_seed,
        &caps,
        &audit_secrets,
        fuel_limit,
        mem_limit,
    );
    while let Ok((stream, _)) = listener.accept() {
        if shutdown_flag.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        local_slot = local_slot.wrapping_add(1);
        let conn_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            serve_connection_with_isolate(
                stream,
                local_slot,
                &mut isolate,
                &bytecode,
                tenant_dir_arc.as_deref().map(|p| p.as_path()),
                static_table_arc.as_deref(),
                fuel_limit,
                mem_limit,
                &mut read_buf,
                &mut write_buf,
                max_requests_opt.map(|max| (&*req_counter, max)),
                tls_server_cfg.as_ref(),
            )
        }));
        if conn_res.is_err() {
            read_buf.fill(0);
            write_buf.clear();
            isolate = spawn_worker_isolate(
                local_slot,
                deterministic_seed,
                &caps,
                &audit_secrets,
                fuel_limit,
                mem_limit,
            );
        }
        if let Some(max) = max_requests_opt {
            if max > 0 && req_counter.load(std::sync::atomic::Ordering::Acquire) >= max {
                if !shutdown_flag.swap(true, std::sync::atomic::Ordering::AcqRel) {
                    wake_accept_workers(wake_ip, bound_port, workers);
                }
                break;
            }
        }
    }
    for h in handles {
        let _ = h.join();
    }
    Ok(())
}

fn wake_accept_workers(wake_ip: std::net::IpAddr, port: u16, workers: usize) {
    let target = std::net::SocketAddr::new(wake_ip, port);
    for _ in 0..workers {
        let _ = TcpStream::connect_timeout(&target, std::time::Duration::from_millis(50));
    }
}

const MAX_TENANT_CACHE_ENTRIES: usize = 1024;

const TENANT_STAT_TTL: std::time::Duration = std::time::Duration::from_millis(250);

static TENANT_FS_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

struct TenantDirWatchRegistry {
    watched_dirs: rustc_hash::FxHashSet<PathBuf>,
    watcher: Option<notify::RecommendedWatcher>,
}

static TENANT_DIR_WATCHERS: std::sync::OnceLock<std::sync::Mutex<TenantDirWatchRegistry>> =
    std::sync::OnceLock::new();

#[inline]
pub(crate) fn notify_tenant_cas_updated() {
    TENANT_FS_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Release);
}

fn ensure_tenant_dir_watched(tenant_dir: &Path) {
    use notify::{RecursiveMode, Watcher};
    let reg = TENANT_DIR_WATCHERS.get_or_init(|| {
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if res.is_ok() {
                TENANT_FS_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Release);
            }
        })
        .ok();
        std::sync::Mutex::new(TenantDirWatchRegistry {
            watched_dirs: rustc_hash::FxHashSet::default(),
            watcher,
        })
    });
    if let Ok(mut guard) = reg.lock() {
        if !guard.watched_dirs.contains(tenant_dir) && guard.watched_dirs.len() < 64 {
            if let Some(ref mut w) = guard.watcher {
                let _ = w.watch(tenant_dir, RecursiveMode::NonRecursive);
            }
            guard.watched_dirs.insert(tenant_dir.to_path_buf());
        }
    }
}

pub(crate) struct TenantHotSwapEntry {
    pub(crate) tenant_dir: PathBuf,
    pub(crate) resolved_path: PathBuf,
    pub(crate) mtime_ns: u128,
    pub(crate) file_len: u64,
    pub(crate) verified_gen: u64,
    pub(crate) last_stat_check: std::time::Instant,
    pub(crate) bytecode: std::sync::Arc<Vec<u8>>,
}

struct CompiledCasBundleEntry {
    mtime_ns: u128,
    file_len: u64,
    bytecode: std::sync::Arc<Vec<u8>>,
}

#[derive(Default)]
struct TenantCasCacheState {
    by_tenant: FxHashMap<String, TenantHotSwapEntry>,
    by_canonical_path: FxHashMap<PathBuf, CompiledCasBundleEntry>,
}

thread_local! {
    static TENANT_CAS_CACHE: std::cell::RefCell<TenantCasCacheState> =
        std::cell::RefCell::new(TenantCasCacheState::default());
}

#[cfg(test)]
pub(crate) fn tenant_cas_cache_len() -> (usize, usize) {
    TENANT_CAS_CACHE.with(|c| {
        let borrow = c.borrow();
        (borrow.by_tenant.len(), borrow.by_canonical_path.len())
    })
}

#[cfg(test)]
pub(crate) fn clear_tenant_cas_cache() {
    TENANT_CAS_CACHE.with(|c| {
        let mut borrow = c.borrow_mut();
        borrow.by_tenant.clear();
        borrow.by_canonical_path.clear();
    });
}

#[inline]
fn is_clean_tenant_id(s: &str) -> bool {
    if s.is_empty() || s.len() > 128 {
        return false;
    }
    let bytes = s.as_bytes();
    if bytes[0] == b'.' || bytes[bytes.len() - 1] == b'.' {
        return false;
    }
    let mut prev_dot = false;
    for &b in bytes {
        if b == b'.' {
            if prev_dot {
                return false;
            }
            prev_dot = true;
        } else if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
            prev_dot = false;
        } else {
            return false;
        }
    }
    true
}

/// Sanitizes `tenant_id` without heap allocation when already valid (`[A-Za-z0-9_-]` plus single
/// interior `.`, max 128 bytes). Strips `:` (preventing `<tenant>:<key>` KV collisions), path
/// separators, and `..` traversal sequences.
#[inline]
pub(crate) fn sanitize_tenant_id(tenant_id: &str) -> Cow<'_, str> {
    if is_clean_tenant_id(tenant_id) {
        return Cow::Borrowed(tenant_id);
    }
    let mut out = String::with_capacity(tenant_id.len().min(128));
    let mut prev_dot = false;
    for ch in tenant_id.chars() {
        if out.len() >= 128 {
            break;
        }
        if ch == '.' {
            if !out.is_empty() && !prev_dot {
                out.push('.');
                prev_dot = true;
            }
        } else if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
            prev_dot = false;
        }
    }
    while out.ends_with('.') {
        out.pop();
    }
    Cow::Owned(out)
}

/// Resolves a per-tenant `.cas` archive from `tenant_dir` (`<tenant_id>.cas` or `default.cas`),
/// automatically hot-swapping and pre-warming the bundle when the file's modification timestamp
/// or byte length changes on disk, deduplicating `default.cas` across unknown tenant headers,
/// and bounding cache entries at `1,024`.
pub(crate) fn resolve_tenant_bytecode_hotswap(
    tenant_dir: &Path,
    tenant_id: &str,
) -> Option<std::sync::Arc<Vec<u8>>> {
    let safe_tenant = sanitize_tenant_id(tenant_id);
    if safe_tenant.is_empty() {
        return None;
    }

    TENANT_CAS_CACHE.with(|cache| {
        let cur_gen = TENANT_FS_GENERATION.load(std::sync::atomic::Ordering::Acquire);
        // Fast path: dedicated `<tenant_id>.cas` already cached for this `tenant_dir`
        {
            let mut borrow = cache.borrow_mut();
            if let Some(entry) = borrow.by_tenant.get_mut(safe_tenant.as_ref()) {
                if entry.tenant_dir == tenant_dir {
                    if entry.verified_gen == cur_gen
                        && entry.last_stat_check.elapsed() < TENANT_STAT_TTL
                    {
                        return Some(std::sync::Arc::clone(&entry.bytecode));
                    }
                    if let Ok(meta) = std::fs::metadata(&entry.resolved_path) {
                        if meta.is_file() {
                            let file_len = meta.len();
                            let mtime_ns = meta
                                .modified()
                                .ok()
                                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|d| d.as_nanos())
                                .unwrap_or(0);
                            if entry.mtime_ns == mtime_ns && entry.file_len == file_len {
                                entry.verified_gen = cur_gen;
                                entry.last_stat_check = std::time::Instant::now();
                                return Some(std::sync::Arc::clone(&entry.bytecode));
                            }
                        }
                    }
                }
            }
        }

        ensure_tenant_dir_watched(tenant_dir);
        let cur_gen = TENANT_FS_GENERATION.load(std::sync::atomic::Ordering::Acquire);

        let mut candidate = tenant_dir.join(format!("{}.cas", safe_tenant));
        let mut is_default_fallback = false;
        let meta = match std::fs::metadata(&candidate) {
            Ok(m) if m.is_file() => m,
            _ => {
                candidate = tenant_dir.join("default.cas");
                is_default_fallback = true;
                let m = std::fs::metadata(&candidate).ok()?;
                if !m.is_file() {
                    return None;
                }
                m
            }
        };

        let file_len = meta.len();
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0);

        let canonical_path =
            std::fs::canonicalize(&candidate).unwrap_or_else(|_| candidate.clone());

        if let Some(bundle) = cache.borrow().by_canonical_path.get(&canonical_path) {
            if bundle.mtime_ns == mtime_ns && bundle.file_len == file_len {
                let arc_bc = std::sync::Arc::clone(&bundle.bytecode);
                if !is_default_fallback {
                    let mut borrow = cache.borrow_mut();
                    if borrow.by_tenant.len() >= MAX_TENANT_CACHE_ENTRIES
                        && !borrow.by_tenant.contains_key(safe_tenant.as_ref())
                    {
                        borrow.by_tenant.clear();
                    }
                    borrow.by_tenant.insert(
                        safe_tenant.into_owned(),
                        TenantHotSwapEntry {
                            tenant_dir: tenant_dir.to_path_buf(),
                            resolved_path: canonical_path,
                            mtime_ns,
                            file_len,
                            verified_gen: cur_gen,
                            last_stat_check: std::time::Instant::now(),
                            bytecode: std::sync::Arc::clone(&arc_bc),
                        },
                    );
                }
                return Some(arc_bc);
            }
        }

        let archive = MappedCasArchive::open(&canonical_path).ok()?;
        let entry_blob = archive.entrypoint_blob()?;
        let bc_vec = archive.get_bytecode_slice(entry_blob).to_vec();
        SliceIsolate::prewarm_boa_bundle(&bc_vec);
        let arc_bc = std::sync::Arc::new(bc_vec);

        let mut borrow = cache.borrow_mut();
        if borrow.by_canonical_path.len() >= MAX_TENANT_CACHE_ENTRIES
            && !borrow.by_canonical_path.contains_key(&canonical_path)
        {
            borrow.by_canonical_path.clear();
        }
        borrow.by_canonical_path.insert(
            canonical_path.clone(),
            CompiledCasBundleEntry {
                mtime_ns,
                file_len,
                bytecode: std::sync::Arc::clone(&arc_bc),
            },
        );
        if !is_default_fallback {
            if borrow.by_tenant.len() >= MAX_TENANT_CACHE_ENTRIES
                && !borrow.by_tenant.contains_key(safe_tenant.as_ref())
            {
                borrow.by_tenant.clear();
            }
            borrow.by_tenant.insert(
                safe_tenant.into_owned(),
                TenantHotSwapEntry {
                    tenant_dir: tenant_dir.to_path_buf(),
                    resolved_path: canonical_path,
                    mtime_ns,
                    file_len,
                    verified_gen: cur_gen,
                    last_stat_check: std::time::Instant::now(),
                    bytecode: std::sync::Arc::clone(&arc_bc),
                },
            );
        }
        Some(arc_bc)
    })
}
