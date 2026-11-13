use crate::*;

#[test]
fn test_completion_ring_batch_roundtrip() {
    let mut engine = CompletionRingEngine::new(128, true).unwrap();
    let ops = vec![
        IoSubmission::Nop { user_data: 101 },
        IoSubmission::Nop { user_data: 202 },
        IoSubmission::Nop { user_data: 303 },
    ];
    let completions = engine.submit_and_reap(&ops).unwrap();
    assert_eq!(completions.len(), 3);
    assert_eq!(completions[0].user_data, 101);
    assert_eq!(completions[2].user_data, 303);
}

#[test]
fn test_submission_bounds_and_pinned_safety() {
    let mut engine = CompletionRingEngine::new(64, false).unwrap();
    let bad_index_op = vec![IoSubmission::ReadFixed {
        fd: 0,
        buf_index: 999,
        offset: 0,
        len: 64,
        user_data: 1,
    }];
    assert!(engine.submit_and_reap(&bad_index_op).is_err());

    let bad_len_op = vec![IoSubmission::WriteFixed {
        fd: 1,
        buf_index: 0,
        offset: 0,
        len: (FIXED_BUFFER_SIZE as u32) + 1,
        user_data: 2,
    }];
    assert!(engine.submit_and_reap(&bad_len_op).is_err());
    assert_eq!(engine.buffer_pool.get_slice(999, 16), &[]);
}

#[test]
fn test_ring_entry_clamping_and_extended_pool_aliasing() {
    let min_engine = CompletionRingEngine::new(0, false).unwrap();
    assert_eq!(min_engine.status.ring_capacity, 1);

    let mut max_engine = CompletionRingEngine::new(u32::MAX, false).unwrap();
    assert_eq!(max_engine.status.ring_capacity, MAX_RING_ENTRIES);
    assert_eq!(max_engine.buffer_pool.capacity(), FIXED_BUFFER_COUNT);
    assert_eq!(max_engine.buffer_pool.buffer_size(), FIXED_BUFFER_SIZE);

    // Attach an extended pool (> 256 slots) and verify high-index bounds and DMA aliasing guards
    max_engine.buffer_pool = RegisteredBufferPool::new(320, 1024);
    assert_eq!(max_engine.buffer_pool.capacity(), 320);
    assert_eq!(max_engine.buffer_pool.buffer_size(), 1024);

    let aliased_high_slot = vec![
        IoSubmission::ReadFixed {
            fd: 0,
            buf_index: 300,
            offset: 0,
            len: 512,
            user_data: 10,
        },
        IoSubmission::WriteFixed {
            fd: 1,
            buf_index: 300,
            offset: 0,
            len: 512,
            user_data: 11,
        },
    ];
    assert!(
        max_engine.submit_and_reap(&aliased_high_slot).is_err(),
        "Expected concurrent ReadFixed + WriteFixed aliasing on high slot >= 256 to be rejected"
    );

    let over_slot_len = vec![IoSubmission::WriteFixed {
        fd: 1,
        buf_index: 300,
        offset: 0,
        len: 2048,
        user_data: 12,
    }];
    assert!(max_engine.submit_and_reap(&over_slot_len).is_err());
}
