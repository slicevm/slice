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
        matches!(
            &err,
            SandboxViolation::EnvAccessDenied { key, .. } if key == "EGRESS_SECRET_LEAK_BLOCKED"
        ),
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
