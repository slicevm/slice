use crate::*;

#[test]
fn test_registered_buffer_pool_lifecycle() {
    let mut pool = RegisteredBufferPool::new(2, 1024);
    let (idx0, buf0) = pool.acquire().unwrap();
    assert_eq!(idx0, 0);
    buf0[..5].copy_from_slice(b"slice");
    assert_eq!(pool.get_slice(idx0, 5), b"slice");

    let (idx1, _) = pool.acquire().unwrap();
    assert_eq!(idx1, 1);
    assert!(pool.acquire().is_none());

    pool.release(idx0);
    // Duplicate release of an already-free slot must be a no-op
    pool.release(idx0);
    let (idx_reused, _) = pool.acquire().unwrap();
    assert_eq!(idx_reused, 0);
    assert!(pool.acquire().is_none());
}

#[test]
fn test_page_alignment_and_in_flight_dma_quarantine() {
    let mut pool = RegisteredBufferPool::new(4, 1024);
    for idx in 0..4u16 {
        let addr = pool.slot_addr(idx).unwrap();
        assert_eq!(
            addr % PAGE_ALIGNMENT,
            0,
            "Slot {} base address {:#x} must be 4096-byte page aligned",
            idx,
            addr
        );
    }

    let (slot0, buf0) = pool.acquire().unwrap();
    buf0[..4].copy_from_slice(b"RING");
    pool.mark_slot_in_flight(slot0);
    assert!(pool.is_slot_in_flight(slot0));
    assert!(pool.buffer_mut(slot0).is_none());
    assert_eq!(pool.get_slice(slot0, 4), &[]);
    assert!(pool.validate_submission_bounds(slot0, 4).is_err());

    // Premature release while kernel DMA is in flight must be ignored
    pool.release(slot0);
    pool.release_zeroed(slot0, 4);
    assert!(pool.is_slot_in_use(slot0));

    // Once completion reaping clears `in_flight`, safe access and release resume cleanly
    pool.clear_all_in_flight();
    assert!(!pool.is_slot_in_flight(slot0));
    assert_eq!(pool.get_slice(slot0, 4), b"RING");
    pool.release_zeroed(slot0, 4);
    assert!(!pool.is_slot_in_use(slot0));
}
