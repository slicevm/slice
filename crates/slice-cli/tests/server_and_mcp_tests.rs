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

    // 3. Execute `cmd_test` against `tmp_dir` with `--env-file` and `--json`
    let test_args = vec![
        tmp_dir.to_string_lossy().to_string(),
        format!("--env-file={}", env_path.to_string_lossy()),
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
