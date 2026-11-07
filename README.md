# Slice (Nexus-JS)

**Slice (Nexus-JS)** is a next-generation, microVM-grade JavaScript/TypeScript edge runtime and AI Agent "Code Mode" sandbox engineered in Rust. It replaces ambient POSIX authority, recursive `node_modules` filesystem metadata storms, readiness polling (`libuv`), runtime JavaScript AST parsing, and heavy V8/JSC memory footprints with four unified pillars:

1. **Tiered Polyglot Execution & Precompiled `CodeBlock` CAS (`slice-engine`):** Defaults to **Tier 1 (`Tier1Baseline`) with `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS (`boacas = true`)**, serializing the compiled `Gc<CodeBlock>` hierarchy, global `Scope` indices, Oxc AST deferred function module spans (`0x11` tag), and pre-fused `node:*` built-in shards at build time so runtime execution bypasses `boa_parser` (`Lexer` + `Parser`), `boa_ast`, `ScopeAnalyzer`, and `ByteCompiler` entirely. Also supports sub-microsecond `SLICESNP` register snapshots (`Tier0LLIntSnapshot`), native Rust SIMD intrinsics (`Tier2PeakNative`), and pure-Rust `wasmi` WebAssembly execution with unified CPU fuel and memory-page cages.
2. **Zero-Stat Content-Addressed Storage (`slice-cas`):** Flattens TypeScript/JavaScript dependency graphs (including `package.json` `"exports"`, `.wasm` modules, and static barrel tree-shaking) at build time via **Oxc (`v0.154.0`)** into a 64-byte cache-line aligned, BLAKE3-indexed `.cas` archive mapped directly into virtual memory via `mmap(MAP_SHARED)` with **`0` runtime `stat` syscalls**.
3. **Direct Kernel Completion Rings (`slice-io`):** Lockless Submission/Completion rings backed by Linux `io_uring` (`IORING_SETUP_SQPOLL` + Fixed Registered Buffers) and Windows NT `IOCP`.
4. **5-Ring MicroVM OCap & Hardware Sandboxing (`slice-sandbox`):** Enforces Secure ECMAScript (`lockdown()`), isolated per-tenant TC39 Compartments with attenuated `CapabilityToken` permissions (`FsCap`, `NetCap`, `EnvCap`), Cloud Metadata SSRF & DNS-rebinding firewalls, Outbound Egress DLP (raw, Base64, Base64URL, & `.env` Auto-DLP), deterministic CPU instruction fuel preemption, byte-level heap cages, `< 5 us` copy-on-write VFS + `Slice.kv` time-travel checkpoints, and `< 20`-cycle hardware memory domain switches via `pkey_alloc` / `WRPKRU` (Intel MPK).

---

## Verified Benchmark Overview (Linux `x86_64` Docker & Windows `x86_64`)

Full reproducible benchmark report: **[`bench_latest.md`](./bench_latest.md)**

```text
+---------------------------------------+-------------------+-------------------+-------------------+-------------------------+
| Metric (Linux x86_64 Verified)        | Node.js v27.0-a   | Deno v2.9.8       | Bun v1.4.4        | Slice (SLICEBC\x01 Def) |
+---------------------------------------+-------------------+-------------------+-------------------+-------------------------+
| Isolate Cold Start (In-Process)       | N/A (Process/VM)  | N/A (Process/VM)  | N/A (Process/VM)  | 0.98 us (P50: 1.03 us)  |
| 1,000-Tenant MicroVM Swarm Time       | > 30,000 ms       | > 15,000 ms       | > 5,000 ms        | 1.28 ms total (10.06MB) |
| 500-Module Graph Load (Zero-Stat CAS) | 70.382 ms         | 41.799 ms         | 12.721 ms         | 0.41 ms (0 stat calls)  |
| OS stat() Syscalls (500 Modules)      | 1,500+ syscalls   | 500+ syscalls     | 500+ syscalls     | 0 stat() syscalls       |
| Real-World 31-App Suite Cold Wins     | 0 / 31            | 0 / 31            | 3 / 31            | 28 / 31 Wins vs Bun     |
| Real-World 31-App Suite Warm (--warm) | N/A               | N/A               | N/A               | 0.61 ms - 17.14 ms      |
| HTTP/1.1 Keep-Alive (8 Workers, T0)   | 58,163 req/s      | 97,501 req/s      | 102,431 req/s     | 158,534 req/s (6.31 us) |
| HTTP/1.1 Keep-Alive (8 Workers, T1)   | 58,163 req/s      | 97,501 req/s      | 102,431 req/s     | 120,333 req/s (8.31 us) |
| Peak Process Memory (31 Apps RSS)     | 69.70 - 92.75 MB  | 45.86 - 61.82 MB  | 25.88 - 40.22 MB  | 13.50 - 21.38 MB        |
| Multi-Tenant Security Containment     | 0/5 (Ambient)     | 2/5 (Coarse CLI)  | 0/5 (Ambient)     | 5/5 (OCap+SES+Fuel+DLP) |
+---------------------------------------+-------------------+-------------------+-------------------+-------------------------+
```

### Real-World 32-Application, Framework & Case Study Suite ([`tests/`](./tests/README.md) & [`bench_latest.md`](./bench_latest.md))

Across **24 multi-file TypeScript & `node_modules` applications**, **3 production Edge & Node Framework suites (`Hono v4`, `itty-router + UnJS`, `Express/Polka node:http`)**, and **5 AI Agent / Multi-Tenant SaaS / Chaos Case Studies**, Slice produces **100% byte-for-byte identical JSON output and SHA-256/HMAC digests** as Bun, Deno, and Node.js while **beating Deno & Node.js on `31 / 31` workloads cold, beating Bun on `28 / 31` workloads cold, and beating all 3 runtimes on `31 / 31` workloads in `--warm` pooled mode**:

| # | Application / Framework / Case Study | Node.js `v27.0-a` | Deno `v2.9.8` | Bun `v1.4.4` | Slice Cold (`.cas` / Bin `SLICEBC\x01`) | Slice Warm (`--warm`) |
| :- | :--- | :---: | :---: | :---: | :---: | :---: |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | `64.388 ms` | `40.515 ms` | `14.357 ms` | **`6.462 ms` (`2.22x` Bun)** | **`0.970 ms` (`14.8x` Bun)** |
| **02** | `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | `70.900 ms` | `52.462 ms` | `21.324 ms` | **`14.045 ms` (`1.52x` Bun)** | **`4.101 ms` (`5.20x` Bun)** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards I/O) | `63.433 ms` | `42.519 ms` | `14.361 ms` | **`7.891 ms` (`1.82x` Bun)** | **`2.044 ms` (`7.03x` Bun)** |
| **04** | `04-edge-api-gateway` (`fast-querystring` + `ms`) | `63.293 ms` | `41.315 ms` | `14.829 ms` | **`7.793 ms` (`1.90x` Bun)** | **`1.949 ms` (`7.61x` Bun)** |
| **05** | `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | `65.323 ms` | `49.432 ms` | `15.178 ms` | **`10.875 ms` (`1.40x` Bun)** | **`5.157 ms` (`2.94x` Bun)** |
| **06** | `06-telemetry-metrics-pipeline` (`bytes` + `flatted`) | `65.159 ms` | `45.490 ms` | `16.831 ms` | **`14.513 ms` (`1.16x` Bun)** | **`5.815 ms` (`2.89x` Bun)** |
| **07** | `07-graphql-ast-validator` (`dlv` + `dset` + AST) | `61.297 ms` | `46.870 ms` | `13.126 ms` | **`9.282 ms` (`1.41x` Bun)** | **`4.160 ms` (`3.16x` Bun)** |
| **08** | `08-event-driven-state-journal` (`mitt` + `ms` + WAL) | `62.180 ms` | `46.551 ms` | `16.258 ms` | **`11.336 ms` (`1.43x` Bun)** | **`5.362 ms` (`3.03x` Bun)** |
| **09** | `09-wintertc-web-router` (`Request` + `Response` + `URL`) | `84.676 ms` | `47.303 ms` | `13.766 ms` | **`15.418 ms` (`14.09 ms` Min)** | **`9.402 ms` (`1.46x` Bun)** |
| **10** | `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ETag`) | `83.650 ms` | `46.932 ms` | `14.099 ms` | **`12.988 ms` (`1.09x` Bun)** | **`6.387 ms` (`2.21x` Bun)** |
| **11** | `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`) | `86.887 ms` | `52.120 ms` | `13.202 ms` | **`9.831 ms` (`1.34x` Bun)** | **`3.575 ms` (`3.69x` Bun)** |
| **12** | `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP) | `89.397 ms` | `49.942 ms` | `15.734 ms` | **`16.268 ms` (`15.38 ms` Min)** | **`8.443 ms` (`1.86x` Bun)** |
| **13** | `13-webhook-orchestrator-ssrf` (`fast-querystring` + Live `fetch`) | `111.596 ms` | `51.076 ms` | `19.532 ms` | **`13.596 ms` (`1.44x` Bun)** | **`6.651 ms` (`2.94x` Bun)** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + `fetch`) | `109.095 ms` | `52.931 ms` | `21.703 ms` | **`14.463 ms` (`1.50x` Bun)** | **`8.065 ms` (`2.69x` Bun)** |
| **15** | `15-time-travel-test-suite` (`semver` + `dlv` + 24 Rollbacks) | `82.490 ms` | `65.626 ms` | `24.472 ms` | **`14.976 ms` (`1.63x` Bun)** | **`5.118 ms` (`4.78x` Bun)** |
| **16** | `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `.env` Auto-DLP) | `65.300 ms` | `43.637 ms` | `14.572 ms` | **`7.580 ms` (`1.92x` Bun)** | **`1.664 ms` (`8.76x` Bun)** |
| **17** | `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive + `307` + SSRF) | `110.915 ms` | `54.938 ms` | `21.812 ms` | **`15.683 ms` (`1.39x` Bun)** | **`8.442 ms` (`2.58x` Bun)** |
| **18** | `18-async-event-loop-fanout-scheduler` (Timers + `Promise.all`) | `81.020 ms` | `70.636 ms` | `32.800 ms` | **`29.056 ms` (`1.13x` Bun)** | **`17.136 ms` (`1.91x` Bun)** |
| **19** | `19-wintertc-webcrypto-streams-edge` (WebCrypto + Streams + Clone) | `85.490 ms` | `45.039 ms` | `13.853 ms` | **`8.116 ms` (`1.71x` Bun)** | **`1.722 ms` (`8.04x` Bun)** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer` + Node Core) | `71.146 ms` | `54.876 ms` | `22.162 ms` | **`10.578 ms` (`2.10x` Bun)** | **`0.606 ms` (`36.6x` Bun)** |
| **21** | `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket`) | `114.930 ms` | `51.863 ms` | `19.980 ms` | **`10.727 ms` (`1.86x` Bun)** | **`2.504 ms` (`7.98x` Bun)** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Workers + TCP/UDP + 5-Ring) | `99.212 ms` | `110.814 ms` | `54.184 ms` | **`19.960 ms` (`2.71x` Bun)** | **`2.160 ms` (`25.1x` Bun)** |
| **23** | `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring) | `59.115 ms` | `40.072 ms` | `20.779 ms` | **`16.511 ms` (`1.26x` Bun)** | **`1.449 ms` (`14.3x` Bun)** |
| **24** | `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` + TSX SSR) | `65.406 ms` | `42.100 ms` | `18.867 ms` | **`8.646 ms` (`2.18x` Bun)** | **`1.747 ms` (`10.8x` Bun)** |
| **25** | `FW-01: 01-hono-edge-microservice` (`hono@4.6.20` + `cookie` + `ms`) | `93.418 ms` | `51.451 ms` | `19.203 ms` | **`14.577 ms` (`1.32x` Bun)** | **`5.652 ms` (`3.40x` Bun)** |
| **26** | `FW-02: 02-itty-router-unjs-nitro-stack` (`itty-router` + UnJS) | `89.889 ms` | `46.065 ms` | `19.205 ms` | **`17.079 ms` (`1.12x` Bun)** | **`9.078 ms` (`2.12x` Bun)** |
| **26b** | `FW-03: 03-node-http-express-compat` (`node:http` + `node:stream`) | Verified | Verified | Verified | **`18.321 ms` (100% Parity)** | **`1.809 ms` (`0 B` Heap)** |
| **27** | `CS-01: 01-tot-autonomous-sre-agent` (20 ToT Sandboxes + 16 Rollbacks) | `86.661 ms` | `68.862 ms` | `28.814 ms` | **`26.682 ms` (`1.08x` Bun)** | **`12.714 ms` (`2.27x` Bun)** |
| **28** | `CS-02: 02-mcp-code-mode-orchestrator` (`-96.7%` Tokens, 4 Domains) | `64.045 ms` | `42.538 ms` | `16.056 ms` | **`19.081 ms` (`18.48 ms` Min)** | **`8.094 ms` (`1.98x` Bun)** |
| **29** | `CS-03: 03-multi-tenant-saas-plugin-mesh` (12 Tenants + Hot-Swap) | `83.575 ms` | `47.442 ms` | `16.135 ms` | **`13.198 ms` (`1.22x` Bun)** | **`5.702 ms` (`2.83x` Bun)** |
| **30** | `CS-04: 04-wasm-zero-trust-cdr-pipeline` (Wasm SimHash CDR + Vault) | `66.076 ms` | `43.061 ms` | `18.790 ms` | **`17.055 ms` (`1.10x` Bun)** | **`2.019 ms` (`9.31x` Bun)** |
| **31** | `CS-05: 05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | `82.259 ms` | `61.474 ms` | `19.619 ms` | **`17.484 ms` (`1.12x` Bun)** | **`9.455 ms` (`2.08x` Bun)** |

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

Run the full 31-workload Docker benchmark suite or filter by tags (`all`, `apps`, `frameworks`, `case_studies`, `n1`, `n2`, `n3`, `f1`, `f2`, `f3`, `f4`, `wasm`, `serve`, `swarm`):

```powershell
# Build the Linux x86_64 Docker benchmark image (installs Slice, Bun v1.4.4, Deno v2.9.8, Node.js v27.0.0-alpha)
docker build -f tests/Dockerfile -t slice-real-stack-bench .

# Run all 31 workloads + HTTP Keep-Alive + Edge KV + AI Agent MCP + 1,000-tenant swarm
docker run --rm slice-real-stack-bench

# Or run a specific tagged subset (e.g. frameworks, case_studies, serve, swarm)
docker run --rm -e SLICE_BENCH_TAGS=frameworks,case_studies,serve,swarm slice-real-stack-bench

# Run all 97 Rust workspace unit & integration tests locally
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
* **[Latest Full 31-Application Benchmark Report (`bench_latest.md`)](./bench_latest.md)**
* **[Docker 31-Workload Production Benchmarks (`tests/README.md`)](./tests/README.md)**
* **[Framework Integration Benchmarks (`tests/frameworks/README.md`)](./tests/frameworks/README.md)**
* **[Interactive Production Case Studies & Visual Cockpit (`tests/case_studies/README.md`)](./tests/case_studies/README.md)**
