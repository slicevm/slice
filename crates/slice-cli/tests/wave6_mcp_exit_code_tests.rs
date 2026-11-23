use crate::commands::init::DEFAULT_SLICE_D_TS;
use crate::diagnostics::handle_mcp_jsonrpc_line;
use slice_engine::{SandboxEvalOptions, SliceIsolate, compile_source_to_bytecode};
use slice_sandbox::CapabilityToken;

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
    let parse_mcp_call = |req: &str, iso: &mut SliceIsolate| -> (bool, serde_json::Value) {
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
    };

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
