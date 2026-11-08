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
