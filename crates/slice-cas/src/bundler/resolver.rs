//! Node/ESM Module Specifier Resolver, `package.json` Conditional `"exports"` / `"imports"`,
//! and Cycle-Safe `tsconfig.json` `"paths"` Resolver.

use crate::linker::clear_canonical_mod_id_cache;
use anyhow::{bail, Result};
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
    if slice_toml.is_file() {
        if let Ok(raw) = std::fs::read_to_string(&slice_toml) {
            for line in raw.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("entrypoint") {
                    if let Some((_, val)) = rest.split_once('=') {
                        let clean = val.trim().trim_matches('"').trim_matches('\'');
                        let candidate = target.join(clean);
                        if candidate.is_file() {
                            return Ok(candidate);
                        }
                    }
                }
            }
        }
    }

    // 2. Check `package.json`
    let pkg_json = target.join("package.json");
    if pkg_json.is_file() {
        if let Ok(raw) = std::fs::read_to_string(&pkg_json) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                for field in ["module", "main"] {
                    if let Some(entry_str) = val.get(field).and_then(|v| v.as_str()) {
                        let candidate = target.join(entry_str);
                        if candidate.is_file() {
                            return Ok(candidate);
                        }
                    }
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
        match ext {
            "js" => {
                for ts_ext in ["ts", "tsx"] {
                    let ts_cand = base.with_extension(ts_ext);
                    if ts_cand.is_file() {
                        return Some(ts_cand);
                    }
                }
            }
            "jsx" => {
                let tsx_cand = base.with_extension("tsx");
                if tsx_cand.is_file() {
                    return Some(tsx_cand);
                }
            }
            "mjs" => {
                let mts_cand = base.with_extension("mts");
                if mts_cand.is_file() {
                    return Some(mts_cand);
                }
            }
            "cjs" => {
                let cts_cand = base.with_extension("cts");
                if cts_cand.is_file() {
                    return Some(cts_cand);
                }
            }
            _ => {}
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
            if let Some(exports) = val.get("exports") {
                if let Some(found) = resolve_package_exports(base, exports, ".") {
                    return Some(found);
                }
            }
            if let Some(mod_str) = val.get("module").and_then(|v| v.as_str()) {
                if !has_parent_traversal_segment(mod_str) {
                    if let Some(found) = try_file_with_extensions(&base.join(mod_str)) {
                        return Some(found);
                    }
                }
            }
            if let Some(main_str) = val.get("main").and_then(|v| v.as_str()) {
                if !has_parent_traversal_segment(main_str) {
                    if let Some(found) = try_file_with_extensions(&base.join(main_str)) {
                        return Some(found);
                    }
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
            if let Some(sub) = wildcard_sub {
                if has_parent_traversal_segment(sub) {
                    return None;
                }
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
                if let Some(v) = map.get(cond) {
                    if let Some(found) = resolve_exports_target(pkg_dir, v, wildcard_sub) {
                        return Some(found);
                    }
                }
            }
            None
        }
        _ => None,
    }
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
                if subpath == "." {
                    return resolve_exports_target(pkg_dir, exports, None);
                }
                return None;
            }
            if let Some(exact) = map.get(subpath) {
                if let Some(found) = resolve_exports_target(pkg_dir, exact, None) {
                    return Some(found);
                }
            }
            let mut pattern_keys: Vec<(&String, &serde_json::Value)> =
                map.iter().filter(|(k, _)| k.contains('*')).collect();
            pattern_keys.sort_by_key(|(b, _)| std::cmp::Reverse(b.len()));
            for (pat_key, target_val) in pattern_keys {
                if let Some((prefix, suffix)) = pat_key.split_once('*') {
                    if subpath.starts_with(prefix)
                        && subpath.ends_with(suffix)
                        && subpath.len() >= prefix.len() + suffix.len()
                    {
                        let matched = &subpath[prefix.len()..subpath.len() - suffix.len()];
                        if let Some(found) =
                            resolve_exports_target(pkg_dir, target_val, Some(matched))
                        {
                            return Some(found);
                        }
                    }
                }
            }
            None
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

pub fn strip_jsonc(raw: &str) -> Cow<'_, str> {
    let bytes = raw.as_bytes();
    let has_comments = memchr::memmem::find(bytes, b"//").is_some()
        || memchr::memmem::find(bytes, b"/*").is_some();
    if !has_comments && memchr::memchr(b',', bytes).is_none() {
        return Cow::Borrowed(raw);
    }

    let mut no_comments = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let mut in_str = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            no_comments.push(b);
            if b == b'\\' && i + 1 < bytes.len() {
                no_comments.push(bytes[i + 1]);
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
            no_comments.push(b);
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
        no_comments.push(b);
        i += 1;
    }

    let mut out = Vec::with_capacity(no_comments.len());
    let mut j = 0;
    let mut in_str2 = false;
    while j < no_comments.len() {
        let b = no_comments[j];
        if in_str2 {
            out.push(b);
            if b == b'\\' && j + 1 < no_comments.len() {
                out.push(no_comments[j + 1]);
                j += 2;
                continue;
            }
            if b == b'"' {
                in_str2 = false;
            }
            j += 1;
            continue;
        }
        if b == b'"' {
            in_str2 = true;
            out.push(b);
            j += 1;
            continue;
        }
        if b == b',' {
            let mut k = j + 1;
            while k < no_comments.len() && no_comments[k].is_ascii_whitespace() {
                k += 1;
            }
            if k < no_comments.len() && (no_comments[k] == b'}' || no_comments[k] == b']') {
                j += 1;
                continue;
            }
        }
        out.push(b);
        j += 1;
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
    if let Some(exact) = map.get(spec) {
        if let Some(found) = resolve_exports_target(pkg_dir, exact, None) {
            return Some(found);
        }
    }
    let mut pattern_keys: Vec<(&String, &serde_json::Value)> =
        map.iter().filter(|(k, _)| k.contains('*')).collect();
    pattern_keys.sort_by_key(|(b, _)| std::cmp::Reverse(b.len()));
    for (pat_key, target_val) in pattern_keys {
        if let Some((prefix, suffix)) = pat_key.split_once('*') {
            if spec.starts_with(prefix)
                && spec.ends_with(suffix)
                && spec.len() >= prefix.len() + suffix.len()
            {
                let matched = &spec[prefix.len()..spec.len() - suffix.len()];
                if let Some(found) = resolve_exports_target(pkg_dir, target_val, Some(matched)) {
                    return Some(found);
                }
            }
        }
    }
    None
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
    if let Some(ext_str) = val
        .get("extends")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
    {
        if let Some(parent_dir) = config_path.parent() {
            let mut ext_path = parent_dir.join(&ext_str);
            if !ext_path.is_file() && ext_path.extension().is_none() {
                ext_path.set_extension("json");
            }
            if let Some(parent_val) = load_tsconfig_value(&ext_path) {
                if let Some(parent_co) = parent_val
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
            if let Some(paths_map) = co.get("paths").and_then(|v| v.as_object()) {
                if let Some(exact_targets) = paths_map.get(spec).and_then(|v| v.as_array()) {
                    for t in exact_targets.iter().filter_map(|x| x.as_str()) {
                        let rel = t.strip_prefix("./").unwrap_or(t);
                        if let Some(found) = try_file_with_extensions(&base_url_dir.join(rel)) {
                            return Some(found);
                        }
                    }
                }
                let mut pattern_keys: Vec<(&String, &serde_json::Value)> =
                    paths_map.iter().filter(|(k, _)| k.contains('*')).collect();
                pattern_keys.sort_by_key(|(b, _)| std::cmp::Reverse(b.len()));
                for (pat_key, target_val) in pattern_keys {
                    let Some((prefix, suffix)) = pat_key.split_once('*') else {
                        continue;
                    };
                    if spec.starts_with(prefix)
                        && spec.ends_with(suffix)
                        && spec.len() >= prefix.len() + suffix.len()
                    {
                        let matched = &spec[prefix.len()..spec.len() - suffix.len()];
                        if let Some(targets) = target_val.as_array() {
                            for t in targets.iter().filter_map(|x| x.as_str()) {
                                let expanded = t.replace('*', matched);
                                let rel = expanded.strip_prefix("./").unwrap_or(&expanded);
                                if let Some(found) =
                                    try_file_with_extensions(&base_url_dir.join(rel))
                                {
                                    return Some(found);
                                }
                            }
                        }
                    }
                }
            }
            if has_explicit_base_url {
                if let Some(found) = try_file_with_extensions(&base_url_dir.join(spec)) {
                    return Some(found);
                }
            }
        }
        curr = dir.parent();
    }
    None
}

pub fn resolve_module_specifier(from_dir: &Path, spec: &str) -> Option<PathBuf> {
    if spec.starts_with("./") || spec.starts_with("../") || spec.starts_with('/') {
        return try_file_with_extensions(&from_dir.join(spec));
    }

    // 1. `package.json` Subpath `"imports"` (`#...`)
    if spec.starts_with('#') {
        let mut curr = Some(from_dir);
        while let Some(dir) = curr {
            let pkg_json = dir.join("package.json");
            if let Some(val) = load_cached_package_json(&pkg_json) {
                if let Some(imports) = val.get("imports") {
                    if let Some(found) = resolve_package_imports(dir, imports, spec) {
                        return Some(found);
                    }
                }
            }
            curr = dir.parent();
        }
        return None;
    }

    // 2. `tsconfig.json` / `jsconfig.json` `"paths"` & `"baseUrl"` (`@/*`, `~/*`, etc.)
    if !from_dir
        .components()
        .any(|c| c.as_os_str() == "node_modules")
    {
        if let Some(found) = resolve_tsconfig_paths(from_dir, spec) {
            return Some(found);
        }
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
            if let Some(val) = load_cached_package_json(&pkg_json) {
                if let Some(exports) = val.get("exports") {
                    if let Some(found) = resolve_package_exports(&pkg_dir, exports, &subpath) {
                        return Some(found);
                    }
                }
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
