//! Directory Creation/Listing, `statSync`/`lstatSync`, `existsSync`, `mtime`, and `mkdtempSync` Host Bindings.

use crate::bindings::fs::classify_and_cow::{
    ResolvedPathKind, SymlinkResolveError, classify_path_lstat, classify_path_stat,
    default_mtime_ms_for_path, is_host_symlink_or_reparse, is_host_unc_blocked,
    normalize_host_readlink_target, resolve_effective_vfs_path, resolve_or_js_err,
};
use crate::bindings::fs::state::{PathEntry, VfsEntryKind, dir_child_prefix, is_vfs_root_path};
use crate::boa_realm::{
    BOA_DET_STATE, BOA_VFS_STATE, check_boa_fs_read_ocap, check_boa_fs_write_ocap,
    has_boa_fs_read_ocap, norm_vfs_path,
};
use crate::intrinsics::fill_random_bytes;

pub(super) fn register_fs_path_dir_stat_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::object::builtins::JsArray;
    use boa_engine::{JsValue as BoaVal, NativeFunction, js_string};

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
                classify_path_stat(&v.borrow(), &resolved.norm, allow_host).0
                    != ResolvedPathKind::Missing
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
                        if !vfs.is_host_masked(&child_norm) {
                            entries_set.insert(name);
                        }
                    }
                }
                entries_set.into_iter().collect()
            });
            let json = serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string());
            Ok(BoaVal::from(js_string!(json)))
        }),
    );

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
                let (kind, sz) = classify_path_stat(&v.borrow(), &resolved.norm, allow_host);
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
}
