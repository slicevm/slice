use crate::{
    compile_source_to_bytecode, set_thread_tier_override, ExecutionTier, HttpRequestContext,
    SandboxEvalOptions, SliceIsolate,
};
use slice_sandbox::{CapabilityToken, SandboxViolation};
use std::time::Instant;

#[test]
fn test_iso_sec01_wait_until_fuel_memory_dlp_and_ses_lockdown() {
    // 1. CPU Fuel Exhaustion inside Slice.waitUntil(fn) must trap with CpuFuelExhausted
    let script_fuel = r#"
        Slice.respond(200, "ok", "text/plain");
        Slice.waitUntil(() => {
            while (true) {}
        });
    "#;
    let bc_fuel = compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(
        script_fuel,
        true,
    ));
    let mut iso_fuel = SliceIsolate::spawn(
        "iso-sec01-fuel",
        1,
        CapabilityToken::pure_compute(),
        5_000,
        8 * 1024 * 1024,
    );
    let req = HttpRequestContext {
        method: "GET",
        path: "/test",
        query: "",
        tenant_id: "iso-sec01-fuel",
        body: "",
        headers: &[],
    };
    let outcome = iso_fuel
        .execute_http_request(&bc_fuel, None, &req)
        .expect("initial HTTP response should succeed before waitUntil drain");
    assert_eq!(outcome.wait_until_count, 1);
    let err_fuel = iso_fuel
        .drain_wait_until_tasks()
        .expect_err("infinite loop inside Slice.waitUntil must trap with CpuFuelExhausted");
    assert!(
        matches!(err_fuel, SandboxViolation::CpuFuelExhausted { .. }),
        "expected CpuFuelExhausted, got: {:?}",
        err_fuel
    );

    // 2. Memory Ceiling Exceeded inside Slice.waitUntil(fn) via Slice.kv.set must trap
    let script_mem = r#"
        Slice.respond(200, "ok", "text/plain");
        Slice.waitUntil(() => {
            Slice.kv.set("huge_key", "X".repeat(32768));
        });
    "#;
    let bc_mem =
        compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(script_mem, true));
    let mut iso_mem = SliceIsolate::spawn(
        "iso-sec01-mem",
        1,
        CapabilityToken::pure_compute(),
        500_000,
        16 * 1024, // 16 KB total ceiling
    );
    let _ = iso_mem
        .execute_http_request(&bc_mem, None, &req)
        .expect("initial HTTP response should succeed");
    let err_mem = iso_mem.drain_wait_until_tasks().expect_err(
        "oversized KV write inside Slice.waitUntil must trap with MemoryCeilingExceeded",
    );
    assert!(
        matches!(err_mem, SandboxViolation::MemoryCeilingExceeded { .. }),
        "expected MemoryCeilingExceeded, got: {:?}",
        err_mem
    );

    // 3. Egress DLP secret leak to stdout inside Slice.waitUntil(fn) must be blocked
    let script_dlp = r#"
        Slice.respond(200, "safe response", "text/plain");
        Slice.waitUntil(() => {
            console.log("leaked: sk-live-waituntil-999999");
        });
    "#;
    let bc_dlp =
        compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(script_dlp, true));
    let mut iso_dlp = SliceIsolate::spawn(
        "iso-sec01-dlp",
        1,
        CapabilityToken::pure_compute(),
        500_000,
        8 * 1024 * 1024,
    );
    iso_dlp.add_egress_audit_secret("sk-live-waituntil-999999");
    let _ = iso_dlp
        .execute_http_request(&bc_dlp, None, &req)
        .expect("initial safe HTTP response should succeed");
    let err_dlp = iso_dlp
        .drain_wait_until_tasks()
        .expect_err("secret logged inside Slice.waitUntil must be blocked by Egress DLP");
    assert!(
        matches!(
            &err_dlp,
            SandboxViolation::EnvAccessDenied { key, .. } if key == "EGRESS_SECRET_LEAK_BLOCKED"
        ),
        "expected EGRESS_SECRET_LEAK_BLOCKED, got: {:?}",
        err_dlp
    );
}

#[test]
fn test_iso_sec02_websocket_event_fs_ocap_fuel_and_dlp_enforcement() {
    // 1. Server WebSocket handler attempting unauthorized fs.readFileSync must trap with FsReadDenied
    let setup_ws_fs = r#"
        import fs from "node:fs";
        Slice.onWebSocket({
            message(ws, data) {
                const secret = fs.readFileSync("/etc/passwd", "utf8");
                ws.send(secret);
            }
        });
    "#;
    let bc_fs = compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(
        setup_ws_fs,
        true,
    ));
    let caps_fs = CapabilityToken::pure_compute().with_read_prefix("/public");
    let mut iso_fs = SliceIsolate::spawn("iso-sec02-ws-fs", 1, caps_fs, 500_000, 8 * 1024 * 1024);
    iso_fs
        .execute_bytecode(&bc_fs, None)
        .expect("registering Slice.onWebSocket should succeed");
    let err_fs = iso_fs
        .execute_websocket_event("message", false, b"read_secret", 1000, "")
        .expect_err("unauthorized fs.readFileSync inside WebSocket handler must fail OCap");
    assert!(
        matches!(&err_fs, SandboxViolation::FsReadDenied { path, .. } if path == "/etc/passwd"),
        "expected FsReadDenied(/etc/passwd), got: {:?}",
        err_fs
    );

    // 2. Server WebSocket handler with infinite loop must trap with CpuFuelExhausted
    let setup_ws_fuel = r#"
        Slice.onWebSocket({
            message(ws, data) {
                while (true) {}
            }
        });
    "#;
    let bc_fuel = compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(
        setup_ws_fuel,
        true,
    ));
    let mut iso_fuel = SliceIsolate::spawn(
        "iso-sec02-ws-fuel",
        1,
        CapabilityToken::pure_compute(),
        5_000,
        8 * 1024 * 1024,
    );
    iso_fuel
        .execute_bytecode(&bc_fuel, None)
        .expect("registering Slice.onWebSocket should succeed");
    let err_fuel = iso_fuel
        .execute_websocket_event("message", false, b"spin", 1000, "")
        .expect_err("infinite loop inside WebSocket event handler must trap with CpuFuelExhausted");
    assert!(
        matches!(err_fuel, SandboxViolation::CpuFuelExhausted { .. }),
        "expected CpuFuelExhausted, got: {:?}",
        err_fuel
    );
}

#[test]
fn test_iso_sec03_asymmetric_fs_read_only_and_write_only_ocap_enforcement() {
    // 1. Read-only compartment (allowed_read_prefixes = ["/tmp"], allowed_write_prefixes = [])
    //    must block fs.writeFileSync("/tmp/readonly_test.txt", ...).
    let read_only_caps = CapabilityToken::pure_compute().with_read_prefix("/tmp");
    let write_script = r#"
        import fs from "node:fs";
        fs.writeFileSync("/tmp/readonly_violation.txt", "should_be_blocked");
    "#;
    let bc_write = compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(
        write_script,
        true,
    ));
    let mut iso_ro =
        SliceIsolate::spawn("iso-sec03-ro", 1, read_only_caps, 500_000, 8 * 1024 * 1024);
    let err_ro = iso_ro
        .execute_bytecode(&bc_write, None)
        .expect_err("read-only compartment must reject fs.writeFileSync");
    assert!(
        matches!(&err_ro, SandboxViolation::FsWriteDenied { path, .. } if path == "/tmp/readonly_violation.txt"),
        "expected FsWriteDenied, got: {:?}",
        err_ro
    );

    // 2. Write-only compartment (allowed_write_prefixes = ["/tmp"], allowed_read_prefixes = [])
    //    must block fs.readFileSync("/tmp/writeonly_test.txt").
    let write_only_caps = CapabilityToken::pure_compute().with_write_prefix("/tmp");
    let read_script = r#"
        import fs from "node:fs";
        fs.readFileSync("/tmp/writeonly_violation.txt", "utf8");
    "#;
    let bc_read = compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(
        read_script,
        true,
    ));
    let mut iso_wo =
        SliceIsolate::spawn("iso-sec03-wo", 1, write_only_caps, 500_000, 8 * 1024 * 1024);
    let err_wo = iso_wo
        .execute_bytecode(&bc_read, None)
        .expect_err("write-only compartment must reject fs.readFileSync");
    assert!(
        matches!(&err_wo, SandboxViolation::FsReadDenied { path, .. } if path == "/tmp/writeonly_violation.txt"),
        "expected FsReadDenied, got: {:?}",
        err_wo
    );
}

#[test]
fn test_iso_sec04_eval_sandbox_parent_fuel_memory_and_env_attenuation() {
    // 1. Parent with a tight 16 KB memory limit spawning a child via JS `Slice.evalSandbox`
    //    must not allow the child to allocate 32 KB (fixing the old `.max(65536)` floor).
    let script = r#"
        const res = Slice.evalSandbox(`
            Slice.kv.set("child_k", "Z".repeat(24576));
        `, { memoryBytes: 1048576 });
        console.log(JSON.stringify({ ok: res.ok, violation: res.violation }));
    "#;
    let bc = compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(script, true));
    let mut iso = SliceIsolate::spawn(
        "iso-sec04-mem",
        1,
        CapabilityToken::pure_compute(),
        500_000,
        16 * 1024, // 16 KB parent limit (12 KB remaining after 4 KB base frame)
    );
    let m = iso
        .execute_bytecode(&bc, None)
        .expect("parent script should complete and report child violation");
    let parsed: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(
        parsed["ok"], false,
        "child sandbox exceeded parent's 16 KB memory limit: {}",
        m.stdout[0]
    );

    // 2. Direct Rust `eval_sandbox` env attenuation (`allow_env`) and parent memory accounting
    let parent_caps = CapabilityToken::pure_compute()
        .with_env("PUBLIC_STAGE", "production")
        .with_env("DB_ROOT_PASSWORD", "top_secret_pw");
    let mut parent_iso = SliceIsolate::spawn("iso-sec04-env", 1, parent_caps, 500_000, 64 * 1024);
    let mem_before = parent_iso.compartment.memory_used_bytes;

    // Grant only PUBLIC_STAGE to child sandbox; DB_ROOT_PASSWORD and NON_EXISTENT must be denied
    let outcome_allowed = parent_iso.eval_sandbox(
        r#"
        const s = env("PUBLIC_STAGE");
        Slice.kv.set("stage_copy", "A".repeat(4096));
        console.log(s);
        "#,
        SandboxEvalOptions {
            allow_env: vec!["PUBLIC_STAGE".to_string(), "NON_EXISTENT".to_string()],
            ..Default::default()
        },
    );
    assert!(
        outcome_allowed.ok,
        "allowed env read failed: {:?}",
        outcome_allowed.violation
    );
    assert_eq!(outcome_allowed.stdout, vec!["production"]);
    assert!(
        parent_iso.compartment.memory_used_bytes >= mem_before + 4096,
        "child sandbox net memory growth must be charged to parent compartment"
    );

    let outcome_denied = parent_iso.eval_sandbox(
        r#"console.log(env("DB_ROOT_PASSWORD"));"#,
        SandboxEvalOptions {
            allow_env: vec!["PUBLIC_STAGE".to_string()],
            ..Default::default()
        },
    );
    assert!(
        !outcome_denied.ok,
        "ungranted parent env key DB_ROOT_PASSWORD must be denied in child sandbox"
    );
}

#[test]
fn test_iso_sec05_tier0_ast_sparse_array_memory_bomb_and_recursion_guard() {
    set_thread_tier_override(Some(0));

    // 1. Sparse array index assignment `a[100000000] = 1` in Tier-0 AST VM must trap
    //    on `alloc_memory` before resizing the underlying Vec.
    let sparse_script = r#"
        let a = [1, 2];
        a[100000000] = 99;
        console.log(a.length);
    "#;
    let bc_sparse = compile_source_to_bytecode(sparse_script);
    assert_eq!(
        &bc_sparse[0..8],
        crate::SNAPSHOT_MAGIC,
        "must compile to Tier-0 SLICESNP"
    );
    let mut iso_sparse = SliceIsolate::spawn(
        "iso-sec05-sparse",
        1,
        CapabilityToken::pure_compute(),
        500_000,
        64 * 1024, // 64 KB ceiling
    );
    let err_sparse = iso_sparse
        .execute_bytecode(&bc_sparse, None)
        .expect_err("sparse array bomb in Tier-0 AST VM must trap with MemoryCeilingExceeded");
    assert!(
        matches!(err_sparse, SandboxViolation::MemoryCeilingExceeded { .. }),
        "expected MemoryCeilingExceeded, got: {:?}",
        err_sparse
    );

    // 2. Unbounded string concatenation in a Tier-0 loop must trap on `alloc_memory`
    let str_bomb_script = r#"
        let s = "A1234567890123456789012345678901";
        let i = 0;
        while (i < 20) {
            s = s + s;
            i = i + 1;
        }
        console.log(s.length);
    "#;
    let bc_str = compile_source_to_bytecode(str_bomb_script);
    let mut iso_str = SliceIsolate::spawn(
        "iso-sec05-str",
        1,
        CapabilityToken::pure_compute(),
        500_000,
        32 * 1024, // 32 KB ceiling
    );
    let err_str = iso_str.execute_bytecode(&bc_str, None).expect_err(
        "exponential string doubling in Tier-0 AST VM must trap with MemoryCeilingExceeded",
    );
    assert!(
        matches!(err_str, SandboxViolation::MemoryCeilingExceeded { .. }),
        "expected MemoryCeilingExceeded, got: {:?}",
        err_str
    );

    // 3. Deep non-numeric recursion in Tier-0 AST VM must trap at MAX_TIER0_RECURSION_DEPTH (512)
    //    instead of overflowing the native thread stack, even when given 50,000,000 fuel.
    let rec_script = r#"
        function recurseStr(s) {
            return recurseStr(s);
        }
        console.log(recurseStr("x"));
    "#;
    let bc_rec = compile_source_to_bytecode(rec_script);
    let mut iso_rec = SliceIsolate::spawn(
        "iso-sec05-rec",
        1,
        CapabilityToken::pure_compute(),
        50_000_000,
        8 * 1024 * 1024,
    );
    let err_rec = iso_rec
        .execute_bytecode(&bc_rec, None)
        .expect_err("deep Tier-0 AST recursion must trap at MAX_TIER0_RECURSION_DEPTH");
    assert!(
        matches!(err_rec, SandboxViolation::CpuFuelExhausted { .. }),
        "expected CpuFuelExhausted on recursion limit, got: {:?}",
        err_rec
    );

    set_thread_tier_override(None);
}

#[test]
fn test_iso_perf01_02_03_reset_for_request_http_dlp_and_warm_isolate_throughput() {
    // 1. Benchmark `reset_for_request` in-place reuse
    let mut iso = SliceIsolate::spawn(
        "perf-tenant-a",
        1,
        CapabilityToken::pure_compute(),
        1_000_000,
        8 * 1024 * 1024,
    );
    iso.add_egress_audit_secret("sk-live-perf-secret-token-12345");

    let start_reset = Instant::now();
    for i in 0..10_000u32 {
        iso.reset_for_request("perf-tenant-a", (i & 7) + 1, 1_000_000, 8 * 1024 * 1024);
    }
    let reset_ns_per_op = start_reset.elapsed().as_nanos() as f64 / 10_000.0;
    assert!(
        reset_ns_per_op < 5_000.0,
        "reset_for_request should be sub-microsecond, got {:.2} ns/op",
        reset_ns_per_op
    );

    // 2. Benchmark warm HTTP request execution with active Egress DLP guard (zero-clone BOA_HTTP_RES check)
    let http_script = r#"
        export default {
            fetch(req) {
                return new Response(JSON.stringify({ ok: true, path: req.url }), {
                    status: 200,
                    headers: { "content-type": "application/json", "x-slice-edge": "warm" }
                });
            }
        };
    "#;
    let bundled = slice_cas::strip_typescript_and_normalize(http_script, true);
    let bc = compile_source_to_bytecode(&bundled);
    SliceIsolate::prewarm_boa_bundle(&bc);

    let req = HttpRequestContext {
        method: "GET",
        path: "/api/v1/health",
        query: "",
        tenant_id: "perf-tenant-a",
        body: "",
        headers: &[("accept", "application/json")],
    };

    let iters = 200;
    let start_http = Instant::now();
    for i in 0..iters {
        iso.reset_for_request("perf-tenant-a", (i & 7) + 1, 1_000_000, 8 * 1024 * 1024);
        let out = iso
            .execute_http_request(&bc, None, &req)
            .expect("warm HTTP request with DLP should succeed");
        assert_eq!(out.status_code, 200);
        assert_eq!(
            out.metrics.highest_tier_reached,
            ExecutionTier::Tier1Baseline
        );
    }
    let http_us_per_op = (start_http.elapsed().as_nanos() as f64 / 1_000.0) / (iters as f64);
    println!(
        "[ISO-PERF] reset_for_request: {:.1} ns/op | warm DLP HTTP request: {:.2} us/op",
        reset_ns_per_op, http_us_per_op
    );
}
