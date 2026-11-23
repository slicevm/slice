//! Canonical VFS Path Classification, `ELOOP` Error Helpers, and Copy-on-Write Host Materialization.

use crate::bindings::fs::state::{
    PathEntry, VfsEntryKind, VfsState, dir_child_prefix, is_vfs_root_path,
};
pub(crate) use crate::bindings::fs::symlink_resolver::{
    ResolvedVfsPath, SymlinkResolveError, anchor_vfs_path, is_host_symlink_or_reparse,
    is_host_unc_blocked, normalize_host_readlink_target, resolve_effective_vfs_path,
    resolve_symlink_target_against_link,
};
use crate::bindings::helpers::{ascii_bytes_to_js_val, try_consume_isolate_memory};
use crate::boa_realm::{
    BOA_DET_STATE, BOA_VFS_STATE, check_boa_fs_read_ocap, has_boa_fs_read_ocap,
    has_boa_fs_write_ocap,
};
use std::sync::Arc;

#[inline]
pub(crate) fn is_vfs_tombstoned(norm: &str) -> bool {
    let anchored = anchor_vfs_path(norm);
    BOA_VFS_STATE.with(|v| v.borrow().is_host_masked(&anchored))
}

pub(crate) type ResolvedScriptOutcome = (String, Option<Arc<Vec<u8>>>);

/// Resolves `raw_path` (including both VFS and host symlinks under per-hop OCap checks) and
/// returns `Ok((resolved_norm, Some(bytes)))` if backed by an in-memory [`VfsState`] file,
/// `Ok((resolved_norm, None))` if not materialized in VFS, or `Err(SymlinkResolveError)` on
/// `ELOOP` or OCap denial.
pub(crate) fn resolve_vfs_or_host_script(
    raw_path: &str,
) -> Result<ResolvedScriptOutcome, SymlinkResolveError> {
    let allow_host = !is_host_unc_blocked(raw_path);
    let resolved = resolve_effective_vfs_path(raw_path, true, allow_host)?;
    if resolved.has_file_ancestor {
        return Ok((resolved.norm, None));
    }
    let bytes = BOA_VFS_STATE.with(|v| {
        let vfs = v.borrow();
        match vfs.lookup_present(&resolved.norm).map(|(_, i)| &i.kind) {
            Some(VfsEntryKind::File(b)) => Some(Arc::clone(b)),
            _ => None,
        }
    });
    Ok((resolved.norm, bytes))
}

#[cold]
pub(crate) fn js_eloop_error(
    syscall: &str,
    path: &str,
    context: &mut boa_engine::Context,
) -> boa_engine::JsError {
    use boa_engine::{JsNativeError, js_string};
    let msg = format!(
        "ELOOP: too many symbolic links encountered, {} '{}'",
        syscall, path
    );
    let base_err: boa_engine::JsError = JsNativeError::error().with_message(msg).into();
    if let Ok(opaque) = base_err.into_opaque(context) {
        if let Some(obj) = opaque.as_object() {
            let _ = obj.set(js_string!("code"), js_string!("ELOOP"), false, context);
            let _ = obj.set(js_string!("errno"), -40, false, context);
            let _ = obj.set(js_string!("syscall"), js_string!(syscall), false, context);
            let _ = obj.set(js_string!("path"), js_string!(path), false, context);
        }
        return boa_engine::JsError::from_opaque(opaque);
    }
    JsNativeError::error()
        .with_message("ELOOP: too many symbolic links encountered")
        .into()
}

/// Helper mapping [`SymlinkResolveError`] into either a recorded OCap read violation (`Ok(None)`)
/// or a thrown `ELOOP` `JsError` (`Err(...)`).
#[inline]
pub(crate) fn resolve_or_js_err(
    raw_path: &str,
    follow_final: bool,
    allow_host: bool,
    syscall: &str,
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<Option<ResolvedVfsPath>> {
    match resolve_effective_vfs_path(raw_path, follow_final, allow_host) {
        Ok(res) => Ok(Some(res)),
        Err(SymlinkResolveError::ReadOcapDenied(denied)) => {
            let _ = check_boa_fs_read_ocap(&denied);
            Ok(None)
        }
        Err(SymlinkResolveError::Loop) => Err(js_eloop_error(syscall, raw_path, context)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResolvedPathKind {
    Missing,
    File,
    Directory,
    Symlink,
}

/// Classifies `norm` without dereferencing a final symlink (`lstat` semantics).
pub(crate) fn classify_path_lstat(
    vfs: &VfsState,
    norm: &str,
    allow_host: bool,
) -> ResolvedPathKind {
    match vfs.paths.get(norm) {
        Some(PathEntry::Tombstone) => ResolvedPathKind::Missing,
        Some(PathEntry::Present(id)) => match vfs.get_inode(*id).map(|i| &i.kind) {
            Some(VfsEntryKind::File(_)) => ResolvedPathKind::File,
            Some(VfsEntryKind::Directory { .. }) => ResolvedPathKind::Directory,
            Some(VfsEntryKind::Symlink(_)) => ResolvedPathKind::Symlink,
            None => ResolvedPathKind::Missing,
        },
        None => {
            if is_vfs_root_path(norm) {
                return ResolvedPathKind::Directory;
            }
            if !allow_host
                || vfs.is_host_masked(norm)
                || !(has_boa_fs_read_ocap(norm) || has_boa_fs_write_ocap(norm))
            {
                return ResolvedPathKind::Missing;
            }
            if norm == "/tmp" {
                return match std::fs::metadata(norm) {
                    Ok(m) if m.is_dir() => ResolvedPathKind::Directory,
                    Ok(_) => ResolvedPathKind::File,
                    Err(_) => ResolvedPathKind::Missing,
                };
            }
            match std::fs::symlink_metadata(norm) {
                Ok(m) if is_host_symlink_or_reparse(&m) => ResolvedPathKind::Symlink,
                Ok(m) if m.is_dir() => ResolvedPathKind::Directory,
                Ok(_) => ResolvedPathKind::File,
                Err(_) => ResolvedPathKind::Missing,
            }
        }
    }
}

/// Classifies `norm` after [`resolve_effective_vfs_path`] with `follow_final = true`.
/// Uses `symlink_metadata` as defense-in-depth so OS `stat` never follows an unverified symlink.
pub(crate) fn classify_path_stat(
    vfs: &VfsState,
    norm: &str,
    allow_host: bool,
) -> (ResolvedPathKind, u64) {
    match vfs.paths.get(norm) {
        Some(PathEntry::Tombstone) => (ResolvedPathKind::Missing, 0),
        Some(PathEntry::Present(id)) => match vfs.get_inode(*id).map(|i| &i.kind) {
            Some(VfsEntryKind::File(bytes)) => (ResolvedPathKind::File, bytes.len() as u64),
            Some(VfsEntryKind::Directory { .. }) => (ResolvedPathKind::Directory, 0),
            Some(VfsEntryKind::Symlink(_)) | None => (ResolvedPathKind::Missing, 0),
        },
        None => {
            if is_vfs_root_path(norm) {
                return (ResolvedPathKind::Directory, 0);
            }
            if !allow_host
                || vfs.is_host_masked(norm)
                || !(has_boa_fs_read_ocap(norm) || has_boa_fs_write_ocap(norm))
            {
                return (ResolvedPathKind::Missing, 0);
            }
            if norm == "/tmp" {
                return match std::fs::metadata(norm) {
                    Ok(m) if m.is_dir() => (ResolvedPathKind::Directory, 0),
                    Ok(m) => (ResolvedPathKind::File, m.len()),
                    Err(_) => (ResolvedPathKind::Missing, 0),
                };
            }
            match std::fs::symlink_metadata(norm) {
                Ok(m) if is_host_symlink_or_reparse(&m) => (ResolvedPathKind::Missing, 0),
                Ok(m) if m.is_dir() => (ResolvedPathKind::Directory, 0),
                Ok(m) => (ResolvedPathKind::File, m.len()),
                Err(_) => (ResolvedPathKind::Missing, 0),
            }
        }
    }
}

#[inline]
pub(crate) fn default_mtime_ms_for_path(vfs: &VfsState, norm: &str, allow_host: bool) -> f64 {
    if let Some((_, ino)) = vfs.lookup_present(norm)
        && let Some(mt) = ino.mtime_ms
    {
        return mt;
    }
    if BOA_DET_STATE.with(|s| s.get().is_some()) {
        return 1_700_000_000_000.0;
    }
    let host_mt = if !allow_host || vfs.is_host_masked(norm) || !has_boa_fs_read_ocap(norm) {
        None
    } else {
        std::fs::symlink_metadata(norm)
            .ok()
            .filter(|m| !is_host_symlink_or_reparse(m))
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as f64)
    };
    host_mt.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as f64)
            .unwrap_or(1_700_000_000_000.0)
    })
}

pub(crate) fn dir_has_any_children(vfs: &VfsState, norm_dir: &str, allow_host: bool) -> bool {
    let prefix = dir_child_prefix(norm_dir);
    for (k, entry) in &vfs.paths {
        if matches!(entry, PathEntry::Present(_)) && k.starts_with(&prefix) {
            return true;
        }
    }
    let is_opaque = matches!(
        vfs.lookup_present(norm_dir).map(|(_, i)| &i.kind),
        Some(VfsEntryKind::Directory { host_opaque: true })
    );
    if allow_host
        && !is_opaque
        && !vfs.is_host_masked(norm_dir)
        && has_boa_fs_read_ocap(norm_dir)
        && let Ok(rd) = std::fs::read_dir(norm_dir)
    {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let child_norm = if norm_dir.ends_with('/') {
                format!("{}{}", norm_dir, name)
            } else {
                format!("{}/{}", norm_dir, name)
            };
            if !vfs.is_host_masked(&child_norm) {
                return true;
            }
        }
    }
    false
}

pub(crate) fn materialize_host_dir_into_vfs(
    vfs: &mut VfsState,
    host_dir: &str,
) -> Result<(), String> {
    if !has_boa_fs_read_ocap(host_dir) {
        return Ok(());
    }
    let Ok(rd) = std::fs::read_dir(host_dir) else {
        return Ok(());
    };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let child_norm = if host_dir.ends_with('/') {
            format!("{}{}", host_dir, name)
        } else {
            format!("{}/{}", host_dir, name)
        };
        if vfs.is_host_masked(&child_norm) {
            continue;
        }
        let already_present = matches!(vfs.paths.get(&child_norm), Some(PathEntry::Present(_)));
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_symlink() {
            if !already_present && let Ok(target_path) = std::fs::read_link(entry.path()) {
                let target_str = normalize_host_readlink_target(&target_path);
                let resolved_target = resolve_symlink_target_against_link(&child_norm, &target_str);
                if !has_boa_fs_read_ocap(&resolved_target) {
                    continue;
                }
                try_consume_isolate_memory(target_str.len())?;
                let id = vfs.alloc_inode(VfsEntryKind::Symlink(target_str), None, 1, 0);
                vfs.paths.insert(child_norm, PathEntry::Present(id));
            }
        } else if ft.is_dir() {
            if !already_present {
                let id =
                    vfs.alloc_inode(VfsEntryKind::Directory { host_opaque: false }, None, 1, 0);
                vfs.paths.insert(child_norm.clone(), PathEntry::Present(id));
            }
            materialize_host_dir_into_vfs(vfs, &child_norm)?;
        } else if ft.is_file()
            && !already_present
            && let Ok(bytes) = std::fs::read(entry.path())
        {
            try_consume_isolate_memory(bytes.len())?;
            let id = vfs.alloc_inode(VfsEntryKind::File(Arc::new(bytes)), None, 1, 0);
            vfs.paths.insert(child_norm, PathEntry::Present(id));
        }
    }
    Ok(())
}

#[inline]
pub(crate) fn utf8_vec_to_js_val(bytes: Vec<u8>) -> boa_engine::JsValue {
    if bytes.is_ascii() {
        return ascii_bytes_to_js_val(&bytes);
    }
    match String::from_utf8(bytes) {
        Ok(s) => boa_engine::JsValue::from(boa_engine::js_string!(s)),
        Err(e) => {
            let content = String::from_utf8_lossy(e.as_bytes());
            boa_engine::JsValue::from(boa_engine::js_string!(content.as_ref()))
        }
    }
}
