//! Canonical VFS Path Classification, Copy-on-Write Host Materialization,
//! and Path-Based `node:fs` Host Bindings.

use crate::bindings::fs::state::{
    PathEntry, VfsEntryKind, VfsState, dir_child_prefix, is_vfs_root_path,
};
pub(crate) use crate::bindings::fs::symlink_resolver::{
    ResolvedVfsPath, SymlinkResolveError, anchor_vfs_path, is_host_symlink_or_reparse,
    is_host_unc_blocked, normalize_host_readlink_target, resolve_effective_vfs_path,
    resolve_symlink_target_against_link,
};
use crate::bindings::helpers::{
    bytes_to_js_uint8_array, extract_buffer_source_bytes, release_isolate_memory,
    trip_isolate_oom_limits, try_adjust_isolate_memory_ctx, try_consume_isolate_memory,
    try_consume_isolate_memory_ctx,
};
use crate::boa_realm::{
    BOA_DET_STATE, BOA_VFS_STATE, check_boa_fs_read_ocap, check_boa_fs_write_ocap,
    has_boa_fs_read_ocap, has_boa_fs_write_ocap, norm_vfs_path,
};
use crate::intrinsics::fill_random_bytes;
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
            Some(VfsEntryKind::Symlink(_)) => (ResolvedPathKind::Missing, 0),
            None => (ResolvedPathKind::Missing, 0),
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

fn dir_has_any_children(vfs: &VfsState, norm_dir: &str, allow_host: bool) -> bool {
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

fn materialize_host_dir_into_vfs(vfs: &mut VfsState, host_dir: &str) -> Result<(), String> {
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
pub(crate) fn utf8_slice_to_js_val(bytes: &[u8]) -> boa_engine::JsValue {
    if bytes.is_ascii() {
        return boa_engine::JsValue::from(boa_engine::js_string!(
            boa_engine::string::JsStr::latin1(bytes)
        ));
    }
    let content = String::from_utf8_lossy(bytes);
    boa_engine::JsValue::from(boa_engine::js_string!(content.as_ref()))
}

#[inline]
pub(crate) fn utf8_vec_to_js_val(bytes: Vec<u8>) -> boa_engine::JsValue {
    if bytes.is_ascii() {
        return boa_engine::JsValue::from(boa_engine::js_string!(
            boa_engine::string::JsStr::latin1(&bytes)
        ));
    }
    match String::from_utf8(bytes) {
        Ok(s) => boa_engine::JsValue::from(boa_engine::js_string!(s)),
        Err(e) => {
            let content = String::from_utf8_lossy(e.as_bytes());
            boa_engine::JsValue::from(boa_engine::js_string!(content.as_ref()))
        }
    }
}

pub(crate) fn register_fs_path_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::object::builtins::JsArray;
    use boa_engine::{JsNativeError, JsValue as BoaVal, NativeFunction, js_string};

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_check_write_sync"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let follow_final = args.get(1).and_then(|v| v.as_boolean()).unwrap_or(true);
            if !check_boa_fs_write_ocap(&path) {
                return Ok(BoaVal::from(false));
            }
            let allow_host = !is_host_unc_blocked(&path);
            let Some(resolved) =
                resolve_or_js_err(&path, follow_final, allow_host, "open", context)?
            else {
                return Ok(BoaVal::from(false));
            };
            if resolved.traversed_symlink && !check_boa_fs_write_ocap(&resolved.norm) {
                return Ok(BoaVal::from(false));
            }
            Ok(BoaVal::from(true))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_read_file_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_read_ocap(&path) {
                return Ok(BoaVal::from(js_string!("")));
            }
            let allow_host = !is_host_unc_blocked(&path);
            let Some(resolved) = resolve_or_js_err(&path, true, allow_host, "open", context)?
            else {
                return Ok(BoaVal::from(js_string!("")));
            };
            if resolved.traversed_symlink && !check_boa_fs_read_ocap(&resolved.norm) {
                return Ok(BoaVal::from(js_string!("")));
            }
            if resolved.has_file_ancestor {
                return Ok(BoaVal::from(-20));
            }
            let norm = resolved.norm;
            enum ReadOutcome {
                VfsFile(Arc<Vec<u8>>),
                IsDir,
                HostCandidate,
                Missing,
            }
            let outcome = BOA_VFS_STATE.with(|v| {
                let vfs = v.borrow();
                match vfs.paths.get(&norm) {
                    Some(PathEntry::Tombstone) => ReadOutcome::Missing,
                    Some(PathEntry::Present(id)) => match vfs.get_inode(*id).map(|i| &i.kind) {
                        Some(VfsEntryKind::File(b)) => ReadOutcome::VfsFile(Arc::clone(b)),
                        Some(VfsEntryKind::Directory { .. }) => ReadOutcome::IsDir,
                        _ => ReadOutcome::Missing,
                    },
                    None => {
                        if is_vfs_root_path(&norm) {
                            ReadOutcome::IsDir
                        } else if allow_host && !vfs.is_host_masked(&norm) {
                            ReadOutcome::HostCandidate
                        } else {
                            ReadOutcome::Missing
                        }
                    }
                }
            });
            match outcome {
                ReadOutcome::VfsFile(b) => Ok(utf8_slice_to_js_val(&b)),
                ReadOutcome::IsDir => Ok(BoaVal::from(-21)),
                ReadOutcome::HostCandidate => {
                    if let Ok(meta) = std::fs::symlink_metadata(&norm) {
                        if is_host_symlink_or_reparse(&meta) {
                            return Ok(BoaVal::null());
                        }
                        if meta.is_dir() {
                            return Ok(BoaVal::from(-21));
                        }
                    }
                    match std::fs::read(&norm) {
                        Ok(bytes) => Ok(utf8_vec_to_js_val(bytes)),
                        Err(_) => Ok(BoaVal::null()),
                    }
                }
                ReadOutcome::Missing => Ok(BoaVal::null()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_read_bytes_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_read_ocap(&path) {
                return bytes_to_js_uint8_array(&[], context);
            }
            let allow_host = !is_host_unc_blocked(&path);
            let Some(resolved) = resolve_or_js_err(&path, true, allow_host, "open", context)?
            else {
                return bytes_to_js_uint8_array(&[], context);
            };
            if resolved.traversed_symlink && !check_boa_fs_read_ocap(&resolved.norm) {
                return bytes_to_js_uint8_array(&[], context);
            }
            if resolved.has_file_ancestor {
                return Ok(BoaVal::from(-20));
            }
            let norm = resolved.norm;
            enum ReadOutcome {
                VfsFile(Arc<Vec<u8>>),
                IsDir,
                HostCandidate,
                Missing,
            }
            let outcome = BOA_VFS_STATE.with(|v| {
                let vfs = v.borrow();
                match vfs.paths.get(&norm) {
                    Some(PathEntry::Tombstone) => ReadOutcome::Missing,
                    Some(PathEntry::Present(id)) => match vfs.get_inode(*id).map(|i| &i.kind) {
                        Some(VfsEntryKind::File(b)) => ReadOutcome::VfsFile(Arc::clone(b)),
                        Some(VfsEntryKind::Directory { .. }) => ReadOutcome::IsDir,
                        _ => ReadOutcome::Missing,
                    },
                    None => {
                        if is_vfs_root_path(&norm) {
                            ReadOutcome::IsDir
                        } else if allow_host && !vfs.is_host_masked(&norm) {
                            ReadOutcome::HostCandidate
                        } else {
                            ReadOutcome::Missing
                        }
                    }
                }
            });
            match outcome {
                ReadOutcome::VfsFile(b) => bytes_to_js_uint8_array(&b, context),
                ReadOutcome::IsDir => Ok(BoaVal::from(-21)),
                ReadOutcome::HostCandidate => {
                    if let Ok(meta) = std::fs::symlink_metadata(&norm) {
                        if is_host_symlink_or_reparse(&meta) {
                            return Ok(BoaVal::null());
                        }
                        if meta.is_dir() {
                            return Ok(BoaVal::from(-21));
                        }
                    }
                    match std::fs::read(&norm) {
                        Ok(bytes) => bytes_to_js_uint8_array(&bytes, context),
                        Err(_) => Ok(BoaVal::null()),
                    }
                }
                ReadOutcome::Missing => Ok(BoaVal::null()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_write_file_sync"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_write_ocap(&path) {
                return Ok(BoaVal::undefined());
            }
            let allow_host = !is_host_unc_blocked(&path);
            let Some(resolved) = resolve_or_js_err(&path, true, allow_host, "open", context)?
            else {
                return Ok(BoaVal::undefined());
            };
            if resolved.traversed_symlink && !check_boa_fs_write_ocap(&resolved.norm) {
                return Ok(BoaVal::undefined());
            }
            if resolved.has_file_ancestor {
                return Ok(BoaVal::from(-20));
            }
            let norm = resolved.norm;
            let is_dir = BOA_VFS_STATE.with(|v| {
                classify_path_stat(&v.borrow(), &norm, allow_host).0 == ResolvedPathKind::Directory
            });
            if is_dir {
                return Ok(BoaVal::from(-21));
            }
            let data_bytes = if let Some(val) = args.get(1) {
                if val.is_object() {
                    extract_buffer_source_bytes(val, context)?
                } else {
                    val.to_string(context)?.to_std_string_escaped().into_bytes()
                }
            } else {
                Vec::new()
            };
            let existing = BOA_VFS_STATE.with(|v| {
                let vfs = v.borrow();
                vfs.lookup_present(&norm)
                    .map(|(id, ino)| (id, ino.accounted_bytes))
            });
            if let Some((id, prev_acc)) = existing {
                let delta = data_bytes.len() as isize - prev_acc as isize;
                if let Err(msg) = try_adjust_isolate_memory_ctx(delta, context) {
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                BOA_VFS_STATE.with(|v| {
                    let mut vfs = v.borrow_mut();
                    vfs.update_inode_kind(id, VfsEntryKind::File(Arc::new(data_bytes)));
                });
            } else {
                if let Err(msg) = try_consume_isolate_memory_ctx(data_bytes.len(), context) {
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                BOA_VFS_STATE.with(|v| {
                    let mut vfs = v.borrow_mut();
                    vfs.ensure_parent_dirs(&norm);
                    let id = vfs.alloc_inode(VfsEntryKind::File(Arc::new(data_bytes)), None, 1, 0);
                    vfs.paths.insert(norm, PathEntry::Present(id));
                });
            }
            Ok(BoaVal::from(0))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_append_file_sync"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_write_ocap(&path) {
                return Ok(BoaVal::undefined());
            }
            let allow_host = !is_host_unc_blocked(&path);
            let Some(resolved) = resolve_or_js_err(&path, true, allow_host, "open", context)?
            else {
                return Ok(BoaVal::undefined());
            };
            if resolved.traversed_symlink && !check_boa_fs_write_ocap(&resolved.norm) {
                return Ok(BoaVal::undefined());
            }
            if resolved.has_file_ancestor {
                return Ok(BoaVal::from(-20));
            }
            let norm = resolved.norm;
            let host_stat_kind =
                BOA_VFS_STATE.with(|v| classify_path_stat(&v.borrow(), &norm, allow_host).0);
            if host_stat_kind == ResolvedPathKind::Directory {
                return Ok(BoaVal::from(-21));
            }
            let data_bytes = if let Some(val) = args.get(1) {
                if val.is_object() {
                    extract_buffer_source_bytes(val, context)?
                } else {
                    val.to_string(context)?.to_std_string_escaped().into_bytes()
                }
            } else {
                Vec::new()
            };
            let existing_id = BOA_VFS_STATE.with(|v| {
                let vfs = v.borrow();
                vfs.lookup_present(&norm).map(|(id, _)| id)
            });
            if let Some(id) = existing_id {
                if let Err(msg) = try_consume_isolate_memory_ctx(data_bytes.len(), context) {
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                BOA_VFS_STATE.with(|v| {
                    let mut vfs = v.borrow_mut();
                    let mut arc_buf = match vfs.get_inode(id).map(|i| &i.kind) {
                        Some(VfsEntryKind::File(b)) => Arc::clone(b),
                        _ => Arc::new(Vec::new()),
                    };
                    Arc::make_mut(&mut arc_buf).extend_from_slice(&data_bytes);
                    vfs.update_inode_kind(id, VfsEntryKind::File(arc_buf));
                });
            } else {
                let host_prefix = if host_stat_kind == ResolvedPathKind::File {
                    if !check_boa_fs_read_ocap(&norm) {
                        return Ok(BoaVal::undefined());
                    }
                    std::fs::read(&norm).ok()
                } else {
                    None
                };
                let mut combined = match host_prefix {
                    Some(mut h) => {
                        h.reserve(data_bytes.len());
                        h.extend_from_slice(&data_bytes);
                        h
                    }
                    None => data_bytes,
                };
                combined.shrink_to_fit();
                if let Err(msg) = try_consume_isolate_memory_ctx(combined.len(), context) {
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                BOA_VFS_STATE.with(|v| {
                    let mut vfs = v.borrow_mut();
                    vfs.ensure_parent_dirs(&norm);
                    let id = vfs.alloc_inode(VfsEntryKind::File(Arc::new(combined)), None, 1, 0);
                    vfs.paths.insert(norm, PathEntry::Present(id));
                });
            }
            Ok(BoaVal::from(0))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_exists_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_read_ocap(&path) {
                return Ok(BoaVal::from(false));
            }
            let allow_host = !is_host_unc_blocked(&path);
            let resolved = match resolve_effective_vfs_path(&path, true, allow_host) {
                Ok(res) => res,
                Err(SymlinkResolveError::ReadOcapDenied(denied)) => {
                    let _ = check_boa_fs_read_ocap(&denied);
                    return Ok(BoaVal::from(false));
                }
                Err(SymlinkResolveError::Loop) => return Ok(BoaVal::from(false)),
            };
            if resolved.traversed_symlink && !check_boa_fs_read_ocap(&resolved.norm) {
                return Ok(BoaVal::from(false));
            }
            if resolved.has_file_ancestor {
                return Ok(BoaVal::from(false));
            }
            let exists = BOA_VFS_STATE.with(|v| {
                let vfs = v.borrow();
                classify_path_stat(&vfs, &resolved.norm, allow_host).0 != ResolvedPathKind::Missing
            });
            Ok(BoaVal::from(exists))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_mkdir_sync"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let recursive = args.get(1).and_then(|v| v.as_boolean()).unwrap_or(false);
            if !check_boa_fs_write_ocap(&path) {
                return Ok(BoaVal::undefined());
            }
            let allow_host = !is_host_unc_blocked(&path);
            let Some(resolved) = resolve_or_js_err(&path, recursive, allow_host, "mkdir", context)?
            else {
                return Ok(BoaVal::undefined());
            };
            if resolved.traversed_symlink && !check_boa_fs_write_ocap(&resolved.norm) {
                return Ok(BoaVal::undefined());
            }
            if resolved.has_file_ancestor {
                return Ok(BoaVal::from(-20));
            }
            let norm = resolved.norm;
            let err_code = BOA_VFS_STATE.with(|v| {
                let mut vfs = v.borrow_mut();
                if !recursive
                    && classify_path_lstat(&vfs, &norm, allow_host) != ResolvedPathKind::Missing
                {
                    return Some(-17);
                }
                vfs.ensure_dir_with_ancestors(&norm, false);
                None
            });
            if let Some(code) = err_code {
                return Ok(BoaVal::from(code));
            }
            Ok(BoaVal::from(0))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_readdir_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_read_ocap(&path) {
                return Ok(BoaVal::from(js_string!("[]")));
            }
            let allow_host = !is_host_unc_blocked(&path);
            let Some(resolved) = resolve_or_js_err(&path, true, allow_host, "scandir", context)?
            else {
                return Ok(BoaVal::from(js_string!("[]")));
            };
            if resolved.traversed_symlink && !check_boa_fs_read_ocap(&resolved.norm) {
                return Ok(BoaVal::from(js_string!("[]")));
            }
            let norm_dir = resolved.norm;
            let prefix = dir_child_prefix(&norm_dir);
            let entries: Vec<String> = BOA_VFS_STATE.with(|v| {
                let vfs = v.borrow();
                let mut entries_set = std::collections::BTreeSet::new();
                for (k, entry) in &vfs.paths {
                    if !matches!(entry, PathEntry::Present(_)) {
                        continue;
                    }
                    if let Some(rest) = k.strip_prefix(&prefix)
                        && let Some(first_seg) = rest.split('/').next()
                        && !first_seg.is_empty()
                    {
                        entries_set.insert(first_seg.to_string());
                    }
                }
                let is_opaque = matches!(
                    vfs.lookup_present(&norm_dir).map(|(_, i)| &i.kind),
                    Some(VfsEntryKind::Directory { host_opaque: true })
                );
                if allow_host
                    && !is_opaque
                    && !vfs.is_host_masked(&norm_dir)
                    && has_boa_fs_read_ocap(&norm_dir)
                    && let Ok(rd) = std::fs::read_dir(&norm_dir)
                {
                    for entry in rd.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        let child_norm = if norm_dir.ends_with('/') {
                            format!("{}{}", norm_dir, name)
                        } else {
                            format!("{}/{}", norm_dir, name)
                        };
                        if vfs.is_host_masked(&child_norm) {
                            continue;
                        }
                        entries_set.insert(name);
                    }
                }
                entries_set.into_iter().collect()
            });
            let json = serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string());
            Ok(BoaVal::from(js_string!(json)))
        }),
    );

    // Unified single-crossing stat/lstat binding returning `[size, mtime_ms, kind_code]`
    // (`0 = file`, `1 = dir`, `2 = symlink`), `null` on `ENOENT`, or `undefined` on OCap deny.
    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_stat_info_sync"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let follow_final = args.get(1).and_then(|v| v.as_boolean()).unwrap_or(true);
            if !check_boa_fs_read_ocap(&path) {
                return Ok(BoaVal::undefined());
            }
            let allow_host = !is_host_unc_blocked(&path);
            let syscall = if follow_final { "stat" } else { "lstat" };
            let Some(resolved) =
                resolve_or_js_err(&path, follow_final, allow_host, syscall, context)?
            else {
                return Ok(BoaVal::undefined());
            };
            if resolved.traversed_symlink && !check_boa_fs_read_ocap(&resolved.norm) {
                return Ok(BoaVal::undefined());
            }
            if resolved.has_file_ancestor {
                return Ok(BoaVal::null());
            }
            let norm = resolved.norm;
            let info_opt: Option<(f64, f64, i32)> = BOA_VFS_STATE.with(|v| {
                let vfs = v.borrow();
                if follow_final {
                    let (kind, sz) = classify_path_stat(&vfs, &norm, allow_host);
                    match kind {
                        ResolvedPathKind::Missing | ResolvedPathKind::Symlink => None,
                        ResolvedPathKind::File => {
                            let mt = default_mtime_ms_for_path(&vfs, &norm, allow_host);
                            Some((sz as f64, mt, 0))
                        }
                        ResolvedPathKind::Directory => {
                            let mt = default_mtime_ms_for_path(&vfs, &norm, allow_host);
                            Some((0.0, mt, 1))
                        }
                    }
                } else {
                    match vfs.paths.get(&norm) {
                        Some(PathEntry::Tombstone) => None,
                        Some(PathEntry::Present(id)) => {
                            let ino = vfs.get_inode(*id)?;
                            let mt = default_mtime_ms_for_path(&vfs, &norm, allow_host);
                            match &ino.kind {
                                VfsEntryKind::File(b) => Some((b.len() as f64, mt, 0)),
                                VfsEntryKind::Directory { .. } => Some((0.0, mt, 1)),
                                VfsEntryKind::Symlink(t) => Some((t.len() as f64, mt, 2)),
                            }
                        }
                        None => {
                            if is_vfs_root_path(&norm) {
                                let mt = default_mtime_ms_for_path(&vfs, &norm, allow_host);
                                return Some((0.0, mt, 1));
                            }
                            if !allow_host
                                || vfs.is_host_masked(&norm)
                                || !has_boa_fs_read_ocap(&norm)
                            {
                                return None;
                            }
                            let meta = std::fs::symlink_metadata(&norm).ok()?;
                            let mt = if BOA_DET_STATE.with(|s| s.get().is_some()) {
                                1_700_000_000_000.0
                            } else {
                                meta.modified()
                                    .ok()
                                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                    .map(|d| d.as_millis() as f64)
                                    .unwrap_or(1_700_000_000_000.0)
                            };
                            if is_host_symlink_or_reparse(&meta) {
                                let sym_len = std::fs::read_link(&norm)
                                    .map(|p| normalize_host_readlink_target(&p).len() as f64)
                                    .unwrap_or_else(|_| meta.len() as f64);
                                Some((sym_len, mt, 2))
                            } else if meta.is_dir() {
                                Some((0.0, mt, 1))
                            } else {
                                Some((meta.len() as f64, mt, 0))
                            }
                        }
                    }
                }
            });
            match info_opt {
                Some((sz, mt, kind)) => {
                    let arr = JsArray::from_iter(
                        [BoaVal::from(sz), BoaVal::from(mt), BoaVal::from(kind)],
                        context,
                    );
                    Ok(arr.into())
                }
                None => Ok(BoaVal::null()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_stat_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_read_ocap(&path) {
                return Ok(BoaVal::from(0));
            }
            let allow_host = !is_host_unc_blocked(&path);
            let Some(resolved) = resolve_or_js_err(&path, true, allow_host, "stat", context)?
            else {
                return Ok(BoaVal::from(0));
            };
            if resolved.traversed_symlink && !check_boa_fs_read_ocap(&resolved.norm) {
                return Ok(BoaVal::from(0));
            }
            if resolved.has_file_ancestor {
                return Ok(BoaVal::from(-1.0));
            }
            let res = BOA_VFS_STATE.with(|v| {
                let vfs = v.borrow();
                let (kind, sz) = classify_path_stat(&vfs, &resolved.norm, allow_host);
                match kind {
                    ResolvedPathKind::File => sz as f64,
                    ResolvedPathKind::Directory => 0.0,
                    ResolvedPathKind::Missing | ResolvedPathKind::Symlink => -1.0,
                }
            });
            Ok(BoaVal::from(res))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_mtime_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_read_ocap(&path) {
                return Ok(BoaVal::from(1_700_000_000_000.0));
            }
            let allow_host = !is_host_unc_blocked(&path);
            let resolved = match resolve_effective_vfs_path(&path, true, allow_host) {
                Ok(res) => res,
                Err(SymlinkResolveError::ReadOcapDenied(denied)) => {
                    let _ = check_boa_fs_read_ocap(&denied);
                    return Ok(BoaVal::from(1_700_000_000_000.0));
                }
                Err(SymlinkResolveError::Loop) => return Ok(BoaVal::from(1_700_000_000_000.0)),
            };
            if resolved.traversed_symlink && !check_boa_fs_read_ocap(&resolved.norm) {
                return Ok(BoaVal::from(1_700_000_000_000.0));
            }
            let mt = BOA_VFS_STATE
                .with(|v| default_mtime_ms_for_path(&v.borrow(), &resolved.norm, allow_host));
            Ok(BoaVal::from(mt))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_is_dir_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_read_ocap(&path) {
                return Ok(BoaVal::from(false));
            }
            let allow_host = !is_host_unc_blocked(&path);
            let resolved = match resolve_effective_vfs_path(&path, true, allow_host) {
                Ok(res) => res,
                Err(SymlinkResolveError::ReadOcapDenied(denied)) => {
                    let _ = check_boa_fs_read_ocap(&denied);
                    return Ok(BoaVal::from(false));
                }
                Err(SymlinkResolveError::Loop) => return Ok(BoaVal::from(false)),
            };
            if resolved.traversed_symlink && !check_boa_fs_read_ocap(&resolved.norm) {
                return Ok(BoaVal::from(false));
            }
            if resolved.has_file_ancestor {
                return Ok(BoaVal::from(false));
            }
            let is_dir = BOA_VFS_STATE.with(|v| {
                classify_path_stat(&v.borrow(), &resolved.norm, allow_host).0
                    == ResolvedPathKind::Directory
            });
            Ok(BoaVal::from(is_dir))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_copy_file_sync"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let src = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let dest = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_read_ocap(&src) || !check_boa_fs_write_ocap(&dest) {
                return Ok(BoaVal::undefined());
            }
            let allow_host_src = !is_host_unc_blocked(&src);
            let allow_host_dest = !is_host_unc_blocked(&dest);
            let Some(res_src) = resolve_or_js_err(&src, true, allow_host_src, "copyfile", context)?
            else {
                return Ok(BoaVal::undefined());
            };
            let Some(res_dest) =
                resolve_or_js_err(&dest, true, allow_host_dest, "copyfile", context)?
            else {
                return Ok(BoaVal::undefined());
            };
            if (res_src.traversed_symlink && !check_boa_fs_read_ocap(&res_src.norm))
                || (res_dest.traversed_symlink && !check_boa_fs_write_ocap(&res_dest.norm))
            {
                return Ok(BoaVal::undefined());
            }
            if res_src.has_file_ancestor || res_dest.has_file_ancestor {
                return Ok(BoaVal::from(-20));
            }
            let norm_src = res_src.norm;
            let norm_dest = res_dest.norm;
            let pre_check = BOA_VFS_STATE.with(|v| {
                let vfs = v.borrow();
                let src_kind = classify_path_stat(&vfs, &norm_src, allow_host_src).0;
                if src_kind == ResolvedPathKind::Missing {
                    return Err(-2);
                }
                if src_kind == ResolvedPathKind::Directory {
                    return Err(-21);
                }
                let dst_kind = classify_path_stat(&vfs, &norm_dest, allow_host_dest).0;
                if dst_kind == ResolvedPathKind::Directory {
                    return Err(-21);
                }
                let vfs_src_arc = match vfs.lookup_present(&norm_src).map(|(_, i)| &i.kind) {
                    Some(VfsEntryKind::File(b)) => Some(Arc::clone(b)),
                    _ => None,
                };
                let dst_existing = vfs
                    .lookup_present(&norm_dest)
                    .map(|(id, i)| (id, i.accounted_bytes));
                Ok((vfs_src_arc, dst_existing))
            });
            let (vfs_src_arc, dst_existing) = match pre_check {
                Ok(pair) => pair,
                Err(errno) => return Ok(BoaVal::from(errno)),
            };
            if norm_src == norm_dest {
                return Ok(BoaVal::from(0));
            }
            let content_arc = match vfs_src_arc {
                Some(arc) => arc,
                None => {
                    if !check_boa_fs_read_ocap(&norm_src) {
                        return Ok(BoaVal::undefined());
                    }
                    let Ok(bytes) = std::fs::read(&norm_src) else {
                        return Ok(BoaVal::from(-2));
                    };
                    Arc::new(bytes)
                }
            };
            if let Some((dst_id, prev_acc)) = dst_existing {
                let delta = content_arc.len() as isize - prev_acc as isize;
                if let Err(msg) = try_adjust_isolate_memory_ctx(delta, context) {
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                BOA_VFS_STATE.with(|v| {
                    v.borrow_mut()
                        .update_inode_kind(dst_id, VfsEntryKind::File(content_arc));
                });
            } else {
                if let Err(msg) = try_consume_isolate_memory_ctx(content_arc.len(), context) {
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                BOA_VFS_STATE.with(|v| {
                    let mut vfs = v.borrow_mut();
                    vfs.ensure_parent_dirs(&norm_dest);
                    let id = vfs.alloc_inode(VfsEntryKind::File(content_arc), None, 1, 0);
                    vfs.paths.insert(norm_dest, PathEntry::Present(id));
                });
            }
            Ok(BoaVal::from(0))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_rename_sync"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let old_p = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let new_p = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_read_ocap(&old_p)
                || !check_boa_fs_write_ocap(&old_p)
                || !check_boa_fs_write_ocap(&new_p)
            {
                return Ok(BoaVal::undefined());
            }
            let allow_host_old = !is_host_unc_blocked(&old_p);
            let allow_host_new = !is_host_unc_blocked(&new_p);
            // Resolve intermediate parent directory symlinks ONLY (`follow_final = false`) so
            // renaming a symlink moves the directory entry itself rather than its target (#77).
            let Some(res_old) =
                resolve_or_js_err(&old_p, false, allow_host_old, "rename", context)?
            else {
                return Ok(BoaVal::undefined());
            };
            let Some(res_new) =
                resolve_or_js_err(&new_p, false, allow_host_new, "rename", context)?
            else {
                return Ok(BoaVal::undefined());
            };
            if (res_old.traversed_symlink
                && (!check_boa_fs_read_ocap(&res_old.norm)
                    || !check_boa_fs_write_ocap(&res_old.norm)))
                || (res_new.traversed_symlink && !check_boa_fs_write_ocap(&res_new.norm))
            {
                return Ok(BoaVal::undefined());
            }
            if res_old.has_file_ancestor || res_new.has_file_ancestor {
                return Ok(BoaVal::from(-20)); // ENOTDIR
            }
            let norm_old = res_old.norm;
            let norm_new = res_new.norm;
            let (src_kind, dst_kind) = BOA_VFS_STATE.with(|v| {
                let vfs = v.borrow();
                (
                    classify_path_lstat(&vfs, &norm_old, allow_host_old),
                    classify_path_lstat(&vfs, &norm_new, allow_host_new),
                )
            });
            if src_kind == ResolvedPathKind::Missing {
                return Ok(BoaVal::from(-2)); // ENOENT
            }
            if norm_old == norm_new {
                return Ok(BoaVal::from(0));
            }

            // Branch A: Source is a regular file or symlink (either VFS or host)
            if matches!(src_kind, ResolvedPathKind::File | ResolvedPathKind::Symlink) {
                if dst_kind == ResolvedPathKind::Directory {
                    return Ok(BoaVal::from(-21)); // EISDIR (#109)
                }
                let vfs_src = BOA_VFS_STATE.with(|v| {
                    let vfs = v.borrow();
                    vfs.lookup_present(&norm_old).map(|(id, ino)| {
                        let sym_target = match &ino.kind {
                            VfsEntryKind::Symlink(t) => Some(t.clone()),
                            _ => None,
                        };
                        (id, sym_target)
                    })
                });

                if let Some((src_id, sym_target_opt)) = vfs_src {
                    if let Some(ref raw_target) = sym_target_opt {
                        let resolved_new_target =
                            resolve_symlink_target_against_link(&norm_new, raw_target);
                        if !check_boa_fs_read_ocap(&resolved_new_target) {
                            return Ok(BoaVal::undefined());
                        }
                    }
                    let freed_dst = BOA_VFS_STATE.with(|v| {
                        let mut vfs = v.borrow_mut();
                        let freed = match vfs.paths.remove(&norm_new) {
                            Some(PathEntry::Present(old_dst_id)) => {
                                vfs.unlink_inode_ref(old_dst_id)
                            }
                            _ => 0,
                        };
                        vfs.paths.remove(&norm_old);
                        if allow_host_old && std::fs::symlink_metadata(&norm_old).is_ok() {
                            vfs.paths.insert(norm_old, PathEntry::Tombstone);
                        }
                        vfs.ensure_parent_dirs(&norm_new);
                        vfs.paths.insert(norm_new, PathEntry::Present(src_id));
                        freed
                    });
                    if freed_dst > 0 {
                        release_isolate_memory(freed_dst);
                    }
                    return Ok(BoaVal::from(0));
                }

                // CoW rename of a host symlink or host regular file
                if src_kind == ResolvedPathKind::Symlink {
                    let Ok(target_path) = std::fs::read_link(&norm_old) else {
                        return Ok(BoaVal::from(-2));
                    };
                    let target_str = normalize_host_readlink_target(&target_path);
                    let resolved_orig_target =
                        resolve_symlink_target_against_link(&norm_old, &target_str);
                    let resolved_new_target =
                        resolve_symlink_target_against_link(&norm_new, &target_str);
                    if !check_boa_fs_read_ocap(&resolved_orig_target)
                        || !check_boa_fs_read_ocap(&resolved_new_target)
                    {
                        return Ok(BoaVal::undefined());
                    }
                    let existing_dst_acc = BOA_VFS_STATE.with(|v| {
                        v.borrow()
                            .lookup_present(&norm_new)
                            .map(|(_, i)| i.accounted_bytes)
                            .unwrap_or(0)
                    });
                    let delta = target_str.len() as isize - existing_dst_acc as isize;
                    if let Err(msg) = try_adjust_isolate_memory_ctx(delta, context) {
                        return Err(JsNativeError::range().with_message(msg).into());
                    }
                    BOA_VFS_STATE.with(|v| {
                        let mut vfs = v.borrow_mut();
                        if let Some(PathEntry::Present(old_dst_id)) = vfs.paths.remove(&norm_new) {
                            let _ = vfs.unlink_inode_ref(old_dst_id);
                        }
                        vfs.paths.insert(norm_old, PathEntry::Tombstone);
                        vfs.ensure_parent_dirs(&norm_new);
                        let id = vfs.alloc_inode(VfsEntryKind::Symlink(target_str), None, 1, 0);
                        vfs.paths.insert(norm_new, PathEntry::Present(id));
                    });
                    return Ok(BoaVal::from(0));
                }

                let Ok(bytes) = std::fs::read(&norm_old) else {
                    return Ok(BoaVal::from(-2));
                };
                let existing_dst_acc = BOA_VFS_STATE.with(|v| {
                    v.borrow()
                        .lookup_present(&norm_new)
                        .map(|(_, i)| i.accounted_bytes)
                        .unwrap_or(0)
                });
                let delta = bytes.len() as isize - existing_dst_acc as isize;
                if let Err(msg) = try_adjust_isolate_memory_ctx(delta, context) {
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                BOA_VFS_STATE.with(|v| {
                    let mut vfs = v.borrow_mut();
                    if let Some(PathEntry::Present(old_dst_id)) = vfs.paths.remove(&norm_new) {
                        let _ = vfs.unlink_inode_ref(old_dst_id);
                    }
                    vfs.paths.insert(norm_old, PathEntry::Tombstone);
                    vfs.ensure_parent_dirs(&norm_new);
                    let id = vfs.alloc_inode(VfsEntryKind::File(Arc::new(bytes)), None, 1, 0);
                    vfs.paths.insert(norm_new, PathEntry::Present(id));
                });
                return Ok(BoaVal::from(0));
            }

            // Branch B: Source is a directory (VFS or host)
            let old_prefix = dir_child_prefix(&norm_old);
            if is_vfs_root_path(&norm_old) || norm_new.starts_with(&old_prefix) {
                return Ok(BoaVal::from(-22)); // EINVAL
            }
            if matches!(dst_kind, ResolvedPathKind::File | ResolvedPathKind::Symlink) {
                return Ok(BoaVal::from(-20)); // ENOTDIR
            }
            let dst_non_empty = BOA_VFS_STATE
                .with(|v| dir_has_any_children(&v.borrow(), &norm_new, allow_host_new));
            if dst_kind == ResolvedPathKind::Directory && dst_non_empty {
                return Ok(BoaVal::from(-39)); // ENOTEMPTY
            }

            let mat_res = BOA_VFS_STATE.with(|v| -> Result<(), String> {
                let mut vfs = v.borrow_mut();
                let host_old_is_dir = allow_host_old
                    && !vfs.is_host_masked(&norm_old)
                    && std::fs::symlink_metadata(&norm_old)
                        .map(|m| m.is_dir())
                        .unwrap_or(false);
                if host_old_is_dir {
                    materialize_host_dir_into_vfs(&mut vfs, &norm_old)?;
                }

                if let Some(PathEntry::Present(old_dst_id)) = vfs.paths.remove(&norm_new) {
                    let _ = vfs.unlink_inode_ref(old_dst_id);
                }

                let moved_children: Vec<(String, PathEntry)> = vfs
                    .paths
                    .keys()
                    .filter(|k| k.starts_with(&old_prefix))
                    .cloned()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .filter_map(|k| vfs.paths.remove(&k).map(|e| (k, e)))
                    .collect();

                let old_dir_entry = vfs.paths.remove(&norm_old);
                if host_old_is_dir
                    || (allow_host_old && std::fs::symlink_metadata(&norm_old).is_ok())
                {
                    vfs.paths.insert(norm_old.clone(), PathEntry::Tombstone);
                }

                vfs.ensure_parent_dirs(&norm_new);
                let dst_opaque = true;
                let dir_id = match old_dir_entry {
                    Some(PathEntry::Present(existing_dir_id)) => {
                        vfs.update_inode_kind(
                            existing_dir_id,
                            VfsEntryKind::Directory {
                                host_opaque: dst_opaque,
                            },
                        );
                        existing_dir_id
                    }
                    _ => vfs.alloc_inode(
                        VfsEntryKind::Directory {
                            host_opaque: dst_opaque,
                        },
                        None,
                        1,
                        0,
                    ),
                };
                vfs.paths
                    .insert(norm_new.clone(), PathEntry::Present(dir_id));

                for (old_k, entry) in moved_children {
                    let rel = &old_k[old_prefix.len()..];
                    let new_k = format!("{}/{}", norm_new, rel);
                    vfs.paths.insert(new_k, entry);
                }
                Ok(())
            });
            if let Err(msg) = mat_res {
                trip_isolate_oom_limits(context);
                return Err(JsNativeError::range().with_message(msg).into());
            }
            Ok(BoaVal::from(0))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_rm_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_write_ocap(&path) {
                return Ok(BoaVal::undefined());
            }
            let allow_host = !is_host_unc_blocked(&path);
            let Some(resolved) = resolve_or_js_err(&path, false, allow_host, "rm", context)? else {
                return Ok(BoaVal::undefined());
            };
            if resolved.traversed_symlink && !check_boa_fs_write_ocap(&resolved.norm) {
                return Ok(BoaVal::undefined());
            }
            let norm = resolved.norm;
            let prefix = dir_child_prefix(&norm);
            let freed_total = BOA_VFS_STATE.with(|v| {
                let mut vfs = v.borrow_mut();
                let mut freed = 0usize;
                if let Some(PathEntry::Present(id)) = vfs.paths.remove(&norm) {
                    freed = freed.saturating_add(vfs.unlink_inode_ref(id));
                }
                let child_keys: Vec<String> = vfs
                    .paths
                    .keys()
                    .filter(|k| k.starts_with(&prefix))
                    .cloned()
                    .collect();
                for ck in child_keys {
                    if let Some(PathEntry::Present(cid)) = vfs.paths.remove(&ck) {
                        freed = freed.saturating_add(vfs.unlink_inode_ref(cid));
                    }
                }
                if allow_host && std::fs::symlink_metadata(&norm).is_ok() {
                    vfs.paths.insert(norm, PathEntry::Tombstone);
                }
                freed
            });
            if freed_total > 0 {
                release_isolate_memory(freed_total);
            }
            Ok(BoaVal::from(0))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_realpath_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_read_ocap(&path) {
                return Ok(BoaVal::from(js_string!("")));
            }
            let allow_host = !is_host_unc_blocked(&path);
            let Some(resolved) = resolve_or_js_err(&path, true, allow_host, "realpath", context)?
            else {
                return Ok(BoaVal::from(js_string!("")));
            };
            if resolved.traversed_symlink && !check_boa_fs_read_ocap(&resolved.norm) {
                return Ok(BoaVal::from(js_string!("")));
            }
            if resolved.has_file_ancestor {
                return Ok(BoaVal::from(js_string!("")));
            }
            let exists = BOA_VFS_STATE.with(|v| {
                classify_path_stat(&v.borrow(), &resolved.norm, allow_host).0
                    != ResolvedPathKind::Missing
            });
            if !exists {
                return Ok(BoaVal::from(js_string!("")));
            }
            Ok(BoaVal::from(js_string!(resolved.norm)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_mkdtemp_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let prefix = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "/tmp/slice-".to_string());
            let mut rnd = [0u8; 6];
            BOA_DET_STATE.with(|s| {
                if let Some((mut prng, clock)) = s.get() {
                    fill_random_bytes(&mut rnd, Some(&mut prng));
                    s.set(Some((prng, clock)));
                } else {
                    fill_random_bytes(&mut rnd, None);
                }
            });
            const ALPH: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
            let mut suffix = String::with_capacity(6);
            for &b in &rnd {
                suffix.push(ALPH[(b as usize) % ALPH.len()] as char);
            }
            let full_path = format!("{}{}", prefix, suffix);
            if !check_boa_fs_write_ocap(&full_path) {
                return Ok(BoaVal::from(js_string!("")));
            }
            let allow_host = !is_host_unc_blocked(&full_path);
            let Some(resolved) =
                resolve_or_js_err(&full_path, false, allow_host, "mkdtemp", context)?
            else {
                return Ok(BoaVal::from(js_string!("")));
            };
            if resolved.traversed_symlink && !check_boa_fs_write_ocap(&resolved.norm) {
                return Ok(BoaVal::from(js_string!("")));
            }
            let ret_path = if resolved.traversed_symlink {
                resolved.norm.clone()
            } else {
                norm_vfs_path(&full_path)
            };
            BOA_VFS_STATE.with(|v| {
                v.borrow_mut()
                    .ensure_dir_with_ancestors(&resolved.norm, true);
            });
            Ok(BoaVal::from(js_string!(ret_path)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_chmod_sync"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_write_ocap(&path) {
                return Ok(BoaVal::undefined());
            }
            let allow_host = !is_host_unc_blocked(&path);
            let Some(resolved) = resolve_or_js_err(&path, true, allow_host, "chmod", context)?
            else {
                return Ok(BoaVal::undefined());
            };
            if resolved.traversed_symlink && !check_boa_fs_write_ocap(&resolved.norm) {
                return Ok(BoaVal::undefined());
            }
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_symlink_sync"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let target = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let link_path = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_write_ocap(&link_path) {
                return Ok(BoaVal::undefined());
            }
            let allow_host = !is_host_unc_blocked(&link_path);
            let Some(resolved_link) =
                resolve_or_js_err(&link_path, false, allow_host, "symlink", context)?
            else {
                return Ok(BoaVal::undefined());
            };
            if resolved_link.traversed_symlink && !check_boa_fs_write_ocap(&resolved_link.norm) {
                return Ok(BoaVal::undefined());
            }
            let norm_link = resolved_link.norm;
            if target.as_bytes().contains(&0) || is_host_unc_blocked(&target) {
                let _ = check_boa_fs_read_ocap(&target);
                return Ok(BoaVal::undefined());
            }
            let resolved_target = resolve_symlink_target_against_link(&norm_link, &target);
            if !check_boa_fs_read_ocap(&resolved_target) {
                return Ok(BoaVal::undefined());
            }
            if resolved_link.has_file_ancestor {
                return Ok(BoaVal::from(-20));
            }
            let exists_lstat = BOA_VFS_STATE.with(|v| {
                classify_path_lstat(&v.borrow(), &norm_link, allow_host)
                    != ResolvedPathKind::Missing
            });
            if exists_lstat {
                return Ok(BoaVal::from(-17));
            }
            let stored_target = target.trim().to_string();
            if let Err(msg) = try_consume_isolate_memory_ctx(stored_target.len(), context) {
                return Err(JsNativeError::range().with_message(msg).into());
            }
            BOA_VFS_STATE.with(|v| {
                let mut vfs = v.borrow_mut();
                vfs.ensure_parent_dirs(&norm_link);
                let id = vfs.alloc_inode(VfsEntryKind::Symlink(stored_target), None, 1, 0);
                vfs.paths.insert(norm_link, PathEntry::Present(id));
            });
            Ok(BoaVal::from(0))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_is_symlink_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_read_ocap(&path) {
                return Ok(BoaVal::from(false));
            }
            let allow_host = !is_host_unc_blocked(&path);
            let resolved = match resolve_effective_vfs_path(&path, false, allow_host) {
                Ok(res) => res,
                Err(SymlinkResolveError::ReadOcapDenied(denied)) => {
                    let _ = check_boa_fs_read_ocap(&denied);
                    return Ok(BoaVal::from(false));
                }
                Err(SymlinkResolveError::Loop) => return Ok(BoaVal::from(false)),
            };
            if resolved.traversed_symlink && !check_boa_fs_read_ocap(&resolved.norm) {
                return Ok(BoaVal::from(false));
            }
            if resolved.has_file_ancestor {
                return Ok(BoaVal::from(false));
            }
            let is_sym = BOA_VFS_STATE.with(|v| {
                classify_path_lstat(&v.borrow(), &resolved.norm, allow_host)
                    == ResolvedPathKind::Symlink
            });
            Ok(BoaVal::from(is_sym))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_readlink_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_read_ocap(&path) {
                return Ok(BoaVal::from(js_string!("")));
            }
            let allow_host = !is_host_unc_blocked(&path);
            let Some(resolved) = resolve_or_js_err(&path, false, allow_host, "readlink", context)?
            else {
                return Ok(BoaVal::from(js_string!("")));
            };
            if resolved.traversed_symlink && !check_boa_fs_read_ocap(&resolved.norm) {
                return Ok(BoaVal::from(js_string!("")));
            }
            if resolved.has_file_ancestor {
                return Ok(BoaVal::from(-20));
            }
            let norm = resolved.norm;
            enum ReadlinkState {
                Target(String),
                NotSymlink,
                Missing,
            }
            let state = BOA_VFS_STATE.with(|v| {
                let vfs = v.borrow();
                match vfs.paths.get(&norm) {
                    Some(PathEntry::Tombstone) => ReadlinkState::Missing,
                    Some(PathEntry::Present(id)) => match vfs.get_inode(*id).map(|i| &i.kind) {
                        Some(VfsEntryKind::Symlink(t)) => ReadlinkState::Target(t.clone()),
                        Some(_) => ReadlinkState::NotSymlink,
                        None => ReadlinkState::Missing,
                    },
                    None => {
                        if !allow_host || vfs.is_host_masked(&norm) || !has_boa_fs_read_ocap(&norm)
                        {
                            return ReadlinkState::Missing;
                        }
                        match std::fs::symlink_metadata(&norm) {
                            Ok(m) if is_host_symlink_or_reparse(&m) => std::fs::read_link(&norm)
                                .map(|p| ReadlinkState::Target(normalize_host_readlink_target(&p)))
                                .unwrap_or(ReadlinkState::Missing),
                            Ok(_) => ReadlinkState::NotSymlink,
                            Err(_) => ReadlinkState::Missing,
                        }
                    }
                }
            });
            match state {
                ReadlinkState::Target(target) => {
                    let resolved_target = resolve_symlink_target_against_link(&norm, &target);
                    if !check_boa_fs_read_ocap(&resolved_target) {
                        return Ok(BoaVal::from(js_string!("")));
                    }
                    Ok(BoaVal::from(js_string!(target)))
                }
                ReadlinkState::NotSymlink => Ok(BoaVal::from(-22)),
                ReadlinkState::Missing => Ok(BoaVal::from(js_string!(""))),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_truncate_sync"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let new_len = args
                .get(1)
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as usize)
                .unwrap_or(0);
            if !check_boa_fs_read_ocap(&path) || !check_boa_fs_write_ocap(&path) {
                return Ok(BoaVal::undefined());
            }
            let allow_host = !is_host_unc_blocked(&path);
            let Some(resolved) = resolve_or_js_err(&path, true, allow_host, "truncate", context)?
            else {
                return Ok(BoaVal::undefined());
            };
            if resolved.traversed_symlink
                && (!check_boa_fs_read_ocap(&resolved.norm)
                    || !check_boa_fs_write_ocap(&resolved.norm))
            {
                return Ok(BoaVal::undefined());
            }
            if resolved.has_file_ancestor {
                return Ok(BoaVal::from(-20));
            }
            let norm = resolved.norm;
            let existing = BOA_VFS_STATE.with(|v| {
                let vfs = v.borrow();
                let (kind, _) = classify_path_stat(&vfs, &norm, allow_host);
                match kind {
                    ResolvedPathKind::Missing | ResolvedPathKind::Symlink => Err(-2),
                    ResolvedPathKind::Directory => Err(-21),
                    ResolvedPathKind::File => Ok(vfs
                        .lookup_present(&norm)
                        .map(|(id, i)| (id, i.accounted_bytes))),
                }
            });
            let existing_opt = match existing {
                Ok(opt) => opt,
                Err(code) => return Ok(BoaVal::from(code)),
            };
            if let Some((id, prev_acc)) = existing_opt {
                let delta = new_len as isize - prev_acc as isize;
                if let Err(msg) = try_adjust_isolate_memory_ctx(delta, context) {
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                BOA_VFS_STATE.with(|v| {
                    let mut vfs = v.borrow_mut();
                    let mut arc_buf = match vfs.get_inode(id).map(|i| &i.kind) {
                        Some(VfsEntryKind::File(b)) => Arc::clone(b),
                        _ => Arc::new(Vec::new()),
                    };
                    Arc::make_mut(&mut arc_buf).resize(new_len, 0u8);
                    vfs.update_inode_kind(id, VfsEntryKind::File(arc_buf));
                });
            } else {
                if !check_boa_fs_read_ocap(&norm) {
                    return Ok(BoaVal::undefined());
                }
                let Ok(mut data) = std::fs::read(&norm) else {
                    return Ok(BoaVal::from(-2));
                };
                if let Err(msg) = try_consume_isolate_memory_ctx(new_len, context) {
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                data.resize(new_len, 0u8);
                BOA_VFS_STATE.with(|v| {
                    let mut vfs = v.borrow_mut();
                    vfs.ensure_parent_dirs(&norm);
                    let id = vfs.alloc_inode(VfsEntryKind::File(Arc::new(data)), None, 1, 0);
                    vfs.paths.insert(norm, PathEntry::Present(id));
                });
            }
            Ok(BoaVal::from(0))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_utimes_sync"),
        4,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let follow_final = args.get(3).and_then(|v| v.as_boolean()).unwrap_or(true);
            if !check_boa_fs_write_ocap(&path) {
                return Ok(BoaVal::undefined());
            }
            let allow_host = !is_host_unc_blocked(&path);
            let Some(resolved) =
                resolve_or_js_err(&path, follow_final, allow_host, "utime", context)?
            else {
                return Ok(BoaVal::undefined());
            };
            if resolved.traversed_symlink && !check_boa_fs_write_ocap(&resolved.norm) {
                return Ok(BoaVal::undefined());
            }
            if resolved.has_file_ancestor {
                return Ok(BoaVal::from(-20));
            }
            let norm = resolved.norm;
            let mtime_ms = args
                .get(2)
                .and_then(|v| v.as_number())
                .unwrap_or(1_700_000_000_000.0);
            enum UtimeState {
                Present(crate::bindings::fs::state::InodeId),
                HostDir,
                HostFile,
                HostSymlink,
                Missing,
            }
            let st = BOA_VFS_STATE.with(|v| {
                let vfs = v.borrow();
                if let Some((id, _)) = vfs.lookup_present(&norm) {
                    return UtimeState::Present(id);
                }
                let kind = if follow_final {
                    classify_path_stat(&vfs, &norm, allow_host).0
                } else {
                    classify_path_lstat(&vfs, &norm, allow_host)
                };
                match kind {
                    ResolvedPathKind::Directory => UtimeState::HostDir,
                    ResolvedPathKind::File => UtimeState::HostFile,
                    ResolvedPathKind::Symlink => UtimeState::HostSymlink,
                    ResolvedPathKind::Missing => UtimeState::Missing,
                }
            });
            match st {
                UtimeState::Present(id) => {
                    BOA_VFS_STATE.with(|v| {
                        if let Some(ino) = v.borrow_mut().get_inode_mut(id) {
                            ino.mtime_ms = Some(mtime_ms);
                        }
                    });
                }
                UtimeState::HostDir => {
                    BOA_VFS_STATE.with(|v| {
                        let mut vfs = v.borrow_mut();
                        vfs.ensure_dir_with_ancestors(&norm, false);
                        if let Some((id, _)) = vfs.lookup_present(&norm)
                            && let Some(ino) = vfs.get_inode_mut(id)
                        {
                            ino.mtime_ms = Some(mtime_ms);
                        }
                    });
                }
                UtimeState::HostSymlink => {
                    if !check_boa_fs_read_ocap(&norm) {
                        return Ok(BoaVal::undefined());
                    }
                    let Ok(target_path) = std::fs::read_link(&norm) else {
                        return Ok(BoaVal::from(-2));
                    };
                    let target_str = normalize_host_readlink_target(&target_path);
                    let resolved_target = resolve_symlink_target_against_link(&norm, &target_str);
                    if !check_boa_fs_read_ocap(&resolved_target) {
                        return Ok(BoaVal::undefined());
                    }
                    if let Err(msg) = try_consume_isolate_memory_ctx(target_str.len(), context) {
                        return Err(JsNativeError::range().with_message(msg).into());
                    }
                    BOA_VFS_STATE.with(|v| {
                        let mut vfs = v.borrow_mut();
                        vfs.ensure_parent_dirs(&norm);
                        let id = vfs.alloc_inode(
                            VfsEntryKind::Symlink(target_str),
                            Some(mtime_ms),
                            1,
                            0,
                        );
                        vfs.paths.insert(norm, PathEntry::Present(id));
                    });
                }
                UtimeState::HostFile => {
                    if !check_boa_fs_read_ocap(&norm) {
                        return Ok(BoaVal::undefined());
                    }
                    let Ok(bytes) = std::fs::read(&norm) else {
                        return Ok(BoaVal::from(-2));
                    };
                    if let Err(msg) = try_consume_isolate_memory_ctx(bytes.len(), context) {
                        return Err(JsNativeError::range().with_message(msg).into());
                    }
                    BOA_VFS_STATE.with(|v| {
                        let mut vfs = v.borrow_mut();
                        vfs.ensure_parent_dirs(&norm);
                        let id = vfs.alloc_inode(
                            VfsEntryKind::File(Arc::new(bytes)),
                            Some(mtime_ms),
                            1,
                            0,
                        );
                        vfs.paths.insert(norm, PathEntry::Present(id));
                    });
                }
                UtimeState::Missing => return Ok(BoaVal::from(-2)),
            }
            Ok(BoaVal::from(0))
        }),
    );
}
