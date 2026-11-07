//! Module Graph Bundler, `package.json` Conditional `"exports"` Resolver,
//! and Bundled Source-Map Error Location Translator.

use crate::esm_cjs_linker::{
    analyze_module_used_exports, canonical_mod_id, clear_canonical_mod_id_cache,
    collect_eager_module_indices, has_top_level_await, inline_cold_method_requires_ast,
    prune_and_minify_cjs_module_ast, transform_module_to_cjs, UsedExports,
};
use crate::oxc_compiler::{source_contains_jsx, strip_typescript_raw_with_ext};
use anyhow::{bail, Context, Result};
use rustc_hash::{FxHashMap, FxHashSet};
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

thread_local! {
    static PACKAGE_JSON_CACHE: std::cell::RefCell<FxHashMap<PathBuf, Option<Arc<serde_json::Value>>>> =
        std::cell::RefCell::new(FxHashMap::default());
    static TSCONFIG_VALUE_CACHE: std::cell::RefCell<FxHashMap<PathBuf, Option<Arc<serde_json::Value>>>> =
        std::cell::RefCell::new(FxHashMap::default());
}

struct ResolverCacheGuard;

impl ResolverCacheGuard {
    fn new() -> Self {
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

pub(crate) fn try_file_with_extensions(base: &Path) -> Option<PathBuf> {
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
                if let Some(found) = try_file_with_extensions(&base.join(mod_str)) {
                    return Some(found);
                }
            }
            if let Some(main_str) = val.get("main").and_then(|v| v.as_str()) {
                if let Some(found) = try_file_with_extensions(&base.join(main_str)) {
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
            let expanded = if let Some(sub) = wildcard_sub {
                s.replace('*', sub)
            } else {
                s.clone()
            };
            let rel = expanded.strip_prefix("./").unwrap_or(&expanded);
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

fn strip_jsonc(raw: &str) -> Cow<'_, str> {
    let bytes = raw.as_bytes();
    let has_comments =
        memchr::memmem::find(bytes, b"//").is_some() || memchr::memmem::find(bytes, b"/*").is_some();
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

fn load_tsconfig_value(config_path: &Path) -> Option<Arc<serde_json::Value>> {
    TSCONFIG_VALUE_CACHE.with(|cache| {
        if let Some(cached) = cache.borrow().get(config_path) {
            return cached.clone();
        }
        let parsed = load_tsconfig_value_uncached(config_path).map(Arc::new);
        cache
            .borrow_mut()
            .insert(config_path.to_path_buf(), parsed.clone());
        parsed
    })
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

pub(crate) fn resolve_module_specifier(from_dir: &Path, spec: &str) -> Option<PathBuf> {
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
        let nm_candidate = dir.join("node_modules").join(spec);
        if let Some(found) = try_file_with_extensions(&nm_candidate) {
            return Some(found);
        }
        curr = dir.parent();
    }
    None
}

#[inline]
fn push_usize_csv(out: &mut String, n: usize) {
    if !out.is_empty() {
        out.push(',');
    }
    let mut buf = itoa::Buffer::new();
    out.push_str(buf.format(n));
}

fn compute_source_line_map(raw_src: &str, cjs_body: &str) -> String {
    let orig_lines: Vec<&str> = raw_src.lines().collect();
    if orig_lines.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity((cjs_body.len() / 8).max(32));
    let mut search_cursor = 0usize;

    for raw_cjs_line in cjs_body.lines() {
        let trimmed = raw_cjs_line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        let probe_raw = if let Some(rest) = trimmed.strip_prefix("const __slice_default = ") {
            rest.trim_end_matches(';').trim()
        } else {
            trimmed.trim_end_matches(';').trim()
        };

        if probe_raw.starts_with("exports.")
            || probe_raw.starts_with("module.exports = __slice_default")
            || probe_raw.starts_with("var __filename =")
        {
            let fallback_line = (search_cursor + 1).min(orig_lines.len()).max(1);
            push_usize_csv(&mut out, fallback_line);
            continue;
        }

        let rhs_probe = probe_raw
            .split_once(" = ")
            .map(|(_, r)| r.trim())
            .filter(|r| r.len() >= 4);

        let mut found_idx: Option<usize> = None;
        let forward_limit = (search_cursor + 128).min(orig_lines.len());
        for (offset, orig_line) in orig_lines[search_cursor..forward_limit].iter().enumerate() {
            if orig_line.contains(probe_raw)
                || rhs_probe.is_some_and(|rp| !rp.starts_with("require(") && orig_line.contains(rp))
            {
                found_idx = Some(search_cursor + offset);
                break;
            }
        }
        if found_idx.is_none() {
            for (idx, orig_line) in orig_lines.iter().enumerate() {
                if orig_line.contains(probe_raw)
                    || rhs_probe
                        .is_some_and(|rp| !rp.starts_with("require(") && orig_line.contains(rp))
                {
                    found_idx = Some(idx);
                    break;
                }
            }
        }
        if let Some(idx) = found_idx {
            search_cursor = idx;
            push_usize_csv(&mut out, idx + 1);
        } else {
            let next_guess = (search_cursor + 1).min(orig_lines.len()).max(1);
            push_usize_csv(&mut out, next_guess);
        }
    }
    out
}

#[inline]
fn entrypoint_needs_bundler_ac() -> &'static aho_corasick::AhoCorasick {
    static AC: std::sync::OnceLock<aho_corasick::AhoCorasick> = std::sync::OnceLock::new();
    AC.get_or_init(|| {
        aho_corasick::AhoCorasick::new([
            "import ",
            "import(",
            "import.meta",
            "__dirname",
            "__filename",
            "throw ",
            "require(",
            "export ",
            "class ",
            "Slice.",
            "Buffer.",
            "new URL(",
            "new URLPattern(",
            "new URLSearchParams(",
            "new CompressionStream(",
            "new DecompressionStream(",
            "new Headers(",
            "new Request(",
            "new Response(",
            "Response.json(",
            "Response.redirect(",
            "Math.random(",
            "Date.now(",
            "performance.now(",
        ])
        .expect("Valid entrypoint_needs_bundler_ac")
    })
}

/// Bundles a TypeScript/JavaScript/JSX/TSX entrypoint and its entire transitive dependency graph
/// (including relative `.ts`/`.tsx`/`.js`/`.jsx` files AND `node_modules` CommonJS/ESM packages)
/// into a single zero-stat self-contained ECMAScript payload.
pub fn bundle_entrypoint_with_deps(entry_path: &Path) -> Result<String> {
    let raw = std::fs::read_to_string(entry_path)
        .with_context(|| format!("Failed to read entrypoint {}", entry_path.display()))?;
    let entry_ext = entry_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let is_jsx_entry = matches!(entry_ext, "tsx" | "jsx") || source_contains_jsx(&raw);

    // Fast path for single-file scripts with no imports/exports/requires/classes/JSX/WinterTC bridge
    if !is_jsx_entry && !entrypoint_needs_bundler_ac().is_match(&raw) {
        return Ok(raw);
    }

    let _cache_guard = ResolverCacheGuard::new();
    let mut visited_order: Vec<(String, String)> = Vec::new();
    let mut seen: FxHashSet<String> = FxHashSet::default();

    fn visit_module(
        path: &Path,
        seen: &mut FxHashSet<String>,
        visited_order: &mut Vec<(String, String)>,
    ) -> Result<String> {
        let mod_id = canonical_mod_id(path);
        if seen.contains(&mod_id) {
            return Ok(mod_id);
        }
        seen.insert(mod_id.clone());

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext == "wasm" {
            let wasm_bytes = std::fs::read(path)
                .with_context(|| format!("Failed to read .wasm module {}", path.display()))?;
            let b64 = encode_wasm_base64(&wasm_bytes);
            let cjs_body = format!(
                "globalThis.__slice_ensure_shard(5);\nvar __wasm_u8 = globalThis.__slice_base64_decode_bytes(\"{}\");\nmodule.exports = __wasm_u8;\nmodule.exports.default = __wasm_u8;",
                b64
            );
            visited_order.push((mod_id.clone(), cjs_body));
            return Ok(mod_id);
        }

        let raw_src = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read module {}", path.display()))?;

        if ext == "json" {
            let cjs_body = format!("module.exports = {};", raw_src.trim());
            visited_order.push((mod_id.clone(), cjs_body));
            return Ok(mod_id);
        }

        let is_ts = matches!(ext, "ts" | "tsx" | "jsx");
        let is_jsx_ext = matches!(ext, "tsx" | "jsx");
        let js_stripped = strip_typescript_raw_with_ext(&raw_src, is_ts, is_jsx_ext);
        let from_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let mut child_deps = Vec::new();
        let mut cjs_body =
            transform_module_to_cjs(&js_stripped, from_dir, Some(path), &mut child_deps);
        if cjs_body.contains("__dirname") || cjs_body.contains("__filename") {
            let dir_id = canonical_mod_id(from_dir);
            cjs_body = format!(
                "var __filename = {:?}, __dirname = {:?};\n{}",
                mod_id, dir_id, cjs_body
            );
        }
        if !mod_id.contains("/node_modules/") {
            let short_label = short_source_label(path);
            let line_map = compute_source_line_map(&raw_src, &cjs_body);
            cjs_body = format!(
                "// #slice-source: {}|{}|{}\n{}",
                short_label, mod_id, line_map, cjs_body
            );
        }

        for dep_path in child_deps {
            let _ = visit_module(&dep_path, seen, visited_order)?;
        }

        visited_order.push((mod_id.clone(), cjs_body));
        Ok(mod_id)
    }

    let _entry_id = visit_module(entry_path, &mut seen, &mut visited_order)?;

    fn compute_bundle_rc_key(payload: &str) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = rustc_hash::FxHasher::default();
        payload.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    // Single-Module Unwrapping + Persistent Realm Cache (`__slice_rc`):
    if visited_order.len() == 1 {
        let (_, entry_body) = visited_order.pop().unwrap();
        let direct_body = crate::oxc_compiler::rewrite_cjs_require(&entry_body);
        let rc_key = compute_bundle_rc_key(&direct_body);
        let fn_kw = if has_top_level_await(&direct_body) {
            "async function()"
        } else {
            "function()"
        };
        let mut bundle = String::with_capacity(direct_body.len() + 384);
        if direct_body.contains("exports.default = __slice_default;") {
            bundle.push_str(&format!(
                "var __slice_modules = {{}};//__SLICE_RC:{}\nvar __rc = globalThis.__slice_rc || (globalThis.__slice_rc = Object.create(null));\nvar __run = __rc[\"{}\"];\nif (!__run) {{\nvar __h = null;\n__run = __rc[\"{}\"] = {} {{\nif (!__h) {{\nvar exports = {{}}, module = {{ exports: exports }};\n{}\n__h = exports.default || module.exports;\n}}\nif (__h) globalThis.__slice_dispatch_fetch(__h);\n}};\n}}\n__run();\n",
                rc_key, rc_key, rc_key, fn_kw, direct_body
            ));
        } else {
            bundle.push_str(&format!(
                "var __slice_modules = {{}};//__SLICE_RC:{}\nvar __rc = globalThis.__slice_rc || (globalThis.__slice_rc = Object.create(null));\nvar __run = __rc[\"{}\"] || (__rc[\"{}\"] = {} {{\nvar exports = {{}}, module = {{ exports: exports }};\n{}\n}});\n__run();\n",
                rc_key, rc_key, rc_key, fn_kw, direct_body
            ));
        }
        return Ok(bundle);
    }

    // Pass 1: Cold Method-Scoped `require` Inlining for single-consumer `node_modules` dependencies
    if visited_order.len() > 2 {
        let mut single_consumer_deps: FxHashSet<String> = FxHashSet::default();
        for i in 0..visited_order.len() - 1 {
            let cand_id = &visited_order[i].0;
            if !cand_id.contains("/node_modules/") {
                continue;
            }
            let consumer_count = visited_order
                .iter()
                .enumerate()
                .filter(|&(j, (_, body))| j != i && body.contains(cand_id.as_str()))
                .count();
            if consumer_count == 1 {
                single_consumer_deps.insert(cand_id.clone());
            }
        }
        if !single_consumer_deps.is_empty() {
            for i in 0..visited_order.len() - 1 {
                if visited_order[i].0.contains("/node_modules/") {
                    visited_order[i].1 =
                        inline_cold_method_requires_ast(&visited_order[i].1, &single_consumer_deps);
                }
            }
        }
    }

    // Pass 2: Reverse-topological Used-Exports Analysis & Oxc AST Dead-Declaration Elimination
    for i in (0..visited_order.len() - 1).rev() {
        if !visited_order[i].0.contains("/node_modules/") {
            continue;
        }
        let target_id = visited_order[i].0.clone();
        let used = {
            let other_refs: Vec<&str> = visited_order
                .iter()
                .enumerate()
                .filter_map(|(j, (_, body))| if j != i { Some(body.as_str()) } else { None })
                .collect();
            analyze_module_used_exports(&target_id, &other_refs)
        };
        match used {
            UsedExports::Unreferenced => {
                visited_order[i].1.clear();
            }
            UsedExports::Subset(used_set) => {
                visited_order[i].1 =
                    prune_and_minify_cjs_module_ast(&visited_order[i].1, Some(&used_set), false);
            }
            UsedExports::All => {}
        }
    }

    // Dense Integer Module IDs + Persistent Realm Module Cache (`__slice_rc`):
    let (_entry_mod_id, mut entry_body) = visited_order.pop().unwrap();
    if !visited_order.is_empty() {
        let mut patterns: Vec<String> = Vec::with_capacity(visited_order.len() * 2);
        let mut replacements: Vec<String> = Vec::with_capacity(visited_order.len() * 2);
        for (idx, (mod_id, _)) in visited_order.iter().enumerate() {
            let mut idx_buf = itoa::Buffer::new();
            let to_pat = format!("require({})", idx_buf.format(idx));
            patterns.push(format!("require({:?})", mod_id));
            replacements.push(to_pat.clone());
            patterns.push(format!("require('{}')", mod_id));
            replacements.push(to_pat);
        }
        if let Ok(ac) = aho_corasick::AhoCorasick::new(&patterns) {
            for (_, body) in &mut visited_order {
                if !body.is_empty() && body.contains("require(") {
                    *body = ac.replace_all(body, &replacements);
                }
            }
            if entry_body.contains("require(") {
                entry_body = ac.replace_all(&entry_body, &replacements);
            }
        }
    }

    let eager_indices = collect_eager_module_indices(&entry_body, &visited_order);

    let mut modules_code = String::with_capacity(raw.len() + 1024);
    for (idx, (id, body)) in visited_order.iter().enumerate() {
        if body.trim().is_empty() {
            continue;
        }
        let final_body = if id.contains("/node_modules/") {
            prune_and_minify_cjs_module_ast(body, None, true)
        } else {
            body.clone()
        };
        if !eager_indices.contains(&idx) && id.contains("/node_modules/") {
            if let Ok(escaped) = serde_json::to_string(&final_body) {
                modules_code.push_str(&format!("__slice_modules[{}] = {};\n", idx, escaped));
                continue;
            }
        }
        modules_code.push_str(&format!(
            "__slice_modules[{}] = function(exports, require, module) {{\n{}\n}};\n",
            idx, final_body
        ));
    }

    let rc_key = compute_bundle_rc_key(&format!("{}\n{}", modules_code, entry_body));
    let mut bundle = String::with_capacity(modules_code.len() + entry_body.len() + 512);
    bundle.push_str(&format!(
        "var __slice_modules = {{}};//__SLICE_RC:{}\n",
        rc_key
    ));
    bundle.push_str(
        "var __rc = globalThis.__slice_rc || (globalThis.__slice_rc = Object.create(null));\n",
    );
    bundle.push_str(&format!("var __run = __rc[\"{}\"];\n", rc_key));
    bundle.push_str("if (!__run) {\n");
    bundle.push_str("__slice_modules = [];\n");
    bundle.push_str("var __slice_cache = [];\n");
    bundle.push_str("function __slice_require(id) {\n");
    bundle.push_str("    if (typeof id === 'string') return globalThis.__slice_cjs_require(id);\n");
    bundle.push_str("    var m = __slice_cache[id];\n");
    bundle.push_str("    if (m) return m.exports;\n");
    bundle.push_str("    m = __slice_cache[id] = { exports: {} };\n");
    bundle.push_str("    var fn = __slice_modules[id];\n");
    bundle.push_str("    if (typeof fn === 'string') { if (globalThis.__slice_init_lazy_getters) globalThis.__slice_init_lazy_getters(); fn = __slice_modules[id] = new Function('exports', 'require', 'module', fn); }\n");
    bundle.push_str("    fn(m.exports, __slice_require, m);\n");
    bundle.push_str("    return m.exports;\n");
    bundle.push_str("}\n");
    bundle.push_str(&modules_code);
    let fn_kw = if has_top_level_await(&entry_body) {
        "async function()"
    } else {
        "function()"
    };
    if entry_body.contains("exports.default = __slice_default;") {
        bundle.push_str("var __h = null;\n");
        bundle.push_str(&format!("__run = __rc[\"{}\"] = {} {{\n", rc_key, fn_kw));
        bundle.push_str("if (!__h) {\nvar require = __slice_require, exports = {}, module = { exports: exports };\n");
        bundle.push_str(&entry_body);
        bundle.push_str("\n__h = exports.default || module.exports;\n}\nif (__h) globalThis.__slice_dispatch_fetch(__h);\n};\n}\n__run();\n");
    } else {
        bundle.push_str(&format!("__run = __rc[\"{}\"] = {} {{\n", rc_key, fn_kw));
        bundle.push_str(
            "var require = __slice_require, exports = {}, module = { exports: exports };\n",
        );
        bundle.push_str(&entry_body);
        bundle.push_str("\n};\n}\n__run();\n");
    }
    Ok(bundle)
}

fn short_source_label(path: &Path) -> String {
    let fname = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("module.ts");
    if let Some(parent_name) = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
    {
        if matches!(parent_name, "src" | "tests" | "test" | "lib" | "app") {
            return format!("{}/{}", parent_name, fname);
        }
    }
    fname.to_string()
}

fn parse_slice_source_marker(rest: &str) -> (&str, &str, &str) {
    let mut parts = rest.splitn(3, '|');
    let short = parts.next().unwrap_or(rest).trim();
    let full = parts.next().unwrap_or(short).trim();
    let line_map = parts.next().unwrap_or("").trim();
    (short, full, line_map)
}

fn lookup_embedded_line_map(line_map: &str, rel_idx_0: usize) -> Option<usize> {
    if line_map.is_empty() {
        return None;
    }
    line_map
        .split(',')
        .nth(rel_idx_0)
        .and_then(|s| s.trim().parse::<usize>().ok())
}

/// Maps an uncaught JS error or bundled line reference back to its original TypeScript/JavaScript
/// source file and 1-based line number (`<short_file>:<line>`) using `// #slice-source:` markers.
pub fn map_bundled_error_location(bundled_js: &str, error_msg: &str) -> String {
    let lines: Vec<&str> = bundled_js.lines().collect();

    let mut explicit_line: Option<usize> = None;
    if let Some(idx) = error_msg.find("line ") {
        let digits: String = error_msg[idx + 5..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(n) = digits.parse::<usize>() {
            explicit_line = Some(n);
        }
    }

    if let Some(b_line) = explicit_line {
        let mut active_short = "";
        let mut active_full = "";
        let mut active_line_map = "";
        let mut marker_line_idx = 0usize;
        for (idx, line) in lines.iter().enumerate() {
            let line_1 = idx + 1;
            if line_1 > b_line {
                break;
            }
            if let Some(rest) = line.trim().strip_prefix("// #slice-source: ") {
                let (s, f, lm) = parse_slice_source_marker(rest);
                active_short = s;
                active_full = f;
                active_line_map = lm;
                marker_line_idx = line_1;
            }
        }
        if !active_short.is_empty() && b_line > marker_line_idx {
            let rel_idx_0 = (b_line - marker_line_idx).saturating_sub(1);
            let mut resolved_line = lookup_embedded_line_map(active_line_map, rel_idx_0)
                .unwrap_or(b_line - marker_line_idx);
            if let Some(stmt_text) = lines.get(b_line - 1).map(|s| s.trim()) {
                if !stmt_text.is_empty() {
                    if let Ok(orig_src) = std::fs::read_to_string(active_full) {
                        if let Some((orig_idx, _)) = orig_src
                            .lines()
                            .enumerate()
                            .find(|(_, ol)| ol.contains(stmt_text))
                        {
                            resolved_line = orig_idx + 1;
                        }
                    }
                }
            }
            return format!("{} ({}:{})", error_msg, active_short, resolved_line);
        }
    }

    let core_msg = error_msg
        .trim()
        .trim_start_matches("Uncaught ")
        .trim_start_matches("Error: ")
        .trim_start_matches("TypeError: ")
        .trim_start_matches("RangeError: ")
        .trim_matches('"')
        .trim();

    if !core_msg.is_empty() {
        let mut active_short = "";
        let mut active_full = "";
        let mut active_line_map = "";
        let mut marker_line_idx = 0usize;
        for (idx, line) in lines.iter().enumerate() {
            let line_1 = idx + 1;
            if let Some(rest) = line.trim().strip_prefix("// #slice-source: ") {
                let (s, f, lm) = parse_slice_source_marker(rest);
                active_short = s;
                active_full = f;
                active_line_map = lm;
                marker_line_idx = line_1;
                continue;
            }
            if !active_short.is_empty() && line.contains(core_msg) {
                let rel_idx_0 = (line_1 - marker_line_idx).saturating_sub(1);
                let mut resolved_line = lookup_embedded_line_map(active_line_map, rel_idx_0)
                    .unwrap_or(line_1 - marker_line_idx);
                if let Ok(orig_src) = std::fs::read_to_string(active_full) {
                    if let Some((orig_idx, _)) = orig_src
                        .lines()
                        .enumerate()
                        .find(|(_, ol)| ol.contains(core_msg))
                    {
                        resolved_line = orig_idx + 1;
                    }
                }
                return format!("{} ({}:{})", error_msg, active_short, resolved_line);
            }
        }
    }

    error_msg.to_string()
}

#[derive(Debug, Clone)]
struct BundledSourceSection<'a> {
    short_label: &'a str,
    full_path: &'a str,
    line_map: &'a str,
    marker_line_1: usize,
    end_line_1: usize,
}

fn count_code_lines_between(lines: &[&str], start_line_1: usize, target_line_1: usize) -> usize {
    if target_line_1 <= start_line_1 {
        return 0;
    }
    let start_idx = start_line_1;
    let end_idx = (target_line_1 - 1).min(lines.len());
    if start_idx >= end_idx {
        return 0;
    }
    lines[start_idx..end_idx]
        .iter()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with("//")
        })
        .count()
}

fn resolve_section_line(
    lines: &[&str],
    sec: &BundledSourceSection<'_>,
    target_line_1: usize,
) -> usize {
    let rel_idx_0 = count_code_lines_between(lines, sec.marker_line_1, target_line_1);
    let mut resolved_line =
        lookup_embedded_line_map(sec.line_map, rel_idx_0).unwrap_or(rel_idx_0 + 1);
    if let Some(stmt_text) = lines.get(target_line_1.saturating_sub(1)).map(|s| s.trim()) {
        let probe = stmt_text
            .strip_prefix("exports.")
            .and_then(|r| r.split_once(" = ").map(|(_, rhs)| rhs.trim()))
            .unwrap_or(stmt_text)
            .trim_end_matches(';')
            .trim();
        if !probe.is_empty() && probe.len() >= 4 {
            if let Ok(orig_src) = std::fs::read_to_string(sec.full_path) {
                if let Some((orig_idx, _)) = orig_src
                    .lines()
                    .enumerate()
                    .find(|(_, ol)| ol.contains(probe))
                {
                    resolved_line = orig_idx + 1;
                }
            }
        }
    }
    resolved_line
}

fn parse_raw_stack_frame(trimmed: &str) -> Option<(Option<&str>, Option<(usize, usize)>)> {
    let after_at = trimmed.strip_prefix("at ")?.trim();
    let (fn_name_opt, loc_str) = if let Some((fn_part, rest)) = after_at.split_once(" (") {
        let fn_clean = fn_part.trim();
        let fn_opt = if fn_clean.is_empty()
            || fn_clean == "unknown"
            || fn_clean == "__run"
            || fn_clean == "<anonymous>"
            || fn_clean.starts_with("__slice")
        {
            None
        } else {
            Some(fn_clean)
        };
        (fn_opt, rest.trim_end_matches(')').trim())
    } else {
        (None, after_at)
    };

    let mut line_col = None;
    let mut rsplit = loc_str.rsplitn(3, ':');
    if let (Some(col_s), Some(line_s)) = (rsplit.next(), rsplit.next()) {
        if let (Ok(l), Ok(c)) = (line_s.trim().parse::<usize>(), col_s.trim().parse::<usize>()) {
            line_col = Some((l, c));
        }
    }
    Some((fn_name_opt, line_col))
}

/// Translates a multi-frame ECMAScript `Error.prototype.stack` string against embedded
/// `// #slice-source: <short_label>|<full_path>|<line_map>` (or compacted `//@sm:`) markers inside `bundled_js`,
/// mapping each stack frame back to its original TypeScript/JavaScript `<short_label>:<line>:<col>`.
pub fn map_bundled_stack_trace(bundled_js: &str, raw_stack: &str) -> String {
    if (!bundled_js.contains("// #slice-source: ") && !bundled_js.contains("//@sm:"))
        || raw_stack.trim().is_empty()
    {
        return raw_stack.to_string();
    }

    let lines: Vec<&str> = bundled_js.lines().collect();
    let mut sections: Vec<BundledSourceSection<'_>> = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let line_1 = idx + 1;
        let trimmed = line.trim();
        if let Some(rest) = trimmed
            .strip_prefix("// #slice-source: ")
            .or_else(|| trimmed.strip_prefix("//@sm:"))
        {
            if let Some(prev) = sections.last_mut() {
                prev.end_line_1 = line_1.saturating_sub(1);
            }
            let (short_label, full_path, line_map) = parse_slice_source_marker(rest);
            sections.push(BundledSourceSection {
                short_label,
                full_path,
                line_map,
                marker_line_1: line_1,
                end_line_1: lines.len(),
            });
        }
    }

    if sections.is_empty() {
        return raw_stack.to_string();
    }

    let raw_lines: Vec<&str> = raw_stack.lines().collect();
    let parsed_frames: Vec<Option<(Option<&str>, Option<(usize, usize)>)>> = raw_lines
        .iter()
        .map(|l| parse_raw_stack_frame(l.trim()))
        .collect();

    let mut out = String::with_capacity(raw_stack.len() + 128);
    let mut frame_ordinal = 0usize;
    let mut prev_callee_fn: Option<&str> = None;

    for (line_idx, &raw_line) in raw_lines.iter().enumerate() {
        let Some((fn_name_opt, line_col_opt)) = parsed_frames[line_idx] else {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(raw_line);
            continue;
        };

        let col = line_col_opt.map(|(_, c)| c).unwrap_or(1);
        let mut resolved: Option<(&str, usize)> = None;

        if let Some(fn_name) = fn_name_opt {
            let fn_pat_1 = format!("function {}(", fn_name);
            let fn_pat_2 = format!("function {} (", fn_name);
            let fn_pat_3 = format!("{} = function", fn_name);

            for sec in &sections {
                let mut decl_line_1 = None;
                for l_1 in (sec.marker_line_1 + 1)..=sec.end_line_1.min(lines.len()) {
                    let text = lines[l_1 - 1];
                    if text.contains(&fn_pat_1)
                        || text.contains(&fn_pat_2)
                        || text.contains(&fn_pat_3)
                    {
                        decl_line_1 = Some(l_1);
                        break;
                    }
                }
                if let Some(d_line_1) = decl_line_1 {
                    let mut fn_end_1 = sec.end_line_1.min(lines.len());
                    let mut brace_depth = 0i32;
                    let mut seen_open = false;
                    for l_1 in d_line_1..=sec.end_line_1.min(lines.len()) {
                        for ch in lines[l_1 - 1].chars() {
                            if ch == '{' {
                                brace_depth += 1;
                                seen_open = true;
                            } else if ch == '}' {
                                brace_depth -= 1;
                            }
                        }
                        if seen_open && brace_depth <= 0 {
                            fn_end_1 = l_1;
                            break;
                        }
                    }

                    let mut target_line_1 = d_line_1;
                    if frame_ordinal == 0 {
                        for l_1 in d_line_1..=fn_end_1 {
                            let t = lines[l_1 - 1];
                            if t.contains("throw ") || t.contains("Error(") {
                                target_line_1 = l_1;
                                break;
                            }
                        }
                    } else if let Some(callee) = prev_callee_fn {
                        let call_pat = format!("{}(", callee);
                        for l_1 in d_line_1..=fn_end_1 {
                            if lines[l_1 - 1].contains(&call_pat) {
                                target_line_1 = l_1;
                                break;
                            }
                        }
                    }
                    if target_line_1 == d_line_1 {
                        if let Some((r_line, _)) = line_col_opt {
                            if r_line >= d_line_1 && r_line <= fn_end_1 {
                                target_line_1 = r_line;
                            }
                        }
                    }

                    let orig_line = resolve_section_line(&lines, sec, target_line_1);
                    resolved = Some((sec.short_label, orig_line));
                    break;
                }
            }
        } else {
            if let Some(callee) = prev_callee_fn {
                let call_pat = format!("{}(", callee);
                for sec in sections.iter().rev() {
                    for l_1 in (sec.marker_line_1 + 1)..=sec.end_line_1.min(lines.len()) {
                        let t = lines[l_1 - 1].trim();
                        if t.contains(&call_pat) && !t.starts_with("function ") {
                            let orig_line = resolve_section_line(&lines, sec, l_1);
                            resolved = Some((sec.short_label, orig_line));
                            break;
                        }
                    }
                    if resolved.is_some() {
                        break;
                    }
                }
            }
            if resolved.is_none() {
                if let Some((r_line, _)) = line_col_opt {
                    if let Some(sec) = sections
                        .iter()
                        .rev()
                        .find(|s| r_line > s.marker_line_1 && r_line <= s.end_line_1)
                    {
                        let orig_line = resolve_section_line(&lines, sec, r_line);
                        resolved = Some((sec.short_label, orig_line));
                    }
                }
            }
        }

        if !out.is_empty() {
            out.push('\n');
        }
        if let Some((short_label, orig_line)) = resolved {
            if let Some(fn_name) = fn_name_opt {
                out.push_str(&format!("    at {} ({}:{}:{})", fn_name, short_label, orig_line, col));
            } else {
                out.push_str(&format!("    at {}:{}:{}", short_label, orig_line, col));
            }
        } else {
            out.push_str(raw_line);
        }

        prev_callee_fn = fn_name_opt;
        frame_ordinal += 1;
    }

    out
}

fn encode_wasm_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((n >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(n & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}
