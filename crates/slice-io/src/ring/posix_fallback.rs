//! Positioned `pread` / `pwrite` POSIX Completion Loop (macOS, BSD, and Linux Seccomp Fallback).

use super::{IoCompletion, IoSubmission};
use crate::slab::RegisteredBufferPool;
use anyhow::Result;

#[inline]
fn signed_posix_io_result(n: isize) -> i32 {
    if n >= 0 {
        n.min(i32::MAX as isize) as i32
    } else {
        -std::io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or(libc::EIO)
            .abs()
    }
}

pub(crate) fn submit_and_reap_posix(
    buffer_pool: &mut RegisteredBufferPool,
    ops: &[IoSubmission],
) -> Result<Vec<IoCompletion>> {
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
                let ptr = buffer_pool.slot_mut_ptr(buf_index as usize);
                // SAFETY: `validate_submission_batch` verified `buf_index` and `io_len <= buf_size`
                // on an exclusive, non-in-flight `RegisteredBufferPool` slab slot.
                let n = unsafe { libc::pread(fd as i32, ptr as *mut libc::c_void, io_len, off) };
                (user_data, signed_posix_io_result(n))
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
                let ptr = buffer_pool.slot_ptr(buf_index as usize);
                // SAFETY: `validate_submission_batch` verified `buf_index` and `io_len <= buf_size`
                // on a pinned, non-aliased `RegisteredBufferPool` slab slot.
                let n = unsafe { libc::pwrite(fd as i32, ptr as *const libc::c_void, io_len, off) };
                (user_data, signed_posix_io_result(n))
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
