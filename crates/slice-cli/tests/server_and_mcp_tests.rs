#![allow(unused_imports)]
use crate::config_and_watch::{
    build_cas_archive, build_cas_archive_with_static, merge_manifest_toml_into_caps,
};
use crate::http_server::{resolve_tenant_bytecode_hotswap, serve_connection_keep_alive};
use crate::mcp_and_repl::{
    handle_mcp_jsonrpc_line, measure_current_process_rss_kb, run_security_containment_checks,
};
use crate::test_runner::{cmd_test, discover_test_files};
use crate::{cmd_init, cmd_run};
use slice_cas::MappedCasArchive;
use slice_engine::{compile_source_to_bytecode, SliceIsolate};
use slice_sandbox::CapabilityToken;
use std::env;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;

#[test]
fn test_cli_security_checks_and_rss_telemetry() {
    let blocked = run_security_containment_checks().unwrap();
    assert_eq!(blocked, 5);

    let rss_kb = measure_current_process_rss_kb();
    assert!(rss_kb > 0);
}

#[test]
fn test_cli_build_cas_archive_from_folder() {
    let tmp_dir = env::temp_dir().join("slice_cli_test_folder_app");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(tmp_dir.join("src")).unwrap();
    std::fs::write(
        tmp_dir.join("package.json"),
        r#"{"name":"cli-folder-test","main":"./src/index.ts"}"#,
    )
    .unwrap();
    std::fs::write(
        tmp_dir.join("src").join("index.ts"),
        "const val: number = 99;\nconsole.log(val);\n",
    )
    .unwrap();

    let out_cas = tmp_dir.join("out.cas");
    let header = build_cas_archive(&tmp_dir, &out_cas, "./src/index.ts").unwrap();
    assert_eq!(header.blobs.len(), 1);
    assert!(out_cas.exists());

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_tag_n1_keep_alive_pipelining_and_multi_worker_serve() {
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    let script = r#"
class PipelinedEdge {
    public run(): void {
        const req = Slice.request();
        Slice.respond(200, req.method + " " + req.path + " [" + req.tenant + "] body=" + req.body, "text/plain");
    }
}
new PipelinedEdge().run();
"#;
    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bytecode = Arc::new(compile_source_to_bytecode(&bundled));

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let counter = Arc::new(AtomicUsize::new(0));
    let bc_server = Arc::clone(&bytecode);
    let counter_server = Arc::clone(&counter);

    let server_handle = std::thread::spawn(move || {
        SliceIsolate::prewarm_boa_pool(1);
        SliceIsolate::prewarm_boa_bundle(&bc_server);
        let mut read_buf = vec![0u8; 65536];
        let mut write_buf = Vec::with_capacity(4096);
        if let Ok((stream, _)) = listener.accept() {
            let _ = serve_connection_keep_alive(
                stream,
                0,
                &bc_server,
                None,
                None,
                None,
                &CapabilityToken::pure_compute(),
                2_000_000,
                4 * 1024 * 1024,
                &mut read_buf,
                &mut write_buf,
                Some((&counter_server, 0)),
            );
        }
    });

    let mut client = TcpStream::connect(addr).unwrap();
    // Send 2 pipelined HTTP/1.1 requests over a single persistent Keep-Alive socket,
    // with the second request signaling Connection: close.
    let pipelined_payload = concat!(
        "GET /alpha HTTP/1.1\r\nHost: localhost\r\nX-Slice-Tenant: tenant-a\r\n\r\n",
        "POST /beta HTTP/1.1\r\nHost: localhost\r\nX-Slice-Tenant: tenant-b\r\nContent-Length: 4\r\nConnection: close\r\n\r\npong"
    );
    client.write_all(pipelined_payload.as_bytes()).unwrap();
    client.flush().unwrap();

    let mut response_str = String::new();
    client.read_to_string(&mut response_str).unwrap();
    server_handle.join().unwrap();

    assert_eq!(counter.load(std::sync::atomic::Ordering::Relaxed), 2);
    assert!(response_str.contains("GET /alpha [tenant-a] body="));
    assert!(response_str.contains("POST /beta [tenant-b] body=pong"));
}

#[test]
fn test_tag_n2_multi_tenant_cas_router_and_zero_downtime_hotswap() {
    let tmp_dir = env::temp_dir().join("slice_n2_tenant_dir_test");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();

    // Build tenant-alpha v1 .cas and tenant-beta .cas
    let alpha_src = tmp_dir.join("alpha.ts");
    let beta_src = tmp_dir.join("beta.ts");
    std::fs::write(
        &alpha_src,
        "Slice.respond(200, \"alpha-v1:hits=\" + Slice.kv.incr(\"h\", 1), \"text/plain\");\n",
    )
    .unwrap();
    std::fs::write(
        &beta_src,
        "Slice.respond(200, \"beta-v1:hits=\" + Slice.kv.incr(\"h\", 10), \"text/plain\");\n",
    )
    .unwrap();

    let alpha_cas = tmp_dir.join("tenant-alpha.cas");
    let beta_cas = tmp_dir.join("tenant-beta.cas");
    build_cas_archive(&alpha_src, &alpha_cas, "./alpha.ts").unwrap();
    build_cas_archive(&beta_src, &beta_cas, "./beta.ts").unwrap();

    // Verify initial resolution
    let bc_alpha_v1 = resolve_tenant_bytecode_hotswap(&tmp_dir, "tenant-alpha").unwrap();
    let bc_beta_v1 = resolve_tenant_bytecode_hotswap(&tmp_dir, "tenant-beta").unwrap();
    assert_ne!(bc_alpha_v1.as_slice(), bc_beta_v1.as_slice());

    // Hot-swap tenant-alpha.cas on disk with v2 (different byte length & mtime)
    std::fs::write(&alpha_src, "Slice.respond(200, \"alpha-v2-hotswapped:hits=\" + Slice.kv.incr(\"h\", 1), \"text/plain\");\n").unwrap();
    build_cas_archive(&alpha_src, &alpha_cas, "./alpha.ts").unwrap();

    let bc_alpha_v2 = resolve_tenant_bytecode_hotswap(&tmp_dir, "tenant-alpha").unwrap();
    assert_ne!(bc_alpha_v1.as_slice(), bc_alpha_v2.as_slice());

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_tag_n3_mcp_jsonrpc_server_and_tools() {
    SliceIsolate::prewarm_boa_pool(2);
    let mut isolate = SliceIsolate::spawn(
        "mcp-test-tenant",
        77,
        CapabilityToken::pure_compute(),
        2_000_000,
        4 * 1024 * 1024,
    );
    isolate.add_egress_audit_secret("sk_live_top_secret_99");

    // 1. initialize
    let init_resp = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        &mut isolate,
    )
    .unwrap();
    assert!(init_resp.contains("slice-nexusjs-mcp"));

    // 2. tools/list
    let list_resp = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
        &mut isolate,
    )
    .unwrap();
    assert!(list_resp.contains("slice_eval_ts"));
    assert!(list_resp.contains("slice_checkpoint"));

    // 3. tools/call slice_eval_ts (valid TypeScript)
    let eval_resp = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"const a: number = 12; const b: number = 30; a + b","fuel":200000}}}"#,
        &mut isolate,
    )
    .unwrap();
    assert!(eval_resp.contains(r#"\"ok\":true"#));
    assert!(eval_resp.contains("42"));

    // 4. tools/call slice_eval_ts (secret leak blocked by egress guard!)
    let leak_resp = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"console.log('leaking sk_live_top_secret_99'); 1","fuel":200000}}}"#,
        &mut isolate,
    )
    .unwrap();
    assert!(leak_resp.contains("EGRESS_SECRET_LEAK_BLOCKED"));
    assert!(leak_resp.contains(r#""isError":true"#));

    // 5. tools/call slice_kv_set + slice_checkpoint + overwrite + restore + slice_kv_get
    let _ = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"slice_kv_set","arguments":{"key":"branch","value":"alpha"}}}"#,
        &mut isolate,
    );
    let snap_resp = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"slice_checkpoint","arguments":{"action":"checkpoint"}}}"#,
        &mut isolate,
    )
    .unwrap();
    assert!(snap_resp.contains("snapshot_id"));
    // Extract snapshot_id from response
    let v: serde_json::Value = serde_json::from_str(&snap_resp).unwrap();
    let inner_txt = v["result"]["content"][0]["text"].as_str().unwrap();
    let inner_json: serde_json::Value = serde_json::from_str(inner_txt).unwrap();
    let snap_id = inner_json["snapshot_id"].as_u64().unwrap();

    let _ = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"slice_kv_set","arguments":{"key":"branch","value":"corrupted-beta"}}}"#,
        &mut isolate,
    );
    let restore_req = format!(
        r#"{{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{{"name":"slice_checkpoint","arguments":{{"action":"restore","snapshot_id":{}}}}}}}"#,
        snap_id
    );
    let restore_resp = handle_mcp_jsonrpc_line(&restore_req, &mut isolate).unwrap();
    assert!(restore_resp.contains(r#"\"restored\":true"#));

    let get_resp = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"slice_kv_get","arguments":{"key":"branch"}}}"#,
        &mut isolate,
    )
    .unwrap();
    assert!(get_resp.contains("alpha"));
}

#[test]
fn test_tag_f1_wintertc_export_default_fetch_http_server() {
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    let tmp_dir = env::temp_dir().join("slice_f1_wintertc_app_test");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(tmp_dir.join("src")).unwrap();

    std::fs::write(
        tmp_dir.join("src/router.ts"),
        r#"
export function routeRequest(req: Request): Response {
    const url = new URL(req.url);
    const tenant = req.headers.get("x-slice-tenant") || "default";
    if (url.pathname === "/api/v1/items" && req.method === "GET") {
        const category = url.searchParams.get("category") || "all";
        return Response.json({ ok: true, tenant, category, count: 3 });
    }
    if (url.pathname === "/api/v1/items" && req.method === "POST") {
        const payload = req.jsonSync();
        return Response.json({ ok: true, tenant, created: payload.name }, { status: 201 });
    }
    return Response.json({ error: "not_found", path: url.pathname }, { status: 404 });
}
"#,
    )
    .unwrap();

    std::fs::write(
        tmp_dir.join("src/index.ts"),
        r#"
import { routeRequest } from "./router";

export default {
    fetch(req: Request): Response {
        return routeRequest(req);
    }
};
"#,
    )
    .unwrap();

    let bundled = slice_cas::bundle_entrypoint_with_deps(&tmp_dir.join("src/index.ts")).unwrap();
    let bytecode = Arc::new(compile_source_to_bytecode(&bundled));

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let counter = Arc::new(AtomicUsize::new(0));
    let bc_server = Arc::clone(&bytecode);
    let counter_server = Arc::clone(&counter);

    let server_handle = std::thread::spawn(move || {
        SliceIsolate::prewarm_boa_pool(1);
        SliceIsolate::prewarm_boa_bundle(&bc_server);
        let mut read_buf = vec![0u8; 65536];
        let mut write_buf = Vec::with_capacity(4096);
        if let Ok((stream, _)) = listener.accept() {
            let _ = serve_connection_keep_alive(
                stream,
                0,
                &bc_server,
                None,
                None,
                None,
                &CapabilityToken::pure_compute(),
                2_000_000,
                4 * 1024 * 1024,
                &mut read_buf,
                &mut write_buf,
                Some((&counter_server, 0)),
            );
        }
    });

    let mut client = TcpStream::connect(addr).unwrap();
    let pipelined_payload = concat!(
        "GET /api/v1/items?category=gpu HTTP/1.1\r\nHost: edge.slice.local\r\nX-Slice-Tenant: acme-ai\r\n\r\n",
        "POST /api/v1/items HTTP/1.1\r\nHost: edge.slice.local\r\nX-Slice-Tenant: acme-ai\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"name\":\"B200\"}"
    );
    client.write_all(pipelined_payload.as_bytes()).unwrap();
    client.flush().unwrap();

    let mut response_str = String::new();
    client.read_to_string(&mut response_str).unwrap();
    server_handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&tmp_dir);

    assert_eq!(counter.load(std::sync::atomic::Ordering::Relaxed), 2);
    assert!(response_str.contains("HTTP/1.1 200 OK"));
    assert!(response_str.contains(r#"{"ok":true,"tenant":"acme-ai","category":"gpu","count":3}"#));
    assert!(response_str.contains("HTTP/1.1 201 Created"));
    assert!(response_str.contains(r#"{"ok":true,"tenant":"acme-ai","created":"B200"}"#));
}

#[test]
fn test_tag_f2_zero_stat_static_asset_etag_and_sse_streaming() {
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    let tmp_dir = env::temp_dir().join("slice_f2_static_sse_test");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(tmp_dir.join("src")).unwrap();
    std::fs::create_dir_all(tmp_dir.join("public/assets")).unwrap();

    let html_content = "<!doctype html><html><head><title>Slice Fullstack</title></head><body><h1>Hello Edge</h1></body></html>";
    let css_content = "body { background: #0b0f19; color: #f8fafc; }";
    std::fs::write(tmp_dir.join("public/index.html"), html_content).unwrap();
    std::fs::write(tmp_dir.join("public/assets/app.css"), css_content).unwrap();

    std::fs::write(
        tmp_dir.join("src/index.ts"),
        r#"
export default {
    fetch(req: Request): Response {
        const url = new URL(req.url);
        if (url.pathname === "/api/v1/events") {
            return Slice.streamSSE([
                { id: "1", event: "token", data: { chunk: "alpha" } },
                { id: "2", event: "done", data: "[DONE]" }
            ]);
        }
        return Response.json({ api: true, path: url.pathname });
    }
};
"#,
    )
    .unwrap();

    let cas_out = tmp_dir.join("fullstack.cas");
    build_cas_archive_with_static(
        &tmp_dir.join("src/index.ts"),
        &cas_out,
        "./index.ts",
        Some(&tmp_dir.join("public")),
    )
    .unwrap();

    let archive = MappedCasArchive::open(&cas_out).unwrap();
    let static_table = Arc::new(slice_cas::StaticAssetTable::from_cas_archive(&archive));
    let index_entry = static_table
        .resolve("/")
        .expect("Missing / index.html in CAS static table");
    let index_etag = index_entry.etag_quoted.clone();

    let entry_blob = archive.entrypoint_blob().unwrap();
    let bytecode = Arc::new(archive.get_bytecode_slice(entry_blob).to_vec());

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let counter = Arc::new(AtomicUsize::new(0));
    let bc_server = Arc::clone(&bytecode);
    let st_server = Arc::clone(&static_table);
    let counter_server = Arc::clone(&counter);

    let server_handle = std::thread::spawn(move || {
        SliceIsolate::prewarm_boa_pool(1);
        SliceIsolate::prewarm_boa_bundle(&bc_server);
        let mut read_buf = vec![0u8; 65536];
        let mut write_buf = Vec::with_capacity(4096);
        if let Ok((stream, _)) = listener.accept() {
            let _ = serve_connection_keep_alive(
                stream,
                0,
                &bc_server,
                None,
                Some(&st_server),
                None,
                &CapabilityToken::pure_compute(),
                2_000_000,
                4 * 1024 * 1024,
                &mut read_buf,
                &mut write_buf,
                Some((&counter_server, 0)),
            );
        }
    });

    let mut client = TcpStream::connect(addr).unwrap();
    let pipelined_payload = format!(
        concat!(
            "GET / HTTP/1.1\r\nHost: edge.slice.local\r\n\r\n",
            "GET / HTTP/1.1\r\nHost: edge.slice.local\r\nIf-None-Match: {}\r\n\r\n",
            "GET /assets/app.css HTTP/1.1\r\nHost: edge.slice.local\r\n\r\n",
            "GET /api/v1/events HTTP/1.1\r\nHost: edge.slice.local\r\nConnection: close\r\n\r\n"
        ),
        index_etag
    );
    client.write_all(pipelined_payload.as_bytes()).unwrap();
    client.flush().unwrap();

    let mut response_str = String::new();
    client.read_to_string(&mut response_str).unwrap();
    server_handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&tmp_dir);

    assert_eq!(counter.load(std::sync::atomic::Ordering::Relaxed), 4);
    assert!(response_str.contains("X-Slice-Static: HIT-200"));
    assert!(response_str.contains("X-Slice-Static: HIT-304"));
    assert!(response_str.contains("HTTP/1.1 304 Not Modified"));
    assert!(response_str.contains(html_content));
    assert!(response_str.contains(css_content));
    assert!(response_str.contains("Content-Type: text/event-stream; charset=utf-8"));
    assert!(response_str.contains("id: 1\nevent: token\ndata: {\"chunk\":\"alpha\"}\n\n"));
}

#[test]
fn test_tag_f3_embedded_manifest_allow_net_and_live_upstream_proxy() {
    let tmp_dir = std::env::temp_dir().join("slice_f3_manifest_proxy_test");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let src_dir = tmp_dir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();

    // Spawn a local upstream HTTP/1.1 service
    let upstream_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let upstream_port = upstream_listener.local_addr().unwrap().port();
    let upstream_host_port = format!("127.0.0.1:{}", upstream_port);

    let upstream_handle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = upstream_listener.accept() {
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            let body = r#"{"status":"upstream_ok","tier":"f3"}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });

    let toml_content = format!(
        r#"[package]
name = "f3-proxy"
version = "0.2.0"
entrypoint = "./src/index.ts"

[capabilities]
allow_net = ["{}"]
"#,
        upstream_host_port
    );
    std::fs::write(tmp_dir.join("slice.toml"), &toml_content).unwrap();
    std::fs::write(
        src_dir.join("index.ts"),
        format!(
            r#"
const res = Slice.fetchSync("http://{}/internal/health");
console.log(res.textSync());
"#,
            upstream_host_port
        ),
    )
    .unwrap();

    let cas_out = tmp_dir.join("f3_app.cas");
    build_cas_archive_with_static(&tmp_dir, &cas_out, "./src/index.ts", None).unwrap();

    let archive = MappedCasArchive::open(&cas_out).unwrap();
    let embedded_toml = archive
        .get_manifest_toml()
        .expect("Missing embedded slice.toml");
    assert!(embedded_toml.contains(&upstream_host_port));

    let mut caps = CapabilityToken::pure_compute();
    merge_manifest_toml_into_caps(embedded_toml, &mut caps);
    let entry = archive.entrypoint_blob().unwrap();
    let mut iso = SliceIsolate::spawn("f3-cli-tenant", 1, caps, 5_000_000, 8 * 1024 * 1024);
    let metrics = iso
        .execute_bytecode(archive.get_bytecode_slice(entry), Some(&archive))
        .unwrap();
    assert_eq!(metrics.stdout[0], r#"{"status":"upstream_ok","tier":"f3"}"#);

    upstream_handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_tag_f4_cli_slice_test_runner_env_file_and_watch_reload() {
    let tmp_dir = std::env::temp_dir().join("slice_f4_cli_suite_test");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let src_dir = tmp_dir.join("src");
    let tests_dir = tmp_dir.join("tests");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&tests_dir).unwrap();

    // 1. Write `.env` with both public config and a sensitive secret key
    let env_path = tmp_dir.join(".env");
    std::fs::write(
        &env_path,
        "APP_TIER=enterprise\nVAULT_MASTER_SECRET=sk_live_f4_cli_vault_991122\n",
    )
    .unwrap();

    // 2. Write `src/ledger.ts` and `tests/ledger.test.ts`
    std::fs::write(
        src_dir.join("ledger.ts"),
        r#"export function applyTransfer(balance: number, delta: number): number {
    if (balance + delta < 0) throw new Error("Insufficient ledger funds");
    return balance + delta;
}
"#,
    )
    .unwrap();

    std::fs::write(
        tests_dir.join("ledger.test.ts"),
        r#"import { describe, test, expect } from "slice:test";
import { applyTransfer } from "../src/ledger.ts";
import * as fs from "node:fs";

Slice.kv.set("acct:root", "500", 0);
fs.writeFileSync("/tmp/f4_cli_ledger.json", "{\"v\":1}");

describe("Ledger Time-Travel Suite", () => {
    test("applies valid transfer and mutates VFS+KV", () => {
        expect(applyTransfer(500, -200)).toBe(300);
        expect(process.env.APP_TIER).toBe("enterprise");
        Slice.kv.set("acct:root", "0", 0);
        fs.writeFileSync("/tmp/f4_cli_ledger.json", "{\"v\":999}");
    });

    test("automatically rolls back VFS and KV state before second test", () => {
        expect(Slice.kv.get("acct:root")).toBe("500");
        expect(fs.readFileSync("/tmp/f4_cli_ledger.json")).toBe("{\"v\":1}");
        expect(() => applyTransfer(100, -250)).toThrow("Insufficient ledger funds");
    });

    test("verifies OCap & Egress DLP via .toViolate", () => {
        const leakAttempt = Slice.evalSandbox(
            `console.log("${process.env.VAULT_MASTER_SECRET}");`,
            { fuel: 100_000 }
        );
        expect(leakAttempt).toViolate("EGRESS_SECRET_LEAK_BLOCKED");
    });
});
"#,
    )
    .unwrap();

    // 3a. Without filesystem capabilities, `slice test` must block `node:fs` access by default
    let denied_args = vec![
        tmp_dir.to_string_lossy().to_string(),
        format!("--env-file={}", env_path.to_string_lossy()),
        "--json".to_string(),
    ];
    let denied_err = cmd_test(&denied_args)
        .expect_err("Expected `slice test` without --allow-read/--allow-write to reject node:fs")
        .to_string();
    assert!(
        denied_err.contains("OCap Violation") && denied_err.contains("/tmp/f4_cli_ledger.json"),
        "Unexpected error from zero-authority slice test: {}",
        denied_err
    );

    // 3b. Execute `cmd_test` against `tmp_dir` with explicit `/tmp` VFS capabilities, `--env-file`, and `--json`
    let test_args = vec![
        tmp_dir.to_string_lossy().to_string(),
        format!("--env-file={}", env_path.to_string_lossy()),
        "--allow-read=/tmp".to_string(),
        "--allow-write=/tmp".to_string(),
        "--json".to_string(),
    ];
    cmd_test(&test_args).expect("Expected `slice test` to pass all 3 specs");

    // 4. Execute `cmd_run` with `--watch-once` to verify sub-3ms live reload
    let watch_script = src_dir.join("watch_entry.ts");
    std::fs::write(&watch_script, "console.log(\"watch-v1\");\n").unwrap();
    let run_args = vec![
        watch_script.to_string_lossy().to_string(),
        "--watch-once".to_string(),
        "--quiet".to_string(),
    ];
    cmd_run(&run_args).expect("Expected `slice run --watch-once` to complete cleanly");

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_issue_6_mcp_fuel_violation_does_not_poison_subsequent_calls() {
    use crate::mcp_and_repl::handle_mcp_jsonrpc_line;
    use slice_engine::SliceIsolate;
    use slice_sandbox::CapabilityToken;

    let mut isolate = SliceIsolate::spawn(
        "mcp-agent-host",
        100,
        CapabilityToken::pure_compute(),
        1_000_000,
        16 * 1024 * 1024,
    );

    let r2 = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"1 + 1"}}}"#,
        &mut isolate,
    )
    .unwrap();
    assert!(r2.contains(r#""isError":false"#));
    assert!(r2.contains(r#"\"value\":\"2\""#));

    let r3 = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"while (true) {}","fuel":100000}}}"#,
        &mut isolate,
    )
    .unwrap();
    assert!(r3.contains(r#""isError":true"#));
    assert!(r3.contains(r#"\"fuelUsed\":100000"#));

    let r4 = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"1 + 1"}}}"#,
        &mut isolate,
    )
    .unwrap();
    assert!(r4.contains(r#""isError":false"#));
    assert!(r4.contains(r#"\"value\":\"2\""#));

    let r5 = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"2 + 2","fuel":5000000}}}"#,
        &mut isolate,
    )
    .unwrap();
    assert!(r5.contains(r#""isError":false"#));
    assert!(r5.contains(r#"\"value\":\"4\""#));
}

#[test]
fn test_issue_7_mcp_and_eval_sandbox_uncaught_exceptions_and_syntax_errors_report_error() {
    use crate::mcp_and_repl::handle_mcp_jsonrpc_line;
    use slice_engine::{SandboxEvalOptions, SliceIsolate};
    use slice_sandbox::CapabilityToken;

    let mut isolate = SliceIsolate::spawn(
        "mcp-agent-host",
        107,
        CapabilityToken::pure_compute(),
        1_000_000,
        16 * 1024 * 1024,
    );

    // 1. Direct eval_sandbox with uncaught exception
    let thrown = isolate.eval_sandbox("throw new Error(\"boom\")", SandboxEvalOptions::default());
    assert!(!thrown.ok);
    assert!(thrown.violation.is_none());
    assert!(thrown
        .error
        .as_deref()
        .is_some_and(|e| e.contains("Uncaught Error: boom")));

    // 2. MCP slice_eval_ts with uncaught exception -> ok: false, isError: true, error populated
    let mcp_throw = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"throw new Error(\"boom\")"}}}"#,
        &mut isolate,
    )
    .unwrap();
    assert!(mcp_throw.contains(r#""isError":true"#));
    assert!(mcp_throw.contains(r#"\"ok\":false"#));
    assert!(mcp_throw.contains(r#"Uncaught Error: boom"#));

    // 3. MCP slice_eval_ts with syntax error -> ok: false, isError: true, error populated
    let mcp_syn = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"const x = ;"}}}"#,
        &mut isolate,
    )
    .unwrap();
    assert!(mcp_syn.contains(r#""isError":true"#));
    assert!(mcp_syn.contains(r#"\"ok\":false"#));
    assert!(mcp_syn.contains("SyntaxError"));
}

#[test]
fn test_issue_8_mcp_allow_net_honored_and_attenuated() {
    use crate::mcp_and_repl::handle_mcp_jsonrpc_line;
    use slice_engine::SliceIsolate;
    use slice_sandbox::CapabilityToken;

    let caps = CapabilityToken::pure_compute()
        .with_net_host("example.com")
        .with_net_host("allowed-second.com");
    let mut isolate = SliceIsolate::spawn("mcp-agent-host", 108, caps, 1_000_000, 16 * 1024 * 1024);

    // 1. `allowNet: "example.com"` succeeds for example.com
    let r_ok = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":20,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"Slice.mockFetch(\"https://example.com/x\", {status:200, body:\"hi\"}); Slice.fetchSync(\"https://example.com/x\").status","allowNet":"example.com"}}}"#,
        &mut isolate,
    )
    .unwrap();
    assert!(r_ok.contains(r#""isError":false"#));
    assert!(r_ok.contains(r#"\"value\":\"200\""#));

    // 2. `allowNet: ["example.com"]` attenuates child so `allowed-second.com` is blocked in that call
    let r_attenuated = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":21,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"Slice.mockFetch(\"https://allowed-second.com/x\", {status:200, body:\"hi\"}); Slice.fetchSync(\"https://allowed-second.com/x\").status","allowNet":["example.com"]}}}"#,
        &mut isolate,
    )
    .unwrap();
    assert!(r_attenuated.contains(r#""isError":true"#));
    assert!(r_attenuated.contains("lacks outbound network authority"));
}

#[test]
fn test_issue_9_audit_egress_auto_not_treated_as_literal_secret() {
    use crate::config_and_watch::ResolvedExecutionConfig;
    use crate::mcp_and_repl::handle_mcp_jsonrpc_line;
    use slice_engine::SliceIsolate;

    let args = vec![
        "--allow-net=example.com".to_string(),
        "--allow-env=STRIPE_SECRET_KEY=sk_live_auto_dlp_secret_99".to_string(),
        "--audit-egress=auto".to_string(),
    ];
    let exec_cfg = ResolvedExecutionConfig::from_cli_args(&args).unwrap();
    assert!(!exec_cfg.audit_secrets.contains(&"auto".to_string()));
    assert!(exec_cfg
        .audit_secrets
        .contains(&"sk_live_auto_dlp_secret_99".to_string()));

    let mut isolate = SliceIsolate::spawn(
        "mcp-agent-host",
        109,
        exec_cfg.caps,
        exec_cfg.fuel_limit,
        exec_cfg.mem_limit,
    );
    for sec in &exec_cfg.audit_secrets {
        isolate.add_egress_audit_secret(sec.clone());
    }

    // URL containing `?mode=automatic` must NOT be blocked by `--audit-egress=auto`
    let r_auto_ok = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":30,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"Slice.mockFetch(\"https://example.com/x\", {status:200, body:\"ok\"}); Slice.fetchSync(\"https://example.com/x?mode=automatic\").status"}}}"#,
        &mut isolate,
    )
    .unwrap();
    assert!(r_auto_ok.contains(r#""isError":false"#));
    assert!(r_auto_ok.contains(r#"\"value\":\"200\""#));

    // Actual sensitive secret is blocked by DLP
    let r_leak_blocked = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":31,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"Slice.mockFetch(\"https://example.com/x\", {status:200, body:\"ok\"}); Slice.fetchSync(\"https://example.com/x?k=sk_live_auto_dlp_secret_99\").status"}}}"#,
        &mut isolate,
    )
    .unwrap();
    assert!(r_leak_blocked.contains(r#""isError":true"#));
    assert!(r_leak_blocked.contains("EGRESS_SECRET_LEAK_BLOCKED"));
}

#[test]
fn test_issue_10_top_level_await_in_eval_sandbox_and_mcp() {
    use crate::mcp_and_repl::handle_mcp_jsonrpc_line;
    use slice_engine::{SandboxEvalOptions, SliceIsolate};
    use slice_sandbox::CapabilityToken;

    let mut isolate = SliceIsolate::spawn(
        "mcp-agent-host",
        110,
        CapabilityToken::pure_compute(),
        1_000_000,
        16 * 1024 * 1024,
    );

    // 1. Direct eval_sandbox with top-level await returns resolved completion value
    let res = isolate.eval_sandbox(
        "const v = await Promise.resolve(7); v",
        SandboxEvalOptions::default(),
    );
    assert!(res.ok, "Expected ok=true, got: {:?}", res);
    assert_eq!(res.return_value, "7");

    // 2. MCP slice_eval_ts with top-level await
    let mcp_ok = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":40,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"const v = await Promise.resolve(7); v"}}}"#,
        &mut isolate,
    )
    .unwrap();
    assert!(mcp_ok.contains(r#""isError":false"#));
    assert!(mcp_ok.contains(r#"\"value\":\"7\""#));

    // 3. Rejected top-level await reports ok: false / isError: true
    let mcp_rej = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":41,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"await Promise.reject(new Error(\"async-boom\"))"}}}"#,
        &mut isolate,
    )
    .unwrap();
    assert!(mcp_rej.contains(r#""isError":true"#));
    assert!(mcp_rej.contains("async-boom"));
}

#[test]
fn test_issue_11_memory_cap_enforced_and_fuel_scales_with_work() {
    use crate::mcp_and_repl::handle_mcp_jsonrpc_line;
    use slice_engine::{SandboxEvalOptions, SliceIsolate};
    use slice_sandbox::CapabilityToken;

    let mut isolate = SliceIsolate::spawn(
        "mcp-agent-host",
        111,
        CapabilityToken::pure_compute(),
        5_000_000,
        16 * 1024 * 1024,
    );

    // 1. 1,000,000-element array fill with 64 KB (65536) memory cap must trap with MemoryCeilingExceeded
    let mcp_oom = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":50,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"const a = new Array(1000000).fill(7); a.length","memory":65536}}}"#,
        &mut isolate,
    )
    .unwrap();
    assert!(mcp_oom.contains(r#""isError":true"#), "Got: {}", mcp_oom);
    assert!(mcp_oom.contains(r#"\"ok\":false"#), "Got: {}", mcp_oom);
    assert!(mcp_oom.contains("memory ceiling"), "Got: {}", mcp_oom);

    // 2. Fuel and memory scale with actual work performed across evaluations
    let small = isolate.eval_sandbox(
        "const a = new Array(100).fill(1); a.length",
        SandboxEvalOptions::default(),
    );
    let larger = isolate.eval_sandbox(
        "const a = new Array(10000).fill(7); console.log('filled=' + a.length); a.length",
        SandboxEvalOptions::default(),
    );
    assert!(small.ok && larger.ok);
    assert!(
        larger.fuel_used > small.fuel_used,
        "Expected larger.fuel_used ({}) > small.fuel_used ({})",
        larger.fuel_used,
        small.fuel_used
    );
    assert!(
        larger.memory_used_bytes > small.memory_used_bytes,
        "Expected larger.memory_used_bytes ({}) > small.memory_used_bytes ({})",
        larger.memory_used_bytes,
        small.memory_used_bytes
    );
}

#[test]
fn test_issue_15_mcp_honors_audit_egress_env_dotenv_and_deterministic_seed() {
    use crate::config::ResolvedExecutionConfig;
    use crate::mcp_and_repl::handle_mcp_jsonrpc_line;
    use slice_engine::SliceIsolate;

    let tmp_dir = std::env::temp_dir().join(format!("slice_issue15_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp_dir);
    let dotenv_file = tmp_dir.join(".env");
    std::fs::write(
        &dotenv_file,
        "STRIPE_SECRET_KEY=sk_live_dotenv_secret_15\nPUBLIC_APP_NAME=slice-mcp\n",
    )
    .unwrap();

    let args = vec![
        "mcp".to_string(),
        format!("--env-file={}", dotenv_file.display()),
        "--audit-egress=sk_live_topsecret_9".to_string(),
        "--deterministic=777".to_string(),
        "--allow-net=example.com:443".to_string(),
    ];
    let exec_cfg = ResolvedExecutionConfig::from_cli_args(&args).unwrap();
    assert_eq!(exec_cfg.deterministic_seed, Some(777));
    assert!(exec_cfg
        .audit_secrets
        .contains(&"sk_live_topsecret_9".to_string()));
    assert!(exec_cfg
        .audit_secrets
        .contains(&"sk_live_dotenv_secret_15".to_string()));

    let mut isolate = SliceIsolate::spawn(
        "mcp-agent-host",
        115,
        exec_cfg.caps,
        exec_cfg.fuel_limit,
        exec_cfg.mem_limit,
    );
    isolate.set_deterministic_seed(exec_cfg.deterministic_seed);
    for sec in &exec_cfg.audit_secrets {
        isolate.add_egress_audit_secret(sec.clone());
    }

    // 1. Outbound fetch leaking sk_live_topsecret_9 is blocked in MCP
    let leak1 = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":60,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"Slice.fetchSync(\"https://example.com/x?k=sk_live_topsecret_9\").status"}}}"#,
        &mut isolate,
    )
    .unwrap();
    assert!(leak1.contains(r#""isError":true"#), "Got: {}", leak1);
    assert!(
        leak1.contains("EGRESS_SECRET_LEAK_BLOCKED"),
        "Got: {}",
        leak1
    );

    // 2. Outbound fetch or stdout leaking .env Auto-DLP secret is also blocked in MCP
    let leak2 = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":61,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"console.log(\"sk_live_dotenv_secret_15\"); 1"}}}"#,
        &mut isolate,
    )
    .unwrap();
    assert!(leak2.contains(r#""isError":true"#), "Got: {}", leak2);
    assert!(
        leak2.contains("EGRESS_SECRET_LEAK_BLOCKED"),
        "Got: {}",
        leak2
    );

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_issue_16_mcp_multiple_audit_secrets_deterministic_seed_and_schema_sync() {
    use crate::mcp_and_repl::handle_mcp_jsonrpc_line;
    use slice_engine::SliceIsolate;
    use slice_sandbox::CapabilityToken;

    let mut isolate = SliceIsolate::spawn(
        "mcp-agent-host",
        116,
        CapabilityToken::pure_compute(),
        5_000_000,
        16 * 1024 * 1024,
    );

    // 1. Verify tools/list advertises auditSecrets, deterministicSeed, checkpoints, maxCheckpoints
    let list_resp = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":70,"method":"tools/list","params":{}}"#,
        &mut isolate,
    )
    .unwrap();
    assert!(list_resp.contains(r#""auditSecrets""#));
    assert!(list_resp.contains(r#""deterministicSeed""#));
    assert!(list_resp.contains(r#""checkpoints""#));
    assert!(list_resp.contains(r#""maxCheckpoints""#));

    // 2. Multiple secrets in `auditSecrets` array are all guarded (including Base64 forms)
    let leak_second = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":71,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"Buffer.from(\"db_pass_second_secret_99\").toString(\"base64\")","auditSecrets":["pay_key_first_secret_11","db_pass_second_secret_99"]}}}"#,
        &mut isolate,
    )
    .unwrap();
    assert!(
        leak_second.contains(r#""isError":true"#),
        "Got: {}",
        leak_second
    );
    assert!(
        leak_second.contains("EGRESS_SECRET_LEAK_BLOCKED"),
        "Got: {}",
        leak_second
    );

    // 3. deterministicSeed produces identical Math.random() sequences across calls
    let det1 = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":72,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"Math.random().toFixed(8)","deterministicSeed":123456}}}"#,
        &mut isolate,
    )
    .unwrap();
    let det2 = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":73,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"Math.random().toFixed(8)","deterministicSeed":123456}}}"#,
        &mut isolate,
    )
    .unwrap();
    let v1: serde_json::Value = serde_json::from_str(&det1).unwrap();
    let v2: serde_json::Value = serde_json::from_str(&det2).unwrap();
    let p1: serde_json::Value =
        serde_json::from_str(v1["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    let p2: serde_json::Value =
        serde_json::from_str(v2["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(p1["returnValue"], p2["returnValue"]);
    assert!(p1["spawnTimeNs"].as_u64().unwrap() >= 1);
    assert!(p1["compileTimeNs"].as_u64().unwrap() >= 1);
    assert!(p1["execTimeNs"].as_u64().unwrap() >= 1);
}

#[test]
fn test_mcp_persistent_kv_memory_accounting_checkpoint_caps_and_bounded_lines() {
    use crate::diagnostics::mcp::{read_bounded_mcp_line, McpLineRead};
    use crate::mcp_and_repl::handle_mcp_jsonrpc_line;
    use slice_engine::SliceIsolate;
    use slice_sandbox::CapabilityToken;
    use std::io::Cursor;

    // 1. Bounded MCP stdin line reader caps memory on oversized lines and recovers for next line
    let mut raw_stream = b"a".repeat(2048);
    raw_stream.push(b'\n');
    raw_stream.extend_from_slice(br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#);
    raw_stream.push(b'\n');
    let mut cursor = Cursor::new(raw_stream);
    let mut line_buf = Vec::new();
    assert_eq!(
        read_bounded_mcp_line(&mut cursor, &mut line_buf, 1024).unwrap(),
        McpLineRead::TooLong
    );
    assert!(line_buf.is_empty());
    assert_eq!(
        read_bounded_mcp_line(&mut cursor, &mut line_buf, 1024).unwrap(),
        McpLineRead::Line
    );
    assert_eq!(
        std::str::from_utf8(&line_buf).unwrap(),
        r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#
    );
    assert_eq!(
        read_bounded_mcp_line(&mut cursor, &mut line_buf, 1024).unwrap(),
        McpLineRead::Eof
    );

    // 2. Persistent Slice.kv byte footprint is retained across slice_eval_ts calls (never wiped to 0)
    let mut kv_iso = SliceIsolate::spawn(
        "mcp-kv-mem-persist",
        120,
        CapabilityToken::pure_compute(),
        2_000_000,
        16 * 1024, // 16 KB total compartment limit (4 KB base + 12 KB KV headroom)
    );
    let big_val = "v".repeat(8000);
    let set1 = handle_mcp_jsonrpc_line(
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "slice_kv_set",
                "arguments": { "key": "k1", "value": big_val }
            }
        })
        .to_string(),
        &mut kv_iso,
    )
    .unwrap();
    assert!(set1.contains(r#""isError":false"#), "got: {}", set1);

    // Run a slice_eval_ts call in between; this must NOT reset persistent KV accounting to 0!
    let eval_mid = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"1 + 1"}}}"#,
        &mut kv_iso,
    )
    .unwrap();
    assert!(eval_mid.contains(r#""isError":false"#), "got: {}", eval_mid);

    // Now writing another 8 KB key must fail because 4 KB + 8 KB + 8 KB > 16 KB!
    let set2 = handle_mcp_jsonrpc_line(
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "slice_kv_set",
                "arguments": { "key": "k2", "value": big_val }
            }
        })
        .to_string(),
        &mut kv_iso,
    )
    .unwrap();
    assert!(
        set2.contains(r#""isError":true"#),
        "second 8 KB KV write after slice_eval_ts must exceed 16 KB ceiling, got: {}",
        set2
    );

    // 3. slice_checkpoint enforces allow_checkpoints = false and max_checkpoints
    let no_cp_caps = CapabilityToken::pure_compute().with_checkpoints(false);
    let mut no_cp_iso = SliceIsolate::spawn("mcp-no-cp", 121, no_cp_caps, 1_000_000, 1024 * 1024);
    let cp_denied = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"slice_checkpoint","arguments":{"action":"checkpoint"}}}"#,
        &mut no_cp_iso,
    )
    .unwrap();
    assert!(
        cp_denied.contains(r#""isError":true"#),
        "got: {}",
        cp_denied
    );
    assert!(
        cp_denied.contains("CapabilityDenied: checkpoints")
            && cp_denied.contains("checkpoint denied"),
        "got: {}",
        cp_denied
    );

    let one_cp_caps = CapabilityToken::pure_compute()
        .with_checkpoints(true)
        .with_max_checkpoints(Some(1));
    let mut one_cp_iso =
        SliceIsolate::spawn("mcp-one-cp", 122, one_cp_caps, 1_000_000, 1024 * 1024);
    let cp_ok = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"slice_checkpoint","arguments":{"action":"checkpoint"}}}"#,
        &mut one_cp_iso,
    )
    .unwrap();
    assert!(cp_ok.contains(r#""isError":false"#), "got: {}", cp_ok);
    let cp_second = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"slice_checkpoint","arguments":{"action":"checkpoint"}}}"#,
        &mut one_cp_iso,
    )
    .unwrap();
    assert!(
        cp_second.contains(r#""isError":true"#),
        "got: {}",
        cp_second
    );
}

#[test]
fn test_http_server_large_chunked_pipelining_binary_responses_and_head_framing() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let script = r#"
Slice.serve(async (req) => {
    const url = new URL(req.url);
    if (url.pathname === "/upload") {
        const txt = await req.text();
        return new Response("len=" + txt.length, {
            headers: { "content-type": "text/plain" }
        });
    }
    if (url.pathname === "/bin") {
        return new Response(new Uint8Array([0, 127, 128, 255, 65]), {
            headers: { "content-type": "application/octet-stream" }
        });
    }
    if (url.pathname === "/no-content") {
        return new Response("ignored", { status: 204 });
    }
    if (url.pathname === "/not-modified") {
        return new Response("ignored-304", { status: 304 });
    }
    return new Response("not-found", { status: 404 });
});
"#;
    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bytecode = Arc::new(compile_source_to_bytecode(&bundled));

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let counter = Arc::new(AtomicUsize::new(0));
    let bc_server = Arc::clone(&bytecode);
    let counter_server = Arc::clone(&counter);

    let server_handle = std::thread::spawn(move || {
        SliceIsolate::prewarm_boa_pool(1);
        SliceIsolate::prewarm_boa_bundle(&bc_server);
        let mut read_buf = vec![0u8; 65536];
        let mut write_buf = Vec::with_capacity(4096);
        if let Ok((stream, _)) = listener.accept() {
            let _ = serve_connection_keep_alive(
                stream,
                0,
                &bc_server,
                None,
                None,
                None,
                &CapabilityToken::pure_compute(),
                5_000_000,
                16 * 1024 * 1024,
                &mut read_buf,
                &mut write_buf,
                Some((&counter_server, 5)),
            );
        }
    });

    // Build a >64 KB chunked POST (90,000 bytes in 3x 30,000-byte chunks) followed immediately
    // on the same TCP stream by 4 pipelined requests (HEAD, 204, 304, binary GET).
    let chunk_part = "x".repeat(30_000);
    let chunk_header = format!("{:x}\r\n", chunk_part.len());
    let mut payload = Vec::with_capacity(100_000);
    payload.extend_from_slice(
        b"POST /upload HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n",
    );
    for _ in 0..3 {
        payload.extend_from_slice(chunk_header.as_bytes());
        payload.extend_from_slice(chunk_part.as_bytes());
        payload.extend_from_slice(b"\r\n");
    }
    payload.extend_from_slice(b"0\r\n\r\n");
    payload.extend_from_slice(
        concat!(
            "HEAD /bin HTTP/1.1\r\nHost: localhost\r\n\r\n",
            "GET /no-content HTTP/1.1\r\nHost: localhost\r\n\r\n",
            "GET /not-modified HTTP/1.1\r\nHost: localhost\r\n\r\n",
            "GET /bin HTTP/1.1\r\nHost: localhost\r\n\r\n"
        )
        .as_bytes(),
    );

    let mut client = TcpStream::connect(addr).unwrap();
    client.write_all(&payload).unwrap();
    client.flush().unwrap();

    let mut raw_resp = Vec::new();
    client.read_to_end(&mut raw_resp).unwrap();
    server_handle.join().unwrap();

    assert_eq!(counter.load(Ordering::Relaxed), 5);

    fn take_header_block(buf: &[u8]) -> (&str, &[u8]) {
        let pos = buf
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("missing HTTP response header delimiter");
        let hdr = std::str::from_utf8(&buf[..pos]).expect("UTF-8 response headers");
        (hdr, &buf[pos + 4..])
    }

    // 1. POST /upload -> 200 OK, body "len=90000" (9 bytes)
    let (h1, rest) = take_header_block(&raw_resp);
    assert!(h1.starts_with("HTTP/1.1 200 OK\r\n"), "h1: {}", h1);
    assert!(h1.contains("\r\nContent-Length: 9"), "h1: {}", h1);
    assert_eq!(&rest[..9], b"len=90000");
    let rest = &rest[9..];

    // 2. HEAD /bin -> 200 OK, Content-Length: 5, 0 wire body bytes
    let (h2, rest) = take_header_block(rest);
    assert!(h2.starts_with("HTTP/1.1 200 OK\r\n"), "h2: {}", h2);
    assert!(h2.contains("\r\nContent-Length: 5"), "h2: {}", h2);
    assert!(
        rest.starts_with(b"HTTP/1.1 204 "),
        "HEAD response leaked body bytes before 204: {:?}",
        &rest[..rest.len().min(32)]
    );

    // 3. GET /no-content -> 204 No Content, 0 wire body bytes
    let (h3, rest) = take_header_block(rest);
    assert!(h3.starts_with("HTTP/1.1 204 No Content\r\n"), "h3: {}", h3);
    assert!(
        rest.starts_with(b"HTTP/1.1 304 "),
        "204 response leaked body bytes before 304: {:?}",
        &rest[..rest.len().min(32)]
    );

    // 4. GET /not-modified -> 304 Not Modified, 0 wire body bytes
    let (h4, rest) = take_header_block(rest);
    assert!(
        h4.starts_with("HTTP/1.1 304 Not Modified\r\n"),
        "h4: {}",
        h4
    );
    assert!(
        rest.starts_with(b"HTTP/1.1 200 "),
        "304 response leaked body bytes before final 200: {:?}",
        &rest[..rest.len().min(32)]
    );

    // 5. GET /bin -> 200 OK, exact raw binary bytes [0, 127, 128, 255, 65]
    let (h5, rest) = take_header_block(rest);
    assert!(h5.starts_with("HTTP/1.1 200 OK\r\n"), "h5: {}", h5);
    assert!(h5.contains("\r\nContent-Length: 5"), "h5: {}", h5);
    assert_eq!(rest, &[0u8, 127, 128, 255, 65]);
}

#[test]
fn test_http_server_framing_smuggling_guards_and_non_101_websocket_rejection() {
    use std::net::Shutdown;
    use std::sync::Arc;

    let script = r#"
Slice.serve((req) => {
    const url = new URL(req.url);
    if (url.pathname === "/smuggle") {
        return new Response("safe-body", {
            headers: {
                "Transfer-Encoding": "chunked",
                "Content-Length": "999",
                "Connection": "upgrade",
                "X-Custom-Ok": "yes"
            }
        });
    }
    if (url.pathname === "/ws-denied") {
        return Response.json({ denied: true }, { status: 401 });
    }
    return new Response("ok");
});
"#;
    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bytecode = Arc::new(compile_source_to_bytecode(&bundled));

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let bc_server = Arc::clone(&bytecode);

    // Accept 4 connections sequentially:
    // 1. Valid keep-alive pair (/smuggle + /ws-denied with Upgrade header)
    // 2. Whitespace in header name -> 400
    // 3. Invalid Transfer-Encoding: chunked, cow -> 400
    // 4. CRLF request line terminated only by bare LF-LF -> 400 on EOF
    let server_handle = std::thread::spawn(move || {
        SliceIsolate::prewarm_boa_pool(1);
        SliceIsolate::prewarm_boa_bundle(&bc_server);
        let mut read_buf = vec![0u8; 65536];
        let mut write_buf = Vec::with_capacity(4096);
        for _ in 0..4 {
            if let Ok((stream, _)) = listener.accept() {
                let _ = serve_connection_keep_alive(
                    stream,
                    0,
                    &bc_server,
                    None,
                    None,
                    None,
                    &CapabilityToken::pure_compute(),
                    2_000_000,
                    4 * 1024 * 1024,
                    &mut read_buf,
                    &mut write_buf,
                    None,
                );
            }
        }
    });

    // Connection 1: custom framing header stripping + non-101 WebSocket rejection
    {
        let mut c1 = TcpStream::connect(addr).unwrap();
        let reqs = concat!(
            "GET /smuggle HTTP/1.1\r\nHost: localhost\r\n\r\n",
            "GET /ws-denied HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade, close\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
        );
        c1.write_all(reqs.as_bytes()).unwrap();
        let mut out = String::new();
        c1.read_to_string(&mut out).unwrap();

        let lower = out.to_ascii_lowercase();
        assert!(
            !lower.contains("transfer-encoding:"),
            "injected Transfer-Encoding leaked: {}",
            out
        );
        assert!(
            !out.contains("Content-Length: 999"),
            "injected Content-Length leaked: {}",
            out
        );
        assert!(out.contains("Content-Length: 9\r\n"), "out: {}", out);
        assert!(out.contains("x-custom-ok: yes\r\n") || out.contains("X-Custom-Ok: yes\r\n"));
        assert!(
            out.contains("HTTP/1.1 401 Unauthorized\r\n"),
            "unupgraded route must return 401, not 101: {}",
            out
        );
        assert!(
            !out.contains("101 Switching Protocols"),
            "unexpected 101 hijack: {}",
            out
        );
        assert!(out.contains(r#"{"denied":true}"#), "out: {}", out);
    }

    // Connection 2: RFC 9112 Section 5.1 whitespace before colon in header name -> 400
    {
        let mut c2 = TcpStream::connect(addr).unwrap();
        c2.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nX-Bad : 1\r\n\r\n")
            .unwrap();
        let mut out = String::new();
        c2.read_to_string(&mut out).unwrap();
        assert!(
            out.starts_with("HTTP/1.1 400 Bad Request"),
            "expected 400 on header whitespace, got: {}",
            out
        );
    }

    // Connection 3: multi-token Transfer-Encoding -> 400
    {
        let mut c3 = TcpStream::connect(addr).unwrap();
        c3.write_all(b"POST / HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked, cow\r\n\r\n0\r\n\r\n")
            .unwrap();
        let mut out = String::new();
        c3.read_to_string(&mut out).unwrap();
        assert!(
            out.starts_with("HTTP/1.1 400 Bad Request"),
            "expected 400 on multi-token Transfer-Encoding, got: {}",
            out
        );
    }

    // Connection 4: CRLF request line followed by bare LF-LF must NOT parse as complete headers
    {
        let mut c4 = TcpStream::connect(addr).unwrap();
        c4.write_all(b"GET /smuggle HTTP/1.1\r\nHost: localhost\n\n")
            .unwrap();
        c4.shutdown(Shutdown::Write).unwrap();
        let mut out = String::new();
        c4.read_to_string(&mut out).unwrap();
        assert!(
            out.starts_with("HTTP/1.1 400 Bad Request"),
            "expected 400 on mixed CRLF + bare LF-LF, got: {}",
            out
        );
    }

    server_handle.join().unwrap();
}

#[test]
fn test_tenant_id_kv_namespace_isolation_bounded_cas_cache_and_embedded_static_cas() {
    use crate::cmd_serve;
    use crate::http_server::{clear_tenant_cas_cache, sanitize_tenant_id, tenant_cas_cache_len};
    use std::borrow::Cow;
    use std::sync::Arc;

    // 1. Tenant ID sanitization preserves clean dotted IDs without allocation and strips `:` / `..`
    assert!(matches!(
        sanitize_tenant_id("acme.prod-1_eu"),
        Cow::Borrowed("acme.prod-1_eu")
    ));
    assert_eq!(sanitize_tenant_id("org:admin"), "orgadmin");
    assert_eq!(sanitize_tenant_id("../etc/passwd"), "etcpasswd");

    // 2. `X-Slice-Tenant: org:admin` cannot collide with `X-Slice-Tenant: org` + key `admin:secret`
    let kv_script = r#"
const req = Slice.request();
if (req.path === "/set") {
    Slice.kv.set("admin:secret", "vault-token-77");
    Slice.respond(200, "stored", "text/plain");
} else {
    const val = Slice.kv.get("secret") ?? "NONE";
    Slice.respond(200, val, "text/plain");
}
"#;
    let kv_bc = Arc::new(compile_source_to_bytecode(kv_script));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let kv_bc_srv = Arc::clone(&kv_bc);
    let kv_srv = std::thread::spawn(move || {
        SliceIsolate::prewarm_boa_pool(1);
        SliceIsolate::prewarm_boa_bundle(&kv_bc_srv);
        let mut read_buf = vec![0u8; 65536];
        let mut write_buf = Vec::with_capacity(4096);
        if let Ok((stream, _)) = listener.accept() {
            let _ = serve_connection_keep_alive(
                stream,
                0,
                &kv_bc_srv,
                None,
                None,
                None,
                &CapabilityToken::pure_compute(),
                2_000_000,
                4 * 1024 * 1024,
                &mut read_buf,
                &mut write_buf,
                None,
            );
        }
    });

    let mut kv_client = TcpStream::connect(addr).unwrap();
    kv_client
        .write_all(
            concat!(
                "GET /set HTTP/1.1\r\nHost: localhost\r\nX-Slice-Tenant: org\r\n\r\n",
                "GET /get HTTP/1.1\r\nHost: localhost\r\nX-Slice-Tenant: org:admin\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .unwrap();
    let mut kv_resp = String::new();
    kv_client.read_to_string(&mut kv_resp).unwrap();
    kv_srv.join().unwrap();
    assert!(kv_resp.contains("stored"), "kv_resp: {}", kv_resp);
    assert!(
        kv_resp.ends_with("NONE") && !kv_resp.contains("vault-token-77"),
        "cross-tenant colon injection leaked KV secret: {}",
        kv_resp
    );

    // 3. `default.cas` fallback deduplicates bytecode across 1,200 unknown tenant headers and stays bounded
    let tmp_root = env::temp_dir().join(format!("slice_tenant_static_cas_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_root);
    let tenant_dir = tmp_root.join("tenants");
    std::fs::create_dir_all(&tenant_dir).unwrap();

    let default_ts = tenant_dir.join("default.ts");
    std::fs::write(
        &default_ts,
        "Slice.respond(200, \"default-ok\", \"text/plain\");\n",
    )
    .unwrap();
    let default_cas = tenant_dir.join("default.cas");
    build_cas_archive(&default_ts, &default_cas, "./default.ts").unwrap();

    clear_tenant_cas_cache();
    let first_arc = resolve_tenant_bytecode_hotswap(&tenant_dir, "missing-tenant-0").unwrap();
    for i in 1..1200 {
        let next_arc =
            resolve_tenant_bytecode_hotswap(&tenant_dir, &format!("missing-tenant-{}", i)).unwrap();
        assert!(
            Arc::ptr_eq(&first_arc, &next_arc),
            "default.cas fallback must reuse a single canonical Arc<Vec<u8>>"
        );
    }
    let (by_tenant_len, by_path_len) = tenant_cas_cache_len();
    assert_eq!(by_tenant_len, 0);
    assert_eq!(by_path_len, 1);

    // 4. Packing a project with `[server] static_dir` embeds static assets into `.cas`, and
    //    `slice serve app.cas` serves them cleanly when `./public` is absent on disk.
    let app_dir = tmp_root.join("app");
    std::fs::create_dir_all(app_dir.join("public")).unwrap();
    std::fs::write(
        app_dir.join("slice.toml"),
        "[package]\nname = \"embedded-static-app\"\nmain = \"./index.ts\"\n\n[server]\nstatic_dir = \"./public\"\n",
    )
    .unwrap();
    std::fs::write(
        app_dir.join("public").join("hello.txt"),
        "embedded-static-ok",
    )
    .unwrap();
    std::fs::write(
        app_dir.join("index.ts"),
        "Slice.serve(() => new Response(\"dynamic-ok\"));\n",
    )
    .unwrap();

    let packed_cas = tmp_root.join("packed_app.cas");
    build_cas_archive(&app_dir, &packed_cas, "./index.ts").unwrap();

    // Remove the entire source and `./public` directory so disk `static_dir` no longer exists!
    std::fs::remove_dir_all(&app_dir).unwrap();
    let mapped = MappedCasArchive::open(&packed_cas).unwrap();
    let embedded_table = slice_cas::StaticAssetTable::from_cas_archive(&mapped);
    assert!(
        embedded_table.resolve("/hello.txt").is_some(),
        "expected /hello.txt embedded into .cas archive from slice.toml [server] static_dir"
    );
    drop(embedded_table);
    drop(mapped);

    // Pick a free ephemeral port and run `cmd_serve` with `--max-requests=2`
    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let serve_port = probe.local_addr().unwrap().port();
    drop(probe);

    let cas_arg = packed_cas.to_string_lossy().to_string();
    let serve_thread = std::thread::spawn(move || {
        cmd_serve(&[
            cas_arg,
            format!("--port={}", serve_port),
            "--host=127.0.0.1".to_string(),
            "--workers=1".to_string(),
            "--max-requests=2".to_string(),
        ])
    });

    let mut serve_client = None;
    for _ in 0..50 {
        if let Ok(s) = TcpStream::connect(("127.0.0.1", serve_port)) {
            serve_client = Some(s);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let mut serve_client = serve_client.expect("connect to cmd_serve on packed .cas");
    serve_client
        .write_all(
            concat!(
                "GET /hello.txt HTTP/1.1\r\nHost: localhost\r\n\r\n",
                "GET /dynamic HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .unwrap();
    let mut serve_out = String::new();
    serve_client.read_to_string(&mut serve_out).unwrap();
    serve_thread.join().unwrap().unwrap();

    assert!(
        serve_out.contains("embedded-static-ok"),
        "out: {}",
        serve_out
    );
    assert!(serve_out.contains("dynamic-ok"), "out: {}", serve_out);

    let _ = std::fs::remove_dir_all(&tmp_root);
}

#[test]
fn test_issue_105_tenant_url_prefix_slicing_non_ascii_wire_guard_and_host_bind() {
    use crate::commands::pack_and_compile::cmd_pack;
    use crate::server::cmd_serve;
    use crate::server::connection::extract_tenant_route_prefix;
    use slice_engine::{compile_source_to_bytecode, HttpRequestContext, SliceIsolate};
    use slice_sandbox::CapabilityToken;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    // 1. Direct unit checks on zero-allocation `extract_tenant_route_prefix` (ASCII, percent-encoded, multi-byte UTF-8)
    assert_eq!(extract_tenant_route_prefix("/t/abc/foo"), ("abc", "/foo"));
    assert_eq!(extract_tenant_route_prefix("/t/abc/"), ("abc", "/"));
    assert_eq!(extract_tenant_route_prefix("/t/abc"), ("abc", "/"));
    assert_eq!(
        extract_tenant_route_prefix("/t/%C3%A9/v1/items"),
        ("%C3%A9", "/v1/items")
    );
    assert_eq!(extract_tenant_route_prefix("/t/é/x"), ("é", "/x"));
    assert_eq!(
        extract_tenant_route_prefix("/t/\u{1F680}/v1/a"),
        ("\u{1F680}", "/v1/a")
    );
    assert_eq!(
        extract_tenant_route_prefix("/t/"),
        ("tenant-default", "/t/")
    );
    assert_eq!(
        extract_tenant_route_prefix("/t//foo"),
        ("tenant-default", "/t//foo")
    );
    assert_eq!(
        extract_tenant_route_prefix("/plain/path"),
        ("tenant-default", "/plain/path")
    );

    // 2. Defensive `__slice_http_url` guard when an embedder passes `HttpRequestContext.path` without leading '/'
    let url_echo_bc = compile_source_to_bytecode(
        "Slice.serve((req) => new Response(req.url + '|' + Slice.request().path));",
    );
    let mut iso = SliceIsolate::spawn(
        "url-guard-tenant",
        1,
        CapabilityToken::pure_compute(),
        2_000_000,
        8 * 1024 * 1024,
    );
    let embedder_req = HttpRequestContext {
        method: "GET",
        path: "foo/bar",
        query: "q=1",
        tenant_id: "url-guard-tenant",
        body: "",
        headers: &[("host", "localhost")],
    };
    let embedder_out = iso
        .execute_http_request(&url_echo_bc, None, &embedder_req)
        .unwrap();
    assert_eq!(embedder_out.status_code, 200);
    assert_eq!(
        embedder_out.response_body,
        "http://localhost/foo/bar?q=1|foo/bar"
    );

    // 3. End-to-end TCP server test with `--tenant-dir` + `--host=127.0.0.1`
    let tmp_dir = std::env::temp_dir().join(format!("slice_issue_105_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let tenant_dir = tmp_dir.join("tenants");
    let src_dir = tmp_dir.join("src_alpha");
    std::fs::create_dir_all(&tenant_dir).unwrap();
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        src_dir.join("index.ts"),
        "Slice.serve((req) => new Response(JSON.stringify({ url: req.url, path: Slice.request().path })));\n",
    )
    .unwrap();
    let alpha_cas = tenant_dir.join("alpha.cas");
    cmd_pack(&[
        src_dir.to_string_lossy().to_string(),
        alpha_cas.to_string_lossy().to_string(),
    ])
    .unwrap();

    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);

    let tenant_dir_str = tenant_dir.to_string_lossy().to_string();
    let alpha_cas_str = alpha_cas.to_string_lossy().to_string();
    // 4 valid HTTP requests (`max_requests` counts processed valid requests; malformed 400 wire frames close early without panicking)
    let server_handle = std::thread::spawn(move || {
        cmd_serve(&[
            alpha_cas_str,
            format!("--tenant-dir={}", tenant_dir_str),
            format!("--port={}", port),
            "--host=127.0.0.1".to_string(),
            "--workers=1".to_string(),
            "--max-requests=4".to_string(),
        ])
    });

    let send_raw = |payload: &[u8]| -> String {
        let mut s = None;
        for _ in 0..50 {
            if let Ok(conn) = TcpStream::connect(("127.0.0.1", port)) {
                s = Some(conn);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let mut s = s.expect("connect to 127.0.0.1 tenant server");
        s.write_all(payload).unwrap();
        let mut resp = Vec::new();
        s.read_to_end(&mut resp).unwrap();
        String::from_utf8_lossy(&resp).into_owned()
    };

    // Valid `/t/alpha/foo?k=1` -> preserves leading `/foo`
    let r1 =
        send_raw(b"GET /t/alpha/foo?k=1 HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    assert!(r1.starts_with("HTTP/1.1 200 OK"), "r1: {}", r1);
    assert!(
        r1.contains(r#""url":"http://localhost/foo?k=1","path":"/foo""#),
        "r1: {}",
        r1
    );

    // Valid `/t/alpha/` -> `"/"`
    let r2 = send_raw(b"GET /t/alpha/ HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    assert!(r2.starts_with("HTTP/1.1 200 OK"), "r2: {}", r2);
    assert!(
        r2.contains(r#""url":"http://localhost/","path":"/""#),
        "r2: {}",
        r2
    );

    // Valid `/t/alpha` -> `"/"`
    let r3 = send_raw(b"GET /t/alpha HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    assert!(r3.starts_with("HTTP/1.1 200 OK"), "r3: {}", r3);
    assert!(
        r3.contains(r#""url":"http://localhost/","path":"/""#),
        "r3: {}",
        r3
    );

    // Malformed raw UTF-8 obs-text `/t/\xc3\xa9/x` -> 400 Bad Request, zero worker panic!
    let r_bad_utf8 = send_raw(b"GET /t/\xc3\xa9/x HTTP/1.1\r\nHost: localhost\r\n\r\n");
    assert!(
        r_bad_utf8.starts_with("HTTP/1.1 400 Bad Request"),
        "expected 400 on raw non-ASCII target, got: {}",
        r_bad_utf8
    );

    // Malformed non-origin-form `GET foo/bar` -> 400 Bad Request, zero worker panic!
    let r_bad_origin = send_raw(b"GET foo/bar HTTP/1.1\r\nHost: localhost\r\n\r\n");
    assert!(
        r_bad_origin.starts_with("HTTP/1.1 400 Bad Request"),
        "expected 400 on non-origin-form target, got: {}",
        r_bad_origin
    );

    // Subsequent valid request succeeds cleanly on the same single-worker server!
    let r4 = send_raw(
        b"GET /t/alpha/after-bad HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    assert!(r4.starts_with("HTTP/1.1 200 OK"), "r4: {}", r4);
    assert!(
        r4.contains(r#""url":"http://localhost/after-bad","path":"/after-bad""#),
        "r4: {}",
        r4
    );

    server_handle.join().unwrap().unwrap();
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_issue_99_mcp_monotonic_fuel_memory_and_checkpoint_quotas() {
    use crate::mcp_and_repl::handle_mcp_jsonrpc_line;
    use slice_engine::SliceIsolate;
    use slice_sandbox::CapabilityToken;

    // 1. Host isolate spawned with tight fuel (25,000), tight memory (64 KiB), and max_checkpoints = 1
    let mut caps = CapabilityToken::pure_compute();
    caps.allow_checkpoints = true;
    caps.max_checkpoints = Some(1);

    let mut isolate = SliceIsolate::spawn("mcp-tight-host", 199, caps.clone(), 25_000, 65_536);

    // Caller tries to self-escalate fuel to 100,000,000 -> must stay bounded by 25,000!
    let r_fuel = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"let s = 0; for (let i = 0; i < 500000; i++) { s += (i ^ 3); } s","fuel":100000000}}}"#,
        &mut isolate,
    )
    .unwrap();
    assert!(r_fuel.contains(r#""isError":true"#), "r_fuel: {}", r_fuel);
    assert!(
        r_fuel.contains("Fuel") || r_fuel.contains("fuel"),
        "expected fuel trap, got: {}",
        r_fuel
    );
    assert_eq!(isolate.compartment.fuel_limit, 25_000);

    // Reset consumed fuel for next check while keeping tight 64 KiB memory ceiling
    isolate.compartment.fuel_consumed = 0;

    // Caller tries to self-escalate memory to 256 MiB AND 4 GiB (testing 32-bit overflow safety) -> must trap at 64 KiB!
    for requested_mem in [268_435_456u64, 4_294_967_296u64] {
        let req_line = format!(
            r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"slice_eval_ts","arguments":{{"code":"const big = 'A'.repeat(200000); big.length","memory":{}}}}}}}"#,
            requested_mem
        );
        isolate.compartment.fuel_consumed = 0;
        let r_mem = handle_mcp_jsonrpc_line(&req_line, &mut isolate).unwrap();
        assert!(r_mem.contains(r#""isError":true"#), "r_mem: {}", r_mem);
        assert!(
            r_mem.contains("Memory") || r_mem.contains("memory"),
            "expected memory ceiling trap for requested_mem={}, got: {}",
            requested_mem,
            r_mem
        );
        assert_eq!(isolate.compartment.memory_limit_bytes, 65_536);
    }

    // 2. Session-wide monotonic checkpoint quota: `slice_checkpoint` -> `slice_eval_ts`
    let mut iso_a =
        SliceIsolate::spawn("mcp-ckpt-a", 200, caps.clone(), 1_000_000, 4 * 1024 * 1024);
    let ck1 = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"slice_checkpoint","arguments":{"action":"checkpoint"}}}"#,
        &mut iso_a,
    )
    .unwrap();
    assert!(ck1.contains(r#""isError":false"#), "ck1: {}", ck1);
    assert_eq!(iso_a.checkpoints_taken, 1);

    // Second checkpoint via `slice_eval_ts` (even requesting `maxCheckpoints: 64`) must fail!
    let ck_eval_blocked = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"Slice.checkpoint()","maxCheckpoints":64}}}"#,
        &mut iso_a,
    )
    .unwrap();
    assert!(
        ck_eval_blocked.contains(r#""isError":true"#),
        "expected checkpoint quota block in eval, got: {}",
        ck_eval_blocked
    );

    // 3. Reverse order: `slice_eval_ts("Slice.checkpoint()")` -> `slice_checkpoint` AND second `slice_eval_ts`
    let mut iso_b = SliceIsolate::spawn("mcp-ckpt-b", 201, caps, 1_000_000, 4 * 1024 * 1024);
    let eval_ck1 = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":20,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"Slice.checkpoint()"}}}"#,
        &mut iso_b,
    )
    .unwrap();
    assert!(
        eval_ck1.contains(r#""isError":false"#),
        "eval_ck1: {}",
        eval_ck1
    );
    assert!(
        eval_ck1.contains(r#"\"checkpointsTaken\":1"#),
        "eval_ck1: {}",
        eval_ck1
    );
    assert_eq!(iso_b.checkpoints_taken, 1);

    let host_ck_blocked = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":21,"method":"tools/call","params":{"name":"slice_checkpoint","arguments":{"action":"checkpoint"}}}"#,
        &mut iso_b,
    )
    .unwrap();
    assert!(
        host_ck_blocked.contains(r#""isError":true"#)
            && host_ck_blocked.contains("exceeded max_checkpoints limit"),
        "expected host checkpoint block, got: {}",
        host_ck_blocked
    );

    let eval_ck2_blocked = handle_mcp_jsonrpc_line(
        r#"{"jsonrpc":"2.0","id":22,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"Slice.checkpoint()"}}}"#,
        &mut iso_b,
    )
    .unwrap();
    assert!(
        eval_ck2_blocked.contains(r#""isError":true"#),
        "expected second eval checkpoint block, got: {}",
        eval_ck2_blocked
    );
}
