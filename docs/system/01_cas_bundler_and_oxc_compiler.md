# `slice-cas`: Zero-Stat Content-Addressed Storage, Oxc Compiler & AST Tree-Shaker

[`crates/slice-cas`](../../crates/slice-cas/src/lib.rs) shifts all module resolution, TypeScript/JSX compilation, hygienic ESM/CJS linking, dead-export elimination, and cold-method `require` deferring from runtime to build/pack time (`slice pack` / `slice compile`).

---

## 1. Production Rust TypeScript & JSX/TSX Compilation via Oxc (`v0.152.0`)

- Implemented in [`strip_typescript_raw`](../../crates/slice-cas/src/compiler/transpiler.rs) ([`compiler/mod.rs`](../../crates/slice-cas/src/compiler/mod.rs)) using `oxc::parser::Parser`, `oxc::semantic::SemanticBuilder`, `oxc::transformer::Transformer`, and `oxc::codegen::Codegen`.
- **Arena-Allocated Single-Pass Parsing & Transformation:** Oxc parses ASTs in a bump-allocated memory arena (`oxc::allocator::Allocator`) and strips TypeScript interfaces, type aliases, generics, parameter properties, and access modifiers (`private`, `protected`, `public`, `readonly`) at native Rust speeds (3x-5x faster than SWC, 40x+ faster than `tsc`). Modules with `return` statements inside functions are parsed once via direct `Program.body` inspection, and live mutable ESM export analysis (`get_module_mutable_exports`) inspects the parsed TypeScript/TSX AST directly without running redundant type-erasure codegen.
- **Native `.tsx` / `.jsx` Compilation:** [`strip_typescript_and_normalize_with_ext`](../../crates/slice-cas/src/compiler/transpiler.rs) selects `SourceType::tsx()` directly on `.tsx`/`.jsx` specifiers with `transform_options.jsx.jsx_plugin = true`, compiling JSX elements into zero-dependency `slice:jsx` (`jsx`, `jsxs`, `Fragment`, `renderToString`) calls on the first parse pass.
- **Plain `.js` Dependency Compaction:** During [`bundle_entrypoint_with_deps`](../../crates/slice-cas/src/bundler/graph_bundler.rs), third-party `.js` files inside `node_modules/` are also passed through Oxc's parser + minifying codegen (`CodegenOptions::minify()`), stripping all JSDoc blocks, license banners, and whitespace before bundling.

---

## 2. Type-Directed Oxc Semantic Lowering & AST Array Pipeline Fusion

### 2.1 Type-Directed Oxc Semantic Lowering ([`analyze_typescript_semantics`](../../crates/slice-cas/src/compiler/semantics.rs))
- Before Oxc strips TypeScript types, `slice-cas` walks `oxc::parser::Parser` AST nodes (`TSInterfaceDeclaration`, `TSTypeAnnotation`, `BindingPattern`) and `oxc::semantic::SemanticBuilder` symbol tables, performing 100% AST-driven lowering:
  1. Extracts exact `string` / `number` / `boolean` property schemas from `TSInterfaceDeclaration` AST nodes and variable/parameter `TSTypeAnnotation` bindings without fragile line/colon text heuristics.
  2. Lowers unshadowed global `Buffer.from(<arg>, <enc>?).toString("base64" | "base64url" | "utf8")` and `Buffer.byteLength(<arg>)` via Oxc `SemanticBuilder` scope verification + AST `CallExpression` span replacement ([`rewrite_buffer_fast_paths_ast`](../../crates/slice-cas/src/linker/cjs_rewrites.rs)) directly to `__slice_base64_encode`, `__slice_base64_decode`, `__slice_buf_to_utf8`, and `__slice_utf8_byte_len` native Rust intrinsics without touching local/shadowed `Buffer` bindings, string literals, or comments.
  3. Collects all unshadowed `require("<spec>")` `CallExpression` string literal spans in a single Oxc AST + `SemanticBuilder` pass per module ([`collect_require_edits_from_program`](../../crates/slice-cas/src/linker/cjs_rewrites.rs) + [`slice_with_edits`](../../crates/slice-cas/src/linker/ast_walker.rs)), eliminating per-statement Oxc parser re-invocations and duplicate CJS parses while preserving user-defined `require` parameters or bindings.

### 2.2 Opt-In Hygienic Oxc AST Array Pipeline Fusion ([`fuse_array_pipelines_ast`](../../crates/slice-cas/src/compiler/pipeline_fusion.rs))
- Standard TypeScript and JavaScript compilation delegates higher-order array and iterator operations (`map`, `filter`, `reduce`, `some`, `every`) directly to `boa_engine`'s ECMAScript-compliant intrinsics, guaranteeing exact sparse-array hole skipping, initial `.length` capture, custom receiver method dispatch, and multi-stage callback ordering.
- For explicit source-to-source optimization workflows, `slice-cas` provides [`fuse_array_pipelines_ast`](../../crates/slice-cas/src/compiler/pipeline_fusion.rs), which selects collision-free temporary prefixes (`choose_hygienic_fusion_prefix`), rejects default parameter initializers and side-effecting multi-stage callbacks, guards `Array.isArray` + standard `Array.prototype` method identity at runtime, snapshots `.length >>> 0`, and skips sparse array holes (`!(i in arr)`).

---

## 3. ESM-First Conditional `"exports"` & General-Purpose Oxc Semantic/AST Tree-Shaker

Implemented across [`crates/slice-cas/src/bundler/resolver.rs`](../../crates/slice-cas/src/bundler/resolver.rs) and [`crates/slice-cas/src/linker/mod.rs`](../../crates/slice-cas/src/linker/mod.rs) with **zero package-specific or framework-specific rules**:

1. **ESM-First `package.json` Conditional `"exports"`, Subpath `"imports"` (`#*`), & `tsconfig.json` `"paths"` Resolution ([`resolve_module_specifier`](../../crates/slice-cas/src/bundler/resolver.rs)):**
   - Resolves Node.js `package.json` Subpath `"imports"` (`#internal/*`, `#config`) with exact, wildcard (`*`), and conditional target support ([`resolve_package_imports`](../../crates/slice-cas/src/bundler/resolver.rs)).
   - Resolves JSONC-comment-tolerant `tsconfig.json` / `jsconfig.json` `"compilerOptions.paths"` and `"compilerOptions.baseUrl"` aliases (`@/*`, `~/*`, including `"extends"` base config inheritance) ([`resolve_tsconfig_paths`](../../crates/slice-cas/src/bundler/resolver.rs)).
   - Prioritizes `"workerd"`, `"worker"`, `"edge-light"`, `"import"`, `"module"`, `"browser"`, `"require"`, `"node"`, and `"default"` in `package.json` `"exports"` and top-level `"module"` before `"main"`.
   - Selecting ESM entrypoints (`dist/index.mjs`, `esm/index.js`) exposes static `import` / `export` declarations to Oxc's AST linker across all modern npm packages (`hono`, `itty-router`, `ufo`, `radix3`, `destr`, `ohash`, `cookie`, `flatted`, `zod`, etc.).
   - Lowers `import.meta` (`url`, `dirname`, `filename`, `env`), dynamic `import("<spec>")` (`ImportExpression`), top-level `await` ([`has_top_level_await`](../../crates/slice-cas/src/linker/ast_walker.rs)), and `__dirname` / `__filename` bindings in a single Oxc AST pass.
2. **Oxc AST Cold-Method `require` Inliner ([`inline_cold_method_requires_ast`](../../crates/slice-cas/src/linker/lazy_and_inline.rs)):**
   - Parses each CommonJS module with `oxc::parser::Parser` and inspects top-level `var/const/let X = require("...")` bindings.
   - If `X` is referenced **exclusively** inside cold class method bodies (never at top level, never in `constructor`, and never in hot dispatch methods `fetch`, `route`, `dispatch`, `match`, `handle`, `run`, `exec`), inlines `require("...")` directly into those cold method bodies and removes the top-level `require` declaration (while protecting non-computed class method/property keys via `non_computed_key_spans`).
   - Example impact: In `hono`'s `HonoRequest` class (`request.js`), `body.js` (`parseBody`) and `url.js` (`tryDecodeURIComponent`) are only used inside `.parseBody()`, `.param()`, and `.query()`, so they are never eagerly evaluated during basic request routing.
3. **Reverse-Topological Oxc Semantic Scope-Resolved Dead-Export Tree-Shaker ([`analyze_module_used_exports`](../../crates/slice-cas/src/linker/tree_shaker.rs) & [`prune_and_minify_cjs_module_ast`](../../crates/slice-cas/src/linker/tree_shaker.rs)):**
   - Computes used export sets (`UsedExports::Named(FxHashSet<String>)` vs `UsedExports::All`) in reverse topological order from the entrypoint down to leaf `node_modules` dependencies:
     - Tracks `const { a, b: alias } = require("mod")` (`ObjectPattern`) and `const ns = require("mod"); ns.foo()` (`StaticMemberExpression` property accesses).
   - Inside `prune_and_minify_cjs_module_ast`:
     - Removes unused `exports.<Unused> = ...` assignments and unused `Object.defineProperty(exports, "<Unused>", ...)` live-getter exports while preserving non-enumerable `Object.defineProperty(exports, "__esModule", { value: true })` namespace metadata (ESM lowering binds default exports on `exports.default` without ever aliasing or mutating user objects via `module.exports`).
     - Runs an iterative fixed-point pass using **`oxc::semantic::SemanticBuilder`** (`scoping.get_resolved_reference_ids(sym_id).is_empty()`), which uses Oxc's full ECMAScript lexical scope resolver so shadowed single-letter local variables (`e`, `t`, `s`, `n`, `r`) inside live functions never false-keep dead top-level declarators, functions, classes, or destructured `require(...)` properties.
     - Emits the pruned AST with `oxc::codegen::Codegen::new().with_options(CodegenOptions::minify())`.
4. **Eager-Edge BFS Lazy Module Stringifier ([`collect_eager_module_indices`](../../crates/slice-cas/src/linker/lazy_and_inline.rs)):**
   - Performs a breadth-first traversal of top-level eager `require(id)` calls starting from the entrypoint.
   - Modules on the eager startup path are emitted as direct function expressions (`__slice_modules[id] = function(module,exports,require){...}`), while cold modules are emitted as JSON string literals (`__slice_modules[id] = "..."`) that are only compiled via `new Function` if dynamically required at runtime, skipping `Boa` lexing and AST construction for all cold transitive modules on startup.

---

## 4. 64-Byte Cache-Line Aligned `SLICE_CAS` v1 Binary Format, Default `SLICEBC\x01` Precompiled `CodeBlock`, & `SLICEEXE`

- Every module blob in a `.cas` archive is hashed with SIMD-accelerated **BLAKE3-256** (`blake3::hash`) and padded to a strict **64-byte boundary** (`CACHE_LINE_ALIGN = 64`):
  ```
  +-----------------------------------------------------------------------+
  | 0..8   : Magic Bytes b"SLICECAS"                                      |
  | 8..16  : Version (u32 = 1) + Reserved Flags (u32 = 0)                 |
  | 16..24 : JSON Radix Header Length (u64 LE)                            |
  | 24..32 : 64-Byte Aligned Data Payload Start Offset (u64 LE)           |
  | 32..N  : CasIndexHeader (Radix Map: Specifier -> BLAKE3 -> Offsets)   |
  | [Padding to 64-byte CPU cache-line boundary]                          |
  | Data   : [64B-aligned Source/Bytecode Slice] ...                      |
  +-----------------------------------------------------------------------+
  ```
- **Default `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS (`boacas = true`) & Single-Copy CAS Deduplication ([`CasBuilder::add_module`](../../crates/slice-cas/src/archive/builder.rs)):**
  - By default (`boacas = true`), `compile_source_to_bytecode` packs bundled JavaScript into a **32-byte `BoaCasHeader` (`SLICEBC\x01`) + `SLCB\x01` recursive `CodeBlock` binary image** ([`crates/slice-engine/src/vm/boacas.rs`](../../crates/slice-engine/src/vm/boacas.rs), [`vendor/boa_engine/SLICE_PATCH.md`](../../vendor/boa_engine/SLICE_PATCH.md)) with pre-fused runtime prelude shards and `node:*` built-in scripts.
  - When a module's compiled bytecode is `SLICEBC\x01` or `SLICEBOA`, `CasBuilder::add_module` writes only the 64B-aligned `bytecode_bytes` into `data_buffer` and aliases `source_offset` directly into the embedded fallback UTF-8 slice inside `bytecode_bytes`, eliminating duplicate source storage in `.cas` archives with zero runtime overhead.
- **Why 64-Byte Alignment & `mmap`?** When [`MappedCasArchive::open`](../../crates/slice-cas/src/archive/reader.rs) maps the file via `memmap2::MmapOptions::map`, every bytecode slice (`get_bytecode_slice`) starts at the beginning of a hardware CPU L1/L2 cache line, avoiding split-line loads and executing **0 `stat()` or `read()` system calls**.
- **Self-Contained Executables (`SLICEEXE`):** [`embed_cas_into_executable`](../../crates/slice-cas/src/archive/reader.rs) streams the host `slice` binary and `.cas` archive through bounded `256 KB` `BufReader`/`BufWriter` buffers into an atomic staging executable followed by a 24-byte trailer (`[u64 cas_offset][u64 cas_len][b"SLICEEXE"]`). On startup, [`MappedCasArchive::open_embedded_self`](../../crates/slice-cas/src/archive/reader.rs) memory-maps `std::env::current_exe()` and slices directly into the embedded archive in `0.21 microseconds`.
- **Embedded Source-Line Table (`CasIndexHeader.source_map_markers`):** While `SLICEBC\x01` and `SLICEBOA` bytecode strip all `//` and `// #slice-source:` comment lines for compact execution, `CasBuilder::write_to_file` stores a compact `(compacted_line_1, "<short>|<full>|<line_map>")` table in `CasIndexHeader.source_map_markers`. [`MappedCasArchive::reconstruct_annotated_source`](../../crates/slice-cas/src/archive/reader.rs) and [`map_bundled_error_location`](../../crates/slice-cas/src/bundler/source_map.rs) map uncaught exceptions back to their exact 1-based TypeScript `file.ts:line` even when running from `.cas` archives or standalone `SLICEEXE` binaries without source files on disk.
- **Zero-Stat Static Asset Table ([`StaticAssetTable`](../../crates/slice-cas/src/archive/static_assets.rs)):** Pre-computes `BLAKE3` `ETag` headers and MIME classifications at pack time for instant `304 Not Modified` edge asset serving.
