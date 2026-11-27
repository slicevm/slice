//! Post-Execution VFS Artifact Drain API (`SliceIsolate::drain_vfs`) with Zero-Copy Byte Extraction
//! and Ring 4 Egress DLP Scanning (`#226`).

use crate::bindings::fs::state::{PathEntry, VfsEntryKind};
use crate::boa_realm::{BOA_VFS_OWNER_TENANT, BOA_VFS_STATE, norm_vfs_path, vfs_owner_namespace};
use crate::intrinsics::base64_encode;
use crate::isolate::SliceIsolate;
use crate::state_and_kv::verify_egress_secrets_slice;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Bounds enforced when draining regular files from an isolate's in-memory VFS.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DrainLimits {
    pub max_files: usize,
    pub max_file_bytes: usize,
    pub max_total_bytes: usize,
    pub max_path_len: usize,
}

impl Default for DrainLimits {
    fn default() -> Self {
        Self {
            max_files: 64,
            max_file_bytes: 4 * 1024 * 1024,
            max_total_bytes: 16 * 1024 * 1024,
            max_path_len: 256,
        }
    }
}

/// Reason a VFS entry under the drained prefix was skipped instead of collected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkipReason {
    Symlink,
    TooLarge,
    OverTotal,
    TooMany,
    PathTooLong,
    AuditedSecret,
}

/// A regular file collected from the isolate's in-memory VFS after execution.
#[derive(Debug, Clone)]
pub struct DrainedFile {
    /// Path relative to the drained prefix, using forward slashes with no leading `/`.
    pub path: String,
    /// Raw file bytes moved out of the VFS inode.
    pub bytes: Vec<u8>,
    /// Modification timestamp in milliseconds since Unix epoch (0 if unset).
    pub mtime_ms: u64,
}

/// Metadata for a VFS entry skipped during [`SliceIsolate::drain_vfs`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedFile {
    pub path: String,
    pub reason: SkipReason,
}

/// Result of calling [`SliceIsolate::drain_vfs`].
#[derive(Debug, Clone, Default)]
pub struct DrainReport {
    pub files: Vec<DrainedFile>,
    pub skipped: Vec<SkippedFile>,
}

impl SliceIsolate {
    /// Collects regular files written by guest code under `prefix` in this isolate's in-memory
    /// VFS (`BOA_VFS_STATE`), moving file bytes out without cloning when uniquely owned and
    /// releasing their accounted memory budget.
    ///
    /// - Callable after `execute_bytecode` regardless of outcome (success, caught/uncaught OCap
    ///   denial, CPU fuel exhaustion, wall-clock timeout, or interrupt), before `reset_for_request`.
    /// - Returns entries in deterministic sorted path order.
    /// - Ignores empty directories and never follows symlinks (reporting them as
    ///   [`SkipReason::Symlink`]).
    /// - Scans every file against `self.egress_audit_secrets` (raw, hex, URL-encoded, Base64,
    ///   Base64URL), dropping matching files as [`SkipReason::AuditedSecret`] and recording
    ///   `artifact:<path>` in `self.denied_capabilities`.
    pub fn drain_vfs(&mut self, prefix: &str, limits: &DrainLimits) -> DrainReport {
        let mut report = DrainReport::default();
        let owner_ns = vfs_owner_namespace(&self.compartment.tenant_id);
        let owns_vfs = BOA_VFS_OWNER_TENANT.with(|o| o.borrow().as_str() == owner_ns);
        if !owns_vfs {
            return report;
        }

        let norm_prefix = norm_vfs_path(prefix);
        if norm_prefix.is_empty() {
            return report;
        }
        let prefix_slash = if norm_prefix.ends_with('/') {
            norm_prefix.clone()
        } else {
            format!("{norm_prefix}/")
        };

        // 1. Collect matching present entries under `prefix_slash` in deterministic sorted order.
        let mut candidates: Vec<(String, String)> = BOA_VFS_STATE.with(|v| {
            let vfs = v.borrow();
            let mut out = Vec::new();
            for (full_path, entry) in &vfs.paths {
                if !matches!(entry, PathEntry::Present(_)) {
                    continue;
                }
                if let Some(rel) = full_path.strip_prefix(&prefix_slash)
                    && !rel.is_empty()
                {
                    out.push((rel.to_string(), full_path.clone()));
                }
            }
            out
        });
        candidates.sort_by(|a, b| a.0.cmp(&b.0));

        if candidates.is_empty() {
            return report;
        }

        let mut total_bytes: usize = 0;
        let has_dlp = !self.egress_audit_secrets.is_empty();
        let tenant_id = self.compartment.tenant_id.clone();

        for (rel_path, full_path) in candidates {
            // Inspect entry kind first without removing directories.
            enum EntryAction {
                Ignore,
                Skip(SkipReason),
                TakeFile {
                    bytes: Vec<u8>,
                    mtime_ms: u64,
                    freed: usize,
                },
            }

            let action = BOA_VFS_STATE.with(|v| {
                let mut vfs = v.borrow_mut();
                let Some(PathEntry::Present(inode_id)) = vfs.paths.get(&full_path).copied() else {
                    return EntryAction::Ignore;
                };
                let Some(ino) = vfs.get_inode(inode_id) else {
                    return EntryAction::Ignore;
                };
                match &ino.kind {
                    VfsEntryKind::Directory { .. } => EntryAction::Ignore,
                    VfsEntryKind::Symlink(_) => EntryAction::Skip(SkipReason::Symlink),
                    VfsEntryKind::File(arc_bytes) => {
                        if rel_path.len() > limits.max_path_len {
                            return EntryAction::Skip(SkipReason::PathTooLong);
                        }
                        if report.files.len() >= limits.max_files {
                            return EntryAction::Skip(SkipReason::TooMany);
                        }
                        let file_len = arc_bytes.len();
                        if file_len > limits.max_file_bytes {
                            return EntryAction::Skip(SkipReason::TooLarge);
                        }
                        if total_bytes.saturating_add(file_len) > limits.max_total_bytes {
                            return EntryAction::Skip(SkipReason::OverTotal);
                        }

                        let mtime_ms = ino.mtime_ms.map(|m| m.max(0.0) as u64).unwrap_or(0);
                        // Remove path entry and extract bytes zero-copy when uniquely owned.
                        vfs.paths.remove(&full_path);
                        let (bytes, freed) = if let Some(slot) =
                            vfs.inodes.get_mut(inode_id.0 as usize)
                            && let Some(ino_mut) = slot.as_mut()
                        {
                            ino_mut.link_count = ino_mut.link_count.saturating_sub(1);
                            if ino_mut.link_count == 0 && ino_mut.open_fd_count == 0 {
                                let freed_bytes = ino_mut.accounted_bytes;
                                let owned = slot.take().unwrap();
                                vfs.total_accounted_bytes =
                                    vfs.total_accounted_bytes.saturating_sub(freed_bytes);
                                vfs.free_inodes.push(inode_id.0);
                                let extracted = match owned.kind {
                                    VfsEntryKind::File(arc) => {
                                        Arc::try_unwrap(arc).unwrap_or_else(|a| (*a).clone())
                                    }
                                    _ => Vec::new(),
                                };
                                (extracted, freed_bytes)
                            } else {
                                let cloned = match &ino_mut.kind {
                                    VfsEntryKind::File(arc) => (**arc).clone(),
                                    _ => Vec::new(),
                                };
                                (cloned, 0)
                            }
                        } else {
                            (Vec::new(), 0)
                        };
                        EntryAction::TakeFile {
                            bytes,
                            mtime_ms,
                            freed,
                        }
                    }
                }
            });

            match action {
                EntryAction::Ignore => {}
                EntryAction::Skip(reason) => {
                    report.skipped.push(SkippedFile {
                        path: rel_path,
                        reason,
                    });
                }
                EntryAction::TakeFile {
                    bytes,
                    mtime_ms,
                    freed,
                } => {
                    if freed > 0 {
                        let baseline = self.compartment.memory_used_bytes.min(4096);
                        self.compartment.memory_used_bytes = self
                            .compartment
                            .memory_used_bytes
                            .saturating_sub(freed)
                            .max(baseline);
                    }
                    if has_dlp {
                        let lossy = String::from_utf8_lossy(&bytes);
                        let dlp_hit = verify_egress_secrets_slice(
                            &tenant_id,
                            &self.egress_audit_secrets,
                            &lossy,
                        )
                        .and_then(|()| {
                            if !bytes.is_empty() && !bytes.is_ascii() {
                                let b64 = base64_encode(&bytes, false);
                                verify_egress_secrets_slice(
                                    &tenant_id,
                                    &self.egress_audit_secrets,
                                    &b64,
                                )
                            } else {
                                Ok(())
                            }
                        })
                        .is_err();
                        if dlp_hit {
                            if self.denied_capabilities.len() < 64 {
                                self.denied_capabilities
                                    .push(format!("artifact:{rel_path}"));
                            }
                            report.skipped.push(SkippedFile {
                                path: rel_path,
                                reason: SkipReason::AuditedSecret,
                            });
                            continue;
                        }
                    }
                    total_bytes = total_bytes.saturating_add(bytes.len());
                    report.files.push(DrainedFile {
                        path: rel_path,
                        bytes,
                        mtime_ms,
                    });
                }
            }
        }

        report
    }
}
