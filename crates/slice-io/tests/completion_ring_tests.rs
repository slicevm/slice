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
    assert_eq!(engine.buffer_pool().get_slice(999, 16), &[]);
}

#[test]
fn test_ring_entry_clamping_and_extended_pool_aliasing() {
    let min_engine = CompletionRingEngine::new(0, false).unwrap();
    assert_eq!(min_engine.status.ring_capacity, 1);

    let default_max = CompletionRingEngine::new(u32::MAX, false).unwrap();
    assert_eq!(default_max.status.ring_capacity, MAX_RING_ENTRIES);
    assert_eq!(default_max.buffer_pool().capacity(), FIXED_BUFFER_COUNT);
    assert_eq!(default_max.buffer_pool().buffer_size(), FIXED_BUFFER_SIZE);

    // Construct an engine with an extended pool (> 256 slots) and verify high-index bounds and DMA aliasing guards
    let mut max_engine =
        CompletionRingEngine::with_pool_config(u32::MAX, false, 320, 1024).unwrap();
    assert_eq!(max_engine.buffer_pool().capacity(), 320);
    assert_eq!(max_engine.buffer_pool().buffer_size(), 1024);

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

#[test]
fn test_large_batch_completion_reaping_across_multi_chunk_windows() {
    let mut engine = CompletionRingEngine::new(256, false).unwrap();
    let ops: Vec<IoSubmission> = (0..80u64)
        .map(|i| IoSubmission::Nop {
            user_data: 10_000 + i,
        })
        .collect();
    let completions = engine.submit_and_reap(&ops).unwrap();
    assert_eq!(completions.len(), 80);
    let mut seen: Vec<u64> = completions.iter().map(|c| c.user_data).collect();
    seen.sort_unstable();
    let expected: Vec<u64> = (10_000..10_080).collect();
    assert_eq!(seen, expected);
}

fn file_raw_io_descriptor(file: &std::fs::File) -> i64 {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        file.as_raw_fd() as i64
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        file.as_raw_handle() as isize as i64
    }
}

#[test]
fn test_positioned_write_fixed_read_fixed_eof_error_and_64bit_userdata() {
    let mut engine = CompletionRingEngine::new(64, false).unwrap();
    let temp_dir = std::env::temp_dir();
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let rw_path = temp_dir.join(format!(
        "slice_io_ring_rw_{}_{}.bin",
        std::process::id(),
        unique
    ));
    let wo_path = temp_dir.join(format!(
        "slice_io_ring_wo_{}_{}.bin",
        std::process::id(),
        unique
    ));

    let rw_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(&rw_path)
        .expect("open temp rw file");
    let wo_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&wo_path)
        .expect("open temp write-only file");

    let rw_fd = file_raw_io_descriptor(&rw_file);
    let wo_fd = file_raw_io_descriptor(&wo_file);

    let payload_head = b"SLICE-RING-HEAD-0001";
    let payload_tail = b"SLICE-RING-OFFSET-4096-PAYLOAD!!";
    let write_slot = engine.buffer_pool_mut().acquire_slot().expect("write slot");
    let read_slot = engine.buffer_pool_mut().acquire_slot().expect("read slot");
    assert_eq!(
        engine.buffer_pool().slot_addr(read_slot).unwrap() & (PAGE_ALIGNMENT - 1),
        0
    );
    engine.buffer_pool_mut().buffer_mut(write_slot).unwrap()[..payload_head.len()]
        .copy_from_slice(payload_head);
    let hi_tag_a: u64 = (0xDEAD_BEEFu64 << 32) | 0x0000_CAFEu64;
    let hi_tag_b: u64 = (0xFEED_FACEu64 << 32) | 0x1234_5678u64;

    // 1. Positioned WriteFixed at offset 0 with 64-bit user_data > u32::MAX
    let c_head = engine
        .submit_and_reap(&[IoSubmission::WriteFixed {
            fd: rw_fd,
            buf_index: write_slot,
            offset: 0,
            len: payload_head.len() as u32,
            user_data: hi_tag_a,
        }])
        .expect("WriteFixed head");
    assert_eq!(c_head.len(), 1);
    assert_eq!(c_head[0].user_data, hi_tag_a);
    assert_eq!(c_head[0].result, payload_head.len() as i32);

    // 2. Positioned WriteFixed at non-zero offset 4096
    engine.buffer_pool_mut().buffer_mut(write_slot).unwrap()[..payload_tail.len()]
        .copy_from_slice(payload_tail);
    let c_tail = engine
        .submit_and_reap(&[IoSubmission::WriteFixed {
            fd: rw_fd,
            buf_index: write_slot,
            offset: 4096,
            len: payload_tail.len() as u32,
            user_data: hi_tag_b,
        }])
        .expect("WriteFixed tail");
    assert_eq!(c_tail[0].user_data, hi_tag_b);
    assert_eq!(c_tail[0].result, payload_tail.len() as i32);

    // 3. Positioned ReadFixed back from offset 4096, EOF at offset 65536, and kernel error on write-only fd
    let (eof_slot, _) = engine.buffer_pool_mut().acquire().expect("eof slot");
    let (err_slot, _) = engine.buffer_pool_mut().acquire().expect("err slot");
    let batch = vec![
        IoSubmission::ReadFixed {
            fd: rw_fd,
            buf_index: read_slot,
            offset: 4096,
            len: payload_tail.len() as u32,
            user_data: hi_tag_a ^ 1,
        },
        IoSubmission::ReadFixed {
            fd: rw_fd,
            buf_index: eof_slot,
            offset: 65_536,
            len: 64,
            user_data: hi_tag_a ^ 2,
        },
        IoSubmission::ReadFixed {
            fd: wo_fd,
            buf_index: err_slot,
            offset: 0,
            len: 32,
            user_data: hi_tag_a ^ 3,
        },
    ];
    let mut reaped = engine.submit_and_reap(&batch).expect("ReadFixed batch");
    reaped.sort_by_key(|c| c.user_data);

    let got_tail = reaped
        .iter()
        .find(|c| c.user_data == (hi_tag_a ^ 1))
        .unwrap();
    assert_eq!(got_tail.result, payload_tail.len() as i32);
    assert_eq!(
        engine
            .buffer_pool()
            .get_slice(read_slot, payload_tail.len()),
        payload_tail
    );

    let got_eof = reaped
        .iter()
        .find(|c| c.user_data == (hi_tag_a ^ 2))
        .unwrap();
    assert_eq!(got_eof.result, 0, "ReadFixed past EOF must report 0 bytes");

    let got_err = reaped
        .iter()
        .find(|c| c.user_data == (hi_tag_a ^ 3))
        .unwrap();
    assert!(
        got_err.result < 0,
        "ReadFixed on write-only descriptor must return negative kernel error result, got {}",
        got_err.result
    );

    // Negative fd is rejected at validation gate before kernel submission
    assert!(
        engine
            .submit_and_reap(&[IoSubmission::ReadFixed {
                fd: -1,
                buf_index: read_slot,
                offset: 0,
                len: 16,
                user_data: 99,
            }])
            .is_err()
    );

    engine.buffer_pool_mut().release(write_slot);
    engine.buffer_pool_mut().release(read_slot);
    engine.buffer_pool_mut().release(eof_slot);
    engine.buffer_pool_mut().release(err_slot);
    drop(rw_file);
    drop(wo_file);
    let _ = std::fs::remove_file(&rw_path);
    let _ = std::fs::remove_file(&wo_path);
}
