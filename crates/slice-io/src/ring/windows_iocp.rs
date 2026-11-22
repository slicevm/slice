//! Windows NT I/O Completion Port (`IOCP`) and Overlapped File I/O Kernel FFI Bindings.

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OverlappedEntry {
    pub lp_completion_key: usize,
    pub lp_overlapped: *mut core::ffi::c_void,
    pub internal: usize,
    pub dw_number_of_bytes_transferred: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Overlapped {
    pub internal: usize,
    pub internal_high: usize,
    pub offset: u32,
    pub offset_high: u32,
    pub h_event: isize,
}

#[link(name = "kernel32")]
unsafe extern "system" {
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

    pub fn GetQueuedCompletionStatusEx(
        completion_port: isize,
        lp_completion_port_entries: *mut OverlappedEntry,
        ul_count: u32,
        ul_num_entries_removed: *mut u32,
        dw_milliseconds: u32,
        f_alertable: i32,
    ) -> i32;

    pub fn ReadFile(
        h_file: isize,
        lp_buffer: *mut u8,
        n_number_of_bytes_to_read: u32,
        lp_number_of_bytes_read: *mut u32,
        lp_overlapped: *mut Overlapped,
    ) -> i32;

    pub fn WriteFile(
        h_file: isize,
        lp_buffer: *const u8,
        n_number_of_bytes_to_write: u32,
        lp_number_of_bytes_written: *mut u32,
        lp_overlapped: *mut Overlapped,
    ) -> i32;

    pub fn GetOverlappedResult(
        h_file: isize,
        lp_overlapped: *mut Overlapped,
        lp_number_of_bytes_transferred: *mut u32,
        b_wait: i32,
    ) -> i32;

    pub fn GetLastError() -> u32;

    pub fn CloseHandle(h_object: isize) -> i32;
}

pub const INVALID_HANDLE_VALUE: isize = -1;
pub const INFINITE: u32 = 0xFFFF_FFFF;
pub const ERROR_HANDLE_EOF: u32 = 38;
pub const ERROR_IO_PENDING: u32 = 997;

#[inline]
pub(crate) fn win32_pread_fixed(handle: isize, ptr: *mut u8, len: u32, offset: u64) -> i32 {
    let io_len = len.min(i32::MAX as u32);
    let mut ov = Overlapped {
        internal: 0,
        internal_high: 0,
        offset: (offset & 0xFFFF_FFFF) as u32,
        offset_high: (offset >> 32) as u32,
        h_event: 0,
    };
    let mut transferred: u32 = 0;
    // SAFETY: `ptr` points to a validated, exclusive `RegisteredBufferPool` slot of at least `len` bytes,
    // and `ov` lives on the stack until synchronous return or `GetOverlappedResult(..., b_wait = 1)` finishes.
    let ok = unsafe { ReadFile(handle, ptr, io_len, &mut transferred, &mut ov) };
    if ok != 0 {
        return transferred.min(i32::MAX as u32) as i32;
    }
    // SAFETY: Queries thread-local Win32 `LastError` immediately after `ReadFile` returned `0`.
    let err = unsafe { GetLastError() };
    if err == ERROR_HANDLE_EOF {
        return 0;
    }
    if err == ERROR_IO_PENDING {
        // SAFETY: `ov` remains pinned on this stack frame until `GetOverlappedResult` waits (`b_wait = 1`).
        let ok2 = unsafe { GetOverlappedResult(handle, &mut ov, &mut transferred, 1) };
        if ok2 != 0 {
            return transferred.min(i32::MAX as u32) as i32;
        }
        // SAFETY: Queries thread-local Win32 `LastError` immediately after `GetOverlappedResult` failed.
        let err2 = unsafe { GetLastError() };
        if err2 == ERROR_HANDLE_EOF {
            return 0;
        }
        return -(err2.clamp(1, i32::MAX as u32) as i32);
    }
    -(err.clamp(1, i32::MAX as u32) as i32)
}

#[inline]
pub(crate) fn win32_pwrite_fixed(handle: isize, ptr: *const u8, len: u32, offset: u64) -> i32 {
    let io_len = len.min(i32::MAX as u32);
    let mut ov = Overlapped {
        internal: 0,
        internal_high: 0,
        offset: (offset & 0xFFFF_FFFF) as u32,
        offset_high: (offset >> 32) as u32,
        h_event: 0,
    };
    let mut transferred: u32 = 0;
    // SAFETY: `ptr` points to a validated `RegisteredBufferPool` slot of at least `len` bytes,
    // and `ov` lives on the stack until synchronous return or `GetOverlappedResult(..., b_wait = 1)` finishes.
    let ok = unsafe { WriteFile(handle, ptr, io_len, &mut transferred, &mut ov) };
    if ok != 0 {
        return transferred.min(i32::MAX as u32) as i32;
    }
    // SAFETY: Queries thread-local Win32 `LastError` immediately after `WriteFile` returned `0`.
    let err = unsafe { GetLastError() };
    if err == ERROR_IO_PENDING {
        // SAFETY: `ov` remains pinned on this stack frame until `GetOverlappedResult` waits (`b_wait = 1`).
        let ok2 = unsafe { GetOverlappedResult(handle, &mut ov, &mut transferred, 1) };
        if ok2 != 0 {
            return transferred.min(i32::MAX as u32) as i32;
        }
        // SAFETY: Queries thread-local Win32 `LastError` immediately after `GetOverlappedResult` failed.
        let err2 = unsafe { GetLastError() };
        return -(err2.clamp(1, i32::MAX as u32) as i32);
    }
    -(err.clamp(1, i32::MAX as u32) as i32)
}
