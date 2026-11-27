//! `slice run`, `slice check`, and embedded standalone `SLICEEXE` execution.

use crate::config::{
    OsNativeFileWatcher, ResolvedExecutionConfig, install_ctrlc_handler,
    is_known_slice_runtime_flag, skip_runtime_flag_with_separate_value,
};
use anyhow::{Context, Result, bail};
use slice_cas::MappedCasArchive;
use slice_engine::{ExecutionMetrics, SliceIsolate, compile_source_to_bytecode};
use slice_sandbox::SliceManifest;
use std::borrow::Cow;
use std::cell::Cell;
use std::env;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Instant;

thread_local! {
    static TLS_LAST_CLI_EXIT_CODE: Cell<i32> = const { Cell::new(0) };
}

pub(crate) struct ThreadLocalExitCode;

impl ThreadLocalExitCode {
    #[inline]
    pub(crate) fn load(&self, _ordering: Ordering) -> i32 {
        TLS_LAST_CLI_EXIT_CODE.with(Cell::get)
    }

    #[inline]
    pub(crate) fn store(&self, val: i32, _ordering: Ordering) {
        TLS_LAST_CLI_EXIT_CODE.with(|slot| slot.set(val));
    }
}

pub(crate) static LAST_CLI_EXIT_CODE: ThreadLocalExitCode = ThreadLocalExitCode;

fn compile_cli_source_for_run(
    source: &str,
    warm_pool: bool,
    cli_boacas_override: Option<bool>,
) -> Vec<u8> {
    if !warm_pool && cli_boacas_override.is_none() {
        slice_engine::set_thread_exper_boacas_override(Some(false));
    }
    let bytecode = compile_source_to_bytecode(source);
    if !warm_pool && cli_boacas_override.is_none() {
        slice_engine::set_thread_exper_boacas_override(None);
    }
    bytecode
}

#[allow(clippy::too_many_arguments)]
fn execute_configured_cli_isolate<'a>(
    tenant_id: &str,
    exec_cfg: &ResolvedExecutionConfig,
    bytecode: &[u8],
    archive_opt: Option<&MappedCasArchive>,
    argv_opt: Option<Vec<String>>,
    prewarm_bundle: bool,
    capture_return_json: bool,
    quiet: bool,
    json_output: bool,
    error_source_fn: impl FnOnce() -> Cow<'a, str>,
) -> Result<(ExecutionMetrics, Option<String>)> {
    let mut caps = exec_cfg.caps.clone();
    if exec_cfg.artifacts_dir.is_some() {
        caps.push_virtual_mount("/artifacts");
    }
    let mut isolate = SliceIsolate::spawn(
        tenant_id,
        1,
        caps.clone(),
        exec_cfg.fuel_limit,
        exec_cfg.mem_limit,
    );
    isolate.set_deterministic_seed(exec_cfg.deterministic_seed);
    if capture_return_json || json_output {
        isolate.set_capture_return_json(true);
    }
    if let Some(argv) = argv_opt {
        isolate.set_process_argv(argv);
    }
    for sec in &exec_cfg.audit_secrets {
        isolate.add_egress_audit_secret(sec.clone());
    }
    if prewarm_bundle {
        SliceIsolate::prewarm_boa_bundle_with_caps(bytecode, &caps);
        let snap = isolate.checkpoint();
        let _ = isolate.execute_bytecode(bytecode, archive_opt);
        isolate.restore(snap);
        isolate.reset_for_request(tenant_id, 1, exec_cfg.fuel_limit, exec_cfg.mem_limit);
    }
    if exec_cfg.trace_output_path.is_some() {
        isolate.enable_tracing(true);
    }
    let exec_res = isolate.execute_bytecode(bytecode, archive_opt);
    let (written_artifacts, skipped_artifacts) =
        if let Some(ref artifacts_dir) = exec_cfg.artifacts_dir {
            let report = isolate.drain_vfs("/artifacts", &slice_engine::DrainLimits::default());
            let written =
                crate::commands::artifacts::write_drained_artifacts_to_dir(artifacts_dir, &report)?;
            if !json_output && !quiet {
                crate::commands::artifacts::print_artifacts_stderr_summary(
                    artifacts_dir,
                    &written,
                    &report.skipped,
                );
            }
            (written, report.skipped)
        } else {
            (Vec::new(), Vec::new())
        };
    let mut metrics = match exec_res {
        Ok(m) => m,
        Err(violation) => {
            if json_output {
                LAST_CLI_EXIT_CODE.store(1, Ordering::Relaxed);
                let payload = serde_json::json!({
                    "ok": false,
                    "exitCode": serde_json::Value::Null,
                    "processExited": false,
                    "stdout": Vec::<String>::new(),
                    "returnValue": "undefined",
                    "returnJson": serde_json::Value::Null,
                    "error": serde_json::Value::Null,
                    "violation": violation.to_string(),
                    "deniedCapabilities": isolate.denied_capabilities(),
                    "artifacts": written_artifacts,
                    "skippedArtifacts": skipped_artifacts,
                    "fuelUsed": isolate.compartment.fuel_consumed,
                    "memoryUsedBytes": isolate.compartment.memory_used_bytes,
                });
                println!("{}", payload);
            }
            return Err(violation.into());
        }
    };
    if isolate.denied_capabilities.len() > metrics.denied_capabilities.len() {
        metrics
            .denied_capabilities
            .extend_from_slice(&isolate.denied_capabilities[metrics.denied_capabilities.len()..]);
    }
    if let Some(ref trace_path) = exec_cfg.trace_output_path {
        let trace_json = isolate.export_chrome_trace_json(&metrics);
        isolate.enable_tracing(false);
        std::fs::write(trace_path, trace_json)
            .with_context(|| format!("Failed to write Chrome trace to {}", trace_path.display()))?;
    }
    if let Some(code) = metrics.exit_code {
        LAST_CLI_EXIT_CODE.store(code, Ordering::Relaxed);
    }
    let is_uncaught = metrics.return_value.starts_with("Uncaught ");
    let mapped_error = if is_uncaught {
        if metrics.exit_code.is_none() {
            LAST_CLI_EXIT_CODE.store(1, Ordering::Relaxed);
        }
        let annotated = error_source_fn();
        Some(slice_cas::map_bundled_error_location(
            &annotated,
            &metrics.return_value,
        ))
    } else {
        None
    };
    let captured_json = if capture_return_json || json_output {
        isolate.last_return_json().map(str::to_owned)
    } else {
        None
    };
    if json_output {
        let ok = !is_uncaught && metrics.exit_code.unwrap_or(0) == 0;
        let payload = serde_json::json!({
            "ok": ok,
            "exitCode": metrics.exit_code,
            "processExited": metrics.process_exited,
            "stdout": metrics.stdout,
            "returnValue": metrics.return_value,
            "returnJson": captured_json,
            "error": mapped_error,
            "violation": serde_json::Value::Null,
            "deniedCapabilities": metrics.denied_capabilities,
            "artifacts": written_artifacts,
            "skippedArtifacts": skipped_artifacts,
            "fuelUsed": metrics.fuel_consumed,
            "memoryUsedBytes": metrics.memory_used_bytes,
            "totalTimeUs": metrics.total_time_ns as f64 / 1_000.0,
        });
        println!("{}", payload);
        if let Some(mapped) = mapped_error {
            bail!("{}", mapped);
        }
        return Ok((metrics, captured_json));
    }
    if !quiet {
        for line in &metrics.stdout {
            println!("{}", line);
        }
    }
    if let Some(mapped) = mapped_error {
        bail!("{}", mapped);
    }
    Ok((metrics, captured_json))
}

pub(crate) fn cmd_check(args: &[String]) -> Result<()> {
    LAST_CLI_EXIT_CODE.store(0, Ordering::Relaxed);
    let mut targets: Vec<String> = Vec::new();
    let mut eval_snippets: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            targets.extend(args[i + 1..].iter().cloned());
            break;
        }
        if skip_runtime_flag_with_separate_value(args, &mut i)? {
            continue;
        }
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
        } else if !is_known_slice_runtime_flag(arg) {
            LAST_CLI_EXIT_CODE.store(1, Ordering::Relaxed);
            bail!("Unknown option: {}", arg);
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
    if args.iter().any(|a| a == "--warm") {
        SliceIsolate::prewarm_boa_pool(1);
    }

    let mut exec_cfg = ResolvedExecutionConfig::from_cli_args_with_ambient_cwd(&args[1..], false)?;
    if let Some(toml_str) = archive.get_manifest_toml() {
        exec_cfg.apply_manifest_toml(toml_str, None)?;
    }

    let standalone_argv = if !args.is_empty() {
        let mut argv = Vec::with_capacity(args.len() + 1);
        argv.push("slice".to_string());
        argv.push(args[0].clone());
        let mut ai = 1;
        while ai < args.len() {
            let a = &args[ai];
            if a == "--" {
                argv.extend(args[ai + 1..].iter().cloned());
                break;
            }
            if skip_runtime_flag_with_separate_value(&args, &mut ai)? {
                continue;
            }
            if !is_known_slice_runtime_flag(a) {
                argv.push(a.clone());
            }
            ai += 1;
        }
        Some(argv)
    } else {
        None
    };

    let entry = archive
        .entrypoint_blob()
        .context("Embedded CAS archive has no entrypoint blob")?;
    let bc = archive.get_bytecode_slice(entry);
    let (metrics, _) = execute_configured_cli_isolate(
        "standalone-exe",
        &exec_cfg,
        bc,
        Some(&archive),
        standalone_argv,
        false,
        false,
        quiet,
        false,
        || Cow::Owned(archive.reconstruct_annotated_source(entry)),
    )?;

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
    let mut quiet = false;
    let mut print_metrics = false;
    let mut json_output = false;
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
        if skip_runtime_flag_with_separate_value(args, &mut i)? {
            continue;
        }
        let arg = &args[i];
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
        } else if arg == "--json" {
            json_output = true;
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

    let is_external_cas = target_opt.as_deref().is_some_and(|t| t.ends_with(".cas"));
    let mut exec_cfg =
        ResolvedExecutionConfig::from_cli_args_with_ambient_cwd(args, !is_external_cas)?;

    if let Some((raw_code, print_result)) = eval_opt {
        exec_cfg.apply_inline_directives(&raw_code, Some(Path::new(".")))?;
        if let Err(syn_err) = slice_cas::check_syntax_source(&raw_code, None) {
            LAST_CLI_EXIT_CODE.store(1, Ordering::Relaxed);
            bail!("{}", syn_err);
        }
        let bytecode =
            compile_cli_source_for_run(&raw_code, exec_cfg.warm_pool, exec_cfg.cli_boacas_override);
        let argv_opt = (!script_args.is_empty()).then(|| {
            let mut argv = Vec::with_capacity(1 + script_args.len());
            argv.push("slice".to_string());
            argv.extend(script_args);
            argv
        });
        let (m, return_json_opt) = execute_configured_cli_isolate(
            "tenant-cli-eval",
            &exec_cfg,
            &bytecode,
            None,
            argv_opt,
            false,
            print_result,
            quiet,
            json_output,
            || Cow::Borrowed(raw_code.as_str()),
        )?;
        if print_result && !m.process_exited && !quiet && !json_output {
            if m.return_value == "[object Object]" {
                println!(
                    "{}",
                    return_json_opt
                        .as_deref()
                        .unwrap_or(m.return_value.as_str())
                );
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
                bail!(
                    "Missing target script/archive path (and no slice.toml found in current directory)"
                );
            }
        }
    };

    if target == "-" {
        let mut stdin_src = String::new();
        std::io::stdin().read_to_string(&mut stdin_src)?;
        exec_cfg.apply_inline_directives(&stdin_src, Some(Path::new(".")))?;
        if let Err(syn_err) = slice_cas::check_syntax_source(&stdin_src, None) {
            LAST_CLI_EXIT_CODE.store(1, Ordering::Relaxed);
            bail!("{}", syn_err);
        }
        let bytecode = compile_cli_source_for_run(
            &stdin_src,
            exec_cfg.warm_pool,
            exec_cfg.cli_boacas_override,
        );
        let argv_opt = (!script_args.is_empty()).then(|| {
            let mut argv = Vec::with_capacity(2 + script_args.len());
            argv.push("slice".to_string());
            argv.push("-".to_string());
            argv.extend(script_args);
            argv
        });
        let _ = execute_configured_cli_isolate(
            "tenant-cli-stdin",
            &exec_cfg,
            &bytecode,
            None,
            argv_opt,
            false,
            false,
            quiet,
            json_output,
            || Cow::Borrowed(stdin_src.as_str()),
        )?;
        return Ok(());
    }

    let path = Path::new(&target);
    let target_argv = {
        let mut argv = Vec::with_capacity(2 + script_args.len());
        argv.push("slice".to_string());
        argv.push(target.clone());
        argv.extend(script_args);
        argv
    };

    let metrics = if target.ends_with(".cas") {
        let archive = MappedCasArchive::open(path)?;
        if let Some(toml_str) = archive.get_manifest_toml() {
            let cas_base_dir = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .or_else(|| Some(Path::new(".")));
            exec_cfg.apply_manifest_toml(toml_str, cas_base_dir)?;
        }
        let entry = archive
            .entrypoint_blob()
            .context("CAS archive has no valid entrypoint blob")?;
        let bytecode = archive.get_bytecode_slice(entry);
        let (m, _) = execute_configured_cli_isolate(
            "tenant-cli",
            &exec_cfg,
            bytecode,
            Some(&archive),
            Some(target_argv),
            exec_cfg.warm_pool,
            false,
            quiet,
            json_output,
            || Cow::Owned(archive.reconstruct_annotated_source(entry)),
        )?;
        m
    } else {
        exec_cfg.apply_target_path_manifest(path)?;
        let entry_file = slice_cas::resolve_project_entrypoint(path, None)?;
        let raw_entry = std::fs::read_to_string(&entry_file)
            .with_context(|| format!("Failed to read {}", entry_file.display()))?;
        exec_cfg.apply_inline_directives(&raw_entry, entry_file.parent())?;
        let bundled_source = match slice_cas::bundle_entrypoint_with_deps(&entry_file) {
            Ok(s) => s,
            Err(e) => {
                LAST_CLI_EXIT_CODE.store(1, Ordering::Relaxed);
                return Err(e);
            }
        };
        let bytecode = compile_cli_source_for_run(
            &bundled_source,
            exec_cfg.warm_pool,
            exec_cfg.cli_boacas_override,
        );
        let (m, _) = execute_configured_cli_isolate(
            "tenant-cli",
            &exec_cfg,
            &bytecode,
            None,
            Some(target_argv),
            exec_cfg.warm_pool,
            false,
            quiet,
            json_output,
            || Cow::Borrowed(bundled_source.as_str()),
        )?;
        m
    };

    if print_metrics && !json_output {
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
        let base_watch_cfg = ResolvedExecutionConfig::from_cli_args(args)?;
        if !watch_once {
            install_ctrlc_handler();
        }
        let mut watcher = OsNativeFileWatcher::new(path);
        while watcher.wait_for_change(watch_once) {
            let reload_start = Instant::now();
            let reload_outcome: Result<_> = (|| {
                let mut reload_cfg = base_watch_cfg.clone();
                reload_cfg.apply_target_path_manifest(path)?;
                let entry_file = slice_cas::resolve_project_entrypoint(path, None)?;
                let raw_entry = std::fs::read_to_string(&entry_file)
                    .with_context(|| format!("Failed to read {}", entry_file.display()))?;
                reload_cfg.apply_inline_directives(&raw_entry, entry_file.parent())?;
                let bundled_source = slice_cas::bundle_entrypoint_with_deps(&entry_file)?;
                let bytecode = compile_source_to_bytecode(&bundled_source);
                if reload_cfg.artifacts_dir.is_some() {
                    reload_cfg.caps.push_virtual_mount("/artifacts");
                }
                let mut isolate = SliceIsolate::spawn(
                    "tenant-cli-watch",
                    1,
                    reload_cfg.caps,
                    reload_cfg.fuel_limit,
                    reload_cfg.mem_limit,
                );
                isolate.set_deterministic_seed(reload_cfg.deterministic_seed);
                for sec in &reload_cfg.audit_secrets {
                    isolate.add_egress_audit_secret(sec.clone());
                }
                let exec_res = isolate.execute_bytecode(&bytecode, None);
                if let Some(ref artifacts_dir) = reload_cfg.artifacts_dir {
                    let report =
                        isolate.drain_vfs("/artifacts", &slice_engine::DrainLimits::default());
                    let written = crate::commands::artifacts::write_drained_artifacts_to_dir(
                        artifacts_dir,
                        &report,
                    )?;
                    if !quiet {
                        crate::commands::artifacts::print_artifacts_stderr_summary(
                            artifacts_dir,
                            &written,
                            &report.skipped,
                        );
                    }
                }
                let reload_metrics = exec_res
                    .map_err(|v| anyhow::anyhow!("Sandbox containment violation: {}", v))?;
                Ok((bundled_source, reload_metrics))
            })();

            match reload_outcome {
                Ok((bundled_source, reload_metrics)) => {
                    if !quiet {
                        for line in &reload_metrics.stdout {
                            println!("{}", line);
                        }
                    }
                    LAST_CLI_EXIT_CODE
                        .store(reload_metrics.exit_code.unwrap_or(0), Ordering::Relaxed);
                    if reload_metrics.return_value.starts_with("Uncaught ") {
                        if reload_metrics.exit_code.is_none() {
                            LAST_CLI_EXIT_CODE.store(1, Ordering::Relaxed);
                        }
                        let mapped = slice_cas::map_bundled_error_location(
                            &bundled_source,
                            &reload_metrics.return_value,
                        );
                        if watch_once {
                            bail!("{}", mapped);
                        }
                        eprintln!("[Slice Watch Error] {}", mapped);
                    } else if watch_once
                        && let Some(code) = reload_metrics.exit_code
                        && code != 0
                    {
                        bail!("Process exited with status code {}", code);
                    } else if !quiet {
                        let reload_ms = reload_start.elapsed().as_secs_f64() * 1_000.0;
                        println!("[Slice Watch] Reloaded in {:.2} ms", reload_ms);
                    }
                }
                Err(err) => {
                    LAST_CLI_EXIT_CODE.store(1, Ordering::Relaxed);
                    if watch_once {
                        return Err(err);
                    }
                    eprintln!("[Slice Watch Error] {:#}", err);
                }
            }
            if watch_once {
                break;
            }
        }
    }

    Ok(())
}
