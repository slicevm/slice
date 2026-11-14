use criterion::{criterion_group, criterion_main, Criterion};
use slice_engine::boacas::{compute_boacas_metadata, pack_boacas_container};
use slice_engine::intrinsics::{
    aes_cbc_decrypt_with_expanded, aes_cbc_encrypt_with_expanded, aes_gcm_encrypt,
    aes_gcm_open_with_key, aes_gcm_seal_with_key, base64_decode, base64_encode, crc32_checksum,
    ed25519_keypair_from_seed, ed25519_sign_bytes, ed25519_verify_bytes, fast_parse_bytes,
    fast_parse_ms, hmac_sha256_hex, make_ring_aead_key, parse_whatwg_url, sha256_hex,
    zlib_compress_bytes, zlib_decompress_bytes, AesExpandedKey,
};
use slice_engine::{
    compile_source_to_bytecode, is_tier0_slicesnp_compatible_ast, HttpRequestContext, SliceIsolate,
};
use slice_sandbox::CapabilityToken;
use std::hint::black_box;

fn bench_boacas_and_automaton(c: &mut Criterion) {
    let mut group = c.benchmark_group("slice_engine::boacas");

    let sample_js = r#"
const url = new URL("https://edge.slice.dev/v1/orders?limit=25");
const enc = new TextEncoder().encode(url.pathname);
const digest = crypto.randomUUID();
const out = JSON.stringify({ path: url.pathname, len: enc.length, id: digest });
"#;

    group.bench_function("compute_boacas_metadata_shard_automaton", |b| {
        b.iter(|| {
            let (shards, flags, hash) = compute_boacas_metadata(black_box(sample_js));
            black_box((shards, flags, hash))
        });
    });

    group.bench_function("pack_boacas_container_precompile", |b| {
        b.iter(|| {
            let blob = pack_boacas_container(black_box(sample_js));
            black_box(blob.len())
        });
    });

    let precompiled_bc = compile_source_to_bytecode(sample_js);
    group.bench_function("warm_isolate_execute_boacas_bytecode", |b| {
        SliceIsolate::prewarm_boa_pool(1);
        b.iter(|| {
            let mut iso = SliceIsolate::spawn(
                "bench",
                1,
                CapabilityToken::pure_compute(),
                10_000_000,
                16 * 1024 * 1024,
            );
            let res = iso
                .execute_bytecode(black_box(&precompiled_bc), None)
                .expect("execute_bytecode");
            black_box(res.highest_tier_reached)
        });
    });

    group.bench_function("tier0_ast_eligibility_gate", |b| {
        b.iter(|| {
            let a = is_tier0_slicesnp_compatible_ast(black_box(
                "function sum(n) { let acc = 0; for (let i = 0; i < n; i++) { acc += i; } return acc; } sum(32);",
            ));
            let b_res = is_tier0_slicesnp_compatible_ast(black_box("process.exitCode = 2;"));
            black_box((a, b_res))
        });
    });

    group.finish();
}

fn bench_intrinsics_and_numeric_vm(c: &mut Criterion) {
    let mut group = c.benchmark_group("slice_engine::intrinsics_and_numeric_vm");

    group.bench_function("sha256_hex_64b", |b| {
        let payload = b"slice-nexus-js-zero-trust-edge-runtime-benchmark-payload-64bytes";
        b.iter(|| {
            let hex = sha256_hex(black_box(payload));
            black_box(hex.len())
        });
    });

    group.bench_function("hmac_sha256_hex_64b", |b| {
        let key = b"super-secret-signing-key-32bytes";
        let payload = b"slice-nexus-js-zero-trust-edge-runtime-benchmark-payload-64bytes";
        b.iter(|| {
            let mac = hmac_sha256_hex(black_box(key), black_box(payload));
            black_box(mac.len())
        });
    });

    group.bench_function("fast_parse_ms_and_bytes", |b| {
        b.iter(|| {
            let ms = fast_parse_ms(black_box("2.5h"));
            let bytes = fast_parse_bytes(black_box("128MB"));
            black_box((ms, bytes))
        });
    });

    let numeric_src = r#"
function fib(n) {
    if (n <= 1) return n;
    return fib(n - 1) + fib(n - 2);
}
fib(30);
"#;
    let numeric_bc = compile_source_to_bytecode(numeric_src);
    group.bench_function("numeric_vm_recurrence_fib30", |b| {
        b.iter(|| {
            let mut iso = SliceIsolate::spawn(
                "bench-num",
                1,
                CapabilityToken::pure_compute(),
                10_000_000,
                16 * 1024 * 1024,
            );
            let res = iso
                .execute_bytecode(black_box(&numeric_bc), None)
                .expect("numeric execute");
            black_box(res.highest_tier_reached)
        });
    });

    // Phase 2: AES-GCM-256 cached `LessSafeKey` handle vs uncached key expansion + Ed25519 sign/verify
    let key_256 = [0x42u8; 32];
    let iv_12 = [0x11u8; 12];
    let aad = b"slice-jwt-jwe-header";
    let payload_256b = [0x5au8; 256];
    let cached_key = make_ring_aead_key(&key_256).expect("valid AES-256-GCM key");

    group.bench_function("aes_256_gcm_seal_open_cached_handle_256b", |b| {
        b.iter(|| {
            let ct = aes_gcm_seal_with_key(
                black_box(&cached_key),
                black_box(&iv_12),
                black_box(aad),
                black_box(&payload_256b),
                128,
            )
            .expect("seal");
            let pt = aes_gcm_open_with_key(
                black_box(&cached_key),
                black_box(&key_256),
                black_box(&iv_12),
                black_box(aad),
                black_box(&ct),
                128,
            )
            .expect("open");
            black_box(pt.len())
        });
    });

    group.bench_function("aes_256_gcm_seal_uncached_256b", |b| {
        b.iter(|| {
            let ct = aes_gcm_encrypt(
                black_box(&key_256),
                black_box(&iv_12),
                black_box(aad),
                black_box(&payload_256b),
                128,
            )
            .expect("encrypt");
            black_box(ct.len())
        });
    });

    let (ed_seed, ed_pub) = ed25519_keypair_from_seed([0x77u8; 32]).expect("ed25519 keypair");
    let ed_sig =
        ed25519_sign_bytes(&ed_seed, b"slice-ed25519-jwt-token-payload").expect("ed25519 sign");
    group.bench_function("ed25519_verify_signature", |b| {
        b.iter(|| {
            let ok = ed25519_verify_bytes(
                black_box(&ed_pub),
                black_box(b"slice-ed25519-jwt-token-payload"),
                black_box(&ed_sig),
            );
            black_box(ok)
        });
    });

    let cbc_expanded = AesExpandedKey::new(&key_256).expect("valid AES-256-CBC key");
    let iv_16 = [0x22u8; 16];
    group.bench_function("aes_256_cbc_seal_open_branchless_256b", |b| {
        b.iter(|| {
            let ct = aes_cbc_encrypt_with_expanded(
                black_box(&cbc_expanded),
                black_box(&iv_16),
                black_box(&payload_256b),
                true,
            )
            .expect("cbc encrypt");
            let pt = aes_cbc_decrypt_with_expanded(
                black_box(&cbc_expanded),
                black_box(&iv_16),
                black_box(&ct),
                true,
            )
            .expect("cbc decrypt");
            black_box(pt.len())
        });
    });

    let sample_4kb =
        r#"{"service":"slice-edge","status":"ok","items":[1,2,3,4,5],"region":"us-east-1"}"#
            .repeat(52);
    let sample_gz = zlib_compress_bytes(sample_4kb.as_bytes(), "gzip").expect("gzip");
    group.bench_function("crc32_table_4kb", |b| {
        b.iter(|| black_box(crc32_checksum(black_box(sample_4kb.as_bytes()))));
    });
    group.bench_function("gunzip_huffman_lut_4kb", |b| {
        b.iter(|| {
            let out = zlib_decompress_bytes(black_box(&sample_gz), "gzip").expect("gunzip");
            black_box(out.len())
        });
    });
    group.bench_function("base64_lut_roundtrip_256b", |b| {
        b.iter(|| {
            let enc = base64_encode(black_box(&payload_256b), false);
            let dec = base64_decode(black_box(&enc));
            black_box(dec.len())
        });
    });
    group.bench_function("whatwg_url_parse", |b| {
        b.iter(|| {
            let parsed = parse_whatwg_url(
                black_box("https://edge.slice.dev:8443/v1/orders/../items?limit=25&sort=asc#top"),
                None,
            )
            .expect("parse url");
            black_box(parsed.pathname.len())
        });
    });

    group.finish();
}

fn bench_json_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("slice_engine::json");

    let json_batch_src = r#"
const payload = JSON.stringify(
    Array.from({ length: 100 }, (_, i) => ({
        id: i,
        name: "item-" + i,
        escaped: "line1\nline2\t\"quoted\"\\slash",
        score: i * 1.25,
        ts: 1731274800000 + i,
        active: (i & 1) === 0,
        tags: ["alpha", "beta", "gamma"],
        meta: { nested: i + 100, ok: true }
    }))
);
const parsed = JSON.parse(payload);
const out = JSON.stringify(parsed);
if (parsed.length !== 100 || out.length < 1000) throw new Error("json batch mismatch");
"#;
    let json_batch_bc = compile_source_to_bytecode(json_batch_src);
    group.bench_function("json_parse_and_stringify_batch_100_records", |b| {
        SliceIsolate::prewarm_boa_pool(1);
        b.iter(|| {
            let mut iso = SliceIsolate::spawn(
                "bench-json-batch",
                1,
                CapabilityToken::pure_compute(),
                50_000_000,
                16 * 1024 * 1024,
            );
            let res = iso
                .execute_bytecode(black_box(&json_batch_bc), None)
                .expect("json batch execute");
            black_box(res.highest_tier_reached)
        });
    });

    let json_edge_src = r#"
const deepArr = "[".repeat(256) + "42" + "]".repeat(256);
const parsedDeep = JSON.parse(deepArr);
const edgeObj = JSON.parse('{"f1":9007199254740993.0,"f2":1e23,"inf":1e400,"surr":"\\ud800"}');
const revived = JSON.parse('{"id":9007199254740993,"tag":"ok"}', (k, v, ctx) => {
    if (k === "id" && ctx && ctx.source) return ctx.source;
    return v;
});
const out = JSON.stringify({ edgeObj, revived });
if (!Array.isArray(parsedDeep) || revived.id !== "9007199254740993" || out.length < 20) {
    throw new Error("json edge mismatch");
}
"#;
    let json_edge_bc = compile_source_to_bytecode(json_edge_src);
    group.bench_function("json_deep_nesting_reviver_and_edge_cases", |b| {
        SliceIsolate::prewarm_boa_pool(1);
        b.iter(|| {
            let mut iso = SliceIsolate::spawn(
                "bench-json-edge",
                1,
                CapabilityToken::pure_compute(),
                50_000_000,
                16 * 1024 * 1024,
            );
            let res = iso
                .execute_bytecode(black_box(&json_edge_bc), None)
                .expect("json edge execute");
            black_box(res.highest_tier_reached)
        });
    });

    group.finish();
}

fn bench_vfs_and_stream_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("slice_engine::vfs_and_streams");

    let vfs_fd_src = r#"
const fs = require("node:fs");
fs.mkdirSync("/tmp/bench-dir/sub", { recursive: true });
const fd = fs.openSync("/tmp/bench-dir/sub/data.bin", "w+");
const payload = Buffer.from("slice-vfs-fd-benchmark-payload-64bytes-0123456789abcdef01234567");
fs.writeSync(fd, payload, 0, payload.length, 0);
const rbuf = Buffer.allocUnsafe(64);
const n = fs.readSync(fd, rbuf, 0, 64, 0);
fs.closeSync(fd);
const raw = fs.readFileSync("/tmp/bench-dir/sub/data.bin");
const entries = fs.readdirSync("/tmp/bench-dir", { recursive: true });
if (n !== 64 || raw.length !== 64 || entries.length !== 2) throw new Error("vfs bench mismatch");
"#;
    let vfs_fd_bc = compile_source_to_bytecode(vfs_fd_src);
    group.bench_function("vfs_fd_open_read_write_and_recursive_readdir", |b| {
        SliceIsolate::prewarm_boa_pool(1);
        b.iter(|| {
            let caps = CapabilityToken::pure_compute()
                .with_read_prefix("/tmp")
                .with_write_prefix("/tmp");
            let mut iso =
                SliceIsolate::spawn("bench-vfs-fd", 1, caps, 50_000_000, 16 * 1024 * 1024);
            let res = iso
                .execute_bytecode(black_box(&vfs_fd_bc), None)
                .expect("vfs fd execute");
            black_box(res.highest_tier_reached)
        });
    });

    let stream_decoder_src = r#"
const { StringDecoder } = require("node:string_decoder");
const { Readable, Writable } = require("node:stream");
const sd = new StringDecoder("utf8");
const chunk = Buffer.from("Café €100 🎉 日本語 - streaming benchmark payload ", "utf8");
let out = "";
for (let i = 0; i < chunk.length; i += 5) {
    out += sd.write(chunk.subarray(i, Math.min(i + 5, chunk.length)));
}
out += sd.end();
let drainedCount = 0;
const w = new Writable({
    highWaterMark: 32,
    write(_c, _enc, cb) { cb(null); }
});
w.write(chunk);
w.end();
if (out.length < 20) throw new Error("stream decoder mismatch");
"#;
    let stream_decoder_bc = compile_source_to_bytecode(stream_decoder_src);
    group.bench_function("string_decoder_multibyte_and_writable_hwm", |b| {
        SliceIsolate::prewarm_boa_pool(1);
        b.iter(|| {
            let mut iso = SliceIsolate::spawn(
                "bench-stream-decoder",
                1,
                CapabilityToken::pure_compute(),
                50_000_000,
                16 * 1024 * 1024,
            );
            let res = iso
                .execute_bytecode(black_box(&stream_decoder_bc), None)
                .expect("stream decoder execute");
            black_box(res.highest_tier_reached)
        });
    });

    group.finish();
}

fn bench_wave3_als_intl_crypto_and_buffer(c: &mut Criterion) {
    let mut group = c.benchmark_group("slice_engine::wave3");

    let als_intl_src = r#"
const { AsyncLocalStorage } = require("node:async_hooks");
const als = new AsyncLocalStorage();
const nf = new Intl.NumberFormat("de-DE", { style: "currency", currency: "EUR" });
const dtf = new Intl.DateTimeFormat("en-US", { timeZone: "Asia/Tokyo", hour: "2-digit", minute: "2-digit", hour12: false });
const seg = new Intl.Segmenter("en", { granularity: "grapheme" });
const buf = Buffer.from("slice-edge-runtime-buffer-search-needle-benchmark-payload");
const c1 = nf.format(1234567.89);
const d1 = dtf.format(new Date(0));
const gCount = [...seg.segment("a👍🏽é\r\n🇺🇸")].length;
const idx = buf.indexOf("needle");
const ab = new ArrayBuffer(64);
const ab2 = ab.transfer(64);
if (idx < 0 || gCount !== 5 || ab2.byteLength !== 64 || !c1 || !d1) throw new Error("wave3 bench mismatch");
"#;
    let als_intl_bc = compile_source_to_bytecode(als_intl_src);
    group.bench_function("intl_buffer_indexof_and_arraybuffer_transfer", |b| {
        SliceIsolate::prewarm_boa_pool(1);
        b.iter(|| {
            let mut iso = SliceIsolate::spawn(
                "bench-wave3",
                1,
                CapabilityToken::pure_compute(),
                50_000_000,
                16 * 1024 * 1024,
            );
            let res = iso
                .execute_bytecode(black_box(&als_intl_bc), None)
                .expect("wave3 bench execute");
            black_box(res.highest_tier_reached)
        });
    });

    group.finish();
}

fn bench_http_cli_and_host_fs(c: &mut Criterion) {
    let mut group = c.benchmark_group("slice_engine::http_and_cli");

    let tier0_http_bc =
        compile_source_to_bytecode("const base = 1400; const fee = 37; return base + fee;");
    group.bench_function("http_fast_dispatch_tier0_slicesnp", |b| {
        let mut iso = SliceIsolate::spawn(
            "tenant-default",
            1,
            CapabilityToken::pure_compute(),
            10_000_000,
            16 * 1024 * 1024,
        );
        let hdrs = [("host", "localhost"), ("accept", "application/json")];
        let req = HttpRequestContext {
            method: "GET",
            path: "/v1/quote",
            query: "tier=0",
            tenant_id: "tenant-default",
            body: "",
            headers: &hdrs,
        };
        let mut slot = 1u32;
        b.iter(|| {
            slot = slot.wrapping_add(1);
            iso.reset_for_request("tenant-default", slot, 10_000_000, 16 * 1024 * 1024);
            let out = iso
                .execute_http_request_fast(black_box(&tier0_http_bc), None, black_box(&req))
                .expect("tier0 http fast");
            black_box(out.response_body.len())
        });
    });

    let wintertc_http_src = r#"
const req = Slice.request();
Slice.respond(
    200,
    JSON.stringify({ ok: true, method: req.method, path: req.path, tenant: req.tenant }),
    "application/json"
);
"#;
    let wintertc_http_bc = compile_source_to_bytecode(wintertc_http_src);
    group.bench_function("http_fast_dispatch_wintertc_boacas", |b| {
        SliceIsolate::prewarm_boa_pool(1);
        let mut iso = SliceIsolate::spawn(
            "tenant-default",
            1,
            CapabilityToken::pure_compute(),
            10_000_000,
            16 * 1024 * 1024,
        );
        let hdrs = [("host", "localhost"), ("user-agent", "slice-bench/1.0")];
        let req = HttpRequestContext {
            method: "GET",
            path: "/v1/health",
            query: "region=iad",
            tenant_id: "tenant-default",
            body: "",
            headers: &hdrs,
        };
        let mut slot = 1u32;
        b.iter(|| {
            slot = slot.wrapping_add(1);
            iso.reset_for_request("tenant-default", slot, 10_000_000, 16 * 1024 * 1024);
            let out = iso
                .execute_http_request_fast(black_box(&wintertc_http_bc), None, black_box(&req))
                .expect("wintertc http fast");
            black_box(out.response_body.len())
        });
    });

    let kv_cli_src = r#"
Slice.kv.set("bench:k1", "alpha-value", 0);
const val = Slice.kv.get("bench:k1");
const bl = Buffer.byteLength(val + process.argv[0], "utf8");
if (bl < 10) throw new Error("kv cli bench mismatch");
"#;
    let kv_cli_bc = compile_source_to_bytecode(kv_cli_src);
    group.bench_function("warm_kv_and_process_cli_boacas", |b| {
        SliceIsolate::prewarm_boa_pool(1);
        b.iter(|| {
            let mut iso = SliceIsolate::spawn(
                "bench-kv-cli",
                1,
                CapabilityToken::pure_compute(),
                10_000_000,
                16 * 1024 * 1024,
            );
            let res = iso
                .execute_bytecode(black_box(&kv_cli_bc), None)
                .expect("kv cli execute");
            black_box(res.fuel_consumed)
        });
    });

    let tmp_dir = std::env::temp_dir().join("slice_microbench_host_fs");
    let _ = std::fs::create_dir_all(&tmp_dir);
    let host_file = tmp_dir.join("sample_payload.json");
    let sample_json = r#"{"service":"slice-runtime","ok":true,"count":42,"tags":["fast","safe","deterministic"]}"#
        .repeat(8);
    let _ = std::fs::write(&host_file, sample_json.as_bytes());
    let host_file_norm = host_file.to_string_lossy().replace('\\', "/");
    let tmp_dir_norm = tmp_dir.to_string_lossy().replace('\\', "/");
    let host_fs_src = format!(
        r#"
const fs = require("node:fs");
const txt = fs.readFileSync({:?}, "utf8");
const raw = fs.readFileSync({:?});
if (txt.length !== raw.length || txt.length < 64) throw new Error("host fs mismatch");
"#,
        host_file_norm, host_file_norm
    );
    let host_fs_bc = compile_source_to_bytecode(&host_fs_src);
    group.bench_function("host_fs_read_file_sync_utf8_and_bytes", |b| {
        SliceIsolate::prewarm_boa_pool(1);
        let caps = CapabilityToken::pure_compute().with_read_prefix(&tmp_dir_norm);
        b.iter(|| {
            let mut iso = SliceIsolate::spawn(
                "bench-host-fs",
                1,
                caps.clone(),
                10_000_000,
                16 * 1024 * 1024,
            );
            let res = iso
                .execute_bytecode(black_box(&host_fs_bc), None)
                .expect("host fs execute");
            black_box(res.fuel_consumed)
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_boacas_and_automaton,
    bench_intrinsics_and_numeric_vm,
    bench_json_operations,
    bench_vfs_and_stream_operations,
    bench_wave3_als_intl_crypto_and_buffer,
    bench_http_cli_and_host_fs
);
criterion_main!(benches);
