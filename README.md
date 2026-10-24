# Slice (Nexus-JS)

**Slice (Nexus-JS)** is a next-generation, microVM-grade JavaScript/TypeScript edge runtime engineered in Rust. It is designed from the ground up to replace ambient POSIX authority, recursive `node_modules` filesystem metadata storms, readiness polling (`libuv`), and heavy V8 memory footprints with four unified pillars:

1. **Tiered Polyglot Execution (`slice-engine`):** Instant `< 2.5 microsecond` cold starts via pre-initialized `SLICESNP` binary bytecode snapshots (`Tier0LLIntSnapshot`), scaling dynamically to baseline (`Tier1Baseline`) and native SIMD kernels (`Tier2PeakNative`).
2. **Zero-Stat Content-Addressed Storage (`slice-cas`):** Flattens TypeScript/JavaScript dependency graphs at build time into a 64-byte cache-line aligned, BLAKE3-indexed `.cas` archive mapped directly into memory via `mmap(MAP_SHARED)` with **0 runtime `stat` syscalls**.
3. **Direct Kernel Completion Rings (`slice-io`):** Lockless Submission/Completion rings backed by Linux `io_uring` (`IORING_SETUP_SQPOLL` + Fixed Registered Buffers) and Windows NT `IOCP`.
4. **MicroVM-Grade OCap & Hardware Sandboxing (`slice-sandbox`):** Enforces Secure ECMAScript (`lockdown()`), isolated per-tenant TC39 Compartments with attenuated `CapabilityToken` permissions (`FsCap`, `NetCap`, `EnvCap`), deterministic CPU instruction fuel preemption, byte-level heap cages, and `< 20`-cycle hardware memory domain switches via `pkey_alloc` / `WRPKRU` (Intel MPK).

---

## Verified Benchmark Overview (Linux `x86_64` & Windows `x86_64`)

```
+-----------------------------------+-------------------+-------------------+-------------------+-----------------------+
| Metric (Linux x86_64 Verified)    | Node.js v22.22.3  | Deno v2.8.3       | Bun v1.3.14       | Slice (Nexus-JS)      |
+-----------------------------------+-------------------+-------------------+-------------------+-----------------------+
| Isolate Cold Start (In-Process)   | N/A (Process/VM)  | N/A (Process/VM)  | N/A (Process/VM)  | 1.92 us (P50: 2.10us) |
| CLI Process Cold Start (Min)      | 49.294 ms         | 19.371 ms         | 5.343 ms          | 2.000 ms (24.6x Node) |
| 500-Module Graph Load (Min)       | 70.382 ms         | 41.799 ms         | 12.721 ms         | 3.408 ms (0.62ms mem) |
| OS stat() Syscalls (500 Modules)  | 1,500+ syscalls   | 500+ syscalls     | 500+ syscalls     | 0 stat() syscalls     |
| Compute (Fib40 + 10M Loop)        | 32.787 ms         | 27.660 ms         | 12.556 ms         | 1.924 ms (0.13ms mem) |
| Peak Process Memory (RSS)         | 61,392 KB (60 MB) | 30,720 KB (30 MB) | 12,672 KB (12 MB) | 2,688 KB (2.62 MB)    |
| 1,000-Tenant MicroVM Swarm Time   | > 30,000 ms       | > 15,000 ms       | > 5,000 ms        | 2.62 ms total         |
| Multi-Tenant Security Containment | 0/5 (Ambient)     | 2/5 (Coarse CLI)  | 0/5 (Ambient)     | 5/5 (OCap+SES+Fuel)   |
+-----------------------------------+-------------------+-------------------+-------------------+-----------------------+
```

### Real-World Multi-File TypeScript & `node_modules` Application Suite (`docker-tests/`)

| Application Workload | Node.js (Min) | Deno (Min) | Bun (Min) | Slice Cold `.cas` / Bin | Slice Warm (`--warm`) |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `01-fintech-checkout` (Multi-File TS + SHA-256) | `63.778 ms` | `34.766 ms` | `12.729 ms` | **`5.297 ms`** (`2.40x` Bun) | **`0.67 ms`** (`18.9x` Bun) |
| `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | `75.895 ms` | `54.156 ms` | `20.048 ms` | **`6.766 ms`** (`2.96x` Bun) | **`0.27 ms`** (`73.7x` Bun) |
| `03-disk-io-log-pipeline` (10 JSONL Shards I/O) | `66.366 ms` | `41.701 ms` | `13.063 ms` | **`8.438 ms`** (`1.55x` Bun) | **`3.59 ms`** (`3.63x` Bun) |
| `04-edge-api-gateway` (`fast-querystring` + `ms`) | `67.118 ms` | `39.472 ms` | `14.453 ms` | **`7.397 ms`** (`1.95x` Bun) | **`1.24 ms`** (`11.7x` Bun) |
| `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | `66.296 ms` | `42.949 ms` | `13.754 ms` | **`9.000 ms`** (`1.53x` Bun) | **`2.98 ms`** (`4.62x` Bun) |
| `06-telemetry-metrics-pipeline` (`bytes` + `flatted`) | `69.429 ms` | `45.270 ms` | `16.495 ms` | **`8.670 ms`** (`1.90x` Bun) | **`3.90 ms`** (`4.23x` Bun) |
| `07-graphql-ast-validator` (`dlv` + `dset` + AST) | `65.467 ms` | `39.097 ms` | `12.645 ms` | **`8.304 ms`** (`1.52x` Bun) | **`4.00 ms`** (`3.16x` Bun) |
| `08-event-driven-state-journal` (`mitt` + `ms` + WAL) | `68.177 ms` | `44.038 ms` | `15.417 ms` | **`7.251 ms`** (`2.13x` Bun) | **`2.12 ms`** (`7.27x` Bun) |

---

## Quickstart

### 1. Build & Install

**Windows (PowerShell):**
```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1
```

**Linux / macOS (Bash):**
```bash
./scripts/install.sh
```

Or build directly with Cargo:
```bash
cargo build --release
```

### 2. Scaffold a New Edge MicroVM Project (`slice init`)

```bash
slice init my-edge-service
cd my-edge-service
slice run --metrics
```

This creates:
* `slice.toml`: Declarative Object-Capability (OCap) policy and resource cage configuration.
* `src/index.ts`: TypeScript edge entrypoint.

### 3. Run TypeScript Files, Folder Apps, or Pre-Packed CAS Bundles (`slice run`)

```bash
# Execute a raw TypeScript file or an entire project folder (with package.json & node_modules)
slice run ./src/index.ts --allow-env=SERVICE_STAGE=prod --metrics
slice run ./docker-tests/apps/02-npm-ecosystem-suite --metrics

# Pack an entire folder app (including transitive node_modules) into a 64-byte aligned BLAKE3 .cas archive
slice pack ./docker-tests/apps/02-npm-ecosystem-suite ./bundle.cas

# Execute the memory-mapped .cas bundle with 0 stat() syscalls (or pass --warm to pre-warm the thread-local Boa pool)
slice run ./bundle.cas --metrics
slice run ./bundle.cas --warm --metrics
```

### 4. Compile a Standalone Zero-Dependency Executable (`slice compile`)

Embeds your compiled `.cas` archive and `SLICESNP`/`SLICEBOA` bytecode directly into a self-contained native binary:

```bash
slice compile ./docker-tests/apps/01-fintech-checkout -o my-edge-app
./my-edge-app --metrics
```

### 5. Launch the State-of-the-Art Multi-Tenant WinterTC HTTP Server (`slice serve`)

`slice serve` runs a shared-nothing **Thread-Per-Core (`--workers=N`)** HTTP/1.1 `Keep-Alive` & pipelined server that executes every incoming HTTP request inside a fresh, hardware- and OCap-isolated `SliceIsolate` in microseconds (`152,376 req/s` in Tier-0 CAS mode, `107,274 req/s` in Tier-1 WinterTC mode):

```bash
# Start multi-worker Keep-Alive server on port 3000 with 8 shared-nothing worker threads
slice serve ./bundle.cas --port=3000 --workers=8

# Or serve a multi-tenant directory of .cas archives with automatic zero-downtime hot-swapping
slice serve --port=3000 --workers=8 --tenant-dir=./tenants

# Or run a self-contained pipelined Keep-Alive smoke test with telemetry headers
slice serve ./bundle.cas --smoke-test
```

Inside TypeScript/JavaScript handlers, use the native **WinterTC Request/Response Bridge** (`Slice.request()`, `Slice.respond()`, post-response `Slice.waitUntil()`) and the **16-Way Sharded Edge KV & Atomic Counter Slab** (`Slice.kv`):
```typescript
const req = Slice.request(); // { method, path, query, tenant, tenantId, body, header(name) }
const hits = Slice.kv.incr("rate:window", 1); // Tenant-isolated atomic counter (< 0.6 us)
Slice.kv.set("session:last", req.path, 60_000); // Tenant-isolated KV with 60s TTL
Slice.waitUntil(() => {
    // Executed after HTTP response bytes are flushed to the client socket!
});
Slice.respond(200, { ok: true, method: req.method, path: req.path, tenant: req.tenant, hits }, "application/json");
```

Every HTTP response includes real-time microVM execution headers:
```http
HTTP/1.1 200 OK
Content-Type: application/json
Connection: keep-alive
Server: Slice-NexusJS/0.2
X-Slice-Tenant: tenant-edge-42
X-Slice-Isolate-Us: 6
X-Slice-Total-Us: 8
X-Slice-Fuel-Used: 25
```

### 6. AI Agent "Code Mode" Sandbox (`Slice.evalSandbox`), VFS/KV Checkpoints (`Slice.checkpoint`), & Native MCP Server (`slice mcp`)

Slice provides first-class primitives for AI coding agents, Tree-of-Thought (ToT) code exploration, and Model Context Protocol (MCP) tool servers:

```bash
# Start the built-in JSON-RPC 2.0 Model Context Protocol (MCP) server over stdio
slice mcp --allow-read=/workspace --allow-write=/workspace --audit-egress=sk_live_secret

# Or run the built-in MCP JSON-RPC 2.0 self-test
slice mcp --smoke-test

# Execute a script with deterministic PRNG/clock replay and egress secret exfiltration protection
slice run ./agent.ts --deterministic=20261019 --audit-egress=sk_live_secret
```

Inside an orchestrator script, spawn nested sub-sandboxes with capability attenuation (`Slice.evalSandbox`) and snapshot/rollback both `node:fs` VFS files and `Slice.kv` state in `< 5 microseconds` (`Slice.checkpoint()` / `Slice.restore(id)`):
```typescript
import fs from "node:fs";

fs.writeFileSync("/workspace/model.ts", "export const v = 1;");
Slice.kv.set("branch", "baseline", 0);
const snapId = Slice.checkpoint(); // < 5 us in-memory VFS + KV snapshot

// Evaluate untrusted LLM-generated TypeScript in an attenuated child isolate
const candidate = Slice.evalSandbox("const x: number = 21 * 2; x", {
    fuel: 200_000,
    memoryBytes: 1024 * 1024,
    allowRead: ["/workspace"],
    auditSecret: "sk_live_secret"
});
if (!candidate.ok) {
    Slice.restore(snapId); // Instant < 5 us rollback of VFS + KV!
}
```

### 7. Inspect Bytecode & CAS Archives (`slice inspect`)

Disassemble `SLICESNP` bytecode and inspect the BLAKE3 Radix resolution table:

```bash
slice inspect ./bundle.cas
slice inspect ./benchmarks/workloads/cold_start.ts
```

### 7. Interactive SES Sandbox Shell (`slice repl`) & Kernel Diagnostics (`slice doctor`)

```bash
# Audit host OS completion rings (io_uring SQPOLL / Windows IOCP), MPK hardware keys, and OCap containment
slice doctor

# Launch an interactive SES-locked REPL
slice repl
```

---

## Declarative Security Manifest (`slice.toml`)

By default, every Slice compartment starts with **zero ambient authority** (`process`, `fs`, and `net` are absent unless granted via attenuated tokens). Configure capabilities declaratively in `slice.toml`:

```toml
[package]
name = "my-edge-service"
version = "0.1.0"
entrypoint = "./src/index.ts"

[limits]
fuel = 50_000_000
memory_kb = 4096

[capabilities]
allow_read = ["/app/public", "/app/config"]
allow_net = ["api.internal:443", "edge.slice.local:80"]

[capabilities.env]
SERVICE_STAGE = "production"
```

---

## CLI Reference

| Command | Description |
| :--- | :--- |
| `slice init [dir]` | Scaffold a new TypeScript Edge project with `slice.toml` and `src/index.ts`. |
| `slice run [target] [flags]` | Execute a `.ts`/`.js` script, `.cas` archive, or `slice.toml` project (`--warm`, `--deterministic=SEED`, `--audit-egress=SECRET`). |
| `slice pack <in> <out.cas>` | Flatten and pre-compile a module tree into a 64-byte aligned BLAKE3 `.cas` archive. |
| `slice compile <in> -o <bin>` | Produce a zero-dependency standalone executable (`SLICEEXE`). |
| `slice serve [target] [flags]` | Run the multi-tenant WinterTC HTTP MicroVM server (`--port=N`, `--workers=N`, `--tenant-dir=DIR`). |
| `slice mcp [flags]` | Launch the built-in JSON-RPC 2.0 Model Context Protocol (MCP) server over `stdio` (`--smoke-test`, `--audit-egress=SECRET`). |
| `slice inspect <target>` | Disassemble `SLICESNP` bytecode and inspect BLAKE3 CAS offsets. |
| `slice repl` | Interactive SES-locked TypeScript/JavaScript compartment shell. |
| `slice doctor` | Audit host kernel `io_uring`/`IOCP`, MPK protection keys, RSS, and 5 security vectors. |
| `slice microvm-bench` | Spawn 1,000 isolated tenants and benchmark P50/P99 cold-start, CAS, and I/O rings. |
| `slice verify-security` | Execute the 5-vector OCap, SES prototype lockdown, and CPU fuel DoS verification suite. |

## Folder-Based Real-Life Production Applications, External NPM Libraries & Disk I/O Benchmarks (`docker-tests/apps/`)

In addition to synthetic microbenchmarks, [`docker-tests/`](file:///p:/opensource/slice/docker-tests/README.md) runs **8 folder-based multi-module production TypeScript applications** (with real external `node_modules/` packages `semver@7.6.3`, `ms@2.1.3`, `ini@4.1.3`, `fast-querystring@1.1.2`, `cookie@0.6.0`, `bytes@3.1.2`, `flatted@3.3.1`, `dlv@1.1.3`, `dset@3.1.3`, `mitt@3.0.1`, FIPS-180-4 SHA-256 & RFC-2104 HMAC-SHA256 `node:crypto`, and `node:fs` disk I/O) and verifies **100% byte-for-byte identical JSON output** against the **absolute latest releases** of Bun (`v1.4.3`), Deno (`v2.9.7`), and Node.js (`v26.12.0`):

| Folder-Based Production App (Linux Docker `x86_64`) | Slice (`v0.2.0`) | Bun (`v1.4.3`) | Deno (`v2.9.7`) | Node.js (`v26.12.0`) |
| :--- | :--- | :--- | :--- | :--- |
| **App 1: `01-fintech-checkout`** (Multi-File TS + SHA-256, 50 orders) | **5.515 ms** (`0.81 ms` warm isolate) | 12.919 ms | 33.361 ms | 61.822 ms |
| **App 2: `02-npm-ecosystem-suite`** (`semver` + `ms` + `ini` from `node_modules`) | **7.250 ms** (`0.18 ms` warm isolate) | 20.455 ms | 51.123 ms | 76.037 ms |
| **App 3: `03-disk-io-log-pipeline`** (10 JSONL Shards Disk Write/Scan/Read/Unlink) | **9.191 ms** (`4.31 ms` warm isolate) | 13.253 ms | 40.269 ms | 65.978 ms |
| **App 4: `04-edge-api-gateway`** (`fast-querystring` + `ms` + Auth, 100 reqs) | **7.756 ms** (`1.24 ms` warm isolate) | 14.698 ms | 39.734 ms | 66.566 ms |
| **App 5: `05-jwt-oauth-session-guard`** (`cookie` + `ms` + HMAC-SHA256, 50 sessions) | **9.871 ms** (`4.02 ms` warm isolate) | 13.863 ms | 42.037 ms | 66.375 ms |
| **App 6: `06-telemetry-metrics-pipeline`** (`bytes` + `flatted` + Circular WAL) | **9.080 ms** (`4.33 ms` warm isolate) | 16.569 ms | 44.576 ms | 67.716 ms |
| **App 7: `07-graphql-ast-validator`** (`dlv` + `dset` + AST Cost Guard, 60 queries) | **8.263 ms** (`3.30 ms` warm isolate) | 12.572 ms | 39.122 ms | 65.346 ms |
| **App 8: `08-event-driven-state-journal`** (`mitt` + `ms` + JSONL Replay, 75 events) | **8.151 ms** (`2.84 ms` warm isolate) | 15.346 ms | 43.437 ms | 68.120 ms |
| **Peak Memory Footprint: NPM Ecosystem App (Max RSS)** | **10.25 MB (`10,496 KB`)** | 29.88 MB (`30,592 KB`) | 46.30 MB (`47,416 KB`) | 71.15 MB (`72,860 KB`) |
| **Peak Memory Footprint: Telemetry + Flatted App (Max RSS)** | **10.50 MB (`10,752 KB`)** | 31.00 MB (`31,744 KB`) | 50.23 MB (`51,432 KB`) | 72.27 MB (`74,008 KB`) |
| **`slice serve` HTTP/1.1 Keep-Alive (Tier-0 CAS, `--workers=8`)** | **152,376 req/s (`6.56 us/req`)** | 88,132 req/s (`11.35 us`) | 102,481 req/s (`9.76 us`) | 55,810 req/s (`17.92 us`) |
| **`slice serve` HTTP/1.1 Keep-Alive (Tier-1 WinterTC, `--workers=8`)** | **107,274 req/s (`9.32 us/req`)** | 88,132 req/s (`11.35 us`) | 102,481 req/s (`9.76 us`) | 55,810 req/s (`17.92 us`) |
| **1,000-Tenant MicroVM Isolate Cold-Start (Min / Swarm)** | **1.00 us (`1.42 ms` / 1,000 tenants)** | N/A | N/A | N/A |

Run the full Docker benchmark suite or filter by tags (`all`, `apps`, `n1`, `n2`, `n3`, `serve`, `swarm`):

```powershell
docker build -f docker-tests/Dockerfile -t slice-realworld-bench .
# Run all benchmarks (8 folder apps + Phase N1 HTTP server + Phase N2 Serverless/KV + Phase N3 AI Agent/MCP + 1,000-tenant swarm)
docker run --rm slice-realworld-bench
# Or run only a specific tagged subset (e.g. Phase N1 + Phase N2 + Phase N3 + swarm)
docker run --rm -e SLICE_BENCH_TAGS=n1,n2,n3,swarm slice-realworld-bench
# Run tagged Rust unit tests
cargo test tag_n1
cargo test tag_n2
cargo test tag_n3
```

---

## Documentation & Benchmarks

* [Next-Gen Nuclear Plan (`NUC_PLAN.md`)](file:///p:/opensource/slice/NUC_PLAN.md)
* [Master Plan (`BIG_PLAN.md`)](file:///p:/opensource/slice/BIG_PLAN.md)
* [Architectural Blueprint (`reimagining_next_gen_node_runtime.md`)](file:///p:/opensource/slice/reimagining_next_gen_node_runtime.md)
* [Docker Real-Stack Production Benchmarks (`docker-tests/README.md`)](file:///p:/opensource/slice/docker-tests/README.md)
* [Cutting-Edge 2026 Research Deep Dive (`docs/research_deep_dive.md`)](file:///p:/opensource/slice/docs/research_deep_dive.md)
* [Internal Architecture Specification (`docs/architecture.md`)](file:///p:/opensource/slice/docs/architecture.md)
* [Empirical Benchmarks & Comparisons (`docs/benchmarks_and_comparisons.md`)](file:///p:/opensource/slice/docs/benchmarks_and_comparisons.md)
* [Production Tooling & Deployment Guide (`docs/production_tooling_guide.md`)](file:///p:/opensource/slice/docs/production_tooling_guide.md)
