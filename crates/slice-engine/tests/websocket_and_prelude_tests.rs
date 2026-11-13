#![allow(unused_imports)]
use crate::intrinsics::{
    digest_to_hex, fast_format_bytes, fast_format_ms, fast_parse_bytes, fast_parse_ms,
    hmac_sha256_hex, sha256_hex, Sha256State,
};
use crate::{
    compile_source_to_bytecode, disassemble_bytecode, run_source_isolated, ExecutionTier,
    HttpRequestContext, JsValue, SliceIsolate,
};
use slice_sandbox::{CapabilityToken, SandboxViolation};

#[test]
fn test_wave_b_network_security_hardening_and_rfc6455_websocket_engine() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    // 1. Verify RFC 6455 Section 1.3 official Sec-WebSocket-Accept test vector
    assert_eq!(
        crate::compute_websocket_accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
        "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
    );

    // 2. Verify RFC 6455 frame encode/decode across 7-bit, 16-bit, and 64-bit lengths + 4-byte XOR masking
    for len in [0usize, 11, 125, 126, 1024, 65536] {
        let payload: Vec<u8> = (0..len).map(|i| (i * 37) as u8).collect();
        let mask = [0x37, 0xfa, 0x21, 0x3d];
        let encoded =
            crate::encode_websocket_frame(true, crate::WS_OPCODE_BINARY, Some(mask), &payload);
        let decoded = crate::decode_websocket_frame(&encoded, crate::MAX_WS_FRAME_BYTES)
            .unwrap()
            .unwrap();
        assert!(decoded.fin);
        assert_eq!(decoded.opcode, crate::WS_OPCODE_BINARY);
        assert!(decoded.masked);
        assert_eq!(decoded.payload, payload);
        assert_eq!(decoded.bytes_consumed, encoded.len());
    }

    // Reject non-zero RSV bits and oversized frames before allocation
    let bad_rsv = [0x91u8, 0x00];
    assert!(crate::decode_websocket_frame(&bad_rsv, 1024).is_err());
    let oversized_hdr =
        crate::encode_websocket_frame(true, crate::WS_OPCODE_TEXT, None, &[b'a'; 200]);
    assert!(crate::decode_websocket_frame(&oversized_hdr, 64)
        .unwrap_err()
        .contains("MemoryCeilingExceeded"));

    // 3. Verify N-Sec-2 (CRLF / tchar header validators) and N-Sec-5 (checked chunked body parser)
    assert!(crate::is_valid_http_header_name("X-Custom-Token"));
    assert!(!crate::is_valid_http_header_name("X-Bad\r\nHeader"));
    assert!(!crate::is_valid_http_header_name("X Bad"));
    assert!(crate::is_valid_http_header_value("Bearer abc.123"));
    assert!(!crate::is_valid_http_header_value("val\r\nX-Injected: 1"));
    assert!(!crate::is_valid_request_target("/api/v1/test HTTP/1.1\r\n"));

    let valid_chunked = b"5;ext=1\r\nhello\r\n6\r\n-world\r\n0\r\n\r\n";
    let (decoded_chunks, consumed) = crate::parse_chunked_body(valid_chunked, 1024)
        .unwrap()
        .unwrap();
    assert_eq!(decoded_chunks, b"hello-world");
    assert_eq!(consumed, valid_chunked.len());

    let bad_hex_chunk = b"ZZZZ\r\nhello\r\n0\r\n\r\n";
    assert!(crate::parse_chunked_body(bad_hex_chunk, 1024).is_err());
    let missing_crlf_chunk = b"5\r\nhelloXX0\r\n\r\n";
    assert!(crate::parse_chunked_body(missing_crlf_chunk, 1024).is_err());

    // 4. Verify N-Sec-1: Outbound HTTP response exceeding tenant memory budget traps with MemoryCeilingExceeded
    let bomb_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let bomb_port = bomb_listener.local_addr().unwrap().port();
    let bomb_thread = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = bomb_listener.accept() {
            let mut req_buf = [0u8; 1024];
            let _ = stream.read(&mut req_buf);
            let resp = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 1048576\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    let bomb_caps =
        CapabilityToken::pure_compute().with_net_host(format!("127.0.0.1:{}", bomb_port));
    let bomb_js = format!(
        r#"async function main() {{ await fetch("http://127.0.0.1:{}/bomb"); }} main();"#,
        bomb_port
    );
    let bomb_err = run_source_isolated(
        "nsec1-bomb-tenant",
        &bomb_js,
        bomb_caps,
        1_000_000,
        64 * 1024, // 64 KB tenant memory limit vs 1 MB Content-Length
    )
    .unwrap_err();
    assert!(
        bomb_err.to_string().contains("exceeded memory ceiling"),
        "Expected MemoryCeilingExceeded on 1MB response into 64KB tenant, got: {}",
        bomb_err
    );
    let _ = bomb_thread.join();

    // 5. Verify Live RFC 6455 Client WebSocket (`ws://`) + Ping/Pong + Binary Uint8Array + Close
    let ws_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let ws_port = ws_listener.local_addr().unwrap().port();
    let ws_server = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = ws_listener.accept() {
            let mut req_buf = [0u8; 2048];
            let n = stream.read(&mut req_buf).unwrap();
            let req_str = String::from_utf8_lossy(&req_buf[..n]);
            let key = req_str
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    if k.trim().eq_ignore_ascii_case("sec-websocket-key") {
                        Some(v.trim().to_string())
                    } else {
                        None
                    }
                })
                .expect("Missing Sec-WebSocket-Key from client");
            let accept = crate::compute_websocket_accept_key(&key);
            let handshake = format!(
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\nSec-WebSocket-Protocol: chat.v1\r\n\r\n",
                accept
            );
            stream.write_all(handshake.as_bytes()).unwrap();
            stream.flush().unwrap();

            let mut pending = Vec::new();
            // 1) Read text frame from client, send a PING first, verify client PONG, then send text reply
            let (op1, msg1) =
                crate::read_websocket_message_from_stream(&mut stream, &mut pending, true, 65536)
                    .unwrap()
                    .unwrap();
            assert_eq!(op1, crate::WS_OPCODE_TEXT);
            assert_eq!(msg1, b"hello-slice-ws");

            let ping_frame =
                crate::encode_websocket_frame(true, crate::WS_OPCODE_PING, None, b"hb-1");
            stream.write_all(&ping_frame).unwrap();
            let reply_text = crate::encode_websocket_frame(
                true,
                crate::WS_OPCODE_TEXT,
                None,
                b"ack:hello-slice-ws",
            );
            stream.write_all(&reply_text).unwrap();
            stream.flush().unwrap();

            // 2) Read binary frame from client (`read_websocket_message_from_stream` consumes client's PONG automatically)
            let (op2, msg2) =
                crate::read_websocket_message_from_stream(&mut stream, &mut pending, true, 65536)
                    .unwrap()
                    .unwrap();
            assert_eq!(op2, crate::WS_OPCODE_BINARY);
            assert_eq!(msg2, vec![10, 20, 30, 40]);

            let reply_bin = crate::encode_websocket_frame(
                true,
                crate::WS_OPCODE_BINARY,
                None,
                &[40, 30, 20, 10],
            );
            stream.write_all(&reply_bin).unwrap();
            stream.flush().unwrap();

            // 3) Read CLOSE frame from client and echo CLOSE
            let (op3, close_payload) =
                crate::read_websocket_message_from_stream(&mut stream, &mut pending, true, 65536)
                    .unwrap()
                    .unwrap();
            assert_eq!(op3, crate::WS_OPCODE_CLOSE);
            let close_reply =
                crate::encode_websocket_frame(true, crate::WS_OPCODE_CLOSE, None, &close_payload);
            let _ = stream.write_all(&close_reply);
            let _ = stream.flush();
        }
    });

    let ws_client_js = format!(
        r#"
const events = [];
const ws = new WebSocket("ws://127.0.0.1:{ws_port}/realtime", ["chat.v1"]);
ws.binaryType = "arraybuffer";

ws.onopen = () => {{
    events.push("open:" + ws.protocol);
    ws.send("hello-slice-ws");
    ws.send(new Uint8Array([10, 20, 30, 40]));
    ws.close(1000, "normal-done");
}};

ws.onmessage = (ev) => {{
    if (typeof ev.data === "string") {{
        events.push("text:" + ev.data);
    }} else if (ev.data instanceof ArrayBuffer) {{
        const u8 = new Uint8Array(ev.data);
        events.push("bin:" + Array.from(u8).join(","));
    }}
}};

ws.onclose = (ev) => {{
    events.push("close:" + ev.code + ":" + ev.reason);
    console.log(JSON.stringify(events));
}};
"#,
        ws_port = ws_port
    );
    let ws_caps = CapabilityToken::pure_compute().with_net_host(format!("127.0.0.1:{}", ws_port));
    let ws_metrics = run_source_isolated(
        "wave-b-ws-client",
        &ws_client_js,
        ws_caps.clone(),
        2_000_000,
        4 * 1024 * 1024,
    )
    .unwrap();
    let _ = ws_server.join();

    let ws_events: Vec<String> = serde_json::from_str(&ws_metrics.stdout[0]).unwrap();
    assert_eq!(
        ws_events,
        vec![
            "open:chat.v1",
            "text:ack:hello-slice-ws",
            "bin:40,30,20,10",
            "close:1000:normal-done"
        ]
    );

    // 6. Verify WebSocket Outbound Egress Secret DLP traps when sending raw or Base64-encoded secrets over `ws.send()`
    let dlp_ws_js = r#"
Slice.mockWebSocket("ws://stream.partner.example.com/ingest", "echo");
const ws = new WebSocket("ws://stream.partner.example.com/ingest");
ws.send(btoa("sk_live_WS_SECRET_TOKEN_777"));
"#;
    let bc_ws_dlp = compile_source_to_bytecode(dlp_ws_js);
    let mut iso_ws_dlp = SliceIsolate::spawn(
        "wave-b-ws-dlp",
        10,
        CapabilityToken::pure_compute().with_net_host("stream.partner.example.com:80"),
        1_000_000,
        4 * 1024 * 1024,
    );
    iso_ws_dlp.add_egress_audit_secret("sk_live_WS_SECRET_TOKEN_777");
    let ws_dlp_err = iso_ws_dlp.execute_bytecode(&bc_ws_dlp, None).unwrap_err();
    assert!(
        ws_dlp_err
            .to_string()
            .contains("EGRESS_SECRET_LEAK_BLOCKED"),
        "Expected EGRESS_SECRET_LEAK_BLOCKED on WebSocket secret frame exfiltration, got: {}",
        ws_dlp_err
    );
}

#[test]
fn test_wave_b_nsec7_cross_tenant_outbound_connection_pool_isolation_and_nsec2_header_guards() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    // 1. N-Sec-7: Verify that two tenants hitting the exact same upstream origin NEVER share
    // an idle Keep-Alive socket, while same-tenant sequential requests DO reuse their own socket.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let accept_count = Arc::new(AtomicUsize::new(0));
    let accept_clone = Arc::clone(&accept_count);

    let srv = std::thread::spawn(move || {
        // Expect 2 distinct TCP connections (1 for tenant-alpha, 1 for tenant-beta),
        // each serving 2 Keep-Alive requests.
        for conn_idx in 1..=2 {
            if let Ok((mut stream, _)) = listener.accept() {
                accept_clone.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 2048];
                for req_idx in 1..=2 {
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    let body = format!("{{\"conn\":{},\"req\":{}}}", conn_idx, req_idx);
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes());
                    let _ = stream.flush();
                }
            }
        }
    });

    let js = format!(
        r#"
async function main() {{
    const r1 = await (await fetch("http://127.0.0.1:{port}/ping?step=1")).json();
    const r2 = await (await fetch("http://127.0.0.1:{port}/ping?step=2")).json();
    console.log(JSON.stringify([r1, r2]));
}}
main();
"#,
        port = port
    );
    let caps = CapabilityToken::pure_compute().with_net_host(format!("127.0.0.1:{}", port));

    crate::clear_outbound_conn_pool_for_tenant("tenant-alpha");
    crate::clear_outbound_conn_pool_for_tenant("tenant-beta");

    let m_alpha = run_source_isolated(
        "tenant-alpha",
        &js,
        caps.clone(),
        2_000_000,
        4 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(
        m_alpha.stdout[0],
        r#"[{"conn":1,"req":1},{"conn":1,"req":2}]"#
    );
    assert_eq!(accept_count.load(Ordering::SeqCst), 1);

    let m_beta = run_source_isolated("tenant-beta", &js, caps, 2_000_000, 4 * 1024 * 1024).unwrap();
    assert_eq!(
        m_beta.stdout[0],
        r#"[{"conn":2,"req":1},{"conn":2,"req":2}]"#
    );
    assert_eq!(accept_count.load(Ordering::SeqCst), 2);
    let _ = srv.join();
    crate::clear_outbound_conn_pool_for_tenant("tenant-alpha");
    crate::clear_outbound_conn_pool_for_tenant("tenant-beta");

    // 2. N-Sec-2: Verify outbound `fetch()` rejects CRLF header injection on live outbound sockets
    let bad_hdr_js = format!(
        r#"
async function main() {{
    try {{
        await fetch("http://127.0.0.1:{port}/test", {{
            headers: {{ "x-safe": "ok\r\nX-Injected: pwned" }}
        }});
        console.log("UNEXPECTED_SUCCESS");
    }} catch (e) {{
        console.log("BLOCKED:" + String(e));
    }}
}}
main();
"#,
        port = port
    );
    let m_hdr = run_source_isolated(
        "tenant-nsec2",
        &bad_hdr_js,
        CapabilityToken::pure_compute().with_net_host(format!("127.0.0.1:{}", port)),
        1_000_000,
        4 * 1024 * 1024,
    )
    .unwrap();
    assert!(
        m_hdr.stdout[0].starts_with("BLOCKED:"),
        "Expected outbound header injection to be blocked, got: {}",
        m_hdr.stdout[0]
    );

    // 3. N-Sec-8 + WebSocket Pre-Connect SSRF Firewall:
    //    - Cloud metadata IP (`ws://169.254.169.254/latest`) -> SSRF_METADATA_BLOCKED
    //    - Non-canonical octal loopback (`ws://0177.0.0.1:8080/ws`) under wildcard `allow_net = ["*"]` -> blocked
    let ws_ssrf_meta_js = r#"const ws = new WebSocket("ws://169.254.169.254/latest/meta-data");"#;
    let err_meta = run_source_isolated(
        "tenant-ws-ssrf-meta",
        ws_ssrf_meta_js,
        CapabilityToken::pure_compute().with_net_host("*"),
        1_000_000,
        4 * 1024 * 1024,
    )
    .unwrap_err();
    assert!(
        err_meta.to_string().contains("SSRF_METADATA_BLOCKED"),
        "Expected SSRF_METADATA_BLOCKED for WebSocket metadata target, got: {}",
        err_meta
    );

    let ws_ssrf_octal_js = r#"const ws = new WebSocket("ws://0177.0.0.1:8080/internal");"#;
    let err_octal = run_source_isolated(
        "tenant-ws-ssrf-octal",
        ws_ssrf_octal_js,
        CapabilityToken::pure_compute().with_net_host("*"),
        1_000_000,
        4 * 1024 * 1024,
    )
    .unwrap_err();
    assert!(
        err_octal
            .to_string()
            .contains("lacks outbound network authority for host '0177.0.0.1:8080'"),
        "Expected NetAccessDenied for octal loopback WebSocket under wildcard allow_net, got: {}",
        err_octal
    );
}

#[test]
fn test_wave_b_rfc6455_fragmented_continuation_interleaved_ping_and_frame_limits() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    // 1. Verify extended 16-bit (126) and 64-bit (127) payload lengths + 4-byte u32 XOR masking
    let mask = [0x12, 0x34, 0x56, 0x78];
    let payload_16bit = vec![0xABu8; 2048];
    let encoded_16 =
        crate::encode_websocket_frame(true, crate::WS_OPCODE_BINARY, Some(mask), &payload_16bit);
    let decoded_16 = crate::decode_websocket_frame(&encoded_16, 8192)
        .unwrap()
        .unwrap();
    assert_eq!(decoded_16.opcode, crate::WS_OPCODE_BINARY);
    assert_eq!(decoded_16.payload.len(), 2048);
    assert!(decoded_16.payload.iter().all(|&b| b == 0xAB));

    let payload_64bit = vec![0x5Cu8; 70_000];
    let encoded_64 =
        crate::encode_websocket_frame(true, crate::WS_OPCODE_BINARY, Some(mask), &payload_64bit);
    let decoded_64 = crate::decode_websocket_frame(&encoded_64, 100_000)
        .unwrap()
        .unwrap();
    assert_eq!(decoded_64.opcode, crate::WS_OPCODE_BINARY);
    assert_eq!(decoded_64.payload.len(), 70_000);
    assert!(decoded_64.payload.iter().all(|&b| b == 0x5C));

    // 2. Verify RFC 6455 protocol violation checks:
    //    - Non-zero RSV bits (`0x40` RSV1 set) -> Err
    let bad_rsv = [0x81 | 0x40, 0x01, b'a'];
    assert!(crate::decode_websocket_frame(&bad_rsv, 1024)
        .unwrap_err()
        .contains("non-zero RSV bits"));
    //    - Fragmented control frame (`FIN=0` with opcode `0x9` PING) -> Err
    let frag_ping = [0x09, 0x02, b'h', b'i'];
    assert!(crate::decode_websocket_frame(&frag_ping, 1024)
        .unwrap_err()
        .contains("fragmented control frame"));
    //    - Control frame > 125 bytes -> Err
    let mut huge_ping = vec![0x89, 126, 0x00, 126];
    huge_ping.extend_from_slice(&[b'x'; 126]);
    assert!(crate::decode_websocket_frame(&huge_ping, 1024)
        .unwrap_err()
        .contains("exceeds 125 bytes"));

    // 3. Verify live Client WebSocket reassembling multi-frame fragmented messages (`FIN=0` + interleaved `PING` + `0x0` continuation + `FIN=1`)
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let srv = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut req_buf = [0u8; 2048];
            let n = stream.read(&mut req_buf).unwrap();
            let req_str = String::from_utf8_lossy(&req_buf[..n]);
            let key = req_str
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    if k.trim().eq_ignore_ascii_case("sec-websocket-key") {
                        Some(v.trim().to_string())
                    } else {
                        None
                    }
                })
                .unwrap();
            let accept = crate::compute_websocket_accept_key(&key);
            let handshake = format!(
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
                accept
            );
            stream.write_all(handshake.as_bytes()).unwrap();
            stream.flush().unwrap();

            let mut pending = Vec::new();
            // Read client trigger message
            let (op, msg) =
                crate::read_websocket_message_from_stream(&mut stream, &mut pending, true, 65536)
                    .unwrap()
                    .unwrap();
            assert_eq!(op, crate::WS_OPCODE_TEXT);
            assert_eq!(msg, b"send-fragmented");

            // Send 3 fragments of a single text message with an interleaved PING in the middle!
            let frag1 =
                crate::encode_websocket_frame(false, crate::WS_OPCODE_TEXT, None, b"part1-");
            let ping =
                crate::encode_websocket_frame(true, crate::WS_OPCODE_PING, None, b"mid-frag-ping");
            let frag2 = crate::encode_websocket_frame(
                false,
                crate::WS_OPCODE_CONTINUATION,
                None,
                b"part2-",
            );
            let frag3 =
                crate::encode_websocket_frame(true, crate::WS_OPCODE_CONTINUATION, None, b"part3!");
            let mut combined = Vec::new();
            combined.extend_from_slice(&frag1);
            combined.extend_from_slice(&ping);
            combined.extend_from_slice(&frag2);
            combined.extend_from_slice(&frag3);
            stream.write_all(&combined).unwrap();
            stream.flush().unwrap();

            // Read client close (which also drains the auto-PONG sent by the client during fragmentation)
            let (op_close, close_payload) =
                crate::read_websocket_message_from_stream(&mut stream, &mut pending, true, 65536)
                    .unwrap()
                    .unwrap();
            assert_eq!(op_close, crate::WS_OPCODE_CLOSE);
            let close_reply =
                crate::encode_websocket_frame(true, crate::WS_OPCODE_CLOSE, None, &close_payload);
            let _ = stream.write_all(&close_reply);
            let _ = stream.flush();
        }
    });

    let client_js = format!(
        r#"
const received = [];
const ws = new WebSocket("ws://127.0.0.1:{port}/frag");
ws.onopen = () => {{
    ws.send("send-fragmented");
    ws.close(1000, "ok");
}};
ws.onmessage = (ev) => {{
    received.push(ev.data);
}};
ws.onclose = (ev) => {{
    received.push("close:" + ev.code);
    console.log(JSON.stringify(received));
}};
"#,
        port = port
    );
    let m = run_source_isolated(
        "tenant-ws-frag",
        &client_js,
        CapabilityToken::pure_compute().with_net_host(format!("127.0.0.1:{}", port)),
        2_000_000,
        4 * 1024 * 1024,
    )
    .unwrap();
    let _ = srv.join();
    assert_eq!(m.stdout[0], r#"["part1-part2-part3!","close:1000"]"#);
}

#[test]
fn test_compile_time_prelude_minifier_and_lazy_sharded_globals() {
    use crate::js_minifier::minify_js_source;
    use crate::js_preludes::{
        BLOB_FORMDATA_PRELUDE, BLOB_FORMDATA_PRELUDE_SRC, BUFFER_BINARY_RW_PRELUDE,
        BUFFER_BINARY_RW_PRELUDE_SRC, CONSOLE_EXT_PRELUDE, CONSOLE_EXT_PRELUDE_SRC,
        DOM_EVENTS_PRELUDE, DOM_EVENTS_PRELUDE_SRC, ENCODING_EVENTS_PRELUDE,
        ENCODING_EVENTS_PRELUDE_SRC, FETCH_HEADERS_REQ_RES_PRELUDE,
        FETCH_HEADERS_REQ_RES_PRELUDE_SRC, FULL_BUFFER_PRELUDE, FULL_BUFFER_PRELUDE_SRC,
        INTL_PRELUDE, INTL_PRELUDE_SRC, OUTBOUND_FETCH_SSE_PRELUDE, OUTBOUND_FETCH_SSE_PRELUDE_SRC,
        PROCESS_CLI_PRELUDE, PROCESS_CLI_PRELUDE_SRC, STREAMS_FORMDATA_PRELUDE,
        STREAMS_FORMDATA_PRELUDE_SRC, STRUCTURED_CLONE_PRELUDE, STRUCTURED_CLONE_PRELUDE_SRC,
        TRANSFORM_COMPRESSION_STREAMS_PRELUDE, TRANSFORM_COMPRESSION_STREAMS_PRELUDE_SRC,
        ULTRA_CORE_PRELUDE, ULTRA_CORE_PRELUDE_SRC, URL_PATTERN_PRELUDE, URL_PATTERN_PRELUDE_SRC,
        WASM_PRELUDE, WASM_PRELUDE_SRC, WEBCRYPTO_CLONE_PRELUDE, WEBCRYPTO_CLONE_PRELUDE_SRC,
        WEBSOCKET_PRELUDE, WEBSOCKET_PRELUDE_SRC, WEB_FETCH_URL_PRELUDE, WEB_FETCH_URL_PRELUDE_SRC,
        WORKERS_SOCKETS_PRELUDE, WORKERS_SOCKETS_PRELUDE_SRC,
    };

    // 1. Verify all 9 readable `.js` prelude sources are preserved and compile-time minified
    let pairs: &[(&str, &str, &[u8])] = &[
        (
            "00_ultra_core.js",
            ULTRA_CORE_PRELUDE_SRC,
            ULTRA_CORE_PRELUDE,
        ),
        (
            "01_web_fetch_url.js",
            WEB_FETCH_URL_PRELUDE_SRC,
            WEB_FETCH_URL_PRELUDE,
        ),
        (
            "02_encoding_events.js",
            ENCODING_EVENTS_PRELUDE_SRC,
            ENCODING_EVENTS_PRELUDE,
        ),
        (
            "03_streams_formdata.js",
            STREAMS_FORMDATA_PRELUDE_SRC,
            STREAMS_FORMDATA_PRELUDE,
        ),
        (
            "04_webcrypto_clone.js",
            WEBCRYPTO_CLONE_PRELUDE_SRC,
            WEBCRYPTO_CLONE_PRELUDE,
        ),
        (
            "05_full_buffer.js",
            FULL_BUFFER_PRELUDE_SRC,
            FULL_BUFFER_PRELUDE,
        ),
        (
            "06_console_ext.js",
            CONSOLE_EXT_PRELUDE_SRC,
            CONSOLE_EXT_PRELUDE,
        ),
        ("07_websocket.js", WEBSOCKET_PRELUDE_SRC, WEBSOCKET_PRELUDE),
        (
            "08_workers_sockets.js",
            WORKERS_SOCKETS_PRELUDE_SRC,
            WORKERS_SOCKETS_PRELUDE,
        ),
        ("09_wasm.js", WASM_PRELUDE_SRC, WASM_PRELUDE),
        (
            "10_outbound_fetch_sse.js",
            OUTBOUND_FETCH_SSE_PRELUDE_SRC,
            OUTBOUND_FETCH_SSE_PRELUDE,
        ),
        (
            "11_fetch_headers_req_res.js",
            FETCH_HEADERS_REQ_RES_PRELUDE_SRC,
            FETCH_HEADERS_REQ_RES_PRELUDE,
        ),
        (
            "12_url_pattern.js",
            URL_PATTERN_PRELUDE_SRC,
            URL_PATTERN_PRELUDE,
        ),
        (
            "13_dom_events.js",
            DOM_EVENTS_PRELUDE_SRC,
            DOM_EVENTS_PRELUDE,
        ),
        (
            "14_transform_compression_streams.js",
            TRANSFORM_COMPRESSION_STREAMS_PRELUDE_SRC,
            TRANSFORM_COMPRESSION_STREAMS_PRELUDE,
        ),
        (
            "15_blob_formdata.js",
            BLOB_FORMDATA_PRELUDE_SRC,
            BLOB_FORMDATA_PRELUDE,
        ),
        (
            "16_structured_clone.js",
            STRUCTURED_CLONE_PRELUDE_SRC,
            STRUCTURED_CLONE_PRELUDE,
        ),
        (
            "17_buffer_binary_rw.js",
            BUFFER_BINARY_RW_PRELUDE_SRC,
            BUFFER_BINARY_RW_PRELUDE,
        ),
        ("18_intl.js", INTL_PRELUDE_SRC, INTL_PRELUDE),
        (
            "19_process_cli.js",
            PROCESS_CLI_PRELUDE_SRC,
            PROCESS_CLI_PRELUDE,
        ),
    ];

    let mut total_src_bytes = 0usize;
    let mut total_min_bytes = 0usize;
    for &(name, src, minified) in pairs {
        assert!(
            src.lines().count() > 25,
            "{} should be human-readable multi-line source",
            name
        );
        assert!(
            minified.len() < src.len(),
            "{} compile-time minified bytes ({}) should be smaller than source ({})",
            name,
            minified.len(),
            src.len()
        );
        // Verify deterministic reproducibility between runtime `minify_js_source` and `build.rs` output
        let runtime_min = minify_js_source(src);
        assert_eq!(
            runtime_min.as_bytes(),
            minified,
            "{} build.rs output must match minify_js_source deterministically",
            name
        );
        total_src_bytes += src.len();
        total_min_bytes += minified.len();
    }
    assert!(
        total_min_bytes * 100 < total_src_bytes * 75,
        "Expected >= 25% compile-time size reduction across preludes (src={}, min={})",
        total_src_bytes,
        total_min_bytes
    );

    // 2. Verify dynamic `globalThis["Web" + "Socket"]` self-replacing lazy getter behavior:
    //    Because the string `"WebSocket"` does not appear literally in `dynamic_js`, `detect_needed_shards`
    //    returns 0, testing pure on-demand `__defLazy` getter self-replacement on `globalThis`.
    let dynamic_js = r#"
const key = "Web" + "Socket";
const descBefore = Object.getOwnPropertyDescriptor(globalThis, key);
const wasLazyGetter = Boolean(descBefore && typeof descBefore.get === "function" && descBefore.enumerable === false);
const ctor = globalThis[key];
const descAfter = Object.getOwnPropertyDescriptor(globalThis, key);
const replacedWithValue = Boolean(descAfter && typeof descAfter.value === "function" && descAfter.get === undefined);
console.log(JSON.stringify({
    wasLazyGetter,
    isFunc: typeof ctor === "function",
    openConst: ctor.OPEN,
    replacedWithValue
}));
"#;
    let m = run_source_isolated(
        "lazy-shard-tenant",
        dynamic_js,
        CapabilityToken::pure_compute(),
        500_000,
        4 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(
        m.stdout[0],
        r#"{"wasLazyGetter":true,"isFunc":true,"openConst":1,"replacedWithValue":true}"#
    );
}

#[test]
fn test_process_argv_defaults_custom_cli_args_and_pooled_isolation() {
    use crate::{compile_source_to_bytecode, SliceIsolate};
    use slice_sandbox::CapabilityToken;

    let check_argv_bc = compile_source_to_bytecode(r#"console.log(JSON.stringify(process.argv));"#);
    let mutate_argv_bc = compile_source_to_bytecode(
        r#"
        process.argv.push("--injected");
        const afterPush = process.argv.slice();
        process.argv = ["replaced-bin", "replaced.js"];
        console.log(JSON.stringify({ afterPush, replaced: process.argv }));
        "#,
    );

    // 1. Default isolate sees `["slice"]`
    let mut iso1 = SliceIsolate::spawn(
        "argv-default",
        1,
        CapabilityToken::pure_compute(),
        500_000,
        4 * 1024 * 1024,
    );
    let m1 = iso1.execute_bytecode(&check_argv_bc, None).unwrap();
    assert_eq!(m1.stdout[0], r#"["slice"]"#);

    // 2. Custom CLI argv forwarding (`set_process_argv`) and in-place mutation
    let mut iso2 = SliceIsolate::spawn(
        "argv-custom",
        2,
        CapabilityToken::pure_compute(),
        500_000,
        4 * 1024 * 1024,
    );
    iso2.set_process_argv(vec![
        "slice".to_string(),
        "cli_tool.ts".to_string(),
        "--port".to_string(),
        "8080".to_string(),
    ]);
    let m2 = iso2.execute_bytecode(&mutate_argv_bc, None).unwrap();
    assert_eq!(
        m2.stdout[0],
        r#"{"afterPush":["slice","cli_tool.ts","--port","8080","--injected"],"replaced":["replaced-bin","replaced.js"]}"#
    );

    // 3. Pooled warm realm resets `process.argv` cleanly back to `["slice"]`
    let mut iso3 = SliceIsolate::spawn(
        "argv-after-mutation",
        3,
        CapabilityToken::pure_compute(),
        500_000,
        4 * 1024 * 1024,
    );
    let m3 = iso3.execute_bytecode(&check_argv_bc, None).unwrap();
    assert_eq!(m3.stdout[0], r#"["slice"]"#);
}

#[test]
fn test_json_parse_and_stringify_shapes_numbers_strings_and_reviver_source() {
    use crate::run_source_isolated;
    use slice_sandbox::CapabilityToken;

    let js = r#"
    const longAscii = "abcdefgh01234567".repeat(12);
    const longUtf16 = "\u20ac\u00e9\u65e5\u672c\u8a9e_segment_".repeat(8);
    const strPayload = JSON.stringify({
        ascii: longAscii,
        utf16: longUtf16,
        esc: "line1\nline2\r\t\"quote\"\\slash\b\f",
        loneHigh: "\ud800tail",
        loneLow: "head\udc00"
    });
    const strParsed = JSON.parse(strPayload);

    const nums = JSON.parse('[-0, 0, 42, -2147483648, 2147483647, 9007199254740991, -9007199254740991, 3.141592653589793, 1.5e-7, 1e400]');
    const negZeroOk = Object.is(nums[0], -0);

    // Homogeneous array of objects (exercises SharedShape chain prediction + monomorphic IC reads)
    const batchText = JSON.stringify(
        Array.from({ length: 32 }, (_, i) => ({
            id: i,
            label: "row-" + i,
            nested: { x: i * 2, active: (i & 1) === 0 }
        }))
    );
    const batch = JSON.parse(batchText);
    let sum = 0;
    for (let i = 0; i < batch.length; i++) {
        sum += batch[i].id + batch[i].nested.x;
    }

    // Heterogeneous keys, duplicate keys, numeric index keys, and __proto__ own property
    const hetero = JSON.parse('[{"a":1,"b":2},{"b":20,"a":10,"c":30},{"0":"zero","10":"ten","dup":1,"dup":2,"__proto__":{"admin":true}}]');
    const protoNotPolluted = ({}).admin === undefined;
    const ownProtoDesc = Object.getOwnPropertyDescriptor(hetero[2], "__proto__");

    // Reviver with `context.source`
    const sources = [];
    const revived = JSON.parse('{"big":9007199254740993,"flag":true}', (k, v, ctx) => {
        if (k) sources.push(k + "=" + ctx.source);
        return k === "big" ? ctx.source : v;
    });

    console.log(JSON.stringify({
        asciiEq: strParsed.ascii === longAscii,
        utf16Eq: strParsed.utf16 === longUtf16,
        loneOk: strParsed.loneHigh.charCodeAt(0) === 0xd800 && strParsed.loneLow.charCodeAt(4) === 0xdc00,
        negZeroOk,
        maxSafe: nums[5] === 9007199254740991,
        minSafe: nums[6] === -9007199254740991,
        infOk: nums[9] === Infinity,
        sum,
        heteroCheck: hetero[0].a === 1 && hetero[1].c === 30 && hetero[2]["0"] === "zero" && hetero[2]["10"] === "ten" && hetero[2].dup === 2,
        protoNotPolluted,
        ownProtoAdmin: Boolean(ownProtoDesc && ownProtoDesc.enumerable && ownProtoDesc.value.admin === true),
        revivedBig: revived.big,
        sources
    }));
    "#;

    let m = run_source_isolated(
        "json-shape-tenant",
        js,
        CapabilityToken::pure_compute(),
        2_000_000,
        8 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(
        m.stdout[0],
        r#"{"asciiEq":true,"utf16Eq":true,"loneOk":true,"negZeroOk":true,"maxSafe":true,"minSafe":true,"infOk":true,"sum":1488,"heteroCheck":true,"protoNotPolluted":true,"ownProtoAdmin":true,"revivedBig":"9007199254740993","sources":["big=9007199254740993","flag=true"]}"#
    );
}

#[test]
fn test_fs_host_reads_directory_errors_symlinks_watch_and_lazy_promises() {
    use crate::run_source_isolated;
    use slice_sandbox::CapabilityToken;

    let tmp_dir = std::env::temp_dir().join(format!("slice_fs_host_test_{}", std::process::id()));
    let sub_dir = tmp_dir.join("nested_dir");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&sub_dir).unwrap();

    let ascii_file = tmp_dir.join("ascii.txt");
    let utf8_file = tmp_dir.join("utf8.txt");
    std::fs::write(&ascii_file, b"hello-host-ascii-payload").unwrap();
    std::fs::write(
        &utf8_file,
        "Cafe \u{20ac}100 \u{65e5}\u{672c}\u{8a9e}".as_bytes(),
    )
    .unwrap();

    let tmp_norm = tmp_dir.to_string_lossy().replace('\\', "/");
    let sub_norm = sub_dir.to_string_lossy().replace('\\', "/");
    let ascii_norm = ascii_file.to_string_lossy().replace('\\', "/");
    let utf8_norm = utf8_file.to_string_lossy().replace('\\', "/");

    let js = format!(
        r#"
        const fs = require("node:fs");
        const asciiTxt = fs.readFileSync({:?}, "utf8");
        const utf8Txt = fs.readFileSync({:?}, "utf8");
        const rawBytes = fs.readFileSync({:?});

        let hostDirCode = "";
        try {{ fs.readFileSync({:?}, "utf8"); }} catch (e) {{ hostDirCode = e.code; }}

        fs.mkdirSync("/tmp/vfs-dir-check", {{ recursive: true }});
        let vfsDirCode = "";
        try {{ fs.readFileSync("/tmp/vfs-dir-check", "utf8"); }} catch (e) {{ vfsDirCode = e.code; }}

        fs.writeFileSync("/tmp/vfs-dir-check/target.txt", "symlink-body");
        fs.symlinkSync("/tmp/vfs-dir-check/target.txt", "/tmp/vfs-dir-check/link.txt");
        const linkTxt = fs.readFileSync("/tmp/vfs-dir-check/link.txt", "utf8");
        const isSym = fs.lstatSync("/tmp/vfs-dir-check/link.txt").isSymbolicLink();

        const events = [];
        const watcher = fs.watch("/tmp/vfs-dir-check", (ev, name) => events.push(ev + ":" + name));
        fs.writeFileSync("/tmp/vfs-dir-check/watched.txt", "ok");
        watcher.close();

        const fsp = require("node:fs/promises");
        const samePromisesRef = (fs.promises === fsp);
        fsp.readFile({:?}, "utf8").then((asyncTxt) => {{
            console.log(JSON.stringify({{
                asciiTxt,
                utf8Len: utf8Txt.length,
                rawLen: rawBytes.length,
                hostDirCode,
                vfsDirCode,
                linkTxt,
                isSym,
                watchEvents: events.length,
                samePromisesRef,
                asyncMatch: asyncTxt === asciiTxt
            }}));
        }});
        "#,
        ascii_norm, utf8_norm, ascii_norm, sub_norm, ascii_norm
    );

    let caps = CapabilityToken::pure_compute()
        .with_read_prefix(&tmp_norm)
        .with_read_prefix("/tmp")
        .with_write_prefix("/tmp");
    let m = run_source_isolated("fs-host-tenant", &js, caps, 2_000_000, 8 * 1024 * 1024).unwrap();
    let _ = std::fs::remove_dir_all(&tmp_dir);

    assert_eq!(
        m.stdout[0],
        r#"{"asciiTxt":"hello-host-ascii-payload","utf8Len":13,"rawLen":24,"hostDirCode":"EISDIR","vfsDirCode":"EISDIR","linkTxt":"symlink-body","isSym":true,"watchEvents":1,"samePromisesRef":true,"asyncMatch":true}"#
    );
}

#[test]
fn test_buffer_byte_length_encodings_and_array_buffer_views() {
    use crate::run_source_isolated;
    use slice_sandbox::CapabilityToken;

    let js = r#"
    const asciiLen = Buffer.byteLength("hello-world", "utf8");
    const latin1HighUtf8 = Buffer.byteLength("\u00e9\u00ffA", "utf8");
    const latin1Raw = Buffer.byteLength("\u00e9\u00ffA", "latin1");
    const bmpAndAstralUtf8 = Buffer.byteLength("\u20ac\ud83d\ude80", "utf8");
    const loneSurrogateUtf8 = Buffer.byteLength("\ud800", "utf8");
    const utf16leLen = Buffer.byteLength("abcd", "utf16le");
    const hexLen = Buffer.byteLength("deadbeef01", "hex");
    const b64Padded = Buffer.byteLength("aGVsbG8=", "base64");
    const b64UrlUnpadded = Buffer.byteLength("aGVsbG8", "base64url");

    const ab = new ArrayBuffer(32);
    const sub = new Uint8Array(ab, 8, 12);
    const dv = new DataView(ab, 4, 10);
    const buf = Buffer.from("slice");

    const { Writable } = require("node:stream");
    const w = new Writable({ highWaterMark: 16, write(_c, _e, cb) { cb(null); } });
    w.write("\u20ac\u20ac"); // 6 UTF-8 bytes without needing explicit Buffer import

    console.log(JSON.stringify({
        asciiLen,
        latin1HighUtf8,
        latin1Raw,
        bmpAndAstralUtf8,
        loneSurrogateUtf8,
        utf16leLen,
        hexLen,
        b64Padded,
        b64UrlUnpadded,
        abLen: Buffer.byteLength(ab),
        subLen: Buffer.byteLength(sub),
        dvLen: Buffer.byteLength(dv),
        bufLen: Buffer.byteLength(buf)
    }));
    "#;

    let m = run_source_isolated(
        "buf-bytelen-tenant",
        js,
        CapabilityToken::pure_compute(),
        1_000_000,
        4 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(
        m.stdout[0],
        r#"{"asciiLen":11,"latin1HighUtf8":5,"latin1Raw":3,"bmpAndAstralUtf8":7,"loneSurrogateUtf8":3,"utf16leLen":8,"hexLen":5,"b64Padded":5,"b64UrlUnpadded":5,"abLen":32,"subLen":12,"dvLen":10,"bufLen":5}"#
    );
}

#[test]
fn test_shard_isolation_for_node_crypto_process_cli_intl_dom_exception_and_streams() {
    use crate::boacas::compute_boacas_metadata;
    use crate::run_source_isolated;
    use slice_sandbox::CapabilityToken;

    // 1. `node:crypto` (`crypto.createHash`) must NOT pull in Shard 4 (`WebCrypto`, bit 4)
    let (_, node_crypto_mask, _) = compute_boacas_metadata(
        r#"const crypto = require("node:crypto"); const h = crypto.createHash("sha256").update("x").digest("hex");"#,
    );
    assert_ne!(node_crypto_mask & (1u32 << 19), 0);
    assert_eq!(node_crypto_mask & (1u32 << 4), 0);

    // WebCrypto (`crypto.randomUUID`, `crypto.subtle`, `globalThis.crypto`) MUST pull in Shard 4
    let (_, web_crypto_mask, _) = compute_boacas_metadata(r#"const id = crypto.randomUUID();"#);
    assert_ne!(web_crypto_mask & (1u32 << 4), 0);

    // 2. `AbortController` must NOT pull in Shard 6 (`06_console_ext.js`)
    let (_, abort_mask, _) = compute_boacas_metadata(r#"const ac = new AbortController();"#);
    assert_ne!(abort_mask & (1u32 << 10), 0);
    assert_ne!(abort_mask & (1u32 << 13), 0);
    assert_eq!(abort_mask & (1u32 << 6), 0);

    // 3. `process.stdout` / `hrtime` / `node:process` must load Shard 29 (`19_process_cli.js`) without Shard 18 (`18_intl.js`)
    let (_, proc_cli_mask, _) = compute_boacas_metadata(
        r#"const p = require("node:process"); const t = process.hrtime.bigint(); process.stdout.write(String(t));"#,
    );
    assert_ne!(proc_cli_mask & (1u32 << 29), 0);
    assert_eq!(proc_cli_mask & (1u32 << 18), 0);

    let (_, intl_mask, _) =
        compute_boacas_metadata(r#"const nf = new Intl.NumberFormat("en-US");"#);
    assert_ne!(intl_mask & (1u32 << 18), 0);

    // 4. Runtime execution of `process` CLI, `Intl`, `DOMException` lazy stack, and `ReadableStream`
    let runtime_js = r#"
    let evFired = false;
    process.on("customCliEvent", (v) => { evFired = (v === 99); });
    process.emit("customCliEvent", 99);
    const hr = process.hrtime();
    const formatted = new Intl.NumberFormat("en-US").format(12345);

    const domErr = new DOMException("timed out", "TimeoutError");
    const stackBefore = domErr.stack;
    domErr.stack = "custom-stack";

    const rs = new ReadableStream({
        start(ctrl) {
            ctrl.enqueue("chunk-1");
        },
        pull(ctrl) {
            ctrl.enqueue("chunk-2");
            ctrl.close();
        }
    });
    const reader = rs.getReader();
    Promise.all([reader.read(), reader.read(), reader.read()]).then((steps) => {
        console.log(JSON.stringify({
            evFired,
            hrPair: Array.isArray(hr) && hr.length === 2,
            formatted,
            domCode: domErr.code,
            domConst: DOMException.TIMEOUT_ERR,
            stackHasName: stackBefore.indexOf("TimeoutError: timed out") === 0,
            stackOverridden: domErr.stack === "custom-stack",
            chunks: [steps[0].value, steps[1].value, steps[2].done]
        }));
    });
    "#;
    let m = run_source_isolated(
        "shard-runtime-tenant",
        runtime_js,
        CapabilityToken::pure_compute(),
        2_000_000,
        8 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(
        m.stdout[0],
        r#"{"evFired":true,"hrPair":true,"formatted":"12,345","domCode":23,"domConst":23,"stackHasName":true,"stackOverridden":true,"chunks":["chunk-1","chunk-2",true]}"#
    );
}

#[test]
fn test_http_fast_outcome_and_server_websocket_dispatch() {
    use crate::{compile_source_to_bytecode, HttpRequestContext, ServerWsOutFrame, SliceIsolate};
    use slice_sandbox::CapabilityToken;

    // 1. Tier-0 SLICESNP fast HTTP dispatch + pointer-cached `decode_slicesnp_cached`
    let tier0_bc = compile_source_to_bytecode("const a = 200; const b = 56; return a + b;");
    let mut iso = SliceIsolate::spawn(
        "http-fast-tenant",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        4 * 1024 * 1024,
    );
    let req = HttpRequestContext {
        method: "GET",
        path: "/calc",
        query: "x=1",
        tenant_id: "http-fast-tenant",
        body: "",
        headers: &[("host", "localhost")],
    };
    for slot in 1..=3 {
        iso.reset_for_request("http-fast-tenant", slot, 5_000_000, 4 * 1024 * 1024);
        let out = iso
            .execute_http_request_fast(&tier0_bc, None, &req)
            .unwrap();
        assert_eq!(out.status_code, 200);
        assert_eq!(out.content_type, "application/json");
        assert_eq!(out.response_body, "256");
    }

    // 2. Tier-1 SLICEBC custom `Slice.respond` + `Slice.waitUntil` + Egress DLP guard
    let wintertc_bc = compile_source_to_bytecode(
        r#"
        const r = Slice.request();
        Slice.waitUntil(() => { Slice.kv.set("wu_path", r.path, 0); });
        Slice.respond(201, JSON.stringify({ created: r.path }), "application/json", [["x-edge", "ok"]]);
        "#,
    );
    iso.reset_for_request("http-fast-tenant", 10, 5_000_000, 4 * 1024 * 1024);
    let out_w = iso
        .execute_http_request_fast(&wintertc_bc, None, &req)
        .unwrap();
    assert_eq!(out_w.status_code, 201);
    assert_eq!(out_w.response_body, r#"{"created":"/calc"}"#);
    assert_eq!(out_w.wait_until_count, 1);
    assert_eq!(iso.drain_wait_until_tasks().unwrap(), 1);
    assert_eq!(iso.kv_get("wu_path").as_deref(), Some("/calc"));

    // Egress DLP must also block secret exfiltration on `execute_http_request_fast`
    iso.add_egress_audit_secret("sk_live_topsecret999");
    let leak_bc =
        compile_source_to_bytecode(r#"Slice.respond(200, "token=sk_live_topsecret999");"#);
    iso.reset_for_request("http-fast-tenant", 11, 5_000_000, 4 * 1024 * 1024);
    assert!(iso.execute_http_request_fast(&leak_bc, None, &req).is_err());

    // 3. Direct callable server WebSocket frame dispatch (`execute_websocket_event`)
    let mut ws_iso = SliceIsolate::spawn(
        "ws-fast-tenant",
        20,
        CapabilityToken::pure_compute(),
        5_000_000,
        4 * 1024 * 1024,
    );
    let ws_setup_bc = compile_source_to_bytecode(
        r#"
        Slice.onWebSocket({
            message(ws, data) {
                ws.send("ack:" + String(data));
            }
        });
        "#,
    );
    ws_iso.execute_bytecode(&ws_setup_bc, None).unwrap();
    let frames = ws_iso
        .execute_websocket_event("message", false, b"ping-payload", 0, "")
        .unwrap();
    assert_eq!(frames.len(), 1);
    match &frames[0] {
        ServerWsOutFrame::Text(t) => assert_eq!(t, "ack:ping-payload"),
        other => panic!("expected text frame, got {:?}", other),
    }
}

#[test]
fn test_websocket_zero_delay_interactive_roundtrips_and_unidirectional_burst() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        // Session 1: 6 sequential request-response echoes + server welcome frame
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.set_nodelay(true);
            let mut req_buf = [0u8; 2048];
            let n = stream.read(&mut req_buf).unwrap();
            let req_str = String::from_utf8_lossy(&req_buf[..n]);
            let key = req_str
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    if k.trim().eq_ignore_ascii_case("sec-websocket-key") {
                        Some(v.trim().to_string())
                    } else {
                        None
                    }
                })
                .unwrap();
            let accept = crate::compute_websocket_accept_key(&key);
            let handshake = format!(
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
                accept
            );
            stream.write_all(handshake.as_bytes()).unwrap();
            stream.flush().unwrap();

            let welcome =
                crate::encode_websocket_frame(true, crate::WS_OPCODE_TEXT, None, b"welcome");
            stream.write_all(&welcome).unwrap();
            stream.flush().unwrap();

            let mut pending = Vec::new();
            loop {
                match crate::read_websocket_message_from_stream(
                    &mut stream,
                    &mut pending,
                    true,
                    65536,
                ) {
                    Ok(Some((crate::WS_OPCODE_TEXT, payload))) => {
                        let mut reply_bytes = b"echo:".to_vec();
                        reply_bytes.extend_from_slice(&payload);
                        let reply = crate::encode_websocket_frame(
                            true,
                            crate::WS_OPCODE_TEXT,
                            None,
                            &reply_bytes,
                        );
                        stream.write_all(&reply).unwrap();
                        stream.flush().unwrap();
                    }
                    Ok(Some((crate::WS_OPCODE_CLOSE, close_payload))) => {
                        let close_reply = crate::encode_websocket_frame(
                            true,
                            crate::WS_OPCODE_CLOSE,
                            None,
                            &close_payload,
                        );
                        let _ = stream.write_all(&close_reply);
                        let _ = stream.flush();
                        break;
                    }
                    _ => break,
                }
            }
        }
        // Session 2: Unidirectional burst of 20 frames with no server replies until close
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.set_nodelay(true);
            let mut req_buf = [0u8; 2048];
            let n = stream.read(&mut req_buf).unwrap();
            let req_str = String::from_utf8_lossy(&req_buf[..n]);
            let key = req_str
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    if k.trim().eq_ignore_ascii_case("sec-websocket-key") {
                        Some(v.trim().to_string())
                    } else {
                        None
                    }
                })
                .unwrap();
            let accept = crate::compute_websocket_accept_key(&key);
            let handshake = format!(
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
                accept
            );
            stream.write_all(handshake.as_bytes()).unwrap();
            stream.flush().unwrap();

            let mut pending = Vec::new();
            let mut count = 0usize;
            loop {
                match crate::read_websocket_message_from_stream(
                    &mut stream,
                    &mut pending,
                    true,
                    65536,
                ) {
                    Ok(Some((crate::WS_OPCODE_TEXT, _))) => {
                        count += 1;
                    }
                    Ok(Some((crate::WS_OPCODE_CLOSE, close_payload))) => {
                        assert_eq!(count, 20);
                        let close_reply = crate::encode_websocket_frame(
                            true,
                            crate::WS_OPCODE_CLOSE,
                            None,
                            &close_payload,
                        );
                        let _ = stream.write_all(&close_reply);
                        let _ = stream.flush();
                        break;
                    }
                    _ => break,
                }
            }
        }
    });

    let js = format!(
        r#"
        (async () => {{
            const acks = await new Promise((resolve, reject) => {{
                const ws = new WebSocket("ws://127.0.0.1:{port}/echo");
                const list = [];
                let step = 0;
                ws.onerror = reject;
                ws.onmessage = (ev) => {{
                    list.push(String(ev.data));
                    if (step < 5) {{
                        step++;
                        ws.send("msg-" + step);
                    }} else {{
                        ws.close(1000, "done");
                    }}
                }};
                ws.onclose = () => resolve(list);
            }});

            await new Promise((resolve, reject) => {{
                const ws2 = new WebSocket("ws://127.0.0.1:{port}/burst");
                ws2.onerror = reject;
                ws2.onopen = () => {{
                    for (let i = 0; i < 20; i++) {{
                        ws2.send("fire-" + i);
                    }}
                    ws2.close(1000, "burst-done");
                }};
                ws2.onclose = () => resolve(true);
            }});

            console.log(JSON.stringify(acks));
        }})();
        "#,
        port = port
    );

    let caps = CapabilityToken::pure_compute().with_net_host(format!("127.0.0.1:{}", port));
    let start = std::time::Instant::now();
    let metrics =
        run_source_isolated("ws-zero-delay", &js, caps, 5_000_000, 8 * 1024 * 1024).unwrap();
    let elapsed = start.elapsed();
    let _ = server.join();

    assert_eq!(
        metrics.stdout[0],
        r#"["welcome","echo:msg-1","echo:msg-2","echo:msg-3","echo:msg-4","echo:msg-5"]"#
    );
    assert!(
        elapsed < std::time::Duration::from_millis(250),
        "interactive + burst WebSocket sessions took {:?} (expected fast zero-delay dispatch)",
        elapsed
    );
}
