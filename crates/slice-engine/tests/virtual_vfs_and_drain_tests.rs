use crate::{
    DrainLimits, SandboxEvalOptions, SkipReason, SliceIsolate, compile_source_to_bytecode,
};
use slice_sandbox::{CapabilityToken, SandboxViolation};

#[test]
fn test_v0150_virtual_mount_zero_host_fallthrough_and_preseeded_dir() {
    let unique = format!(
        "slice_vm_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let host_dir = std::env::temp_dir().join(&unique);
    std::fs::create_dir_all(&host_dir).unwrap();
    let host_secret = host_dir.join("host_secret.txt");
    std::fs::write(&host_secret, "TOP_SECRET_HOST_CONTENT").unwrap();
    let host_dir_str = host_dir.to_string_lossy().replace('\\', "/");

    let caps = CapabilityToken::pure_compute()
        .with_virtual_mount(&host_dir_str)
        .with_virtual_mount("/artifacts");

    let code = format!(
        r#"
        import fs from "node:fs";
        const hostDir = {host_dir_json};
        const secretExists = fs.existsSync(hostDir + "/host_secret.txt");
        const initialEntries = fs.readdirSync(hostDir);
        let readThrew = false;
        try {{
            fs.readFileSync(hostDir + "/host_secret.txt", "utf8");
        }} catch (_e) {{
            readThrew = true;
        }}
        fs.writeFileSync(hostDir + "/guest_only.txt", "virtual-data");
        const guestRead = fs.readFileSync(hostDir + "/guest_only.txt", "utf8");

        // Pre-seeded /artifacts directory works immediately without mkdirSync
        const artIsDir = fs.statSync("/artifacts").isDirectory();
        fs.writeFileSync("/artifacts/hello.txt", "from-guest");
        const artRead = fs.readFileSync("/artifacts/hello.txt", "utf8");

        JSON.stringify({{
            secretExists,
            initialLen: initialEntries.length,
            readThrew,
            guestRead,
            artIsDir,
            artRead
        }});
        "#,
        host_dir_json = serde_json::to_string(&host_dir_str).unwrap()
    );

    let bc = compile_source_to_bytecode(&code);
    let mut iso = SliceIsolate::spawn("vm-zero-fallthrough", 1, caps, 20_000_000, 16 * 1024 * 1024);
    let metrics = iso.execute_bytecode(&bc, None).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&metrics.return_value).unwrap();
    assert_eq!(parsed["secretExists"], false);
    assert_eq!(parsed["initialLen"], 0);
    assert_eq!(parsed["readThrew"], true);
    assert_eq!(parsed["guestRead"], "virtual-data");
    assert_eq!(parsed["artIsDir"], true);
    assert_eq!(parsed["artRead"], "from-guest");

    // Host directory must be untouched: guest_only.txt must NOT exist on host, host_secret.txt intact
    assert!(!host_dir.join("guest_only.txt").exists());
    assert_eq!(
        std::fs::read_to_string(&host_secret).unwrap(),
        "TOP_SECRET_HOST_CONTENT"
    );

    let _ = std::fs::remove_dir_all(&host_dir);
}

#[test]
fn test_v0150_virtual_mount_checkpoint_restore_and_memory_ceiling() {
    // 1. Checkpoint / restore works on virtual mounts
    let caps = CapabilityToken::pure_compute()
        .with_virtual_mount("/artifacts")
        .with_checkpoints(true);
    let code = r#"
        import fs from "node:fs";
        fs.writeFileSync("/artifacts/a.txt", "keep");
        const snap = Slice.checkpoint();
        fs.writeFileSync("/artifacts/b.txt", "rollback-me");
        Slice.restore(snap);
        JSON.stringify({
            a: fs.existsSync("/artifacts/a.txt"),
            b: fs.existsSync("/artifacts/b.txt"),
        });
    "#;
    let bc = compile_source_to_bytecode(code);
    let mut iso = SliceIsolate::spawn("vm-checkpoint", 2, caps, 20_000_000, 16 * 1024 * 1024);
    let metrics = iso.execute_bytecode(&bc, None).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&metrics.return_value).unwrap();
    assert_eq!(parsed["a"], true);
    assert_eq!(parsed["b"], false);

    // 2. Memory ceiling bounds virtual-mount writes
    let caps_small = CapabilityToken::pure_compute().with_virtual_mount("/artifacts");
    let big_code = r#"
        import fs from "node:fs";
        fs.writeFileSync("/artifacts/huge.bin", "X".repeat(512 * 1024));
    "#;
    let big_bc = compile_source_to_bytecode(big_code);
    let mut iso_small = SliceIsolate::spawn("vm-mem-cap", 3, caps_small, 20_000_000, 128 * 1024);
    let err = iso_small.execute_bytecode(&big_bc, None).unwrap_err();
    assert!(
        matches!(err, SandboxViolation::MemoryCeilingExceeded { .. }),
        "expected MemoryCeilingExceeded, got {err:?}"
    );
}

#[test]
fn test_v0150_drain_vfs_sorted_symlinks_dlp_limits_and_post_violation() {
    // 1. Sorted order, empty dir ignored, symlink skipped, DLP secret dropped + denied_capabilities
    let caps = CapabilityToken::pure_compute().with_virtual_mount("/artifacts");
    let code = r#"
        import fs from "node:fs";
        fs.mkdirSync("/artifacts/empty_dir", { recursive: true });
        fs.mkdirSync("/artifacts/nested", { recursive: true });
        fs.writeFileSync("/artifacts/z_last.txt", "hello-z");
        fs.writeFileSync("/artifacts/nested/a_first.txt", "hello-a");
        fs.writeFileSync("/artifacts/secret_leak.txt", "api_key=sk_live_drain_secret_987654");
        fs.symlinkSync("/artifacts/z_last.txt", "/artifacts/bad_link");
        "ok";
    "#;
    let bc = compile_source_to_bytecode(code);
    let mut iso = SliceIsolate::spawn("drain-full", 4, caps, 20_000_000, 16 * 1024 * 1024);
    iso.add_egress_audit_secret("sk_live_drain_secret_987654");
    let _ = iso.execute_bytecode(&bc, None).unwrap();

    let report = iso.drain_vfs("/artifacts", &DrainLimits::default());
    let drained_paths: Vec<&str> = report.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(drained_paths, vec!["nested/a_first.txt", "z_last.txt"]);
    assert_eq!(report.files[0].bytes, b"hello-a");
    assert_eq!(report.files[1].bytes, b"hello-z");

    assert!(
        report
            .skipped
            .iter()
            .any(|s| s.path == "bad_link" && s.reason == SkipReason::Symlink),
        "expected symlink to be skipped: {:?}",
        report.skipped
    );
    assert!(
        report
            .skipped
            .iter()
            .any(|s| s.path == "secret_leak.txt" && s.reason == SkipReason::AuditedSecret),
        "expected DLP secret file to be skipped: {:?}",
        report.skipped
    );
    assert!(
        iso.denied_capabilities()
            .iter()
            .any(|c| c == "artifact:secret_leak.txt"),
        "expected artifact:secret_leak.txt in denied_capabilities: {:?}",
        iso.denied_capabilities()
    );

    // Second drain is empty because files were moved out of VFS
    let second = iso.drain_vfs("/artifacts", &DrainLimits::default());
    assert!(second.files.is_empty());

    // 2. Drain after fuel exhaustion / OCap violation still collects files written before abort
    let caps_fail = CapabilityToken::pure_compute().with_virtual_mount("/artifacts");
    let fail_code = r#"
        import fs from "node:fs";
        fs.writeFileSync("/artifacts/partial.txt", "saved-before-abort");
        while (true) {}
    "#;
    let fail_bc = compile_source_to_bytecode(fail_code);
    let mut iso_fail =
        SliceIsolate::spawn("drain-after-fail", 5, caps_fail, 50_000, 16 * 1024 * 1024);
    let err = iso_fail.execute_bytecode(&fail_bc, None).unwrap_err();
    assert!(matches!(err, SandboxViolation::CpuFuelExhausted { .. }));

    let partial_report = iso_fail.drain_vfs("/artifacts", &DrainLimits::default());
    assert_eq!(partial_report.files.len(), 1);
    assert_eq!(partial_report.files[0].path, "partial.txt");
    assert_eq!(partial_report.files[0].bytes, b"saved-before-abort");

    // 3. DrainLimits enforcement (max_files, max_file_bytes, max_total_bytes, max_path_len)
    let caps_lim = CapabilityToken::pure_compute().with_virtual_mount("/artifacts");
    let lim_code = r#"
        import fs from "node:fs";
        fs.writeFileSync("/artifacts/a.txt", "1234");
        fs.writeFileSync("/artifacts/b.txt", "1234567890123456");
        fs.writeFileSync("/artifacts/c.txt", "1234");
        fs.writeFileSync("/artifacts/very_long_filename_exceeding_limit.txt", "ok");
    "#;
    let lim_bc = compile_source_to_bytecode(lim_code);
    let mut iso_lim =
        SliceIsolate::spawn("drain-limits", 6, caps_lim, 20_000_000, 16 * 1024 * 1024);
    let _ = iso_lim.execute_bytecode(&lim_bc, None).unwrap();
    let limits = DrainLimits {
        max_files: 1,
        max_file_bytes: 8,
        max_total_bytes: 8,
        max_path_len: 12,
    };
    let lim_report = iso_lim.drain_vfs("/artifacts", &limits);
    assert_eq!(lim_report.files.len(), 1);
    assert_eq!(lim_report.files[0].path, "a.txt");
    assert!(
        lim_report
            .skipped
            .iter()
            .any(|s| s.path == "very_long_filename_exceeding_limit.txt"
                && s.reason == SkipReason::PathTooLong)
    );
    assert!(
        lim_report
            .skipped
            .iter()
            .any(|s| s.path == "b.txt" && s.reason == SkipReason::TooMany)
    );
}

#[test]
fn test_v0150_eval_sandbox_artifacts_drain_on_success_and_violation() {
    let mut host = SliceIsolate::spawn(
        "eval-sandbox-art-host",
        7,
        CapabilityToken::pure_compute(),
        20_000_000,
        16 * 1024 * 1024,
    );
    host.add_egress_audit_secret("sk_live_eval_sandbox_secret");

    let outcome = host.eval_sandbox(
        r#"
        import fs from "node:fs";
        fs.writeFileSync("/artifacts/chart.csv", "x,y\n1,2");
        fs.writeFileSync("/artifacts/bad.txt", "sk_live_eval_sandbox_secret");
        "done";
        "#,
        SandboxEvalOptions {
            artifacts: true,
            ..Default::default()
        },
    );
    assert!(outcome.ok);
    let report = outcome
        .drained_artifacts
        .expect("expected drained_artifacts");
    assert_eq!(report.files.len(), 1);
    assert_eq!(report.files[0].path, "chart.csv");
    assert_eq!(report.files[0].bytes, b"x,y\n1,2");
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].reason, SkipReason::AuditedSecret);
    assert!(
        outcome
            .denied_capabilities
            .iter()
            .any(|c| c == "artifact:bad.txt")
    );
}
