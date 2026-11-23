//! Windows NT I/O Completion Port (`IOCP`) and Overlapped File I/O Kernel FFI Backend.

use super::{CompletionRingEngine, IoCompletion, IoSubmission, RingStatus};
use crate::slab::RegisteredBufferPool;
use anyhow::Result;

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct OverlappedEntry {
    pub(crate) lp_completion_key: usize,
    pub(crate) lp_overlapped: *mut core::ffi::c_void,
    pub(crate) internal: usize,
    pub(crate) dw_number_of_bytes_transferred: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Overlapped {
    pub(crate) internal: usize,
    pub(crate) internal_high: usize,
    pub(crate) offset: u32,
    pub(crate) offset_high: u32,
    pub(crate) h_event: isize,
}

// SAFETY: Exact Win32 `kernel32.dll` `system` ABI signatures matching the Windows SDK.
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateIoCompletionPort(file: isize, port: isize, key: usize, threads: u32) -> isize;
    fn PostQueuedCompletionStatus(
        port: isize,
        bytes: u32,
        key: usize,
        ov: *mut core::ffi::c_void,
    ) -> i32;
    fn GetQueuedCompletionStatusEx(
        port: isize,
        entries: *mut OverlappedEntry,
        count: u32,
        removed: *mut u32,
        ms: u32,
        alertable: i32,
    ) -> i32;
    fn ReadFile(h: isize, buf: *mut u8, len: u32, read: *mut u32, ov: *mut Overlapped) -> i32;
    fn WriteFile(h: isize, buf: *const u8, len: u32, n: *mut u32, ov: *mut Overlapped) -> i32;
    fn GetOverlappedResult(h: isize, ov: *mut Overlapped, n: *mut u32, wait: i32) -> i32;
    fn CloseHandle(handle: isize) -> i32;
}

const INVALID_HANDLE_VALUE: isize = -1;
const INFINITE: u32 = 0xFFFF_FFFF;
const ERROR_HANDLE_EOF: u32 = 38;
const ERROR_IO_PENDING: u32 = 997;

#[inline]
fn last_win32_error_code() -> u32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(1) as u32
}

#[inline]
fn make_overlapped(offset: u64) -> Overlapped {
    Overlapped {
        internal: 0,
        internal_high: 0,
        offset: offset as u32,
        offset_high: (offset >> 32) as u32,
        h_event: 0,
    }
}

/// Completes a synchronous or `ERROR_IO_PENDING` Win32 `OVERLAPPED` I/O call (`ReadFile`/`WriteFile`).
///
/// # Safety
/// `ov` and any associated I/O buffer must remain valid for `handle` until this function returns.
#[inline]
unsafe fn finish_win32_overlapped(
    handle: isize,
    ov: &mut Overlapped,
    ok: i32,
    mut transferred: u32,
    eof_is_zero: bool,
) -> i32 {
    if ok != 0 {
        return transferred.min(i32::MAX as u32) as i32;
    }
    let mut err = last_win32_error_code();
    if err == ERROR_IO_PENDING {
        // SAFETY: `ov` and the caller's slab buffer stay live while `GetOverlappedResult` waits (`wait = 1`).
        if unsafe { GetOverlappedResult(handle, ov, &mut transferred, 1) } != 0 {
            return transferred.min(i32::MAX as u32) as i32;
        }
        err = last_win32_error_code();
    }
    if eof_is_zero && err == ERROR_HANDLE_EOF {
        0
    } else {
        -(err.clamp(1, i32::MAX as u32) as i32)
    }
}

/// Positioned Win32 read into a pinned slab buffer at `offset`.
///
/// # Safety
/// `ptr` must be valid for exclusive writes of `len` bytes for the duration of this call.
#[inline]
pub(crate) unsafe fn win32_pread_fixed(handle: isize, ptr: *mut u8, len: u32, offset: u64) -> i32 {
    let io_len = len.min(i32::MAX as u32);
    let mut ov = make_overlapped(offset);
    let mut xfer: u32 = 0;
    // SAFETY: Caller guarantees `ptr..ptr+io_len` is valid for exclusive writes while `ov` is live.
    let ok = unsafe { ReadFile(handle, ptr, io_len, &mut xfer, &mut ov) };
    // SAFETY: `ov` and `ptr` remain valid across `GetOverlappedResult(..., wait = 1)`.
    unsafe { finish_win32_overlapped(handle, &mut ov, ok, xfer, true) }
}

/// Positioned Win32 write from a pinned slab buffer at `offset`.
///
/// # Safety
/// `ptr` must be valid for shared reads of `len` bytes for the duration of this call.
#[inline]
pub(crate) unsafe fn win32_pwrite_fixed(
    handle: isize,
    ptr: *const u8,
    len: u32,
    offset: u64,
) -> i32 {
    let io_len = len.min(i32::MAX as u32);
    let mut ov = make_overlapped(offset);
    let mut xfer: u32 = 0;
    // SAFETY: Caller guarantees `ptr..ptr+io_len` is valid for reads while `ov` is live.
    let ok = unsafe { WriteFile(handle, ptr, io_len, &mut xfer, &mut ov) };
    // SAFETY: `ov` and `ptr` remain valid across `GetOverlappedResult(..., wait = 1)`.
    unsafe { finish_win32_overlapped(handle, &mut ov, ok, xfer, false) }
}

pub(crate) fn init_windows(
    clamped_entries: u32,
    buffer_pool: RegisteredBufferPool,
) -> Result<CompletionRingEngine> {
    // SAFETY: Creates an unbound Win32 I/O completion port (`INVALID_HANDLE_VALUE`).
    let handle = unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, 0, 0, 0) };
    if handle == 0 {
        anyhow::bail!("Failed to initialize Windows IOCP completion ring");
    }
    let registered_buffers = buffer_pool.capacity();
    Ok(CompletionRingEngine {
        iocp_handle: handle,
        status: RingStatus {
            backend_name: "Windows NT IOCP (Completion Port + Registered Slab Buffers)",
            sqpoll_enabled: false,
            registered_buffers,
            ring_capacity: clamped_entries,
        },
        buffer_pool,
    })
}

pub(crate) fn close_iocp_port(iocp_handle: &mut isize) {
    if *iocp_handle != 0 && *iocp_handle != INVALID_HANDLE_VALUE {
        // SAFETY: `*iocp_handle` was created via `CreateIoCompletionPort` and is closed once here.
        unsafe {
            CloseHandle(*iocp_handle);
        }
        *iocp_handle = 0;
    }
}

#[inline]
fn reap_iocp_batch(
    iocp_handle: isize,
    entries: &mut [OverlappedEntry; 64],
    want: usize,
) -> Option<usize> {
    let count = want.min(entries.len()) as u32;
    let mut removed: u32 = 0;
    // SAFETY: `iocp_handle` is a live completion port and `entries` holds at least `count` slots.
    let ok = unsafe {
        GetQueuedCompletionStatusEx(
            iocp_handle,
            entries.as_mut_ptr(),
            count,
            &mut removed,
            INFINITE,
            0,
        )
    };
    (ok != 0 && removed > 0).then(|| (removed as usize).min(entries.len()))
}

pub(crate) fn submit_and_reap_iocp(
    iocp_handle: isize,
    buffer_pool: &mut RegisteredBufferPool,
    ops: &[IoSubmission],
) -> Result<Vec<IoCompletion>> {
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
        let (user_data, res) = match *op {
            IoSubmission::Nop { user_data } => (user_data, 0i32),
            IoSubmission::ReadFixed {
                fd,
                buf_index,
                offset,
                len,
                user_data,
            } => {
                let ptr = buffer_pool.slot_mut_ptr(buf_index as usize);
                // SAFETY: `validate_submission_batch` verified `buf_index` and `len <= buf_size` with no aliasing.
                (user_data, unsafe {
                    win32_pread_fixed(fd as isize, ptr, len, offset)
                })
            }
            IoSubmission::WriteFixed {
                fd,
                buf_index,
                offset,
                len,
                user_data,
            } => {
                let ptr = buffer_pool.slot_ptr(buf_index as usize);
                // SAFETY: `validate_submission_batch` verified `buf_index` and `len <= buf_size` with no aliasing.
                (user_data, unsafe {
                    win32_pwrite_fixed(fd as isize, ptr, len, offset)
                })
            }
        };
        meta[op_idx] = (user_data, res);
        let xfer = res.max(0) as u32;
        // SAFETY: `iocp_handle` is a valid completion port and `op_idx` indexes `meta`.
        if unsafe { PostQueuedCompletionStatus(iocp_handle, xfer, op_idx, core::ptr::null_mut()) }
            == 0
        {
            post_err = Some(std::io::Error::last_os_error());
            break;
        }
        posted_count += 1;
    }

    let mut batch = [OverlappedEntry {
        lp_completion_key: 0,
        lp_overlapped: core::ptr::null_mut(),
        internal: 0,
        dw_number_of_bytes_transferred: 0,
    }; 64];

    if let Some(err) = post_err {
        let mut drained = 0usize;
        while drained < posted_count {
            let Some(n) = reap_iocp_batch(iocp_handle, &mut batch, posted_count - drained) else {
                break;
            };
            drained += n;
        }
        anyhow::bail!("Windows IOCP PostQueuedCompletionStatus failed: {}", err);
    }

    let mut completions = Vec::with_capacity(posted_count);
    while completions.len() < posted_count {
        let want = posted_count - completions.len();
        let Some(n) = reap_iocp_batch(iocp_handle, &mut batch, want) else {
            anyhow::bail!(
                "Windows IOCP GetQueuedCompletionStatusEx failed: {}",
                std::io::Error::last_os_error()
            );
        };
        for entry in &batch[..n] {
            let fallback = entry.dw_number_of_bytes_transferred.min(i32::MAX as u32) as i32;
            let (user_data, result) = meta
                .get(entry.lp_completion_key)
                .copied()
                .unwrap_or((entry.lp_completion_key as u64, fallback));
            completions.push(IoCompletion {
                user_data,
                result,
                flags: 0,
            });
        }
    }
    Ok(completions)
}
