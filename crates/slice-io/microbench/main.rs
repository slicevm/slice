use criterion::{criterion_group, criterion_main, Criterion};
use slice_io::{CompletionRingEngine, IoSubmission, RegisteredBufferPool, FIXED_BUFFER_SIZE};
use std::hint::black_box;

fn bench_slab_pool(c: &mut Criterion) {
    let mut group = c.benchmark_group("slice_io::slab_pool");
    let mut pool = RegisteredBufferPool::new(64, FIXED_BUFFER_SIZE);

    group.bench_function("acquire_write_release_64kb_slab", |b| {
        b.iter(|| {
            let (idx, buf) = pool.acquire().expect("slab available");
            buf[..16].copy_from_slice(black_box(b"slice-zero-copy!"));
            black_box(buf[0]);
            pool.release(idx);
        });
    });

    group.finish();
}

fn bench_completion_ring(c: &mut Criterion) {
    let mut group = c.benchmark_group("slice_io::completion_ring");
    let mut engine = CompletionRingEngine::new(128, false).expect("ring init");
    let batch = vec![
        IoSubmission::Nop { user_data: 1 },
        IoSubmission::Nop { user_data: 2 },
        IoSubmission::Nop { user_data: 3 },
        IoSubmission::Nop { user_data: 4 },
    ];

    group.bench_function("submit_and_reap_4_nops", |b| {
        b.iter(|| {
            let res = engine.submit_and_reap(black_box(&batch)).unwrap();
            black_box(res.len());
        });
    });

    group.finish();
}

criterion_group!(benches, bench_slab_pool, bench_completion_ring);
criterion_main!(benches);

