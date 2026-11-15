#![allow(unused_imports)]
use crate::intrinsics::{
    Sha256State, digest_to_hex, fast_format_bytes, fast_format_ms, fast_parse_bytes, fast_parse_ms,
    hmac_sha256_hex, sha256_hex,
};
use crate::{
    ExecutionTier, HttpRequestContext, JsValue, SliceIsolate, compile_source_to_bytecode,
    disassemble_bytecode, run_source_isolated,
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
        body_bytes: None,
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

#[test]
fn test_sveltekit_whatwg_opaque_url_signal_and_async_stream_response() {
    use crate::intrinsics::{can_parse_whatwg_url, parse_whatwg_url};

    // 1. Native Rust WHATWG URL non-special opaque-path URLs (used by SvelteKit `$app/state` `new URL("a:")`)
    let u_a = parse_whatwg_url("a:", None).expect("new URL('a:') must parse");
    assert_eq!(u_a.protocol, "a:");
    assert_eq!(u_a.pathname, "");
    assert_eq!(u_a.origin, "null");
    assert_eq!(u_a.href, "a:");
    assert!(can_parse_whatwg_url("a:", None));
    assert!(can_parse_whatwg_url("about:blank", None));
    assert!(can_parse_whatwg_url(
        "data:text/plain;base64,SGVsbG8=",
        None
    ));
    assert!(!can_parse_whatwg_url("http:foo", None));

    // 2. JS `new URL("a:")`, `Request.prototype.signal`, and async `ReadableStream` `Response` body
    let js = r#"
        (async () => {
            const u1 = new URL("a:");
            const u2 = new URL("about:blank?x=1#top");
            const req = new Request("https://edge.slice.local/items");
            let abortListenerOk = false;
            req.signal.addEventListener("abort", () => {});
            abortListenerOk = (req.signal.aborted === false);

            const stream = new ReadableStream({
                async start(controller) {
                    await Promise.resolve();
                    controller.enqueue("chunk-1:");
                    await Promise.resolve();
                    controller.enqueue("chunk-2");
                    controller.close();
                }
            });
            const res = new Response(stream, { headers: { "x-test": "1" } });
            res.headers.set("content-type", "text/sveltekit-data");
            res.headers.append("set-cookie", "a=1; Path=/");
            res.headers.append("set-cookie", "b=2; Path=/");
            const text = await res.text();
            console.log(JSON.stringify({
                u1Href: u1.href,
                u1Proto: u1.protocol,
                u1Origin: u1.origin,
                u2Path: u2.pathname,
                u2Search: u2.search,
                canParseA: URL.canParse("a:"),
                abortListenerOk,
                text,
                cookies: res.headers.getSetCookie()
            }));
        })();
    "#;
    let m = run_source_isolated(
        "sveltekit-primitives-tenant",
        js,
        CapabilityToken::pure_compute(),
        1_000_000,
        8 * 1024 * 1024,
    )
    .unwrap();
    let res: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(res["u1Href"], "a:");
    assert_eq!(res["u1Proto"], "a:");
    assert_eq!(res["u1Origin"], "null");
    assert_eq!(res["u2Path"], "blank");
    assert_eq!(res["u2Search"], "?x=1");
    assert_eq!(res["canParseA"], true);
    assert_eq!(res["abortListenerOk"], true);
    assert_eq!(res["text"], "chunk-1:chunk-2");
    assert_eq!(
        res["cookies"],
        serde_json::json!(["a=1; Path=/", "b=2; Path=/"])
    );
}

#[test]
fn test_wave4_issue_114_structured_clone_shared_arraybuffer_offsets_transfer_and_datacloneerror() {
    let js = r#"
(async () => {
    // 1. Exact #114 repro + multi-view shared ArrayBuffer with nonzero byteOffsets
    const ab = new ArrayBuffer(16);
    const full = new Uint8Array(ab);
    for (let i = 0; i < 16; i++) full[i] = i + 10;
    const left = new Uint8Array(ab, 0, 8);
    const right = new Uint8Array(ab, 4, 8);
    const u16Mid = new Uint16Array(ab, 2, 4);
    const dvTail = new DataView(ab, 8, 8);

    // Case A: views without direct buffer property (and reverse order)
    const cViewsOnly = structuredClone({ right, left, u16Mid, dvTail });
    const sharedBufA =
        cViewsOnly.left.buffer === cViewsOnly.right.buffer &&
        cViewsOnly.left.buffer === cViewsOnly.u16Mid.buffer &&
        cViewsOnly.left.buffer === cViewsOnly.dvTail.buffer &&
        cViewsOnly.left.buffer !== ab &&
        cViewsOnly.left.buffer.byteLength === 16;
    const offsetsPreservedA =
        cViewsOnly.left.byteOffset === 0 &&
        cViewsOnly.left.byteLength === 8 &&
        cViewsOnly.right.byteOffset === 4 &&
        cViewsOnly.right.byteLength === 8 &&
        cViewsOnly.u16Mid.byteOffset === 2 &&
        cViewsOnly.u16Mid.length === 4 &&
        cViewsOnly.dvTail.byteOffset === 8 &&
        cViewsOnly.dvTail.byteLength === 8;

    // Mutate overlap in clone and verify sibling view observes it while original does not
    cViewsOnly.left[4] = 0xee;
    const overlapSyncA = cViewsOnly.right[0] === 0xee && full[4] === 14;

    // Case B: direct buffer property before and after views
    const cWithBuf = structuredClone({ buffer: ab, right, left });
    const sharedBufB =
        cWithBuf.buffer === cWithBuf.left.buffer &&
        cWithBuf.buffer === cWithBuf.right.buffer &&
        cWithBuf.right.byteOffset === 4;

    // 2. Transfer list detachment + atomic rollback on DataCloneError
    const abTransfer = new ArrayBuffer(12);
    const viewTransfer = new Uint8Array(abTransfer, 4, 4);
    viewTransfer.set([91, 92, 93, 94]);
    const transferred = structuredClone({ v: viewTransfer }, { transfer: [abTransfer] });
    const transferOk =
        abTransfer.detached === true &&
        abTransfer.byteLength === 0 &&
        transferred.v.byteOffset === 4 &&
        transferred.v.buffer.byteLength === 12 &&
        Array.from(transferred.v).join(",") === "91,92,93,94";

    let dupTransferErr = "";
    const abDup = new ArrayBuffer(4);
    try {
        structuredClone(abDup, { transfer: [abDup, abDup] });
    } catch (e) {
        dupTransferErr = e && e.name;
    }

    let detachedTransferErr = "";
    try {
        structuredClone({ ok: 1 }, { transfer: [abTransfer] });
    } catch (e) {
        detachedTransferErr = e && e.name;
    }

    // Atomic rollback: if graph serialization throws DataCloneError, transfer list must NOT detach!
    const abAtomic = new ArrayBuffer(8);
    let atomicErr = "";
    try {
        structuredClone({ buf: abAtomic, bad: () => 1 }, { transfer: [abAtomic] });
    } catch (e) {
        atomicErr = e && e.name;
    }
    const atomicIntact = abAtomic.detached === false && abAtomic.byteLength === 8;

    // 3. Cyclic Map/Set/Object/Error.cause + diamond Date/RegExp/File identity
    const d = new Date(1700000000000);
    const rx = /slice-[0-9]+/gi;
    rx.lastIndex = 9;
    const binFile = new File([new Uint8Array([0, 128, 255])], "raw.bin", { type: "application/octet-stream" });
    const cycMap = new Map();
    cycMap.set("self", cycMap);
    const cycSet = new Set();
    cycSet.add(cycSet);
    const rootErr = new TypeError("outer", { cause: new RangeError("inner") });
    const graph = { d1: d, d2: d, r1: rx, r2: rx, f1: binFile, f2: binFile, cycMap, cycSet, rootErr };
    graph.self = graph;
    const cg = structuredClone(graph);
    const cgBytes = Array.from(await cg.f1.bytes());
    const diamondAndCyclesOk =
        cg.self === cg &&
        cg.d1 === cg.d2 &&
        cg.d1 !== d &&
        cg.d1.getTime() === 1700000000000 &&
        cg.r1 === cg.r2 &&
        cg.r1.lastIndex === 0 &&
        cg.f1 === cg.f2 &&
        cg.f1 !== binFile &&
        cgBytes.join(",") === "0,128,255" &&
        cg.cycMap.get("self") === cg.cycMap &&
        cg.cycSet.has(cg.cycSet) &&
        cg.rootErr instanceof TypeError &&
        cg.rootErr.cause instanceof RangeError &&
        cg.rootErr.cause.message === "inner";

    // 4. DataCloneError on non-cloneable platform types
    const uncloneables = [
        Promise.resolve(1),
        new WeakMap(),
        new WeakSet(),
        function namedFn() {},
        Symbol("sym"),
        new EventTarget(),
    ];
    const allRejected = uncloneables.every((item) => {
        try {
            structuredClone(item);
            return false;
        } catch (e) {
            return e && e.name === "DataCloneError";
        }
    });

    // 5. MessageChannel postMessage uses structuredClone preserving shared buffer offsets
    const mc = new MessageChannel();
    const msgProm = new Promise((resolve) => {
        mc.port2.addEventListener("message", (ev) => {
            resolve(
                ev.data.a.buffer === ev.data.b.buffer &&
                ev.data.b.byteOffset === 4 &&
                ev.data.b[0] === 14
            );
        });
    });
    mc.port1.postMessage({ a: left, b: right });
    const portCloneOk = await msgProm;

    console.log(JSON.stringify({
        sharedBufA,
        offsetsPreservedA,
        overlapSyncA,
        sharedBufB,
        transferOk,
        dupTransferErr,
        detachedTransferErr,
        atomicErr,
        atomicIntact,
        diamondAndCyclesOk,
        allRejected,
        portCloneOk,
    }));
})();
"#;
    let m = run_source_isolated(
        "wave4-issue-114-tenant",
        js,
        CapabilityToken::pure_compute(),
        5_000_000,
        16 * 1024 * 1024,
    )
    .unwrap();
    let res: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(res["sharedBufA"], true, "res: {}", res);
    assert_eq!(res["offsetsPreservedA"], true, "res: {}", res);
    assert_eq!(res["overlapSyncA"], true, "res: {}", res);
    assert_eq!(res["sharedBufB"], true, "res: {}", res);
    assert_eq!(res["transferOk"], true, "res: {}", res);
    assert_eq!(res["dupTransferErr"], "DataCloneError", "res: {}", res);
    assert_eq!(res["detachedTransferErr"], "DataCloneError", "res: {}", res);
    assert_eq!(res["atomicErr"], "DataCloneError", "res: {}", res);
    assert_eq!(res["atomicIntact"], true, "res: {}", res);
    assert_eq!(res["diamondAndCyclesOk"], true, "res: {}", res);
    assert_eq!(res["allRejected"], true, "res: {}", res);
    assert_eq!(res["portCloneOk"], true, "res: {}", res);
}

#[test]
fn test_wave4_issues_115_116_121_response_json_binary_formdata_and_content_type_authority() {
    let js = r#"
(async () => {
    // 1. Issue #115: Eager Response.json serialization, mutation isolation, and sync TypeError
    let toJsonCalls = 0;
    const payload = {
        status: "initial",
        nested: { count: 1 },
        custom: {
            toJSON() {
                toJsonCalls++;
                return "snap";
            }
        }
    };
    const rJsonFast = Response.json(payload);
    const rJsonInit = Response.json(payload, {
        status: 201,
        headers: { "x-trace": "t1" }
    });
    const toJsonAfterConstruct = toJsonCalls;
    payload.status = "MUTATED";
    payload.nested.count = 999;

    const p1 = await rJsonFast.clone().json();
    p1.nested.count = 777;
    const p2 = await rJsonFast.json();
    const txtInit = await rJsonInit.text();

    const unserializable = [undefined, Symbol("s"), () => {}, 42n];
    const cyc = {};
    cyc.self = cyc;
    unserializable.push(cyc);
    const syncJsonErrors = unserializable.every((bad) => {
        try {
            Response.json(bad);
            return false;
        } catch (e) {
            return e instanceof TypeError;
        }
    });

    // 2. Issue #116: Lossless binary FormData round-trip (NUL, invalid UTF-8 0x80/0xff, embedded CRLF)
    const rawBin = new Uint8Array([0x00, 0x80, 0xff, 0x0d, 0x0a, 0x41, 0xfe, 0x00]);
    const fd = new FormData();
    fd.append("meta", "alpha\r\nbeta");
    fd.append("blobFile", new File([rawBin], 'semi;quote"name.bin', { type: "application/x-raw" }));
    fd.append("emptyFile", new File([], "empty.dat", { type: "application/octet-stream" }));

    const reqFd = new Request("https://edge.slice.dev/upload", { method: "POST", body: fd });
    // Post-construction mutation of FormData must NOT mutate already-constructed Request (#115/#116)
    fd.set("meta", "MUTATED_AFTER");

    const parsedReqFd = await reqFd.clone().formData();
    const gotFile = parsedReqFd.get("blobFile");
    const gotBytes = Array.from(await gotFile.bytes());
    const gotEmpty = parsedReqFd.get("emptyFile");

    const resFd = new Response(parsedReqFd);
    const parsedResFd = await resFd.formData();
    const gotResBytes = Array.from(await parsedResFd.get("blobFile").bytes());

    // Malformed multipart missing closing boundary must reject with TypeError
    let truncatedRejected = false;
    try {
        await new Response(
            '--B\r\nContent-Disposition: form-data; name="k"\r\n\r\nval\r\n',
            { headers: { "content-type": "multipart/form-data; boundary=B" } }
        ).formData();
    } catch (e) {
        truncatedRejected = e instanceof TypeError;
    }

    // 3. Issue #121: Explicit Content-Type headers never shadowed by string default _ct
    const mpStr =
        '--X\r\nContent-Disposition: form-data; name="greeting"\r\n\r\nhello-world\r\n' +
        '--X\r\nContent-Disposition: form-data; name="doc"; filename="note.txt"\r\n\r\nplain-body\r\n' +
        '--X--\r\n';
    const mpFromStrRes = await new Response(mpStr, {
        headers: { "Content-Type": "multipart/form-data; boundary=X" }
    }).formData();
    const noteFile = mpFromStrRes.get("doc");

    const customBlob = await new Response("hello", {
        headers: { "Content-Type": "application/octet-stream" }
    }).blob();
    const emptyRes = new Response();
    const u8Res = new Response(new Uint8Array([1, 2, 3]));

    const mutHdrRes = new Response("hello");
    mutHdrRes.headers.set("content-type", "text/x-custom");
    const mutSetBlob = await mutHdrRes.clone().blob();
    const delHdrRes = new Response("hello");
    delHdrRes.headers.delete("content-type");
    const delBlob = await delHdrRes.blob();

    // URL-encoded body starting with leading '?' preserves '?first' key
    const urlEncFd = await new Request("https://edge.slice.dev/form", {
        method: "POST",
        headers: { "content-type": "application/x-www-form-urlencoded" },
        body: "?first=1&second=two"
    }).formData();

    console.log(JSON.stringify({
        toJsonAfterConstruct,
        toJsonTotal: toJsonCalls,
        p2Status: p2.status,
        p2Count: p2.nested.count,
        txtInit,
        syncJsonErrors,
        metaVal: parsedReqFd.get("meta"),
        fileName: gotFile.name,
        fileType: gotFile.type,
        bytesMatch: gotBytes.join(",") === Array.from(rawBin).join(","),
        resBytesMatch: gotResBytes.join(",") === Array.from(rawBin).join(","),
        emptySize: gotEmpty.size,
        truncatedRejected,
        mpGreeting: mpFromStrRes.get("greeting"),
        noteDefaultType: noteFile.type,
        noteText: await noteFile.text(),
        customBlobType: customBlob.type,
        emptyCt: emptyRes.headers.get("content-type"),
        emptyBlobType: (await emptyRes.blob()).type,
        u8Ct: u8Res.headers.get("content-type"),
        u8BlobType: (await u8Res.blob()).type,
        mutSetType: mutSetBlob.type,
        delBlobType: delBlob.type,
        urlEncFirst: urlEncFd.get("?first"),
        urlEncSecond: urlEncFd.get("second"),
    }));
})();
"#;
    let m = run_source_isolated(
        "wave4-115-116-121-tenant",
        js,
        CapabilityToken::pure_compute(),
        5_000_000,
        16 * 1024 * 1024,
    )
    .unwrap();
    let res: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(res["toJsonAfterConstruct"], 2, "res: {}", res);
    assert_eq!(res["toJsonTotal"], 2, "res: {}", res);
    assert_eq!(res["p2Status"], "initial", "res: {}", res);
    assert_eq!(res["p2Count"], 1, "res: {}", res);
    assert_eq!(
        res["txtInit"],
        r#"{"status":"initial","nested":{"count":1},"custom":"snap"}"#
    );
    assert_eq!(res["syncJsonErrors"], true, "res: {}", res);
    assert_eq!(res["metaVal"], "alpha\r\nbeta", "res: {}", res);
    assert_eq!(res["fileName"], "semi;quote\"name.bin", "res: {}", res);
    assert_eq!(res["fileType"], "application/x-raw", "res: {}", res);
    assert_eq!(res["bytesMatch"], true, "res: {}", res);
    assert_eq!(res["resBytesMatch"], true, "res: {}", res);
    assert_eq!(res["emptySize"], 0, "res: {}", res);
    assert_eq!(res["truncatedRejected"], true, "res: {}", res);
    assert_eq!(res["mpGreeting"], "hello-world", "res: {}", res);
    assert_eq!(res["noteDefaultType"], "text/plain", "res: {}", res);
    assert_eq!(res["noteText"], "plain-body", "res: {}", res);
    assert_eq!(res["customBlobType"], "application/octet-stream");
    assert!(res["emptyCt"].is_null(), "res: {}", res);
    assert_eq!(res["emptyBlobType"], "");
    assert!(res["u8Ct"].is_null(), "res: {}", res);
    assert_eq!(res["u8BlobType"], "");
    assert_eq!(res["mutSetType"], "text/x-custom");
    assert_eq!(res["delBlobType"], "");
    assert_eq!(res["urlEncFirst"], "1");
    assert_eq!(res["urlEncSecond"], "two");

    // Also verify live HttpRequestContext binary `body_bytes` ingress (#116)
    let live_handler_ts = r#"
export default {
    async fetch(req: Request): Promise<Response> {
        const cloned = req.clone();
        const fd = await req.formData();
        const f = fd.get("bin") as File;
        const u8 = await f.bytes();
        const rawClone = await cloned.bytes();
        return Response.json({
            tag: fd.get("tag"),
            name: f.name,
            bytes: Array.from(u8),
            rawLen: rawClone.byteLength,
        });
    }
};
"#;
    let bc = compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(
        live_handler_ts,
        true,
    ));
    let mut iso = SliceIsolate::spawn(
        "wave4-live-bin-req",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        16 * 1024 * 1024,
    );
    let mut wire_body = Vec::new();
    wire_body.extend_from_slice(
        b"--LiveBound\r\nContent-Disposition: form-data; name=\"tag\"\r\n\r\nok\r\n",
    );
    wire_body.extend_from_slice(
        b"--LiveBound\r\nContent-Disposition: form-data; name=\"bin\"; filename=\"p.bin\"\r\nContent-Type: application/octet-stream\r\n\r\n",
    );
    wire_body.extend_from_slice(&[0x00, 0x80, 0xff, 0x0d, 0x0a, 0x7f]);
    wire_body.extend_from_slice(b"\r\n--LiveBound--\r\n");

    let req_ctx = HttpRequestContext {
        method: "POST",
        path: "/upload",
        query: "",
        tenant_id: "wave4-live-bin-req",
        body: "",
        body_bytes: Some(&wire_body),
        headers: &[
            ("host", "edge.slice.local"),
            ("content-type", "multipart/form-data; boundary=LiveBound"),
        ],
    };
    let out = iso.execute_http_request(&bc, None, &req_ctx).unwrap();
    assert_eq!(out.status_code, 200);
    let parsed_live: serde_json::Value = serde_json::from_str(&out.response_body).unwrap();
    assert_eq!(parsed_live["tag"], "ok");
    assert_eq!(parsed_live["name"], "p.bin");
    assert_eq!(
        parsed_live["bytes"],
        serde_json::json!([0, 128, 255, 13, 10, 127])
    );
    assert_eq!(parsed_live["rawLen"], wire_body.len());
}

#[test]
fn test_wave4_issues_117_119_120_async_readablestream_buffer_from_and_event_target() {
    let js = r#"
import { EventEmitter } from "node:events";

(async () => {
    // 1. Issue #117: Async pull() resolving before timer enqueue + reader.closed + tee + pipeThrough
    let closedResolved = false;
    let step = 0;
    const timerStream = new ReadableStream({
        async start(c) {
            await Promise.resolve();
            c.enqueue("from-start");
        },
        async pull(c) {
            step++;
            if (step === 1) {
                setTimeout(() => {
                    c.enqueue("delayed-1");
                }, 2);
                return; // resolves BEFORE enqueue!
            }
            if (step === 2) {
                setTimeout(() => {
                    c.enqueue("delayed-2");
                    c.close();
                }, 2);
            }
        }
    });
    const reader = timerStream.getReader();
    reader.closed.then(() => { closedResolved = true; });
    const r0 = await reader.read();
    const closedMidway = closedResolved;
    const [r1, r2, r3] = await Promise.all([reader.read(), reader.read(), reader.read()]);
    await reader.closed;

    // Async tee() + pipeThrough(TransformStream)
    let teeTick = 0;
    const baseAsync = new ReadableStream({
        pull(c) {
            return new Promise((res) => {
                setTimeout(() => {
                    teeTick++;
                    if (teeTick <= 2) c.enqueue("chunk-" + teeTick);
                    else c.close();
                    res();
                }, 1);
            });
        }
    });
    const [b1, b2] = baseAsync.tee();
    const upperB1 = b1.pipeThrough(new TransformStream({
        transform(ch, ctrl) {
            ctrl.enqueue(String(ch).toUpperCase());
        }
    }));
    const out1 = [];
    for await (const x of upperB1) out1.push(x);
    const out2 = [];
    for await (const y of b2) out2.push(y);

    // Async pull() rejection transitions stream & reader.closed to errored
    const errStream = new ReadableStream({
        async pull() {
            throw new Error("boom-pull");
        }
    });
    const errReader = errStream.getReader();
    let readErrMsg = "";
    let closedErrMsg = "";
    try { await errReader.read(); } catch (e) { readErrMsg = e.message; }
    try { await errReader.closed; } catch (e) { closedErrMsg = e.message; }

    // 2. Issue #119: Buffer.from TypedArray element conversion, ArrayBuffer bounds, and unboxing
    const u16 = new Uint16Array([0x1234, 0x5678, 0x00ff]);
    const bufElem = Buffer.from(u16);
    const bufSub = Buffer.from(u16.subarray(1));
    const bufInt32 = Buffer.from(new Int32Array([-1, 258]));
    const bufFloat = Buffer.from(new Float64Array([65.9, 66.1]));
    const bufView = Buffer.from(u16.buffer, 2, 4);
    u16[1] = 0x1122;
    const zeroCopyShared = bufView[0] === 0x22 && bufView[1] === 0x11;

    let oobNegErr = "";
    try { Buffer.from(new ArrayBuffer(4), -1); } catch (e) { oobNegErr = e.name; }
    let oobLenErr = "";
    try { Buffer.from(new ArrayBuffer(4), 2, 4); } catch (e) { oobLenErr = e.name; }
    let dvErr = "";
    try { Buffer.from(new DataView(new ArrayBuffer(4))); } catch (e) { dvErr = e.name; }
    let bigErr = "";
    try { Buffer.from(new BigUint64Array([1n])); } catch (e) { bigErr = e.name; }

    const unboxedStr = Buffer.from(new String("abc")).toString("utf8");
    const unboxedPrim = Buffer.from({ [Symbol.toPrimitive]() { return "xyz"; } }).toString("utf8");
    const unboxedValOf = Array.from(Buffer.from({ valueOf() { return new Uint8Array([7, 8]); } }));

    // 3. Issue #120: WHATWG EventTarget removed-listener semantics, AbortSignal, & MessagePort
    const et = new EventTarget();
    const log = [];
    function listenerB() { log.push("B"); }
    et.addEventListener("go", () => {
        log.push("A");
        et.removeEventListener("go", listenerB);
        et.addEventListener("go", listenerB); // re-add must NOT fire in current dispatch!
    });
    et.addEventListener("go", listenerB);
    et.dispatchEvent(new Event("go"));
    const afterFirstDispatch = log.join(",");
    et.dispatchEvent(new Event("go"));
    const afterSecondDispatch = log.join(",");

    // Capture 3-tuple independence: removing (fn, false) leaves (fn, true) intact
    const capLog = [];
    function capFn() { capLog.push("C"); }
    et.addEventListener("cap", capFn, true);
    et.addEventListener("cap", capFn, false);
    et.removeEventListener("cap", capFn, false);
    et.dispatchEvent(new Event("cap"));

    // AbortSignal instanceof EventTarget + sibling removal during abort
    const ac = new AbortController();
    const sigIsEt = ac.signal instanceof EventTarget;
    const abortLog = [];
    function secondAbort() { abortLog.push("second"); }
    ac.signal.addEventListener("abort", () => {
        abortLog.push("first");
        ac.signal.removeEventListener("abort", secondAbort);
    });
    ac.signal.addEventListener("abort", secondAbort);
    ac.abort();

    // MessagePort onmessage + addEventListener("message") fire once each
    const ch = new MessageChannel();
    const portHits = [];
    ch.port1.onmessage = (e) => portHits.push("prop:" + e.data);
    ch.port1.addEventListener("message", (e) => portHits.push("et:" + e.data));
    ch.port2.postMessage("hi");
    await new Promise((r) => setTimeout(r, 5));

    // Control: Node EventEmitter preserves snapshot-at-emit behavior
    const ee = new EventEmitter();
    const eeHits = [];
    function eeSecond() { eeHits.push("ee2"); }
    ee.on("ev", () => {
        eeHits.push("ee1");
        ee.off("ev", eeSecond);
    });
    ee.on("ev", eeSecond);
    ee.emit("ev");

    console.log(JSON.stringify({
        r0,
        closedMidway,
        r1,
        r2,
        r3,
        closedResolved,
        out1,
        out2,
        readErrMsg,
        closedErrMsg,
        bufElem: Array.from(bufElem),
        bufSub: Array.from(bufSub),
        bufInt32: Array.from(bufInt32),
        bufFloat: Array.from(bufFloat),
        zeroCopyShared,
        oobNegErr,
        oobLenErr,
        dvErr,
        bigErr,
        unboxedStr,
        unboxedPrim,
        unboxedValOf,
        afterFirstDispatch,
        afterSecondDispatch,
        capCount: capLog.length,
        sigIsEt,
        abortLog,
        portHits,
        eeHits,
    }));
})();
"#;
    let m = run_source_isolated(
        "wave4-117-119-120-tenant",
        js,
        CapabilityToken::pure_compute(),
        5_000_000,
        16 * 1024 * 1024,
    )
    .unwrap();
    let res: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(
        res["r0"],
        serde_json::json!({"value": "from-start", "done": false})
    );
    assert_eq!(res["closedMidway"], false);
    assert_eq!(
        res["r1"],
        serde_json::json!({"value": "delayed-1", "done": false})
    );
    assert_eq!(
        res["r2"],
        serde_json::json!({"value": "delayed-2", "done": false})
    );
    assert_eq!(res["r3"]["done"], true);
    assert_eq!(res["closedResolved"], true);
    assert_eq!(res["out1"], serde_json::json!(["CHUNK-1", "CHUNK-2"]));
    assert_eq!(res["out2"], serde_json::json!(["chunk-1", "chunk-2"]));
    assert_eq!(res["readErrMsg"], "boom-pull");
    assert_eq!(res["closedErrMsg"], "boom-pull");
    assert_eq!(res["bufElem"], serde_json::json!([0x34, 0x78, 0xff]));
    assert_eq!(res["bufSub"], serde_json::json!([0x78, 0xff]));
    assert_eq!(res["bufInt32"], serde_json::json!([255, 2]));
    assert_eq!(res["bufFloat"], serde_json::json!([65, 66]));
    assert_eq!(res["zeroCopyShared"], true);
    assert_eq!(res["oobNegErr"], "RangeError");
    assert_eq!(res["oobLenErr"], "RangeError");
    assert_eq!(res["dvErr"], "TypeError");
    assert_eq!(res["bigErr"], "TypeError");
    assert_eq!(res["unboxedStr"], "abc");
    assert_eq!(res["unboxedPrim"], "xyz");
    assert_eq!(res["unboxedValOf"], serde_json::json!([7, 8]));
    assert_eq!(res["afterFirstDispatch"], "A");
    assert_eq!(res["afterSecondDispatch"], "A,A");
    assert_eq!(res["capCount"], 1);
    assert_eq!(res["sigIsEt"], true);
    assert_eq!(res["abortLog"], serde_json::json!(["first"]));
    assert_eq!(res["portHits"], serde_json::json!(["prop:hi", "et:hi"]));
    assert_eq!(res["eeHits"], serde_json::json!(["ee1", "ee2"]));
}
