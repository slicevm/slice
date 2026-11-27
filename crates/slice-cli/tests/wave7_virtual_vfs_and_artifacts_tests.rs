use crate::commands::artifacts::{detect_artifact_mime, write_single_artifact_safely};
use crate::commands::cmd_run;
use crate::diagnostics::mcp::{
    handle_mcp_jsonrpc_line, handle_mcp_jsonrpc_line_with_artifacts_dir,
};
use slice_engine::SliceIsolate;
use slice_sandbox::CapabilityToken;

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "slice_wave7_{label}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn test_v0150_cli_run_artifacts_dir_success_dlp_and_post_violation() {
    let out_dir = unique_temp_dir("run_art");
    let out_dir_str = out_dir.to_string_lossy().into_owned();

    // 1. Success run writes clean artifacts to host dir and drops DLP-tainted artifact
    let script = r#"
        import fs from "node:fs";
        fs.mkdirSync("/artifacts/sub", { recursive: true });
        fs.writeFileSync("/artifacts/sub/summary.csv", "k,v\na,1\n");
        fs.writeFileSync("/artifacts/leaked.txt", "secret=sk_live_cli_art_secret_12345");
        console.log("ran-ok");
    "#;
    let args = vec![
        format!("--artifacts-dir={out_dir_str}"),
        "--audit-egress=sk_live_cli_art_secret_12345".to_string(),
        "--json".to_string(),
        "-e".to_string(),
        script.to_string(),
    ];
    cmd_run(&args).expect("cmd_run with --artifacts-dir should succeed");

    let written_csv = out_dir.join("sub").join("summary.csv");
    assert_eq!(std::fs::read_to_string(&written_csv).unwrap(), "k,v\na,1\n");
    assert!(
        !out_dir.join("leaked.txt").exists(),
        "DLP-tainted artifact must never be written to host artifacts_dir"
    );

    // 2. Post-violation drain: file written before fuel exhaustion is still drained to host dir
    let fail_dir = unique_temp_dir("run_art_fail");
    let fail_dir_str = fail_dir.to_string_lossy().into_owned();
    let fail_script = r#"
        import fs from "node:fs";
        fs.writeFileSync("/artifacts/before_abort.txt", "preserved");
        while (true) {}
    "#;
    let fail_args = vec![
        format!("--artifacts-dir={fail_dir_str}"),
        "--fuel=50000".to_string(),
        "--quiet".to_string(),
        "-e".to_string(),
        fail_script.to_string(),
    ];
    assert!(
        cmd_run(&fail_args).is_err(),
        "infinite loop must fail with fuel exhaustion"
    );
    assert_eq!(
        std::fs::read_to_string(fail_dir.join("before_abort.txt")).unwrap(),
        "preserved"
    );

    // 3. Safe host writer rejects traversal, absolute, and backslash paths
    assert!(write_single_artifact_safely(&out_dir, "../escape.txt", b"x").is_err());
    assert!(write_single_artifact_safely(&out_dir, "/abs/path.txt", b"x").is_err());
    assert!(write_single_artifact_safely(&out_dir, r"a\b.txt", b"x").is_err());

    let _ = std::fs::remove_dir_all(&out_dir);
    let _ = std::fs::remove_dir_all(&fail_dir);
}

#[test]
fn test_v0150_mcp_artifacts_image_resource_dlp_and_resource_link() {
    let mut iso = SliceIsolate::spawn(
        "mcp-art-test",
        101,
        CapabilityToken::pure_compute(),
        20_000_000,
        16 * 1024 * 1024,
    );
    iso.add_egress_audit_secret("sk_live_mcp_art_secret_555");

    // 1. tools/list advertises `artifacts` and `virtualMounts`
    let list_resp = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#,
        &mut iso,
    )
    .unwrap();
    let list_json: serde_json::Value = serde_json::from_str(&list_resp).unwrap();
    let props = &list_json["result"]["tools"][0]["inputSchema"]["properties"];
    assert_eq!(props["artifacts"]["type"], "boolean");
    assert!(props["virtualMounts"].is_object());

    // 2. slice_eval_ts with artifacts:true -> PNG -> image, SVG -> resource (NOT image!), CSV -> resource, secret -> skipped
    let code = r#"
        import fs from "node:fs";
        const png = new Uint8Array([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x01]);
        fs.writeFileSync("/artifacts/chart.png", png);
        fs.writeFileSync("/artifacts/diagram.svg", "<svg><circle r='5'/></svg>");
        fs.writeFileSync("/artifacts/table.csv", "col1,col2\n1,2\n");
        fs.writeFileSync("/artifacts/secret.txt", "leak=sk_live_mcp_art_secret_555");
        "ok";
    "#;
    let call_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "slice_eval_ts",
            "arguments": {
                "code": code,
                "artifacts": true
            }
        }
    })
    .to_string();
    let call_resp = handle_mcp_jsonrpc_line(&call_req, &mut iso).unwrap();
    let call_json: serde_json::Value = serde_json::from_str(&call_resp).unwrap();
    assert_eq!(call_json["result"]["isError"], false);

    let content = call_json["result"]["content"].as_array().unwrap();
    assert_eq!(
        content.len(),
        4,
        "expected 1 text + 1 image + 2 resource blocks: {content:?}"
    );

    let payload: serde_json::Value =
        serde_json::from_str(content[0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(payload["artifacts"].as_array().unwrap().len(), 3);
    assert_eq!(payload["skippedArtifacts"].as_array().unwrap().len(), 1);
    assert_eq!(payload["skippedArtifacts"][0]["path"], "secret.txt");
    assert_eq!(payload["skippedArtifacts"][0]["reason"], "auditedSecret");
    assert!(
        payload["deniedCapabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str() == Some("artifact:secret.txt"))
    );

    // Block 1: PNG as image
    assert_eq!(content[1]["type"], "image");
    assert_eq!(content[1]["mimeType"], "image/png");
    assert!(content[1]["data"].as_str().is_some_and(|s| !s.is_empty()));

    // Block 2: SVG as resource (NEVER image!)
    assert_eq!(content[2]["type"], "resource");
    assert_eq!(
        content[2]["resource"]["uri"],
        "slice://artifacts/diagram.svg"
    );
    assert_eq!(content[2]["resource"]["mimeType"], "image/svg+xml");
    assert_eq!(content[2]["resource"]["text"], "<svg><circle r='5'/></svg>");

    // Block 3: CSV as resource
    assert_eq!(content[3]["type"], "resource");
    assert_eq!(content[3]["resource"]["uri"], "slice://artifacts/table.csv");
    assert_eq!(content[3]["resource"]["mimeType"], "text/csv");

    // 3. Binary PDF with --artifacts-dir emits resource_link + writes safely to host dir
    let mcp_art_dir = unique_temp_dir("mcp_pdf");
    let pdf_code = r#"
        import fs from "node:fs";
        const pdf = new Uint8Array([0x25, 0x50, 0x44, 0x46, 0x2D, 0x31, 0x2E, 0x37, 0x0A]);
        fs.writeFileSync("/artifacts/report.pdf", pdf);
        "pdf-ok";
    "#;
    let pdf_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "slice_eval_ts",
            "arguments": {
                "code": pdf_code,
                "artifacts": true
            }
        }
    })
    .to_string();
    let pdf_resp =
        handle_mcp_jsonrpc_line_with_artifacts_dir(&pdf_req, &mut iso, Some(mcp_art_dir.as_path()))
            .unwrap();
    let pdf_json: serde_json::Value = serde_json::from_str(&pdf_resp).unwrap();
    let pdf_content = pdf_json["result"]["content"].as_array().unwrap();
    assert_eq!(pdf_content.len(), 2);
    assert_eq!(pdf_content[1]["type"], "resource_link");
    assert_eq!(pdf_content[1]["name"], "report.pdf");
    assert_eq!(pdf_content[1]["mimeType"], "application/pdf");
    assert!(
        pdf_content[1]["uri"]
            .as_str()
            .is_some_and(|u| u.starts_with("file://"))
    );
    assert!(mcp_art_dir.join("report.pdf").exists());

    // 4. MIME sniffing sanity checks
    assert_eq!(
        detect_artifact_mime("x.bin", b"\x89PNG\r\n\x1a\n"),
        "image/png"
    );
    assert_eq!(detect_artifact_mime("x.svg", b"<svg/>"), "image/svg+xml");
    assert_eq!(
        detect_artifact_mime("x.bin", &[0x00, 0x01, 0x02]),
        "application/octet-stream"
    );

    let _ = std::fs::remove_dir_all(&mcp_art_dir);
}
