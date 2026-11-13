//! Multi-Pass Module Graph Bundler (`bundle_entrypoint_with_deps`).

use crate::bundler::{
    compute_source_line_map, short_source_label, strip_jsonc, ResolverCacheGuard,
};
use crate::compiler::{rewrite_cjs_require, source_contains_jsx, strip_typescript_raw_with_ext};
use crate::linker::{
    analyze_module_used_exports_from_summaries, canonical_mod_id, collect_eager_module_indices,
    extract_module_require_summary, has_top_level_await, inline_cold_method_requires_ast,
    prune_and_minify_cjs_module_ast, transform_module_to_cjs, ModuleRequireSummary, UsedExports,
};
use anyhow::{Context, Result};
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::Path;

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
    let is_ts_entry = matches!(entry_ext, "ts" | "tsx" | "mts" | "cts");

    // Fast path for single-file scripts with no imports/exports/requires/classes/JSX/WinterTC bridge
    if !is_jsx_entry && !entrypoint_needs_bundler_ac().is_match(&raw) {
        if !is_ts_entry {
            return Ok(raw);
        }
        let stripped = strip_typescript_raw_with_ext(&raw, true, false);
        if !entrypoint_needs_bundler_ac().is_match(&stripped) {
            return Ok(stripped);
        }
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

        // Security (`CAS-SEC-01`): Validate and canonicalize `.json` modules via `serde_json`
        // so malformed or maliciously crafted `.json` files cannot inject executable JS statements.
        if ext == "json" {
            let parsed_json: serde_json::Value = serde_json::from_str(&raw_src)
                .or_else(|_| {
                    let cleaned = strip_jsonc(&raw_src);
                    serde_json::from_str(&cleaned)
                })
                .with_context(|| format!("Invalid JSON module at {}", path.display()))?;
            let canonical_json = serde_json::to_string(&parsed_json)
                .with_context(|| format!("Failed to serialize JSON module {}", path.display()))?;
            let cjs_body = format!("module.exports = {};", canonical_json);
            visited_order.push((mod_id.clone(), cjs_body));
            return Ok(mod_id);
        }

        let is_ts = matches!(ext, "ts" | "tsx" | "jsx" | "mts" | "cts");
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
        let digest = blake3::hash(payload.as_bytes());
        let key_u64 = u64::from_le_bytes(digest.as_bytes()[0..8].try_into().unwrap());
        format!("{:016x}", key_u64)
    }

    fn compute_bundle_rc_key_parts(modules_code: &str, entry_body: &str) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(modules_code.as_bytes());
        hasher.update(b"\n");
        hasher.update(entry_body.as_bytes());
        let digest = hasher.finalize();
        let key_u64 = u64::from_le_bytes(digest.as_bytes()[0..8].try_into().unwrap());
        format!("{:016x}", key_u64)
    }

    // Single-Module Unwrapping + Persistent Realm Cache (`__slice_rc`):
    if visited_order.len() == 1 {
        let (_, entry_body) = visited_order.pop().unwrap();
        let direct_body = rewrite_cjs_require(&entry_body);
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
        } else if (direct_body.contains("Slice.serve(")
            || direct_body.contains("Deno.serve(")
            || direct_body.contains("Bun.serve("))
            && !direct_body.contains("Slice.respond(")
        {
            bundle.push_str(&format!(
                "var __slice_modules = {{}};//__SLICE_RC:{}\nvar __rc = globalThis.__slice_rc || (globalThis.__slice_rc = Object.create(null));\nvar __run = __rc[\"{}\"];\nif (!__run) {{\nvar __h = null;\n__run = __rc[\"{}\"] = {} {{\nif (__h && globalThis.__slice_active_http_server && typeof __slice_http_is_active === 'function' && __slice_http_is_active()) {{ globalThis.__slice_dispatch_fetch(__h); return; }}\nvar exports = {{}}, module = {{ exports: exports }};\n{}\nif (globalThis.__slice_active_http_server) __h = globalThis.__slice_active_http_server._handler;\n}};\n}}\n__run();\n",
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

    // Pre-extract `ModuleRequireSummary` once per module (`CAS-PERF-02`) to eliminate $O(M^2)$ AST reparsing
    let mut require_summaries: Vec<ModuleRequireSummary> = visited_order
        .iter()
        .map(|(_, body)| extract_module_require_summary(body))
        .collect();

    // Pass 1: Cold Method-Scoped `require` Inlining for single-consumer `node_modules` dependencies
    if visited_order.len() > 2 {
        let mut consumer_counts: FxHashMap<String, usize> = FxHashMap::default();
        for summary in &require_summaries {
            for (target_id, usage) in &summary.targets {
                if usage.occurrences > 0 {
                    *consumer_counts.entry(target_id.clone()).or_insert(0) += 1;
                }
            }
        }
        let mut single_consumer_deps: FxHashSet<String> = FxHashSet::default();
        for (cand_id, _) in &visited_order[..visited_order.len() - 1] {
            if cand_id.contains("/node_modules/") && consumer_counts.get(cand_id) == Some(&1) {
                single_consumer_deps.insert(cand_id.clone());
            }
        }
        if !single_consumer_deps.is_empty() {
            for i in 0..visited_order.len() - 1 {
                if visited_order[i].0.contains("/node_modules/") {
                    let updated =
                        inline_cold_method_requires_ast(&visited_order[i].1, &single_consumer_deps);
                    if updated != visited_order[i].1 {
                        visited_order[i].1 = updated;
                        require_summaries[i] = extract_module_require_summary(&visited_order[i].1);
                    }
                }
            }
        }
    }

    // Pass 2: Reverse-topological Used-Exports Analysis & Oxc AST Dead-Declaration Elimination
    let mut subset_pruned = vec![false; visited_order.len().saturating_sub(1)];
    for i in (0..visited_order.len() - 1).rev() {
        if !visited_order[i].0.contains("/node_modules/") {
            continue;
        }
        let target_id = &visited_order[i].0;
        let used = analyze_module_used_exports_from_summaries(target_id, &require_summaries, i);
        match used {
            UsedExports::Unreferenced => {
                visited_order[i].1.clear();
                require_summaries[i] = ModuleRequireSummary::default();
            }
            UsedExports::Subset(used_set) => {
                subset_pruned[i] = true;
                let pruned =
                    prune_and_minify_cjs_module_ast(&visited_order[i].1, Some(&used_set), false);
                if pruned != visited_order[i].1 {
                    visited_order[i].1 = pruned;
                    require_summaries[i] = extract_module_require_summary(&visited_order[i].1);
                }
            }
            UsedExports::All => {}
        }
    }

    // Dense Integer Module IDs + Persistent Realm Module Cache (`__slice_rc`):
    let (entry_mod_id, mut entry_body) = visited_order.pop().unwrap();
    let req_entry_dq = format!("require({:?})", entry_mod_id);
    let req_entry_sq = format!("require('{}')", entry_mod_id);
    let entry_is_imported = visited_order
        .iter()
        .any(|(_, b)| b.contains(&req_entry_dq) || b.contains(&req_entry_sq))
        || entry_body.contains(&req_entry_dq)
        || entry_body.contains(&req_entry_sq);
    let entry_mod_idx = visited_order.len();

    if !visited_order.is_empty() || entry_is_imported {
        let cap = (visited_order.len() + usize::from(entry_is_imported)) * 2;
        let mut patterns: Vec<String> = Vec::with_capacity(cap);
        let mut replacements: Vec<String> = Vec::with_capacity(cap);
        for (idx, (mod_id, _)) in visited_order.iter().enumerate() {
            let mut idx_buf = itoa::Buffer::new();
            let to_pat = format!("require({})", idx_buf.format(idx));
            patterns.push(format!("require({:?})", mod_id));
            replacements.push(to_pat.clone());
            patterns.push(format!("require('{}')", mod_id));
            replacements.push(to_pat);
        }
        if entry_is_imported {
            let mut idx_buf = itoa::Buffer::new();
            let to_pat = format!("require({})", idx_buf.format(entry_mod_idx));
            patterns.push(format!("require({:?})", entry_mod_id));
            replacements.push(to_pat.clone());
            patterns.push(format!("require('{}')", entry_mod_id));
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

    let eager_indices = collect_eager_module_indices(&entry_body, &visited_order, &subset_pruned);

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
    if entry_is_imported {
        modules_code.push_str(&format!(
            "__slice_modules[{}] = function(exports, require, module) {{\n{}\n}};\n",
            entry_mod_idx, entry_body
        ));
    }

    let rc_key = compute_bundle_rc_key_parts(&modules_code, &entry_body);
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
    let fn_kw = if !entry_is_imported && has_top_level_await(&entry_body) {
        "async function()"
    } else {
        "function()"
    };
    if entry_is_imported {
        if entry_body.contains("exports.default = __slice_default;") {
            bundle.push_str("var __h = null;\n");
            bundle.push_str(&format!("__run = __rc[\"{}\"] = {} {{\n", rc_key, fn_kw));
            bundle.push_str(&format!(
                "if (!__h) {{ var __ee = __slice_require({}); __h = (__ee && __ee.default !== undefined) ? __ee.default : __ee; }}\nif (__h) globalThis.__slice_dispatch_fetch(__h);\n}};\n}}\n__run();\n",
                entry_mod_idx
            ));
        } else if (entry_body.contains("Slice.serve(")
            || entry_body.contains("Deno.serve(")
            || entry_body.contains("Bun.serve("))
            && !entry_body.contains("Slice.respond(")
        {
            bundle.push_str("var __h = null;\n");
            bundle.push_str(&format!("__run = __rc[\"{}\"] = {} {{\n", rc_key, fn_kw));
            bundle.push_str("if (__h && globalThis.__slice_active_http_server && typeof __slice_http_is_active === 'function' && __slice_http_is_active()) { globalThis.__slice_dispatch_fetch(__h); return; }\n");
            bundle.push_str(&format!(
                "__slice_cache[{}] = undefined;\n__slice_require({});\nif (globalThis.__slice_active_http_server) __h = globalThis.__slice_active_http_server._handler;\n}};\n}}\n__run();\n",
                entry_mod_idx, entry_mod_idx
            ));
        } else {
            bundle.push_str(&format!("__run = __rc[\"{}\"] = {} {{\n", rc_key, fn_kw));
            bundle.push_str(&format!(
                "__slice_cache[{}] = undefined;\n__slice_require({});\n}};\n}}\n__run();\n",
                entry_mod_idx, entry_mod_idx
            ));
        }
    } else if entry_body.contains("exports.default = __slice_default;") {
        bundle.push_str("var __h = null;\n");
        bundle.push_str(&format!("__run = __rc[\"{}\"] = {} {{\n", rc_key, fn_kw));
        bundle.push_str("if (!__h) {\nvar require = __slice_require, exports = {}, module = { exports: exports };\n");
        bundle.push_str(&entry_body);
        bundle.push_str("\n__h = exports.default || module.exports;\n}\nif (__h) globalThis.__slice_dispatch_fetch(__h);\n};\n}\n__run();\n");
    } else if (entry_body.contains("Slice.serve(")
        || entry_body.contains("Deno.serve(")
        || entry_body.contains("Bun.serve("))
        && !entry_body.contains("Slice.respond(")
    {
        bundle.push_str("var __h = null;\n");
        bundle.push_str(&format!("__run = __rc[\"{}\"] = {} {{\n", rc_key, fn_kw));
        bundle.push_str("if (__h && globalThis.__slice_active_http_server && typeof __slice_http_is_active === 'function' && __slice_http_is_active()) { globalThis.__slice_dispatch_fetch(__h); return; }\n");
        bundle.push_str(
            "var require = __slice_require, exports = {}, module = { exports: exports };\n",
        );
        bundle.push_str(&entry_body);
        bundle.push_str("\nif (globalThis.__slice_active_http_server) __h = globalThis.__slice_active_http_server._handler;\n};\n}\n__run();\n");
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
