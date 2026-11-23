//! File Descriptor I/O & Metadata Bindings (`readSync`, `writeSync`, `fstatSync`, `ftruncateSync`, `futimesSync`, FD `readFileSync`/`writeFileSync`).

use super::fd_open_close::parse_fd_arg;
use crate::bindings::fs::classify_and_cow::default_mtime_ms_for_path;
use crate::bindings::fs::state::VfsEntryKind;
use crate::bindings::helpers::{
    bytes_to_js_uint8_array, copy_into_js_buffer_view, extract_buffer_source_bytes,
    trip_isolate_oom_limits, try_adjust_isolate_memory_ctx, try_consume_isolate_memory,
    utf8_slice_to_js_val, with_buffer_source_bytes,
};
use crate::boa_realm::{BOA_VFS_STATE, DIRTY_VFS, mark_subsystem_dirty};
use std::sync::Arc;

/// Shared write routine for `__slice_fs_write_fd_sync` and `__slice_fs_write_all_fd_sync`.
/// Returns `Ok(Ok(bytes_written))` on success, `Ok(Err(errno))` on `EBADF`, or `Err(oom_msg)` on OOM.
fn write_bytes_to_open_fd(
    fd: i32,
    src_bytes: &[u8],
    explicit_pos: Option<usize>,
    force_append: bool,
) -> Result<Result<usize, i32>, String> {
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
        return Ok(Err(-9)); // EBADF
    };

    mark_subsystem_dirty(DIRTY_VFS);
    let write_pos = if is_append {
        cur_len
    } else {
        explicit_pos.unwrap_or(cur_pos)
    };
    let new_len = cur_len.max(write_pos.saturating_add(src_bytes.len()));
    let delta = new_len as isize - prev_acc as isize;
    if delta > 0 {
        try_consume_isolate_memory(delta as usize)?;
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
            vec_mut[write_pos..write_pos + src_bytes.len()].copy_from_slice(src_bytes);
        }
        vfs.update_inode_kind(inode_id, VfsEntryKind::File(arc_buf));
        if (is_append || explicit_pos.is_none())
            && let Some(open_fd) = vfs.get_fd_mut(fd)
        {
            open_fd.position = (write_pos + src_bytes.len()) as u64;
        }
    });

    Ok(Ok(src_bytes.len()))
}

pub(super) fn register_fs_fd_io_and_stat_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::object::builtins::JsArray;
    use boa_engine::{JsNativeError, JsValue as BoaVal, NativeFunction, js_string};

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_read_fd_sync"),
        5,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(fd) = parse_fd_arg(args.first()) else {
                return Ok(BoaVal::from(-9));
            };
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
                    return FdReadSource::ErrCode(-9);
                };
                if !open_fd.readable {
                    return FdReadSource::ErrCode(-9);
                }
                let read_pos = explicit_pos.unwrap_or(open_fd.position as usize);
                match vfs.get_inode(open_fd.inode).map(|i| &i.kind) {
                    Some(VfsEntryKind::File(b)) => FdReadSource::Bytes(Arc::clone(b), read_pos),
                    Some(VfsEntryKind::Directory { .. }) => FdReadSource::ErrCode(-21),
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
            let Some(fd) = parse_fd_arg(args.first()) else {
                return Ok(BoaVal::from(-9));
            };
            let Some(src_val) = args.get(1) else {
                return Ok(BoaVal::from(0));
            };
            let explicit_pos: Option<usize> = args
                .get(2)
                .and_then(|v| v.as_number())
                .filter(|&n| n.is_finite() && n >= 0.0)
                .map(|n| n as usize);

            let res = with_buffer_source_bytes(src_val, context, |src_bytes| {
                write_bytes_to_open_fd(fd, src_bytes, explicit_pos, false)
            })?;
            match res {
                Ok(Ok(n)) => Ok(BoaVal::from(n as f64)),
                Ok(Err(errno)) => Ok(BoaVal::from(errno)),
                Err(msg) => {
                    trip_isolate_oom_limits(context);
                    Err(JsNativeError::range().with_message(msg).into())
                }
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_fstat_info_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(fd) = parse_fd_arg(args.first()) else {
                return Ok(BoaVal::from(-9));
            };
            let info_opt = BOA_VFS_STATE.with(|v| {
                let vfs = v.borrow();
                let open_fd = vfs.get_fd(fd)?;
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
            let Some(fd) = parse_fd_arg(args.first()) else {
                return Ok(BoaVal::from(-9));
            };
            let new_len = args
                .get(1)
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as usize)
                .unwrap_or(0);
            let meta_opt = BOA_VFS_STATE.with(|v| {
                let vfs = v.borrow();
                let open_fd = vfs.get_fd(fd)?;
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
            let Some(fd) = parse_fd_arg(args.first()) else {
                return Ok(BoaVal::from(-9));
            };
            let mtime_ms = args
                .get(1)
                .and_then(|v| v.as_number())
                .unwrap_or(1_700_000_000_000.0);
            let ok = BOA_VFS_STATE.with(|v| {
                let mut vfs = v.borrow_mut();
                let Some(inode_id) = vfs.get_fd(fd).map(|f| f.inode) else {
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
            let Some(fd) = parse_fd_arg(args.first()) else {
                return Ok(BoaVal::from(-9));
            };
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
            let Some(fd) = parse_fd_arg(args.first()) else {
                return Ok(BoaVal::from(-9));
            };
            let force_append = args.get(2).and_then(|v| v.as_boolean()).unwrap_or(false);
            let data_bytes = if let Some(val) = args.get(1) {
                if val.is_object() {
                    extract_buffer_source_bytes(val, context)?
                } else {
                    val.to_string(context)?.to_std_string_escaped().into_bytes()
                }
            } else {
                Vec::new()
            };
            match write_bytes_to_open_fd(fd, &data_bytes, None, force_append) {
                Ok(Ok(_)) => Ok(BoaVal::from(0)),
                Ok(Err(errno)) => Ok(BoaVal::from(errno)),
                Err(msg) => {
                    trip_isolate_oom_limits(context);
                    Err(JsNativeError::range().with_message(msg).into())
                }
            }
        }),
    );
}
