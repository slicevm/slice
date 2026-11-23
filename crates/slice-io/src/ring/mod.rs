//! Cross-Platform Direct Completion Ring Engine (`io_uring` SQPOLL on Linux / `IOCP` on Windows).

#[cfg(target_os = "linux")]
mod linux_uring;
#[cfg(unix)]
mod posix_fallback;
#[cfg(target_os = "windows")]
mod windows_iocp;

use crate::slab::{FIXED_BUFFER_COUNT, FIXED_BUFFER_SIZE, RegisteredBufferPool};
use anyhow::{Result, ensure};

pub const DEFAULT_RING_ENTRIES: u32 = 256;
pub const MAX_RING_ENTRIES: u32 = 4096;

/// Operation submitted to the Direct Completion Ring.
#[derive(Debug, Clone)]
pub enum IoSubmission {
    Nop {
        user_data: u64,
    },
    ReadFixed {
        fd: i64,
        buf_index: u16,
        offset: u64,
        len: u32,
        user_data: u64,
    },
    WriteFixed {
        fd: i64,
        buf_index: u16,
        offset: u64,
        len: u32,
        user_data: u64,
    },
}

/// Completion event harvested from the Completion Queue (CQ).
#[derive(Debug, Clone, Copy)]
pub struct IoCompletion {
    pub user_data: u64,
    pub result: i32,
    pub flags: u32,
}

/// Telemetry & backend metadata for the active completion ring.
#[derive(Debug, Clone)]
pub struct RingStatus {
    pub backend_name: &'static str,
    pub sqpoll_enabled: bool,
    pub registered_buffers: usize,
    pub ring_capacity: u32,
}

#[inline]
fn validate_fd_or_handle(fd: i64, op_name: &'static str) -> Result<()> {
    #[cfg(unix)]
    ensure!(
        fd >= 0 && fd <= i32::MAX as i64,
        "Invalid file descriptor {} in IoSubmission::{}",
        fd,
        op_name
    );
    #[cfg(not(unix))]
    ensure!(
        fd >= 0 && isize::try_from(fd).is_ok(),
        "Invalid handle {} in IoSubmission::{}",
        fd,
        op_name
    );
    Ok(())
}

/// Validates FD/handle bounds, slab slot bounds, and single-batch DMA read/write aliasing.
fn validate_submission_batch(pool: &RegisteredBufferPool, ops: &[IoSubmission]) -> Result<()> {
    let pool_cap = pool.capacity();
    let mut stack_read_mask = [0u64; 4];
    let mut stack_any_mask = [0u64; 4];
    let mut heap_read_mask: Vec<u64>;
    let mut heap_any_mask: Vec<u64>;
    let (read_slots_mask, any_slots_mask): (&mut [u64], &mut [u64]) = if pool_cap <= 256 {
        (&mut stack_read_mask[..], &mut stack_any_mask[..])
    } else {
        let words = pool_cap.div_ceil(64);
        heap_read_mask = vec![0u64; words];
        heap_any_mask = vec![0u64; words];
        (&mut heap_read_mask[..], &mut heap_any_mask[..])
    };

    for op in ops {
        match *op {
            IoSubmission::Nop { .. } => {}
            IoSubmission::ReadFixed {
                fd, buf_index, len, ..
            } => {
                validate_fd_or_handle(fd, "ReadFixed")?;
                pool.validate_submission_bounds(buf_index, len)?;
                let word = (buf_index as usize) >> 6;
                let bit = 1u64 << ((buf_index as usize) & 63);
                ensure!(
                    (any_slots_mask[word] & bit) == 0,
                    "Concurrent ReadFixed DMA aliasing on slab slot {} within the same submission batch",
                    buf_index
                );
                read_slots_mask[word] |= bit;
                any_slots_mask[word] |= bit;
            }
            IoSubmission::WriteFixed {
                fd, buf_index, len, ..
            } => {
                validate_fd_or_handle(fd, "WriteFixed")?;
                pool.validate_submission_bounds(buf_index, len)?;
                let word = (buf_index as usize) >> 6;
                let bit = 1u64 << ((buf_index as usize) & 63);
                ensure!(
                    (read_slots_mask[word] & bit) == 0,
                    "Concurrent WriteFixed + ReadFixed DMA aliasing on slab slot {} within the same submission batch",
                    buf_index
                );
                any_slots_mask[word] |= bit;
            }
        }
    }
    Ok(())
}

/// Cross-platform Direct Completion Ring Engine.
///
/// * On Linux: Uses native kernel `io_uring` (`IORING_SETUP_SQPOLL` when permitted by kernel capabilities,
///   automatically falling back to standard `io_uring` with registered fixed buffers or `pread`/`pwrite`
///   under seccomp).
/// * On Windows: Uses native Windows NT I/O Completion Ports (`CreateIoCompletionPort`,
///   `ReadFile`/`WriteFile` with 64-bit `OVERLAPPED` offsets, `GetQueuedCompletionStatusEx`) via FFI.
/// * On other Unix platforms: Executes positioned `pread`/`pwrite` against the page-aligned slab pool.
///
/// Field drop order & encapsulation invariant (`#162`):
/// `buffer_pool` is private so callers cannot drop or swap kernel-registered `iovec` slabs while the ring
/// is live, and `Drop` explicitly drains/unregisters kernel ring state before `buffer_pool` deallocates.
pub struct CompletionRingEngine {
    #[cfg(target_os = "linux")]
    linux_ring: Option<io_uring::IoUring>,
    #[cfg(target_os = "linux")]
    linux_registered_addrs: Box<[usize]>,
    #[cfg(target_os = "linux")]
    linux_pending_cqes: usize,
    #[cfg(target_os = "windows")]
    iocp_handle: isize,
    pub status: RingStatus,
    buffer_pool: RegisteredBufferPool,
}

impl CompletionRingEngine {
    pub fn new(entries: u32, try_sqpoll: bool) -> Result<Self> {
        Self::with_pool_config(entries, try_sqpoll, FIXED_BUFFER_COUNT, FIXED_BUFFER_SIZE)
    }

    /// Initializes a Direct Completion Ring with custom slab slot count and per-slot byte capacity.
    pub fn with_pool_config(
        entries: u32,
        try_sqpoll: bool,
        buffer_count: usize,
        buffer_size: usize,
    ) -> Result<Self> {
        let clamped_entries = entries.clamp(1, MAX_RING_ENTRIES);
        let buffer_pool = RegisteredBufferPool::new(buffer_count, buffer_size);

        #[cfg(target_os = "linux")]
        {
            Self::init_linux(clamped_entries, try_sqpoll, buffer_pool)
        }

        #[cfg(target_os = "windows")]
        {
            let _ = try_sqpoll;
            windows_iocp::init_windows(clamped_entries, buffer_pool)
        }

        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            let _ = try_sqpoll;
            Ok(Self {
                status: RingStatus {
                    backend_name: "POSIX Direct Completion Ring",
                    sqpoll_enabled: false,
                    registered_buffers: 0,
                    ring_capacity: clamped_entries,
                },
                buffer_pool,
            })
        }
    }

    /// Returns a shared reference to the engine's page-aligned slab buffer pool.
    #[inline]
    pub fn buffer_pool(&self) -> &RegisteredBufferPool {
        &self.buffer_pool
    }

    /// Returns an exclusive reference to the engine's page-aligned slab buffer pool.
    ///
    /// Note: You cannot replace `RegisteredBufferPool` as a whole (`std::mem::replace` is prevented
    /// because the pool is owned internally), preserving kernel `io_uring` `iovec` registration invariants.
    #[inline]
    pub fn buffer_pool_mut(&mut self) -> &mut RegisteredBufferPool {
        &mut self.buffer_pool
    }

    /// Submits a batch of I/O operations into the Submission Queue (SQ) and harvests completions.
    pub fn submit_and_reap(&mut self, ops: &[IoSubmission]) -> Result<Vec<IoCompletion>> {
        if ops.is_empty() {
            return Ok(Vec::new());
        }
        ensure!(
            ops.len() <= self.status.ring_capacity as usize,
            "Submission batch size {} exceeds completion ring capacity {}",
            ops.len(),
            self.status.ring_capacity
        );

        #[cfg(target_os = "linux")]
        if self.linux_pending_cqes > 0 {
            self.drain_aborted_linux_cqes()?;
        }

        validate_submission_batch(&self.buffer_pool, ops)?;

        #[cfg(target_os = "linux")]
        if let Some(completions) = self.try_submit_and_reap_linux(ops)? {
            return Ok(completions);
        }

        #[cfg(target_os = "windows")]
        {
            windows_iocp::submit_and_reap_iocp(self.iocp_handle, &mut self.buffer_pool, ops)
        }

        #[cfg(unix)]
        {
            posix_fallback::submit_and_reap_posix(&mut self.buffer_pool, ops)
        }

        #[cfg(not(any(unix, target_os = "windows")))]
        {
            let _ = ops;
            anyhow::bail!("CompletionRingEngine is not supported on this target OS")
        }
    }
}

impl Drop for CompletionRingEngine {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        self.close_linux_ring();

        #[cfg(target_os = "windows")]
        windows_iocp::close_iocp_port(&mut self.iocp_handle);
    }
}
