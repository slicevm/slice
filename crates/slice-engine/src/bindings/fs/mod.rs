//! Modularized OCap-Enforced Virtual Filesystem (`VfsState`) and `node:fs` Host Bindings.
//!
//! Submodules:
//! - [`state`]: Type-tagged [`VfsEntryKind`], [`InodeId`], [`OpenFd`], [`VfsState`], and [`VfsSnapshot`].
//! - [`path_ops`]: Canonical CWD path anchoring, symlink loop resolution, CoW host fallback, and path-based bindings.
//! - [`fd_ops`]: Inode-backed open file descriptor bindings (`openSync`, `closeSync`, `readSync`, `writeSync`, `fstatSync`, `ftruncateSync`).

pub(crate) mod fd_ops;
pub(crate) mod path_ops;
pub(crate) mod state;

pub(crate) use self::path_ops::{
    is_host_unc_blocked, is_vfs_tombstoned, read_vfs_file_bytes_resolved,
};
pub(crate) use self::state::{VfsSnapshot, VfsState};

pub(crate) fn register_fs_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::js_string;
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_fs_read_file_sync"), ctx)
        .unwrap_or(false)
    {
        return;
    }
    fd_ops::register_fs_fd_host_functions(ctx);
    path_ops::register_fs_path_host_functions(ctx);
}
