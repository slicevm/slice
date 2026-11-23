use crate::config::{
    ResolvedExecutionConfig, build_cas_archive, parse_byte_size_usize, parse_strict_u64,
};
use crate::http_server::connection::write_all_vectored2;
use crate::http_server::{
    clear_tenant_cas_cache, notify_tenant_cas_updated, resolve_tenant_cas_hotswap,
    serve_connection_with_isolate, spawn_worker_isolate, tenant_cas_cache_len,
};
use crate::{
    LAST_CLI_EXIT_CODE, cmd_check, cmd_compile, cmd_doctor, cmd_init, cmd_inspect, cmd_mcp,
    cmd_microvm_bench, cmd_pack, cmd_run, cmd_serve, cmd_test,
};
use slice_cas::StaticAssetTable;
use slice_engine::{
    HttpRequestContext, SliceIsolate, WS_OPCODE_CLOSE, WS_OPCODE_TEXT, compile_source_to_bytecode,
    encode_websocket_frame, parse_close_frame_payload, read_websocket_message_from_stream,
};
use slice_sandbox::CapabilityToken;
use std::io::{IoSlice, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static WAVE5_TEMP_SEQ: AtomicU64 = AtomicU64::new(1);

fn wave5_temp_dir(prefix: &str) -> PathBuf {
    let seq = WAVE5_TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create wave5 temp dir");
    dir
}

#[test]
fn test_wave5_slice_init_tier1_e2e_run_test_pack_and_serve() {
    let proj_dir = wave5_temp_dir("slice_wave5_init_e2e");
    let proj_str = proj_dir.to_string_lossy().to_string();

    cmd_init(std::slice::from_ref(&proj_str)).expect("slice init default project");
    assert!(proj_dir.join("slice.toml").is_file());
    assert!(proj_dir.join("slice.d.ts").is_file());
    assert!(proj_dir.join("tsconfig.json").is_file());
    assert!(proj_dir.join("src/index.ts").is_file());
    assert!(proj_dir.join("tests/index.test.ts").is_file());

    let entry_src = std::fs::read_to_string(proj_dir.join("src/index.ts")).unwrap();
    assert!(entry_src.contains("export default"));
    assert!(entry_src.contains("process.env.SERVICE_STAGE"));

    // 1. Out-of-the-box `slice run`, `slice test`, `slice pack`, and `slice serve --smoke-test`
    cmd_run(std::slice::from_ref(&proj_str)).expect("slice run on scaffolded project");
    cmd_test(std::slice::from_ref(&proj_str)).expect("slice test on scaffolded project");

    let packed_cas = proj_dir.join("app.cas");
    let packed_str = packed_cas.to_string_lossy().to_string();
    cmd_pack(&[proj_str.clone(), packed_str.clone()]).expect("slice pack on scaffolded project");
    cmd_run(std::slice::from_ref(&packed_str)).expect("slice run packed.cas");
    cmd_serve(&[packed_str, "--smoke-test".to_string()]).expect("slice serve packed.cas smoke");

    // 2. Targetless `slice serve --smoke-test` under Tier-1 AND legacy `WinterTC.route` shim
    cmd_serve(&[
        "--tier".to_string(),
        "1".to_string(),
        "--smoke-test".to_string(),
    ])
    .expect("targetless slice serve --tier 1 --smoke-test");

    let legacy_ts = proj_dir.join("legacy.ts");
    std::fs::write(
        &legacy_ts,
        r#"const r: string = WinterTC.route("GET", "https://edge.slice.local/api/v1/status");
if (!r.includes("\"wintertc\":true")) throw new Error("bad route shim: " + r);
"#,
    )
    .unwrap();
    cmd_run(&[
        "--tier=1".to_string(),
        legacy_ts.to_string_lossy().to_string(),
    ])
    .expect("legacy WinterTC.route shim under Tier-1");

    let _ = std::fs::remove_dir_all(&proj_dir);
}

#[test]
fn test_wave5_http_500_on_uncaught_js_with_prior_stdout_and_syntax_error() {
    let dir = wave5_temp_dir("slice_wave5_http_500");
    let crash_src = r#"
console.log("step-1-logged-before-crash");
throw new Error("boom-uncaught-handler");
"#;
    let crash_bc = compile_source_to_bytecode(crash_src);

    let mut iso = SliceIsolate::spawn(
        "http-500-tenant",
        1,
        CapabilityToken::pure_compute(),
        500_000,
        8 * 1024 * 1024,
    );
    let req = HttpRequestContext {
        method: "GET",
        path: "/crash",
        query: "",
        tenant_id: "http-500-tenant",
        body: "",
        body_bytes: None,
        headers: &[],
    };

    let slow_out = iso
        .execute_http_request(&crash_bc, None, &req)
        .expect("uncaught is HTTP 500, not SandboxViolation");
    assert_eq!(slow_out.status_code, 500);
    assert!(
        slow_out
            .response_body
            .contains("Uncaught Error: boom-uncaught-handler")
    );
    let parsed_slow: serde_json::Value =
        serde_json::from_str(&slow_out.response_body).expect("valid JSON 500 error");
    assert!(
        parsed_slow["error"]
            .as_str()
            .unwrap()
            .contains("boom-uncaught-handler")
    );

    iso.reset_for_request("http-500-tenant", 1, 500_000, 8 * 1024 * 1024);
    let fast_out = iso
        .execute_http_request_fast(&crash_bc, None, &req)
        .expect("fast uncaught is HTTP 500");
    assert_eq!(fast_out.status_code, 500);
    assert!(
        fast_out
            .response_body
            .contains("Uncaught Error: boom-uncaught-handler")
    );

    // Syntax error (`SLICEERR` bytecode) must also return 500 Internal Server Error
    let syn_bc = compile_source_to_bytecode("const broken = ;");
    iso.reset_for_request("http-500-tenant", 1, 500_000, 8 * 1024 * 1024);
    let syn_out = iso
        .execute_http_request_fast(&syn_bc, None, &req)
        .expect("syntax error HTTP 500");
    assert_eq!(syn_out.status_code, 500);
    assert!(syn_out.response_body.contains("Uncaught SyntaxError"));

    // Live TCP `serve_connection_with_isolate` + `cmd_serve --smoke-test` fail-closed check
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let crash_bc_srv = crash_bc.clone();
    let srv_handle = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let caps = CapabilityToken::pure_compute();
        let mut srv_iso = spawn_worker_isolate(1, None, &caps, &[], 500_000, 8 * 1024 * 1024);
        let mut rbuf = vec![0u8; 16384];
        let mut wbuf = Vec::with_capacity(4096);
        serve_connection_with_isolate(
            stream,
            1,
            &mut srv_iso,
            &crash_bc_srv,
            None,
            None,
            None,
            500_000,
            8 * 1024 * 1024,
            &mut rbuf,
            &mut wbuf,
            None,
            None,
        )
        .unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    client
        .write_all(b"GET /crash HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut wire_resp = String::new();
    client.read_to_string(&mut wire_resp).unwrap();
    srv_handle.join().unwrap();
    assert!(
        wire_resp.starts_with("HTTP/1.1 500 Internal Server Error\r\n"),
        "expected HTTP 500 status line, got: {}",
        wire_resp
    );
    assert!(wire_resp.contains("boom-uncaught-handler"));

    let crash_file = dir.join("crash_server.ts");
    std::fs::write(&crash_file, crash_src).unwrap();
    let smoke_err = cmd_serve(&[
        crash_file.to_string_lossy().to_string(),
        "--smoke-test".to_string(),
    ])
    .expect_err("smoke-test must fail when handler returns 500");
    assert!(smoke_err.to_string().contains("expected 2 HTTP/1.1 200 OK"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_wave5_websocket_idle_beyond_10s_open_1008_violation_and_eof_close_event() {
    // 1. Upgraded WebSocket survives >10s idle period (HTTP 10s Slowloris read_timeout cleared)
    //    AND fires guest `"close"` (code 1006) when peer disconnects abruptly (EOF without Close frame)
    let ws_echo_bc = Arc::new(compile_source_to_bytecode(
        r#"
Slice.upgradeWebSocket({
    message(ws, msg) {
        ws.send("echo:" + msg);
    },
    close(_ws, code, _reason) {
        Slice.kv.set("ws_eof_close_code", String(code));
    }
});
"#,
    ));

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let bc_clone = Arc::clone(&ws_echo_bc);
    let srv = std::thread::spawn(move || -> Option<String> {
        let (stream, _) = listener.accept().unwrap();
        let caps = CapabilityToken::pure_compute();
        let mut iso = spawn_worker_isolate(1, None, &caps, &[], 2_000_000, 8 * 1024 * 1024);
        let mut rbuf = vec![0u8; 16384];
        let mut wbuf = Vec::with_capacity(4096);
        serve_connection_with_isolate(
            stream,
            1,
            &mut iso,
            &bc_clone,
            None,
            None,
            None,
            2_000_000,
            8 * 1024 * 1024,
            &mut rbuf,
            &mut wbuf,
            None,
            None,
        )
        .unwrap();
        let check_bc =
            compile_source_to_bytecode(r#"Slice.kv.get("ws_eof_close_code") ?? "missing""#);
        iso.reset_for_request("ws-idle-tenant", 1, 500_000, 8 * 1024 * 1024);
        Some(iso.execute_bytecode(&check_bc, None).unwrap().return_value)
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    client
        .write_all(
            b"GET /ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nX-Slice-Tenant: ws-idle-tenant\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n",
        )
        .unwrap();
    let mut hdr_buf = [0u8; 512];
    let n = client.read(&mut hdr_buf).unwrap();
    let handshake = std::str::from_utf8(&hdr_buf[..n]).unwrap();
    assert!(handshake.starts_with("HTTP/1.1 101 Switching Protocols\r\n"));

    // Sleep 10.4s (> 10.0s Slowloris timeout!) with zero frames sent
    std::thread::sleep(Duration::from_millis(10_400));

    let masked_ping = encode_websocket_frame(
        true,
        WS_OPCODE_TEXT,
        Some([0x12, 0x34, 0x56, 0x78]),
        b"alive-after-10s",
    );
    client.write_all(&masked_ping).unwrap();
    client.flush().unwrap();

    let mut pending = Vec::new();
    let (opcode, payload) =
        read_websocket_message_from_stream(&mut client, &mut pending, false, 65536)
            .unwrap()
            .expect("server must keep idle WebSocket open past 10s and reply");
    assert_eq!(opcode, WS_OPCODE_TEXT);
    assert_eq!(payload, b"echo:alive-after-10s");

    // Abruptly close client TCP socket without sending RFC 6455 Close frame -> server must fire `onClose(1006)`
    drop(client);
    let close_code_recorded = srv.join().unwrap().unwrap();
    assert_eq!(close_code_recorded, "1006");

    // 2. SandboxViolation inside WebSocket `"open"` (`onOpen` alias also verified) sends RFC 6455 `1008 Policy Violation` Close frame
    let ws_open_viol_bc = Arc::new(compile_source_to_bytecode(
        r#"
const fs = require("node:fs");
Slice.upgradeWebSocket({
    onOpen(_ws) {
        fs.readFileSync("/etc/shadow");
    }
});
"#,
    ));
    let listener2 = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port2 = listener2.local_addr().unwrap().port();
    let bc2 = Arc::clone(&ws_open_viol_bc);
    let srv2 = std::thread::spawn(move || {
        let (stream, _) = listener2.accept().unwrap();
        let caps = CapabilityToken::pure_compute();
        let mut iso = spawn_worker_isolate(1, None, &caps, &[], 2_000_000, 8 * 1024 * 1024);
        let mut rbuf = vec![0u8; 16384];
        let mut wbuf = Vec::with_capacity(4096);
        serve_connection_with_isolate(
            stream,
            1,
            &mut iso,
            &bc2,
            None,
            None,
            None,
            2_000_000,
            8 * 1024 * 1024,
            &mut rbuf,
            &mut wbuf,
            None,
            None,
        )
        .unwrap();
    });

    let mut client2 = TcpStream::connect(("127.0.0.1", port2)).unwrap();
    client2
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    client2
        .write_all(
            b"GET /ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n",
        )
        .unwrap();
    let mut raw2 = Vec::new();
    client2.read_to_end(&mut raw2).unwrap();
    srv2.join().unwrap();
    let split_idx = raw2
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("HTTP 101 header terminator")
        + 4;
    let mut ws_tail = raw2[split_idx..].to_vec();
    let mut empty_stream = std::io::Cursor::new(Vec::<u8>::new());
    let (close_op, close_payload) =
        read_websocket_message_from_stream(&mut empty_stream, &mut ws_tail, false, 65536)
            .unwrap()
            .expect("1008 Close frame on open SandboxViolation");
    assert_eq!(close_op, WS_OPCODE_CLOSE);
    let (code, reason) = parse_close_frame_payload(&close_payload);
    assert_eq!(code, 1008);
    assert_eq!(reason, "Policy Violation");
}

#[test]
fn test_wave5_watch_once_propagates_uncaught_errors() {
    let dir = wave5_temp_dir("slice_wave5_watch_once");
    let script = dir.join("app.ts");
    std::fs::write(&script, "console.log('initial-ok');\n").unwrap();

    let writer_path = script.clone();
    let writer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(180));
        std::fs::write(
            &writer_path,
            "console.log('reloading');\nthrow new Error('watch-reload-uncaught-boom');\n",
        )
        .unwrap();
    });

    let res = cmd_run(&[
        "--quiet".to_string(),
        "--watch-once".to_string(),
        script.to_string_lossy().to_string(),
    ]);
    writer.join().unwrap();

    let err = res.expect_err("--watch-once must fail closed on uncaught JS error during reload");
    assert!(
        err.to_string().contains("watch-reload-uncaught-boom"),
        "unexpected watch error: {}",
        err
    );
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 1);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_wave5_space_vs_equals_flag_parity_byte_units_and_fail_closed_validation() {
    // 1. Strict integer & byte-unit suffix parsers
    assert_eq!(parse_strict_u64("50_000_000"), Some(50_000_000));
    assert_eq!(parse_strict_u64("0"), Some(0));
    assert_eq!(parse_strict_u64("16MB"), None);
    assert_eq!(parse_strict_u64("bad"), None);

    assert_eq!(parse_byte_size_usize("1048576"), Some(1_048_576));
    assert_eq!(parse_byte_size_usize("64KB"), Some(65_536));
    assert_eq!(parse_byte_size_usize("64KiB"), Some(65_536));
    assert_eq!(parse_byte_size_usize("16MB"), Some(16 * 1024 * 1024));
    assert_eq!(parse_byte_size_usize("16 mib"), Some(16 * 1024 * 1024));
    assert_eq!(parse_byte_size_usize("1GB"), Some(1024 * 1024 * 1024));
    assert_eq!(parse_byte_size_usize("16TB"), None);
    assert_eq!(parse_byte_size_usize("bad"), None);

    // 2. Space-separated `--flag value` vs `--flag=value` parity + comma-split `--allow-read`/`--allow-write`
    let space_args: Vec<String> = [
        "--fuel",
        "123_456",
        "--memory",
        "16MB",
        "--port",
        "18443",
        "--workers",
        "3",
        "--static",
        "./pub",
        "--tls-cert",
        "./c.pem",
        "--tls-key",
        "./k.pem",
        "--allow-read",
        "/tmp/a,/tmp/b",
        "--allow-write",
        "/tmp/w1,/tmp/w2",
        "--allow-net",
        "api.example.com:443,cdn.example.com:443",
        "--allow-env",
        "A=1,B=2",
        "--max-checkpoints",
        "12",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    let eq_args: Vec<String> = [
        "--fuel=123_456",
        "--memory=16MB",
        "--port=18443",
        "--workers=3",
        "--static=./pub",
        "--tls-cert=./c.pem",
        "--tls-key=./k.pem",
        "--allow-read=/tmp/a,/tmp/b",
        "--allow-write=/tmp/w1,/tmp/w2",
        "--allow-net=api.example.com:443,cdn.example.com:443",
        "--allow-env=A=1,B=2",
        "--max-checkpoints=12",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    let cfg_space =
        ResolvedExecutionConfig::from_cli_args_with_ambient_cwd(&space_args, false).unwrap();
    let cfg_eq = ResolvedExecutionConfig::from_cli_args_with_ambient_cwd(&eq_args, false).unwrap();
    assert_eq!(cfg_space.fuel_limit, 123_456);
    assert_eq!(cfg_space.mem_limit, 16 * 1024 * 1024);
    assert_eq!(cfg_space.server_port, Some(18443));
    assert_eq!(cfg_space.server_workers, Some(3));
    assert_eq!(cfg_space.static_dir, Some(PathBuf::from("./pub")));
    assert_eq!(cfg_space.tls_cert, Some(PathBuf::from("./c.pem")));
    assert_eq!(cfg_space.tls_key, Some(PathBuf::from("./k.pem")));
    assert_eq!(cfg_space.caps.max_checkpoints, Some(12));
    assert_eq!(cfg_space.caps.allowed_read_prefixes.len(), 2);
    assert_eq!(cfg_space.caps.allowed_write_prefixes.len(), 2);
    assert_eq!(cfg_space.caps.allowed_net_hosts.len(), 2);
    assert_eq!(
        cfg_space.caps.allowed_env_keys.get("A").map(String::as_str),
        Some("1")
    );
    assert_eq!(
        cfg_space.caps.allowed_env_keys.get("B").map(String::as_str),
        Some("2")
    );
    assert_eq!(cfg_space.fuel_limit, cfg_eq.fuel_limit);
    assert_eq!(cfg_space.mem_limit, cfg_eq.mem_limit);
    assert_eq!(cfg_space.server_port, cfg_eq.server_port);
    assert_eq!(cfg_space.server_workers, cfg_eq.server_workers);

    // 3. Fail-closed validation on invalid numbers and unknown flags across commands
    for bad in [
        vec!["--fuel=bad"],
        vec!["--fuel", "bad"],
        vec!["--memory=bad"],
        vec!["--memory=16TB"],
        vec!["--memory-kb=16MB"],
        vec!["--port=99999"],
        vec!["--workers=0"],
        vec!["--tier=99"],
        vec!["--checkpoints=maybe"],
    ] {
        let owned: Vec<String> = bad.into_iter().map(String::from).collect();
        assert!(
            ResolvedExecutionConfig::from_cli_args_with_ambient_cwd(&owned, false).is_err(),
            "expected fail-closed error for {:?}",
            owned
        );
    }

    assert!(cmd_run(&["--unknown-flag".to_string()]).is_err());
    assert!(cmd_check(&["--unknown-flag".to_string()]).is_err());
    assert!(cmd_serve(&["--unknown-flag".to_string()]).is_err());
    assert!(cmd_test(&["--unknown-flag".to_string()]).is_err());
    assert!(cmd_mcp(&["--unknown-flag".to_string()]).is_err());
    assert!(
        cmd_pack(&[
            "a.ts".to_string(),
            "b.cas".to_string(),
            "--unknown-flag".to_string()
        ])
        .is_err()
    );
    assert!(cmd_compile(&["a.ts".to_string(), "--unknown-flag".to_string()]).is_err());
    assert!(cmd_inspect(&["--unknown-flag".to_string()]).is_err());
    assert!(cmd_microvm_bench(&["--tenants=bad".to_string()]).is_err());
    assert!(cmd_microvm_bench(&["--unknown-flag".to_string()]).is_err());

    // 4. `pack` `--entry <p>` / `--static <d>` space-separated & `doctor` / `mcp --smoke-test` assertions
    let dir = wave5_temp_dir("slice_wave5_pack_and_diag");
    std::fs::create_dir_all(dir.join("custom")).unwrap();
    std::fs::create_dir_all(dir.join("pub")).unwrap();
    std::fs::write(
        dir.join("custom/main.ts"),
        "console.log('pack-space-ok');\n",
    )
    .unwrap();
    std::fs::write(dir.join("pub/hello.txt"), "static-ok").unwrap();
    let out_cas = dir.join("packed.cas");
    cmd_pack(&[
        "--entry".to_string(),
        "./custom/main.ts".to_string(),
        "--static".to_string(),
        dir.join("pub").to_string_lossy().to_string(),
        dir.to_string_lossy().to_string(),
        out_cas.to_string_lossy().to_string(),
    ])
    .expect("slice pack with space-separated --entry and --static");
    assert!(out_cas.is_file());

    cmd_doctor().expect("slice doctor 5/5 containment check");
    cmd_mcp(&["--smoke-test".to_string()]).expect("slice mcp --smoke-test verified JSON-RPC");

    let _ = std::fs::remove_dir_all(&dir);
}

struct SmallChunkMockWriter {
    max_chunk: usize,
    written: Vec<u8>,
    vectored_calls: usize,
}

impl Write for SmallChunkMockWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let take = buf.len().min(self.max_chunk);
        self.written.extend_from_slice(&buf[..take]);
        Ok(take)
    }

    fn write_vectored(&mut self, bufs: &[IoSlice<'_>]) -> std::io::Result<usize> {
        self.vectored_calls += 1;
        let mut rem = self.max_chunk;
        let mut total = 0;
        for b in bufs {
            if rem == 0 {
                break;
            }
            let take = b.len().min(rem);
            self.written.extend_from_slice(&b[..take]);
            rem -= take;
            total += take;
        }
        Ok(total)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn test_wave5_tenant_dir_default_and_negative_cache_transient_copy_and_write_vectored() {
    let dir = wave5_temp_dir("slice_wave5_tenant_cache_and_writev");
    let tenant_dir = dir.join("tenants");
    std::fs::create_dir_all(&tenant_dir).unwrap();

    // 1. Negative lookup caching (`None` when neither `<tenant>.cas` nor `default.cas` exists)
    clear_tenant_cas_cache();
    assert!(resolve_tenant_cas_hotswap(&tenant_dir, "ghost-0").is_none());
    let neg_start = Instant::now();
    for i in 0..5_000 {
        assert!(resolve_tenant_cas_hotswap(&tenant_dir, &format!("ghost-{}", i)).is_none());
    }
    let neg_ns_per_lookup = neg_start.elapsed().as_nanos() as f64 / 5_000.0;
    assert_eq!(tenant_cas_cache_len(), (0, 0));
    assert!(
        neg_ns_per_lookup < 5_000.0,
        "negative lookup cache too slow: {:.1} ns/op",
        neg_ns_per_lookup
    );

    // 2. `default.cas` lookup caching across unknown tenants (`0` `by_tenant` growth, `< 5 us` / op in debug)
    let default_src = dir.join("default.ts");
    std::fs::write(
        &default_src,
        "Slice.respond(200, 'default-v1', 'text/plain');\n",
    )
    .unwrap();
    let default_cas = tenant_dir.join("default.cas");
    build_cas_archive(&default_src, &default_cas, "./default.ts").unwrap();
    notify_tenant_cas_updated();

    let (first_bc, _) =
        resolve_tenant_cas_hotswap(&tenant_dir, "unknown-0").expect("default.cas hit");
    for i in 1..2_000 {
        let (bc, _) = resolve_tenant_cas_hotswap(&tenant_dir, &format!("unknown-{}", i))
            .expect("cached default.cas");
        assert!(Arc::ptr_eq(&first_bc, &bc));
    }
    assert_eq!(tenant_cas_cache_len(), (0, 1));

    // 3. Dedicated `<tenant>.cas` transient mid-copy open resilience + clean fallback on delete
    let alpha_v1_src = dir.join("alpha_v1.ts");
    std::fs::write(
        &alpha_v1_src,
        "Slice.respond(200, 'alpha-v1', 'text/plain');\n",
    )
    .unwrap();
    let alpha_cas = tenant_dir.join("alpha.cas");
    build_cas_archive(&alpha_v1_src, &alpha_cas, "./alpha_v1.ts").unwrap();
    notify_tenant_cas_updated();

    let (alpha_v1_bc, _) = resolve_tenant_cas_hotswap(&tenant_dir, "alpha").expect("alpha v1");
    assert!(!Arc::ptr_eq(&alpha_v1_bc, &first_bc));

    // Simulate non-atomic file copy mid-write (truncated invalid `.cas` bytes on disk)
    std::fs::write(&alpha_cas, b"SLICE_PARTIAL_INCOMPLETE_WRITE").unwrap();
    notify_tenant_cas_updated();
    let (during_copy_bc, _) = resolve_tenant_cas_hotswap(&tenant_dir, "alpha")
        .expect("must retain last-known-good Arc during transient partial .cas write");
    assert!(Arc::ptr_eq(&alpha_v1_bc, &during_copy_bc));

    // Finish atomic/complete write of v2 -> hot-swaps to v2!
    let alpha_v2_src = dir.join("alpha_v2.ts");
    std::fs::write(
        &alpha_v2_src,
        "Slice.respond(200, 'alpha-v2-updated', 'text/plain');\n",
    )
    .unwrap();
    build_cas_archive(&alpha_v2_src, &alpha_cas, "./alpha_v2.ts").unwrap();
    notify_tenant_cas_updated();
    let (alpha_v2_bc, _) = resolve_tenant_cas_hotswap(&tenant_dir, "alpha").expect("alpha v2");
    assert!(!Arc::ptr_eq(&alpha_v1_bc, &alpha_v2_bc));

    // Delete `alpha.cas` -> transitions cleanly to `default.cas`!
    std::fs::remove_file(&alpha_cas).unwrap();
    notify_tenant_cas_updated();
    let (after_del_bc, _) =
        resolve_tenant_cas_hotswap(&tenant_dir, "alpha").expect("falls back to default.cas");
    assert!(Arc::ptr_eq(&first_bc, &after_del_bc));

    // 4. `write_all_vectored2` partial-write advancement + 128 KB static & dynamic HTTP responses
    let mut mock_w = SmallChunkMockWriter {
        max_chunk: 7,
        written: Vec::new(),
        vectored_calls: 0,
    };
    write_all_vectored2(&mut mock_w, b"HTTP/1.1 200 OK\r\n\r\n", b"0123456789abcdef").unwrap();
    assert_eq!(mock_w.written, b"HTTP/1.1 200 OK\r\n\r\n0123456789abcdef");
    assert!(mock_w.vectored_calls >= 2);

    let pub_dir = dir.join("public");
    std::fs::create_dir_all(&pub_dir).unwrap();
    let big_static = "S".repeat(131_072);
    std::fs::write(pub_dir.join("big.txt"), &big_static).unwrap();
    let mut static_table = StaticAssetTable::new();
    static_table.extend_from_directory(&pub_dir).unwrap();
    let static_arc = Arc::new(static_table);

    let dyn_bc = Arc::new(compile_source_to_bytecode(
        r#"Slice.respond(200, "D".repeat(131072), "text/plain");"#,
    ));

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let st_srv = Arc::clone(&static_arc);
    let bc_srv = Arc::clone(&dyn_bc);
    let srv = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let caps = CapabilityToken::pure_compute();
        let mut iso = spawn_worker_isolate(1, None, &caps, &[], 5_000_000, 16 * 1024 * 1024);
        let mut rbuf = vec![0u8; 16384];
        let mut wbuf = Vec::with_capacity(4096);
        serve_connection_with_isolate(
            stream,
            1,
            &mut iso,
            &bc_srv,
            None,
            None,
            Some(&st_srv),
            5_000_000,
            16 * 1024 * 1024,
            &mut rbuf,
            &mut wbuf,
            None,
            None,
        )
        .unwrap();
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    client
        .write_all(b"GET /big.txt HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\n\r\n")
        .unwrap();
    // Read first response (`131072` S bytes) then send dynamic request (`131072` D bytes)
    client
        .write_all(b"GET /dyn HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut full_resp = String::new();
    client.read_to_string(&mut full_resp).unwrap();
    srv.join().unwrap();
    assert_eq!(full_resp.matches("HTTP/1.1 200 OK").count(), 2);
    assert!(full_resp.contains(&big_static));
    assert!(full_resp.contains(&"D".repeat(131_072)));

    let _ = std::fs::remove_dir_all(&dir);
}
