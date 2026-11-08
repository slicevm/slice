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
