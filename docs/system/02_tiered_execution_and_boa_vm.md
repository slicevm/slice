# `slice-engine`: Hybrid Three-Tier Execution Pipeline, `WarmBoaRealm` & Native Intrinsics

[`crates/slice-engine`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs) is designed around a single principle: **use the lightest execution tier capable of running the workload, pay `0.00 ns` and `0 B` for unused Web/Node globals via compile-time minified lazy prelude shards, and accelerate hot standard-library primitives in native Rust**.

---

## 1. Default Tier-1 `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS & Zero-Copy Framing (`0.02 us - 0.21 us` Instantiation)

- **The Problem Eliminated:**
  1. Wrapping bundled JS inside a JSON-serialized `BytecodeModule` (`SLICESNP`) required `serde_json::from_slice` to allocate and unescape a 30 KB - 130 KB `String` on every isolate run (`~229 microseconds`).
  2. Even with raw UTF-8 `SLICEBOA` framing, `boa_engine::Context::eval` had to run the full ECMAScript lexer, recursive-descent AST parser, `ScopeAnalyzer` identifier resolution, and `ByteCompiler` on `ULTRA_CORE_PRELUDE`, needed feature shards, `node:*` built-in scripts, and the user's bundled JS on every cold process launch (`~4.5 ms - 12 ms` of pure parser overhead).
- **Default `SLICEBC\x01` Precompiled `CodeBlock` Architecture ([`crates/slice-engine/src/boacas.rs`](file:///p:/opensource/slice/crates/slice-engine/src/boacas.rs) & [`vendor/boa_engine/SLICE_PATCH.md`](file:///p:/opensource/slice/vendor/boa_engine/SLICE_PATCH.md)):**
  - By default (`boacas = true`), [`compile_source_to_bytecode`](file:///p:/opensource/slice/crates/slice-engine/src/parser_and_compiler.rs) calls [`pack_boacas_container`](file:///p:/opensource/slice/crates/slice-engine/src/boacas.rs), which emits a **32-byte `BoaCasHeader` (`b"SLICEBC\x01"`)** containing:
    - `feature_flags: u16` (replacing all 18 runtime substring scans with a single build-time bitmask)
    - `shards_mask: u32` (precomputed Aho-Corasick `detect_needed_shards` bitmask)
    - `rc_key: u64` + `rc_key_hex: [u8; 16]` (pre-extracted persistent realm cache fingerprint)
    - `precompiled_len: u32` (`SLICEBC1` binary `CodeBlock` payload) + `fallback_js_len: u32`
  - **Pre-Fused Prelude + Shards + `node:*` Built-Ins:** At build time, `pack_boacas_container` fuses `ULTRA_CORE_PRELUDE`, all required runtime shards (`1..=17`), and all statically referenced `node:*` built-in modules (`globalThis.__slice_nb[...]`) directly with the user script into a **single unified `Gc<CodeBlock>`**, serialized via [`boa_engine::slice_precompile::compile_js_to_codeblock_bytes_with_deferred`](file:///p:/opensource/slice/vendor/boa_engine/src/slice_precompile.rs).
  - **Oxc AST Deferred Function Module Spans (`0x11` Tag):** For multi-module bundles (`var __slice_modules = [function(module, exports, require){...}]`), [`extract_deferred_module_function_spans_oxc`](file:///p:/opensource/slice/crates/slice-engine/src/boacas.rs) locates top-level module factory functions via `oxc::parser::Parser` and records them as 16-byte deferred spans (`0x11` tag) in `SLICEBC1`. Uncalled `node_modules` functions are never byte-compiled until invoked at runtime via Boa's `Call`/`New` VM dispatch hook ([`vendor/boa_engine/src/vm/opcode/call/mod.rs`](file:///p:/opensource/slice/vendor/boa_engine/src/vm/opcode/call/mod.rs)).
  - **Zero-Parse Runtime Execution:** At runtime, [`SliceIsolate::exec_boacas_vm`](file:///p:/opensource/slice/crates/slice-engine/src/isolate.rs) creates a bare `boa_engine::Context` (`create_bare_boa_realm`), registers only the required native Rust host functions in a single pass, deserializes the `Gc<CodeBlock>` directly from the `mmap` slice, and executes it on the VM — eliminating **100% of runtime JS lexing and parsing** (`5.11 ms` cold / `0.21 ms` warm on `01-fintech-checkout`, `13.11 ms` cold / `0.56 ms` warm on `01-hono-edge-microservice`).
  - **Legacy `SLICEBOA` Source-in-CAS Fallback (`--no-boacas`):** If `--no-boacas` (or `// @slice-boacas: false` / `SLICE_BOACAS=0` / `[engine] boacas = false`) is set, `compile_source_to_bytecode` emits the 8-byte `b"SLICEBOA"` header followed by raw UTF-8 transpiled JS.

---

## 2. Pre-Warmed `Boa` Context Pool (`BOA_CONTEXT_POOL`), `__slice_rc` Persistent Realm Cache, & Sharded Preludes

### 2.1 Pre-Warmed `WarmBoaRealm` Pool ([`crates/slice-engine/src/boa_realm.rs`](file:///p:/opensource/slice/crates/slice-engine/src/boa_realm.rs))
1. Maintains a thread-local pool of pre-initialized `WarmBoaRealm` instances (`SliceIsolate::prewarm_boa_pool(count)`), wrapped in `std::mem::ManuallyDrop<Vec<WarmBoaRealm>>` so thread exit on Windows never runs `boa_engine::Context::drop` after `boa_gc`'s TLS collector has already torn down.
2. Wraps executed scripts inside a lexical block (`{\n<js_code>\n}`) in [`exec_boa_full_es_vm`](file:///p:/opensource/slice/crates/slice-engine/src/isolate.rs) so top-level `const`, `let`, and `class` declarations are scoped to that invocation and never collide when the warm context is reused across requests in `slice serve` or `slice run --warm`.
3. Routes `console.log` directly through a native Rust `__slice_console_log` host function writing straight into thread-local `BOA_STDOUT`.

### 2.2 Persistent Realm Module Cache (`__slice_rc`) & Direct `JsObject::call`
- [`bundle_entrypoint_with_deps`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs) emits a 16-hex-char BLAKE3 fingerprint header (`//__SLICE_RC:<16_hex_chars>`) and caches evaluated `node_modules` exports (`__slice_cache`) plus the compiled entrypoint closure in `globalThis.__slice_rc`.
- On warm hits (`slice serve` or `slice run --warm`), [`WarmBoaRealm::loaded_bundles`](file:///p:/opensource/slice/crates/slice-engine/src/boa_realm.rs) stores the compiled `boa_engine::JsObject` entrypoint function handle and invokes `fn_obj.call(...)` directly in Rust with **0 JS parser passes** and **0 `node_modules` CJS factory re-evaluations**, cutting warm isolate execution down to **`272 us` (`0.27 ms`)** on `02-npm-ecosystem-suite`, **`600 us` (`0.60 ms`)** on `02-itty-router-unjs-nitro-stack`, and **`900 us` (`0.90 ms`)** on `01-hono-edge-microservice`.

### 2.3 Compile-Time Minified 17-Shard Surgical Preludes (`00_ultra_core.js` + Shards `1..=17`)
- Implemented in [`crates/slice-engine/build.rs`](file:///p:/opensource/slice/crates/slice-engine/build.rs), [`crates/slice-engine/src/js_preludes.rs`](file:///p:/opensource/slice/crates/slice-engine/src/js_preludes.rs), and [`crates/slice-engine/src/boa_realm.rs`](file:///p:/opensource/slice/crates/slice-engine/src/boa_realm.rs) (`WarmBoaRealm::loaded_shards_mask: u32`):
  - Instead of evaluating a monolithic ~48 KB JS prelude on every cold process launch, `00_ultra_core.js` (`~6 KB` minified, preserving native Boa `Date.now` and `Math.random` fast paths unless `--deterministic` is active) registers self-replacing `Object.defineProperty` lazy getters (`__defLazy(names, shardId)`) on `globalThis` across **17 orthogonal sub-shards**:
    - **Shard 1 (`01_web_fetch_url.js`):** `URL` and `URLSearchParams` (backed by native Rust [`web_primitives.rs`](file:///p:/opensource/slice/crates/slice-engine/src/web_primitives.rs)).
    - **Shard 2 (`02_encoding_events.js`):** `TextEncoder`, `TextDecoder`, `btoa`, `atob`.
    - **Shard 3 (`03_streams_formdata.js`):** `ReadableStream`.
    - **Shard 4 (`04_webcrypto_clone.js`):** `crypto`, `CryptoKey`, `crypto.subtle`, `randomUUID`, `getRandomValues`.
    - **Shard 5 (`05_full_buffer.js`):** Core `Uint8Array`-backed `Buffer` (`from`, `alloc`, `concat`, `byteLength`, `compare`, `toString`, `subarray`, `equals`) with lazy Shard 17 prototype forwarders.
    - **Shard 6 (`06_console_ext.js`):** `AbortController`, `AbortSignal` (`timeout`, `any`, `abort`), `DOMException`.
    - **Shard 7 (`07_websocket.js`):** `WebSocket`, `MessageEvent`, `CloseEvent`.
    - **Shard 8 (`08_workers_sockets.js`):** `Worker`, `MessageChannel`, `MessagePort`, `Slice.connect`.
    - **Shard 9 (`09_wasm.js`):** `WebAssembly` (`Module`, `Instance`, `Memory`, `Table`, `Global`, `compile`, `instantiate`).
    - **Shard 10 (`10_outbound_fetch_sse.js`):** `fetch`, `EventSource`, `Slice.streamSSE`, `Response.sse`.
    - **Shard 11 (`11_fetch_headers_req_res.js`):** `Headers` (with `_isHeaders` fast copy, O(1) lowercase `get()`, and `getSetCookie()`), `Request`, `Response`, `__slice_make_web_request`.
    - **Shard 12 (`12_url_pattern.js`):** `URLPattern`.
    - **Shard 13 (`13_dom_events.js`):** `Event`, `CustomEvent`, `EventTarget`.
    - **Shard 14 (`14_transform_compression_streams.js`):** `WritableStream`, `TransformStream`, `TextEncoderStream`, `TextDecoderStream`, `CompressionStream`, `DecompressionStream`.
    - **Shard 15 (`15_blob_formdata.js`):** `Blob`, `File`, `FormData`.
    - **Shard 16 (`16_structured_clone.js`):** `structuredClone`.
    - **Shard 17 (`17_buffer_binary_rw.js`):** `Buffer.prototype` binary integer readers/writers (`readUInt*`, `readInt*`, `readBig*`, `writeUInt*`, `writeInt*`, `writeBig*`) and slice mutators (`copy`, `fill`, `write`, `compare`, `toJSON`).

---

## 3. Native Rust Fast-Path Intrinsics (`web_primitives`, `SHA-NI`, `SemVer`, `Base64URL`, `DEFLATE`/`Gzip`, `AsyncLocalStorage`, & In-Memory VFS Slab)

Implemented in [`crates/slice-engine/src/web_primitives.rs`](file:///p:/opensource/slice/crates/slice-engine/src/web_primitives.rs), [`crates/slice-engine/src/intrinsics.rs`](file:///p:/opensource/slice/crates/slice-engine/src/intrinsics.rs), [`crates/slice-engine/src/boa_realm.rs`](file:///p:/opensource/slice/crates/slice-engine/src/boa_realm.rs), and [`crates/slice-engine/src/host_bindings.rs`](file:///p:/opensource/slice/crates/slice-engine/src/host_bindings.rs):

0. **Native Rust WHATWG `URL` & `URLSearchParams` Primitives ([`crates/slice-engine/src/web_primitives.rs`](file:///p:/opensource/slice/crates/slice-engine/src/web_primitives.rs)):**
   - Provides [`parse_whatwg_url`](file:///p:/opensource/slice/crates/slice-engine/src/web_primitives.rs), [`can_parse_whatwg_url`](file:///p:/opensource/slice/crates/slice-engine/src/web_primitives.rs), [`normalize_url_pathname`](file:///p:/opensource/slice/crates/slice-engine/src/web_primitives.rs), [`usp_encode_component`](file:///p:/opensource/slice/crates/slice-engine/src/web_primitives.rs), [`usp_decode_component`](file:///p:/opensource/slice/crates/slice-engine/src/web_primitives.rs), and [`usp_parse_query`](file:///p:/opensource/slice/crates/slice-engine/src/web_primitives.rs), exposed to Shard 1 via `__slice_url_can_parse`, `__slice_usp_encode`, and `__slice_usp_decode`.

1. **Hardware `SHA-NI` Block Compression ([`Sha256State::compress_block_sha_ni`](file:///p:/opensource/slice/crates/slice-engine/src/intrinsics.rs)):**
   - Uses Intel/AMD `SHA-NI` instructions (`_mm_sha256rnds2_epu32`, `_mm_sha256msg1_epu32`, `_mm_sha256msg2_epu32`) with cached `AtomicU8` runtime feature detection (`sha,sse4.1,ssse3`) and automatic fallback to [`Sha256State::compress_block_soft`](file:///p:/opensource/slice/crates/slice-engine/src/intrinsics.rs).
2. **Pure-Rust FIPS-180-4 SHA-1/256/384/512, RFC-2104 HMAC & Base64/Base64URL:**
   - Direct `JsArrayBuffer` / `JsUint8Array` zero-copy Rust host functions (`__slice_crypto_sha256`, `__slice_crypto_hmac_sha256`, `__slice_crypto_digest`, `__slice_crypto_hmac_sign`, `__slice_base64_encode`, `__slice_base64_decode`).
3. **Self-Contained RFC 1951 (`deflate-raw`), RFC 1950 (`deflate`), & RFC 1952 (`gzip`) Compression Engine:**
   - Implemented in [`zlib_compress_bytes`](file:///p:/opensource/slice/crates/slice-engine/src/intrinsics.rs) and [`zlib_decompress_bytes`](file:///p:/opensource/slice/crates/slice-engine/src/intrinsics.rs) with LZ77 32 KB sliding-window compression, Stored/Fixed/Dynamic Huffman decompression, Adler-32, and CRC-32 validation, powering both WinterTC `CompressionStream` / `DecompressionStream` and `node:zlib` with zero external crate dependencies.
4. **Zero-Overhead `SliceJobExecutor` for `node:async_hooks` (`AsyncLocalStorage` & `AsyncResource`):**
   - Implemented in [`SliceJobExecutor`](file:///p:/opensource/slice/crates/slice-engine/src/boa_realm.rs) and [`event_loop.rs`](file:///p:/opensource/slice/crates/slice-engine/src/event_loop.rs): when `BOA_ALS_ACTIVE` is `false`, promise microtasks execute with zero snapshot overhead; when `AsyncLocalStorage` is active, `SliceJobExecutor` and the event loop capture and restore `BOA_ALS_SNAP` across every `await`, `.then()`, `Promise.all`, `setTimeout`, `setImmediate`, `queueMicrotask`, `process.nextTick`, and `fetch` boundary.
5. **Zero-Regex Native Rust SemVer & Utility Scanners:**
   - Implemented in [`semver_parse_strict_xyz`](file:///p:/opensource/slice/crates/slice-engine/src/intrinsics.rs), [`semver_coerce_xyz`](file:///p:/opensource/slice/crates/slice-engine/src/intrinsics.rs), and [`semver_satisfies_xyz`](file:///p:/opensource/slice/crates/slice-engine/src/intrinsics.rs) (~150 ns per check).
6. **Thread-Local Binary-Safe In-Memory VFS Slab Cache (`BOA_VFS_CACHE` & `BOA_VFS_DIRS`):**
   - Ephemeral files created during an isolate run (`writeFileSync`, `appendFileSync`, `mkdirSync`) are stored as raw `Vec<u8>` byte slabs in thread-local memory (`FxHashMap<String, Vec<u8>>`) and served in nanoseconds on subsequent `statSync` (with real OS `mtimeMs`), `readdirSync`, and binary/UTF-8 `readFileSync` calls, while falling back transparently to `std::fs` for pre-existing host files.
7. **Lazy On-Demand Host Binding Registration by Category:**
   - Cold `create_warm_boa_realm()` initialization registers only the 19 ultra-core host functions (`__slice_console_log`, `__slice_load_builtin`, `__slice_lazy_prelude`, timers, env, SHA-256, Base64, UTF-8), while specialized host bindings (`register_http_bridge_host_functions`, `register_kv_and_sandbox_host_functions`, `register_semver_and_fast_intrinsics_host_functions`, `register_wintertc_phase_n3_host_functions`, `register_async_hooks_host_functions`) are registered lazily only when their corresponding shard or built-in module is loaded.

---

## 4. Tier-0 (`SLICESNP`) and Tier-2 Unboxed 64-Bit Numeric Register Compiler & VM ([`numeric_vm.rs`](file:///p:/opensource/slice/crates/slice-engine/src/numeric_vm.rs))

- **Zero Name Heuristics:** Every JS/TS function (`Stmt::FuncDecl`, `Expr::ArrowFunc`, and function expressions) is analyzed structurally by [`try_compile_numeric_func`](file:///p:/opensource/slice/crates/slice-engine/src/numeric_vm.rs) regardless of function or parameter names.
- **Unboxed 32-Register Execution (`[f64; 32]`):** Pure numeric functions compile into 8-byte/16-byte [`NumericOp`](file:///p:/opensource/slice/crates/slice-engine/src/numeric_vm.rs) instructions (`LoadConst`, `Move`, `BinaryReg`, `BinaryImm`, `UnaryReg`, `MathIntrinsic`, `Jump`, `JumpIfFalse`, `CallSelf1`, `CallSelf2`, `LinearRecurrence2`, `Return`). Execution inside [`NumericFunc::execute`](file:///p:/opensource/slice/crates/slice-engine/src/numeric_vm.rs) operates entirely on unboxed `f64` and `i64`/`i32` bitwise values on the CPU stack (`256 bytes`, fitting in 4 CPU L1 cache lines) with **0 heap allocations** and **0 `FxHashMap` variable lookups**.
- **Structural Linear Recurrence Lowering ([`detect_linear_recurrence_ast`](file:///p:/opensource/slice/crates/slice-engine/src/numeric_vm.rs)):** Recognizes any 1-parameter self-recursive function matching `if (param <= 1) return ...; return c1 * f(param - 1) + c2 * f(param - 2);` (including unweighted Fibonacci-style and weighted Pell/Lucas-style recurrences under any identifier name) and lowers it to `NumericOp::LinearRecurrence2`, reducing $O(2^n)$ call trees to an $O(n)$ 2-register loop.
- **Amortized Fuel Metering:** Checks `(local_steps & 255) == 0` every 256 register instructions (and at every recursive call and loop recurrence), guaranteeing deterministic `SandboxViolation::CpuFuelExhausted` preemption on infinite numeric loops with `< 0.4%` branch overhead.

