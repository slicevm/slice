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

---

## 4. Summary of Architectural Gains Across All 8 Production Workloads

| Architectural Innovation | Bottleneck Eliminated | Measured Gain (Linux `x86_64` Docker) |
| :--- | :--- | :--- |
| **`SLICE_CAS` v1 + `mmap` + `SLICEEXE`** | `1,500+` `stat()` / `open()` syscalls & JSON AST parsing | **`0` `stat()` syscalls**, `484 us` 500-module load (`20.6x` faster than Node.js) |
| **Zero-Copy `SLICEBOA` Framing** | `serde_json` string unescaping of bundled JS payloads | Isolate instantiation dropped **`229.66 us` -> `0.21 us`** (**`1,093x` faster**) |
| **Phase 7.1 Pre-Warmed `Boa` Pool (`BOA_CONTEXT_POOL`)** | Repeated realm intrinsic creation & JS prelude eval | `01-fintech-checkout` isolate exec dropped **`5.07 ms` -> `1.88 ms`** (**`2.70x` faster**) |
| **Phase 7.2 Static Barrel Tree-Shaking (`slice-cas`)** | Bundling 30+ unused `semver` & `fast-querystring` barrel submodules | `02-npm-ecosystem-suite` `.cas` size **`-49.1%`** (`195,978 B` -> `99,709 B`), Max RSS **`10.38 MB`** (`6.86x < Node`) |
| **Phase 7.3 Native Rust `SemVer` + `VFS` + `Crypto` Intrinsics** | Slow interpreted JS regexes, temp-file syscalls & closures | `02-npm-ecosystem-suite` warm isolate **`19.70 ms` -> `4.73 ms`** (**`4.16x` faster**; CLI **`7.705 ms`** vs Bun `20.568 ms`) |
| **Phase 7.4 Native `Boa` Host OCap Enforcement** | Unchecked `node:fs` access inside third-party packages | **5/5 Security Vectors Contained** across both `SLICESNP` and `SLICEBOA` |
| **Phase 9A Zero-Alloc Isolate Spawn & `16-Byte JsValue`** | 11 heap allocations per isolate spawn + `80B` `JsValue` enum | 1,000-Tenant MicroVM Swarm **`6.57 ms` -> `3.42 ms`** (`2.61 us` min cold-start, `9.79 MB` total RSS) |
| **Phase 9B Ultra-Compact Lazy Module Injection & CJS Constant Folding** | `~2.2 ms` cold `PRELUDE` parser tax + CJS regex/comment overhead | App 5 dropped to **`10.275 ms`**, App 8 dropped **`11.92 ms` -> `8.592 ms`** (**`1.82x` faster than Bun**) |

