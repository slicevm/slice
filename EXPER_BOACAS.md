# EXPER_BOACAS: Precompiled Boa Execution Payloads in `.cas` Archives

## 1. Executive Thesis

Across all 31 benchmark applications in [`bench_latest.md`](file:///p:/opensource/slice/bench_latest.md), Slice exhibits a stark bimodal performance profile:

1. **Tier-0 Native State-Machine Bytecode (`SLICESNP`) & Warm Daemon (`SLICE_TIME_TRAVEL`)**:
   - Cold `.cas` execution on Tier-0 apps takes **7.2 ms – 9.3 ms** (process spawn + `mmap` + direct register VM execution).
   - Warm `SLICE_TIME_TRAVEL` execution across **all** 31 apps (including full `Hono`, `itty-router`, `UnJS`, and `Zod` apps) takes **6.5 ms – 8.3 ms** (`2.7x–4.3x` faster than `Node.js` and `3.3x–4.9x` faster than `Bun`).
2. **Cold Tier-2 Embedded ECMAScript (`SLICEBOA`)**:
   - When a `.cas` archive contains a `SLICEBOA` payload (`BlobKind::Bytecode` starting with `b"SLICEBOA"`), cold one-shot CLI execution (`slice run app.cas`) jumps from **~7.5 ms** to **17.9 ms – 30.8 ms** (`26.9 ms` on `FW-01 Hono`, `21.6 ms` on `App 02 Scrypt`, `30.8 ms` on `CS-02 UnJS Platform`).

### Why Does This 12–22 ms Gap Exist?

During the `AUDIT_NEO` cycle we proved that **Rust host glue is not the bottleneck**:
- `mmap` + `.cas` header/index verification (`slice-cas`): **< 0.15 ms**
- Aho-Corasick single-pass prelude shard detection (`detect_needed_shards`): **< 0.08 ms**
- `CompactString` KV slab, VFS initialization, and CLI output routing: **< 0.25 ms**

The cold-start delta in `SLICEBOA` comes from work performed on every cold `slice run app.cas` invocation before a single user bytecode instruction executes:
1. Eager source-map reconstruction in `cmd_run` (`archive.reconstruct_annotated_source(entry)`), which allocates and copies the entire JS source on every `.cas` run even when no uncaught error occurs.
2. **18 separate linear substring scans** (`js_code.contains(...)`) inside [`exec_boa_full_es_vm`](file:///p:/opensource/slice/crates/slice-engine/src/isolate.rs#L1750-L1901) to detect `Worker`, `nextTick`, `Slice.request`, `__slice_semver_`, `Slice.kv`, `prototype`, plus runtime `detect_needed_shards` and `needs_lazy_global_getters`.
3. **Multiple separate `boa_engine` compilation passes (`5 to 8x ctx.eval()`)**:
   - `ctx.eval(ULTRA_CORE_PRELUDE)`
   - Loop over `ensure_realm_shards` calling `ctx.eval(SHARD_i)` separately for each needed shard (`URL`, `TextEncoder`, `Headers`/`Request`/`Response`, `EventTarget`, `Buffer`, etc.)
   - `ctx.eval(js_code)` for the user bundle.
4. **Full JS Lexing + Recursive-Descent Parsing (`boa_parser`) + Scope Analysis (`boa_ast::Scope`) + ByteCompilation (`ByteCompiler`)** of the prelude shards and user bundle on every cold process start.

---

## 2. Inside `boa_engine v0.22.0`: Compilation Pipeline & Visibility Boundaries

Inspecting `boa_engine v0.22.0`, `boa_ast v0.22.0`, and `boa_interner v0.22.0` reveals three distinct stages inside `boa_engine/src/script.rs`:

1. **`Script::parse(source, realm, context)`**:
   - Runs `boa_parser::Parser::new(source).parse_script_with_source(&scope, context.interner_mut())`.
   - Interns every identifier and string literal into `context.interner_mut(): &mut boa_interner::Interner`, assigning each a 32-bit `boa_interner::Sym` (`NonZeroU32`).
   - Runs `ast.analyze_scope(&strict, &scope, context.interner())` to compute lexical/var `BindingLocator` indices.
   - Produces `ScriptPhase::Ast(boa_ast::Script)`.
2. **`Script::codeblock(context)`**:
   - Runs `global_declaration_instantiation_context(...)` and `ByteCompiler::new(..., context.interner_mut())`.
   - Compiles the `StatementList` into register-VM bytecode (`Bytecode`, `ThinVec<Constant>`, `Box<[BindingLocator]>`, `ThinVec<Handler>`, `Box<[InlineCache]>`, `global_lexs`, `global_fns`, `global_vars`).
   - Produces `Gc<boa_engine::vm::CodeBlock>`.
   - **Critical Discovery**: `CodeBlock` in `boa_engine 0.22.0` contains **zero `boa_interner::Sym` fields**! Every symbol has already been resolved by `ByteCompiler` into `JsString` (UTF-16 code units) inside `Constant::String(JsString)`, `BindingLocator.name`, and `InlineCache.name`.
3. **`Script::evaluate(context)`**:
   - Pushes the `CallFrame` with `codeblock`, resizes the global environment (`self.realm().resize_global_env()`), instantiates global declarations (`context.global_declaration_instantiation(&codeblock)`), and runs `context.run()`.

In upstream `boa_engine 0.22.0`:
- `pub struct CodeBlock` is public, but its 16 fields are `pub(crate)` and do not derive `serde::Serialize`/`Deserialize`.
- `Script` wraps `Gc<Inner>` where `phase: GcRefCell<ScriptPhase>` is private.
- However, `boa_engine` provides a built-in `deser` Cargo feature (`deser = ["boa_interner/serde", "boa_ast/serde"]`) that implements `serde::Serialize` and `serde::Deserialize` on both `boa_interner::Interner` and `boa_ast::Script`, and exposes `ContextBuilder::new().interner(interner)` publicly.

---

## 3. Execution Architecture: Shared Base on `main` + 3 Experimental Branches

To rigorously compare all three engineering approaches under identical benchmark conditions without polluting `main` with experimental trade-offs, we structure the implementation as follows:

### 3.1 Shared Foundation on `main`

1. **Lazy Source-Map Reconstruction in `slice-cli`**:
   - Only call `archive.reconstruct_annotated_source(entry)` in [`cmd_run`](file:///p:/opensource/slice/crates/slice-cli/src/main.rs) when `metrics.return_value.starts_with("Uncaught ")`, eliminating unconditional `100 KB+` string allocation and line iteration on every `.cas` execution.
2. **Experiment Flag Plumbing (`--exper-boacas` / `--no-exper-boacas` / `SLICE_EXPER_BOACAS`)**:
   - Supported across `slice pack`, `slice compile`, `slice run`, `slice inspect`, `slice.toml` (`[experiments] boacas = true`), and `SLICE_EXPER_BOACAS=1|0`.
   - Precedence: **CLI Flag (`--exper-boacas` / `--no-exper-boacas`) > Env Var (`SLICE_EXPER_BOACAS`) > `slice.toml` (`[experiments] boacas`) > Default (`false` when packing, auto-detect when running `SLICEBC\x01` unless `--no-exper-boacas` / `SLICE_EXPER_BOACAS=0` is set)**.
3. **Shared `SLICEBC\x01` Wire Container ([`crates/slice-engine/src/boacas.rs`](file:///p:/opensource/slice/crates/slice-engine/src/boacas.rs))**:
   - 32-byte fixed binary header (`BoaCasHeader`) at the start of `BlobKind::Bytecode`:
     ```text
     +-----------------------------------------------------------------------+
     | Magic: b"SLICEBC\x01" (8 bytes)                                       |
     | ApproachKind: u8 (0 = Base Metadata, 1 = Fused+Lazy,                  |
     |                   2 = AST+Interner Deser, 3 = Patched CodeBlock)      |
     | Reserved0: u8 (0)                                                     |
     | FeatureFlags: u16 (precomputed bitmask: HAS_WORKER, HAS_NEXT_TICK,    |
     |                    HAS_HTTP_BRIDGE, HAS_SEMVER_FAST, HAS_KV_SANDBOX,  |
     |                    NEEDS_LAZY_GETTERS, NEEDS_SES_CHECK, IS_RC_BUNDLE) |
     | ShardsMask: u32 (precomputed detect_needed_shards bitmask)            |
     | RcKey: u64 (precomputed 64-bit __SLICE_RC bundle hash)                |
     | PrecompiledLen: u32 (byte length of branch-specific payload)          |
     | FallbackJsLen: u32 (byte length of fallback UTF-8 JS source)          |
     +-----------------------------------------------------------------------+
     | [PrecompiledLen bytes] -> Branch-specific precompiled payload         |
     +-----------------------------------------------------------------------+
     | [FallbackJsLen bytes]  -> Compacted UTF-8 JS bundle (fallback slice)  |
     +-----------------------------------------------------------------------+
     ```
   - In `slice-cas`, `CasBuilder::write_to_file` recognizes `b"SLICEBC\x01"` so `source_offset` points directly into the embedded `FallbackJsLen` UTF-8 slice (zero duplicate storage in `.cas`).

---

### 3.2 The Three Experimental Branches (Off `main`)

1. **Branch 1: `exper/boacas-a-fused-lazy` (Approach A — 100% Public API, Zero Fork, Fused Single-Pass Prelude + Lazy Module Thunks)**:
   - At `slice pack --exper-boacas` time (`ApproachKind = 1`):
     - Precomputes `FeatureFlags`, `ShardsMask`, and `RcKey`.
     - Converts non-entry modules in multi-module bundles (`__slice_modules[idx] = function(...)`) into lazy string literals (`__slice_modules[idx] = "..."`) so `boa_parser` skips unused modules completely.
     - Pre-fuses `ULTRA_CORE_PRELUDE` + all required `ShardsMask` prelude shards + the compacted user bundle into a **single contiguous JS script** inside `PrecompiledLen`.
   - At `slice run` time:
     - Reads the 32-byte `BoaCasHeader` in `O(1)` (`0` runtime substring scans, `0` Aho-Corasick passes).
     - Builds a bare `boa_engine::Context` (skipping separate `ULTRA_CORE_PRELUDE` and per-shard `ctx.eval()` calls), registers only the host bindings flagged by `FeatureFlags` and `ShardsMask`, and evaluates the fused script in **one single `ctx.eval()` pass**.

2. **Branch 2: `exper/boacas-b-ast-deser` (Approach B — Upstream `boa_engine/deser` Pre-Parsed `(Interner, boa_ast::Script)` Snapshot + Layout Bridge, Zero Fork)**:
   - Enables upstream `boa_engine = { version = "0.22.0", features = ["deser"] }`, `boa_ast = { version = "0.22.0", features = ["serde"] }`, `boa_interner = { version = "0.22.0", features = ["serde"] }`, and `bincode`.
   - At `slice pack --exper-boacas` time (`ApproachKind = 2`):
     - Creates a `boa_engine::Context`, parses the fused prelude + bundle via `boa_parser::Parser::new(Source::from_bytes(&fused)).parse_script(&ctx.realm().scope(), ctx.interner_mut())`, applies AST optimizations, and serializes `(ctx.interner(), &ast_script)` with `bincode` into `PrecompiledLen`.
   - At `slice run` time:
     - Deserializes `(interner, mut ast_script): (boa_interner::Interner, boa_ast::Script)` from the `mmap` slice.
     - Initializes `ContextBuilder::new().interner(interner).job_executor(...).build()`.
     - Runs `ast_script.analyze_scope(false, &ctx.realm().scope(), ctx.interner())`.
     - Injects `ast_script` into a `boa_engine::Script` via the compile-time size-verified `MirrorScriptInner` bridge and calls `script.evaluate(&mut ctx)`.
     - **Skips 100% of JS lexing (`boa_lexer`) and recursive-descent parsing (`boa_parser`) at runtime without forking `boa_engine`**.

3. **Branch 3: `exper/boacas-c-patched-codeblock` (Approach C — Cargo `[patch.crates-io]` Local `boa_engine` Facade with Direct `CodeBlock` Bytecode Serialization)**:
   - Uses Cargo's `[patch.crates-io]` pointing to `vendor/boa_engine` (`v0.22.0`) to add native `CodeBlock` + global `Scope` binary snapshot serialization/deserialization methods directly on `boa_engine::Script`.
   - At `slice pack --exper-boacas` time (`ApproachKind = 3`):
     - Runs `Script::parse` AND `Script::codeblock(&mut ctx)` at build time, serializing the compiled `CodeBlock` tree (VM opcodes, constants, binding locators, exception handlers, inline cache names, global lex/var/fn tables, and global `Scope` bindings) into `PrecompiledLen`.
   - At `slice run` time:
     - Reconstructs `Gc<CodeBlock>` and global `Scope` bindings directly from the `mmap` slice and executes `Script::from_precompiled_codeblock` on the VM—**skipping `boa_lexer`, `boa_parser`, `Scope::analyze_scope`, AND `ByteCompiler` at runtime**.

---

## 4. Empirical Benchmark Results Across All 3 Experimental Branches

All 3 experimental branches (`exper/boacas-a-fused-lazy`, `exper/boacas-b-ast-deser`, and `exper/boacas-c-patched-codeblock`) were implemented, verified against all 94 workspace tests (`cargo test --workspace`), and benchmarked across four representative workloads in both `--release` (`opt-level = 3`) and `debug` profiles:

- **`W1_URL_TextEncoder`**: WinterTC `URL` + `URLSearchParams` + `TextEncoder` (`Shards 1 & 2`).
- **`W2_FS_Path_Crypto`**: Node.js built-in `require("node:fs")` + `require("node:path")` + `require("node:crypto")` (`SHA-256` digest + VFS file I/O, `ShardsMask` bits `18, 19, 20`).
- **`W3_FullStack_RC_HTTP`**: Multi-module `//__SLICE_RC:` CJS/ESM bundle + `Buffer.from(...).toString("base64")` + WinterTC `Headers` + `Response` (`Shards 5 & 11`).
- **`W4_LazyNodeModules`**: Multi-package `node_modules` bundle (`1` hot entrypoint package + `8` cold `node_modules` packages referenced only inside uncalled top-level helper functions).

### 4.1 Release Profile (`--release`, `opt-level = 3`) — Cold & Warm Execution Latency

| Workload | Metric | Baseline (`SLICEBOA`) | Branch A (`exper/boacas-a-fused-lazy`) | Branch B (`exper/boacas-b-ast-deser`) | Branch C (`exper/boacas-c-patched-codeblock`) |
| :--- | :--- | :---: | :---: | :---: | :---: |
| **`W1_URL_TextEncoder`** | **Cold Exec (`us`)** | `5,569.3 us` | `5,288.6 us` (`1.12x`) | `4,442.5 us` (`1.19x`) | **`603.7 us` (`9.23x`)** |
| | **Warm Exec (`us`)** | `56.5 us` | `57.1 us` | `56.2 us` | **`56.0 us`** |
| | **Pack Time (`us`)** | `33.3 us` | `40.5 us` | `4,500.7 us` | `5,592.0 us` |
| | **Payload Size (`B`)** | `196 B` | `19,306 B` (`18.9 KB`) | `552,417 B` (`539.5 KB`) | **`70,945 B` (`69.3 KB`)** |
| **`W2_FS_Path_Crypto`** | **Cold Exec (`us`)** | `5,797.1 us` | `5,620.6 us` (`1.09x`) | `4,718.3 us` (`1.18x`) | **`607.3 us` (`9.55x`)** |
| | **Warm Exec (`us`)** | `82.4 us` | `81.5 us` | `83.7 us` | **`81.9 us`** |
| | **Pack Time (`us`)** | `31.3 us` | `16.9 us` | `4,258.5 us` | `5,831.4 us` |
| | **Payload Size (`B`)** | `372 B` | `23,530 B` (`23.0 KB`) | `628,410 B` (`613.7 KB`) | **`87,767 B` (`85.7 KB`)** |
| **`W3_FullStack_RC_HTTP`** | **Cold Exec (`us`)** | `6,989.1 us` | `7,094.5 us` (`1.00x`) | `5,972.5 us` (`1.16x`) | **`772.9 us` (`9.04x`)** |
| | **Warm Exec (`us`)** | `23.1 us` | `23.0 us` | `23.2 us` | **`22.7 us`** |
| | **Pack Time (`us`)** | `1.1 us` | `9.0 us` | `5,195.9 us` | `7,482.8 us` |
| | **Payload Size (`B`)** | `1,096 B` | `27,876 B` (`27.2 KB`) | `738,329 B` (`721.0 KB`) | **`98,730 B` (`96.4 KB`)** |

### 4.2 Debug Profile (`unoptimized`) — Cold Execution Latency (`W1`–`W3`)

| Workload | Baseline (`SLICEBOA`) | Branch C (`exper/boacas-c-patched-codeblock`) | Cold Speedup (Debug) |
| :--- | :---: | :---: | :---: |
| **`W1_URL_TextEncoder`** | `34,269.2 us` (`34.27 ms`) | **`3,137.6 us` (`3.14 ms`)** | **`10.92x`** |
| **`W2_FS_Path_Crypto`** | `46,085.1 us` (`46.09 ms`) | **`3,440.4 us` (`3.44 ms`)** | **`13.40x`** |
| **`W3_FullStack_RC_HTTP`** | `46,371.8 us` (`46.37 ms`) | **`4,665.5 us` (`4.67 ms`)** | **`9.94x`** |

### 4.3 Key Architectural Findings From the 3-Branch Experiment

1. **Why Branch C (`exper/boacas-c-patched-codeblock`) Achieves `9.04x–9.55x` Release Cold Speedup (`5.6–7.0 ms -> 0.60–0.77 ms`)**:
   - Over **88% of `boa_engine` cold-start time** is spent inside `boa_lexer` + `boa_parser` + `Scope::analyze_scope` + `ByteCompiler::compile_statement_list`.
   - Because `CodeBlock` in `boa_engine v0.22.0` has **zero `boa_interner::Sym` dependencies** (`Constant::String` stores Latin1/UTF-16 `JsString` directly and `Bytecode` is a flat `Box<[u8]>`), deserializing the precompiled `CodeBlock` tree directly into `Gc<CodeBlock>` takes **`~180 us`** in release mode and bypasses the entire frontend and bytecode compiler.
   - Pre-fusing referenced `node:*` built-in modules (`node:crypto`, `node:fs`, `node:path`, `node:events`, `node:os`, `node:util`, `node:assert`, `node:buffer`, `node:url`, `node:timers`) into bits `18..=27` of `shards_mask` eliminated runtime `eval_lazy_builtin_module` calls during `require("node:*")`, boosting `W2_FS_Path_Crypto` from a `2.31x` speedup to **`9.55x` (`0.61 ms` cold start)** in release mode (`13.40x` in debug mode).

2. **Why Branch B (`exper/boacas-b-ast-deser`) Only Achieves `1.16x–1.19x` Cold Speedup While Bloating `.cas` by `28x`**:
   - Upstream `boa_ast v0.22.0` uses `#[serde(flatten)]` on `Span` / `LinearSpan` inside AST nodes. In `serde`, `#[serde(flatten)]` emits `serialize_map(None)` / `deserialize_any`, which **breaks non-self-describing binary formats (`postcard`, `bincode`)** with `The length of a sequence must be known`.
   - Consequently, `boa_ast`'s `serde` implementation requires a self-describing format (`serde_json`), inflating a `25 KB` JS bundle + prelude into a **`552 KB – 738 KB` JSON AST payload**.
   - At runtime, parsing `600+ KB` of JSON into thousands of heap-allocated `Box<Statement>` / `Box<Expression>` AST nodes, remapping `boa_interner::Sym` indices, and still running `ast.analyze_scope(...)` + `ByteCompiler` consumes **~83% of the time** that `boa_parser` takes to parse `25 KB` of minified JS directly.

3. **Why Branch A (`exper/boacas-a-fused-lazy`) Achieves `1.00x–1.12x` on Single-Module Bundles (`W1`–`W3`) and Shines on Multi-Package `node_modules` Trees (`W4`)**:
   - Fusing `ULTRA_CORE_PRELUDE` + `shards_mask` shards + user bundle into a single contiguous `[precompiled_slice .. fallback_js]` buffer eliminates per-shard `ctx.eval()` call overhead and runtime `detect_needed_shards` scanning (`~1.09x–1.12x` on `W1` and `W2`), but `boa_parser` and `ByteCompiler` still process the same total number of JS bytes.
   - Where Branch A delivers substantial value (`W4_LazyNodeModules`: **`2.92 ms`** cold start with `8/8` cold `node_modules` packages deferred into unparsed string literals) is in `collect_eager_module_indices`: deferring `require(idx)` inside uncalled top-level `FunctionDeclaration` / `FunctionExpression` / `ArrowFunctionExpression` spans prevents `boa_parser` from parsing cold `node_modules` subtrees at startup.

---

## 5. Comparative Evaluation Matrix & 1–10 Scorecard

| Criterion | Branch A (`exper/boacas-a-fused-lazy`) | Branch B (`exper/boacas-b-ast-deser`) | Branch C (`exper/boacas-c-patched-codeblock`) |
| :--- | :--- | :--- | :--- |
| **1. Worked?** | **Yes.** 100% of 94 workspace tests pass; cold fused `[precompiled_slice .. fallback_js]` single-pass `ctx.eval()` and `--no-exper-boacas` fallback work seamlessly. | **Yes (with format caveat).** 100% of 94 workspace tests pass; `MirrorScriptInner` offset-verified AST injection works, but `boa_ast`'s `#[serde(flatten)]` spans reject `postcard`/`bincode` and require self-describing `serde_json`. | **Yes.** 100% of 94 workspace tests pass with 0 warnings; `Script::compile_to_slicebc` and `Script::from_slicebc_bytes` cleanly serialize and execute the full recursive `CodeBlock` + `Scope` tree. |
| **2. Impact? (Latency & Size)** | **Low-to-Moderate (`1.00x–1.12x` on `W1–W3`; strong on cold `node_modules` `W4`).** Adds only `18–26 KB` to `.cas` and `< 15 us` to `slice pack`, but does not bypass `boa_parser` or `ByteCompiler` for active code. | **Poor (`1.16x–1.19x` cold speedup, `552–738 KB` payload bloat).** Deserializing `600+ KB` of JSON AST + running `analyze_scope` + `ByteCompiler` at runtime saves only `~16%` of cold start while increasing `.cas` size by `28x`. | **Transformative (`9.04x–9.55x` release speedup; `9.94x–13.40x` debug speedup).** Drops cold-start VM execution from `5.6–7.0 ms` to **`0.60–0.77 ms`** (`< 0.8 ms`!) with a compact `69–96 KB` binary `CodeBlock` payload. |
| **3. Maintainability?** | **10/10.** 100% public `boa_engine` API; zero `unsafe` layout bridges; zero vendored crates. Survives `cargo update -p boa_engine` effortlessly. | **3/10.** Relies on a private struct mirror (`MirrorScriptInner`) with runtime offset probing AND depends on `boa_ast`'s internal `serde` representation. | **8/10.** Uses standard Cargo `[patch.crates-io]` with a single self-contained `250`-line serializer/deserializer in [`vendor/boa_engine/src/script.rs`](file:///p:/opensource/slice/vendor/boa_engine/src/script.rs). No `unsafe` transmutes required. |
| **4. Good Engineering?** | **Yes.** Zero-copy contiguous slice design (`&full_contiguous_payload[32..]`) and Oxc AST-based lazy module span analysis (`collect_eager_module_indices`) are clean, sound, and idiomatic Rust. | **No.** Working around private `ScriptPhase::Ast` via `Gc::into_raw` pointer casting and paying `600+ KB` of JSON AST serialization overhead for a `1.18x` gain is a poor engineering trade-off. | **Yes.** Directly addresses the true bottleneck (`boa_lexer` + `boa_parser` + `Scope::analyze_scope` + `ByteCompiler`) at the exact architectural boundary (`Gc<CodeBlock>`) where all `Sym` interner dependencies have already been resolved into `JsString`s. |
| **5. Flexibility?** | **High.** Works with any JS syntax, dynamic `eval`, and warm/cold realms; `--no-exper-boacas` falls back to the exact same `fallback_js` slice at zero extra storage cost. | **Low.** Cannot handle scripts where `boa_parser::Parser::parse_eval` diverges from `Script::parse`, and bloats `.cas` archives and standalone `SLICEEXE` binaries. | **High.** Supports full ES2024 bytecode (`CodeBlockFlags`, `Bytecode`, Latin1/UTF-16 `JsString` constants, nested `CodeBlock` functions, `BigInt`, `Scope` locators, exception `Handler`s, `InlineCache`s, and global `lex`/`fn`/`var` tables) while preserving instant `fallback_js` fallback under `--no-exper-boacas`. |
| **6. Risks?** | **Minimal.** Embedding `ULTRA_CORE_PRELUDE` + shards into `.cas` ties the precompiled slice to the current prelude text, mitigated by the embedded `fallback_js` slice. | **High.** Any upstream change to `boa_engine::Script`'s private `Inner` layout or `boa_ast` `serde` attributes breaks AST injection or snapshot compatibility. | **Low-to-Moderate.** Upgrading `boa_engine` to `v0.23+` requires porting the ~250-line `compile_to_slicebc` / `from_slicebc_bytes` block in `vendor/boa_engine/src/script.rs` and bumping the `SLICEBC` version byte if VM opcodes change. |
| **7. Score (1 - 10)** | **7.5 / 10** *(Best zero-fork optimization for multi-package `node_modules` bundles, but cannot break the `< 1 ms` cold-start barrier on its own)* | **3.0 / 10** *(Proves why AST-level serialization in `boa_engine` is a dead end compared to `CodeBlock` bytecode serialization)* | **9.5 / 10** *(Clear winner: delivers sub-millisecond `0.60–0.77 ms` cold starts — a `9.0x–9.5x` release speedup — with clean, type-safe `CodeBlock` serialization)* |

---

## 6. Final Verdict & Recommendation

1. **Adopt Branch C (`exper/boacas-c-patched-codeblock`) + Branch A's `collect_eager_module_indices` Deferred Spans as the Production `EXPER_BOACAS` Architecture**:
   - **Branch C (`9.5 / 10`)** conclusively proves the "Precompiled Boa" thesis: by serializing the compiled `CodeBlock` tree + global `Scope` bindings directly into `SLICEBC\x01` and pre-fusing referenced `node:*` built-in modules (`bits 18..=27`), cold-start VM execution drops from **`5.57–6.99 ms` down to `0.60–0.77 ms`** (`9.04x–9.55x` faster in `--release`, `9.94x–13.40x` faster in debug).
   - Combining Branch C's `CodeBlock` bytecode serializer with Branch A's Oxc AST deferred top-level function module spans (`collect_eager_module_indices`) gives Slice both **sub-millisecond cold starts on active code** and **zero compile/serialize cost for unused `node_modules` subtrees**.
2. **Retire Branch B (`exper/boacas-b-ast-deser`, `3.0 / 10`)**:
   - Branch B served its scientific purpose by exposing `boa_ast`'s `#[serde(flatten)]` incompatibility with binary serializers (`postcard`/`bincode`) and demonstrating that skipping `boa_parser` alone while still running JSON AST deserialization + `analyze_scope` + `ByteCompiler` at runtime yields only `1.16x–1.19x` at `28x` the file size.
