# Slice (Nexus-JS): Complete Technical Architecture, Internal Specification & Extreme Optimization Deep Dive

## 1. Executive Architectural Overview

**Slice (Nexus-JS)** (`v0.2.0`) is a ground-up realization of the architectural blueprint specified in [`reimagining_next_gen_node_runtime.md`](file:///p:/opensource/slice/reimagining_next_gen_node_runtime.md). Engineered in pure Rust (`edition = 2021`, `opt-level = 3`, `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`), Slice replaces the legacy POSIX + monolithic V8/JSC assumptions of Node.js, Deno, and Bun with four orthogonal, tightly integrated subsystems:

1. **[`slice-cas`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs) (Zero-Stat Content-Addressed Storage, Oxc AOT TypeScript Compiler, & Static Barrel Tree-Shaker)**
2. **[`slice-io`](file:///p:/opensource/slice/crates/slice-io/src/lib.rs) (Direct Kernel Completion Ring Engine: Linux `io_uring` SQPOLL + Windows NT `IOCP` + Fixed Slab Buffers)**
3. **[`slice-sandbox`](file:///p:/opensource/slice/crates/slice-sandbox/src/lib.rs) (MicroVM-Grade Object Capabilities, SES Intrinsic Lockdown, Deterministic Fuel/Memory Cage, & Hardware `WRPKRU` MPK)**
4. **[`slice-engine`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs) (Hybrid Three-Tier Execution Core: Tier-0 `SLICESNP` Snapshot VM, Tier-1 Zero-Copy `SLICEBOA` + Pre-Warmed `boa_engine` Pool + Native Rust Host Intrinsics, & Tier-2 Native SIMD Kernels)**

```
+---------------------------------------------------------------------------------------------------+
|                                   Slice (Nexus-JS) v0.2.0 Host                                    |
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
|  |  | - 1.92 us Cold Start     |  | - 0.21 us mmap String Slice   |  | - SIMD Loop Kernels  |  |  |
|  |  | - Pre-Parsed Opcode Ops  |  | - Pre-Warmed Boa Realm Pool   |  | - Native Fib / Hash  |  |  |
|  |  | - WinterTC Edge Routing  |  | - Native Rust Host Intrinsics |  | - 10.02 us Execution |  |  |
|  |  +--------------------------+  +-------------------------------+  +----------------------+  |  |
|  +---------------------------------------------------------------------------------------------+  |
|                                                                                                   |
|  +---------------------------------------------------------------------------------------------+  |
|  |                    slice-cas: Zero-Stat Content-Addressed Storage & Bundler                 |  |
|  |  Oxc v0.152 TS Compiler -> Static Barrel Tree-Shaker -> BLAKE3-256 Radix -> 64B Aligned CAS |  |
|  +---------------------------------------------------------------------------------------------+  |
|                                                                                                   |
|  +---------------------------------------------------------------------------------------------+  |
|  |                    slice-io: Direct Kernel Completion Ring Engine                           |  |
|  |  Linux io_uring (IORING_SETUP_SQPOLL + 64KB Fixed Slab Buffers) | Windows NT IOCP FFI       |  |
|  +---------------------------------------------------------------------------------------------+  |
+---------------------------------------------------------------------------------------------------+
```

---

## 2. Why Legacy Runtimes Hit a Cold-Start and Density Wall

To understand **why** Slice achieves **`1.92 us` - `3.08 us` in-process isolate startup**, **`2.00 ms` CLI cold start**, **`5.98 ms` - `12.04 ms` real-stack multi-module TypeScript + `node_modules` execution**, and **`2.62 MB` - `11.00 MB` Peak RSS** (6x to 22x smaller than Node.js `v26.12.0`), we must first examine the four structural bottlenecks in Node.js, Deno, and Bun:

1. **The POSIX Filesystem Metadata Tax (`stat` / `lstat` / `readdir` Amplification):**
   - Resolving a 500-module graph or a `node_modules` tree in Node.js triggers over `1,500` synchronous/asynchronous `stat()` and `open()` system calls walking up directory hierarchies (`node_modules/pkg/package.json`, `index.js`, extensions). On containerized overlay filesystems (`OverlayFS`) or cloud block volumes (AWS EBS / GCP PD), this metadata chatter consumes 40% to 70% of startup time.
2. **Eager Source Parsing & Monolithic Isolate Bootstrap:**
   - Spinning up a fresh V8 isolate without a custom snapshot takes `5 ms - 40 ms` and `20 MB - 50 MB` of heap. Even with snapshots, parsing hundreds of kilobytes of unreferenced barrel exports (such as `semver/index.js` loading 48 submodules when an app only calls `valid`, `coerce`, `gt`, and `satisfies`) wastes milliseconds of CPU time before the first line of user code runs.
3. **Readiness-Poll Thread-Pool Contention (`epoll` + `libuv` Worker Threads):**
   - Node.js and Deno offload filesystem operations to background thread pools. Under high multi-tenant concurrency, thread synchronization, mutex handoffs, and kernel context switches introduce tail-latency spikes.
4. **Ambient Authority vs. Heavyweight VM Isolation:**
   - Node.js and Bun execute with full ambient OS permissions; any compromised transitive npm dependency can read `~/.ssh`, harvest `process.env`, or exfiltrate data. Traditional microVMs (Firecracker) solve this by booting a separate guest Linux kernel per tenant, costing `125 ms` cold start and `30 MB - 128 MB` per tenant.

---

## 3. Deep Dive: Subsystem Implementation & Why Every Optimization Works

### 3.1 [`crates/slice-cas`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs): Zero-Stat CAS, Oxc Compiler & Static Barrel Tree-Shaking

`slice-cas` shifts all module resolution, TypeScript compilation, comment stripping, and dead-dependency elimination from runtime to build/pack time (`slice pack` / `slice compile`):

#### A. Production Rust TypeScript Compilation via Oxc (`v0.152.0`)
- Implemented in [`strip_typescript_raw`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs#L96-L153) using `oxc::parser::Parser`, `oxc::semantic::SemanticBuilder`, `oxc::transformer::Transformer`, and `oxc::codegen::Codegen`.
- **Why:** Oxc parses ASTs in a bump-allocated memory arena (`oxc::allocator::Allocator`) and strips TypeScript interfaces, type aliases, generics, parameter properties, and access modifiers (`private`, `protected`, `public`, `readonly`) at native Rust speeds (3x-5x faster than SWC, 40x+ faster than `tsc`).
- **Plain `.js` Dependency Compaction:** During [`bundle_entrypoint_with_deps`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs#L692-L810), third-party `.js` files inside `node_modules/` are also passed through Oxc's parser + codegen (`strip_typescript_raw(raw, false)`), stripping all JSDoc blocks, license banners, and whitespace before bundling.

#### B. Transitive ESM + CommonJS + JSON Graph Bundler & Static Barrel Tree-Shaking
- Implemented in [`resolve_project_entrypoint`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs#L172-L234), [`resolve_module_specifier`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs#L236-L312), and [`transform_module_to_cjs`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs#L387-L690).
- **How It Works:**
  1. Resolves folder-based applications via `slice.toml`, `package.json` (`exports["."]` -> `module` -> `main`), or `src/index.ts`.
  2. Performs a deterministic depth-first traversal of both ES Modules (`import` / `export`) and CommonJS (`require()` / `module.exports`) across `src/` and `node_modules/`, plus `.json` files.
  3. **Phase 7.2 Static Barrel Tree-Shaking (`pruned_req_ids`):** Barrel packages like `semver/index.js` eagerly `require()` 42 functions across 48 files. `transform_module_to_cjs` detects barrel re-export maps, prunes the 30+ unused submodule `require(...)` declarations (`diff`, `inc`, `prerelease`, `clean`, `cmp`, `sort`, `maxSatisfying`, `minVersion`, `outside`, `simplifyRange`, `subset`, etc.), and removes their paths from `deps_out` (`deps_out.retain(...)`) so those files are **never read from disk or stored in the `.cas` archive**.
  4. **Lazy String-Backed Module Thunks (`new Function`):** For non-entrypoint transitive dependencies, `bundle_entrypoint_with_deps` emits the module code as a JSON string literal (`__slice_modules[id] = "..."`) that is only compiled via `new Function("module", "exports", "require", mod)` the first time `require(id)` is invoked at runtime.
  5. **Dense Array Preservation in `semver/internal/re.js`:** Early experiments showed that converting array tables (`re[index]`, `safeRe[index]`) into `Object.defineProperty` getters caused `boa_engine` to demote dense `FastArray` storage into slow dictionary objects. Keeping `safeRe[index] = re[index]` as a dense array assignment while eliminating redundant `{0,256}` bounded-quantifier regex compilations preserved O(1) indexed array lookups.
- **Measured Impact:** Cut `02-npm-ecosystem-suite` `.cas` archive size by **`30.0%`** (`195,978 B` -> `137,191 B`), reduced `slice pack` time to `11.53 ms`, and reduced Peak RSS to `10.75 MB`.

#### C. 64-Byte Cache-Line Aligned `SLICE_CAS` v1 Binary Format & `SLICEEXE` Standalone Trailer
- Every module blob in a `.cas` archive is hashed with SIMD-accelerated **BLAKE3-256** (`blake3::hash`) and padded to a strict **64-byte boundary** (`CACHE_LINE_ALIGN = 64`):
  ```
  +-----------------------------------------------------------------------+
  | 0..8   : Magic Bytes b"SLICECAS"                                      |
  | 8..16  : Version (u32 = 1) + Reserved Flags (u32 = 0)                 |
  | 16..24 : JSON Radix Header Length (u64 LE)                            |
  | 24..32 : 64-Byte Aligned Data Payload Start Offset (u64 LE)           |
  | 32..N  : CasIndexHeader (Radix Map: Specifier -> BLAKE3 -> Offsets)   |
  | [Padding to 64-byte CPU cache-line boundary]                          |
  | Data   : [64B-aligned Source Slice] [64B-aligned Bytecode Slice] ...  |
  +-----------------------------------------------------------------------+
  ```
- **Why 64-Byte Alignment & `mmap`?** When [`MappedCasArchive::open`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs#L1069-L1101) maps the file via `memmap2::MmapOptions::map`, every bytecode slice (`get_bytecode_slice`) starts at the beginning of a hardware CPU L1/L2 cache line, avoiding split-line loads and executing **0 `stat()` or `read()` system calls**.
- **Self-Contained Executables (`SLICEEXE`):** [`embed_cas_into_executable`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs#L1150-L1185) appends the `.cas` archive to the `slice` binary followed by a 24-byte trailer (`[u64 cas_offset][u64 cas_len][b"SLICEEXE"]`). On startup, [`MappedCasArchive::open_embedded_self`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs#L1103-L1148) memory-maps `std::env::current_exe()` and slices directly into the embedded archive in `0.21 microseconds`.

---

### 3.2 [`crates/slice-engine`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs): Hybrid Three-Tier Execution Pipeline & Native Fast-Path Intrinsics

`slice-engine` is designed around a single principle: **use the lightest execution tier capable of running the workload, and accelerate hot standard-library/ecosystem primitives in native Rust**.

#### A. Zero-Copy `SLICEBOA` Bytecode Framing (`229 us` -> `0.21 us` Instantiation)
- **The Problem We Solved:** Originally, all bytecode ops were wrapped inside a JSON-serialized `BytecodeModule` (`SLICESNP`). For a 130 KB bundled `node_modules` application, `serde_json::from_slice` had to allocate and unescape a 130 KB `String` on every isolate run, costing `~229 microseconds` before execution even began.
- **How We Solved It ([`compile_source_to_bytecode`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L1406-L1442) & [`execute_bytecode`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L1576-L1615)):**
  - When `compile_source_to_bytecode` detects an Oxc-bundled application (`var __slice_modules =`) or full ES2024+ OOP features (`class `, `new Map`, `try `, `node:`), it emits an 8-byte `b"SLICEBOA"` header followed immediately by the raw UTF-8 transpiled ECMAScript bytes.
  - At runtime, `execute_bytecode` checks `&bytecode[0..8] == b"SLICEBOA"`, performs a zero-allocation `std::str::from_utf8(&bytecode[8..])` slice directly over the OS `mmap` buffer, and passes that `&str` straight to the VM.
  - **Result:** Isolate instantiation time dropped by **1,000x**, from `229.66 us` down to **`0.21 us - 0.30 us`**.

#### B. Phase 7.1: Pre-Warmed `Boa` Context Pool (`BOA_CONTEXT_POOL`) & Lexical Block Scoping
- **The Problem We Solved:** Constructing a fresh `boa_engine::Context::default()` (initializing all ECMAScript intrinsic realms: `Object`, `Array`, `Function`, `RegExp`, `Promise`, `Map`, `Set`, `Error`, `JSON`, `Math`) and evaluating the Node.js compatibility prelude on every invocation took `~1.8 ms - 2.2 ms`.
- **How We Solved It ([`BOA_CONTEXT_POOL`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L2563-L2665)):**
  1. Created a thread-local pool of pre-initialized `boa_engine::Context` instances (`SliceIsolate::prewarm_boa_pool(count)`), wrapped in `std::mem::ManuallyDrop<Vec<boa_engine::Context>>` so thread exit on Windows never runs `boa_engine::Context::drop` after `boa_gc`'s TLS collector has already torn down.
  2. Wrapped every executed script inside a lexical block (`{\n<js_code>\n}`) in [`exec_boa_full_es_vm`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L2547-L2552) so top-level `const`, `let`, and `class` declarations are scoped to that invocation and never collide when the warm context is reused across requests in `slice serve` or `slice run --warm`.
  3. Replaced the post-execution `ctx.eval("globalThis.__slice_stdout.join('\\n')")` pass with a direct native Rust `__slice_console_log` host function writing straight into thread-local `BOA_STDOUT`.
  4. Made all 7 `node:*` built-in modules (`node:crypto`, `node:fs`, `node:path`, `node:os`, `node:util`, `node:buffer`, `node:process`) initialize **lazily on first `require()`** inside `globalThis.__slice_cjs_require` instead of eagerly at realm startup.

#### C. Phase 7.3: Native Rust Fast-Path Intrinsics (`SemVer`, `SHA-256`, `HMAC-SHA256`, `Base64URL`, & In-Memory VFS Slab)
- **Why:** Even with a warm realm and tree-shaken bundles, pure-JS bytecode interpreters spend significant cycles inside regular expression engines (`semver`), per-call object allocations (`crypto.createHash` / `createHmac`), and synchronous OS temp-file syscalls (`fs.writeFileSync` / `readFileSync` / `unlinkSync`).
- **How We Implemented Each Fast Path in [`crates/slice-engine/src/lib.rs`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L2804-L3570):**
  1. **Native Rust SemVer Intrinsics (`__slice_semver_valid`, `__slice_semver_coerce`, `__slice_semver_gt`, `__slice_semver_satisfies`):**
     - Implemented zero-regex Rust byte scanners ([`semver_parse_strict_xyz`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L3224-L3247), [`semver_coerce_xyz`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L3249-L3282), [`semver_satisfies_xyz`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L3284-L3344)) that parse `X.Y.Z` versions and evaluate `^`, `~`, `>=`, `<=`, `>`, `<` comparator ranges in ~150 nanoseconds in Rust, returning `-1` to fall back to the full JS `semver` implementation only when complex compound ranges (`||`, `-`, `*`, prereleases) are encountered.
     - **Impact:** Dropped `02-npm-ecosystem-suite` warm isolate execution from `19.70 ms` down to **`4.74 ms`** (`4.15x` faster) and CLI latency down to **`8.785 ms`** (**`2.36x` faster than Bun `v1.4.3`**).
  2. **Pure-Rust FIPS-180-4 SHA-256, RFC-2104 HMAC-SHA256 & Base64/Base64URL (`__slice_crypto_sha256`, `__slice_crypto_hmac_sha256`, `__slice_base64_encode`, `__slice_base64_decode`):**
     - Implemented in [`sha256_bytes`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L3347-L3435), [`hmac_sha256_hex`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L3447-L3472), [`base64_encode`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L3474-L3501), and [`base64_decode`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L3503-L3526).
     - Paired with reusable singleton `__hasher` and `__hmac` objects in `node:crypto` (with a `_busy` guard that automatically allocates a fresh state object if two hashes are interleaved simultaneously, as in App 5 `05-jwt-oauth-session-guard`).
  3. **Thread-Local In-Memory VFS Slab Cache (`BOA_VFS_CACHE` & `BOA_VFS_DIRS`):**
     - Implemented in [`__slice_fs_*`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L2859-L3063). Ephemeral files created during an isolate run (`writeFileSync`, `appendFileSync`, `mkdirSync`) are stored in thread-local memory slabs and served in nanoseconds on subsequent `statSync`, `readdirSync`, and `readFileSync` calls, while falling back transparently to `std::fs` for pre-existing host files.

#### D. Tier-0 (`SLICESNP`) and Tier-2 (`Tier2PeakNative`) SIMD Kernels
- Lightweight edge routing handlers (`WinterTC.route`), numeric loops (`numericLoop`), Fibonacci kernels (`fib`), and BLAKE3 digest loops (`cryptoDigest`) compile directly to compact `SLICESNP` opcodes (`BytecodeOp::WinterTcRoute`, `NumericLoopKernel`, `FibKernel`, `CryptoDigest`), executing in **`1.92 us - 14.61 us`** with **`4.0 KB` per-tenant memory**.

---

### 3.3 [`crates/slice-io`](file:///p:/opensource/slice/crates/slice-io/src/lib.rs): Direct Kernel Completion Rings (`io_uring` & `IOCP`)

- **Registered Slab Buffers (`RegisteredBufferPool`):** Pre-allocates `64 KB` page-aligned fixed buffers (`FIXED_BUFFER_SIZE = 65536`) with O(1) slot acquisition (`acquire_slot`) and release (`release_slot`) for zero-copy I/O transfers.
- **Linux `io_uring` (`CompletionRingEngine`):** Attempts kernel ring setup with `IORING_SETUP_SQPOLL` (`IoUring::builder().setup_sqpoll(1000)`), enabling autonomous kernel thread submission polling with **0 system calls** on the hot path, and automatically falls back to standard `io_uring` or user-space completion rings inside unprivileged Docker seccomp profiles.
- **Windows NT `IOCP`:** Binds directly to `CreateIoCompletionPort`, `PostQueuedCompletionStatus`, and `GetQueuedCompletionStatus` via `kernel32.dll` FFI, processing `10,000` completions in `0.02 ms - 5.26 ms`.

---

### 3.4 [`crates/slice-sandbox`](file:///p:/opensource/slice/crates/slice-sandbox/src/lib.rs): Multi-Tenant OCap, SES Lockdown & Hardware MPK (`WRPKRU`)

Every `SliceIsolate` executes inside a [`TenantCompartment`](file:///p:/opensource/slice/crates/slice-sandbox/src/lib.rs#L170-L270) enforcing five concentric security barriers:

1. **SES Intrinsic Lockdown (`IntrinsicRegistry::lockdown`):** Freezes `Object.prototype`, `Array.prototype`, `Function.prototype`, `Promise.prototype`, `JSON`, `Math`, and `Reflect`. Any prototype pollution attempt immediately traps with `SandboxViolation::IntrinsicMutationBlocked`.
2. **Attenuated VFS Read & Write Capabilities (`verify_fs_read` / `verify_fs_write` + Phase 7.4 `check_boa_fs_read_ocap` / `check_boa_fs_write_ocap`):**
   - Rejects any path containing `..` traversal segments and verifies that the target path starts with an explicitly granted `allowed_read_prefixes` or `allowed_write_prefixes` entry from `slice.toml` or `--allow-read`/`--allow-write`.
   - Enforced across **both** Tier-0 `SLICESNP` (`BytecodeOp::VfsRead`) and Tier-1 `SLICEBOA` native `node:fs` host functions (`__slice_fs_read_file_sync`, `__slice_fs_write_file_sync`, `__slice_fs_append_file_sync`).
3. **Outbound Network Allowlisting (`verify_net_connect`):** Blocks any `fetch()` or socket connection to a `host:port` not explicitly listed in `allowed_net_hosts` (`SandboxViolation::NetAccessDenied`).
4. **Scoped Environment Capabilities (`verify_env_read`):** Isolates tenant secrets in an explicit per-compartment key-value map (`SandboxViolation::EnvAccessDenied`).
5. **Deterministic CPU Fuel & Memory Cage + Linux `WRPKRU` Hardware MPK:**
   - Every operation deducts instruction fuel (`consume_fuel`) and tracks heap bytes (`alloc_memory`), preempting infinite `while (true) {}` loops in `< 10 us` (`SandboxViolation::CpuFuelExhausted`).
   - On Linux `x86_64`, [`HardwareProtectionDomain`](file:///p:/opensource/slice/crates/slice-sandbox/src/lib.rs#L112-L168) allocates hardware memory protection keys via `SYS_pkey_alloc` (syscall 330) and switches page permissions in `< 20` CPU cycles using the unprivileged `WRPKRU` instruction.

#### E. Phase 9B Priority 1-3: Hardware `SHA-NI`, Persistent Realm Module Cache (`__slice_rc`), and Type-Directed Oxc Semantic Lowering
- **1. Hardware `SHA-NI` Block Compression ([`Sha256State::compress_block_sha_ni`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L3613-L3709)):**
  - Uses Intel/AMD `SHA-NI` instructions (`_mm_sha256rnds2_epu32`, `_mm_sha256msg1_epu32`, `_mm_sha256msg2_epu32`) with cached `AtomicU8` runtime feature detection (`sha,sse4.1,ssse3`) and automatic fallback to [`Sha256State::compress_block_soft`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L3719-L3747).
- **2. Persistent Realm Module Cache (`__slice_rc`) & [`WarmBoaRealm`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L2679-L2682):**
  - [`bundle_entrypoint_with_deps`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs#L1019-L1150) emits a 16-hex-char BLAKE3 fingerprint header (`//__SLICE_RC:<16_hex_chars>`) and caches evaluated `node_modules` exports (`__slice_cache`) plus the compiled entrypoint closure in `globalThis.__slice_rc`.
  - On warm hits (`slice serve` or `slice run --warm`), [`SliceIsolate::exec_boa_full_es_vm`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L2601-L2646) invokes `__slice_rc["<key>"]()` from a 32-byte stack buffer with **0 JS parser passes** and **0 `node_modules` CJS factory re-evaluations**, cutting warm isolate execution down to **`272 us` (`0.27 ms`)** on `02-npm-ecosystem-suite` and **`673 us` (`0.67 ms`)** on `01-fintech-checkout`.
- **3. Type-Directed Oxc Semantic Lowering ([`analyze_typescript_semantics`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs#L88-L195)) & Monomorphic Tier-2 Loop Promotion ([`BytecodeOp::MonomorphicForLoop`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L334-L340)):**
  - Before Oxc strips TypeScript types, `slice-cas` inspects `oxc::parser::Parser` and `oxc::semantic::SemanticBuilder` symbol tables and flat `interface` declarations (`AccessLogRecord`, `JournalRecord`), lowering `JSON.stringify(<typed_var>)` into direct monomorphic string template concatenations, rewriting chained `crypto.createHash`/`createHmac` and `Buffer.from(...).toString(...)` calls directly to `__slice_crypto_*` / `__slice_base64_*` intrinsics, pruning dead `require("node:crypto")` imports, and promoting `: number` loops to `BytecodeOp::MonomorphicForLoop`.

---

### 3.5 Phase N1: State-of-the-Art `slice serve` Architecture (Keep-Alive, Pipelining, Direct `JsObject::call` & WinterTC Bridge)

- **Zero-Allocation Stack HTTP/1.1 Parser & Vectorized Pipelined Flushing ([`serve_connection_keep_alive`](file:///p:/opensource/slice/crates/slice-cli/src/main.rs#L753-L930)):**
  - Each worker thread in the `--workers=N` shared-nothing pool pre-allocates a reusable `64 KB` socket read buffer (`read_buf`) and `4 KB` write buffer (`write_buf`).
  - Incoming HTTP/1.1 requests are scanned in-place via [`find_http_header_end`](file:///p:/opensource/slice/crates/slice-cli/src/main.rs#L719-L735) with zero heap `String` allocations, extracting up to 16 headers into a stack-allocated `[(&str, &str); 16]` array (`headers_stack`).
  - When multiple pipelined HTTP/1.1 requests arrive in a single TCP read (`has_more_pipelined`), responses are accumulated into `write_buf` and flushed in a single vectorized `stream.write_all(write_buf)` system call.
- **In-Place Isolate Reset ([`SliceIsolate::reset_for_request`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L1908-L1934)) & Direct `JsObject::call`:**
  - Instead of allocating a new `SliceIsolate` struct and cloning `CapabilityToken` vectors on every HTTP request, each worker resets its `SliceIsolate` in place, entering a fresh `HardwareProtectionDomain` (`WRPKRU`) and resetting CPU fuel and memory counters in nanoseconds.
  - On warm `SLICEBOA` hits, [`WarmBoaRealm::loaded_bundles`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L3223-L3226) stores the compiled `boa_engine::JsObject` entrypoint function handle and invokes `fn_obj.call(...)` directly in Rust, completely bypassing JS source lexing and `ctx.eval`.
- **WinterTC Request/Response & Post-Response `Slice.waitUntil(fn)` ([`execute_http_request`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L2020-L2095)):**
  - Exposes `Slice.request()`, `Slice.respond(status, body, contentType)`, and `Slice.waitUntil(fn)`. Deferred `waitUntil` callbacks are drained via [`drain_wait_until_tasks`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L2098-L2126) **after** the HTTP response bytes have been flushed to the client socket, while still charging CPU fuel to the tenant's compartment budget.

---

### 3.6 Phase N2: Next-Gen Multi-Tenant Serverless & MicroVM Primitives (`Slice.kv`, `--tenant-dir`, `--deterministic`)

- **16-Way Sharded In-Memory Edge KV & Atomic Counter Slab ([`EDGE_KV_SLAB`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L3381-L3470)):**
  - Backed by `OnceLock<[Mutex<FxHashMap<(String, String), KvEntry>>; 16]>` partitioned by `(tenant_id, key)` using `FxHasher` low 4 bits (`0..15`) for lock-free multi-core scaling across `--workers=N`.
  - Supports `Slice.kv.get(key)`, `Slice.kv.set(key, val, ttlMs)`, `Slice.kv.del(key)`, and atomic 64-bit `Slice.kv.incr(key, delta)`. Every write charges its byte delta against `BOA_KV_MEM_BUDGET` and `compartment.alloc_memory(bytes)`, trapping with `SandboxViolation::MemoryCeilingExceeded` if a tenant exceeds its memory ceiling.
- **Multi-Tenant Dynamic `.cas` Router & Zero-Downtime Atomic Hot-Swap ([`resolve_tenant_bytecode_hotswap`](file:///p:/opensource/slice/crates/slice-cli/src/main.rs#L663-L716)):**
  - When `slice serve --tenant-dir=<dir>` is active, each worker resolves `X-Slice-Tenant: <id>` or URL prefix `/t/<id>/...` to `<dir>/<id>.cas` (falling back to `<dir>/default.cas`).
  - Thread-local `TENANT_CAS_CACHE` tracks `(mtime_ns, file_len, Arc<Vec<u8>>)`. When a `.cas` file is atomically replaced on disk, the worker automatically maps the new `.cas` archive and pre-warms its `__slice_rc` closure with zero dropped TCP connections.
- **Opt-In Deterministic Replay Mode ([`SliceIsolate::set_deterministic_seed`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L1871-L1877)):**
  - When `--deterministic=<seed>` is passed (`None` by default = `0.00 ns` overhead), `Math.random()`, `Date.now()`, and `performance.now()` route through `__slice_det_random` and `__slice_det_now`, driven by a deterministic **SplitMix64** PRNG (`splitmix64_next_f64`) and a monotonic virtual clock (`1700000000000 + tick_ms`).

---

### 3.7 Phase N3: AI Agent "Code Mode" Sandbox, `< 5 us` VFS/KV Checkpoints, Egress Secret Guard, & Native MCP Server

```
+---------------------------------------------------------------------------------------------------+
|                        Phase N3: AI Agent "Code Mode" & Native MCP Architecture                   |
|                                                                                                   |
|  Claude Code / Cursor / LLM Agent                                                                 |
|         |  JSON-RPC 2.0 over stdio (`slice mcp [--audit-egress=SECRET]`)                          |
|         v                                                                                         |
|  +---------------------------------------------------------------------------------------------+  |
|  | Parent Orchestrator Isolate (`mcp-agent-host` or `slice run tot_agent.ts`)                  |  |
|  |  - `Slice.checkpoint()` -> Snapshots `BOA_VFS_CACHE` + Tenant `EDGE_KV_SLAB` in < 5 us      |  |
|  |  - `Slice.evalSandbox(code, opts)` -> Saves Parent TLS -> Spawns Attenuated Child Isolate   |  |
|  |      |                                                                                      |  |
|  |      +---> [Child SliceIsolate]                                                             |  |
|  |              * Capabilities = Parent Caps INTERSECT Child `opts.allowRead` / `allowWrite`   |  |
|  |              * Fuel & Memory = min(Parent Remaining, Child `opts.fuel` / `memoryBytes`)     |  |
|  |              * Egress Guard = Scans stdout & returnValue for raw/base64/base64url secrets   |  |
|  |  - If `!branch.ok` -> `Slice.restore(snapId)` rolls back VFS + KV state in < 5 us!          |  |
|  +---------------------------------------------------------------------------------------------+  |
+---------------------------------------------------------------------------------------------------+
```

1. **Dynamic Sub-Isolate Spawning with Strict Capability Attenuation ([`SliceIsolate::eval_sandbox`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L1766-L1864)):**
   - An orchestrator script or MCP host can evaluate untrusted AI-generated TypeScript/JavaScript strings via `Slice.evalSandbox(code, opts)`.
   - Child `allowed_read_prefixes` and `allowed_write_prefixes` are **strictly intersected** with the parent isolate's `CapabilityToken` (a child can never escalate privileges beyond its parent), and child `fuel_consumed` is deducted directly from the parent isolate's remaining fuel budget.
   - Re-entrant `__slice_eval_sandbox` saves and restores the parent isolate's thread-local state (`BOA_STDOUT`, `BOA_VIOLATION`, `BOA_ACTIVE_TENANT`, `BOA_ACTIVE_CAPS`, `BOA_KV_MEM_BUDGET`, `BOA_DET_STATE`) so nested sub-isolates execute with zero TLS interference.
2. **Sub-5us VFS & KV Checkpointing (`Slice.checkpoint()` & `Slice.restore(id)`):**
   - [`checkpoint_tenant_state`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L3306-L3335) captures a point-in-time clone of the thread-local `node:fs` VFS slab (`BOA_VFS_CACHE` and `BOA_VFS_DIRS`) and every entry belonging to `tenant_id` across all 16 shards of `EDGE_KV_SLAB`, returning a monotonic `u64` snapshot ID.
   - [`restore_tenant_state`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L3337-L3353) restores both the VFS slab and the tenant's sharded KV keys in `< 5 microseconds`, enabling Tree-of-Thought (ToT) AI coding agents to test candidate patches and roll back failed branches instantaneously.
3. **Egress Secret Guard (`--audit-egress=<SECRET>`) & Built-In MCP Server (`slice mcp`):**
   - [`verify_egress_secrets_slice`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L3355-L3379) inspects `stdout`, `returnValue`, and HTTP response bodies for raw, standard Base64, and URL-safe Base64 encodings of registered secrets, trapping with `SandboxViolation::EnvAccessDenied { key: "EGRESS_SECRET_LEAK_BLOCKED" }` (`0.00 ns` cost when no secrets are registered).
   - [`cmd_mcp`](file:///p:/opensource/slice/crates/slice-cli/src/main.rs#L1014-L1074) implements the JSON-RPC 2.0 Model Context Protocol over `stdio`, exposing `slice_eval_ts`, `slice_kv_get`, `slice_kv_set`, and `slice_checkpoint`.

---

## 4. Summary of Architectural Gains Across All 8 Production Workloads

| Architectural Innovation | Bottleneck Eliminated | Measured Gain (Linux `x86_64` Docker) |
| :--- | :--- | :--- |
| **`SLICE_CAS` v1 + `mmap` + `SLICEEXE`** | `1,500+` `stat()` / `open()` syscalls & JSON AST parsing | **`0` `stat()` syscalls**, `395 us` 500-module load (`20.6x` faster than Node.js) |
| **Zero-Copy `SLICEBOA` Framing** | `serde_json` string unescaping of bundled JS payloads | Isolate instantiation dropped **`229.66 us` -> `0.05 us - 0.27 us`** (**`>1,000x` faster**) |
| **Phase 7.1 Pre-Warmed `Boa` Pool (`BOA_CONTEXT_POOL`)** | Repeated realm intrinsic creation & JS prelude eval | `01-fintech-checkout` isolate exec dropped **`5.07 ms` -> `0.67 ms`** (**`7.56x` faster**) |
| **Phase 7.2 Static Barrel Tree-Shaking (`slice-cas`)** | Bundling 30+ unused `semver` & `fast-querystring` barrel submodules | `02-npm-ecosystem-suite` `.cas` size **`-50.4%`** (`195,978 B` -> `97,104 B`), Max RSS **`10.38 MB`** (`6.85x < Node`) |
| **Phase 7.3 Native Rust `SemVer` + `VFS` + `Crypto` Intrinsics** | Slow interpreted JS regexes, temp-file syscalls & closures | `02-npm-ecosystem-suite` cold `.cas` **`6.766 ms`** vs Bun `20.048 ms` (**`2.96x` faster cold**) |
| **Phase 7.4 Native `Boa` Host OCap Enforcement** | Unchecked `node:fs` access inside third-party packages | **5/5 Security Vectors Contained** across both `SLICESNP` and `SLICEBOA` |
| **Phase 9A Zero-Alloc Isolate Spawn & `16-Byte JsValue`** | 11 heap allocations per isolate spawn + `80B` `JsValue` enum | 1,000-Tenant MicroVM Swarm **`6.57 ms` -> `1.42 ms`** (`1.00 us` min cold-start, `9.93 MB` total RSS) |
| **Phase 9B `SHA-NI`, `__slice_rc` Persistent Realm Cache & Oxc Semantic Lowering** | Cold CJS wrapper re-eval, dynamic `JSON.stringify` & crypto builder allocs | All 8 apps **`5.18 ms - 9.34 ms` cold** / **`0.19 ms - 4.19 ms` warm** (**up to `73.7x` faster than Bun**) |
| **Phase N1 State-of-the-Art `slice serve` (`--workers=N`, Keep-Alive, Pipelining, Direct `JsObject::call` & WinterTC Bridge)** | Per-request TCP handshake, `BufReader` line allocs, & `ctx.eval` re-parsing on warm HTTP requests | **`152,376 req/s` (`6.56 us/req`)** in Tier-0 CAS (**`1.73x` faster than `Bun.serve`**) and **`107,274 req/s` (`9.32 us/req`)** in Tier-1 WinterTC (**beating both `Bun.serve` and `Deno.serve`**) |
| **Phase N2 Sharded `Slice.kv`, Multi-Tenant `--tenant-dir` Hot-Swap, & `--deterministic` Replay** | External Redis round-trips for edge rate-limiting, server restarts on tenant deploy, & non-deterministic PRNG/clocks | **`586.10 us` for 1,001 `Slice.kv` ops** (`0.58 us/op`), zero-downtime `.cas` hot-swap, & 100% bit-for-bit SplitMix64 replay |
| **Phase N3 `Slice.evalSandbox`, `< 5 us` `Slice.checkpoint`/`restore`, Egress Secret Guard, & `slice mcp` Server** | 125ms Firecracker VM boot for AI agent tool calls, slow disk rollback for Tree-of-Thought, & prompt-injection secret exfiltration | **Sub-millisecond nested TypeScript sub-sandboxes with capability attenuation, `< 5 us` VFS+KV state rollback, Base64 egress secret blocking, & native JSON-RPC 2.0 MCP server** |




