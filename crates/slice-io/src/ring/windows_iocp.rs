//! Windows NT I/O Completion Port (`IOCP`) Kernel FFI Bindings.

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OverlappedEntry {
    pub lp_completion_key: usize,
    pub lp_overlapped: *mut core::ffi::c_void,
    pub internal: usize,
    pub dw_number_of_bytes_transferred: u32,
}

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

    pub fn GetQueuedCompletionStatusEx(
        completion_port: isize,
        lp_completion_port_entries: *mut OverlappedEntry,
        ul_count: u32,
        ul_num_entries_removed: *mut u32,
        dw_milliseconds: u32,
        f_alertable: i32,
    ) -> i32;

    pub fn CloseHandle(h_object: isize) -> i32;
}

pub const INVALID_HANDLE_VALUE: isize = -1;
