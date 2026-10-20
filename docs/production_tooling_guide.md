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

## 3. Workflow 2: State-of-the-Art Multi-Tenant HTTP MicroVM Server (`slice serve`)

`slice serve` runs a shared-nothing **Thread-Per-Core (`--workers=N`)** HTTP/1.1 `Keep-Alive` and pipelined edge server where **every incoming HTTP request** executes inside a hardware- and OCap-isolated `SliceIsolate` (`152,376 req/s` in Tier-0 CAS mode, `96,287 req/s` in Tier-1 WinterTC mode):

```bash
slice serve ./service.cas --port=3000 --workers=8
```

### WinterTC Request/Response & `waitUntil` Bridge
Handlers can inspect zero-copy HTTP request metadata, return custom status codes and content types, and schedule post-response background tasks within the tenant's remaining CPU fuel budget:
```typescript
const req = Slice.request(); // { method, path, query, tenant, tenantId, body, header(name) }
Slice.waitUntil(() => {
    // Runs after the HTTP response bytes have been flushed to the client socket
});
Slice.respond(200, { ok: true, method: req.method, path: req.path, tenant: req.tenant }, "application/json");
```

### Per-Request Isolation Lifecycle
1. Zero-allocation stack HTTP/1.1 parser (`find_http_header_end` + `[(&str, &str); 16]`) extracts `method`, `path`, `query`, `X-Slice-Tenant`, `Content-Length`, `Connection: keep-alive`, and request body directly from the worker's reusable 64 KB buffer.
2. Resets the worker's `SliceIsolate` in place (`reset_for_request`) with zero heap allocations for `CapabilityToken` or `tenant_id`, assigning and entering a fresh `MPK/PKU` hardware protection domain (`WRPKRU` on Linux `x86_64`) and resetting CPU fuel and memory limits.
3. Executes the pre-compiled `SLICESNP` (`SNP_MODULE_CACHE`) or zero-copy `SLICEBOA` bytecode (`WarmBoaRealm::loaded_bundles` direct `JsObject::call` with zero `ctx.eval` overhead).
4. Flushes the HTTP/1.1 response (batching multiple responses into a single vectorized `write_all` syscall when pipelined requests are present in the read buffer), and then drains any queued `Slice.waitUntil(fn)` callbacks.
5. Emits execution telemetry headers on the HTTP response:
   * `X-Slice-Tenant`: Active tenant compartment identifier.
   * `X-Slice-Isolate-Us`: Isolate execution latency in microseconds.
   * `X-Slice-Total-Us`: Total end-to-end request latency in microseconds.
   * `X-Slice-Fuel-Used`: Deterministic instruction fuel consumed by the request.

---

## 4. Workflow 3: Bytecode Disassembly & CAS Auditing (`slice inspect`)

Use `slice inspect` on `.ts` scripts, project folders, or `.cas` archives to verify that hot loops and cryptographic routines are lowered to Tier-2 native SIMD opcodes or zero-copy `SLICEBOA` frames:

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

## 5. Workflow 4: Host Kernel & Warm Pool Diagnostics (`slice doctor` & `slice info`)

Before deploying to production edge nodes, run `slice doctor` and `slice info` to verify:
* **Completion Ring Backend:** Confirms whether `Linux io_uring (SQPOLL + Fixed Buffers)` or `Windows NT IOCP` is active.
* **Registered Slab Buffers:** Verifies pre-allocated `128 x 64 KB` (`8 MB`) zero-copy I/O slab buffers.
* **Pre-Warmed Boa ES2024 Pool:** Verifies that `BOA_CONTEXT_POOL` pre-warms and recycles ready ECMAScript realms for zero-init `SLICEBOA` execution (`slice run --warm`).
* **Hardware Memory Protection Keys:** Reports the active `PKEY` slot and `PKRU` bitmask (`0xffffffcf`).
* **Security Containment Audit:** Executes all 5 multi-tenant attack vectors (prototype pollution, unauthorized `/etc/shadow` read, outbound network exfiltration, secret `env` harvesting, and `while(true)` CPU DoS) and verifies 5/5 traps.


