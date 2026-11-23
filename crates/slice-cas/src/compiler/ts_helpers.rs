//! TypeScript / Decorator / Explicit-Resource-Management / Auto-Accessor / Standalone ESM Helpers.

use crate::linker::{
    VIRTUAL_NO_FS_DIR, has_esm_default_export_marker, is_builtin_module, is_jsx_runtime_builtin,
    slice_with_edits, transform_module_to_cjs,
};
use std::path::Path;

#[inline]
fn is_ascii_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

#[inline]
pub(crate) fn contains_ident_token(bytes: &[u8], kw: &[u8]) -> bool {
    for pos in memchr::memmem::find_iter(bytes, kw) {
        let left_ok = pos == 0 || !is_ascii_ident_byte(bytes[pos - 1]);
        let end = pos + kw.len();
        let right_ok = end == bytes.len() || !is_ascii_ident_byte(bytes[end]);
        if left_ok && right_ok {
            return true;
        }
    }
    false
}

/// Fast zero-allocation pre-scan checking whether `source` may contain any TypeScript, JSX,
/// decorator (`@`), auto-accessor (`accessor`), or explicit resource management (`using`) syntax.
///
/// Uses identifier-boundary checks instead of space-padded `" kw "` substrings so keywords at
/// BOF, after `(`, `{`, `;`, or `\n` (such as `constructor(readonly x = 1)` or `abstract class A`)
/// are never missed.
#[inline]
pub(crate) fn may_contain_ts_or_jsx(source: &str) -> bool {
    let bytes = source.as_bytes();
    if memchr::memchr3(b':', b'<', b'!', bytes).is_some() || memchr::memchr(b'@', bytes).is_some() {
        return true;
    }
    for kw in [
        &b"interface"[..],
        &b"type"[..],
        &b"enum"[..],
        &b"namespace"[..],
        &b"declare"[..],
        &b"as"[..],
        &b"satisfies"[..],
        &b"implements"[..],
        &b"abstract"[..],
        &b"readonly"[..],
        &b"public"[..],
        &b"private"[..],
        &b"protected"[..],
        &b"override"[..],
        &b"accessor"[..],
        &b"using"[..],
    ] {
        if contains_ident_token(bytes, kw) {
            return true;
        }
    }
    false
}

/// Returns `(needs_cjs_module_mode, has_accessor_property)` for an already-parsed Oxc `Program`:
/// - `needs_cjs_module_mode`: `true` when `import x = require("...")` or `export = ...` is present,
///   telling `oxc::transformer` to emit CommonJS (`const x = require(...)` / `module.exports = ...`)
///   instead of raising an ESM diagnostic.
/// - `has_accessor_property`: `true` when any class contains an `accessor` property.
pub(crate) fn inspect_ts_transform_flags(program: &oxc::ast::ast::Program<'_>) -> (bool, bool) {
    use oxc::ast::ast::{
        AccessorProperty, TSExportAssignment, TSImportEqualsDeclaration, TSModuleReference,
    };
    use oxc::ast_visit::{Visit, walk};

    struct Inspector {
        needs_cjs_mode: bool,
        has_accessor: bool,
    }

    impl<'a> Visit<'a> for Inspector {
        #[inline]
        fn visit_ts_export_assignment(&mut self, _it: &TSExportAssignment<'a>) {
            self.needs_cjs_mode = true;
        }

        #[inline]
        fn visit_ts_import_equals_declaration(&mut self, it: &TSImportEqualsDeclaration<'a>) {
            if matches!(
                it.module_reference,
                TSModuleReference::ExternalModuleReference(_)
            ) {
                self.needs_cjs_mode = true;
            }
            walk::walk_ts_import_equals_declaration(self, it);
        }

        #[inline]
        fn visit_accessor_property(&mut self, _it: &AccessorProperty<'a>) {
            self.has_accessor = true;
        }
    }

    let mut inspector = Inspector {
        needs_cjs_mode: false,
        has_accessor: false,
    };
    inspector.visit_program(program);
    (inspector.needs_cjs_mode, inspector.has_accessor)
}

/// Self-contained ES2020 runtime helpers emitted only when Oxc lowers `@decorator` or `using` / `await using`.
pub(crate) const SLICE_OXC_BABEL_HELPERS: &str = r#"var babelHelpers = globalThis.babelHelpers || (globalThis.babelHelpers = {
  decorate: function(decorators, target, key, desc) {
    var c = arguments.length, r = c < 3 ? target : desc === null ? desc = Object.getOwnPropertyDescriptor(target, key) : desc, d;
    for (var i = decorators.length - 1; i >= 0; i--) {
      if (d = decorators[i]) r = (c < 3 ? d(r) : c > 3 ? d(target, key, r) : d(target, key)) || r;
    }
    if (c > 3 && r) Object.defineProperty(target, key, r);
    return r;
  },
  decorateParam: function(index, decorator) {
    return function(target, key) { decorator(target, key, index); };
  },
  decorateMetadata: function(k, v) {
    if (typeof Reflect === "object" && typeof Reflect.metadata === "function") return Reflect.metadata(k, v);
  },
  usingCtx: function() {
    var symSync = Symbol.dispose || Symbol.for("Symbol.dispose");
    var symAsync = Symbol.asyncDispose || Symbol.for("Symbol.asyncDispose");
    var stack = [], NO_ERR = {}, err = NO_ERR, hasErr = false;
    function pushDisp(v, asyncMode) {
      if (v === null || v === undefined) return v;
      if (typeof v !== "object" && typeof v !== "function") throw new TypeError("Object expected to be disposable");
      var fn = asyncMode ? (v[symAsync] || v[symSync]) : v[symSync];
      if (typeof fn !== "function") throw new TypeError("Object is not disposable");
      stack.push({ v: v, fn: fn, a: asyncMode });
      return v;
    }
    function wrapSuppressed(nextErr, prevErr) {
      if (typeof SuppressedError === "function") return new SuppressedError(nextErr, prevErr, "An error was suppressed during disposal");
      var e = new Error("An error was suppressed during disposal");
      e.name = "SuppressedError";
      e.error = nextErr;
      e.suppressed = prevErr;
      return e;
    }
    return {
      e: NO_ERR,
      u: function(v) { return pushDisp(v, false); },
      a: function(v) { return pushDisp(v, true); },
      d: function() {
        var caught = this.e !== NO_ERR;
        if (caught) { err = this.e; hasErr = true; }
        function next() {
          while (stack.length > 0) {
            var item = stack.pop();
            try {
              var res = item.fn.call(item.v);
              if (item.a) return Promise.resolve(res).then(next, onDispErr);
            } catch (e) {
              onDispErr(e);
            }
          }
          if (hasErr) throw err;
        }
        function onDispErr(e) {
          err = hasErr ? wrapSuppressed(e, err) : e;
          hasErr = true;
          return next();
        }
        return next();
      }
    };
  }
});
"#;

/// Lowers remaining ES Stage-3 `accessor` class elements (`accessor x = 1;`, `static accessor y = 2;`)
/// into standard ES2022 private backing fields (`#__slice_acc_N`) + `get` / `set` pairs.
pub(crate) fn lower_auto_accessors_in_js(js_code: &str) -> String {
    use oxc::ast::ast::{AccessorProperty, Class, PropertyKey};
    use oxc::ast_visit::{Visit, walk};
    use oxc::span::GetSpan;

    if !contains_ident_token(js_code.as_bytes(), b"accessor") {
        return js_code.to_string();
    }

    let allocator = oxc::allocator::Allocator::default();
    let parse_opts = oxc::parser::ParseOptions {
        allow_return_outside_function: true,
        ..Default::default()
    };
    let mut ret = oxc::parser::Parser::new(
        &allocator,
        js_code,
        oxc::span::SourceType::ts().with_module(true),
    )
    .with_options(parse_opts)
    .parse();
    if !ret.diagnostics.is_empty() {
        ret = oxc::parser::Parser::new(
            &allocator,
            js_code,
            oxc::span::SourceType::ts().with_module(false),
        )
        .with_options(parse_opts)
        .parse();
        if !ret.diagnostics.is_empty() {
            return js_code.to_string();
        }
    }

    struct AccessorCollector<'s> {
        src: &'s str,
        counter: usize,
        edits: Vec<(usize, usize, String)>,
    }

    impl<'a> Visit<'a> for AccessorCollector<'_> {
        fn visit_class(&mut self, cls: &Class<'a>) {
            walk::walk_class(self, cls);
        }

        fn visit_accessor_property(&mut self, acc: &AccessorProperty<'a>) {
            if acc.r#type.is_abstract() {
                self.edits.push((
                    acc.span.start as usize,
                    acc.span.end as usize,
                    String::new(),
                ));
                return;
            }
            let id = self.counter;
            self.counter += 1;
            let priv_field = format!("#__slice_acc_{id}");
            let static_kw = if acc.r#static { "static " } else { "" };
            let init_rhs = match &acc.value {
                Some(expr) => {
                    let s = expr.span().start as usize;
                    let e = expr.span().end as usize;
                    format!(" = {}", &self.src[s..e])
                }
                None => String::new(),
            };
            let key_src = {
                let ks = acc.key.span().start as usize;
                let ke = acc.key.span().end as usize;
                if acc.computed {
                    format!("[{}]", &self.src[ks..ke])
                } else if matches!(acc.key, PropertyKey::PrivateIdentifier(_)) {
                    let raw = &self.src[ks..ke];
                    // Private accessor `#x`: use a distinct backing slot `#__slice_acc_N`
                    raw.to_string()
                } else {
                    self.src[ks..ke].to_string()
                }
            };
            let replacement = format!(
                "{static_kw}{priv_field}{init_rhs}; {static_kw}get {key_src}() {{ return this.{priv_field}; }} {static_kw}set {key_src}(__v) {{ this.{priv_field} = __v; }}"
            );
            self.edits
                .push((acc.span.start as usize, acc.span.end as usize, replacement));
        }
    }

    let mut collector = AccessorCollector {
        src: js_code,
        counter: 0,
        edits: Vec::new(),
    };
    collector.visit_program(&ret.program);
    if collector.edits.is_empty() {
        return js_code.to_string();
    }
    collector.edits.sort_by_key(|e| e.0);
    slice_with_edits(js_code, 0, js_code.len(), &collector.edits)
}

/// Checks whether a standalone (non-bundled) JS module contains `export default`, CJS `module.exports`,
/// or an ESM `import` targeting a built-in runtime module (including bare `"fs"`, `"path"`, `"crypto"`,
/// and whitespace-less `import{join}from"path"`).
pub(crate) fn has_standalone_builtin_import_or_cjs_export(js_code: &str) -> bool {
    let bytes = js_code.as_bytes();
    if memchr::memmem::find(bytes, b"var __slice_modules =").is_some() {
        return false;
    }
    if memchr::memmem::find(bytes, b"module.exports").is_some() {
        return true;
    }
    for pos in memchr::memmem::find_iter(bytes, b"export") {
        let left_ok = pos == 0 || !is_ascii_ident_byte(bytes[pos - 1]);
        if !left_ok {
            continue;
        }
        let rest = &js_code[pos + 6..];
        let trimmed = rest.trim_start();
        if let Some(after_def) = trimmed.strip_prefix("default")
            && after_def
                .as_bytes()
                .first()
                .is_none_or(|&b| !is_ascii_ident_byte(b))
        {
            return true;
        }
    }
    if !contains_ident_token(bytes, b"import") {
        return false;
    }
    // Parse quickly only when `import` token is present to inspect exact `ImportDeclaration` specifiers
    let allocator = oxc::allocator::Allocator::default();
    let parse_opts = oxc::parser::ParseOptions {
        allow_return_outside_function: true,
        ..Default::default()
    };
    let ret = oxc::parser::Parser::new(
        &allocator,
        js_code,
        oxc::span::SourceType::ts().with_module(true),
    )
    .with_options(parse_opts)
    .parse();
    for stmt in &ret.program.body {
        if let oxc::ast::ast::Statement::ImportDeclaration(imp) = stmt {
            let spec = imp.source.value.as_str();
            if is_builtin_module(spec) || is_jsx_runtime_builtin(spec) {
                return true;
            }
        }
    }
    false
}

/// Fast pre-check returning `true` when `js_code` is not already a bundled `__slice_modules`
/// envelope and contains an `import`, `export`, `module.exports`, or `require(` token that
/// may need unbundled ESM/CJS lowering for Tier-1 `Boa` execution.
#[inline]
pub fn needs_unbundled_esm_lowering(js_code: &str) -> bool {
    let bytes = js_code.as_bytes();
    if memchr::memmem::find(bytes, b"var __slice_modules =").is_some() {
        return false;
    }
    contains_ident_token(bytes, b"import")
        || contains_ident_token(bytes, b"export")
        || memchr::memmem::find(bytes, b"module.exports").is_some()
        || memchr::memmem::find(bytes, b"require(").is_some()
}

/// Lowers any unbundled ESM `import` / `export` statements into CommonJS (`__slice_cjs_require` +
/// `exports` / `module.exports`) with zero host filesystem `stat()` syscalls so Tier-1 `Boa` can
/// execute both standalone `export const` scripts and multi-blob `MappedCasArchive` modules.
pub fn lower_unbundled_esm_for_boa(
    js_code: &str,
    rewrite_require_fn: fn(&str) -> String,
) -> String {
    let bytes = js_code.as_bytes();
    if memchr::memmem::find(bytes, b"var __slice_modules =").is_some() {
        return js_code.to_string();
    }
    let has_imp = contains_ident_token(bytes, b"import");
    let has_exp = contains_ident_token(bytes, b"export");
    let has_mod_exp = memchr::memmem::find(bytes, b"module.exports").is_some();
    if !has_imp && !has_exp && !has_mod_exp {
        return if memchr::memmem::find(bytes, b"require(").is_some() {
            rewrite_require_fn(js_code)
        } else {
            js_code.to_string()
        };
    }

    let mut dummy_deps = Vec::new();
    let cjs = if has_imp || has_exp {
        transform_module_to_cjs(js_code, Path::new(VIRTUAL_NO_FS_DIR), None, &mut dummy_deps)
    } else {
        js_code.to_string()
    };
    let had_esm_transform = cjs != js_code;
    if has_esm_default_export_marker(&cjs) {
        let digest = blake3::hash(cjs.as_bytes());
        let rc_u64 = u64::from_le_bytes(digest.as_bytes()[0..8].try_into().unwrap());
        let rc_key = format!("{:016x}", rc_u64);
        let direct_body = rewrite_require_fn(&cjs);
        return format!(
            "var __slice_modules = {{}};//__SLICE_RC:{rc_key}\nif (typeof globalThis.__slice_cas_exports === \"object\" && globalThis.__slice_cas_exports !== null) {{\nvar exports = globalThis.__slice_cas_exports, module = globalThis.__slice_cas_module;\n{direct_body}\n}} else {{\nvar __rc = globalThis.__slice_rc || (globalThis.__slice_rc = Object.create(null));\nvar __run = __rc[\"{rc_key}\"];\nif (!__run) {{\nvar __h = null;\n__run = __rc[\"{rc_key}\"] = function() {{\nif (!__h) {{\nvar exports = {{}}, module = {{ exports: exports }};\n{direct_body}\n__h = exports.__esModule ? exports.default : module.exports;\n}}\nif (__h) globalThis.__slice_dispatch_fetch(__h);\n}};\n}}\n__run();\n}}\n"
        );
    }
    let rewritten = rewrite_require_fn(&cjs);
    if (had_esm_transform || has_mod_exp)
        && memchr::memmem::find(rewritten.as_bytes(), b"exports").is_some()
    {
        format!(
            "var exports = (typeof globalThis.__slice_cas_exports === \"object\" && globalThis.__slice_cas_exports !== null) ? globalThis.__slice_cas_exports : {{}}, module = (typeof globalThis.__slice_cas_module === \"object\" && globalThis.__slice_cas_module !== null) ? globalThis.__slice_cas_module : {{ exports: exports }};\n{rewritten}"
        )
    } else {
        rewritten
    }
}
