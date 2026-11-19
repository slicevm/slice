//! Unified VFS + Host Component-by-Component Symlink Resolver (`RESOLVE_BENEATH` OCap Hop Guard).
//!
//! Resolves both in-memory [`VfsEntryKind::Symlink`] entries and host-filesystem symbolic links /
//! junctions in a single forward scan while enforcing per-hop OCap read containment, `ELOOP` loop
//! bounds (`MAX_SYMLINK_HOPS = 40`), and zero OS syscalls on pure-compute / `.cas` workloads.

use crate::bindings::fs::state::{PathEntry, VfsEntryKind, is_vfs_root_path};
use crate::boa_realm::{
    BOA_VFS_STATE, has_any_boa_fs_read_ocap, has_any_boa_fs_write_ocap, has_boa_fs_read_ocap,
    has_boa_fs_write_ocap, norm_vfs_path,
};
use std::path::Path;

/// Maximum symlink expansions permitted during a single path resolution (POSIX `SYMLOOP_MAX`).
pub(crate) const MAX_SYMLINK_HOPS: usize = 40;

/// Result of resolving a guest path through [`VfsState`] and permitted host symlinks.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedVfsPath {
    pub(crate) norm: String,
    pub(crate) traversed_symlink: bool,
    pub(crate) has_file_ancestor: bool,
}

/// Failure modes produced during symlink traversal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SymlinkResolveError {
    /// Exceeded [`MAX_SYMLINK_HOPS`] expansions (`ELOOP`).
    Loop,
    /// A symlink target escaped the compartment's `--allow-read` capability boundary.
    ReadOcapDenied(String),
}

/// Blocks raw forward-slash Windows SMB UNC paths (`//server/share`) from reaching host `std::fs`
/// on Windows while keeping POSIX/in-memory VFS root paths (`//file.txt` -> `/file.txt`) intact.
#[inline]
pub(crate) fn is_host_unc_blocked(raw_path: &str) -> bool {
    #[cfg(windows)]
    {
        let tb = raw_path.trim_start().as_bytes();
        if tb.len() >= 2 && tb[0] == b'/' && tb[1] == b'/' {
            let is_ext_drive = tb.len() >= 7
                && tb[2] == b'?'
                && (tb[3] == b'/' || tb[3] == b'\\')
                && tb[4].is_ascii_alphabetic()
                && tb[5] == b':'
                && (tb[6] == b'/' || tb[6] == b'\\');
            if !is_ext_drive && tb.get(2) != Some(&b'/') && tb[2..].contains(&b'/') {
                return true;
            }
        }
    }
    let _ = raw_path;
    false
}

/// Anchors `raw` into a canonical forward-slash VFS key, resolving relative paths against the
/// cached workspace directory (`cached_normalized_cwd()`) with zero OS syscalls.
#[inline]
pub(crate) fn anchor_vfs_path(raw: &str) -> String {
    slice_sandbox::anchor_vfs_path_to_cwd(raw)
}

/// Returns `true` when `meta` (obtained via `std::fs::symlink_metadata`) represents a symbolic
/// link or, on Windows, an NTFS directory junction / mount reparse point (`0x0400`).
#[cfg(windows)]
#[inline]
pub(crate) fn is_host_symlink_or_reparse(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    meta.file_type().is_symlink() || (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0
}

/// Returns `true` when `meta` (obtained via `std::fs::symlink_metadata`) represents a symbolic link.
#[cfg(not(windows))]
#[inline]
pub(crate) fn is_host_symlink_or_reparse(meta: &std::fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

/// Normalizes a `std::fs::read_link` target path to forward slashes and strips Windows verbatim
/// drive prefixes (`\\?\C:\` or `\??\C:\`) returned by NTFS junctions and symlinks.
#[inline]
pub(crate) fn normalize_host_readlink_target(target: &Path) -> String {
    let s = target.to_string_lossy().replace('\\', "/");
    let b = s.as_bytes();
    if b.len() >= 6
        && b[0] == b'/'
        && (b[1] == b'/' || b[1] == b'?')
        && b[2] == b'?'
        && b[3] == b'/'
        && b[4].is_ascii_alphabetic()
        && b[5] == b':'
    {
        return s[4..].to_string();
    }
    if let Some(rest) = s.strip_prefix("//?/UNC/") {
        return format!("//{}", rest);
    }
    s
}

/// Resolves a raw symlink target string (`raw_target`) relative to the directory containing
/// `norm_link`.
#[inline]
pub(crate) fn resolve_symlink_target_against_link(norm_link: &str, raw_target: &str) -> String {
    let trimmed = raw_target.trim();
    let tb = trimmed.as_bytes();
    let is_abs = trimmed.starts_with('/')
        || trimmed.starts_with('\\')
        || (tb.len() >= 2 && tb[0].is_ascii_alphabetic() && tb[1] == b':');
    if is_abs {
        return anchor_vfs_path(trimmed);
    }
    let parent = match norm_link.rfind('/') {
        Some(0) => "/",
        Some(idx) => &norm_link[..idx],
        None => slice_sandbox::cached_normalized_cwd(),
    };
    if parent == "/" {
        norm_vfs_path(&format!("/{}", trimmed))
    } else if parent.ends_with('/') {
        anchor_vfs_path(&format!("{}{}", parent, trimmed))
    } else {
        anchor_vfs_path(&format!("{}/{}", parent, trimmed))
    }
}

/// Resolves `raw_path` component-by-component across both [`VfsState`] and permitted host
/// symlinks.
///
/// Security & Performance Invariants:
/// 1. **Zero-Syscall Fast Path**: When neither `--allow-read` nor `--allow-write` is granted (or
///    `allow_host` is `false`), `can_query_host` is `false` and **zero** host `symlink_metadata` /
///    `read_link` syscalls are ever issued.
/// 2. **Per-Hop OCap Guard (`RESOLVE_BENEATH` Semantics)**: Every expanded VFS or host symlink
///    validates `has_boa_fs_read_ocap(&resolved_prefix)` immediately before splicing remaining
///    path segments, preventing intermediate hop bounce probes outside `--allow-read`.
/// 3. **Opaque & Ancestor Syscall Pruning**: Ancestors strictly above configured capability
///    prefixes, `/tmp` root, `Tombstone` entries, `Directory { host_opaque: true }` subtrees, and
///    missing host parent directories suppress redundant host `lstat` syscalls.
pub(crate) fn resolve_effective_vfs_path(
    raw_path: &str,
    follow_final: bool,
    allow_host: bool,
) -> Result<ResolvedVfsPath, SymlinkResolveError> {
    let mut current = anchor_vfs_path(raw_path);
    let can_query_host = allow_host && (has_any_boa_fs_read_ocap() || has_any_boa_fs_write_ocap());
    if !can_query_host && BOA_VFS_STATE.with(|v| v.borrow().paths.is_empty()) {
        return Ok(ResolvedVfsPath {
            norm: current,
            traversed_symlink: false,
            has_file_ancestor: false,
        });
    }

    let mut traversed_symlink = false;
    let mut hops = 0usize;

    loop {
        enum WalkStep {
            Done { has_file_ancestor: bool },
            Expanded(String),
        }

        let step: Result<WalkStep, SymlinkResolveError> = BOA_VFS_STATE.with(|v| {
            let vfs = v.borrow();
            let bytes = current.as_bytes();
            let root_end = if bytes.len() >= 3 && bytes[1] == b':' && bytes[2] == b'/' {
                3
            } else if bytes.first() == Some(&b'/') {
                1
            } else {
                0
            };

            let mut scan = root_end;
            let mut host_masked_in_walk = false;

            while scan <= bytes.len() {
                let seg_end = match memchr::memchr(b'/', &bytes[scan..]) {
                    Some(rel) => scan + rel,
                    None => bytes.len(),
                };
                let is_final = seg_end == bytes.len();
                if seg_end > 0 {
                    let prefix = &current[..seg_end];
                    match vfs.paths.get(prefix) {
                        Some(PathEntry::Tombstone) => {
                            host_masked_in_walk = true;
                        }
                        Some(PathEntry::Present(id)) => {
                            if let Some(ino) = vfs.get_inode(*id) {
                                match &ino.kind {
                                    VfsEntryKind::Symlink(raw_target) => {
                                        if !is_final || follow_final {
                                            if raw_target.as_bytes().contains(&0)
                                                || is_host_unc_blocked(raw_target)
                                            {
                                                return Err(SymlinkResolveError::ReadOcapDenied(
                                                    raw_target.clone(),
                                                ));
                                            }
                                            let resolved_prefix =
                                                resolve_symlink_target_against_link(
                                                    prefix, raw_target,
                                                );
                                            if !has_boa_fs_read_ocap(&resolved_prefix) {
                                                return Err(SymlinkResolveError::ReadOcapDenied(
                                                    resolved_prefix,
                                                ));
                                            }
                                            let next_full = if is_final {
                                                resolved_prefix
                                            } else {
                                                anchor_vfs_path(&format!(
                                                    "{}{}",
                                                    resolved_prefix,
                                                    &current[seg_end..]
                                                ))
                                            };
                                            return Ok(WalkStep::Expanded(next_full));
                                        }
                                    }
                                    VfsEntryKind::Directory { host_opaque } => {
                                        if *host_opaque {
                                            host_masked_in_walk = true;
                                        }
                                    }
                                    VfsEntryKind::File(_) => {
                                        if !is_final {
                                            return Ok(WalkStep::Done {
                                                has_file_ancestor: true,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                        None => {
                            if can_query_host
                                && !host_masked_in_walk
                                && prefix != "/tmp"
                                && !is_vfs_root_path(prefix)
                                && (has_boa_fs_read_ocap(prefix) || has_boa_fs_write_ocap(prefix))
                            {
                                match std::fs::symlink_metadata(prefix) {
                                    Ok(meta) if is_host_symlink_or_reparse(&meta) => {
                                        if !is_final || follow_final {
                                            let Ok(target_path) = std::fs::read_link(prefix) else {
                                                return Err(SymlinkResolveError::ReadOcapDenied(
                                                    prefix.to_string(),
                                                ));
                                            };
                                            let raw_target =
                                                normalize_host_readlink_target(&target_path);
                                            if raw_target.as_bytes().contains(&0)
                                                || is_host_unc_blocked(&raw_target)
                                            {
                                                return Err(SymlinkResolveError::ReadOcapDenied(
                                                    raw_target,
                                                ));
                                            }
                                            let resolved_prefix =
                                                resolve_symlink_target_against_link(
                                                    prefix,
                                                    &raw_target,
                                                );
                                            if !has_boa_fs_read_ocap(&resolved_prefix) {
                                                return Err(SymlinkResolveError::ReadOcapDenied(
                                                    resolved_prefix,
                                                ));
                                            }
                                            let next_full = if is_final {
                                                resolved_prefix
                                            } else {
                                                anchor_vfs_path(&format!(
                                                    "{}{}",
                                                    resolved_prefix,
                                                    &current[seg_end..]
                                                ))
                                            };
                                            return Ok(WalkStep::Expanded(next_full));
                                        }
                                    }
                                    Ok(meta) => {
                                        if !is_final && !meta.is_dir() {
                                            return Ok(WalkStep::Done {
                                                has_file_ancestor: true,
                                            });
                                        }
                                    }
                                    Err(_) => {
                                        host_masked_in_walk = true;
                                    }
                                }
                            }
                        }
                    }
                }
                if is_final {
                    break;
                }
                scan = seg_end + 1;
            }

            Ok(WalkStep::Done {
                has_file_ancestor: false,
            })
        });

        match step? {
            WalkStep::Done { has_file_ancestor } => {
                return Ok(ResolvedVfsPath {
                    norm: current,
                    traversed_symlink,
                    has_file_ancestor,
                });
            }
            WalkStep::Expanded(next) => {
                hops += 1;
                if hops > MAX_SYMLINK_HOPS {
                    return Err(SymlinkResolveError::Loop);
                }
                current = next;
                traversed_symlink = true;
            }
        }
    }
}
