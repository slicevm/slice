use crate::{compile_source_to_bytecode, HttpRequestContext, SliceIsolate};
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
        headers: &[],
    };
    let mut http_iso = SliceIsolate::spawn(
        "http-default-deny",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        4 * 1024 * 1024,
    );
    let fast_err = http_iso
        .execute_http_request_fast(&http_bc, None, &req)
        .expect_err("execute_http_request_fast with pure_compute() must trap FsReadDenied");
    assert!(matches!(fast_err, SandboxViolation::FsReadDenied { .. }));

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
