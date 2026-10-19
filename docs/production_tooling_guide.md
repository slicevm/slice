# Slice (Nexus-JS): Production Tooling & Deployment Guide

## 1. Overview of the Production Toolchain

Slice `v0.2.0` provides a complete developer and production deployment workflow packaged inside a single self-contained native Rust executable (`slice`, embedding the **Oxc** TypeScript compiler, transitive `node_modules` bundler, **Boa** ES2024+ VM, and **Slice** Zero-Stat CAS + Tier-0/Tier-2 engine):

1. **Project Initialization & Policy Configuration (`slice init` & `slice.toml`)**
2. **Zero-Stat Content-Addressed Packaging (`slice pack` for single files or full `package.json` + `node_modules` folders)**
3. **Standalone Single-Binary Compilation (`slice compile`)**
4. **Bytecode & CAS Radix Disassembly (`slice inspect`)**
5. **Multi-Tenant WinterTC HTTP MicroVM Hosting (`slice serve`)**
6. **Interactive SES Compartment Shell (`slice repl`)**
7. **Kernel & Security Readiness Auditing (`slice doctor`)**

---

## 2. Workflow 1: Zero-Stat CAS Packaging vs. Standalone Executables

### 2.1 Packing a Folder Application or Module Graph (`slice pack`)
When deploying a multi-file TypeScript project (including third-party `node_modules/` dependencies) to a multi-tenant edge cluster:
```bash
# Pack an entire project folder (resolves slice.toml / package.json / src/index.ts + node_modules)
slice pack ./my-edge-app ./service.cas

# Or pack a directory with an explicit entrypoint
slice pack ./src ./service.cas --entry=./index.ts
```
* Strips all TypeScript type annotations at build time via **Oxc (`v0.152.0`)** and links transitive ESM (`import`/`export`) and CommonJS (`require`/`module.exports`) modules from `src/` and `node_modules/`.
* Compiles every module into `SLICESNP` or zero-copy `SLICEBOA` binary bytecode.
* Computes a 256-bit **BLAKE3** hash over the canonicalized source and bytecode.
* Aligns every blob to a **64-byte CPU cache line** boundary.
* At runtime (`slice run ./service.cas`), the file is mapped via `mmap(MAP_SHARED)` with **0 `stat()` syscalls** (`0.23 us` isolate instantiation).

### 2.2 Compiling a Self-Contained Binary (`slice compile`)
When deploying to minimal container images (`FROM scratch` or `distroless`) or distributing CLI tools:
```bash
slice compile ./my-edge-app -o ./my-service
```
* Appends the 64-byte aligned `.cas` archive directly to the `slice` runtime executable followed by a 24-byte `SLICEEXE` trailer (`[u64 cas_offset][u64 cas_len][b"SLICEEXE"]`).
* When `./my-service` boots, it maps its own executable image via `mmap`, locates the `SLICEEXE` trailer in O(1) time, and executes the embedded `SLICESNP`/`SLICEBOA` snapshot without extracting any temporary files to disk.

---

## 3. Workflow 2: Multi-Tenant HTTP MicroVM Server (`slice serve`)

`slice serve` runs an edge HTTP server where **every incoming HTTP request** executes inside a freshly spawned, hardware- and OCap-isolated `SliceIsolate`:

```bash
slice serve ./service.cas --port=3000
```

### Per-Request Isolation Lifecycle
1. Parses the `X-Slice-Tenant` HTTP header (or assigns a deterministic slot ID).
2. Allocates a `TenantCompartment` with frozen SES intrinsics (`Object.prototype` locked down), attenuated `CapabilityToken` permissions from `slice.toml`, deterministic CPU fuel limits, and an isolated `PKEY` memory protection domain.
3. Enters the hardware protection domain (`WRPKRU` on Linux `x86_64`).
4. Executes the pre-compiled `SLICESNP` bytecode directly from the shared `mmap` region.
5. Emits execution telemetry headers on the HTTP response:
   * `X-Slice-Tenant`: Active tenant compartment identifier.
   * `X-Slice-Isolate-Us`: Isolate instantiation + execution latency in microseconds.
   * `X-Slice-Tier`: Highest execution tier reached (`Tier0LLIntSnapshot`, `Tier1Baseline`, or `Tier2PeakNative`).
   * `X-Slice-Fuel-Used`: Deterministic instruction fuel consumed by the request.
   * `X-Slice-Pkey-Domain`: Active hardware protection key domain.

---

## 4. Workflow 3: Bytecode Disassembly & CAS Auditing (`slice inspect`)

Use `slice inspect` to verify that hot loops and cryptographic routines are lowered to Tier-2 native SIMD opcodes and that all OCap gates are present:

```
=========================================================================
  SLICESNP BYTECODE DISASSEMBLY: .\benchmarks\workloads\cold_start.ts (189 bytes)
=========================================================================
  0000  CONST_NUM       %a <- 40
  0001  CONST_NUM       %b <- 2
  0002  ADD_F64         %result <- %a + %b
  0003  STDOUT_EMIT     print(%result)
=========================================================================
```

---

## 5. Workflow 4: Host Kernel Diagnostics (`slice doctor`)

Before deploying to production edge nodes, run `slice doctor` to verify:
* **Completion Ring Backend:** Confirms whether `Linux io_uring (SQPOLL + Fixed Buffers)` or `Windows NT IOCP` is active.
* **Registered Slab Buffers:** Verifies pre-allocated `64 x 64 KB` (`4 MB`) zero-copy I/O slab buffers.
* **Hardware Memory Protection Keys:** Reports the active `PKEY` slot and `PKRU` bitmask (`0xffffffcf`).
* **Security Containment Audit:** Executes all 5 multi-tenant attack vectors (prototype pollution, unauthorized `/etc/shadow` read, outbound network exfiltration, secret `env` harvesting, and `while(true)` CPU DoS) and verifies 5/5 traps.

