//! `slice` (Nexus-JS): Production Next-Generation MicroVM-Grade Edge Runtime CLI
//!
//! Production Commands:
//! - `slice init [dir]`
//! - `slice run [file.ts|file.tsx|file.js|file.jsx|archive.cas] [--allow-read=/path] [--allow-net=host] [--allow-env=KEY=VAL] [--quiet] [--metrics]`
//! - `slice pack <entry_or_dir> <output.cas> [--entry=./index.ts]`
//! - `slice compile <entry_or_dir> -o <standalone_binary>`
//! - `slice inspect <archive.cas|script.ts>`
//! - `slice serve [archive.cas|script.ts|script.tsx] [--port=3000] [--workers=N] [--tenant-dir=<dir>] [--smoke-test]`
//! - `slice mcp [--smoke-test] [--audit-egress=<secret>]`
//! - `slice repl`
//! - `slice doctor`
//! - `slice microvm-bench [--tenants=1000] [--json]`
//! - `slice verify-security`
//! - `slice info`

mod config_and_watch;
mod http_server;
mod mcp_and_repl;
mod test_runner;

#[cfg(test)]
mod cli_tests;

use crate::config_and_watch::{
    apply_dotenv_file_to_caps_and_dlp, build_cas_archive_with_static, find_project_manifest_path,
    load_manifest_or_default, merge_manifest_toml_into_caps, merge_manifest_toml_into_caps_and_dlp,
    OsNativeFileWatcher,
};
use crate::http_server::cmd_serve;
use crate::mcp_and_repl::{
    cmd_doctor, cmd_mcp, cmd_microvm_bench, cmd_repl, cmd_verify_security,
};
use crate::test_runner::cmd_test;
use anyhow::{bail, Context, Result};
use slice_cas::{embed_cas_into_executable, MappedCasArchive};
use slice_engine::{compile_source_to_bytecode, disassemble_bytecode, SliceIsolate};
use slice_io::CompletionRingEngine;
use slice_sandbox::SliceManifest;
use std::env;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn main() -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        let builder = std::thread::Builder::new()
            .name("slice-main".to_string())
            .stack_size(16 * 1024 * 1024);
        let handle = builder.spawn(cli_entrypoint)?;
        match handle.join() {
            Ok(res) => res,
            Err(_) => bail!("Slice main thread panicked"),
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        cli_entrypoint()
    }
}

fn cli_entrypoint() -> Result<()> {
    // 1. Check if this binary is a self-contained `SLICEEXE` compiled application (`slice compile`)
    if let Ok(Some(embedded_cas)) = MappedCasArchive::open_embedded_self() {
        return run_embedded_standalone(embedded_cas);
    }

    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        // If `slice.toml` exists in the current directory, show a helpful hint alongside usage
        print_usage();
        return Ok(());
    }

    match args[1].as_str() {
        "init" => cmd_init(&args[2..]),
        "run" => cmd_run(&args[2..]),
        "test" => cmd_test(&args[2..]),
        "pack" => cmd_pack(&args[2..]),
        "compile" => cmd_compile(&args[2..]),
        "inspect" | "check" => cmd_inspect(&args[2..]),
        "serve" => cmd_serve(&args[2..]),
        "mcp" => cmd_mcp(&args[2..]),
        "repl" => cmd_repl(),
        "doctor" => cmd_doctor(),
        "microvm-bench" => cmd_microvm_bench(&args[2..]),
        "verify-security" => cmd_verify_security(),
        "info" | "--version" | "-v" => cmd_info(),
        "help" | "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        _ => {
            // Allow `slice <file.ts>` shorthand
            cmd_run(&args[1..])
        }
    }
}

fn run_embedded_standalone(archive: MappedCasArchive) -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let quiet = args.iter().any(|a| a == "--quiet" || a == "-q");
    let print_metrics = args.iter().any(|a| a == "--metrics");
    let warm = args.iter().any(|a| a == "--warm");
    if warm {
        SliceIsolate::prewarm_boa_pool(1);
    }

    let (mut caps, fuel_limit, mem_limit) = load_manifest_or_default();
    if let Some(toml_str) = archive.get_manifest_toml() {
        merge_manifest_toml_into_caps(toml_str, &mut caps);
    }
    let mut isolate = SliceIsolate::spawn("standalone-exe", 1, caps, fuel_limit, mem_limit);
    let entry = archive
        .entrypoint_blob()
        .context("Embedded CAS archive has no entrypoint blob")?;
    let bc = archive.get_bytecode_slice(entry);
    let metrics = isolate.execute_bytecode(bc, Some(&archive))?;

    if !quiet {
        for line in &metrics.stdout {
            println!("{}", line);
        }
    }
    if print_metrics {
        println!(
            "[Slice Standalone] Tier: {:?} | Instantiate: {:.2} us | Exec: {:.2} us | Total: {:.2} us | Memory: {} B | Stat Syscalls: 0",
            metrics.highest_tier_reached,
            metrics.instantiate_time_ns as f64 / 1_000.0,
            metrics.execution_time_ns as f64 / 1_000.0,
            metrics.total_time_ns as f64 / 1_000.0,
            metrics.memory_used_bytes
        );
    }
    Ok(())
}

fn print_usage() {
    println!("slice (Nexus-JS) v0.2.0 - Production MicroVM-Grade Edge Runtime");
    println!();
    println!("Core Commands:");
    println!("  slice init [dir]                               Scaffold a new TypeScript Edge project with slice.toml");
    println!("  slice run [target] [--warm] [--watch]          Execute a script, folder app, .cas bundle, or slice.toml");
    println!("  slice test [target] [--filter=PAT] [--json]    Run Time-Travel Test Suite (< 5us VFS+KV rollback per test)");
    println!("  slice serve [target] [--port=N] [--smoke-test] Launch multi-tenant WinterTC HTTP MicroVM server");
    println!("  slice pack <input> <output.cas>                Pack a file or folder app into a Zero-Stat BLAKE3 .cas archive");
    println!("  slice compile <input> -o <binary>              Compile a script, folder app, or .cas into a standalone binary");
    println!("  slice inspect <bundle.cas|script.ts|dir>       Disassemble SLICESNP/SLICEBOA bytecode and inspect CAS Radix table");
    println!("  slice mcp [--smoke-test] [--audit-egress=SEC]  Start built-in Model Context Protocol (MCP) JSON-RPC 2.0 server");
    println!("  slice repl                                     Start interactive SES-locked TypeScript/JS shell");
    println!("  slice doctor                                   Audit host OS kernel io_uring/IOCP, MPK, Warm Pool, and OCap");
    println!("  slice microvm-bench [--tenants=1000] [--json]  Run 1,000-tenant MicroVM cold-start & density benchmark");
    println!("  slice verify-security                          Run 5-vector OCap, SES, and Fuel containment verification");
    println!("  slice info                                     Display runtime architecture and active kernel rings");
}

fn cmd_info() -> Result<()> {
    let io_engine = CompletionRingEngine::new(64, true)?;
    println!(
        "slice (Nexus-JS) v0.2.0 [{}-{}]",
        env::consts::OS,
        env::consts::ARCH
    );
    println!(
        "  I/O Completion Engine : {}",
        io_engine.status.backend_name
    );
    println!(
        "  Zero-Stat CAS Format  : SLICE_CAS v{} (64-byte aligned BLAKE3 mmap + SLICEEXE)",
        slice_cas::CAS_VERSION
    );
    println!("  TypeScript & JSX/TSX  : Oxc v0.152.0 AOT Compiler + Built-In SSR (`slice:jsx`) + Tree-Shaking");
    println!("  Execution Pipeline    : Tier-0 SLICESNP -> Tier-1 Zero-Copy SLICEBOA (Warm Pool) -> Tier-2 SIMD");
    println!("  Native Fast Intrinsics: FIPS-180-4 SHA-256 / RFC-2104 HMAC / SemVer / In-Memory VFS Slab");
    println!(
        "  Security Architecture : SES Lockdown + TC39 OCap Compartments + Hardware PKEY/Cage"
    );
    Ok(())
}

fn cmd_init(args: &[String]) -> Result<()> {
    let target_dir = if args.is_empty() {
        PathBuf::from(".")
    } else {
        PathBuf::from(&args[0])
    };

    std::fs::create_dir_all(target_dir.join("src"))?;
    let project_name = target_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("slice-edge-app");

    let manifest_path = target_dir.join("slice.toml");
    if !manifest_path.exists() {
        std::fs::write(
            &manifest_path,
            SliceManifest::default_template(project_name),
        )?;
    }

    let entry_path = target_dir.join("src").join("index.ts");
    if !entry_path.exists() {
        let sample_ts = r#"// Slice (Nexus-JS) Edge MicroVM Entrypoint
interface TenantContext {
    service: string;
    version: number;
}

function fib(n: number): number {
    if (n <= 1) return n;
    return fib(n - 1) + fib(n - 2);
}

const stage: string = env("SERVICE_STAGE");
const f: number = fib(30);
const route_response: string = WinterTC.route("GET", "https://edge.slice.local/api/v1/status");
console.log(stage);
console.log(f);
console.log(route_response);
"#;
        std::fs::write(&entry_path, sample_ts)?;
    }

    println!(
        "Initialized Slice (Nexus-JS) project '{}' in {}",
        project_name,
        target_dir.display()
    );
    println!("  Created {}", manifest_path.display());
    println!("  Created {}", entry_path.display());
    println!("Next steps:");
    println!("  slice run                        (Execute with slice.toml OCap policy)");
    println!("  slice compile ./src/index.ts -o app  (Build standalone zero-dependency binary)");
    println!("  slice serve --port=3000          (Start multi-tenant HTTP MicroVM server)");
    Ok(())
}

pub(crate) fn cmd_run(args: &[String]) -> Result<()> {
    let (mut caps, mut fuel_limit, mut mem_limit) = load_manifest_or_default();
    let mut quiet = false;
    let mut print_metrics = false;
    let mut prewarm_pool = false;
    let mut deterministic_seed: Option<u64> = None;
    let mut audit_secrets: Vec<String> = Vec::new();
    let mut env_files: Vec<PathBuf> = Vec::new();
    let mut watch_mode = false;
    let mut watch_once = false;
    let mut target_opt: Option<String> = None;

    for arg in args {
        if let Some(prefix) = arg.strip_prefix("--allow-read=") {
            caps = caps.with_read_prefix(prefix);
        } else if let Some(prefix) = arg.strip_prefix("--allow-write=") {
            caps = caps.with_write_prefix(prefix);
        } else if let Some(host) = arg.strip_prefix("--allow-net=") {
            for h in host.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                caps = caps.with_net_host(h);
            }
        } else if let Some(kv) = arg.strip_prefix("--allow-env=") {
            if let Some((k, v)) = kv.split_once('=') {
                caps = caps.with_env(k, v);
            }
        } else if let Some(ef) = arg.strip_prefix("--env-file=") {
            if !ef.is_empty() {
                env_files.push(PathBuf::from(ef));
            }
        } else if let Some(sec) = arg.strip_prefix("--audit-egress=") {
            if !sec.is_empty() {
                audit_secrets.push(sec.to_string());
            }
        } else if let Some(f) = arg.strip_prefix("--fuel=") {
            fuel_limit = f.parse().unwrap_or(fuel_limit);
        } else if let Some(m) = arg.strip_prefix("--memory=") {
            mem_limit = m.parse().unwrap_or(mem_limit);
        } else if let Some(seed_str) = arg.strip_prefix("--deterministic=") {
            deterministic_seed = Some(seed_str.parse().unwrap_or(42));
        } else if arg == "--deterministic" {
            deterministic_seed = Some(42);
        } else if arg == "--quiet" || arg == "-q" {
            quiet = true;
        } else if arg == "--metrics" {
            print_metrics = true;
        } else if arg == "--warm" {
            prewarm_pool = true;
        } else if arg == "--watch" {
            watch_mode = true;
        } else if arg == "--watch-once" {
            watch_mode = true;
            watch_once = true;
        } else if !arg.starts_with('-') && target_opt.is_none() {
            target_opt = Some(arg.clone());
        }
    }

    for ef in &env_files {
        apply_dotenv_file_to_caps_and_dlp(ef, &mut caps, &mut audit_secrets)?;
    }

    let target = match target_opt {
        Some(t) => t,
        None => {
            let manifest_path = Path::new("slice.toml");
            if manifest_path.exists() {
                let raw = std::fs::read_to_string(manifest_path)?;
                let m = SliceManifest::parse_toml(&raw);
                m.entrypoint
            } else {
                bail!("Missing target script/archive path (and no slice.toml found in current directory)");
            }
        }
    };

    let path = Path::new(&target);
    let mut bundled_for_sourcemap: Option<String> = None;
    let metrics = if target.ends_with(".cas") {
        let archive = MappedCasArchive::open(path)?;
        if let Some(toml_str) = archive.get_manifest_toml() {
            merge_manifest_toml_into_caps_and_dlp(
                toml_str,
                path.parent(),
                &mut caps,
                &mut audit_secrets,
            );
        }
        let entry = archive
            .entrypoint_blob()
            .context("CAS archive has no valid entrypoint blob")?;
        let bytecode = archive.get_bytecode_slice(entry);
        let mut isolate = SliceIsolate::spawn("tenant-cli", 1, caps.clone(), fuel_limit, mem_limit);
        isolate.set_deterministic_seed(deterministic_seed);
        for sec in &audit_secrets {
            isolate.add_egress_audit_secret(sec.clone());
        }
        if prewarm_pool {
            SliceIsolate::prewarm_boa_bundle(bytecode);
            let snap = isolate.checkpoint();
            let _ = isolate.execute_bytecode(bytecode, Some(&archive));
            isolate.restore(snap);
            isolate.reset_for_request("tenant-cli", 1, fuel_limit, mem_limit);
        }
        isolate.execute_bytecode(bytecode, Some(&archive))?
    } else {
        if let Some(manifest_file) = find_project_manifest_path(path) {
            if let Ok(raw) = std::fs::read_to_string(&manifest_file) {
                merge_manifest_toml_into_caps_and_dlp(
                    &raw,
                    manifest_file.parent(),
                    &mut caps,
                    &mut audit_secrets,
                );
            }
        }
        let entry_file = slice_cas::resolve_project_entrypoint(path, None)?;
        let bundled_source = slice_cas::bundle_entrypoint_with_deps(&entry_file)?;
        let bytecode = compile_source_to_bytecode(&bundled_source);
        bundled_for_sourcemap = Some(bundled_source);
        let mut isolate = SliceIsolate::spawn("tenant-cli", 1, caps.clone(), fuel_limit, mem_limit);
        isolate.set_deterministic_seed(deterministic_seed);
        for sec in &audit_secrets {
            isolate.add_egress_audit_secret(sec.clone());
        }
        if prewarm_pool {
            SliceIsolate::prewarm_boa_bundle(&bytecode);
            let snap = isolate.checkpoint();
            let _ = isolate.execute_bytecode(&bytecode, None);
            isolate.restore(snap);
            isolate.reset_for_request("tenant-cli", 1, fuel_limit, mem_limit);
        }
        isolate.execute_bytecode(&bytecode, None)?
    };

    if metrics.return_value.starts_with("Uncaught ") {
        let mapped = match bundled_for_sourcemap.as_deref() {
            Some(b) => slice_cas::map_bundled_error_location(b, &metrics.return_value),
            None => metrics.return_value.clone(),
        };
        bail!("{}", mapped);
    }

    if !quiet {
        for line in &metrics.stdout {
            println!("{}", line);
        }
    }

    if print_metrics {
        println!(
            "[Slice Telemetry] Tier: {:?} | Instantiate: {:.2} us | Exec: {:.2} us | Total: {:.2} us | Memory: {} B | Stat Syscalls: {}",
            metrics.highest_tier_reached,
            metrics.instantiate_time_ns as f64 / 1_000.0,
            metrics.execution_time_ns as f64 / 1_000.0,
            metrics.total_time_ns as f64 / 1_000.0,
            metrics.memory_used_bytes,
            metrics.os_stat_syscalls
        );
    }

    if watch_mode && !target.ends_with(".cas") {
        let mut watcher = OsNativeFileWatcher::new(path);
        while watcher.wait_for_change(watch_once) {
            let reload_start = Instant::now();
            let entry_file = slice_cas::resolve_project_entrypoint(path, None)?;
            let bundled_source = slice_cas::bundle_entrypoint_with_deps(&entry_file)?;
            let bytecode = compile_source_to_bytecode(&bundled_source);
            let mut isolate =
                SliceIsolate::spawn("tenant-cli-watch", 1, caps.clone(), fuel_limit, mem_limit);
            isolate.set_deterministic_seed(deterministic_seed);
            for sec in &audit_secrets {
                isolate.add_egress_audit_secret(sec.clone());
            }
            let reload_metrics = isolate.execute_bytecode(&bytecode, None)?;
            let reload_ms = reload_start.elapsed().as_secs_f64() * 1_000.0;
            if !quiet {
                for line in &reload_metrics.stdout {
                    println!("{}", line);
                }
                println!("[Slice Watch] Reloaded in {:.2} ms", reload_ms);
            }
            if watch_once {
                break;
            }
        }
    }

    Ok(())
}

fn cmd_pack(args: &[String]) -> Result<()> {
    if args.len() < 2 {
        bail!("Usage: slice pack <input_file_or_dir> <output.cas> [--entry=./index.ts] [--static=./public]");
    }
    let input_path = Path::new(&args[0]);
    let output_path = Path::new(&args[1]);
    let mut entrypoint = String::from("./index.ts");
    let mut static_dir_opt: Option<PathBuf> = None;

    for arg in &args[2..] {
        if let Some(e) = arg.strip_prefix("--entry=") {
            entrypoint = e.to_string();
        } else if let Some(sd) = arg.strip_prefix("--static=") {
            static_dir_opt = Some(PathBuf::from(sd));
        }
    }

    let start = Instant::now();
    let header = build_cas_archive_with_static(
        input_path,
        output_path,
        &entrypoint,
        static_dir_opt.as_deref(),
    )?;
    let elapsed_ms = start.elapsed().as_secs_f64() * 1_000.0;
    let file_size = std::fs::metadata(output_path)?.len();

    println!(
        "Packed {} unique BLAKE3 module blobs into {} ({} bytes) in {:.2} ms",
        header.blobs.len(),
        output_path.display(),
        file_size,
        elapsed_ms
    );
    Ok(())
}

fn cmd_compile(args: &[String]) -> Result<()> {
    if args.is_empty() {
        bail!("Usage: slice compile <input_script_or_dir_or_cas> [-o <output_binary>] [--entry=./index.ts] [--static=./public]");
    }
    let input_path = Path::new(&args[0]);
    let mut output_bin: Option<PathBuf> = None;
    let mut entrypoint = String::from("./index.ts");
    let mut static_dir_opt: Option<PathBuf> = None;

    let mut i = 1;
    while i < args.len() {
        if args[i] == "-o" || args[i] == "--output" {
            if i + 1 < args.len() {
                output_bin = Some(PathBuf::from(&args[i + 1]));
                i += 2;
                continue;
            }
        } else if let Some(out) = args[i].strip_prefix("-o=") {
            output_bin = Some(PathBuf::from(out));
        } else if let Some(e) = args[i].strip_prefix("--entry=") {
            entrypoint = e.to_string();
        } else if let Some(sd) = args[i].strip_prefix("--static=") {
            static_dir_opt = Some(PathBuf::from(sd));
        }
        i += 1;
    }

    let default_name = input_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("slice_app");
    let mut out_path = output_bin.unwrap_or_else(|| PathBuf::from(default_name));
    if cfg!(windows) && out_path.extension().is_none() {
        out_path.set_extension("exe");
    }

    let start = Instant::now();
    let temp_cas = env::temp_dir().join("slice_compile_staging.cas");
    let cas_to_embed = if input_path.extension().and_then(|e| e.to_str()) == Some("cas") {
        input_path.to_path_buf()
    } else {
        build_cas_archive_with_static(
            input_path,
            &temp_cas,
            &entrypoint,
            static_dir_opt.as_deref(),
        )?;
        temp_cas.clone()
    };

    let current_exe = env::current_exe()?;
    let total_bytes = embed_cas_into_executable(&current_exe, &cas_to_embed, &out_path)?;
    if temp_cas.exists() {
        let _ = std::fs::remove_file(&temp_cas);
    }

    println!(
        "Compiled standalone MicroVM executable {} ({:.2} MB) in {:.2} ms",
        out_path.display(),
        total_bytes as f64 / (1024.0 * 1024.0),
        start.elapsed().as_secs_f64() * 1_000.0
    );
    Ok(())
}

fn cmd_inspect(args: &[String]) -> Result<()> {
    if args.is_empty() {
        bail!("Usage: slice inspect <archive.cas | script.ts>");
    }
    let path = Path::new(&args[0]);
    if path.extension().and_then(|e| e.to_str()) == Some("cas") {
        let archive = MappedCasArchive::open(path)?;
        println!("=========================================================================");
        println!(
            "  SLICE_CAS v{} ARCHIVE INSPECTOR: {}",
            archive.header.version,
            path.display()
        );
        println!("=========================================================================");
        println!(
            "  Entrypoint Specifier : {}",
            archive.header.entrypoint_specifier
        );
        println!(
            "  Entrypoint BLAKE3    : {}",
            archive.header.entrypoint_hash
        );
        println!("  Unique Module Blobs  : {}", archive.header.blobs.len());
        println!(
            "  Radix Index Entries  : {}",
            archive.header.radix_resolution_map.len()
        );
        println!("-------------------------------------------------------------------------");
        let mut blobs: Vec<_> = archive.header.blobs.values().collect();
        blobs.sort_by_key(|b| b.offset);
        for blob in blobs.iter().take(15) {
            println!(
                "  [Blob {}..{}] Specifier: {} | SrcOffset: {} ({} B) | BytecodeOffset: {} ({} B)",
                &blob.hash_hex[..12],
                &blob.hash_hex[56..],
                blob.original_specifier,
                blob.offset,
                blob.source_len,
                blob.bytecode_offset,
                blob.bytecode_len
            );
            let bc = archive.get_bytecode_slice(blob);
            if let Ok(disasm) = disassemble_bytecode(bc) {
                for line in disasm {
                    println!("      {}", line);
                }
            }
        }
        if blobs.len() > 15 {
            println!("  ... ({} additional blobs omitted)", blobs.len() - 15);
        }
        println!("=========================================================================");
    } else {
        let entry_file = slice_cas::resolve_project_entrypoint(path, None)?;
        let bundled = slice_cas::bundle_entrypoint_with_deps(&entry_file)?;
        let bc = compile_source_to_bytecode(&bundled);
        let disasm = disassemble_bytecode(&bc)?;
        println!("=========================================================================");
        println!(
            "  SLICESNP BYTECODE DISASSEMBLY: {} ({} bytes)",
            path.display(),
            bc.len()
        );
        println!("=========================================================================");
        for line in disasm {
            println!("  {}", line);
        }
        println!("=========================================================================");
    }
    Ok(())
}
