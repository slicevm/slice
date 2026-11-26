use crate::commands::init::DEFAULT_SLICE_D_TS;
use crate::diagnostics::handle_mcp_jsonrpc_line;
use slice_engine::{SandboxEvalOptions, SliceIsolate, compile_source_to_bytecode};
use slice_sandbox::CapabilityToken;

fn parse_mcp_call(req: &str, iso: &mut SliceIsolate) -> (bool, serde_json::Value) {
    let raw = handle_mcp_jsonrpc_line(req, iso).expect("MCP response expected");
    let envelope: serde_json::Value = serde_json::from_str(&raw).expect("valid MCP JSON");
    let is_error = envelope["result"]["isError"]
        .as_bool()
        .expect("isError bool");
    let text = envelope["result"]["content"][0]["text"]
        .as_str()
        .expect("content[0].text");
    let payload: serde_json::Value = serde_json::from_str(text).expect("payload JSON");
    (is_error, payload)
}

#[test]
fn test_issue_172_eval_sandbox_and_mcp_exit_code_and_process_exited_e2e() {
    // 0. Verify `slice.d.ts` ambient declaration advertises `exitCode` and `processExited`
    assert!(DEFAULT_SLICE_D_TS.contains("exitCode: number | null;"));
    assert!(DEFAULT_SLICE_D_TS.contains("processExited: boolean;"));

    let mut isolate = SliceIsolate::spawn(
        "issue-172-host",
        172,
        CapabilityToken::pure_compute(),
        5_000_000,
        16 * 1024 * 1024,
    );

    // 1. Direct Rust `isolate.eval_sandbox(...)`
    let normal = isolate.eval_sandbox("1 + 1", SandboxEvalOptions::default());
    assert!(normal.ok);
    assert_eq!(normal.exit_code, None);
    assert!(!normal.process_exited);
    assert!(normal.error.is_none());
    assert_eq!(normal.return_value, "2");

    let exit_zero = isolate.eval_sandbox(
        "console.log('before-0'); process.exit(0); console.log('after-0');",
        SandboxEvalOptions::default(),
    );
    assert!(exit_zero.ok);
    assert_eq!(exit_zero.exit_code, Some(0));
    assert!(exit_zero.process_exited);
    assert!(exit_zero.error.is_none());
    assert_eq!(exit_zero.stdout, vec!["before-0".to_string()]);

    let exit_three = isolate.eval_sandbox(
        "console.log('before-3'); process.exit(3); console.log('after-3');",
        SandboxEvalOptions::default(),
    );
    assert!(!exit_three.ok);
    assert_eq!(exit_three.exit_code, Some(3));
    assert!(exit_three.process_exited);
    assert!(
        exit_three
            .error
            .as_deref()
            .is_some_and(|e| e.contains("status code 3")),
        "Unexpected error field: {:?}",
        exit_three.error
    );
    assert_eq!(exit_three.stdout, vec!["before-3".to_string()]);

    let graceful_three =
        isolate.eval_sandbox("process.exitCode = 3; 42", SandboxEvalOptions::default());
    assert!(!graceful_three.ok);
    assert_eq!(graceful_three.exit_code, Some(3));
    assert!(!graceful_three.process_exited);
    assert_eq!(graceful_three.return_value, "42");
    assert!(
        graceful_three
            .error
            .as_deref()
            .is_some_and(|e| e.contains("status code 3")),
        "Unexpected error field: {:?}",
        graceful_three.error
    );

    // 2. Guest JS `Slice.evalSandbox(...)`
    let guest_js = r#"
const rNormal = Slice.evalSandbox("1 + 1");
if (rNormal.ok !== true || rNormal.exitCode !== null || rNormal.processExited !== false || rNormal.error !== null) {
    throw new Error("rNormal mismatch: " + JSON.stringify(rNormal));
}

const rZero = Slice.evalSandbox("console.log('z'); process.exit(0); console.log('unreachable');");
if (rZero.ok !== true || rZero.exitCode !== 0 || rZero.processExited !== true || rZero.error !== null || rZero.stdout.join(',') !== "z") {
    throw new Error("rZero mismatch: " + JSON.stringify(rZero));
}

const rThree = Slice.evalSandbox("console.log('t'); process.exit(3); console.log('unreachable');");
if (rThree.ok !== false || rThree.exitCode !== 3 || rThree.processExited !== true || !String(rThree.error).includes("status code 3") || rThree.stdout.join(',') !== "t") {
    throw new Error("rThree mismatch: " + JSON.stringify(rThree));
}

const rCodeProp = Slice.evalSandbox("process.exitCode = 3; 'done'");
if (rCodeProp.ok !== false || rCodeProp.exitCode !== 3 || rCodeProp.processExited !== false || !String(rCodeProp.error).includes("status code 3")) {
    throw new Error("rCodeProp mismatch: " + JSON.stringify(rCodeProp));
}

console.log("guest-eval-sandbox-ok");
"#;
    let guest_bc = compile_source_to_bytecode(guest_js);
    let mut guest_iso = SliceIsolate::spawn(
        "issue-172-guest",
        173,
        CapabilityToken::pure_compute(),
        5_000_000,
        16 * 1024 * 1024,
    );
    let guest_metrics = guest_iso.execute_bytecode(&guest_bc, None).unwrap();
    assert!(
        !guest_metrics.return_value.starts_with("Uncaught "),
        "Guest JS Slice.evalSandbox assertion failed: {}",
        guest_metrics.return_value
    );
    assert_eq!(
        guest_metrics.stdout,
        vec!["guest-eval-sandbox-ok".to_string()]
    );

    // 3. MCP JSON-RPC `slice_eval_ts` (`handle_mcp_jsonrpc_line`)
    let (err_1p1, p_1p1) = parse_mcp_call(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"1 + 1"}}}"#,
        &mut isolate,
    );
    assert!(!err_1p1);
    assert_eq!(p_1p1["ok"], serde_json::json!(true));
    assert_eq!(p_1p1["exitCode"], serde_json::Value::Null);
    assert_eq!(p_1p1["processExited"], serde_json::json!(false));
    assert_eq!(p_1p1["returnValue"], serde_json::json!("2"));

    let (err_ex0, p_ex0) = parse_mcp_call(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"process.exit(0)"}}}"#,
        &mut isolate,
    );
    assert!(!err_ex0);
    assert_eq!(p_ex0["ok"], serde_json::json!(true));
    assert_eq!(p_ex0["exitCode"], serde_json::json!(0));
    assert_eq!(p_ex0["processExited"], serde_json::json!(true));
    assert_eq!(p_ex0["error"], serde_json::Value::Null);

    let (err_ex3, p_ex3) = parse_mcp_call(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"process.exit(3)"}}}"#,
        &mut isolate,
    );
    assert!(err_ex3);
    assert_eq!(p_ex3["ok"], serde_json::json!(false));
    assert_eq!(p_ex3["exitCode"], serde_json::json!(3));
    assert_eq!(p_ex3["processExited"], serde_json::json!(true));
    assert!(
        p_ex3["error"]
            .as_str()
            .is_some_and(|s| s.contains("status code 3"))
    );

    let (err_ec3, p_ec3) = parse_mcp_call(
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"process.exitCode = 3; 'graceful'","tenantId":"worker-a","maxMemoryBytes":4194304}}}"#,
        &mut isolate,
    );
    assert!(err_ec3);
    assert_eq!(p_ec3["ok"], serde_json::json!(false));
    assert_eq!(p_ec3["exitCode"], serde_json::json!(3));
    assert_eq!(p_ec3["processExited"], serde_json::json!(false));
    assert_eq!(p_ec3["returnValue"], serde_json::json!("graceful"));
    assert!(
        p_ec3["error"]
            .as_str()
            .is_some_and(|s| s.contains("status code 3"))
    );

    // Ensure host MCP isolate remains healthy after non-zero child exits
    let (err_after, p_after) = parse_mcp_call(
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"40 + 2"}}}"#,
        &mut isolate,
    );
    assert!(!err_after);
    assert_eq!(p_after["ok"], serde_json::json!(true));
    assert_eq!(p_after["exitCode"], serde_json::Value::Null);
    assert_eq!(p_after["processExited"], serde_json::json!(false));
    assert_eq!(p_after["returnValue"], serde_json::json!("42"));
}

#[test]
fn test_wave6_deterministic_nested_eval_sandbox_preserves_ses_baseline() {
    // Runs on a fresh thread so the outer isolate boots a cold bare realm in deterministic mode
    // (matching `slice test`), invokes nested `Slice.evalSandbox(...)`, and verifies that
    // `SavedIsolateContextGuard` preserves `BOA_SES_BASELINE_SIGS` without false `Math` SES errors.
    let handle = std::thread::spawn(|| {
        let mut iso = SliceIsolate::spawn(
            "slice-test-runner",
            1,
            CapabilityToken::pure_compute(),
            20_000_000,
            16 * 1024 * 1024,
        );
        iso.set_deterministic_seed(Some(20261024));
        let src = r#"
const r1 = Math.random();
const child = Slice.evalSandbox("1 + 2", { tenantId: "nested-child" });
if (!child.ok || child.returnValue !== "3") {
    throw new Error("nested child failed: " + JSON.stringify(child));
}
console.log(JSON.stringify({ r1, childOk: child.ok }));
"#;
        let bc = slice_engine::compile_source_to_bytecode(src);
        let metrics = iso
            .execute_bytecode(&bc, None)
            .expect("deterministic outer isolate with nested evalSandbox must pass SES check");
        assert_eq!(metrics.stdout.len(), 1);
        assert!(metrics.stdout[0].contains("\"childOk\":true"));
    });
    handle.join().expect("fresh thread test must succeed");
}

#[test]
fn test_issue_176_top_level_return_and_return_await_parity_across_tiers_and_entrypoints() {
    // Zero-overhead invariant: functions containing `return` without a top-level `return`
    // must NOT be wrapped by `wrap_top_level_return_script`.
    assert!(
        slice_cas::wrap_top_level_return_script("function f(x: number) { return x + 1; } f(41);")
            .is_none()
    );
    assert!(slice_cas::has_top_level_await(
        "async function m() { return 7 }\nreturn await m();"
    ));

    let cases: [(&str, &str, &str); 6] = [
        ("a_t0_return", "const a = 21 * 2;\nreturn a;", "42"),
        (
            "b_class_return",
            "class A { v = 42 }\nreturn new A().v;",
            "42",
        ),
        (
            "c_async_return",
            "async function m() { return 7 }\nreturn await m();",
            "7",
        ),
        (
            "d_async_expr",
            "async function m() { return 7 }\nawait m();",
            "7",
        ),
        (
            "e_early_branch_return_stops_trailing_throw",
            "class Box { v: number; constructor(v: number) { this.v = v; } }\nif (true) { return new Box(99).v; }\nthrow new Error('unreachable');",
            "99",
        ),
        (
            "f_branch_fallthrough_preserves_last_expr",
            "class Box { v: number; constructor(v: number) { this.v = v; } }\nif (false) { return 0; }\nnew Box(55).v;",
            "55",
        ),
    ];

    // 1. Direct MCP `slice_eval_ts` + `SliceIsolate::eval_sandbox` + repeated warm reuse
    let mut host = SliceIsolate::spawn(
        "issue-176-mcp-host",
        1,
        CapabilityToken::pure_compute(),
        50_000_000,
        16 * 1024 * 1024,
    );
    for (name, code, expected_val) in cases {
        for run_idx in 0..2 {
            let req = serde_json::json!({
                "jsonrpc": "2.0",
                "id": run_idx + 1,
                "method": "tools/call",
                "params": {
                    "name": "slice_eval_ts",
                    "arguments": { "code": code }
                }
            });
            let (is_err, payload) = parse_mcp_call(&req.to_string(), &mut host);
            assert!(
                !is_err,
                "case {name} (run {run_idx}) failed in slice mcp: {payload}"
            );
            assert_eq!(payload["ok"], serde_json::json!(true), "case {name}");
            assert_eq!(
                payload["returnValue"],
                serde_json::json!(expected_val),
                "case {name}"
            );
        }
    }

    // 2. Realistic complex script from #176: class + generator + async main() + `return await main()`
    let complex_ts = r#"
class Helper {
    base: number;
    constructor(base: number) { this.base = base; }
    *seq() { yield this.base; yield this.base + 1; yield this.base + 2; }
}
async function main() {
    const h = new Helper(10);
    const items = Array.from(h.seq());
    return { sum: items.reduce((a, b) => a + b, 0), items };
}
return await main();
"#;
    let complex_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 99,
        "method": "tools/call",
        "params": {
            "name": "slice_eval_ts",
            "arguments": { "code": complex_ts }
        }
    });
    let (complex_err, complex_payload) = parse_mcp_call(&complex_req.to_string(), &mut host);
    assert!(
        !complex_err,
        "complex #176 script failed: {complex_payload}"
    );
    assert_eq!(complex_payload["ok"], serde_json::json!(true));
    assert_eq!(
        complex_payload["returnJson"],
        serde_json::json!("{\"items\":[10,11,12],\"sum\":33}")
    );

    // 3. Bundled `slice run` path (`bundle_entrypoint_with_deps`) for both class & async return
    let tmp_dir =
        std::env::temp_dir().join(format!("slice_issue_176_bundle_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();
    for (fname, src) in [
        (
            "b_class_return.ts",
            "class A { v = 42 }\nconsole.log('b=' + new A().v);\nreturn new A().v;\n",
        ),
        (
            "c_async_return.ts",
            "async function m() { return 7 }\nconst r = await m();\nconsole.log('c=' + r);\nreturn await m();\n",
        ),
    ] {
        let fpath = tmp_dir.join(fname);
        std::fs::write(&fpath, src).unwrap();
        let bundled = slice_cas::bundle_entrypoint_with_deps(&fpath)
            .unwrap_or_else(|e| panic!("bundle {fname} failed: {e}"));
        let bc = slice_engine::compile_source_to_bytecode(&bundled);
        let mut iso = SliceIsolate::spawn(
            "issue-176-run",
            1,
            CapabilityToken::pure_compute(),
            20_000_000,
            16 * 1024 * 1024,
        );
        let m = iso
            .execute_bytecode(&bc, None)
            .unwrap_or_else(|e| panic!("exec {fname} failed: {e}"));
        assert!(
            !m.return_value.starts_with("Uncaught "),
            "{fname} returned uncaught error: {}",
            m.return_value
        );
        assert_eq!(m.stdout.len(), 1, "{fname} expected stdout");
    }
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_issue_200_mcp_eval_ts_includes_denied_capabilities_and_issue_198_e2e() {
    let mut isolate = SliceIsolate::spawn(
        "issue-200-mcp-host",
        200,
        CapabilityToken::pure_compute(),
        5_000_000,
        16 * 1024 * 1024,
    );

    // 1. Issue #200: Caught fetch + caught fs refusal must be visible in `deniedCapabilities`
    //    without leaking internal compartment names.
    let req_denied = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2001,
        "method": "tools/call",
        "params": {
            "name": "slice_eval_ts",
            "arguments": {
                "code": "try { await fetch('https://example.com/'); } catch (_e) {}\ntry { require('fs').readFileSync('/etc/hostname', 'utf8'); } catch (_e) {}\nreturn { a: 1 };"
            }
        }
    })
    .to_string();
    let (is_err, payload) = parse_mcp_call(&req_denied, &mut isolate);
    assert!(!is_err, "caught refusals must succeed: {payload}");
    assert_eq!(payload["ok"], serde_json::json!(true));
    assert!(payload["violation"].is_null());
    assert_eq!(payload["returnJson"], serde_json::json!("{\"a\":1}"));
    let denied = payload["deniedCapabilities"]
        .as_array()
        .expect("deniedCapabilities array must be present in slice_eval_ts result");
    assert_eq!(
        denied.len(),
        2,
        "expected 2 denied capabilities: {denied:?}"
    );
    assert_eq!(
        denied[0],
        serde_json::json!("lacks outbound network authority for host 'example.com:443'")
    );
    assert_eq!(
        denied[1],
        serde_json::json!("lacks filesystem read authority for path '/etc/hostname'")
    );

    // 2. Clean run has empty `deniedCapabilities` array
    let req_clean = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2002,
        "method": "tools/call",
        "params": {
            "name": "slice_eval_ts",
            "arguments": { "code": "return 21 * 2;" }
        }
    })
    .to_string();
    let (is_err_clean, clean_payload) = parse_mcp_call(&req_clean, &mut isolate);
    assert!(!is_err_clean);
    assert_eq!(clean_payload["deniedCapabilities"], serde_json::json!([]));

    // 3. Issue #198 E2E through `slice mcp`: string literal containing `require(` with top-level return
    let req_198 = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2003,
        "method": "tools/call",
        "params": {
            "name": "slice_eval_ts",
            "arguments": {
                "code": "const r = { \"require(fs)\": 1 };\nreturn JSON.stringify(r);"
            }
        }
    })
    .to_string();
    let (is_err_198, payload_198) = parse_mcp_call(&req_198, &mut isolate);
    assert!(!is_err_198, "issue #198 MCP call failed: {payload_198}");
    assert_eq!(
        payload_198["value"],
        serde_json::json!("{\"require(fs)\":1}")
    );
}
