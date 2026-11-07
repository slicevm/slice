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

