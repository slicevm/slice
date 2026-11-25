//! CoW Subtree `renameSync` and `rmSync` Host Bindings.

use crate::bindings::fs::classify_and_cow::{
    ResolvedPathKind, classify_path_lstat, dir_has_any_children, is_host_unc_blocked,
    materialize_host_dir_into_vfs, normalize_host_readlink_target, resolve_or_js_err,
    resolve_symlink_target_against_link,
};
use crate::bindings::fs::state::{PathEntry, VfsEntryKind, dir_child_prefix, is_vfs_root_path};
use crate::bindings::helpers::{
    last_ocap_violation_message, ocap_denied_js_error, release_isolate_memory,
    trip_isolate_oom_limits, try_adjust_isolate_memory_ctx,
};
use crate::boa_realm::{BOA_VFS_STATE, check_boa_fs_read_ocap, check_boa_fs_write_ocap};
use std::sync::Arc;

pub(super) fn register_fs_path_tree_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{JsNativeError, JsValue as BoaVal, NativeFunction, js_string};

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
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
            }
            let allow_host_old = !is_host_unc_blocked(&old_p);
            let allow_host_new = !is_host_unc_blocked(&new_p);
            // Resolve intermediate parent directory symlinks ONLY (`follow_final = false`) so
            // renaming a symlink moves the directory entry itself rather than its target (#77).
            let Some(res_old) =
                resolve_or_js_err(&old_p, false, allow_host_old, "rename", context)?
            else {
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
            };
            let Some(res_new) =
                resolve_or_js_err(&new_p, false, allow_host_new, "rename", context)?
            else {
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
            };
            if (res_old.traversed_symlink
                && (!check_boa_fs_read_ocap(&res_old.norm)
                    || !check_boa_fs_write_ocap(&res_old.norm)))
                || (res_new.traversed_symlink && !check_boa_fs_write_ocap(&res_new.norm))
            {
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
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
                let dir_id = match old_dir_entry {
                    Some(PathEntry::Present(existing_dir_id)) => {
                        vfs.update_inode_kind(
                            existing_dir_id,
                            VfsEntryKind::Directory { host_opaque: true },
                        );
                        existing_dir_id
                    }
                    _ => vfs.alloc_inode(VfsEntryKind::Directory { host_opaque: true }, None, 1, 0),
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
                return Err(ocap_denied_js_error(
                    "ERR_SLICE_FS_DENIED",
                    last_ocap_violation_message(),
                    context,
                ));
            }
            let allow_host = !is_host_unc_blocked(&path);
            let Some(resolved) = resolve_or_js_err(&path, false, allow_host, "rm", context)? else {
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
}
