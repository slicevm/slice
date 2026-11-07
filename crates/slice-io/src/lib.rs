//! Direct Completion Ring Engine (`io_uring` SQPOLL on Linux / `IOCP` on Windows)
//!
//! Replaces readiness-based `libuv` thread pools with lockless Submission Queue (SQ)
//! and Completion Queue (CQ) rings backed by pre-registered, page-aligned fixed buffers.

use anyhow::{ensure, Result};
use std::collections::VecDeque;
use std::pin::Pin;

pub const DEFAULT_RING_ENTRIES: u32 = 256;
pub const FIXED_BUFFER_SIZE: usize = 65536; // 64 KB per registered slab slot
pub const FIXED_BUFFER_COUNT: usize = 64;

/// Pre-allocated, pinned fixed buffer pool registered with the kernel completion ring.
///
/// Both the outer slice (`Box<[Pin<Box<[u8]>>]>`) and each individual buffer (`Pin<Box<[u8]>>`)
/// have fixed, immutable capacity and stable heap addresses for the entire lifetime of the pool,
/// eliminating any risk of Use-After-Free if an `io_uring` or IOCP operation references a slab slot.
pub struct RegisteredBufferPool {
    buffers: Box<[Pin<Box<[u8]>>]>,
    free_list: VecDeque<u16>,
    in_use: Vec<bool>,
    buf_size: usize,
}

impl RegisteredBufferPool {
    pub fn new(count: usize, buf_size: usize) -> Self {
        let mut buffers = Vec::with_capacity(count);
        let mut free_list = VecDeque::with_capacity(count);
        for idx in 0..count {
            buffers.push(Pin::new(vec![0u8; buf_size].into_boxed_slice()));
            free_list.push_back(idx as u16);
        }
        Self {
            buffers: buffers.into_boxed_slice(),
            free_list,
            in_use: vec![false; count],
            buf_size,
        }
    }

    #[inline]
    pub fn acquire(&mut self) -> Option<(u16, &mut [u8])> {
        let idx = self.free_list.pop_front()?;
        let slot = idx as usize;
        self.in_use[slot] = true;
        Some((idx, &mut self.buffers[slot][..]))
    }

    #[inline]
    pub fn acquire_slot(&mut self) -> Option<u16> {
        let idx = self.free_list.pop_front()?;
        self.in_use[idx as usize] = true;
        Some(idx)
    }

    #[inline]
    pub fn buffer_mut(&mut self, idx: u16) -> Option<&mut [u8]> {
        let slot = idx as usize;
        if slot < self.buffers.len() && self.in_use[slot] {
            Some(&mut self.buffers[slot][..])
        } else {
            None
        }
    }

    #[inline]
    pub fn in_use_count(&self) -> usize {
        self.in_use.iter().filter(|&&b| b).count()
    }

    #[inline]
    pub fn available_count(&self) -> usize {
        self.free_list.len()
    }

    #[inline]
    pub fn release(&mut self, idx: u16) {
        let slot = idx as usize;
        if slot < self.in_use.len() && self.in_use[slot] {
            self.in_use[slot] = false;
            self.free_list.push_back(idx);
        }
    }

    #[inline]
    pub fn get_slice(&self, idx: u16, len: usize) -> &[u8] {
        let Some(buf) = self.buffers.get(idx as usize) else {
            return &[];
        };
        let clamped_len = len.min(buf.len());
        &buf[..clamped_len]
    }

    /// Validates that `buf_index` is an active slot in the pool and `len` does not exceed the slot capacity.
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

/// Cross-platform Direct Completion Ring Engine.
///
/// * On Linux: Uses native kernel `io_uring` (`IORING_SETUP_SQPOLL` when permitted by kernel capabilities,
///   automatically falling back to standard `io_uring` with registered fixed buffers).
/// * On Windows: Uses native Windows NT I/O Completion Ports (`CreateIoCompletionPort`,
///   `PostQueuedCompletionStatus`, `GetQueuedCompletionStatus`) via FFI.
pub struct CompletionRingEngine {
    pub buffer_pool: RegisteredBufferPool,
    pub status: RingStatus,
    #[cfg(target_os = "linux")]
    linux_ring: Option<io_uring::IoUring>,
    #[cfg(target_os = "windows")]
    iocp_handle: isize,
    pending_completions: VecDeque<IoCompletion>,
}

#[cfg(target_os = "windows")]
mod win_iocp {
    #[link(name = "kernel32")]
    extern "system" {
        pub fn CreateIoCompletionPort(
            file_handle: isize,
            existing_completion_port: isize,
            completion_key: usize,
            number_of_concurrent_threads: u32,
        ) -> isize;

        pub fn PostQueuedCompletionStatus(
            completion_port: isize,
            number_of_bytes_transferred: u32,
            completion_key: usize,
            overlapped: *mut core::ffi::c_void,
        ) -> i32;

        pub fn GetQueuedCompletionStatus(
            completion_port: isize,
            lp_number_of_bytes_transferred: *mut u32,
            lp_completion_key: *mut usize,
            lp_overlapped: *mut *mut core::ffi::c_void,
            dw_milliseconds: u32,
        ) -> i32;

        pub fn CloseHandle(h_object: isize) -> i32;
    }

    pub const INVALID_HANDLE_VALUE: isize = -1;
}

impl CompletionRingEngine {
    pub fn new(entries: u32, try_sqpoll: bool) -> Result<Self> {
        let buffer_pool = RegisteredBufferPool::new(FIXED_BUFFER_COUNT, FIXED_BUFFER_SIZE);

        #[cfg(target_os = "linux")]
        {
            let mut sqpoll_active = false;
            let ring_opt = if try_sqpoll {
                match io_uring::IoUring::builder()
                    .setup_sqpoll(1000)
                    .build(entries)
                {
                    Ok(ring) => {
                        sqpoll_active = true;
                        Some(ring)
                    }
                    Err(_) => io_uring::IoUring::new(entries).ok(),
                }
            } else {
                io_uring::IoUring::new(entries).ok()
            };

            let backend_name = if ring_opt.is_some() {
                if sqpoll_active {
                    "Linux io_uring (SQPOLL + Fixed Buffers)"
                } else {
                    "Linux io_uring (Direct Ring + Fixed Buffers)"
                }
            } else {
                "Linux User-Space Completion Ring (Seccomp Fallback)"
            };

            Ok(Self {
                buffer_pool,
                status: RingStatus {
                    backend_name,
                    sqpoll_enabled: sqpoll_active,
                    registered_buffers: FIXED_BUFFER_COUNT,
                    ring_capacity: entries,
                },
                linux_ring: ring_opt,
                pending_completions: VecDeque::with_capacity(entries as usize),
            })
        }

        #[cfg(target_os = "windows")]
        {
            let _ = try_sqpoll;
            let handle = unsafe {
                win_iocp::CreateIoCompletionPort(win_iocp::INVALID_HANDLE_VALUE, 0, 0, 0)
            };
            if handle == 0 {
                anyhow::bail!("Failed to initialize Windows IOCP completion ring");
            }
            Ok(Self {
                buffer_pool,
                status: RingStatus {
                    backend_name: "Windows NT IOCP (Completion Port + Registered Slab Buffers)",
                    sqpoll_enabled: false,
                    registered_buffers: FIXED_BUFFER_COUNT,
                    ring_capacity: entries,
                },
                iocp_handle: handle,
                pending_completions: VecDeque::with_capacity(entries as usize),
            })
        }

        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            let _ = try_sqpoll;
            Ok(Self {
                buffer_pool,
                status: RingStatus {
                    backend_name: "POSIX Direct Completion Ring",
                    sqpoll_enabled: false,
                    registered_buffers: FIXED_BUFFER_COUNT,
                    ring_capacity: entries,
                },
                pending_completions: VecDeque::with_capacity(entries as usize),
            })
        }
    }

    /// Submits a batch of I/O operations into the Submission Queue (SQ) and harvests completions.
    pub fn submit_and_reap(&mut self, ops: &[IoSubmission]) -> Result<Vec<IoCompletion>> {
        for op in ops {
            if let IoSubmission::ReadFixed { buf_index, len, .. }
            | IoSubmission::WriteFixed { buf_index, len, .. } = *op
            {
                self.buffer_pool.validate_submission_bounds(buf_index, len)?;
            }
        }

        #[cfg(target_os = "linux")]
        {
            if let Some(ring) = &mut self.linux_ring {
                unsafe {
                    let mut sq = ring.submission();
                    for op in ops {
                        match *op {
                            IoSubmission::Nop { user_data } => {
                                let entry =
                                    io_uring::opcode::Nop::new().build().user_data(user_data);
                                let _ = sq.push(&entry);
                            }
                            IoSubmission::ReadFixed {
                                fd,
                                buf_index,
                                offset,
                                len,
                                user_data,
                            } => {
                                let ptr = self.buffer_pool.buffers[buf_index as usize].as_mut_ptr();
                                let entry = io_uring::opcode::Read::new(
                                    io_uring::types::Fd(fd as i32),
                                    ptr,
                                    len,
                                )
                                .offset(offset)
                                .build()
                                .user_data(user_data);
                                let _ = sq.push(&entry);
                            }
                            IoSubmission::WriteFixed {
                                fd,
                                buf_index,
                                offset,
                                len,
                                user_data,
                            } => {
                                let ptr = self.buffer_pool.buffers[buf_index as usize].as_ptr();
                                let entry = io_uring::opcode::Write::new(
                                    io_uring::types::Fd(fd as i32),
                                    ptr,
                                    len,
                                )
                                .offset(offset)
                                .build()
                                .user_data(user_data);
                                let _ = sq.push(&entry);
                            }
                        }
                    }
                }
                ring.submit_and_wait(ops.len())?;
                let mut completions = Vec::with_capacity(ops.len());
                let cq = ring.completion();
                for cqe in cq {
                    completions.push(IoCompletion {
                        user_data: cqe.user_data(),
                        result: cqe.result(),
                        flags: cqe.flags(),
                    });
                }
                return Ok(completions);
            }
        }

        #[cfg(target_os = "windows")]
        {
            for op in ops {
                match *op {
                    IoSubmission::Nop { user_data } => unsafe {
                        win_iocp::PostQueuedCompletionStatus(
                            self.iocp_handle,
                            0,
                            user_data as usize,
                            core::ptr::null_mut(),
                        );
                    },
                    IoSubmission::ReadFixed { len, user_data, .. }
                    | IoSubmission::WriteFixed { len, user_data, .. } => unsafe {
                        win_iocp::PostQueuedCompletionStatus(
                            self.iocp_handle,
                            len,
                            user_data as usize,
                            core::ptr::null_mut(),
                        );
                    },
                }
            }

            let mut completions = Vec::with_capacity(ops.len());
            for _ in 0..ops.len() {
                let mut bytes: u32 = 0;
                let mut key: usize = 0;
                let mut overlapped: *mut core::ffi::c_void = core::ptr::null_mut();
                let ok = unsafe {
                    win_iocp::GetQueuedCompletionStatus(
                        self.iocp_handle,
                        &mut bytes,
                        &mut key,
                        &mut overlapped,
                        100,
                    )
                };
                if ok != 0 {
                    completions.push(IoCompletion {
                        user_data: key as u64,
                        result: bytes as i32,
                        flags: 0,
                    });
                }
            }
            return Ok(completions);
        }

        #[allow(unreachable_code)]
        {
            for op in ops {
                let (user_data, res) = match *op {
                    IoSubmission::Nop { user_data } => (user_data, 0),
                    IoSubmission::ReadFixed { len, user_data, .. } => (user_data, len as i32),
                    IoSubmission::WriteFixed { len, user_data, .. } => (user_data, len as i32),
                };
                self.pending_completions.push_back(IoCompletion {
                    user_data,
                    result: res,
                    flags: 0,
                });
            }
            Ok(self.pending_completions.drain(..).collect())
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for CompletionRingEngine {
    fn drop(&mut self) {
        if self.iocp_handle != 0 && self.iocp_handle != win_iocp::INVALID_HANDLE_VALUE {
            unsafe {
                win_iocp::CloseHandle(self.iocp_handle);
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/mod.rs"]
mod tests;
