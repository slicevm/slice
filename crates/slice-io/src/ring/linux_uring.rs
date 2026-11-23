//! Linux `io_uring` Backend (SQPOLL / Fixed Buffers / Seccomp Fallback).

use super::{CompletionRingEngine, IoCompletion, IoSubmission, RingStatus};
use crate::slab::RegisteredBufferPool;
use anyhow::{Result, ensure};

impl CompletionRingEngine {
    pub(super) fn init_linux(
        clamped_entries: u32,
        try_sqpoll: bool,
        buffer_pool: RegisteredBufferPool,
    ) -> Result<Self> {
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

    pub(super) fn drain_aborted_linux_cqes(&mut self) -> Result<()> {
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

    pub(super) fn try_submit_and_reap_linux(
        &mut self,
        ops: &[IoSubmission],
    ) -> Result<Option<Vec<IoCompletion>>> {
        let Some(ring) = &mut self.linux_ring else {
            return Ok(None);
        };
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
                let entry = match *op {
                    IoSubmission::Nop { user_data } => {
                        io_uring::opcode::Nop::new().build().user_data(user_data)
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
                        if is_reg {
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
                            io_uring::opcode::Read::new(io_uring::types::Fd(fd as i32), ptr, len)
                                .offset(offset)
                                .build()
                                .user_data(user_data)
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
                        if is_reg {
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
                            io_uring::opcode::Write::new(io_uring::types::Fd(fd as i32), ptr, len)
                                .offset(offset)
                                .build()
                                .user_data(user_data)
                        }
                    }
                };
                // SAFETY: `free_sq >= ops.len()` was verified above, and any Read/Write SQE points into
                // a bounds-checked, non-aliased, pinned `RegisteredBufferPool` slab slot.
                unsafe {
                    sq.push(&entry).expect("io_uring SQ pre-checked capacity");
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
        for cqe in ring.completion() {
            completions.push(IoCompletion {
                user_data: cqe.user_data(),
                result: cqe.result(),
                flags: cqe.flags(),
            });
        }
        Ok(Some(completions))
    }

    pub(super) fn close_linux_ring(&mut self) {
        let _ = self.drain_aborted_linux_cqes();
        if let Some(ring) = self.linux_ring.take() {
            if !self.linux_registered_addrs.is_empty() {
                let _ = ring.submitter().unregister_buffers();
            }
            drop(ring);
        }
    }
}
