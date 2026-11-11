use crate::config::{install_ctrlc_handler, OsNativeFileWatcher, ResolvedExecutionConfig};
use anyhow::{bail, Context, Result};
use slice_cas::MappedCasArchive;
use slice_engine::{compile_source_to_bytecode, SliceIsolate};
use slice_sandbox::SliceManifest;
use std::env;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Instant;

pub(crate) static LAST_CLI_EXIT_CODE: AtomicI32 = AtomicI32::new(0);

#[inline]
fn is_known_slice_runtime_flag(arg: &str) -> bool {
    matches!(
        arg,
        "--quiet"
            | "-q"
            | "--metrics"
            | "--watch"
            | "--watch-once"
            | "--warm"
            | "--no-warm"
            | "--boacas"
            | "--no-boacas"
            | "--exper-boacas"
            | "--no-exper-boacas"
            | "--deterministic"
            | "--no-deterministic"
            | "--checkpoints"
            | "--no-checkpoints"
            | "--trust-directives"
            | "--no-trust-directives"
    ) || arg.starts_with("--allow-")
        || arg.starts_with("--fuel=")
        || arg.starts_with("--memory=")
        || arg.starts_with("--memory-kb=")
        || arg.starts_with("--tier=")
        || arg.starts_with("--boacas=")
        || arg.starts_with("--exper-boacas=")
        || arg.starts_with("--deterministic=")
        || arg.starts_with("--env-file=")
        || arg.starts_with("--audit-egress=")
        || arg.starts_with("--audit-egress-file=")
        || arg.starts_with("--checkpoints=")
        || arg.starts_with("--max-checkpoints=")
        || arg.starts_with("--trace=")
        || arg.starts_with("--port=")
        || arg.starts_with("--workers=")
        || arg.starts_with("--static=")
        || arg.starts_with("--tls-cert=")
        || arg.starts_with("--tls-key=")
}

pub(crate) fn cmd_check(args: &[String]) -> Result<()> {
    LAST_CLI_EXIT_CODE.store(0, Ordering::Relaxed);
    let mut targets: Vec<String> = Vec::new();
    let mut eval_snippets: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--check" || arg == "-c" {
            i += 1;
            continue;
        } else if arg == "-e" || arg == "--eval" || arg == "-p" || arg == "--print" {
            if i + 1 >= args.len() {
                LAST_CLI_EXIT_CODE.store(1, Ordering::Relaxed);
                bail!("Flag {} requires a code argument", arg);
            }
            eval_snippets.push(args[i + 1].clone());
            i += 2;
            continue;
        } else if let Some(code) = arg
            .strip_prefix("--eval=")
            .or_else(|| arg.strip_prefix("-e="))
            .or_else(|| {
                arg.strip_prefix("--print=")
                    .or_else(|| arg.strip_prefix("-p="))
            })
        {
            eval_snippets.push(code.to_string());
        } else if arg == "-" || !arg.starts_with('-') {
            targets.push(arg.clone());
        }
        i += 1;
    }

    if targets.is_empty() && eval_snippets.is_empty() {
        LAST_CLI_EXIT_CODE.store(1, Ordering::Relaxed);
        bail!("Usage: slice --check <script.js | script.ts>");
    }

    for snippet in &eval_snippets {
        if let Err(err) = slice_cas::check_syntax_source(snippet, None) {
            LAST_CLI_EXIT_CODE.store(1, Ordering::Relaxed);
            bail!("{}", err);
        }
    }
    for target in &targets {
        if target == "-" {
            let mut stdin_src = String::new();
            std::io::stdin().read_to_string(&mut stdin_src)?;
            if let Err(err) = slice_cas::check_syntax_source(&stdin_src, None) {
                LAST_CLI_EXIT_CODE.store(1, Ordering::Relaxed);
                bail!("{}", err);
            }
        } else if target.ends_with(".cas") {
            let _ = MappedCasArchive::open(Path::new(target))?;
        } else {
            let path = Path::new(target);
            let entry_file = slice_cas::resolve_project_entrypoint(path, None)?;
            let raw = std::fs::read_to_string(&entry_file)
                .with_context(|| format!("Failed to read {}", entry_file.display()))?;
            if let Err(err) = slice_cas::check_syntax_source(&raw, Some(&entry_file)) {
                LAST_CLI_EXIT_CODE.store(1, Ordering::Relaxed);
                bail!("{}", err);
            }
        }
    }
    Ok(())
}

pub(crate) fn run_embedded_standalone(archive: MappedCasArchive) -> Result<()> {
    LAST_CLI_EXIT_CODE.store(0, Ordering::Relaxed);
    let args: Vec<String> = env::args().collect();
    let quiet = args.iter().any(|a| a == "--quiet" || a == "-q");
    let print_metrics = args.iter().any(|a| a == "--metrics");
    let warm = args.iter().any(|a| a == "--warm");
    if warm {
        SliceIsolate::prewarm_boa_pool(1);
    }

    let mut exec_cfg = ResolvedExecutionConfig::from_cli_args(&args[1..])?;
    if let Some(toml_str) = archive.get_manifest_toml() {
        exec_cfg.apply_manifest_toml(toml_str, None);
    }
    let mut isolate = SliceIsolate::spawn(
        "standalone-exe",
        1,
        exec_cfg.caps,
        exec_cfg.fuel_limit,
        exec_cfg.mem_limit,
    );
    if !args.is_empty() {
        let mut standalone_argv = Vec::with_capacity(args.len() + 1);
        standalone_argv.push("slice".to_string());
        standalone_argv.push(args[0].clone());
        for a in &args[1..] {
            if a != "--quiet" && a != "-q" && a != "--metrics" && a != "--warm" {
                standalone_argv.push(a.clone());
            }
        }
        isolate.set_process_argv(standalone_argv);
    }
    for sec in &exec_cfg.audit_secrets {
        isolate.add_egress_audit_secret(sec.clone());
    }
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

    if metrics.return_value.starts_with("Uncaught ") {
        let annotated = archive.reconstruct_annotated_source(entry);
        let mapped = slice_cas::map_bundled_error_location(&annotated, &metrics.return_value);
        bail!("{}", mapped);
    }
    if let Some(code) = metrics.exit_code {
        LAST_CLI_EXIT_CODE.store(code, Ordering::Relaxed);
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

pub(crate) fn cmd_run(args: &[String]) -> Result<()> {
    LAST_CLI_EXIT_CODE.store(0, Ordering::Relaxed);
    let mut exec_cfg = ResolvedExecutionConfig::from_cli_args(args)?;
    let mut quiet = false;
    let mut print_metrics = false;
    let mut watch_mode = false;
    let mut watch_once = false;
    let mut target_opt: Option<String> = None;
    let mut eval_opt: Option<(String, bool)> = None;
    let mut script_args: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            for rem in &args[i + 1..] {
                if target_opt.is_none() && eval_opt.is_none() {
                    target_opt = Some(rem.clone());
                } else {
                    script_args.push(rem.clone());
                }
            }
            break;
        }
        if target_opt.is_none() && eval_opt.is_none() {
            if arg == "--check" || arg == "-c" {
                return cmd_check(args);
            } else if arg == "--version" || arg == "-v" || arg == "-V" {
                println!("v{}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            } else if arg == "--help" || arg == "-h" {
                crate::commands::print_usage();
                return Ok(());
            } else if arg == "-e" || arg == "--eval" {
                if i + 1 >= args.len() {
                    bail!("Flag {} requires a code argument", arg);
                }
                eval_opt = Some((args[i + 1].clone(), false));
                i += 2;
                continue;
            } else if let Some(code) = arg
                .strip_prefix("--eval=")
                .or_else(|| arg.strip_prefix("-e="))
            {
                eval_opt = Some((code.to_string(), false));
                i += 1;
                continue;
            } else if arg == "-p" || arg == "--print" {
                if i + 1 >= args.len() {
                    bail!("Flag {} requires a code argument", arg);
                }
                eval_opt = Some((args[i + 1].clone(), true));
                i += 2;
                continue;
            } else if let Some(code) = arg
                .strip_prefix("--print=")
                .or_else(|| arg.strip_prefix("-p="))
            {
                eval_opt = Some((code.to_string(), true));
                i += 1;
                continue;
            }
        }
        if arg == "--quiet" || arg == "-q" {
            quiet = true;
        } else if arg == "--metrics" {
            print_metrics = true;
        } else if arg == "--watch" {
            watch_mode = true;
        } else if arg == "--watch-once" {
            watch_mode = true;
            watch_once = true;
        } else if target_opt.is_none() && eval_opt.is_none() {
            if arg == "-" || !arg.starts_with('-') {
                target_opt = Some(arg.clone());
            } else if !is_known_slice_runtime_flag(arg) {
                bail!("Unknown option: {}", arg);
            }
        } else if !is_known_slice_runtime_flag(arg) {
            script_args.push(arg.clone());
        }
        i += 1;
    }

    if let Some((raw_code, print_result)) = eval_opt {
        exec_cfg.apply_inline_directives(&raw_code, Some(Path::new(".")));
        if let Err(syn_err) = slice_cas::check_syntax_source(&raw_code, None) {
            bail!("{}", syn_err);
        }
        let transpiled = slice_cas::strip_typescript_and_normalize(&raw_code, true);
        if !exec_cfg.warm_pool && exec_cfg.cli_boacas_override.is_none() {
            slice_engine::set_thread_exper_boacas_override(Some(false));
        }
        let bytecode = compile_source_to_bytecode(&transpiled);
        if !exec_cfg.warm_pool && exec_cfg.cli_boacas_override.is_none() {
            slice_engine::set_thread_exper_boacas_override(None);
        }
        let mut isolate = SliceIsolate::spawn(
            "tenant-cli-eval",
            1,
            exec_cfg.caps.clone(),
            exec_cfg.fuel_limit,
            exec_cfg.mem_limit,
        );
        isolate.set_deterministic_seed(exec_cfg.deterministic_seed);
        if print_result {
            isolate.set_capture_return_json(true);
        }
        if !script_args.is_empty() {
            let mut argv = Vec::with_capacity(1 + script_args.len());
            argv.push("slice".to_string());
            argv.extend(script_args);
            isolate.set_process_argv(argv);
        }
        for sec in &exec_cfg.audit_secrets {
            isolate.add_egress_audit_secret(sec.clone());
        }
        let m = isolate.execute_bytecode(&bytecode, None)?;
        if !quiet {
            for line in &m.stdout {
                println!("{}", line);
            }
        }
        if let Some(code) = m.exit_code {
            LAST_CLI_EXIT_CODE.store(code, Ordering::Relaxed);
        }
        if m.return_value.starts_with("Uncaught ") {
            if m.exit_code.is_none() {
                LAST_CLI_EXIT_CODE.store(1, Ordering::Relaxed);
            }
            let mapped = slice_cas::map_bundled_error_location(&transpiled, &m.return_value);
            bail!("{}", mapped);
        }
        if print_result && m.exit_code.is_none() && !quiet {
            if m.return_value == "[object Object]" {
                if let Some(json_str) = isolate.last_return_json() {
                    println!("{}", json_str);
                } else {
                    println!("{}", m.return_value);
                }
            } else {
                println!("{}", m.return_value);
            }
        }
        return Ok(());
    }

    let target = match target_opt {
        Some(t) => t,
        None => {
            let manifest_path = Path::new("slice.toml");
            if manifest_path.exists() {
                let raw = std::fs::read_to_string(manifest_path)?;
                let m = SliceManifest::parse_toml(&raw);
                m.entrypoint
            } else if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                "-".to_string()
            } else {
                bail!("Missing target script/archive path (and no slice.toml found in current directory)");
            }
        }
    };

    if target == "-" {
        let mut stdin_src = String::new();
        std::io::stdin().read_to_string(&mut stdin_src)?;
        exec_cfg.apply_inline_directives(&stdin_src, Some(Path::new(".")));
        if let Err(syn_err) = slice_cas::check_syntax_source(&stdin_src, None) {
            bail!("{}", syn_err);
        }
        let transpiled = slice_cas::strip_typescript_and_normalize(&stdin_src, true);
        if !exec_cfg.warm_pool && exec_cfg.cli_boacas_override.is_none() {
            slice_engine::set_thread_exper_boacas_override(Some(false));
        }
        let bytecode = compile_source_to_bytecode(&transpiled);
        if !exec_cfg.warm_pool && exec_cfg.cli_boacas_override.is_none() {
            slice_engine::set_thread_exper_boacas_override(None);
        }
        let mut isolate = SliceIsolate::spawn(
            "tenant-cli-stdin",
            1,
            exec_cfg.caps.clone(),
            exec_cfg.fuel_limit,
            exec_cfg.mem_limit,
        );
        isolate.set_deterministic_seed(exec_cfg.deterministic_seed);
        if !script_args.is_empty() {
            let mut argv = Vec::with_capacity(2 + script_args.len());
            argv.push("slice".to_string());
            argv.push("-".to_string());
            argv.extend(script_args);
            isolate.set_process_argv(argv);
        }
        for sec in &exec_cfg.audit_secrets {
            isolate.add_egress_audit_secret(sec.clone());
        }
        let m = isolate.execute_bytecode(&bytecode, None)?;
        if !quiet {
            for line in &m.stdout {
                println!("{}", line);
            }
        }
        if let Some(code) = m.exit_code {
            LAST_CLI_EXIT_CODE.store(code, Ordering::Relaxed);
        }
        if m.return_value.starts_with("Uncaught ") {
            if m.exit_code.is_none() {
                LAST_CLI_EXIT_CODE.store(1, Ordering::Relaxed);
            }
            let mapped = slice_cas::map_bundled_error_location(&transpiled, &m.return_value);
            bail!("{}", mapped);
        }
        return Ok(());
    }

    let path = Path::new(&target);
    let mut bundled_for_sourcemap = String::new();
    let metrics = if target.ends_with(".cas") {
        let archive = MappedCasArchive::open(path)?;
        if let Some(toml_str) = archive.get_manifest_toml() {
            exec_cfg.apply_manifest_toml(toml_str, path.parent());
        }
        let prewarm_pool = exec_cfg.warm_pool;
        let deterministic_seed = exec_cfg.deterministic_seed;
        let entry = archive
            .entrypoint_blob()
            .context("CAS archive has no valid entrypoint blob")?;
        let bytecode = archive.get_bytecode_slice(entry);
        let mut isolate = SliceIsolate::spawn(
            "tenant-cli",
            1,
            exec_cfg.caps.clone(),
            exec_cfg.fuel_limit,
            exec_cfg.mem_limit,
        );
        isolate.set_deterministic_seed(deterministic_seed);
        {
            let mut argv = Vec::with_capacity(2 + script_args.len());
            argv.push("slice".to_string());
            argv.push(target.clone());
            argv.extend(script_args.iter().cloned());
            isolate.set_process_argv(argv);
        }
        for sec in &exec_cfg.audit_secrets {
            isolate.add_egress_audit_secret(sec.clone());
        }
        if prewarm_pool {
            SliceIsolate::prewarm_boa_bundle(bytecode);
            let snap = isolate.checkpoint();
            let _ = isolate.execute_bytecode(bytecode, Some(&archive));
            isolate.restore(snap);
            isolate.reset_for_request("tenant-cli", 1, exec_cfg.fuel_limit, exec_cfg.mem_limit);
        }
        if exec_cfg.trace_output_path.is_some() {
            isolate.enable_tracing(true);
        }
        let m = isolate.execute_bytecode(bytecode, Some(&archive))?;
        if let Some(ref trace_path) = exec_cfg.trace_output_path {
            let trace_json = isolate.export_chrome_trace_json(&m);
            isolate.enable_tracing(false);
            let _ = std::fs::write(trace_path, trace_json);
        }
        if m.return_value.starts_with("Uncaught ") {
            bundled_for_sourcemap = archive.reconstruct_annotated_source(entry);
        }
        m
    } else {
        exec_cfg.apply_target_path_manifest(path);
        let entry_file = slice_cas::resolve_project_entrypoint(path, None)?;
        if let Ok(raw_entry) = std::fs::read_to_string(&entry_file) {
            exec_cfg.apply_inline_directives(&raw_entry, entry_file.parent());
        }
        let prewarm_pool = exec_cfg.warm_pool;
        let deterministic_seed = exec_cfg.deterministic_seed;
        let bundled_source = slice_cas::bundle_entrypoint_with_deps(&entry_file)?;
        if !prewarm_pool && exec_cfg.cli_boacas_override.is_none() {
            slice_engine::set_thread_exper_boacas_override(Some(false));
        }
        let bytecode = compile_source_to_bytecode(&bundled_source);
        if !prewarm_pool && exec_cfg.cli_boacas_override.is_none() {
            slice_engine::set_thread_exper_boacas_override(None);
        }
        bundled_for_sourcemap = bundled_source;
        let mut isolate = SliceIsolate::spawn(
            "tenant-cli",
            1,
            exec_cfg.caps.clone(),
            exec_cfg.fuel_limit,
            exec_cfg.mem_limit,
        );
        isolate.set_deterministic_seed(deterministic_seed);
        {
            let mut argv = Vec::with_capacity(2 + script_args.len());
            argv.push("slice".to_string());
            argv.push(target.clone());
            argv.extend(script_args.iter().cloned());
            isolate.set_process_argv(argv);
        }
        for sec in &exec_cfg.audit_secrets {
            isolate.add_egress_audit_secret(sec.clone());
        }
        if prewarm_pool {
            SliceIsolate::prewarm_boa_bundle(&bytecode);
            let snap = isolate.checkpoint();
            let _ = isolate.execute_bytecode(&bytecode, None);
            isolate.restore(snap);
            isolate.reset_for_request("tenant-cli", 1, exec_cfg.fuel_limit, exec_cfg.mem_limit);
        }
        if exec_cfg.trace_output_path.is_some() {
            isolate.enable_tracing(true);
        }
        let m = isolate.execute_bytecode(&bytecode, None)?;
        if let Some(ref trace_path) = exec_cfg.trace_output_path {
            let trace_json = isolate.export_chrome_trace_json(&m);
            isolate.enable_tracing(false);
            let _ = std::fs::write(trace_path, trace_json);
        }
        m
    };

    if !quiet {
        for line in &metrics.stdout {
            println!("{}", line);
        }
    }

    if let Some(code) = metrics.exit_code {
        LAST_CLI_EXIT_CODE.store(code, Ordering::Relaxed);
    }

    if metrics.return_value.starts_with("Uncaught ") {
        if metrics.exit_code.is_none() {
            LAST_CLI_EXIT_CODE.store(1, Ordering::Relaxed);
        }
        let mapped =
            slice_cas::map_bundled_error_location(&bundled_for_sourcemap, &metrics.return_value);
        bail!("{}", mapped);
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
        if !watch_once {
            install_ctrlc_handler();
        }
        let mut watcher = OsNativeFileWatcher::new(path);
        while watcher.wait_for_change(watch_once) {
            let reload_start = Instant::now();
            let entry_file = slice_cas::resolve_project_entrypoint(path, None)?;
            let bundled_source = slice_cas::bundle_entrypoint_with_deps(&entry_file)?;
            let bytecode = compile_source_to_bytecode(&bundled_source);
            let mut isolate = SliceIsolate::spawn(
                "tenant-cli-watch",
                1,
                exec_cfg.caps.clone(),
                exec_cfg.fuel_limit,
                exec_cfg.mem_limit,
            );
            isolate.set_deterministic_seed(exec_cfg.deterministic_seed);
            for sec in &exec_cfg.audit_secrets {
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
