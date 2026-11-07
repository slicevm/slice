use criterion::{criterion_group, criterion_main, Criterion};
use slice_engine::boacas::{compute_boacas_metadata, pack_boacas_container};
use slice_engine::intrinsics::{fast_parse_bytes, fast_parse_ms, hmac_sha256_hex, sha256_hex};
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

    group.finish();
}

criterion_group!(benches, bench_boacas_and_automaton, bench_intrinsics_and_numeric_vm);
criterion_main!(benches);
