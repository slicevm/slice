use crate::config::build_cas_archive_with_static;
use anyhow::{bail, Result};
use slice_cas::{embed_cas_into_executable, MappedCasArchive};
use slice_engine::{compile_source_to_bytecode, disassemble_bytecode};
use slice_io::CompletionRingEngine;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

static TEMP_CAS_SEQ: AtomicU64 = AtomicU64::new(1);

/// Generates a collision-resistant, non-predictable temporary `.cas` staging path (`CLI-SEC-02`)
/// combining `prefix`, process ID, monotonic sequence counter, and nanosecond timestamp,
/// and removes any pre-existing symlink at that path.
pub(crate) fn unique_temp_cas_path(prefix: &str) -> Result<PathBuf> {
    let pid = std::process::id();
    let seq = TEMP_CAS_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let candidate = env::temp_dir().join(format!("{}_{}_{}_{}.cas", prefix, pid, seq, nanos));
    if let Ok(meta) = std::fs::symlink_metadata(&candidate) {
        if meta.file_type().is_symlink() {
            bail!(
                "Refusing to write staging CAS archive to pre-existing symlink: {}",
                candidate.display()
            );
        }
    }
    Ok(candidate)
}

pub(crate) fn print_usage() {
    println!(
        "slice (Nexus-JS) v{} - Production MicroVM-Grade Edge Runtime",
        env!("CARGO_PKG_VERSION")
    );
    println!();
    println!("Core Commands:");
    println!("  slice init [dir] [--template=sveltekit]        Scaffold a new TypeScript Edge or SvelteKit 3 project");
    println!("  slice run [target] [--warm] [--watch]          Execute a script, folder app, .cas bundle, or slice.toml");
    println!("  slice check <target>                           Parse and syntax-check TypeScript/JavaScript without executing");
    println!("  slice test [target] [--filter=PAT] [--json]    Run Time-Travel Test Suite (< 5us VFS+KV rollback per test)");
    println!("  slice serve [target] [--port=N] [--smoke-test] Launch multi-tenant WinterTC HTTP MicroVM server");
    println!("  slice pack <input> <output.cas>                Pack a file or folder app into a Zero-Stat BLAKE3 .cas archive");
    println!("  slice compile <input> -o <binary>              Compile a script, folder app, or .cas into a standalone binary");
    println!("  slice inspect <bundle.cas|script.ts|dir>       Disassemble SLICEBC/SLICEBOA/SLICESNP bytecode and inspect CAS Radix table");
    println!("  slice mcp [--smoke-test] [--audit-egress=SEC]  Start built-in Model Context Protocol (MCP) JSON-RPC 2.0 server");
    println!("  slice repl                                     Start interactive SES-Locked TypeScript/JS shell");
    println!("  slice doctor                                   Audit host OS kernel io_uring/IOCP, MPK, Warm Pool, and OCap");
    println!("  slice microvm-bench [--tenants=1000] [--json]  Run 1,000-tenant MicroVM cold-start & density benchmark");
    println!("  slice verify-security                          Run 5-vector OCap, SES, and Fuel containment verification");
    println!("  slice info                                     Display runtime architecture and active kernel rings");
}

pub(crate) fn cmd_info() -> Result<()> {
    let io_engine = CompletionRingEngine::new(64, true)?;
    println!(
        "slice (Nexus-JS) v{} [{}-{}]",
        env!("CARGO_PKG_VERSION"),
        env::consts::OS,
        env::consts::ARCH
    );
    println!(
        "  I/O Completion Engine : {}",
        io_engine.status.backend_name
    );
    println!(
        "  Zero-Stat CAS Format  : SLICE_CAS v{} (64-byte aligned BLAKE3 mmap + SLICEEXE + SLICEBC\\x01)",
        slice_cas::CAS_VERSION
    );
    println!("  TypeScript & JSX/TSX  : Oxc v0.152.0 AOT Compiler + Built-In SSR (`slice:jsx`) + Tree-Shaking");
    println!("  Execution Pipeline    : Tier-1 Precompiled Boa CodeBlock (`SLICEBC\\x01` Default) + Tier-2 SIMD + Tier-0 SLICESNP");
    println!("  Native Fast Intrinsics: FIPS-180-4 SHA-256 / RFC-2104 HMAC / SemVer / In-Memory VFS Slab");
    println!(
        "  Security Architecture : SES Lockdown + TC39 OCap Compartments + Hardware PKEY/Cage"
    );
    Ok(())
}

pub(crate) fn cmd_pack(args: &[String]) -> Result<()> {
    if args.len() < 2 {
        bail!("Usage: slice pack <input_file_or_dir> <output.cas> [--entry=./index.ts] [--static=./public]");
    }
    let input_path = Path::new(&args[0]);
    let output_path = Path::new(&args[1]);
    let mut entrypoint = String::from("./index.ts");
    let mut static_dir_opt: Option<PathBuf> = None;

    let mut exper_override: Option<bool> = None;

    for arg in &args[2..] {
        if let Some(e) = arg.strip_prefix("--entry=") {
            entrypoint = e.to_string();
        } else if let Some(sd) = arg.strip_prefix("--static=") {
            static_dir_opt = Some(PathBuf::from(sd));
        } else if arg == "--boacas" || arg == "--exper-boacas" {
            exper_override = Some(true);
        } else if arg == "--no-boacas" || arg == "--no-exper-boacas" {
            exper_override = Some(false);
        } else if let Some(b_str) = arg
            .strip_prefix("--boacas=")
            .or_else(|| arg.strip_prefix("--exper-boacas="))
        {
            exper_override = slice_sandbox::parse_bool_setting(b_str);
        } else if let Some(t_str) = arg.strip_prefix("--tier=") {
            if let Some(t) = slice_sandbox::parse_tier_setting(t_str) {
                slice_engine::set_tier_override(t);
            }
        }
    }
    if let Some(ov) = exper_override {
        slice_engine::set_exper_boacas_override(Some(ov));
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

pub(crate) fn cmd_compile(args: &[String]) -> Result<()> {
    if args.is_empty() {
        bail!("Usage: slice compile <input_script_or_dir_or_cas> [-o <output_binary>] [--entry=./index.ts] [--static=./public]");
    }
    let input_path = Path::new(&args[0]);
    let mut output_bin: Option<PathBuf> = None;
    let mut entrypoint = String::from("./index.ts");
    let mut static_dir_opt: Option<PathBuf> = None;

    let mut exper_override: Option<bool> = None;

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
        } else if args[i] == "--boacas" || args[i] == "--exper-boacas" {
            exper_override = Some(true);
        } else if args[i] == "--no-boacas" || args[i] == "--no-exper-boacas" {
            exper_override = Some(false);
        } else if let Some(b_str) = args[i]
            .strip_prefix("--boacas=")
            .or_else(|| args[i].strip_prefix("--exper-boacas="))
        {
            exper_override = slice_sandbox::parse_bool_setting(b_str);
        } else if let Some(t_str) = args[i].strip_prefix("--tier=") {
            if let Some(t) = slice_sandbox::parse_tier_setting(t_str) {
                slice_engine::set_tier_override(t);
            }
        }
        i += 1;
    }
    if let Some(ov) = exper_override {
        slice_engine::set_exper_boacas_override(Some(ov));
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
    // CLI-SEC-02: Use unique per-invocation staging path in temp_dir
    let temp_cas = unique_temp_cas_path("slice_compile_staging")?;
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

    let current_exe = env::current_exe();
    let embed_res = match current_exe {
        Ok(exe) => embed_cas_into_executable(&exe, &cas_to_embed, &out_path),
        Err(e) => Err(e.into()),
    };
    if temp_cas.exists() {
        let _ = std::fs::remove_file(&temp_cas);
    }
    let total_bytes = embed_res?;

    println!(
        "Compiled standalone MicroVM executable {} ({:.2} MB) in {:.2} ms",
        out_path.display(),
        total_bytes as f64 / (1024.0 * 1024.0),
        start.elapsed().as_secs_f64() * 1_000.0
    );
    Ok(())
}

pub(crate) fn cmd_inspect(args: &[String]) -> Result<()> {
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
        if let Some(ref lf_hash) = archive.header.lockfile_blake3 {
            println!("  Lockfile BLAKE3      : {}", lf_hash);
        }
        if !archive.header.resolved_dependencies.is_empty() {
            let deps_str = archive
                .header
                .resolved_dependencies
                .iter()
                .map(|(k, v)| format!("{}@{}", k, v))
                .collect::<Vec<_>>()
                .join(", ");
            println!("  Resolved NPM Deps    : {}", deps_str);
        }
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
