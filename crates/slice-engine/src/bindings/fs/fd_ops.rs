//! Inode-Backed Open File Descriptor Table (`openSync`, `closeSync`, `readSync`, `writeSync`,
//! `fstatSync`, `ftruncateSync`, `futimesSync`, and FD-overloaded `readFileSync`/`writeFileSync`).

use crate::bindings::fs::path_ops::{
    anchor_vfs_path, classify_path_lstat, classify_path_stat, default_mtime_ms_for_path,
    has_file_ancestor, is_host_unc_blocked, js_eloop_error, try_resolve_vfs_path_symlinks,
    utf8_slice_to_js_val, ResolvedPathKind,
};
use crate::bindings::fs::state::{OpenFd, PathEntry, VfsEntryKind};
use crate::bindings::helpers::{
    bytes_to_js_uint8_array, extract_buffer_source_bytes, release_isolate_memory,
    try_adjust_isolate_memory_ctx, try_consume_isolate_memory_ctx, with_buffer_source_bytes,
};
use crate::boa_realm::{
    check_boa_fs_read_ocap, check_boa_fs_write_ocap, mark_subsystem_dirty, BOA_DET_STATE,
    BOA_VFS_STATE, DIRTY_VFS,
};
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
struct ParsedOpenFlags {
    readable: bool,
    writable: bool,
    create: bool,
    excl: bool,
    trunc: bool,
    append: bool,
}

fn parse_open_flags(
    val: Option<&boa_engine::JsValue>,
    ctx: &mut boa_engine::Context,
) -> Result<ParsedOpenFlags, ()> {
    let Some(v) = val else {
        return Ok(ParsedOpenFlags {
            readable: true,
            writable: false,
            create: false,
            excl: false,
            trunc: false,
            append: false,
        });
    };
    if v.is_undefined() || v.is_null() {
        return Ok(ParsedOpenFlags {
            readable: true,
            writable: false,
            create: false,
            excl: false,
            trunc: false,
            append: false,
        });
    }
    if let Some(num) = v.as_number() {
        if !num.is_finite() || num < 0.0 {
            return Err(());
        }
        let bits = num as u32;
        let acc = bits & 3;
        let (readable, writable) = match acc {
            0 => (true, false),
            1 => (false, true),
            2 => (true, true),
            _ => return Err(()),
        };
        return Ok(ParsedOpenFlags {
            readable,
            writable,
            create: (bits & 64) != 0,
            excl: (bits & 128) != 0,
            trunc: (bits & 512) != 0,
            append: (bits & 1024) != 0,
        });
    }
    let s = v
        .to_string(ctx)
        .map(|js| js.to_std_string_escaped())
        .map_err(|_| ())?;
    if let Ok(bits) = s.trim().parse::<u32>() {
        let acc = bits & 3;
        let (readable, writable) = match acc {
            0 => (true, false),
            1 => (false, true),
            2 => (true, true),
            _ => return Err(()),
        };
        return Ok(ParsedOpenFlags {
            readable,
            writable,
            create: (bits & 64) != 0,
            excl: (bits & 128) != 0,
            trunc: (bits & 512) != 0,
            append: (bits & 1024) != 0,
        });
    }
    match s.as_str() {
        "r" | "rs" | "sr" => Ok(ParsedOpenFlags {
            readable: true,
            writable: false,
            create: false,
            excl: false,
            trunc: false,
            append: false,
        }),
        "r+" | "rs+" | "sr+" => Ok(ParsedOpenFlags {
            readable: true,
            writable: true,
            create: false,
            excl: false,
            trunc: false,
            append: false,
        }),
        "w" => Ok(ParsedOpenFlags {
            readable: false,
            writable: true,
            create: true,
            excl: false,
            trunc: true,
            append: false,
        }),
        "wx" | "xw" => Ok(ParsedOpenFlags {
            readable: false,
            writable: true,
            create: true,
            excl: true,
            trunc: true,
            append: false,
        }),
        "w+" => Ok(ParsedOpenFlags {
            readable: true,
            writable: true,
            create: true,
            excl: false,
            trunc: true,
            append: false,
        }),
        "wx+" | "xw+" => Ok(ParsedOpenFlags {
            readable: true,
            writable: true,
            create: true,
            excl: true,
            trunc: true,
            append: false,
        }),
        "a" => Ok(ParsedOpenFlags {
            readable: false,
            writable: true,
            create: true,
            excl: false,
            trunc: false,
            append: true,
        }),
        "ax" | "xa" => Ok(ParsedOpenFlags {
            readable: false,
            writable: true,
            create: true,
            excl: true,
            trunc: false,
            append: true,
        }),
        "a+" => Ok(ParsedOpenFlags {
            readable: true,
            writable: true,
            create: true,
            excl: false,
            trunc: false,
            append: true,
        }),
        "ax+" | "xa+" => Ok(ParsedOpenFlags {
            readable: true,
            writable: true,
            create: true,
            excl: true,
            trunc: false,
            append: true,
        }),
        _ => Err(()),
    }
}

/// Copies `src` directly into `dest_obj` starting at `dest_offset` with zero intermediate heap
/// allocations when `dest_obj` is a `Uint8Array`, `Buffer`, or `ArrayBuffer`.
fn copy_into_js_buffer_view(
    dest_val: &boa_engine::JsValue,
    dest_offset: usize,
    src: &[u8],
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<usize> {
    use boa_engine::builtins::typed_array::TypedArray;
    use boa_engine::js_string;
    use boa_engine::object::builtins::JsArrayBuffer;

    if src.is_empty() {
        return Ok(0);
    }
    let Some(obj) = dest_val.as_object() else {
        return Ok(0);
    };
    if let Some(ta) = obj.downcast_ref::<TypedArray>() {
        if let Some(copied) = ta.with_bytes_mut(|dest_slice| {
            if dest_offset >= dest_slice.len() {
                return 0;
            }
            let avail = src.len().min(dest_slice.len() - dest_offset);
            dest_slice[dest_offset..dest_offset + avail].copy_from_slice(&src[..avail]);
            avail
        }) {
            return Ok(copied);
        }
    }
    if let Ok(ab) = JsArrayBuffer::from_object(obj.clone()) {
        if let Some(mut data_mut) = ab.data_mut() {
            if dest_offset >= data_mut.len() {
                return Ok(0);
            }
            let avail = src.len().min(data_mut.len() - dest_offset);
            data_mut[dest_offset..dest_offset + avail].copy_from_slice(&src[..avail]);
            return Ok(avail);
        }
    }
    if let Ok(buf_val) = obj.get(js_string!("buffer"), context) {
        if let Some(buf_obj) = buf_val.as_object() {
            if let Ok(ab) = JsArrayBuffer::from_object(buf_obj) {
                let view_off = obj
                    .get(js_string!("byteOffset"), context)
                    .ok()
                    .and_then(|v| v.as_number())
                    .map(|n| n.max(0.0) as usize)
                    .unwrap_or(0);
                let view_len = obj
                    .get(js_string!("byteLength"), context)
                    .ok()
                    .and_then(|v| v.as_number())
                    .map(|n| n.max(0.0) as usize);
                if let Some(mut data_mut) = ab.data_mut() {
                    let base = view_off.min(data_mut.len());
                    let max_end = view_len
                        .map(|l| (base + l).min(data_mut.len()))
                        .unwrap_or(data_mut.len());
                    let start = base.saturating_add(dest_offset);
                    if start >= max_end {
                        return Ok(0);
                    }
                    let avail = src.len().min(max_end - start);
                    data_mut[start..start + avail].copy_from_slice(&src[..avail]);
                    return Ok(avail);
                }
            }
        }
    }
    Ok(0)
}

pub(crate) fn register_fs_fd_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::object::builtins::JsArray;
    use boa_engine::{js_string, JsNativeError, JsValue as BoaVal, NativeFunction};

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_mark_watch_sync"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            mark_subsystem_dirty(DIRTY_VFS);
            BOA_VFS_STATE.with(|v| {
                v.borrow_mut().has_active_watchers = true;
            });
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_open_sync"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let Ok(fl) = parse_open_flags(args.get(1), context) else {
                return Ok(BoaVal::from(-22)); // EINVAL
            };
            let needs_write = fl.writable || fl.create || fl.trunc || fl.append;
            if fl.readable && !check_boa_fs_read_ocap(&path) {
                return Ok(BoaVal::undefined());
            }
            if needs_write && !check_boa_fs_write_ocap(&path) {
                return Ok(BoaVal::undefined());
            }
            mark_subsystem_dirty(DIRTY_VFS);

            let allow_host = !is_host_unc_blocked(&path);
            let anchored = anchor_vfs_path(&path);

            // POSIX O_CREAT | O_EXCL must fail with EEXIST on an existing symlink (even dangling)
            // without following the final symlink component.
            if fl.create && fl.excl {
                let (norm_lstat, trav_lstat) =
                    try_resolve_vfs_path_symlinks(anchored.clone(), false)
                        .map_err(|()| js_eloop_error("open", &path, context))?;
                if trav_lstat {
                    if fl.readable && !check_boa_fs_read_ocap(&norm_lstat) {
                        return Ok(BoaVal::undefined());
                    }
                    if needs_write && !check_boa_fs_write_ocap(&norm_lstat) {
                        return Ok(BoaVal::undefined());
                    }
                }
                let exists_lstat = BOA_VFS_STATE.with(|v| {
                    classify_path_lstat(&v.borrow(), &norm_lstat, allow_host)
                        != ResolvedPathKind::Missing
                });
                if exists_lstat {
                    return Ok(BoaVal::from(-17)); // EEXIST
                }
            }

            let (norm, traversed) = try_resolve_vfs_path_symlinks(anchored, true)
                .map_err(|()| js_eloop_error("open", &path, context))?;
            if traversed {
                if fl.readable && !check_boa_fs_read_ocap(&norm) {
                    return Ok(BoaVal::undefined());
                }
                if needs_write && !check_boa_fs_write_ocap(&norm) {
                    return Ok(BoaVal::undefined());
                }
            }

            enum OpenTarget {
                VfsFile(crate::bindings::fs::state::InodeId, usize),
                VfsDir(crate::bindings::fs::state::InodeId),
                HostDir,
                HostFile,
                Missing,
                ErrCode(i32),
            }

            let target = BOA_VFS_STATE.with(|v| {
                let vfs = v.borrow();
                if has_file_ancestor(&vfs, &norm, allow_host) {
                    return OpenTarget::ErrCode(-20); // ENOTDIR
                }
                if let Some((id, ino)) = vfs.lookup_present(&norm) {
                    return match &ino.kind {
                        VfsEntryKind::Directory { .. } => {
                            if needs_write {
                                OpenTarget::ErrCode(-21) // EISDIR
                            } else {
                                OpenTarget::VfsDir(id)
                            }
                        }
                        VfsEntryKind::File(_) => {
                            if fl.excl {
                                OpenTarget::ErrCode(-17) // EEXIST
                            } else {
                                OpenTarget::VfsFile(id, ino.accounted_bytes)
                            }
                        }
                        VfsEntryKind::Symlink(_) => OpenTarget::Missing,
                    };
                }
                let (stat_kind, _) = classify_path_stat(&vfs, &norm, allow_host);
                match stat_kind {
                    ResolvedPathKind::Directory => {
                        if needs_write {
                            OpenTarget::ErrCode(-21)
                        } else {
                            OpenTarget::HostDir
                        }
                    }
                    ResolvedPathKind::File => {
                        if fl.excl {
                            OpenTarget::ErrCode(-17)
                        } else {
                            OpenTarget::HostFile
                        }
                    }
                    ResolvedPathKind::Missing | ResolvedPathKind::Symlink => OpenTarget::Missing,
                }
            });

            match target {
                OpenTarget::ErrCode(code) => Ok(BoaVal::from(code)),
                OpenTarget::VfsDir(id) => {
                    let fd = BOA_VFS_STATE.with(|v| {
                        let mut vfs = v.borrow_mut();
                        if let Some(ino) = vfs.get_inode_mut(id) {
                            ino.open_fd_count = ino.open_fd_count.saturating_add(1);
                        }
                        vfs.alloc_fd(OpenFd {
                            inode: id,
                            position: 0,
                            readable: fl.readable,
                            writable: false,
                            append: false,
                            opened_path: norm,
                        })
                    });
                    Ok(BoaVal::from(fd))
                }
                OpenTarget::HostDir => {
                    let fd = BOA_VFS_STATE.with(|v| {
                        let mut vfs = v.borrow_mut();
                        vfs.ensure_dir_with_ancestors(&norm, false);
                        let id = match vfs.lookup_present(&norm) {
                            Some((id, _)) => id,
                            None => vfs.alloc_inode(
                                VfsEntryKind::Directory { host_opaque: false },
                                None,
                                1,
                                0,
                            ),
                        };
                        if let Some(ino) = vfs.get_inode_mut(id) {
                            ino.open_fd_count = ino.open_fd_count.saturating_add(1);
                        }
                        vfs.alloc_fd(OpenFd {
                            inode: id,
                            position: 0,
                            readable: fl.readable,
                            writable: false,
                            append: false,
                            opened_path: norm,
                        })
                    });
                    Ok(BoaVal::from(fd))
                }
                OpenTarget::VfsFile(id, prev_acc) => {
                    if fl.trunc && prev_acc > 0 {
                        let _ = try_adjust_isolate_memory_ctx(-(prev_acc as isize), context);
                    }
                    let fd = BOA_VFS_STATE.with(|v| {
                        let mut vfs = v.borrow_mut();
                        if fl.trunc {
                            vfs.update_inode_kind(id, VfsEntryKind::File(Arc::new(Vec::new())));
                        }
                        if let Some(ino) = vfs.get_inode_mut(id) {
                            ino.open_fd_count = ino.open_fd_count.saturating_add(1);
                        }
                        vfs.alloc_fd(OpenFd {
                            inode: id,
                            position: 0,
                            readable: fl.readable,
                            writable: fl.writable,
                            append: fl.append,
                            opened_path: norm,
                        })
                    });
                    Ok(BoaVal::from(fd))
                }
                OpenTarget::HostFile => {
                    let bytes = if fl.trunc {
                        Vec::new()
                    } else {
                        match std::fs::read(&norm) {
                            Ok(b) => b,
                            Err(_) => return Ok(BoaVal::from(-2)),
                        }
                    };
                    if let Err(msg) = try_consume_isolate_memory_ctx(bytes.len(), context) {
                        return Err(JsNativeError::range().with_message(msg).into());
                    }
                    let host_mt = if BOA_DET_STATE.with(|s| s.get().is_some()) {
                        Some(1_700_000_000_000.0)
                    } else {
                        std::fs::metadata(&norm)
                            .ok()
                            .and_then(|m| m.modified().ok())
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_millis() as f64)
                    };
                    let fd = BOA_VFS_STATE.with(|v| {
                        let mut vfs = v.borrow_mut();
                        vfs.ensure_parent_dirs(&norm);
                        let id =
                            vfs.alloc_inode(VfsEntryKind::File(Arc::new(bytes)), host_mt, 1, 1);
                        vfs.paths.insert(norm.clone(), PathEntry::Present(id));
                        vfs.alloc_fd(OpenFd {
                            inode: id,
                            position: 0,
                            readable: fl.readable,
                            writable: fl.writable,
                            append: fl.append,
                            opened_path: norm,
                        })
                    });
                    Ok(BoaVal::from(fd))
                }
                OpenTarget::Missing => {
                    if !fl.create {
                        return Ok(BoaVal::from(-2)); // ENOENT
                    }
                    let fd = BOA_VFS_STATE.with(|v| {
                        let mut vfs = v.borrow_mut();
                        vfs.ensure_parent_dirs(&norm);
                        let id =
                            vfs.alloc_inode(VfsEntryKind::File(Arc::new(Vec::new())), None, 1, 1);
                        vfs.paths.insert(norm.clone(), PathEntry::Present(id));
                        vfs.alloc_fd(OpenFd {
                            inode: id,
                            position: 0,
                            readable: fl.readable,
                            writable: fl.writable,
                            append: fl.append,
                            opened_path: norm,
                        })
                    });
                    Ok(BoaVal::from(fd))
                }
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_close_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let Some(fd_num) = args.first().and_then(|v| v.as_number()) else {
                return Ok(BoaVal::from(-9));
            };
            if !fd_num.is_finite() || fd_num.fract() != 0.0 {
                return Ok(BoaVal::from(-9));
            }
            let res = BOA_VFS_STATE.with(|v| v.borrow_mut().close_fd(fd_num as i32));
            match res {
                Ok(freed) => {
                    if freed > 0 {
                        release_isolate_memory(freed);
                    }
                    Ok(BoaVal::from(0))
                }
                Err(()) => Ok(BoaVal::from(-9)),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_fd_path_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let Some(fd_num) = args.first().and_then(|v| v.as_number()) else {
                return Ok(BoaVal::null());
            };
            if !fd_num.is_finite() || fd_num.fract() != 0.0 {
                return Ok(BoaVal::null());
            }
            let p_opt = BOA_VFS_STATE.with(|v| {
                v.borrow()
                    .get_fd(fd_num as i32)
                    .map(|f| f.opened_path.clone())
            });
            match p_opt {
                Some(p) => Ok(BoaVal::from(js_string!(p))),
                None => Ok(BoaVal::null()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_read_fd_sync"),
        5,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(fd_num) = args.first().and_then(|v| v.as_number()) else {
                return Ok(BoaVal::from(-9));
            };
            if !fd_num.is_finite() || fd_num.fract() != 0.0 {
                return Ok(BoaVal::from(-9));
            }
            let fd = fd_num as i32;
            let Some(dest_val) = args.get(1) else {
                return Ok(BoaVal::from(0));
            };
            let offset = args
                .get(2)
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as usize)
                .unwrap_or(0);
            let length = args
                .get(3)
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as usize)
                .unwrap_or(0);
            let explicit_pos: Option<usize> = args
                .get(4)
                .and_then(|v| v.as_number())
                .filter(|&n| n.is_finite() && n >= 0.0)
                .map(|n| n as usize);

            enum FdReadSource {
                Bytes(Arc<Vec<u8>>, usize),
                ErrCode(i32),
            }

            let src_res = BOA_VFS_STATE.with(|v| {
                let vfs = v.borrow();
                let Some(open_fd) = vfs.get_fd(fd) else {
                    return FdReadSource::ErrCode(-9); // EBADF
                };
                if !open_fd.readable {
                    return FdReadSource::ErrCode(-9);
                }
                let read_pos = explicit_pos.unwrap_or(open_fd.position as usize);
                match vfs.get_inode(open_fd.inode).map(|i| &i.kind) {
                    Some(VfsEntryKind::File(b)) => FdReadSource::Bytes(Arc::clone(b), read_pos),
                    Some(VfsEntryKind::Directory { .. }) => FdReadSource::ErrCode(-21), // EISDIR
                    _ => FdReadSource::ErrCode(-9),
                }
            });

            let (arc_bytes, read_pos) = match src_res {
                FdReadSource::Bytes(b, p) => (b, p),
                FdReadSource::ErrCode(c) => return Ok(BoaVal::from(c)),
            };

            if length == 0 || read_pos >= arc_bytes.len() {
                return Ok(BoaVal::from(0));
            }
            let avail = length.min(arc_bytes.len() - read_pos);
            let copied = copy_into_js_buffer_view(
                dest_val,
                offset,
                &arc_bytes[read_pos..read_pos + avail],
                context,
            )?;
            if explicit_pos.is_none() && copied > 0 {
                BOA_VFS_STATE.with(|v| {
                    if let Some(open_fd) = v.borrow_mut().get_fd_mut(fd) {
                        open_fd.position = (read_pos + copied) as u64;
                    }
                });
            }
            Ok(BoaVal::from(copied as f64))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_write_fd_sync"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(fd_num) = args.first().and_then(|v| v.as_number()) else {
                return Ok(BoaVal::from(-9));
            };
            if !fd_num.is_finite() || fd_num.fract() != 0.0 {
                return Ok(BoaVal::from(-9));
            }
            let fd = fd_num as i32;
            let Some(src_val) = args.get(1) else {
                return Ok(BoaVal::from(0));
            };
            let explicit_pos: Option<usize> = args
                .get(2)
                .and_then(|v| v.as_number())
                .filter(|&n| n.is_finite() && n >= 0.0)
                .map(|n| n as usize);

            let meta_opt = BOA_VFS_STATE.with(|v| {
                let vfs = v.borrow();
                let open_fd = vfs.get_fd(fd)?;
                if !open_fd.writable {
                    return None;
                }
                let ino = vfs.get_inode(open_fd.inode)?;
                let cur_len = match &ino.kind {
                    VfsEntryKind::File(b) => b.len(),
                    _ => return None,
                };
                Some((
                    open_fd.inode,
                    open_fd.append,
                    open_fd.position as usize,
                    cur_len,
                    ino.accounted_bytes,
                ))
            });
            let Some((inode_id, is_append, cur_pos, cur_len, prev_acc)) = meta_opt else {
                return Ok(BoaVal::from(-9)); // EBADF
            };

            mark_subsystem_dirty(DIRTY_VFS);
            let written =
                with_buffer_source_bytes(src_val, context, |src_bytes| -> Result<usize, String> {
                    let write_pos = if is_append {
                        cur_len
                    } else {
                        explicit_pos.unwrap_or(cur_pos)
                    };
                    let new_len = cur_len.max(write_pos.saturating_add(src_bytes.len()));
                    let delta = new_len as isize - prev_acc as isize;
                    if delta > 0 {
                        crate::bindings::helpers::try_consume_isolate_memory(delta as usize)?;
                    }
                    BOA_VFS_STATE.with(|v| {
                        let mut vfs = v.borrow_mut();
                        let mut arc_buf = match vfs.get_inode(inode_id).map(|i| &i.kind) {
                            Some(VfsEntryKind::File(b)) => Arc::clone(b),
                            _ => Arc::new(Vec::new()),
                        };
                        let vec_mut = Arc::make_mut(&mut arc_buf);
                        if new_len > vec_mut.len() {
                            vec_mut.resize(new_len, 0u8);
                        }
                        if !src_bytes.is_empty() {
                            vec_mut[write_pos..write_pos + src_bytes.len()]
                                .copy_from_slice(src_bytes);
                        }
                        vfs.update_inode_kind(inode_id, VfsEntryKind::File(arc_buf));
                        if is_append || explicit_pos.is_none() {
                            if let Some(open_fd) = vfs.get_fd_mut(fd) {
                                open_fd.position = (write_pos + src_bytes.len()) as u64;
                            }
                        }
                    });
                    Ok(src_bytes.len())
                })?;
            match written {
                Ok(n) => Ok(BoaVal::from(n as f64)),
                Err(msg) => {
                    crate::bindings::helpers::trip_isolate_oom_limits(context);
                    Err(JsNativeError::range().with_message(msg).into())
                }
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_fstat_info_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(fd_num) = args.first().and_then(|v| v.as_number()) else {
                return Ok(BoaVal::from(-9));
            };
            if !fd_num.is_finite() || fd_num.fract() != 0.0 {
                return Ok(BoaVal::from(-9));
            }
            let info_opt = BOA_VFS_STATE.with(|v| {
                let vfs = v.borrow();
                let open_fd = vfs.get_fd(fd_num as i32)?;
                let ino = vfs.get_inode(open_fd.inode)?;
                let mt = ino.mtime_ms.unwrap_or_else(|| {
                    default_mtime_ms_for_path(&vfs, &open_fd.opened_path, false)
                });
                match &ino.kind {
                    VfsEntryKind::File(b) => Some((b.len() as f64, mt, 0)),
                    VfsEntryKind::Directory { .. } => Some((0.0, mt, 1)),
                    VfsEntryKind::Symlink(t) => Some((t.len() as f64, mt, 2)),
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
                None => Ok(BoaVal::from(-9)),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_ftruncate_sync"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(fd_num) = args.first().and_then(|v| v.as_number()) else {
                return Ok(BoaVal::from(-9));
            };
            if !fd_num.is_finite() || fd_num.fract() != 0.0 {
                return Ok(BoaVal::from(-9));
            }
            let new_len = args
                .get(1)
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as usize)
                .unwrap_or(0);
            let meta_opt = BOA_VFS_STATE.with(|v| {
                let vfs = v.borrow();
                let open_fd = vfs.get_fd(fd_num as i32)?;
                if !open_fd.writable {
                    return None;
                }
                let ino = vfs.get_inode(open_fd.inode)?;
                if !matches!(ino.kind, VfsEntryKind::File(_)) {
                    return None;
                }
                Some((open_fd.inode, ino.accounted_bytes))
            });
            let Some((inode_id, prev_acc)) = meta_opt else {
                return Ok(BoaVal::from(-9));
            };
            mark_subsystem_dirty(DIRTY_VFS);
            let delta = new_len as isize - prev_acc as isize;
            if let Err(msg) = try_adjust_isolate_memory_ctx(delta, context) {
                return Err(JsNativeError::range().with_message(msg).into());
            }
            BOA_VFS_STATE.with(|v| {
                let mut vfs = v.borrow_mut();
                let mut arc_buf = match vfs.get_inode(inode_id).map(|i| &i.kind) {
                    Some(VfsEntryKind::File(b)) => Arc::clone(b),
                    _ => Arc::new(Vec::new()),
                };
                Arc::make_mut(&mut arc_buf).resize(new_len, 0u8);
                vfs.update_inode_kind(inode_id, VfsEntryKind::File(arc_buf));
            });
            Ok(BoaVal::from(0))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_futimes_sync"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let Some(fd_num) = args.first().and_then(|v| v.as_number()) else {
                return Ok(BoaVal::from(-9));
            };
            if !fd_num.is_finite() || fd_num.fract() != 0.0 {
                return Ok(BoaVal::from(-9));
            }
            let mtime_ms = args
                .get(1)
                .and_then(|v| v.as_number())
                .unwrap_or(1_700_000_000_000.0);
            let ok = BOA_VFS_STATE.with(|v| {
                let mut vfs = v.borrow_mut();
                let Some(inode_id) = vfs.get_fd(fd_num as i32).map(|f| f.inode) else {
                    return false;
                };
                if let Some(ino) = vfs.get_inode_mut(inode_id) {
                    ino.mtime_ms = Some(mtime_ms);
                    true
                } else {
                    false
                }
            });
            Ok(BoaVal::from(if ok { 0 } else { -9 }))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_read_all_fd_sync"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(fd_num) = args.first().and_then(|v| v.as_number()) else {
                return Ok(BoaVal::from(-9));
            };
            if !fd_num.is_finite() || fd_num.fract() != 0.0 {
                return Ok(BoaVal::from(-9));
            }
            let fd = fd_num as i32;
            let as_utf8 = args.get(1).and_then(|v| v.as_boolean()).unwrap_or(false);
            enum ReadAllFd {
                Bytes(Arc<Vec<u8>>, usize),
                ErrCode(i32),
            }
            let res = BOA_VFS_STATE.with(|v| {
                let mut vfs = v.borrow_mut();
                let Some(open_fd) = vfs.get_fd(fd) else {
                    return ReadAllFd::ErrCode(-9);
                };
                if !open_fd.readable {
                    return ReadAllFd::ErrCode(-9);
                }
                let inode_id = open_fd.inode;
                let pos = open_fd.position as usize;
                let arc_b = match vfs.get_inode(inode_id).map(|i| &i.kind) {
                    Some(VfsEntryKind::File(b)) => Arc::clone(b),
                    Some(VfsEntryKind::Directory { .. }) => return ReadAllFd::ErrCode(-21),
                    _ => return ReadAllFd::ErrCode(-9),
                };
                let end = arc_b.len();
                if let Some(fd_mut) = vfs.get_fd_mut(fd) {
                    fd_mut.position = end as u64;
                }
                ReadAllFd::Bytes(arc_b, pos.min(end))
            });
            match res {
                ReadAllFd::Bytes(b, start) => {
                    let slice = &b[start..];
                    if as_utf8 {
                        Ok(utf8_slice_to_js_val(slice))
                    } else {
                        bytes_to_js_uint8_array(slice, context)
                    }
                }
                ReadAllFd::ErrCode(c) => Ok(BoaVal::from(c)),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_write_all_fd_sync"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(fd_num) = args.first().and_then(|v| v.as_number()) else {
                return Ok(BoaVal::from(-9));
            };
            if !fd_num.is_finite() || fd_num.fract() != 0.0 {
                return Ok(BoaVal::from(-9));
            }
            let fd = fd_num as i32;
            let force_append = args.get(2).and_then(|v| v.as_boolean()).unwrap_or(false);
            let meta_opt = BOA_VFS_STATE.with(|v| {
                let vfs = v.borrow();
                let open_fd = vfs.get_fd(fd)?;
                if !open_fd.writable {
                    return None;
                }
                let ino = vfs.get_inode(open_fd.inode)?;
                let cur_len = match &ino.kind {
                    VfsEntryKind::File(b) => b.len(),
                    _ => return None,
                };
                Some((
                    open_fd.inode,
                    open_fd.append || force_append,
                    open_fd.position as usize,
                    cur_len,
                    ino.accounted_bytes,
                ))
            });
            let Some((inode_id, is_append, cur_pos, cur_len, prev_acc)) = meta_opt else {
                return Ok(BoaVal::from(-9));
            };
            let data_bytes = if let Some(val) = args.get(1) {
                if val.is_object() {
                    extract_buffer_source_bytes(val, context)?
                } else {
                    val.to_string(context)?.to_std_string_escaped().into_bytes()
                }
            } else {
                Vec::new()
            };
            mark_subsystem_dirty(DIRTY_VFS);
            let write_pos = if is_append { cur_len } else { cur_pos };
            let new_len = cur_len.max(write_pos.saturating_add(data_bytes.len()));
            let delta = new_len as isize - prev_acc as isize;
            if let Err(msg) = try_adjust_isolate_memory_ctx(delta, context) {
                return Err(JsNativeError::range().with_message(msg).into());
            }
            BOA_VFS_STATE.with(|v| {
                let mut vfs = v.borrow_mut();
                let mut arc_buf = match vfs.get_inode(inode_id).map(|i| &i.kind) {
                    Some(VfsEntryKind::File(b)) => Arc::clone(b),
                    _ => Arc::new(Vec::new()),
                };
                let vec_mut = Arc::make_mut(&mut arc_buf);
                if new_len > vec_mut.len() {
                    vec_mut.resize(new_len, 0u8);
                }
                if !data_bytes.is_empty() {
                    vec_mut[write_pos..write_pos + data_bytes.len()].copy_from_slice(&data_bytes);
                }
                vfs.update_inode_kind(inode_id, VfsEntryKind::File(arc_buf));
                if let Some(open_fd) = vfs.get_fd_mut(fd) {
                    open_fd.position = (write_pos + data_bytes.len()) as u64;
                }
            });
            Ok(BoaVal::from(0))
        }),
    );
}
