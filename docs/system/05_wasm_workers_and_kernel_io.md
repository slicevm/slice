# Pure-Rust WebAssembly, Capability-Attenuated Web Workers & Direct Kernel I/O

This document covers Slice's Pure-Rust WebAssembly engine ([`wasm/`](../../crates/slice-engine/src/runtime/wasm/mod.rs) via `wasmi` `v2.0.0`), capability-attenuated MicroVM Web Workers and `node:worker_threads` ([`workers.rs`](../../crates/slice-engine/src/runtime/workers.rs)), direct kernel completion rings ([`slice-io`](../../crates/slice-io/src/lib.rs): Linux `io_uring` & Windows `IOCP`), and OS-native kernel event file watching ([`OsNativeFileWatcher`](../../crates/slice-cli/src/config/watcher.rs)).

---

## 1. Pure-Rust WebAssembly Engine (`wasmi` `v2.0.0`), Shared Per-Isolate `Store` & 5-Ring WASM Cage

Implemented across [`crates/slice-engine/src/runtime/wasm/`](../../crates/slice-engine/src/runtime/wasm/mod.rs) ([`mod.rs`](../../crates/slice-engine/src/runtime/wasm/mod.rs), [`store.rs`](../../crates/slice-engine/src/runtime/wasm/store.rs), [`memory.rs`](../../crates/slice-engine/src/runtime/wasm/memory.rs), [`linking.rs`](../../crates/slice-engine/src/runtime/wasm/linking.rs), [`values.rs`](../../crates/slice-engine/src/runtime/wasm/values.rs)) and [`crates/slice-engine/src/preludes/09_wasm.js`](../../crates/slice-engine/src/preludes/09_wasm.js):

1. **Zero Overhead When Unused (`Shard 9`):**
   - `globalThis.WebAssembly` is registered as a self-replacing lazy getter (`__defLazy(["WebAssembly"], 9)` via `LAZY_GETTERS_INIT_PRELUDE`) and gated in `detect_shards_and_boacas_flags` so non-WASM workloads pay `0.00 ns` and `0 B`.
2. **Speculative `GLOBAL_WASM_MODULE_CACHE` & Shared Per-Isolate `wasmi::Store`:**
   - A process-wide `OnceLock<wasmi::Engine>` (`CompilationMode::LazyTranslation`, `consume_fuel(true)`) + `RwLock<FxHashMap<(u64, usize), wasmi::Module>>` validates and translates each `.wasm` binary at most once per process, enabling `< 15 us` warm module instantiation across isolates and worker threads.
   - All `WebAssembly.Instance`, `WebAssembly.Memory`, `WebAssembly.Global`, and `WebAssembly.Table` objects within an isolate turn share a single per-isolate `wasmi::Store<WasmStoreData>` (with `SavedIsolateContextGuard` isolation across nested `Slice.evalSandbox` calls and full teardown on `reset_thread_wasm_state()`). Imported and re-exported `Memory`, `Global`, `Table`, and exported Wasm functions share live underlying storage and preserve canonical JS wrapper identity (`instance.exports.g === global`, `instance.exports.memory === memory`, `instance.exports.t === table`).
3. **Zero-Stat `.wasm` Module Bundling (`slice-cas`):**
   - Static or dynamic `.wasm` imports (`import wasmBytes from "./kernel.wasm"`) are embedded directly into `.cas` archives and standalone `SLICEEXE` binaries as zero-stat `Uint8Array` payloads (`__slice_b64ToU8`).
4. **W3C WebAssembly JS-API 2.0 Semantics & Zero-Allocation Fast Call Path:**
   - Exported Wasm functions bind directly to native Rust `NativeFunction::from_copy_closure` closures backed by stack-allocated `SmallVec<[Val; 8]>`, eliminating JS argument arrays and string serialization on numeric (`i32`/`i64`/`f32`/`f64`) calls.
   - `ValType::I64` parameters, `WebAssembly.Global` `i64` values, and host `i64` returns apply ECMAScript `ToBigInt64` (`val.to_big_int64`) with exact two's-complement modulo-`2^64` wrapping for arbitrary-precision `BigInt`s and strict `TypeError` rejection on `Number`s.
   - Authentic `WebAssembly.CompileError`, `WebAssembly.LinkError`, and `WebAssembly.RuntimeError` instances are constructed directly against realm prototypes, while JS exceptions thrown inside host import callbacks propagate unmangled (`PENDING_HOST_JS_ERROR`).
5. **5-Ring WebAssembly Security & Resource Cage:**
   - **Ring 1 (Binary Validation):** Enforces `MAX_WASM_BINARY_BYTES = 8 MB` and WASM magic header (`\0asm\x01\0\0\0`) validation before compilation.
   - **Ring 2 (Unified Re-Entrant CPU Fuel Metering):** Synchronizes `wasmi::Store::set_fuel` / `get_fuel` at every JS-to-Wasm and Wasm-to-JS host callback boundary with the parent `SliceIsolate`'s `BOA_FUEL_BUDGET`, preempting infinite WASM loops deterministically with `SandboxViolation::CpuFuelExhausted`.
   - **Ring 3 (Unified Linear Memory Cage via `SliceWasmLimiter`):** Implements `wasmi::ResourceLimiter` on the shared `Store` to enforce `MAX_WASM_MEMORY_PAGES = 256` (`16 MiB`) and charge net linear memory growth (`desired - current`) against `BOA_KV_MEM_BUDGET` across both JS `memory.grow()` and native Wasm `(memory.grow ...)` instructions (`SandboxViolation::MemoryCeilingExceeded`), without double-charging imported `Memory` objects.
   - **Ring 4 (Zero Ambient WASI Authority & Fuel-Metered Host Imports):** Exposes zero ambient OS/filesystem WASI descriptors by default while supporting fuel-metered JavaScript host function imports (`importObject.<module>.*`) and direct native `wasmi::Func` cross-instance linking.
   - **Ring 5 (Lazy Linear Memory Mirroring, `ResetMemoryBuffer` Detachment & Post-Trap Coherence):** Defers `WebAssembly.Memory.prototype.buffer` allocation (created with `[[ArrayBufferDetachKey]] = "WebAssembly.Memory"`) until JavaScript explicitly accesses `.buffer`, synchronizes modifications between JS `ArrayBuffer` views and `wasmi::Memory` using 8-byte `u64` word-aligned delta scans across pre-call, host-callback, post-call, and post-trap (`unreachable`, out-of-bounds, `(start)` trap) boundaries, detaches stale `ArrayBuffer`s (`detached === true`, `byteLength === 0`) on both native Wasm `memory.grow` and JS `memory.grow(delta)` (including `delta === 0`), and inspects all outputs through the Outbound Egress Secret DLP Firewall.

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

### 3.1 [`crates/slice-io`](../../crates/slice-io/src/lib.rs): `io_uring`, `IOCP`, & `RegisteredBufferPool` 4096-Byte Page-Aligned Slabs
- **Registered Page-Aligned Slab Buffers (`RegisteredBufferPool`):** Allocates a single contiguous `4096`-byte page-aligned slab (`PAGE_ALIGNMENT = 4096`, `FIXED_BUFFER_COUNT = 64`, `FIXED_BUFFER_SIZE = 65536`) via `std::alloc::Layout::from_size_align` with O(1) LIFO slot acquisition (`acquire` / `acquire_slot` / `buffer_mut`), zeroed `release`, and `in_flight` DMA slot quarantine preventing CPU aliasing if a kernel submission batch fails mid-flight. Exposed safely on `CompletionRingEngine` via `buffer_pool()`, `buffer_pool_mut()`, and `with_pool_config(...)`, and integrated into `outbound_http::send_and_read_framed_http11` (`SHARED_FETCH_SLAB_POOL` + `FetchSlabSlotGuard`) and `event_loop::FetchReactorPool` (`MAX_FETCH_REACTOR_WORKERS = 16`).
- **Linux `io_uring` (`CompletionRingEngine`):** Attempts kernel ring setup with `IORING_SETUP_SQPOLL` (`IoUring::builder().setup_sqpoll(1000)`), registers pinned `64 KB` slab `iovec` buffers via `IORING_REGISTER_BUFFERS` (setting `RingStatus.registered_buffers` strictly from kernel registration success and calling `unregister_buffers()` on `Drop`), pre-checks SQ capacity before pushing SQEs, drains residual CQEs cleanly across batches, and falls back to positioned `libc::pread` / `libc::pwrite` when `io_uring` is restricted by unprivileged seccomp policies.
- **Windows NT `IOCP` & Overlapped Positioned File I/O:** Binds directly to `CreateIoCompletionPort`, `PostQueuedCompletionStatus`, batched `GetQueuedCompletionStatusEx`, and positioned `ReadFile` / `WriteFile` (`OVERLAPPED` 64-bit file offset + `GetOverlappedResult`) via `kernel32.dll` FFI, preserving lossless 64-bit `user_data` tags and signed `i32` byte/error results across `Nop`, `ReadFixed`, and `WriteFixed` batches.

### 3.2 OS-Native Kernel Event File Watcher ([`config/mod.rs`](../../crates/slice-cli/src/config/mod.rs) & [`OsNativeFileWatcher`](../../crates/slice-cli/src/config/watcher.rs))
- Uses the cross-platform `notify` (`v8.2`) crate (`inotify` on Linux, `ReadDirectoryChangesW` on Windows, `FSEvents`/`kqueue` on macOS) blocking directly on kernel notifications (`rx.recv()`) with zero idle wakeups and zero idle `stat` walks in `--watch` mode (`15 ms` save-burst debounce), while `slice serve --tenant-dir=<dir>` combines watcher generation tracking with a `250 ms` `stat` TTL for zero-syscall warm request routing and instant `.cas` hot-swaps.
