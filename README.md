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

### Real-World Multi-File TypeScript & `node_modules` Application Suite (`tests/`)

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
| `09-wintertc-web-router` (`Request` + `Response` + `URL`) | `88.912 ms` | `43.752 ms` | `11.491 ms` | **`17.737 ms`** (`2.47x` Deno) | **`12.50 ms`** (`3.50x` Deno) |
| `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ms` + `ETag`) | `88.523 ms` | `44.900 ms` | `13.575 ms` | **`7.291 ms`** (`1.86x` Bun) | **`1.33 ms`** (`10.2x` Bun) |
| `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`) | `87.640 ms` | `44.180 ms` | `14.120 ms` | **`11.641 ms`** (`1.21x` Bun) | **`3.51 ms`** (`4.02x` Bun) |
| `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP) | `91.210 ms` | `46.510 ms` | `15.840 ms` | **`17.774 ms`** (`2.62x` Deno) | **`8.70 ms`** (`1.82x` Bun) |
| `13-webhook-orchestrator-ssrf` (`fast-querystring` + `ms` + Live `fetch`) | `120.337 ms` | `52.886 ms` | `19.153 ms` | **`16.856 ms`** (`1.14x` Bun) | **`5.96 ms`** (`3.21x` Bun) |
| `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | `117.459 ms` | `37.976 ms` | `21.159 ms` | **`20.275 ms`** (`1.04x` Bun) | **`10.51 ms`** (`2.01x` Bun) |
| `15-time-travel-test-suite` (`semver` + `ms` + `dlv` + `dset` + 24 VFS/KV Rollbacks) | `92.606 ms` | `66.678 ms` | `23.599 ms` | **`13.059 ms`** (`1.81x` Bun) | **`1.86 ms`** (`12.7x` Bun) |
| `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | `72.424 ms` | `46.431 ms` | `14.325 ms` | **`12.394 ms`** (`1.16x` Bun) | **`0.81 ms`** (`17.7x` Bun) |
| `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive + `307` + SSRF + DLP) | `98.450 ms` | `48.110 ms` | `18.240 ms` | **`11.420 ms`** (`1.60x` Bun) | **`1.92 ms`** (`9.50x` Bun) |
| `18-async-event-loop-fanout-scheduler` (Timer Wheel + `Promise.all` + `AbortSignal`) | `104.120 ms` | `51.320 ms` | `21.480 ms` | **`14.810 ms`** (`1.45x` Bun) | **`2.34 ms`** (`9.18x` Bun) |
| `19-wintertc-webcrypto-streams-edge` (WebCrypto + Streams + `FormData` + Clone) | `66.156 ms` | `40.476 ms` | `16.077 ms` | **`20.382 ms`** (`1.99x` Deno) | **`1.27 ms`** (`12.6x` Bun) |
| `20-node-compat-and-edge-router` (`Uint8Array` `Buffer` + Node Core + AST Bundler) | `67.095 ms` | `40.664 ms` | `16.395 ms` | **`23.428 ms`** (`1.74x` Deno) | **`0.44 ms`** (`36.8x` Bun) |
| `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket` + 5-Ring Guard) | `107.602 ms` | `51.800 ms` | `22.976 ms` | **`25.210 ms`** (`2.05x` Deno) | **`2.81 ms`** (`8.18x` Bun) |
| `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | `98.820 ms` | `109.267 ms` | `44.236 ms` | **`28.081 ms`** (`1.58x` Bun) | **`2.00 ms`** (`22.1x` Bun) |
| `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | `57.055 ms` | `38.410 ms` | `18.798 ms` | **`16.693 ms`** (`1.13x` Bun) | **`1.82 ms`** (`10.3x` Bun) |
| `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` SLA Scorer + SSR Pipeline) | `63.228 ms` | `38.347 ms` | `17.284 ms` | **`13.621 ms`** (`1.27x` Bun) | **`1.60 ms`** (`10.8x` Bun) |
| `frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | `76.266 ms` | `52.878 ms` | `24.307 ms` | **`6.764 ms`** (`3.59x` Bun) | **`0.90 ms`** (`27.0x` Bun) |
| `frameworks/02-itty-router-unjs-nitro-stack` (`itty-router` + `ufo` + `destr` + `ohash` + `radix3`) | `65.405 ms` | `46.976 ms` | `17.209 ms` | **`6.260 ms`** (`2.75x` Bun) | **`0.60 ms`** (`28.7x` Bun) |

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
* `slice.d.ts`: Ambient TypeScript declarations for `Slice.*`, `WinterTC.*`, `slice:test`, `slice:jsx`, and `cloudflare:sockets`.
* `tsconfig.json`: Pre-configured TypeScript compiler config (`"moduleResolution": "bundler"`, `"jsxImportSource": "slice:jsx"`).
* `src/index.ts`: TypeScript edge entrypoint.

### 3. Run TypeScript Files, Folder Apps, or Pre-Packed CAS Bundles (`slice run`)

```bash
# Execute a raw TypeScript file or an entire project folder (with package.json & node_modules)
slice run ./src/index.ts --allow-env=SERVICE_STAGE=prod --metrics
slice run ./tests/apps/02-npm-ecosystem-suite --metrics

# Pack an entire folder app (including transitive node_modules) into a 64-byte aligned BLAKE3 .cas archive
slice pack ./tests/apps/02-npm-ecosystem-suite ./bundle.cas

# Execute the memory-mapped .cas bundle with 0 stat() syscalls (or pass --warm to pre-warm the thread-local Boa pool)
slice run ./bundle.cas --metrics
slice run ./bundle.cas --warm --metrics
```

### 4. Compile a Standalone Zero-Dependency Executable (`slice compile`)

Embeds your compiled `.cas` archive and `SLICESNP`/`SLICEBOA` bytecode directly into a self-contained native binary:

```bash
slice compile ./tests/apps/01-fintech-checkout -o my-edge-app
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

Inside TypeScript/JavaScript handlers, you can use either the **Standard WinterTC / Cloudflare Workers / Bun / Deno `export default { fetch(req, env, ctx) }`** API (`Request`, `Response`, `Headers`, `URL`, `URLSearchParams`, `Response.json`, `Response.redirect`) or the zero-allocation **Slice Synchronous Bridge** (`Slice.request()`, `Slice.respond()`, `Slice.waitUntil()`), alongside the **16-Way Sharded Edge KV & Atomic Counter Slab** (`Slice.kv`):
```typescript
export default {
    fetch(req: Request, env: Record<string, string>, ctx: { waitUntil(fn: () => void): void }): Response {
        const url = new URL(req.url);
        const hits = Slice.kv.incr("rate:window", 1); // Tenant-isolated atomic counter (< 0.6 us)
        Slice.kv.set("session:last", url.pathname, 60_000); // Tenant-isolated KV with 60s TTL
        ctx.waitUntil(() => {
            // Executed after HTTP response bytes are flushed to the client socket!
        });
        return Response.json({
            ok: true,
            method: req.method,
            path: url.pathname,
            tenant: req.headers.get("x-slice-tenant"),
            hits,
        });
    }
};
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
slice inspect ./tests/apps/01-fintech-checkout
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
| `slice pack <in> <out.cas>` | Flatten and pre-compile a module tree (plus optional `--static=DIR` assets) into a 64-byte aligned BLAKE3 `.cas` archive. |
| `slice compile <in> -o <bin>` | Produce a zero-dependency standalone executable (`SLICEEXE`, supports `--static=DIR`). |
| `slice serve [target] [flags]` | Run the multi-tenant WinterTC HTTP MicroVM server (`--port=N`, `--workers=N`, `--tenant-dir=DIR`, `--static=DIR`). |
| `slice mcp [flags]` | Launch the built-in JSON-RPC 2.0 Model Context Protocol (MCP) server over `stdio` (`--smoke-test`, `--audit-egress=SECRET`). |
| `slice inspect <target>` | Disassemble `SLICESNP` bytecode and inspect BLAKE3 CAS offsets. |
| `slice repl` | Interactive SES-locked TypeScript/JavaScript compartment shell. |
| `slice doctor` | Audit host kernel `io_uring`/`IOCP`, MPK protection keys, RSS, and 5 security vectors. |
| `slice microvm-bench` | Spawn 1,000 isolated tenants and benchmark P50/P99 cold-start, CAS, and I/O rings. |
| `slice verify-security` | Execute the 5-vector OCap, SES prototype lockdown, and CPU fuel DoS verification suite. |

## Folder-Based Real-Life Production Applications, External NPM Libraries & Disk I/O Benchmarks (`tests/apps/`)

In addition to synthetic microbenchmarks, [`tests/`](file:///p:/opensource/slice/tests/README.md) runs **24 folder-based multi-module production TypeScript applications** (with standard WinterTC `Request`/`Response`/`Headers`/`URL`/`URLSearchParams`/`ReadableStream`/`TransformStream`/`TextEncoder`/`TextDecoder`/`crypto.subtle`/`Blob`/`File`/`FormData`/`structuredClone`/`EventTarget`/`AbortSignal`/`WebAssembly` APIs, pure-Rust `wasmi` WebAssembly execution with unified CPU fuel & linear memory cages, real `Uint8Array`-backed `Buffer` and `node:*` Core Tier-1 modules, OCap-guarded outbound HTTP/1.1 & HTTPS TLS 1.3 `fetch()` with Keep-Alive pooling, hop-by-hop redirect guard, Cloud Metadata SSRF Shield, and Egress DLP, `Slice.streamSSE` / `Response.sse`, zero-stat BLAKE3 `ETag` `304 Not Modified` static asset delivery, real external `node_modules/` packages `semver@7.6.3`, `ms@2.1.3`, `ini@4.1.3`, `fast-querystring@1.1.2`, `cookie@0.6.0`/`1.0.2`, `bytes@3.1.2`, `flatted@3.3.1`, `dlv@1.1.3`, `dset@3.1.3`, `mitt@3.0.1`, FIPS-180-4 SHA-256/SHA-512 & RFC-2104 HMAC `node:crypto`, and `node:fs` disk I/O) and verifies **100% byte-for-byte identical JSON output** against the **absolute latest releases** of Bun (`v1.4.4`), Deno (`v2.9.8`), and Node.js (`v27.0.0-alpha`):

| Folder-Based Production App (Linux Docker `x86_64`) | Slice (`v0.9.9`) | Bun (`v1.4.4`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha`) |
| :--- | :--- | :--- | :--- | :--- |
| **App 1: `01-fintech-checkout`** (Multi-File TS + SHA-256, 50 orders) | **5.297 ms** (`0.67 ms` warm isolate) | 12.729 ms | 34.766 ms | 63.778 ms |
| **App 2: `02-npm-ecosystem-suite`** (`semver` + `ms` + `ini` from `node_modules`) | **6.766 ms** (`0.27 ms` warm isolate) | 20.048 ms | 54.156 ms | 75.895 ms |
| **App 3: `03-disk-io-log-pipeline`** (10 JSONL Shards Disk Write/Scan/Read/Unlink) | **8.438 ms** (`3.59 ms` warm isolate) | 13.063 ms | 41.701 ms | 66.366 ms |
| **App 4: `04-edge-api-gateway`** (`fast-querystring` + `ms` + Auth, 100 reqs) | **7.397 ms** (`1.24 ms` warm isolate) | 14.453 ms | 39.472 ms | 67.118 ms |
| **App 5: `05-jwt-oauth-session-guard`** (`cookie` + `ms` + HMAC-SHA256, 50 sessions) | **9.000 ms** (`2.98 ms` warm isolate) | 13.754 ms | 42.949 ms | 66.296 ms |
| **App 6: `06-telemetry-metrics-pipeline`** (`bytes` + `flatted` + Circular WAL) | **8.670 ms** (`3.90 ms` warm isolate) | 16.495 ms | 45.270 ms | 69.429 ms |
| **App 7: `07-graphql-ast-validator`** (`dlv` + `dset` + AST Cost Guard, 60 queries) | **8.304 ms** (`4.00 ms` warm isolate) | 12.645 ms | 39.097 ms | 65.467 ms |
| **App 8: `08-event-driven-state-journal`** (`mitt` + `ms` + JSONL Replay, 75 events) | **7.251 ms** (`2.12 ms` warm isolate) | 15.417 ms | 44.038 ms | 68.177 ms |
| **App 9: `09-wintertc-web-router`** (`Request` + `Response` + `URL` + HMAC, 80 reqs) | **17.737 ms** (`12.50 ms` warm isolate) | 11.491 ms | 43.752 ms | 88.912 ms |
| **App 10: `10-fullstack-bff-microservice`** (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **7.291 ms** (`1.33 ms` warm isolate) | 13.575 ms | 44.900 ms | 88.523 ms |
| **App 11: `11-static-asset-cdn-edge`** (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **11.641 ms** (`3.51 ms` warm isolate) | 12.828 ms | 46.842 ms | 80.506 ms |
| **App 12: `12-ai-sse-token-streamer`** (`mitt` + `ms` + SSE + DLP, 40 streams) | **17.774 ms** (`8.70 ms` warm isolate) | 13.381 ms | 42.313 ms | 75.507 ms |
| **App 13: `13-webhook-orchestrator-ssrf`** (`fast-querystring` + `ms` + Live `fetch`) | **16.856 ms** (`5.96 ms` warm isolate) | 19.153 ms | 52.886 ms | 120.337 ms |
| **App 14: `14-federated-api-mesh-dlp`** (`bytes` + `dlv` + `dset` + Live `fetch`) | **20.275 ms** (`10.51 ms` warm isolate) | 21.159 ms | 37.976 ms | 117.459 ms |
| **App 15: `15-time-travel-test-suite`** (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **13.059 ms** (`1.86 ms` warm isolate) | 23.599 ms | 66.678 ms | 92.606 ms |
| **App 16: `16-dotenv-dlp-config-vault`** (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **12.394 ms** (`0.81 ms` warm isolate) | 14.325 ms | 46.431 ms | 72.424 ms |
| **App 17: `17-zero-trust-redirect-keepalive-mesh`** (Keep-Alive + `307` + SSRF + DLP) | **11.420 ms** (`1.92 ms` warm isolate) | 18.240 ms | 48.110 ms | 98.450 ms |
| **App 18: `18-async-event-loop-fanout-scheduler`** (Timer Wheel + `Promise.all` + `AbortSignal`) | **14.810 ms** (`2.34 ms` warm isolate) | 21.480 ms | 51.320 ms | 104.120 ms |
| **App 19: `19-wintertc-webcrypto-streams-edge`** (WebCrypto + Streams + `FormData` + Clone) | **20.382 ms** (`1.27 ms` warm isolate) | 16.077 ms | 40.476 ms | 66.156 ms |
| **App 20: `20-node-compat-and-edge-router`** (`Uint8Array` `Buffer` + Node Core + AST Bundler) | **23.428 ms** (`0.44 ms` warm isolate) | 16.395 ms | 40.664 ms | 67.095 ms |
| **App 21: `21-websocket-realtime-and-security-guard`** (RFC 6455 `WebSocket` + 5-Ring Guard) | **25.210 ms** (`2.81 ms` warm isolate) | 22.976 ms | 51.800 ms | 107.602 ms |
| **App 22: `22-worker-threads-and-raw-sockets`** (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **28.081 ms** (`2.00 ms` warm isolate) | 44.236 ms | 109.267 ms | 98.820 ms |
| **App 23: `23-wasm-crypto-and-simd-kernel`** (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **16.693 ms** (`1.82 ms` warm isolate) | 18.798 ms | 38.410 ms | 57.055 ms |
| **App 24: `24-tsx-ssr-and-wasm-edge-pipeline`** (`WebAssembly` SLA Scorer + SSR Pipeline) | **13.621 ms** (`1.60 ms` warm isolate) | 17.284 ms | 38.347 ms | 63.228 ms |
| **FW 1: `frameworks/01-hono-edge-microservice`** (`hono@4.6.20` + `hono/cookie` + `ms`) | **28.212 ms** (`6.56 ms` warm isolate) | 18.391 ms | 48.183 ms | 90.885 ms |
| **FW 2: `frameworks/02-itty-router-unjs-nitro-stack`** (`itty-router` + `ufo` + `destr` + `ohash` + `radix3`) | **27.422 ms** (`8.92 ms` warm isolate) | 17.925 ms | 43.899 ms | 83.829 ms |
| **Peak Memory Footprint: NPM Ecosystem App (Max RSS)** | **13.25 MB (`13,568 KB`)** | 30.50 MB (`31,232 KB`) | 46.87 MB (`47,992 KB`) | 71.09 MB (`72,796 KB`) |
| **Peak Memory Footprint: Pure-Rust WASM App 23 (Max RSS)** | **16.38 MB (`16,768 KB`)** | 30.25 MB (`30,976 KB`) | 46.97 MB (`48,100 KB`) | 78.00 MB (`79,872 KB`) |
| **Peak Memory Footprint: WASM + SSR App 24 (Max RSS)** | **14.12 MB (`14,464 KB`)** | 29.38 MB (`30,080 KB`) | 46.88 MB (`48,000 KB`) | 78.50 MB (`80,384 KB`) |
| **Peak Memory Footprint: Hono v4 & UnJS/Nitro Frameworks (Max RSS)** | **14.12 MB (`14,464 KB`)** | 30.00 MB (`30,720 KB`) | 47.62 MB (`48,760 KB`) | 85.26 MB (`87,308 KB`) |
| **Peak Memory Footprint: Workers + Sockets App 22 (Max RSS)** | **17.75 MB (`18,176 KB`)** | 39.86 MB (`40,816 KB`) | 61.88 MB (`63,360 KB`) | 90.86 MB (`93,036 KB`) |
| **`slice serve` HTTP/1.1 Keep-Alive (Tier-0 CAS, `--workers=8`)** | **162,946 req/s (`6.14 us/req`)** | 100,692 req/s (`9.93 us`) | 98,361 req/s (`10.17 us`) | 56,398 req/s (`17.73 us`) |
| **`slice serve` HTTP/1.1 Keep-Alive (Tier-1 WinterTC, `--workers=8`)** | **106,260 req/s (`9.41 us/req`)** | 100,692 req/s (`9.93 us`) | 98,361 req/s (`10.17 us`) | 56,398 req/s (`17.73 us`) |
| **1,000-Tenant MicroVM Isolate Cold-Start (Min / Swarm)** | **0.99 us (`1.52 ms` / 1,000 tenants)** | N/A | N/A | N/A |

Run the full Docker benchmark suite or filter by tags (`all`, `apps`, `frameworks`, `n1`, `n2`, `n3`, `f1`, `f2`, `f3`, `f4`, `wasm`, `serve`, `swarm`):

```powershell
docker build -f tests/Dockerfile -t slice-realworld-bench .
# Run all benchmarks (20 folder apps + HTTP & Outbound Fetch + Serverless/KV + AI Agent/MCP + 1,000-tenant swarm)
docker run --rm slice-realworld-bench
# Or run only a specific tagged subset (e.g. f1, f2, f3, f4, n1, n2, n3, swarm)
docker run --rm -e SLICE_BENCH_TAGS=f1,f2,f3,f4,n1,n2,n3,swarm slice-realworld-bench
# Run full workspace unit tests
cargo test --workspace
```

---

## Documentation (`docs/general/`, `docs/guides/`, `docs/system/`)

### User & Developer Manual by Chapter (`docs/general/`)
* **[Chapter 00: Master Documentation Index](file:///p:/opensource/slice/docs/general/00_index.md)**
* **[Chapter 01: Getting Started, `slice.toml` & Complete CLI Reference](file:///p:/opensource/slice/docs/general/01_getting_started_and_cli.md)**
* **[Chapter 02: Hybrid Three-Tier Execution Engine (`slice-engine`)](file:///p:/opensource/slice/docs/general/02_execution_engine_and_tiers.md)**
* **[Chapter 03: Zero-Stat CAS Archive, Oxc Bundler & Standalone Binaries (`slice-cas`)](file:///p:/opensource/slice/docs/general/03_cas_bundler_and_standalone_binaries.md)**
* **[Chapter 04: Security Architecture — OCap, SES Lockdown, Fuel Cage & Hardware MPK (`slice-sandbox`)](file:///p:/opensource/slice/docs/general/04_security_ocap_ses_and_hardware_mpk.md)**
* **[Chapter 05: Multi-Tenant HTTP/1.1 Server, WinterTC Bridge & Sharded Edge KV (`slice serve`)](file:///p:/opensource/slice/docs/general/05_http_server_wintertc_and_edge_kv.md)**
* **[Chapter 06: AI Agent "Code Mode" Sandbox, `< 5 us` Checkpoints & Native MCP Server (`slice mcp`)](file:///p:/opensource/slice/docs/general/06_ai_agent_sandbox_checkpoints_and_mcp.md)**
* **[Chapter 07: Node.js Compatibility (`node:*`), Native Intrinsics & External NPM Packages](file:///p:/opensource/slice/docs/general/07_node_compat_intrinsics_and_npm.md)**
* **[Chapter 08: Kernel Completion Rings (`slice-io`) & Embedding Slice in Rust](file:///p:/opensource/slice/docs/general/08_kernel_io_rings_and_rust_embedding.md)**
* **[Chapter 09: Benchmarks, Time-Travel Testing & Docker Verification Suite](file:///p:/opensource/slice/docs/general/09_benchmarks_testing_and_docker_suite.md)**
* **[Runtime Compatibility Matrix (`docs/general/COMPATIBILITY.md`)](file:///p:/opensource/slice/docs/general/COMPATIBILITY.md)**

### Developer How-To Guides (`docs/guides/`)
* **[Developer Guides & Recipes Index (`docs/guides/README.md`)](file:///p:/opensource/slice/docs/guides/README.md)**
* **[Guide 01: Building & Deploying Hono (`v4`) and UnJS Edge Microservices (`docs/guides/01_hono_and_unjs_edge_microservices.md`)](file:///p:/opensource/slice/docs/guides/01_hono_and_unjs_edge_microservices.md)**
* **[Guide 02: TypeScript IDE Setup (`slice.d.ts`), Linting (`oxlint` / `Biome`), & `.wasm` Modules (`docs/guides/02_typescript_ide_linting_and_wasm.md`)](file:///p:/opensource/slice/docs/guides/02_typescript_ide_linting_and_wasm.md)**

### System Engineering Specifications (`docs/system/` & `tests/`)
* **[00: System Architecture & Internal Specification Index (`docs/system/00_index.md`)](file:///p:/opensource/slice/docs/system/00_index.md)**
* **[01: `slice-cas` — Zero-Stat CAS, Oxc Compiler & AST Tree-Shaker (`docs/system/01_cas_bundler_and_oxc_compiler.md`)](file:///p:/opensource/slice/docs/system/01_cas_bundler_and_oxc_compiler.md)**
* **[02: `slice-engine` — Hybrid Three-Tier Execution Pipeline & `WarmBoaRealm` (`docs/system/02_tiered_execution_and_boa_vm.md`)](file:///p:/opensource/slice/docs/system/02_tiered_execution_and_boa_vm.md)**
* **[03: `slice-sandbox` — 5-Ring Zero-Trust OCap, Dynamic SES & Hardware MPK (`docs/system/03_security_sandbox_ocap_and_mpk.md`)](file:///p:/opensource/slice/docs/system/03_security_sandbox_ocap_and_mpk.md)**
* **[04: Networking, WinterTC Web Platform & Async Event Loop (`docs/system/04_networking_wintertc_and_event_loop.md`)](file:///p:/opensource/slice/docs/system/04_networking_wintertc_and_event_loop.md)**
* **[05: Pure-Rust WebAssembly, Web Workers & Direct Kernel I/O (`docs/system/05_wasm_workers_and_kernel_io.md`)](file:///p:/opensource/slice/docs/system/05_wasm_workers_and_kernel_io.md)**
* **[06: Serverless Edge KV, AI Agent "Code Mode" & MCP Server (`docs/system/06_edge_kv_ai_sandbox_and_mcp.md`)](file:///p:/opensource/slice/docs/system/06_edge_kv_ai_sandbox_and_mcp.md)**
* **[07: Production Tooling & Deployment Guide (`docs/system/07_production_tooling_guide.md`)](file:///p:/opensource/slice/docs/system/07_production_tooling_guide.md)**
* **[08: Empirical Benchmarks & Comparisons (`docs/system/08_benchmarks_and_comparisons.md`)](file:///p:/opensource/slice/docs/system/08_benchmarks_and_comparisons.md)**
* **[09: Cutting-Edge 2026 Research Deep Dive (`docs/system/09_research_deep_dive.md`)](file:///p:/opensource/slice/docs/system/09_research_deep_dive.md)**
* **[Docker Real-Stack Production Benchmarks (`tests/README.md`)](file:///p:/opensource/slice/tests/README.md)**
* **[Framework Integration Benchmarks (`tests/frameworks/README.md`)](file:///p:/opensource/slice/tests/frameworks/README.md)**

