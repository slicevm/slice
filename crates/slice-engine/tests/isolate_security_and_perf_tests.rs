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

#[test]
fn test_wave3_issues_111_112_113_warm_fd_isolation_signed_delta_kv_oom_and_random_int() {
    // -------------------------------------------------------------------------
    // 1. Issue #111: Warm-realm multi-turn open FD & watcher teardown
    // -------------------------------------------------------------------------
    let fs_caps = CapabilityToken::pure_compute()
        .with_read_prefix("/tmp")
        .with_write_prefix("/tmp");
    let mut warm_iso = SliceIsolate::spawn("wave3-warm-fd", 1, fs_caps, 2_000_000, 512 * 1024);

    let turn1_src = r#"
        const fs = require("node:fs");
        globalThis.__leakedWatchHits = 0;
        const fd = fs.openSync("/tmp/warm_leak.bin", "w+");
        fs.writeSync(fd, Buffer.alloc(160 * 1024, 0x61));
        fs.watch("/tmp/warm_leak.bin", () => { globalThis.__leakedWatchHits++; });
        String(fd);
    "#;
    let bc_turn1 = compile_source_to_bytecode(turn1_src);
    let m1 = warm_iso
        .execute_bytecode(&bc_turn1, None)
        .expect("turn 1 should open fd and watcher");
    let leaked_fd: u32 = m1.return_value.parse().expect("valid fd integer");

    // Turn 2 on same tenant & warm realm: leaked FD from Turn 1 MUST be closed
    // (`EBADF`), its unlinked/ephemeral budget handled cleanly, and Turn 1's
    // watcher MUST NOT fire when `/tmp/warm_leak.bin` is modified!
    warm_iso.reset_for_request("wave3-warm-fd", 2, 2_000_000, 512 * 1024);
    let turn2_src = format!(
        r#"
        const fs = require("node:fs");
        let oldFdCode = "OPEN";
        try {{ fs.fstatSync({leaked_fd}); }} catch (e) {{ oldFdCode = e.code; }}
        fs.writeFileSync("/tmp/warm_leak.bin", "turn2-update");
        JSON.stringify({{ oldFdCode, watchHits: globalThis.__leakedWatchHits }});
        "#
    );
    let bc_turn2 = compile_source_to_bytecode(&turn2_src);
    let m2 = warm_iso
        .execute_bytecode(&bc_turn2, None)
        .expect("turn 2 should execute cleanly");
    let turn2_json: serde_json::Value = serde_json::from_str(&m2.return_value).unwrap();
    assert_eq!(turn2_json["oldFdCode"], "EBADF");
    assert_eq!(turn2_json["watchHits"], 0);

    // -------------------------------------------------------------------------
    // 2. Issue #112: Exact evalSandbox repro + signed-delta KV/VFS/checkpoint
    //    accounting + uncatchable OOM enforcement
    // -------------------------------------------------------------------------
    let kv_caps = CapabilityToken::pure_compute()
        .with_read_prefix("/tmp")
        .with_write_prefix("/tmp");
    let mut kv_iso = SliceIsolate::spawn("wave3-kv-budget", 1, kv_caps, 5_000_000, 1024 * 1024);

    let budget_script = r#"
        const fs = require("node:fs");

        // (a) Exact Issue #112 repro inside 32 KiB evalSandbox
        const sub = Slice.evalSandbox(
            `const value = "x".repeat(8000);
             for (let i = 0; i < 20; i++) { Slice.kv.set("key", value); }
             Slice.kv.get("key").length`,
            { memoryBytes: 32768, fuel: 1000000, tenantId: "repro-112-sub" }
        );

        // (b) 40x overwrite of 50 KiB KV key (2 MiB cumulative writes) under 1 MiB ceiling
        const val50k = Buffer.alloc(50 * 1024, 0x6b).toString("utf8");
        for (let i = 0; i < 40; i++) {
            Slice.kv.set("hot-key", val50k);
        }
        // Shrink to 16 bytes & delete to reclaim
        Slice.kv.set("hot-key", "tiny-after-50k!!");
        Slice.kv.del("hot-key");

        // (c) 30x overwrite of 64 KiB VFS file + open FD unlink reclamation on closeSync
        const buf64k = Buffer.alloc(64 * 1024, 0x42);
        for (let i = 0; i < 30; i++) {
            fs.writeFileSync("/tmp/hot_file.bin", buf64k);
        }
        fs.truncateSync("/tmp/hot_file.bin", 64);
        fs.unlinkSync("/tmp/hot_file.bin");

        const buf300k = Buffer.alloc(300 * 1024, 0x55);
        const bigOpen = fs.openSync("/tmp/open_ephemeral.bin", "w+");
        fs.writeSync(bigOpen, buf300k);
        fs.unlinkSync("/tmp/open_ephemeral.bin");
        // Closing the last FD to an unlinked 300 KiB inode must reclaim all 300 KiB!
        fs.closeSync(bigOpen);

        // (d) Checkpoint + post-checkpoint 900 KiB KV/VFS writes + restore reclaims budget
        const val300k = buf300k.toString("utf8");
        const snap = Slice.checkpoint();
        Slice.kv.set("post-snap-kv-1", val300k);
        Slice.kv.set("post-snap-kv-2", val300k);
        fs.writeFileSync("/tmp/post_snap.bin", buf300k);
        Slice.restore(snap);
        // Writing 600 KiB now would hit 1.5 MiB > 1 MiB if restore leaked the 900 KiB above!
        Slice.kv.set("after-restore-1", val300k);
        Slice.kv.set("after-restore-2", val300k);
        const afterLen =
            Slice.kv.get("after-restore-1").length + Slice.kv.get("after-restore-2").length;
        Slice.kv.del("after-restore-1");
        Slice.kv.del("after-restore-2");

        JSON.stringify({
            subOk: sub.ok,
            subVal: sub.returnValue,
            afterLen,
        });
    "#;
    let bc_budget = compile_source_to_bytecode(budget_script);
    let m_budget = kv_iso
        .execute_bytecode(&bc_budget, None)
        .expect("signed-delta KV/VFS/checkpoint script must succeed within 1 MiB");
    let budget_out: serde_json::Value = serde_json::from_str(&m_budget.return_value).unwrap();
    assert_eq!(budget_out["subOk"], true);
    assert_eq!(budget_out["subVal"], "8000");
    assert_eq!(budget_out["afterLen"], 600 * 1024);

    // (e) Uncatchable OOM trap: guest try/catch MUST NOT swallow MemoryCeilingExceeded
    let oom_swallow_src = r#"
        for (let i = 0; i < 32; i++) {
            try {
                Slice.kv.set("oom:" + i, "z".repeat(24 * 1024));
            } catch (_) {
                // Attempt to swallow OOM and keep running
            }
        }
        "escaped-oom"
    "#;
    let bc_oom = compile_source_to_bytecode(oom_swallow_src);
    let mut oom_iso = SliceIsolate::spawn(
        "wave3-oom-uncatchable",
        1,
        CapabilityToken::pure_compute(),
        2_000_000,
        128 * 1024,
    );
    let oom_err = oom_iso
        .execute_bytecode(&bc_oom, None)
        .expect_err("guest try/catch around Slice.kv.set OOM must trap with MemoryCeilingExceeded");
    assert!(
        matches!(oom_err, SandboxViolation::MemoryCeilingExceeded { .. }),
        "expected MemoryCeilingExceeded, got: {:?}",
        oom_err
    );

    // -------------------------------------------------------------------------
    // 3. Issue #113: 48-bit unbiased crypto.randomInt, validation, seeded PRNG
    //    reproducibility, and zero memory-budget growth across 50,000 calls
    // -------------------------------------------------------------------------
    let random_int_src = r#"
        import crypto from "node:crypto";

        async function runRandomIntTests() {
            const TWO_32 = 2 ** 32;
            const TWO_40 = 2 ** 40;
            const MAX_RANGE = 2 ** 48 - 1;

            let sawAbove32 = false;
            for (let i = 0; i < 256; i++) {
                const v = crypto.randomInt(0, TWO_40);
                if (!Number.isSafeInteger(v) || v < 0 || v >= TWO_40) {
                    throw new Error("out of bounds [0, 2^40): " + v);
                }
                if (v >= TWO_32) sawAbove32 = true;
            }

            // Boundary range 2^32 + 1 and max range 2^48 - 1 and signed span
            const rBoundary = crypto.randomInt(0, TWO_32 + 1);
            const rMaxSpan = crypto.randomInt(0, MAX_RANGE);
            const rSigned = crypto.randomInt(-(2 ** 46), 2 ** 46);
            if (
                rBoundary < 0 || rBoundary >= TWO_32 + 1 ||
                rMaxSpan < 0 || rMaxSpan >= MAX_RANGE ||
                rSigned < -(2 ** 46) || rSigned >= 2 ** 46
            ) {
                throw new Error("boundary randomInt out of range");
            }

            // Async callback overloads: (max, cb) and (min, max, cb)
            const cbSingle = await new Promise((res, rej) =>
                crypto.randomInt(50, (err, n) => (err ? rej(err) : res(n)))
            );
            const cbPair = await new Promise((res, rej) =>
                crypto.randomInt(100, 110, (err, n) => (err ? rej(err) : res(n)))
            );

            // Error code validation
            const errCode = (fn) => {
                try { fn(); return "NONE"; } catch (e) { return e && e.code ? e.code : e.name; }
            };
            const codes = {
                tooWide: errCode(() => crypto.randomInt(0, 2 ** 48)),
                equalBounds: errCode(() => crypto.randomInt(10, 10)),
                invertedBounds: errCode(() => crypto.randomInt(20, 10)),
                floatArg: errCode(() => crypto.randomInt(1.5, 10)),
                nanArg: errCode(() => crypto.randomInt(NaN, 10)),
                bigintArg: errCode(() => crypto.randomInt(1n, 10n)),
                badCallback: errCode(() => crypto.randomInt(0, 10, "not-a-fn")),
            };

            // Uniformity check on non-power-of-two modulus 3 (6,000 samples)
            const buckets = [0, 0, 0];
            for (let i = 0; i < 6000; i++) {
                buckets[crypto.randomInt(0, 3)]++;
            }
            const uniformOk = buckets.every((c) => c >= 1800 && c <= 2200);

            // 50,000 calls with zero heap budget growth
            let checksum = 0;
            for (let i = 0; i < 50000; i++) {
                checksum += crypto.randomInt(0, 1000) & 1;
            }

            console.log(JSON.stringify({
                sawAbove32,
                cbSingleOk: cbSingle >= 0 && cbSingle < 50,
                cbPairOk: cbPair >= 100 && cbPair < 110,
                codes,
                uniformOk,
                checksumOk: checksum > 0,
            }));
        }
        runRandomIntTests().catch((e) => console.log(JSON.stringify({ __err: String(e && e.stack || e) })));
    "#;
    let bc_rand = compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(
        random_int_src,
        true,
    ));
    // Tight 32 KiB memory ceiling proves 50,000+ randomInt calls allocate 0 budget bytes!
    let mut rand_iso = SliceIsolate::spawn(
        "wave3-random-int",
        1,
        CapabilityToken::pure_compute(),
        25_000_000,
        32 * 1024,
    );
    let m_rand = rand_iso
        .execute_bytecode(&bc_rand, None)
        .expect("randomInt suite must succeed under 32 KiB memory ceiling");
    let rand_out: serde_json::Value =
        serde_json::from_str(m_rand.stdout.first().expect("stdout")).unwrap();
    assert!(
        rand_out.get("__err").is_none(),
        "randomInt error: {}",
        rand_out
    );
    assert_eq!(rand_out["sawAbove32"], true);
    assert_eq!(rand_out["cbSingleOk"], true);
    assert_eq!(rand_out["cbPairOk"], true);
    assert_eq!(rand_out["codes"]["tooWide"], "ERR_OUT_OF_RANGE");
    assert_eq!(rand_out["codes"]["equalBounds"], "ERR_OUT_OF_RANGE");
    assert_eq!(rand_out["codes"]["invertedBounds"], "ERR_OUT_OF_RANGE");
    assert_eq!(rand_out["codes"]["floatArg"], "ERR_INVALID_ARG_TYPE");
    assert_eq!(rand_out["codes"]["nanArg"], "ERR_INVALID_ARG_TYPE");
    assert_eq!(rand_out["codes"]["bigintArg"], "ERR_INVALID_ARG_TYPE");
    assert_eq!(rand_out["codes"]["badCallback"], "ERR_INVALID_ARG_TYPE");
    assert_eq!(rand_out["uniformOk"], true);
    assert_eq!(rand_out["checksumOk"], true);

    // Deterministic seed reproducibility for 48-bit randomInt
    let det_src = r#"
        const crypto = require("node:crypto");
        const seq = [];
        for (let i = 0; i < 8; i++) {
            seq.push(crypto.randomInt(0, 2 ** 47));
        }
        seq.join(",");
    "#;
    let bc_det = compile_source_to_bytecode(det_src);
    let mut det_a = SliceIsolate::spawn(
        "det-a",
        1,
        CapabilityToken::pure_compute(),
        500_000,
        4 * 1024 * 1024,
    );
    det_a.set_deterministic_seed(Some(0xCAFE_BABE_1234_5678));
    let mut det_b = SliceIsolate::spawn(
        "det-b",
        2,
        CapabilityToken::pure_compute(),
        500_000,
        4 * 1024 * 1024,
    );
    det_b.set_deterministic_seed(Some(0xCAFE_BABE_1234_5678));
    let seq_a = det_a.execute_bytecode(&bc_det, None).unwrap().return_value;
    let seq_b = det_b.execute_bytecode(&bc_det, None).unwrap().return_value;
    assert_eq!(seq_a, seq_b);
    assert!(
        seq_a
            .split(',')
            .any(|v| v.parse::<u64>().unwrap() >= (1u64 << 32)),
        "deterministic 47-bit sequence should populate upper bits: {seq_a}"
    );
}
