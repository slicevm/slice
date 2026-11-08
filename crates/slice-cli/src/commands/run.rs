use crate::config::{install_ctrlc_handler, OsNativeFileWatcher, ResolvedExecutionConfig};
use anyhow::{bail, Context, Result};
use slice_cas::MappedCasArchive;
use slice_engine::{compile_source_to_bytecode, SliceIsolate};
use slice_sandbox::SliceManifest;
use std::env;
use std::path::Path;
use std::time::Instant;

pub(crate) fn run_embedded_standalone(archive: MappedCasArchive) -> Result<()> {
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
    for sec in &exec_cfg.audit_secrets {
        isolate.add_egress_audit_secret(sec.clone());
    }
    let entry = archive
        .entrypoint_blob()
        .context("Embedded CAS archive has no entrypoint blob")?;
    let bc = archive.get_bytecode_slice(entry);
    let metrics = isolate.execute_bytecode(bc, Some(&archive))?;

    if metrics.return_value.starts_with("Uncaught ") {
        let annotated = archive.reconstruct_annotated_source(entry);
        let mapped = slice_cas::map_bundled_error_location(&annotated, &metrics.return_value);
        bail!("{}", mapped);
    }

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

pub(crate) fn cmd_run(args: &[String]) -> Result<()> {
    let mut exec_cfg = ResolvedExecutionConfig::from_cli_args(args)?;
    let mut quiet = false;
    let mut print_metrics = false;
    let mut watch_mode = false;
    let mut watch_once = false;
    let mut target_opt: Option<String> = None;

    for arg in args {
        if arg == "--quiet" || arg == "-q" {
            quiet = true;
        } else if arg == "--metrics" {
            print_metrics = true;
        } else if arg == "--watch" {
            watch_mode = true;
        } else if arg == "--watch-once" {
            watch_mode = true;
            watch_once = true;
        } else if !arg.starts_with('-') && target_opt.is_none() {
            target_opt = Some(arg.clone());
        }
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

    if metrics.return_value.starts_with("Uncaught ") {
        let mapped =
            slice_cas::map_bundled_error_location(&bundled_for_sourcemap, &metrics.return_value);
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
