//! File Descriptor Open Flags Decoding and `openSync`, `closeSync`, `fdPathSync`, `markWatchSync` Bindings.

use crate::bindings::fs::classify_and_cow::{
    ResolvedPathKind, classify_path_lstat, classify_path_stat, is_host_unc_blocked,
    resolve_or_js_err,
};
use crate::bindings::fs::state::{InodeId, OpenFd, PathEntry, VfsEntryKind};
use crate::bindings::helpers::{
    release_isolate_memory, try_adjust_isolate_memory_ctx, try_consume_isolate_memory_ctx,
};
use crate::boa_realm::{
    BOA_DET_STATE, BOA_VFS_STATE, DIRTY_VFS, check_boa_fs_read_ocap, check_boa_fs_write_ocap,
    mark_subsystem_dirty,
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

impl ParsedOpenFlags {
    const READ_ONLY: Self = Self {
        readable: true,
        writable: false,
        create: false,
        excl: false,
        trunc: false,
        append: false,
    };
}

#[inline]
fn decode_numeric_open_flags(bits: u32) -> Result<ParsedOpenFlags, ()> {
    let (readable, writable) = match bits & 3 {
        0 => (true, false),
        1 => (false, true),
        2 => (true, true),
        _ => return Err(()),
    };
    Ok(ParsedOpenFlags {
        readable,
        writable,
        create: (bits & 64) != 0,
        excl: (bits & 128) != 0,
        trunc: (bits & 512) != 0,
        append: (bits & 1024) != 0,
    })
}

fn parse_open_flags(
    val: Option<&boa_engine::JsValue>,
    ctx: &mut boa_engine::Context,
) -> Result<ParsedOpenFlags, ()> {
    let Some(v) = val else {
        return Ok(ParsedOpenFlags::READ_ONLY);
    };
    if v.is_undefined() || v.is_null() {
        return Ok(ParsedOpenFlags::READ_ONLY);
    }
    if let Some(num) = v.as_number() {
        if !num.is_finite() || num < 0.0 {
            return Err(());
        }
        return decode_numeric_open_flags(num as u32);
    }
    let s = v
        .to_string(ctx)
        .map(|js| js.to_std_string_escaped())
        .map_err(|_| ())?;
    if let Ok(bits) = s.trim().parse::<u32>() {
        return decode_numeric_open_flags(bits);
    }
    match s.as_str() {
        "r" | "rs" | "sr" => Ok(ParsedOpenFlags::READ_ONLY),
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

#[inline]
pub(super) fn parse_fd_arg(arg: Option<&boa_engine::JsValue>) -> Option<i32> {
    let fd_num = arg?.as_number()?;
    (fd_num.is_finite() && fd_num.fract() == 0.0).then_some(fd_num as i32)
}

pub(super) fn register_fs_fd_open_close_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{JsNativeError, JsValue as BoaVal, NativeFunction, js_string};

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

            // POSIX O_CREAT | O_EXCL must fail with EEXIST on an existing symlink (even dangling)
            // without following the final symlink component.
            if fl.create && fl.excl {
                let Some(res_lstat) = resolve_or_js_err(&path, false, allow_host, "open", context)?
                else {
                    return Ok(BoaVal::undefined());
                };
                if res_lstat.traversed_symlink {
                    if fl.readable && !check_boa_fs_read_ocap(&res_lstat.norm) {
                        return Ok(BoaVal::undefined());
                    }
                    if needs_write && !check_boa_fs_write_ocap(&res_lstat.norm) {
                        return Ok(BoaVal::undefined());
                    }
                }
                if res_lstat.has_file_ancestor {
                    return Ok(BoaVal::from(-20));
                }
                let exists_lstat = BOA_VFS_STATE.with(|v| {
                    classify_path_lstat(&v.borrow(), &res_lstat.norm, allow_host)
                        != ResolvedPathKind::Missing
                });
                if exists_lstat {
                    return Ok(BoaVal::from(-17)); // EEXIST
                }
            }

            let Some(resolved) = resolve_or_js_err(&path, true, allow_host, "open", context)?
            else {
                return Ok(BoaVal::undefined());
            };
            if resolved.traversed_symlink {
                if fl.readable && !check_boa_fs_read_ocap(&resolved.norm) {
                    return Ok(BoaVal::undefined());
                }
                if needs_write && !check_boa_fs_write_ocap(&resolved.norm) {
                    return Ok(BoaVal::undefined());
                }
            }
            if resolved.has_file_ancestor {
                return Ok(BoaVal::from(-20)); // ENOTDIR
            }
            let norm = resolved.norm;

            enum OpenTarget {
                VfsFile(InodeId, usize),
                VfsDir(InodeId),
                HostDir,
                HostFile,
                Missing,
                ErrCode(i32),
            }

            let target = BOA_VFS_STATE.with(|v| {
                let vfs = v.borrow();
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
                        if !check_boa_fs_read_ocap(&norm) {
                            return Ok(BoaVal::undefined());
                        }
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
            let Some(fd) = parse_fd_arg(args.first()) else {
                return Ok(BoaVal::from(-9));
            };
            let res = BOA_VFS_STATE.with(|v| v.borrow_mut().close_fd(fd));
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
            let Some(fd) = parse_fd_arg(args.first()) else {
                return Ok(BoaVal::null());
            };
            let p_opt =
                BOA_VFS_STATE.with(|v| v.borrow().get_fd(fd).map(|f| f.opened_path.clone()));
            match p_opt {
                Some(p) => Ok(BoaVal::from(js_string!(p))),
                None => Ok(BoaVal::null()),
            }
        }),
    );
}
