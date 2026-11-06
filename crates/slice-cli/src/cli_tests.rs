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
fn test_phase_n1_wintertc_redirect_keepalive_and_ssrf_mesh_roundtrip() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let bound_port = listener.local_addr().unwrap().port();
    let host_port = format!("127.0.0.1:{}", bound_port);

    let upstream_ts = r#"
export default {
    fetch(req: Request): Response {
        const url = new URL(req.url);
        if (url.pathname === "/upstream/subgraph/account") {
            const accountId = url.searchParams.get("account") || "acct-0";
            return Response.json({
                accountId,
                tier: "enterprise",
                balanceCents: 14500,
                region: "us-east-1"
            });
        }
        if (url.pathname === "/upstream/redirect-307") {
            return Response.redirect("/upstream/subgraph/account?" + url.searchParams.toString(), 307);
        }
        if (url.pathname === "/upstream/redirect-302") {
            const accountId = url.searchParams.get("account") || "acct-0";
            return Response.redirect("https://partner-gateway.example.com/upstream/subgraph/account?account=" + accountId, 302);
        }
        return Response.json({ ok: true });
    }
};
"#;
    let upstream_bundled = slice_cas::strip_typescript_and_normalize(upstream_ts, true);
    let upstream_bc = std::sync::Arc::new(compile_source_to_bytecode(&upstream_bundled));
    let bc_clone = std::sync::Arc::clone(&upstream_bc);

    // Serve ALL 20 requests over a SINGLE accepted Keep-Alive TCP stream!
    let srv_handle = std::thread::spawn(move || {
        SliceIsolate::prewarm_boa_bundle(&bc_clone);
        let mut read_buf = vec![0u8; 65536];
        let mut write_buf = Vec::with_capacity(4096);
        if let Ok((stream, _)) = listener.accept() {
            let _ = serve_connection_keep_alive(
                stream,
                1,
                &bc_clone,
                None,
                None,
                None,
                &CapabilityToken::pure_compute(),
                10_000_000,
                8 * 1024 * 1024,
                &mut read_buf,
                &mut write_buf,
                None,
            );
        }
    });

    // Copy App 17 (`tests/apps/17-zero-trust-redirect-keepalive-mesh`) into a temp dir bound to `bound_port`
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let app17_src = repo_root
        .join("tests")
        .join("apps")
        .join("17-zero-trust-redirect-keepalive-mesh")
        .join("src");
    let tmp_dir = std::env::temp_dir().join("slice_n1_app17_roundtrip");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let dst_src = tmp_dir.join("src");
    std::fs::create_dir_all(&dst_src).unwrap();

    for file_name in [
        "types.ts",
        "redirect_security_guard.ts",
        "keepalive_mesh_client.ts",
        "index.ts",
    ] {
        let content = std::fs::read_to_string(app17_src.join(file_name)).unwrap();
        let patched = content.replace("127.0.0.1:18095", &host_port);
        std::fs::write(dst_src.join(file_name), patched).unwrap();
    }
    std::fs::write(
        tmp_dir.join("package.json"),
        r#"{"name":"17-zero-trust-redirect-keepalive-mesh","version":"1.0.0","main":"./src/index.ts"}"#,
    )
    .unwrap();
    std::fs::write(
        tmp_dir.join("slice.toml"),
        format!(
            "[package]\nname = \"17-zero-trust-redirect-keepalive-mesh\"\nentrypoint = \"./src/index.ts\"\n[capabilities]\nallow_net = [\"{}\"]\n",
            host_port
        ),
    )
    .unwrap();

    // Populate node_modules in `tmp_dir` so CI works without external `bun install`
    let nm = tmp_dir.join("node_modules");
    for (pkg, code) in [
        (
            "bytes",
            "module.exports = { format: function(n) { return n < 1024 ? n + 'B' : (n / 1024).toFixed(2) + 'KB'; } };",
        ),
        (
            "cookie",
            "module.exports = { serialize: function(k, v) { return k + '=' + v + '; Path=/; HttpOnly'; }, parse: function(s) { var o = {}; String(s).split(';').forEach(function(p) { var kv = p.trim().split('='); if (kv.length === 2) o[kv[0]] = kv[1]; }); return o; } };",
        ),
        (
            "fast-querystring",
            "module.exports = { parse: function(q) { var o = {}; String(q).split('&').forEach(function(p) { var kv = p.split('='); if (kv[0]) o[decodeURIComponent(kv[0])] = decodeURIComponent(kv[1] || ''); }); return o; } };",
        ),
        (
            "ms",
            "module.exports = function(s) { var str = String(s); if (str.endsWith('ms')) return Number(str.slice(0, -2)); if (str.endsWith('s')) return Number(str.slice(0, -1)) * 1000; return Number(str); };",
        ),
    ] {
        let pkg_dir = nm.join(pkg);
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("index.js"), code).unwrap();
    }

    let cas_out = tmp_dir.join("app17.cas");
    build_cas_archive_with_static(&tmp_dir, &cas_out, "./src/index.ts", None).unwrap();

    let archive = MappedCasArchive::open(&cas_out).unwrap();
    let mut caps = CapabilityToken::pure_compute();
    if let Some(manifest_toml) = archive.get_manifest_toml() {
        merge_manifest_toml_into_caps(manifest_toml, &mut caps);
    }
    let entry = archive.entrypoint_blob().unwrap();
    slice_engine::clear_outbound_conn_pool();
    let reuse_before = slice_engine::outbound_pool_reuse_count();
    let mut iso = SliceIsolate::spawn("app17-tenant", 1, caps, 50_000_000, 16 * 1024 * 1024);
    let metrics = iso
        .execute_bytecode(archive.get_bytecode_slice(entry), Some(&archive))
        .unwrap();
    let reuse_after = slice_engine::outbound_pool_reuse_count();

    // Drop pooled connection so `srv_handle` sees EOF and exits cleanly
    slice_engine::clear_outbound_conn_pool();
    let _ = srv_handle.join();
    let _ = std::fs::remove_dir_all(&tmp_dir);

    assert_eq!(
        metrics.stdout[0],
        r#"{"app":"17-zero-trust-redirect-keepalive-mesh","libraries":["bytes@3.1.2","cookie@0.6.0","fast-querystring@1.1.2","ms@2.1.3"],"tasksProcessed":30,"keepAliveSubgraphHits":5,"redirectFollow307Completed":5,"redirectManual302Inspected":5,"ssrfRedirectMetadataBlocked":5,"dnsRebindPrivateIpBlocked":5,"dlpExfiltrationBlocked":5,"crossOriginAuthStrippedCount":15,"totalTimeoutBudgetMs":50000,"formattedPayloadVolume":"860B","meshAuditHmac":"90f5a8253a5d07de"}"#
    );
    assert!(
        reuse_after >= reuse_before + 19,
        "Expected at least 19 Keep-Alive socket reuses across 20 HTTP hops, got {}",
        reuse_after - reuse_before
    );
}

#[test]
fn test_phase_n2_wintertc_async_event_loop_fanout_and_time_travel_roundtrip() {
    // 1. Spawn a real WinterTC shard upstream server (`serve_connection_keep_alive`)
    //    that handles concurrent `POST /v1/shards/0..3` fan-out requests.
    let shard_gateway_ts = r#"
export default {
    fetch(req: Request): Response {
        const url = new URL(req.url);
        if (url.pathname.startsWith("/v1/shards/")) {
            const shardId = Number(url.pathname.split("/").pop() || "0");
            return Response.json(
                { shardId, records: (shardId + 1) * 25, status: "synced" },
                { status: 200, headers: { "x-shard-ack": "ack-" + shardId } }
            );
        }
        return new Response("Not Found", { status: 404 });
    }
};
"#;
    let gw_bundled = slice_cas::strip_typescript_and_normalize(shard_gateway_ts, true);
    let gw_bc: std::sync::Arc<[u8]> =
        std::sync::Arc::from(compile_source_to_bytecode(&gw_bundled).into_boxed_slice());

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let host_port = format!("127.0.0.1:{}", port);

    let stop_srv = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_clone = std::sync::Arc::clone(&stop_srv);
    let gw_bc_clone = std::sync::Arc::clone(&gw_bc);
    let srv_handle = std::thread::spawn(move || {
        let _ = listener.set_nonblocking(true);
        let mut conn_threads = Vec::new();
        let mut slot = 1u32;
        while !stop_clone.load(std::sync::atomic::Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    let bc = std::sync::Arc::clone(&gw_bc_clone);
                    let s = slot;
                    slot += 1;
                    conn_threads.push(std::thread::spawn(move || {
                        let mut read_buf = vec![0u8; 65536];
                        let mut write_buf = Vec::with_capacity(65536);
                        let _ = serve_connection_keep_alive(
                            stream,
                            s,
                            &bc,
                            None,
                            None,
                            None,
                            &CapabilityToken::pure_compute(),
                            15_000_000,
                            16 * 1024 * 1024,
                            &mut read_buf,
                            &mut write_buf,
                            None,
                        );
                    }));
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(_) => break,
            }
        }
        for ct in conn_threads {
            let _ = ct.join();
        }
    });

    // 2. Bundle App 18 (`18-async-event-loop-fanout-scheduler`) into a CAS archive
    let app18_src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/apps/18-async-event-loop-fanout-scheduler");
    let tmp_dir = std::env::temp_dir().join(format!("slice_n2_app18_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(tmp_dir.join("src")).unwrap();

    for file in [
        "types.ts",
        "timer_retry_scheduler.ts",
        "concurrent_fanout_client.ts",
        "index.ts",
    ] {
        let content = std::fs::read_to_string(app18_src.join("src").join(file)).unwrap();
        std::fs::write(tmp_dir.join("src").join(file), content).unwrap();
    }
    std::fs::copy(app18_src.join("package.json"), tmp_dir.join("package.json")).unwrap();
    std::fs::write(
        tmp_dir.join("slice.toml"),
        format!(
            "[package]\nname = \"18-async-event-loop-fanout-scheduler\"\nentrypoint = \"./src/index.ts\"\n[capabilities]\nallow_net = [\"api.fanout.slice.local\", \"{}\"]\nallow_env = [\"NODE_ENV\", \"SLICE_FANOUT_UPSTREAM_BASE\"]\n",
            host_port
        ),
    )
    .unwrap();

    let nm = tmp_dir.join("node_modules");
    for (pkg, code) in [
        (
            "bytes",
            "module.exports = { parse: function(s) { var str = String(s).toUpperCase(); if (str.endsWith('KB')) return Number(str.slice(0, -2)) * 1024; return Number(str); }, format: function(n) { return n < 1024 ? n + 'B' : (n / 1024).toFixed(2) + 'KB'; } };",
        ),
        (
            "flatted",
            "module.exports = { stringify: function(v) { return JSON.stringify([v]); }, parse: function(s) { return JSON.parse(s)[0]; } };",
        ),
        (
            "mitt",
            "module.exports = function() { var m = {}; return { on: function(t, f) { (m[t] = m[t] || []).push(f); }, emit: function(t, e) { (m[t] || []).forEach(function(f) { f(e); }); } }; };",
        ),
        (
            "ms",
            "module.exports = function(s) { var str = String(s); if (str.endsWith('ms')) return Number(str.slice(0, -2)); if (str.endsWith('s')) return Number(str.slice(0, -1)) * 1000; return Number(str); };",
        ),
    ] {
        let pkg_dir = nm.join(pkg);
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("index.js"), code).unwrap();
    }

    let cas_out = tmp_dir.join("app18.cas");
    build_cas_archive_with_static(&tmp_dir, &cas_out, "./src/index.ts", None).unwrap();

    let archive = MappedCasArchive::open(&cas_out).unwrap();
    let entry = archive.entrypoint_blob().unwrap();

    // 3A. Run in Deterministic Virtual Time-Warp mode (`mockFetch` + 0ns timer warp)
    let mut caps_det = CapabilityToken::pure_compute();
    if let Some(manifest_toml) = archive.get_manifest_toml() {
        merge_manifest_toml_into_caps(manifest_toml, &mut caps_det);
    }
    slice_engine::reset_virtual_time_warped_ms();
    let mut iso_det = SliceIsolate::spawn(
        "app18-det-tenant",
        1,
        caps_det,
        50_000_000,
        16 * 1024 * 1024,
    );
    iso_det.set_deterministic_seed(Some(20261102));
    let metrics_det = iso_det
        .execute_bytecode(archive.get_bytecode_slice(entry), Some(&archive))
        .unwrap();
    let warped_ms = slice_engine::virtual_time_warped_ms();
    assert!(
        warped_ms >= 10,
        "Expected deterministic virtual time warp >= 10ms, got {}ms",
        warped_ms
    );

    let expected_output = r#"{"app":"18-async-event-loop-fanout-scheduler","libraries":["bytes@3.1.2","flatted@3.3.1","mitt@3.0.1","ms@2.1.3"],"scheduleOrder":"promise-micro-1>queue-micro-1>timer-stage-a:p1>micro-inside-timer>heartbeat:1>heartbeat:2>heartbeat:3","intervalTicks":3,"cancelledTimerFired":false,"totalRetryBackoffMs":50,"totalShardsExecuted":25,"totalRecordsProcessed":1250,"totalAbortedTasks":5,"totalRetryAttempts":30,"busEventsDispatched":5,"timerBombContained":true,"formattedWireVolume":"1.92KB","auditDigest":"0b76cd366dc934f2"}"#;
    assert_eq!(metrics_det.stdout[0], expected_output);

    // 3B. Run in Live Concurrent Socket Fan-Out mode against the 4-worker WinterTC server
    let reuse_before = slice_engine::outbound_pool_reuse_count();
    let mut caps_live = CapabilityToken::pure_compute().with_env(
        "SLICE_FANOUT_UPSTREAM_BASE",
        format!("http://{}", host_port),
    );
    if let Some(manifest_toml) = archive.get_manifest_toml() {
        merge_manifest_toml_into_caps(manifest_toml, &mut caps_live);
    }
    let mut iso_live = SliceIsolate::spawn(
        "app18-live-tenant",
        2,
        caps_live,
        50_000_000,
        16 * 1024 * 1024,
    );
    let metrics_live = iso_live
        .execute_bytecode(archive.get_bytecode_slice(entry), Some(&archive))
        .unwrap();
    let reuse_after = slice_engine::outbound_pool_reuse_count();

    assert_eq!(metrics_live.stdout[0], expected_output);
    assert!(
        reuse_after >= reuse_before + 15,
        "Expected >= 15 Keep-Alive socket reuses across 5 rounds of 4-shard concurrent fan-out, got {}",
        reuse_after - reuse_before
    );

    stop_srv.store(true, std::sync::atomic::Ordering::Relaxed);
    slice_engine::clear_outbound_conn_pool();
    let _ = srv_handle.join();
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_phase_n3_wintertc_webcrypto_streams_cloning_and_http_roundtrip() {
    let tmp_dir = std::env::temp_dir().join("slice_phase_n3_wintertc_test");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let src_dir = tmp_dir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();

    let manifest_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/apps/19-wintertc-webcrypto-streams-edge");
    std::fs::copy(
        manifest_src.join("package.json"),
        tmp_dir.join("package.json"),
    )
    .unwrap();
    std::fs::copy(manifest_src.join("slice.toml"), tmp_dir.join("slice.toml")).unwrap();
    for f in [
        "types.ts",
        "webcrypto_jwt_verifier.ts",
        "streams_and_formdata_pipeline.ts",
        "index.ts",
    ] {
        std::fs::copy(manifest_src.join("src").join(f), src_dir.join(f)).unwrap();
    }

    // Stub minimal CJS modules for cookie, ms, bytes, and flatted in node_modules
    let nm = tmp_dir.join("node_modules");
    std::fs::create_dir_all(nm.join("ms")).unwrap();
    std::fs::write(
        nm.join("ms/package.json"),
        r#"{"name":"ms","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        nm.join("ms/index.js"),
        r#"module.exports = function(v, o) { return __slice_fast_ms(v, Boolean(o && o.long)); };"#,
    )
    .unwrap();

    std::fs::create_dir_all(nm.join("bytes")).unwrap();
    std::fs::write(
        nm.join("bytes/package.json"),
        r#"{"name":"bytes","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        nm.join("bytes/index.js"),
        r#"module.exports = { parse: function(v){ return __slice_fast_bytes_parse(v); }, format: function(v){ return __slice_fast_bytes_format(v); } };"#,
    )
    .unwrap();

    std::fs::create_dir_all(nm.join("cookie")).unwrap();
    std::fs::write(
        nm.join("cookie/package.json"),
        r#"{"name":"cookie","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        nm.join("cookie/index.js"),
        r#"exports.parse = function(str) {
  var out = {};
  var parts = String(str || "").split(";");
  for (var i = 0; i < parts.length; i++) {
    var p = parts[i].trim();
    var eq = p.indexOf("=");
    if (eq !== -1) out[p.slice(0, eq).trim()] = p.slice(eq + 1).trim();
  }
  return out;
};"#,
    )
    .unwrap();

    std::fs::create_dir_all(nm.join("flatted")).unwrap();
    std::fs::write(
        nm.join("flatted/package.json"),
        r#"{"name":"flatted","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        nm.join("flatted/index.js"),
        r#"exports.stringify = function(obj) {
  var copy = Object.assign({}, obj);
  delete copy.root;
  return JSON.stringify(copy);
};
exports.parse = function(str) {
  var obj = JSON.parse(str);
  obj.root = obj;
  return obj;
};"#,
    )
    .unwrap();

    let cas_path = tmp_dir.join("app19.cas");
    build_cas_archive_with_static(&tmp_dir, &cas_path, "./src/index.ts", None).unwrap();
    let archive = MappedCasArchive::open(&cas_path).unwrap();
    let entry = archive.entrypoint_blob().unwrap();

    let mut caps = CapabilityToken::pure_compute();
    if let Some(manifest_toml) = archive.get_manifest_toml() {
        merge_manifest_toml_into_caps(manifest_toml, &mut caps);
    }
    let mut iso = SliceIsolate::spawn("app19-tenant", 1, caps, 50_000_000, 16 * 1024 * 1024);
    let metrics = iso
        .execute_bytecode(archive.get_bytecode_slice(entry), Some(&archive))
        .unwrap();

    let expected_output = r#"{"app":"19-wintertc-webcrypto-streams-edge","libraries":["bytes@3.1.2","cookie@1.0.2","flatted@3.3.1","ms@2.1.3"],"hs256TokenValid":true,"hs256TamperedRejected":true,"hs512WebhookValid":true,"jwkAlg":"HS256","encodeIntoRead":8,"encodeIntoWritten":10,"transformedPayload":"ROUTE:API  |AUTH:JWT  |STREAM:OK::EOF","totalTransformedBytes":37,"formFieldsCount":3,"uploadedFileName":"bundle-manifest.bin","uploadedFileSize":38,"slicedHeaderText":"SLICE-EDGE","cyclicCloneVerified":true,"dispatchedEvents":["once:init","live:init","live:transform"],"anySignalAbortedBy":"secondary-deadline-exceeded","formattedWireVolume":"75B","auditDigest":"7319225b3dc10146"}"#;
    assert_eq!(metrics.stdout[0], expected_output);

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_phase_n4_buffer_node_core_and_ast_bundler_roundtrip() {
    let tmp_dir = std::env::temp_dir().join("slice_phase_n4_compat_test");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let src_dir = tmp_dir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();

    let manifest_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/apps/20-node-compat-and-edge-router");
    std::fs::copy(
        manifest_src.join("package.json"),
        tmp_dir.join("package.json"),
    )
    .unwrap();
    std::fs::copy(manifest_src.join("slice.toml"), tmp_dir.join("slice.toml")).unwrap();
    for f in [
        "types.ts",
        "binary_wire_codec.ts",
        "router_and_events_bus.ts",
        "index.ts",
    ] {
        std::fs::copy(manifest_src.join("src").join(f), src_dir.join(f)).unwrap();
    }

    // Stub minimal CJS modules for cookie, ms, bytes, and semver in node_modules
    let nm = tmp_dir.join("node_modules");
    std::fs::create_dir_all(nm.join("ms")).unwrap();
    std::fs::write(
        nm.join("ms/package.json"),
        r#"{"name":"ms","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        nm.join("ms/index.js"),
        r#"module.exports = function(v, o) { return __slice_fast_ms(v, Boolean(o && o.long)); };"#,
    )
    .unwrap();

    std::fs::create_dir_all(nm.join("bytes")).unwrap();
    std::fs::write(
        nm.join("bytes/package.json"),
        r#"{"name":"bytes","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        nm.join("bytes/index.js"),
        r#"var fn = function(v){ return typeof v === "number" ? __slice_fast_bytes_format(v) : __slice_fast_bytes_parse(v); };
fn.parse = function(v){ return __slice_fast_bytes_parse(v); };
fn.format = function(v){ return __slice_fast_bytes_format(v); };
module.exports = fn;"#,
    )
    .unwrap();

    std::fs::create_dir_all(nm.join("cookie")).unwrap();
    std::fs::write(
        nm.join("cookie/package.json"),
        r#"{"name":"cookie","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        nm.join("cookie/index.js"),
        r#"exports.parse = function(str) {
  var out = {};
  var parts = String(str || "").split(";");
  for (var i = 0; i < parts.length; i++) {
    var p = parts[i].trim();
    var eq = p.indexOf("=");
    if (eq !== -1) out[p.slice(0, eq).trim()] = p.slice(eq + 1).trim();
  }
  return out;
};"#,
    )
    .unwrap();

    std::fs::create_dir_all(nm.join("semver")).unwrap();
    std::fs::write(
        nm.join("semver/package.json"),
        r#"{"name":"semver","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        nm.join("semver/index.js"),
        r#"module.exports = { satisfies: function(v, r){ return __slice_semver_satisfies(String(v), String(r)); } };"#,
    )
    .unwrap();

    let cas_path = tmp_dir.join("app20.cas");
    build_cas_archive_with_static(&tmp_dir, &cas_path, "./src/index.ts", None).unwrap();
    let archive = MappedCasArchive::open(&cas_path).unwrap();
    let entry = archive.entrypoint_blob().unwrap();

    let mut caps = CapabilityToken::pure_compute();
    if let Some(manifest_toml) = archive.get_manifest_toml() {
        merge_manifest_toml_into_caps(manifest_toml, &mut caps);
    }
    let mut iso = SliceIsolate::spawn("app20-tenant", 1, caps, 50_000_000, 16 * 1024 * 1024);
    let metrics = iso
        .execute_bytecode(archive.get_bytecode_slice(entry), Some(&archive))
        .unwrap();

    let expected_output = r#"{"suite":"20-node-compat-and-edge-router","wireFrame":{"magicHex":"534c4334","version":4,"flags":258,"sequenceId":"9007199254740993","payloadUtf8":"{\"rpc\":\"edge.dispatch\",\"region\":\"iad-01\",\"currency\":\"€\"}","payloadBase64Url":"eyJycGMiOiJlZGdlLmRpc3BhdGNoIiwicmVnaW9uIjoiaWFkLTAxIiwiY3VycmVuY3kiOiLigqwifQ","sha512DigestPrefix":"06c23b3b4132558f8dd7b430","hmacSha256Hex":"fda684d490d2bd86c7023653983dab877506c69edf9bb254675a7305e9d91c6c","isValidBufferInstance":true},"routeSummary":{"normalizedRoute":"/api/v2/ tenants/dispatch/invoke","relativeAssetPath":"edge/assets/bundles/client.v4.mjs","parsedBase":"client.v4.mjs","sessionTenant":"acme-fintech-eu","ttlMs":900000,"maxBodyBytes":65536,"engineSemverSatisfied":true,"formattedAuditLine":"tenant=acme-fintech-eu route=/api/v2/ tenants/dispatch/invoke status=200 asset=client.v4.mjs","eventSequence":["prepend:/api/v2/ tenants/dispatch/invoke","on:/api/v2/ tenants/dispatch/invoke","auth-once:acme-fintech-eu","done:200"]}}"#;
    assert_eq!(metrics.stdout[0], expected_output);

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_wave_a_cli_tsx_ssr_and_test_runner() {
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    let tmp_dir = env::temp_dir().join("slice_wave_a_tsx_cli_test");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(tmp_dir.join("src")).unwrap();
    std::fs::create_dir_all(tmp_dir.join("tests")).unwrap();

    // 1. Write a TSX component (`src/Card.tsx`) and TSX server entrypoint (`src/index.tsx`)
    std::fs::write(
        tmp_dir.join("src/Card.tsx"),
        r#"
export interface CardProps {
    title: string;
    items: string[];
}

export function Card(props: CardProps) {
    return (
        <section className="edge-card" data-count={props.items.length}>
            <h2>{props.title}</h2>
            <ul>
                {props.items.map((item) => (
                    <li key={item}>{item}</li>
                ))}
            </ul>
        </section>
    );
}
"#,
    )
    .unwrap();

    std::fs::write(
        tmp_dir.join("src/index.tsx"),
        r#"
import { renderToString } from "slice:jsx";
import { Card } from "./Card";

export default {
    fetch(req: Request): Response {
        const url = new URL(req.url);
        const title = url.searchParams.get("title") ?? "Default <Title>";
        const html = renderToString(
            <Card title={title} items={["Oxc TSX", "Slice SSR & <Safe>"]} />
        );
        return new Response("<!doctype html>" + html, {
            status: 200,
            headers: { "content-type": "text/html; charset=utf-8" }
        });
    }
};
"#,
    )
    .unwrap();

    // 2. Write a `.test.tsx` test file verifying JSX SSR, `setImmediate`/`nextTick`, and `console.table`
    std::fs::write(
        tmp_dir.join("tests/ssr.test.tsx"),
        r#"
import { describe, test, expect } from "slice:test";
import { renderToString } from "slice:jsx";
import { Card } from "../src/Card";

describe("TSX SSR & Event Loop Suite", () => {
    test("renders TSX component with XSS escaping", () => {
        const html = renderToString(<Card title="Hello <XSS>" items={["A", "B"]} />);
        expect(html).toContain('<section class="edge-card" data-count="2">');
        expect(html).toContain("<h2>Hello &lt;XSS&gt;</h2>");
        expect(html).toContain("<li>A</li><li>B</li>");
    });

    test("console.table and setImmediate execute cleanly", () => {
        setImmediate(() => {});
        console.table([{ feature: "TSX", ok: true }]);
        expect(typeof setImmediate).toBe("function");
    });
});
"#,
    )
    .unwrap();

    // 3. Verify `discover_test_files` and `cmd_test` execute `tests/ssr.test.tsx`
    let discovered = discover_test_files(&tmp_dir).unwrap();
    assert_eq!(discovered.len(), 1);
    assert!(discovered[0].to_string_lossy().ends_with("ssr.test.tsx"));
    let test_args = vec![tmp_dir.to_string_lossy().to_string()];
    cmd_test(&test_args).unwrap();

    // 4. Verify `.tsx` HTTP Server SSR over TCP
    let bundled = slice_cas::bundle_entrypoint_with_deps(&tmp_dir.join("src/index.tsx")).unwrap();
    let bytecode = Arc::new(compile_source_to_bytecode(&bundled));

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let counter = Arc::new(AtomicUsize::new(0));
    let bc_srv = Arc::clone(&bytecode);
    let counter_srv = Arc::clone(&counter);

    let srv = std::thread::spawn(move || {
        SliceIsolate::prewarm_boa_pool(1);
        SliceIsolate::prewarm_boa_bundle(&bc_srv);
        let mut read_buf = vec![0u8; 65536];
        let mut write_buf = Vec::with_capacity(4096);
        if let Ok((stream, _)) = listener.accept() {
            let _ = serve_connection_keep_alive(
                stream,
                0,
                &bc_srv,
                None,
                None,
                None,
                &CapabilityToken::pure_compute(),
                2_000_000,
                4 * 1024 * 1024,
                &mut read_buf,
                &mut write_buf,
                Some((&counter_srv, 0)),
            );
        }
    });

    let mut client = TcpStream::connect(addr).unwrap();
    client
        .write_all(
            b"GET /?title=Wave%20A%20%3CRust%3E HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .unwrap();
    let mut resp = String::new();
    client.read_to_string(&mut resp).unwrap();
    srv.join().unwrap();
    let _ = std::fs::remove_dir_all(&tmp_dir);

    assert!(resp.contains("HTTP/1.1 200 OK"));
    assert!(resp.contains("Content-Type: text/html; charset=utf-8"));
    assert!(resp.contains(
        "<!doctype html><section class=\"edge-card\" data-count=\"2\"><h2>Wave A &lt;Rust&gt;</h2><ul><li>Oxc TSX</li><li>Slice SSR &amp; &lt;Safe&gt;</li></ul></section>"
    ));
}

#[test]
fn test_wave_b_http_server_security_hardening_and_websocket_upgrade_roundtrip() {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{atomic::AtomicUsize, Arc};

    let handler_ts = r#"
export default {
    async fetch(req: Request): Promise<Response> {
        const url = new URL(req.url);
        if (req.headers.get("upgrade") === "websocket") {
            return Slice.upgradeWebSocket({
                open(ws: any) {
                    ws.send("welcome:" + url.pathname);
                },
                message(ws: any, msg: any) {
                    if (typeof msg === "string") {
                        ws.send("server-echo:" + msg);
                    } else if (msg instanceof Uint8Array) {
                        const out = new Uint8Array(msg.length);
                        for (let i = 0; i < msg.length; i++) out[i] = msg[i] ^ 0xff;
                        ws.send(out);
                    }
                }
            });
        }
        const bodyText = await req.text();
        return Response.json({
            path: url.pathname,
            body: bodyText,
            hdr25: req.headers.get("x-custom-25") || null
        });
    }
};
"#;
    let bundled = slice_cas::strip_typescript_and_normalize(handler_ts, true);
    let bytecode = Arc::new(compile_source_to_bytecode(&bundled));

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();
    let counter = Arc::new(AtomicUsize::new(0));
    let bc_srv = Arc::clone(&bytecode);
    let counter_srv = Arc::clone(&counter);

    // Accept 6 connections:
    // 1) Smuggling CL.TE conflict -> 400 Bad Request
    // 2) Smuggling conflicting duplicate Content-Length -> 400 Bad Request
    // 3) Oversized Content-Length (> 4MB) -> 413 Payload Too Large
    // 4) > 64 headers -> 431 Request Header Fields Too Large
    // 5) Valid inbound Transfer-Encoding: chunked + 30 custom headers -> 200 OK
    // 6) RFC 6455 WebSocket Upgrade (`101 Switching Protocols` + full frame loop)
    let srv = std::thread::spawn(move || {
        SliceIsolate::prewarm_boa_pool(1);
        SliceIsolate::prewarm_boa_bundle(&bc_srv);
        let mut read_buf = vec![0u8; 65536];
        let mut write_buf = Vec::with_capacity(4096);
        for i in 0..6 {
            if let Ok((stream, _)) = listener.accept() {
                let _ = serve_connection_keep_alive(
                    stream,
                    i,
                    &bc_srv,
                    None,
                    None,
                    None,
                    &CapabilityToken::pure_compute(),
                    4_000_000,
                    8 * 1024 * 1024,
                    &mut read_buf,
                    &mut write_buf,
                    Some((&counter_srv, 0)),
                );
            }
        }
    });

    // 1. N-Sec-3: CL.TE Request Smuggling attempt -> 400 Bad Request
    {
        let mut c = TcpStream::connect(addr).unwrap();
        c.write_all(
            b"POST /smuggle HTTP/1.1\r\nHost: localhost\r\nContent-Length: 6\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
        )
        .unwrap();
        let mut r = String::new();
        c.read_to_string(&mut r).unwrap();
        assert!(r.starts_with("HTTP/1.1 400 Bad Request"), "Got: {}", r);
        assert!(r.contains("SMUGGLING_CL_TE_CONFLICT_BLOCKED"));
    }

    // 2. N-Sec-3: Duplicate conflicting Content-Length -> 400 Bad Request
    {
        let mut c = TcpStream::connect(addr).unwrap();
        c.write_all(
            b"POST /dup-cl HTTP/1.1\r\nHost: localhost\r\nContent-Length: 4\r\nContent-Length: 8\r\n\r\ntest",
        )
        .unwrap();
        let mut r = String::new();
        c.read_to_string(&mut r).unwrap();
        assert!(r.starts_with("HTTP/1.1 400 Bad Request"), "Got: {}", r);
        assert!(r.contains("SMUGGLING_DUPLICATE_CONTENT_LENGTH_BLOCKED"));
    }

    // 3. N-Sec-3: Content-Length > 4MB -> 413 Payload Too Large
    {
        let mut c = TcpStream::connect(addr).unwrap();
        c.write_all(b"POST /huge HTTP/1.1\r\nHost: localhost\r\nContent-Length: 9999999\r\n\r\n")
            .unwrap();
        let mut r = String::new();
        c.read_to_string(&mut r).unwrap();
        assert!(
            r.starts_with("HTTP/1.1 413 Payload Too Large"),
            "Got: {}",
            r
        );
    }

    // 4. N-Sec-6: > 64 request headers -> 431 Request Header Fields Too Large
    {
        let mut c = TcpStream::connect(addr).unwrap();
        let mut req = String::from("GET /too-many-hdrs HTTP/1.1\r\nHost: localhost\r\n");
        for idx in 0..65 {
            req.push_str(&format!("X-Extra-{}: val-{}\r\n", idx, idx));
        }
        req.push_str("\r\n");
        c.write_all(req.as_bytes()).unwrap();
        let mut r = String::new();
        c.read_to_string(&mut r).unwrap();
        assert!(
            r.starts_with("HTTP/1.1 431 Request Header Fields Too Large"),
            "Got: {}",
            r
        );
    }

    // 5. N-Sec-3 + N-Sec-6: Valid inbound Transfer-Encoding: chunked + 30 headers -> 200 OK
    {
        let mut c = TcpStream::connect(addr).unwrap();
        let mut req = String::from(
            "POST /chunked-in HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n",
        );
        for idx in 1..=30 {
            req.push_str(&format!("X-Custom-{}: hdr-val-{}\r\n", idx, idx));
        }
        req.push_str("\r\n5\r\nchunk\r\n7\r\n-stream\r\n0\r\n\r\n");
        c.write_all(req.as_bytes()).unwrap();
        let mut r = String::new();
        c.read_to_string(&mut r).unwrap();
        assert!(r.starts_with("HTTP/1.1 200 OK"), "Got: {}", r);
        assert!(r.contains(r#""body":"chunk-stream""#), "Got: {}", r);
        assert!(r.contains(r#""hdr25":"hdr-val-25""#), "Got: {}", r);
    }

    // 6. End-to-End Server-Side RFC 6455 WebSocket Upgrade with a Slice `new WebSocket` Client
    {
        let client_js = format!(
            r#"
const log = [];
const ws = new WebSocket("ws://127.0.0.1:{port}/live-room");
ws.binaryType = "arraybuffer";
ws.onmessage = (ev) => {{
    if (typeof ev.data === "string") {{
        log.push(ev.data);
        if (ev.data === "welcome:/live-room") {{
            ws.send("ping-from-client");
            ws.send(new Uint8Array([0x00, 0x0f, 0xf0]));
            ws.close(1000, "bye");
        }}
    }} else if (ev.data instanceof ArrayBuffer) {{
        log.push("bin:" + Array.from(new Uint8Array(ev.data)).join(","));
    }}
}};
ws.onclose = (ev) => {{
    log.push("closed:" + ev.code);
    console.log(JSON.stringify(log));
}};
"#,
            port = port
        );
        let ws_caps = CapabilityToken::pure_compute().with_net_host(format!("127.0.0.1:{}", port));
        let metrics = slice_engine::run_source_isolated(
            "wave-b-e2e-ws",
            &client_js,
            ws_caps,
            2_000_000,
            4 * 1024 * 1024,
        )
        .unwrap();
        let out: Vec<String> = serde_json::from_str(&metrics.stdout[0]).unwrap();
        assert_eq!(
            out,
            vec![
                "welcome:/live-room",
                "server-echo:ping-from-client",
                "bin:255,240,15",
                "closed:1000"
            ]
        );
    }

    srv.join().unwrap();
}

#[test]
fn test_wave_b_inbound_spillover_body_crlf_headers_and_server_ws_dlp() {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{atomic::AtomicUsize, Arc};

    let handler_ts = r#"
export default {
    async fetch(req: Request): Promise<Response> {
        const url = new URL(req.url);
        if (req.headers.get("upgrade") === "websocket") {
            return Slice.upgradeWebSocket({
                message(ws: any, msg: any) {
                    if (msg === "leak-secret") {
                        // Attempt to exfiltrate an audited secret from the server WS handler
                        ws.send("token:" + process.env.WS_SERVER_MASTER_SECRET);
                    } else {
                        ws.send("clean:" + msg);
                    }
                }
            });
        }
        const bodyText = await req.text();
        return Response.json({
            path: url.pathname,
            len: bodyText.length,
            prefix: bodyText.slice(0, 8),
            suffix: bodyText.slice(-8)
        });
    }
};
"#;
    let bundled = slice_cas::strip_typescript_and_normalize(handler_ts, true);
    let bytecode = Arc::new(compile_source_to_bytecode(&bundled));

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();
    let counter = Arc::new(AtomicUsize::new(0));
    let bc_srv = Arc::clone(&bytecode);
    let counter_srv = Arc::clone(&counter);

    let caps = CapabilityToken::pure_compute()
        .with_env("WS_SERVER_MASTER_SECRET", "sk_live_SERVER_WS_SECRET_999");

    // 3 connections:
    // 1) 100 KB POST payload (> 64 KB stack read_buf, exercising spillover_body) -> 200 OK
    // 2) Inbound header with control char (`\x07`) -> 400 Bad Request
    // 3) Server-side WebSocket handler attempting to send `WS_SERVER_MASTER_SECRET` -> 1008 Policy Violation DLP trap
    let srv = std::thread::spawn(move || {
        SliceIsolate::prewarm_boa_pool(1);
        SliceIsolate::prewarm_boa_bundle(&bc_srv);
        let mut read_buf = vec![0u8; 65536];
        let mut write_buf = Vec::with_capacity(4096);
        for i in 0..3 {
            if let Ok((stream, _)) = listener.accept() {
                let _ = serve_connection_keep_alive(
                    stream,
                    i,
                    &bc_srv,
                    None,
                    None,
                    None,
                    &caps,
                    4_000_000,
                    8 * 1024 * 1024,
                    &mut read_buf,
                    &mut write_buf,
                    Some((&counter_srv, 0)),
                );
            }
        }
    });

    // 1. N-Sec-3: 100 KB POST body spilling beyond 64 KB `read_buf` into `spillover_body`
    {
        let mut c = TcpStream::connect(addr).unwrap();
        let body = format!("START123{}98765END", "A".repeat(100_000 - 16));
        let req = format!(
            "POST /spillover HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        c.write_all(req.as_bytes()).unwrap();
        let mut r = String::new();
        c.read_to_string(&mut r).unwrap();
        assert!(r.starts_with("HTTP/1.1 200 OK"), "Got: {}", r);
        assert!(r.contains(r#""len":100000"#), "Got: {}", r);
        assert!(r.contains(r#""prefix":"START123""#), "Got: {}", r);
        assert!(r.contains(r#""suffix":"98765END""#), "Got: {}", r);
    }

    // 2. N-Sec-2: Inbound header with control char (`\x07` BEL) -> 400 Bad Request
    {
        let mut c = TcpStream::connect(addr).unwrap();
        c.write_all(b"GET /bad-hdr HTTP/1.1\r\nHost: localhost\r\nX-Bell: ring\x07here\r\n\r\n")
            .unwrap();
        let mut r = String::new();
        c.read_to_string(&mut r).unwrap();
        assert!(r.starts_with("HTTP/1.1 400 Bad Request"), "Got: {}", r);
        assert!(r.contains("INVALID_HEADER_CHARACTERS"), "Got: {}", r);
    }

    // 3. Server-Side WebSocket Outbound Egress DLP Trap (`1008 Policy Violation`)
    {
        let client_js = format!(
            r#"
const events = [];
const ws = new WebSocket("ws://127.0.0.1:{port}/dlp-room");
ws.onopen = () => {{
    ws.send("safe-ping");
    ws.send("leak-secret");
}};
ws.onmessage = (ev) => {{
    events.push("msg:" + ev.data);
}};
ws.onclose = (ev) => {{
    events.push("close:" + ev.code);
    console.log(JSON.stringify(events));
}};
"#,
            port = port
        );
        let ws_caps = CapabilityToken::pure_compute().with_net_host(format!("127.0.0.1:{}", port));
        let metrics = slice_engine::run_source_isolated(
            "wave-b-srv-ws-dlp-client",
            &client_js,
            ws_caps,
            2_000_000,
            4 * 1024 * 1024,
        )
        .unwrap();
        let out_line = &metrics.stdout[0];
        assert!(
            !out_line.contains("sk_live_SERVER_WS_SECRET_999"),
            "Server secret MUST NEVER leak over WebSocket frame: {}",
            out_line
        );
        let out: Vec<String> = serde_json::from_str(out_line).unwrap();
        assert_eq!(out, vec!["msg:clean:safe-ping", "close:1008"]);
    }

    srv.join().unwrap();
}

#[test]
fn test_wave_b_app21_websocket_realtime_and_security_guard_roundtrip() {
    use std::net::TcpListener;
    use std::path::Path;
    use std::sync::{atomic::AtomicUsize, Arc};

    let upstream_ts = r#"
export default {
    fetch(req: Request): Response {
        const url = new URL(req.url);
        if ((req.headers.get("upgrade") || "").toLowerCase() === "websocket") {
            return Slice.upgradeWebSocket({
                open(ws: any) {
                    ws.send(JSON.stringify({ type: "handshake_ack", path: url.pathname }));
                },
                message(ws: any, msg: any) {
                    if (typeof msg === "string") {
                        const parsed = JSON.parse(msg);
                        ws.send(JSON.stringify({
                            type: "telemetry_ack",
                            seq: parsed.seq,
                            tenant: parsed.tenant,
                            digest: parsed.digest,
                            ok: true
                        }));
                    } else if (msg instanceof Uint8Array) {
                        const out = new Uint8Array(msg.length);
                        for (let i = 0; i < msg.length; i++) {
                            out[i] = (msg[i] ^ 0x5a) & 0xff;
                        }
                        ws.send(out);
                    }
                }
            });
        }
        if (url.pathname === "/upstream/subgraph/account") {
            const accountId = url.searchParams.get("account") || "acct-0";
            return Response.json({
                accountId,
                tier: "enterprise",
                balanceCents: 14500,
                region: "us-east-1"
            });
        }
        return Response.json({ ok: true });
    }
};
"#;
    let bundled_srv = slice_cas::strip_typescript_and_normalize(upstream_ts, true);
    let bc_srv = Arc::new(compile_source_to_bytecode(&bundled_srv));

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let host_port = format!("127.0.0.1:{}", port);
    let counter = Arc::new(AtomicUsize::new(0));
    let bc_srv_clone = Arc::clone(&bc_srv);
    let counter_clone = Arc::clone(&counter);

    // Accept 2 connections from App 21:
    // 1) WebSocket connection (`ws://127.0.0.1:{port}/upstream/ws...`)
    // 2) HTTP Keep-Alive GET (`http://127.0.0.1:{port}/upstream/subgraph/account...`)
    let srv_handle = std::thread::spawn(move || {
        SliceIsolate::prewarm_boa_pool(1);
        SliceIsolate::prewarm_boa_bundle(&bc_srv_clone);
        let mut read_buf = vec![0u8; 65536];
        let mut write_buf = Vec::with_capacity(4096);
        for i in 0..2 {
            if let Ok((stream, _)) = listener.accept() {
                let _ = serve_connection_keep_alive(
                    stream,
                    i,
                    &bc_srv_clone,
                    None,
                    None,
                    None,
                    &CapabilityToken::pure_compute(),
                    10_000_000,
                    8 * 1024 * 1024,
                    &mut read_buf,
                    &mut write_buf,
                    Some((&counter_clone, 2)),
                );
            }
        }
    });

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let app21_src = repo_root
        .join("tests")
        .join("apps")
        .join("21-websocket-realtime-and-security-guard")
        .join("src");
    let tmp_dir = std::env::temp_dir().join("slice_wave_b_app21_roundtrip");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let dst_src = tmp_dir.join("src");
    std::fs::create_dir_all(&dst_src).unwrap();

    for file_name in [
        "types.ts",
        "network_security_auditor.ts",
        "realtime_ws_client.ts",
        "index.ts",
    ] {
        let content = std::fs::read_to_string(app21_src.join(file_name)).unwrap();
        let patched = content.replace("127.0.0.1:18095", &host_port);
        std::fs::write(dst_src.join(file_name), patched).unwrap();
    }
    std::fs::write(
        tmp_dir.join("package.json"),
        r#"{"name":"21-websocket-realtime-and-security-guard","version":"1.0.0","main":"./src/index.ts"}"#,
    )
    .unwrap();
    std::fs::write(
        tmp_dir.join("slice.toml"),
        format!(
            "[package]\nname = \"21-websocket-realtime-and-security-guard\"\nentrypoint = \"./src/index.ts\"\n[capabilities]\nallow_net = [\"{}\"]\n",
            host_port
        ),
    )
    .unwrap();

    let nm = tmp_dir.join("node_modules");
    for (pkg, code) in [
        (
            "bytes",
            r#"var fn = function(v){ return typeof v === "number" ? __slice_fast_bytes_format(v) : __slice_fast_bytes_parse(v); };
fn.parse = function(v){ return __slice_fast_bytes_parse(v); };
fn.format = function(v){ return __slice_fast_bytes_format(v); };
module.exports = fn;"#,
        ),
        (
            "cookie",
            "module.exports = { serialize: function(k, v) { return k + '=' + v + '; Path=/; HttpOnly'; }, parse: function(s) { var o = {}; String(s).split(';').forEach(function(p) { var kv = p.trim().split('='); if (kv.length === 2) o[kv[0]] = kv[1]; }); return o; } };",
        ),
        (
            "fast-querystring",
            "module.exports = { parse: function(q) { var o = {}; String(q).split('&').forEach(function(p) { var kv = p.split('='); if (kv[0]) o[decodeURIComponent(kv[0])] = decodeURIComponent(kv[1] || ''); }); return o; } };",
        ),
        (
            "ms",
            "module.exports = function(v, o) { return __slice_fast_ms(v, Boolean(o && o.long)); };",
        ),
    ] {
        let pkg_dir = nm.join(pkg);
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("index.js"), code).unwrap();
    }

    let cas_out = tmp_dir.join("app21.cas");
    build_cas_archive_with_static(&tmp_dir, &cas_out, "./src/index.ts", None).unwrap();

    let archive = MappedCasArchive::open(&cas_out).unwrap();
    let mut caps = CapabilityToken::pure_compute();
    if let Some(manifest_toml) = archive.get_manifest_toml() {
        merge_manifest_toml_into_caps(manifest_toml, &mut caps);
    }
    let entry = archive.entrypoint_blob().unwrap();
    let mut iso = SliceIsolate::spawn("app21-tenant", 1, caps, 50_000_000, 16 * 1024 * 1024);
    let metrics = iso
        .execute_bytecode(archive.get_bytecode_slice(entry), Some(&archive))
        .unwrap();

    let _ = srv_handle.join();
    let _ = std::fs::remove_dir_all(&tmp_dir);

    assert_eq!(
        metrics.stdout[0],
        r#"{"suite":"21-websocket-realtime-and-security-guard","auditSummary":{"totalCasesEvaluated":14,"smugglingClTeBlocked":1,"smugglingDupClBlocked":1,"crlfHeaderInjectionBlocked":2,"chunkedOverflowBlocked":2,"ssrfNonCanonicalIpBlocked":4,"wsFrameDlpBlocked":2,"cleanRequestsAllowed":2,"maxHeaderBudgetBytes":65536,"maxInboundBodyBudgetBytes":4194304,"maxWsFrameBudgetBytes":4194304,"auditTrailDigest":"85de31109bd3918142bcf47d1561acc7"},"wsSummary":{"handshakePath":"/upstream/ws","rfc6455AcceptVerified":true,"maskedFrameRoundtripVerified":true,"textFramesSent":4,"textAcksReceived":4,"binaryFramesSent":2,"binaryXorBytesVerified":14,"closeCode":1000,"closeReason":"audit-complete","upstreamSubgraphStatus":200,"upstreamAccountTier":"enterprise","sessionTranscriptDigest":"4b83c4f300269774642ac7316925946e"}}"#
    );
}

#[test]
fn test_cli_wave_c_workers_and_raw_sockets_e2e() {
    let tmp_dir = std::env::temp_dir().join(format!("slice_wave_c_e2e_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();

    // 1. Write a dedicated worker file on disk that computes SHA-256 + multiplies workerData
    let worker_path = tmp_dir.join("compute_worker.ts");
    std::fs::write(
        &worker_path,
        r#"
import crypto from "node:crypto";
import { workerData, parentPort, isMainThread } from "node:worker_threads";

parentPort.once("message", (msg: { label: string; values: number[] }) => {
    const sum = msg.values.reduce((acc, v) => acc + v, 0) * (workerData.scale || 1);
    const digest = crypto.createHash("sha256").update(msg.label + ":" + sum).digest("hex").slice(0, 16);
    parentPort.postMessage({
        workerMainThread: isMainThread,
        sum,
        digest
    });
});
"#,
    )
    .unwrap();

    let worker_path_norm = worker_path.to_string_lossy().replace('\\', "/");
    let main_path = tmp_dir.join("main.ts");
    std::fs::write(
        &main_path,
        format!(
            r#"
import {{ Worker }} from "node:worker_threads";
import {{ connect }} from "cloudflare:sockets";
import net from "node:net";

async function run() {{
    Slice.mockSocket("cache.internal:6379", "redis");
    const sock = connect("cache.internal:6379");
    const writer = sock.writable.getWriter();
    await writer.write("GET session:42\r\n");
    const reader = sock.readable.getReader();
    const {{ value }} = await reader.read();
    const redisRaw = new TextDecoder().decode(value).trim();
    await sock.close();

    const workerRes: any = await new Promise((resolve, reject) => {{
        const w = new Worker("{worker_path_norm}", {{
            workerData: {{ scale: 4 }}
        }});
        w.on("message", resolve);
        w.on("error", reject);
        w.postMessage({{ label: "batch-c", values: [2, 3, 5] }});
    }});

    console.log(JSON.stringify({{
        redisOk: redisRaw.includes("slice-fast"),
        workerMainThread: workerRes.workerMainThread,
        workerSum: workerRes.sum,
        workerDigest: workerRes.digest,
        ipCheck: net.isIPv4("10.0.0.1") && net.isIPv6("2001:db8::1")
    }}));
}}
run();
"#
        ),
    )
    .unwrap();

    let src = std::fs::read_to_string(&main_path).unwrap();
    let bc = compile_source_to_bytecode(&src);
    let caps = CapabilityToken::pure_compute()
        .with_read_prefix(tmp_dir.to_string_lossy().replace('\\', "/"))
        .with_net_host("cache.internal:6379");
    let mut iso = SliceIsolate::spawn("wave-c-cli", 1, caps, 5_000_000, 16 * 1024 * 1024);
    let metrics = iso.execute_bytecode(&bc, None).unwrap();
    let _ = std::fs::remove_dir_all(&tmp_dir);

    assert_eq!(
        metrics.stdout[0],
        r#"{"redisOk":true,"workerMainThread":false,"workerSum":40,"workerDigest":"985e4a2a3ae406a5","ipCheck":true}"#
    );
}

#[test]
fn test_wave_c_app22_worker_threads_and_raw_sockets_roundtrip() {
    use std::io::{Read, Write};
    use std::net::{TcpListener, UdpSocket};
    use std::path::Path;

    let tcp_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let tcp_port = tcp_listener.local_addr().unwrap().port();
    let tcp_host_port = format!("127.0.0.1:{}", tcp_port);

    let udp_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let udp_port = udp_sock.local_addr().unwrap().port();
    let udp_host_port = format!("127.0.0.1:{}", udp_port);

    let tcp_srv = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = tcp_listener.accept() {
            let mut buf = [0u8; 512];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(b"+PONG\r\n+OK\r\n$13\r\nactive-wave-c\r\n+BYE\r\n");
            let _ = stream.flush();
        }
    });

    let udp_srv = std::thread::spawn(move || {
        let mut buf = [0u8; 512];
        if let Ok((n, peer)) = udp_sock.recv_from(&mut buf) {
            let msg = String::from_utf8_lossy(&buf[..n]);
            if msg.starts_with("SLICE_UDP_PROBE:") {
                let _ = udp_sock.send_to(b"UDP_ACK:acme-realtime:seq=1:ok", peer);
            }
        }
    });

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let app22_src = repo_root
        .join("tests")
        .join("apps")
        .join("22-worker-threads-and-raw-sockets")
        .join("src");
    let tmp_dir = std::env::temp_dir().join(format!("slice_wave_c_app22_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let dst_src = tmp_dir.join("src");
    std::fs::create_dir_all(&dst_src).unwrap();

    for file_name in [
        "types.ts",
        "worker_socket_security_auditor.ts",
        "worker_and_socket_engine.ts",
        "index.ts",
    ] {
        let content = std::fs::read_to_string(app22_src.join(file_name)).unwrap();
        let patched = content
            .replace("18096", &tcp_port.to_string())
            .replace("18097", &udp_port.to_string());
        std::fs::write(dst_src.join(file_name), patched).unwrap();
    }
    std::fs::write(
        tmp_dir.join("package.json"),
        r#"{"name":"22-worker-threads-and-raw-sockets","version":"1.0.0","main":"./src/index.ts"}"#,
    )
    .unwrap();
    std::fs::write(
        tmp_dir.join("slice.toml"),
        format!(
            "[package]\nname = \"22-worker-threads-and-raw-sockets\"\nentrypoint = \"./src/index.ts\"\n[capabilities]\nallow_net = [\"{}\", \"{}\"]\n",
            tcp_host_port, udp_host_port
        ),
    )
    .unwrap();

    let nm = tmp_dir.join("node_modules");
    for (pkg, code) in [
        (
            "bytes",
            r#"var fn = function(v){ return typeof v === "number" ? __slice_fast_bytes_format(v) : __slice_fast_bytes_parse(v); };
fn.parse = function(v){ return __slice_fast_bytes_parse(v); };
fn.format = function(v){ return __slice_fast_bytes_format(v); };
module.exports = fn;"#,
        ),
        (
            "cookie",
            "module.exports = { serialize: function(k, v) { return k + '=' + v + '; Path=/; HttpOnly'; }, parse: function(s) { var o = {}; String(s).split(';').forEach(function(p) { var kv = p.trim().split('='); if (kv.length === 2) o[kv[0]] = kv[1]; }); return o; } };",
        ),
        (
            "fast-querystring",
            "module.exports = { parse: function(q) { var o = {}; String(q).split('&').forEach(function(p) { var kv = p.split('='); if (kv[0]) o[decodeURIComponent(kv[0])] = decodeURIComponent(kv[1] || ''); }); return o; } };",
        ),
        (
            "ms",
            "module.exports = function(v, o) { return __slice_fast_ms(v, Boolean(o && o.long)); };",
        ),
    ] {
        let pkg_dir = nm.join(pkg);
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("index.js"), code).unwrap();
    }

    let cas_out = tmp_dir.join("app22.cas");
    build_cas_archive_with_static(&tmp_dir, &cas_out, "./src/index.ts", None).unwrap();

    let archive = MappedCasArchive::open(&cas_out).unwrap();
    let mut caps = CapabilityToken::pure_compute();
    if let Some(manifest_toml) = archive.get_manifest_toml() {
        merge_manifest_toml_into_caps(manifest_toml, &mut caps);
    }
    let entry = archive.entrypoint_blob().unwrap();
    let mut iso = SliceIsolate::spawn("app22-tenant", 1, caps, 50_000_000, 16 * 1024 * 1024);
    let metrics = iso
        .execute_bytecode(archive.get_bytecode_slice(entry), Some(&archive))
        .unwrap();

    let _ = std::fs::remove_dir_all(&tmp_dir);
    assert!(
        !metrics.stdout.is_empty(),
        "App 22 produced no stdout! return_value={}",
        metrics.return_value
    );
    assert_eq!(
        metrics.stdout[0],
        r#"{"suite":"22-worker-threads-and-raw-sockets","auditSummary":{"totalCasesEvaluated":12,"ring1PrivilegedPortBlocked":2,"ring2SsrfIpBlocked":4,"ring3EgressDlpBlocked":2,"ring4BudgetOverflowBlocked":1,"ring5WorkerEscalationBlocked":1,"cleanOperationsAllowed":2,"urlCanParseVerified":true,"auditDigest":"2723413bc42cf3c6"},"sessionSummary":{"workersSpawned":3,"workerTasksCompleted":3,"totalRecordsProcessed":18,"combinedWeightedSum":3475,"combinedXorChecksum":37,"messageChannelSyncVerified":true,"tcpCommandsSent":4,"tcpRepliesVerified":4,"tcpStatusValue":"active-wave-c","udpDatagramsSent":1,"udpAckBytesReceived":30,"udpAckVerified":true,"sessionDigest":"68aebc6ff5256024"}}"#
    );
    let _ = tcp_srv.join();
    let _ = udp_srv.join();
}

#[test]
fn test_wave_d_wasm_import_bundling_and_os_native_watch_test_runner() {
    let tmp_dir = env::temp_dir().join("slice_wave_d_wasm_bundle_and_watch");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(tmp_dir.join("src")).unwrap();
    std::fs::create_dir_all(tmp_dir.join("tests")).unwrap();

    // Minimal valid WebAssembly binary exporting `add(i32, i32) -> i32`
    let add_wasm: [u8; 41] = [
        0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, // magic + version
        0x01, 0x07, 0x01, 0x60, 0x02, 0x7F, 0x7F, 0x01, 0x7F, // type 0: (i32, i32) -> i32
        0x03, 0x02, 0x01, 0x00, // func 0: type 0
        0x07, 0x07, 0x01, 0x03, 0x61, 0x64, 0x64, 0x00, 0x00, // export "add" -> func 0
        0x0A, 0x09, 0x01, 0x07, 0x00, 0x20, 0x00, 0x20, 0x01, 0x6A,
        0x0B, // local.get 0, local.get 1, i32.add
    ];
    std::fs::write(tmp_dir.join("src/add.wasm"), add_wasm).unwrap();

    std::fs::write(
        tmp_dir.join("src/index.ts"),
        r#"import addWasmBytes from "./add.wasm";

const mod = new WebAssembly.Module(addWasmBytes);
const inst = new WebAssembly.Instance(mod);
const sum = (inst.exports.add as (a: number, b: number) => number)(19, 23);
console.log(JSON.stringify({
    wasmImported: true,
    isUint8Array: addWasmBytes instanceof Uint8Array,
    byteLen: addWasmBytes.byteLength,
    sum
}));
"#,
    )
    .unwrap();

    let cas_out = tmp_dir.join("wasm_bundle.cas");
    build_cas_archive_with_static(&tmp_dir, &cas_out, "./src/index.ts", None).unwrap();

    let archive = MappedCasArchive::open(&cas_out).unwrap();
    let entry = archive.entrypoint_blob().unwrap();
    let mut iso = SliceIsolate::spawn(
        "wave-d-wasm-bundle-tenant",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        16 * 1024 * 1024,
    );
    let metrics = iso
        .execute_bytecode(archive.get_bytecode_slice(entry), Some(&archive))
        .unwrap();

    assert_eq!(metrics.os_stat_syscalls, 0);
    assert_eq!(
        metrics.stdout[0],
        r#"{"wasmImported":true,"isUint8Array":true,"byteLen":41,"sum":42}"#
    );

    // Also test OsNativeFileWatcher (`notify` crate) directly
    let test_file = tmp_dir.join("tests/wasm.test.ts");
    std::fs::write(
        &test_file,
        r#"import { describe, test, expect } from "slice:test";
describe("wasm watch suite", () => {
    test("adds numbers", () => {
        expect(19 + 23).toBe(42);
    });
});
"#,
    )
    .unwrap();

    let mut watcher = crate::OsNativeFileWatcher::new(&tmp_dir);
    let test_file_clone = test_file.clone();
    let writer_handle = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(40));
        let _ = std::fs::write(
            &test_file_clone,
            r#"import { describe, test, expect } from "slice:test";
describe("wasm watch suite", () => {
    test("adds numbers updated", () => {
        expect(20 + 22).toBe(42);
    });
});
"#,
        );
    });

    assert!(watcher.wait_for_change(true));
    writer_handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_wave_e_hono_and_itty_unjs_framework_suites() {
    // 1. Verify Annex B escape() / unescape() used by UnJS ohash UTF-8 parser
    let annex_b_js = r#"
        const raw = "slice edge?key=val&unicode=\u2603";
        const enc = escape(raw);
        const dec = unescape(enc);
        const utf8Roundtrip = unescape(encodeURIComponent("Hono+UnJS:100%"));
        console.log(JSON.stringify({ enc, match: dec === raw, utf8Roundtrip }));
    "#;
    let bc = compile_source_to_bytecode(annex_b_js);
    let mut iso = SliceIsolate::spawn(
        "wave-e-annex-b",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        16 * 1024 * 1024,
    );
    let m = iso.execute_bytecode(&bc, None).unwrap();
    assert_eq!(
        m.stdout[0],
        r#"{"enc":"slice%20edge%3Fkey%3Dval%26unicode%3D%u2603","match":true,"utf8Roundtrip":"Hono+UnJS:100%"}"#
    );

    // 2. Verify multi-module bundle containing Object.prototype.hasOwnProperty with prepended test harness lines
    iso.reset_for_request("wave-e-annex-b", 2, 5_000_000, 16 * 1024 * 1024);
    let wrapped_bundle = "globalThis.__slice_test_filter = \"\";\nvar __slice_modules = {};\nconst hasOwn = Object.prototype.hasOwnProperty.call({ a: 1 }, \"a\");\nconsole.log(String(hasOwn));\n";
    let wrapped_bc = compile_source_to_bytecode(wrapped_bundle);
    assert!(
        wrapped_bc.starts_with(slice_engine::BOACAS_MAGIC)
            || wrapped_bc.starts_with(b"SLICEBOA{\n")
    );
    let m2 = iso.execute_bytecode(&wrapped_bc, None).unwrap();
    assert_eq!(m2.stdout[0], "true");

    // 3. Verify framework suite structure under tests/frameworks/ (and full execution when node_modules is installed)
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let fw1_dir = repo_root
        .join("tests")
        .join("frameworks")
        .join("01-hono-edge-microservice");
    let fw2_dir = repo_root
        .join("tests")
        .join("frameworks")
        .join("02-itty-router-unjs-nitro-stack");

    assert!(fw1_dir.join("slice.toml").is_file());
    assert!(fw1_dir.join("src/index.ts").is_file());
    assert!(fw2_dir.join("slice.toml").is_file());
    assert!(fw2_dir.join("src/index.ts").is_file());

    if fw1_dir.join("node_modules/hono").is_dir() {
        let bundled1 =
            slice_cas::bundle_entrypoint_with_deps(&fw1_dir.join("src/index.ts")).unwrap();
        let bc1 = compile_source_to_bytecode(&bundled1);
        let mut iso1 = SliceIsolate::spawn(
            "wave-e-hono",
            1,
            CapabilityToken::pure_compute(),
            20_000_000,
            32 * 1024 * 1024,
        );
        let out1 = iso1.execute_bytecode(&bc1, None).unwrap();
        assert!(out1.stdout[0].contains(r#""framework":"01-hono-edge-microservice""#));
        assert!(out1.stdout[0].contains(r#""totalRequestsDispatched":12"#));
        assert!(out1.stdout[0].contains(r#""healthStatus":"healthy:Hono-v4""#));
    }

    if fw2_dir.join("node_modules/itty-router").is_dir() {
        let bundled2 =
            slice_cas::bundle_entrypoint_with_deps(&fw2_dir.join("src/index.ts")).unwrap();
        let bc2 = compile_source_to_bytecode(&bundled2);
        let mut iso2 = SliceIsolate::spawn(
            "wave-e-unjs",
            1,
            CapabilityToken::pure_compute(),
            20_000_000,
            32 * 1024 * 1024,
        );
        let out2 = iso2.execute_bytecode(&bc2, None).unwrap();
        assert!(out2.stdout[0].contains(r#""framework":"02-itty-router-unjs-nitro-stack""#));
        assert!(out2.stdout[0].contains(r#""totalRequestsDispatched":12"#));
        assert!(out2.stdout[0].contains(r#""status304Count":2"#));
    }
}

#[test]
fn test_wave_f_slice_init_generates_slice_d_ts_and_tsconfig() {
    let tmp_dir = env::temp_dir().join("slice_cli_test_wave_f_init");
    let _ = std::fs::remove_dir_all(&tmp_dir);

    cmd_init(&[tmp_dir.to_string_lossy().to_string()]).unwrap();

    assert!(tmp_dir.join("slice.toml").is_file());
    assert!(tmp_dir.join("slice.d.ts").is_file());
    assert!(tmp_dir.join("tsconfig.json").is_file());
    assert!(tmp_dir.join("src/index.ts").is_file());

    let dts = std::fs::read_to_string(tmp_dir.join("slice.d.ts")).unwrap();
    assert!(dts.contains("declare namespace Slice"));
    assert!(dts.contains("declare module \"slice:test\""));
    assert!(dts.contains("declare module \"slice:jsx\""));

    let tsconfig = std::fs::read_to_string(tmp_dir.join("tsconfig.json")).unwrap();
    assert!(tsconfig.contains("\"jsxImportSource\": \"slice:jsx\""));
    assert!(tsconfig.contains("\"slice.d.ts\""));

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_resolved_execution_config_subdirectory_manifest_and_cli_override_precedence() {
    use crate::config_and_watch::ResolvedExecutionConfig;

    let tmp_dir = env::temp_dir().join("slice_cli_test_resolved_exec_cfg");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let src_dir = tmp_dir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();

    let toml_content = r#"
[package]
name = "subdir-app"
version = "0.1.0"
entrypoint = "./src/index.ts"

[limits]
fuel = 250000000
memory_kb = 65536

[capabilities]
allow_read = ["/tmp", "."]
allow_write = ["/tmp"]
allow_net = ["api.example.com:443"]
allow_env = ["STRIPE_SECRET_KEY=sk_live_subdir_manifest_998877"]
"#;
    std::fs::write(tmp_dir.join("slice.toml"), toml_content).unwrap();
    let entry_path = src_dir.join("index.ts");
    std::fs::write(&entry_path, "console.log('ok');\n").unwrap();

    // 1. Without CLI `--fuel` / `--memory` overrides, `apply_target_path_manifest` must load
    //    both `[limits]` (`fuel = 250_000_000`, `memory_kb = 65536`) and `[capabilities]` + DLP secrets.
    let mut cfg = ResolvedExecutionConfig::from_cli_args(&[]).unwrap();
    cfg.apply_target_path_manifest(&entry_path);
    assert_eq!(cfg.fuel_limit, 250_000_000);
    assert_eq!(cfg.mem_limit, 65536 * 1024);
    assert!(cfg.caps.allowed_read_prefixes.contains(&"/tmp".to_string()));
    assert!(cfg.caps.allowed_net_hosts.contains("api.example.com:443"));
    assert!(cfg
        .audit_secrets
        .contains(&"sk_live_subdir_manifest_998877".to_string()));

    // 2. With explicit CLI `--fuel=` and `--memory=` flags, CLI overrides must win over `slice.toml` `[limits]`.
    let args = vec![
        "--fuel=777000".to_string(),
        "--memory=8388608".to_string(),
        "--allow-read=/custom".to_string(),
    ];
    let mut cfg_override = ResolvedExecutionConfig::from_cli_args(&args).unwrap();
    cfg_override.apply_target_path_manifest(&entry_path);
    assert_eq!(cfg_override.fuel_limit, 777_000);
    assert_eq!(cfg_override.mem_limit, 8_388_608);
    assert!(cfg_override
        .caps
        .allowed_read_prefixes
        .contains(&"/custom".to_string()));
    assert!(cfg_override
        .caps
        .allowed_read_prefixes
        .contains(&"/tmp".to_string()));

    // 3. Verify 4-layer precedence: Defaults (Tier 1, boacas=true) < slice.toml < Inline Directives < CLI Flags
    let inline_source = r#"// @slice-tier: tier1
// @slice-boacas: false
// @slice-warm: true
// @slice-deterministic: 9001
// @slice-fuel: 12000000
// @slice-memory-kb: 4096
// @slice-allow-read: ["/inline_read"]
// @slice-port: 4567
// @slice-workers: 3
console.log("hi");
"#;
    let mut cfg_inline = ResolvedExecutionConfig::from_cli_args(&[]).unwrap();
    assert!(slice_engine::is_exper_boacas_build_enabled());
    assert_eq!(slice_engine::active_tier_override(), None);
    let default_iso =
        SliceIsolate::spawn("tier-check", 1, CapabilityToken::pure_compute(), 1000, 1024);
    assert_eq!(
        default_iso.current_tier,
        slice_engine::ExecutionTier::Tier1Baseline
    );
    cfg_inline.apply_target_path_manifest(&entry_path);
    cfg_inline.apply_inline_directives(inline_source, Some(&src_dir));
    assert_eq!(slice_engine::active_tier_override(), Some(1));
    assert!(!slice_engine::is_exper_boacas_build_enabled());
    assert!(cfg_inline.warm_pool);
    assert_eq!(cfg_inline.deterministic_seed, Some(9001));
    assert_eq!(cfg_inline.fuel_limit, 12_000_000);
    assert_eq!(cfg_inline.mem_limit, 4096 * 1024);
    assert!(cfg_inline
        .caps
        .allowed_read_prefixes
        .contains(&"/inline_read".to_string()));
    assert_eq!(cfg_inline.server_port, Some(4567));
    assert_eq!(cfg_inline.server_workers, Some(3));

    // Explicit CLI flags win over inline directives
    let cli_win_args = vec![
        "--boacas".to_string(),
        "--fuel=99000000".to_string(),
        "--port=7777".to_string(),
    ];
    let mut cfg_cli_win = ResolvedExecutionConfig::from_cli_args(&cli_win_args).unwrap();
    cfg_cli_win.apply_target_path_manifest(&entry_path);
    cfg_cli_win.apply_inline_directives(inline_source, Some(&src_dir));
    assert!(slice_engine::is_exper_boacas_build_enabled());
    assert_eq!(cfg_cli_win.fuel_limit, 99_000_000);
    assert_eq!(cfg_cli_win.server_port, Some(7777));

    // Reset global overrides back to default and verify `compile_source_to_bytecode` honors inline `// @slice-boacas: false` vs default `SLICEBC\x01`
    slice_engine::set_exper_boacas_override(None);
    slice_engine::set_thread_exper_boacas_override(None);
    slice_engine::set_tier_override(None);
    slice_engine::set_thread_tier_override(None);
    let bc_default =
        compile_source_to_bytecode("class App { run() { return 1 + 2; } } console.log(new App().run());\n");
    assert!(bc_default.starts_with(slice_engine::BOACAS_MAGIC));
    let bc_no_boacas = compile_source_to_bytecode(
        "// @slice-boacas: false\nclass App { run() { return 1 + 2; } } console.log(new App().run());\n",
    );
    assert!(bc_no_boacas.starts_with(b"SLICEBOA"));
    let bc_tier1_forced =
        compile_source_to_bytecode("// @slice-tier: 1\nconsole.log(1 + 2);\n");
    assert!(bc_tier1_forced.starts_with(slice_engine::BOACAS_MAGIC));

    let _ = std::fs::remove_dir_all(&tmp_dir);
}
