//! Cross-Platform Direct Completion Ring Engine (`io_uring` SQPOLL on Linux / `IOCP` on Windows).

#[cfg(target_os = "windows")]
pub mod windows_iocp;

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
            let mut sqpoll_active = false;
            let ring_opt = if try_sqpoll {
                match io_uring::IoUring::builder()
                    .setup_sqpoll(1000)
                    .build(clamped_entries)
                {
                    Ok(ring) => {
                        sqpoll_active = true;
                        Some(ring)
                    }
                    Err(_) => io_uring::IoUring::new(clamped_entries).ok(),
                }
            } else {
                io_uring::IoUring::new(clamped_entries).ok()
            };

            let mut registered_addrs: Box<[usize]> = Box::new([]);
            let slot_cnt = buffer_pool.capacity();
            let slot_len = buffer_pool.buffer_size();
            if let Some(ref ring) = ring_opt
                && slot_cnt > 0
                && slot_len > 0
            {
                let iovecs: Vec<libc::iovec> = (0..slot_cnt)
                    .map(|slot| libc::iovec {
                        iov_base: buffer_pool.slot_ptr(slot) as *mut libc::c_void,
                        iov_len: slot_len,
                    })
                    .collect();
                // SAFETY: Every `iov_base..iov_base+iov_len` lies inside `buffer_pool`'s pinned
                // 4096-byte-aligned slab, which cannot be replaced and outlives `linux_ring`.
                if unsafe { ring.submitter().register_buffers(&iovecs) }.is_ok() {
                    registered_addrs = (0..slot_cnt)
                        .map(|slot| buffer_pool.slot_ptr(slot) as usize)
                        .collect::<Vec<_>>()
                        .into_boxed_slice();
                }
            }

            let has_fixed = !registered_addrs.is_empty();
            let backend_name = if ring_opt.is_some() {
                match (sqpoll_active, has_fixed) {
                    (true, true) => "Linux io_uring (SQPOLL + Fixed Buffers)",
                    (true, false) => "Linux io_uring (SQPOLL)",
                    (false, true) => "Linux io_uring (Direct Ring + Fixed Buffers)",
                    (false, false) => "Linux io_uring (Direct Ring)",
                }
            } else {
                "Linux User-Space Completion Ring (Seccomp Fallback)"
            };

            Ok(Self {
                linux_ring: ring_opt,
                linux_registered_addrs: registered_addrs,
                linux_pending_cqes: 0,
                status: RingStatus {
                    backend_name,
                    sqpoll_enabled: sqpoll_active,
                    registered_buffers: if has_fixed { slot_cnt } else { 0 },
                    ring_capacity: clamped_entries,
                },
                buffer_pool,
            })
        }

        #[cfg(target_os = "windows")]
        {
            let _ = try_sqpoll;
            // SAFETY: Creates an unbound Win32 I/O completion port (`INVALID_HANDLE_VALUE`).
            let handle = unsafe {
                windows_iocp::CreateIoCompletionPort(windows_iocp::INVALID_HANDLE_VALUE, 0, 0, 0)
            };
            if handle == 0 {
                anyhow::bail!("Failed to initialize Windows IOCP completion ring");
            }
            let reg_count = buffer_pool.capacity();
            Ok(Self {
                iocp_handle: handle,
                status: RingStatus {
                    backend_name: "Windows NT IOCP (Completion Port + Registered Slab Buffers)",
                    sqpoll_enabled: false,
                    registered_buffers: reg_count,
                    ring_capacity: clamped_entries,
                },
                buffer_pool,
            })
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

    #[cfg(target_os = "linux")]
    fn drain_aborted_linux_cqes(&mut self) -> Result<()> {
        if self.linux_pending_cqes == 0 {
            return Ok(());
        }
        if let Some(ring) = &mut self.linux_ring {
            while self.linux_pending_cqes > 0 {
                let reaped_now = ring.completion().count();
                self.linux_pending_cqes = self.linux_pending_cqes.saturating_sub(reaped_now);
                if self.linux_pending_cqes == 0 {
                    break;
                }
                match ring.submit_and_wait(self.linux_pending_cqes) {
                    Ok(_) => {}
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(err) => return Err(err.into()),
                }
            }
        } else {
            self.linux_pending_cqes = 0;
        }
        self.buffer_pool.clear_all_in_flight();
        Ok(())
    }

    /// Submits a batch of I/O operations into the Submission Queue (SQ) and harvests completions.
    ///
    /// Security (`IO-SEC-03`, `IO-SEC-04`, `#162`) & Performance (`IO-PERF-02`):
    /// - Fast-paths empty submission slices without entering FFI/syscalls.
    /// - Enforces `ops.len() <= self.status.ring_capacity` to prevent Submission Queue overflow hangs.
    /// - Rejects negative file descriptors/handles (`fd < 0`, plus `fd > i32::MAX` on Unix).
    /// - Rejects concurrent `ReadFixed` aliasing on the same `buf_index` within a single batch using
    ///   a stack-allocated 256-slot bitmask (`[u64; 4]`), spilling to heap only if `capacity() > 256`.
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

        let pool_cap = self.buffer_pool.capacity();
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
                    #[cfg(unix)]
                    ensure!(
                        fd >= 0 && fd <= i32::MAX as i64,
                        "Invalid file descriptor {} in IoSubmission::ReadFixed",
                        fd
                    );
                    #[cfg(not(unix))]
                    ensure!(
                        fd >= 0 && isize::try_from(fd).is_ok(),
                        "Invalid handle {} in IoSubmission::ReadFixed",
                        fd
                    );
                    self.buffer_pool
                        .validate_submission_bounds(buf_index, len)?;
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
                    #[cfg(unix)]
                    ensure!(
                        fd >= 0 && fd <= i32::MAX as i64,
                        "Invalid file descriptor {} in IoSubmission::WriteFixed",
                        fd
                    );
                    #[cfg(not(unix))]
                    ensure!(
                        fd >= 0 && isize::try_from(fd).is_ok(),
                        "Invalid handle {} in IoSubmission::WriteFixed",
                        fd
                    );
                    self.buffer_pool
                        .validate_submission_bounds(buf_index, len)?;
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

        #[cfg(target_os = "linux")]
        {
            if let Some(ring) = &mut self.linux_ring {
                {
                    let mut sq = ring.submission();
                    let free_sq = sq.capacity().saturating_sub(sq.len());
                    ensure!(
                        free_sq >= ops.len(),
                        "io_uring submission queue has {} free slots for {} operations",
                        free_sq,
                        ops.len()
                    );
                    for op in ops {
                        match *op {
                            IoSubmission::Nop { user_data } => {
                                let entry =
                                    io_uring::opcode::Nop::new().build().user_data(user_data);
                                // SAFETY: `entry` is a pure NOP SQE and `free_sq >= ops.len()` was verified.
                                unsafe {
                                    sq.push(&entry).expect("io_uring SQ pre-checked capacity");
                                }
                            }
                            IoSubmission::ReadFixed {
                                fd,
                                buf_index,
                                offset,
                                len,
                                user_data,
                            } => {
                                let slot = buf_index as usize;
                                let ptr = self.buffer_pool.slot_mut_ptr(slot);
                                let is_reg = self
                                    .linux_registered_addrs
                                    .get(slot)
                                    .is_some_and(|&addr| addr == ptr as usize);
                                let entry = if is_reg {
                                    io_uring::opcode::ReadFixed::new(
                                        io_uring::types::Fd(fd as i32),
                                        ptr,
                                        len,
                                        buf_index,
                                    )
                                    .offset(offset)
                                    .build()
                                    .user_data(user_data)
                                } else {
                                    io_uring::opcode::Read::new(
                                        io_uring::types::Fd(fd as i32),
                                        ptr,
                                        len,
                                    )
                                    .offset(offset)
                                    .build()
                                    .user_data(user_data)
                                };
                                // SAFETY: `ptr..ptr+len` is a validated, non-aliased pinned slab slot.
                                unsafe {
                                    sq.push(&entry).expect("io_uring SQ pre-checked capacity");
                                }
                            }
                            IoSubmission::WriteFixed {
                                fd,
                                buf_index,
                                offset,
                                len,
                                user_data,
                            } => {
                                let slot = buf_index as usize;
                                let ptr = self.buffer_pool.slot_ptr(slot);
                                let is_reg = self
                                    .linux_registered_addrs
                                    .get(slot)
                                    .is_some_and(|&addr| addr == ptr as usize);
                                let entry = if is_reg {
                                    io_uring::opcode::WriteFixed::new(
                                        io_uring::types::Fd(fd as i32),
                                        ptr,
                                        len,
                                        buf_index,
                                    )
                                    .offset(offset)
                                    .build()
                                    .user_data(user_data)
                                } else {
                                    io_uring::opcode::Write::new(
                                        io_uring::types::Fd(fd as i32),
                                        ptr,
                                        len,
                                    )
                                    .offset(offset)
                                    .build()
                                    .user_data(user_data)
                                };
                                // SAFETY: `ptr..ptr+len` is a validated, pinned slab slot.
                                unsafe {
                                    sq.push(&entry).expect("io_uring SQ pre-checked capacity");
                                }
                            }
                        }
                    }
                }
                loop {
                    let remaining = ops.len().saturating_sub(ring.completion().len());
                    if remaining == 0 {
                        break;
                    }
                    match ring.submit_and_wait(remaining) {
                        Ok(_) => break,
                        Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(err) => {
                            let done = ring.completion().count();
                            let unharvested = ops.len().saturating_sub(done);
                            if unharvested > 0 {
                                self.linux_pending_cqes =
                                    self.linux_pending_cqes.saturating_add(unharvested);
                                for op in ops {
                                    match *op {
                                        IoSubmission::ReadFixed { buf_index, .. }
                                        | IoSubmission::WriteFixed { buf_index, .. } => {
                                            self.buffer_pool.mark_slot_in_flight(buf_index);
                                        }
                                        IoSubmission::Nop { .. } => {}
                                    }
                                }
                            }
                            return Err(err.into());
                        }
                    }
                }
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
            let mut stack_meta = [(0u64, 0i32); 256];
            let mut heap_meta: Vec<(u64, i32)>;
            let meta: &mut [(u64, i32)] = if ops.len() <= stack_meta.len() {
                &mut stack_meta[..ops.len()]
            } else {
                heap_meta = vec![(0u64, 0i32); ops.len()];
                &mut heap_meta[..]
            };

            let mut posted_count = 0usize;
            let mut post_err: Option<std::io::Error> = None;
            for (op_idx, op) in ops.iter().enumerate() {
                let (user_data, signed_res) = match *op {
                    IoSubmission::Nop { user_data } => (user_data, 0i32),
                    IoSubmission::ReadFixed {
                        fd,
                        buf_index,
                        offset,
                        len,
                        user_data,
                    } => {
                        let ptr = self.buffer_pool.slot_mut_ptr(buf_index as usize);
                        let res = windows_iocp::win32_pread_fixed(fd as isize, ptr, len, offset);
                        (user_data, res)
                    }
                    IoSubmission::WriteFixed {
                        fd,
                        buf_index,
                        offset,
                        len,
                        user_data,
                    } => {
                        let ptr = self.buffer_pool.slot_ptr(buf_index as usize);
                        let res = windows_iocp::win32_pwrite_fixed(fd as isize, ptr, len, offset);
                        (user_data, res)
                    }
                };
                meta[op_idx] = (user_data, signed_res);
                let transferred = signed_res.max(0) as u32;
                // SAFETY: `self.iocp_handle` is a valid completion port and `op_idx` indexes `meta`.
                let posted = unsafe {
                    windows_iocp::PostQueuedCompletionStatus(
                        self.iocp_handle,
                        transferred,
                        op_idx,
                        core::ptr::null_mut(),
                    )
                };
                if posted == 0 {
                    post_err = Some(std::io::Error::last_os_error());
                    break;
                }
                posted_count += 1;
            }

            let mut batch_entries = [windows_iocp::OverlappedEntry {
                lp_completion_key: 0,
                lp_overlapped: core::ptr::null_mut(),
                internal: 0,
                dw_number_of_bytes_transferred: 0,
            }; 64];

            if let Some(err) = post_err {
                // Drain all already-posted packets before returning so zero packets remain stranded.
                let mut drained = 0usize;
                while drained < posted_count {
                    let want = (posted_count - drained).min(batch_entries.len()) as u32;
                    let mut removed: u32 = 0;
                    // SAFETY: Reaps up to `want` packets into stack-allocated `batch_entries`.
                    let ok = unsafe {
                        windows_iocp::GetQueuedCompletionStatusEx(
                            self.iocp_handle,
                            batch_entries.as_mut_ptr(),
                            want,
                            &mut removed,
                            windows_iocp::INFINITE,
                            0,
                        )
                    };
                    if ok == 0 {
                        break;
                    }
                    drained += removed as usize;
                }
                anyhow::bail!("Windows IOCP PostQueuedCompletionStatus failed: {}", err);
            }

            let mut completions = Vec::with_capacity(posted_count);
            while completions.len() < posted_count {
                let want = (posted_count - completions.len()).min(batch_entries.len()) as u32;
                let mut removed: u32 = 0;
                // SAFETY: Reaps exact `posted_count` queued packets into stack-allocated `batch_entries`.
                let ok = unsafe {
                    windows_iocp::GetQueuedCompletionStatusEx(
                        self.iocp_handle,
                        batch_entries.as_mut_ptr(),
                        want,
                        &mut removed,
                        windows_iocp::INFINITE,
                        0,
                    )
                };
                ensure!(
                    ok != 0 && removed > 0,
                    "Windows IOCP GetQueuedCompletionStatusEx failed: {}",
                    std::io::Error::last_os_error()
                );
                for entry in &batch_entries[..(removed as usize).min(batch_entries.len())] {
                    let (user_data, result) =
                        meta.get(entry.lp_completion_key).copied().unwrap_or((
                            entry.lp_completion_key as u64,
                            entry.dw_number_of_bytes_transferred.min(i32::MAX as u32) as i32,
                        ));
                    completions.push(IoCompletion {
                        user_data,
                        result,
                        flags: 0,
                    });
                }
            }
            Ok(completions)
        }

        #[cfg(unix)]
        {
            let mut completions = Vec::with_capacity(ops.len());
            for op in ops {
                let (user_data, res) = match *op {
                    IoSubmission::Nop { user_data } => (user_data, 0i32),
                    IoSubmission::ReadFixed {
                        fd,
                        buf_index,
                        offset,
                        len,
                        user_data,
                    } => {
                        let Ok(off) = libc::off_t::try_from(offset) else {
                            completions.push(IoCompletion {
                                user_data,
                                result: -libc::EINVAL,
                                flags: 0,
                            });
                            continue;
                        };
                        let io_len = (len as usize).min(i32::MAX as usize);
                        let ptr = self.buffer_pool.slot_mut_ptr(buf_index as usize);
                        // SAFETY: `ptr..ptr+io_len` is a validated exclusive `RegisteredBufferPool` slot.
                        let n = unsafe {
                            libc::pread(fd as i32, ptr as *mut libc::c_void, io_len, off)
                        };
                        let r = if n >= 0 {
                            n.min(i32::MAX as isize) as i32
                        } else {
                            -std::io::Error::last_os_error()
                                .raw_os_error()
                                .unwrap_or(libc::EIO)
                                .abs()
                        };
                        (user_data, r)
                    }
                    IoSubmission::WriteFixed {
                        fd,
                        buf_index,
                        offset,
                        len,
                        user_data,
                    } => {
                        let Ok(off) = libc::off_t::try_from(offset) else {
                            completions.push(IoCompletion {
                                user_data,
                                result: -libc::EINVAL,
                                flags: 0,
                            });
                            continue;
                        };
                        let io_len = (len as usize).min(i32::MAX as usize);
                        let ptr = self.buffer_pool.slot_ptr(buf_index as usize);
                        // SAFETY: `ptr..ptr+io_len` is a validated `RegisteredBufferPool` slot.
                        let n = unsafe {
                            libc::pwrite(fd as i32, ptr as *const libc::c_void, io_len, off)
                        };
                        let r = if n >= 0 {
                            n.min(i32::MAX as isize) as i32
                        } else {
                            -std::io::Error::last_os_error()
                                .raw_os_error()
                                .unwrap_or(libc::EIO)
                                .abs()
                        };
                        (user_data, r)
                    }
                };
                completions.push(IoCompletion {
                    user_data,
                    result: res,
                    flags: 0,
                });
            }
            Ok(completions)
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
        {
            let _ = self.drain_aborted_linux_cqes();
            if let Some(ring) = self.linux_ring.take() {
                if !self.linux_registered_addrs.is_empty() {
                    let _ = ring.submitter().unregister_buffers();
                }
                drop(ring);
            }
        }
        #[cfg(target_os = "windows")]
        {
            if self.iocp_handle != 0 && self.iocp_handle != windows_iocp::INVALID_HANDLE_VALUE {
                // SAFETY: `self.iocp_handle` was created via `CreateIoCompletionPort` and is closed once.
                unsafe {
                    windows_iocp::CloseHandle(self.iocp_handle);
                }
                self.iocp_handle = 0;
            }
        }
    }
}
