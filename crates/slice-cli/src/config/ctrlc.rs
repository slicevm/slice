//! Cross-platform `Ctrl+C` / console termination signal handler.

#[cfg(windows)]
#[allow(unsafe_code)]
mod win_ctrlc {
    // SAFETY: Exact Win32 `kernel32.dll` `SetConsoleCtrlHandler` `system` ABI signature
    // (`BOOL WINAPI SetConsoleCtrlHandler(PHANDLER_ROUTINE, BOOL)`).
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetConsoleCtrlHandler(handler: Option<extern "system" fn(u32) -> i32>, add: i32) -> i32;
    }

    extern "system" fn ctrl_handler(_ctrl_type: u32) -> i32 {
        // Terminate directly without locking Rust `stdout`/`stderr` mutexes on the Win32 control
        // thread so QuickEdit console selection or concurrent main-thread writes never deadlock.
        std::process::exit(0);
    }

    pub fn install() {
        // SAFETY: `ctrl_handler` is a valid `extern "system" fn(u32) -> i32` matching
        // `PHANDLER_ROUTINE`, and `add = 1` (`TRUE`) registers it in the process handler list.
        unsafe {
            let _ = SetConsoleCtrlHandler(Some(ctrl_handler), 1);
        }
    }
}

/// Installs a cross-platform `Ctrl+C` / console termination signal handler so long-running
/// subcommands (`slice serve`, `slice run --watch`, `slice test --watch`) exit cleanly without
/// leaving worker threads blocked on sockets or freezing Windows consoles.
pub(crate) fn install_ctrlc_handler() {
    #[cfg(windows)]
    win_ctrlc::install();
}
