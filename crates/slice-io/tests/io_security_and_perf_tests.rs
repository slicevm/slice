use crate::*;
use std::time::Instant;

#[test]
fn test_io_sec_01_unallocated_slot_read_blocked_and_zeroed_release() {
    let mut pool = RegisteredBufferPool::new(4, 256);
    let (idx, buf) = pool.acquire().unwrap();
    assert_eq!(idx, 0);
    buf[..12].copy_from_slice(b"TOP_SECRET!!");
    assert_eq!(pool.in_use_count(), 1);
    assert_eq!(pool.available_count(), 3);
    assert!(pool.is_slot_in_use(idx));
    assert_eq!(pool.get_slice(idx, 12), b"TOP_SECRET!!");

    // Unallocated slot 1 must return `&[]` from `get_slice` and `None` from `buffer_mut`
    assert_eq!(pool.get_slice(1, 12), &[]);
    assert!(pool.buffer_mut(1).is_none());

    // Releasing slot 0 with `release_zeroed` scrubs the secret bytes and marks slot 0 inactive
    pool.release_zeroed(idx, 12);
    assert!(!pool.is_slot_in_use(idx));
    assert_eq!(pool.in_use_count(), 0);
    // Reading released slot 0 via `get_slice` must return `&[]` (blocks Use-After-Release)
    assert_eq!(pool.get_slice(idx, 12), &[]);

    // Re-acquiring slot 0 (LIFO cache-warm) yields scrubbed zeroed memory
    let (re_idx, re_buf) = pool.acquire().unwrap();
    assert_eq!(re_idx, 0);
    assert_eq!(&re_buf[..12], &[0u8; 12]);
}

#[test]
fn test_io_sec_03_and_04_ring_capacity_negative_fd_and_dma_aliasing_blocked() {
    let mut engine = CompletionRingEngine::new(4, false).unwrap();

    // 1. Empty batch fast-path returns immediately
    assert!(engine.submit_and_reap(&[]).unwrap().is_empty());

    // 2. Batch exceeding `ring_capacity` (5 > 4) must be rejected before submission
    let oversized_batch = vec![
        IoSubmission::Nop { user_data: 1 },
        IoSubmission::Nop { user_data: 2 },
        IoSubmission::Nop { user_data: 3 },
        IoSubmission::Nop { user_data: 4 },
        IoSubmission::Nop { user_data: 5 },
    ];
    assert!(
        engine.submit_and_reap(&oversized_batch).is_err(),
        "Expected batch exceeding ring_capacity to be rejected"
    );

    // 3. Negative `fd` (-1) must be rejected
    let neg_fd_batch = vec![IoSubmission::ReadFixed {
        fd: -1,
        buf_index: 0,
        offset: 0,
        len: 64,
        user_data: 10,
    }];
    assert!(
        engine.submit_and_reap(&neg_fd_batch).is_err(),
        "Expected negative fd (-1) to be rejected"
    );

    // 4. Concurrent `ReadFixed` + `ReadFixed` or `ReadFixed` + `WriteFixed` on the same `buf_index`
    //    within a single batch must be rejected to prevent concurrent kernel DMA data races
    let aliased_read_batch = vec![
        IoSubmission::ReadFixed {
            fd: 0,
            buf_index: 2,
            offset: 0,
            len: 64,
            user_data: 20,
        },
        IoSubmission::ReadFixed {
            fd: 0,
            buf_index: 2,
            offset: 64,
            len: 64,
            user_data: 21,
        },
    ];
    assert!(
        engine.submit_and_reap(&aliased_read_batch).is_err(),
        "Expected concurrent ReadFixed DMA aliasing on slot 2 to be rejected"
    );

    let aliased_rw_batch = vec![
        IoSubmission::WriteFixed {
            fd: 1,
            buf_index: 3,
            offset: 0,
            len: 64,
            user_data: 30,
        },
        IoSubmission::ReadFixed {
            fd: 0,
            buf_index: 3,
            offset: 0,
            len: 64,
            user_data: 31,
        },
    ];
    assert!(
        engine.submit_and_reap(&aliased_rw_batch).is_err(),
        "Expected concurrent WriteFixed + ReadFixed DMA aliasing on slot 3 to be rejected"
    );
}

#[test]
fn test_io_perf_lifo_slab_cache_warm_and_o1_telemetry() {
    let mut pool = RegisteredBufferPool::new(FIXED_BUFFER_COUNT, FIXED_BUFFER_SIZE);
    let iterations = 500_000usize;

    let start = Instant::now();
    for _ in 0..iterations {
        let (idx, buf) = pool.acquire().unwrap();
        buf[0] = 0xAA;
        std::hint::black_box(pool.in_use_count());
        pool.release(idx);
    }
    let elapsed = start.elapsed();
    assert_eq!(pool.in_use_count(), 0);
    println!(
        "[IO-PERF] 500,000 iterations of (acquire + write + O(1) in_use_count + release): {:?} ({:.2} ns/iter)",
        elapsed,
        elapsed.as_nanos() as f64 / (iterations as f64)
    );

    let scrub_iters = 50_000usize;
    let scrub_start = Instant::now();
    for _ in 0..scrub_iters {
        let (idx, buf) = pool.acquire().unwrap();
        buf[..256].fill(0x5A);
        pool.release_zeroed(idx, 256);
    }
    let scrub_elapsed = scrub_start.elapsed();
    println!(
        "[IO-PERF] 50,000 iterations of (acquire + 256B write + release_zeroed): {:?} ({:.2} ns/iter)",
        scrub_elapsed,
        scrub_elapsed.as_nanos() as f64 / (scrub_iters as f64)
    );
}
