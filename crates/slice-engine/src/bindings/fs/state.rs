//! Inode-Backed Virtual Filesystem State (`VfsState`), Open File Descriptor Table (`OpenFd`),
//! and Copy-on-Write Checkpoint / Worker Snapshot Primitives.

use rustc_hash::FxHashMap;
use std::sync::Arc;

/// Stable 32-bit inode identifier inside [`VfsState`].
///
/// Decouples directory entries (`paths: FxHashMap<String, PathEntry>`) from open file descriptors
/// (`fds: Vec<Option<OpenFd>>`), guaranteeing POSIX inode identity across `renameSync`, atomic
/// path replacement, and `unlinkSync` while a descriptor remains open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct InodeId(pub u32);

/// Explicitly type-tagged VFS inode payload.
///
/// Eliminates in-band magic byte prefixes so arbitrary binary regular files (including files
/// starting with `\0__SLICE_SYMLINK__:`) can never be confused with symbolic links.
#[derive(Clone, Debug)]
pub(crate) enum VfsEntryKind {
    File(Arc<Vec<u8>>),
    Directory { host_opaque: bool },
    Symlink(String),
}

impl VfsEntryKind {
    #[inline]
    pub(crate) fn accounted_bytes(&self) -> usize {
        match self {
            Self::File(bytes) => bytes.len(),
            Self::Directory { .. } => 0,
            Self::Symlink(target) => target.len(),
        }
    }
}

/// Reference-counted in-memory VFS inode.
#[derive(Clone, Debug)]
pub(crate) struct VfsInode {
    pub(crate) kind: VfsEntryKind,
    pub(crate) mtime_ms: Option<f64>,
    pub(crate) link_count: u32,
    pub(crate) open_fd_count: u32,
    pub(crate) accounted_bytes: usize,
}

/// Single-source-of-truth path table entry enforcing strict mutual exclusion between regular
/// files, directories, symlinks, and copy-on-write deletion tombstones at any canonical path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PathEntry {
    Present(InodeId),
    Tombstone,
}

/// Rust-managed open file descriptor bound to an [`InodeId`].
#[derive(Clone, Debug)]
pub(crate) struct OpenFd {
    pub(crate) inode: InodeId,
    pub(crate) position: u64,
    pub(crate) readable: bool,
    pub(crate) writable: bool,
    pub(crate) append: bool,
    pub(crate) opened_path: String,
}

/// Immutable O(1)-payload-cloning snapshot of [`VfsState`] for `Slice.checkpoint()` /
/// `Slice.restore()`, `Slice.evalSandbox`, and `Worker` initialization.
#[derive(Clone, Debug, Default)]
pub(crate) struct VfsSnapshot {
    pub(crate) inodes: Vec<Option<VfsInode>>,
    pub(crate) free_inodes: Vec<u32>,
    pub(crate) paths: FxHashMap<String, PathEntry>,
    pub(crate) fds: Vec<Option<OpenFd>>,
    pub(crate) free_fds: Vec<u32>,
    pub(crate) open_fd_total: u32,
    pub(crate) total_accounted_bytes: usize,
}

/// Unified per-thread VFS state replacing disjoint file/dir/tombstone/mtime maps.
#[derive(Debug, Default)]
pub(crate) struct VfsState {
    pub(crate) inodes: Vec<Option<VfsInode>>,
    pub(crate) free_inodes: Vec<u32>,
    pub(crate) paths: FxHashMap<String, PathEntry>,
    pub(crate) fds: Vec<Option<OpenFd>>,
    pub(crate) free_fds: Vec<u32>,
    pub(crate) open_fd_total: u32,
    pub(crate) total_accounted_bytes: usize,
    pub(crate) has_active_watchers: bool,
}

#[inline]
pub(crate) fn is_vfs_root_path(norm: &str) -> bool {
    if norm.is_empty() || norm == "/" || norm == "." {
        return true;
    }
    let b = norm.as_bytes();
    (b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
        || (b.len() == 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'/')
}

#[inline]
pub(crate) fn dir_child_prefix(norm_dir: &str) -> String {
    if norm_dir.ends_with('/') {
        norm_dir.to_string()
    } else {
        let mut s = String::with_capacity(norm_dir.len() + 1);
        s.push_str(norm_dir);
        s.push('/');
        s
    }
}

impl VfsState {
    #[inline]
    pub(crate) fn clear(&mut self) {
        self.inodes.clear();
        self.free_inodes.clear();
        self.paths.clear();
        self.fds.clear();
        self.free_fds.clear();
        self.open_fd_total = 0;
        self.total_accounted_bytes = 0;
        self.has_active_watchers = false;
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.open_fd_total == 0
    }

    #[inline]
    pub(crate) fn total_accounted_bytes(&self) -> usize {
        self.total_accounted_bytes
    }

    #[inline]
    pub(crate) fn get_inode(&self, id: InodeId) -> Option<&VfsInode> {
        self.inodes
            .get(id.0 as usize)
            .and_then(|slot| slot.as_ref())
    }

    #[inline]
    pub(crate) fn get_inode_mut(&mut self, id: InodeId) -> Option<&mut VfsInode> {
        self.inodes
            .get_mut(id.0 as usize)
            .and_then(|slot| slot.as_mut())
    }

    #[inline]
    pub(crate) fn lookup_present(&self, norm: &str) -> Option<(InodeId, &VfsInode)> {
        match self.paths.get(norm) {
            Some(PathEntry::Present(id)) => self.get_inode(*id).map(|ino| (*id, ino)),
            _ => None,
        }
    }

    pub(crate) fn alloc_inode(
        &mut self,
        kind: VfsEntryKind,
        mtime_ms: Option<f64>,
        link_count: u32,
        open_fd_count: u32,
    ) -> InodeId {
        let accounted_bytes = kind.accounted_bytes();
        self.total_accounted_bytes = self.total_accounted_bytes.saturating_add(accounted_bytes);
        let inode = VfsInode {
            kind,
            mtime_ms,
            link_count,
            open_fd_count,
            accounted_bytes,
        };
        if let Some(idx) = self.free_inodes.pop() {
            self.inodes[idx as usize] = Some(inode);
            InodeId(idx)
        } else {
            let idx = self.inodes.len() as u32;
            self.inodes.push(Some(inode));
            InodeId(idx)
        }
    }

    pub(crate) fn update_inode_kind(&mut self, id: InodeId, new_kind: VfsEntryKind) {
        let new_acc = new_kind.accounted_bytes();
        if let Some(ino) = self.inodes.get_mut(id.0 as usize).and_then(|s| s.as_mut()) {
            let old_acc = ino.accounted_bytes;
            self.total_accounted_bytes = self
                .total_accounted_bytes
                .saturating_sub(old_acc)
                .saturating_add(new_acc);
            ino.accounted_bytes = new_acc;
            ino.kind = new_kind;
        }
    }

    /// Decrements `link_count` on `id` when a path entry is unlinked or overwritten.
    /// Frees the inode and returns its `accounted_bytes` iff both `link_count == 0` and
    /// `open_fd_count == 0`.
    pub(crate) fn unlink_inode_ref(&mut self, id: InodeId) -> usize {
        let idx = id.0 as usize;
        let Some(slot) = self.inodes.get_mut(idx) else {
            return 0;
        };
        let Some(ino) = slot.as_mut() else {
            return 0;
        };
        ino.link_count = ino.link_count.saturating_sub(1);
        if ino.link_count == 0 && ino.open_fd_count == 0 {
            let freed = ino.accounted_bytes;
            self.total_accounted_bytes = self.total_accounted_bytes.saturating_sub(freed);
            *slot = None;
            self.free_inodes.push(id.0);
            freed
        } else {
            0
        }
    }

    /// Allocates a new file descriptor number (`>= 3`) referencing `inode`.
    pub(crate) fn alloc_fd(&mut self, entry: OpenFd) -> i32 {
        self.open_fd_total = self.open_fd_total.saturating_add(1);
        if let Some(idx) = self.free_fds.pop() {
            self.fds[idx as usize] = Some(entry);
            (idx + 3) as i32
        } else {
            let idx = self.fds.len() as u32;
            self.fds.push(Some(entry));
            (idx + 3) as i32
        }
    }

    #[inline]
    pub(crate) fn get_fd(&self, fd: i32) -> Option<&OpenFd> {
        if fd < 3 {
            return None;
        }
        let idx = (fd - 3) as usize;
        self.fds.get(idx).and_then(|s| s.as_ref())
    }

    #[inline]
    pub(crate) fn get_fd_mut(&mut self, fd: i32) -> Option<&mut OpenFd> {
        if fd < 3 {
            return None;
        }
        let idx = (fd - 3) as usize;
        self.fds.get_mut(idx).and_then(|s| s.as_mut())
    }

    /// Closes `fd` (`>= 3`). Returns `Ok(freed_bytes)` on success or `Err(())` (`EBADF`) if `fd`
    /// is invalid or already closed.
    pub(crate) fn close_fd(&mut self, fd: i32) -> Result<usize, ()> {
        if fd < 3 {
            return Err(());
        }
        let idx = (fd - 3) as usize;
        let Some(slot) = self.fds.get_mut(idx) else {
            return Err(());
        };
        let Some(open_fd) = slot.take() else {
            return Err(());
        };
        self.free_fds.push(idx as u32);
        self.open_fd_total = self.open_fd_total.saturating_sub(1);

        let ino_idx = open_fd.inode.0 as usize;
        let mut freed = 0usize;
        if let Some(ino_slot) = self.inodes.get_mut(ino_idx) {
            if let Some(ino) = ino_slot.as_mut() {
                ino.open_fd_count = ino.open_fd_count.saturating_sub(1);
                if ino.link_count == 0 && ino.open_fd_count == 0 {
                    freed = ino.accounted_bytes;
                    self.total_accounted_bytes = self.total_accounted_bytes.saturating_sub(freed);
                    *ino_slot = None;
                    self.free_inodes.push(open_fd.inode.0);
                }
            }
        }
        Ok(freed)
    }

    /// Closes all currently open file descriptors at turn teardown, freeing any unlinked
    /// (`link_count == 0`) inodes and returning the total reclaimed byte count.
    #[inline]
    pub(crate) fn close_all_open_fds(&mut self) -> usize {
        if self.open_fd_total == 0 {
            return 0;
        }
        let mut total_freed = 0usize;
        for idx in 0..self.fds.len() {
            if let Some(open_fd) = self.fds[idx].take() {
                let ino_idx = open_fd.inode.0 as usize;
                if let Some(ino_slot) = self.inodes.get_mut(ino_idx) {
                    if let Some(ino) = ino_slot.as_mut() {
                        ino.open_fd_count = ino.open_fd_count.saturating_sub(1);
                        if ino.link_count == 0 && ino.open_fd_count == 0 {
                            let freed = ino.accounted_bytes;
                            self.total_accounted_bytes =
                                self.total_accounted_bytes.saturating_sub(freed);
                            total_freed = total_freed.saturating_add(freed);
                            *ino_slot = None;
                            self.free_inodes.push(open_fd.inode.0);
                        }
                    }
                }
            }
        }
        self.fds.clear();
        self.free_fds.clear();
        self.open_fd_total = 0;
        total_freed
    }

    /// Returns `true` when host filesystem fallback for `norm` is masked by a tombstone on `norm`,
    /// a tombstone on an ancestor directory, or an ancestor `Directory { host_opaque: true }`.
    pub(crate) fn is_host_masked(&self, norm: &str) -> bool {
        if self.paths.is_empty() {
            return false;
        }
        if matches!(self.paths.get(norm), Some(PathEntry::Tombstone)) {
            return true;
        }
        let mut cur = norm;
        while let Some(idx) = cur.rfind('/') {
            if idx == 0 {
                break;
            }
            let anc = &cur[..idx];
            if is_vfs_root_path(anc) {
                break;
            }
            match self.paths.get(anc) {
                Some(PathEntry::Tombstone) => return true,
                Some(PathEntry::Present(id)) => {
                    if let Some(ino) = self.get_inode(*id) {
                        if matches!(ino.kind, VfsEntryKind::Directory { host_opaque: true }) {
                            return true;
                        }
                    }
                }
                None => {}
            }
            cur = anc;
        }
        false
    }

    /// Ensures `norm_dir` and all non-root ancestor directories exist in `paths` as `Directory`
    /// inodes without overwriting existing `Present` entries.
    pub(crate) fn ensure_dir_with_ancestors(&mut self, norm_dir: &str, leaf_opaque_if_new: bool) {
        let mut ancestors: smallvec::SmallVec<[&str; 16]> = smallvec::SmallVec::new();
        let mut cur = norm_dir;
        while !is_vfs_root_path(cur) {
            ancestors.push(cur);
            if let Some(idx) = cur.rfind('/') {
                if idx == 0 {
                    break;
                }
                cur = &cur[..idx];
            } else {
                break;
            }
        }
        // Insert top-down so parent directories are created before children.
        for (rev_i, &dir_path) in ancestors.iter().rev().enumerate() {
            let is_leaf = rev_i + 1 == ancestors.len();
            match self.paths.get(dir_path).copied() {
                Some(PathEntry::Present(_)) => {}
                prev => {
                    let was_tombstoned = matches!(prev, Some(PathEntry::Tombstone));
                    let host_opaque = was_tombstoned
                        || (is_leaf && leaf_opaque_if_new)
                        || (dir_path.starts_with("/tmp/")
                            && !std::path::Path::new(dir_path).exists());
                    let id = self.alloc_inode(VfsEntryKind::Directory { host_opaque }, None, 1, 0);
                    self.paths
                        .insert(dir_path.to_string(), PathEntry::Present(id));
                }
            }
        }
    }

    #[inline]
    pub(crate) fn ensure_parent_dirs(&mut self, norm_file: &str) {
        if let Some(idx) = norm_file.rfind('/') {
            let parent = &norm_file[..idx];
            if !is_vfs_root_path(parent) {
                self.ensure_dir_with_ancestors(parent, false);
            }
        }
    }

    /// Captures a full checkpoint snapshot (including open file descriptors and cursors).
    pub(crate) fn snapshot_with_fds(&self) -> VfsSnapshot {
        if self.is_empty() {
            return VfsSnapshot::default();
        }
        VfsSnapshot {
            inodes: self.inodes.clone(),
            free_inodes: self.free_inodes.clone(),
            paths: self.paths.clone(),
            fds: self.fds.clone(),
            free_fds: self.free_fds.clone(),
            open_fd_total: self.open_fd_total,
            total_accounted_bytes: self.total_accounted_bytes,
        }
    }

    /// Captures a VFS tree snapshot for child `Worker` / `evalSandbox` transfer without inheriting
    /// open file descriptors from the parent isolate.
    pub(crate) fn snapshot_without_fds(&self) -> VfsSnapshot {
        if self.paths.is_empty() {
            return VfsSnapshot::default();
        }
        let mut inodes = self.inodes.clone();
        let mut free_inodes = self.free_inodes.clone();
        let mut accounted = self.total_accounted_bytes;
        if self.open_fd_total > 0 {
            for (idx, slot) in inodes.iter_mut().enumerate() {
                if let Some(ino) = slot.as_mut() {
                    ino.open_fd_count = 0;
                    if ino.link_count == 0 {
                        accounted = accounted.saturating_sub(ino.accounted_bytes);
                        *slot = None;
                        free_inodes.push(idx as u32);
                    }
                }
            }
        }
        VfsSnapshot {
            inodes,
            free_inodes,
            paths: self.paths.clone(),
            fds: Vec::new(),
            free_fds: Vec::new(),
            open_fd_total: 0,
            total_accounted_bytes: accounted,
        }
    }

    pub(crate) fn restore_from_snapshot(&mut self, snap: VfsSnapshot) {
        self.inodes = snap.inodes;
        self.free_inodes = snap.free_inodes;
        self.paths = snap.paths;
        self.fds = snap.fds;
        self.free_fds = snap.free_fds;
        self.open_fd_total = snap.open_fd_total;
        self.total_accounted_bytes = snap.total_accounted_bytes;
        self.has_active_watchers = false;
    }
}
