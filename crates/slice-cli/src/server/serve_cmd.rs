//! `slice serve` CLI entrypoint and shared multi-worker TCP accept loop.

use super::{load_tls_server_config, serve_connection_with_isolate, spawn_worker_isolate};
use crate::config::{
    ResolvedExecutionConfig, install_ctrlc_handler, is_known_slice_runtime_flag,
    match_non_empty_flag_path, match_non_empty_flag_value, parse_strict_u64,
    skip_runtime_flag_with_separate_value,
};
use anyhow::{Context, Result, bail};
use slice_cas::{MappedCasArchive, StaticAssetTable};
use slice_engine::{SliceIsolate, compile_source_to_bytecode};
use slice_sandbox::CapabilityToken;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct AcceptWorkerShared {
    listener: Arc<TcpListener>,
    bytecode: Arc<Vec<u8>>,
    cas_archive: Option<Arc<MappedCasArchive>>,
    tenant_dir: Option<Arc<PathBuf>>,
    static_table: Option<Arc<StaticAssetTable>>,
    tls_server_cfg: Option<Arc<rustls::ServerConfig>>,
    caps: CapabilityToken,
    audit_secrets: Arc<Vec<String>>,
    deterministic_seed: Option<u64>,
    fuel_limit: u64,
    mem_limit: usize,
    max_requests_opt: Option<usize>,
    req_counter: Arc<AtomicUsize>,
    shutdown_flag: Arc<AtomicBool>,
    wake_ip: IpAddr,
    bound_port: u16,
    workers: usize,
}

fn run_accept_worker_loop(initial_slot: u32, shared: &AcceptWorkerShared) {
    SliceIsolate::prewarm_boa_bundle_with_caps(&shared.bytecode, &shared.caps);
    let mut read_buf = vec![0u8; 65536];
    let mut write_buf = Vec::with_capacity(4096);
    let mut local_slot = initial_slot;
    let mut isolate = spawn_worker_isolate(
        local_slot,
        shared.deterministic_seed,
        &shared.caps,
        &shared.audit_secrets,
        shared.fuel_limit,
        shared.mem_limit,
    );

    while let Ok((stream, _)) = shared.listener.accept() {
        if shared.shutdown_flag.load(Ordering::Acquire) {
            break;
        }
        local_slot = local_slot.wrapping_add(1);
        let conn_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            serve_connection_with_isolate(
                stream,
                local_slot,
                &mut isolate,
                &shared.bytecode,
                shared.cas_archive.as_deref(),
                shared.tenant_dir.as_deref().map(PathBuf::as_path),
                shared.static_table.as_deref(),
                shared.fuel_limit,
                shared.mem_limit,
                &mut read_buf,
                &mut write_buf,
                shared
                    .max_requests_opt
                    .map(|max| (&*shared.req_counter, max)),
                shared.tls_server_cfg.as_ref(),
            )
        }));
        if conn_res.is_err() {
            read_buf.fill(0);
            write_buf.clear();
            isolate = spawn_worker_isolate(
                local_slot,
                shared.deterministic_seed,
                &shared.caps,
                &shared.audit_secrets,
                shared.fuel_limit,
                shared.mem_limit,
            );
        }
        if let Some(max) = shared.max_requests_opt
            && max > 0
            && shared.req_counter.load(Ordering::Acquire) >= max
        {
            if !shared.shutdown_flag.swap(true, Ordering::AcqRel) {
                wake_accept_workers(shared.wake_ip, shared.bound_port, shared.workers);
            }
            break;
        }
    }
}

fn wake_accept_workers(wake_ip: IpAddr, port: u16, workers: usize) {
    let target = SocketAddr::new(wake_ip, port);
    for _ in 0..workers {
        let _ = TcpStream::connect_timeout(&target, std::time::Duration::from_millis(50));
    }
}

pub(crate) fn cmd_serve(args: &[String]) -> Result<()> {
    let mut smoke_test = false;
    let mut max_requests_opt: Option<usize> = None;
    let mut tenant_dir_opt: Option<PathBuf> = None;
    let mut bind_host = String::from("0.0.0.0");
    let mut target_opt: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        if args[i] == "--" {
            if let Some(first_rem) = args.get(i + 1)
                && target_opt.is_none()
            {
                target_opt = Some(first_rem.clone());
            }
            break;
        }
        if let Some(h) = match_non_empty_flag_value(args, &mut i, "--host")? {
            bind_host = h.to_string();
            i += 1;
            continue;
        }
        if let Some(m_str) = match_non_empty_flag_value(args, &mut i, "--max-requests")? {
            let parsed = parse_strict_u64(m_str)
                .and_then(|n| usize::try_from(n).ok())
                .with_context(|| format!("Invalid --max-requests value: {:?}", m_str))?;
            max_requests_opt = Some(parsed);
            i += 1;
            continue;
        }
        if let Some(td) = match_non_empty_flag_path(args, &mut i, "--tenant-dir")? {
            tenant_dir_opt = Some(td);
            i += 1;
            continue;
        }
        if skip_runtime_flag_with_separate_value(args, &mut i)? {
            continue;
        }
        let arg = &args[i];
        if arg == "--smoke-test" || arg == "--once" {
            smoke_test = true;
        } else if !arg.starts_with('-') && target_opt.is_none() {
            target_opt = Some(arg.clone());
        } else if arg.starts_with('-') && !is_known_slice_runtime_flag(arg) {
            bail!("Unknown option for 'slice serve': {}", arg);
        } else if !arg.starts_with('-') {
            bail!(
                "Unexpected extra positional argument {:?} for 'slice serve'",
                arg
            );
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
const req = Slice.request();
Slice.respond(
    200,
    JSON.stringify({
        status: 200,
        runtime: "Slice-NexusJS",
        wintertc: true,
        method: req.method,
        path: req.path,
        tenant: req.tenantId,
    }),
    "application/json"
);
"#;
    let mut static_table = StaticAssetTable::new();
    let mut cas_archive_arc: Option<Arc<MappedCasArchive>> = None;
    let bytecode = Arc::new(if let Some(target) = &target_opt {
        let p = Path::new(target);
        if target.ends_with(".cas") {
            let archive = MappedCasArchive::open(p)?;
            if let Some(toml_str) = archive.get_manifest_toml() {
                let cas_base_dir = p
                    .parent()
                    .filter(|d| !d.as_os_str().is_empty())
                    .or_else(|| Some(Path::new(".")));
                exec_cfg.apply_manifest_toml(toml_str, cas_base_dir)?;
            }
            static_table = StaticAssetTable::from_cas_archive(&archive);
            let entry = archive
                .entrypoint_blob()
                .context("Missing CAS entrypoint")?;
            let bc_vec = archive.get_bytecode_slice(entry).to_vec();
            cas_archive_arc = Some(Arc::new(archive));
            bc_vec
        } else {
            exec_cfg.apply_target_path_manifest(p)?;
            let entry_file = slice_cas::resolve_project_entrypoint(p, None)?;
            let raw_entry = std::fs::read_to_string(&entry_file)
                .with_context(|| format!("Failed to read {}", entry_file.display()))?;
            exec_cfg.apply_inline_directives(&raw_entry, entry_file.parent())?;
            let bundled = slice_cas::bundle_entrypoint_with_deps(&entry_file)?;
            compile_source_to_bytecode(&bundled)
        }
    } else if let Some(archive) = embedded_archive {
        if let Some(toml_str) = archive.get_manifest_toml() {
            exec_cfg.apply_manifest_toml(toml_str, None)?;
        }
        static_table = StaticAssetTable::from_cas_archive(&archive);
        if let Some(entry) = archive.entrypoint_blob() {
            let bc_vec = archive.get_bytecode_slice(entry).to_vec();
            cas_archive_arc = Some(Arc::new(archive));
            bc_vec
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
    let audit_secrets = Arc::new(exec_cfg.audit_secrets);

    let tls_server_cfg = match (&tls_cert_opt, &tls_key_opt) {
        (Some(cert_path), Some(key_path)) => Some(load_tls_server_config(cert_path, key_path)?),
        (Some(_), None) => bail!("--tls-cert specified without --tls-key"),
        (None, Some(_)) => bail!("--tls-key specified without --tls-cert"),
        (None, None) => None,
    };

    if let Some(static_dir) = &static_dir_opt
        && (!is_cas_archive_mode || has_cli_static_override || static_dir.is_dir())
    {
        static_table.extend_from_directory(static_dir)?;
    }
    let static_table_arc = (!static_table.is_empty()).then(|| Arc::new(static_table));

    if smoke_test {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let bound_port = listener.local_addr()?.port();
        let bc_clone = Arc::clone(&bytecode);
        let cas_clone = cas_archive_arc.as_ref().map(Arc::clone);
        let st_clone = static_table_arc.as_ref().map(Arc::clone);
        let caps_clone = caps.clone();
        let secrets_clone = Arc::clone(&audit_secrets);

        let server_thread = std::thread::spawn(move || -> Result<()> {
            SliceIsolate::prewarm_boa_bundle_with_caps(&bc_clone, &caps_clone);
            let mut read_buf = vec![0u8; 65536];
            let mut write_buf = Vec::with_capacity(4096);
            if let Ok((stream, _)) = listener.accept() {
                let mut isolate = spawn_worker_isolate(
                    1,
                    deterministic_seed,
                    &caps_clone,
                    &secrets_clone,
                    fuel_limit,
                    mem_limit,
                );
                let _ = serve_connection_with_isolate(
                    stream,
                    1,
                    &mut isolate,
                    &bc_clone,
                    cas_clone.as_deref(),
                    None,
                    st_clone.as_deref(),
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
        let ok_count = resp.matches("HTTP/1.1 200 OK").count();
        if ok_count != 2 || resp.contains(r#"{"error":"Uncaught "#) {
            bail!(
                "Server smoke test failed: expected 2 HTTP/1.1 200 OK responses, got {}:\n{}",
                ok_count,
                resp
            );
        }
        return Ok(());
    }

    install_ctrlc_handler();

    let listener = Arc::new(TcpListener::bind((bind_host.as_str(), port))?);
    let bound_addr = listener.local_addr()?;
    let bound_port = bound_addr.port();
    let wake_ip = if bound_addr.ip().is_unspecified() {
        if bound_addr.is_ipv6() {
            IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)
        } else {
            IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
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

    let shared = Arc::new(AcceptWorkerShared {
        listener,
        bytecode,
        cas_archive: cas_archive_arc,
        tenant_dir: tenant_dir_opt.map(Arc::new),
        static_table: static_table_arc,
        tls_server_cfg,
        caps,
        audit_secrets,
        deterministic_seed,
        fuel_limit,
        mem_limit,
        max_requests_opt,
        req_counter: Arc::new(AtomicUsize::new(0)),
        shutdown_flag: Arc::new(AtomicBool::new(false)),
        wake_ip,
        bound_port,
        workers,
    });

    let mut handles = Vec::with_capacity(workers.saturating_sub(1));
    for worker_idx in 1..workers {
        let worker_shared = Arc::clone(&shared);
        handles.push(std::thread::spawn(move || {
            run_accept_worker_loop((worker_idx as u32) << 20, &worker_shared);
        }));
    }

    run_accept_worker_loop(0, &shared);
    for h in handles {
        let _ = h.join();
    }
    Ok(())
}
