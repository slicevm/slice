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

