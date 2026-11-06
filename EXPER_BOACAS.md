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

## 4. Comparative Evaluation Matrix (To Be Completed After Branch Benchmarks)

Each branch will be evaluated and scored (`1–10`) across the 7 engineering dimensions below:

| Criterion | Branch A (`exper/boacas-a-fused-lazy`) | Branch B (`exper/boacas-b-ast-deser`) | Branch C (`exper/boacas-c-patched-codeblock`) |
| :--- | :--- | :--- | :--- |
| **1. Worked?** | Pending | Pending | Pending |
| **2. Impact? (Latency & Size)** | Pending | Pending | Pending |
| **3. Maintainability?** | Pending | Pending | Pending |
| **4. Good Engineering?** | Pending | Pending | Pending |
| **5. Flexibility?** | Pending | Pending | Pending |
| **6. Risks?** | Pending | Pending | Pending |
| **7. Score (1 - 10)** | Pending | Pending | Pending |
