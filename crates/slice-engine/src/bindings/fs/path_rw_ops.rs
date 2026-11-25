//! Path-Based File Read, Write, Append, and Copy Host Bindings (`readFileSync`, `writeFileSync`, `appendFileSync`, `copyFileSync`).

use crate::bindings::fs::classify_and_cow::{
    ResolvedPathKind, classify_path_stat, is_host_symlink_or_reparse, is_host_unc_blocked,
    resolve_or_js_err, utf8_vec_to_js_val,
};
use crate::bindings::fs::state::{PathEntry, VfsEntryKind, is_vfs_root_path};
use crate::bindings::helpers::{
    bytes_to_js_uint8_array, extract_buffer_source_bytes, last_ocap_violation_message,
    ocap_denied_js_error, try_adjust_isolate_memory_ctx, try_consume_isolate_memory_ctx,
    utf8_slice_to_js_val,
};
use crate::boa_realm::{BOA_VFS_STATE, check_boa_fs_read_ocap, check_boa_fs_write_ocap};
use std::sync::Arc;

enum PathFileReadResult {
    OcapDenied,
    Errno(i32),
    Missing,
    VfsBytes(Arc<Vec<u8>>),
    HostBytes(Vec<u8>),
}

/// Shared OCap + symlink + VFS/host file reader used by `__slice_fs_read_file_sync` and `__slice_fs_read_bytes_sync`.
fn read_vfs_or_host_file_bytes(
    path: &str,
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<PathFileReadResult> {
    if !check_boa_fs_read_ocap(path) {
        return Ok(PathFileReadResult::OcapDenied);
    }
    let allow_host = !is_host_unc_blocked(path);
    let Some(resolved) = resolve_or_js_err(path, true, allow_host, "open", context)? else {
        return Ok(PathFileReadResult::OcapDenied);
    };
    if resolved.traversed_symlink && !check_boa_fs_read_ocap(&resolved.norm) {
        return Ok(PathFileReadResult::OcapDenied);
    }
    if resolved.has_file_ancestor {
        return Ok(PathFileReadResult::Errno(-20));
    }
    let norm = resolved.norm;
    enum LookupStep {
        VfsFile(Arc<Vec<u8>>),
        IsDir,
        HostCandidate,
        Missing,
    }
    let step = BOA_VFS_STATE.with(|v| {
        let vfs = v.borrow();
        match vfs.paths.get(&norm) {
            Some(PathEntry::Tombstone) => LookupStep::Missing,
            Some(PathEntry::Present(id)) => match vfs.get_inode(*id).map(|i| &i.kind) {
                Some(VfsEntryKind::File(b)) => LookupStep::VfsFile(Arc::clone(b)),
                Some(VfsEntryKind::Directory { .. }) => LookupStep::IsDir,
                _ => LookupStep::Missing,
            },
            None => {
                if is_vfs_root_path(&norm) {
                    LookupStep::IsDir
                } else if allow_host && !vfs.is_host_masked(&norm) {
                    LookupStep::HostCandidate
                } else {
                    LookupStep::Missing
                }
            }
        }
    });
    match step {
        LookupStep::VfsFile(b) => Ok(PathFileReadResult::VfsBytes(b)),
        LookupStep::IsDir => Ok(PathFileReadResult::Errno(-21)),
        LookupStep::Missing => Ok(PathFileReadResult::Missing),
        LookupStep::HostCandidate => {
            if let Ok(meta) = std::fs::symlink_metadata(&norm) {
                if is_host_symlink_or_reparse(&meta) {
                    return Ok(PathFileReadResult::Missing);
                }
                if meta.is_dir() {
                    return Ok(PathFileReadResult::Errno(-21));
                }
            }
            match std::fs::read(&norm) {
                Ok(bytes) => Ok(PathFileReadResult::HostBytes(bytes)),
                Err(_) => Ok(PathFileReadResult::Missing),
            }
        }
    }
}

pub(super) fn register_fs_path_rw_host_functions(ctx: &mut boa_engine::Context) {
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
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
            }
            let allow_host = !is_host_unc_blocked(&path);
            let Some(resolved) =
                resolve_or_js_err(&path, follow_final, allow_host, "open", context)?
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
            match read_vfs_or_host_file_bytes(&path, context)? {
                PathFileReadResult::OcapDenied => Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                )),
                PathFileReadResult::Errno(code) => Ok(BoaVal::from(code)),
                PathFileReadResult::Missing => Ok(BoaVal::null()),
                PathFileReadResult::VfsBytes(b) => Ok(utf8_slice_to_js_val(&b)),
                PathFileReadResult::HostBytes(bytes) => Ok(utf8_vec_to_js_val(bytes)),
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
            match read_vfs_or_host_file_bytes(&path, context)? {
                PathFileReadResult::OcapDenied => Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                )),
                PathFileReadResult::Errno(code) => Ok(BoaVal::from(code)),
                PathFileReadResult::Missing => Ok(BoaVal::null()),
                PathFileReadResult::VfsBytes(b) => bytes_to_js_uint8_array(&b, context),
                PathFileReadResult::HostBytes(bytes) => bytes_to_js_uint8_array(&bytes, context),
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
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
            }
            let allow_host = !is_host_unc_blocked(&path);
            let Some(resolved) = resolve_or_js_err(&path, true, allow_host, "open", context)?
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
                    v.borrow_mut()
                        .update_inode_kind(id, VfsEntryKind::File(Arc::new(data_bytes)));
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
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
            }
            let allow_host = !is_host_unc_blocked(&path);
            let Some(resolved) = resolve_or_js_err(&path, true, allow_host, "open", context)?
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
                        return Err(ocap_denied_js_error(
                            "ERR_SLICE_FS_DENIED",
                            last_ocap_violation_message(),
                            context,
                        ));
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
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
            }
            let allow_host_src = !is_host_unc_blocked(&src);
            let allow_host_dest = !is_host_unc_blocked(&dest);
            let Some(res_src) = resolve_or_js_err(&src, true, allow_host_src, "copyfile", context)?
            else {
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
            };
            let Some(res_dest) =
                resolve_or_js_err(&dest, true, allow_host_dest, "copyfile", context)?
            else {
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
            };
            if (res_src.traversed_symlink && !check_boa_fs_read_ocap(&res_src.norm))
                || (res_dest.traversed_symlink && !check_boa_fs_write_ocap(&res_dest.norm))
            {
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
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
}
