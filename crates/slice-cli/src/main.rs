//! `slice` (Nexus-JS): Production Next-Generation MicroVM-Grade Edge Runtime CLI
//!
//! Domain Architecture:
//! - [`commands`] — CLI subcommands (`init`, `run`, `pack`, `compile`, `inspect`, `test`, `info`).
//! - [`config`] — Unified configuration (`slice.toml`, `.env`, inline directives, `SLICE_*`), CAS packager, and OS-native file watcher.
//! - [`server`] — Multi-threaded WinterTC HTTP/1.1 keep-alive, SSE, WebSocket, TLS 1.3 (`rustls`), and zero-downtime `.cas` hot-swap server.
//! - [`diagnostics`] — Model Context Protocol (MCP) JSON-RPC 2.0 server, interactive SES REPL, kernel doctor, and 1,000-tenant microVM benchmark.

pub(crate) mod commands;
pub(crate) mod config;
pub(crate) mod diagnostics;
pub(crate) mod server;

// Backward-compatible module aliases for crate-level integration tests in `crates/slice-cli/tests/`
#[cfg(test)]
pub(crate) mod config_and_watch {
    pub(crate) use crate::config::*;
}
#[cfg(test)]
pub(crate) mod http_server {
    pub(crate) use crate::server::*;
}
#[cfg(test)]
pub(crate) mod mcp_and_repl {
    pub(crate) use crate::diagnostics::*;
}
#[cfg(test)]
pub(crate) mod test_runner {
    pub(crate) use crate::commands::test_runner::*;
}

#[cfg(test)]
#[path = "../tests/mod.rs"]
mod cli_tests;

pub(crate) use commands::{
    cmd_check, cmd_compile, cmd_info, cmd_init, cmd_inspect, cmd_pack, cmd_run, cmd_test,
    print_usage, run_embedded_standalone, LAST_CLI_EXIT_CODE,
};
#[cfg(test)]
pub(crate) use config::OsNativeFileWatcher;
pub(crate) use diagnostics::{
    cmd_doctor, cmd_mcp, cmd_microvm_bench, cmd_repl, cmd_verify_security,
};
pub(crate) use server::cmd_serve;

use anyhow::{bail, Result};
use slice_cas::MappedCasArchive;
use std::env;

fn main() -> Result<()> {
    let builder = std::thread::Builder::new()
        .name("slice-main".to_string())
        .stack_size(16 * 1024 * 1024);
    let handle = builder.spawn(cli_entrypoint)?;
    match handle.join() {
        Ok(Ok(())) => {
            let exit_code = LAST_CLI_EXIT_CODE.load(std::sync::atomic::Ordering::Relaxed);
            if exit_code != 0 {
                let _ = std::io::Write::flush(&mut std::io::stdout());
                let _ = std::io::Write::flush(&mut std::io::stderr());
                std::process::exit(exit_code);
            }
            Ok(())
        }
        Ok(Err(err)) => Err(err),
        Err(_) => bail!("Slice main thread panicked"),
    }
}

fn cli_entrypoint() -> Result<()> {
    // 1. Check if this binary is a self-contained `SLICEEXE` compiled application (`slice compile`)
    if let Ok(Some(embedded_cas)) = MappedCasArchive::open_embedded_self() {
        return run_embedded_standalone(embedded_cas);
    }

    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            return cmd_run(&["-".to_string()]);
        }
        print_usage();
        return Ok(());
    }

    match args[1].as_str() {
        "init" => cmd_init(&args[2..]),
        "run" => cmd_run(&args[2..]),
        "test" => cmd_test(&args[2..]),
        "pack" => cmd_pack(&args[2..]),
        "compile" => cmd_compile(&args[2..]),
        "inspect" => cmd_inspect(&args[2..]),
        "check" | "--check" | "-c" => cmd_check(&args[2..]),
        "serve" => cmd_serve(&args[2..]),
        "mcp" => cmd_mcp(&args[2..]),
        "repl" => cmd_repl(),
        "doctor" => cmd_doctor(),
        "microvm-bench" => cmd_microvm_bench(&args[2..]),
        "verify-security" => cmd_verify_security(),
        "info" => cmd_info(),
        "version" | "--version" | "-v" | "-V" => {
            println!("v{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "help" | "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        _ => {
            // Allow `slice <file.ts>`, `slice -e <code>`, `slice -p <code>`, `slice -` shorthand
            cmd_run(&args[1..])
        }
    }
}
