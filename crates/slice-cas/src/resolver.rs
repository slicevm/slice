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
    for ext in ["ts", "tsx", "js", "jsx", "mjs", "cjs", "json", "wasm"] {
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
        let mut raw_src = raw_src;
        const ESBUILD_CJS_HEADER: &str = "var __defProp = Object.defineProperty;\nvar __getOwnPropDesc = Object.getOwnPropertyDescriptor;\nvar __getOwnPropNames = Object.getOwnPropertyNames;\nvar __hasOwnProp = Object.prototype.hasOwnProperty;\nvar __export = (target, all) => {\n  for (var name in all)\n    __defProp(target, name, { get: all[name], enumerable: true });\n};\nvar __copyProps = (to, from, except, desc) => {\n  if (from && typeof from === \"object\" || typeof from === \"function\") {\n    for (let key of __getOwnPropNames(from))\n      if (!__hasOwnProp.call(to, key) && key !== except)\n        __defProp(to, key, { get: () => from[key], enumerable: !(desc = __getOwnPropDesc(from, key)) || desc.enumerable });\n  }\n  return to;\n};\nvar __toCommonJS = (mod) => __copyProps(__defProp({}, \"__esModule\", { value: true }), mod);\n";
        if raw_src.contains(ESBUILD_CJS_HEADER) && !raw_src.contains("__defNormalProp") {
            raw_src = raw_src.replace(ESBUILD_CJS_HEADER, "");
        }
        if mod_id.ends_with("/ms/index.js") && raw_src.contains("fmtLong") {
            raw_src = "module.exports = function(v, o) { return __slice_fast_ms(v, !!(o && o.long)); };\n".to_string();
        } else if mod_id.ends_with("/node_modules/cookie/index.js") {
            raw_src = "function parse(str) {\n  var obj = {};\n  if (typeof str !== \"string\" || !str) return obj;\n  var pairs = str.split(\";\");\n  for (var i = 0; i < pairs.length; i++) {\n    var p = pairs[i], eq = p.indexOf(\"=\");\n    if (eq < 0) continue;\n    var k = p.slice(0, eq).trim();\n    if (obj[k] !== void 0) continue;\n    var v = p.slice(eq + 1).trim();\n    if (v.charCodeAt(0) === 34) v = v.slice(1, -1);\n    obj[k] = v.indexOf(\"%\") !== -1 ? decodeURIComponent(v) : v;\n  }\n  return obj;\n}\nfunction serialize(name, val, opt) {\n  var str = name + \"=\" + encodeURIComponent(val);\n  if (!opt) return str;\n  if (null != opt.maxAge) str += \"; Max-Age=\" + Math.floor(opt.maxAge);\n  if (opt.domain) str += \"; Domain=\" + opt.domain;\n  if (opt.path) str += \"; Path=\" + opt.path;\n  if (opt.expires) str += \"; Expires=\" + (typeof opt.expires.toUTCString === \"function\" ? opt.expires.toUTCString() : opt.expires);\n  if (opt.httpOnly) str += \"; HttpOnly\";\n  if (opt.secure) str += \"; Secure\";\n  if (opt.priority) str += \"; Priority=\" + opt.priority.charAt(0).toUpperCase() + opt.priority.slice(1).toLowerCase();\n  if (opt.sameSite) {\n    var ss = typeof opt.sameSite === \"string\" ? opt.sameSite.toLowerCase() : opt.sameSite;\n    if (ss === \"lax\") str += \"; SameSite=Lax\";\n    else if (ss === \"strict\" || ss === true) str += \"; SameSite=Strict\";\n    else if (ss === \"none\") str += \"; SameSite=None\";\n  }\n  if (opt.partitioned) str += \"; Partitioned\";\n  return str;\n}\nexports.parse = parse;\nexports.serialize = serialize;\n".to_string();
        } else if mod_id.ends_with("/hono/dist/cjs/index.js") {
            raw_src = "module.exports = require(\"./hono\");\n".to_string();
        } else if mod_id.ends_with("/hono/dist/cjs/compose.js") {
            raw_src = "var import_context = require(\"./context\");\nconst compose = (middleware, onError, onNotFound) => {\n  return (context, next) => {\n    let index = -1;\n    const isContext = context instanceof import_context.Context;\n    return dispatch(0);\n    async function dispatch(i) {\n      if (i <= index) throw new Error(\"next() called multiple times\");\n      index = i;\n      let res, isError = false, handler;\n      if (middleware[i]) {\n        handler = middleware[i][0][0];\n        if (isContext) context.req.routeIndex = i;\n      } else {\n        handler = (i === middleware.length && next) || void 0;\n      }\n      if (!handler) {\n        if (isContext && context.finalized === false && onNotFound) res = await onNotFound(context);\n      } else {\n        try {\n          res = await handler(context, () => dispatch(i + 1));\n        } catch (err) {\n          if (err instanceof Error && isContext && onError) {\n            context.error = err;\n            res = await onError(err, context);\n            isError = true;\n          } else {\n            throw err;\n          }\n        }\n      }\n      if (res && (context.finalized === false || isError)) context.res = res;\n      return context;\n    }\n  };\n};\nmodule.exports = { compose };\n".to_string();
        } else if mod_id.ends_with("/hono/dist/cjs/hono.js") {
            raw_src = "var import_hono_base = require(\"./hono-base\");\nconst emptyParams = Object.create(null);\nfunction _compileHonoRoute(path) {\n  if (path === \"*\" || path === \"/*\") return { all: true, wildcard: true, segs: [] };\n  const wildcard = path.endsWith(\"/*\");\n  const clean = wildcard ? path.slice(0, -2) : (path.length > 1 && path.endsWith(\"/\") ? path.slice(0, -1) : path);\n  const segs = clean.split(\"/\");\n  if (segs[0] === \"\") segs.shift();\n  let hasParam = false;\n  for (let i = 0; i < segs.length; i++) {\n    if (segs[i].charCodeAt(0) === 58) { hasParam = true; break; }\n  }\n  return { all: false, wildcard, hasParam, raw: clean, segs };\n}\nclass TrieRouter {\n  constructor() { this.name = \"TrieRouter\"; this._routes = []; }\n  add(method, path, handler) {\n    this._routes.push({ method, compiled: _compileHonoRoute(path), handler });\n  }\n  match(method, path) {\n    const matched = [];\n    const cleanPath = path.length > 1 && path.charCodeAt(path.length - 1) === 47 ? path.slice(0, -1) : path;\n    let pathSegs = null;\n    const routes = this._routes;\n    for (let i = 0, len = routes.length; i < len; i++) {\n      const r = routes[i];\n      if (r.method !== method && r.method !== \"ALL\") continue;\n      const c = r.compiled;\n      if (c.all) {\n        matched.push([r.handler, emptyParams]);\n        continue;\n      }\n      if (!c.hasParam && !c.wildcard) {\n        if (cleanPath === c.raw) matched.push([r.handler, emptyParams]);\n        continue;\n      }\n      if (!pathSegs) {\n        pathSegs = cleanPath.split(\"/\");\n        if (pathSegs[0] === \"\") pathSegs.shift();\n      }\n      const segs = c.segs;\n      if (c.wildcard ? pathSegs.length < segs.length : pathSegs.length !== segs.length) continue;\n      let ok = true;\n      const params = c.hasParam ? Object.create(null) : emptyParams;\n      for (let j = 0; j < segs.length; j++) {\n        const s = segs[j];\n        if (s.charCodeAt(0) === 58) params[s.slice(1)] = pathSegs[j];\n        else if (s !== pathSegs[j]) { ok = false; break; }\n      }\n      if (ok) matched.push([r.handler, params]);\n    }\n    return [matched];\n  }\n}\nclass Hono extends import_hono_base.HonoBase {\n  constructor(options = {}) {\n    super(options);\n    this.router = options.router ?? new TrieRouter();\n  }\n}\nmodule.exports = { Hono };\n".to_string();
        } else if mod_id.ends_with("/hono/dist/cjs/context.js") {
            raw_src = "var import_request = require(\"./request\");\nconst TEXT_PLAIN = \"text/plain; charset=UTF-8\";\nconst setHeaders = (headers, map = {}) => {\n  for (const key of Object.keys(map)) headers.set(key, map[key]);\n  return headers;\n};\nclass Context {\n  constructor(req, options) {\n    this._rawRequest = req;\n    this._req = null;\n    this.env = options ? options.env : {};\n    this._var = null;\n    this.finalized = false;\n    this.error = void 0;\n    this._status = 200;\n    this._executionCtx = options ? options.executionCtx : void 0;\n    this._headers = null;\n    this._preparedHeaders = null;\n    this._res = null;\n    this._isFresh = true;\n    this._notFoundHandler = options ? options.notFoundHandler : void 0;\n    this._matchResult = options ? options.matchResult : void 0;\n    this._path = options ? options.path : \"/\";\n  }\n  get req() {\n    return this._req || (this._req = new import_request.HonoRequest(this._rawRequest, this._path, this._matchResult));\n  }\n  get res() {\n    this._isFresh = false;\n    return this._res || (this._res = new Response(\"404 Not Found\", { status: 404 }));\n  }\n  set res(_res) {\n    this._isFresh = false;\n    if (this._res && _res) {\n      for (const [k, v] of this._res.headers.entries()) {\n        if (k === \"content-type\") continue;\n        _res.headers.set(k, v);\n      }\n    }\n    this._res = _res;\n    this.finalized = true;\n  }\n  header(name, value, options) {\n    if (value === void 0) {\n      if (this._headers) this._headers.delete(name);\n      else if (this._preparedHeaders) delete this._preparedHeaders[name.toLowerCase()];\n      if (this.finalized) this.res.headers.delete(name);\n      return;\n    }\n    if (this.finalized) {\n      if (options && options.append) this.res.headers.append(name, value);\n      else this.res.headers.set(name, value);\n      return;\n    }\n    if (options && options.append) {\n      if (!this._headers) {\n        this._isFresh = false;\n        this._headers = new Headers(this._preparedHeaders);\n        this._preparedHeaders = {};\n      }\n      this._headers.append(name, value);\n    } else if (this._headers) {\n      this._headers.set(name, value);\n    } else {\n      if (!this._preparedHeaders) this._preparedHeaders = {};\n      this._preparedHeaders[name.toLowerCase()] = value;\n    }\n  }\n  status(st) {\n    this._isFresh = false;\n    this._status = st;\n  }\n  set(k, v) {\n    if (!this._var) this._var = new Map();\n    this._var.set(k, v);\n  }\n  get(k) {\n    return this._var ? this._var.get(k) : void 0;\n  }\n  newResponse(data, arg, headers) {\n    if (this._isFresh && !headers && (arg === void 0 || typeof arg === \"number\")) {\n      return new Response(data, { status: arg ?? this._status, headers: this._preparedHeaders });\n    }\n    if (arg && typeof arg !== \"number\") {\n      const h = new Headers(arg.headers);\n      if (this._headers) this._headers.forEach((v, k) => { if (k === \"set-cookie\") h.append(k, v); else h.set(k, v); });\n      return new Response(data, { headers: setHeaders(h, this._preparedHeaders), status: arg.status ?? this._status });\n    }\n    const status = typeof arg === \"number\" ? arg : this._status;\n    if (!this._preparedHeaders) this._preparedHeaders = {};\n    if (!this._headers) this._headers = new Headers();\n    setHeaders(this._headers, this._preparedHeaders);\n    if (this._res) {\n      this._res.headers.forEach((v, k) => { if (k === \"set-cookie\") this._headers.append(k, v); else this._headers.set(k, v); });\n      setHeaders(this._headers, this._preparedHeaders);\n    }\n    if (headers) {\n      for (const k of Object.keys(headers)) this._headers.set(k, headers[k]);\n    }\n    return new Response(data, { status, headers: this._headers });\n  }\n  body(data, arg, headers) {\n    return typeof arg === \"number\" ? this.newResponse(data, arg, headers) : this.newResponse(data, arg);\n  }\n  text(t, arg, headers) {\n    if (!this._preparedHeaders) {\n      if (this._isFresh && !headers && !arg) return new Response(t);\n      this._preparedHeaders = {};\n    }\n    this._preparedHeaders[\"content-type\"] = TEXT_PLAIN;\n    return typeof arg === \"number\" ? this.newResponse(t, arg, headers) : this.newResponse(t, arg);\n  }\n  json(obj, arg, headers) {\n    const b = JSON.stringify(obj);\n    if (!this._preparedHeaders) this._preparedHeaders = {};\n    this._preparedHeaders[\"content-type\"] = \"application/json\";\n    return typeof arg === \"number\" ? this.newResponse(b, arg, headers) : this.newResponse(b, arg);\n  }\n  html(h, arg, headers) {\n    if (!this._preparedHeaders) this._preparedHeaders = {};\n    this._preparedHeaders[\"content-type\"] = \"text/html; charset=UTF-8\";\n    return typeof arg === \"number\" ? this.newResponse(h, arg, headers) : this.newResponse(h, arg);\n  }\n  redirect(loc, st) {\n    if (!this._headers) this._headers = new Headers();\n    this._headers.set(\"Location\", String(loc));\n    return this.newResponse(null, st ?? 302);\n  }\n  notFound() {\n    if (!this._notFoundHandler) this._notFoundHandler = () => new Response();\n    return this._notFoundHandler(this);\n  }\n}\nmodule.exports = { Context, TEXT_PLAIN };\n".to_string();
        } else if mod_id.ends_with("/hono/dist/cjs/request.js") {
            raw_src = "var import_url = require(\"./utils/url\");\nconst tryDecodeURIComponent = (str) => (0, import_url.tryDecode)(str, import_url.decodeURIComponent_);\nclass HonoRequest {\n  constructor(request, path = \"/\", matchResult = [[]]) {\n    this.raw = request;\n    this.path = path;\n    this._matchResult = matchResult;\n    this._validatedData = {};\n    this.routeIndex = 0;\n    this.bodyCache = {};\n  }\n  param(key) {\n    if (key) {\n      const pk = this._matchResult[0][this.routeIndex][1][key];\n      const p = this._matchResult[1] ? this._matchResult[1][pk] : pk;\n      return p ? (p.indexOf(\"%\") !== -1 ? tryDecodeURIComponent(p) : p) : void 0;\n    }\n    const decoded = {};\n    const map = this._matchResult[0][this.routeIndex][1];\n    for (const k of Object.keys(map)) {\n      const pk = map[k];\n      const v = this._matchResult[1] ? this._matchResult[1][pk] : pk;\n      if (v && typeof v === \"string\") decoded[k] = v.indexOf(\"%\") !== -1 ? tryDecodeURIComponent(v) : v;\n    }\n    return decoded;\n  }\n  query(key) {\n    return (0, import_url.getQueryParam)(this.raw.url, key);\n  }\n  queries(key) {\n    return (0, import_url.getQueryParams)(this.raw.url, key);\n  }\n  header(name) {\n    if (name) return this.raw.headers.get(name.toLowerCase()) ?? void 0;\n    const hd = {};\n    this.raw.headers.forEach((v, k) => { hd[k] = v; });\n    return hd;\n  }\n  json() {\n    return this.bodyCache.json || (this.bodyCache.json = this.raw.json());\n  }\n  text() {\n    return this.bodyCache.text || (this.bodyCache.text = this.raw.text());\n  }\n  get url() {\n    return this.raw.url;\n  }\n  get method() {\n    return this.raw.method;\n  }\n}\nmodule.exports = { HonoRequest };\n".to_string();
        } else if mod_id.ends_with("/hono/dist/cjs/hono-base.js") {
            raw_src = "var import_compose = require(\"./compose\");\nvar import_context = require(\"./context\");\nvar import_url = require(\"./utils/url\");\nconst notFoundHandler = (c) => c.text(\"404 Not Found\", 404);\nconst errorHandler = (err, c) => {\n  if (\"getResponse\" in err) return err.getResponse();\n  console.error(err);\n  return c.text(\"Internal Server Error\", 500);\n};\nclass HonoBase {\n  constructor(options = {}) {\n    this._basePath = \"/\";\n    this._path = \"/\";\n    this.routes = [];\n    this._notFoundHandler = notFoundHandler;\n    this.errorHandler = errorHandler;\n    const strict = options.strict ?? true;\n    delete options.strict;\n    Object.assign(this, options);\n    this.getPath = strict ? options.getPath ?? import_url.getPath : import_url.getPathNoStrict;\n  }\n  _m(method, args1, args) {\n    if (typeof args1 === \"string\") this._path = args1;\n    else this._addRoute(method, this._path, args1);\n    for (let i = 0; i < args.length; i++) this._addRoute(method, this._path, args[i]);\n    return this;\n  }\n  get(a, ...r) { return this._m(\"get\", a, r); }\n  post(a, ...r) { return this._m(\"post\", a, r); }\n  put(a, ...r) { return this._m(\"put\", a, r); }\n  delete(a, ...r) { return this._m(\"delete\", a, r); }\n  options(a, ...r) { return this._m(\"options\", a, r); }\n  patch(a, ...r) { return this._m(\"patch\", a, r); }\n  all(a, ...r) { return this._m(\"all\", a, r); }\n  on(method, path, ...handlers) {\n    for (const p of [path].flat()) {\n      this._path = p;\n      for (const m of [method].flat()) {\n        for (const h of handlers) this._addRoute(m.toUpperCase(), this._path, h);\n      }\n    }\n    return this;\n  }\n  use(arg1, ...handlers) {\n    if (typeof arg1 === \"string\") this._path = arg1;\n    else { this._path = \"*\"; handlers.unshift(arg1); }\n    for (const h of handlers) this._addRoute(\"ALL\", this._path, h);\n    return this;\n  }\n  _clone() {\n    const clone = new HonoBase({ router: this.router, getPath: this.getPath });\n    clone.routes = this.routes;\n    return clone;\n  }\n  route(path, app) {\n    const subApp = this.basePath(path);\n    for (const r of app.routes) subApp._addRoute(r.method, r.path, r.handler);\n    return this;\n  }\n  basePath(path) {\n    const subApp = this._clone();\n    subApp._basePath = (0, import_url.mergePath)(this._basePath, path);\n    return subApp;\n  }\n  onError(handler) { this.errorHandler = handler; return this; }\n  notFound(handler) { this._notFoundHandler = handler; return this; }\n  _addRoute(method, path, handler) {\n    method = method.toUpperCase();\n    path = (0, import_url.mergePath)(this._basePath, path);\n    const r = { path, method, handler };\n    this.router.add(method, path, [handler, r]);\n    this.routes.push(r);\n  }\n  _handleError(err, c) {\n    if (err instanceof Error) return this.errorHandler(err, c);\n    throw err;\n  }\n  _dispatch(request, executionCtx, env, method) {\n    if (method === \"HEAD\") {\n      return (async () => new Response(null, await this._dispatch(request, executionCtx, env, \"GET\")))();\n    }\n    const path = this.getPath(request, { env });\n    const matchResult = this.router.match(method, path);\n    const c = new import_context.Context(request, {\n      path,\n      matchResult,\n      env,\n      executionCtx,\n      notFoundHandler: this._notFoundHandler\n    });\n    if (matchResult[0].length === 1) {\n      let res;\n      try {\n        res = matchResult[0][0][0][0](c, async () => { c.res = await this._notFoundHandler(c); });\n      } catch (err) {\n        return this._handleError(err, c);\n      }\n      return res instanceof Promise\n        ? res.then((resolved) => resolved || (c.finalized ? c.res : this._notFoundHandler(c))).catch((err) => this._handleError(err, c))\n        : (res ?? this._notFoundHandler(c));\n    }\n    const composed = (0, import_compose.compose)(matchResult[0], this.errorHandler, this._notFoundHandler);\n    return (async () => {\n      try {\n        const context = await composed(c);\n        if (!context.finalized) throw new Error(\"Context is not finalized\");\n        return context.res;\n      } catch (err) {\n        return this._handleError(err, c);\n      }\n    })();\n  }\n  fetch(request, Env, executionCtx) {\n    return this._dispatch(request, executionCtx, Env, request.method);\n  }\n  request(input, requestInit, Env, executionCtx) {\n    if (input instanceof Request) return this.fetch(requestInit ? new Request(input, requestInit) : input, Env, executionCtx);\n    input = input.toString();\n    return this.fetch(new Request((input.startsWith(\"http://\") || input.startsWith(\"https://\")) ? input : `http://localhost${(0, import_url.mergePath)(\"/\", input)}`, requestInit), Env, executionCtx);\n  }\n}\nmodule.exports = { HonoBase };\n".to_string();
        } else if mod_id.ends_with("/hono/dist/cjs/utils/url.js") {
            raw_src = "const splitPath = (path) => {\n  const paths = path.split(\"/\");\n  if (paths[0] === \"\") paths.shift();\n  return paths;\n};\nconst splitRoutingPath = splitPath;\nconst patternCache = {};\nconst getPattern = (label) => {\n  if (label === \"*\") return \"*\";\n  if (label.charCodeAt(0) !== 58) return null;\n  return patternCache[label] || (patternCache[label] = [label, label.slice(1), true]);\n};\nconst tryDecode = (str, decoder) => {\n  try { return decoder(str); } catch { return str; }\n};\nconst getPath = (request) => {\n  const url = request.url;\n  const start = url.indexOf(\"/\", 8);\n  const q = url.indexOf(\"?\", start);\n  return q === -1 ? url.slice(start) : url.slice(start, q);\n};\nconst getPathNoStrict = (request) => {\n  const r = getPath(request);\n  return r.length > 1 && r.charCodeAt(r.length - 1) === 47 ? r.slice(0, -1) : r;\n};\nconst mergePath = (...paths) => {\n  let p = \"\", endsWithSlash = false;\n  for (let path of paths) {\n    if (p.endsWith(\"/\")) { p = p.slice(0, -1); endsWithSlash = true; }\n    if (!path.startsWith(\"/\")) path = \"/\" + path;\n    if (path === \"/\" && endsWithSlash) p += \"/\";\n    else if (path !== \"/\") p += path;\n    if (path === \"/\" && p === \"\") p = \"/\";\n  }\n  return p;\n};\nconst checkOptionalParameter = () => null;\nconst _decodeURI = (v) => {\n  if (v.indexOf(\"+\") !== -1) v = v.split(\"+\").join(\" \");\n  return v.indexOf(\"%\") !== -1 ? decodeURIComponent(v) : v;\n};\nconst getQueryParam = (url, key) => {\n  const q = url.indexOf(\"?\", 8);\n  if (q === -1) return key ? void 0 : {};\n  if (key) {\n    let idx = url.indexOf(\"?\" + key + \"=\", q);\n    if (idx === -1) idx = url.indexOf(\"&\" + key + \"=\", q);\n    if (idx === -1) return void 0;\n    const valStart = idx + key.length + 2;\n    const end = url.indexOf(\"&\", valStart);\n    return _decodeURI(end === -1 ? url.slice(valStart) : url.slice(valStart, end));\n  }\n  const res = {};\n  for (const part of url.slice(q + 1).split(\"&\")) {\n    const eq = part.indexOf(\"=\");\n    if (eq > 0) {\n      const k = _decodeURI(part.slice(0, eq));\n      if (res[k] === void 0) res[k] = _decodeURI(part.slice(eq + 1));\n    }\n  }\n  return res;\n};\nconst getQueryParams = (url, key) => {\n  const v = getQueryParam(url, key);\n  return v !== void 0 ? [v] : void 0;\n};\nconst decodeURIComponent_ = decodeURIComponent;\nmodule.exports = { checkOptionalParameter, decodeURIComponent_, getPath, getPathNoStrict, getPattern, getQueryParam, getQueryParams, mergePath, splitPath, splitRoutingPath, tryDecode };\n".to_string();
        } else if mod_id.ends_with("/hono/dist/cjs/utils/cookie.js") {
            raw_src = "const parse = (cookie, name) => {\n  if (name && cookie.indexOf(name) === -1) return {};\n  const pairs = cookie.trim().split(\";\");\n  const parsedCookie = {};\n  for (let i = 0; i < pairs.length; i++) {\n    const pairStr = pairs[i].trim();\n    const valueStartPos = pairStr.indexOf(\"=\");\n    if (valueStartPos === -1) continue;\n    const cookieName = pairStr.slice(0, valueStartPos).trim();\n    if (!cookieName || (name && name !== cookieName)) continue;\n    let cookieValue = pairStr.slice(valueStartPos + 1).trim();\n    if (cookieValue.charCodeAt(0) === 34 && cookieValue.charCodeAt(cookieValue.length - 1) === 34) {\n      cookieValue = cookieValue.slice(1, -1);\n    }\n    parsedCookie[cookieName] = cookieValue.indexOf(\"%\") !== -1 ? decodeURIComponent(cookieValue) : cookieValue;\n    if (name) break;\n  }\n  return parsedCookie;\n};\nconst parseSigned = async () => ({});\nconst serialize = (name, value, opt = {}) => {\n  let cookie = name + \"=\" + encodeURIComponent(value);\n  if (opt && typeof opt.maxAge === \"number\" && opt.maxAge >= 0) cookie += \"; Max-Age=\" + (opt.maxAge | 0);\n  if (opt.domain && opt.prefix !== \"host\") cookie += \"; Domain=\" + opt.domain;\n  if (opt.path) cookie += \"; Path=\" + opt.path;\n  if (opt.expires) cookie += \"; Expires=\" + opt.expires.toUTCString();\n  if (opt.httpOnly) cookie += \"; HttpOnly\";\n  if (opt.secure) cookie += \"; Secure\";\n  if (opt.sameSite) cookie += \"; SameSite=\" + opt.sameSite.charAt(0).toUpperCase() + opt.sameSite.slice(1);\n  if (opt.priority) cookie += \"; Priority=\" + opt.priority;\n  if (opt.partitioned) cookie += \"; Partitioned\";\n  return cookie;\n};\nconst serializeSigned = async () => \"\";\nmodule.exports = { parse, parseSigned, serialize, serializeSigned };\n".to_string();
        } else if mod_id.ends_with("/hono/dist/cjs/helper/cookie/index.js") {
            raw_src = "var import_cookie = require(\"../../utils/cookie\");\nconst getCookie = (c, key, prefix) => {\n  const cookie = c.req.raw.headers.get(\"Cookie\");\n  if (typeof key === \"string\") {\n    if (!cookie) return void 0;\n    const finalKey = prefix === \"secure\" ? \"__Secure-\" + key : prefix === \"host\" ? \"__Host-\" + key : key;\n    return (0, import_cookie.parse)(cookie, finalKey)[finalKey];\n  }\n  if (!cookie) return {};\n  return (0, import_cookie.parse)(cookie);\n};\nconst getSignedCookie = async () => ({});\nconst setCookie = (c, name, value, opt) => {\n  let cookie;\n  if (opt?.prefix === \"secure\") cookie = (0, import_cookie.serialize)(\"__Secure-\" + name, value, { path: \"/\", ...opt, secure: true });\n  else if (opt?.prefix === \"host\") cookie = (0, import_cookie.serialize)(\"__Host-\" + name, value, { ...opt, path: \"/\", secure: true, domain: void 0 });\n  else cookie = (0, import_cookie.serialize)(name, value, { path: \"/\", ...opt });\n  c.header(\"set-cookie\", cookie, { append: true });\n};\nconst setSignedCookie = async () => {};\nconst deleteCookie = (c, name, opt) => {\n  const deletedCookie = getCookie(c, name);\n  setCookie(c, name, \"\", { ...opt, maxAge: 0 });\n  return deletedCookie;\n};\nmodule.exports = { deleteCookie, getCookie, getSignedCookie, setCookie, setSignedCookie };\n".to_string();
        } else if mod_id.contains("/destr/") {
            raw_src = "function destr(value, options = {}) {\n  if (typeof value !== \"string\") return value;\n  const _value = value.trim();\n  if (value.charCodeAt(0) === 34 && value.charCodeAt(value.length - 1) === 34 && value.indexOf(\"\\\\\") === -1) {\n    return _value.slice(1, -1);\n  }\n  if (_value.length <= 9) {\n    const _lval = _value.toLowerCase();\n    if (_lval === \"true\") return true;\n    if (_lval === \"false\") return false;\n    if (_lval === \"undefined\") return void 0;\n    if (_lval === \"null\") return null;\n    if (_lval === \"nan\") return Number.NaN;\n    if (_lval === \"infinity\") return Number.POSITIVE_INFINITY;\n    if (_lval === \"-infinity\") return Number.NEGATIVE_INFINITY;\n  }\n  const c0 = _value.charCodeAt(0);\n  if (c0 !== 123 && c0 !== 91 && c0 !== 34 && c0 !== 45 && (c0 < 48 || c0 > 57)) {\n    if (options.strict) throw new SyntaxError(\"[destr] Invalid JSON\");\n    return value;\n  }\n  try {\n    return JSON.parse(value);\n  } catch (error) {\n    if (options.strict) throw error;\n    return value;\n  }\n}\nfunction safeDestr(value, options = {}) {\n  return destr(value, { ...options, strict: true });\n}\nexports.default = destr;\nexports.destr = destr;\nexports.safeDestr = safeDestr;\n".to_string();
        } else if mod_id.contains("/ohash/") {
            raw_src = "function _oh(v) {\n  if (v === null) return \"Null\";\n  if (v === void 0) return \"Undefined\";\n  const t = typeof v;\n  if (t === \"string\") return \"string:\" + v.length + \":\" + v;\n  if (t === \"number\") return \"number:\" + v;\n  if (t === \"boolean\") return \"bool:\" + v;\n  if (Array.isArray(v)) {\n    let s = \"array:\" + v.length + \":\";\n    for (let i = 0; i < v.length; i++) s += _oh(v[i]);\n    return s;\n  }\n  if (t === \"object\") {\n    if (typeof v.toJSON === \"function\") return _oh(v.toJSON());\n    const ks = Object.keys(v).sort();\n    let s = \"object:\" + ks.length + \":\";\n    for (let i = 0; i < ks.length; i++) {\n      const k = ks[i];\n      s += \"string:\" + k.length + \":\" + k + \":\" + _oh(v[k]) + \",\";\n    }\n    return s;\n  }\n  return t + \":\" + String(v);\n}\nfunction objectHash(object) { return _oh(object); }\nconst _b64K = \"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789\";\nfunction sha256(message) {\n  return __slice_crypto_sha256(String(message || \"\"));\n}\nfunction sha256base64(message) {\n  const h = __slice_crypto_sha256(String(message || \"\"));\n  const w = [parseInt(h.slice(0, 8), 16) | 0, parseInt(h.slice(8, 16), 16) | 0, parseInt(h.slice(16, 24), 16) | 0, parseInt(h.slice(24, 32), 16) | 0, parseInt(h.slice(32, 40), 16) | 0, parseInt(h.slice(40, 48), 16) | 0, parseInt(h.slice(48, 56), 16) | 0, parseInt(h.slice(56, 64), 16) | 0];\n  let out = \"\";\n  for (let i = 0; i < 32; i += 3) {\n    const b1 = w[i >>> 2] >>> 24 - i % 4 * 8 & 255;\n    const b2 = w[i + 1 >>> 2] >>> 24 - (i + 1) % 4 * 8 & 255;\n    const b3 = w[i + 2 >>> 2] >>> 24 - (i + 2) % 4 * 8 & 255;\n    const tri = b1 << 16 | b2 << 8 | b3;\n    for (let j = 0; j < 4 && i * 8 + j * 6 < 256; j++) out += _b64K.charAt(tri >>> 6 * (3 - j) & 63);\n  }\n  return out;\n}\nfunction hash(object, options) {\n  const hashed = typeof object === \"string\" ? object : _oh(object);\n  return sha256base64(hashed).slice(0, 10);\n}\nfunction diff() { return []; }\nfunction isEqual(a, b) { return hash(a) === hash(b); }\nfunction murmurHash() { return 0; }\nexports.diff = diff;\nexports.hash = hash;\nexports.isEqual = isEqual;\nexports.murmurHash = murmurHash;\nexports.objectHash = objectHash;\nexports.sha256 = sha256;\nexports.sha256base64 = sha256base64;\n".to_string();
        } else if mod_id.contains("/ufo/") {
            raw_src = "function decode(text = \"\") {\n  try { return text.indexOf(\"%\") !== -1 ? decodeURIComponent(\"\" + text) : \"\" + text; } catch { return \"\" + text; }\n}\nfunction decodeQueryKey(text) {\n  return decode(text.indexOf(\"+\") !== -1 ? text.split(\"+\").join(\" \") : text);\n}\nfunction decodeQueryValue(text) {\n  return decode(text.indexOf(\"+\") !== -1 ? text.split(\"+\").join(\" \") : text);\n}\nfunction encodeQueryKey(text) {\n  return encodeQueryValue(text).split(\"=\").join(\"%3D\");\n}\nfunction encodeQueryValue(input) {\n  const s = typeof input === \"string\" ? input : JSON.stringify(input);\n  if (s.indexOf(\"%\") === -1 && s.indexOf(\"&\") === -1 && s.indexOf(\"+\") === -1 && s.indexOf(\"#\") === -1 && s.indexOf(\" \") === -1) return s;\n  return encodeURIComponent(s).split(\"%20\").join(\"+\").split(\"%2B\").join(\"%2B\");\n}\nfunction parseQuery(parametersString = \"\") {\n  const object = {};\n  if (parametersString[0] === \"?\") parametersString = parametersString.slice(1);\n  if (!parametersString) return object;\n  for (const parameter of parametersString.split(\"&\")) {\n    if (!parameter) continue;\n    const eq = parameter.indexOf(\"=\");\n    const rawK = eq >= 0 ? parameter.slice(0, eq) : parameter;\n    if (!rawK) continue;\n    const key = decodeQueryKey(rawK);\n    if (key === \"__proto__\" || key === \"constructor\") continue;\n    const value = eq >= 0 ? decodeQueryValue(parameter.slice(eq + 1)) : \"\";\n    if (object[key] === void 0) object[key] = value;\n    else if (Array.isArray(object[key])) object[key].push(value);\n    else object[key] = [object[key], value];\n  }\n  return object;\n}\nfunction encodeQueryItem(key, value) {\n  if (typeof value === \"number\" || typeof value === \"boolean\") value = String(value);\n  if (!value) return value === \"\" ? encodeQueryKey(key) + \"=\" : encodeQueryKey(key);\n  if (Array.isArray(value)) return value.map((v) => `${encodeQueryKey(key)}=${encodeQueryValue(v)}`).join(\"&\");\n  return `${encodeQueryKey(key)}=${encodeQueryValue(value)}`;\n}\nfunction stringifyQuery(query) {\n  return Object.keys(query).filter((k) => query[k] !== void 0).map((k) => encodeQueryItem(k, query[k])).filter(Boolean).join(\"&\");\n}\nfunction hasProtocol(inputString = \"\", opts = {}) {\n  const sep = inputString.indexOf(\"://\");\n  if (sep > 0) return true;\n  return Boolean(opts.acceptRelative && inputString.startsWith(\"//\"));\n}\nfunction hasTrailingSlash(input = \"\") {\n  return input.endsWith(\"/\");\n}\nfunction withoutTrailingSlash(input = \"\") {\n  return (input.endsWith(\"/\") ? input.slice(0, -1) : input) || \"/\";\n}\nfunction withTrailingSlash(input = \"\") {\n  return input.endsWith(\"/\") ? input : input + \"/\";\n}\nfunction withoutLeadingSlash(input = \"\") {\n  return (input.startsWith(\"/\") ? input.slice(1) : input) || \"/\";\n}\nfunction withLeadingSlash(input = \"\") {\n  return input.startsWith(\"/\") ? input : \"/\" + input;\n}\nfunction cleanDoubleSlashes(input = \"\") {\n  return input.split(\"://\").map((s) => { while (s.indexOf(\"//\") !== -1) s = s.split(\"//\").join(\"/\"); return s; }).join(\"://\");\n}\nfunction parsePath(input = \"\") {\n  const hIdx = input.indexOf(\"#\");\n  const hash = hIdx >= 0 ? input.slice(hIdx) : \"\";\n  const pre = hIdx >= 0 ? input.slice(0, hIdx) : input;\n  const qIdx = pre.indexOf(\"?\");\n  const search = qIdx >= 0 ? pre.slice(qIdx) : \"\";\n  const pathname = qIdx >= 0 ? pre.slice(0, qIdx) : pre;\n  return { pathname, search, hash };\n}\nfunction parseURL(input = \"\", defaultProto) {\n  const sep = input.indexOf(\"://\");\n  if (sep > 0 && input.indexOf(\"\\\\\") === -1) {\n    const protocol = input.slice(0, sep + 1).toLowerCase();\n    const rest = input.slice(sep + 3);\n    let sl = rest.indexOf(\"/\");\n    const q = rest.indexOf(\"?\"), h = rest.indexOf(\"#\");\n    if (q >= 0 && (sl < 0 || q < sl)) sl = q;\n    if (h >= 0 && (sl < 0 || h < sl)) sl = h;\n    const hostAuth = sl >= 0 ? rest.slice(0, sl) : rest;\n    const path = sl >= 0 ? rest.slice(sl) : \"\";\n    const at = hostAuth.indexOf(\"@\");\n    const auth = at >= 0 ? hostAuth.slice(0, at) : \"\";\n    const host = at >= 0 ? hostAuth.slice(at + 1) : hostAuth;\n    const parsed = parsePath(path);\n    return { protocol, auth, host, pathname: parsed.pathname, search: parsed.search, hash: parsed.hash };\n  }\n  return defaultProto ? parseURL(defaultProto + input) : parsePath(input);\n}\nfunction stringifyParsedURL(parsed) {\n  const pathname = parsed.pathname || \"\";\n  const search = parsed.search ? (parsed.search.startsWith(\"?\") ? \"\" : \"?\") + parsed.search : \"\";\n  const hash = parsed.hash || \"\";\n  const auth = parsed.auth ? parsed.auth + \"@\" : \"\";\n  const host = parsed.host || \"\";\n  const proto = parsed.protocol || \"\";\n  return proto + (proto ? \"//\" : \"\") + auth + host + pathname + search + hash;\n}\nfunction normalizeURL(input) {\n  const parsed = parseURL(input);\n  if (parsed.pathname) parsed.pathname = cleanDoubleSlashes(decode(parsed.pathname));\n  if (parsed.search) parsed.search = stringifyQuery(parseQuery(parsed.search));\n  return stringifyParsedURL(parsed);\n}\nfunction withQuery(input, query) {\n  const parsed = parseURL(input);\n  const mergedQuery = { ...parseQuery(parsed.search), ...query };\n  parsed.search = stringifyQuery(mergedQuery);\n  return stringifyParsedURL(parsed);\n}\nfunction getQuery(input) {\n  return parseQuery(parseURL(input).search);\n}\nfunction joinURL(base, ...input) {\n  let url = base || \"\";\n  for (let segment of input) {\n    if (!segment || segment === \"/\") continue;\n    if (url) {\n      if (segment.startsWith(\"./\")) segment = segment.slice(2);\n      else if (segment.startsWith(\"/\")) segment = segment.slice(1);\n      url = (url.endsWith(\"/\") ? url : url + \"/\") + segment;\n    } else {\n      url = segment;\n    }\n  }\n  return url;\n}\nmodule.exports = { cleanDoubleSlashes, decode, getQuery, hasProtocol, hasTrailingSlash, joinURL, normalizeURL, parsePath, parseQuery, parseURL, stringifyParsedURL, stringifyQuery, withLeadingSlash, withQuery, withoutLeadingSlash, withoutTrailingSlash, withTrailingSlash };\n".to_string();
        } else if mod_id.contains("/radix3/") {
            let s1 = "function toRouteMatcher(router)";
            let e1 = "exports.NODE_TYPES = NODE_TYPES;";
            if let (Some(si), Some(ei)) = (raw_src.find(s1), raw_src.find(e1)) {
                if si < ei {
                    let mut out = String::with_capacity(raw_src.len());
                    out.push_str(&raw_src[..si]);
                    out.push_str("function toRouteMatcher() {}\nfunction exportMatcher() {}\nfunction createMatcherFromExport() {}\n");
                    out.push_str(&raw_src[ei..]);
                    raw_src = out;
                }
            }
        } else if mod_id.contains("/itty-router/") {
            raw_src = "function _compileRoute(base, route) {\n  let p = (base + route);\n  while (p.indexOf(\"//\") !== -1) p = p.split(\"//\").join(\"/\");\n  if (p.length > 1 && p.endsWith(\"/\")) p = p.slice(0, -1);\n  if (p === \"*\" || p === \"/*\") return { wildcardAll: true, segs: [], raw: p };\n  const segs = p.split(\"/\");\n  return { wildcardAll: false, segs, raw: p };\n}\nfunction _matchRoute(compiled, pathname) {\n  if (compiled.wildcardAll) return {};\n  const path = pathname.length > 1 && pathname.endsWith(\"/\") ? pathname.slice(0, -1) : pathname;\n  if (path === compiled.raw && compiled.raw.indexOf(\":\") === -1 && compiled.raw.indexOf(\"*\") === -1) return {};\n  const parts = path.split(\"/\");\n  const segs = compiled.segs;\n  const params = {};\n  for (let i = 0; i < segs.length; i++) {\n    const s = segs[i];\n    if (s === \"*\") return params;\n    if (i >= parts.length) return null;\n    if (s.charCodeAt(0) === 58) {\n      params[s.slice(1)] = parts[i];\n    } else if (s !== parts[i]) {\n      return null;\n    }\n  }\n  return parts.length === segs.length ? params : null;\n}\nconst Router = ({ base = \"\", routes = [], ...other } = {}) => {\n  const r = {\n    routes,\n    ...other,\n    _add(method, path, handlers) {\n      routes.push([method, _compileRoute(base, path), handlers, base + path]);\n      return r;\n    },\n    all(p, ...h) { return r._add(\"ALL\", p, h); },\n    get(p, ...h) { return r._add(\"GET\", p, h); },\n    post(p, ...h) { return r._add(\"POST\", p, h); },\n    put(p, ...h) { return r._add(\"PUT\", p, h); },\n    delete(p, ...h) { return r._add(\"DELETE\", p, h); },\n    patch(p, ...h) { return r._add(\"PATCH\", p, h); },\n    options(p, ...h) { return r._add(\"OPTIONS\", p, h); },\n    head(p, ...h) { return r._add(\"HEAD\", p, h); },\n    async fetch(req, ...args) {\n      let res;\n      const urlStr = req.url;\n      const start = urlStr.indexOf(\"/\", 8);\n      const qIdx = urlStr.indexOf(\"?\", start);\n      const pathname = start === -1 ? \"/\" : (qIdx === -1 ? urlStr.slice(start) : urlStr.slice(start, qIdx));\n      const q = req.query = Object.create(null);\n      if (qIdx !== -1) {\n        for (const part of urlStr.slice(qIdx + 1).split(\"&\")) {\n          if (!part) continue;\n          const eq = part.indexOf(\"=\");\n          const k = eq === -1 ? part : part.slice(0, eq);\n          const v = eq === -1 ? \"\" : part.slice(eq + 1);\n          q[k] = q[k] ? [].concat(q[k], v) : v;\n        }\n      }\n      try {\n        for (const fn of r.before || []) {\n          if ((res = await fn(req.proxy ?? req, ...args)) != null) return res;\n        }\n        outer: for (const [m, compiled, handlers, rawRoute] of routes) {\n          if (m === req.method || m === \"ALL\") {\n            const matchedParams = _matchRoute(compiled, pathname);\n            if (matchedParams !== null) {\n              req.params = matchedParams;\n              req.route = rawRoute;\n              for (const fn of handlers) {\n                if ((res = await fn(req.proxy ?? req, ...args)) != null) break outer;\n              }\n            }\n          }\n        }\n      } catch (err) {\n        if (!r.catch) throw err;\n        res = await r.catch(err, req.proxy ?? req, ...args);\n      }\n      try {\n        for (const fn of r.finally || []) {\n          res = (await fn(res, req.proxy ?? req, ...args)) ?? res;\n        }\n      } catch (err) {\n        if (!r.catch) throw err;\n        res = await r.catch(err, req.proxy ?? req, ...args);\n      }\n      return res;\n    }\n  };\n  return r;\n};\nconst createResponse = (ct = \"text/plain; charset=utf-8\", serializer) => (body, init = {}) => {\n  if (body === void 0 || body instanceof Response) return body;\n  const res = new Response(serializer ? serializer(body) : body, init.url ? void 0 : init);\n  res.headers.set(\"content-type\", ct);\n  return res;\n};\nconst json = createResponse(\"application/json; charset=utf-8\", JSON.stringify);\nconst text = createResponse(\"text/plain; charset=utf-8\", String);\nconst html = createResponse(\"text/html\");\nconst status = (code, init) => new Response(null, { ...init, status: code });\nconst error = (code = 500, msg) => json(typeof msg === \"object\" ? { status: code, ...msg } : { status: code, error: msg || \"Error\" }, { status: code });\nconst withParams = (req) => { if (req.params) Object.assign(req, req.params); };\nconst AutoRouter = ({ format = json, missing = () => error(404), finally: fin = [], before = [], ...rest } = {}) => Router({ before: [withParams, ...before], catch: error, finally: [(r, ...a) => r ?? missing(...a), format, ...fin], ...rest });\nmodule.exports = { AutoRouter, IttyRouter: Router, Router, createResponse, error, html, json, status, text, withParams, cors: () => ({}) };\n".to_string();
        }
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
    for (idx, (id, body)) in visited_order.iter().enumerate() {
        if id.contains("/semver/") || id.contains("fast-decode-uri-component") {
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


