//! OCap-Enforced Virtual Filesystem (`BOA_VFS_CACHE`) and Host Filesystem Bindings (`node:fs`).

use crate::bindings::helpers::{
    bytes_to_js_uint8_array, extract_buffer_source_bytes, try_consume_isolate_memory,
};
use crate::boa_realm::{
    check_boa_fs_read_ocap, check_boa_fs_write_ocap, norm_vfs_path, BOA_DET_STATE, BOA_VFS_CACHE,
    BOA_VFS_DIRS,
};
use crate::intrinsics::fill_random_bytes;

const VFS_SYMLINK_PREFIX: &[u8] = b"\x00__SLICE_SYMLINK__:";

pub(crate) fn resolve_vfs_symlink_norm(norm: &str) -> String {
    let mut current = norm.to_string();
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
            let norm = resolve_vfs_symlink_norm(&norm_vfs_path(&path));
            if !check_boa_fs_read_ocap(&norm) {
                return Ok(BoaVal::from(js_string!("")));
            }
            if let Some(arc_bytes) = BOA_VFS_CACHE.with(|c| c.borrow().get(&norm).cloned()) {
                let content = String::from_utf8_lossy(&arc_bytes).into_owned();
                return Ok(BoaVal::from(js_string!(content)));
            }
            let bytes = std::fs::read(&norm)
                .or_else(|_| std::fs::read(&path))
                .unwrap_or_default();
            let content = String::from_utf8_lossy(&bytes).into_owned();
            Ok(BoaVal::from(js_string!(content)))
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
            let norm = resolve_vfs_symlink_norm(&norm_vfs_path(&path));
            if !check_boa_fs_read_ocap(&norm) {
                return bytes_to_js_uint8_array(&[], context);
            }
            if let Some(arc_bytes) = BOA_VFS_CACHE.with(|c| c.borrow().get(&norm).cloned()) {
                return bytes_to_js_uint8_array(&arc_bytes, context);
            }
            let bytes = std::fs::read(&norm)
                .or_else(|_| std::fs::read(&path))
                .unwrap_or_default();
            bytes_to_js_uint8_array(&bytes, context)
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
            let norm = resolve_vfs_symlink_norm(&norm_vfs_path(&path));
            if !check_boa_fs_write_ocap(&norm) {
                return Ok(BoaVal::undefined());
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
            let norm = resolve_vfs_symlink_norm(&norm_vfs_path(&path));
            if !check_boa_fs_write_ocap(&norm) {
                return Ok(BoaVal::undefined());
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
            BOA_VFS_CACHE.with(|c| {
                let mut map = c.borrow_mut();
                let entry = map.entry(norm.clone()).or_insert_with(|| {
                    let initial = if check_boa_fs_read_ocap(&norm) {
                        std::fs::read(&norm)
                            .or_else(|_| std::fs::read(&path))
                            .unwrap_or_default()
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
            let norm = resolve_vfs_symlink_norm(&norm_vfs_path(&path));
            if !check_boa_fs_read_ocap(&norm) {
                return Ok(BoaVal::from(false));
            }
            let exists = BOA_VFS_CACHE.with(|c| c.borrow().contains_key(&norm))
                || BOA_VFS_DIRS.with(|d| d.borrow().contains(&norm))
                || std::path::Path::new(&norm).exists()
                || std::path::Path::new(&path).exists();
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
            let norm = norm_vfs_path(&path);
            BOA_VFS_DIRS.with(|d| {
                d.borrow_mut().insert(norm);
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
            let norm_dir = resolve_vfs_symlink_norm(&norm_vfs_path(&path));
            if !check_boa_fs_read_ocap(&norm_dir) {
                return Ok(BoaVal::from(js_string!("[]")));
            }
            let prefix = format!("{}/", norm_dir);
            let mut entries_set = std::collections::BTreeSet::new();
            BOA_VFS_CACHE.with(|c| {
                for k in c.borrow().keys() {
                    if let Some(rest) = k.strip_prefix(&prefix) {
                        if !rest.contains('/') && !rest.is_empty() {
                            entries_set.insert(rest.to_string());
                        }
                    }
                }
            });
            if let Ok(rd) = std::fs::read_dir(&norm_dir).or_else(|_| std::fs::read_dir(&path)) {
                for entry in rd.flatten() {
                    entries_set.insert(entry.file_name().to_string_lossy().to_string());
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
            let norm = resolve_vfs_symlink_norm(&norm_vfs_path(&path));
            if !check_boa_fs_read_ocap(&norm) {
                return Ok(BoaVal::from(0));
            }
            let size = BOA_VFS_CACHE
                .with(|c| c.borrow().get(&norm).map(|v| v.len() as u64))
                .unwrap_or_else(|| {
                    std::fs::metadata(&norm)
                        .or_else(|_| std::fs::metadata(&path))
                        .map(|m| m.len())
                        .unwrap_or(0)
                });
            Ok(BoaVal::from(size as f64))
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
            if !check_boa_fs_read_ocap(&path) || BOA_DET_STATE.with(|s| s.get().is_some()) {
                return Ok(BoaVal::from(1_700_000_000_000.0));
            }
            let norm = resolve_vfs_symlink_norm(&norm_vfs_path(&path));
            if !check_boa_fs_read_ocap(&norm) {
                return Ok(BoaVal::from(1_700_000_000_000.0));
            }
            let mtime_ms = std::fs::metadata(&norm)
                .or_else(|_| std::fs::metadata(&path))
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as f64)
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
            let norm = resolve_vfs_symlink_norm(&norm_vfs_path(&path));
            if !check_boa_fs_read_ocap(&norm) {
                return Ok(BoaVal::from(false));
            }
            let prefix = format!("{}/", norm);
            let is_dir = BOA_VFS_DIRS.with(|d| d.borrow().contains(&norm))
                || BOA_VFS_CACHE.with(|c| c.borrow().keys().any(|k| k.starts_with(&prefix)))
                || std::path::Path::new(&norm).is_dir()
                || std::path::Path::new(&path).is_dir();
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
            let norm_src = resolve_vfs_symlink_norm(&norm_vfs_path(&src));
            let norm_dest = resolve_vfs_symlink_norm(&norm_vfs_path(&dest));
            if !check_boa_fs_read_ocap(&norm_src) || !check_boa_fs_write_ocap(&norm_dest) {
                return Ok(BoaVal::undefined());
            }
            let content = BOA_VFS_CACHE
                .with(|c| c.borrow().get(&norm_src).cloned())
                .unwrap_or_else(|| {
                    std::sync::Arc::new(
                        std::fs::read(&norm_src)
                            .or_else(|_| std::fs::read(&src))
                            .unwrap_or_default(),
                    )
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
            let norm_old = resolve_vfs_symlink_norm(&norm_vfs_path(&old_p));
            let norm_new = resolve_vfs_symlink_norm(&norm_vfs_path(&new_p));
            if !check_boa_fs_read_ocap(&norm_old) || !check_boa_fs_write_ocap(&norm_new) {
                return Ok(BoaVal::undefined());
            }
            let content = BOA_VFS_CACHE
                .with(|c| c.borrow_mut().remove(&norm_old))
                .unwrap_or_else(|| {
                    std::sync::Arc::new(
                        std::fs::read(&norm_old)
                            .or_else(|_| std::fs::read(&old_p))
                            .unwrap_or_default(),
                    )
                });
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
            let p = std::path::Path::new(&path);
            if p.exists() {
                if p.is_dir() {
                    let _ = std::fs::remove_dir_all(p);
                } else {
                    let _ = std::fs::remove_file(p);
                }
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
            let norm = resolve_vfs_symlink_norm(&norm_vfs_path(&path));
            if !check_boa_fs_read_ocap(&norm) {
                return Ok(BoaVal::from(js_string!("")));
            }
            if BOA_VFS_CACHE.with(|c| c.borrow().contains_key(&norm))
                || BOA_VFS_DIRS.with(|d| d.borrow().contains(&norm))
            {
                let resolved = if norm.starts_with('/')
                    || (norm.len() >= 2 && norm.as_bytes()[1] == b':')
                {
                    norm
                } else {
                    let cwd = std::env::current_dir()
                        .map(|p| norm_vfs_path(&p.to_string_lossy()))
                        .unwrap_or_else(|_| "/workspace".to_string());
                    format!("{}/{}", cwd.trim_end_matches('/'), norm)
                };
                return Ok(BoaVal::from(js_string!(resolved)));
            }
            if let Ok(canon) = std::fs::canonicalize(&norm).or_else(|_| std::fs::canonicalize(&path))
            {
                let mut s = canon.to_string_lossy().replace('\\', "/");
                if let Some(stripped) = s.strip_prefix("//?/") {
                    s = stripped.to_string();
                }
                return Ok(BoaVal::from(js_string!(s)));
            }
            let fallback = if norm.starts_with('/')
                || (norm.len() >= 2 && norm.as_bytes()[1] == b':')
            {
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
            let _ = check_boa_fs_write_ocap(&path);
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
                    std::fs::symlink_metadata(&path)
                        .map(|m| m.file_type().is_symlink())
                        .unwrap_or(false)
                });
            Ok(BoaVal::from(is_sym))
        }),
    );
}
