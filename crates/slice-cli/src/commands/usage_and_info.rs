//! CLI help banner (`slice --help`) and runtime architecture summary (`slice info`).

use anyhow::Result;
use slice_io::CompletionRingEngine;
use std::env;

pub(crate) fn print_usage() {
    println!(
        "slice (Nexus-JS) v{} - Production MicroVM-Grade Edge Runtime",
        env!("CARGO_PKG_VERSION")
    );
    println!();
    println!("Core Commands:");
    println!(
        "  slice init [dir] [--template=sveltekit]        Scaffold a new TypeScript Edge or SvelteKit 3 project"
    );
    println!(
        "  slice run [target] [--warm] [--watch]          Execute a script, folder app, .cas bundle, or slice.toml"
    );
    println!(
        "  slice check <target>                           Parse and syntax-check TypeScript/JavaScript without executing"
    );
    println!(
        "  slice test [target] [--filter=PAT] [--json]    Run Time-Travel Test Suite (< 5us VFS+KV rollback per test)"
    );
    println!(
        "  slice serve [target] [--port=N] [--smoke-test] Launch multi-tenant WinterTC HTTP MicroVM server"
    );
    println!(
        "  slice pack <input> <output.cas>                Pack a file or folder app into a Zero-Stat BLAKE3 .cas archive"
    );
    println!(
        "  slice compile <input> -o <binary>              Compile a script, folder app, or .cas into a standalone binary"
    );
    println!(
        "  slice inspect <bundle.cas|script.ts|dir>       Disassemble SLICEBC/SLICEBOA/SLICESNP bytecode and inspect CAS Radix table"
    );
    println!(
        "  slice mcp [--smoke-test] [--audit-egress=SEC]  Start built-in Model Context Protocol (MCP) JSON-RPC 2.0 server"
    );
    println!(
        "  slice repl                                     Start interactive SES-Locked TypeScript/JS shell"
    );
    println!(
        "  slice doctor                                   Audit host OS kernel io_uring/IOCP, MPK, Warm Pool, and OCap"
    );
    println!(
        "  slice microvm-bench [--tenants=1000] [--json]  Run 1,000-tenant MicroVM cold-start & density benchmark"
    );
    println!(
        "  slice verify-security                          Run 5-vector OCap, SES, and Fuel containment verification"
    );
    println!(
        "  slice info                                     Display runtime architecture and active kernel rings"
    );
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
    println!(
        "  TypeScript & JSX/TSX  : Oxc v0.152.0 AOT Compiler + Built-In SSR (`slice:jsx`) + Tree-Shaking"
    );
    println!(
        "  Execution Pipeline    : Tier-1 Precompiled Boa CodeBlock (`SLICEBC\\x01` Default) + Tier-2 SIMD + Tier-0 SLICESNP"
    );
    println!(
        "  Native Fast Intrinsics: FIPS-180-4 SHA-256 / RFC-2104 HMAC / SemVer / In-Memory VFS Slab"
    );
    println!(
        "  Security Architecture : SES Lockdown + TC39 OCap Compartments + Hardware PKEY/Cage"
    );
    Ok(())
}
