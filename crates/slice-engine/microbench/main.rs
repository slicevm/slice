use criterion::{criterion_group, criterion_main, Criterion};
use slice_engine::boacas::{compute_boacas_metadata, pack_boacas_container};
use slice_engine::intrinsics::{
    aes_gcm_encrypt, aes_gcm_open_with_key, aes_gcm_seal_with_key, ed25519_keypair_from_seed,
    ed25519_sign_bytes, ed25519_verify_bytes, fast_parse_bytes, fast_parse_ms, hmac_sha256_hex,
    make_ring_aead_key, sha256_hex,
};
use slice_engine::{compile_source_to_bytecode, SliceIsolate};
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
    let ed_sig = ed25519_sign_bytes(&ed_seed, b"slice-ed25519-jwt-token-payload").expect("ed25519 sign");
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

    group.finish();
}

criterion_group!(benches, bench_boacas_and_automaton, bench_intrinsics_and_numeric_vm);
criterion_main!(benches);
