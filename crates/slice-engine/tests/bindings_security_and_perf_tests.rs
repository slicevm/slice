use crate::{HttpRequestContext, SliceIsolate, compile_source_to_bytecode};
use slice_sandbox::{CapabilityToken, SandboxViolation};

fn eval_ts_with_caps(
    tenant: &str,
    script: &str,
    caps: CapabilityToken,
) -> Result<String, SandboxViolation> {
    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let mut iso = SliceIsolate::spawn(tenant, 1, caps, 10_000_000, 16 * 1024 * 1024);
    let m = iso.execute_bytecode(&bc, None)?;
    Ok(m.stdout.join("\n"))
}

#[test]
fn test_sec01_vfs_symlink_target_ocap_escape_defense() {
    // 1. Direct symlinkSync creation pointing to an unauthorized target must require FsRead on target.
    let caps = CapabilityToken::pure_compute()
        .with_read_prefix("/tmp")
        .with_write_prefix("/tmp");
    let err = eval_ts_with_caps(
        "sec01-symlink-create",
        r#"
        import fs from "node:fs";
        fs.mkdirSync("/tmp", { recursive: true });
        fs.symlinkSync("/etc/shadow", "/tmp/evil_link");
        "#,
        caps,
    )
    .expect_err("symlinkSync to unauthorized target must fail OCap check");
    assert!(
        matches!(&err, SandboxViolation::FsReadDenied { path, .. } if path == "/etc/shadow"),
        "expected FsReadDenied(/etc/shadow) violation, got: {:?}",
        err
    );

    // 2. If a parent isolate creates a legitimate VFS symlink to `/secret/key.pem`, and then
    //    spawns a restricted `Slice.evalSandbox` that only has `/tmp` access, any attempt by the
    //    child sandbox to read, copy, overwrite, append, stat, or probe `/tmp/key_link` must be blocked.
    let parent_caps = CapabilityToken::pure_compute()
        .with_read_prefix("/")
        .with_write_prefix("/");
    let out = eval_ts_with_caps(
        "sec01-symlink-sandbox",
        r#"
        import fs from "node:fs";
        fs.mkdirSync("/secret", { recursive: true });
        fs.mkdirSync("/tmp", { recursive: true });
        fs.writeFileSync("/secret/key.pem", "TOP_SECRET_PEM_DATA");
        fs.symlinkSync("/secret/key.pem", "/tmp/key_link");

        const childCaps = { readPrefixes: ["/tmp"], writePrefixes: ["/tmp"] };

        const copyAttempt = Slice.evalSandbox(`
            const fs = require("node:fs");
            fs.copyFileSync("/tmp/key_link", "/tmp/stolen.pem");
        `, childCaps);

        const writeAttempt = Slice.evalSandbox(`
            const fs = require("node:fs");
            fs.writeFileSync("/tmp/key_link", "OVERWRITTEN");
        `, childCaps);

        const appendAttempt = Slice.evalSandbox(`
            const fs = require("node:fs");
            fs.appendFileSync("/tmp/key_link", "_APPENDED");
        `, childCaps);

        const statAttempt = Slice.evalSandbox(`
            const fs = require("node:fs");
            fs.statSync("/tmp/key_link");
        `, childCaps);

        const existsAttempt = Slice.evalSandbox(`
            const fs = require("node:fs");
            fs.existsSync("/tmp/key_link");
        `, childCaps);

        console.log(JSON.stringify({
            copyOk: copyAttempt.ok,
            writeOk: writeAttempt.ok,
            appendOk: appendAttempt.ok,
            statOk: statAttempt.ok,
            existsOk: existsAttempt.ok,
            secretIntact: fs.readFileSync("/secret/key.pem", "utf8"),
        }));
        "#,
        parent_caps,
    )
    .expect("parent eval should succeed");

    let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        parsed["copyOk"], false,
        "copyFileSync via symlink escaped OCap: {}",
        out
    );
    assert_eq!(
        parsed["writeOk"], false,
        "writeFileSync via symlink escaped OCap: {}",
        out
    );
    assert_eq!(
        parsed["appendOk"], false,
        "appendFileSync via symlink escaped OCap: {}",
        out
    );
    assert_eq!(
        parsed["statOk"], false,
        "statSync via symlink escaped OCap: {}",
        out
    );
    assert_eq!(
        parsed["existsOk"], false,
        "existsSync via symlink leaked target existence: {}",
        out
    );
    assert_eq!(
        parsed["secretIntact"], "TOP_SECRET_PEM_DATA",
        "secret was modified: {}",
        out
    );
}

#[test]
fn test_sec02_isolate_memory_ceiling_on_vfs_stdout_and_random_bytes() {
    let caps = CapabilityToken::pure_compute()
        .with_read_prefix("/tmp")
        .with_write_prefix("/tmp");
    let out = eval_ts_with_caps(
        "sec02-mem-ceiling",
        r#"
        const vfsRes = Slice.evalSandbox(`
            const fs = require("node:fs");
            fs.writeFileSync("/tmp/big.txt", "A".repeat(16384));
        `, { maxMemoryBytes: 4096, readPrefixes: ["/tmp"], writePrefixes: ["/tmp"] });

        const stdoutRes = Slice.evalSandbox(`
            console.log("B".repeat(16384));
        `, { maxMemoryBytes: 4096 });

        const randRes = Slice.evalSandbox(`
            const crypto = require("node:crypto");
            crypto.randomBytes(16384);
        `, { maxMemoryBytes: 4096 });

        console.log(JSON.stringify({
            vfsOk: vfsRes.ok,
            stdoutOk: stdoutRes.ok,
            randOk: randRes.ok,
        }));
        "#,
        caps,
    )
    .expect("memory ceiling sandbox test should run");

    let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        parsed["vfsOk"], false,
        "VFS write bypassed memory ceiling: {}",
        out
    );
    assert_eq!(
        parsed["stdoutOk"], false,
        "console.log bypassed memory ceiling: {}",
        out
    );
    assert_eq!(
        parsed["randOk"], false,
        "crypto.randomBytes bypassed memory ceiling: {}",
        out
    );
}

#[test]
fn test_sec03_zlib_zip_bomb_memory_ceiling_and_fuel_accounting() {
    let caps = CapabilityToken::pure_compute();
    let out = eval_ts_with_caps(
        "sec03-zlib-bomb",
        r#"
        import zlib from "node:zlib";
        // 128 KB of zeros compresses down to a tiny ~150-byte DEFLATE payload.
        const rawZeros = new Uint8Array(131072);
        const compressed = zlib.deflateRawSync(rawZeros);
        const b64 = Buffer.from(compressed).toString("base64");

        // Attempt to decompress inside a sandbox with a 16 KB memory ceiling.
        const bombRes = Slice.evalSandbox(`
            const zlib = require("node:zlib");
            const payload = Buffer.from("${b64}", "base64");
            zlib.inflateRawSync(payload);
        `, { maxMemoryBytes: 16384 });

        console.log(JSON.stringify({
            compressedLen: compressed.byteLength,
            bombOk: bombRes.ok,
            bombErr: bombRes.error || "",
        }));
        "#,
        caps,
    )
    .expect("zip bomb sandbox test should run");

    let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        parsed["bombOk"], false,
        "DEFLATE zip-bomb bypassed isolate memory ceiling: {}",
        out
    );
}

#[test]
fn test_sec04_http_response_header_egress_dlp_blocking() {
    let script = r#"
    export default {
        fetch(req) {
            return new Response("publicly safe body", {
                status: 200,
                headers: {
                    "X-Exfiltrated-Key": "Bearer sk-live-super-secret-9999"
                }
            });
        }
    };
    "#;
    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let caps = CapabilityToken::pure_compute();
    let mut iso = SliceIsolate::spawn("sec04-http-dlp", 1, caps, 5_000_000, 8 * 1024 * 1024);
    let dlp_count = iso.load_dotenv_str("STRIPE_SECRET_KEY=sk-live-super-secret-9999\n");
    assert_eq!(dlp_count, 1);

    let req = HttpRequestContext {
        method: "GET",
        path: "/leak",
        query: "",
        tenant_id: "sec04-http-dlp",
        headers: &[],
        body: "",
        body_bytes: None,
    };

    let err = iso
        .execute_http_request(&bc, None, &req)
        .expect_err("HTTP response header containing secret must be blocked by Egress DLP");

    assert!(
        matches!(&err, SandboxViolation::EgressSecretLeakBlocked { .. }),
        "expected EGRESS_SECRET_LEAK_BLOCKED violation, got: {:?}",
        err
    );
}

#[test]
fn test_sec05_child_process_cwd_ocap_and_loader_env_sanitization() {
    let caps = CapabilityToken::pure_compute()
        .with_exec_binary("echo")
        .with_read_prefix("/allowed");
    let err = eval_ts_with_caps(
        "sec05-cp-cwd",
        r#"
        import cp from "node:child_process";
        cp.spawnSync("echo", ["hi"], { cwd: "/forbidden/dir" });
        "#,
        caps,
    )
    .expect_err("spawnSync with unauthorized cwd must be rejected by OCap");

    assert!(
        matches!(&err, SandboxViolation::FsReadDenied { path, .. } if path == "/forbidden/dir"),
        "expected FsReadDenied(/forbidden/dir) violation, got: {:?}",
        err
    );
}

#[test]
fn test_perf01_02_03_zero_copy_typed_array_subarray_dataview_and_btoa_throughput() {
    let caps = CapabilityToken::pure_compute();
    let out = eval_ts_with_caps(
        "perf01-zero-copy",
        r#"
        import crypto from "node:crypto";

        // Verify subarray & DataView with non-zero byteOffset work identically via zero-copy downcast.
        const backing = new Uint8Array([0, 1, 2, 3, 65, 66, 67, 68, 9, 9]);
        const sub = backing.subarray(4, 8); // "ABCD"
        const dv = new DataView(backing.buffer, 4, 4);

        const hSub = crypto.createHash("sha256").update(sub).digest("hex");
        const hDv = crypto.createHash("sha256").update(dv).digest("hex");
        const hStr = crypto.createHash("sha256").update("ABCD").digest("hex");
        if (hSub !== hStr || hDv !== hStr) {
            throw new Error(`Hash mismatch: sub=${hSub} dv=${hDv} str=${hStr}`);
        }

        const macSub = crypto.createHmac("sha256", sub).update(sub).digest("hex");
        const macStr = crypto.createHmac("sha256", "ABCD").update("ABCD").digest("hex");
        if (macSub !== macStr) {
            throw new Error(`HMAC mismatch: ${macSub} vs ${macStr}`);
        }

        if (!crypto.timingSafeEqual(sub, dv)) {
            throw new Error("timingSafeEqual failed on matching subarray and DataView");
        }

        // TextEncoder.encodeInto on subarray view
        const destBacking = new Uint8Array(16);
        const destSub = destBacking.subarray(4, 12);
        const encRes = new TextEncoder().encodeInto("Hello!", destSub);
        const decoded = new TextDecoder().decode(destBacking.subarray(4, 4 + encRes.written));
        if (decoded !== "Hello!" || destBacking[0] !== 0) {
            throw new Error(`encodeInto subarray corrupted offset: ${decoded}`);
        }

        // crypto.getRandomValues on integer TypedArray subarray
        const rngBacking = new Uint8Array(32);
        crypto.getRandomValues(rngBacking.subarray(8, 24));
        if (rngBacking[0] !== 0 || rngBacking[31] !== 0) {
            throw new Error("getRandomValues wrote outside subarray bounds");
        }

        // High-throughput micro-benchmark: 1,000 iterations of zero-copy TypedArray ops
        const payload = new Uint8Array(256);
        crypto.getRandomValues(payload);
        const encTarget = new Uint8Array(64);
        const enc = new TextEncoder();
        const dec = new TextDecoder();
        const t0 = performance.now();
        for (let i = 0; i < 1000; i++) {
            crypto.createHash("sha256").update(payload).digest("hex");
            crypto.getRandomValues(encTarget);
            enc.encodeInto("fast-zero-copy-host-bindings-benchmark", encTarget);
            dec.decode(encTarget);
            const b = btoa("fast-ascii-binary-string-without-vec-alloc");
            atob(b);
        }
        const elapsedMs = performance.now() - t0;
        if (elapsedMs < 0) {
            throw new Error("Invalid elapsed time");
        }

        console.log("OK");
        "#,
        caps,
    )
    .expect("zero-copy TypedArray correctness and throughput test should succeed");

    assert_eq!(out.trim(), "OK");
}

#[test]
fn test_default_pure_compute_fs_deny_by_default_across_all_surfaces() {
    let host_dir =
        std::env::temp_dir().join(format!("slice_default_ocap_probe_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&host_dir);
    std::fs::create_dir_all(&host_dir).unwrap();
    let host_file = host_dir.join("probe.txt");
    std::fs::write(&host_file, "SENSITIVE_HOST_PAYLOAD").unwrap();
    let p_file = host_file.to_string_lossy().replace('\\', "/");
    let p_dir = host_dir.to_string_lossy().replace('\\', "/");

    // 1. Default pure_compute() blocks readFileSync, readdirSync, existsSync, statSync
    for (label, expr) in [
        (
            "readFileSync",
            format!("fs.readFileSync({:?}, 'utf8')", p_file),
        ),
        ("readdirSync", format!("fs.readdirSync({:?})", p_dir)),
        ("existsSync", format!("fs.existsSync({:?})", p_file)),
        ("statSync", format!("fs.statSync({:?})", p_file)),
    ] {
        let script = format!("import * as fs from 'node:fs';\n{};\n", expr);
        let err = eval_ts_with_caps(
            "default-read-deny",
            &script,
            CapabilityToken::pure_compute(),
        )
        .expect_err("default pure_compute() must block host/VFS read");
        assert!(
            matches!(err, SandboxViolation::FsReadDenied { .. }),
            "expected FsReadDenied for {}, got: {:?}",
            label,
            err
        );
    }

    // 2. Default pure_compute() blocks writeFileSync, unlinkSync, rmSync, rmdirSync, mkdirSync
    for (label, expr) in [
        (
            "writeFileSync",
            format!("fs.writeFileSync({:?}, 'x')", p_file),
        ),
        ("unlinkSync", format!("fs.unlinkSync({:?})", p_file)),
        (
            "rmSync",
            format!("fs.rmSync({:?}, {{ recursive: true, force: true }})", p_dir),
        ),
        (
            "mkdirSync",
            format!("fs.mkdirSync({:?} + '/sub', {{ recursive: true }})", p_dir),
        ),
    ] {
        let script = format!("import * as fs from 'node:fs';\n{};\n", expr);
        let err = eval_ts_with_caps(
            "default-write-deny",
            &script,
            CapabilityToken::pure_compute(),
        )
        .expect_err("default pure_compute() must block host/VFS write and delete");
        assert!(
            matches!(err, SandboxViolation::FsWriteDenied { .. }),
            "expected FsWriteDenied for {}, got: {:?}",
            label,
            err
        );
    }
    assert!(
        host_file.exists(),
        "host file must remain untouched after denied rmSync/unlinkSync"
    );

    // 3. HTTP fast & standard request handlers and prewarm_boa_bundle also enforce default deny
    let http_script = format!(
        r#"
        import * as fs from "node:fs";
        let cached = "";
        try {{ cached = fs.readFileSync({:?}, "utf8"); }} catch (_e) {{}}
        (globalThis as any).__cachedLeak = cached;
        Slice.respond(200, cached || "empty", "text/plain");
        "#,
        p_file
    );
    let http_bundled = slice_cas::strip_typescript_and_normalize(&http_script, true);
    let http_bc = compile_source_to_bytecode(&http_bundled);
    SliceIsolate::prewarm_boa_bundle(&http_bc);

    let req = HttpRequestContext {
        method: "GET",
        path: "/leak",
        query: "",
        tenant_id: "http-default-deny",
        body: "",
        body_bytes: None,
        headers: &[],
    };
    let mut http_iso = SliceIsolate::spawn(
        "http-default-deny",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        4 * 1024 * 1024,
    );
    let fast_out = http_iso
        .execute_http_request_fast(&http_bc, None, &req)
        .expect("caught FsReadDenied in guest try/catch must return fallback response with audit log (#179)");
    assert_eq!(fast_out.response_body, "empty");
    assert!(
        http_iso
            .denied_capabilities()
            .iter()
            .any(|d| d.contains("lacks filesystem read authority")),
        "expected FsReadDenied to be recorded in denied_capabilities audit log, got: {:?}",
        http_iso.denied_capabilities()
    );

    // 4. Win32 device namespace and UNC share candidates are rejected even when "/" is granted
    let broad_caps = CapabilityToken::pure_compute()
        .with_read_prefix("/")
        .with_write_prefix("/");
    for bad_path in [
        r"\\.\PhysicalDrive0",
        "//./pipe/docker_engine",
        r"\\server\share\secret.txt",
        "//?/UNC/server/share/secret.txt",
    ] {
        let bad_script = format!(
            "import * as fs from 'node:fs';\nfs.readFileSync({:?}, 'utf8');\n",
            bad_path
        );
        let err = eval_ts_with_caps("device-unc-deny", &bad_script, broad_caps.clone())
            .expect_err("device/UNC path must be rejected");
        assert!(matches!(err, SandboxViolation::FsReadDenied { .. }));
    }

    let _ = std::fs::remove_dir_all(&host_dir);
}

#[test]
fn test_vfs_copy_on_write_rm_tombstones_and_checkpoint_restore_preserve_host_disk() {
    let victim_dir =
        std::env::temp_dir().join(format!("slice_cow_vfs_tombstone_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&victim_dir);
    let sub_dir = victim_dir.join("dir").join("sub");
    std::fs::create_dir_all(&sub_dir).unwrap();
    let root_file = victim_dir.join("file.txt");
    let nested_file = sub_dir.join("nested.txt");
    let rename_src = victim_dir.join("rename_me.txt");
    std::fs::write(&root_file, "host-root-data").unwrap();
    std::fs::write(&nested_file, "host-nested-data").unwrap();
    std::fs::write(&rename_src, "host-rename-payload").unwrap();

    let v_norm = victim_dir.to_string_lossy().replace('\\', "/");
    let caps = CapabilityToken::pure_compute()
        .with_read_prefix(&v_norm)
        .with_write_prefix(&v_norm);

    let script = format!(
        r#"
        import * as fs from "node:fs";
        const V = {:?};
        const snap = Slice.checkpoint();

        // Unlink file, recursively remove directory tree, and rename host-backed file
        fs.unlinkSync(V + "/file.txt");
        fs.rmSync(V + "/dir", {{ recursive: true, force: true }});
        fs.renameSync(V + "/rename_me.txt", V + "/renamed.txt");

        const goneAfterRm = !fs.existsSync(V + "/file.txt")
            && !fs.existsSync(V + "/dir")
            && !fs.existsSync(V + "/dir/sub/nested.txt")
            && !fs.existsSync(V + "/rename_me.txt")
            && fs.readFileSync(V + "/renamed.txt", "utf8") === "host-rename-payload";

        let readUnlinkedThrew = false;
        try {{ fs.readFileSync(V + "/file.txt", "utf8"); }} catch (_e) {{ readUnlinkedThrew = true; }}

        const entriesAfterRm = fs.readdirSync(V);

        // Recreate V + "/dir" in VFS and verify old host child does NOT reappear as a ghost
        fs.mkdirSync(V + "/dir", {{ recursive: true }});
        fs.writeFileSync(V + "/dir/fresh.txt", "fresh-vfs");
        const dirEntriesRecreated = fs.readdirSync(V + "/dir");
        const ghostStayedHidden = !fs.existsSync(V + "/dir/sub/nested.txt");

        // Roll back to pre-deletion checkpoint
        Slice.restore(snap);
        const restoredOk = fs.readFileSync(V + "/file.txt", "utf8") === "host-root-data"
            && fs.readFileSync(V + "/dir/sub/nested.txt", "utf8") === "host-nested-data"
            && fs.readFileSync(V + "/rename_me.txt", "utf8") === "host-rename-payload"
            && !fs.existsSync(V + "/renamed.txt");

        console.log(JSON.stringify({{
            goneAfterRm,
            readUnlinkedThrew,
            entriesAfterRm,
            dirEntriesRecreated,
            ghostStayedHidden,
            restoredOk
        }}));
        "#,
        v_norm
    );

    let out = eval_ts_with_caps("cow-tombstone-tenant", &script, caps).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(parsed["goneAfterRm"], true);
    assert_eq!(parsed["readUnlinkedThrew"], true);
    assert_eq!(parsed["entriesAfterRm"], serde_json::json!(["renamed.txt"]));
    assert_eq!(
        parsed["dirEntriesRecreated"],
        serde_json::json!(["fresh.txt"])
    );
    assert_eq!(parsed["ghostStayedHidden"], true);
    assert_eq!(parsed["restoredOk"], true);

    // Real host files and directory hierarchy must remain 100% intact on disk
    assert_eq!(
        std::fs::read_to_string(&root_file).unwrap(),
        "host-root-data"
    );
    assert_eq!(
        std::fs::read_to_string(&nested_file).unwrap(),
        "host-nested-data"
    );
    assert_eq!(
        std::fs::read_to_string(&rename_src).unwrap(),
        "host-rename-payload"
    );

    let _ = std::fs::remove_dir_all(&victim_dir);
}

#[test]
fn test_worker_fs_capability_attenuation_and_symlink_write_containment() {
    // 1. Zero-authority parent cannot mint allowRead/allowWrite for a spawned Worker
    let zero_worker_script = r#"
        async function main() {
            const w = new Worker(
                `const fs = require("node:fs"); fs.readFileSync("/tmp/probe.txt", "utf8");`,
                {
                    eval: true,
                    slice: { allowRead: ["/tmp"], allowWrite: ["/tmp"], fuel: 500000 }
                }
            );
            let errMsg = "";
            w.on("error", (err: any) => { errMsg = String(err && err.message ? err.message : err); });
            await new Promise<void>((resolve) => w.on("exit", () => resolve()));
            console.log(errMsg);
        }
        main();
    "#;
    let out_zero = eval_ts_with_caps(
        "worker-zero-parent",
        zero_worker_script,
        CapabilityToken::pure_compute(),
    )
    .unwrap();
    assert!(
        out_zero.contains("FsReadDenied") || out_zero.contains("filesystem read authority"),
        "expected worker spawned by pure_compute() parent to trap FsReadDenied, got: {}",
        out_zero
    );

    // 2. Scoped parent (/tmp/app) cannot escalate worker to sibling prefix (/tmp/app-secrets)
    let boundary_caps = CapabilityToken::pure_compute()
        .with_read_prefix("/tmp/app")
        .with_write_prefix("/tmp/app");
    let boundary_script = r#"
        async function main() {
            const w = new Worker(
                `const fs = require("node:fs"); fs.readFileSync("/tmp/app-secrets/token.txt", "utf8");`,
                {
                    eval: true,
                    slice: { allowRead: ["/tmp/app-secrets"], fuel: 500000 }
                }
            );
            let errMsg = "";
            w.on("error", (err: any) => { errMsg = String(err && err.message ? err.message : err); });
            await new Promise<void>((resolve) => w.on("exit", () => resolve()));
            console.log(errMsg);
        }
        main();
    "#;
    let out_boundary =
        eval_ts_with_caps("worker-boundary-parent", boundary_script, boundary_caps).unwrap();
    assert!(
        out_boundary.contains("FsReadDenied") || out_boundary.contains("filesystem read authority"),
        "expected worker prefix-boundary escalation to trap FsReadDenied, got: {}",
        out_boundary
    );

    // 3. VFS symlink pointing into read-only prefix blocks mkdirSync and chmodSync
    let sym_caps = CapabilityToken::pure_compute()
        .with_read_prefix("/ro_zone")
        .with_read_prefix("/rw_zone")
        .with_write_prefix("/rw_zone");
    let sym_chmod_err = eval_ts_with_caps(
        "symlink-chmod-ro",
        r#"
        import * as fs from "node:fs";
        fs.symlinkSync("/ro_zone/config.json", "/rw_zone/cfg_link");
        fs.chmodSync("/rw_zone/cfg_link", 0o777);
        "#,
        sym_caps.clone(),
    )
    .expect_err("chmodSync through symlink into read-only prefix must fail FsWriteDenied");
    assert!(matches!(
        sym_chmod_err,
        SandboxViolation::FsWriteDenied { .. }
    ));

    let sym_mkdir_err = eval_ts_with_caps(
        "symlink-mkdir-ro",
        r#"
        import * as fs from "node:fs";
        fs.symlinkSync("/ro_zone/new_dir", "/rw_zone/dir_link");
        fs.mkdirSync("/rw_zone/dir_link", { recursive: true });
        "#,
        sym_caps,
    )
    .expect_err("mkdirSync through symlink into read-only prefix must fail FsWriteDenied");
    assert!(matches!(
        sym_mkdir_err,
        SandboxViolation::FsWriteDenied { .. }
    ));
}

#[test]
fn test_child_process_custom_shell_metacharacters_and_interpreter_env_filtering() {
    let echo_only = CapabilityToken::pure_compute().with_exec_binary("echo");

    // 1. Custom `options.shell` executable path must not piggyback on `echo` allowance
    let custom_shell_err = eval_ts_with_caps(
        "cp-custom-shell",
        r#"
        import cp from "node:child_process";
        cp.spawnSync("echo", ["ok"], { shell: "/usr/bin/python3" });
        "#,
        echo_only.clone(),
    )
    .expect_err("custom shell binary without explicit ExecCap must fail");
    assert!(
        matches!(&custom_shell_err, SandboxViolation::ExecDenied { binary, .. } if binary == "/usr/bin/python3"),
        "expected ExecDenied(/usr/bin/python3), got: {:?}",
        custom_shell_err
    );

    // 2. Shell command injection via embedded newline, globbing, or backslash must require shell ExecCap
    for bad_cmd in [
        "echo ok\\n/usr/bin/id",
        "echo *.json",
        "echo \\\\$HOME",
        "echo ~root",
    ] {
        let script = format!(
            "import cp from 'node:child_process';\ncp.execSync(\"{}\");\n",
            bad_cmd
        );
        let err = eval_ts_with_caps("cp-meta-shell", &script, echo_only.clone()).expect_err(
            "shell metacharacter or newline command must require explicit shell ExecCap",
        );
        assert!(
            matches!(err, SandboxViolation::ExecDenied { .. }),
            "expected ExecDenied for {}, got: {:?}",
            bad_cmd,
            err
        );
    }

    // 3. Interpreter injection env vars (`NODE_OPTIONS`, `PYTHONPATH`, `PATH`, `IFS`, `BASH_FUNC_*`)
    // are stripped from child `opts.env` unless explicitly granted in `EnvCap`.
    #[cfg(unix)]
    {
        let printenv_caps = CapabilityToken::pure_compute().with_exec_binary("/usr/bin/env");
        let out = eval_ts_with_caps(
            "cp-env-filter",
            r#"
            import cp from "node:child_process";
            const res = cp.spawnSync("/usr/bin/env", [], {
                encoding: "utf8",
                env: {
                    SAFE_CUSTOM_VAR: "allowed_val",
                    NODE_OPTIONS: "--require /tmp/pwn.js",
                    PYTHONPATH: "/tmp/evil",
                    IFS: "/",
                    "BASH_FUNC_myfn%%": "() { id; }"
                }
            });
            console.log(String(res.stdout));
            "#,
            printenv_caps,
        )
        .unwrap();
        assert!(out.contains("SAFE_CUSTOM_VAR=allowed_val"));
        assert!(!out.contains("NODE_OPTIONS="));
        assert!(!out.contains("PYTHONPATH="));
        assert!(!out.contains("BASH_FUNC_"));
    }
}

#[test]
fn test_vfs_root_readdir_symlink_eloop_relative_symlinks_and_directory_rename() {
    // 1. Relative symlink target anchored outside capability zone is rejected at creation time
    let scoped_caps = CapabilityToken::pure_compute()
        .with_read_prefix("/workspace")
        .with_write_prefix("/workspace");
    let escape_err = eval_ts_with_caps(
        "vfs-rel-symlink-escape",
        r#"
        import * as fs from "node:fs";
        fs.mkdirSync("/workspace/sub", { recursive: true });
        fs.symlinkSync("../../etc/passwd", "/workspace/sub/escape_link");
        "#,
        scoped_caps.clone(),
    )
    .expect_err("relative symlink escaping /workspace must trap FsReadDenied");
    assert!(matches!(escape_err, SandboxViolation::FsReadDenied { .. }));

    // 2. Root readdirSync("/"), relative symlinks inside scope, 9-hop ELOOP, symlink rename, and directory subtree rename
    let root_caps = CapabilityToken::pure_compute()
        .with_read_prefix("/")
        .with_write_prefix("/workspace");
    let out = eval_ts_with_caps(
        "vfs-rich-ops",
        r#"
        import * as fs from "node:fs";

        fs.mkdirSync("/workspace/tree/nested", { recursive: true });
        fs.writeFileSync("/workspace/tree/root.txt", "root-val");
        fs.writeFileSync("/workspace/tree/nested/leaf.txt", "leaf-val");

        // Root readdirSync("/") must see "workspace"
        const rootEntries = fs.readdirSync("/");
        const sawWorkspaceAtRoot = rootEntries.includes("workspace");

        // Relative symlink inside /workspace/tree
        fs.symlinkSync("root.txt", "/workspace/tree/rel_link.txt");
        const relRead = fs.readFileSync("/workspace/tree/rel_link.txt", "utf8");
        const relTarget = fs.readlinkSync("/workspace/tree/rel_link.txt");

        // Rename symlink itself without modifying or deleting target
        fs.renameSync("/workspace/tree/rel_link.txt", "/workspace/tree/moved_link.txt");
        const linkMovedOk = !fs.existsSync("/workspace/tree/rel_link.txt")
            && fs.lstatSync("/workspace/tree/moved_link.txt").isSymbolicLink()
            && fs.readFileSync("/workspace/tree/moved_link.txt", "utf8") === "root-val"
            && fs.readFileSync("/workspace/tree/root.txt", "utf8") === "root-val";

        // Directory subtree rename
        fs.renameSync("/workspace/tree", "/workspace/renamed_tree");
        const dirRenamedOk = !fs.existsSync("/workspace/tree")
            && !fs.existsSync("/workspace/tree/nested/leaf.txt")
            && fs.readFileSync("/workspace/renamed_tree/nested/leaf.txt", "utf8") === "leaf-val"
            && fs.readFileSync("/workspace/renamed_tree/moved_link.txt", "utf8") === "root-val";

        // 9-hop symlink loop throws ELOOP (without leaking internal sentinel) and existsSync returns false
        fs.symlinkSync("/workspace/loop_b", "/workspace/loop_a");
        fs.symlinkSync("/workspace/loop_a", "/workspace/loop_b");
        let eloopCode = "";
        let eloopMsg = "";
        try {
            fs.readFileSync("/workspace/loop_a", "utf8");
        } catch (err: any) {
            eloopCode = String(err.code || "");
            eloopMsg = String(err.message || "");
        }
        const loopExists = fs.existsSync("/workspace/loop_a");

        console.log(JSON.stringify({
            sawWorkspaceAtRoot,
            relRead,
            relTarget,
            linkMovedOk,
            dirRenamedOk,
            eloopCode,
            leakedSentinel: eloopMsg.includes("SLICE_SYMLINK"),
            loopExists
        }));
        "#,
        root_caps,
    )
    .unwrap();

    let parsed: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(parsed["sawWorkspaceAtRoot"], true);
    assert_eq!(parsed["relRead"], "root-val");
    assert_eq!(parsed["relTarget"], "root.txt");
    assert_eq!(parsed["linkMovedOk"], true);
    assert_eq!(parsed["dirRenamedOk"], true);
    assert_eq!(parsed["eloopCode"], "ELOOP");
    assert_eq!(parsed["leakedSentinel"], false);
    assert_eq!(parsed["loopExists"], false);
}

#[cfg(unix)]
fn try_create_host_file_symlink(target: &str, link: &std::path::Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn try_create_host_file_symlink(target: &str, link: &std::path::Path) -> bool {
    std::os::windows::fs::symlink_file(target, link).is_ok()
}

#[cfg(unix)]
fn create_host_dir_link(target_dir: &std::path::Path, link_dir: &std::path::Path) -> bool {
    std::os::unix::fs::symlink(target_dir, link_dir).is_ok()
}

#[cfg(windows)]
fn create_host_dir_link(target_dir: &std::path::Path, link_dir: &std::path::Path) -> bool {
    if std::os::windows::fs::symlink_dir(target_dir, link_dir).is_ok() {
        return true;
    }
    // Fallback to unprivileged NTFS directory junction when Developer Mode is off
    std::process::Command::new("cmd")
        .args([
            "/C",
            "mklink",
            "/J",
            &link_dir.to_string_lossy(),
            &target_dir.to_string_lossy(),
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn test_issue_142_host_symlink_vfs_cow_coherence_and_ocap_containment() {
    let base_dir = std::env::temp_dir().join(format!(
        "slice_issue_142_host_sym_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let sandbox_dir = base_dir.join("sandbox");
    let ro_dir = base_dir.join("ro_zone");
    let outside_dir = base_dir.join("outside_secret");
    let real_sub_dir = sandbox_dir.join("real_sub");
    std::fs::create_dir_all(&real_sub_dir).unwrap();
    std::fs::create_dir_all(&ro_dir).unwrap();
    std::fs::create_dir_all(&outside_dir).unwrap();

    let host_target = sandbox_dir.join("host-target");
    std::fs::write(&host_target, b"original").unwrap();
    std::fs::write(real_sub_dir.join("item.txt"), b"sub-orig").unwrap();
    std::fs::write(ro_dir.join("readonly.cfg"), b"ro-immutable").unwrap();
    std::fs::write(outside_dir.join("secret.txt"), b"TOP_SECRET_HOST_BYTES").unwrap();

    // 1. Directory link / junction tests (runs on all Unix + Windows hosts including unprivileged NTFS junctions)
    let dir_link = sandbox_dir.join("dir-link");
    let dir_escape = sandbox_dir.join("dir-escape");
    assert!(
        create_host_dir_link(&real_sub_dir, &dir_link),
        "failed to create host directory link/junction"
    );
    assert!(
        create_host_dir_link(&outside_dir, &dir_escape),
        "failed to create outside host directory link/junction"
    );

    let sandbox_norm = sandbox_dir.to_string_lossy().replace('\\', "/");
    let ro_norm = ro_dir.to_string_lossy().replace('\\', "/");

    let rw_caps = CapabilityToken::pure_compute()
        .with_read_prefix(&sandbox_norm)
        .with_write_prefix(&sandbox_norm)
        .with_read_prefix(&ro_norm);

    // 1a. Host directory symlink / junction CoW write, VFS-only realpath, readdir, and tombstone coherence
    let dir_coherence_src = format!(
        r#"
        import * as fs from "node:fs";
        const root = {root:?};
        const beforeViaLink = fs.readFileSync(root + "/dir-link/item.txt", "utf8");
        fs.writeFileSync(root + "/real_sub/item.txt", "sub-updated");
        const afterViaLink = fs.readFileSync(root + "/dir-link/item.txt", "utf8");

        fs.writeFileSync(root + "/dir-link/created-via-link.txt", "from-link");
        const readDirect = fs.readFileSync(root + "/real_sub/created-via-link.txt", "utf8");
        const realVfsFile = fs.realpathSync(root + "/dir-link/created-via-link.txt");
        const listed = fs.readdirSync(root + "/dir-link");

        fs.unlinkSync(root + "/real_sub/item.txt");
        const existsViaLinkAfterUnlink = fs.existsSync(root + "/dir-link/item.txt");
        let readAfterUnlinkCode = "";
        try {{
            fs.readFileSync(root + "/dir-link/item.txt", "utf8");
        }} catch (e: any) {{
            readAfterUnlinkCode = String(e.code || "");
        }}

        // Unlinking a VFS symlink pointing into ro_zone or an ELOOP symlink in rw_zone must succeed on the link itself
        fs.symlinkSync({ro:?} + "/readonly.cfg", root + "/temp_ro_sym");
        fs.unlinkSync(root + "/temp_ro_sym");
        fs.symlinkSync(root + "/loop_2", root + "/loop_1");
        fs.symlinkSync(root + "/loop_1", root + "/loop_2");
        fs.unlinkSync(root + "/loop_1");
        fs.unlinkSync(root + "/loop_2");

        console.log(JSON.stringify({{
            beforeViaLink,
            afterViaLink,
            readDirect,
            realEndsWithRealSub: realVfsFile.endsWith("/real_sub/created-via-link.txt"),
            listedHasCreated: listed.includes("created-via-link.txt"),
            existsViaLinkAfterUnlink,
            readAfterUnlinkCode
        }}));
        "#,
        root = sandbox_norm,
        ro = ro_norm,
    );

    // Verify across direct source, precompiled bytecode, and warm pooled reset
    let dir_js = slice_cas::strip_typescript_and_normalize(&dir_coherence_src, true);
    let dir_bc = compile_source_to_bytecode(&dir_js);
    let mut pooled_iso = SliceIsolate::spawn(
        "host-dir-sym-pool-init",
        1,
        rw_caps.clone(),
        20_000_000,
        16 * 1024 * 1024,
    );
    for turn in 1..=2u32 {
        let tid = format!("host-dir-sym-pool-{}", turn);
        pooled_iso.reset_for_request(&tid, turn, 20_000_000, 16 * 1024 * 1024);
        let rec = pooled_iso.execute_bytecode(&dir_bc, None).unwrap();
        let out_str = rec.stdout.join("\n");
        assert!(
            !out_str.trim().is_empty(),
            "unexpected JS error in dir_coherence_src: return_value={:?}",
            rec.return_value
        );
        let p: serde_json::Value = serde_json::from_str(out_str.trim()).unwrap();
        assert_eq!(p["beforeViaLink"], "sub-orig");
        assert_eq!(p["afterViaLink"], "sub-updated");
        assert_eq!(p["readDirect"], "from-link");
        assert_eq!(p["realEndsWithRealSub"], true);
        assert_eq!(p["listedHasCreated"], true);
        assert_eq!(p["existsViaLinkAfterUnlink"], false);
        assert_eq!(p["readAfterUnlinkCode"], "ENOENT");
    }
    // Host file on disk must remain untouched
    assert_eq!(
        std::fs::read_to_string(real_sub_dir.join("item.txt")).unwrap(),
        "sub-orig"
    );

    // 1b. P0 OCap Escape Guard: host directory link pointing outside `--allow-read` MUST trap FsReadDenied
    for bad_op in [
        format!("fs.readdirSync({:?} + '/dir-escape');", sandbox_norm),
        format!(
            "fs.readFileSync({:?} + '/dir-escape/secret.txt', 'utf8');",
            sandbox_norm
        ),
        format!(
            "fs.lstatSync({:?} + '/dir-escape/secret.txt');",
            sandbox_norm
        ),
        format!(
            "fs.existsSync({:?} + '/dir-escape/secret.txt');",
            sandbox_norm
        ),
    ] {
        let script = format!("import * as fs from 'node:fs';\n{}", bad_op);
        let err = eval_ts_with_caps("host-dir-sym-escape", &script, rw_caps.clone())
            .expect_err("host dir symlink escaping allow-read must trap FsReadDenied");
        assert!(
            matches!(err, SandboxViolation::FsReadDenied { .. }),
            "expected FsReadDenied for `{}`, got: {:?}",
            bad_op,
            err
        );
    }

    // 1c. Write-only compartment cannot read existing host file bytes via appendFileSync or openSync("a")
    let write_only_caps = CapabilityToken::pure_compute().with_write_prefix(&sandbox_norm);
    let append_new_ok = eval_ts_with_caps(
        "write-only-new-file",
        &format!(
            "import * as fs from 'node:fs'; fs.appendFileSync({:?} + '/brand-new.log', 'ok'); console.log('PASS');",
            sandbox_norm
        ),
        write_only_caps.clone(),
    )
    .unwrap();
    assert_eq!(append_new_ok.trim(), "PASS");

    let append_host_err = eval_ts_with_caps(
        "write-only-existing-host",
        &format!(
            "import * as fs from 'node:fs'; fs.appendFileSync({:?} + '/host-target', 'leak');",
            sandbox_norm
        ),
        write_only_caps,
    )
    .expect_err("write-only compartment appending existing host file must require read OCap");
    assert!(matches!(
        append_host_err,
        SandboxViolation::FsReadDenied { .. }
    ));

    // 2. Host file symlinks (exact Issue #142 repro + write-through + FD sharing + OCap file escape)
    let host_link = sandbox_dir.join("host-link");
    let escape_link = sandbox_dir.join("escape-link");
    let ro_link = sandbox_dir.join("ro-link");
    let file_sym_supported = try_create_host_file_symlink("host-target", &host_link)
        && try_create_host_file_symlink("../outside_secret/secret.txt", &escape_link)
        && try_create_host_file_symlink("../ro_zone/readonly.cfg", &ro_link);

    if file_sym_supported {
        let file_sym_src = format!(
            r#"
            import * as fs from "node:fs";
            const root = {root:?};
            const before = fs.readFileSync(root + "/host-link", "utf8");
            fs.symlinkSync("host-target", root + "/guest-link");
            fs.writeFileSync(root + "/host-target", "updated");
            const target = fs.readFileSync(root + "/host-target", "utf8");
            const viaLink = fs.readFileSync(root + "/host-link", "utf8");
            const viaGuestLink = fs.readFileSync(root + "/guest-link", "utf8");

            // Write THROUGH host-link updates target without clobbering host-link's symlink status
            fs.appendFileSync(root + "/host-link", "-more");
            const linkStillSym = fs.lstatSync(root + "/host-link").isSymbolicLink();
            const targetAfterAppend = fs.readFileSync(root + "/host-target", "utf8");

            // Open FDs via host-link and host-target share the exact same live inode
            const fdLink = fs.openSync(root + "/host-link", "r+");
            const fdTarget = fs.openSync(root + "/host-target", "r");
            fs.writeSync(fdLink, Buffer.from("FD!"), 0, 3, 0);
            const rbuf = Buffer.alloc(12);
            const n = fs.readSync(fdTarget, rbuf, 0, 12, 0);
            fs.closeSync(fdLink);
            fs.closeSync(fdTarget);
            const fdShared = rbuf.subarray(0, n).toString("utf8");

            // Deleting target makes host-link dangling (ENOENT on read/realpath, true on lstat)
            fs.unlinkSync(root + "/host-target");
            const danglingExists = fs.existsSync(root + "/host-link");
            const danglingLstatSym = fs.lstatSync(root + "/host-link").isSymbolicLink();
            let danglingRealpathErr = "";
            try {{
                fs.realpathSync(root + "/host-link");
            }} catch (e: any) {{
                danglingRealpathErr = String(e.code || "");
            }}

            console.log(JSON.stringify({{
                before,
                target,
                viaLink,
                viaGuestLink,
                linkStillSym,
                targetAfterAppend,
                fdShared,
                danglingExists,
                danglingLstatSym,
                danglingRealpathErr
            }}));
            "#,
            root = sandbox_norm,
        );
        let out = eval_ts_with_caps("issue-142-file-sym", &file_sym_src, rw_caps.clone()).unwrap();
        let p: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(p["before"], "original");
        assert_eq!(p["target"], "updated");
        assert_eq!(p["viaLink"], "updated");
        assert_eq!(p["viaGuestLink"], "updated");
        assert_eq!(p["linkStillSym"], true);
        assert_eq!(p["targetAfterAppend"], "updated-more");
        assert_eq!(p["fdShared"], "FD!ated-more");
        assert_eq!(p["danglingExists"], false);
        assert_eq!(p["danglingLstatSym"], true);
        assert_eq!(p["danglingRealpathErr"], "ENOENT");
        assert_eq!(std::fs::read_to_string(&host_target).unwrap(), "original");

        // Host file symlink escaping `--allow-read` traps FsReadDenied
        for esc_op in [
            format!(
                "fs.readFileSync({:?} + '/escape-link', 'utf8');",
                sandbox_norm
            ),
            format!("fs.openSync({:?} + '/escape-link', 'r');", sandbox_norm),
            format!(
                "fs.copyFileSync({:?} + '/escape-link', {:?} + '/stolen.txt');",
                sandbox_norm, sandbox_norm
            ),
        ] {
            let script = format!("import * as fs from 'node:fs';\n{}", esc_op);
            let err = eval_ts_with_caps("host-file-sym-escape", &script, rw_caps.clone())
                .expect_err("host file symlink escaping allow-read must trap FsReadDenied");
            assert!(matches!(err, SandboxViolation::FsReadDenied { .. }));
        }

        // Host file symlink pointing into read-only `/ro_zone` traps FsWriteDenied on mutation
        for wr_op in [
            format!("fs.writeFileSync({:?} + '/ro-link', 'pwn');", sandbox_norm),
            format!("fs.appendFileSync({:?} + '/ro-link', 'pwn');", sandbox_norm),
            format!("fs.truncateSync({:?} + '/ro-link', 0);", sandbox_norm),
            format!("fs.openSync({:?} + '/ro-link', 'w');", sandbox_norm),
        ] {
            let script = format!("import * as fs from 'node:fs';\n{}", wr_op);
            let err = eval_ts_with_caps("host-file-sym-ro-write", &script, rw_caps.clone())
                .expect_err(
                    "writing through host symlink into read-only zone must trap FsWriteDenied",
                );
            assert!(matches!(err, SandboxViolation::FsWriteDenied { .. }));
        }
    } else {
        #[cfg(unix)]
        panic!("creating Unix symlinks in temp_dir must never fail");
    }

    let _ = std::fs::remove_dir_all(&base_dir);
}

#[test]
fn test_issue_179_uniform_catchable_ocap_denials_with_audit_and_uncatchable_traps() {
    use crate::isolate::SandboxEvalOptions;

    let mut parent = SliceIsolate::spawn(
        "issue-179-parent",
        1,
        CapabilityToken::pure_compute(),
        50_000_000,
        32 * 1024 * 1024,
    );

    // 1. C1: Denied fetch() is catchable with ERR_SLICE_NET_DENIED + deniedCapabilities audit
    let c1_caught = parent.eval_sandbox(
        r#"
        let code = "";
        let caught = false;
        try {
            await fetch("https://example.com");
        } catch (e: any) {
            caught = true;
            code = String(e && e.code || "");
        }
        return JSON.stringify({ caught, code });
        "#,
        SandboxEvalOptions {
            fuel: Some(500_000),
            ..Default::default()
        },
    );
    assert!(
        c1_caught.ok,
        "caught denied fetch must succeed with ok=true, got: {:?}",
        c1_caught
    );
    let c1_v: serde_json::Value = serde_json::from_str(&c1_caught.return_value).unwrap();
    assert_eq!(c1_v["caught"], true);
    assert_eq!(c1_v["code"], "ERR_SLICE_NET_DENIED");
    assert!(
        c1_caught
            .denied_capabilities
            .iter()
            .any(|d| d.contains("lacks outbound network authority")),
        "expected NetAccessDenied in denied_capabilities, got: {:?}",
        c1_caught.denied_capabilities
    );

    // C1 uncaught promotes to SandboxViolation::NetAccessDenied
    let c1_uncaught = parent.eval_sandbox(
        r#"await fetch("https://example.com");"#,
        SandboxEvalOptions {
            fuel: Some(500_000),
            ..Default::default()
        },
    );
    assert!(!c1_uncaught.ok);
    assert!(
        c1_uncaught
            .violation
            .as_deref()
            .unwrap_or("")
            .contains("lacks outbound network authority"),
        "uncaught denied fetch must report NetAccessDenied violation, got: {:?}",
        c1_uncaught
    );

    // 2. C2: Denied fs.readFileSync and fs.promises.readFile are catchable with ERR_SLICE_FS_DENIED
    let c2_caught = parent.eval_sandbox(
        r#"
        import * as fs from "node:fs";
        let syncCode = "";
        let asyncCode = "";
        try { fs.readFileSync("/etc/passwd", "utf8"); } catch (e: any) { syncCode = String(e && e.code || ""); }
        try { await fs.promises.readFile("/etc/passwd", "utf8"); } catch (e: any) { asyncCode = String(e && e.code || ""); }
        return JSON.stringify({ syncCode, asyncCode });
        "#,
        SandboxEvalOptions {
            fuel: Some(500_000),
            ..Default::default()
        },
    );
    assert!(
        c2_caught.ok,
        "caught denied fs reads must succeed with ok=true, got: {:?}",
        c2_caught
    );
    let c2_v: serde_json::Value = serde_json::from_str(&c2_caught.return_value).unwrap();
    assert_eq!(c2_v["syncCode"], "ERR_SLICE_FS_DENIED");
    assert_eq!(c2_v["asyncCode"], "ERR_SLICE_FS_DENIED");
    assert!(
        c2_caught.denied_capabilities.len() >= 2,
        "expected both denied fs reads in audit log, got: {:?}",
        c2_caught.denied_capabilities
    );

    // 3. C3: Denied execSync is catchable with ERR_SLICE_EXEC_DENIED + deniedCapabilities audit
    let c3_caught = parent.eval_sandbox(
        r#"
        import { execSync } from "node:child_process";
        let code = "";
        try { execSync("whoami"); } catch (e: any) { code = String(e && e.code || ""); }
        return code;
        "#,
        SandboxEvalOptions {
            fuel: Some(500_000),
            ..Default::default()
        },
    );
    assert!(
        c3_caught.ok,
        "caught execSync must succeed: {:?}",
        c3_caught
    );
    assert_eq!(c3_caught.return_value, "ERR_SLICE_EXEC_DENIED");
    assert!(
        c3_caught
            .denied_capabilities
            .iter()
            .any(|d| d.contains("lacks exec authority")),
        "expected ExecDenied in denied_capabilities, got: {:?}",
        c3_caught.denied_capabilities
    );

    // 4. Denied env() and disabled Slice.checkpoint() are catchable with stable codes
    let c4_caught = parent.eval_sandbox(
        r#"
        let envCode = "";
        let cpCode = "";
        try { env("UNGRANTED_SECRET"); } catch (e: any) { envCode = String(e && e.code || ""); }
        try { Slice.checkpoint(); } catch (e: any) { cpCode = String(e && e.code || ""); }
        return JSON.stringify({ envCode, cpCode });
        "#,
        SandboxEvalOptions {
            fuel: Some(500_000),
            checkpoints: Some(false),
            ..Default::default()
        },
    );
    assert!(
        c4_caught.ok,
        "caught env/checkpoint must succeed: {:?}",
        c4_caught
    );
    let c4_v: serde_json::Value = serde_json::from_str(&c4_caught.return_value).unwrap();
    assert_eq!(c4_v["envCode"], "ERR_SLICE_ENV_DENIED");
    assert_eq!(c4_v["cpCode"], "ERR_SLICE_CHECKPOINT_DENIED");

    // 5. Resource / Integrity / SSRF traps remain strictly UNCATCHABLE (bypass guest catch)
    let fuel_trap = parent.eval_sandbox(
        r#"
        try { while (true) {} } catch (_e) { console.log("ESCAPED_FUEL_CATCH"); }
        "#,
        SandboxEvalOptions {
            fuel: Some(2_000),
            ..Default::default()
        },
    );
    assert!(!fuel_trap.ok);
    assert!(
        fuel_trap.stdout.is_empty(),
        "uncatchable fuel trap must NOT run guest catch block: {:?}",
        fuel_trap.stdout
    );
    assert!(
        fuel_trap
            .violation
            .as_deref()
            .unwrap_or("")
            .contains("fuel limit"),
        "expected CpuFuelExhausted violation, got: {:?}",
        fuel_trap
    );

    let ssrf_trap = parent.eval_sandbox(
        r#"
        try {
            await fetch("http://169.254.169.254/latest/meta-data/");
        } catch (_e) {
            console.log("ESCAPED_SSRF_CATCH");
        }
        "#,
        SandboxEvalOptions {
            fuel: Some(500_000),
            ..Default::default()
        },
    );
    assert!(
        !ssrf_trap.ok
            && ssrf_trap
                .violation
                .as_deref()
                .unwrap_or("")
                .contains("SSRF_METADATA_BLOCKED"),
        "SSRF Cloud Metadata Shield must remain an uncatchable fatal violation: {:?}",
        ssrf_trap
    );
}

#[test]
fn test_issue_180_builtin_fuel_memory_wallclock_and_interrupt_metering() {
    use crate::isolate::SandboxEvalOptions;

    let mut parent = SliceIsolate::spawn(
        "issue-180-parent",
        1,
        CapabilityToken::pure_compute(),
        100_000_000,
        64 * 1024 * 1024,
    );

    // 1. A1/A3: Built-in Array.from + Array.prototype.sort MUST charge fuel and trap under tight budget
    let sort_trap = parent.eval_sandbox(
        r#"
        const d = Array.from({ length: 100000 }, (_, i) => (i * 1103515245 + 12345) >>> 0);
        const s = [...d].sort((a, b) => a - b);
        return s.length;
        "#,
        SandboxEvalOptions {
            fuel: Some(5_000),
            ..Default::default()
        },
    );
    assert!(
        !sort_trap.ok
            && sort_trap
                .violation
                .as_deref()
                .unwrap_or("")
                .contains("fuel limit"),
        "Array.from + sort must trap CpuFuelExhausted under fuel=5000, got: {:?}",
        sort_trap
    );

    // Smaller built-in workload succeeds and reports proportional fuel (> 2000 for 2000 items)
    let sort_ok = parent.eval_sandbox(
        r#"
        const d = Array.from({ length: 2000 }, (_, i) => (i * 1103515245 + 12345) >>> 0);
        const s = [...d].sort((a, b) => a - b);
        return s.length;
        "#,
        SandboxEvalOptions {
            fuel: Some(5_000_000),
            ..Default::default()
        },
    );
    assert!(
        sort_ok.ok,
        "2000-element Array.from + sort should succeed: {:?}",
        sort_ok
    );
    assert_eq!(sort_ok.return_value, "2000");
    assert!(
        sort_ok.fuel_used > 2_000,
        "built-in Array.from + sort must charge proportional fuel, got: {}",
        sort_ok.fuel_used
    );

    // 2. B1/B2: Ordinary JS object/array/Map heap allocations MUST enforce memory ceiling and report real peakMemoryBytes
    let oom_trap = parent.eval_sandbox(
        r#"
        const a: any[] = [];
        for (let i = 0; i < 200000; i++) {
            a.push({ x: i, y: i * 2 });
        }
        return a.length;
        "#,
        SandboxEvalOptions {
            fuel: Some(50_000_000),
            memory_bytes: Some(512 * 1024),
            ..Default::default()
        },
    );
    assert!(
        !oom_trap.ok
            && oom_trap
                .violation
                .as_deref()
                .unwrap_or("")
                .contains("exceeded memory ceiling"),
        "200k object allocations must trap MemoryCeilingExceeded under 512 KB, got: {:?}",
        oom_trap
    );
    assert!(
        oom_trap.peak_memory_bytes > 50_000,
        "peakMemoryBytes must reflect metered JS heap allocations, got: {}",
        oom_trap.peak_memory_bytes
    );

    // Map/Set heap metering also enforces ceiling
    let map_oom = parent.eval_sandbox(
        r#"
        const m = new Map();
        for (let i = 0; i < 50000; i++) m.set(i, i);
        return m.size;
        "#,
        SandboxEvalOptions {
            fuel: Some(50_000_000),
            memory_bytes: Some(256 * 1024),
            ..Default::default()
        },
    );
    assert!(
        !map_oom.ok
            && map_oom
                .violation
                .as_deref()
                .unwrap_or("")
                .contains("exceeded memory ceiling"),
        "50k Map entries must trap MemoryCeilingExceeded under 256 KB, got: {:?}",
        map_oom
    );

    // 3. Wall-clock deadline (`timeoutMs`) aborts long-running compute with WallClockExceeded
    let wall_trap = parent.eval_sandbox(
        r#"
        let x = 1;
        for (let i = 0; i < 50_000_000; i++) {
            x = ((x * 33) ^ i) & 0x0fffffff;
        }
        return x;
        "#,
        SandboxEvalOptions {
            fuel: Some(100_000_000),
            timeout_ms: Some(25),
            ..Default::default()
        },
    );
    assert!(
        !wall_trap.ok
            && wall_trap
                .violation
                .as_deref()
                .unwrap_or("")
                .contains("WallClockExceeded"),
        "timeoutMs=25 must trap WallClockExceeded, got: {:?}",
        wall_trap
    );

    // 4. Cross-thread `IsolateInterruptHandle::interrupt()` aborts with ExecutionInterrupted
    let mut iso = SliceIsolate::spawn(
        "issue-180-interrupt",
        2,
        CapabilityToken::pure_compute(),
        100_000_000,
        16 * 1024 * 1024,
    );
    let handle = iso.interrupt_handle();
    let bc = compile_source_to_bytecode("let n = 0; while (true) { n++; }");
    let interrupter = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(20));
        handle.interrupt();
    });
    let int_err = iso
        .execute_bytecode(&bc, None)
        .expect_err("interrupt_handle().interrupt() must abort infinite loop");
    interrupter.join().unwrap();
    assert!(
        matches!(int_err, SandboxViolation::ExecutionInterrupted { .. }),
        "expected ExecutionInterrupted, got: {:?}",
        int_err
    );
}

#[test]
fn test_issue_183_regex_backtracking_fuel_wallclock_and_interrupt() {
    use crate::isolate::SandboxEvalOptions;

    let mut parent = SliceIsolate::spawn(
        "issue-183-parent",
        1,
        CapabilityToken::pure_compute(),
        100_000_000,
        32 * 1024 * 1024,
    );

    // 1. Fuel limit bounds catastrophic backtracking and cannot be caught by guest try/catch
    let t0 = std::time::Instant::now();
    let fuel_trap = parent.eval_sandbox(
        r#"
        try {
            /^(a+)+$/.test("a".repeat(28) + "b");
        } catch (_e) {
            console.log("ESCAPED_REGEX_FUEL");
        }
        "#,
        SandboxEvalOptions {
            fuel: Some(1_000),
            ..Default::default()
        },
    );
    assert!(
        t0.elapsed().as_millis() < 500,
        "regex fuel trap took too long: {:?}",
        t0.elapsed()
    );
    assert!(
        !fuel_trap.ok
            && fuel_trap.stdout.is_empty()
            && fuel_trap
                .violation
                .as_deref()
                .unwrap_or("")
                .contains("fuel limit"),
        "expected uncatchable CpuFuelExhausted on ReDoS, got: {:?}",
        fuel_trap
    );

    // 2. Wall-clock deadline (`timeoutMs`) bounds catastrophic backtracking even with huge fuel
    let t1 = std::time::Instant::now();
    let wall_trap = parent.eval_sandbox(
        r#"/^(a+)+$/.test("a".repeat(28) + "b");"#,
        SandboxEvalOptions {
            fuel: Some(50_000_000),
            timeout_ms: Some(80),
            ..Default::default()
        },
    );
    assert!(
        t1.elapsed().as_millis() < 1_000,
        "regex wall-clock trap took too long: {:?}",
        t1.elapsed()
    );
    assert!(
        !wall_trap.ok
            && wall_trap
                .violation
                .as_deref()
                .unwrap_or("")
                .contains("WallClockExceeded"),
        "expected WallClockExceeded on ReDoS, got: {:?}",
        wall_trap
    );

    // 3. String.prototype.replace with catastrophic backtracking also traps
    let replace_trap = parent.eval_sandbox(
        r#"("a".repeat(28) + "b").replace(/^(a+)+$/, "x");"#,
        SandboxEvalOptions {
            fuel: Some(1_000),
            ..Default::default()
        },
    );
    assert!(
        !replace_trap.ok
            && replace_trap
                .violation
                .as_deref()
                .unwrap_or("")
                .contains("fuel limit"),
        "expected CpuFuelExhausted on String.prototype.replace ReDoS, got: {:?}",
        replace_trap
    );

    // 4. Cross-thread IsolateInterruptHandle::interrupt() aborts catastrophic backtracking
    let mut iso = SliceIsolate::spawn(
        "issue-183-interrupt",
        2,
        CapabilityToken::pure_compute(),
        u64::MAX / 2,
        16 * 1024 * 1024,
    );
    iso.set_wall_clock_limit_ms(Some(2_000));
    let handle = iso.interrupt_handle();
    let bc = compile_source_to_bytecode(r#"/^(a+)+$/.test("a".repeat(28) + "b");"#);
    let t2 = std::time::Instant::now();
    let interrupter = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(40));
        handle.interrupt();
    });
    let int_err = iso
        .execute_bytecode(&bc, None)
        .expect_err("interrupt handle must abort catastrophic regex backtracking");
    interrupter.join().unwrap();
    assert!(
        t2.elapsed().as_millis() < 1_000,
        "regex interrupt took too long: {:?}",
        t2.elapsed()
    );
    assert!(
        matches!(int_err, SandboxViolation::ExecutionInterrupted { .. }),
        "expected ExecutionInterrupted, got: {:?}",
        int_err
    );

    // 5. Normal regexes continue to work cleanly
    let ok_res = parent.eval_sandbox(
        r#"
        const m = /^\d{4}-\d{2}-\d{2}$/.test("2026-11-25");
        const r = "hello world".replace(/(\w+)/g, "[$1]");
        return JSON.stringify({ m, r });
        "#,
        SandboxEvalOptions {
            fuel: Some(500_000),
            ..Default::default()
        },
    );
    assert!(ok_res.ok, "normal regexes must succeed: {:?}", ok_res);
    let parsed: serde_json::Value = serde_json::from_str(&ok_res.return_value).unwrap();
    assert_eq!(parsed["m"], true);
    assert_eq!(parsed["r"], "[hello] [world]");
}

#[test]
fn test_issue_184_arraybuffer_typedarray_buffer_metering() {
    use crate::isolate::SandboxEvalOptions;

    let mut parent = SliceIsolate::spawn(
        "issue-184-parent",
        1,
        CapabilityToken::pure_compute(),
        50_000_000,
        32 * 1024 * 1024,
    );

    // 1. ArrayBuffer / BigInt64Array / Buffer.alloc / ArrayBuffer.transfer enforce memoryBytes (uncatchable)
    for (label, code) in [
        (
            "ArrayBuffer",
            r#"try { new ArrayBuffer(2 * 1024 * 1024); } catch (_e) { console.log("ESCAPED"); }"#,
        ),
        ("BigInt64Array", r#"new BigInt64Array(200_000);"#),
        ("Buffer.alloc", r#"Buffer.alloc(2 * 1024 * 1024);"#),
        (
            "ArrayBuffer.transfer",
            r#"new ArrayBuffer(1).transfer(2 * 1024 * 1024);"#,
        ),
    ] {
        let out = parent.eval_sandbox(
            code,
            SandboxEvalOptions {
                fuel: Some(5_000_000),
                memory_bytes: Some(512 * 1024),
                ..Default::default()
            },
        );
        assert!(
            !out.ok
                && out.stdout.is_empty()
                && out
                    .violation
                    .as_deref()
                    .unwrap_or("")
                    .contains("exceeded memory ceiling"),
            "{} must trap uncatchable MemoryCeilingExceeded under 512 KB, got: {:?}",
            label,
            out
        );
    }

    // 2. TypedArray bulk ops (fill, sort) charge fuel and trap under tight budget
    let fill_trap = parent.eval_sandbox(
        r#"new Uint8Array(200_000).fill(7);"#,
        SandboxEvalOptions {
            fuel: Some(1_000),
            memory_bytes: Some(4 * 1024 * 1024),
            ..Default::default()
        },
    );
    assert!(
        !fill_trap.ok
            && fill_trap
                .violation
                .as_deref()
                .unwrap_or("")
                .contains("fuel limit"),
        "TypedArray.fill must trap CpuFuelExhausted, got: {:?}",
        fill_trap
    );

    let sort_trap = parent.eval_sandbox(
        r#"new Uint8Array(50_000).sort();"#,
        SandboxEvalOptions {
            fuel: Some(5_000),
            memory_bytes: Some(4 * 1024 * 1024),
            ..Default::default()
        },
    );
    assert!(
        !sort_trap.ok
            && sort_trap
                .violation
                .as_deref()
                .unwrap_or("")
                .contains("fuel limit"),
        "TypedArray.sort must trap CpuFuelExhausted, got: {:?}",
        sort_trap
    );

    // 3. Normal Buffer / TypedArray usage succeeds cleanly
    let ok_res = parent.eval_sandbox(
        r#"
        const b = Buffer.alloc(64, 0x41);
        const u = new Uint8Array([3, 1, 2]).sort();
        return JSON.stringify({ len: b.length, sorted: Array.from(u) });
        "#,
        SandboxEvalOptions {
            fuel: Some(500_000),
            memory_bytes: Some(2 * 1024 * 1024),
            ..Default::default()
        },
    );
    assert!(
        ok_res.ok,
        "normal Buffer/TypedArray must succeed: {:?}",
        ok_res
    );
    let parsed: serde_json::Value = serde_json::from_str(&ok_res.return_value).unwrap();
    assert_eq!(parsed["len"], 64);
    assert_eq!(parsed["sorted"], serde_json::json!([1, 2, 3]));
}

#[test]
fn test_issue_185_string_builtin_fuel_and_memory_metering() {
    use crate::isolate::SandboxEvalOptions;

    let mut parent = SliceIsolate::spawn(
        "issue-185-parent",
        1,
        CapabilityToken::pure_compute(),
        50_000_000,
        32 * 1024 * 1024,
    );

    // 1. String.repeat doubling, padStart, replaceAll amplification, and split("") enforce memoryBytes
    for (label, code) in [
        (
            "repeat doubling",
            r#"let s = "a"; for (let i = 0; i < 22; i++) s = s.repeat(2);"#,
        ),
        (
            "padStart",
            r#"try { "".padStart(2_000_000, "x"); } catch (_e) { console.log("ESCAPED"); }"#,
        ),
        (
            "replaceAll amplification",
            r#"("a".repeat(2_000)).replaceAll("", "x".repeat(500));"#,
        ),
        ("split empty sep", r#"("a".repeat(30_000)).split("");"#),
        (
            "concat doubling",
            r#"let s = "a"; for (let i = 0; i < 22; i++) s = s.concat(s);"#,
        ),
    ] {
        let out = parent.eval_sandbox(
            code,
            SandboxEvalOptions {
                fuel: Some(5_000_000),
                memory_bytes: Some(512 * 1024),
                ..Default::default()
            },
        );
        assert!(
            !out.ok
                && out.stdout.is_empty()
                && out
                    .violation
                    .as_deref()
                    .unwrap_or("")
                    .contains("exceeded memory ceiling"),
            "{} must trap uncatchable MemoryCeilingExceeded under 512 KB, got: {:?}",
            label,
            out
        );
    }

    // 2. Normal string operations succeed cleanly
    let ok_res = parent.eval_sandbox(
        r#"
        const a = "ab".repeat(3);
        const b = "7".padStart(4, "0");
        const c = "a-b-c".replaceAll("-", "_").split("_").join(",");
        return JSON.stringify({ a, b, c });
        "#,
        SandboxEvalOptions {
            fuel: Some(500_000),
            memory_bytes: Some(2 * 1024 * 1024),
            ..Default::default()
        },
    );
    assert!(ok_res.ok, "normal string ops must succeed: {:?}", ok_res);
    let parsed: serde_json::Value = serde_json::from_str(&ok_res.return_value).unwrap();
    assert_eq!(parsed["a"], "ababab");
    assert_eq!(parsed["b"], "0007");
    assert_eq!(parsed["c"], "a,b,c");
}

#[test]
fn test_issue_186_sparse_array_builtin_fuel_memory_and_recursion() {
    use crate::isolate::SandboxEvalOptions;

    let mut parent = SliceIsolate::spawn(
        "issue-186-parent",
        1,
        CapabilityToken::pure_compute(),
        50_000_000,
        32 * 1024 * 1024,
    );

    // 1. Sparse Array.join pre-allocation & loop must trap MemoryCeilingExceeded or CpuFuelExhausted
    let join_trap = parent.eval_sandbox(
        r#"try { new Array(5_000_000).join("x"); } catch (_e) { console.log("ESCAPED"); }"#,
        SandboxEvalOptions {
            fuel: Some(10_000),
            memory_bytes: Some(512 * 1024),
            ..Default::default()
        },
    );
    assert!(
        !join_trap.ok && join_trap.stdout.is_empty() && join_trap.violation.is_some(),
        "sparse Array.join must trap uncatchably, got: {:?}",
        join_trap
    );

    // 2. Sparse Array fill, reverse, indexOf, includes, and sparse forEach holes trap fuel
    for (label, code) in [
        ("fill", r#"new Array(5_000_000).fill(0);"#),
        (
            "reverse",
            r#"Array.prototype.reverse.call({ length: 5_000_000 });"#,
        ),
        (
            "indexOf",
            r#"Array.prototype.indexOf.call({ length: 5_000_000 }, 1);"#,
        ),
        (
            "includes",
            r#"Array.prototype.includes.call({ length: 5_000_000 }, 1);"#,
        ),
        (
            "sparse forEach",
            r#"new Array(5_000_000).forEach(() => {});"#,
        ),
        ("concat sparse", r#"[].concat(new Array(5_000_000));"#),
    ] {
        let out = parent.eval_sandbox(
            code,
            SandboxEvalOptions {
                fuel: Some(2_000),
                memory_bytes: Some(2 * 1024 * 1024),
                ..Default::default()
            },
        );
        assert!(
            !out.ok && out.violation.is_some(),
            "sparse Array.{} must trap fuel/memory, got: {:?}",
            label,
            out
        );
    }

    // 3. Self-referential Array.flat(Infinity) must not overflow the Rust stack
    let flat_res = parent.eval_sandbox(
        r#"
        const a: any[] = [];
        a.push(a);
        a.flat(Infinity);
        "#,
        SandboxEvalOptions {
            fuel: Some(50_000),
            memory_bytes: Some(2 * 1024 * 1024),
            ..Default::default()
        },
    );
    assert!(
        flat_res.ok || flat_res.violation.is_some() || flat_res.error.is_some(),
        "circular flat(Infinity) must complete safely without stack overflow"
    );

    // 4. Normal dense Array operations succeed cleanly
    let ok_res = parent.eval_sandbox(
        r#"
        const a = [1, 2, 3, 4].map(x => x * 2).filter(x => x > 2);
        return JSON.stringify({ joined: a.join("-"), rev: a.toReversed(), has6: a.includes(6) });
        "#,
        SandboxEvalOptions {
            fuel: Some(500_000),
            memory_bytes: Some(2 * 1024 * 1024),
            ..Default::default()
        },
    );
    assert!(
        ok_res.ok,
        "normal dense array ops must succeed: {:?}",
        ok_res
    );
    let parsed: serde_json::Value = serde_json::from_str(&ok_res.return_value).unwrap();
    assert_eq!(parsed["joined"], "4-6-8");
    assert_eq!(parsed["rev"], serde_json::json!([8, 6, 4]));
    assert_eq!(parsed["has6"], true);
}

#[test]
fn test_issue_187_bigint_json_object_and_codec_metering() {
    use crate::isolate::SandboxEvalOptions;

    let mut parent = SliceIsolate::spawn(
        "issue-187-parent",
        1,
        CapabilityToken::pure_compute(),
        50_000_000,
        32 * 1024 * 1024,
    );

    // 1. BigInt shift / pow / toString blowup must trap uncatchably under tight memory/fuel
    for (label, code) in [
        (
            "bigint shl",
            r#"try { const x = 1n << 10_000_000n; } catch (_e) { console.log("ESCAPED"); }"#,
        ),
        (
            "bigint pow",
            r#"try { const x = 3n ** 10_000_000n; } catch (_e) { console.log("ESCAPED"); }"#,
        ),
        (
            "bigint toString",
            r#"try { const x = (1n << 200_000n).toString(); } catch (_e) { console.log("ESCAPED"); }"#,
        ),
    ] {
        let res = parent.eval_sandbox(
            code,
            SandboxEvalOptions {
                fuel: Some(10_000),
                memory_bytes: Some(64 * 1024),
                ..Default::default()
            },
        );
        assert!(
            !res.ok && res.stdout.is_empty() && res.violation.is_some(),
            "{} must trap uncatchably, got: {:?}",
            label,
            res
        );
    }

    // 2. JSON.stringify on sparse arrays and non-circular exponential DAGs must trap
    for (label, code) in [
        (
            "json sparse array",
            r#"try { JSON.stringify(new Array(5_000_000)); } catch (_e) { console.log("ESCAPED"); }"#,
        ),
        (
            "json exponential DAG",
            r#"
            try {
                let a: any = [1];
                for (let i = 0; i < 24; i++) a = [a, a];
                JSON.stringify(a);
            } catch (_e) {
                console.log("ESCAPED");
            }
            "#,
        ),
    ] {
        let res = parent.eval_sandbox(
            code,
            SandboxEvalOptions {
                fuel: Some(50_000),
                memory_bytes: Some(256 * 1024),
                ..Default::default()
            },
        );
        assert!(
            !res.ok && res.stdout.is_empty() && res.violation.is_some(),
            "{} must trap uncatchably, got: {:?}",
            label,
            res
        );
    }

    // 3. Object.entries / Object.keys on large typed array must trap
    let obj_trap = parent.eval_sandbox(
        r#"
        const u = new Uint8Array(200_000);
        Object.entries(u);
        "#,
        SandboxEvalOptions {
            fuel: Some(50_000),
            memory_bytes: Some(512 * 1024),
            ..Default::default()
        },
    );
    assert!(
        !obj_trap.ok && obj_trap.violation.is_some(),
        "Object.entries on large Uint8Array must trap memory/fuel, got: {:?}",
        obj_trap
    );

    // 4. Native codecs (btoa / TextEncoder / TextDecoder) must charge heap + fuel on large payloads
    for (label, code) in [
        ("btoa", r#"const s = "a".repeat(200_000); btoa(s);"#),
        (
            "TextEncoder",
            r#"const s = "a".repeat(200_000); new TextEncoder().encode(s);"#,
        ),
        (
            "TextDecoder",
            r#"const u = new Uint8Array(200_000); new TextDecoder().decode(u);"#,
        ),
    ] {
        let res = parent.eval_sandbox(
            code,
            SandboxEvalOptions {
                fuel: Some(500_000),
                memory_bytes: Some(256 * 1024),
                ..Default::default()
            },
        );
        assert!(
            !res.ok && res.violation.is_some(),
            "{} must trap memory ceiling, got: {:?}",
            label,
            res
        );
    }

    // 5. Normal BigInt, JSON, Object, and codec operations succeed cleanly
    let ok_res = parent.eval_sandbox(
        r#"
        const big = (2n ** 64n).toString();
        const j = JSON.parse(JSON.stringify({ a: 1, b: [2, 3] }));
        const entries = Object.entries({ x: 10, y: 20 }).length;
        const b64 = atob(btoa("hello-slice"));
        const rt = new TextDecoder().decode(new TextEncoder().encode("utf8-ok"));
        return JSON.stringify({ big, sum: j.a + j.b[0] + j.b[1], entries, b64, rt });
        "#,
        SandboxEvalOptions {
            fuel: Some(500_000),
            memory_bytes: Some(2 * 1024 * 1024),
            ..Default::default()
        },
    );
    assert!(
        ok_res.ok,
        "normal BigInt/JSON/Object/codec ops must succeed: {:?}",
        ok_res
    );
    let parsed: serde_json::Value = serde_json::from_str(&ok_res.return_value).unwrap();
    assert_eq!(parsed["big"], "18446744073709551616");
    assert_eq!(parsed["sum"], 6);
    assert_eq!(parsed["entries"], 2);
    assert_eq!(parsed["b64"], "hello-slice");
    assert_eq!(parsed["rt"], "utf8-ok");
}
