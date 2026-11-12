use crate::config::{install_ctrlc_handler, OsNativeFileWatcher, ResolvedExecutionConfig};
use anyhow::{bail, Result};
use slice_engine::{compile_source_to_bytecode, SliceIsolate};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Maximum directory recursion depth when discovering test files (`CLI-SEC-03`).
pub(crate) const MAX_TEST_WALK_DEPTH: usize = 64;

#[inline]
fn is_test_filename(name: &str) -> bool {
    name.ends_with(".test.ts")
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
}

/// Discovers test files under `target`, skipping symlinks (`CLI-SEC-03`) and using `DirEntry::file_type()`
/// to avoid redundant OS `stat` syscalls (`CLI-PERF-03`).
pub(crate) fn discover_test_files(target: &Path) -> Result<Vec<PathBuf>> {
    if target.is_file() {
        return Ok(vec![target.to_path_buf()]);
    }
    let mut found = Vec::new();
    fn walk_tests(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        if depth > MAX_TEST_WALK_DEPTH {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<(PathBuf, std::fs::FileType)> = rd
            .flatten()
            .filter_map(|e| e.file_type().ok().map(|ft| (e.path(), ft)))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (p, ft) in entries {
            if ft.is_symlink() {
                continue;
            }
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or_default();
            if matches!(name, "node_modules" | "target" | ".git" | ".archive") {
                continue;
            }
            if ft.is_dir() {
                walk_tests(&p, depth + 1, out);
            } else if ft.is_file() && is_test_filename(name) {
                out.push(p);
            }
        }
    }
    walk_tests(target, 0, &mut found);
    Ok(found)
}

pub(crate) fn cmd_test(args: &[String]) -> Result<()> {
    let mut exec_cfg = ResolvedExecutionConfig::from_cli_args(args)?;
    let mut filter_opt: Option<String> = None;
    let mut deterministic_seed: Option<u64> = Some(20261024);
    let mut json_output = false;
    let mut watch_mode = false;
    let mut watch_once = false;
    let mut target_opt: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--config" {
            i += 2;
            continue;
        }
        if let Some(f) = arg.strip_prefix("--filter=") {
            filter_opt = Some(f.to_string());
        } else if let Some(seed_str) = arg.strip_prefix("--deterministic=") {
            deterministic_seed = Some(seed_str.parse().unwrap_or(20261024));
        } else if arg == "--no-deterministic" {
            deterministic_seed = None;
        } else if arg == "--json" {
            json_output = true;
        } else if arg == "--watch" {
            watch_mode = true;
        } else if arg == "--watch-once" {
            watch_mode = true;
            watch_once = true;
        } else if !arg.starts_with('-') && target_opt.is_none() {
            target_opt = Some(arg.clone());
        }
        i += 1;
    }

    let target_str = target_opt.unwrap_or_else(|| ".".to_string());
    let target_path = Path::new(&target_str);
    exec_cfg.apply_target_path_manifest(target_path);

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
        let mut file_cfg = exec_cfg.clone();
        if let Ok(raw_test) = std::fs::read_to_string(test_file) {
            file_cfg.apply_inline_directives(&raw_test, test_file.parent());
        }
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

        let mut isolate = SliceIsolate::spawn(
            "slice-test-runner",
            1,
            file_cfg.caps,
            file_cfg.fuel_limit,
            file_cfg.mem_limit,
        );
        isolate.set_deterministic_seed(deterministic_seed.or(file_cfg.deterministic_seed));
        for sec in &file_cfg.audit_secrets {
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

    if total_failed > 0 && !watch_mode {
        bail!("{} test(s) failed", total_failed);
    }

    if watch_mode {
        if !watch_once {
            install_ctrlc_handler();
        }
        let mut watcher = OsNativeFileWatcher::new(target_path);
        while watcher.wait_for_change(watch_once) {
            let reload_start = Instant::now();
            let mut rerun_passed = 0usize;
            let mut rerun_failed = 0usize;
            for test_file in &discover_test_files(target_path)? {
                let mut file_cfg = exec_cfg.clone();
                if let Ok(raw_test) = std::fs::read_to_string(test_file) {
                    file_cfg.apply_inline_directives(&raw_test, test_file.parent());
                }
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
                let mut isolate = SliceIsolate::spawn(
                    "slice-test-watch",
                    1,
                    file_cfg.caps,
                    file_cfg.fuel_limit,
                    file_cfg.mem_limit,
                );
                isolate.set_deterministic_seed(deterministic_seed.or(file_cfg.deterministic_seed));
                for sec in &file_cfg.audit_secrets {
                    isolate.add_egress_audit_secret(sec.clone());
                }
                let metrics = isolate.execute_bytecode(&bytecode, None)?;
                for line in &metrics.stdout {
                    if let Some(report_json) = line.strip_prefix("__SLICE_TEST_REPORT__:") {
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(report_json) {
                            rerun_passed += parsed["passed"].as_u64().unwrap_or(0) as usize;
                            rerun_failed += parsed["failed"].as_u64().unwrap_or(0) as usize;
                        }
                    }
                }
            }
            let reload_ms = reload_start.elapsed().as_secs_f64() * 1_000.0;
            if !json_output {
                println!(
                    "[Slice Test Watch] Re-ran {} passed, {} failed in {:.2} ms",
                    rerun_passed, rerun_failed, reload_ms
                );
            }
            if watch_once {
                if rerun_failed > 0 {
                    bail!("{} test(s) failed on watch rerun", rerun_failed);
                }
                break;
            }
        }
    }

    Ok(())
}
