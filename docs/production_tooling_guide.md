# Slice (Nexus-JS): Production Tooling & Deployment Guide

## 1. Overview of the Production Toolchain

Slice `v0.2.0` provides a complete developer and production deployment workflow packaged inside a single self-contained native Rust executable (`slice`, embedding the **Oxc** TypeScript compiler, transitive `node_modules` bundler, **Boa** ES2024+ VM, and **Slice** Zero-Stat CAS + Tier-0/Tier-2 engine):

1. **Project Initialization & Policy Configuration (`slice init` & `slice.toml`)**
2. **Zero-Stat Content-Addressed Packaging (`slice pack` for single files or full `package.json` + `node_modules` folders)**
3. **Standalone Single-Binary Compilation (`slice compile`)**
4. **Bytecode & CAS Radix Disassembly (`slice inspect`)**
5. **Multi-Tenant WinterTC HTTP MicroVM Hosting (`slice serve`)**
6. **AI Agent "Code Mode" Sandbox & Model Context Protocol Server (`slice mcp`, `Slice.evalSandbox`, `Slice.checkpoint`)**
7. **Interactive SES Compartment Shell (`slice repl`)**
8. **Kernel & Security Readiness Auditing (`slice doctor`)**

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
# Or route per-tenant .cas bundles dynamically with live zero-downtime hot-swapping:
slice serve --port=3000 --workers=8 --tenant-dir=./tenants
# Or run in deterministic replay mode (SplitMix64 PRNG + monotonic virtual clock):
slice run ./service.cas --deterministic=20261019
```

### WinterTC Request/Response, `waitUntil`, & Sharded `Slice.kv` Bridge
Handlers can inspect zero-copy HTTP request metadata, query or mutate their tenant-isolated partition in the 16-way sharded `Slice.kv` slab (`get`, `set`, `del`, `incr`), return custom status codes and content types, and schedule post-response background tasks within the tenant's remaining CPU fuel budget:
```typescript
const req = Slice.request(); // { method, path, query, tenant, tenantId, body, header(name) }
const hits = Slice.kv.incr("rate:window", 1);
Slice.kv.set("last:path", req.path, 60_000);
Slice.waitUntil(() => {
    // Runs after the HTTP response bytes have been flushed to the client socket
});
Slice.respond(200, { ok: true, method: req.method, path: req.path, tenant: req.tenant, hits }, "application/json");
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

---

## 6. Workflow 5: AI Agent "Code Mode" Sandbox, VFS/KV Checkpoints, Egress Secret Guard, & Native MCP Server (`slice mcp`)

### 6.1 Launching the Built-In Model Context Protocol Server (`slice mcp`)
Connect Claude Code, Cursor, or any JSON-RPC 2.0 MCP client directly to Slice over `stdio`:
```bash
# Start MCP server with scoped VFS access and egress secret leak protection
slice mcp --allow-read=/workspace --allow-write=/workspace --audit-egress=sk_live_secret

# Or run the built-in JSON-RPC 2.0 smoke test
slice mcp --smoke-test
```

Exposed MCP tools:
* **`slice_eval_ts`**: Compiles (via Oxc) and executes untrusted TypeScript/JavaScript inside an attenuated child `SliceIsolate` in `~27 us` (`code`, `fuel`, `memory`, `allowRead`, `allowWrite`, `auditSecret`).
* **`slice_kv_get` / `slice_kv_set`**: Reads and writes state in the tenant's 16-way sharded `Slice.kv` slab.
* **`slice_checkpoint`**: Captures (`{"action": "checkpoint"}`) or rolls back (`{"action": "restore", "snapshot_id": 1}`) both the in-memory `node:fs` VFS slab and `Slice.kv` partition in `< 5 microseconds`.

### 6.2 Tree-of-Thought (ToT) Branching inside TypeScript (`Slice.checkpoint`, `Slice.evalSandbox`, `Slice.restore`)
```typescript
import fs from "node:fs";

fs.writeFileSync("/workspace/solution.ts", "export const v = 1;");
Slice.kv.set("agent:state", "clean", 0);

const snapId = Slice.checkpoint(); // < 5 us in-memory VFS + KV snapshot

// Candidate LLM branch mutates state and runs in an attenuated sub-isolate
fs.writeFileSync("/workspace/solution.ts", "UNTRUSTED_MUTATION");
const res = Slice.evalSandbox("const x: number = 21 * 2; x", {
    fuel: 100_000,
    allowRead: ["/workspace"],
    auditSecret: "sk_live_secret"
});

if (!res.ok) {
    Slice.restore(snapId); // Instant < 5 us rollback of VFS + KV!
}
```



