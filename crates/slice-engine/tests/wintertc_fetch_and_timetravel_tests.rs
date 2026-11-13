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
fn test_tag_f1_wintertc_url_headers_request_response_and_export_default_fetch() {
    SliceIsolate::prewarm_boa_pool(2);

    // 1. Test standard URL, URLSearchParams, Headers, Request, Response, Response.json, Response.redirect
    let wintertc_std_ts = r#"
const u = new URL("/v2/items/../orders?tag=gpu&limit=10#summary", "https://api.slice.dev:443/v1/base");
u.searchParams.set("limit", "25");
u.searchParams.append("tag", "edge");
u.searchParams.sort();

const h = new Headers({ "X-Tenant-Id": "acme-prod", "Content-Type": "application/json" });
h.append("X-Trace", "t1");
h.append("x-trace", "t2");

const req = new Request(u, {
    method: "post",
    headers: h,
    body: JSON.stringify({ sku: "H100", qty: 4 })
});
const parsedBody = req.jsonSync();

const jsonRes = Response.json({ ok: true, sku: parsedBody.sku, qty: parsedBody.qty }, { status: 201 });
const redirRes = Response.redirect("https://slice.dev/login", 307);

console.log(JSON.stringify({
    origin: u.origin,
    pathname: u.pathname,
    search: u.search,
    tags: u.searchParams.getAll("tag"),
    href: u.href,
    method: req.method,
    tenant: req.headers.get("x-tenant-id"),
    trace: req.headers.get("X-TRACE"),
    resStatus: jsonRes.status,
    resOk: jsonRes.ok,
    resCt: jsonRes.headers.get("content-type"),
    resBody: jsonRes.jsonSync(),
    redirStatus: redirRes.status,
    redirLoc: redirRes.headers.get("Location")
}));
"#;
    let bc_std = compile_source_to_bytecode(wintertc_std_ts);
    let mut iso = SliceIsolate::spawn(
        "f1-std-tenant",
        1,
        CapabilityToken::pure_compute(),
        2_000_000,
        8 * 1024 * 1024,
    );
    let metrics = iso.execute_bytecode(&bc_std, None).unwrap();
    assert_eq!(
        metrics.stdout[0],
        r#"{"origin":"https://api.slice.dev","pathname":"/v2/orders","search":"?limit=25&tag=gpu&tag=edge","tags":["gpu","edge"],"href":"https://api.slice.dev/v2/orders?limit=25&tag=gpu&tag=edge#summary","method":"POST","tenant":"acme-prod","trace":"t1, t2","resStatus":201,"resOk":true,"resCt":"application/json","resBody":{"ok":true,"sku":"H100","qty":4},"redirStatus":307,"redirLoc":"https://slice.dev/login"}"#
    );

    // 2. Test `export default { fetch(req, env, ctx) }` handler with zero-copy Request & Headers
    let export_default_ts = r#"
let moduleInitCount = 0;
moduleInitCount += 1;

export default {
    fetch(req: Request, env: any, ctx: any): Response {
        const url = new URL(req.url);
        const role = req.headers.get("x-role") || "guest";
        const hits = Slice.kv.incr("f1_hits", 1);
        ctx.waitUntil(() => {
            Slice.kv.set("f1_last_path", url.pathname, 60000);
        });
        if (req.method === "POST") {
            const payload = req.jsonSync();
            return Response.json({
                initCount: moduleInitCount,
                path: url.pathname,
                filter: url.searchParams.get("filter"),
                role,
                hits,
                echo: payload.msg,
                nodeEnv: env.NODE_ENV
            }, { status: 201 });
        }
        return Response.json({ initCount: moduleInitCount, path: url.pathname, hits });
    }
};
"#;
    let fetch_bc = compile_source_to_bytecode(export_default_ts);
    SliceIsolate::prewarm_boa_bundle(&fetch_bc);

    let headers = [
        ("Host", "edge.slice.local"),
        ("X-Role", "admin"),
        ("Content-Type", "application/json"),
    ];
    let req_ctx = HttpRequestContext {
        method: "POST",
        path: "/api/v1/deploy",
        query: "filter=canary",
        tenant_id: "f1-edge-tenant",
        body: r#"{"msg":"ship-it"}"#,
        headers: &headers,
    };
    iso.reset_for_request("f1-edge-tenant", 2, 2_000_000, 8 * 1024 * 1024);
    let out1 = iso.execute_http_request(&fetch_bc, None, &req_ctx).unwrap();
    assert_eq!(out1.status_code, 201);
    assert_eq!(out1.content_type, "application/json");
    assert_eq!(out1.wait_until_count, 1);
    assert_eq!(
        out1.response_body,
        r#"{"initCount":1,"path":"/api/v1/deploy","filter":"canary","role":"admin","hits":1,"echo":"ship-it","nodeEnv":"production"}"#
    );
    let drained = iso.drain_wait_until_tasks().unwrap();
    assert_eq!(drained, 1);
    assert_eq!(
        iso.kv_get("f1_last_path"),
        Some("/api/v1/deploy".to_string())
    );

    // Second request on warm isolate must reuse `moduleInitCount === 1` without re-running module top-level!
    let req_ctx2 = HttpRequestContext {
        method: "GET",
        path: "/api/v1/status",
        query: "",
        tenant_id: "f1-edge-tenant",
        body: "",
        headers: &headers,
    };
    iso.reset_for_request("f1-edge-tenant", 2, 2_000_000, 8 * 1024 * 1024);
    let out2 = iso
        .execute_http_request(&fetch_bc, None, &req_ctx2)
        .unwrap();
    assert_eq!(out2.status_code, 200);
    assert_eq!(
        out2.response_body,
        r#"{"initCount":1,"path":"/api/v1/status","hits":2}"#
    );
    let _ = iso.drain_wait_until_tasks().unwrap();
}

#[test]
fn test_tag_f2_sse_token_streaming_readablestream_and_egress_dlp() {
    SliceIsolate::prewarm_boa_pool(2);

    // 1. Test `Response.sse`, `Slice.streamSSE`, and `ReadableStream`
    let sse_ts = r#"
export default {
    fetch(req: Request): Response {
        const url = new URL(req.url);
        if (url.pathname === "/api/v1/chat/stream") {
            return Response.sse((emit: any) => {
                emit({ id: "tok-1", event: "token", data: { delta: "Slice " }, retry: 1500 });
                emit({ id: "tok-2", event: "token", data: { delta: "streams " } });
                emit({ id: "tok-3", event: "done", data: "[DONE]" });
            });
        }
        if (url.pathname === "/api/v1/raw-stream") {
            const stream = new ReadableStream({
                start(controller: any) {
                    controller.enqueue("chunk-alpha|");
                    controller.enqueue("chunk-beta");
                    controller.close();
                }
            });
            return new Response(stream, { headers: { "content-type": "text/plain" } });
        }
        return new Response("Not Found", { status: 404 });
    }
};
"#;
    let bc = compile_source_to_bytecode(sse_ts);
    SliceIsolate::prewarm_boa_bundle(&bc);

    let mut iso = SliceIsolate::spawn(
        "f2-sse-tenant",
        1,
        CapabilityToken::pure_compute(),
        2_000_000,
        8 * 1024 * 1024,
    );
    iso.add_egress_audit_secret("sk_live_sse_secret_888");

    let headers = [("Host", "edge.slice.local")];
    let req1 = HttpRequestContext {
        method: "GET",
        path: "/api/v1/chat/stream",
        query: "",
        tenant_id: "f2-sse-tenant",
        body: "",
        headers: &headers,
    };
    let out1 = iso.execute_http_request(&bc, None, &req1).unwrap();
    assert_eq!(out1.status_code, 200);
    assert_eq!(out1.content_type, "text/event-stream; charset=utf-8");
    assert!(out1
        .response_body
        .contains("id: tok-1\nevent: token\nretry: 1500\ndata: {\"delta\":\"Slice \"}\n\n"));
    assert!(out1
        .response_body
        .contains("id: tok-2\nevent: token\ndata: {\"delta\":\"streams \"}\n\n"));
    assert!(out1
        .response_body
        .contains("id: tok-3\nevent: done\ndata: [DONE]\n\n"));

    // 2. Test `ReadableStream` passed to `new Response(stream)`
    iso.reset_for_request("f2-sse-tenant", 2, 2_000_000, 8 * 1024 * 1024);
    let req2 = HttpRequestContext {
        method: "GET",
        path: "/api/v1/raw-stream",
        query: "",
        tenant_id: "f2-sse-tenant",
        body: "",
        headers: &headers,
    };
    let out2 = iso.execute_http_request(&bc, None, &req2).unwrap();
    assert_eq!(out2.status_code, 200);
    assert_eq!(out2.response_body, "chunk-alpha|chunk-beta");

    // 3. Test Egress DLP blocking Base64-encoded secret inside an SSE token stream
    let leak_sse_ts = r#"
const b64Leak = Buffer.from("sk_live_sse_secret_888").toString("base64");
Slice.streamSSE([
    { id: "1", event: "token", data: "Safe token 1" },
    { id: "2", event: "token", data: "Exfiltrating: " + b64Leak }
]);
"#;
    let leak_bc = compile_source_to_bytecode(leak_sse_ts);
    iso.reset_for_request("f2-sse-tenant", 3, 2_000_000, 8 * 1024 * 1024);
    let leak_res = iso.execute_http_request(&leak_bc, None, &req1);
    assert!(leak_res.is_err());
    assert!(leak_res
        .unwrap_err()
        .to_string()
        .contains("EGRESS_SECRET_LEAK_BLOCKED"));
}

#[test]
fn test_tag_f3_outbound_fetch_ocap_ssrf_egress_dlp_and_cassette() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    // Spawn a lightweight local HTTP/1.1 upstream server for real socket fetch testing
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let host_port = format!("127.0.0.1:{}", port);

    let upstream_thread = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req_str = String::from_utf8_lossy(&buf[..n]);
            let body_part = req_str.split("\r\n\r\n").nth(1).unwrap_or("{}");
            let resp_body = format!(r#"{{"received":{},"upstream":"slice-f3"}}"#, body_part);
            let resp = format!(
                "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nX-Upstream-Ack: ok-f3\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                resp_body.len(),
                resp_body
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });

    // 1. Authorized outbound `fetch()` over real TCP + deterministic cassette recording in `Slice.kv`
    let ts_fetch_ok = format!(
        r#"
async function main() {{
    const res = await fetch("http://{}/v1/webhook", {{
        method: "POST",
        headers: {{ "Content-Type": "application/json", "X-Trace": "trace-99" }},
        body: JSON.stringify({{ orderId: "ORD-77", total: 1500 }})
    }});
    const data = await res.json();
    console.log(JSON.stringify({{
        status: res.status,
        ok: res.ok,
        ack: res.headers.get("x-upstream-ack"),
        orderId: data.received.orderId,
        upstream: data.upstream
    }}));
}}
main();
"#,
        host_port
    );
    let bc_ok = compile_source_to_bytecode(&ts_fetch_ok);
    let mut iso_ok = SliceIsolate::spawn(
        "f3-fetch-tenant",
        1,
        CapabilityToken::pure_compute().with_net_host(&host_port),
        5_000_000,
        8 * 1024 * 1024,
    );
    iso_ok.set_deterministic_seed(Some(2026));
    let metrics_ok = iso_ok.execute_bytecode(&bc_ok, None).unwrap();
    assert_eq!(
        metrics_ok.stdout[0],
        r#"{"status":201,"ok":true,"ack":"ok-f3","orderId":"ORD-77","upstream":"slice-f3"}"#
    );
    let _ = upstream_thread.join();

    // Verify deterministic cassette replay (`fetch_cassette:POST:http://...`) works with 0 network calls!
    let cassette_key = format!("fetch_cassette:POST:http://{}/v1/webhook", host_port);
    assert!(iso_ok.kv_get(&cassette_key).is_some());
    let mut iso_replay = SliceIsolate::spawn(
        "f3-fetch-tenant",
        2,
        CapabilityToken::pure_compute().with_net_host(&host_port),
        5_000_000,
        8 * 1024 * 1024,
    );
    let metrics_replay = iso_replay.execute_bytecode(&bc_ok, None).unwrap();
    assert_eq!(metrics_replay.stdout[0], metrics_ok.stdout[0]);

    // 2. Cloud Metadata SSRF Shield (`169.254.169.254`) blocked even under wildcard `allow_net = ["*"]`
    let ts_ssrf = r#"
async function attack() {
    const r = await fetch("http://169.254.169.254/latest/meta-data/iam/security-credentials/");
    console.log(r.status);
}
attack();
"#;
    let bc_ssrf = compile_source_to_bytecode(ts_ssrf);
    let mut iso_ssrf = SliceIsolate::spawn(
        "f3-ssrf-tenant",
        3,
        CapabilityToken::pure_compute().with_net_host("*"),
        5_000_000,
        8 * 1024 * 1024,
    );
    let ssrf_res = iso_ssrf.execute_bytecode(&bc_ssrf, None);
    assert!(matches!(
        ssrf_res,
        Err(SandboxViolation::NetAccessDenied { .. })
    ));

    // 3. Outbound Egress Secret DLP blocks Base64-encoded secret in outbound `fetch()` body/headers to an allowed host
    let ts_dlp = format!(
        r#"
async function exfil() {{
    const encoded = Buffer.from("sk_live_TOP_SECRET_777").toString("base64");
    await fetch("http://{}/v1/webhook", {{
        method: "POST",
        headers: {{ "X-Leak": encoded }},
        body: "benign"
    }});
}}
exfil();
"#,
        host_port
    );
    let bc_dlp = compile_source_to_bytecode(&ts_dlp);
    let mut iso_dlp = SliceIsolate::spawn(
        "f3-dlp-tenant",
        4,
        CapabilityToken::pure_compute().with_net_host(&host_port),
        5_000_000,
        8 * 1024 * 1024,
    );
    iso_dlp.add_egress_audit_secret("sk_live_TOP_SECRET_777");
    let dlp_res = iso_dlp.execute_bytecode(&bc_dlp, None);
    assert!(dlp_res.is_err());
    assert!(dlp_res
        .unwrap_err()
        .to_string()
        .contains("EGRESS_SECRET_LEAK_BLOCKED"));
}

#[test]
fn test_tag_f4_time_travel_test_runner_and_dotenv_auto_dlp() {
    // 1. Verify Time-Travel Test Runner (`slice:test` / global `describe`, `test`, `expect`):
    //    - Automatic `< 5 us` VFS (`node:fs`) + `Slice.kv` state rollback between every `test()` block
    //    - `.toViolate(...)`, `.toStayWithinFuel(...)`, `.toStayWithinMemory(...)`
    let ts_test_suite = r#"
import { describe, test, expect, beforeEach } from "slice:test";
import * as fs from "node:fs";

Slice.resetTests();
Slice.kv.set("ledger:balance", "1000", 0);
fs.writeFileSync("/tmp/f4_clean_state.txt", "pristine-v1");

describe("Phase F4 Time-Travel Spec Suite", () => {
    beforeEach(() => {
        Slice.kv.incr("hook:runs", 1);
    });

    test("mutates VFS and KV state during test 1", () => {
        expect(Slice.kv.get("ledger:balance")).toBe("1000");
        expect(fs.readFileSync("/tmp/f4_clean_state.txt")).toBe("pristine-v1");
        expect(Slice.kv.get("hook:runs")).toBe("1");

        // Dirty both VFS and KV
        Slice.kv.set("ledger:balance", "999999", 0);
        fs.writeFileSync("/tmp/f4_clean_state.txt", "corrupted-by-test-1");
        fs.writeFileSync("/tmp/f4_ephemeral.txt", "should-vanish");
        expect(Slice.kv.get("ledger:balance")).toBe("999999");
        expect(fs.existsSync("/tmp/f4_ephemeral.txt")).toBeTruthy();
    });

    test("sees 100% clean VFS and KV state in test 2 via automatic < 5us rollback", () => {
        expect(Slice.kv.get("ledger:balance")).toBe("1000");
        expect(fs.readFileSync("/tmp/f4_clean_state.txt")).toBe("pristine-v1");
        expect(fs.existsSync("/tmp/f4_ephemeral.txt")).toBeFalsy();
        // even `beforeEach` counter from test 1 was rolled back before test 2's `beforeEach`!
        expect(Slice.kv.get("hook:runs")).toBe("1");
    });

    test("enforces OCap and resource assertions (.toViolate, .toStayWithinFuel)", () => {
        const fsAttack = Slice.evalSandbox(`const s = fs.readFileSync("/etc/shadow");`, {
            allowRead: ["/app/public"],
            fuel: 100_000,
        });
        expect(fsAttack).toViolate("FsReadDenied");

        const netAttack = Slice.evalSandbox(`const r = fetch("http://169.254.169.254/latest/meta-data/");`, {
            allowNet: ["*"],
            fuel: 100_000,
        });
        expect(netAttack).toViolate("169.254.169.254");

        const boundedCompute = Slice.evalSandbox(`const x: number = 21 * 2; x`, {
            fuel: 50_000,
            memoryBytes: 65536,
        });
        expect(boundedCompute).toStayWithinFuel(1000);
        expect(boundedCompute).toStayWithinMemory(4096);
    });
});

console.log(JSON.stringify(Slice.testSummary()));
"#;
    let bundled = slice_cas::strip_typescript_and_normalize(ts_test_suite, true);
    let bc = compile_source_to_bytecode(&bundled);
    let mut iso = SliceIsolate::spawn(
        "f4-test-runner",
        1,
        CapabilityToken::pure_compute()
            .with_read_prefix("/tmp")
            .with_write_prefix("/tmp"),
        10_000_000,
        8 * 1024 * 1024,
    );
    iso.set_deterministic_seed(Some(20261024));
    let metrics = iso.execute_bytecode(&bc, None).unwrap();
    let summary: serde_json::Value = serde_json::from_str(&metrics.stdout[0]).unwrap();
    assert_eq!(summary["total"], 3);
    assert_eq!(summary["passed"], 3);
    assert_eq!(summary["failed"], 0);
    assert_eq!(summary["checkpointsRestored"], 3);

    // 2. Verify Auto-DLP `.env` loading (`load_dotenv_str`):
    //    - Non-sensitive `APP_ENV` can be logged to stdout
    //    - Sensitive `STRIPE_SECRET_KEY` loaded from `.env` is readable inside the isolate via `process.env` / `env()`,
    //      but attempting to print it (even Base64-encoded!) is automatically blocked by Egress DLP!
    let dotenv_content = r#"
APP_ENV=staging
STRIPE_SECRET_KEY="sk_live_f4_auto_dlp_vault_secret"
"#;
    let ts_safe = r#"
const mode = process.env.APP_ENV;
const hasSecret = env("STRIPE_SECRET_KEY").startsWith("sk_live_");
console.log(JSON.stringify({ mode, hasSecret }));
"#;
    let bc_safe =
        compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(ts_safe, true));
    let mut iso_env = SliceIsolate::spawn(
        "f4-dotenv-tenant",
        2,
        CapabilityToken::pure_compute(),
        5_000_000,
        4 * 1024 * 1024,
    );
    let auto_secrets = iso_env.load_dotenv_str(dotenv_content);
    assert_eq!(auto_secrets, 1);
    let safe_metrics = iso_env.execute_bytecode(&bc_safe, None).unwrap();
    assert_eq!(
        safe_metrics.stdout[0],
        r#"{"mode":"staging","hasSecret":true}"#
    );

    let ts_leak = r#"
const leakedB64 = Buffer.from(process.env.STRIPE_SECRET_KEY).toString("base64");
console.log("exfil:" + leakedB64);
"#;
    let bc_leak =
        compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(ts_leak, true));
    let leak_res = iso_env.execute_bytecode(&bc_leak, None);
    assert!(leak_res.is_err());
    assert!(leak_res
        .unwrap_err()
        .to_string()
        .contains("EGRESS_SECRET_LEAK_BLOCKED"));
}

#[test]
fn test_phase_n0_general_purpose_numeric_vm_any_function_name() {
    // 1. Structural linear recurrence detection on ANY function name and parameter name
    //    (not hardcoded to `fib(n)`), including weighted 2-term recurrences `c1 * f(k-1) + c2 * f(k-2)`.
    let src_recurrence = r#"
function customSequenceAlpha(stepIdx) {
    if (stepIdx <= 1) {
        return stepIdx;
    }
    return customSequenceAlpha(stepIdx - 1) + customSequenceAlpha(stepIdx - 2);
}
function weightedPellSequence(k) {
    if (k <= 1) {
        return k;
    }
    return 2 * weightedPellSequence(k - 1) + weightedPellSequence(k - 2);
}
const f40 = customSequenceAlpha(40);
const pell7 = weightedPellSequence(7);
console.log(f40 + ":" + pell7);
return f40;
"#;
    let m1 = run_source_isolated(
        "n0-recurrence-tenant",
        src_recurrence,
        CapabilityToken::pure_compute(),
        500_000,
        65536,
    )
    .unwrap();
    assert_eq!(m1.return_value, "102334155");
    assert_eq!(m1.stdout[0], "102334155:169");
    assert_eq!(m1.highest_tier_reached, ExecutionTier::Tier2PeakNative);

    // 2. General-purpose unboxed 64-bit register compilation of multi-argument functions with
    //    `while`, `for`, `if/else`, `break`, `continue`, bitwise XOR/shifts, and `Math.*` intrinsics.
    let src_general_numeric = r#"
function computeBitwiseMixAndCollatz(limit, seedVal) {
    let acc = seedVal;
    for (let i = 1; i <= limit; i++) {
        if (i % 7 === 0) {
            continue;
        }
        let n = i;
        let steps = 0;
        while (n > 1) {
            if ((n & 1) === 0) {
                n = n >> 1;
            } else {
                n = 3 * n + 1;
            }
            steps += 1;
            if (steps > 64) {
                break;
            }
        }
        acc = (acc ^ (steps << 2)) + Math.abs(Math.floor(Math.sqrt(i)));
    }
    return Math.max(acc, 1);
}
function euclideanGcd(a, b) {
    if (b === 0) {
        return a;
    }
    return euclideanGcd(b, a % b);
}
const mix = computeBitwiseMixAndCollatz(120, 17);
const g = euclideanGcd(1071, 462);
console.log(mix + ":" + g);
return g;
"#;
    let m2 = run_source_isolated(
        "n0-numeric-vm-tenant",
        src_general_numeric,
        CapabilityToken::pure_compute(),
        500_000,
        65536,
    )
    .unwrap();
    assert_eq!(m2.return_value, "21");
    assert!(m2.stdout[0].ends_with(":21"));
    assert_eq!(m2.highest_tier_reached, ExecutionTier::Tier2PeakNative);

    // 3. Fuel preemption inside the unboxed register VM on unbounded numeric loops
    let src_infinite_numeric = r#"
function runawayNumericLoop(startVal) {
    let cur = startVal;
    while (cur > 0) {
        cur = cur + 1;
    }
    return cur;
}
return runawayNumericLoop(1);
"#;
    let err = run_source_isolated(
        "n0-runaway-tenant",
        src_infinite_numeric,
        CapabilityToken::pure_compute(),
        5_000,
        65536,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("CPU instruction fuel limit"),
        "Expected CpuFuelExhausted from unboxed register VM, got: {}",
        err
    );
}

#[test]
fn test_phase_n0_dynamic_ses_lockdown_and_boa_loop_fuel_limits() {
    // 1. Dynamic computed-property prototype pollution inside Boa (`Object["proto" + "type"].polluted = 1`)
    //    must be caught, blocked with `IntrinsicMutationBlocked`, and rolled back so the warm realm stays clean!
    let dynamic_pollution_js = r#"
class ExploitAttempt {
    run() {
        const key = "proto" + "type";
        Object[key].pollutedByTenantA = "compromised";
        return 1;
    }
}
new ExploitAttempt().run();
"#;
    let err_ses = run_source_isolated(
        "n0-ses-attacker",
        dynamic_pollution_js,
        CapabilityToken::pure_compute(),
        200_000,
        2 * 1024 * 1024,
    )
    .unwrap_err();
    assert!(
        err_ses
            .to_string()
            .contains("attempted to mutate frozen intrinsic 'Object.prototype'"),
        "Expected IntrinsicMutationBlocked on dynamic Object.prototype mutation, got: {}",
        err_ses
    );

    // Verify that the next tenant acquiring the warm Boa realm sees a completely clean `Object.prototype`
    let verify_clean_js = r#"
class CleanCheck {
    check() {
        const obj = {};
        return String(obj.pollutedByTenantA === undefined);
    }
}
console.log(new CleanCheck().check());
"#;
    let clean_metrics = run_source_isolated(
        "n0-clean-tenant",
        verify_clean_js,
        CapabilityToken::pure_compute(),
        200_000,
        2 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(clean_metrics.stdout[0], "true");

    // 2. Infinite loop inside Boa (`class Spin { loop() { while (true) {} } }`) must be preempted by Boa RuntimeLimits
    let boa_infinite_loop_js = r#"
class SpinWorker {
    loopForever() {
        let k = 0;
        while (true) {
            k++;
        }
        return k;
    }
}
new SpinWorker().loopForever();
"#;
    let err_loop = run_source_isolated(
        "n0-boa-loop-tenant",
        boa_infinite_loop_js,
        CapabilityToken::pure_compute(),
        2_000,
        2 * 1024 * 1024,
    )
    .unwrap_err();
    assert!(
        err_loop.to_string().contains("CPU instruction fuel limit"),
        "Expected CpuFuelExhausted from Boa RuntimeLimits loop preemption, got: {}",
        err_loop
    );

    // 3. Real host `os.platform()` / `process.platform` in normal mode vs `"linux"` in deterministic replay mode
    let os_check_js = r#"
const os = require("node:os");
console.log(os.platform() + ":" + process.platform + ":" + os.arch());
"#;
    let bc_os = compile_source_to_bytecode(os_check_js);
    let mut iso_host = SliceIsolate::spawn(
        "n0-os-host",
        1,
        CapabilityToken::pure_compute(),
        200_000,
        2 * 1024 * 1024,
    );
    let host_m = iso_host.execute_bytecode(&bc_os, None).unwrap();
    let expected_host_os = match std::env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        other => other,
    };
    assert!(
        host_m.stdout[0].starts_with(&format!("{}:{}:", expected_host_os, expected_host_os)),
        "Expected real host platform {}, got: {}",
        expected_host_os,
        host_m.stdout[0]
    );

    let mut iso_det = SliceIsolate::spawn(
        "n0-os-det",
        1,
        CapabilityToken::pure_compute(),
        200_000,
        2 * 1024 * 1024,
    );
    iso_det.set_deterministic_seed(Some(42));
    let det_m = iso_det.execute_bytecode(&bc_os, None).unwrap();
    assert_eq!(det_m.stdout[0], "linux:linux:x64");
}

#[test]
fn test_phase_n1_tls13_https_keepalive_pooling_chunked_and_redirect_ssrf_guard() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;

    // P-256 X.509v3 test CA + localhost server cert valid 2020-01-01 to 2049-12-31
    const TEST_ROOT_CA_DER: &[u8] = &[
        0x30, 0x82, 0x01, 0x6c, 0x30, 0x82, 0x01, 0x12, 0xa0, 0x03, 0x02, 0x01, 0x02, 0x02, 0x02,
        0x10, 0x00, 0x30, 0x0a, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02, 0x30,
        0x1d, 0x31, 0x1b, 0x30, 0x19, 0x06, 0x03, 0x55, 0x04, 0x03, 0x0c, 0x12, 0x53, 0x6c, 0x69,
        0x63, 0x65, 0x20, 0x54, 0x65, 0x73, 0x74, 0x20, 0x52, 0x6f, 0x6f, 0x74, 0x20, 0x43, 0x41,
        0x30, 0x1e, 0x17, 0x0d, 0x32, 0x30, 0x30, 0x31, 0x30, 0x31, 0x30, 0x30, 0x30, 0x30, 0x30,
        0x30, 0x5a, 0x17, 0x0d, 0x34, 0x39, 0x31, 0x32, 0x33, 0x31, 0x32, 0x33, 0x35, 0x39, 0x35,
        0x39, 0x5a, 0x30, 0x1d, 0x31, 0x1b, 0x30, 0x19, 0x06, 0x03, 0x55, 0x04, 0x03, 0x0c, 0x12,
        0x53, 0x6c, 0x69, 0x63, 0x65, 0x20, 0x54, 0x65, 0x73, 0x74, 0x20, 0x52, 0x6f, 0x6f, 0x74,
        0x20, 0x43, 0x41, 0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02,
        0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00, 0x04,
        0x9e, 0x7b, 0xab, 0x85, 0x74, 0xeb, 0xa0, 0x49, 0xba, 0xf0, 0xe4, 0x03, 0x87, 0x4b, 0x60,
        0x9d, 0x24, 0xaf, 0x5b, 0x2d, 0xae, 0xfd, 0x37, 0x91, 0x4c, 0x0b, 0x05, 0x24, 0xb1, 0x78,
        0x5b, 0xb7, 0xaa, 0x3e, 0xd8, 0xc9, 0x9f, 0xf0, 0x79, 0xc7, 0x11, 0x2e, 0xf7, 0x36, 0x9a,
        0xb8, 0xac, 0x25, 0x86, 0x09, 0xb1, 0xad, 0xfa, 0x88, 0x19, 0x38, 0x1d, 0x11, 0xb3, 0x90,
        0xc9, 0xe5, 0xf9, 0xbb, 0xa3, 0x42, 0x30, 0x40, 0x30, 0x0f, 0x06, 0x03, 0x55, 0x1d, 0x13,
        0x01, 0x01, 0xff, 0x04, 0x05, 0x30, 0x03, 0x01, 0x01, 0xff, 0x30, 0x0e, 0x06, 0x03, 0x55,
        0x1d, 0x0f, 0x01, 0x01, 0xff, 0x04, 0x04, 0x03, 0x02, 0x01, 0x06, 0x30, 0x1d, 0x06, 0x03,
        0x55, 0x1d, 0x0e, 0x04, 0x16, 0x04, 0x14, 0x48, 0x17, 0x63, 0x74, 0x40, 0xab, 0xaa, 0x18,
        0xe8, 0xa7, 0x5f, 0x2a, 0xc2, 0xb0, 0x9b, 0xa9, 0x4b, 0x77, 0xd8, 0x14, 0x30, 0x0a, 0x06,
        0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02, 0x03, 0x48, 0x00, 0x30, 0x45, 0x02,
        0x20, 0x3b, 0xc7, 0xf2, 0x8a, 0xaa, 0xa5, 0xe8, 0xc1, 0x37, 0xdc, 0x7c, 0xbd, 0x2f, 0x84,
        0xf3, 0xc1, 0x81, 0x35, 0xee, 0x13, 0x5d, 0x68, 0x72, 0x82, 0x4e, 0xf0, 0xbe, 0x89, 0xf4,
        0x20, 0xdd, 0xca, 0x02, 0x21, 0x00, 0x89, 0xf8, 0x4a, 0x0a, 0x86, 0x0a, 0x57, 0x8f, 0x5b,
        0x96, 0x9e, 0x27, 0x90, 0xa6, 0x3c, 0x47, 0xaf, 0x7f, 0x48, 0x7f, 0xe0, 0x80, 0xed, 0xaa,
        0x38, 0x34, 0xd2, 0x64, 0x80, 0x6f, 0x16, 0xa5,
    ];
    const TEST_SERVER_CERT_DER: &[u8] = &[
        0x30, 0x82, 0x01, 0xb1, 0x30, 0x82, 0x01, 0x57, 0xa0, 0x03, 0x02, 0x01, 0x02, 0x02, 0x02,
        0x10, 0x01, 0x30, 0x0a, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02, 0x30,
        0x1d, 0x31, 0x1b, 0x30, 0x19, 0x06, 0x03, 0x55, 0x04, 0x03, 0x0c, 0x12, 0x53, 0x6c, 0x69,
        0x63, 0x65, 0x20, 0x54, 0x65, 0x73, 0x74, 0x20, 0x52, 0x6f, 0x6f, 0x74, 0x20, 0x43, 0x41,
        0x30, 0x1e, 0x17, 0x0d, 0x32, 0x30, 0x30, 0x31, 0x30, 0x31, 0x30, 0x30, 0x30, 0x30, 0x30,
        0x30, 0x5a, 0x17, 0x0d, 0x34, 0x39, 0x31, 0x32, 0x33, 0x31, 0x32, 0x33, 0x35, 0x39, 0x35,
        0x39, 0x5a, 0x30, 0x14, 0x31, 0x12, 0x30, 0x10, 0x06, 0x03, 0x55, 0x04, 0x03, 0x0c, 0x09,
        0x6c, 0x6f, 0x63, 0x61, 0x6c, 0x68, 0x6f, 0x73, 0x74, 0x30, 0x59, 0x30, 0x13, 0x06, 0x07,
        0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03,
        0x01, 0x07, 0x03, 0x42, 0x00, 0x04, 0xcd, 0xa4, 0xe9, 0x55, 0x58, 0x89, 0xe7, 0xc6, 0x5e,
        0x05, 0xf0, 0x5f, 0x6e, 0xd0, 0x53, 0x4b, 0x4e, 0x16, 0x9a, 0xa3, 0xbe, 0x45, 0x8c, 0x5d,
        0x47, 0xe9, 0x54, 0x5b, 0x03, 0x5e, 0xec, 0x15, 0x71, 0xf8, 0x47, 0xfc, 0xeb, 0x33, 0xde,
        0xef, 0x4e, 0xc6, 0xdf, 0x32, 0x93, 0xdf, 0x22, 0x50, 0xf2, 0x8c, 0xbf, 0xd3, 0xa3, 0x94,
        0xa4, 0xf3, 0x57, 0xff, 0xa8, 0xb2, 0x5c, 0x7f, 0x12, 0x5c, 0xa3, 0x81, 0x8f, 0x30, 0x81,
        0x8c, 0x30, 0x09, 0x06, 0x03, 0x55, 0x1d, 0x13, 0x04, 0x02, 0x30, 0x00, 0x30, 0x0e, 0x06,
        0x03, 0x55, 0x1d, 0x0f, 0x01, 0x01, 0xff, 0x04, 0x04, 0x03, 0x02, 0x05, 0xa0, 0x30, 0x13,
        0x06, 0x03, 0x55, 0x1d, 0x25, 0x04, 0x0c, 0x30, 0x0a, 0x06, 0x08, 0x2b, 0x06, 0x01, 0x05,
        0x05, 0x07, 0x03, 0x01, 0x30, 0x1a, 0x06, 0x03, 0x55, 0x1d, 0x11, 0x04, 0x13, 0x30, 0x11,
        0x82, 0x09, 0x6c, 0x6f, 0x63, 0x61, 0x6c, 0x68, 0x6f, 0x73, 0x74, 0x87, 0x04, 0x7f, 0x00,
        0x00, 0x01, 0x30, 0x1d, 0x06, 0x03, 0x55, 0x1d, 0x0e, 0x04, 0x16, 0x04, 0x14, 0x86, 0x03,
        0xe1, 0xe2, 0x8a, 0x26, 0x16, 0x3f, 0x55, 0xb8, 0x3e, 0xaa, 0x31, 0x2a, 0xca, 0x3e, 0xd3,
        0x65, 0x04, 0xbf, 0x30, 0x1f, 0x06, 0x03, 0x55, 0x1d, 0x23, 0x04, 0x18, 0x30, 0x16, 0x80,
        0x14, 0x48, 0x17, 0x63, 0x74, 0x40, 0xab, 0xaa, 0x18, 0xe8, 0xa7, 0x5f, 0x2a, 0xc2, 0xb0,
        0x9b, 0xa9, 0x4b, 0x77, 0xd8, 0x14, 0x30, 0x0a, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d,
        0x04, 0x03, 0x02, 0x03, 0x48, 0x00, 0x30, 0x45, 0x02, 0x21, 0x00, 0xf7, 0x2d, 0xf4, 0x6c,
        0xf0, 0x37, 0x42, 0x96, 0xe9, 0x1d, 0x39, 0xbe, 0xa9, 0x96, 0x19, 0x80, 0x5d, 0x6d, 0x83,
        0xb6, 0xc0, 0x0a, 0xec, 0x40, 0x08, 0x8a, 0xf8, 0x45, 0xf2, 0xd6, 0x30, 0x1e, 0x02, 0x20,
        0x69, 0x67, 0xce, 0x04, 0x69, 0xf0, 0xec, 0xac, 0xcb, 0x45, 0x57, 0x43, 0x22, 0x6a, 0x5f,
        0x0f, 0x98, 0x1a, 0xf5, 0x10, 0x2a, 0x7b, 0x65, 0xb7, 0x4e, 0x54, 0x2c, 0x39, 0x16, 0xd6,
        0x0b, 0x8e,
    ];
    const TEST_SERVER_KEY_DER: &[u8] = &[
        0x30, 0x81, 0x87, 0x02, 0x01, 0x00, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d,
        0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x04, 0x6d, 0x30,
        0x6b, 0x02, 0x01, 0x01, 0x04, 0x20, 0x85, 0x70, 0x8c, 0xa7, 0xe2, 0xdb, 0xd1, 0x7d, 0x7d,
        0xf1, 0x75, 0x61, 0x7b, 0x2e, 0x93, 0x6d, 0x0c, 0xd1, 0x45, 0x19, 0xa6, 0xfe, 0x14, 0xc9,
        0x6c, 0x5f, 0x32, 0xf4, 0xab, 0x3b, 0x3d, 0x2a, 0xa1, 0x44, 0x03, 0x42, 0x00, 0x04, 0xcd,
        0xa4, 0xe9, 0x55, 0x58, 0x89, 0xe7, 0xc6, 0x5e, 0x05, 0xf0, 0x5f, 0x6e, 0xd0, 0x53, 0x4b,
        0x4e, 0x16, 0x9a, 0xa3, 0xbe, 0x45, 0x8c, 0x5d, 0x47, 0xe9, 0x54, 0x5b, 0x03, 0x5e, 0xec,
        0x15, 0x71, 0xf8, 0x47, 0xfc, 0xeb, 0x33, 0xde, 0xef, 0x4e, 0xc6, 0xdf, 0x32, 0x93, 0xdf,
        0x22, 0x50, 0xf2, 0x8c, 0xbf, 0xd3, 0xa3, 0x94, 0xa4, 0xf3, 0x57, 0xff, 0xa8, 0xb2, 0x5c,
        0x7f, 0x12, 0x5c,
    ];

    crate::register_custom_root_cert_der(TEST_ROOT_CA_DER.to_vec());
    crate::clear_outbound_conn_pool_for_tenant("n1-tls-tenant");
    crate::clear_outbound_conn_pool_for_tenant("n1-http-tenant");

    // 1. Real TLS 1.3 HTTPS server (`rustls::ServerConnection`) serving 2 Keep-Alive requests on 1 TLS session
    let tls_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let tls_port = tls_listener.local_addr().unwrap().port();
    let tls_server_handle = std::thread::spawn(move || {
        let cert = rustls_pki_types::CertificateDer::from(TEST_SERVER_CERT_DER.to_vec());
        let key = rustls_pki_types::PrivateKeyDer::Pkcs8(
            rustls_pki_types::PrivatePkcs8KeyDer::from(TEST_SERVER_KEY_DER.to_vec()),
        );
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let srv_cfg = Arc::new(
            rustls::ServerConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(vec![cert], key)
                .unwrap(),
        );
        if let Ok((tcp_stream, _)) = tls_listener.accept() {
            let conn = rustls::ServerConnection::new(srv_cfg).unwrap();
            let mut tls_stream = rustls::StreamOwned::new(conn, tcp_stream);
            let mut buf = [0u8; 4096];
            // Serve 2 sequential HTTPS requests over the SAME TLS 1.3 Keep-Alive connection
            for seq in 1..=2 {
                let _ = tls_stream.read(&mut buf);
                let body = format!(r#"{{"tls13":true,"seq":{}}}"#, seq);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: keep-alive\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = tls_stream.write_all(resp.as_bytes());
                let _ = tls_stream.flush();
            }
        }
    });

    let reuse_before_tls = crate::outbound_pool_reuse_count();
    let tls_js = format!(
        r#"
async function runTls() {{
    const r1 = await fetch("https://localhost:{port}/v1/alpha");
    const j1 = await r1.json();
    const r2 = await fetch("https://localhost:{port}/v1/beta");
    const j2 = await r2.json();
    console.log(JSON.stringify({{ s1: j1.seq, s2: j2.seq, ok: r1.ok && r2.ok }}));
}}
runTls();
"#,
        port = tls_port
    );
    let tls_caps = CapabilityToken::pure_compute().with_net_host(format!("localhost:{}", tls_port));
    let tls_metrics = run_source_isolated(
        "n1-tls-tenant",
        &tls_js,
        tls_caps,
        2_000_000,
        4 * 1024 * 1024,
    )
    .unwrap();
    let _ = tls_server_handle.join();
    assert_eq!(tls_metrics.stdout[0], r#"{"s1":1,"s2":2,"ok":true}"#);
    assert!(
        crate::outbound_pool_reuse_count() > reuse_before_tls,
        "Expected TLS 1.3 connection pool reuse on second HTTPS request"
    );

    // 2. HTTP/1.1 Keep-Alive + Transfer-Encoding: chunked + Redirects + Cross-Origin Auth Stripping
    let origin_a = TcpListener::bind("127.0.0.1:0").unwrap();
    let port_a = origin_a.local_addr().unwrap().port();
    let origin_b = TcpListener::bind("127.0.0.1:0").unwrap();
    let port_b = origin_b.local_addr().unwrap().port();

    // Origin B accepts 1 cross-origin redirected request and verifies `Authorization` was stripped
    let handle_b = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = origin_b.accept() {
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req_str = String::from_utf8_lossy(&buf[..n]).to_lowercase();
            let leaked_auth = req_str.contains("authorization:");
            let leaked_cookie = req_str.contains("cookie:");
            let body = format!(
                r#"{{"origin":"B","leakedAuth":{},"leakedCookie":{}}}"#,
                leaked_auth, leaked_cookie
            );
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });

    // Origin A serves 5 sequential requests over a SINGLE Keep-Alive TcpStream:
    //   Req 1: GET /chunked -> 200 OK with `Transfer-Encoding: chunked`
    //   Req 2: GET /hop1    -> 302 Found -> `/hop2` (same-origin redirect)
    //   Req 3: GET /hop2    -> 200 OK with `Content-Length` (verifies `Authorization` was preserved same-origin)
    //   Req 4: GET /manual  -> 302 Found -> `/hop2` (client uses `redirect: "manual"`)
    //   Req 5: GET /xorigin -> 302 Found -> `http://127.0.0.1:<port_b>/target` (cross-origin redirect)
    //   Req 6: GET /ssrf-redir -> 302 Found -> `http://169.254.169.254/latest/meta-data/iam`
    let handle_a = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = origin_a.accept() {
            let mut buf = [0u8; 4096];
            for _ in 0..6 {
                let n = match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let req_str = String::from_utf8_lossy(&buf[..n]);
                let first_line = req_str.lines().next().unwrap_or("");
                if first_line.contains("/chunked") {
                    let resp = concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: text/plain\r\n",
                        "Transfer-Encoding: chunked\r\n",
                        "Connection: keep-alive\r\n\r\n",
                        "5\r\nhello\r\n",
                        "6\r\n-world\r\n",
                        "0\r\n\r\n"
                    );
                    let _ = stream.write_all(resp.as_bytes());
                } else if first_line.contains("/hop1") || first_line.contains("/manual") {
                    let resp = "HTTP/1.1 302 Found\r\nLocation: /hop2\r\nContent-Length: 0\r\nConnection: keep-alive\r\n\r\n";
                    let _ = stream.write_all(resp.as_bytes());
                } else if first_line.contains("/hop2") {
                    let has_auth = req_str
                        .to_lowercase()
                        .contains("authorization: bearer same-origin-token");
                    let body = format!(r#"{{"hop2":true,"sameOriginAuth":{}}}"#, has_auth);
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: keep-alive\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes());
                } else if first_line.contains("/xorigin") {
                    let resp = format!(
                        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{}/target\r\nContent-Length: 0\r\nConnection: keep-alive\r\n\r\n",
                        port_b
                    );
                    let _ = stream.write_all(resp.as_bytes());
                } else if first_line.contains("/ssrf-redir") {
                    let resp = "HTTP/1.1 302 Found\r\nLocation: http://169.254.169.254/latest/meta-data/iam\r\nContent-Length: 0\r\nConnection: keep-alive\r\n\r\n";
                    let _ = stream.write_all(resp.as_bytes());
                }
                let _ = stream.flush();
            }
        }
    });

    let reuse_before_http = crate::outbound_pool_reuse_count();
    let mesh_js = format!(
        r#"
async function runSuite() {{
    const rChunked = await fetch("http://127.0.0.1:{port_a}/chunked");
    const txtChunked = await rChunked.text();

    const rRedir = await fetch("http://127.0.0.1:{port_a}/hop1", {{
        headers: {{ "Authorization": "Bearer same-origin-token" }}
    }});
    const jRedir = await rRedir.json();

    const rManual = await fetch("http://127.0.0.1:{port_a}/manual", {{
        redirect: "manual"
    }});

    const rCross = await fetch("http://127.0.0.1:{port_a}/xorigin", {{
        headers: {{
            "Authorization": "Bearer secret-jwt-token",
            "Cookie": "sid=private-cookie"
        }}
    }});
    const jCross = await rCross.json();

    console.log(JSON.stringify({{
        chunked: txtChunked,
        redirected: rRedir.redirected,
        finalPath: new URL(rRedir.url).pathname,
        sameOriginAuth: jRedir.sameOriginAuth,
        manualStatus: rManual.status,
        manualLocation: rManual.headers.get("location"),
        crossOriginLeakedAuth: jCross.leakedAuth,
        crossOriginLeakedCookie: jCross.leakedCookie
    }}));
}}
runSuite();
"#,
        port_a = port_a
    );
    let mesh_caps = CapabilityToken::pure_compute()
        .with_net_host(format!("127.0.0.1:{}", port_a))
        .with_net_host(format!("127.0.0.1:{}", port_b));
    let mesh_metrics = run_source_isolated(
        "n1-mesh-tenant",
        &mesh_js,
        mesh_caps.clone(),
        4_000_000,
        4 * 1024 * 1024,
    )
    .unwrap();
    let summary: serde_json::Value = serde_json::from_str(&mesh_metrics.stdout[0]).unwrap();
    assert_eq!(summary["chunked"], "hello-world");
    assert_eq!(summary["redirected"], true);
    assert_eq!(summary["finalPath"], "/hop2");
    assert_eq!(summary["sameOriginAuth"], true);
    assert_eq!(summary["manualStatus"], 302);
    assert_eq!(summary["manualLocation"], "/hop2");
    assert_eq!(summary["crossOriginLeakedAuth"], false);
    assert_eq!(summary["crossOriginLeakedCookie"], false);
    assert!(
        crate::outbound_pool_reuse_count() >= reuse_before_http + 3,
        "Expected Keep-Alive connection reuse across Origin A requests"
    );

    // 3. Verify 302 Redirect to Cloud Metadata IP (`169.254.169.254`) traps immediately!
    let ssrf_redir_js = format!(
        r#"
async function attack() {{
    await fetch("http://127.0.0.1:{}/ssrf-redir");
}}
attack();
"#,
        port_a
    );
    let err_redir = run_source_isolated(
        "n1-mesh-tenant",
        &ssrf_redir_js,
        mesh_caps,
        1_000_000,
        4 * 1024 * 1024,
    )
    .unwrap_err();
    assert!(
        err_redir.to_string().contains("SSRF_METADATA_BLOCKED"),
        "Expected SSRF_METADATA_BLOCKED on 302 redirect to 169.254.169.254, got: {}",
        err_redir
    );

    let _ = handle_a.join();
    let _ = handle_b.join();
}

#[test]
fn test_outbound_fetch_port_isolation_authority_alignment_and_https_proxy_connect_tunnel() {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};

    // Reuse valid P-256 test Root CA + `localhost` leaf cert
    const ROOT_CA_DER: &[u8] = &[
        0x30, 0x82, 0x01, 0x6c, 0x30, 0x82, 0x01, 0x12, 0xa0, 0x03, 0x02, 0x01, 0x02, 0x02, 0x02,
        0x10, 0x00, 0x30, 0x0a, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02, 0x30,
        0x1d, 0x31, 0x1b, 0x30, 0x19, 0x06, 0x03, 0x55, 0x04, 0x03, 0x0c, 0x12, 0x53, 0x6c, 0x69,
        0x63, 0x65, 0x20, 0x54, 0x65, 0x73, 0x74, 0x20, 0x52, 0x6f, 0x6f, 0x74, 0x20, 0x43, 0x41,
        0x30, 0x1e, 0x17, 0x0d, 0x32, 0x30, 0x30, 0x31, 0x30, 0x31, 0x30, 0x30, 0x30, 0x30, 0x30,
        0x30, 0x5a, 0x17, 0x0d, 0x34, 0x39, 0x31, 0x32, 0x33, 0x31, 0x32, 0x33, 0x35, 0x39, 0x35,
        0x39, 0x5a, 0x30, 0x1d, 0x31, 0x1b, 0x30, 0x19, 0x06, 0x03, 0x55, 0x04, 0x03, 0x0c, 0x12,
        0x53, 0x6c, 0x69, 0x63, 0x65, 0x20, 0x54, 0x65, 0x73, 0x74, 0x20, 0x52, 0x6f, 0x6f, 0x74,
        0x20, 0x43, 0x41, 0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02,
        0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00, 0x04,
        0x9e, 0x7b, 0xab, 0x85, 0x74, 0xeb, 0xa0, 0x49, 0xba, 0xf0, 0xe4, 0x03, 0x87, 0x4b, 0x60,
        0x9d, 0x24, 0xaf, 0x5b, 0x2d, 0xae, 0xfd, 0x37, 0x91, 0x4c, 0x0b, 0x05, 0x24, 0xb1, 0x78,
        0x5b, 0xb7, 0xaa, 0x3e, 0xd8, 0xc9, 0x9f, 0xf0, 0x79, 0xc7, 0x11, 0x2e, 0xf7, 0x36, 0x9a,
        0xb8, 0xac, 0x25, 0x86, 0x09, 0xb1, 0xad, 0xfa, 0x88, 0x19, 0x38, 0x1d, 0x11, 0xb3, 0x90,
        0xc9, 0xe5, 0xf9, 0xbb, 0xa3, 0x42, 0x30, 0x40, 0x30, 0x0f, 0x06, 0x03, 0x55, 0x1d, 0x13,
        0x01, 0x01, 0xff, 0x04, 0x05, 0x30, 0x03, 0x01, 0x01, 0xff, 0x30, 0x0e, 0x06, 0x03, 0x55,
        0x1d, 0x0f, 0x01, 0x01, 0xff, 0x04, 0x04, 0x03, 0x02, 0x01, 0x06, 0x30, 0x1d, 0x06, 0x03,
        0x55, 0x1d, 0x0e, 0x04, 0x16, 0x04, 0x14, 0x48, 0x17, 0x63, 0x74, 0x40, 0xab, 0xaa, 0x18,
        0xe8, 0xa7, 0x5f, 0x2a, 0xc2, 0xb0, 0x9b, 0xa9, 0x4b, 0x77, 0xd8, 0x14, 0x30, 0x0a, 0x06,
        0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02, 0x03, 0x48, 0x00, 0x30, 0x45, 0x02,
        0x20, 0x3b, 0xc7, 0xf2, 0x8a, 0xaa, 0xa5, 0xe8, 0xc1, 0x37, 0xdc, 0x7c, 0xbd, 0x2f, 0x84,
        0xf3, 0xc1, 0x81, 0x35, 0xee, 0x13, 0x5d, 0x68, 0x72, 0x82, 0x4e, 0xf0, 0xbe, 0x89, 0xf4,
        0x20, 0xdd, 0xca, 0x02, 0x21, 0x00, 0x89, 0xf8, 0x4a, 0x0a, 0x86, 0x0a, 0x57, 0x8f, 0x5b,
        0x96, 0x9e, 0x27, 0x90, 0xa6, 0x3c, 0x47, 0xaf, 0x7f, 0x48, 0x7f, 0xe0, 0x80, 0xed, 0xaa,
        0x38, 0x34, 0xd2, 0x64, 0x80, 0x6f, 0x16, 0xa5,
    ];
    const SERVER_CERT_DER: &[u8] = &[
        0x30, 0x82, 0x01, 0xb1, 0x30, 0x82, 0x01, 0x57, 0xa0, 0x03, 0x02, 0x01, 0x02, 0x02, 0x02,
        0x10, 0x01, 0x30, 0x0a, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02, 0x30,
        0x1d, 0x31, 0x1b, 0x30, 0x19, 0x06, 0x03, 0x55, 0x04, 0x03, 0x0c, 0x12, 0x53, 0x6c, 0x69,
        0x63, 0x65, 0x20, 0x54, 0x65, 0x73, 0x74, 0x20, 0x52, 0x6f, 0x6f, 0x74, 0x20, 0x43, 0x41,
        0x30, 0x1e, 0x17, 0x0d, 0x32, 0x30, 0x30, 0x31, 0x30, 0x31, 0x30, 0x30, 0x30, 0x30, 0x30,
        0x30, 0x5a, 0x17, 0x0d, 0x34, 0x39, 0x31, 0x32, 0x33, 0x31, 0x32, 0x33, 0x35, 0x39, 0x35,
        0x39, 0x5a, 0x30, 0x14, 0x31, 0x12, 0x30, 0x10, 0x06, 0x03, 0x55, 0x04, 0x03, 0x0c, 0x09,
        0x6c, 0x6f, 0x63, 0x61, 0x6c, 0x68, 0x6f, 0x73, 0x74, 0x30, 0x59, 0x30, 0x13, 0x06, 0x07,
        0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03,
        0x01, 0x07, 0x03, 0x42, 0x00, 0x04, 0xcd, 0xa4, 0xe9, 0x55, 0x58, 0x89, 0xe7, 0xc6, 0x5e,
        0x05, 0xf0, 0x5f, 0x6e, 0xd0, 0x53, 0x4b, 0x4e, 0x16, 0x9a, 0xa3, 0xbe, 0x45, 0x8c, 0x5d,
        0x47, 0xe9, 0x54, 0x5b, 0x03, 0x5e, 0xec, 0x15, 0x71, 0xf8, 0x47, 0xfc, 0xeb, 0x33, 0xde,
        0xef, 0x4e, 0xc6, 0xdf, 0x32, 0x93, 0xdf, 0x22, 0x50, 0xf2, 0x8c, 0xbf, 0xd3, 0xa3, 0x94,
        0xa4, 0xf3, 0x57, 0xff, 0xa8, 0xb2, 0x5c, 0x7f, 0x12, 0x5c, 0xa3, 0x81, 0x8f, 0x30, 0x81,
        0x8c, 0x30, 0x09, 0x06, 0x03, 0x55, 0x1d, 0x13, 0x04, 0x02, 0x30, 0x00, 0x30, 0x0e, 0x06,
        0x03, 0x55, 0x1d, 0x0f, 0x01, 0x01, 0xff, 0x04, 0x04, 0x03, 0x02, 0x05, 0xa0, 0x30, 0x13,
        0x06, 0x03, 0x55, 0x1d, 0x25, 0x04, 0x0c, 0x30, 0x0a, 0x06, 0x08, 0x2b, 0x06, 0x01, 0x05,
        0x05, 0x07, 0x03, 0x01, 0x30, 0x1a, 0x06, 0x03, 0x55, 0x1d, 0x11, 0x04, 0x13, 0x30, 0x11,
        0x82, 0x09, 0x6c, 0x6f, 0x63, 0x61, 0x6c, 0x68, 0x6f, 0x73, 0x74, 0x87, 0x04, 0x7f, 0x00,
        0x00, 0x01, 0x30, 0x1d, 0x06, 0x03, 0x55, 0x1d, 0x0e, 0x04, 0x16, 0x04, 0x14, 0x86, 0x03,
        0xe1, 0xe2, 0x8a, 0x26, 0x16, 0x3f, 0x55, 0xb8, 0x3e, 0xaa, 0x31, 0x2a, 0xca, 0x3e, 0xd3,
        0x65, 0x04, 0xbf, 0x30, 0x1f, 0x06, 0x03, 0x55, 0x1d, 0x23, 0x04, 0x18, 0x30, 0x16, 0x80,
        0x14, 0x48, 0x17, 0x63, 0x74, 0x40, 0xab, 0xaa, 0x18, 0xe8, 0xa7, 0x5f, 0x2a, 0xc2, 0xb0,
        0x9b, 0xa9, 0x4b, 0x77, 0xd8, 0x14, 0x30, 0x0a, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d,
        0x04, 0x03, 0x02, 0x03, 0x48, 0x00, 0x30, 0x45, 0x02, 0x21, 0x00, 0xf7, 0x2d, 0xf4, 0x6c,
        0xf0, 0x37, 0x42, 0x96, 0xe9, 0x1d, 0x39, 0xbe, 0xa9, 0x96, 0x19, 0x80, 0x5d, 0x6d, 0x83,
        0xb6, 0xc0, 0x0a, 0xec, 0x40, 0x08, 0x8a, 0xf8, 0x45, 0xf2, 0xd6, 0x30, 0x1e, 0x02, 0x20,
        0x69, 0x67, 0xce, 0x04, 0x69, 0xf0, 0xec, 0xac, 0xcb, 0x45, 0x57, 0x43, 0x22, 0x6a, 0x5f,
        0x0f, 0x98, 0x1a, 0xf5, 0x10, 0x2a, 0x7b, 0x65, 0xb7, 0x4e, 0x54, 0x2c, 0x39, 0x16, 0xd6,
        0x0b, 0x8e,
    ];
    const SERVER_KEY_DER: &[u8] = &[
        0x30, 0x81, 0x87, 0x02, 0x01, 0x00, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d,
        0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x04, 0x6d, 0x30,
        0x6b, 0x02, 0x01, 0x01, 0x04, 0x20, 0x85, 0x70, 0x8c, 0xa7, 0xe2, 0xdb, 0xd1, 0x7d, 0x7d,
        0xf1, 0x75, 0x61, 0x7b, 0x2e, 0x93, 0x6d, 0x0c, 0xd1, 0x45, 0x19, 0xa6, 0xfe, 0x14, 0xc9,
        0x6c, 0x5f, 0x32, 0xf4, 0xab, 0x3b, 0x3d, 0x2a, 0xa1, 0x44, 0x03, 0x42, 0x00, 0x04, 0xcd,
        0xa4, 0xe9, 0x55, 0x58, 0x89, 0xe7, 0xc6, 0x5e, 0x05, 0xf0, 0x5f, 0x6e, 0xd0, 0x53, 0x4b,
        0x4e, 0x16, 0x9a, 0xa3, 0xbe, 0x45, 0x8c, 0x5d, 0x47, 0xe9, 0x54, 0x5b, 0x03, 0x5e, 0xec,
        0x15, 0x71, 0xf8, 0x47, 0xfc, 0xeb, 0x33, 0xde, 0xef, 0x4e, 0xc6, 0xdf, 0x32, 0x93, 0xdf,
        0x22, 0x50, 0xf2, 0x8c, 0xbf, 0xd3, 0xa3, 0x94, 0xa4, 0xf3, 0x57, 0xff, 0xa8, 0xb2, 0x5c,
        0x7f, 0x12, 0x5c,
    ];

    crate::register_custom_root_cert_der(ROOT_CA_DER.to_vec());

    // 1. Strict port isolation in outbound fetch(): `--allow-net=api.trusted.com:443` must block `:6379` and `:80`
    let port_caps = CapabilityToken::pure_compute().with_net_host("api.trusted.com:443");
    for blocked_url in [
        "http://api.trusted.com:6379/ping",
        "http://api.trusted.com/plain",
        "https://169.254.169.254\\@api.trusted.com/",
        "http://%31%36%39.%32%35%34.%31%36%39.%32%35%34/",
    ] {
        let code = format!(
            "async function run() {{ await fetch({:?}); }}\nrun();",
            blocked_url
        );
        let err = run_source_isolated(
            "fetch-port-isolation",
            &code,
            port_caps.clone(),
            500_000,
            4 * 1024 * 1024,
        )
        .unwrap_err()
        .downcast::<SandboxViolation>()
        .unwrap();
        assert!(
            matches!(err, SandboxViolation::NetAccessDenied { .. }),
            "expected NetAccessDenied for {}, got: {:?}",
            blocked_url,
            err
        );
    }

    // 2. startTls expectedServerHostname OCap check + non-ephemeral UDP bind OCap check
    let sock_caps = CapabilityToken::pure_compute().with_net_host("relay.allowed.example:443");
    let starttls_err = run_source_isolated(
        "starttls-sni-ocap",
        r#"
        import { connect } from "cloudflare:sockets";
        Slice.mockSocket("relay.allowed.example:443", "OK");
        const sock = connect("relay.allowed.example:443", { secureTransport: "starttls" });
        sock.startTls({ expectedServerHostname: "ungranted.evil.example" });
        "#,
        sock_caps.clone(),
        500_000,
        4 * 1024 * 1024,
    )
    .unwrap_err()
    .downcast::<SandboxViolation>()
    .unwrap();
    assert!(matches!(
        starttls_err,
        SandboxViolation::NetAccessDenied { .. }
    ));

    let udp_bind_err = run_source_isolated(
        "udp-bind-ocap",
        r#"
        import dgram from "node:dgram";
        const s = dgram.createSocket("udp4");
        s.bind(5353, "127.0.0.1");
        "#,
        sock_caps,
        500_000,
        4 * 1024 * 1024,
    )
    .unwrap_err()
    .downcast::<SandboxViolation>()
    .unwrap();
    assert!(matches!(
        udp_bind_err,
        SandboxViolation::NetAccessDenied { .. }
    ));

    // 3. HTTPS_PROXY HTTP CONNECT + TLS 1.3 tunnel (verifies zero cleartext leakage to proxy)
    let origin_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin_port = origin_listener.local_addr().unwrap().port();
    let origin_handle = std::thread::spawn(move || {
        let cert = rustls_pki_types::CertificateDer::from(SERVER_CERT_DER.to_vec());
        let key = rustls_pki_types::PrivateKeyDer::Pkcs8(
            rustls_pki_types::PrivatePkcs8KeyDer::from(SERVER_KEY_DER.to_vec()),
        );
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let srv_cfg = Arc::new(
            rustls::ServerConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(vec![cert], key)
                .unwrap(),
        );
        if let Ok((tcp_stream, _)) = origin_listener.accept() {
            let conn = rustls::ServerConnection::new(srv_cfg).unwrap();
            let mut tls_stream = rustls::StreamOwned::new(conn, tcp_stream);
            let mut buf = [0u8; 4096];
            for seq in 1..=2 {
                let n = tls_stream.read(&mut buf).unwrap_or(0);
                let req_txt = String::from_utf8_lossy(&buf[..n]);
                let saw_bearer = req_txt.contains("Bearer super-secret-token");
                let body = format!(
                    r#"{{"tunneled":true,"seq":{},"sawBearer":{}}}"#,
                    seq, saw_bearer
                );
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: keep-alive\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = tls_stream.write_all(resp.as_bytes());
                let _ = tls_stream.flush();
            }
        }
    });

    let proxy_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy_port = proxy_listener.local_addr().unwrap().port();
    let captured_proxy_wire = Arc::new(Mutex::new(Vec::<u8>::new()));
    let captured_clone = Arc::clone(&captured_proxy_wire);

    let proxy_handle = std::thread::spawn(move || {
        if let Ok((mut client_sock, _)) = proxy_listener.accept() {
            let mut hdr = Vec::new();
            let mut b = [0u8; 1];
            while hdr.len() < 4096 {
                if client_sock.read(&mut b).unwrap_or(0) == 0 {
                    break;
                }
                hdr.push(b[0]);
                if hdr.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            captured_clone.lock().unwrap().extend_from_slice(&hdr);

            let mut upstream = TcpStream::connect(("127.0.0.1", origin_port)).unwrap();
            client_sock
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .unwrap();
            client_sock.flush().unwrap();

            let mut c2u_client = client_sock.try_clone().unwrap();
            let mut c2u_upstream = upstream.try_clone().unwrap();
            let cap_c2u = Arc::clone(&captured_clone);
            let t1 = std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = c2u_client.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    cap_c2u.lock().unwrap().extend_from_slice(&buf[..n]);
                    if c2u_upstream.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            });
            let mut buf = [0u8; 4096];
            while let Ok(n) = upstream.read(&mut buf) {
                if n == 0 {
                    break;
                }
                if client_sock.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
            let _ = client_sock.shutdown(std::net::Shutdown::Both);
            let _ = upstream.shutdown(std::net::Shutdown::Both);
            let _ = t1.join();
        }
    });

    crate::clear_outbound_conn_pool_for_tenant("https-proxy-connect-tenant");
    let proxy_caps = CapabilityToken::pure_compute()
        .with_net_host(format!("localhost:{}", origin_port))
        .with_net_host(format!("127.0.0.1:{}", proxy_port))
        .with_env("HTTPS_PROXY", format!("http://127.0.0.1:{}", proxy_port));
    let proxy_js = format!(
        r#"
        async function main() {{
            const r1 = await fetch("https://localhost:{port}/confidential-endpoint?key=secret_q", {{
                headers: {{ Authorization: "Bearer super-secret-token" }}
            }});
            const j1 = await r1.json();
            const r2 = await fetch("https://localhost:{port}/confidential-endpoint?key=secret_q", {{
                headers: {{ Authorization: "Bearer super-secret-token" }}
            }});
            const j2 = await r2.json();
            console.log(JSON.stringify({{ j1, j2 }}));
        }}
        main();
        "#,
        port = origin_port
    );
    let m = run_source_isolated(
        "https-proxy-connect-tenant",
        &proxy_js,
        proxy_caps,
        3_000_000,
        4 * 1024 * 1024,
    )
    .unwrap();
    crate::clear_outbound_conn_pool_for_tenant("https-proxy-connect-tenant");
    let _ = origin_handle.join();
    let _ = proxy_handle.join();

    assert_eq!(
        m.stdout[0],
        r#"{"j1":{"tunneled":true,"seq":1,"sawBearer":true},"j2":{"tunneled":true,"seq":2,"sawBearer":true}}"#
    );

    let wire = captured_proxy_wire.lock().unwrap();
    let wire_lossy = String::from_utf8_lossy(&wire);
    assert!(
        wire_lossy.starts_with(&format!("CONNECT localhost:{} HTTP/1.1\r\n", origin_port)),
        "expected HTTP CONNECT request to proxy, got: {}",
        wire_lossy
    );
    assert!(
        !wire_lossy.contains("confidential-endpoint")
            && !wire_lossy.contains("secret_q")
            && !wire_lossy.contains("super-secret-token"),
        "cleartext path/query/headers must never be visible to HTTPS_PROXY!"
    );
}

#[test]
fn test_wintertc_binary_fetch_1xx_streams_headers_formdata_and_ws_upgrade() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    // 1. Spawn a local HTTP/1.1 Keep-Alive server that emits `100 Continue` + binary `[0, 128, 255, 65]`
    //    on request 1, and echoes binary request body bytes back on the SAME Keep-Alive socket on request 2.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server_handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut buf = [0u8; 4096];

        // Request 1: read headers, send `100 Continue` followed by `200 OK` with non-UTF-8 binary payload
        let mut req1 = Vec::new();
        loop {
            let n = stream.read(&mut buf).unwrap();
            assert!(n > 0);
            req1.extend_from_slice(&buf[..n]);
            if req1.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let bin_payload: &[u8] = &[0x00, 0x80, 0xFF, 0x41, 0xFE, 0xC0];
        let mut resp1 = Vec::new();
        resp1.extend_from_slice(b"HTTP/1.1 100 Continue\r\n\r\n");
        resp1.extend_from_slice(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                bin_payload.len()
            )
            .as_bytes(),
        );
        resp1.extend_from_slice(bin_payload);
        stream.write_all(&resp1).unwrap();
        stream.flush().unwrap();

        // Request 2 on the same Keep-Alive socket: read binary POST body `[0, 129, 254, 255]` and echo it back
        let mut req2 = Vec::new();
        let hdr_end = loop {
            let n = stream.read(&mut buf).unwrap();
            assert!(n > 0);
            req2.extend_from_slice(&buf[..n]);
            if let Some(pos) = req2.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        while req2.len() - hdr_end < 4 {
            let n = stream.read(&mut buf).unwrap();
            assert!(n > 0);
            req2.extend_from_slice(&buf[..n]);
        }
        let posted = &req2[hdr_end..hdr_end + 4];
        assert_eq!(posted, &[0x00, 0x81, 0xFE, 0xFF]);
        let mut resp2 = Vec::new();
        resp2.extend_from_slice(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 4\r\nConnection: close\r\n\r\n",
        );
        resp2.extend_from_slice(posted);
        stream.write_all(&resp2).unwrap();
        stream.flush().unwrap();
    });

    let caps = CapabilityToken::pure_compute().with_net_host(format!("127.0.0.1:{}", port));
    let js = format!(
        r#"
async function main() {{
    // A. Outbound fetch: 100 Continue + lossless non-UTF-8 binary response + binary POST on Keep-Alive socket
    const r1 = await fetch("http://127.0.0.1:{port}/bin", {{ headers: {{ "Expect": "100-continue" }} }});
    const b1 = Array.from(await r1.bytes());
    const r2 = await fetch("http://127.0.0.1:{port}/echo", {{
        method: "POST",
        body: new Uint8Array([0, 129, 254, 255]),
    }});
    const b2 = Array.from(new Uint8Array(await r2.arrayBuffer()));

    // B. URL resolution with colon inside query/hash of relative input
    const uRel = new URL("/callback?next=https://example.com:8443/a", "https://app.io/dir/page");
    const uHash = new URL("step#ref=http://other.test", "https://app.io/dir/page");

    // C. ReadableStream > 4096 chunks without buffer overflow + reader.releaseLock() cursor continuity + async Response(stream).bytes()
    let produced = 0;
    const bigStream = new ReadableStream({{
        pull(c) {{
            if (produced < 10000) {{
                c.enqueue(produced++);
            }} else {{
                c.close();
            }}
        }}
    }});
    const rdA = bigStream.getReader();
    const firstThree = [
        (await rdA.read()).value,
        (await rdA.read()).value,
        (await rdA.read()).value,
    ];
    rdA.releaseLock();
    const rdB = bigStream.getReader();
    let countAfterRelease = 0;
    let fourthVal = -1;
    while (true) {{
        const step = await rdB.read();
        if (step.done) break;
        if (countAfterRelease === 0) fourthVal = step.value;
        countAfterRelease++;
    }}
    rdB.releaseLock();

    let pullStep = 0;
    const asyncBinStream = new ReadableStream({{
        async pull(c) {{
            await Promise.resolve();
            if (pullStep === 0) {{
                pullStep++;
                c.enqueue(new Uint8Array([10, 128]));
            }} else if (pullStep === 1) {{
                pullStep++;
                c.enqueue(new Uint8Array([255, 20]));
            }} else {{
                c.close();
            }}
        }}
    }});
    const resStreamBytes = Array.from(await new Response(asyncBinStream).bytes());

    // D. Headers RFC 9110 tchar & CRLF validation +Response status [200, 599] RangeError + Response.error()
    let badHdrName = false;
    try {{ new Headers({{ "Bad Name": "1" }}); }} catch (e) {{ badHdrName = e instanceof TypeError; }}
    let badHdrVal = false;
    try {{ const h = new Headers(); h.set("x-ok", "v1\r\nv2"); }} catch (e) {{ badHdrVal = e instanceof TypeError; }}
    let badStatus101 = false;
    try {{ new Response("no", {{ status: 101 }}); }} catch (e) {{ badStatus101 = e instanceof RangeError; }}
    let badStatus999 = false;
    try {{ new Response("no", {{ status: 999 }}); }} catch (e) {{ badStatus999 = e instanceof RangeError; }}
    const errRes = Response.error();
    const errClone = errRes.clone();

    // E. FormData multipart quote/newline escaping + Request.formData() round-trip
    const fd = new FormData();
    fd.append('field"quote\nline', "val-1");
    fd.append("upload", new File(["hello-mp"], 'my"file.txt', {{ type: "text/plain" }}));
    const fdReq = new Request("https://app.io/submit", {{ method: "POST", body: fd }});
    const rawMp = await fdReq.clone().text();
    const parsedFromWire = await new Request("https://app.io/submit", {{
        method: "POST",
        headers: {{ "content-type": fdReq.headers.get("content-type") || "" }},
        body: rawMp,
    }}).formData();
    const uploaded = parsedFromWire.get("upload");

    console.log(JSON.stringify({{
        r1Status: r1.status,
        b1,
        b2,
        uRel: uRel.href,
        uHash: uHash.href,
        firstThree,
        fourthVal,
        countAfterRelease,
        resStreamBytes,
        badHdrName,
        badHdrVal,
        badStatus101,
        badStatus999,
        errStatus: errClone.status,
        hasEscapedName: rawMp.includes('name="field%22quote%0Aline"'),
        hasEscapedFile: rawMp.includes('filename="my%22file.txt"'),
        parsedField: parsedFromWire.get('field"quote\nline'),
        parsedFileName: uploaded && uploaded.name,
        parsedFileText: uploaded ? await uploaded.text() : "",
    }}));
}}
main().catch(e => console.log("ERR:" + e + " | " + (e && e.stack)));
"#,
        port = port
    );

    let m = run_source_isolated(
        "wintertc-step10-tenant",
        &js,
        caps,
        25_000_000,
        16 * 1024 * 1024,
    )
    .unwrap();
    crate::clear_outbound_conn_pool_for_tenant("wintertc-step10-tenant");
    server_handle.join().unwrap();

    assert!(
        !m.stdout.is_empty() && !m.stdout[0].starts_with("ERR:"),
        "wintertc-step10 JS error: stdout={:?}, exit={:?}, ret={}",
        m.stdout,
        m.exit_code,
        m.return_value
    );
    let v: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(v["r1Status"], 200);
    assert_eq!(v["b1"], serde_json::json!([0, 128, 255, 65, 254, 192]));
    assert_eq!(v["b2"], serde_json::json!([0, 129, 254, 255]));
    assert_eq!(
        v["uRel"],
        "https://app.io/callback?next=https://example.com:8443/a"
    );
    assert_eq!(v["uHash"], "https://app.io/dir/step#ref=http://other.test");
    assert_eq!(v["firstThree"], serde_json::json!([0, 1, 2]));
    assert_eq!(v["fourthVal"], 3);
    assert_eq!(v["countAfterRelease"], 9997);
    assert_eq!(v["resStreamBytes"], serde_json::json!([10, 128, 255, 20]));
    assert_eq!(v["badHdrName"], true);
    assert_eq!(v["badHdrVal"], true);
    assert_eq!(v["badStatus101"], true);
    assert_eq!(v["badStatus999"], true);
    assert_eq!(v["errStatus"], 0);
    assert_eq!(v["hasEscapedName"], true);
    assert_eq!(v["hasEscapedFile"], true);
    assert_eq!(v["parsedField"], "val-1");
    assert_eq!(v["parsedFileName"], "my\"file.txt");
    assert_eq!(v["parsedFileText"], "hello-mp");

    // 2. Verify live HTTP request `Set-Cookie` cloning (`new Headers(req.headers).getSetCookie()`)
    //    and `Slice.upgradeWebSocket` returning HTTP `101` without throwing RangeError.
    let ws_src = r#"
export default {
    fetch(req) {
        const url = new URL(req.url);
        if (url.pathname === "/cookies") {
            const cloned = new Headers(req.headers);
            return Response.json({ cookies: cloned.getSetCookie() });
        }
        return Slice.upgradeWebSocket({
            open(ws) { ws.send("upgraded-ok"); }
        });
    }
};
"#;
    let bundled_ws = slice_cas::strip_typescript_and_normalize(ws_src, true);
    let bc_ws = crate::compile_source_to_bytecode(&bundled_ws);
    let mut iso_http = SliceIsolate::spawn(
        "ws-and-live-headers",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        8 * 1024 * 1024,
    );
    let cookie_outcome = iso_http
        .execute_http_request(
            &bc_ws,
            None,
            &HttpRequestContext {
                method: "GET",
                path: "/cookies",
                query: "",
                tenant_id: "ws-and-live-headers",
                body: "",
                headers: &[
                    ("Host", "edge.local"),
                    ("Set-Cookie", "a=1; Path=/"),
                    ("Set-Cookie", "b=2; HttpOnly"),
                ],
            },
        )
        .unwrap();
    assert_eq!(cookie_outcome.status_code, 200);
    let cj: serde_json::Value = serde_json::from_str(&cookie_outcome.response_body).unwrap();
    assert_eq!(
        cj["cookies"],
        serde_json::json!(["a=1; Path=/", "b=2; HttpOnly"])
    );

    iso_http.reset_for_request("ws-and-live-headers", 2, 5_000_000, 8 * 1024 * 1024);
    let ws_outcome = iso_http
        .execute_http_request(
            &bc_ws,
            None,
            &HttpRequestContext {
                method: "GET",
                path: "/ws",
                query: "",
                tenant_id: "ws-and-live-headers",
                body: "",
                headers: &[
                    ("Host", "edge.local"),
                    ("Upgrade", "websocket"),
                    ("Connection", "Upgrade"),
                ],
            },
        )
        .unwrap();
    assert_eq!(ws_outcome.status_code, 101);
    assert!(ws_outcome.ws_upgraded);
}
