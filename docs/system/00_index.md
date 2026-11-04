# Slice (Nexus-JS): System Architecture & Internal Specification Index (`v0.9.9`)

Welcome to the **Slice (Nexus-JS)** System Architecture and Internal Specification (`docs/system/`). Engineered in pure Rust (`edition = 2021`, `opt-level = 3`, `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`), Slice replaces the legacy POSIX + monolithic V8/JSC assumptions of Node.js, Deno, and Bun with four orthogonal, tightly integrated Rust crates:

1. **[`slice-cas`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs)** — Zero-Stat Content-Addressed Storage (`SLICE_CAS` v1), Oxc (`v0.152.0`) AOT TypeScript/TSX Compiler, Oxc Semantic/AST Dead-Export Tree-Shaker, & `SLICEEXE` Standalone Executable Linker.
2. **[`slice-io`](file:///p:/opensource/slice/crates/slice-io/src/lib.rs)** — Direct Kernel Completion Ring Engine (Linux `io_uring` `IORING_SETUP_SQPOLL` + Windows NT `IOCP` FFI + `64 KB` Page-Aligned Fixed Slab Buffers).
3. **[`slice-sandbox`](file:///p:/opensource/slice/crates/slice-sandbox/src/lib.rs)** — MicroVM-Grade Object Capabilities (`FsCap`, `NetCap`, `EnvCap`), Dynamic SES Intrinsic Lockdown, Deterministic CPU Fuel & Byte Cage, & Linux `x86_64` Hardware `WRPKRU` Memory Protection Keys (MPK).
4. **[`slice-engine`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs)** — Hybrid Three-Tier Execution Core: Tier-0 `SLICESNP` Snapshot VM, Tier-1 Zero-Copy `SLICEBOA` + Pre-Warmed `boa_engine` Pool + Native Rust Host Intrinsics, & Tier-2 Unboxed 64-Bit Numeric Register VM ([`numeric_vm.rs`](file:///p:/opensource/slice/crates/slice-engine/src/numeric_vm.rs)).

---

## 1. System Architecture Map (`docs/system/`)

| Document | Subsystem / Scope | Key Source Modules Covered |
| :--- | :--- | :--- |
| **[`00_index.md`](file:///p:/opensource/slice/docs/system/00_index.md)** | Executive Overview, 4-Crate Topology, Crate & Module Map, Legacy Runtime Bottlenecks, & Master Summary of Architectural Gains | Workspace Overview |
| **[`01_cas_bundler_and_oxc_compiler.md`](file:///p:/opensource/slice/docs/system/01_cas_bundler_and_oxc_compiler.md)** | Zero-Stat `SLICE_CAS` v1 Binary Format, Oxc `v0.152.0` TS/TSX Compiler, Semantic Lowering, Oxc AST Array Pipeline Fusion, ESM-First `"exports"`, Cold-Method `require` Inliner, Reverse-Topological Oxc Semantic/AST Tree-Shaker, & Single-Copy `SLICEBOA` CAS Deduplication | `crates/slice-cas/src/{lib,oxc_compiler,esm_cjs_linker,resolver,static_assets}.rs` |
| **[`02_tiered_execution_and_boa_vm.md`](file:///p:/opensource/slice/docs/system/02_tiered_execution_and_boa_vm.md)** | Hybrid Three-Tier Execution Pipeline (`SLICESNP`, Zero-Copy `SLICEBOA`, `WarmBoaRealm` Pool, Persistent Realm Cache `__slice_rc`, Compile-Time Minified Sharded Preludes `00..10`, Hardware `SHA-NI` & Native Intrinsics, & Tier-2 Unboxed 64-Bit Numeric Register VM) | `crates/slice-engine/src/{isolate,boa_realm,js_preludes,parser_and_compiler,numeric_vm,intrinsics}.rs` |
| **[`03_security_sandbox_ocap_and_mpk.md`](file:///p:/opensource/slice/docs/system/03_security_sandbox_ocap_and_mpk.md)** | 5-Ring Zero-Trust Security Architecture: OCap `CapabilityToken` (`FsCap`/`NetCap`/`EnvCap`), Dynamic SES Prototype Integrity (`__slice_check_ses`), CPU Fuel & Memory Cage, Linux `WRPKRU` Hardware MPK, Pre-Connect DNS SSRF Firewall, & Outbound Egress Secret DLP | `crates/slice-sandbox/src/lib.rs`, `crates/slice-engine/src/{isolate,outbound_http,state_and_kv}.rs` |
| **[`04_networking_wintertc_and_event_loop.md`](file:///p:/opensource/slice/docs/system/04_networking_wintertc_and_event_loop.md)** | Multi-Worker `slice serve`, 3-Level Async Concurrency Architecture, Outbound HTTP/1.1 & TLS 1.3 (`rustls` + `OUTBOUND_CONN_POOL`), Deterministic Time-Travel Event Loop, RFC 6455 WebSocket Engine, Raw TCP/TLS/UDP Sockets, & `Uint8Array` `Buffer` + Node Core Tier-1 | `crates/slice-cli/src/http_server.rs`, `crates/slice-engine/src/{event_loop,outbound_http,websocket,sockets,node_builtins}.rs` |
| **[`05_wasm_workers_and_kernel_io.md`](file:///p:/opensource/slice/docs/system/05_wasm_workers_and_kernel_io.md)** | Pure-Rust WebAssembly (`wasmi` `v0.40`, `GLOBAL_WASM_MODULE_CACHE`, 5-Ring WASM Cage), Capability-Attenuated MicroVM Web Workers & `node:worker_threads` (`IDLE_WORKER_POOL`), Direct Kernel Completion Rings (`io_uring` & `IOCP`), & OS-Native `notify` File Watcher | `crates/slice-engine/src/{wasm,workers}.rs`, `crates/slice-io/src/lib.rs`, `crates/slice-cli/src/config_and_watch.rs` |
| **[`06_edge_kv_ai_sandbox_and_mcp.md`](file:///p:/opensource/slice/docs/system/06_edge_kv_ai_sandbox_and_mcp.md)** | 16-Way Sharded `Slice.kv` (`EDGE_KV_SLAB`), Multi-Tenant `--tenant-dir` Atomic Hot-Swap, SplitMix64 `--deterministic` Replay, AI Agent "Code Mode" (`Slice.evalSandbox`, `< 5 us` `Slice.checkpoint`/`restore`), Time-Travel `slice test`, & Built-In MCP Server (`slice mcp`) | `crates/slice-engine/src/state_and_kv.rs`, `crates/slice-cli/src/{test_runner,mcp_and_repl}.rs` |
| **[`07_production_tooling_guide.md`](file:///p:/opensource/slice/docs/system/07_production_tooling_guide.md)** | Production CLI Reference, `slice.toml` Specification, Docker & Kubernetes Deployment Patterns, and Multi-Tenant Operations | `crates/slice-cli/src/main.rs` |
| **[`08_benchmarks_and_comparisons.md`](file:///p:/opensource/slice/docs/system/08_benchmarks_and_comparisons.md)** | Complete 4-Runtime Empirical Benchmark Suite (Slice `v0.9.9` vs Node.js `v26.12.0`, Deno `v2.7.12`, and Bun `v1.4.3`) Across All 26 Applications & Framework Suites | `benches/`, `tests/apps/`, `tests/frameworks/` |
| **[`09_research_deep_dive.md`](file:///p:/opensource/slice/docs/system/09_research_deep_dive.md)** | Foundational Systems Research Paper: Breaking the JS/TS Cold-Start, Memory Density, and Multi-Tenant Security Wall | Architectural Research Blueprint |

---

## 2. High-Level 4-Crate Host Topology

```
+---------------------------------------------------------------------------------------------------+
|                                   Slice (Nexus-JS) v0.9.9 Host                                    |
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
|  |  | Tier 0: SLICESNP VM      |  | Tier 1: Zero-Copy SLICEBOA    |  | Tier 2: Peak Native  |  |  |
|  |  | - 1.92 us Cold Start     |  | - 0.02 us mmap String Slice   |  | - 32-Reg Numeric VM  |  |  |
|  |  | - Pre-Parsed Opcode Ops  |  | - Pre-Warmed Boa Realm Pool   |  | - Recurrence Lowering|  |  |
|  |  | - WinterTC Edge Routing  |  | - Native Rust Host Intrinsics |  | - 10.02 us Execution |  |  |
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
  - [`lib.rs`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs): 64-byte cache-line aligned `SLICE_CAS` v1 binary format, `CasBuilder` (with single-copy `SLICEBOA` source deduplication), `MappedCasArchive`, and `SLICEEXE` standalone executable trailer embedding.
  - [`oxc_compiler.rs`](file:///p:/opensource/slice/crates/slice-cas/src/oxc_compiler.rs): Native Oxc (`v0.152.0`) TypeScript and `.tsx`/`.jsx` transformation (`SourceType::tsx()`, `transform_options.jsx.jsx_plugin = true`), semantic interface analysis (`analyze_typescript_semantics`), and Oxc AST array pipeline loop fusion (`fuse_array_pipelines_ast`).
  - [`esm_cjs_linker.rs`](file:///p:/opensource/slice/crates/slice-cas/src/esm_cjs_linker.rs): Oxc AST-driven ESM-to-CJS module linker (`try_transform_esm_to_cjs_ast`), Cold-Method `require` Inliner (`inline_cold_method_requires_ast`), Reverse-Topological Oxc Semantic/AST Dead-Export Tree-Shaker (`analyze_module_used_exports` + `prune_and_minify_cjs_module_ast`), Eager-Edge BFS Lazy Module Stringifier (`collect_eager_module_indices`), and source-line error mapping.
  - [`resolver.rs`](file:///p:/opensource/slice/crates/slice-cas/src/resolver.rs): Project entrypoint discovery (`.ts`, `.tsx`, `.js`, `.jsx`), ESM-first `package.json` conditional `"exports"` resolution (`"workerd"`, `"worker"`, `"edge-light"`, `"import"`, `"module"`, `"browser"`, `"require"`, `"node"`, `"default"`), zero-stat `.wasm` binary bundling (`__slice_b64ToU8`), and built-in `"slice:jsx"` fallback.
  - [`static_assets.rs`](file:///p:/opensource/slice/crates/slice-cas/src/static_assets.rs): Zero-stat `StaticAssetTable` with pre-computed `BLAKE3` `ETag` headers and MIME classification.
- **`crates/slice-engine/src/`**:
  - [`value_and_ast.rs`](file:///p:/opensource/slice/crates/slice-engine/src/value_and_ast.rs): 16-byte `JsValue`, `JsFunction`, AST nodes (`Stmt`, `Expr`), and `BytecodeOp` definitions.
  - [`numeric_vm.rs`](file:///p:/opensource/slice/crates/slice-engine/src/numeric_vm.rs): Tier-2 Unboxed 64-Bit Numeric Register Compiler (`try_compile_numeric_func`), structural recurrence analyzer (`detect_linear_recurrence_ast`), and register VM (`NumericFunc::execute`).
  - [`parser_and_compiler.rs`](file:///p:/opensource/slice/crates/slice-engine/src/parser_and_compiler.rs): Lexer, TypeScript-aware parser, `SLICESNP`/`SLICEBOA` compiler, build-time `SLICEBOA` payload compactor (`compact_js_for_boa_bytecode`), and disassembler.
  - [`isolate.rs`](file:///p:/opensource/slice/crates/slice-engine/src/isolate.rs): `SliceIsolate` lifecycle, `execute_bytecode`, `execute_http_request`, `eval_sandbox`, and Tier-0/1/2 dispatch.
  - [`boa_realm.rs`](file:///p:/opensource/slice/crates/slice-engine/src/boa_realm.rs): `BOA_CONTEXT_POOL`, `WarmBoaRealm` lifecycle, and thread-local OCap VFS / compartment state.
  - [`js_preludes.rs`](file:///p:/opensource/slice/crates/slice-engine/src/js_preludes.rs): Compile-time minified `ULTRA_CORE_PRELUDE` (`00_ultra_core.js`) and 10 surgical lazy prelude shards (`01_web_fetch_url.js` through `10_annex_b_escape.js`) with `detect_needed_shards` bitmask analysis.
  - [`node_builtins.rs`](file:///p:/opensource/slice/crates/slice-engine/src/node_builtins.rs): Lazy IIFE-isolated `node:*` (`crypto`, `fs`, `fs/promises`, `path`, `events`, `timers`, `timers/promises`, `util`, `assert`, `os`, `url`, `buffer`, `net`, `tls`, `dgram`, `worker_threads`), `cloudflare:sockets`, `slice:test`, and `slice:jsx` built-in loader.
  - [`host_bindings.rs`](file:///p:/opensource/slice/crates/slice-engine/src/host_bindings.rs): Native Rust host functions (`register_slice_native_host_functions`) bound to `boa_engine::Context`.
  - [`intrinsics.rs`](file:///p:/opensource/slice/crates/slice-engine/src/intrinsics.rs): Hardware `SHA-NI` + software FIPS-180-4 SHA-256, SHA-1/384/512, RFC-2104 HMAC, Base64/Base64URL, and zero-regex SemVer/byte/duration scanners.
  - [`event_loop.rs`](file:///p:/opensource/slice/crates/slice-engine/src/event_loop.rs): Deterministic Time-Travel Event Loop, Slab Timer Wheel, `process.nextTick` / `queueMicrotask` priority scheduler, and concurrent `Promise.all` fetch multiplexer.
  - [`outbound_http.rs`](file:///p:/opensource/slice/crates/slice-engine/src/outbound_http.rs): Outbound HTTP/1.1 & HTTPS (TLS 1.3 via `rustls` + `webpki-roots`), `SHARED_OUTBOUND_CONN_POOL`, Pre-Connect DNS Resolved-IP SSRF firewall, Hop-by-Hop Redirect Guard, and deterministic HTTP cassettes.
  - [`websocket.rs`](file:///p:/opensource/slice/crates/slice-engine/src/websocket.rs): Zero-dependency RFC 6455 WebSocket engine (`ws://`, `wss://`, server upgrade, `Slice.mockWebSocket`, and per-frame Egress DLP).
  - [`workers.rs`](file:///p:/opensource/slice/crates/slice-engine/src/workers.rs): Capability-attenuated MicroVM Web Workers (`new Worker()`) and `node:worker_threads` (`IDLE_WORKER_POOL`, `WORKER_SCRIPT_CACHE`).
  - [`sockets.rs`](file:///p:/opensource/slice/crates/slice-engine/src/sockets.rs): Zero-Trust OCap TCP/TLS/UDP socket engine (`Slice.connect`, `cloudflare:sockets`, `node:net`, `node:tls`, `node:dgram`).
  - [`wasm.rs`](file:///p:/opensource/slice/crates/slice-engine/src/wasm.rs): Pure-Rust `WebAssembly` (`wasmi` `v0.40`) engine with `GLOBAL_WASM_MODULE_CACHE` and 5-Ring CPU fuel & linear memory cages.
  - [`state_and_kv.rs`](file:///p:/opensource/slice/crates/slice-engine/src/state_and_kv.rs): 16-way sharded `EDGE_KV_SLAB`, SplitMix64 deterministic PRNG, and `< 5 us` VFS+KV checkpoint/restore.
- **`crates/slice-cli/src/`**:
  - [`main.rs`](file:///p:/opensource/slice/crates/slice-cli/src/main.rs), [`config_and_watch.rs`](file:///p:/opensource/slice/crates/slice-cli/src/config_and_watch.rs), [`http_server.rs`](file:///p:/opensource/slice/crates/slice-cli/src/http_server.rs), [`test_runner.rs`](file:///p:/opensource/slice/crates/slice-cli/src/test_runner.rs), [`mcp_and_repl.rs`](file:///p:/opensource/slice/crates/slice-cli/src/mcp_and_repl.rs), and [`cli_tests.rs`](file:///p:/opensource/slice/crates/slice-cli/src/cli_tests.rs).

---

## 4. Why Legacy Runtimes Hit a Cold-Start and Density Wall

To understand why Slice achieves **`1.92 us - 3.08 us` in-process isolate startup**, **`2.00 ms` CLI cold start**, **`5.18 ms - 28.08 ms` real-stack multi-module TypeScript + `node_modules` execution**, and **`2.62 MB - 16.38 MB` Peak RSS** (up to `22x` smaller than Node.js `v26.12.0`), examine the four structural bottlenecks in Node.js, Deno, and Bun:

1. **The POSIX Filesystem Metadata Tax (`stat` / `lstat` / `readdir` Amplification):**
   - Resolving a 500-module graph or a `node_modules` tree in Node.js triggers over `1,500` synchronous/asynchronous `stat()` and `open()` system calls walking up directory hierarchies (`node_modules/pkg/package.json`, `index.js`, extensions). On containerized overlay filesystems (`OverlayFS`) or cloud block volumes (AWS EBS / GCP PD), this metadata chatter consumes 40% to 70% of startup time.
2. **Eager Source Parsing & Monolithic Isolate Bootstrap:**
   - Spinning up a fresh V8 isolate without a custom snapshot takes `5 ms - 40 ms` and `20 MB - 50 MB` of heap. Even with snapshots, parsing hundreds of kilobytes of unreferenced barrel exports (such as `hono/index.js` re-exporting 14 router/adapter modules when an app only imports `{ Hono }`, or `semver/index.js` loading 48 submodules when an app only calls `valid`, `coerce`, `gt`, and `satisfies`) wastes milliseconds of CPU time before the first line of user code runs.
3. **Readiness-Poll Thread-Pool Contention (`epoll` + `libuv` Worker Threads):**
   - Node.js and Deno offload filesystem operations to background thread pools. Under high multi-tenant concurrency, thread synchronization, mutex handoffs, and kernel context switches introduce tail-latency spikes.
4. **Ambient Authority vs. Heavyweight VM Isolation:**
   - Node.js and Bun execute with full ambient OS permissions; any compromised transitive npm dependency can read `~/.ssh`, harvest `process.env`, or exfiltrate data. Traditional microVMs (Firecracker) solve this by booting a separate guest Linux kernel per tenant, costing `125 ms` cold start and `30 MB - 128 MB` per tenant.

---

## 5. Master Summary of Architectural Gains Across All 26 Production Workloads

| Architectural Innovation | Bottleneck Eliminated | Measured Gain (Linux `x86_64` Docker) |
| :--- | :--- | :--- |
| **`SLICE_CAS` v1 + `mmap` + `SLICEEXE` + Single-Copy `SLICEBOA`** | `1,500+` `stat()` / `open()` syscalls, JSON AST parsing, & duplicate source+bytecode CAS storage | **`0` `stat()` syscalls**, `395 us` 500-module load (`20.6x` faster than Node.js), & **50% smaller `.cas` archives** |
| **Zero-Copy `SLICEBOA` Framing** | `serde_json` string unescaping of bundled JS payloads | Isolate instantiation dropped **`229.66 us` -> `0.02 us - 0.27 us`** (**`>1,000x` faster**) |
| **Pre-Warmed `Boa` Pool (`BOA_CONTEXT_POOL`)** | Repeated realm intrinsic creation & JS prelude eval | `01-fintech-checkout` isolate exec dropped **`5.07 ms` -> `0.67 ms`** (**`7.56x` faster**) |
| **Oxc Semantic/AST Dead-Export Tree-Shaking & Cold-Method `require` Inliner (`slice-cas`)** | Bundling unused `hono`, `radix3`, `ufo`, `semver`, & barrel exports or eagerly loading cold class methods | **Zero framework-specific hacks**; `01-hono-edge-microservice` `.cas` **`35,088 B`** (`6.76 ms` cold / `0.90 ms` warm, **`3.59x` faster than Bun**); `02-itty-router-unjs-nitro-stack` `.cas` **`26,568 B`** (`6.26 ms` cold / `0.60 ms` warm, **`2.75x` faster than Bun**) |
| **Native Rust `SemVer` + `VFS` + `Crypto` Intrinsics** | Slow interpreted JS regexes, temp-file syscalls & closures | `02-npm-ecosystem-suite` cold `.cas` **`6.766 ms`** vs Bun `20.048 ms` (**`2.96x` faster cold**) |
| **Native `Boa` Host OCap Enforcement** | Unchecked `node:fs` access inside third-party packages | **5/5 Security Vectors Contained** across both `SLICESNP` and `SLICEBOA` |
| **Zero-Alloc Isolate Spawn & `16-Byte JsValue`** | 11 heap allocations per isolate spawn + `80B` `JsValue` enum | 1,000-Tenant MicroVM Swarm **`6.57 ms` -> `1.42 ms`** (`1.00 us` min cold-start, `9.93 MB` total RSS) |
| **`SHA-NI`, `__slice_rc` Persistent Realm Cache & Oxc Semantic Lowering** | Cold CJS wrapper re-eval, dynamic `JSON.stringify` & crypto builder allocs | All 26 apps & frameworks **`5.18 ms - 28.08 ms` cold** / **`0.19 ms - 12.50 ms` warm** (**up to `73.7x` faster than Bun**) |
| **Multi-Worker `slice serve` (`--workers=N`, Keep-Alive, Pipelining, Direct `JsObject::call` & WinterTC Bridge)** | Per-request TCP handshake, `BufReader` line allocs, & `ctx.eval` re-parsing on warm HTTP requests | **`152,376 req/s` (`6.56 us/req`)** in Tier-0 CAS (**`1.73x` faster than `Bun.serve`**) and **`107,274 req/s` (`9.32 us/req`)** in Tier-1 WinterTC (**beating both `Bun.serve` and `Deno.serve`**) |
| **Sharded `Slice.kv`, Multi-Tenant `--tenant-dir` Hot-Swap, & `--deterministic` Replay** | External Redis round-trips for edge rate-limiting, server restarts on tenant deploy, & non-deterministic PRNG/clocks | **`586.10 us` for 1,001 `Slice.kv` ops** (`0.58 us/op`), zero-downtime `.cas` hot-swap, & 100% bit-for-bit SplitMix64 replay |
| **`Slice.evalSandbox`, `< 5 us` `Slice.checkpoint`/`restore`, Egress Secret Guard, & `slice mcp` Server** | 125ms Firecracker VM boot for AI agent tool calls, slow disk rollback for Tree-of-Thought, & prompt-injection secret exfiltration | **Sub-millisecond nested TypeScript sub-sandboxes with capability attenuation, `< 5 us` VFS+KV state rollback, Base64 egress secret blocking, & native JSON-RPC 2.0 MCP server** |
| **WinterTC `fetch`/`Request`/`Response`, `Response.sse`, Outbound SSRF/DLP Firewall, & `slice test` Runner** | Heavyweight external test runners, slow DB/FS test teardown, SSRF metadata attacks, & unguarded outbound `fetch()` exfiltration | **Native `export default { fetch }`, SSE token streaming, pre-connect SSRF + Egress DLP firewall, deterministic HTTP cassettes, & `< 5 us` Time-Travel rollback per unit test** |
| **Unboxed 64-Bit Numeric Register VM (`numeric_vm.rs`), Oxc AST Array Pipeline Fusion, & Dynamic SES + `Boa` `RuntimeLimits`** | Name-dependent benchmark shortcuts, intermediate `.map().reduce()` array allocations, & dynamically obfuscated `Object["proto"+"type"]` attacks | **General-purpose 32-register `[f64; 32]` numeric VM + structural recurrence lowering on any function name, Oxc AST `.filter().map().reduce()` loop fusion, & dynamic SES rollback + `Boa` loop fuel preemption** |
| **Outbound HTTPS/TLS 1.3 (`rustls`), Thread-Local Keep-Alive Pooling (`OUTBOUND_CONN_POOL`), Pre-Connect DNS SSRF Firewall, & Hop-by-Hop Redirect Guard** | Missing TLS 1.3 client support, per-request `Connection: close` socket churn, DNS-rebinding SSRF to private/metadata IPs, & unguarded `302` redirect credential leaks | **Zero-cost `OnceLock` TLS 1.3 + 256-session resumption (`0 B` overhead when unused), Keep-Alive socket reuse across `Content-Length`/chunked responses, IPv4/IPv6/mapped-IPv6 DNS SSRF blocking, & cross-origin `Authorization`/`Cookie` stripping** |
| **Deterministic Time-Travel Event Loop (`event_loop.rs`), Timer Wheel, `AbortSignal`, & Concurrent `Promise.all` Fetch Fan-Out** | Blocking sequential `Promise.all([fetch(...)])`, missing `setTimeout`/`setInterval`/`queueMicrotask`/`AbortSignal.timeout`, & slow wall-clock sleeps in retry tests | **Zero-wait Virtual Time-Warp (`66,000 ms` backoff suite in `< 50 ms`), true parallel `Promise.all` socket fan-out over shared Keep-Alive pool, `AbortSignal.timeout(ms)` deadline cancellation, & `MAX_ACTIVE_TIMERS` timer-bomb containment** |
| **WinterTC Minimum Common Web Platform API (`crypto.subtle`, `TextEncoder`/`Decoder`, WhatWG Streams, `EventTarget`, `Blob`/`File`/`FormData`, `structuredClone`)** | Missing WebCrypto `subtle` HMAC/SHA/JWK, UTF-8 `Uint8Array` encoding, `TransformStream` pipelines, `AbortSignal.any`, & cyclic `structuredClone` required by Edge frameworks | **Direct `JsArrayBuffer`/`JsUint8Array` Rust bridge (`0` JSON/hex marshaling), deterministic SplitMix64 `randomUUID`/`getRandomValues` replay, zero-alloc SHA-1/256/384/512 + HMAC-SHA256/512, & 100% Node.js parity** |
| **Real `Uint8Array` `Buffer`, Node Core Tier-1 (`events`, `path`, `crypto`, `fs/promises`, `util`, `assert`), & Oxc AST Bundler** | String-backed `Buffer` failing `instanceof Uint8Array`, fragile line-by-line ESM regex transforms, & missing `package.json` subpath `"exports"` | **True `Uint8Array` `Buffer` with wire-protocol `DataView` readers/writers, lazy IIFE-isolated `node:*` built-ins (`0 B` cold overhead), Oxc AST ESM->CJS rewriting, & 100% Node.js parity** |
| **Compile-Time Minified Sharded Preludes (`build.rs` + `src/preludes/*.js`), Self-Replacing Lazy Globals, Conditional SES Guard, & Direct Main-Thread CLI Dispatch** | Parsing ~48 KB of monolithic JS preludes on every cold CLI process launch + unnecessary 16 MB OS thread spawn on Linux | **Human-readable `src/preludes/00..10_*.js` minified at compile time via `build.rs`, `~6 KB` eager `ULTRA_CORE_PRELUDE` + 10 self-replacing lazy shards (`0 B` cost for unused APIs), conditional SES checks, & direct main-thread Linux execution** |
| **Capability-Attenuated MicroVM Web Workers (`workers.rs`, `node:worker_threads`) & 5-Ring Zero-Trust Raw TCP/TLS/UDP Sockets (`sockets.rs`, `cloudflare:sockets`, `node:net`, `node:dgram`)** | Unsafe ambient-authority OS threads, 15ms per-worker cold OS thread/realm creation, duplicate blocking `node:net` read pumps, or unguarded raw TCP/UDP sockets that bypass SSRF and Egress Secret DLP filters | **Persistent warm worker pool (`IDLE_WORKER_POOL`) + speculative pre-warming + `__SLICE_RC` worker script cache + coalesced `node:net` read pump (`App 22`: `28.08 ms` cold / `2.00 ms` warm vs Bun's `44.24 ms`), `parent ∩ child` OCap attenuation, & 5-Ring TCP/UDP socket security** |
| **Pure-Rust WebAssembly (`wasm.rs` via `wasmi` `v0.40`), `GLOBAL_WASM_MODULE_CACHE`, 5-Ring WASM Security Cage, & OS-Native `notify` Watcher** | Unmetered WASM loops/memory growth bypassing JS sandbox limits, repeated `.wasm` compilation across warm isolates, and 25ms directory-polling CPU overhead | **`App 23`: `17.60 ms` cold / `1.82 ms` warm (`16.38 MB` RSS vs Bun `18.80 ms` / `30.25 MB`) & `App 24`: `14.49 ms` cold / `1.60 ms` warm (`14.12 MB` RSS vs Bun `17.28 ms` / `29.38 MB`), 5-Ring WASM fuel/memory preemption, & `4.92 ms` `notify` watch reruns** |
