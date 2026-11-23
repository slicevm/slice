//! Generalized Wave 3 (`#164`) Verification Matrix:
//! - `UniqueShape::insert_property_transition` monotonic shape-ID bump (`AddTransitionEntry`,
//!   1-hop `PROTOTYPE` IC, and `FastForInState` prototype shape invalidation)
//! - `FastForInState` non-enumerable own-key shadowing (initial and mid-loop deoptimization)
//! - `Script::from_slicebc_bytes` (`SLCB\x01`) constant-table & handler bounds validation
//! - Settled `Await` `AsyncLocalStorage` (`als.enterWith`) continuation isolation
//! - `node:crypto` lazy `slice:crypto-pki` Buffer host bindings without Shard 5 preloaded
//! - WHATWG `URL` unified component setters, `[SameObject]` `searchParams`, & read-only `origin`
//! - `Response` / `Response.json` iterable (`Map` / `Symbol.iterator`) headers, `Slice.evalSandbox`
//!   `NaN` options, `__slice_fast_ms` `>= 3.89e23` day formatting, and closed-stream `Response.text()`
//!   / `Response.json()` / `Response.bytes()` / `Response.arrayBuffer()` single-pass fast paths

use crate::{
    SliceIsolate, compile_source_to_bytecode, run_source_isolated, set_thread_exper_boacas_override,
};
use slice_cas::{CasBuilder, MappedCasArchive, bundle_entrypoint_with_deps};
use slice_sandbox::CapabilityToken;
use std::path::PathBuf;

struct BoacasThreadOverrideGuard;
impl Drop for BoacasThreadOverrideGuard {
    fn drop(&mut self) {
        set_thread_exper_boacas_override(None);
    }
}

#[test]
fn test_wave3_unique_shape_ic_invalidation_and_fast_for_in_non_enum_shadowing() {
    let src = r#"
(function() {
  "use strict";

  // Build a UniqueShape prototype (> 1030 own properties forces dictionary / UniqueShape mode)
  const dictProto = {};
  for (let i = 0; i < 1040; i++) {
    dictProto["seed_" + i] = i;
  }

  // 1. AddTransitionEntry invalidation when a setter is inserted onto a UniqueShape prototype
  let setterIntercepted = [];
  function attachTag(o, v) {
    o.dynTag = v;
  }
  for (let i = 0; i < 12; i++) {
    const item = Object.create(dictProto);
    attachTag(item, i);
    if (item.dynTag !== i) throw new Error("pre-setter mismatch: " + item.dynTag);
  }
  Object.defineProperty(dictProto, "dynTag", {
    set(v) { setterIntercepted.push(v * 10); },
    get() { return -1; },
    configurable: true
  });
  const afterSetter = Object.create(dictProto);
  attachTag(afterSetter, 7);
  if (setterIntercepted.length !== 1 || setterIntercepted[0] !== 70 || afterSetter.dynTag !== -1) {
    throw new Error("UniqueShape setter did not invalidate AddTransitionEntry: " + JSON.stringify(setterIntercepted));
  }

  // 2. AddTransitionEntry invalidation when a non-writable data property is inserted onto UniqueShape
  function attachLocked(o, v) {
    o.lockedSlot = v;
  }
  for (let i = 0; i < 8; i++) {
    const item = Object.create(dictProto);
    attachLocked(item, i);
  }
  Object.defineProperty(dictProto, "lockedSlot", {
    value: 999,
    writable: false,
    configurable: true
  });
  let nonWritableThrew = false;
  try {
    attachLocked(Object.create(dictProto), 1);
  } catch (e) {
    nonWritableThrew = e instanceof TypeError;
  }
  if (!nonWritableThrew) {
    throw new Error("expected TypeError writing to inherited non-writable UniqueShape property");
  }

  // 3. 1-hop PROTOTYPE GetPropertyByName IC invalidation when UniqueShape receiver shadows slot
  const protoBase = { inheritedVal: 11 };
  const uniqueChild = Object.create(protoBase);
  for (let i = 0; i < 1040; i++) {
    uniqueChild["pad_" + i] = i;
  }
  function readInherited(x) {
    return x.inheritedVal;
  }
  for (let i = 0; i < 10; i++) {
    if (readInherited(uniqueChild) !== 11) throw new Error("proto IC warm mismatch");
  }
  uniqueChild.inheritedVal = 42;
  if (readInherited(uniqueChild) !== 42) {
    throw new Error("UniqueShape insert failed to invalidate 1-hop PROTOTYPE GetPropertyByName IC");
  }

  // 4. FastForInState non-enumerable own-key shadowing (static non-enumerable own key + mid-loop proto mutation)
  const p1 = {};
  const t1 = Object.create(p1);
  Object.defineProperty(t1, "hidden", { value: 1, enumerable: false, configurable: true });
  t1.a = 2;
  t1.b = 3;
  const seen1 = [];
  for (const k in t1) {
    seen1.push(k);
    if (k === "a") {
      delete t1.hidden; // already visited in creation order before "a"!
      p1.hidden = 100;
      p1.visibleProto = 200;
    }
  }
  if (seen1.join(",") !== "a,b,visibleProto") {
    throw new Error("non-enumerable own key failed to shadow proto property: " + seen1.join(","));
  }

  // 5. FastForInState mid-loop deopt when remaining own key becomes non-enumerable + proto adds same key
  const p2 = {};
  const t2 = Object.create(p2);
  t2.first = 1;
  t2.second = 2;
  const seen2 = [];
  for (const k in t2) {
    seen2.push(k);
    if (k === "first") {
      Object.defineProperty(t2, "second", { value: 2, enumerable: false, configurable: true });
      p2.second = 999;
      p2.fromProto = 3;
    }
  }
  if (seen2.join(",") !== "first,fromProto") {
    throw new Error("mid-loop non-enumerable shadowing mismatch: " + seen2.join(","));
  }

  // 6. FastForInState deopt on UniqueShape prototype gaining enumerable property at end of own keys
  const uProto = {};
  for (let i = 0; i < 1040; i++) {
    Object.defineProperty(uProto, "np_" + i, { value: i, enumerable: false });
  }
  const uTarget = Object.create(uProto);
  uTarget.only = 1;
  const seen3 = [];
  for (const k in uTarget) {
    seen3.push(k);
    if (k === "only") {
      uProto.lateEnum = 55;
    }
  }
  if (seen3.join(",") !== "only,lateEnum") {
    throw new Error("UniqueShape proto enumerable insert missed at end of FastForInState: " + seen3.join(","));
  }

  console.log("IC_AND_FOR_IN_OK");
})();
"#;

    // 1. Raw `SLICEBOA` source evaluation (`boacas = false`) + 2. Precompiled `SLICEBC\x01` + warm reuse
    let raw_sliceboa_bc = {
        let _guard = BoacasThreadOverrideGuard;
        set_thread_exper_boacas_override(Some(false));
        compile_source_to_bytecode(src)
    };
    let bc = compile_source_to_bytecode(src);
    let mut iso = SliceIsolate::spawn(
        "wave3-ic",
        1,
        CapabilityToken::pure_compute(),
        50_000_000,
        32 * 1024 * 1024,
    );
    let m1 = iso
        .execute_bytecode(&raw_sliceboa_bc, None)
        .expect("raw sliceboa exec");
    assert_eq!(m1.stdout, vec!["IC_AND_FOR_IN_OK"]);

    for turn in 0..3 {
        iso.reset_for_request("wave3-ic", turn + 2, 50_000_000, 32 * 1024 * 1024);
        let m = iso.execute_bytecode(&bc, None).expect("warm bytecode exec");
        assert_eq!(m.stdout, vec!["IC_AND_FOR_IN_OK"]);
    }
}

#[test]
fn test_wave3_slicebc_corrupted_constants_and_handlers_fail_closed_without_panic() {
    use boa_engine::{Context, Script, Source};

    let mut ctx = Context::default();
    let js = r#"
let gLex = 10n;
var gVar = 20;
function gFn(a) {
    arguments[0] = 99;
    try {
        return a + gVar;
    } catch (e) {
        return 0;
    }
}
gFn(1);
"#;
    let script = Script::parse(Source::from_bytes(js.as_bytes()), None, &mut ctx).expect("parse");
    let valid_bytes = script
        .compile_to_slicebc(&mut ctx)
        .expect("compile slicebc");
    assert!(
        Script::from_slicebc_bytes(&valid_bytes, &mut ctx).is_some(),
        "baseline valid SLCB\\x01 must decode"
    );

    // 1. Exhaustive single-byte corruption + truncation sweep across the entire SLCB\x01 stream
    // verifying `Script::from_slicebc_bytes` never panics on malformed/corrupted payloads.
    for trunc in 0..valid_bytes.len() {
        let panic_check = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = Script::from_slicebc_bytes(&valid_bytes[..trunc], &mut ctx);
        }));
        assert!(
            panic_check.is_ok(),
            "SLCB\\x01 truncation at {trunc} panicked"
        );
    }
    for pos in (0..valid_bytes.len()).step_by(4) {
        for bad_byte in [0xFFu8, 0x00u8, 0x02u8] {
            let mut mutated = valid_bytes.clone();
            mutated[pos] = bad_byte;
            let panic_check = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = Script::from_slicebc_bytes(&mutated, &mut ctx);
            }));
            assert!(
                panic_check.is_ok(),
                "SLCB\\x01 decode at byte offset {pos} (0x{bad_byte:02x}) caused a Rust panic!"
            );
        }
    }

    // 2. Specifically corrupt trailing `global_vars` / `global_fns` / `global_lexs` u32 constant indices
    // (both out-of-bounds `999` and wrong-variant swaps between String/Function/BigInt constants)
    let len = valid_bytes.len();
    // Root CodeBlock tail layout:
    //   glex_len=1 (4B), glex[0] (4B),
    //   gfn_len=1 (4B), gfn[0].name_index (4B), gfn[0].function_index (4B),
    //   gvar_len=1 (4B), gvar[0] (4B) -> last 28 bytes of `valid_bytes`!
    let glex0_pos = len - 24;
    let gfn_name_pos = len - 16;
    let gfn_func_pos = len - 12;
    let gvar0_pos = len - 4;

    let orig_fn_const_idx = u32::from_le_bytes(
        valid_bytes[gfn_func_pos..gfn_func_pos + 4]
            .try_into()
            .unwrap(),
    );
    let orig_str_const_idx = u32::from_le_bytes(
        valid_bytes[gfn_name_pos..gfn_name_pos + 4]
            .try_into()
            .unwrap(),
    );

    // Wrong Constant variant: `global_lexs[0]` pointing at `Constant::Function` instead of `Constant::String`
    let mut bad_glex_variant = valid_bytes.clone();
    bad_glex_variant[glex0_pos..glex0_pos + 4].copy_from_slice(&orig_fn_const_idx.to_le_bytes());
    assert!(
        Script::from_slicebc_bytes(&bad_glex_variant, &mut ctx).is_none(),
        "global_lexs pointing at Constant::Function must return None"
    );

    // Wrong Constant variant: `global_fns[0].function_index` pointing at `Constant::String`
    let mut bad_gfn_variant = valid_bytes.clone();
    bad_gfn_variant[gfn_func_pos..gfn_func_pos + 4]
        .copy_from_slice(&orig_str_const_idx.to_le_bytes());
    assert!(
        Script::from_slicebc_bytes(&bad_gfn_variant, &mut ctx).is_none(),
        "global_fns.function_index pointing at Constant::String must return None"
    );

    // Wrong Constant variant: `global_vars[0]` pointing at `Constant::Function`
    let mut bad_gvar_variant = valid_bytes.clone();
    bad_gvar_variant[gvar0_pos..gvar0_pos + 4].copy_from_slice(&orig_fn_const_idx.to_le_bytes());
    assert!(
        Script::from_slicebc_bytes(&bad_gvar_variant, &mut ctx).is_none(),
        "global_vars pointing at Constant::Function must return None"
    );

    // OOB constant indices in `global_lexs`, `global_fns`, and `global_vars`
    for pos_oob in [glex0_pos, gfn_name_pos, gfn_func_pos, gvar0_pos] {
        let mut oob = valid_bytes.clone();
        oob[pos_oob..pos_oob + 4].copy_from_slice(&999u32.to_le_bytes());
        assert!(
            Script::from_slicebc_bytes(&oob, &mut ctx).is_none(),
            "OOB constant index at offset {pos_oob} must return None"
        );
    }
}

#[test]
fn test_wave3_settled_await_als_enter_with_isolation_and_warm_reuse() {
    let src = r#"
import { AsyncLocalStorage } from "node:async_hooks";

const als = new AsyncLocalStorage();
const log = [];

async function startedBeforeAlsFulfilled() {
  const v = await Promise.resolve("ok");
  log.push("beforeFulfilled:" + v + ":" + JSON.stringify(als.getStore()));
}

async function startedBeforeAlsRejected() {
  try {
    await Promise.reject(new Error("expected-rej"));
  } catch (e) {
    log.push("beforeRejected:" + e.message + ":" + JSON.stringify(als.getStore()));
  }
}

async function startedAfterEnterWith() {
  const v = await Promise.resolve("after");
  log.push("afterEnter:" + v + ":" + JSON.stringify(als.getStore()));
}

// Launch settled awaits BEFORE `als.enterWith` activates `BOA_ALS_ACTIVE`
const p1 = startedBeforeAlsFulfilled();
const p2 = startedBeforeAlsRejected();

// Synchronously activate ALS via `enterWith` before microtasks drain
als.enterWith({ reqId: "sync-entered" });
log.push("syncNow:" + JSON.stringify(als.getStore()));

const p3 = startedAfterEnterWith();

Promise.all([p1, p2, p3]).then(() => {
  log.push("finalSync:" + JSON.stringify(als.getStore()));
  console.log(JSON.stringify(log));
});
"#;

    let bc = compile_source_to_bytecode(src);
    let mut iso = SliceIsolate::spawn(
        "wave3-als",
        1,
        CapabilityToken::pure_compute(),
        20_000_000,
        16 * 1024 * 1024,
    );
    let m1 = iso.execute_bytecode(&bc, None).expect("als turn 1");
    let parsed: Vec<String> = serde_json::from_str(&m1.stdout[0]).expect("json log");
    assert_eq!(
        parsed,
        vec![
            r#"syncNow:{"reqId":"sync-entered"}"#,
            "beforeFulfilled:ok:undefined",
            "beforeRejected:expected-rej:undefined",
            r#"afterEnter:after:{"reqId":"sync-entered"}"#,
            r#"finalSync:{"reqId":"sync-entered"}"#,
        ]
    );

    // Verify warm-pool turn reset leaves zero leaked ALS store
    let check_bc = compile_source_to_bytecode(
        r#"import { AsyncLocalStorage } from "node:async_hooks"; const a = new AsyncLocalStorage(); console.log(String(a.getStore()));"#,
    );
    iso.reset_for_request("wave3-als", 2, 20_000_000, 16 * 1024 * 1024);
    let m2 = iso.execute_bytecode(&check_bc, None).expect("als turn 2");
    assert_eq!(m2.stdout, vec!["undefined"]);
}

#[test]
fn test_wave3_crypto_pki_without_buffer_shard_esm_cjs_and_cas() {
    // Deliberately never mention `Buffer` so Shard 5 is NOT preloaded in `shards_mask`!
    let esm_src = r#"
import {
  generateKeyPairSync,
  createPrivateKey,
  createPublicKey,
  sign,
  verify
} from "node:crypto";

const kpEd = generateKeyPairSync("ed25519");
const pemPub = kpEd.publicKey.export({ type: "spki", format: "pem" });
const pemPriv = kpEd.privateKey.export({ type: "pkcs8", format: "pem" });
const jwkPub = kpEd.publicKey.export({ format: "jwk" });
const jwkPriv = kpEd.privateKey.export({ format: "jwk" });

const msg = new Uint8Array([10, 20, 30, 40, 50]);
const sigFromPem = sign(null, msg, pemPriv);
const okPem = verify(null, msg, createPublicKey(pemPub), sigFromPem);
const okJwk = verify(
  null,
  msg,
  { key: jwkPub, format: "jwk" },
  sign(null, msg, createPrivateKey({ key: jwkPriv, format: "jwk" }))
);

const kpEc = generateKeyPairSync("ec", { namedCurve: "P-256" });
const ecPubPem = kpEc.publicKey.export({ type: "spki", format: "pem" });
const ecSig = sign("sha256", msg, kpEc.privateKey);
const okEc = verify("sha256", msg, ecPubPem, ecSig);

console.log([okPem, okJwk, okEc].join(","));
"#;

    let cjs_src = r#"
const { generateKeyPairSync, createPublicKey, sign, verify } = require("node:crypto");
const { publicKey, privateKey } = generateKeyPairSync("ed25519", {
  publicKeyEncoding: { type: "spki", format: "pem" },
  privateKeyEncoding: { type: "pkcs8", format: "pem" }
});
const payload = new Uint8Array([1, 2, 3, 4]);
const sig = sign(null, payload, privateKey);
console.log(String(verify(null, payload, createPublicKey(publicKey), sig)));
"#;

    // 1. Direct ESM source (via `run_source_isolated` + raw `SLICEBOA` `boacas = false` mode)
    let r1 = run_source_isolated(
        "wave3-pki-esm",
        esm_src,
        CapabilityToken::pure_compute(),
        30_000_000,
        16 * 1024 * 1024,
    )
    .expect("esm crypto pki");
    assert_eq!(r1.stdout, vec!["true,true,true"]);

    let raw_esm_bc = {
        let _guard = BoacasThreadOverrideGuard;
        set_thread_exper_boacas_override(Some(false));
        compile_source_to_bytecode(esm_src)
    };
    let mut iso_raw = SliceIsolate::spawn(
        "wave3-pki-raw",
        1,
        CapabilityToken::pure_compute(),
        30_000_000,
        16 * 1024 * 1024,
    );
    let r_raw = iso_raw
        .execute_bytecode(&raw_esm_bc, None)
        .expect("raw sliceboa pki");
    assert_eq!(r_raw.stdout, vec!["true,true,true"]);

    // 2. Cold CJS bytecode in fresh isolate
    let cjs_bc = compile_source_to_bytecode(cjs_src);
    let mut iso2 = SliceIsolate::spawn(
        "wave3-pki-cjs",
        2,
        CapabilityToken::pure_compute(),
        30_000_000,
        16 * 1024 * 1024,
    );
    let r2 = iso2
        .execute_bytecode(&cjs_bc, None)
        .expect("cjs crypto pki");
    assert_eq!(r2.stdout, vec!["true"]);

    // 3. Multi-blob `.cas` archive execution
    let cas_path = std::env::temp_dir().join(format!("slice_wave3_pki_{}.cas", std::process::id()));
    let mut builder = CasBuilder::new("./index.ts");
    builder.add_module(
        "./signer.ts",
        PathBuf::from("./signer.ts"),
        esm_src,
        compile_source_to_bytecode,
    );
    builder.add_module(
        "./index.ts",
        PathBuf::from("./index.ts"),
        r#"import "./signer.ts";"#,
        compile_source_to_bytecode,
    );
    builder.write_to_file(&cas_path).expect("write pki cas");
    let raw_cas = std::fs::read(&cas_path).expect("read pki cas");
    let _ = std::fs::remove_file(&cas_path);
    let mapped = MappedCasArchive::from_bytes(&raw_cas).expect("map pki cas");
    let entry_bc = mapped.get_bytecode_slice(mapped.entrypoint_blob().unwrap());
    let mut iso3 = SliceIsolate::spawn(
        "wave3-pki-cas",
        3,
        CapabilityToken::pure_compute(),
        30_000_000,
        16 * 1024 * 1024,
    );
    let r3 = iso3
        .execute_bytecode(entry_bc, Some(&mapped))
        .expect("cas pki exec");
    assert_eq!(r3.stdout, vec!["true,true,true"]);
}

#[test]
fn test_wave3_whatwg_url_all_component_setters_repeated_mutations_and_readonly_origin() {
    let src = r##"
(function() {
  // 1. FastUrl -> repeated pathname, search, hash, port, host, hostname, protocol, href mutations
  const u = new URL("https://edge.slice.dev/v1/items?a=1");
  const spRef = u.searchParams;
  u.pathname = "v2/orders";
  if (u.pathname !== "/v2/orders" || u.href !== "https://edge.slice.dev/v2/orders?a=1") {
    throw new Error("pathname #1 failed: " + u.href);
  }
  u.pathname = "/v3/sub/../final";
  if (u.pathname !== "/v3/final" || u.href !== "https://edge.slice.dev/v3/final?a=1") {
    throw new Error("pathname #2 failed: " + u.href);
  }

  u.port = "8443";
  if (u.port !== "8443" || u.host !== "edge.slice.dev:8443" || u.origin !== "https://edge.slice.dev:8443") {
    throw new Error("port setter failed: " + u.href);
  }
  // WHATWG: `u.host` without `:port` preserves existing port!
  u.host = "API.Slice.Dev";
  if (u.hostname !== "api.slice.dev" || u.port !== "8443" || u.host !== "api.slice.dev:8443") {
    throw new Error("host setter without port must preserve existing port: " + u.href);
  }
  u.host = "cdn.slice.dev:9090";
  if (u.hostname !== "cdn.slice.dev" || u.port !== "9090" || u.origin !== "https://cdn.slice.dev:9090") {
    throw new Error("host setter with port failed: " + u.href);
  }
  u.hostname = "static.slice.dev";
  if (u.host !== "static.slice.dev:9090") {
    throw new Error("hostname setter failed: " + u.href);
  }
  u.port = "80";
  u.protocol = "http";
  if (u.protocol !== "http:" || u.port !== "" || u.host !== "static.slice.dev" || u.origin !== "http://static.slice.dev") {
    throw new Error("protocol default port elision failed: " + u.href);
  }
  // Switching special <-> non-special scheme is a no-op per WHATWG
  u.protocol = "mailto:";
  if (u.protocol !== "http:") {
    throw new Error("special->non-special protocol switch should be ignored: " + u.protocol);
  }

  // [SameObject] live searchParams synchronization across `u.search` and `u.href`
  u.search = "k=10&m=20";
  u.hash = "sec1";
  if (u.searchParams !== spRef || spRef.get("k") !== "10" || spRef.get("m") !== "20" || u.hash !== "#sec1") {
    throw new Error("search/hash sync failed: " + u.href);
  }
  u.href = "https://next.slice.dev/ok?k=99";
  if (u.searchParams !== spRef || spRef.get("k") !== "99" || u.origin !== "https://next.slice.dev") {
    throw new Error("href setter failed: " + u.href);
  }

  // 2. Slow-path URL with credentials + searchParams sync preserves credentials
  const uAuth = new URL("https://alice:s3cr3t@vault.slice.dev:8080/keys?v=1#h");
  uAuth.pathname = "/rotated";
  uAuth.searchParams.set("v", "2");
  if (uAuth.href !== "https://alice:s3cr3t@vault.slice.dev:8080/rotated?v=2#h") {
    throw new Error("credentialed URL mutation failed: " + uAuth.href);
  }

  // 3. Read-only `origin` on both FastUrl and materialized URL (sloppy no-op, strict TypeError)
  const uFast = new URL("https://readonly.slice.dev/a");
  uFast.origin = "https://evil.example";
  u.origin = "https://evil.example";
  if (uFast.origin !== "https://readonly.slice.dev" || u.origin !== "https://next.slice.dev") {
    throw new Error("sloppy write to readonly origin mutated URL");
  }
  let strictFastThrew = false;
  let strictMatThrew = false;
  try {
    (function() { "use strict"; uFast.origin = "https://evil.example"; })();
  } catch (e) {
    strictFastThrew = e instanceof TypeError;
  }
  try {
    (function() { "use strict"; u.origin = "https://evil.example"; })();
  } catch (e) {
    strictMatThrew = e instanceof TypeError;
  }
  if (!strictFastThrew || !strictMatThrew) {
    throw new Error("strict write to readonly origin must throw TypeError");
  }

  console.log("URL_SETTERS_OK");
})();
"##;

    let bc = compile_source_to_bytecode(src);
    let mut iso = SliceIsolate::spawn(
        "wave3-url",
        1,
        CapabilityToken::pure_compute(),
        20_000_000,
        16 * 1024 * 1024,
    );
    let res = iso.execute_bytecode(&bc, None).expect("url setters exec");
    assert_eq!(res.stdout, vec!["URL_SETTERS_OK"]);
}

#[test]
fn test_wave3_response_iterable_headers_evalsandbox_nan_fast_ms_and_closed_streams() {
    let src = r#"
(async function() {
  // 1. Response & Response.json with Map and custom Symbol.iterator headers
  const mapHeaders = new Map([
    ["X-Tenant", "acme"],
    ["Content-Type", "text/csv; charset=utf-8"]
  ]);
  const r1 = new Response("a,b\n1,2", { status: 201, headers: mapHeaders });
  if (r1.headers.get("x-tenant") !== "acme" || r1.headers.get("content-type") !== "text/csv; charset=utf-8") {
    throw new Error("Response Map headers lost: " + JSON.stringify([...r1.headers]));
  }

  const customIter = {
    *[Symbol.iterator]() {
      yield ["X-Iter", "works"];
      yield ["X-Mode", "fast"];
    }
  };
  const r2 = Response.json({ ok: true }, { status: 200, headers: customIter });
  if (r2.headers.get("x-iter") !== "works" || r2.headers.get("content-type") !== "application/json") {
    throw new Error("Response.json iterable headers lost: " + JSON.stringify([...r2.headers]));
  }

  // 2. Slice.evalSandbox ignores NaN numeric options instead of clamping to 0
  const sb = Slice.evalSandbox("const arr = [1, 2, 3]; arr.reduce((a, b) => a + b, 0);", {
    fuel: NaN,
    memoryBytes: NaN,
    deterministicSeed: NaN,
    maxCheckpoints: NaN
  });
  if (!sb.ok || sb.returnValue !== "6") {
    throw new Error("evalSandbox NaN options failed: " + JSON.stringify(sb));
  }

  // 3. `slice:ms` formatting for extreme floats >= ~3.89e23 and fractional sub-second numbers
  const ms = require("slice:ms");
  const expectedDays = Math.round(5e23 / 86400000);
  if (ms(5e23) !== expectedDays + "d" || ms(5e23, { long: true }) !== expectedDays + " days") {
    throw new Error("extreme positive ms mismatch: " + ms(5e23) + " / " + ms(5e23, { long: true }));
  }
  const expectedNegDays = Math.round(-5e23 / 86400000);
  if (ms(-5e23) !== expectedNegDays + "d" || ms(-5e23, { long: true }) !== expectedNegDays + " days") {
    throw new Error("extreme negative ms mismatch: " + ms(-5e23));
  }
  if (ms(0.5) !== "0.5ms" || ms(0.5, { long: true }) !== "0.5 ms") {
    throw new Error("sub-second fractional ms mismatch: " + ms(0.5));
  }

  // 4. Non-empty synchronously closed ReadableStream fast paths (string + split multi-byte UTF-8 Uint8Array)
  const sJson = new ReadableStream({
    start(c) {
      c.enqueue('{"stream":');
      c.enqueue('"fast","n":42}');
      c.close();
    }
  });
  const resJson = new Response(sJson);
  const parsed = await resJson.json();
  if (!resJson.bodyUsed || !sJson.locked || parsed.stream !== "fast" || parsed.n !== 42) {
    throw new Error("closed string stream Response.json mismatch");
  }

  // Split 4-byte UTF-8 rocket (0xF0 0x9F 0x9A 0x80) across two chunks in a synchronously closed stream
  const sBin = new ReadableStream({
    start(c) {
      c.enqueue(new Uint8Array([0x6F, 0x6B, 0x3A, 0xF0, 0x9F]));
      c.enqueue(new Uint8Array([0x9A, 0x80]));
      c.close();
    }
  });
  const resBin = new Response(sBin);
  const binText = await resBin.text();
  if (binText !== "ok:" + String.fromCodePoint(0x1F680) || !resBin.bodyUsed || !sBin.locked) {
    throw new Error("closed binary stream Response.text UTF-8 boundary mismatch: " + binText);
  }

  const sBytes = new ReadableStream({
    start(c) {
      c.enqueue(new Uint8Array([1, 2]));
      c.enqueue(new Uint8Array([3, 4]));
      c.close();
    }
  });
  const u8Out = await new Response(sBytes).bytes();
  if (u8Out.join(",") !== "1,2,3,4") {
    throw new Error("closed stream Response.bytes mismatch: " + u8Out.join(","));
  }

  console.log("WINTERTC_AND_STREAMS_OK");
})();
"#;

    let bc = compile_source_to_bytecode(src);
    let mut iso = SliceIsolate::spawn(
        "wave3-wintertc",
        1,
        CapabilityToken::pure_compute(),
        30_000_000,
        16 * 1024 * 1024,
    );
    let m = iso.execute_bytecode(&bc, None).expect("wintertc exec");
    assert_eq!(m.stdout, vec!["WINTERTC_AND_STREAMS_OK"]);

    // Also verify bundled `node_modules/ms/index.js` CJS rewrite handles >= 3.89e23 identically
    let tmp_dir = std::env::temp_dir().join(format!("slice_wave3_ms_cjs_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let nm_ms = tmp_dir.join("node_modules").join("ms");
    std::fs::create_dir_all(&nm_ms).expect("mkdir nm/ms");
    std::fs::write(
        nm_ms.join("package.json"),
        r#"{"name":"ms","version":"2.1.3","main":"index.js"}"#,
    )
    .expect("write pkg");
    std::fs::write(
        nm_ms.join("index.js"),
        "function fmtShort(ms) { return ms + 'ms'; }\nfunction fmtLong(ms) { return ms + ' ms'; }\n// milliseconds?|msecs?|ms\nmodule.exports = function(v, o) { return o && o.long ? fmtLong(v) : fmtShort(v); };\n",
    )
    .expect("write ms/index.js");
    let entry = tmp_dir.join("app.ts");
    std::fs::write(
        &entry,
        r#"import ms from "ms"; console.log(ms(5e23), "|", ms(5e23, { long: true }));"#,
    )
    .expect("write app.ts");
    let bundled = bundle_entrypoint_with_deps(&entry).expect("bundle ms");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    assert!(bundled.contains("__slice_fast_ms"));
    let bundled_bc = compile_source_to_bytecode(&bundled);
    iso.reset_for_request("wave3-ms-bundle", 2, 20_000_000, 16 * 1024 * 1024);
    let m_ms = iso
        .execute_bytecode(&bundled_bc, None)
        .expect("exec ms bundle");
    assert!(
        m_ms.stdout[0].ends_with("d") || m_ms.stdout[0].ends_with(" days"),
        "unexpected bundled ms output: {:?}",
        m_ms.stdout
    );
    assert!(
        !m_ms.stdout[0].contains("ms"),
        "5e23 must format as days, not ms: {:?}",
        m_ms.stdout
    );
}
