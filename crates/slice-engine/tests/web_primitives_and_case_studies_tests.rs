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

#[test]
fn test_coverage_plan_p0_ast_tier0_eligibility_and_nested_sandbox_scope_guard() {
    use crate::is_tier0_slicesnp_compatible_ast;

    // 1. Verify AST-based Tier-0 (`SLICESNP`) vs Tier-1 (`SLICEBOA`) routing:
    // Pure numeric / simple AST expressions are Tier-0 compatible:
    assert!(is_tier0_slicesnp_compatible_ast(
        "const x = 21 * 2; return x;"
    ));
    assert!(is_tier0_slicesnp_compatible_ast(
        "function fib(n) { if (n <= 1) return n; return fib(n - 1) + fib(n - 2); } console.log(fib(10));"
    ));

    // Expressions using ES2024 features not in Tier-0 (e.g., `.find`, `.sort`, `Object.values`,
    // `for..of`, destructuring, template interpolations, nested closures) must be rejected by
    // `is_tier0_slicesnp_compatible_ast` and compiled to `SLICEBOA`:
    for es2024_snippet in [
        "const found = [10, 20, 30].find((x) => x > 15); console.log(found);",
        "const sorted = [3, 1, 2].sort((a, b) => a - b); console.log(sorted.join(','));",
        "const vals = Object.values({ a: 1, b: 2 }); console.log(vals.length);",
        "let sum = 0; for (const x of [1, 2, 3, 4]) { sum += x; } console.log(sum);",
        "const { a, b } = { a: 10, b: 32 }; console.log(a + b);",
        "const name = 'Slice'; console.log(`Hello ${name}!`);",
        "function makeAdder(k) { return (x) => x + k; } console.log(makeAdder(5)(37));",
    ] {
        assert!(
            !is_tier0_slicesnp_compatible_ast(es2024_snippet),
            "Expected ES2024 snippet to route to SLICEBOA: {}",
            es2024_snippet
        );
        let bc = compile_source_to_bytecode(es2024_snippet);
        assert_eq!(
            &bc[0..8],
            crate::BOACAS_MAGIC,
            "Expected SLICEBC\\x01 header for: {}",
            es2024_snippet
        );
    }

    // 2. Verify `SavedIsolateContextGuard` preserves parent HTTP request, deterministic PRNG/clock,
    //    and CPU fuel budget across nested `Slice.evalSandbox` calls:
    let handler_js = r#"
const reqBefore = Slice.request();
const tBefore = Date.now();
const r1 = Slice.evalSandbox(`
    const items = [{ id: 1, score: 40 }, { id: 2, score: 95 }, { id: 3, score: 70 }];
    const best = items.filter(x => Boolean(x)).sort((a, b) => b.score - a.score).find(x => x.score > 50);
    JSON.stringify({ bestId: best.id, now: Date.now() })
`, { fuel: 15000, deterministicSeed: 999 });
const reqAfter = Slice.request();
const tAfter = Date.now();
let heavySum = 0;
for (let i = 0; i < 25000; i++) {
    heavySum += i;
}
Slice.respond(200, JSON.stringify({
    childOk: r1.ok,
    childRet: JSON.parse(r1.returnValue),
    pathPreserved: reqBefore.path === "/v1/nested-check" && reqAfter.path === "/v1/nested-check",
    tBefore,
    tAfter,
    heavySum
}), "application/json");
"#;
    let bc = compile_source_to_bytecode(handler_js);
    let mut isolate = SliceIsolate::spawn(
        "nested-guard-tenant",
        1,
        CapabilityToken::pure_compute(),
        50_000_000,
        16 * 1024 * 1024,
    );
    isolate.set_deterministic_seed(Some(42));
    let req_ctx = HttpRequestContext {
        method: "POST",
        path: "/v1/nested-check",
        query: "mode=strict",
        tenant_id: "nested-guard-tenant",
        headers: &[("X-Test", "1")],
        body: "{}",
    };
    let outcome = isolate.execute_http_request(&bc, None, &req_ctx).unwrap();
    assert_eq!(outcome.status_code, 200);
    let parsed: serde_json::Value = serde_json::from_str(&outcome.response_body).unwrap();
    assert_eq!(parsed["childOk"], true);
    assert_eq!(parsed["childRet"]["bestId"], 2);
    assert_eq!(parsed["pathPreserved"], true);
    assert_eq!(
        parsed["tBefore"].as_u64().unwrap() + 1,
        parsed["tAfter"].as_u64().unwrap()
    );
    assert_eq!(parsed["heavySum"], 312_487_500);
}

#[test]
fn test_audit_neo_phase2_single_pass_shard_automaton_and_precomputed_dlp() {
    use crate::js_preludes::detect_needed_shards;

    // 1. Lazy stringified module lines (`__slice_modules[0] = "..."`) must be ignored in O(1)
    //    without triggering false-positive shard activations:
    let bundle_with_lazy_wasm_str = concat!(
        "__slice_modules[0] = \"WebSocket WebAssembly CompressionStream subtle structuredClone\";\n",
        "const u = new URL('https://edge.slice.dev/v1?x=1');\n"
    );
    let mask = detect_needed_shards(bundle_with_lazy_wasm_str, false);
    assert_eq!(
        mask,
        1 << 1,
        "Only Shard 1 (URL) should be activated when other tokens are inside __slice_modules[0] = \"...\", got mask: {:#b}",
        mask
    );

    // 2. Overlapping patterns (`TextEncoderStream` -> Shards 2, 3, 14; `writeUInt` -> Shards 5, 17):
    let stream_and_bin_js = "const s = new TextEncoderStream(); buf.writeUInt32BE(1, 0);";
    let mask2 = detect_needed_shards(stream_and_bin_js, false);
    assert_ne!(mask2 & (1 << 2), 0);
    assert_ne!(mask2 & (1 << 3), 0);
    assert_ne!(mask2 & (1 << 14), 0);
    assert_ne!(mask2 & (1 << 5), 0);
    assert_ne!(mask2 & (1 << 17), 0);

    // 3. Pre-computed Base64 & Base64URL DLP secret blocking with zero per-call encoding allocations:
    let mut iso = SliceIsolate::spawn(
        "dlp-phase2-tenant",
        1,
        CapabilityToken::pure_compute(),
        500_000,
        4 * 1024 * 1024,
    );
    iso.add_egress_audit_secret("sk_live_9988776655_secret");
    // Must have pre-expanded raw + b64_std + b64_url (>= 2 entries) at registration time
    assert!(iso.egress_audit_secrets.len() >= 2);
    let b64_leak_code = r#"
        const encoded = btoa("sk_live_9988776655_secret");
        console.log("leaking:", encoded);
    "#;
    let bc = compile_source_to_bytecode(b64_leak_code);
    let err = iso.execute_bytecode(&bc, None).unwrap_err();
    assert!(err.to_string().contains("EGRESS_SECRET_LEAK_BLOCKED"));
}

#[test]
fn test_audit_neo_phase3_zero_alloc_kv_counters_and_cow_vfs_checkpoints() {
    let js = r#"
        const fs = require("node:fs");
        fs.writeFileSync("/tmp/cow_phase3.txt", "alpha");
        Slice.kv.set("counter", "10");
        const c1 = Slice.kv.incr("counter", 5);
        const c2 = Slice.kv.incr("counter", -3);
        const snap = Slice.checkpoint();

        // Mutate both VFS (via CoW appendFileSync) and KV counter after checkpoint
        fs.appendFileSync("/tmp/cow_phase3.txt", "-beta");
        const c3 = Slice.kv.incr("counter", 100);
        const mutatedVfs = fs.readFileSync("/tmp/cow_phase3.txt", "utf8");
        const mutatedKv = Slice.kv.get("counter");

        // Restore checkpoint: CoW Arc buffer in snapshot must remain untouched ("alpha")
        Slice.restore(snap);
        const restoredVfs = fs.readFileSync("/tmp/cow_phase3.txt", "utf8");
        const restoredKv = Slice.kv.get("counter");

        console.log(JSON.stringify({
            c1, c2, c3,
            mutatedVfs, mutatedKv,
            restoredVfs, restoredKv
        }));
    "#;
    let m = run_source_isolated(
        "phase3-cow-kv-tenant",
        js,
        CapabilityToken::pure_compute()
            .with_read_prefix("/tmp")
            .with_write_prefix("/tmp"),
        1_000_000,
        8 * 1024 * 1024,
    )
    .unwrap();
    let res: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(res["c1"], 15);
    assert_eq!(res["c2"], 12);
    assert_eq!(res["c3"], 112);
    assert_eq!(res["mutatedVfs"], "alpha-beta");
    assert_eq!(res["mutatedKv"], "112");
    assert_eq!(res["restoredVfs"], "alpha");
    assert_eq!(res["restoredKv"], "12");
}
