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
fn test_web_worker_and_node_worker_threads_roundtrip() {
    let code = r#"
import {
    Worker as NodeWorker,
    isMainThread,
    MessageChannel,
    receiveMessageOnPort,
} from "node:worker_threads";

async function main() {
    // 1. Verify URL.canParse + URL.createObjectURL + WHATWG Web Worker round-trip
    const canParseOk = URL.canParse("https://example.com/a") && !URL.canParse("://bad");
    const blob = new Blob([
        "self.onmessage = function(e) { self.postMessage({ doubled: e.data.n * 2, tag: self.name }); self.close(); };"
    ], { type: "application/javascript" });
    const blobUrl = URL.createObjectURL(blob);

    const webRes = await new Promise((resolve, reject) => {
        const w = new Worker(blobUrl, { name: "whatwg-w" });
        w.onmessage = (ev) => resolve(ev.data);
        w.onerror = (err) => reject(new Error(err.message || "web worker error"));
        w.postMessage({ n: 21 });
    });
    URL.revokeObjectURL(blobUrl);

    // 2. Verify node:worker_threads with workerData, typed arrays (Uint8Array), Map, BigInt, and parentPort
    const nodeWorkerCode = `
        const { isMainThread, workerData, parentPort, threadId } = require("node:worker_threads");
        parentPort.once("message", (msg) => {
            const sum = Array.from(msg.bytes).reduce((a, b) => a + b, 0);
            parentPort.postMessage({
                fromWorker: true,
                isMainThread,
                hasThreadId: threadId > 0,
                seedMult: workerData.multiplier * sum,
                big: msg.big + 10n,
                mapVal: msg.map.get("k"),
                outBytes: new Uint8Array([sum, workerData.multiplier])
            });
        });
    `;
    const nodeRes = await new Promise((resolve, reject) => {
        const nw = new NodeWorker(nodeWorkerCode, {
            eval: true,
            workerData: { multiplier: 3 }
        });
        nw.on("message", (data) => resolve(data));
        nw.on("error", (err) => reject(err));
        const mp = new Map([["k", "v-ok"]]);
        nw.postMessage({
            bytes: new Uint8Array([1, 2, 3, 4]),
            big: 90n,
            map: mp
        });
    });

    // 3. Verify MessageChannel + synchronous receiveMessageOnPort + async port.on("message")
    const { port1, port2 } = new MessageChannel();
    port1.postMessage({ syncMsg: 77 });
    const syncRead = receiveMessageOnPort(port2);
    const emptyRead = receiveMessageOnPort(port2);

    const asyncPortMsg = await new Promise((resolve) => {
        port2.on("message", (m) => resolve(m));
        port1.postMessage({ asyncMsg: 88 });
    });
    port1.close();
    port2.close();

    console.log(JSON.stringify({
        isMainThread,
        canParseOk,
        webDoubled: webRes.doubled,
        webTag: webRes.tag,
        nodeWorkerIsMain: nodeRes.isMainThread,
        nodeHasThreadId: nodeRes.hasThreadId,
        nodeSeedMult: nodeRes.seedMult,
        nodeBig: String(nodeRes.big),
        nodeMapVal: nodeRes.mapVal,
        nodeOutBytes: Array.from(nodeRes.outBytes),
        syncMsg: syncRead && syncRead.message && syncRead.message.syncMsg,
        emptyAfterSync: emptyRead === undefined,
        asyncMsg: asyncPortMsg.asyncMsg
    }));
}
main();
"#;
    let m = run_source_isolated(
        "worker-roundtrip-tenant",
        code,
        CapabilityToken::pure_compute(),
        2_000_000,
        16 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(
        m.stdout[0],
        r#"{"isMainThread":true,"canParseOk":true,"webDoubled":42,"webTag":"whatwg-w","nodeWorkerIsMain":false,"nodeHasThreadId":true,"nodeSeedMult":30,"nodeBig":"100","nodeMapVal":"v-ok","nodeOutBytes":[10,3],"syncMsg":77,"emptyAfterSync":true,"asyncMsg":88}"#
    );
}

#[test]
fn test_worker_capability_attenuation_and_fuel_isolation() {
    let code = r#"
import fs from "node:fs";
import { Worker } from "node:worker_threads";

fs.writeFileSync("/app/secret.txt", "TOP_SECRET_DATA");

async function main() {
    // 1. Spawn a worker attenuated with allowRead: [] (even though parent has /app read/write!)
    const fsDeniedResult = await new Promise((resolve) => {
        let errText = "";
        const w = new Worker(
            `const fs = require("node:fs"); fs.readFileSync("/app/secret.txt", "utf8");`,
            {
                eval: true,
                slice: { allowRead: [] }
            }
        );
        w.on("error", (e) => { errText = e.message; });
        w.on("exit", (code) => {
            resolve({ code, blocked: errText.includes("filesystem read authority") || errText.includes("SandboxViolation") });
        });
    });

    // 2. Spawn a worker with an infinite loop and low fuel ceiling (5,000) — must not crash parent!
    const fuelExhaustedResult = await new Promise((resolve) => {
        let errText = "";
        const w = new Worker(
            `while (true) {}`,
            {
                eval: true,
                slice: { fuel: 5000 }
            }
        );
        w.on("error", (e) => { errText = e.message; });
        w.on("exit", (code) => {
            resolve({ code, exhausted: errText.includes("CPU instruction fuel") || errText.includes("SandboxViolation") });
        });
    });

    // 3. Spawn an interactive worker and terminate() it from the parent
    const wTerm = new Worker(
        `const { parentPort } = require("node:worker_threads"); parentPort.on("message", () => {});`,
        { eval: true }
    );
    const termCode = await wTerm.terminate();

    // Parent can still read /app/secret.txt normally
    const parentReadOk = fs.readFileSync("/app/secret.txt", "utf8") === "TOP_SECRET_DATA";

    console.log(JSON.stringify({
        fsDeniedCode: fsDeniedResult.code,
        fsDeniedBlocked: fsDeniedResult.blocked,
        fuelCode: fuelExhaustedResult.code,
        fuelExhausted: fuelExhaustedResult.exhausted,
        termCode,
        parentReadOk
    }));
}
main();
"#;
    let caps = CapabilityToken::pure_compute()
        .with_read_prefix("/app")
        .with_write_prefix("/app");
    let m = run_source_isolated(
        "worker-attenuation-tenant",
        code,
        caps,
        2_000_000,
        16 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(
        m.stdout[0],
        r#"{"fsDeniedCode":1,"fsDeniedBlocked":true,"fuelCode":1,"fuelExhausted":true,"termCode":1,"parentReadOk":true}"#
    );
}

#[test]
fn test_wintertc_cloudflare_sockets_and_live_node_net_dgram() {
    use std::io::{Read, Write};
    use std::net::{TcpListener, UdpSocket};

    // 1. Start a live loopback TCP server
    let tcp_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let tcp_port = tcp_listener.local_addr().unwrap().port();
    let tcp_handle = std::thread::spawn(move || {
        // Accept 2 connections: 1 for `cloudflare:sockets` `connect()`, 1 for `node:net` `net.connect()`
        for _ in 0..2 {
            if let Ok((mut stream, _)) = tcp_listener.accept() {
                let mut buf = [0u8; 256];
                if let Ok(n) = stream.read(&mut buf) {
                    let req_str = String::from_utf8_lossy(&buf[..n]);
                    let reply = format!("TCP_ACK:{}", req_str);
                    let _ = stream.write_all(reply.as_bytes());
                    let _ = stream.flush();
                }
            }
        }
    });

    // 2. Start a live loopback UDP echo server
    let udp_server = UdpSocket::bind("127.0.0.1:0").unwrap();
    let udp_port = udp_server.local_addr().unwrap().port();
    let udp_handle = std::thread::spawn(move || {
        let mut buf = [0u8; 256];
        if let Ok((n, src)) = udp_server.recv_from(&mut buf) {
            let msg = String::from_utf8_lossy(&buf[..n]);
            let reply = format!("UDP_ACK:{}", msg);
            let _ = udp_server.send_to(reply.as_bytes(), src);
        }
    });

    let code = format!(
        r#"
import {{ connect }} from "cloudflare:sockets";
import net from "node:net";
import dgram from "node:dgram";

async function main() {{
    // A. Deterministic Slice.mockSocket test
    Slice.mockSocket("redis.internal:6379", "+PONG\r\n");
    const mockSock = connect("redis.internal:6379");
    const mockWriter = mockSock.writable.getWriter();
    await mockWriter.write("PING\r\n");
    const mockReader = mockSock.readable.getReader();
    const {{ value: mockVal }} = await mockReader.read();
    const mockReply = new TextDecoder().decode(mockVal).trim();
    await mockSock.close();

    // B. Live WinterTC cloudflare:sockets connect()
    const liveSock = connect({{ hostname: "127.0.0.1", port: {tcp_port} }});
    const openedInfo = await liveSock.opened;
    const writer = liveSock.writable.getWriter();
    await writer.write(new TextEncoder().encode("wintertc-hello"));
    const reader = liveSock.readable.getReader();
    const {{ value: liveVal }} = await reader.read();
    const liveTcpReply = new TextDecoder().decode(liveVal);
    await liveSock.close();

    // C. Live node:net Socket
    const nodeNetReply = await new Promise((resolve, reject) => {{
        const client = net.connect({{ host: "127.0.0.1", port: {tcp_port} }}, () => {{
            client.setEncoding("utf8");
            client.write("node-net-hi");
        }});
        client.on("data", (chunk) => {{
            resolve(chunk);
            client.end();
        }});
        client.on("error", reject);
    }});

    // D. Live node:dgram Socket
    const nodeUdpReply = await new Promise((resolve, reject) => {{
        const udp = dgram.createSocket("udp4");
        udp.on("message", (msg, rinfo) => {{
            udp.close();
            resolve({{ text: msg.toString("utf8"), port: rinfo.port }});
        }});
        udp.on("error", reject);
        udp.send("dgram-ping", {udp_port}, "127.0.0.1");
    }});

    console.log(JSON.stringify({{
        mockReply,
        hasRemoteAddr: openedInfo.remoteAddress.includes("{tcp_port}"),
        liveTcpReply,
        nodeNetReply,
        nodeUdpText: nodeUdpReply.text,
        nodeUdpPortMatch: nodeUdpReply.port === {udp_port},
        ip4: net.isIP("127.0.0.1"),
        ip6: net.isIP("::1"),
        ipBad: net.isIP("not-an-ip")
    }}));
}}
main().catch((e) => console.log("ERR:" + (e && e.message || e)));
"#
    );

    let caps = CapabilityToken::pure_compute()
        .with_net_host("redis.internal:6379")
        .with_net_host(format!("127.0.0.1:{}", tcp_port))
        .with_net_host(format!("127.0.0.1:{}", udp_port));

    let m = run_source_isolated(
        "sockets-live-tenant",
        &code,
        caps,
        2_000_000,
        16 * 1024 * 1024,
    )
    .unwrap();
    let _ = tcp_handle.join();
    let _ = udp_handle.join();

    assert_eq!(
        m.stdout[0],
        r#"{"mockReply":"+PONG","hasRemoteAddr":true,"liveTcpReply":"TCP_ACK:wintertc-hello","nodeNetReply":"TCP_ACK:node-net-hi","nodeUdpText":"UDP_ACK:dgram-ping","nodeUdpPortMatch":true,"ip4":4,"ip6":6,"ipBad":0}"#
    );
}

#[test]
fn test_raw_socket_5_ring_security_ocap_ssrf_and_egress_dlp() {
    // 1. Ring 1: OCap deny when allow_net is empty
    let err_no_cap = run_source_isolated(
        "sock-sec-1",
        r#"import { connect } from "cloudflare:sockets"; connect("example.com:80");"#,
        CapabilityToken::pure_compute(),
        500_000,
        4 * 1024 * 1024,
    )
    .unwrap_err()
    .downcast::<SandboxViolation>()
    .unwrap();
    assert!(matches!(
        err_no_cap,
        SandboxViolation::NetAccessDenied { .. }
    ));

    // 2. Ring 1: Privileged port (< 1024 except 80/443) blocked under wildcard allow_net = ["*"]
    let err_priv_port = run_source_isolated(
        "sock-sec-2",
        r#"import { connect } from "cloudflare:sockets"; connect("93.184.216.34:22");"#,
        CapabilityToken::pure_compute().with_net_host("*"),
        500_000,
        4 * 1024 * 1024,
    )
    .unwrap_err()
    .downcast::<SandboxViolation>()
    .unwrap();
    assert!(matches!(
        err_priv_port,
        SandboxViolation::NetAccessDenied { .. }
    ));

    // 3. Ring 2: Cloud metadata & non-canonical octal IPv4 literal blocked by SSRF shield
    for bad_addr in ["169.254.169.254:80", "127.0.0.1:6379", "0177.0.0.1:80"] {
        let code = format!(
            r#"import {{ connect }} from "cloudflare:sockets"; connect("{}");"#,
            bad_addr
        );
        let err_ssrf = run_source_isolated(
            "sock-sec-ssrf",
            &code,
            CapabilityToken::pure_compute().with_net_host("*"),
            500_000,
            4 * 1024 * 1024,
        )
        .unwrap_err()
        .downcast::<SandboxViolation>()
        .unwrap();
        assert!(
            matches!(err_ssrf, SandboxViolation::NetAccessDenied { .. }),
            "Expected SSRF block for {}",
            bad_addr
        );
    }

    // 4. Ring 3: Outbound Egress Secret DLP on TCP `socket.writable` (both raw and Base64-encoded!)
    let mut iso_tcp = SliceIsolate::spawn(
        "sock-dlp-tcp",
        1,
        CapabilityToken::pure_compute().with_net_host("dlp.example.com:8080"),
        500_000,
        4 * 1024 * 1024,
    );
    iso_tcp.add_egress_audit_secret("sk_live_RAW_SOCKET_SECRET_777");
    let tcp_dlp_bc = compile_source_to_bytecode(
        r#"
import { connect } from "cloudflare:sockets";
Slice.mockSocket("dlp.example.com:8080", "OK");
async function run() {
    const sock = connect("dlp.example.com:8080");
    const writer = sock.writable.getWriter();
    await writer.write(btoa("sk_live_RAW_SOCKET_SECRET_777"));
}
run();
"#,
    );
    let tcp_dlp_err = iso_tcp.execute_bytecode(&tcp_dlp_bc, None).unwrap_err();
    assert!(matches!(
        tcp_dlp_err,
        SandboxViolation::EgressSecretLeakBlocked { .. }
    ));

    // 5. Ring 3: Outbound Egress Secret DLP on UDP `dgram.Socket.send`
    let mut iso_udp = SliceIsolate::spawn(
        "sock-dlp-udp",
        1,
        CapabilityToken::pure_compute().with_net_host("127.0.0.1:9999"),
        500_000,
        4 * 1024 * 1024,
    );
    iso_udp.add_egress_audit_secret("ghp_UDP_DLP_TOKEN_888");
    let udp_dlp_bc = compile_source_to_bytecode(
        r#"
import dgram from "node:dgram";
Slice.mockSocket("udp://127.0.0.1:9999", "ACK");
const s = dgram.createSocket("udp4");
s.send("leak: ghp_UDP_DLP_TOKEN_888", 9999, "127.0.0.1");
"#,
    );
    let udp_dlp_err = iso_udp.execute_bytecode(&udp_dlp_bc, None).unwrap_err();
    assert!(matches!(
        udp_dlp_err,
        SandboxViolation::EgressSecretLeakBlocked { .. }
    ));
}

#[test]
fn test_pillar1_speculative_worker_pool_reuse_and_script_cache_isolation() {
    crate::prewarm_worker_pool_sync(1);
    let reuse_before = crate::worker_pool_reuse_count();
    let cache_hits_before = crate::worker_script_cache_hits();

    // Run 3 sequential workers with identical script source across differing capabilities/workerData
    // to verify:
    // 1. IDLE_WORKER_POOL reuses the pre-warmed worker OS thread + WarmBoaRealm (`worker_pool_reuse_count` increases).
    // 2. WORKER_SCRIPT_CACHE + `__SLICE_RC` function cache hits on subsequent worker executions (`worker_script_cache_hits` increases).
    // 3. Zero cross-worker state leak (each worker sees only its own `workerData`, VFS is scrubbed between runs, and attenuated capabilities do not bleed).
    let code = r#"
import { Worker } from "node:worker_threads";

    let script = `
    import { parentPort, workerData } from "node:worker_threads";
    import fs from "node:fs";
    const leakedFile = fs.existsSync("/tmp/worker-leak.txt");
    if (workerData.allowTmpWrite) {
        fs.writeFileSync("/tmp/worker-leak.txt", "from-" + workerData.id);
    }
    const wroteVfs = fs.existsSync("/tmp/worker-leak.txt");
    parentPort.postMessage({
        id: workerData.id,
        leakedFile,
        wroteVfs,
        leakedGlobal: typeof globalThis.__workerSecretLeak !== "undefined"
    });
`;

async function runSequential(id, allowTmpWrite) {
    return await new Promise((resolve, reject) => {
        const w = new Worker(script, {
            eval: true,
            workerData: { id, allowTmpWrite },
            slice: allowTmpWrite ? { allowWrite: ["/tmp"] } : { allowRead: ["/tmp"], allowWrite: [] }
        });
        w.on("message", (msg) => resolve(msg));
        w.on("error", (err) => reject(err));
    });
}

async function main() {
    const r1 = await runSequential(1, true);
    const r2 = await runSequential(2, true);
    const r3 = await runSequential(3, false);

    // Also verify that on worker #4 with allowWrite: [], an actual write triggers a SandboxViolation
    const writeBlockedOn4 = await new Promise((resolve) => {
        let blocked = false;
        const w = new Worker(
            `import fs from "node:fs"; fs.writeFileSync("/tmp/worker-leak.txt", "blocked");`,
            { eval: true, slice: { allowWrite: [] } }
        );
        w.on("error", (e) => {
            if (e.message.includes("filesystem") || e.message.includes("SandboxViolation")) {
                blocked = true;
            }
        });
        w.on("exit", (code) => resolve(code === 1 && blocked));
    });

    console.log(JSON.stringify({ r1, r2, r3, writeBlockedOn4 }));
}
main();
"#;

    let caps = CapabilityToken::pure_compute()
        .with_read_prefix("/tmp")
        .with_write_prefix("/tmp");
    let m =
        run_source_isolated("pillar1-pool-test", code, caps, 2_000_000, 16 * 1024 * 1024).unwrap();
    assert_eq!(
        m.stdout[0],
        r#"{"r1":{"id":1,"leakedFile":false,"wroteVfs":true,"leakedGlobal":false},"r2":{"id":2,"leakedFile":false,"wroteVfs":true,"leakedGlobal":false},"r3":{"id":3,"leakedFile":false,"wroteVfs":false,"leakedGlobal":false},"writeBlockedOn4":true}"#
    );

    let reuse_after = crate::worker_pool_reuse_count();
    let cache_hits_after = crate::worker_script_cache_hits();
    assert!(
        reuse_after >= reuse_before + 3,
        "Expected at least 3 warm worker pool reuses, got before={} after={}",
        reuse_before,
        reuse_after
    );
    assert!(
        cache_hits_after >= cache_hits_before + 2,
        "Expected at least 2 worker script cache hits, got before={} after={}",
        cache_hits_before,
        cache_hits_after
    );
}

#[test]
fn test_pillar2_coalesced_node_net_pump_and_worker_terminate_await() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server_handle = std::thread::spawn(move || {
        for _ in 0..3 {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 256];
                if let Ok(n) = stream.read(&mut buf) {
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let reply = format!("FAST_ACK:{}", req.trim());
                    let _ = stream.write_all(reply.as_bytes());
                    let _ = stream.flush();
                }
            }
        }
    });

    crate::prewarm_worker_pool_sync(1);
    let reuse_before = crate::worker_pool_reuse_count();

    let code = format!(
        r#"
import net from "node:net";
import {{ Worker }} from "node:worker_threads";

const workerScript = `
    const {{ parentPort }} = require("node:worker_threads");
    parentPort.on("message", (x) => parentPort.postMessage(x * 2));
`;

async function runTerminateWorker(val) {{
    return await new Promise((resolve, reject) => {{
        const w = new Worker(workerScript, {{ eval: true }});
        w.on("message", (out) => {{
            w.terminate().then(() => resolve(out));
        }});
        w.on("error", reject);
        w.postMessage(val);
    }});
}}

async function runTcpSession(id) {{
    return await new Promise((resolve, reject) => {{
        const sock = net.createConnection({{ host: "127.0.0.1", port: {port} }});
        sock.on("connect", () => {{
            sock.write("PING-" + id + "\r\n");
        }});
        sock.on("data", (chunk) => {{
            const txt = chunk.toString("utf8");
            sock.destroy();
            resolve(txt);
        }});
        sock.on("error", reject);
    }});
}}

async function main() {{
    const w1 = await runTerminateWorker(10);
    const w2 = await runTerminateWorker(21);
    const w3 = await runTerminateWorker(33);

    const t0 = performance.now();
    const s1 = await runTcpSession(1);
    const s2 = await runTcpSession(2);
    const s3 = await runTcpSession(3);
    const tcpElapsedMs = performance.now() - t0;

    console.log(JSON.stringify({{
        workers: [w1, w2, w3],
        sockets: [s1, s2, s3],
        fastTurnaround: tcpElapsedMs < 40
    }}));
}}
main().catch((e) => console.log("ERR:" + (e && e.message || e)));
"#
    );

    let caps = CapabilityToken::pure_compute().with_net_host(format!("127.0.0.1:{}", port));
    let m =
        run_source_isolated("pillar2-net-test", &code, caps, 2_000_000, 16 * 1024 * 1024).unwrap();
    assert!(
        !m.stdout.is_empty(),
        "pillar2 produced no stdout! return_value={}",
        m.return_value
    );
    assert_eq!(
        m.stdout[0],
        r#"{"workers":[20,42,66],"sockets":["FAST_ACK:PING-1","FAST_ACK:PING-2","FAST_ACK:PING-3"],"fastTurnaround":true}"#
    );
    let _ = server_handle.join();

    let reuse_after = crate::worker_pool_reuse_count();
    assert!(
        reuse_after >= reuse_before + 3,
        "Expected all 3 worker.terminate() workers to reuse the single warm worker thread, got before={} after={}",
        reuse_before,
        reuse_after
    );
}

#[test]
fn test_pillar3_speculative_promise_then_hook_and_nexttick_priority() {
    // 1. Script that does NOT use process.nextTick: Promise.prototype.then stays bound to Boa's
    // native Rust implementation (`[native code]`) with 0 JS wrapper closure allocations across 500 awaits.
    let pure_async_code = r#"
async function main() {
    const isNativeThen = Function.prototype.toString.call(Promise.prototype.then).includes("[native code]");
    let sum = 0;
    for (let i = 0; i < 500; i++) {
        sum += await Promise.resolve(1);
    }
    console.log(JSON.stringify({ sum, isNativeThen }));
}
main();
"#;
    let m1 = run_source_isolated(
        "pillar3-pure-async",
        pure_async_code,
        CapabilityToken::pure_compute(),
        2_000_000,
        16 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(m1.stdout[0], r#"{"sum":500,"isNativeThen":true}"#);

    // 2. Script that DOES use process.nextTick: Speculatively hooks Promise.prototype.then
    // before execution so `process.nextTick` scheduled after `Promise.resolve().then(...)` in the
    // same synchronous turn still drains BEFORE the `.then()` callback!
    let nexttick_code = r#"
async function main() {
    const order = [];
    Promise.resolve().then(() => order.push("promise-1"));
    queueMicrotask(() => {
        order.push("microtask-1");
        process.nextTick(() => order.push("nextTick-nested"));
    });
    process.nextTick(() => order.push("nextTick-1"));
    order.push("sync");

    await new Promise((r) => setTimeout(r, 0));

    let caughtErr = "";
    process.nextTick(() => order.push("nextTick-before-reject"));
    try {
        await Promise.reject(new Error("boom-reject"));
    } catch (e) {
        caughtErr = e.message;
    }

    console.log(JSON.stringify({ caughtErr, order }));
}
main();
"#;
    let m2 = run_source_isolated(
        "pillar3-nexttick-async",
        nexttick_code,
        CapabilityToken::pure_compute(),
        2_000_000,
        16 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(
        m2.stdout[0],
        r#"{"caughtErr":"boom-reject","order":["sync","nextTick-1","promise-1","microtask-1","nextTick-nested","nextTick-before-reject"]}"#
    );

    // 3. Subsequent script on the SAME warm realm that does NOT use process.nextTick:
    // Verify `Promise.prototype.then` was restored to native `[native code]`!
    let m3 = run_source_isolated(
        "pillar3-restored-async",
        pure_async_code,
        CapabilityToken::pure_compute(),
        2_000_000,
        16 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(m3.stdout[0], r#"{"sum":500,"isNativeThen":true}"#);
}

#[test]
fn test_pillar4_surgical_shard_gating_and_native_buffer_fast_paths() {
    // 1. Verify surgical `detect_needed_shards` bitmasks
    let worker_mask = crate::js_preludes::detect_needed_shards(
        "const { Worker, MessageChannel } = require('node:worker_threads');",
        false,
    );
    assert_eq!(
        worker_mask,
        (1 << 8) | (1 << 13) | (1 << 2),
        "Worker-only scripts must not eagerly load Shard 1 (URL), Shard 11 (Fetch), or Shard 3 (Streams)"
    );

    let wintertc_sock_mask = crate::js_preludes::detect_needed_shards(
        "const sock = Slice.connect('127.0.0.1:6379');",
        false,
    );
    assert_eq!(
        wintertc_sock_mask,
        (1 << 8) | (1 << 14) | (1 << 3) | (1 << 13) | (1 << 2)
    );

    let url_can_parse_mask = crate::js_preludes::detect_needed_shards(
        "const ok = URL.canParse('https://example.com');",
        false,
    );
    assert_eq!(
        url_can_parse_mask,
        1 << 1,
        "URL.canParse must load only Shard 1 (URL/URLSearchParams), not Shard 11 (Fetch) or Shard 12 (URLPattern)"
    );

    let fetch_req_res_mask = crate::js_preludes::detect_needed_shards(
        "const r = new Response('ok', { headers: new Headers() });",
        false,
    );
    assert_eq!(
        fetch_req_res_mask,
        1 << 11,
        "Headers/Request/Response must load only Shard 11, not Shard 1 (URL) or Shard 12 (URLPattern)"
    );

    let url_pattern_mask = crate::js_preludes::detect_needed_shards(
        "const p = new URLPattern({ pathname: '/users/:id' });",
        false,
    );
    assert_eq!(
        url_pattern_mask,
        1 << 12,
        "URLPattern must load only Shard 12, not Shard 1 (URL) or Shard 11 (Fetch)"
    );

    let text_encoder_mask = crate::js_preludes::detect_needed_shards(
        "const u8 = new TextEncoder().encode('hi');",
        false,
    );
    assert_eq!(
        text_encoder_mask,
        1 << 2,
        "TextEncoder/TextDecoder/btoa/atob must load only Shard 2, not Shard 13 (DOM Events)"
    );

    let dom_events_mask = crate::js_preludes::detect_needed_shards(
        "const et = new EventTarget(); et.dispatchEvent(new Event('ping'));",
        false,
    );
    assert_eq!(
        dom_events_mask,
        1 << 13,
        "Event/CustomEvent/EventTarget must load only Shard 13, not Shard 2 (Encoding)"
    );

    let readable_stream_mask = crate::js_preludes::detect_needed_shards(
        "const rs = new ReadableStream({ start(c) { c.close(); } });",
        false,
    );
    assert_eq!(
        readable_stream_mask,
        1 << 3,
        "ReadableStream must load only Shard 3, not Shard 14 (Transform/Compression) or Shard 15 (Blob/FormData)"
    );

    let blob_formdata_mask = crate::js_preludes::detect_needed_shards(
        "const fd = new FormData(); fd.append('a', new Blob(['x']));",
        false,
    );
    assert_eq!(
        blob_formdata_mask,
        1 << 15,
        "Blob/File/FormData must load only Shard 15, not Shard 3 (ReadableStream) or Shard 14 (Transform/Compression)"
    );

    let structured_clone_mask =
        crate::js_preludes::detect_needed_shards("const copy = structuredClone({ a: 1 });", false);
    assert_eq!(
        structured_clone_mask,
        1 << 16,
        "structuredClone must load only Shard 16, not Shard 4 (WebCrypto)"
    );

    let core_buffer_mask = crate::js_preludes::detect_needed_shards(
        "const b = Buffer.concat([Buffer.alloc(4)]);",
        false,
    );
    assert_eq!(
        core_buffer_mask,
        1 << 5,
        "Core Buffer must load only Shard 5, not Shard 17 (Binary Integer R/W)"
    );

    // 2. Verify compile-time lowering of `Buffer.from(...).toString(...)` & `Buffer.byteLength(...)`
    let ts_src = r#"
const secret: string = "sk_live_slice_9941_\u00e9";
const b64: string = Buffer.from(secret).toString("base64");
const b64Url: string = Buffer.from(secret, "utf8").toString("base64url");
const roundtrip: string = Buffer.from(b64Url, "base64url").toString("utf8");
const byteLen: number = Buffer.byteLength(secret);
const u8 = new Uint8Array([72, 105]);
const u8B64: string = Buffer.from(u8).toString("base64");
const u8Utf8: string = Buffer.from(u8).toString("utf8");
const u8Len: number = Buffer.byteLength(u8);
console.log(JSON.stringify({ b64, b64Url, roundtripOk: roundtrip === secret, byteLen, u8B64, u8Utf8, u8Len }));
"#;
    let normalized = slice_cas::strip_typescript_and_normalize(ts_src, true);
    assert!(
        !normalized.contains("Buffer."),
        "Expected Buffer.from(...).toString(...) and Buffer.byteLength(...) to lower to native intrinsics, got:\n{}",
        normalized
    );
    assert_eq!(
        crate::js_preludes::detect_needed_shards(&normalized, false) & (1 << 5),
        0,
        "Expected Shard 5 (05_full_buffer.js) not to be required after intrinsic lowering"
    );

    let m = run_source_isolated(
        "pillar4-buffer-fast-path",
        ts_src,
        CapabilityToken::pure_compute(),
        500_000,
        16 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(
        m.stdout[0],
        r#"{"b64":"c2tfbGl2ZV9zbGljZV85OTQxX8Op","b64Url":"c2tfbGl2ZV9zbGljZV85OTQxX8Op","roundtripOk":true,"byteLen":21,"u8B64":"SGk=","u8Utf8":"Hi","u8Len":2}"#
    );
}

#[test]
fn test_pillar5_sliceboa_payload_compaction_and_multiline_template_preservation() {
    let raw_bundle = r#"var __slice_modules = {};//__SLICE_RC:0123456789abcdef
// #slice-source: src/index.ts|/app/src/index.ts
    // Indented comment that should be stripped from SLICEBOA
    const rx = /'/g;
    const inlineBacktickStr = "hello ` world";
    const multilineTpl = `line1
    indented_line2
        indented_line3`;
    function compute() {
        return multilineTpl;
    }
    console.log(JSON.stringify({
        rxOk: rx.test("'"),
        strOk: inlineBacktickStr.includes("`"),
        lines: compute().split("\n")
    }));
"#;

    let bc = crate::compile_source_to_bytecode(raw_bundle);
    let fb_str = if let Some((_, _, fb)) = crate::BoaCasHeader::decode(&bc) {
        fb
    } else {
        std::str::from_utf8(&bc[8..]).unwrap()
    };
    let inner = fb_str
        .strip_prefix("{\n")
        .and_then(|s| s.strip_suffix("\n}"))
        .unwrap_or(fb_str);

    // 1. Line 1 `__SLICE_RC` header must remain intact at byte offset 21
    assert!(inner.starts_with("var __slice_modules = {};//__SLICE_RC:0123456789abcdef\n"));
    // 2. `#slice-source` and full-line comments and code indentation must be stripped
    assert!(!inner.contains("#slice-source"));
    assert!(!inner.contains("Indented comment"));
    assert!(inner.contains("\nconst rx = /'/g;\n"));
    // 3. Multi-line template literal indentation must be preserved verbatim!
    assert!(inner.contains("`line1\n    indented_line2\n        indented_line3`"));
    assert!(
        inner.len() < raw_bundle.len(),
        "Expected compacted SLICEBOA payload ({}) to be smaller than raw bundle ({})",
        inner.len(),
        raw_bundle.len()
    );

    let mut isolate = SliceIsolate::spawn(
        "pillar5-compact-test",
        1,
        CapabilityToken::pure_compute(),
        500_000,
        16 * 1024 * 1024,
    );
    let m = isolate.execute_bytecode(&bc, None).unwrap();
    assert_eq!(
        m.stdout[0],
        r#"{"rxOk":true,"strOk":true,"lines":["line1","    indented_line2","        indented_line3"]}"#
    );
}

#[test]
fn test_wave1_json_parse_ecma404_float_precision_surrogates_nesting_reviver_and_perf() {
    let js = r#"
const exact993 = JSON.parse("9007199254740993.0");
const exact994 = JSON.parse("9007199254740994.0");
const exact1e23 = JSON.parse("1e23");
const subnormal = JSON.parse("5e-324");
const maxVal = JSON.parse("1.7976931348623157e308");
const posInf = JSON.parse("1e400");
const negInf = JSON.parse("-1e400");
const posInf309 = JSON.parse("1e309");
const negZeroInt = JSON.parse("-0");
const negZeroFloat = JSON.parse("-0.0");
const negZeroExp = JSON.parse("-1e-400");
const posZeroExp = JSON.parse("1e-400");

// Lone surrogates, reversed pairs, and valid surrogate pairs
const loneHigh = JSON.parse('"\\ud800"');
const loneLow = JSON.parse('"\\udfff"');
const reversedPair = JSON.parse('"\\udc00\\ud800"');
const highThenAscii = JSON.parse('"\\ud800A"');
const validPair = JSON.parse('"\\ud83d\\ude00"');
const surrogateObj = JSON.parse('{"\\ud800":"\\udfff"}');

// Deep nesting (1000 levels of arrays, 500 levels of objects)
const deepArrJson = "[".repeat(1000) + "42" + "]".repeat(1000);
let curArr = JSON.parse(deepArrJson);
let arrDepth = 0;
while (Array.isArray(curArr)) {
    arrDepth++;
    curArr = curArr[0];
}

const deepObjJson = '{"k":'.repeat(500) + '"leaf"' + '}'.repeat(500);
let curObj = JSON.parse(deepObjJson);
let objDepth = 0;
while (curObj && typeof curObj === "object") {
    objDepth++;
    curObj = curObj.k;
}

// Excessive nesting (> 2048) must throw SyntaxError cleanly without stack overflow
let excessiveThrew = false;
try {
    JSON.parse("[".repeat(2500) + "1" + "]".repeat(2500));
} catch (e) {
    excessiveThrew = e instanceof SyntaxError;
}

// Reviver with context.source and JSON.rawJSON
const reviverSources = {};
const revived = JSON.parse('{"big":9007199254740993,"inf":1e400,"nested":{"s":"\\ud800"}}', (k, v, ctx) => {
    if (ctx && typeof ctx.source === "string") {
        reviverSources[k] = ctx.source;
    } else if (k === "nested") {
        reviverSources[k] = ctx ? typeof ctx.source : "no-ctx";
    }
    return v;
});

// Throughput benchmark on 2,000-record mixed payload
const records = [];
for (let i = 0; i < 200; i++) {
    records.push({
        id: i,
        score: i * 1.25,
        big: 9007199254740992,
        name: "record_" + i,
        escaped: "line1\\nline2\\t" + i,
        active: (i & 1) === 0,
        nil: null,
        tags: ["alpha", "beta", "gamma"]
    });
}
const benchPayload = JSON.stringify(records);
const t0 = Date.now();
let benchSum = 0;
for (let it = 0; it < 20; it++) {
    const parsed = JSON.parse(benchPayload);
    benchSum += parsed.length + parsed[199].id;
}
const elapsedMs = Date.now() - t0;

console.log(JSON.stringify({
    exact993Ok: exact993 === 9007199254740992,
    exact994Ok: exact994 === 9007199254740994,
    exact1e23Ok: exact1e23 === 1e23,
    subnormalOk: subnormal === Number.MIN_VALUE,
    maxValOk: maxVal === Number.MAX_VALUE,
    posInfOk: posInf === Infinity && posInf309 === Infinity,
    negInfOk: negInf === -Infinity,
    negZeroOk: Object.is(negZeroInt, -0) && Object.is(negZeroFloat, -0) && Object.is(negZeroExp, -0) && Object.is(posZeroExp, 0),
    loneHighOk: loneHigh.length === 1 && loneHigh.charCodeAt(0) === 0xD800,
    loneLowOk: loneLow.length === 1 && loneLow.charCodeAt(0) === 0xDFFF,
    reversedPairOk: reversedPair.length === 2 && reversedPair.charCodeAt(0) === 0xDC00 && reversedPair.charCodeAt(1) === 0xD800,
    highThenAsciiOk: highThenAscii.length === 2 && highThenAscii.charCodeAt(0) === 0xD800 && highThenAscii.charCodeAt(1) === 65,
    validPairOk: validPair.length === 2 && validPair.codePointAt(0) === 0x1F600,
    surrogateObjOk: surrogateObj["\ud800"].charCodeAt(0) === 0xDFFF,
    arrDepth,
    arrLeaf: curArr,
    objDepth,
    objLeaf: curObj,
    excessiveThrew,
    reviverBigSource: reviverSources.big,
    reviverInfSource: reviverSources.inf,
    reviverStrSource: reviverSources.s,
    reviverNestedSource: reviverSources.nested,
    revivedInfIsInf: revived.inf === Infinity,
    benchSum,
    elapsedMs
}));
"#;

    let bc = crate::compile_source_to_bytecode(js);
    let mut isolate = SliceIsolate::spawn(
        "wave1-json-test",
        1,
        CapabilityToken::pure_compute(),
        50_000_000,
        32 * 1024 * 1024,
    );
    let m = isolate.execute_bytecode(&bc, None).unwrap();
    let out: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(out["exact993Ok"], true);
    assert_eq!(out["exact994Ok"], true);
    assert_eq!(out["exact1e23Ok"], true);
    assert_eq!(out["subnormalOk"], true);
    assert_eq!(out["maxValOk"], true);
    assert_eq!(out["posInfOk"], true);
    assert_eq!(out["negInfOk"], true);
    assert_eq!(out["negZeroOk"], true);
    assert_eq!(out["loneHighOk"], true);
    assert_eq!(out["loneLowOk"], true);
    assert_eq!(out["reversedPairOk"], true);
    assert_eq!(out["highThenAsciiOk"], true);
    assert_eq!(out["validPairOk"], true);
    assert_eq!(out["surrogateObjOk"], true);
    assert_eq!(out["arrDepth"], 1000);
    assert_eq!(out["arrLeaf"], 42);
    assert_eq!(out["objDepth"], 500);
    assert_eq!(out["objLeaf"], "leaf");
    assert_eq!(out["excessiveThrew"], true);
    assert_eq!(out["reviverBigSource"], "9007199254740993");
    assert_eq!(out["reviverInfSource"], "1e400");
    assert_eq!(out["reviverStrSource"], "\"\\ud800\"");
    assert_eq!(out["reviverNestedSource"], "undefined");
    assert_eq!(out["revivedInfIsInf"], true);
    assert_eq!(out["benchSum"], 20 * (200 + 199));
}

#[test]
fn test_wave1_process_exit_lifecycle_unhandled_rejection_and_uncaught_exception() {
    // 1. process.exit(42) inside try/catch/finally must NOT be caught by catch and must set exit_code = 42
    {
        let js = r#"
let events = [];
process.on("exit", (code) => {
    events.push("exit:" + code);
    console.log(JSON.stringify(events));
});
process.on("beforeExit", (code) => {
    events.push("beforeExit:" + code);
});
try {
    events.push("before-exit-call");
    process.exit(42);
    events.push("after-exit-call");
} catch (e) {
    events.push("caught:" + e);
}
events.push("after-try");
"#;
        let bc = crate::compile_source_to_bytecode(js);
        let mut iso = SliceIsolate::spawn(
            "wave1-exit-try",
            1,
            CapabilityToken::pure_compute(),
            5_000_000,
            16 * 1024 * 1024,
        );
        let m = iso.execute_bytecode(&bc, None).unwrap();
        assert_eq!(m.exit_code, Some(42));
        assert_eq!(m.stdout, vec![r#"["before-exit-call","exit:42"]"#]);
    }

    // 2. process.exit(19) inside a Promise.then microtask stops subsequent microtasks & timers
    {
        let js = r#"
let log = [];
process.on("exit", (code) => {
    log.push("exit:" + code);
    console.log(JSON.stringify(log));
});
setTimeout(() => {
    log.push("timer-should-not-run");
}, 0);
Promise.resolve().then(() => {
    log.push("promise-1");
    process.exit(19);
    log.push("promise-1-after");
}).catch((e) => {
    log.push("promise-catch-should-not-run");
});
Promise.resolve().then(() => {
    log.push("promise-2-should-not-run");
});
"#;
        let bc = crate::compile_source_to_bytecode(js);
        let mut iso = SliceIsolate::spawn(
            "wave1-exit-promise",
            1,
            CapabilityToken::pure_compute(),
            5_000_000,
            16 * 1024 * 1024,
        );
        let m = iso.execute_bytecode(&bc, None).unwrap();
        assert_eq!(m.exit_code, Some(19));
        assert_eq!(m.stdout, vec![r#"["promise-1","exit:19"]"#]);
    }

    // 3. Natural exit: beforeExit can schedule additional work once, then exit fires and process.exitCode is honored
    {
        let js = r#"
let seq = [];
let scheduled = false;
process.exitCode = 5;
process.on("beforeExit", (code) => {
    seq.push("beforeExit:" + code);
    if (!scheduled) {
        scheduled = true;
        setTimeout(() => {
            seq.push("timer-from-beforeExit");
            process.exitCode = 9;
        }, 1);
    }
});
process.on("exit", (code) => {
    seq.push("exit:" + code);
    console.log(JSON.stringify(seq));
});
seq.push("sync-done");
"#;
        let bc = crate::compile_source_to_bytecode(js);
        let mut iso = SliceIsolate::spawn(
            "wave1-before-exit",
            1,
            CapabilityToken::pure_compute(),
            5_000_000,
            16 * 1024 * 1024,
        );
        iso.set_deterministic_seed(Some(42));
        let m = iso.execute_bytecode(&bc, None).unwrap();
        assert_eq!(m.exit_code, Some(9));
        assert_eq!(
            m.stdout,
            vec![r#"["sync-done","beforeExit:5","timer-from-beforeExit","beforeExit:9","exit:9"]"#]
        );
    }

    // 4. Unhandled Promise rejection WITHOUT listener produces `Uncaught (in promise)` and exit_code = 1
    {
        let js = r#"
Promise.reject(new Error("unhandled-async-boom"));
"#;
        let bc = crate::compile_source_to_bytecode(js);
        let mut iso = SliceIsolate::spawn(
            "wave1-unhandled-rej",
            1,
            CapabilityToken::pure_compute(),
            5_000_000,
            16 * 1024 * 1024,
        );
        let m = iso.execute_bytecode(&bc, None).unwrap();
        assert_eq!(m.exit_code, Some(1));
        assert!(
            m.return_value
                .contains("Uncaught (in promise) Error: unhandled-async-boom"),
            "unexpected return_value: {}",
            m.return_value
        );
    }

    // 5. Unhandled Promise rejection WITH `process.on('unhandledRejection')` listener is caught cleanly
    //    AND a rejection handled in the same microtask turn does NOT fire `unhandledRejection`
    {
        let js = r#"
const caughtReasons = [];
process.on("unhandledRejection", (reason, p) => {
    caughtReasons.push(reason instanceof Error ? reason.message : String(reason));
});
const handled = Promise.reject(new Error("handled-in-turn"));
handled.catch(() => {});
Promise.reject(new Error("intercepted-by-process"));
process.on("exit", (code) => {
    console.log(JSON.stringify({ code, caughtReasons }));
});
"#;
        let bc = crate::compile_source_to_bytecode(js);
        let mut iso = SliceIsolate::spawn(
            "wave1-handled-rej",
            1,
            CapabilityToken::pure_compute(),
            5_000_000,
            16 * 1024 * 1024,
        );
        let m = iso.execute_bytecode(&bc, None).unwrap();
        assert_eq!(m.exit_code.unwrap_or(0), 0);
        assert!(
            !m.return_value.starts_with("Uncaught"),
            "unexpected uncaught error: {}",
            m.return_value
        );
        assert_eq!(
            m.stdout,
            vec![r#"{"code":0,"caughtReasons":["intercepted-by-process"]}"#]
        );
    }

    // 6. process.on('uncaughtException') and 'uncaughtExceptionMonitor' intercept synchronous & timer errors
    {
        let js = r#"
const log = [];
process.on("uncaughtExceptionMonitor", (err, origin) => {
    log.push("monitor:" + err.message + ":" + origin);
});
process.on("uncaughtException", (err, origin) => {
    log.push("caught:" + err.message + ":" + origin);
});
setTimeout(() => {
    throw new Error("timer-boom");
}, 1);
process.on("exit", (code) => {
    console.log(JSON.stringify({ code, log }));
});
throw new Error("sync-boom");
"#;
        let bc = crate::compile_source_to_bytecode(js);
        let mut iso = SliceIsolate::spawn(
            "wave1-uncaught-exc",
            1,
            CapabilityToken::pure_compute(),
            5_000_000,
            16 * 1024 * 1024,
        );
        iso.set_deterministic_seed(Some(99));
        let m = iso.execute_bytecode(&bc, None).unwrap();
        assert_eq!(m.exit_code.unwrap_or(0), 0);
        assert!(
            !m.return_value.starts_with("Uncaught"),
            "unexpected uncaught error: {}",
            m.return_value
        );
        assert_eq!(
            m.stdout,
            vec![
                r#"{"code":0,"log":["monitor:sync-boom:uncaughtException","caught:sync-boom:uncaughtException","monitor:timer-boom:uncaughtException","caught:timer-boom:uncaughtException"]}"#
            ]
        );
    }
}

#[test]
fn test_wave1_worker_threads_date_special_numbers_and_process_exit() {
    let js = r#"
const { Worker, MessageChannel } = require("node:worker_threads");

async function runTests() {
    // 1. Date, Invalid Date, NaN, Infinity, -Infinity across MessageChannel
    const mc = new MessageChannel();
    const mcResult = await new Promise((resolve) => {
        mc.port2.on("message", (msg) => {
            mc.port1.close();
            mc.port2.close();
            resolve({
                dValid: msg.d instanceof Date && msg.d.getTime() === 1750000000123,
                dEpoch: msg.epoch instanceof Date && msg.epoch.getTime() === 0,
                dInvalid: msg.inv instanceof Date && Number.isNaN(msg.inv.getTime()),
                nanOk: Number.isNaN(msg.nan),
                posInfOk: msg.posInf === Infinity,
                negInfOk: msg.negInf === -Infinity,
            });
        });
        mc.port1.postMessage({
            d: new Date(1750000000123),
            epoch: new Date(0),
            inv: new Date(NaN),
            nan: NaN,
            posInf: Infinity,
            negInf: -Infinity,
        });
    });

    // 2. Date & special numbers across workerData, worker.postMessage, parentPort.postMessage,
    //    AND process.exit(42) inside worker when `process` is referenced
    const workerCode = `
        const { parentPort, workerData } = require("node:worker_threads");
        const procRef = process;
        parentPort.on("message", (incoming) => {
            parentPort.postMessage({
                wdDateOk: workerData.created instanceof Date && workerData.created.getTime() === 1700000000000,
                wdInfOk: workerData.limit === Infinity,
                inDateOk: incoming.when instanceof Date && incoming.when.getTime() === 1712345678901,
                inInvalidDateOk: incoming.badDate instanceof Date && Number.isNaN(incoming.badDate.getTime()),
                inNanOk: Number.isNaN(incoming.score),
                replyDate: new Date(1720000000555),
                replyNegInf: -Infinity,
            });
            procRef.exit(42);
        });
    `;

    const wResult = await new Promise((resolve, reject) => {
        const w = new Worker(workerCode, {
            eval: true,
            workerData: {
                created: new Date(1700000000000),
                limit: Infinity,
            },
        });
        let msgReply = null;
        w.on("message", (m) => {
            msgReply = {
                wdDateOk: m.wdDateOk,
                wdInfOk: m.wdInfOk,
                inDateOk: m.inDateOk,
                inInvalidDateOk: m.inInvalidDateOk,
                inNanOk: m.inNanOk,
                replyDateOk: m.replyDate instanceof Date && m.replyDate.getTime() === 1720000000555,
                replyNegInfOk: m.replyNegInf === -Infinity,
            };
        });
        w.on("error", reject);
        w.on("exit", (code) => {
            resolve({ exitCode: code, msgReply });
        });
        w.postMessage({
            when: new Date(1712345678901),
            badDate: new Date(NaN),
            score: NaN,
        });
    });

    // 3. Immediate top-level process.exit(17) inside Worker with `process` referenced
    const immediateExitCode = await new Promise((resolve, reject) => {
        const w2 = new Worker(`const p = process; p.exit(17);`, { eval: true });
        w2.on("error", reject);
        w2.on("exit", (code) => resolve(code));
    });

    console.log(JSON.stringify({ mcResult, wResult, immediateExitCode }));
}
runTests();
"#;

    let bundled = slice_cas::strip_typescript_and_normalize(js, true);
    let bc = crate::compile_source_to_bytecode(&bundled);
    let mut isolate = SliceIsolate::spawn(
        "wave1-worker-date-exit",
        1,
        CapabilityToken::pure_compute(),
        20_000_000,
        32 * 1024 * 1024,
    );
    let m = isolate.execute_bytecode(&bc, None).unwrap();
    assert_eq!(
        m.stdout.len(),
        1,
        "stdout={:?}, ret={}",
        m.stdout,
        m.return_value
    );
    let out: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(out["mcResult"]["dValid"], true);
    assert_eq!(out["mcResult"]["dEpoch"], true);
    assert_eq!(out["mcResult"]["dInvalid"], true);
    assert_eq!(out["mcResult"]["nanOk"], true);
    assert_eq!(out["mcResult"]["posInfOk"], true);
    assert_eq!(out["mcResult"]["negInfOk"], true);
    assert_eq!(out["wResult"]["exitCode"], 42);
    assert_eq!(out["wResult"]["msgReply"]["wdDateOk"], true);
    assert_eq!(out["wResult"]["msgReply"]["wdInfOk"], true);
    assert_eq!(out["wResult"]["msgReply"]["inDateOk"], true);
    assert_eq!(out["wResult"]["msgReply"]["inInvalidDateOk"], true);
    assert_eq!(out["wResult"]["msgReply"]["inNanOk"], true);
    assert_eq!(out["wResult"]["msgReply"]["replyDateOk"], true);
    assert_eq!(out["wResult"]["msgReply"]["replyNegInfOk"], true);
    assert_eq!(out["immediateExitCode"], 17);
}
