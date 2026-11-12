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

fn vfs_or_host_is_dir(norm: &str) -> bool {
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
        let prefix = format!("{}/", norm);
        if map.keys().any(|k| k.starts_with(&prefix)) {
            Some(true)
        } else {
            None
        }
    });
    if let Some(res) = vfs_state {
        return res;
    }
    if is_vfs_tombstoned(norm) {
        return false;
    }
    std::path::Path::new(norm).is_dir()
}

#[inline]
pub(crate) fn resolve_vfs_symlink_norm(mut current: String) -> String {
    let has_vfs = BOA_VFS_CACHE.with(|c| !c.borrow().is_empty());
    if !has_vfs {
        return current;
    }
    for _ in 0..8 {
        let next_opt = BOA_VFS_CACHE.with(|c| {
            c.borrow().get(&current).and_then(|bytes| {
                bytes
                    .strip_prefix(VFS_SYMLINK_PREFIX)
                    .map(|target| norm_vfs_path(&String::from_utf8_lossy(target)))
            })
        });
        if let Some(next) = next_opt {
            current = next;
        } else {
            break;
        }
    }
    current
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
            let norm = resolve_vfs_symlink_norm(norm_vfs_path(&path));
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
            let norm = resolve_vfs_symlink_norm(norm_vfs_path(&path));
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
            if !is_vfs_tombstoned(&norm) {
                if let Ok(bytes) = read_host_bytes(&norm) {
                    return Ok(utf8_vec_to_js_val(bytes));
                }
                if vfs_or_host_is_dir(&norm) {
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
            let norm = resolve_vfs_symlink_norm(norm_vfs_path(&path));
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
            if !is_vfs_tombstoned(&norm) {
                if let Ok(bytes) = read_host_bytes(&norm) {
                    return bytes_to_js_uint8_array(&bytes, context);
                }
                if vfs_or_host_is_dir(&norm) {
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
            let norm = resolve_vfs_symlink_norm(norm_vfs_path(&path));
            if norm != path && !check_boa_fs_write_ocap(&norm) {
                return Ok(BoaVal::undefined());
            }
            if BOA_VFS_DIRS.with(|d| d.borrow().contains(&norm)) {
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
            if let Some(idx) = norm.rfind('/') {
                let parent = &norm[..idx];
                BOA_VFS_DIRS.with(|d| {
                    d.borrow_mut().insert(parent.to_string());
                });
            }
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
            let norm = resolve_vfs_symlink_norm(norm_vfs_path(&path));
            if norm != path && !check_boa_fs_write_ocap(&norm) {
                return Ok(BoaVal::undefined());
            }
            if BOA_VFS_DIRS.with(|d| d.borrow().contains(&norm)) {
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
            if let Some(idx) = norm.rfind('/') {
                let parent = &norm[..idx];
                if !parent.is_empty() {
                    BOA_VFS_DIRS.with(|d| {
                        d.borrow_mut().insert(parent.to_string());
                    });
                }
            }
            BOA_VFS_CACHE.with(|c| {
                let mut map = c.borrow_mut();
                let entry = map.entry(norm.clone()).or_insert_with(|| {
                    let initial = if !is_vfs_tombstoned(&norm) && has_boa_fs_read_ocap(&norm) {
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
            let norm = resolve_vfs_symlink_norm(norm_vfs_path(&path));
            if norm != path && !check_boa_fs_read_ocap(&norm) {
                return Ok(BoaVal::from(false));
            }
            let vfs_exists = BOA_VFS_CACHE.with(|c| {
                let map = c.borrow();
                if map.is_empty() {
                    false
                } else if map.contains_key(&norm) {
                    true
                } else {
                    let prefix = format!("{}/", norm);
                    map.keys().any(|k| k.starts_with(&prefix))
                }
            }) || BOA_VFS_DIRS.with(|d| {
                let dirs = d.borrow();
                if dirs.is_empty() {
                    false
                } else if dirs.contains(&norm) {
                    true
                } else {
                    let prefix = format!("{}/", norm);
                    dirs.iter().any(|k| k.starts_with(&prefix))
                }
            });
            let exists =
                vfs_exists || (!is_vfs_tombstoned(&norm) && std::path::Path::new(&norm).exists());
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
            let norm = resolve_vfs_symlink_norm(norm_vfs_path(&path));
            if norm != path && !check_boa_fs_write_ocap(&norm) {
                return Ok(BoaVal::undefined());
            }
            BOA_VFS_DIRS.with(|d| {
                let mut dirs = d.borrow_mut();
                let mut cur = norm.as_str();
                while !cur.is_empty() && cur != "/" {
                    dirs.insert(cur.to_string());
                    if let Some(idx) = cur.rfind('/') {
                        cur = &cur[..idx];
                    } else {
                        break;
                    }
                }
            });
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
            let norm_dir = resolve_vfs_symlink_norm(norm_vfs_path(&path));
            if norm_dir != path && !check_boa_fs_read_ocap(&norm_dir) {
                return Ok(BoaVal::from(js_string!("[]")));
            }
            let prefix = format!("{}/", norm_dir);
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
            if !is_vfs_tmp_dir && !dir_tombstoned {
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
            let norm = resolve_vfs_symlink_norm(norm_vfs_path(&path));
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
            if !is_vfs_tombstoned(&norm) {
                if let Ok(meta) = host_metadata(&norm) {
                    return Ok(BoaVal::from(meta.len() as f64));
                }
                if vfs_or_host_is_dir(&norm) {
                    return Ok(BoaVal::from(0.0));
                }
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
            let norm = resolve_vfs_symlink_norm(norm_vfs_path(&path));
            if norm != path && !check_boa_fs_read_ocap(&norm) {
                return Ok(BoaVal::from(1_700_000_000_000.0));
            }
            if let Some(custom_mt) = BOA_VFS_MTIMES.with(|m| m.borrow().get(&norm).copied()) {
                return Ok(BoaVal::from(custom_mt));
            }
            if BOA_DET_STATE.with(|s| s.get().is_some()) {
                return Ok(BoaVal::from(1_700_000_000_000.0));
            }
            let mtime_ms = if is_vfs_tombstoned(&norm) {
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
            let norm = resolve_vfs_symlink_norm(norm_vfs_path(&path));
            if norm != path && !check_boa_fs_read_ocap(&norm) {
                return Ok(BoaVal::from(false));
            }
            let is_dir = vfs_or_host_is_dir(&norm);
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
            let norm_src = resolve_vfs_symlink_norm(norm_vfs_path(&src));
            let norm_dest = resolve_vfs_symlink_norm(norm_vfs_path(&dest));
            if !check_boa_fs_read_ocap(&norm_src) || !check_boa_fs_write_ocap(&norm_dest) {
                return Ok(BoaVal::undefined());
            }
            let content = BOA_VFS_CACHE
                .with(|c| c.borrow().get(&norm_src).cloned())
                .unwrap_or_else(|| {
                    let bytes = if is_vfs_tombstoned(&norm_src) {
                        Vec::new()
                    } else {
                        std::fs::read(&norm_src).unwrap_or_default()
                    };
                    std::sync::Arc::new(bytes)
                });
            if let Err(msg) = try_consume_isolate_memory(content.len()) {
                return Err(JsNativeError::range().with_message(msg).into());
            }
            if let Some(idx) = norm_dest.rfind('/') {
                let parent = &norm_dest[..idx];
                BOA_VFS_DIRS.with(|d| {
                    d.borrow_mut().insert(parent.to_string());
                });
            }
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
            let norm_old = resolve_vfs_symlink_norm(norm_vfs_path(&old_p));
            let norm_new = resolve_vfs_symlink_norm(norm_vfs_path(&new_p));
            if !check_boa_fs_read_ocap(&norm_old)
                || !check_boa_fs_write_ocap(&norm_old)
                || !check_boa_fs_write_ocap(&norm_new)
            {
                return Ok(BoaVal::undefined());
            }
            let content = BOA_VFS_CACHE
                .with(|c| c.borrow_mut().remove(&norm_old))
                .unwrap_or_else(|| {
                    let bytes = if is_vfs_tombstoned(&norm_old) {
                        Vec::new()
                    } else {
                        std::fs::read(&norm_old).unwrap_or_default()
                    };
                    std::sync::Arc::new(bytes)
                });
            if norm_old != norm_new && std::path::Path::new(&norm_old).exists() {
                BOA_VFS_TOMBSTONES.with(|t| {
                    t.borrow_mut().insert(norm_old);
                });
            }
            if let Some(idx) = norm_new.rfind('/') {
                let parent = &norm_new[..idx];
                BOA_VFS_DIRS.with(|d| {
                    d.borrow_mut().insert(parent.to_string());
                });
            }
            BOA_VFS_CACHE.with(|c| {
                c.borrow_mut().insert(norm_new, content);
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
            let norm = norm_vfs_path(&path);
            if norm != path && !check_boa_fs_write_ocap(&norm) {
                return Ok(BoaVal::undefined());
            }
            let prefix = format!("{}/", norm);
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
            if std::path::Path::new(&norm).exists() {
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
            let norm = resolve_vfs_symlink_norm(norm_vfs_path(&path));
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
            if !is_vfs_tombstoned(&norm) {
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
            BOA_VFS_DIRS.with(|d| {
                d.borrow_mut().insert(norm.clone());
            });
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
            let norm = resolve_vfs_symlink_norm(norm_vfs_path(&path));
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
            if !check_boa_fs_write_ocap(&link_path) || !check_boa_fs_read_ocap(&target) {
                return Ok(BoaVal::undefined());
            }
            let norm_link = norm_vfs_path(&link_path);
            let norm_target = norm_vfs_path(&target);
            if !check_boa_fs_read_ocap(&norm_target) {
                return Ok(BoaVal::undefined());
            }
            if let Some(idx) = norm_link.rfind('/') {
                let parent = &norm_link[..idx];
                BOA_VFS_DIRS.with(|d| {
                    d.borrow_mut().insert(parent.to_string());
                });
            }
            let mut payload = Vec::with_capacity(VFS_SYMLINK_PREFIX.len() + norm_target.len());
            payload.extend_from_slice(VFS_SYMLINK_PREFIX);
            payload.extend_from_slice(norm_target.as_bytes());
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
            let norm = norm_vfs_path(&path);
            let is_sym = BOA_VFS_CACHE
                .with(|c| {
                    c.borrow()
                        .get(&norm)
                        .map(|b| b.starts_with(VFS_SYMLINK_PREFIX))
                })
                .unwrap_or_else(|| {
                    if is_vfs_tombstoned(&norm) {
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
            let norm = norm_vfs_path(&path);
            if let Some(target) = BOA_VFS_CACHE.with(|c| {
                c.borrow().get(&norm).and_then(|b| {
                    if b.starts_with(VFS_SYMLINK_PREFIX) {
                        Some(String::from_utf8_lossy(&b[VFS_SYMLINK_PREFIX.len()..]).into_owned())
                    } else {
                        None
                    }
                })
            }) {
                if !check_boa_fs_read_ocap(&target) {
                    return Ok(BoaVal::from(js_string!("")));
                }
                return Ok(BoaVal::from(js_string!(target)));
            }
            if !is_vfs_tombstoned(&norm) {
                if let Ok(target_path) = std::fs::read_link(&norm) {
                    let target_str = target_path.to_string_lossy().replace('\\', "/");
                    if check_boa_fs_read_ocap(&target_str) {
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
            let norm = resolve_vfs_symlink_norm(norm_vfs_path(&path));
            if !check_boa_fs_read_ocap(&norm) || !check_boa_fs_write_ocap(&norm) {
                return Ok(BoaVal::undefined());
            }
            let mut data = BOA_VFS_CACHE
                .with(|c| c.borrow().get(&norm).map(|arc| (**arc).clone()))
                .unwrap_or_else(|| {
                    if is_vfs_tombstoned(&norm) {
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
            let norm = resolve_vfs_symlink_norm(norm_vfs_path(&path));
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
