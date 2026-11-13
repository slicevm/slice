//! OCap-Enforced Virtual Filesystem (`BOA_VFS_CACHE`) and Host Filesystem Bindings (`node:fs`).

use crate::bindings::helpers::{
    bytes_to_js_uint8_array, extract_buffer_source_bytes, try_consume_isolate_memory,
};
use crate::boa_realm::{
    check_boa_fs_read_ocap, check_boa_fs_write_ocap, has_boa_fs_read_ocap, norm_vfs_path,
    BOA_DET_STATE, BOA_VFS_CACHE, BOA_VFS_DIRS, BOA_VFS_MTIMES, BOA_VFS_TOMBSTONES,
};
use crate::intrinsics::fill_random_bytes;

const VFS_SYMLINK_PREFIX: &[u8] = b"\x00__SLICE_SYMLINK__:";

/// Blocks raw forward-slash Windows SMB UNC paths (`//server/share`) from reaching host `std::fs`
/// on Windows while keeping POSIX/in-memory VFS root paths (`//file.txt` -> `/file.txt`) intact.
#[inline]
pub(crate) fn is_host_unc_blocked(raw_path: &str) -> bool {
    #[cfg(windows)]
    {
        let tb = raw_path.trim_start().as_bytes();
        if tb.len() >= 2 && tb[0] == b'/' && tb[1] == b'/' {
            let is_ext_drive = tb.len() >= 7
                && tb[2] == b'?'
                && (tb[3] == b'/' || tb[3] == b'\\')
                && tb[4].is_ascii_alphabetic()
                && tb[5] == b':'
                && (tb[6] == b'/' || tb[6] == b'\\');
            if !is_ext_drive && tb.get(2) != Some(&b'/') && tb[2..].contains(&b'/') {
                return true;
            }
        }
    }
    let _ = raw_path;
    false
}

#[inline]
fn dir_child_prefix(norm_dir: &str) -> String {
    if norm_dir == "/" {
        "/".to_string()
    } else {
        format!("{}/", norm_dir)
    }
}

#[inline]
fn insert_vfs_dir_with_ancestors(norm_dir: &str) {
    BOA_VFS_DIRS.with(|d| {
        let mut dirs = d.borrow_mut();
        let mut cur = norm_dir;
        while !cur.is_empty() && cur != "/" && cur != "." {
            dirs.insert(cur.to_string());
            if let Some(idx) = cur.rfind('/') {
                cur = &cur[..idx];
            } else {
                break;
            }
        }
    });
}

#[inline]
fn insert_vfs_parent_dirs(norm_file: &str) {
    if let Some(idx) = norm_file.rfind('/') {
        let parent = &norm_file[..idx];
        if !parent.is_empty() {
            insert_vfs_dir_with_ancestors(parent);
        }
    }
}

#[inline]
pub(crate) fn resolve_symlink_target_against_link(norm_link: &str, raw_target: &str) -> String {
    let trimmed = raw_target.trim();
    let tb = trimmed.as_bytes();
    let is_abs = trimmed.starts_with('/')
        || trimmed.starts_with('\\')
        || (tb.len() >= 2 && tb[0].is_ascii_alphabetic() && tb[1] == b':');
    if is_abs {
        return norm_vfs_path(trimmed);
    }
    let parent = match norm_link.rfind('/') {
        Some(0) => "/",
        Some(idx) => &norm_link[..idx],
        None => ".",
    };
    if parent == "/" {
        norm_vfs_path(&format!("/{}", trimmed))
    } else if parent == "." {
        norm_vfs_path(trimmed)
    } else {
        norm_vfs_path(&format!("{}/{}", parent, trimmed))
    }
}

#[inline]
pub(crate) fn is_vfs_tombstoned(norm: &str) -> bool {
    BOA_VFS_TOMBSTONES.with(|t| {
        let set = t.borrow();
        if set.is_empty() {
            return false;
        }
        if set.contains(norm) {
            return true;
        }
        let mut cur = norm;
        while let Some(idx) = cur.rfind('/') {
            if idx == 0 {
                break;
            }
            cur = &cur[..idx];
            if set.contains(cur) {
                return true;
            }
        }
        false
    })
}

fn vfs_or_host_is_dir(norm: &str, allow_host: bool) -> bool {
    if norm == "/" || norm == "." {
        return true;
    }
    if BOA_VFS_DIRS.with(|d| {
        let dirs = d.borrow();
        !dirs.is_empty() && dirs.contains(norm)
    }) {
        return true;
    }
    let vfs_state = BOA_VFS_CACHE.with(|c| {
        let map = c.borrow();
        if map.is_empty() {
            return None;
        }
        if map.contains_key(norm) {
            return Some(false);
        }
        let prefix = dir_child_prefix(norm);
        if map.keys().any(|k| k.starts_with(&prefix)) {
            Some(true)
        } else {
            None
        }
    });
    if let Some(res) = vfs_state {
        return res;
    }
    if !allow_host || is_vfs_tombstoned(norm) {
        return false;
    }
    std::path::Path::new(norm).is_dir()
}

/// Resolves VFS symlinks along `current` (up to 8 hops), anchoring relative symlink targets
/// against each symlink's containing directory. When `follow_final` is `false`, resolves
/// intermediate directory symlinks without dereferencing the final path component.
/// Returns `Err(())` (`ELOOP`) when symlink traversal exceeds 8 hops.
pub(crate) fn try_resolve_vfs_path_symlinks(
    mut current: String,
    follow_final: bool,
) -> Result<String, ()> {
    let has_vfs = BOA_VFS_CACHE.with(|c| !c.borrow().is_empty());
    if !has_vfs {
        return Ok(current);
    }
    for _ in 0..=8 {
        let rewritten = BOA_VFS_CACHE.with(|c| {
            let map = c.borrow();
            let bytes = current.as_bytes();
            let root_end = if bytes.len() >= 3 && bytes[1] == b':' && bytes[2] == b'/' {
                3
            } else if bytes.first() == Some(&b'/') {
                1
            } else {
                0
            };
            let mut scan = root_end;
            while scan <= bytes.len() {
                let seg_end = match memchr::memchr(b'/', &bytes[scan..]) {
                    Some(rel) => scan + rel,
                    None => bytes.len(),
                };
                let is_final = seg_end == bytes.len();
                if seg_end > 0 && (!is_final || follow_final) {
                    let prefix = &current[..seg_end];
                    if let Some(val) = map.get(prefix) {
                        if let Some(raw_target_bytes) = val.strip_prefix(VFS_SYMLINK_PREFIX) {
                            let raw_target = String::from_utf8_lossy(raw_target_bytes);
                            let resolved_prefix =
                                resolve_symlink_target_against_link(prefix, &raw_target);
                            let next_full = if is_final {
                                resolved_prefix
                            } else {
                                norm_vfs_path(&format!(
                                    "{}{}",
                                    resolved_prefix,
                                    &current[seg_end..]
                                ))
                            };
                            return Some(next_full);
                        }
                    }
                }
                if is_final {
                    break;
                }
                scan = seg_end + 1;
            }
            None
        });
        match rewritten {
            Some(next) => {
                current = next;
            }
            None => return Ok(current),
        }
    }
    Err(())
}

#[cold]
fn js_eloop_error(
    syscall: &str,
    path: &str,
    context: &mut boa_engine::Context,
) -> boa_engine::JsError {
    use boa_engine::{js_string, JsNativeError};
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

fn materialize_host_dir_into_vfs(host_dir: &str) -> Result<(), String> {
    let Ok(rd) = std::fs::read_dir(host_dir) else {
        return Ok(());
    };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let child_norm = if host_dir == "/" {
            format!("/{}", name)
        } else {
            format!("{}/{}", host_dir, name)
        };
        if is_vfs_tombstoned(&child_norm) {
            continue;
        }
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_symlink() {
            let already_cached = BOA_VFS_CACHE.with(|c| c.borrow().contains_key(&child_norm));
            if !already_cached {
                if let Ok(target_path) = std::fs::read_link(entry.path()) {
                    let target_str = target_path.to_string_lossy().replace('\\', "/");
                    let mut payload =
                        Vec::with_capacity(VFS_SYMLINK_PREFIX.len() + target_str.len());
                    payload.extend_from_slice(VFS_SYMLINK_PREFIX);
                    payload.extend_from_slice(target_str.as_bytes());
                    try_consume_isolate_memory(payload.len())?;
                    BOA_VFS_CACHE.with(|c| {
                        c.borrow_mut()
                            .insert(child_norm, std::sync::Arc::new(payload));
                    });
                }
            }
        } else if ft.is_dir() {
            BOA_VFS_DIRS.with(|d| {
                d.borrow_mut().insert(child_norm.clone());
            });
            materialize_host_dir_into_vfs(&child_norm)?;
        } else if ft.is_file() {
            let already_cached = BOA_VFS_CACHE.with(|c| c.borrow().contains_key(&child_norm));
            if !already_cached {
                if let Ok(bytes) = std::fs::read(entry.path()) {
                    try_consume_isolate_memory(bytes.len())?;
                    BOA_VFS_CACHE.with(|c| {
                        c.borrow_mut()
                            .insert(child_norm, std::sync::Arc::new(bytes));
                    });
                }
            }
        }
    }
    Ok(())
}

#[inline]
fn read_host_bytes(norm: &str) -> std::io::Result<Vec<u8>> {
    std::fs::read(norm)
}

#[inline]
fn host_metadata(norm: &str) -> std::io::Result<std::fs::Metadata> {
    std::fs::metadata(norm)
}

#[inline]
fn utf8_slice_to_js_val(bytes: &[u8]) -> boa_engine::JsValue {
    if bytes.is_ascii() {
        return boa_engine::JsValue::from(boa_engine::js_string!(
            boa_engine::string::JsStr::latin1(bytes)
        ));
    }
    let content = String::from_utf8_lossy(bytes);
    boa_engine::JsValue::from(boa_engine::js_string!(content.as_ref()))
}

#[inline]
fn utf8_vec_to_js_val(bytes: Vec<u8>) -> boa_engine::JsValue {
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

pub(crate) fn register_fs_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsNativeError, JsValue as BoaVal, NativeFunction};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_fs_read_file_sync"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_check_write_sync"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let path = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !check_boa_fs_write_ocap(&path) {
                return Ok(BoaVal::from(false));
            }
            let norm = try_resolve_vfs_path_symlinks(norm_vfs_path(&path), true)
                .map_err(|()| js_eloop_error("open", &path, context))?;
            if norm != path && !check_boa_fs_write_ocap(&norm) {
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
            let norm = try_resolve_vfs_path_symlinks(norm_vfs_path(&path), true)
                .map_err(|()| js_eloop_error("open", &path, context))?;
            if norm != path && !check_boa_fs_read_ocap(&norm) {
                return Ok(BoaVal::from(js_string!("")));
            }
            if let Some(arc_bytes) = BOA_VFS_CACHE.with(|c| {
                let map = c.borrow();
                if map.is_empty() {
                    None
                } else {
                    map.get(&norm).cloned()
                }
            }) {
                return Ok(utf8_slice_to_js_val(&arc_bytes));
            }
            if norm == "/"
                || norm == "."
                || BOA_VFS_DIRS.with(|d| {
                    let dirs = d.borrow();
                    !dirs.is_empty() && dirs.contains(&norm)
                })
            {
                return Ok(BoaVal::from(-21));
            }
            if allow_host && !is_vfs_tombstoned(&norm) {
                if let Ok(bytes) = read_host_bytes(&norm) {
                    return Ok(utf8_vec_to_js_val(bytes));
                }
                if vfs_or_host_is_dir(&norm, allow_host) {
                    return Ok(BoaVal::from(-21));
                }
            }
            Ok(BoaVal::null())
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
            let norm = try_resolve_vfs_path_symlinks(norm_vfs_path(&path), true)
                .map_err(|()| js_eloop_error("open", &path, context))?;
            if norm != path && !check_boa_fs_read_ocap(&norm) {
                return bytes_to_js_uint8_array(&[], context);
            }
            if let Some(arc_bytes) = BOA_VFS_CACHE.with(|c| {
                let map = c.borrow();
                if map.is_empty() {
                    None
                } else {
                    map.get(&norm).cloned()
                }
            }) {
                return bytes_to_js_uint8_array(&arc_bytes, context);
            }
            if norm == "/"
                || norm == "."
                || BOA_VFS_DIRS.with(|d| {
                    let dirs = d.borrow();
                    !dirs.is_empty() && dirs.contains(&norm)
                })
            {
                return Ok(BoaVal::from(-21));
            }
            if allow_host && !is_vfs_tombstoned(&norm) {
                if let Ok(bytes) = read_host_bytes(&norm) {
                    return bytes_to_js_uint8_array(&bytes, context);
                }
                if vfs_or_host_is_dir(&norm, allow_host) {
                    return Ok(BoaVal::from(-21));
                }
            }
            Ok(BoaVal::null())
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
            let norm = try_resolve_vfs_path_symlinks(norm_vfs_path(&path), true)
                .map_err(|()| js_eloop_error("open", &path, context))?;
            if norm != path && !check_boa_fs_write_ocap(&norm) {
                return Ok(BoaVal::undefined());
            }
            if norm == "/" || norm == "." || BOA_VFS_DIRS.with(|d| d.borrow().contains(&norm)) {
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
            let prev_len =
                BOA_VFS_CACHE.with(|c| c.borrow().get(&norm).map(|b| b.len()).unwrap_or(0));
            let net_added = data_bytes.len().saturating_sub(prev_len);
            if let Err(msg) = try_consume_isolate_memory(net_added) {
                return Err(JsNativeError::range().with_message(msg).into());
            }
            insert_vfs_parent_dirs(&norm);
            BOA_VFS_CACHE.with(|c| {
                c.borrow_mut().insert(norm, std::sync::Arc::new(data_bytes));
            });
            Ok(BoaVal::undefined())
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
            let norm = try_resolve_vfs_path_symlinks(norm_vfs_path(&path), true)
                .map_err(|()| js_eloop_error("open", &path, context))?;
            if norm != path && !check_boa_fs_write_ocap(&norm) {
                return Ok(BoaVal::undefined());
            }
            if norm == "/" || norm == "." || BOA_VFS_DIRS.with(|d| d.borrow().contains(&norm)) {
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
            if let Err(msg) = try_consume_isolate_memory(data_bytes.len()) {
                return Err(JsNativeError::range().with_message(msg).into());
            }
            insert_vfs_parent_dirs(&norm);
            BOA_VFS_CACHE.with(|c| {
                let mut map = c.borrow_mut();
                let entry = map.entry(norm.clone()).or_insert_with(|| {
                    let initial =
                        if allow_host && !is_vfs_tombstoned(&norm) && has_boa_fs_read_ocap(&norm) {
                            std::fs::read(&norm).unwrap_or_default()
                        } else {
                            Vec::new()
                        };
                    std::sync::Arc::new(initial)
                });
                std::sync::Arc::make_mut(entry).extend_from_slice(&data_bytes);
            });
            Ok(BoaVal::undefined())
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
            let Ok(norm) = try_resolve_vfs_path_symlinks(norm_vfs_path(&path), true) else {
                return Ok(BoaVal::from(false));
            };
            if norm != path && !check_boa_fs_read_ocap(&norm) {
                return Ok(BoaVal::from(false));
            }
            let prefix = dir_child_prefix(&norm);
            let vfs_exists = BOA_VFS_CACHE.with(|c| {
                let map = c.borrow();
                if map.is_empty() {
                    false
                } else if map.contains_key(&norm) {
                    true
                } else {
                    map.keys().any(|k| k.starts_with(&prefix))
                }
            }) || BOA_VFS_DIRS.with(|d| {
                let dirs = d.borrow();
                if dirs.is_empty() {
                    false
                } else if dirs.contains(&norm) {
                    true
                } else {
                    dirs.iter().any(|k| k.starts_with(&prefix))
                }
            });
            let exists = vfs_exists
                || (allow_host
                    && !is_vfs_tombstoned(&norm)
                    && std::path::Path::new(&norm).exists());
            Ok(BoaVal::from(exists))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_mkdir_sync"),
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
            let norm = try_resolve_vfs_path_symlinks(norm_vfs_path(&path), true)
                .map_err(|()| js_eloop_error("mkdir", &path, context))?;
            if norm != path && !check_boa_fs_write_ocap(&norm) {
                return Ok(BoaVal::undefined());
            }
            insert_vfs_dir_with_ancestors(&norm);
            Ok(BoaVal::undefined())
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
            let norm_dir = try_resolve_vfs_path_symlinks(norm_vfs_path(&path), true)
                .map_err(|()| js_eloop_error("scandir", &path, context))?;
            if norm_dir != path && !check_boa_fs_read_ocap(&norm_dir) {
                return Ok(BoaVal::from(js_string!("[]")));
            }
            let prefix = dir_child_prefix(&norm_dir);
            let mut entries_set = std::collections::BTreeSet::new();
            BOA_VFS_CACHE.with(|c| {
                for k in c.borrow().keys() {
                    if let Some(rest) = k.strip_prefix(&prefix) {
                        if let Some(first_seg) = rest.split('/').next() {
                            if !first_seg.is_empty() {
                                entries_set.insert(first_seg.to_string());
                            }
                        }
                    }
                }
            });
            BOA_VFS_DIRS.with(|d| {
                for k in d.borrow().iter() {
                    if let Some(rest) = k.strip_prefix(&prefix) {
                        if let Some(first_seg) = rest.split('/').next() {
                            if !first_seg.is_empty() {
                                entries_set.insert(first_seg.to_string());
                            }
                        }
                    }
                }
            });
            let is_vfs_tmp_dir = norm_dir.starts_with("/tmp/")
                && BOA_VFS_DIRS.with(|d| d.borrow().contains(&norm_dir));
            let has_tombstones = BOA_VFS_TOMBSTONES.with(|t| !t.borrow().is_empty());
            let dir_tombstoned = has_tombstones && is_vfs_tombstoned(&norm_dir);
            if allow_host && !is_vfs_tmp_dir && !dir_tombstoned {
                if let Ok(rd) = std::fs::read_dir(&norm_dir) {
                    for entry in rd.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if has_tombstones {
                            let child_norm = if norm_dir == "/" {
                                format!("/{}", name)
                            } else {
                                format!("{}/{}", norm_dir, name)
                            };
                            if is_vfs_tombstoned(&child_norm) {
                                continue;
                            }
                        }
                        entries_set.insert(name);
                    }
                }
            }
            let entries: Vec<String> = entries_set.into_iter().collect();
            let json = serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string());
            Ok(BoaVal::from(js_string!(json)))
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
            let norm = try_resolve_vfs_path_symlinks(norm_vfs_path(&path), true)
                .map_err(|()| js_eloop_error("stat", &path, context))?;
            if norm != path && !check_boa_fs_read_ocap(&norm) {
                return Ok(BoaVal::from(0));
            }
            if let Some(size) = BOA_VFS_CACHE.with(|c| {
                let map = c.borrow();
                if map.is_empty() {
                    None
                } else {
                    map.get(&norm).map(|v| v.len() as u64)
                }
            }) {
                return Ok(BoaVal::from(size as f64));
            }
            if BOA_VFS_DIRS.with(|d| {
                let dirs = d.borrow();
                !dirs.is_empty() && dirs.contains(&norm)
            }) {
                return Ok(BoaVal::from(0.0));
            }
            if allow_host && !is_vfs_tombstoned(&norm) {
                if let Ok(meta) = host_metadata(&norm) {
                    return Ok(BoaVal::from(meta.len() as f64));
                }
                if vfs_or_host_is_dir(&norm, allow_host) {
                    return Ok(BoaVal::from(0.0));
                }
            } else if norm == "/" || norm == "." || vfs_or_host_is_dir(&norm, false) {
                return Ok(BoaVal::from(0.0));
            }
            Ok(BoaVal::from(-1.0))
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
            let Ok(norm) = try_resolve_vfs_path_symlinks(norm_vfs_path(&path), true) else {
                return Ok(BoaVal::from(1_700_000_000_000.0));
            };
            if norm != path && !check_boa_fs_read_ocap(&norm) {
                return Ok(BoaVal::from(1_700_000_000_000.0));
            }
            if let Some(custom_mt) = BOA_VFS_MTIMES.with(|m| m.borrow().get(&norm).copied()) {
                return Ok(BoaVal::from(custom_mt));
            }
            if BOA_DET_STATE.with(|s| s.get().is_some()) {
                return Ok(BoaVal::from(1_700_000_000_000.0));
            }
            let mtime_ms = if !allow_host || is_vfs_tombstoned(&norm) {
                None
            } else {
                host_metadata(&norm)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as f64)
            }
            .unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as f64)
                    .unwrap_or(1_700_000_000_000.0)
            });
            Ok(BoaVal::from(mtime_ms))
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
            let Ok(norm) = try_resolve_vfs_path_symlinks(norm_vfs_path(&path), true) else {
                return Ok(BoaVal::from(false));
            };
            if norm != path && !check_boa_fs_read_ocap(&norm) {
                return Ok(BoaVal::from(false));
            }
            let is_dir = vfs_or_host_is_dir(&norm, allow_host);
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
            let norm_src = try_resolve_vfs_path_symlinks(norm_vfs_path(&src), true)
                .map_err(|()| js_eloop_error("copyfile", &src, context))?;
            let norm_dest = try_resolve_vfs_path_symlinks(norm_vfs_path(&dest), true)
                .map_err(|()| js_eloop_error("copyfile", &dest, context))?;
            if !check_boa_fs_read_ocap(&norm_src) || !check_boa_fs_write_ocap(&norm_dest) {
                return Ok(BoaVal::undefined());
            }
            let content = BOA_VFS_CACHE
                .with(|c| c.borrow().get(&norm_src).cloned())
                .unwrap_or_else(|| {
                    let bytes = if !allow_host_src || is_vfs_tombstoned(&norm_src) {
                        Vec::new()
                    } else {
                        std::fs::read(&norm_src).unwrap_or_default()
                    };
                    std::sync::Arc::new(bytes)
                });
            if let Err(msg) = try_consume_isolate_memory(content.len()) {
                return Err(JsNativeError::range().with_message(msg).into());
            }
            insert_vfs_parent_dirs(&norm_dest);
            BOA_VFS_CACHE.with(|c| {
                c.borrow_mut().insert(norm_dest, content);
            });
            Ok(BoaVal::undefined())
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
            // Resolve parent directory symlinks ONLY (`follow_final = false`) so renaming a
            // symlink moves the directory entry itself rather than its target (#77).
            let norm_old = try_resolve_vfs_path_symlinks(norm_vfs_path(&old_p), false)
                .map_err(|()| js_eloop_error("rename", &old_p, context))?;
            let norm_new = try_resolve_vfs_path_symlinks(norm_vfs_path(&new_p), false)
                .map_err(|()| js_eloop_error("rename", &new_p, context))?;
            if !check_boa_fs_read_ocap(&norm_old)
                || !check_boa_fs_write_ocap(&norm_old)
                || !check_boa_fs_write_ocap(&norm_new)
            {
                return Ok(BoaVal::undefined());
            }
            if norm_old == norm_new {
                return Ok(BoaVal::undefined());
            }

            // Case 1: `norm_old` is a VFS regular file or VFS symlink
            if let Some(content) = BOA_VFS_CACHE.with(|c| c.borrow().get(&norm_old).cloned()) {
                let dest_is_cached_leaf =
                    BOA_VFS_CACHE.with(|c| c.borrow().contains_key(&norm_new));
                if !dest_is_cached_leaf && vfs_or_host_is_dir(&norm_new, allow_host_new) {
                    return Ok(BoaVal::from(-21)); // EISDIR
                }
                if let Some(raw_sym) = content.strip_prefix(VFS_SYMLINK_PREFIX) {
                    let raw_str = String::from_utf8_lossy(raw_sym);
                    let resolved_new_target =
                        resolve_symlink_target_against_link(&norm_new, &raw_str);
                    if !check_boa_fs_read_ocap(&resolved_new_target) {
                        return Ok(BoaVal::undefined());
                    }
                }
                BOA_VFS_CACHE.with(|c| {
                    c.borrow_mut().remove(&norm_old);
                });
                BOA_VFS_MTIMES.with(|m| {
                    let mut mtimes = m.borrow_mut();
                    if let Some(mt) = mtimes.remove(&norm_old) {
                        mtimes.insert(norm_new.clone(), mt);
                    }
                });
                if allow_host_old && std::path::Path::new(&norm_old).exists() {
                    BOA_VFS_TOMBSTONES.with(|t| {
                        t.borrow_mut().insert(norm_old);
                    });
                }
                insert_vfs_parent_dirs(&norm_new);
                BOA_VFS_CACHE.with(|c| {
                    c.borrow_mut().insert(norm_new, content);
                });
                return Ok(BoaVal::undefined());
            }

            // Case 2: `norm_old` is a VFS or host directory (#74)
            let src_is_host_sym = allow_host_old
                && !is_vfs_tombstoned(&norm_old)
                && std::fs::symlink_metadata(&norm_old)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false);
            if !src_is_host_sym && vfs_or_host_is_dir(&norm_old, allow_host_old) {
                let old_prefix = dir_child_prefix(&norm_old);
                if norm_old == "/" || norm_old == "." || norm_new.starts_with(&old_prefix) {
                    return Ok(BoaVal::from(-22)); // EINVAL
                }
                if BOA_VFS_CACHE.with(|c| c.borrow().contains_key(&norm_new)) {
                    return Ok(BoaVal::from(-20)); // ENOTDIR
                }
                if vfs_or_host_is_dir(&norm_new, allow_host_new) {
                    let new_prefix = dir_child_prefix(&norm_new);
                    let has_vfs_children = BOA_VFS_CACHE
                        .with(|c| c.borrow().keys().any(|k| k.starts_with(&new_prefix)))
                        || BOA_VFS_DIRS
                            .with(|d| d.borrow().iter().any(|k| k.starts_with(&new_prefix)));
                    let has_host_children = allow_host_new
                        && !is_vfs_tombstoned(&norm_new)
                        && std::fs::read_dir(&norm_new)
                            .map(|rd| {
                                rd.flatten().any(|e| {
                                    let child_p =
                                        format!("{}/{}", norm_new, e.file_name().to_string_lossy());
                                    !is_vfs_tombstoned(&child_p)
                                })
                            })
                            .unwrap_or(false);
                    if has_vfs_children || has_host_children {
                        return Ok(BoaVal::from(-39)); // ENOTEMPTY
                    }
                }
                if allow_host_old
                    && !is_vfs_tombstoned(&norm_old)
                    && std::path::Path::new(&norm_old).is_dir()
                {
                    if let Err(msg) = materialize_host_dir_into_vfs(&norm_old) {
                        return Err(JsNativeError::range().with_message(msg).into());
                    }
                    BOA_VFS_TOMBSTONES.with(|t| {
                        t.borrow_mut().insert(norm_old.clone());
                    });
                }
                if allow_host_new && std::path::Path::new(&norm_new).exists() {
                    BOA_VFS_TOMBSTONES.with(|t| {
                        t.borrow_mut().insert(norm_new.clone());
                    });
                }

                BOA_VFS_CACHE.with(|c| {
                    let mut map = c.borrow_mut();
                    let moved: Vec<(String, std::sync::Arc<Vec<u8>>)> = map
                        .keys()
                        .filter(|k| k.starts_with(&old_prefix))
                        .cloned()
                        .collect::<Vec<_>>()
                        .into_iter()
                        .filter_map(|k| map.remove(&k).map(|v| (k, v)))
                        .collect();
                    for (old_k, val) in moved {
                        let rel = &old_k[old_prefix.len()..];
                        let new_k = format!("{}/{}", norm_new, rel);
                        map.insert(new_k, val);
                    }
                });

                insert_vfs_dir_with_ancestors(&norm_new);
                BOA_VFS_DIRS.with(|d| {
                    let mut dirs = d.borrow_mut();
                    dirs.remove(&norm_old);
                    let moved_dirs: Vec<String> = dirs
                        .iter()
                        .filter(|k| k.starts_with(&old_prefix))
                        .cloned()
                        .collect();
                    for old_d in moved_dirs {
                        dirs.remove(&old_d);
                        let rel = &old_d[old_prefix.len()..];
                        dirs.insert(format!("{}/{}", norm_new, rel));
                    }
                });

                BOA_VFS_MTIMES.with(|m| {
                    let mut mtimes = m.borrow_mut();
                    if let Some(mt) = mtimes.remove(&norm_old) {
                        mtimes.insert(norm_new.clone(), mt);
                    }
                    let moved_mt: Vec<(String, f64)> = mtimes
                        .keys()
                        .filter(|k| k.starts_with(&old_prefix))
                        .cloned()
                        .collect::<Vec<_>>()
                        .into_iter()
                        .filter_map(|k| mtimes.remove(&k).map(|v| (k, v)))
                        .collect();
                    for (old_k, mt) in moved_mt {
                        let rel = &old_k[old_prefix.len()..];
                        mtimes.insert(format!("{}/{}", norm_new, rel), mt);
                    }
                });

                return Ok(BoaVal::undefined());
            }

            // Case 3: Host symlink or host regular file CoW rename
            if !allow_host_old || is_vfs_tombstoned(&norm_old) {
                return Ok(BoaVal::from(-2)); // ENOENT
            }
            if src_is_host_sym {
                let Ok(target_path) = std::fs::read_link(&norm_old) else {
                    return Ok(BoaVal::from(-2));
                };
                let target_str = target_path.to_string_lossy().replace('\\', "/");
                let resolved_new_target =
                    resolve_symlink_target_against_link(&norm_new, &target_str);
                if !check_boa_fs_read_ocap(&resolved_new_target) {
                    return Ok(BoaVal::undefined());
                }
                let mut payload = Vec::with_capacity(VFS_SYMLINK_PREFIX.len() + target_str.len());
                payload.extend_from_slice(VFS_SYMLINK_PREFIX);
                payload.extend_from_slice(target_str.as_bytes());
                if let Err(msg) = try_consume_isolate_memory(payload.len()) {
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                BOA_VFS_TOMBSTONES.with(|t| {
                    t.borrow_mut().insert(norm_old);
                });
                insert_vfs_parent_dirs(&norm_new);
                BOA_VFS_CACHE.with(|c| {
                    c.borrow_mut()
                        .insert(norm_new, std::sync::Arc::new(payload));
                });
                return Ok(BoaVal::undefined());
            }

            let Ok(bytes) = std::fs::read(&norm_old) else {
                return Ok(BoaVal::from(-2)); // ENOENT
            };
            if let Err(msg) = try_consume_isolate_memory(bytes.len()) {
                return Err(JsNativeError::range().with_message(msg).into());
            }
            BOA_VFS_TOMBSTONES.with(|t| {
                t.borrow_mut().insert(norm_old);
            });
            insert_vfs_parent_dirs(&norm_new);
            BOA_VFS_CACHE.with(|c| {
                c.borrow_mut().insert(norm_new, std::sync::Arc::new(bytes));
            });
            Ok(BoaVal::undefined())
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
            let norm = try_resolve_vfs_path_symlinks(norm_vfs_path(&path), false)
                .map_err(|()| js_eloop_error("rm", &path, context))?;
            if norm != path && !check_boa_fs_write_ocap(&norm) {
                return Ok(BoaVal::undefined());
            }
            let prefix = dir_child_prefix(&norm);
            BOA_VFS_CACHE.with(|c| {
                let mut map = c.borrow_mut();
                map.remove(&norm);
                map.retain(|k, _| !k.starts_with(&prefix));
            });
            BOA_VFS_DIRS.with(|d| {
                let mut dirs = d.borrow_mut();
                dirs.remove(&norm);
                dirs.retain(|k| !k.starts_with(&prefix));
            });
            BOA_VFS_MTIMES.with(|m| {
                let mut mtimes = m.borrow_mut();
                mtimes.remove(&norm);
                mtimes.retain(|k, _| !k.starts_with(&prefix));
            });
            // Strictly copy-on-write VFS deletion: never remove files or directories from the
            // host OS disk. If a real host entry exists at this path, mask it with a VFS
            // tombstone so subsequent reads/stats inside the isolate see it as unlinked and
            // `Slice.restore()` can roll the deletion back cleanly.
            if allow_host && std::path::Path::new(&norm).exists() {
                BOA_VFS_TOMBSTONES.with(|t| {
                    t.borrow_mut().insert(norm);
                });
            }
            Ok(BoaVal::undefined())
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
            let norm = try_resolve_vfs_path_symlinks(norm_vfs_path(&path), true)
                .map_err(|()| js_eloop_error("realpath", &path, context))?;
            if norm != path && !check_boa_fs_read_ocap(&norm) {
                return Ok(BoaVal::from(js_string!("")));
            }
            if BOA_VFS_CACHE.with(|c| c.borrow().contains_key(&norm))
                || BOA_VFS_DIRS.with(|d| d.borrow().contains(&norm))
            {
                let resolved =
                    if norm.starts_with('/') || (norm.len() >= 2 && norm.as_bytes()[1] == b':') {
                        norm
                    } else {
                        let cwd = std::env::current_dir()
                            .map(|p| norm_vfs_path(&p.to_string_lossy()))
                            .unwrap_or_else(|_| "/workspace".to_string());
                        format!("{}/{}", cwd.trim_end_matches('/'), norm)
                    };
                return Ok(BoaVal::from(js_string!(resolved)));
            }
            if allow_host && !is_vfs_tombstoned(&norm) {
                if let Ok(canon) = std::fs::canonicalize(&norm) {
                    let mut s = canon.to_string_lossy().replace('\\', "/");
                    if let Some(stripped) = s.strip_prefix("//?/") {
                        s = stripped.to_string();
                    }
                    return Ok(BoaVal::from(js_string!(s)));
                }
            }
            let fallback =
                if norm.starts_with('/') || (norm.len() >= 2 && norm.as_bytes()[1] == b':') {
                    norm
                } else {
                    let cwd = std::env::current_dir()
                        .map(|p| norm_vfs_path(&p.to_string_lossy()))
                        .unwrap_or_else(|_| "/workspace".to_string());
                    format!("{}/{}", cwd.trim_end_matches('/'), norm)
                };
            Ok(BoaVal::from(js_string!(fallback)))
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
            let norm = norm_vfs_path(&full_path);
            insert_vfs_dir_with_ancestors(&norm);
            Ok(BoaVal::from(js_string!(norm)))
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
            let norm = try_resolve_vfs_path_symlinks(norm_vfs_path(&path), true)
                .map_err(|()| js_eloop_error("chmod", &path, context))?;
            if norm != path && !check_boa_fs_write_ocap(&norm) {
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
            let norm_link = try_resolve_vfs_path_symlinks(norm_vfs_path(&link_path), false)
                .map_err(|()| js_eloop_error("symlink", &link_path, context))?;
            if norm_link != link_path && !check_boa_fs_write_ocap(&norm_link) {
                return Ok(BoaVal::undefined());
            }
            if target.as_bytes().contains(&0) || is_host_unc_blocked(&target) {
                let _ = check_boa_fs_read_ocap(&target);
                return Ok(BoaVal::undefined());
            }
            // Anchor relative symlink targets against `norm_link`'s containing directory (#78)
            // before enforcing OCap read authority on the resolved destination.
            let resolved_target = resolve_symlink_target_against_link(&norm_link, &target);
            if !check_boa_fs_read_ocap(&resolved_target) {
                return Ok(BoaVal::undefined());
            }
            let stored_target = target.trim();
            let mut payload = Vec::with_capacity(VFS_SYMLINK_PREFIX.len() + stored_target.len());
            payload.extend_from_slice(VFS_SYMLINK_PREFIX);
            payload.extend_from_slice(stored_target.as_bytes());
            if let Err(msg) = try_consume_isolate_memory(payload.len()) {
                return Err(JsNativeError::range().with_message(msg).into());
            }
            insert_vfs_parent_dirs(&norm_link);
            BOA_VFS_CACHE.with(|c| {
                c.borrow_mut()
                    .insert(norm_link, std::sync::Arc::new(payload));
            });
            Ok(BoaVal::undefined())
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
            let Ok(norm) = try_resolve_vfs_path_symlinks(norm_vfs_path(&path), false) else {
                return Ok(BoaVal::from(false));
            };
            if norm != path && !check_boa_fs_read_ocap(&norm) {
                return Ok(BoaVal::from(false));
            }
            let is_sym = BOA_VFS_CACHE
                .with(|c| {
                    c.borrow()
                        .get(&norm)
                        .map(|b| b.starts_with(VFS_SYMLINK_PREFIX))
                })
                .unwrap_or_else(|| {
                    if !allow_host || is_vfs_tombstoned(&norm) {
                        false
                    } else {
                        std::fs::symlink_metadata(&norm)
                            .map(|m| m.file_type().is_symlink())
                            .unwrap_or(false)
                    }
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
            let norm = try_resolve_vfs_path_symlinks(norm_vfs_path(&path), false)
                .map_err(|()| js_eloop_error("readlink", &path, context))?;
            if norm != path && !check_boa_fs_read_ocap(&norm) {
                return Ok(BoaVal::from(js_string!("")));
            }
            if let Some(target) = BOA_VFS_CACHE.with(|c| {
                c.borrow().get(&norm).and_then(|b| {
                    if b.starts_with(VFS_SYMLINK_PREFIX) {
                        Some(String::from_utf8_lossy(&b[VFS_SYMLINK_PREFIX.len()..]).into_owned())
                    } else {
                        None
                    }
                })
            }) {
                let resolved_target = resolve_symlink_target_against_link(&norm, &target);
                if !check_boa_fs_read_ocap(&resolved_target) {
                    return Ok(BoaVal::from(js_string!("")));
                }
                return Ok(BoaVal::from(js_string!(target)));
            }
            if allow_host && !is_vfs_tombstoned(&norm) {
                if let Ok(target_path) = std::fs::read_link(&norm) {
                    let target_str = target_path.to_string_lossy().replace('\\', "/");
                    let resolved_target = resolve_symlink_target_against_link(&norm, &target_str);
                    if check_boa_fs_read_ocap(&resolved_target) {
                        return Ok(BoaVal::from(js_string!(target_str)));
                    }
                }
            }
            Ok(BoaVal::from(js_string!("")))
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
            let norm = try_resolve_vfs_path_symlinks(norm_vfs_path(&path), true)
                .map_err(|()| js_eloop_error("truncate", &path, context))?;
            if !check_boa_fs_read_ocap(&norm) || !check_boa_fs_write_ocap(&norm) {
                return Ok(BoaVal::undefined());
            }
            let mut data = BOA_VFS_CACHE
                .with(|c| c.borrow().get(&norm).map(|arc| (**arc).clone()))
                .unwrap_or_else(|| {
                    if !allow_host || is_vfs_tombstoned(&norm) {
                        Vec::new()
                    } else {
                        std::fs::read(&norm).unwrap_or_default()
                    }
                });
            if new_len > data.len() {
                if let Err(msg) = try_consume_isolate_memory(new_len - data.len()) {
                    return Err(JsNativeError::range().with_message(msg).into());
                }
            }
            data.resize(new_len, 0u8);
            BOA_VFS_CACHE.with(|c| {
                c.borrow_mut().insert(norm, std::sync::Arc::new(data));
            });
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fs_utimes_sync"),
        3,
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
            let norm = try_resolve_vfs_path_symlinks(norm_vfs_path(&path), true)
                .map_err(|()| js_eloop_error("utime", &path, context))?;
            if norm != path && !check_boa_fs_write_ocap(&norm) {
                return Ok(BoaVal::undefined());
            }
            let mtime_ms = args
                .get(2)
                .and_then(|v| v.as_number())
                .unwrap_or(1_700_000_000_000.0);
            BOA_VFS_MTIMES.with(|m| {
                m.borrow_mut().insert(norm, mtime_ms);
            });
            Ok(BoaVal::undefined())
        }),
    );
}
