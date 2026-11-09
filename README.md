# Slice (Nexus-JS)

**Slice (Nexus-JS)** is a next-generation, microVM-grade JavaScript/TypeScript edge runtime and AI Agent "Code Mode" sandbox engineered in Rust. It replaces ambient POSIX authority, recursive `node_modules` filesystem metadata storms, readiness polling (`libuv`), runtime JavaScript AST parsing, and heavy V8/JSC memory footprints with four unified pillars:

1. **Tiered Polyglot Execution & Precompiled `CodeBlock` CAS (`slice-engine`):** Defaults to **Tier 1 (`Tier1Baseline`) with `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS (`boacas = true`)**, serializing the compiled `Gc<CodeBlock>` hierarchy, global `Scope` indices, Oxc AST deferred function module spans (`0x11` tag), and pre-fused `node:*` built-in shards at build time so runtime execution bypasses `boa_parser` (`Lexer` + `Parser`), `boa_ast`, `ScopeAnalyzer`, and `ByteCompiler` entirely. Also supports sub-microsecond `SLICESNP` register snapshots (`Tier0LLIntSnapshot`), native Rust SIMD intrinsics (`Tier2PeakNative`), and pure-Rust `wasmi` WebAssembly execution with unified CPU fuel and memory-page cages.
2. **Zero-Stat Content-Addressed Storage (`slice-cas`):** Flattens TypeScript/JavaScript dependency graphs (including `package.json` `"exports"`, `.wasm` modules, and static barrel tree-shaking) at build time via **Oxc (`v0.154.0`)** into a 64-byte cache-line aligned, BLAKE3-indexed `.cas` archive mapped directly into virtual memory via `mmap(MAP_SHARED)` with **`0` runtime `stat` syscalls**.
3. **Direct Kernel Completion Rings (`slice-io`):** Lockless Submission/Completion rings backed by Linux `io_uring` (`IORING_SETUP_SQPOLL` + Fixed Registered Buffers) and Windows NT `IOCP`.
4. **5-Ring MicroVM OCap & Hardware Sandboxing (`slice-sandbox`):** Enforces Secure ECMAScript (`lockdown()`), isolated per-tenant TC39 Compartments with attenuated `CapabilityToken` permissions (`FsCap`, `NetCap`, `EnvCap`), Cloud Metadata SSRF & DNS-rebinding firewalls, Outbound Egress DLP (raw, Base64, Base64URL, & `.env` Auto-DLP), deterministic CPU instruction fuel preemption, byte-level heap cages, `< 5 us` copy-on-write VFS + `Slice.kv` time-travel checkpoints, and `< 20`-cycle hardware memory domain switches via `pkey_alloc` / `WRPKRU` (Intel MPK).

---

## Verified Benchmark Overview (Linux `x86_64` Docker & Windows `x86_64`)

<!-- BENCHMARK_SUMMARY_START -->
> **Latest Verified Benchmark Report:** **[`tests/reports/latest.md`](./tests/reports/latest.md)** (Archive: [`bench_v0.10.4_20261108_210000Z_abdbf89.md`](./tests/reports/bench_v0.10.4_20261108_210000Z_abdbf89.md))  
> **Provenance:** Slice `v0.10.4` (`abdbf89`) · Generated `2026-11-08T21:00:00Z` · Environment: `Linux x86_64 Docker (io_uring SQPOLL + WRPKRU)`  
> **Scorecard Across All 32 Production Workloads:** Beats **Deno (`v2.9.8`)** on **32/32**, **Node.js (`v27.0.0-alpha`)** on **32/32**, **Bun (`v1.4.4`)** on **29/32** cold one-shot, and **all 3 runtimes on 32/32 workloads in `--warm` pooled execution**.

#### 1. Core Architecture, HTTP Throughput & 1,000-Tenant MicroVM Density

| Metric | Slice (`v0.10.4` `SLICEBC\x01`) | Bun (`v1.4.4`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha`) | Slice Advantage |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **In-Process Isolate Cold Start (`P50` / `P99`)** | **`1.03 us` / `2.42 us`** | N/A (Process/VM) | N/A (Process/VM) | N/A (Process/VM) | **~120,000x faster than 125ms Firecracker VM** |
| **1,000-Tenant MicroVM Swarm (Boot + Exec)** | **`1.28 ms` (`10.06 MB` RSS)** | > 5,000 ms | > 15,000 ms | > 30,000 ms | **> 3,900x faster / ~32,000x denser** |
| **500-Module Monorepo Cold Boot (`stat()` Calls)** | **`3.41 ms` (`0 stat()`)** | `12.72 ms` (`500+ stat`) | `41.80 ms` (`500+ stat`) | `70.38 ms` (`1,500+ stat`) | **`3.7x` faster than Bun / `20.6x` Node.js** |
| **HTTP/1.1 Keep-Alive Throughput (`16k` Reqs)** | **`158,534 req/s`** (Tier-0)<br>**`120,333 req/s`** (Tier-1) | `91,234 req/s` | `98,377 req/s` | `45,123 req/s` | **`1.74x` (`Tier-0`) / `1.32x` (`Tier-1`) vs `Bun.serve`** |
| **Peak Process Memory (`Max RSS` Across Suite)** | **`13.50 – 21.38 MB`** | `26.00 – 40.22 MB` | `45.29 – 61.82 MB` | `69.70 – 92.75 MB` | **`2.0x–2.3x` less than Bun / `4.3x–6.8x` less than Node** |
| **Multi-Tenant Security Containment** | **`5 / 5 PASS`** (OCap+SES+Fuel+DLP) | `0 / 5` (Ambient) | `2 / 5` (Coarse CLI) | `0 / 5` (Ambient) | **Hardware & Language Zero-Trust Isolation** |

#### 2. Real-Life Workload Summary by Domain (All 32 Workloads Aggregated)

| Workload Domain & Stack Coverage | Suites | Slice Cold `.cas` Avg (Range) | Slice Warm (`--warm`) Avg | Bun `v1.4.4` Avg | Deno `v2.9.8` Avg | Node.js `v27.0.0-alpha` Avg | Category Speedup vs Bun / Node.js |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **Core Edge, Auth & Crypto (`Apps 01, 04, 05, 16`)**<br><sub>Multi-file TS, SHA-256/HMAC, JWT/OAuth2, `.env` Auto-DLP</sub> | `4` | **`8.18 ms`** (`6.46–10.88 ms`) | **`2.44 ms`** | `14.73 ms` | `43.72 ms` | `64.58 ms` | **`1.80x Bun` (`6.05x` Warm) / `7.90x Node`** |
| **NPM Ecosystem, AST & Disk I/O (`Apps 02, 03, 06, 07, 08, 20`)**<br><sub>`semver`, `flatted`, `dlv`/`dset`, `mitt`, 10-Shard JSONL I/O, `Buffer`</sub> | `6` | **`11.27 ms`** (`7.89–14.51 ms`) | **`3.68 ms`** | `17.34 ms` | `48.13 ms` | `65.69 ms` | **`1.54x Bun` (`4.71x` Warm) / `5.83x Node`** |
| **WinterTC Web Standards, Fetch Mesh & DLP (`Apps 09–14, 17–19`)**<br><sub>`Request`/`Response`, Static CDN `304`, SSE, Live `fetch`, WebCrypto</sub> | `9` | **`15.05 ms`** (`8.12–29.06 ms`) | **`7.76 ms`** | `18.50 ms` | `52.32 ms` | `93.64 ms` | **`1.23x Bun` (`2.38x` Warm) / `6.22x Node`** |
| **Realtime WebSockets, Workers & Wasm/TSX (`Apps 15, 21–24`)**<br><sub>RFC 6455 WS, `worker_threads` + TCP/UDP, Pure-Rust `wasmi` Wasm, TSX SSR</sub> | `5` | **`14.16 ms`** (`8.65–19.96 ms`) | **`2.60 ms`** | `27.66 ms` | `62.10 ms` | `84.23 ms` | **`1.95x Bun` (`10.66x` Warm) / `5.95x Node`** |
| **Full-Stack Edge & Node Frameworks (`FW 01–03`)**<br><sub>`Hono v4.6.20`, `itty-router + UnJS Nitro`, `Express/Polka node:http`</sub> | `3` | **`13.59 ms`** (`9.12–17.08 ms`) | **`5.51 ms`** | `18.08 ms` | `47.21 ms` | `83.92 ms` | **`1.33x Bun` (`3.28x` Warm) / `6.17x Node`** |
| **AI Agent, Multi-Tenant SaaS & Chaos Case Studies (`CS 01–05`)**<br><sub>Depth-4 ToT Agent, MCP Code Mode, 12-Tenant Mesh, Wasm CDR, 48 Payment Sagas</sub> | `5` | **`18.70 ms`** (`13.20–26.68 ms`) | **`7.60 ms`** | `19.88 ms` | `52.68 ms` | `76.52 ms` | **`1.06x Bun` (`2.62x` Warm) / `4.09x Node`** |

#### 3. Flagship Real-World Representative Workloads (8 of 32 Highlights)

| # | Flagship Workload | Slice Cold `.cas` | Slice Warm (`--warm`) | Bun `v1.4.4` | Deno `v2.9.8` | Node.js `v27.0.0-alpha` | Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`6.46 ms`** | **`0.97 ms`** | `14.36 ms` | `40.52 ms` | `64.39 ms` | **`2.22x Bun` (`14.80x` Warm) / `9.96x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`7.89 ms`** | **`2.04 ms`** | `14.36 ms` | `42.52 ms` | `63.43 ms` | **`1.82x Bun` (`7.03x` Warm) / `8.04x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`14.46 ms`** | **`8.06 ms`** | `21.70 ms` | `52.93 ms` | `109.09 ms` | **`1.50x Bun` (`2.69x` Warm) / `7.54x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`10.58 ms`** | **`0.61 ms`** | `22.16 ms` | `54.88 ms` | `71.15 ms` | **`2.10x Bun` (`36.57x` Warm) / `6.73x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`19.96 ms`** | **`2.16 ms`** | `54.18 ms` | `110.81 ms` | `99.21 ms` | **`2.71x Bun` (`25.09x` Warm) / `4.97x Node`** |
| **FW-01** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`14.58 ms`** | **`5.65 ms`** | `19.20 ms` | `51.45 ms` | `93.42 ms` | **`1.32x Bun` (`3.40x` Warm) / `6.41x Node`** |
| **CS-01** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`26.68 ms`** | **`12.71 ms`** | `28.81 ms` | `68.86 ms` | `86.66 ms` | **`1.08x Bun` (`2.27x` Warm) / `3.25x Node`** |
| **CS-05** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`17.48 ms`** | **`9.46 ms`** | `19.62 ms` | `61.47 ms` | `82.26 ms` | **`1.12x Bun` (`2.07x` Warm) / `4.70x Node`** |

*Full 32-workload latency breakdown, standalone binary metrics, peak RSS per app, and historical reports: **[`tests/reports/latest.md`](./tests/reports/latest.md)**.*
<!-- BENCHMARK_SUMMARY_END -->

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
* `slice.toml`: Declarative Object-Capability (OCap) policy, engine tier (`tier = 1`, `boacas = true`), and resource cage configuration.
* `slice.d.ts`: Ambient TypeScript declarations for `Slice.*`, `WinterTC.*`, `slice:test`, `slice:jsx`, and `cloudflare:sockets`.
* `tsconfig.json`: Pre-configured TypeScript compiler config (`"moduleResolution": "bundler"`, `"jsxImportSource": "slice:jsx"`).
* `src/index.ts`: TypeScript edge entrypoint.

### 3. Run TypeScript Files, Folder Apps, or Pre-Packed CAS Bundles (`slice run`)

By default, Slice runs on its optimal **Tier-1 + `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS** path:

```bash
# Execute a raw TypeScript file or an entire project folder (with package.json & node_modules)
slice run ./src/index.ts --allow-env=SERVICE_STAGE=prod --metrics
slice run ./tests/apps/02-npm-ecosystem-suite --metrics

# Pack an entire folder app (including transitive node_modules) into a 64-byte aligned BLAKE3 .cas archive
# Emits SLICEBC\x01 Precompiled Boa CodeBlock + Oxc deferred function module spans by default
slice pack ./tests/apps/02-npm-ecosystem-suite ./bundle.cas

# Execute the memory-mapped .cas bundle with 0 stat() syscalls and 0 runtime JS parsing
slice run ./bundle.cas --metrics
slice run ./bundle.cas --warm --metrics
```

### 4. Compile a Standalone Zero-Dependency Executable (`slice compile`)

Embeds your compiled `.cas` archive and `SLICEBC\x01` precompiled `CodeBlock` bytecode directly into a self-contained native binary (`SLICEEXE`):

```bash
slice compile ./tests/apps/01-fintech-checkout -o my-edge-app
./my-edge-app --metrics
```

### 5. Launch the Multi-Tenant WinterTC HTTP Server (`slice serve`)

`slice serve` runs a shared-nothing **Thread-Per-Core (`--workers=N`)** HTTP/1.1 `Keep-Alive` & pipelined server that executes every incoming HTTP request inside a fresh, hardware- and OCap-isolated `SliceIsolate` in microseconds (**`163,766 req/s`** in Tier-0 CAS mode, **`126,566 req/s`** in Tier-1 WinterTC mode):

```bash
# Start multi-worker Keep-Alive server on port 3000 with 8 shared-nothing worker threads
slice serve ./bundle.cas --port=3000 --workers=8

# Or serve a multi-tenant directory of .cas archives with automatic zero-downtime hot-swapping
slice serve --port=3000 --workers=8 --tenant-dir=./tenants

# Or run a self-contained pipelined Keep-Alive smoke test with telemetry headers
slice serve ./bundle.cas --smoke-test
```

Inside TypeScript/JavaScript handlers, you can use either the **Standard WinterTC / Cloudflare Workers / Bun / Deno `export default { fetch(req, env, ctx) }`** API (`Request`, `Response`, `Headers`, `URL`, `URLPattern`, `URLSearchParams`, `Response.json`, `Response.redirect`, `Response.sse`) or the zero-allocation **Slice Synchronous Bridge** (`Slice.request()`, `Slice.respond()`, `Slice.waitUntil()`), alongside the **16-Way Sharded Edge KV & Atomic Counter Slab** (`Slice.kv`):
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

### 6. AI Agent "Code Mode" Sandbox (`Slice.evalSandbox`), `< 5 us` Checkpoints (`Slice.checkpoint`), & Native MCP Server (`slice mcp`)

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

### 7. Built-In Time-Travel Test Runner (`slice test`) & Interactive Case Studies Workbench (`tests/case_studies/`)

```bash
# Run all *.test.ts / *.spec.ts suites with automatic < 5 us VFS + KV rollback after every test()
slice test ./tests/apps/15-time-travel-test-suite

# Or watch for file changes via OS-native notify (< 5 ms incremental re-run)
slice test ./tests/apps/24-tsx-ssr-and-wasm-edge-pipeline --watch-once

# Explore the 5 interactive production Case Studies (Visual Terminal Cockpit or Browser UI)
make -C tests/case_studies cockpit
make -C tests/case_studies serve
```

---

## Unified 4-Layer Configuration System (`slice.toml`, Inline Directives, `SLICE_*` Env Vars, & CLI Flags)

Slice resolves every engine, sandbox, capability, and server setting through a deterministic **4-layer precedence hierarchy** (lowest to highest priority):

```text
Built-in Optimal Defaults (tier = 1, boacas = true, fuel = 50M, memory_kb = 4096)
  < 1. Project Manifest (slice.toml + auto-loaded .env)
  < 2. Inline Source Directives (// @slice-... in the top 64 lines of entrypoint .ts/.js)
  < 3. Process Environment Variables (SLICE_*)
  < 4. Explicit CLI Flags (--*)
```

### Layer 1: Project Manifest (`slice.toml`)

```toml
[package]
name = "my-edge-service"
version = "0.1.0"
entrypoint = "./src/index.ts"

[engine]
tier = 1                  # 0 = Tier0LLIntSnapshot, 1 = Tier1Baseline (default), 2 = Tier2PeakNative
boacas = true             # Emit & execute SLICEBC\x01 Precompiled Boa CodeBlock CAS (default: true)
warm = false              # Pre-warm thread-local WarmBoaRealm pool before timing execution
deterministic = 20261106  # Optional deterministic xorshift64* PRNG & monotonic virtual clock seed

[limits]
fuel = 50_000_000         # Deterministic CPU instruction fuel budget
memory_kb = 4096          # Per-isolate heap memory cage in KB (4 MB default)

[capabilities]
allow_read = ["/app/public", "/app/config"]
allow_write = ["/tmp/scratch"]
allow_net = ["api.internal:443", "127.0.0.1:18095"]
audit_egress = ["sk_live_secret"]
env_file = ".env"         # Auto-classifies sensitive keys (*SECRET*, *TOKEN*, *KEY*, *PASSWORD*) into Egress DLP

[capabilities.env]
SERVICE_STAGE = "production"

[server]
port = 3000
workers = 8
static_dir = "./public"
tenant_dir = "./tenants"
```

### Layer 2: Inline Source Directives (`// @slice-...`)

Single-file scripts and self-contained tests can configure their execution tier, resource cages, and OCap permissions directly inside the top 64 comment lines of the entrypoint `.ts`/`.js` file:

```typescript
// @slice-tier: 1
// @slice-boacas: true
// @slice-warm: true
// @slice-fuel: 25000000
// @slice-memory-kb: 8192
// @slice-allow-read: /workspace/data, /workspace/config
// @slice-allow-write: /tmp/out
// @slice-allow-net: 127.0.0.1:8080, api.example.com:443
// @slice-allow-env: NODE_ENV=production, API_REGION=us-east-1
// @slice-audit-egress: sk_live_demo_secret
// @slice-deterministic: 42
```

### Complete Cross-Layer Settings Reference

| Setting | Optimal Default | `slice.toml` Key | Inline Directive (`// @slice-...`) | Environment Variable (`SLICE_*`) | CLI Flag (`--*`) |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Execution Tier** | `1` (`Tier1Baseline`) | `[engine] tier = 0\|1\|2` | `// @slice-tier: 0\|1\|2` | `SLICE_TIER=0\|1\|2` | `--tier=0\|1\|2` |
| **Precompiled Boa CAS** | `true` (`SLICEBC\x01`) | `[engine] boacas = true\|false` | `// @slice-boacas: true\|false` | `SLICE_BOACAS=1\|0` | `--boacas` / `--no-boacas` |
| **Warm Realm Pool** | `false` | `[engine] warm = true\|false` | `// @slice-warm: true\|false` | `SLICE_WARM=1\|0` | `--warm` |
| **Deterministic Seed** | `None` | `[engine] deterministic = <u64>` | `// @slice-deterministic: <u64>` | `SLICE_DETERMINISTIC=<u64>` | `--deterministic=<u64>` |
| **CPU Fuel Ceiling** | `50,000,000` | `[limits] fuel = <u64>` | `// @slice-fuel: <u64>` | `SLICE_FUEL=<u64>` | `--fuel=<u64>` |
| **Memory Cage (KB)** | `4096` (`4 MB`) | `[limits] memory_kb = <usize>` | `// @slice-memory-kb: <usize>` | `SLICE_MEMORY_KB=<usize>` | `--memory-kb=<usize>` |
| **Read Capability** | `[]` (Deny-All) | `[capabilities] allow_read = [...]` | `// @slice-allow-read: p1, p2` | `SLICE_ALLOW_READ=p1,p2` | `--allow-read=p1,p2` |
| **Write Capability** | `[]` (Deny-All) | `[capabilities] allow_write = [...]` | `// @slice-allow-write: p1, p2` | `SLICE_ALLOW_WRITE=p1,p2` | `--allow-write=p1,p2` |
| **Network Capability** | `[]` (Deny-All) | `[capabilities] allow_net = [...]` | `// @slice-allow-net: h1:p1` | `SLICE_ALLOW_NET=h1:p1` | `--allow-net=h1:p1` |
| **Environment Access** | `{}` (Deny-All) | `[capabilities.env] KEY = "VAL"` | `// @slice-allow-env: K=V, K2` | `SLICE_ALLOW_ENV=K=V,K2` | `--allow-env=K=V` |
| **Dotenv Auto-DLP** | `.env` (if present) | `[capabilities] env_file = ".env"` | `// @slice-env-file: .env` | `SLICE_ENV_FILE=.env` | `--env-file=.env` |
| **Egress DLP Secrets** | Auto from `.env` | `[capabilities] audit_egress = [...]` | `// @slice-audit-egress: s1, s2` | `SLICE_AUDIT_EGRESS=s1,s2` | `--audit-egress=s1` |
| **HTTP Server Port** | `3000` | `[server] port = <u16>` | — | `SLICE_PORT=<u16>` | `--port=<u16>` |
| **HTTP Worker Threads** |Logical CPUs | `[server] workers = <usize>` | — | `SLICE_WORKERS=<usize>` | `--workers=<usize>` |
| **Static Assets Dir** | `None` | `[server] static_dir = "DIR"` | — | `SLICE_STATIC_DIR=DIR` | `--static=DIR` |
| **Multi-Tenant CAS Dir**| `None` | `[server] tenant_dir = "DIR"` | — | `SLICE_TENANT_DIR=DIR` | `--tenant-dir=DIR` |

---

## CLI Reference (All 13 Subcommands)

| Command | Description |
| :--- | :--- |
| `slice init [dir]` | Scaffold a new TypeScript Edge project with `slice.toml`, `slice.d.ts`, `tsconfig.json`, and `src/index.ts`. |
| `slice run [target] [flags]` | Execute a `.ts`/`.js`/`.tsx`/`.mjs`/`.cjs` file, project folder, or `.cas` archive (`--tier=0\|1\|2`, `--boacas`/`--no-boacas`, `--warm`, `--deterministic=SEED`, `--audit-egress=SECRET`, `--watch`, `--metrics`). |
| `slice pack <in> <out.cas>` | Flatten and pre-compile a module tree (plus optional `--static=DIR` assets) into a 64-byte aligned BLAKE3 `.cas` archive (`SLICEBC\x01` by default). |
| `slice compile <in> -o <bin>` | Produce a zero-dependency standalone executable (`SLICEEXE`, embeds `SLICEBC\x01` CAS and optional `--static=DIR` assets). |
| `slice serve [target] [flags]` | Run the multi-tenant WinterTC HTTP/1.1 Keep-Alive MicroVM server (`--port=N`, `--workers=N`, `--tenant-dir=DIR`, `--static=DIR`, `--smoke-test`). |
| `slice test [target] [flags]` | Built-in Time-Travel Test Runner (`slice:test` `describe`/`test`/`expect`, `< 5 us` automatic VFS+KV rollback per test, `--watch` / `--watch-once`). |
| `slice mcp [flags]` | Launch the built-in JSON-RPC 2.0 Model Context Protocol (MCP) server over `stdio` (`--smoke-test`, `--audit-egress=SECRET`). |
| `slice inspect <target>` | Disassemble `SLICEBC\x01` / `SLICEBOA` / `SLICESNP` bytecode and inspect BLAKE3 CAS offsets and static asset tables. |
| `slice repl` | Interactive SES-locked TypeScript/JavaScript compartment shell (`:help`, `:metrics`, `:lockdown`, `:tier 0\|1\|2`, `:reset`). |
| `slice doctor` | Audit host kernel `io_uring`/`IOCP` completion rings, Intel MPK hardware keys, RSS, and 5 security containment vectors. |
| `slice bench [flags]` | Run built-in cold-start, Zero-Stat CAS module loading (`--modules=500`), and Tier-2 compute benchmarks (`--json`). |
| `slice microvm-bench [flags]` | Spawn 1,000 isolated tenants (`--tenants=1000`) and benchmark P50/P99 cold-start, CAS, I/O rings, and 5/5 security vectors. |
| `slice verify-security` | Execute the 5-vector OCap, SES prototype lockdown, and CPU fuel DoS verification suite (`--json`). |

---

## Running the Reproducible Docker Benchmark & Verification Suite

Run the full 32-workload Docker benchmark suite (which automatically writes a timestamped report to [`tests/reports/`](./tests/reports/latest.md) and syncs documentation tables) or filter by tags (`all`, `apps`, `frameworks`, `case_studies`, `n1`, `n2`, `n3`, `f1`, `f2`, `f3`, `f4`, `wasm`, `serve`, `swarm`):

```powershell
# Automated one-command runner (builds Docker image, runs benchmarks, generates tests/reports/latest.md, and syncs docs)
powershell -ExecutionPolicy Bypass -File .\scripts\run_benchmarks.ps1

# Or run a specific tagged subset (e.g. frameworks, case_studies, serve, swarm)
powershell -ExecutionPolicy Bypass -File .\scripts\run_benchmarks.ps1 -Tags "frameworks,case_studies,serve,swarm"

# Run all workspace unit & integration tests locally
cargo test --workspace
```

---

## Documentation (`docs/general/`, `docs/guides/`, `docs/system/`)

### User & Developer Manual by Chapter (`docs/general/`)
* **[Chapter 00: Master Documentation Index](./docs/general/00_index.md)**
* **[Chapter 01: Getting Started, Unified 4-Layer Configuration & Complete CLI Reference](./docs/general/01_getting_started_and_cli.md)**
* **[Chapter 02: Hybrid Three-Tier Execution Engine & Default `SLICEBC\x01` Precompiled CAS (`slice-engine`)](./docs/general/02_execution_engine_and_tiers.md)**
* **[Chapter 03: Zero-Stat CAS Archive, Oxc Bundler & Standalone Binaries (`slice-cas`)](./docs/general/03_cas_bundler_and_standalone_binaries.md)**
* **[Chapter 04: Security Architecture — OCap, SES Lockdown, Fuel Cage & Hardware MPK (`slice-sandbox`)](./docs/general/04_security_ocap_ses_and_hardware_mpk.md)**
* **[Chapter 05: Multi-Tenant HTTP/1.1 Server, WinterTC Bridge & Sharded Edge KV (`slice serve`)](./docs/general/05_http_server_wintertc_and_edge_kv.md)**
* **[Chapter 06: AI Agent "Code Mode" Sandbox, `< 5 us` Checkpoints & Native MCP Server (`slice mcp`)](./docs/general/06_ai_agent_sandbox_checkpoints_and_mcp.md)**
* **[Chapter 07: Node.js Compatibility (`node:*`), Native Intrinsics & External NPM Packages](./docs/general/07_node_compat_intrinsics_and_npm.md)**
* **[Chapter 08: Kernel Completion Rings (`slice-io`) & Embedding Slice in Rust](./docs/general/08_kernel_io_rings_and_rust_embedding.md)**
* **[Chapter 09: Benchmarks, Time-Travel Testing & 31-Workload Docker Verification Suite](./docs/general/09_benchmarks_testing_and_docker_suite.md)**
* **[Runtime Compatibility Matrix (`docs/general/COMPATIBILITY.md`)](./docs/general/COMPATIBILITY.md)**

### Developer How-To Guides (`docs/guides/`)
* **[Developer Guides & Recipes Index (`docs/guides/README.md`)](./docs/guides/README.md)**
* **[Guide 01: Building & Deploying Hono (`v4`) and UnJS Edge Microservices (`docs/guides/01_hono_and_unjs_edge_microservices.md`)](./docs/guides/01_hono_and_unjs_edge_microservices.md)**
* **[Guide 02: TypeScript IDE Setup (`slice.d.ts`), Linting (`oxlint` / `Biome`), & `.wasm` Modules (`docs/guides/02_typescript_ide_linting_and_wasm.md`)](./docs/guides/02_typescript_ide_linting_and_wasm.md)**

### System Engineering Specifications (`docs/system/`, `vendor/`, & `tests/`)
* **[00: System Architecture & Internal Specification Index (`docs/system/00_index.md`)](./docs/system/00_index.md)**
* **[01: `slice-cas` — Zero-Stat CAS, Oxc Compiler & AST Tree-Shaker (`docs/system/01_cas_bundler_and_oxc_compiler.md`)](./docs/system/01_cas_bundler_and_oxc_compiler.md)**
* **[02: `slice-engine` — Hybrid Three-Tier Execution Pipeline, `SLICEBC\x01` & `WarmBoaRealm` (`docs/system/02_tiered_execution_and_boa_vm.md`)](./docs/system/02_tiered_execution_and_boa_vm.md)**
* **[03: `slice-sandbox` — 5-Ring Zero-Trust OCap, Dynamic SES & Hardware MPK (`docs/system/03_security_sandbox_ocap_and_mpk.md`)](./docs/system/03_security_sandbox_ocap_and_mpk.md)**
* **[04: Networking, WinterTC Web Platform & Async Event Loop (`docs/system/04_networking_wintertc_and_event_loop.md`)](./docs/system/04_networking_wintertc_and_event_loop.md)**
* **[05: Pure-Rust WebAssembly, Web Workers & Direct Kernel I/O (`docs/system/05_wasm_workers_and_kernel_io.md`)](./docs/system/05_wasm_workers_and_kernel_io.md)**
* **[06: Serverless Edge KV, AI Agent "Code Mode" & MCP Server (`docs/system/06_edge_kv_ai_sandbox_and_mcp.md`)](./docs/system/06_edge_kv_ai_sandbox_and_mcp.md)**
* **[07: Production Tooling & Deployment Guide (`docs/system/07_production_tooling_guide.md`)](./docs/system/07_production_tooling_guide.md)**
* **[08: Empirical Benchmarks & 31-Workload Comparisons (`docs/system/08_benchmarks_and_comparisons.md`)](./docs/system/08_benchmarks_and_comparisons.md)**
* **[09: Cutting-Edge 2026 Research Deep Dive (`docs/system/09_research_deep_dive.md`)](./docs/system/09_research_deep_dive.md)**
* **[Vendored `boa_engine` `SLICEBC\x01` Patch Specification (`vendor/boa_engine/SLICE_PATCH.md`)](./vendor/boa_engine/SLICE_PATCH.md)**
* **[Latest Full 32-Workload Benchmark Report (`tests/reports/latest.md`)](./tests/reports/latest.md)**
* **[Docker 31-Workload Production Benchmarks (`tests/README.md`)](./tests/README.md)**
* **[Framework Integration Benchmarks (`tests/frameworks/README.md`)](./tests/frameworks/README.md)**
* **[Interactive Production Case Studies & Visual Cockpit (`tests/case_studies/README.md`)](./tests/case_studies/README.md)**
