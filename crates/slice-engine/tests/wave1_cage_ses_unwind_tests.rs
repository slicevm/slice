use crate::boa_realm::{
    BOA_ACTIVE_CAPS, BOA_EGRESS_SECRETS, BOA_ENV_MAP, BOA_HTTP_REQ, BOA_NET_HOSTS,
};
use crate::isolate::exec_boa::BoaSessionUnwindGuard;
use crate::isolate::http_and_ws::{ActiveHttpReqGuard, populate_boa_http_req};
use crate::runtime::workers::{
    WorkerTurnUnwindGuard, global_active_workers_for_test, inc_global_active_workers_for_test,
};
use crate::*;
use slice_sandbox::{CapabilityToken, SandboxViolation};

#[test]
fn test_wave1_162_nested_eval_sandbox_persistent_kv_cage_and_fault_rollback() {
    // 1. Nested Slice.evalSandbox loop creating distinct sub-tenants (`s0`, `s1`, ...)
    //    must accumulate persistent EDGE_KV_SLAB usage against the parent's memory ceiling
    //    and trap deterministically with MemoryCeilingExceeded instead of growing unboundedly.
    let parent_limit = 96 * 1024;
    let mut parent_iso = SliceIsolate::spawn(
        "wave1-nested-kv-cage",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        parent_limit,
    );
    let unbounded_sub_kv_src = r#"
        for (let i = 0; i < 32; i++) {
            const r = Slice.evalSandbox(
                `Slice.kv.set("blob", "x".repeat(8192)); "ok"`,
                { tenantId: "s" + i, memoryBytes: 32768, fuel: 200000 }
            );
            if (!r.ok) {
                throw new Error("sub-sandbox failed at " + i + ": " + r.violation);
            }
        }
        "escaped-parent-cage"
    "#;
    let bc = compile_source_to_bytecode(unbounded_sub_kv_src);
    let res = parent_iso.execute_bytecode(&bc, None);
    // Verify total tenant tree KV bytes never exceed parent_limit, and that either parent or
    // child sandbox failed closed with MemoryCeilingExceeded once parent_rem_mem ran out!
    let tree_bytes = crate::state_and_kv::edge_kv_tenant_tree_bytes("wave1-nested-kv-cage");
    assert!(
        tree_bytes <= parent_limit,
        "persistent sub-tenant KV tree ({tree_bytes} B) escaped parent ceiling ({parent_limit} B); res={res:?}"
    );
    match res {
        Err(SandboxViolation::MemoryCeilingExceeded { .. }) => {}
        Ok(m) => {
            assert!(
                m.return_value.contains("exceeded memory ceiling"),
                "expected sub-sandbox MemoryCeilingExceeded stop, got Ok({})",
                m.return_value
            );
        }
        Err(other) => panic!("unexpected violation: {other:?}"),
    }
    crate::state_and_kv::restore_or_clear_tenant_tree_kv("wave1-nested-kv-cage", None);

    // 2. Uncommitted child KV writes MUST roll back when child faults (e.g. FuelExhausted),
    //    and deleting sub-tenant keys MUST reclaim parent budget via signed delta.
    let mut rollback_iso = SliceIsolate::spawn(
        "wave1-kv-rollback",
        2,
        CapabilityToken::pure_compute(),
        5_000_000,
        72 * 1024,
    );
    let rollback_and_reclaim_src = r#"
        // Seed committed baseline in sub-tenant "plugin-a"
        const seed = Slice.evalSandbox(
            `Slice.kv.set("committed", "v1"); "seeded"`,
            { tenantId: "plugin-a", memoryBytes: 32768, fuel: 200000 }
        );
        // Faulting turn mutates "committed" and adds 8 KiB "uncommitted", then spins out of fuel
        const faulted = Slice.evalSandbox(
            `Slice.kv.set("committed", "corrupted");
             Slice.kv.set("uncommitted", "y".repeat(8192));
             while (true) {}`,
            { tenantId: "plugin-a", memoryBytes: 49152, fuel: 5000 }
        );
        // Verify rollback: "committed" is still "v1" and "uncommitted" does not exist
        const check = Slice.evalSandbox(
            `JSON.stringify({
                committed: Slice.kv.get("committed"),
                uncommitted: Slice.kv.get("uncommitted")
             })`,
            { tenantId: "plugin-a", memoryBytes: 32768, fuel: 200000 }
        );
        // Reclaim test: allocate 24 KiB persistent KV in "plugin-b" (peak ~53 KiB), delete it,
        // then allocate 24 KiB persistent KV in "plugin-c" under a 72 KiB parent ceiling
        // (without signed-delta reclamation on delB, remaining parent budget < 44 KiB would OOM!)
        const allocB = Slice.evalSandbox(
            `Slice.kv.set("big", "b".repeat(24576)); "ok"`,
            { tenantId: "plugin-b", memoryBytes: 65536, fuel: 300000 }
        );
        const delB = Slice.evalSandbox(
            `Slice.kv.del("big"); Slice.kv.get("big") === null`,
            { tenantId: "plugin-b", memoryBytes: 32768, fuel: 200000 }
        );
        const allocC = Slice.evalSandbox(
            `Slice.kv.set("big", "c".repeat(24576)); "ok"`,
            { tenantId: "plugin-c", memoryBytes: 65536, fuel: 300000 }
        );
        JSON.stringify({
            seedOk: seed.ok,
            faultedOk: faulted.ok,
            hasFuelViolation: String(faulted.violation || "").toLowerCase().includes("fuel"),
            check: JSON.parse(check.returnValue),
            allocBOk: allocB.ok && allocB.returnValue === "ok",
            delBOk: delB.ok && delB.returnValue === "true",
            allocCOk: allocC.ok && allocC.returnValue === "ok"
        });
    "#;
    let bc_rb = compile_source_to_bytecode(rollback_and_reclaim_src);
    let m_rb = rollback_iso
        .execute_bytecode(&bc_rb, None)
        .expect("rollback and signed-delta reclamation script should succeed");
    let out: serde_json::Value = serde_json::from_str(&m_rb.return_value).unwrap();
    assert_eq!(out["seedOk"], true);
    assert_eq!(out["faultedOk"], false);
    assert_eq!(out["hasFuelViolation"], true);
    assert_eq!(out["check"]["committed"], "v1");
    assert!(out["check"]["uncommitted"].is_null());
    assert_eq!(out["allocBOk"], true);
    assert_eq!(out["delBOk"], true);
    assert_eq!(out["allocCOk"], true);

    crate::state_and_kv::restore_or_clear_tenant_tree_kv("wave1-kv-rollback", None);
}

#[test]
fn test_wave1_162_warm_reset_for_request_persistent_kv_and_vfs_ceiling() {
    let caps = CapabilityToken::pure_compute()
        .with_read_prefix("/tmp")
        .with_write_prefix("/tmp");
    let mem_cap = 84 * 1024;
    let mut warm_iso = SliceIsolate::spawn("wave1-warm-cage", 1, caps, 2_000_000, mem_cap);

    // Turn 1: write 24 KiB to KV and 20 KiB to VFS (44 KiB persistent; peak ~64 KiB <= 80 KiB net)
    let turn1 = compile_source_to_bytecode(
        r#"
        const fs = require("node:fs");
        Slice.kv.set("k1", "a".repeat(24 * 1024));
        fs.writeFileSync("/tmp/f1.bin", Buffer.alloc(20 * 1024, 0x41));
        "turn1-ok"
        "#,
    );
    assert_eq!(
        warm_iso
            .execute_bytecode(&turn1, None)
            .unwrap()
            .return_value,
        "turn1-ok"
    );

    // Turn 2 after reset_for_request: overwriting existing 24 KiB KV key (net delta 0) MUST succeed
    warm_iso.reset_for_request("wave1-warm-cage", 1, 2_000_000, mem_cap);
    let turn2_overwrite = compile_source_to_bytecode(
        r#"
        Slice.kv.set("k1", "b".repeat(24 * 1024));
        Slice.kv.get("k1").length
        "#,
    );
    assert_eq!(
        warm_iso
            .execute_bytecode(&turn2_overwrite, None)
            .unwrap()
            .return_value,
        "24576"
    );

    // Turn 3 after reset_for_request: adding another 44 KiB key pushes persistent state to 88 KiB > 84 KiB
    warm_iso.reset_for_request("wave1-warm-cage", 1, 2_000_000, mem_cap);
    let turn3_oom = compile_source_to_bytecode(
        r#"
        Slice.kv.set("k2", "c".repeat(44 * 1024));
        "escaped-warm-oom"
        "#,
    );
    let err = warm_iso
        .execute_bytecode(&turn3_oom, None)
        .expect_err("persistent KV + VFS across reset_for_request must enforce memory_limit_bytes");
    assert!(
        matches!(err, SandboxViolation::MemoryCeilingExceeded { .. }),
        "expected MemoryCeilingExceeded on turn 3, got: {err:?}"
    );

    // Turn 4 after reset_for_request with tighter 32 KiB limit: Tier-0 SLICESNP turn must also fail closed!
    warm_iso.reset_for_request("wave1-warm-cage", 1, 2_000_000, 32 * 1024);
    let turn4_tier0 = compile_source_to_bytecode("const x = 1 + 2; x");
    let err_t0 = warm_iso.execute_bytecode(&turn4_tier0, None).expect_err(
        "Tier-0 turn after reset_for_request must also enforce live 44 KiB KV+VFS bytes",
    );
    assert!(matches!(
        err_t0,
        SandboxViolation::MemoryCeilingExceeded { .. }
    ));

    crate::state_and_kv::restore_or_clear_tenant_tree_kv("wave1-warm-cage", None);
}

#[test]
fn test_wave1_162_ses_27_intrinsics_tier0_tier1_boacas_warm_rc_and_pool_quarantine() {
    let attack_vectors: &[(&str, &str)] = &[
        ("ctor_dot_method", "Object.create = () => ({}); 1"),
        ("ctor_bracket_method", r#"Object["assign"] = () => ({}); 1"#),
        (
            "globalthis_proto",
            "globalThis.Object.prototype.polluted = 1; 1",
        ),
        (
            "globalthis_array_ctor",
            "globalThis.Array.from = () => []; 1",
        ),
        (
            "destructure_proto",
            "({ x: Object.prototype.polluted } = { x: 1 }); 1",
        ),
        ("optchain_proto", "(Object?.prototype).polluted = 1; 1"),
        (
            "object_assign_same_shape",
            "Object.assign(Array.prototype, { map: () => [99] }); 1",
        ),
        (
            "reflect_set_json",
            r#"Reflect.set(JSON, "parse", () => ({})); 1"#,
        ),
        (
            "reflect_set_reflect",
            r#"Reflect.set(Reflect, "get", () => 42); 1"#,
        ),
        (
            "computed_dunder_proto",
            r#"const p = "__pro" + "to__"; const m = "toSt" + "ring"; ({})[p][m] = () => "pwn"; 1"#,
        ),
        ("map_proto_overwrite", "Map.prototype.get = () => 123; 1"),
        ("set_proto_overwrite", "Set.prototype.has = () => true; 1"),
        (
            "uint8array_proto_overwrite",
            "Uint8Array.prototype.subarray = () => null; 1",
        ),
        (
            "promise_then_with_nexttick",
            "process.nextTick(() => {}); Promise.prototype.then = () => {}; 1",
        ),
    ];

    // 1. Default Tier + Precompiled BoaCAS (`SLICEBC\x01`) execution
    for &(name, snippet) in attack_vectors {
        let bc = compile_source_to_bytecode(snippet);
        let mut iso = SliceIsolate::spawn(
            format!("ses-t1-{name}"),
            1,
            CapabilityToken::pure_compute(),
            1_000_000,
            4 * 1024 * 1024,
        );
        let err = iso
            .execute_bytecode(&bc, None)
            .expect_err(&format!("SES vector '{name}' must fail closed"));
        assert!(
            matches!(err, SandboxViolation::IntrinsicMutationBlocked { .. }),
            "SES vector '{name}' expected IntrinsicMutationBlocked, got: {err:?}"
        );

        let cas_payload = pack_boacas_container(snippet);
        let mut cas_iso = SliceIsolate::spawn(
            format!("ses-cas-{name}"),
            1,
            CapabilityToken::pure_compute(),
            1_000_000,
            4 * 1024 * 1024,
        );
        let cas_err = cas_iso
            .execute_bytecode(&cas_payload, None)
            .expect_err(&format!("BoaCAS SES vector '{name}' must fail closed"));
        assert!(
            matches!(cas_err, SandboxViolation::IntrinsicMutationBlocked { .. }),
            "BoaCAS SES vector '{name}' expected IntrinsicMutationBlocked, got: {cas_err:?}"
        );
    }

    // 2. Forced `--tier=0` parity on constructor index assignment, destructuring, optional chaining, and Object.assign
    set_thread_tier_override(Some(0));
    for &(name, snippet) in &[
        ("tier0_ctor_dot_proto", "Object.prototype = {}; 1"),
        ("tier0_ctor_bracket", r#"Object["prototype"] = {}; 1"#),
        (
            "tier0_destructure",
            "({ x: Object.prototype.polluted } = { x: 1 }); 1",
        ),
        ("tier0_optchain", "(Object?.prototype).polluted = 1; 1"),
        (
            "tier0_object_assign",
            "Object.assign(Object.prototype, { a: 1 }); 1",
        ),
    ] {
        let bc = compile_source_to_bytecode(snippet);
        let mut iso = SliceIsolate::spawn(
            format!("ses-forced0-{name}"),
            1,
            CapabilityToken::pure_compute(),
            1_000_000,
            4 * 1024 * 1024,
        );
        let err = iso
            .execute_bytecode(&bc, None)
            .expect_err(&format!("--tier=0 SES vector '{name}' must fail closed"));
        assert!(
            matches!(err, SandboxViolation::IntrinsicMutationBlocked { .. }),
            "--tier=0 SES vector '{name}' expected IntrinsicMutationBlocked, got: {err:?}"
        );
    }
    set_thread_tier_override(None);

    // 3. Deterministic mode (`--deterministic`) must NOT mask guest tampering with Math.random or Date.now
    for &(name, snippet) in &[
        ("det_math_random_tamper", "Math.random = () => 0.777; 1"),
        ("det_date_now_tamper", "Date.now = () => 999; 1"),
    ] {
        let bc = compile_source_to_bytecode(snippet);
        let mut det_iso = SliceIsolate::spawn(
            format!("ses-{name}"),
            1,
            CapabilityToken::pure_compute(),
            1_000_000,
            4 * 1024 * 1024,
        );
        det_iso.set_deterministic_seed(Some(0x1234_5678));
        let err = det_iso
            .execute_bytecode(&bc, None)
            .expect_err(&format!("deterministic SES vector '{name}' must trap"));
        assert!(
            matches!(err, SandboxViolation::IntrinsicMutationBlocked { .. }),
            "deterministic SES vector '{name}' expected IntrinsicMutationBlocked, got: {err:?}"
        );
    }

    // 4. Warm `__slice_rc` (`BOACAS_FLAG_IS_RC_BUNDLE`) HTTP bundle mutating intrinsic on warm request #2
    //    must trap and evict the tainted realm from BOA_CONTEXT_POOL!
    let rc_src = r#"var __slice_modules = {};//__SLICE_RC:feedface12345678
var __rc = (globalThis.__slice_rc || (globalThis.__slice_rc = Object.create(null)));
if (!__rc["feedface12345678"]) {
  __rc["feedface12345678"] = function() {
    if (Slice.request().path === "/poison") {
      Object.assign(Array.prototype, { map: () => ["poisoned"] });
    }
    Slice.respond(200, String([1, 2].map((x) => x * 10).join(",")), "text/plain");
  };
}
__rc["feedface12345678"]();"#;
    let rc_bc = pack_boacas_container(rc_src);
    let mut http_iso = SliceIsolate::spawn(
        "ses-warm-http-tenant",
        1,
        CapabilityToken::pure_compute(),
        1_000_000,
        4 * 1024 * 1024,
    );
    let ok_req = HttpRequestContext {
        method: "GET",
        path: "/ok",
        query: "",
        tenant_id: "ses-warm-http-tenant",
        body: "",
        body_bytes: None,
        headers: &[],
    };
    let r1 = http_iso
        .execute_http_request_fast(&rc_bc, None, &ok_req)
        .expect("warm req 1");
    assert_eq!(r1.response_body, "10,20");

    http_iso.reset_for_request("ses-warm-http-tenant", 1, 1_000_000, 4 * 1024 * 1024);
    let poison_req = HttpRequestContext {
        path: "/poison",
        ..ok_req
    };
    let poison_err = http_iso
        .execute_http_request_fast(&rc_bc, None, &poison_req)
        .expect_err("warm __slice_rc request mutating Array.prototype.map must trap");
    assert!(matches!(
        poison_err,
        SandboxViolation::IntrinsicMutationBlocked { .. }
    ));

    // Request #3 after eviction must get a fresh pristine realm where Array.prototype.map is intact
    http_iso.reset_for_request("ses-warm-http-tenant", 1, 1_000_000, 4 * 1024 * 1024);
    let r3 = http_iso
        .execute_http_request_fast(&rc_bc, None, &ok_req)
        .expect("clean realm after eviction");
    assert_eq!(r3.response_body, "10,20");

    // 5. Unnamed ("") default-tenant realm must never be adopted by a named tenant after execution
    SliceIsolate::prewarm_boa_pool(1);
    let unnamed_bc =
        compile_source_to_bytecode(r#"globalThis.__unnamedTenantLeak = "secret-42"; "ok""#);
    let mut unnamed_iso = SliceIsolate::spawn(
        "",
        1,
        CapabilityToken::pure_compute(),
        500_000,
        4 * 1024 * 1024,
    );
    assert_eq!(
        unnamed_iso
            .execute_bytecode(&unnamed_bc, None)
            .unwrap()
            .return_value,
        "ok"
    );

    let named_probe_bc = compile_source_to_bytecode(r#"typeof globalThis.__unnamedTenantLeak"#);
    let mut named_iso = SliceIsolate::spawn(
        "named-tenant-alpha",
        2,
        CapabilityToken::pure_compute(),
        500_000,
        4 * 1024 * 1024,
    );
    assert_eq!(
        named_iso
            .execute_bytecode(&named_probe_bc, None)
            .unwrap()
            .return_value,
        "undefined",
        "named tenant must never inherit global state from a previously executed unnamed isolate"
    );
}

#[test]
fn test_wave1_162_tls_and_worker_unwind_guards_scrub_ocap_and_http_state() {
    let join = std::thread::spawn(|| {
        let baseline_workers = global_active_workers_for_test();

        let panic_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // Arm all three Wave 1 unwind guards and populate sensitive OCap + HTTP TLS state
            let _session_guard = BoaSessionUnwindGuard::new();
            inc_global_active_workers_for_test();
            let _worker_guard = WorkerTurnUnwindGuard { armed: true };

            BOA_ACTIVE_CAPS.with(|c| {
                let mut caps = c.borrow_mut();
                caps.read_prefixes.push("/secret-root".to_string());
                caps.write_prefixes.push("/secret-root".to_string());
            });
            BOA_NET_HOSTS.with(|h| {
                h.borrow_mut().insert("internal.corp:8443".to_string());
            });
            BOA_ENV_MAP.with(|e| {
                e.borrow_mut().insert(
                    "AWS_SECRET_ACCESS_KEY".to_string(),
                    "top-secret".to_string(),
                );
            });
            BOA_EGRESS_SECRETS.with(|s| s.borrow_mut().push("canary-secret".to_string()));

            let headers = [("authorization", "Bearer super-secret-jwt")];
            let req = HttpRequestContext {
                method: "POST",
                path: "/admin/keys",
                query: "token=secret",
                tenant_id: "privileged-tenant",
                body: "private-payload",
                body_bytes: Some(b"private-payload"),
                headers: &headers,
            };
            populate_boa_http_req(&req);
            let _http_guard = ActiveHttpReqGuard;

            panic!("simulated host panic mid-turn");
        }));
        assert!(panic_res.is_err());

        // Verify every privileged thread-local and atomic worker slot was scrubbed during unwind
        assert_eq!(global_active_workers_for_test(), baseline_workers);
        BOA_ACTIVE_CAPS.with(|c| {
            let caps = c.borrow();
            assert!(caps.read_prefixes.is_empty());
            assert!(caps.write_prefixes.is_empty());
        });
        BOA_NET_HOSTS.with(|h| assert!(h.borrow().is_empty()));
        BOA_ENV_MAP.with(|e| assert!(e.borrow().is_empty()));
        BOA_EGRESS_SECRETS.with(|s| assert!(s.borrow().is_empty()));
        BOA_HTTP_REQ.with(|r| {
            let req = r.borrow();
            assert!(!req.active);
            assert!(req.path.is_empty());
            assert!(req.body.is_empty());
            assert!(req.body_bytes.is_empty());
            assert!(req.headers.is_empty());
            assert_eq!(req.headers_len, 0);
        });

        // Subsequent pure-compute isolate on this same worker thread must see zero leaked state
        let probe_bc = compile_source_to_bytecode(
            r#"
            JSON.stringify({
                envSecret: process.env.AWS_SECRET_ACCESS_KEY || null,
                httpAuth: Slice.request().header("authorization") || null
            });
            "#,
        );
        let mut clean_iso = SliceIsolate::spawn(
            "post-panic-pure-tenant",
            1,
            CapabilityToken::pure_compute(),
            500_000,
            4 * 1024 * 1024,
        );
        let m = clean_iso
            .execute_bytecode(&probe_bc, None)
            .expect("pure-compute isolate after recovered panic must execute cleanly");
        let parsed: serde_json::Value = serde_json::from_str(&m.return_value).unwrap();
        assert!(parsed["envSecret"].is_null());
        assert!(parsed["httpAuth"].is_null());

        // Attempting to access the previously granted /secret-root path must trap with FsReadDenied!
        clean_iso.reset_for_request("post-panic-pure-tenant", 1, 500_000, 4 * 1024 * 1024);
        let fs_probe_bc = compile_source_to_bytecode(
            r#"
            const fs = require("node:fs");
            fs.existsSync("/secret-root/key.pem")
            "#,
        );
        let fs_err = clean_iso
            .execute_bytecode(&fs_probe_bc, None)
            .expect_err("unwound /secret-root read prefix must not leak to subsequent isolate");
        assert!(matches!(fs_err, SandboxViolation::FsReadDenied { .. }));
    });

    join.join()
        .expect("unwind test thread must complete cleanly");
}
