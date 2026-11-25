//! Symlink (`symlinkSync`, `readlinkSync`, `realpathSync`), `truncateSync`, `utimesSync`, and `chmodSync` Bindings.

use crate::bindings::fs::classify_and_cow::{
    ResolvedPathKind, SymlinkResolveError, classify_path_lstat, classify_path_stat,
    is_host_symlink_or_reparse, is_host_unc_blocked, normalize_host_readlink_target,
    resolve_effective_vfs_path, resolve_or_js_err, resolve_symlink_target_against_link,
};
use crate::bindings::fs::state::{InodeId, PathEntry, VfsEntryKind};
use crate::bindings::helpers::{
    last_ocap_violation_message, ocap_denied_js_error, try_adjust_isolate_memory_ctx,
    try_consume_isolate_memory_ctx,
};
use crate::boa_realm::{
    BOA_VFS_STATE, check_boa_fs_read_ocap, check_boa_fs_write_ocap, has_boa_fs_read_ocap,
};
use std::sync::Arc;

pub(super) fn register_fs_path_link_meta_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{JsNativeError, JsValue as BoaVal, NativeFunction, js_string};

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
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
            }
            let allow_host = !is_host_unc_blocked(&path);
            let Some(resolved) = resolve_or_js_err(&path, true, allow_host, "realpath", context)?
            else {
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
            };
            if resolved.traversed_symlink && !check_boa_fs_read_ocap(&resolved.norm) {
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
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
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
            }
            let allow_host = !is_host_unc_blocked(&path);
            let Some(resolved) = resolve_or_js_err(&path, true, allow_host, "chmod", context)?
            else {
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
            };
            if resolved.traversed_symlink && !check_boa_fs_write_ocap(&resolved.norm) {
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
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
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
            }
            let allow_host = !is_host_unc_blocked(&link_path);
            let Some(resolved_link) =
                resolve_or_js_err(&link_path, false, allow_host, "symlink", context)?
            else {
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
            };
            if resolved_link.traversed_symlink && !check_boa_fs_write_ocap(&resolved_link.norm) {
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
            }
            let norm_link = resolved_link.norm;
            if target.as_bytes().contains(&0) || is_host_unc_blocked(&target) {
                let _ = check_boa_fs_read_ocap(&target);
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
            }
            let resolved_target = resolve_symlink_target_against_link(&norm_link, &target);
            if !check_boa_fs_read_ocap(&resolved_target) {
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
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
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
            }
            let allow_host = !is_host_unc_blocked(&path);
            let Some(resolved) = resolve_or_js_err(&path, false, allow_host, "readlink", context)?
            else {
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
            };
            if resolved.traversed_symlink && !check_boa_fs_read_ocap(&resolved.norm) {
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
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
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
            }
            let allow_host = !is_host_unc_blocked(&path);
            let Some(resolved) = resolve_or_js_err(&path, true, allow_host, "truncate", context)?
            else {
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
            };
            if resolved.traversed_symlink
                && (!check_boa_fs_read_ocap(&resolved.norm)
                    || !check_boa_fs_write_ocap(&resolved.norm))
            {
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
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
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
            }
            let allow_host = !is_host_unc_blocked(&path);
            let Some(resolved) =
                resolve_or_js_err(&path, follow_final, allow_host, "utime", context)?
            else {
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
            };
            if resolved.traversed_symlink && !check_boa_fs_write_ocap(&resolved.norm) {
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
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
                Present(InodeId),
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
