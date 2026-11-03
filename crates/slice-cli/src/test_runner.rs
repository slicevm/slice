use crate::config_and_watch::{
    apply_dotenv_file_to_caps_and_dlp, find_project_manifest_path, load_manifest_or_default,
    merge_manifest_toml_into_caps_and_dlp,
};
use anyhow::{bail, Result};
use slice_engine::{compile_source_to_bytecode, SliceIsolate};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub(crate) fn discover_test_files(target: &Path) -> Result<Vec<PathBuf>> {
    if target.is_file() {
        return Ok(vec![target.to_path_buf()]);
    }
    let mut found = Vec::new();
    fn walk_tests(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or_default();
            if matches!(name, "node_modules" | "target" | ".git" | ".archive") {
                continue;
            }
            if p.is_dir() {
                walk_tests(&p, out);
            } else if name.ends_with(".test.ts")
                || name.ends_with(".spec.ts")
                || name.ends_with("_test.ts")
                || name.ends_with(".test.tsx")
                || name.ends_with(".spec.tsx")
                || name.ends_with("_test.tsx")
                || name.ends_with(".test.js")
                || name.ends_with(".spec.js")
                || name.ends_with("_test.js")
                || name.ends_with(".test.jsx")
                || name.ends_with(".spec.jsx")
                || name.ends_with("_test.jsx")
            {
                out.push(p);
            }
        }
    }
    walk_tests(target, &mut found);
    Ok(found)
}

pub(crate) fn cmd_test(args: &[String]) -> Result<()> {
    let (mut caps, mut fuel_limit, mut mem_limit) = load_manifest_or_default();
    let mut filter_opt: Option<String> = None;
    let mut deterministic_seed: Option<u64> = Some(20261024);
    let mut audit_secrets: Vec<String> = Vec::new();
    let mut env_files: Vec<PathBuf> = Vec::new();
    let mut json_output = false;
    let mut target_opt: Option<String> = None;

    for arg in args {
        if let Some(f) = arg.strip_prefix("--filter=") {
            filter_opt = Some(f.to_string());
        } else if let Some(prefix) = arg.strip_prefix("--allow-read=") {
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
            deterministic_seed = Some(seed_str.parse().unwrap_or(20261024));
        } else if arg == "--no-deterministic" {
            deterministic_seed = None;
        } else if arg == "--json" {
            json_output = true;
        } else if !arg.starts_with('-') && target_opt.is_none() {
            target_opt = Some(arg.clone());
        }
    }

    for ef in &env_files {
        apply_dotenv_file_to_caps_and_dlp(ef, &mut caps, &mut audit_secrets)?;
    }

    let target_str = target_opt.unwrap_or_else(|| ".".to_string());
    let target_path = Path::new(&target_str);
    if let Some(manifest_file) = find_project_manifest_path(target_path) {
        if let Ok(raw) = std::fs::read_to_string(&manifest_file) {
            merge_manifest_toml_into_caps_and_dlp(
                &raw,
                manifest_file.parent(),
                &mut caps,
                &mut audit_secrets,
            );
        }
    }

    let test_files = discover_test_files(target_path)?;
    if test_files.is_empty() {
        bail!(
            "No test files (*.test.ts, *.spec.ts, *_test.ts, *.test.tsx, *.spec.tsx) found in {}",
            target_path.display()
        );
    }

    let suite_start = Instant::now();
    let mut total_passed = 0usize;
    let mut total_failed = 0usize;
    let mut total_skipped = 0usize;
    let mut total_rollbacks = 0usize;
    let mut all_results: Vec<serde_json::Value> = Vec::new();

    for test_file in &test_files {
        let bundled = slice_cas::bundle_entrypoint_with_deps(test_file)?;
        let filter_js = match &filter_opt {
            Some(flt) => format!(
                "globalThis.__slice_test_filter = {};",
                serde_json::to_string(flt).unwrap_or_else(|_| "\"\"".to_string())
            ),
            None => "globalThis.__slice_test_filter = \"\";".to_string(),
        };
        let wrapped_js = format!(
            "{}\nSlice.resetTests();\n{}\nconsole.log(\"__SLICE_TEST_REPORT__:\" + JSON.stringify(Slice.testSummary()));\n",
            filter_js, bundled
        );
        let bytecode = compile_source_to_bytecode(&wrapped_js);

        let mut isolate =
            SliceIsolate::spawn("slice-test-runner", 1, caps.clone(), fuel_limit, mem_limit);
        isolate.set_deterministic_seed(deterministic_seed);
        for sec in &audit_secrets {
            isolate.add_egress_audit_secret(sec.clone());
        }
        let metrics = isolate.execute_bytecode(&bytecode, None)?;
        if metrics.return_value.starts_with("Uncaught ") {
            let mapped = slice_cas::map_bundled_error_location(&bundled, &metrics.return_value);
            bail!("Uncaught error in {}: {}", test_file.display(), mapped);
        }

        for line in &metrics.stdout {
            if let Some(report_json) = line.strip_prefix("__SLICE_TEST_REPORT__:") {
                let parsed: serde_json::Value = serde_json::from_str(report_json)?;
                let p = parsed["passed"].as_u64().unwrap_or(0) as usize;
                let f = parsed["failed"].as_u64().unwrap_or(0) as usize;
                let s = parsed["skipped"].as_u64().unwrap_or(0) as usize;
                let rb = parsed["checkpointsRestored"].as_u64().unwrap_or(0) as usize;
                total_passed += p;
                total_failed += f;
                total_skipped += s;
                total_rollbacks += rb;
                if let Some(arr) = parsed["results"].as_array() {
                    for r in arr {
                        if !json_output {
                            let status = if r["passed"].as_bool().unwrap_or(false) {
                                "PASS"
                            } else {
                                "FAIL"
                            };
                            let full_name = r["fullName"].as_str().unwrap_or("test");
                            let dur_us = r["durationUs"].as_u64().unwrap_or(1);
                            let snap_id = r["checkpointId"].as_u64().unwrap_or(0);
                            println!(
                                "  [{}] {} ({} us, VFS+KV Rollback #{})",
                                status, full_name, dur_us, snap_id
                            );
                            if let Some(err_str) = r["error"].as_str() {
                                let mapped_err =
                                    slice_cas::map_bundled_error_location(&bundled, err_str);
                                println!("         -> {}", mapped_err);
                            }
                        }
                        all_results.push(r.clone());
                    }
                }
            } else if !json_output {
                println!("{}", line);
            }
        }
    }

    let elapsed_ms = suite_start.elapsed().as_secs_f64() * 1_000.0;
    if json_output {
        let out = serde_json::json!({
            "files": test_files.len(),
            "total": total_passed + total_failed + total_skipped,
            "passed": total_passed,
            "failed": total_failed,
            "skipped": total_skipped,
            "checkpointsRestored": total_rollbacks,
            "deterministicSeed": deterministic_seed,
            "elapsedMs": elapsed_ms,
            "results": all_results,
        });
        println!("{}", serde_json::to_string(&out)?);
    } else {
        println!(
            "Slice Time-Travel Test Suite: {} passed, {} failed, {} skipped ({} VFS+KV rollbacks in {:.2} ms, seed={:?})",
            total_passed, total_failed, total_skipped, total_rollbacks, elapsed_ms, deterministic_seed
        );
    }

    if total_failed > 0 {
        bail!("{} test(s) failed", total_failed);
    }
    Ok(())
}
