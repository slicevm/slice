//! Node/ESM Module Specifier Resolver, `package.json` Conditional `"exports"` / `"imports"`,
//! and Cycle-Safe `tsconfig.json` `"paths"` Resolver.

use crate::linker::clear_canonical_mod_id_cache;
use anyhow::{Result, bail};
use rustc_hash::FxHashMap;
use std::borrow::Cow;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

thread_local! {
    static PACKAGE_JSON_CACHE: std::cell::RefCell<FxHashMap<PathBuf, Option<Arc<serde_json::Value>>>> =
        std::cell::RefCell::new(FxHashMap::default());
    static TSCONFIG_VALUE_CACHE: std::cell::RefCell<FxHashMap<PathBuf, Option<Arc<serde_json::Value>>>> =
        std::cell::RefCell::new(FxHashMap::default());
}

pub struct ResolverCacheGuard;

impl Default for ResolverCacheGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl ResolverCacheGuard {
    pub fn new() -> Self {
        clear_canonical_mod_id_cache();
        PACKAGE_JSON_CACHE.with(|c| c.borrow_mut().clear());
        TSCONFIG_VALUE_CACHE.with(|c| c.borrow_mut().clear());
        Self
    }
}

impl Drop for ResolverCacheGuard {
    fn drop(&mut self) {
        clear_canonical_mod_id_cache();
        PACKAGE_JSON_CACHE.with(|c| c.borrow_mut().clear());
        TSCONFIG_VALUE_CACHE.with(|c| c.borrow_mut().clear());
    }
}

#[inline]
fn load_cached_package_json(pkg_json_path: &Path) -> Option<Arc<serde_json::Value>> {
    PACKAGE_JSON_CACHE.with(|cache| {
        if let Some(cached) = cache.borrow().get(pkg_json_path) {
            return cached.clone();
        }
        let parsed = if pkg_json_path.is_file() {
            std::fs::read_to_string(pkg_json_path)
                .ok()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                .map(Arc::new)
        } else {
            None
        };
        cache
            .borrow_mut()
            .insert(pkg_json_path.to_path_buf(), parsed.clone());
        parsed
    })
}

fn strip_toml_inline_comment(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut in_quote: Option<u8> = None;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = in_quote {
            if q == b'"' && b == b'\\' {
                i = (i + 2).min(bytes.len());
                continue;
            }
            if b == q {
                in_quote = None;
            }
            i += 1;
            continue;
        }
        if b == b'"' || b == b'\'' {
            in_quote = Some(b);
            i += 1;
            continue;
        }
        if b == b'#' {
            return &s[..i];
        }
        i += 1;
    }
    s
}

/// Resolves a folder or file path to its concrete entrypoint script (`slice.toml`, `package.json`, `src/index.ts`, `src/index.tsx`, etc.).
pub fn resolve_project_entrypoint(target: &Path, explicit_entry: Option<&str>) -> Result<PathBuf> {
    if target.is_file() {
        return Ok(target.to_path_buf());
    }
    if !target.is_dir() {
        bail!("Target path does not exist: {}", target.display());
    }

    if let Some(exp) = explicit_entry {
        let candidate = target.join(exp);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    // 1. Check `slice.toml`
    let slice_toml = target.join("slice.toml");
    if slice_toml.is_file()
        && let Ok(raw) = std::fs::read_to_string(&slice_toml)
    {
        for line in raw.lines() {
            let trimmed = strip_toml_inline_comment(line).trim();
            if let Some(rest) = trimmed.strip_prefix("entrypoint")
                && rest
                    .as_bytes()
                    .first()
                    .is_some_and(|b| b.is_ascii_whitespace() || *b == b'=')
                && let Some((_, val)) = rest.split_once('=')
            {
                let clean = val.trim().trim_matches('"').trim_matches('\'');
                if !clean.is_empty() {
                    let candidate = target.join(clean);
                    if candidate.is_file() {
                        return Ok(candidate);
                    }
                }
            }
        }
    }

    // 2. Check `package.json`
    if let Some(val) = load_cached_package_json(&target.join("package.json")) {
        for field in ["module", "main"] {
            if let Some(entry_str) = val.get(field).and_then(|v| v.as_str()) {
                let candidate = target.join(entry_str);
                if candidate.is_file() {
                    return Ok(candidate);
                }
            }
        }
    }

    // 3. Conventional entrypoints (including `.tsx` and `.jsx`)
    for conv in [
        "src/index.ts",
        "src/index.tsx",
        "index.ts",
        "index.tsx",
        "src/main.ts",
        "src/main.tsx",
        "src/index.js",
        "src/index.jsx",
        "index.js",
        "index.jsx",
    ] {
        let candidate = target.join(conv);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    bail!(
        "Could not find an entrypoint in directory {}",
        target.display()
    )
}

pub fn try_file_with_extensions(base: &Path) -> Option<PathBuf> {
    if base.is_file() {
        return Some(base.to_path_buf());
    }
    // TypeScript ESM convention: `import { x } from "./foo.js"` where `./foo.ts` or `./foo.tsx` exists on disk
    if let Some(ext) = base.extension().and_then(|e| e.to_str()) {
        let ts_exts: &[&str] = match ext {
            "js" => &["ts", "tsx"],
            "jsx" => &["tsx"],
            "mjs" => &["mts"],
            "cjs" => &["cts"],
            _ => &[],
        };
        for &ts_ext in ts_exts {
            let ts_cand = base.with_extension(ts_ext);
            if ts_cand.is_file() {
                return Some(ts_cand);
            }
        }
    }
    for ext in ["ts", "tsx", "js", "jsx", "mjs", "cjs", "json", "wasm"] {
        let with_ext = base.with_extension(ext);
        if with_ext.is_file() {
            return Some(with_ext);
        }
    }
    if base.is_dir() {
        let pkg_json = base.join("package.json");
        if let Some(val) = load_cached_package_json(&pkg_json) {
            if let Some(exports) = val.get("exports")
                && let Some(found) = resolve_package_exports(base, exports, ".")
            {
                return Some(found);
            }
            for field in ["module", "main"] {
                if let Some(entry_str) = val.get(field).and_then(|v| v.as_str())
                    && !has_parent_traversal_segment(entry_str)
                    && let Some(found) = try_file_with_extensions(&base.join(entry_str))
                {
                    return Some(found);
                }
            }
        }
        for idx in [
            "index.ts",
            "index.tsx",
            "index.js",
            "index.jsx",
            "index.mjs",
            "index.cjs",
        ] {
            let candidate = base.join(idx);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Checks whether a relative package target or wildcard substitution contains `..` path traversal,
/// root prefixes, or backslash separators (`CAS-SEC-02`).
#[inline]
fn has_parent_traversal_segment(rel_str: &str) -> bool {
    if rel_str.contains('\\') {
        return true;
    }
    Path::new(rel_str).components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    })
}

fn resolve_exports_target(
    pkg_dir: &Path,
    target: &serde_json::Value,
    wildcard_sub: Option<&str>,
) -> Option<PathBuf> {
    match target {
        serde_json::Value::String(s) => {
            if s.ends_with(".d.ts") || s.ends_with(".d.mts") || s.ends_with(".d.cts") {
                return None;
            }
            // Per Node.js PACKAGE_TARGET_RESOLVE spec (`CAS-SEC-02`), targets must start with "./"
            // and neither target nor wildcard_sub may contain ".." traversal segments.
            if !s.starts_with("./") {
                return None;
            }
            if let Some(sub) = wildcard_sub
                && has_parent_traversal_segment(sub)
            {
                return None;
            }
            let expanded = if let Some(sub) = wildcard_sub {
                s.replace('*', sub)
            } else {
                s.clone()
            };
            let rel = expanded.strip_prefix("./").unwrap_or(&expanded);
            if rel.is_empty() || has_parent_traversal_segment(rel) {
                return None;
            }
            try_file_with_extensions(&pkg_dir.join(rel))
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                if let Some(found) = resolve_exports_target(pkg_dir, item, wildcard_sub) {
                    return Some(found);
                }
            }
            None
        }
        serde_json::Value::Object(map) => {
            const CONDITIONS: &[&str] = &[
                "workerd",
                "worker",
                "edge-light",
                "browser",
                "import",
                "module",
                "require",
                "node",
                "default",
            ];
            for &cond in CONDITIONS {
                if let Some(v) = map.get(cond)
                    && let Some(found) = resolve_exports_target(pkg_dir, v, wildcard_sub)
                {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn match_sorted_wildcard_map<'a>(
    map: &'a serde_json::Map<String, serde_json::Value>,
    key: &'a str,
    mut try_target: impl FnMut(&'a serde_json::Value, Option<&'a str>) -> Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(exact) = map.get(key)
        && let Some(found) = try_target(exact, None)
    {
        return Some(found);
    }
    let mut pattern_keys: Vec<(&String, &serde_json::Value)> =
        map.iter().filter(|(k, _)| k.contains('*')).collect();
    pattern_keys.sort_by_key(|(b, _)| std::cmp::Reverse(b.len()));
    for (pat_key, target_val) in pattern_keys {
        if let Some((prefix, suffix)) = pat_key.split_once('*')
            && key.starts_with(prefix)
            && key.ends_with(suffix)
            && key.len() >= prefix.len() + suffix.len()
        {
            let matched = &key[prefix.len()..key.len() - suffix.len()];
            if let Some(found) = try_target(target_val, Some(matched)) {
                return Some(found);
            }
        }
    }
    None
}

fn resolve_subpath_pattern_map(
    pkg_dir: &Path,
    map: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<PathBuf> {
    match_sorted_wildcard_map(map, key, |target, sub| {
        resolve_exports_target(pkg_dir, target, sub)
    })
}

fn resolve_package_exports(
    pkg_dir: &Path,
    exports: &serde_json::Value,
    subpath: &str,
) -> Option<PathBuf> {
    if subpath != "." && has_parent_traversal_segment(subpath.strip_prefix("./").unwrap_or(subpath))
    {
        return None;
    }
    match exports {
        serde_json::Value::String(_) | serde_json::Value::Array(_) => {
            if subpath == "." {
                resolve_exports_target(pkg_dir, exports, None)
            } else {
                None
            }
        }
        serde_json::Value::Object(map) => {
            let has_dot_keys = map.keys().any(|k| k.starts_with('.'));
            if !has_dot_keys {
                return if subpath == "." {
                    resolve_exports_target(pkg_dir, exports, None)
                } else {
                    None
                };
            }
            resolve_subpath_pattern_map(pkg_dir, map, subpath)
        }
        _ => None,
    }
}

fn split_bare_package_specifier(spec: &str) -> (&str, String) {
    if let Some(after_at) = spec.strip_prefix('@') {
        if let Some(first_slash) = after_at.find('/') {
            let rest = &after_at[first_slash + 1..];
            if let Some(second_slash) = rest.find('/') {
                let pkg_len = 1 + first_slash + 1 + second_slash;
                return (&spec[..pkg_len], format!(".{}", &spec[pkg_len..]));
            }
        }
        (spec, ".".to_string())
    } else if let Some(slash) = spec.find('/') {
        (&spec[..slash], format!(".{}", &spec[slash..]))
    } else {
        (spec, ".".to_string())
    }
}

fn skip_ws_and_jsonc_comments(bytes: &[u8], mut k: usize) -> usize {
    while k < bytes.len() {
        if bytes[k].is_ascii_whitespace() {
            k += 1;
        } else if bytes[k] == b'/' && k + 1 < bytes.len() && bytes[k + 1] == b'/' {
            k += 2;
            while k < bytes.len() && bytes[k] != b'\n' {
                k += 1;
            }
        } else if bytes[k] == b'/' && k + 1 < bytes.len() && bytes[k + 1] == b'*' {
            k += 2;
            while k + 1 < bytes.len() && !(bytes[k] == b'*' && bytes[k + 1] == b'/') {
                k += 1;
            }
            k = (k + 2).min(bytes.len());
        } else {
            break;
        }
    }
    k
}

pub fn strip_jsonc(raw: &str) -> Cow<'_, str> {
    let bytes = raw.as_bytes();
    let has_comments = memchr::memmem::find(bytes, b"//").is_some()
        || memchr::memmem::find(bytes, b"/*").is_some();
    if !has_comments && memchr::memchr(b',', bytes).is_none() {
        return Cow::Borrowed(raw);
    }

    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let mut in_str = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            out.push(b);
            if b == b'\\' && i + 1 < bytes.len() {
                out.push(bytes[i + 1]);
                i += 2;
                continue;
            }
            if b == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if b == b'"' {
            in_str = true;
            out.push(b);
            i += 1;
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() {
            if bytes[i + 1] == b'/' {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            } else if bytes[i + 1] == b'*' {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
                continue;
            }
        }
        if b == b','
            && matches!(
                bytes.get(skip_ws_and_jsonc_comments(bytes, i + 1)),
                Some(b'}' | b']')
            )
        {
            i += 1;
            continue;
        }
        out.push(b);
        i += 1;
    }
    Cow::Owned(String::from_utf8_lossy(&out).into_owned())
}

fn resolve_package_imports(
    pkg_dir: &Path,
    imports: &serde_json::Value,
    spec: &str,
) -> Option<PathBuf> {
    let serde_json::Value::Object(map) = imports else {
        return None;
    };
    resolve_subpath_pattern_map(pkg_dir, map, spec)
}

/// Loads and caches `tsconfig.json` / `jsconfig.json` with cycle protection (`CAS-SEC-03`).
/// Inserts `None` into `TSCONFIG_VALUE_CACHE` before recursing into `"extends"` so circular
/// `"extends"` chains terminate immediately instead of overflowing the call stack.
fn load_tsconfig_value(config_path: &Path) -> Option<Arc<serde_json::Value>> {
    let key = std::fs::canonicalize(config_path).unwrap_or_else(|_| config_path.to_path_buf());
    let already_cached = TSCONFIG_VALUE_CACHE.with(|cache| cache.borrow().get(&key).cloned());
    if let Some(cached) = already_cached {
        return cached;
    }
    // Insert `None` cycle sentinel before recursing into `extends`
    TSCONFIG_VALUE_CACHE.with(|cache| {
        cache.borrow_mut().insert(key.clone(), None);
    });
    let parsed = load_tsconfig_value_uncached(&key).map(Arc::new);
    TSCONFIG_VALUE_CACHE.with(|cache| {
        cache.borrow_mut().insert(key, parsed.clone());
    });
    parsed
}

fn resolve_tsconfig_extends_path(parent_dir: &Path, ext_str: &str) -> Option<PathBuf> {
    let try_with_json = |cand: PathBuf| -> Option<PathBuf> {
        [
            cand.clone(),
            PathBuf::from(format!("{}.json", cand.display())),
            cand.join("tsconfig.json"),
        ]
        .into_iter()
        .find(|p| p.is_file())
    };

    if ext_str.starts_with('.') || Path::new(ext_str).is_absolute() {
        return try_with_json(parent_dir.join(ext_str));
    }

    let mut cur = Some(parent_dir);
    while let Some(dir) = cur {
        let nm_cand = dir.join("node_modules").join(ext_str);
        if let Some(found) = try_with_json(nm_cand.clone()) {
            return Some(found);
        }
        if let Some(pv) = load_cached_package_json(&nm_cand.join("package.json"))
            && let Some(ts_rel) = pv.get("tsconfig").and_then(|v| v.as_str())
            && let Some(found) = try_with_json(nm_cand.join(ts_rel))
        {
            return Some(found);
        }
        cur = dir.parent();
    }
    None
}

fn load_tsconfig_value_uncached(config_path: &Path) -> Option<serde_json::Value> {
    if !config_path.is_file() {
        return None;
    }
    let raw = std::fs::read_to_string(config_path).ok()?;
    let mut val = serde_json::from_str::<serde_json::Value>(&raw)
        .or_else(|_| {
            let cleaned = strip_jsonc(&raw);
            serde_json::from_str::<serde_json::Value>(&cleaned)
        })
        .ok()?;
    let extends_list: Vec<String> = match val.get("extends") {
        Some(serde_json::Value::String(s)) => vec![s.clone()],
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => Vec::new(),
    };
    if !extends_list.is_empty()
        && let Some(parent_dir) = config_path.parent()
    {
        // TypeScript 5.0+ `extends: [base1, base2]`: later bases override earlier bases,
        // and the child config overrides all bases. Iterating in reverse with `or_insert` achieves this.
        for ext_str in extends_list.iter().rev() {
            if let Some(ext_path) = resolve_tsconfig_extends_path(parent_dir, ext_str)
                && let Some(parent_val) = load_tsconfig_value(&ext_path)
                && let Some(parent_co) = parent_val
                    .get("compilerOptions")
                    .and_then(|c| c.as_object())
            {
                let co_entry = val
                    .as_object_mut()?
                    .entry("compilerOptions")
                    .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
                if let Some(co_map) = co_entry.as_object_mut() {
                    for (k, v) in parent_co {
                        co_map.entry(k.clone()).or_insert_with(|| v.clone());
                    }
                }
            }
        }
    }
    Some(val)
}

fn resolve_tsconfig_paths(from_dir: &Path, spec: &str) -> Option<PathBuf> {
    let mut curr = Some(from_dir);
    while let Some(dir) = curr {
        for cfg_name in ["tsconfig.json", "jsconfig.json"] {
            let cfg_path = dir.join(cfg_name);
            let Some(val) = load_tsconfig_value(&cfg_path) else {
                continue;
            };
            let Some(co) = val.get("compilerOptions").and_then(|v| v.as_object()) else {
                continue;
            };
            let has_explicit_base_url = co.get("baseUrl").and_then(|v| v.as_str()).is_some();
            let base_url_dir = match co.get("baseUrl").and_then(|v| v.as_str()) {
                Some(b) => dir.join(b),
                None => dir.to_path_buf(),
            };
            if let Some(paths_map) = co.get("paths").and_then(|v| v.as_object())
                && let Some(found) =
                    match_sorted_wildcard_map(paths_map, spec, |target_val, sub| {
                        target_val
                            .as_array()?
                            .iter()
                            .filter_map(|x| x.as_str())
                            .find_map(|t| {
                                let expanded = match sub {
                                    Some(m) => Cow::Owned(t.replace('*', m)),
                                    None => Cow::Borrowed(t),
                                };
                                let rel = expanded.strip_prefix("./").unwrap_or(&expanded);
                                try_file_with_extensions(&base_url_dir.join(rel))
                            })
                    })
            {
                return Some(found);
            }
            if has_explicit_base_url
                && let Some(found) = try_file_with_extensions(&base_url_dir.join(spec))
            {
                return Some(found);
            }
        }
        curr = dir.parent();
    }
    None
}

pub fn resolve_module_specifier(from_dir: &Path, spec: &str) -> Option<PathBuf> {
    if from_dir == Path::new(crate::linker::VIRTUAL_NO_FS_DIR) {
        return None;
    }
    if spec.starts_with("./") || spec.starts_with("../") || spec.starts_with('/') {
        return try_file_with_extensions(&from_dir.join(spec));
    }

    // 1. `package.json` Subpath `"imports"` (`#...`)
    if spec.starts_with('#') {
        let mut curr = Some(from_dir);
        while let Some(dir) = curr {
            let pkg_json = dir.join("package.json");
            if let Some(val) = load_cached_package_json(&pkg_json)
                && let Some(imports) = val.get("imports")
                && let Some(found) = resolve_package_imports(dir, imports, spec)
            {
                return Some(found);
            }
            curr = dir.parent();
        }
        return None;
    }

    // 2. `tsconfig.json` / `jsconfig.json` `"paths"` & `"baseUrl"` (`@/*`, `~/*`, etc.)
    if !from_dir
        .components()
        .any(|c| c.as_os_str() == "node_modules")
        && let Some(found) = resolve_tsconfig_paths(from_dir, spec)
    {
        return Some(found);
    }

    // 3. `node_modules` bare package & conditional `"exports"` resolution
    let (pkg_name, subpath) = split_bare_package_specifier(spec);
    if has_parent_traversal_segment(pkg_name) {
        return None;
    }

    let mut curr = Some(from_dir);
    while let Some(dir) = curr {
        let pkg_dir = dir.join("node_modules").join(pkg_name);
        if pkg_dir.is_dir() {
            let pkg_json = pkg_dir.join("package.json");
            if let Some(val) = load_cached_package_json(&pkg_json)
                && let Some(exports) = val.get("exports")
                && let Some(found) = resolve_package_exports(&pkg_dir, exports, &subpath)
            {
                return Some(found);
            }
        }
        if !has_parent_traversal_segment(spec) {
            let nm_candidate = dir.join("node_modules").join(spec);
            if let Some(found) = try_file_with_extensions(&nm_candidate) {
                return Some(found);
            }
        }
        curr = dir.parent();
    }
    None
}
