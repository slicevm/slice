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
        matches!(&err_dlp, SandboxViolation::EgressSecretLeakBlocked { .. }),
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

#[test]
fn test_multi_tenant_warm_realm_wait_until_and_ses_quarantine_isolation() {
    // 1. Cross-tenant WarmBoaRealm affinity and waitUntil closure scrub on aborted turns
    let script_a_abort = r#"
        globalThis.__tenant_marker = "tenant-alpha-secret";
        Slice.waitUntil(() => {
            Slice.kv.set("hijacked_by_alpha", "leaked");
        });
        throw new Error("abort before HTTP response");
    "#;
    let bc_a_abort = compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(
        script_a_abort,
        true,
    ));
    let mut iso_a = SliceIsolate::spawn(
        "mt-alpha",
        1,
        CapabilityToken::pure_compute(),
        500_000,
        8 * 1024 * 1024,
    );
    let req_a = HttpRequestContext {
        method: "GET",
        path: "/abort",
        query: "",
        tenant_id: "mt-alpha",
        body: "",
        headers: &[],
    };
    let _ = iso_a.execute_http_request(&bc_a_abort, None, &req_a);

    let script_b = r#"
        const globalLeak = typeof globalThis.__tenant_marker;
        Slice.kv.set("beta_secret", "top-secret-beta");
        Slice.waitUntil(() => {
            Slice.kv.set("beta_bg", "clean");
        });
        Slice.respond(200, globalLeak, "text/plain");
    "#;
    let bc_b =
        compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(script_b, true));
    let mut iso_b = SliceIsolate::spawn(
        "mt-beta",
        2,
        CapabilityToken::pure_compute(),
        500_000,
        8 * 1024 * 1024,
    );
    let req_b = HttpRequestContext {
        method: "GET",
        path: "/check",
        query: "",
        tenant_id: "mt-beta",
        body: "",
        headers: &[],
    };
    let out_b = iso_b
        .execute_http_request(&bc_b, None, &req_b)
        .expect("tenant beta HTTP request should succeed");
    assert_eq!(out_b.response_body, "undefined");
    assert_eq!(out_b.wait_until_count, 1);
    iso_b
        .drain_wait_until_tasks()
        .expect("tenant beta waitUntil should drain cleanly");
    assert_eq!(iso_b.kv_get("beta_bg").as_deref(), Some("clean"));
    assert_eq!(
        iso_b.kv_get("hijacked_by_alpha"),
        None,
        "aborted waitUntil closure from another tenant must never execute in tenant beta"
    );

    // Same-tenant subsequent request must also not drain stale aborted waitUntil closures
    let script_a_next = r#"
        Slice.waitUntil(() => {
            Slice.kv.set("alpha_legit_bg", "ok");
        });
        Slice.respond(200, "ok", "text/plain");
    "#;
    let bc_a_next = compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(
        script_a_next,
        true,
    ));
    iso_a.reset_for_request("mt-alpha", 1, 500_000, 8 * 1024 * 1024);
    let out_a_next = iso_a
        .execute_http_request(&bc_a_next, None, &req_a)
        .expect("subsequent tenant alpha request should succeed");
    assert_eq!(out_a_next.wait_until_count, 1);
    iso_a
        .drain_wait_until_tasks()
        .expect("legitimate waitUntil should succeed");
    assert_eq!(iso_a.kv_get("alpha_legit_bg").as_deref(), Some("ok"));
    assert_eq!(
        iso_a.kv_get("hijacked_by_alpha"),
        None,
        "aborted waitUntil closure must be cleared when turn aborts"
    );

    // 2. Dynamic bracket prototype mutation + non-enumerable defineProperty + fuel exhaustion
    //    plus attempted overwrite of globalThis.__slice_check_ses must quarantine the realm.
    let script_ses_fuel = r#"
        try { globalThis.__slice_check_ses = () => ""; } catch (_e) {}
        const p = "proto" + "type";
        const c = "const" + "ructor";
        ({})[c][p].injectedAdmin = true;
        Object.defineProperty(Array[p], "hiddenSlot", {
            value: 42,
            enumerable: false,
            configurable: true
        });
        while (true) {}
    "#;
    let bc_ses_fuel = compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(
        script_ses_fuel,
        true,
    ));
    let mut iso_poison = SliceIsolate::spawn(
        "mt-poison",
        3,
        CapabilityToken::pure_compute(),
        5_000,
        8 * 1024 * 1024,
    );
    let poison_err = iso_poison
        .execute_bytecode(&bc_ses_fuel, None)
        .expect_err("dynamic prototype mutation + infinite loop must trap");
    assert!(
        matches!(
            poison_err,
            SandboxViolation::CpuFuelExhausted { .. }
                | SandboxViolation::IntrinsicMutationBlocked { .. }
        ),
        "unexpected violation: {:?}",
        poison_err
    );

    // Verify both same-tenant retry and another tenant observe completely unpolluted prototypes
    let verify_clean_js = r#"
        const cleanObj = !("injectedAdmin" in ({}));
        const cleanArr = Object.getOwnPropertyDescriptor(Array.prototype, "hiddenSlot") === undefined;
        const sesLocked = Object.getOwnPropertyDescriptor(globalThis, "__slice_check_ses")?.writable === false;
        `${cleanObj}:${cleanArr}:${sesLocked}`
    "#;
    let bc_verify = compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(
        verify_clean_js,
        true,
    ));
    iso_poison.reset_for_request("mt-poison", 3, 500_000, 8 * 1024 * 1024);
    let m_same = iso_poison
        .execute_bytecode(&bc_verify, None)
        .expect("quarantined realm must be replaced with a clean realm for same tenant");
    assert_eq!(m_same.return_value, "true:true:true");

    let mut iso_other = SliceIsolate::spawn(
        "mt-clean-observer",
        4,
        CapabilityToken::pure_compute(),
        500_000,
        8 * 1024 * 1024,
    );
    let m_other = iso_other
        .execute_bytecode(&bc_verify, None)
        .expect("other tenant must observe clean prototypes");
    assert_eq!(m_other.return_value, "true:true:true");

    // Also verify dynamic bracket prototype mutation without fuel exhaustion traps directly
    let script_dyn_only = r#"
        const p = "proto" + "type";
        const c = "const" + "ructor";
        ({})[c][p].injectedNoFuelTrap = 99;
    "#;
    let bc_dyn_only = compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(
        script_dyn_only,
        true,
    ));
    let dyn_err = iso_other
        .execute_bytecode(&bc_dyn_only, None)
        .expect_err("dynamic bracket prototype mutation must trap with IntrinsicMutationBlocked");
    assert!(
        matches!(dyn_err, SandboxViolation::IntrinsicMutationBlocked { .. }),
        "expected IntrinsicMutationBlocked, got: {:?}",
        dyn_err
    );
}

#[test]
fn test_multi_tenant_vfs_affinity_checkpoint_mtime_and_nested_sandbox_isolation() {
    let fs_caps = CapabilityToken::pure_compute()
        .with_read_prefix("/tmp")
        .with_write_prefix("/tmp");

    // 1. Checkpoint & restore preserves custom file mtimeMs, and nested evalSandbox enforces
    //    explicit empty allowWrite: [], isolated sub-tenants, and violation rollback.
    let script_tenant_one = r#"
        import * as fs from "node:fs";

        fs.writeFileSync("/tmp/mt_doc.txt", "v1");
        fs.utimesSync("/tmp/mt_doc.txt", 1710000000, 1710000000);
        const mtimeBefore = Math.round(fs.statSync("/tmp/mt_doc.txt").mtimeMs);

        const snap = Slice.checkpoint();
        fs.writeFileSync("/tmp/mt_doc.txt", "v2-mutated");
        fs.utimesSync("/tmp/mt_doc.txt", 1790000000, 1790000000);
        const mtimeMutated = Math.round(fs.statSync("/tmp/mt_doc.txt").mtimeMs);

        Slice.restore(snap);
        const restoredBody = fs.readFileSync("/tmp/mt_doc.txt", "utf8");
        const mtimeRestored = Math.round(fs.statSync("/tmp/mt_doc.txt").mtimeMs);

        // Explicit empty allowWrite: [] must deny VFS writes even though parent has /tmp write
        const deniedWrite = Slice.evalSandbox(
            `const fs = require("node:fs"); fs.writeFileSync("/tmp/mt_doc.txt", "overwritten");`,
            { allowWrite: [] }
        );

        // Isolated sub-tenant must get an isolated VFS without seeing or clobbering parent VFS
        const subTenant = Slice.evalSandbox(
            `const fs = require("node:fs");
             const sawParent = fs.existsSync("/tmp/mt_doc.txt");
             fs.writeFileSync("/tmp/mt_doc.txt", "plugin-local");
             sawParent + ":" + fs.readFileSync("/tmp/mt_doc.txt", "utf8")`,
            { tenantId: "isolated-plugin" }
        );

        // Same-tenant child that writes and then hits a violation must roll back partial VFS writes
        const violatingChild = Slice.evalSandbox(
            `const fs = require("node:fs");
             fs.writeFileSync("/tmp/mt_doc.txt", "partial-before-fuel-trap");
             while (true) {}`,
            { fuel: 2500 }
        );

        const finalBody = fs.readFileSync("/tmp/mt_doc.txt", "utf8");
        JSON.stringify({
            mtimeBefore,
            mtimeMutated,
            mtimeRestored,
            restoredBody,
            deniedOk: deniedWrite.ok,
            hasWriteViolation: String(deniedWrite.violation || "").includes("lacks filesystem write authority"),
            subTenantVal: subTenant.returnValue,
            violatingOk: violatingChild.ok,
            finalBody
        });
    "#;
    let bc_one = compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(
        script_tenant_one,
        true,
    ));
    let mut iso_one = SliceIsolate::spawn(
        "vfs-tenant-one",
        1,
        fs_caps.clone(),
        2_000_000,
        16 * 1024 * 1024,
    );
    let m_one = iso_one
        .execute_bytecode(&bc_one, None)
        .expect("vfs-tenant-one script should succeed");
    let parsed: serde_json::Value =
        serde_json::from_str(&m_one.return_value).expect("valid JSON summary");
    assert_eq!(parsed["restoredBody"], "v1");
    assert_eq!(parsed["mtimeBefore"], parsed["mtimeRestored"]);
    assert_ne!(parsed["mtimeBefore"], parsed["mtimeMutated"]);
    assert_eq!(parsed["deniedOk"], false);
    assert_eq!(parsed["hasWriteViolation"], true);
    assert_eq!(parsed["subTenantVal"], "false:plugin-local");
    assert_eq!(parsed["violatingOk"], false);
    assert_eq!(parsed["finalBody"], "v1");

    // 2. Different tenant executing on the same OS thread must not see vfs-tenant-one's VFS files
    let script_tenant_two = r#"
        import * as fs from "node:fs";
        fs.existsSync("/tmp/mt_doc.txt")
    "#;
    let bc_two = compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(
        script_tenant_two,
        true,
    ));
    let mut iso_two = SliceIsolate::spawn("vfs-tenant-two", 2, fs_caps, 500_000, 8 * 1024 * 1024);
    let m_two = iso_two
        .execute_bytecode(&bc_two, None)
        .expect("vfs-tenant-two script should succeed");
    assert_eq!(
        m_two.return_value, "false",
        "VFS files and mtimes from tenant one must not leak to tenant two on the same thread"
    );

    // 3. Microbenchmark warm single-tenant HTTP keep-alive fast path
    let fast_http_src = r#"Slice.respond(200, "pong", "text/plain");"#;
    let bc_fast = compile_source_to_bytecode(fast_http_src);
    let mut warm_iso = SliceIsolate::spawn(
        "bench-keepalive-tenant",
        1,
        CapabilityToken::pure_compute(),
        500_000,
        8 * 1024 * 1024,
    );
    let req = HttpRequestContext {
        method: "GET",
        path: "/ping",
        query: "",
        tenant_id: "bench-keepalive-tenant",
        body: "",
        headers: &[],
    };
    let _ = warm_iso
        .execute_http_request_fast(&bc_fast, None, &req)
        .expect("warmup");
    let bench_iters = 500u32;
    let t0 = Instant::now();
    for _ in 0..bench_iters {
        warm_iso.reset_for_request("bench-keepalive-tenant", 1, 500_000, 8 * 1024 * 1024);
        let res = warm_iso
            .execute_http_request_fast(&bc_fast, None, &req)
            .expect("fast http");
        assert_eq!(res.status_code, 200);
    }
    let us_per_req = (t0.elapsed().as_nanos() as f64 / 1_000.0) / (bench_iters as f64);
    println!(
        "[ISO-PERF] single-tenant warm execute_http_request_fast: {:.2} us/req",
        us_per_req
    );
}
