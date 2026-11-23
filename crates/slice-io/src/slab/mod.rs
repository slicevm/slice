//! Pre-Registered, Page-Aligned Fixed-Size Slab Buffer Pool (`RegisteredBufferPool`).

use anyhow::{Result, ensure};
use std::alloc::{Layout, alloc_zeroed, dealloc, handle_alloc_error};
use std::ptr::NonNull;

pub const FIXED_BUFFER_SIZE: usize = 65536; // 64 KB per registered slab slot
pub const FIXED_BUFFER_COUNT: usize = 64;
pub const PAGE_ALIGNMENT: usize = 4096; // 4 KB hardware page alignment for O_DIRECT & io_uring iovecs

/// Contiguous 4096-byte page-aligned slab backing all registered slots in a [`RegisteredBufferPool`].
struct PageAlignedSlab {
    ptr: NonNull<u8>,
    layout: Option<Layout>,
}

// SAFETY: `PageAlignedSlab` exclusively owns its heap allocation and exposes disjoint slot slices
// only through `&RegisteredBufferPool` / `&mut RegisteredBufferPool` borrow rules.
unsafe impl Send for PageAlignedSlab {}
// SAFETY: Shared references (`&PageAlignedSlab`) only permit read-only slice borrows on non-in-flight slots.
unsafe impl Sync for PageAlignedSlab {}

impl PageAlignedSlab {
    fn new(slot_count: usize, stride: usize) -> Self {
        if slot_count == 0 || stride == 0 {
            return Self {
                ptr: NonNull::dangling(),
                layout: None,
            };
        }
        let total_bytes = slot_count
            .checked_mul(stride)
            .expect("RegisteredBufferPool slab byte size overflow");
        let layout = Layout::from_size_align(total_bytes, PAGE_ALIGNMENT)
            .expect("Invalid 4096-byte page-aligned layout for RegisteredBufferPool");
        // SAFETY: `total_bytes > 0` because both `slot_count > 0` and `stride > 0` were verified above,
        // and `PAGE_ALIGNMENT` (4096) is a valid power-of-two alignment.
        let raw = unsafe { alloc_zeroed(layout) };
        let Some(ptr) = NonNull::new(raw) else {
            handle_alloc_error(layout);
        };
        Self {
            ptr,
            layout: Some(layout),
        }
    }
}

impl Drop for PageAlignedSlab {
    fn drop(&mut self) {
        if let Some(layout) = self.layout.take() {
            // SAFETY: `self.ptr` was allocated with `alloc_zeroed(layout)` in `PageAlignedSlab::new`
            // and is freed exactly once when `PageAlignedSlab` drops (after any owning kernel ring closes).
            unsafe {
                dealloc(self.ptr.as_ptr(), layout);
            }
        }
    }
}

/// Pre-allocated, `4096`-byte page-aligned fixed buffer pool registered with the kernel completion ring.
///
/// Backed by a single contiguous `4096`-byte aligned slab (`PageAlignedSlab`) so every slot begins on an
/// exact `4 KB` hardware page boundary with immutable capacity and stable virtual addresses for the
/// lifetime of the pool. Uses a LIFO `free_stack` (`Box<[u16]>`) for cache-warm reuse and tracks
/// `in_flight` kernel DMA slots to prevent aliasing safe Rust slice borrows.
pub struct RegisteredBufferPool {
    slab: PageAlignedSlab,
    slot_count: usize,
    stride: usize,
    free_stack: Box<[u16]>,
    free_len: usize,
    in_use: Box<[bool]>,
    in_flight: Box<[bool]>,
    buf_size: usize,
}

impl RegisteredBufferPool {
    pub fn new(count: usize, buf_size: usize) -> Self {
        // Guard against `u16` slot index wrap-around (`IO-SEC-02`)
        let clamped_count = count.min(u16::MAX as usize);
        let stride = if buf_size == 0 {
            0
        } else {
            buf_size
                .checked_next_multiple_of(PAGE_ALIGNMENT)
                .expect("RegisteredBufferPool slot stride overflow")
        };
        let slab = PageAlignedSlab::new(clamped_count, stride);
        let mut free_stack = Vec::with_capacity(clamped_count);
        for idx in 0..clamped_count {
            // Push in reverse order so initial pops yield 0, 1, 2, ... deterministically
            free_stack.push((clamped_count - 1 - idx) as u16);
        }
        Self {
            slab,
            slot_count: clamped_count,
            stride,
            free_stack: free_stack.into_boxed_slice(),
            free_len: clamped_count,
            in_use: vec![false; clamped_count].into_boxed_slice(),
            in_flight: vec![false; clamped_count].into_boxed_slice(),
            buf_size,
        }
    }

    #[inline]
    pub(crate) fn slot_ptr(&self, slot: usize) -> *const u8 {
        assert!(slot < self.slot_count);
        if self.stride == 0 {
            return self.slab.ptr.as_ptr();
        }
        // SAFETY: `assert!(slot < self.slot_count)` guarantees `slot * self.stride` is within the
        // `slot_count * stride` byte allocation owned by `self.slab`.
        unsafe { self.slab.ptr.as_ptr().add(slot * self.stride) }
    }

    #[inline]
    pub(crate) fn slot_mut_ptr(&mut self, slot: usize) -> *mut u8 {
        assert!(slot < self.slot_count);
        if self.stride == 0 {
            return self.slab.ptr.as_ptr();
        }
        // SAFETY: `assert!(slot < self.slot_count)` and `&mut self` guarantee exclusive in-bounds
        // pointer arithmetic within `self.slab`'s `slot_count * stride` allocation.
        unsafe { self.slab.ptr.as_ptr().add(slot * self.stride) }
    }

    #[cfg(any(target_os = "linux", test))]
    #[inline]
    pub(crate) fn mark_slot_in_flight(&mut self, idx: u16) {
        let slot = idx as usize;
        if slot < self.slot_count {
            self.in_flight[slot] = true;
            self.in_use[slot] = true;
        }
    }

    #[cfg(any(target_os = "linux", test))]
    #[inline]
    pub(crate) fn clear_all_in_flight(&mut self) {
        self.in_flight.fill(false);
    }

    /// Returns `true` if `idx` currently has an unharvested kernel DMA operation in flight.
    #[inline]
    pub fn is_slot_in_flight(&self, idx: u16) -> bool {
        let slot = idx as usize;
        slot < self.in_flight.len() && self.in_flight[slot]
    }

    /// Returns the immutable virtual base address of slot `idx` (always `4096`-byte aligned when `buf_size > 0`),
    /// or `None` if `idx` is out of bounds.
    #[inline]
    pub fn slot_addr(&self, idx: u16) -> Option<usize> {
        let slot = idx as usize;
        if slot < self.slot_count {
            Some(self.slot_ptr(slot) as usize)
        } else {
            None
        }
    }

    /// Acquires the most recently released (cache-warm) slab slot and returns `(slot_idx, &mut [u8])`.
    #[inline(always)]
    pub fn acquire(&mut self) -> Option<(u16, &mut [u8])> {
        while self.free_len > 0 {
            self.free_len -= 1;
            let idx = self.free_stack[self.free_len];
            let slot = idx as usize;
            if slot >= self.slot_count || self.in_flight[slot] {
                continue;
            }
            self.in_use[slot] = true;
            // SAFETY: `slot < self.slot_count` and `!self.in_flight[slot]` were verified above,
            // `self.buf_size <= self.stride` (`slot * stride + buf_size <= slab capacity`), and
            // `&mut self` holds exclusive access to the slab for the returned borrow lifetime.
            let slice = unsafe {
                let ptr = if self.stride == 0 {
                    self.slab.ptr.as_ptr()
                } else {
                    self.slab.ptr.as_ptr().add(slot * self.stride)
                };
                core::slice::from_raw_parts_mut(ptr, self.buf_size)
            };
            return Some((idx, slice));
        }
        None
    }

    /// Acquires a slab slot index without borrowing the buffer slice.
    #[inline]
    pub fn acquire_slot(&mut self) -> Option<u16> {
        while self.free_len > 0 {
            self.free_len -= 1;
            let idx = self.free_stack[self.free_len];
            let slot = idx as usize;
            if slot >= self.slot_count || self.in_flight[slot] {
                continue;
            }
            self.in_use[slot] = true;
            return Some(idx);
        }
        None
    }

    /// Returns a mutable slice to an active (`in_use` and not `in_flight`) slab slot.
    #[inline]
    pub fn buffer_mut(&mut self, idx: u16) -> Option<&mut [u8]> {
        let slot = idx as usize;
        if slot < self.slot_count && self.in_use[slot] && !self.in_flight[slot] {
            let ptr = self.slot_mut_ptr(slot);
            // SAFETY: `slot < self.slot_count`, `!self.in_flight[slot]`, `buf_size <= stride`,
            // and `&mut self` holds exclusive access to the slab for the returned borrow lifetime.
            Some(unsafe { core::slice::from_raw_parts_mut(ptr, self.buf_size) })
        } else {
            None
        }
    }

    /// Returns the total number of registered slab slots in `O(1)`.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.slot_count
    }

    /// Returns the byte length of each fixed slab buffer in `O(1)`.
    #[inline]
    pub fn buffer_size(&self) -> usize {
        self.buf_size
    }

    /// Returns the number of currently acquired slab slots in `O(1)` (`IO-PERF-01`).
    #[inline]
    pub fn in_use_count(&self) -> usize {
        self.slot_count - self.free_len
    }

    /// Returns the number of available slab slots in `O(1)`.
    #[inline]
    pub fn available_count(&self) -> usize {
        self.free_len
    }

    /// Returns `true` if `idx` is a valid slot currently marked `in_use`.
    #[inline]
    pub fn is_slot_in_use(&self, idx: u16) -> bool {
        let slot = idx as usize;
        slot < self.in_use.len() && self.in_use[slot]
    }

    /// Releases an active slab slot back onto the cache-warm LIFO free stack (no-op if `in_flight` or free).
    #[inline(always)]
    pub fn release(&mut self, idx: u16) {
        let slot = idx as usize;
        if slot < self.slot_count && self.in_use[slot] && !self.in_flight[slot] {
            self.in_use[slot] = false;
            self.free_stack[self.free_len] = idx;
            self.free_len += 1;
        }
    }

    /// Zeroizes the first `dirty_len` bytes of an active slab slot and releases it back to the pool (`IO-SEC-01`).
    #[inline]
    pub fn release_zeroed(&mut self, idx: u16, dirty_len: usize) {
        if let Some(buf) = self.buffer_mut(idx) {
            let n = dirty_len.min(buf.len());
            let dirty = &mut buf[..n];
            dirty.fill(0);
            std::hint::black_box(dirty);
        }
        self.release(idx);
    }

    /// Returns an immutable slice of up to `len` bytes from an active (`in_use`, non-`in_flight`) slab slot (`IO-SEC-01`).
    /// Returns `&[]` if `idx` is out of bounds, not currently acquired, or has unharvested kernel DMA in flight.
    #[inline]
    pub fn get_slice(&self, idx: u16, len: usize) -> &[u8] {
        let slot = idx as usize;
        if slot >= self.slot_count || !self.in_use[slot] || self.in_flight[slot] {
            return &[];
        }
        let clamped_len = len.min(self.buf_size);
        let ptr = self.slot_ptr(slot);
        // SAFETY: `slot < self.slot_count`, `!self.in_flight[slot]`, `clamped_len <= self.buf_size <= self.stride`,
        // and `&self` prevents concurrent mutable borrows.
        unsafe { core::slice::from_raw_parts(ptr, clamped_len) }
    }

    /// Validates that `buf_index` is within pool bounds, not stranded in flight, and `len <= buf_size`.
    #[inline]
    pub fn validate_submission_bounds(&self, buf_index: u16, len: u32) -> Result<()> {
        let slot = buf_index as usize;
        ensure!(
            slot < self.slot_count,
            "RegisteredBufferPool index {} out of bounds (pool count: {})",
            buf_index,
            self.slot_count
        );
        ensure!(
            !self.in_flight[slot],
            "RegisteredBufferPool slot {} has unharvested kernel DMA in flight",
            buf_index
        );
        ensure!(
            (len as usize) <= self.buf_size,
            "IoSubmission length {} exceeds fixed slab buffer size {}",
            len,
            self.buf_size
        );
        Ok(())
    }
}
