//! Pre-Registered, Pinned Fixed-Size Slab Buffer Pool (`RegisteredBufferPool`).

use anyhow::{ensure, Result};
use std::pin::Pin;

pub const FIXED_BUFFER_SIZE: usize = 65536; // 64 KB per registered slab slot
pub const FIXED_BUFFER_COUNT: usize = 64;

/// Pre-allocated, pinned fixed buffer pool registered with the kernel completion ring.
///
/// Both the outer slice (`Box<[Pin<Box<[u8]>>]>`) and each individual buffer (`Pin<Box<[u8]>>`)
/// have fixed, immutable capacity and stable heap addresses for the entire lifetime of the pool,
/// eliminating any risk of Use-After-Free if an `io_uring` or IOCP operation references a slab slot.
///
/// Performance (`IO-PERF-01`):
/// - Uses a flat pre-allocated LIFO `free_stack` (`Box<[u16]>`) instead of a FIFO `VecDeque<u16>`,
///   so recently released 64 KB slab buffers are immediately reused while still hot in L1/L2 cache.
/// - Computes `in_use_count()` in `O(1)` (`buffers.len() - free_len`) instead of scanning `in_use`.
pub struct RegisteredBufferPool {
    pub(crate) buffers: Box<[Pin<Box<[u8]>>]>,
    free_stack: Box<[u16]>,
    free_len: usize,
    in_use: Box<[bool]>,
    buf_size: usize,
}

impl RegisteredBufferPool {
    pub fn new(count: usize, buf_size: usize) -> Self {
        // Guard against `u16` slot index wrap-around (`IO-SEC-02`)
        let clamped_count = count.min(u16::MAX as usize);
        let mut buffers = Vec::with_capacity(clamped_count);
        let mut free_stack = Vec::with_capacity(clamped_count);
        for idx in 0..clamped_count {
            buffers.push(Pin::new(vec![0u8; buf_size].into_boxed_slice()));
            // Push in reverse order so initial pops yield 0, 1, 2, ... deterministically
            free_stack.push((clamped_count - 1 - idx) as u16);
        }
        Self {
            buffers: buffers.into_boxed_slice(),
            free_stack: free_stack.into_boxed_slice(),
            free_len: clamped_count,
            in_use: vec![false; clamped_count].into_boxed_slice(),
            buf_size,
        }
    }

    /// Acquires the most recently released (cache-warm) slab slot and returns `(slot_idx, &mut [u8])`.
    #[inline]
    pub fn acquire(&mut self) -> Option<(u16, &mut [u8])> {
        if self.free_len == 0 {
            return None;
        }
        self.free_len -= 1;
        let idx = self.free_stack[self.free_len];
        let slot = idx as usize;
        self.in_use[slot] = true;
        Some((idx, &mut self.buffers[slot][..]))
    }

    /// Acquires a slab slot index without borrowing the buffer slice.
    #[inline]
    pub fn acquire_slot(&mut self) -> Option<u16> {
        if self.free_len == 0 {
            return None;
        }
        self.free_len -= 1;
        let idx = self.free_stack[self.free_len];
        self.in_use[idx as usize] = true;
        Some(idx)
    }

    /// Returns a mutable slice to an active (`in_use`) slab slot.
    #[inline]
    pub fn buffer_mut(&mut self, idx: u16) -> Option<&mut [u8]> {
        let slot = idx as usize;
        if slot < self.buffers.len() && self.in_use[slot] {
            Some(&mut self.buffers[slot][..])
        } else {
            None
        }
    }

    /// Returns the number of currently acquired slab slots in `O(1)` (`IO-PERF-01`).
    #[inline]
    pub fn in_use_count(&self) -> usize {
        self.buffers.len() - self.free_len
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

    /// Releases an active slab slot back onto the cache-warm LIFO free stack.
    #[inline]
    pub fn release(&mut self, idx: u16) {
        let slot = idx as usize;
        if slot < self.in_use.len() && self.in_use[slot] {
            self.in_use[slot] = false;
            self.free_stack[self.free_len] = idx;
            self.free_len += 1;
        }
    }

    /// Zeroizes the first `dirty_len` bytes of an active slab slot and releases it back to the pool (`IO-SEC-01`).
    #[inline]
    pub fn release_zeroed(&mut self, idx: u16, dirty_len: usize) {
        let slot = idx as usize;
        if slot < self.in_use.len() && self.in_use[slot] {
            let n = dirty_len.min(self.buffers[slot].len());
            self.buffers[slot][..n].fill(0);
            self.in_use[slot] = false;
            self.free_stack[self.free_len] = idx;
            self.free_len += 1;
        }
    }

    /// Returns an immutable slice of up to `len` bytes from an active (`in_use`) slab slot (`IO-SEC-01`).
    /// Returns `&[]` if `idx` is out of bounds or not currently acquired.
    #[inline]
    pub fn get_slice(&self, idx: u16, len: usize) -> &[u8] {
        let slot = idx as usize;
        if slot >= self.buffers.len() || !self.in_use[slot] {
            return &[];
        }
        let buf = &self.buffers[slot];
        let clamped_len = len.min(buf.len());
        &buf[..clamped_len]
    }

    /// Validates that `buf_index` is within the pool bounds and `len` does not exceed the slot capacity.
    #[inline]
    pub fn validate_submission_bounds(&self, buf_index: u16, len: u32) -> Result<()> {
        let slot = buf_index as usize;
        ensure!(
            slot < self.buffers.len(),
            "RegisteredBufferPool index {} out of bounds (pool count: {})",
            buf_index,
            self.buffers.len()
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
