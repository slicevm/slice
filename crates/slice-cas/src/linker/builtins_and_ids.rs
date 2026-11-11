//! Built-in Module Classifier, Canonical Module ID Cache, and Identifier Sanitizer.

use crate::bundler::resolve_module_specifier;
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};

#[inline]
pub fn is_builtin_module(spec: &str) -> bool {
    if spec.starts_with("node:")
        || spec.starts_with("slice:")
        || spec.starts_with("bun:")
        || spec.starts_with("cloudflare:")
    {
        return true;
    }
    matches!(
        spec,
        "fs" | "fs/promises"
            | "child_process"
            | "http"
            | "https"
            | "path"
            | "path/posix"
            | "path/win32"
            | "crypto"
            | "os"
            | "util"
            | "util/types"
            | "events"
            | "buffer"
            | "process"
            | "url"
            | "stream"
            | "stream/promises"
            | "stream/web"
            | "assert"
            | "assert/strict"
            | "timers"
            | "timers/promises"
            | "worker_threads"
            | "net"
            | "dgram"
            | "async_hooks"
            | "zlib"
            | "perf_hooks"
            | "tty"
            | "module"
            | "vm"
            | "string_decoder"
            | "querystring"
            | "readline"
            | "readline/promises"
            | "diagnostics_channel"
            | "dns"
            | "dns/promises"
            | "tls"
            | "console"
            | "v8"
            | "inspector"
            | "inspector/promises"
            | "cluster"
            | "http2"
            | "wasi"
    )
}

#[inline]
pub fn is_jsx_runtime_builtin(spec: &str) -> bool {
    matches!(
        spec,
        "react"
            | "react/jsx-runtime"
            | "react/jsx-dev-runtime"
            | "preact"
            | "preact/jsx-runtime"
            | "preact/jsx-dev-runtime"
            | "hono/jsx"
            | "hono/jsx/jsx-runtime"
            | "hono/jsx/jsx-dev-runtime"
    )
}

#[inline]
pub fn normalize_builtin_specifier(spec: &str) -> String {
    if spec.starts_with("node:")
        || spec.starts_with("slice:")
        || spec.starts_with("bun:")
        || spec.starts_with("cloudflare:")
    {
        spec.to_string()
    } else if is_jsx_runtime_builtin(spec) {
        "slice:jsx".to_string()
    } else {
        format!("node:{}", spec)
    }
}

/// Validates whether `name` is a safe ASCII ECMAScript identifier (`[A-Za-z_$][A-Za-z0-9_$]*`).
#[inline]
pub fn is_valid_js_identifier(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let first = bytes[0];
    if !(first.is_ascii_alphabetic() || first == b'_' || first == b'$') {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|&b| b.is_ascii_alphanumeric() || b == b'_' || b == b'$')
}

/// Formats `exports.<name>` safely (`CAS-SEC-05`): uses dot notation for valid identifiers
/// and JSON-escaped bracket notation `exports["..."]` for arbitrary ES2022 string-literal export names.
#[inline]
pub fn format_exports_lhs(export_name: &str) -> String {
    if is_valid_js_identifier(export_name) {
        format!("exports.{}", export_name)
    } else {
        format!("exports[{:?}]", export_name)
    }
}

thread_local! {
    static CANONICAL_MOD_ID_CACHE: std::cell::RefCell<FxHashMap<PathBuf, String>> =
        std::cell::RefCell::new(FxHashMap::default());
    pub(crate) static MUTABLE_EXPORTS_CACHE: std::cell::RefCell<FxHashMap<PathBuf, FxHashSet<String>>> =
        std::cell::RefCell::new(FxHashMap::default());
    static WARNED_MISSING_PACKAGES: std::cell::RefCell<FxHashSet<String>> =
        std::cell::RefCell::new(FxHashSet::default());
}

pub fn clear_canonical_mod_id_cache() {
    CANONICAL_MOD_ID_CACHE.with(|c| c.borrow_mut().clear());
    MUTABLE_EXPORTS_CACHE.with(|c| c.borrow_mut().clear());
    WARNED_MISSING_PACKAGES.with(|c| c.borrow_mut().clear());
}

fn bare_package_name(spec: &str) -> Option<&str> {
    if spec.is_empty() || spec.starts_with('.') || spec.starts_with('/') || spec.contains(':') {
        return None;
    }
    if spec.starts_with('@') {
        let mut parts = spec.splitn(3, '/');
        let scope = parts.next()?;
        let pkg = parts.next()?;
        Some(&spec[..scope.len() + 1 + pkg.len()])
    } else {
        Some(spec.split('/').next().unwrap_or(spec))
    }
}

fn warn_if_declared_in_package_json_but_uninstalled(spec: &str, from_dir: &Path) {
    let Some(pkg_name) = bare_package_name(spec) else {
        return;
    };
    let already_warned =
        WARNED_MISSING_PACKAGES.with(|w| !w.borrow_mut().insert(pkg_name.to_string()));
    if already_warned {
        return;
    }
    let mut cur = Some(from_dir);
    while let Some(dir) = cur {
        let pkg_json = dir.join("package.json");
        if pkg_json.is_file() {
            if let Ok(raw) = std::fs::read_to_string(&pkg_json) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                    let declared = ["dependencies", "devDependencies", "peerDependencies"]
                        .iter()
                        .any(|sec| {
                            val.get(*sec)
                                .and_then(|v| v.as_object())
                                .is_some_and(|obj| obj.contains_key(pkg_name))
                        });
                    if declared && !dir.join("node_modules").join(pkg_name).exists() {
                        eprintln!(
                            "Hint: package '{}' is declared in {} but missing from node_modules/ — run 'npm install' (or pnpm/bun install) before packing or running.",
                            pkg_name,
                            pkg_json.display()
                        );
                        return;
                    }
                }
            }
        }
        cur = dir.parent();
    }
}

#[inline]
pub fn canonical_mod_id(path: &Path) -> String {
    CANONICAL_MOD_ID_CACHE.with(|cache| {
        if let Some(cached) = cache.borrow().get(path) {
            return cached.clone();
        }
        let s = std::fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .replace('\\', "/");
        let id = s.strip_prefix("//?/").map(|r| r.to_string()).unwrap_or(s);
        cache.borrow_mut().insert(path.to_path_buf(), id.clone());
        id
    })
}

pub fn resolve_spec_and_record(spec: &str, from_dir: &Path, deps_out: &mut Vec<PathBuf>) -> String {
    if is_builtin_module(spec) {
        normalize_builtin_specifier(spec)
    } else if let Some(resolved) = resolve_module_specifier(from_dir, spec) {
        let id = canonical_mod_id(&resolved);
        deps_out.push(resolved);
        id
    } else if is_jsx_runtime_builtin(spec) {
        normalize_builtin_specifier(spec)
    } else {
        warn_if_declared_in_package_json_but_uninstalled(spec, from_dir);
        spec.to_string()
    }
}
