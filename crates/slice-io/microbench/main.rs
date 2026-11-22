use criterion::{Criterion, criterion_group, criterion_main};
use slice_io::{CompletionRingEngine, FIXED_BUFFER_SIZE, IoSubmission, RegisteredBufferPool};
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

    let temp_path =
        std::env::temp_dir().join(format!("slice_io_microbench_{}.bin", std::process::id()));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(&temp_path)
        .expect("bench temp file");
    #[cfg(unix)]
    let raw_fd = {
        use std::os::fd::AsRawFd;
        file.as_raw_fd() as i64
    };
    #[cfg(windows)]
    let raw_fd = {
        use std::os::windows::io::AsRawHandle;
        file.as_raw_handle() as isize as i64
    };
    let (w_slot, w_buf) = engine.buffer_pool_mut().acquire().expect("w_slot");
    w_buf[..4096].fill(0xA5);
    let (r_slot, _) = engine.buffer_pool_mut().acquire().expect("r_slot");
    let _ = engine
        .submit_and_reap(&[IoSubmission::WriteFixed {
            fd: raw_fd,
            buf_index: w_slot,
            offset: 0,
            len: 4096,
            user_data: 0x1001,
        }])
        .expect("seed 4kb page");

    let read_op = [IoSubmission::ReadFixed {
        fd: raw_fd,
        buf_index: r_slot,
        offset: 0,
        len: 4096,
        user_data: 0x1002,
    }];
    group.bench_function("positioned_read_fixed_4kb", |b| {
        b.iter(|| {
            let res = engine.submit_and_reap(black_box(&read_op)).unwrap();
            black_box(res[0].result);
        });
    });

    engine.buffer_pool_mut().release(w_slot);
    engine.buffer_pool_mut().release(r_slot);
    drop(file);
    let _ = std::fs::remove_file(&temp_path);

    group.finish();
}

criterion_group!(benches, bench_slab_pool, bench_completion_ring);
criterion_main!(benches);
