# Slice (Nexus-JS): Production Tooling & Deployment Guide

## 1. Overview of the Production Toolchain

Slice `v0.10.10` provides a complete developer and production deployment workflow packaged inside a single self-contained native Rust executable (`slice`, embedding the **Oxc** TypeScript compiler, transitive `node_modules` bundler, **Boa** ES2024+ VM, and **Slice** Zero-Stat CAS + Tier-0/Tier-2 engine):

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
* Compiles every module into default **`SLICEBC\x01` Precompiled Boa `CodeBlock`** bytecode (with Oxc AST deferred function module spans and pre-fused `node:*` built-ins), `SLICESNP`, or legacy `SLICEBOA` bytecode.
* Computes a 256-bit **BLAKE3** hash over the canonicalized source and bytecode.
* Aligns every blob to a **64-byte CPU cache line** boundary.
* At runtime (`slice run ./service.cas`), the file is mapped via `mmap(MAP_SHARED)` with **0 `stat()` syscalls** and **0 runtime JS lexing/parsing** (`0.05 us - 0.21 us` isolate instantiation).

### 2.2 Compiling a Self-Contained Binary (`slice compile`)
When deploying to minimal container images (`FROM scratch` or `distroless`) or distributing CLI tools:
```bash
slice compile ./my-edge-app -o ./my-service
```
* Appends the 64-byte aligned `.cas` archive directly to the `slice` runtime executable followed by a 24-byte `SLICEEXE` trailer (`[u64 cas_offset][u64 cas_len][b"SLICEEXE"]`).
* When `./my-service` boots, it maps its own executable image via `mmap`, locates the `SLICEEXE` trailer in O(1) time, and executes the embedded `SLICEBC\x01`/`SLICESNP` snapshot without extracting any temporary files to disk.

---

## 2.3 Unified 4-Layer Configuration Precedence (`slice.toml`, Inline `// @slice-...`, `SLICE_*` Env Vars, & CLI Flags)

Implemented in [`crates/slice-sandbox/src/lib.rs`](../../crates/slice-sandbox/src/lib.rs) (`SliceManifest`, `InlineDirectives`, `parse_inline_directives`) and [`crates/slice-cli/src/config_and_watch.rs`](../../crates/slice-cli/src/config_and_watch.rs) (`ResolvedExecutionConfig`):

1. **Layer 1 — Built-In Optimal Defaults:** `tier = "tier1"`, `boacas = true` (`SLICEBC\x01`), `fuel = 50_000_000`, `memory = 16 MB`, `CapabilityToken::pure_compute()`.
2. **Layer 2 — Project / Embedded `slice.toml` Manifest:** Supports `[package]` (`name`, `version`, `entrypoint`), `[engine]` (`tier`, `boacas`, `warm`, `deterministic_seed`), `[limits]` (`fuel`, `memory_kb`, `memory_mb`, `memory_bytes`), `[capabilities]` (`allow_read`, `allow_write`, `allow_net`, `allow_env`, `env_file`, `audit_egress`), `[capabilities.env]`, and `[server]` (`port`, `workers`, `static_dir`).
3. **Layer 3 — Top-of-File Inline Source Directives (`// @slice-...`):** Leading comments in the first 64 lines of an entrypoint (`// @slice-tier: tier1`, `// @slice-boacas: true`, `// @slice-warm: true`, `// @slice-deterministic: 42`, `// @slice-fuel: 25_000_000`, `// @slice-memory-kb: 8192`, `// @slice-allow-read: [...]`, `// @slice-allow-write: [...]`, `// @slice-allow-net: [...]`, `// @slice-allow-env: [...]`, `// @slice-env-file: .env`, `// @slice-audit-egress: [...]`, `// @slice-port: 8080`, `// @slice-workers: 4`).
4. **Layer 4 — Host Environment Variables (`SLICE_*`):** `SLICE_TIER`, `SLICE_BOACAS` (`SLICE_EXPER_BOACAS`), `SLICE_WARM`, `SLICE_DETERMINISTIC_SEED`, `SLICE_FUEL`, `SLICE_MEMORY_KB` (`SLICE_MEMORY`), `SLICE_ALLOW_READ`, `SLICE_ALLOW_WRITE`, `SLICE_ALLOW_NET`, `SLICE_ALLOW_ENV`, `SLICE_ENV_FILE`, `SLICE_AUDIT_EGRESS`, `SLICE_PORT`, `SLICE_WORKERS`, `SLICE_STATIC_DIR`.
5. **Layer 5 — Explicit CLI Inline Flags (`--*`):** `--tier=`, `--boacas` / `--no-boacas`, `--warm` / `--no-warm`, `--deterministic[=<seed>]` / `--no-deterministic`, `--fuel=`, `--memory=` / `--memory-kb=`, `--allow-read=`, `--allow-write=`, `--allow-net=`, `--allow-env=`, `--env-file=`, `--audit-egress=`, `--port=`, `--workers=`, `--static=`.

---

## 3. Workflow 2: State-of-the-Art Multi-Tenant HTTP MicroVM Server (`slice serve`)

`slice serve` runs a shared-nothing **Thread-Per-Core (`--workers=N`)** HTTP/1.1 `Keep-Alive` and pipelined edge server where **every incoming HTTP request** executes inside a hardware- and OCap-isolated `SliceIsolate` (`163,766 req/s` in Tier-0 CAS mode, `126,566 req/s` in Tier-1 WinterTC mode):

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

---

## 7. Workflow 6: Time-Travel Unit & Security Testing (`slice test`), `.env` Auto-DLP, & Compiler Optimizations

### 7.1 Running Time-Travel Isolated Tests (`slice test`)
`slice test` discovers `*.test.ts` / `*.spec.ts` files and executes each `test()` block inside an automatic `< 5 us` `Slice.checkpoint()` / `Slice.restore()` boundary so VFS and `Slice.kv` mutations never leak between tests:
```bash
slice test
slice test --filter="security" --json
```
First-class OCap & resource matchers built into `slice:test`:
* `expect(fnOrResult).toViolate("FsReadDenied" | "NetAccessDenied" | "CpuFuelExhausted" | "IntrinsicMutationBlocked")`
* `expect(fnOrResult).toStayWithinFuel(maxFuel)`
* `expect(fnOrResult).toStayWithinMemory(maxBytes)`

### 7.2 Automatic `.env` Secret Classification & Egress DLP (`--env-file`)
When running `slice run --env-file=.env` or `slice serve --env-file=.env`, Slice parses the `.env` file and automatically registers any key matching secret patterns (`*_SECRET*`, `*_TOKEN*`, `*_KEY*`, `*_PASSWORD*`, or high-entropy prefixes like `sk_live_*`, `ghp_*`) into the Egress DLP firewall. Tenant code can read the secret for authorized HMAC/API operations, but any attempt to leak it via `stdout`, HTTP response bodies, SSE streams, or unauthorized outbound `fetch()` (in raw, Base64, or Base64URL form) traps with `EGRESS_SECRET_LEAK_BLOCKED`.

### 7.3 General-Purpose AOT & Tier-2 Optimizations Applied Automatically
1. **Unboxed 64-Bit Numeric Register VM ([`numeric_vm.rs`](../../crates/slice-engine/src/vm/numeric_vm.rs)):** Any pure numeric JS/TS function (using arithmetic, bitwise operators, `while`/`for` loops, `if`/`else`, `Math.*` intrinsics, or recursion) is automatically compiled to unboxed 64-bit register instructions (`NumericOp`) on a 32-register stack file (`[f64; 32]`), and 2-term linear recurrences are structurally lowered to $O(n)$ register accumulation.
2. **Oxc AST Array Pipeline Fusion ([`fuse_array_pipelines_ast`](../../crates/slice-cas/src/lib.rs#L231-L575)):** During `slice pack` and `slice compile`, chained `.map().reduce()`, `.filter().map().reduce()`, `.some()`, and `.every()` calls on any array expression are fused via Oxc's AST into single-pass, zero-allocation `for` loop IIFEs.
3. **Dynamic SES Lockdown & `Boa` `RuntimeLimits`:** Both static and dynamically obfuscated prototype mutations (`Object["proto" + "type"].polluted = ...`) are trapped and rolled back via `__slice_check_ses`, and infinite loops or unbounded recursion inside `Boa` are preempted via `RuntimeLimits`.
4. **Native Rust WebCrypto (`crypto.subtle`), WHATWG Streams, & Outbound TLS 1.3 (`rustls`):** Full `crypto.subtle` (`AES-GCM`, `HMAC`, `Ed25519`, `PBKDF2`, `HKDF`, `SHA-256/384/512/1`), `ReadableStream`/`WritableStream`/`TransformStream`, `TextEncoder`/`TextDecoder`, `Blob`/`File`/`FormData`, `AbortController`/`AbortSignal`, and outbound HTTPS (`rustls`) with connection pooling (`OUTBOUND_CONN_POOL`), concurrent `Promise.all` fan-out (`OUTBOUND_FETCH_POOL`), DNS-resolved IP SSRF firewall, and hop-by-hop redirect validation.
5. **Real `Uint8Array`-Backed `Buffer`, Node Core Tier-1 & Oxc AST ESM-to-CJS Bundler:** `Buffer` is a real `Uint8Array` instance with zero-copy `.subarray()` views and big/little-endian integer readers/writers; `node:events`, `node:path`, `node:crypto`, `node:fs`/`node:fs/promises`, `node:util`/`node:util/types`, and `node:assert`/`node:assert/strict` are lazily materialized; and `slice-cas` resolves modern ESM `node_modules` packages (`jose`, `zod`, `radix3`, `superjson`, `nanoid`, `ulid`) via conditional `package.json` `"exports"` and Oxc AST-driven ESM-to-CJS transformation.




