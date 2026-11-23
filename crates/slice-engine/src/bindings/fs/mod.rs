//! Modularized OCap-Enforced Virtual Filesystem (`VfsState`) and `node:fs` Host Bindings.
//!
//! Submodules:
//! - [`state`]: Type-tagged [`VfsEntryKind`], [`InodeId`], [`OpenFd`], [`VfsState`], and [`VfsSnapshot`].
//! - [`symlink_resolver`]: Unified VFS + host component-by-component symlink resolver with per-hop OCap guards.
//! - [`classify_and_cow`]: Canonical path classification, CoW host directory materialization, and `ELOOP` helpers.
//! - [`path_rw_ops`]: Path-based read, write, append, and copy bindings.
//! - [`path_dir_stat_ops`]: Directory listing/creation, `stat`/`lstat`, `mtime`, and `mkdtemp` bindings.
//! - [`path_tree_ops`]: CoW `renameSync` and `rmSync` subtree bindings.
//! - [`path_link_meta_ops`]: `realpathSync`, `symlinkSync`, `readlinkSync`, `truncateSync`, `utimesSync`, `chmodSync`.
//! - [`fd_open_close`]: Open flags decoding and `openSync` / `closeSync` / `fdPathSync` bindings.
//! - [`fd_io_and_stat`]: Descriptor-based `readSync`, `writeSync`, `fstatSync`, `ftruncateSync`, `futimesSync`.

pub(crate) mod classify_and_cow;
pub(crate) mod fd_io_and_stat;
pub(crate) mod fd_open_close;
pub(crate) mod path_dir_stat_ops;
pub(crate) mod path_link_meta_ops;
pub(crate) mod path_rw_ops;
pub(crate) mod path_tree_ops;
pub(crate) mod state;
pub(crate) mod symlink_resolver;

pub(crate) use self::classify_and_cow::{is_vfs_tombstoned, resolve_vfs_or_host_script};
pub(crate) use self::state::{VfsSnapshot, VfsState};
pub(crate) use self::symlink_resolver::{
    SymlinkResolveError, is_host_unc_blocked, resolve_effective_vfs_path,
};

pub(crate) fn register_fs_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::js_string;
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_fs_read_file_sync"), ctx)
        .unwrap_or(false)
    {
        return;
    }
    fd_open_close::register_fs_fd_open_close_host_functions(ctx);
    fd_io_and_stat::register_fs_fd_io_and_stat_host_functions(ctx);
    path_rw_ops::register_fs_path_rw_host_functions(ctx);
    path_dir_stat_ops::register_fs_path_dir_stat_host_functions(ctx);
    path_tree_ops::register_fs_path_tree_host_functions(ctx);
    path_link_meta_ops::register_fs_path_link_meta_host_functions(ctx);
}
