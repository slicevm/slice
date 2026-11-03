//! Module Graph Bundler, `package.json` Conditional `"exports"` Resolver,
//! and Bundled Source-Map Error Location Translator.

use crate::esm_cjs_linker::{canonical_mod_id, transform_module_to_cjs};
use crate::oxc_compiler::{source_contains_jsx, strip_typescript_raw};
use anyhow::{bail, Context, Result};
use rustc_hash::FxHashSet;
use std::path::{Path, PathBuf};

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
    for ext in ["ts", "tsx", "js", "jsx", "mjs", "cjs", "json"] {
        let with_ext = base.with_extension(ext);
        if with_ext.is_file() {
            return Some(with_ext);
        }
    }
    if base.is_dir() {
        let pkg_json = base.join("package.json");
        if pkg_json.is_file() {
            if let Ok(raw) = std::fs::read_to_string(&pkg_json) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                    if let Some(exports) = val.get("exports") {
                        if let Some(found) = resolve_package_exports(base, exports, ".") {
                            return Some(found);
                        }
                    }
                    if let Some(main_str) = val.get("main").and_then(|v| v.as_str()) {
                        if let Some(found) = try_file_with_extensions(&base.join(main_str)) {
                            return Some(found);
                        }
                    }
                    if let Some(mod_str) = val.get("module").and_then(|v| v.as_str()) {
                        if let Some(found) = try_file_with_extensions(&base.join(mod_str)) {
                            return Some(found);
                        }
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
                "require",
                "import",
                "module",
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

pub(crate) fn resolve_module_specifier(from_dir: &Path, spec: &str) -> Option<PathBuf> {
    if spec.starts_with("./") || spec.starts_with("../") || spec.starts_with('/') {
        return try_file_with_extensions(&from_dir.join(spec));
    }

    let (pkg_name, subpath) = split_bare_package_specifier(spec);

    let mut curr = Some(from_dir);
    while let Some(dir) = curr {
        let pkg_dir = dir.join("node_modules").join(pkg_name);
        if pkg_dir.is_dir() {
            let pkg_json = pkg_dir.join("package.json");
            if pkg_json.is_file() {
                if let Ok(raw) = std::fs::read_to_string(&pkg_json) {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                        if let Some(exports) = val.get("exports") {
                            if let Some(found) =
                                resolve_package_exports(&pkg_dir, exports, &subpath)
                            {
                                return Some(found);
                            }
                        }
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
    if !is_jsx_entry
        && !raw.contains("import ")
        && !raw.contains("require(")
        && !raw.contains("export ")
        && !raw.contains("class ")
        && !raw.contains("Slice.")
        && !raw.contains("Buffer.")
        && !raw.contains("new URL(")
        && !raw.contains("new URLSearchParams(")
        && !raw.contains("new Headers(")
        && !raw.contains("new Request(")
        && !raw.contains("new Response(")
        && !raw.contains("Response.json(")
        && !raw.contains("Response.redirect(")
        && !raw.contains("Math.random(")
        && !raw.contains("Date.now(")
        && !raw.contains("performance.now(")
    {
        return Ok(raw);
    }

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

        let raw_src = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read module {}", path.display()))?;
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        if ext == "json" {
            let cjs_body = format!("module.exports = {};", raw_src.trim());
            visited_order.push((mod_id.clone(), cjs_body));
            return Ok(mod_id);
        }

        let is_ts = matches!(ext, "ts" | "tsx" | "jsx");
        let js_stripped = strip_typescript_raw(&raw_src, is_ts);
        let from_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let mut child_deps = Vec::new();
        let mut cjs_body = transform_module_to_cjs(&js_stripped, from_dir, &mut child_deps);
        if !mod_id.contains("/node_modules/") {
            let short_label = short_source_label(path);
            cjs_body = format!("// #slice-source: {}|{}\n{}", short_label, mod_id, cjs_body);
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
        let direct_body = entry_body.replace("require(", "__slice_cjs_require(");
        let rc_key = compute_bundle_rc_key(&direct_body);
        let mut bundle = String::with_capacity(direct_body.len() + 384);
        if direct_body.contains("exports.default = __slice_default;") {
            bundle.push_str(&format!(
                "var __slice_modules = {{}};//__SLICE_RC:{}\nvar __rc = globalThis.__slice_rc || (globalThis.__slice_rc = Object.create(null));\nvar __run = __rc[\"{}\"];\nif (!__run) {{\nvar __h = null;\n__run = __rc[\"{}\"] = function() {{\nif (!__h) {{\nvar exports = {{}}, module = {{ exports: exports }};\n{}\n__h = exports.default || module.exports;\n}}\nif (__h) globalThis.__slice_dispatch_fetch(__h);\n}};\n}}\n__run();\n",
                rc_key, rc_key, rc_key, direct_body
            ));
        } else {
            bundle.push_str(&format!(
                "var __slice_modules = {{}};//__SLICE_RC:{}\nvar __rc = globalThis.__slice_rc || (globalThis.__slice_rc = Object.create(null));\nvar __run = __rc[\"{}\"] || (__rc[\"{}\"] = function() {{\nvar exports = {{}}, module = {{ exports: exports }};\n{}\n}});\n__run();\n",
                rc_key, rc_key, rc_key, direct_body
            ));
        }
        return Ok(bundle);
    }

    // Dense Integer Module IDs + Persistent Realm Module Cache (`__slice_rc`):
    let (_entry_mod_id, mut entry_body) = visited_order.pop().unwrap();
    let id_replacements: Vec<(String, String)> = visited_order
        .iter()
        .enumerate()
        .map(|(idx, (mod_id, _))| {
            (
                format!("require({:?})", mod_id),
                format!("require({})", idx),
            )
        })
        .collect();

    for (_, body) in &mut visited_order {
        for (from_pat, to_pat) in &id_replacements {
            if body.contains(from_pat) {
                *body = body.replace(from_pat, to_pat);
            }
        }
    }
    for (from_pat, to_pat) in &id_replacements {
        if entry_body.contains(from_pat) {
            entry_body = entry_body.replace(from_pat, to_pat);
        }
    }

    let mut modules_code = String::with_capacity(raw.len() + 1024);
    let lazy_eval_strings = visited_order.len() > 8;
    for (idx, (id, body)) in visited_order.iter().enumerate() {
        if lazy_eval_strings || id.contains("fast-decode-uri-component") {
            let encoded_body = serde_json::to_string(body).unwrap_or_default();
            modules_code.push_str(&format!("__slice_modules[{}] = {};\n", idx, encoded_body));
        } else {
            modules_code.push_str(&format!(
                "__slice_modules[{}] = function(exports, require, module) {{\n{}\n}};\n",
                idx, body
            ));
        }
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
    bundle.push_str("var __slice_cache = [];\n");
    bundle.push_str("function __slice_require(id) {\n");
    bundle.push_str("    if (typeof id === 'string') return globalThis.__slice_cjs_require(id);\n");
    bundle.push_str("    var m = __slice_cache[id];\n");
    bundle.push_str("    if (m) return m.exports;\n");
    bundle.push_str("    m = __slice_cache[id] = { exports: {} };\n");
    bundle.push_str("    var fn = __slice_modules[id];\n");
    bundle.push_str("    if (typeof fn === 'string') fn = __slice_modules[id] = new Function('exports', 'require', 'module', fn);\n");
    bundle.push_str("    fn(m.exports, __slice_require, m);\n");
    bundle.push_str("    return m.exports;\n");
    bundle.push_str("}\n");
    bundle.push_str(&modules_code);
    if entry_body.contains("exports.default = __slice_default;") {
        bundle.push_str("var __h = null;\n");
        bundle.push_str(&format!("__run = __rc[\"{}\"] = function() {{\n", rc_key));
        bundle.push_str("if (!__h) {\nvar require = __slice_require, exports = {}, module = { exports: exports };\n");
        bundle.push_str(&entry_body);
        bundle.push_str("\n__h = exports.default || module.exports;\n}\nif (__h) globalThis.__slice_dispatch_fetch(__h);\n};\n}\n__run();\n");
    } else {
        bundle.push_str(&format!("__run = __rc[\"{}\"] = function() {{\n", rc_key));
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
        let mut marker_line_idx = 0usize;
        for (idx, line) in lines.iter().enumerate() {
            let line_1 = idx + 1;
            if line_1 > b_line {
                break;
            }
            if let Some(rest) = line.trim().strip_prefix("// #slice-source: ") {
                let (s, f) = rest.split_once('|').unwrap_or((rest, rest));
                active_short = s.trim();
                active_full = f.trim();
                marker_line_idx = line_1;
            }
        }
        if !active_short.is_empty() && b_line > marker_line_idx {
            let mut resolved_line = b_line - marker_line_idx;
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
        let mut marker_line_idx = 0usize;
        for (idx, line) in lines.iter().enumerate() {
            let line_1 = idx + 1;
            if let Some(rest) = line.trim().strip_prefix("// #slice-source: ") {
                let (s, f) = rest.split_once('|').unwrap_or((rest, rest));
                active_short = s.trim();
                active_full = f.trim();
                marker_line_idx = line_1;
                continue;
            }
            if !active_short.is_empty() && line.contains(core_msg) {
                let mut resolved_line = line_1 - marker_line_idx;
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

