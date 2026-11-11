# Slice (Nexus-JS): System Architecture & Internal Specification Index (`v0.10.12`)

Welcome to the **Slice (Nexus-JS)** System Architecture and Internal Specification (`docs/system/`). Engineered in pure Rust (`edition = 2021`, `opt-level = 3`, `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`), Slice replaces the legacy POSIX + monolithic V8/JSC assumptions of Node.js, Deno, and Bun with four orthogonal, tightly integrated Rust crates:

1. **[`slice-cas`](../../crates/slice-cas/src/lib.rs)** - Zero-Stat Content-Addressed Storage (`SLICE_CAS` v1), Oxc (`v0.152.0`) AOT TypeScript/TSX Compiler, Oxc Semantic/AST Dead-Export Tree-Shaker, & `SLICEEXE` Standalone Executable Linker.
2. **[`slice-io`](../../crates/slice-io/src/lib.rs)** - Direct Kernel Completion Ring Engine (Linux `io_uring` `IORING_SETUP_SQPOLL` + Windows NT `IOCP` FFI + `64 KB` Page-Aligned Fixed Slab Buffers).
3. **[`slice-sandbox`](../../crates/slice-sandbox/src/lib.rs)** - MicroVM-Grade Object Capabilities (`FsCap`, `NetCap`, `EnvCap`, `ExecCap`), Dynamic SES Intrinsic Lockdown, Deterministic CPU Fuel & Byte Cage, & Linux `x86_64` Hardware `WRPKRU` Memory Protection Keys (MPK).
4. **[`slice-engine`](../../crates/slice-engine/src/lib.rs)** - Hybrid Three-Tier Execution Core: Tier-1 Default `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS ([`vm/boacas.rs`](../../crates/slice-engine/src/vm/boacas.rs), [`vendor/boa_engine/SLICE_PATCH.md`](../../vendor/boa_engine/SLICE_PATCH.md)) + Pre-Warmed `boa_engine` Pool + Native Rust Host Intrinsics, Tier-0 `SLICESNP` Snapshot VM, & Tier-2 Unboxed 64-Bit Numeric Register VM ([`vm/numeric_vm.rs`](../../crates/slice-engine/src/vm/numeric_vm.rs)).

For user-facing documentation and guides, see **[`docs/README.md`](../README.md)** and **[`docs/general/09_benchmarks_testing_and_docker_suite.md`](../general/09_benchmarks_testing_and_docker_suite.md)**. For empirical cross-runtime benchmark numbers, see **[`tests/reports/latest.md`](../../tests/reports/latest.md)**.

---

## 1. System Architecture Map (`docs/system/`)

| Document | Subsystem / Scope | Key Source Modules Covered |
| :--- | :--- | :--- |
| **[`00_index.md`](./00_index.md)** | Executive Overview, 4-Crate Topology, Crate & Module Map, & Legacy Runtime Bottlenecks | Workspace Overview |
| **[`01_cas_bundler_and_oxc_compiler.md`](./01_cas_bundler_and_oxc_compiler.md)** | Zero-Stat `SLICE_CAS` v1 Binary Format, Oxc `v0.152.0` TS/TSX Compiler, Semantic Lowering, Oxc AST Array Pipeline Fusion, ESM-First `"exports"`, Cold-Method `require` Inliner, Reverse-Topological Oxc Semantic/AST Tree-Shaker, & Single-Copy `SLICEBC\x01`/`SLICEBOA` CAS Deduplication | `crates/slice-cas/src/{lib.rs,archive/,bundler/,compiler/,linker/}` |
| **[`02_tiered_execution_and_boa_vm.md`](./02_tiered_execution_and_boa_vm.md)** | Hybrid Three-Tier Execution Pipeline (`SLICEBC\x01` Precompiled Boa `CodeBlock` CAS Default, `SLICESNP`, Legacy `SLICEBOA`, `WarmBoaRealm` Pool, Persistent Realm Cache `__slice_rc`, 32-Bit `shards_mask` Preludes `00..19` + Pre-Fused `node:*` Bits `19..28`, Hardware `SHA-NI` & Native Intrinsics, & Tier-2 Unboxed 64-Bit Numeric Register VM) | `crates/slice-engine/src/{vm/,isolate/,runtime/,preludes/,intrinsics/,bindings/}`, `vendor/boa_engine/SLICE_PATCH.md` |
| **[`03_security_sandbox_ocap_and_mpk.md`](./03_security_sandbox_ocap_and_mpk.md)** | 5-Ring Zero-Trust Security Architecture: OCap `CapabilityToken` (`FsCap`/`NetCap`/`EnvCap`/`ExecCap`), Dynamic SES Prototype Integrity (`__slice_check_ses`), CPU Fuel & Memory Cage, Linux `WRPKRU` Hardware MPK, Pre-Connect DNS SSRF Firewall, & Outbound Egress Secret DLP | `crates/slice-sandbox/src/{lib.rs,compartment/,manifest/,net/,vfs/}`, `crates/slice-engine/src/{isolate/,net/outbound_http.rs}` |
| **[`04_networking_wintertc_and_event_loop.md`](./04_networking_wintertc_and_event_loop.md)** | Multi-Worker `slice serve`, 3-Level Async Concurrency Architecture, Outbound HTTP/1.1 & TLS 1.3 (`rustls` + `OUTBOUND_CONN_POOL`), Deterministic Time-Travel Event Loop, RFC 6455 WebSocket Engine, Raw TCP/TLS/UDP Sockets, & `Uint8Array` `Buffer` + Node Core Tier-1 | `crates/slice-cli/src/server/`, `crates/slice-engine/src/{runtime/event_loop.rs,net/,preludes/node_builtins.rs}` |
| **[`05_wasm_workers_and_kernel_io.md`](./05_wasm_workers_and_kernel_io.md)** | Pure-Rust WebAssembly (`wasmi` `v2.0.0`, `GLOBAL_WASM_MODULE_CACHE`, 5-Ring WASM Cage), Capability-Attenuated MicroVM Web Workers & `node:worker_threads` (`IDLE_WORKER_POOL`), Direct Kernel Completion Rings (`io_uring` & `IOCP`), & OS-Native `notify` File Watcher | `crates/slice-engine/src/runtime/{wasm,workers}.rs`, `crates/slice-io/src/lib.rs`, `crates/slice-cli/src/config/` |
| **[`06_edge_kv_ai_sandbox_and_mcp.md`](./06_edge_kv_ai_sandbox_and_mcp.md)** | 16-Way Sharded `Slice.kv` (`EDGE_KV_SLAB`), Multi-Tenant `--tenant-dir` Atomic Hot-Swap, SplitMix64 `--deterministic` Replay, AI Agent "Code Mode" (`Slice.evalSandbox`, `< 5 us` `Slice.checkpoint`/`restore`), Time-Travel `slice test`, & Built-In MCP Server (`slice mcp`) | `crates/slice-engine/src/isolate/state_and_kv.rs`, `crates/slice-cli/src/{server/mod.rs,commands/test_runner.rs,diagnostics/}` |
| **[`docs/README.md`](../README.md)** | Complete Documentation Hub (`docs/general/`, `docs/guides/`, & `docs/system/`) | User Guides, CLI Reference, & Compatibility Matrix |
| **[`tests/reports/latest.md`](../../tests/reports/latest.md)** | Verified Cross-Runtime Linux `x86_64` Docker Benchmark Report (Slice vs Bun, Deno, and Node.js across all 33 workloads) & **[`09_benchmarks_testing_and_docker_suite.md`](../general/09_benchmarks_testing_and_docker_suite.md)** | `benches/`, `tests/apps/`, `tests/frameworks/`, `tests/case_studies/` |
| **[`vendor/boa_engine/SLICE_PATCH.md`](../../vendor/boa_engine/SLICE_PATCH.md)** | `SLCB\x01` Precompiled `CodeBlock` Binary Serializer/Deserializer, Monomorphic Inline Cache, & Direct ECMA-404 JSON Fast Paths | `vendor/boa_engine/src/{script.rs,builtins/json/mod.rs,vm/inline_cache/mod.rs,vm/runtime_limits.rs}` |

---

## 2. High-Level 4-Crate Host Topology

```
+---------------------------------------------------------------------------------------------------+
|                                   Slice (Nexus-JS) v0.10.12 Host                                    |
|                                                                                                   |
|  +---------------------------------------------------------------------------------------------+  |
|  |                    slice-sandbox: Zero-Trust OCap, SES & Hardware Boundary                  |  |
|  |  +----------------------------------------+   +------------------------------------------+  |  |
|  |  | Tenant Compartment A                   |   | Tenant Compartment B                     |  |  |
|  |  | - SES Frozen Intrinsics (lockdown)     |   | - Pure Compute (Zero Ambient Authority)  |  |  |
|  |  | - Scoped FsCap / NetCap / EnvCap       |   | - Deterministic CPU Fuel & Byte Cage     |  |  |
|  |  | - Linux SYS_pkey_alloc + WRPKRU Domain |   | - Linux SYS_pkey_alloc + WRPKRU Domain   |  |  |
|  |  +----------------------------------------+   +------------------------------------------+  |  |
|  +---------------------------------------------------------------------------------------------+  |
|                                                                                                   |
|  +---------------------------------------------------------------------------------------------+  |
|  |                    slice-engine: Hybrid Three-Tier Execution Pipeline                       |  |
|  |  +--------------------------+  +-------------------------------+  +----------------------+  |  |
|  |  | Tier 0: SLICESNP VM      |  | Tier 1: Zero-Copy SLICEBC\x01 |  | Tier 2: Peak Native  |  |  |
|  |  | - Sub-Microsecond Boot   |  | - Precompiled Boa CodeBlock   |  | - 32-Reg Numeric VM  |  |  |
|  |  | - Pre-Parsed Opcode Ops  |  | - Pre-Warmed Boa Realm Pool   |  | - Recurrence Lowering|  |  |
|  |  | - WinterTC Edge Routing  |  | - Native Rust Host Intrinsics |  | - Zero Heap Allocs   |  |  |
|  |  +--------------------------+  +-------------------------------+  +----------------------+  |  |
|  +---------------------------------------------------------------------------------------------+  |
|                                                                                                   |
|  +---------------------------------------------------------------------------------------------+  |
|  |                    slice-cas: Zero-Stat Content-Addressed Storage & Bundler                 |  |
|  |  Oxc v0.152 TS Compiler -> Oxc Semantic/AST Tree-Shaker -> BLAKE3-256 -> 64B Aligned CAS    |  |
|  +---------------------------------------------------------------------------------------------+  |
|                                                                                                   |
|  +---------------------------------------------------------------------------------------------+  |
|  |                    slice-io: Direct Kernel Completion Ring Engine                           |  |
|  |  Linux io_uring (IORING_SETUP_SQPOLL + 64KB Fixed Slab Buffers) | Windows NT IOCP FFI       |  |
|  +---------------------------------------------------------------------------------------------+  |
+---------------------------------------------------------------------------------------------------+
```

---

## 3. Complete Workspace Crate & Module Map

- **`crates/slice-cas/src/`**:
  - [`lib.rs`](../../crates/slice-cas/src/lib.rs) & [`archive/`](../../crates/slice-cas/src/archive/mod.rs) ([`builder.rs`](../../crates/slice-cas/src/archive/builder.rs), [`reader.rs`](../../crates/slice-cas/src/archive/reader.rs), [`static_assets.rs`](../../crates/slice-cas/src/archive/static_assets.rs)): 64-byte cache-line aligned `SLICE_CAS` v1 binary format, `CasBuilder` (with single-copy `SLICEBC\x01`/`SLICEBOA` source deduplication), `MappedCasArchive`, `SLICEEXE` standalone executable trailer embedding, and zero-stat `StaticAssetTable` with pre-computed `BLAKE3` `ETag` headers.
  - [`compiler/`](../../crates/slice-cas/src/compiler/mod.rs) ([`transpiler.rs`](../../crates/slice-cas/src/compiler/transpiler.rs), [`semantics.rs`](../../crates/slice-cas/src/compiler/semantics.rs), [`pipeline_fusion.rs`](../../crates/slice-cas/src/compiler/pipeline_fusion.rs)): Native Oxc (`v0.152.0`) TypeScript and `.tsx`/`.jsx` transformation (`SourceType::tsx()`, `transform_options.jsx.jsx_plugin = true`), semantic interface analysis (`analyze_typescript_semantics`), and Oxc AST array pipeline loop fusion (`fuse_array_pipelines_ast`).
  - [`linker/`](../../crates/slice-cas/src/linker/mod.rs) ([`esm_transform.rs`](../../crates/slice-cas/src/linker/esm_transform.rs), [`cjs_rewrites.rs`](../../crates/slice-cas/src/linker/cjs_rewrites.rs), [`lazy_and_inline.rs`](../../crates/slice-cas/src/linker/lazy_and_inline.rs), [`tree_shaker.rs`](../../crates/slice-cas/src/linker/tree_shaker.rs), [`ast_walker.rs`](../../crates/slice-cas/src/linker/ast_walker.rs), [`builtins_and_ids.rs`](../../crates/slice-cas/src/linker/builtins_and_ids.rs)): Oxc AST-driven ESM-to-CJS module linker (`try_transform_esm_to_cjs_ast`), Cold-Method `require` Inliner (`inline_cold_method_requires_ast`), Reverse-Topological Oxc Semantic/AST Dead-Export Tree-Shaker (`analyze_module_used_exports` + `prune_and_minify_cjs_module_ast`), and Eager-Edge BFS Lazy Module Stringifier (`collect_eager_module_indices`).
  - [`bundler/`](../../crates/slice-cas/src/bundler/mod.rs) ([`graph_bundler.rs`](../../crates/slice-cas/src/bundler/graph_bundler.rs), [`resolver.rs`](../../crates/slice-cas/src/bundler/resolver.rs), [`source_map.rs`](../../crates/slice-cas/src/bundler/source_map.rs)): Multi-pass dependency graph bundler (`bundle_entrypoint_with_deps`), project entrypoint discovery (`.ts`, `.tsx`, `.js`, `.jsx`), ESM-first `package.json` conditional `"exports"` / `"imports"` and `tsconfig.json` `"paths"` resolution, zero-stat `.wasm` binary bundling (`__slice_b64ToU8`), built-in `"slice:jsx"` fallback, and source-line error mapping (`map_bundled_error_location`).
- **`crates/slice-io/src/`**:
  - [`lib.rs`](../../crates/slice-io/src/lib.rs): Direct kernel completion ring engine (`CompletionRingEngine` over Linux `io_uring` `IORING_SETUP_SQPOLL` and Windows NT `IOCP`) and `RegisteredBufferPool` 64 KB page-aligned fixed slab buffers.
- **`crates/slice-sandbox/src/`**:
  - [`lib.rs`](../../crates/slice-sandbox/src/lib.rs) & [`compartment/`](../../crates/slice-sandbox/src/compartment/mod.rs) ([`pkey.rs`](../../crates/slice-sandbox/src/compartment/pkey.rs)): `TenantCompartment`, `CapabilityToken`, `IntrinsicRegistry::lockdown`, deterministic CPU fuel & memory byte cage, and Linux `x86_64` `HardwareProtectionDomain` (`SYS_pkey_alloc` + `WRPKRU`).
  - [`manifest/`](../../crates/slice-sandbox/src/manifest/mod.rs) ([`directives.rs`](../../crates/slice-sandbox/src/manifest/directives.rs), [`dotenv.rs`](../../crates/slice-sandbox/src/manifest/dotenv.rs)): `SliceManifest` (`slice.toml`), inline `// @slice-*` directive parser (`parse_inline_directives`), and `.env` loader with `is_sensitive_env_key` Auto-DLP secret classification.
  - [`vfs/mod.rs`](../../crates/slice-sandbox/src/vfs/mod.rs) & [`net/mod.rs`](../../crates/slice-sandbox/src/net/mod.rs): Path-traversal-safe `FsCap` (`verify_fs_read`, `verify_fs_write`), `ExecCap` (`verify_exec_binary`), `NetCap` (`verify_net_connect`), and cloud metadata / private IP SSRF classifiers (`is_cloud_metadata_ip`, `is_private_or_loopback_ip`).
- **`crates/slice-engine/src/`**:
  - [`vm/`](../../crates/slice-engine/src/vm/mod.rs) ([`boacas.rs`](../../crates/slice-engine/src/vm/boacas.rs), [`value_and_ast.rs`](../../crates/slice-engine/src/vm/value_and_ast.rs), [`numeric_vm.rs`](../../crates/slice-engine/src/vm/numeric_vm.rs), [`parser_and_compiler.rs`](../../crates/slice-engine/src/vm/parser_and_compiler.rs)): Default `SLICEBC\x01` precompiled Boa `CodeBlock` CAS container (`BoaCasHeader`, `pack_boacas_container`, `eval_boacas_on_realm` backed by [`vendor/boa_engine/src/script.rs`](../../vendor/boa_engine/src/script.rs)), 16-byte `JsValue` & `BytecodeOp`, Tier-2 Unboxed 64-Bit Numeric Register Compiler & VM (`try_compile_numeric_func`, `detect_linear_recurrence_ast`, `NumericFunc::execute`), and `SLICESNP`/`SLICEBOA` compiler.
  - [`isolate/`](../../crates/slice-engine/src/isolate/mod.rs) ([`exec_bytecode.rs`](../../crates/slice-engine/src/isolate/exec_bytecode.rs), [`exec_boa.rs`](../../crates/slice-engine/src/isolate/exec_boa.rs), [`exec_ast.rs`](../../crates/slice-engine/src/isolate/exec_ast.rs), [`http_and_ws.rs`](../../crates/slice-engine/src/isolate/http_and_ws.rs), [`kv_and_sandbox.rs`](../../crates/slice-engine/src/isolate/kv_and_sandbox.rs), [`state_and_kv.rs`](../../crates/slice-engine/src/isolate/state_and_kv.rs)): `SliceIsolate` lifecycle, Tier-0/1/2 dispatch, HTTP/WebSocket dispatch, nested `eval_sandbox`, 16-way sharded `EDGE_KV_SLAB`, SplitMix64 deterministic PRNG, Egress Secret DLP, and `< 5 us` VFS+KV checkpoint/restore.
  - [`runtime/`](../../crates/slice-engine/src/runtime/mod.rs) ([`boa_realm.rs`](../../crates/slice-engine/src/runtime/boa_realm.rs), [`event_loop.rs`](../../crates/slice-engine/src/runtime/event_loop.rs), [`wasm.rs`](../../crates/slice-engine/src/runtime/wasm.rs), [`workers.rs`](../../crates/slice-engine/src/runtime/workers.rs)): `BOA_CONTEXT_POOL` & `WarmBoaRealm` lifecycle, Deterministic Time-Travel Event Loop & `FetchReactorPool`, pure-Rust `WebAssembly` (`wasmi` `v2.0.0`) engine with `GLOBAL_WASM_MODULE_CACHE`, and capability-attenuated MicroVM Web Workers (`IDLE_WORKER_POOL`).
  - [`preludes/`](../../crates/slice-engine/src/preludes/mod.rs) ([`js_preludes.rs`](../../crates/slice-engine/src/preludes/js_preludes.rs), [`node_builtins.rs`](../../crates/slice-engine/src/preludes/node_builtins.rs), [`js_minifier.rs`](../../crates/slice-engine/src/preludes/js_minifier.rs)): 32-bit `shards_mask` architecture combining always-on `00_ultra_core.js` with demand-loaded compile-time minified JS shards `01..19` (including `18_intl.js` on bit `18` and `19_process_cli.js` on bit `29`) and pre-fused `node:*` built-in shards on bits `19..28`, scanned in a single `O(N)` Aho-Corasick pass (`detect_shards_and_boacas_flags`).
  - [`bindings/`](../../crates/slice-engine/src/bindings/mod.rs) & [`intrinsics/`](../../crates/slice-engine/src/intrinsics/mod.rs) ([`hash_and_hmac.rs`](../../crates/slice-engine/src/intrinsics/hash_and_hmac.rs), [`symmetric_and_kdf.rs`](../../crates/slice-engine/src/intrinsics/symmetric_and_kdf.rs), [`asymmetric_and_keys.rs`](../../crates/slice-engine/src/intrinsics/asymmetric_and_keys.rs), [`compression.rs`](../../crates/slice-engine/src/intrinsics/compression.rs), [`encoding.rs`](../../crates/slice-engine/src/intrinsics/encoding.rs), [`url_and_usp.rs`](../../crates/slice-engine/src/intrinsics/url_and_usp.rs), [`fast_parsers.rs`](../../crates/slice-engine/src/intrinsics/fast_parsers.rs), [`prng_and_uuid.rs`](../../crates/slice-engine/src/intrinsics/prng_and_uuid.rs)): On-demand native Rust host bindings, hardware `SHA-NI` + software SHA-1/256/384/512 & HMAC, AES-GCM/CBC, PBKDF2/HKDF/scrypt, Ed25519/ECDSA/RSA/X25519 asymmetric crypto, RFC 1950/1951/1952 DEFLATE/Gzip, WHATWG URL/URLSearchParams, and SemVer/utility scanners.
  - [`net/`](../../crates/slice-engine/src/net/mod.rs) ([`outbound_http.rs`](../../crates/slice-engine/src/net/outbound_http.rs), [`websocket.rs`](../../crates/slice-engine/src/net/websocket.rs), [`sockets.rs`](../../crates/slice-engine/src/net/sockets.rs)): Outbound HTTP/1.1 & TLS 1.3 (`rustls` + `SHARED_OUTBOUND_CONN_POOL` + Pre-Connect DNS SSRF firewall), RFC 6455 WebSocket engine, and 5-Ring OCap TCP/TLS/UDP sockets.
- **`crates/slice-cli/src/`**:
  - [`main.rs`](../../crates/slice-cli/src/main.rs), [`commands/`](../../crates/slice-cli/src/commands/mod.rs) ([`init.rs`](../../crates/slice-cli/src/commands/init.rs), [`pack_and_compile.rs`](../../crates/slice-cli/src/commands/pack_and_compile.rs), [`run.rs`](../../crates/slice-cli/src/commands/run.rs), [`test_runner.rs`](../../crates/slice-cli/src/commands/test_runner.rs)), [`config/`](../../crates/slice-cli/src/config/mod.rs) ([`cas_packager.rs`](../../crates/slice-cli/src/config/cas_packager.rs), [`watcher.rs`](../../crates/slice-cli/src/config/watcher.rs)), [`server/`](../../crates/slice-cli/src/server/mod.rs) ([`connection.rs`](../../crates/slice-cli/src/server/connection.rs), [`tls.rs`](../../crates/slice-cli/src/server/tls.rs)), and [`diagnostics/`](../../crates/slice-cli/src/diagnostics/mod.rs) ([`doctor_and_bench.rs`](../../crates/slice-cli/src/diagnostics/doctor_and_bench.rs), [`mcp.rs`](../../crates/slice-cli/src/diagnostics/mcp.rs), [`repl.rs`](../../crates/slice-cli/src/diagnostics/repl.rs)), plus modular integration tests under `crates/*/tests/` and Criterion microbenchmarks under `crates/*/microbench/`.

---

## 4. Why Legacy Runtimes Hit a Cold-Start and Density Wall

To understand why Slice achieves sub-microsecond in-process isolate startup, single-digit millisecond cold CLI execution, and multi-fold lower memory footprints across real-world TypeScript + `node_modules` workloads (see **[`tests/reports/latest.md`](../../tests/reports/latest.md)** for the latest verified benchmark telemetry across all 33 workloads), examine the four structural bottlenecks in Node.js, Deno, and Bun:

1. **The POSIX Filesystem Metadata Tax (`stat` / `lstat` / `readdir` Amplification):**
   - Resolving a 500-module graph or a `node_modules` tree in Node.js triggers over `1,500` synchronous/asynchronous `stat()` and `open()` system calls walking up directory hierarchies (`node_modules/pkg/package.json`, `index.js`, extensions). On containerized overlay filesystems (`OverlayFS`) or cloud block volumes (AWS EBS / GCP PD), this metadata chatter consumes 40% to 70% of startup time.
2. **Eager Source Parsing & Monolithic Isolate Bootstrap:**
   - Spinning up a fresh V8 isolate without a custom snapshot takes `5 ms - 40 ms` and `20 MB - 50 MB` of heap. Even with snapshots, parsing hundreds of kilobytes of unreferenced barrel exports (such as `hono/index.js` re-exporting 14 router/adapter modules when an app only imports `{ Hono }`, or `semver/index.js` loading 48 submodules when an app only calls `valid`, `coerce`, `gt`, and `satisfies`) wastes milliseconds of CPU time before the first line of user code runs.
3. **Readiness-Poll Thread-Pool Contention (`epoll` + `libuv` Worker Threads):**
   - Node.js and Deno offload filesystem operations to background thread pools. Under high multi-tenant concurrency, thread synchronization, mutex handoffs, and kernel context switches introduce tail-latency spikes.
4. **Ambient Authority vs. Heavyweight VM Isolation:**
   - Node.js and Bun execute with full ambient OS permissions; any compromised transitive npm dependency can read `~/.ssh`, harvest `process.env`, or exfiltrate data. Traditional microVMs (Firecracker) solve this by booting a separate guest Linux kernel per tenant, costing `125 ms` cold start and `30 MB - 128 MB` per tenant.
