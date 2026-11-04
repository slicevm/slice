# Pure-Rust WebAssembly, Capability-Attenuated Web Workers & Direct Kernel I/O

This document covers Slice's Pure-Rust WebAssembly engine (`wasm.rs` via `wasmi` `v0.40`), capability-attenuated MicroVM Web Workers and `node:worker_threads` (`workers.rs`), direct kernel completion rings (`slice-io`: Linux `io_uring` & Windows `IOCP`), and OS-native kernel event file watching (`OsNativeFileWatcher`).

---

## 1. Pure-Rust WebAssembly Engine (`wasmi` `v0.40`), `GLOBAL_WASM_MODULE_CACHE` & 5-Ring WASM Cage

Implemented in [`crates/slice-engine/src/wasm.rs`](file:///p:/opensource/slice/crates/slice-engine/src/wasm.rs) and [`crates/slice-engine/src/preludes/09_wasm.js`](file:///p:/opensource/slice/crates/slice-engine/src/preludes/09_wasm.js):

1. **Zero Overhead When Unused (`Shard 9`):**
   - `globalThis.WebAssembly` is registered as a self-replacing lazy getter (`__defLazy(["WebAssembly"], 9)` in `00_ultra_core.js`) and gated in `detect_needed_shards` so non-WASM workloads pay `0.00 ns` and `0 B`.
2. **Speculative `GLOBAL_WASM_MODULE_CACHE`:**
   - A process-wide `OnceLock<wasmi::Engine>` (`CompilationMode::LazyTranslation`, `consume_fuel(true)`) + `RwLock<FxHashMap<(u64, usize), wasmi::Module>>` validates and translates each `.wasm` binary at most once per process, enabling `< 15 us` warm module instantiation across isolates and worker threads.
3. **Zero-Stat `.wasm` Module Bundling (`slice-cas`):**
   - Static or dynamic `.wasm` imports (`import wasmBytes from "./kernel.wasm"`) are embedded directly into `.cas` archives and standalone `SLICEEXE` binaries as zero-stat `Uint8Array` payloads (`__slice_b64ToU8`).
4. **5-Ring WebAssembly Security & Resource Cage:**
   - **Ring 1 (Binary Validation):** Enforces `MAX_WASM_BINARY_BYTES = 8 MB` and WASM magic header (`\0asm\x01\0\0\0`) validation before compilation.
   - **Ring 2 (Unified CPU Fuel Metering):** Synchronizes `wasmi::Store::set_fuel` / `get_fuel` with the parent `SliceIsolate`'s `BOA_FUEL_BUDGET`, preempting infinite WASM loops deterministically with `SandboxViolation::CpuFuelExhausted`.
   - **Ring 3 (Unified Linear Memory Cage):** Enforces `MAX_WASM_MEMORY_PAGES = 256` (`16 MB`) and charges every WASM linear memory allocation or `memory.grow()` delta against the tenant's `BOA_KV_MEM_BUDGET` (`SandboxViolation::MemoryCeilingExceeded`).
   - **Ring 4 (Zero Ambient WASI Authority & Fuel-Metered Host Imports):** Exposes zero ambient OS/filesystem WASI descriptors by default while supporting fuel-metered JavaScript host function imports (`importObject.env.*`).
   - **Ring 5 (8-Byte Word-Aligned `ArrayBuffer` Delta Sync & Egress DLP):** Synchronizes modifications between JavaScript `WebAssembly.Memory.prototype.buffer` (`Uint8Array` / `Int32Array` views) and `wasmi::Memory` using 8-byte `u64` word-aligned delta scans, and inspects all outputs through the Outbound Egress Secret DLP Firewall.

---

## 2. Capability-Attenuated MicroVM Web Workers & `node:worker_threads` (`workers.rs`)

Implemented in [`crates/slice-engine/src/workers.rs`](file:///p:/opensource/slice/crates/slice-engine/src/workers.rs) and [`crates/slice-engine/src/preludes/08_workers_sockets.js`](file:///p:/opensource/slice/crates/slice-engine/src/preludes/08_workers_sockets.js):

- **Standards-Compliant Surface:** Supports both WhatWG `new Worker(urlOrInlineSource, options)` (`postMessage`, `onmessage`, `terminate`), `MessageChannel`, `MessagePort`, `BroadcastChannel`, and Node.js `node:worker_threads` (`Worker`, `isMainThread`, `parentPort`, `workerData`, `threadId`).
- **Strict Parent-Child OCap Intersection ([`build_attenuated_worker_caps`](file:///p:/opensource/slice/crates/slice-engine/src/workers.rs)):**
  - A child worker's `CapabilityToken` is strictly intersected with its parent isolate's capabilities (`parent_caps ∩ child_requested_caps`). A worker can never escalate filesystem, network, or environment privileges beyond its parent compartment, and worker CPU fuel and memory budgets are capped by the parent's remaining limits.
- **Persistent Warm Worker Pool (`IDLE_WORKER_POOL`) & `WORKER_SCRIPT_CACHE`:**
  - Rather than spawning a fresh OS thread and initializing a cold `Boa` realm (`~15 ms`) per `new Worker()`, Slice maintains a warm `IDLE_WORKER_POOL` with speculative pre-warming (`speculative_prewarm_worker_pool`) and caches transpiled worker scripts by `BLAKE3` fingerprint in `WORKER_SCRIPT_CACHE` (`__SLICE_RC`), driving warm multi-worker execution (`App 22`) down to **`2.00 ms`** (`28.08 ms` cold vs Bun's `44.24 ms`).

---

## 3. Direct Kernel Completion Rings (`crates/slice-io`) & OS-Native File Watcher

### 3.1 [`crates/slice-io`](file:///p:/opensource/slice/crates/slice-io/src/lib.rs): `io_uring` & `IOCP`
- **Registered Slab Buffers (`RegisteredBufferPool`):** Pre-allocates `64 KB` page-aligned fixed buffers (`FIXED_BUFFER_SIZE = 65536`) with O(1) slot acquisition (`acquire_slot`) and release (`release_slot`) for zero-copy I/O transfers.
- **Linux `io_uring` (`CompletionRingEngine`):** Attempts kernel ring setup with `IORING_SETUP_SQPOLL` (`IoUring::builder().setup_sqpoll(1000)`), enabling autonomous kernel thread submission polling with **0 system calls** on the hot path, and automatically falls back to standard `io_uring` or user-space completion rings inside unprivileged Docker seccomp profiles.
- **Windows NT `IOCP`:** Binds directly to `CreateIoCompletionPort`, `PostQueuedCompletionStatus`, and `GetQueuedCompletionStatus` via `kernel32.dll` FFI, processing `10,000` completions in `0.02 ms - 5.26 ms`.

### 3.2 OS-Native Kernel Event File Watcher ([`OsNativeFileWatcher`](file:///p:/opensource/slice/crates/slice-cli/src/config_and_watch.rs))
- Uses the cross-platform `notify` (`v6.1`) crate (`inotify` on Linux, `ReadDirectoryChangesW` on Windows, `FSEvents`/`kqueue` on macOS) with a `15 ms` debounce window and automatic fingerprint fallback (`collect_watch_fingerprint`) to power zero-idle-CPU live reload (`4.92 ms` rebuild+rerun) across `slice run --watch`, `slice test --watch` / `--watch-once`, and `slice serve --watch`.

