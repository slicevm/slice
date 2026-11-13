# Pure-Rust WebAssembly, Capability-Attenuated Web Workers & Direct Kernel I/O

This document covers Slice's Pure-Rust WebAssembly engine ([`wasm.rs`](../../crates/slice-engine/src/runtime/wasm.rs) via `wasmi` `v2.0.0`), capability-attenuated MicroVM Web Workers and `node:worker_threads` ([`workers.rs`](../../crates/slice-engine/src/runtime/workers.rs)), direct kernel completion rings ([`slice-io`](../../crates/slice-io/src/lib.rs): Linux `io_uring` & Windows `IOCP`), and OS-native kernel event file watching ([`OsNativeFileWatcher`](../../crates/slice-cli/src/config/watcher.rs)).

---

## 1. Pure-Rust WebAssembly Engine (`wasmi` `v2.0.0`), `GLOBAL_WASM_MODULE_CACHE` & 5-Ring WASM Cage

Implemented in [`crates/slice-engine/src/runtime/wasm.rs`](../../crates/slice-engine/src/runtime/wasm.rs) and [`crates/slice-engine/src/preludes/09_wasm.js`](../../crates/slice-engine/src/preludes/09_wasm.js):

1. **Zero Overhead When Unused (`Shard 9`):**
   - `globalThis.WebAssembly` is registered as a self-replacing lazy getter (`__defLazy(["WebAssembly"], 9)` via `LAZY_GETTERS_INIT_PRELUDE`) and gated in `detect_shards_and_boacas_flags` so non-WASM workloads pay `0.00 ns` and `0 B`.
2. **Speculative `GLOBAL_WASM_MODULE_CACHE`:**
   - A process-wide `OnceLock<wasmi::Engine>` (`CompilationMode::LazyTranslation`, `consume_fuel(true)`) + `RwLock<FxHashMap<(u64, usize), wasmi::Module>>` validates and translates each `.wasm` binary at most once per process, enabling `< 15 us` warm module instantiation across isolates and worker threads.
3. **Zero-Stat `.wasm` Module Bundling (`slice-cas`):**
   - Static or dynamic `.wasm` imports (`import wasmBytes from "./kernel.wasm"`) are embedded directly into `.cas` archives and standalone `SLICEEXE` binaries as zero-stat `Uint8Array` payloads (`__slice_b64ToU8`).
4. **5-Ring WebAssembly Security & Resource Cage:**
   - **Ring 1 (Binary Validation):** Enforces `MAX_WASM_BINARY_BYTES = 8 MB` and WASM magic header (`\0asm\x01\0\0\0`) validation before compilation.
   - **Ring 2 (Unified CPU Fuel Metering):** Synchronizes `wasmi::Store::set_fuel` / `get_fuel` with the parent `SliceIsolate`'s `BOA_FUEL_BUDGET`, preempting infinite WASM loops deterministically with `SandboxViolation::CpuFuelExhausted`.
   - **Ring 3 (Unified Linear Memory Cage):** Enforces `MAX_WASM_MEMORY_PAGES = 256` (`16 MB`) and charges every WASM linear memory allocation or `memory.grow()` delta against the tenant's `BOA_KV_MEM_BUDGET` (`SandboxViolation::MemoryCeilingExceeded`).
   - **Ring 4 (Zero Ambient WASI Authority & Fuel-Metered Host Imports):** Exposes zero ambient OS/filesystem WASI descriptors by default while supporting fuel-metered JavaScript host function imports (`importObject.env.*`).
   - **Ring 5 (Lazy Linear Memory Mirroring & 8-Byte Word-Aligned `ArrayBuffer` Delta Sync):** Defers `WebAssembly.Memory.prototype.buffer` allocation until JavaScript explicitly accesses `.buffer`, synchronizes modifications between JS `ArrayBuffer` views (`Uint8Array` / `Int32Array`) and `wasmi::Memory` using 8-byte `u64` word-aligned delta scans only after `.buffer` has been materialized, and inspects all outputs through the Outbound Egress Secret DLP Firewall.

---

## 2. Capability-Attenuated MicroVM Web Workers & `node:worker_threads` (`workers.rs`)

Implemented in [`crates/slice-engine/src/runtime/workers.rs`](../../crates/slice-engine/src/runtime/workers.rs) and [`crates/slice-engine/src/preludes/08_workers_sockets.js`](../../crates/slice-engine/src/preludes/08_workers_sockets.js):

- **Standards-Compliant Surface:** Supports both WhatWG `new Worker(urlOrInlineSource, options)` (`postMessage`, `onmessage`, `terminate`), `MessageChannel`, `MessagePort`, `BroadcastChannel`, and Node.js `node:worker_threads` (`Worker`, `isMainThread`, `parentPort`, `workerData`, `threadId`).
- **Strict Parent-Child OCap Intersection ([`build_attenuated_worker_caps`](../../crates/slice-engine/src/runtime/workers.rs)):**
  - A child worker's `CapabilityToken` is strictly intersected with its parent isolate's capabilities (`parent_caps` intersect `child_requested_caps`). A worker can never escalate filesystem, network, or environment privileges beyond its parent compartment, and worker CPU fuel and memory budgets are capped by the parent's remaining limits.
- **Persistent Warm Worker Pool (`IDLE_WORKER_POOL`) & `WORKER_SCRIPT_CACHE`:**
  - Rather than spawning a fresh OS thread and initializing a cold `Boa` realm per `new Worker()`, Slice maintains a warm `IDLE_WORKER_POOL` with speculative pre-warming (`speculative_prewarm_worker_pool`) and caches transpiled worker scripts by `BLAKE3` fingerprint in `WORKER_SCRIPT_CACHE` (`__SLICE_RC`), keeping multi-worker cold and warm execution latency low (see **[`tests/reports/latest.md`](../../tests/reports/latest.md)** for `22-worker-threads-and-raw-sockets` benchmarks).

---

## 3. Direct Kernel Completion Rings (`crates/slice-io`) & OS-Native File Watcher

### 3.1 [`crates/slice-io`](../../crates/slice-io/src/lib.rs): `io_uring`, `IOCP`, & `RegisteredBufferPool` 64 KB Fetch Slabs
- **Registered Slab Buffers (`RegisteredBufferPool`):** Pre-allocates 64 page-aligned fixed `64 KB` buffers (`FIXED_BUFFER_COUNT = 64`, `FIXED_BUFFER_SIZE = 65536`) with O(1) slot acquisition (`acquire_slot` / `buffer_mut`) and release (`release`). Integrated directly into `outbound_http::send_and_read_framed_http11` (`SHARED_FETCH_SLAB_POOL` + `FetchSlabSlotGuard`) and `event_loop::FetchReactorPool` (`MAX_FETCH_REACTOR_WORKERS = 16`) for zero-leak concurrent `fetch()` staging.
- **Linux `io_uring` (`CompletionRingEngine`):** Attempts kernel ring setup with `IORING_SETUP_SQPOLL` (`IoUring::builder().setup_sqpoll(1000)`), registers pinned `64 KB` slab `iovec` buffers via `IORING_REGISTER_BUFFERS` to emit `IORING_OP_READ_FIXED` / `IORING_OP_WRITE_FIXED` zero-copy DMA opcodes (falling back gracefully when `RLIMIT_MEMLOCK` or unprivileged Docker seccomp restricts registration).
- **Windows NT `IOCP`:** Binds directly to `CreateIoCompletionPort`, `PostQueuedCompletionStatus`, and batched `GetQueuedCompletionStatusEx` via `kernel32.dll` FFI, reaping up to `64` completions per syscall and processing `10,000` completions in sub-millisecond batches.

### 3.2 OS-Native Kernel Event File Watcher ([`config/mod.rs`](../../crates/slice-cli/src/config/mod.rs) & [`OsNativeFileWatcher`](../../crates/slice-cli/src/config/watcher.rs))
- Uses the cross-platform `notify` (`v8.2`) crate (`inotify` on Linux, `ReadDirectoryChangesW` on Windows, `FSEvents`/`kqueue` on macOS) blocking directly on kernel notifications (`rx.recv()`) with zero idle wakeups and zero idle `stat` walks in `--watch` mode (`15 ms` save-burst debounce), while `slice serve --tenant-dir=<dir>` combines watcher generation tracking with a `250 ms` `stat` TTL for zero-syscall warm request routing and instant `.cas` hot-swaps.
