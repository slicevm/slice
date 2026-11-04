# Cutting-Edge Research & Deep Dive: Building a Sub-Millisecond MicroVM-Grade Edge Runtime (2025-2026)

## 1. Executive Overview

This research document synthesizes the Slice (Nexus-JS) architectural blueprint with the 2025-2026 state-of-the-art in server-side JavaScript engines, microVM isolation, completion-based kernel I/O, and zero-stat content-addressable storage.

Our goal is to engineer **Slice (Nexus-JS)**: a microVM-grade, multi-tenant JavaScript/TypeScript edge runtime in Rust that achieves:
1. **Cold Start Latency:** `< 100 microseconds` (0.1 ms) in-process isolate instantiation; `< 2 ms` full CLI process execution.
2. **Baseline Memory Footprint:** `< 250 KB` per tenant isolate; `< 3.5 MB` total process RSS.
3. **Zero-Stat Module Resolution:** `0` filesystem metadata syscalls (`stat`/`lstat`/`readdir`) at runtime via a memory-mapped BLAKE3 Content-Addressed Storage (CAS) archive and O(1) lookup index.
4. **Object-Capability (OCap) & Hardware Sandboxing:** Intrinsic prototype lockdown (SES), per-tenant Compartment capability tokens, strict memory & CPU fuel ceilings, and Linux `pkey_alloc` / `WRPKRU` (Intel MPK) hardware memory domain protection.
5. **Direct Completion I/O:** Linux `io_uring` (with `SQPOLL` & registered buffers) and Windows `IOCP` completion rings.
6. **Tiered Polyglot Execution:** Instant Tier-0 bytecode/snapshot interpreter (`rquickjs` / QuickJS-NG or pure-Rust fast-path VM + Wasmtime JIT/AOT tier for compute-intensive functions).

---

## 2. Deep Dive: JavaScript & WebAssembly Execution Engines (2026 Landscape)

### 2.1 Comparison of Embeddable Engines in Rust

| Engine | Core Implementation | Cold Start (Isolate) | Base Memory | Compute Throughput | Trade-offs |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Google V8 (`v8` / `deno_core`)** | C++ | 5 ms - 40 ms | 20 MB - 50 MB | Very High (Maglev + TurboFan) | Heavy RSS, slow isolate spinup without huge snapshots, complex C++ build. |
| **Apple JSC (`bun`)** | C++ / Zig | 5 ms - 15 ms | 10 MB - 25 MB | Very High (LLInt + DFG + FTL) | Monolithic process model, no fine-grained in-process multi-tenant OCap isolation. |
| **QuickJS-NG (`rquickjs`)** | C + Rust | **80 us - 300 us** | **150 KB - 400 KB** | Moderate (Bytecode Interpreter) | Instant bytecode load (`eval_binary`), tiny heap, requires C compiler; weak on heavy loops unless paired with Native/Wasm JIT tier. |
| **Boa (`boa_engine`)** | Pure Rust | 300 us - 1.5 ms | 500 KB - 1.5 MB | Low-Moderate (Bytecompiler + VM) | 100% pure Rust (zero C/C++ build friction), great AST/bytecode control, slower raw JS loop speed. |
| **Wasmtime / Cranelift** | Pure Rust | **5 us - 50 us** (precompiled `.cwasm`) | **50 KB - 200 KB** | Near-Native (AOT/JIT Cranelift) | Mathematical SFI bounds + MPK hardware keys, deterministic fuel metering. |
| **Hermes V1 (Static Hermes)** | C++ | 200 us - 1 ms | 1 MB - 3 MB | High (Soundly typed AOT + Bytecode) | Optimized for mobile (React Native 0.84+), heavier FFI integration in Rust. |
| **Porffor** | JS/Wasm AOT | < 100 us | < 100 KB | High on scalar code | Experimental JS-to-Wasm AOT compiler, incomplete ES spec coverage. |

### 2.2 Architectural Synthesis for Quick Gains + Unbeatable Metrics

To win on **all metrics** immediately on both Windows (local) and Linux (Docker) while building toward the ultimate microVM runtime:
* **Build-Time Type Stripping & Bundling:** Use `oxc` (Oxidation Compiler) or zero-copy fast TypeScript stripping + static import flattening at package build time. Oxc strips TypeScript 3x-5x faster than SWC and 40x faster than `tsc`.
* **Pre-Compiled Bytecode & Snapshots:** Never parse JS/TS source strings on the hot path. During `slice pack`, source graphs are stripped of types, resolved statically, and compiled into binary bytecode + pre-initialized state tables inside a single immutable `.cas` blob.
* **Tiered Execution Core (Hybrid Engine):**
  * **Tier 0 (Sub-100us Instant Micro-Isolate):** Bytecode-compiled VM with pre-allocated arena/slab memory and frozen intrinsics (SES `lockdown()`).
  * **Tier 1 / Tier WASI (Native/Wasm JIT Acceleration):** Compute-intensive modules and hot loops are compiled to native machine code / precompiled WebAssembly (`.cwasm`) or SIMD-accelerated Rust host intrinsics (JSON, Crypto, Hashing, Routing, Regex), achieving both `< 100us` cold start AND native C/Rust throughput that beats V8/Bun on real-world server workloads.

---

## 3. Deep Dive: MicroVM Isolation vs. Language/Hardware Sandboxes

### 3.1 The 2026 Isolation Spectrum

1. **Traditional MicroVMs (Firecracker, Cloud Hypervisor):**
   * Boot a stripped Linux kernel via KVM.
   * Cold start: `100 ms - 300 ms`. Memory overhead: `30 MB - 128 MB` per VM.
2. **Hypervisor-Backed Unikernel/Wasm MicroVMs (Microsoft Hyperlight):**
   * Uses KVM (Linux) or WHP/MSHV (Windows Hypervisor Platform) to run a single-purpose microVM with **zero guest OS** and **zero POSIX syscall layer**.
   * Cold start: `0.9 ms - 2 ms`. Memory overhead: `1 MB - 4 MB`.
3. **In-Process Hardware-Enforced Micro-Isolates (Slice / Nexus-JS Architecture):**
   * Combines three concentric rings of defense inside a single shared-nothing host process:
     * **Ring 1 (Language OCap / SES):** Frozen intrinsics (`Object.freeze(Object.prototype)`), isolated global lexical compartments, zero ambient authority (`process`, `fs`, `net` removed from root global; only injected via cryptographic/attenuated Capability Tokens).
     * **Ring 2 (Resource & Preemption Cage):** Deterministic instruction fuel metering (preempts infinite `while(true)` loops in `< 1 ms`) + custom per-tenant memory allocator with hard byte-level memory caps (prevents OOM DoS).
     * **Ring 3 (Hardware Memory Protection Keys & Seccomp):** On Linux x86_64, uses `pkey_alloc()`, `pkey_mprotect()`, and the unprivileged `WRPKRU` instruction (Intel MPK) to switch tenant memory page permissions in **< 20 CPU cycles** (~5 nanoseconds) without a kernel syscall, backed by strict `seccomp-bpf` syscall filtering. On Windows, maps onto VirtualAlloc memory protection domains / Job Objects.

---

## 4. Deep Dive: Zero-Stat Content-Addressable Storage (CAS)

### 4.1 Why Node.js and Deno Lose on Cold-Start I/O
When Node.js resolves `import { Hono } from 'hono'`, it issues dozens of `stat()`, `lstat()`, `readdir()`, and `open()` calls walking up filesystem trees. Even Bun still relies on OS filesystem metadata when loading unbundled graphs.

### 4.2 The Slice Zero-Stat CAS Format (`SLICE_CAS_V1`)
1. **Ingestion (`slice pack`):**
   * Traverses the module graph once at build time.
   * Strips TypeScript types and rewrites CommonJS `require()` into static deterministic linkages.
   * Hashes every module's canonicalized bytecode/source with **BLAKE3-256** (achieving > 4 GB/s hashing speed via SIMD).
2. **Binary Layout (64-Byte Cache-Line Aligned):**
   * **Header:** Magic bytes (`SLICECAS`), version, entry-point hash, total module count, radix/index table offset, data payload offset.
   * **Deterministic Radix / Perfect Hash Index:** Maps `(Issuer Context, Specifier) -> BLAKE3 Hash -> (Page-Aligned Offset, Length, Precompiled Bytecode Offset)`.
   * **Payload Region:** Contiguous, 64-byte aligned module blobs.
3. **Runtime Activation (`mmap`):**
   * At runtime, the `.cas` file is mapped into virtual memory via a **single** `mmap(MAP_SHARED)` call on Linux (or `CreateFileMapping` / `MapViewOfFile` on Windows via `memmap2`).
   * **Zero `stat` calls. Zero `read` calls. Zero heap copies.** Multiple worker threads or processes share the exact same physical RAM pages in the OS page cache.

---

## 5. Deep Dive: Direct Kernel Completion Rings (`io_uring` & `IOCP`)

### 5.1 Replacing `libuv` Readiness Polling
* **Linux (`io_uring`):** Uses Submission Queue (SQ) and Completion Queue (CQ) ring buffers shared between user space and kernel space. With `IORING_SETUP_SQPOLL` and `io_uring_register_buffers`, I/O operations require **zero syscalls** on the hot path and zero intermediate buffer copies.
* **Windows (`IOCP`):** Windows NT was designed from inception around I/O Completion Ports (`CreateIoCompletionPort`, `GetQueuedCompletionStatusEx`), which is conceptually identical to `io_uring` completions.
* **Slice Unified Completion Engine:**
  * Implements a unified Rust completion driver abstracting Linux `io_uring` (via `io-uring` crate on Linux) and Windows `IOCP` / non-blocking completion loops on Windows, with pre-registered fixed buffer pools (`SlabBufferPool`) that expose zero-copy `Uint8Array` views to tenant compartments.

