use crate::config::ResolvedExecutionConfig;
use anyhow::Result;
use slice_engine::{SliceIsolate, compile_source_to_bytecode};
use std::io::Write;

fn spawn_repl_isolate(exec_cfg: &ResolvedExecutionConfig) -> SliceIsolate {
    let mut isolate = SliceIsolate::spawn(
        "repl-tenant",
        1,
        exec_cfg.caps.clone(),
        exec_cfg.fuel_limit,
        exec_cfg.mem_limit,
    );
    isolate.set_deterministic_seed(exec_cfg.deterministic_seed);
    for sec in &exec_cfg.audit_secrets {
        isolate.add_egress_audit_secret(sec.clone());
    }
    isolate
}

pub(crate) fn cmd_repl() -> Result<()> {
    println!(
        "Slice (Nexus-JS) v{} Interactive SES-Locked Compartment Shell",
        env!("CARGO_PKG_VERSION")
    );
    println!("Commands: :help, :metrics, :caps, :vars, :reset, :quit");
    let exec_cfg = ResolvedExecutionConfig::from_cli_args(&[])?;
    let mut isolate = spawn_repl_isolate(&exec_cfg);
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    loop {
        print!("slice(ses)> ");
        stdout.flush()?;
        let mut line = String::new();
        if stdin.read_line(&mut line)? == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match trimmed {
            ":quit" | ":exit" | ":q" | ".exit" | ".quit" => break,
            ":help" | ":h" | ".help" | "help" => {
                println!("Slice (Nexus-JS) Interactive SES Shell Help:");
                println!("  :help / .help  - Show this help message");
                println!(
                    "  :metrics       - Show current Execution Tier, CPU Fuel, Memory, and Hardware PKEY"
                );
                println!(
                    "  :caps          - Show active Object-Capability (OCap) permissions for this compartment"
                );
                println!(
                    "  :reset         - Reset compartment state, SES intrinsics, and fuel counter"
                );
                println!("  :quit / :q     - Exit the shell");
                println!(
                    "  JavaScript/TS  - Supports var/let/const, arithmetic (t * 4), functions, arrays,"
                );
                println!(
                    "                   objects, loops, console.log(), print(), Math.*, JSON.*, fib(n)"
                );
                continue;
            }
            ":reset" | ".clear" => {
                isolate = spawn_repl_isolate(&exec_cfg);
                println!("Compartment reset (fresh SES intrinsics & fuel counter).");
                continue;
            }
            ":caps" => {
                println!("Active OCap Token: {:?}", isolate.compartment.capabilities);
                continue;
            }
            ":metrics" => {
                println!(
                    "Tier: {:?} | Fuel: {}/{} | Memory: {}/{} B | PKEY: 0x{:x}",
                    isolate.current_tier,
                    isolate.compartment.fuel_consumed,
                    isolate.compartment.fuel_limit,
                    isolate.compartment.memory_used_bytes,
                    isolate.compartment.memory_limit_bytes,
                    isolate.compartment.pkey_domain.pkey
                );
                continue;
            }
            _ if trimmed.starts_with(':') => {
                println!(
                    "Unknown shell command '{}'. Type :help for available commands.",
                    trimmed
                );
                continue;
            }
            _ => {}
        }

        let prev_stdout_len = isolate.stdout.len();
        let bc = compile_source_to_bytecode(trimmed);
        match isolate.execute_bytecode(&bc, None) {
            Ok(metrics) => {
                for line_out in &isolate.stdout[prev_stdout_len..] {
                    println!("{}", line_out);
                }
                if metrics.return_value.starts_with("Uncaught ") {
                    println!("{}", metrics.return_value);
                } else if isolate.stdout.len() == prev_stdout_len {
                    println!("=> {}", metrics.return_value);
                }
            }
            Err(violation) => {
                println!("[TRAP] {}", violation);
            }
        }
    }
    Ok(())
}
