# Slice (Nexus-JS)

**Slice (Nexus-JS)** is a next-generation, microVM-grade JavaScript/TypeScript edge runtime and AI Agent "Code Mode" sandbox engineered in Rust. It replaces ambient POSIX authority, recursive `node_modules` filesystem metadata storms, readiness polling (`libuv`), runtime JavaScript AST parsing, and heavy V8/JSC memory footprints with four unified pillars:

1. **Tiered Polyglot Execution & Precompiled `CodeBlock` CAS (`slice-engine`):** Defaults to **Tier 1 (`Tier1Baseline`) with `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS (`boacas = true`)**, serializing the compiled `Gc<CodeBlock>` hierarchy, global `Scope` indices, Oxc AST deferred function module spans (`0x11` tag), and pre-fused `node:*` built-in shards at build time so runtime execution bypasses `boa_parser` (`Lexer` + `Parser`), `boa_ast`, `ScopeAnalyzer`, and `ByteCompiler` entirely. Also supports sub-microsecond `SLICESNP` register snapshots (`Tier0LLIntSnapshot`), native Rust SIMD intrinsics (`Tier2PeakNative`), and pure-Rust `wasmi` WebAssembly execution with unified CPU fuel and memory-page cages.
2. **Zero-Stat Content-Addressed Storage (`slice-cas`):** Flattens TypeScript/JavaScript dependency graphs (including `package.json` `"exports"`, `.wasm` modules, and static barrel tree-shaking) at build time via **Oxc (`v0.154.0`)** into a 64-byte cache-line aligned, BLAKE3-indexed `.cas` archive mapped directly into virtual memory via `mmap(MAP_SHARED)` with **`0` runtime `stat` syscalls**.
3. **Direct Kernel Completion Rings (`slice-io`):** Lockless Submission/Completion rings backed by Linux `io_uring` (`IORING_SETUP_SQPOLL` + Fixed Registered Buffers) and Windows NT `IOCP`.
4. **5-Ring MicroVM OCap & Hardware Sandboxing (`slice-sandbox`):** Enforces Secure ECMAScript (`lockdown()`), isolated per-tenant TC39 Compartments with attenuated `CapabilityToken` permissions (`FsCap`, `NetCap`, `EnvCap`), Cloud Metadata SSRF & DNS-rebinding firewalls, Outbound Egress DLP (raw, Base64, Base64URL, & `.env` Auto-DLP), deterministic CPU instruction fuel preemption, byte-level heap cages, `< 5 us` copy-on-write VFS + `Slice.kv` time-travel checkpoints, and `< 20`-cycle hardware memory domain switches via `pkey_alloc` / `WRPKRU` (Intel MPK).

---

## Verified Benchmark Overview (Linux `x86_64` Docker & Windows `x86_64`)

Full reproducible benchmark report: **[`bench_latest.md`](file:///p:/opensource/slice/bench_latest.md)**

```text
+---------------------------------------+-------------------+-------------------+-------------------+-------------------------+
| Metric (Linux x86_64 Verified)        | Node.js v27.0-a   | Deno v2.9.8       | Bun v1.4.4        | Slice (SLICEBC\x01 Def) |
+---------------------------------------+-------------------+-------------------+-------------------+-------------------------+
| Isolate Cold Start (In-Process)       | N/A (Process/VM)  | N/A (Process/VM)  | N/A (Process/VM)  | 0.99 us (P50: 1.27 us)  |
| 1,000-Tenant MicroVM Swarm Time       | > 30,000 ms       | > 15,000 ms       | > 5,000 ms        | 1.43 ms total (9.68 MB) |
| 500-Module Graph Load (Zero-Stat CAS) | 70.382 ms         | 41.799 ms         | 12.721 ms         | 0.38 ms (0 stat calls)  |
| OS stat() Syscalls (500 Modules)      | 1,500+ syscalls   | 500+ syscalls     | 500+ syscalls     | 0 stat() syscalls       |
| Real-World 31-App Suite Cold Wins     | 0 / 31            | 0 / 31            | 0 / 31 (1 Tie)    | 30 / 31 Wins (1 Tie)    |
| Real-World 31-App Suite Warm (--warm) | N/A               | N/A               | N/A               | 0.48 ms - 15.93 ms      |
| HTTP/1.1 Keep-Alive (8 Workers, T0)   | 58,607 req/s      | 94,185 req/s      | 105,077 req/s     | 163,766 req/s (6.11 us) |
| HTTP/1.1 Keep-Alive (8 Workers, T1)   | 58,607 req/s      | 94,185 req/s      | 105,077 req/s     | 126,566 req/s (7.90 us) |
| Peak Process Memory (31 Apps RSS)     | 69.77 - 94.58 MB  | 45.29 - 62.12 MB  | 26.00 - 40.31 MB  | 12.38 - 18.25 MB        |
| Multi-Tenant Security Containment     | 0/5 (Ambient)     | 2/5 (Coarse CLI)  | 0/5 (Ambient)     | 5/5 (OCap+SES+Fuel+DLP) |
+---------------------------------------+-------------------+-------------------+-------------------+-------------------------+
```

### Real-World 31-Application, Framework & Case Study Suite ([`tests/`](file:///p:/opensource/slice/tests/README.md) & [`bench_latest.md`](file:///p:/opensource/slice/bench_latest.md))

Across **24 multi-file TypeScript & `node_modules` applications**, **2 production Edge Frameworks (`Hono v4`, `itty-router + UnJS`)**, and **5 AI Agent / Multi-Tenant SaaS / Chaos Case Studies**, Slice produces **100% byte-for-byte identical JSON output and SHA-256/HMAC digests** as Bun, Deno, and Node.js while **beating Bun on `30 / 31` workloads cold** (and `31 / 31` warm):

| # | Application / Framework / Case Study | Node.js `v27.0-a` | Deno `v2.9.8` | Bun `v1.4.4` | Slice Cold (`.cas` / Bin `SLICEBC\x01`) | Slice Warm (`--warm`) |
| :- | :--- | :---: | :---: | :---: | :---: | :---: |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | `60.915 ms` | `36.032 ms` | `13.213 ms` | **`5.110 ms` (`2.59x` Bun)** | **`1.062 ms` (`12.4x` Bun)** |
| **02** | `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | `70.932 ms` | `53.288 ms` | `22.287 ms` | **`12.282 ms` (`1.81x` Bun)** | **`3.829 ms` (`5.82x` Bun)** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards I/O) | `63.240 ms` | `44.078 ms` | `14.283 ms` | **`8.839 ms` (`1.62x` Bun)** | **`4.150 ms` (`3.44x` Bun)** |
| **04** | `04-edge-api-gateway` (`fast-querystring` + `ms`) | `65.238 ms` | `44.246 ms` | `15.087 ms` | **`6.419 ms` (`2.35x` Bun)** | **`1.765 ms` (`8.55x` Bun)** |
| **05** | `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | `64.866 ms` | `44.484 ms` | `14.787 ms` | **`11.277 ms` (`1.31x` Bun)** | **`7.490 ms` (`1.97x` Bun)** |
| **06** | `06-telemetry-metrics-pipeline` (`bytes` + `flatted`) | `65.513 ms` | `45.902 ms` | `17.350 ms` | **`12.069 ms` (`1.44x` Bun)** | **`5.902 ms` (`2.94x` Bun)** |
| **07** | `07-graphql-ast-validator` (`dlv` + `dset` + AST) | `66.240 ms` | `42.680 ms` | `13.369 ms` | **`8.208 ms` (`1.63x` Bun)** | **`4.649 ms` (`2.88x` Bun)** |
| **08** | `08-event-driven-state-journal` (`mitt` + `ms` + WAL) | `65.470 ms` | `48.004 ms` | `16.231 ms` | **`11.357 ms` (`1.43x` Bun)** | **`6.280 ms` (`2.58x` Bun)** |
| **09** | `09-wintertc-web-router` (`Request` + `Response` + `URL`) | `86.761 ms` | `48.538 ms` | `13.953 ms` | **`13.560 ms` (`1.03x` Bun)** | **`9.664 ms` (`1.44x` Bun)** |
| **10** | `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ETag`) | `86.287 ms` | `48.888 ms` | `14.676 ms` | **`12.077 ms` (`1.22x` Bun)** | **`7.178 ms` (`2.04x` Bun)** |
| **11** | `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`) | `87.895 ms` | `50.490 ms` | `13.756 ms` | **`8.857 ms` (`1.55x` Bun)** | **`4.491 ms` (`3.06x` Bun)** |
| **12** | `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP) | `91.673 ms` | `50.496 ms` | `15.198 ms` | **`15.138 ms` (`1.01x` Bun)** | **`9.517 ms` (`1.60x` Bun)** |
| **13** | `13-webhook-orchestrator-ssrf` (`fast-querystring` + Live `fetch`) | `111.366 ms` | `51.269 ms` | `20.170 ms` | **`13.224 ms` (`1.53x` Bun)** | **`7.772 ms` (`2.60x` Bun)** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + `fetch`) | `110.073 ms` | `53.472 ms` | `21.868 ms` | **`14.926 ms` (`1.47x` Bun)** | **`9.896 ms` (`2.21x` Bun)** |
| **15** | `15-time-travel-test-suite` (`semver` + `dlv` + 24 Rollbacks) | `83.932 ms` | `66.463 ms` | `25.830 ms` | **`13.924 ms` (`1.86x` Bun)** | **`6.563 ms` (`3.94x` Bun)** |
| **16** | `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `.env` Auto-DLP) | `65.894 ms` | `46.307 ms` | `14.656 ms` | **`5.895 ms` (`2.56x` Bun)** | **`1.348 ms` (`10.9x` Bun)** |
| **17** | `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive + `307` + SSRF) | `112.711 ms` | `55.797 ms` | `22.700 ms` | **`15.351 ms` (`1.48x` Bun)** | **`10.119 ms` (`2.24x` Bun)** |
| **18** | `18-async-event-loop-fanout-scheduler` (Timers + `Promise.all`) | `81.968 ms` | `72.098 ms` | `33.709 ms` | **`26.714 ms` (`1.27x` Bun)** | **`17.652 ms` (`1.91x` Bun)** |
| **19** | `19-wintertc-webcrypto-streams-edge` (WebCrypto + Streams + Clone) | `85.061 ms` | `45.303 ms` | `14.005 ms` | **`6.728 ms` (`2.08x` Bun)** | **`1.412 ms` (`9.92x` Bun)** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer` + Node Core) | `72.054 ms` | `55.473 ms` | `22.199 ms` | **`8.724 ms` (`2.54x` Bun)** | **`0.481 ms` (`46.2x` Bun)** |
| **21** | `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket`) | `116.406 ms` | `52.805 ms` | `20.904 ms` | **`8.921 ms` (`2.34x` Bun)** | **`3.042 ms` (`6.87x` Bun)** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Workers + TCP/UDP + 5-Ring) | `102.414 ms` | `114.823 ms` | `57.714 ms` | **`15.911 ms` (`3.65x` Bun)** | **`2.393 ms` (`24.1x` Bun)** |
| **23** | `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring) | `61.783 ms` | `41.980 ms` | `21.663 ms` | **`12.931 ms` (`1.68x` Bun)** | **`2.122 ms` (`10.2x` Bun)** |
| **24** | `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` + TSX SSR) | `67.616 ms` | `43.289 ms` | `19.055 ms` | **`6.981 ms` (`2.73x` Bun)** | **`1.634 ms` (`11.7x` Bun)** |
| **25** | `FW-01: 01-hono-edge-microservice` (`hono@4.6.20` + `cookie` + `ms`) | `93.408 ms` | `52.293 ms` | `20.227 ms` | **`13.107 ms` (`1.60x` Bun)** | **`6.719 ms` (`3.01x` Bun)** |
| **26** | `FW-02: 02-itty-router-unjs-nitro-stack` (`itty-router` + UnJS) | `86.203 ms` | `46.454 ms` | `19.550 ms` | **`15.521 ms` (`1.26x` Bun)** | **`9.361 ms` (`2.09x` Bun)** |
| **27** | `CS-01: 01-tot-autonomous-sre-agent` (20 ToT Sandboxes + 16 Rollbacks) | `89.907 ms` | `73.275 ms` | `28.510 ms` | **`25.069 ms` (`1.14x` Bun)** | **`14.415 ms` (`1.98x` Bun)** |
| **28** | `CS-02: 02-mcp-code-mode-orchestrator` (`-98.4%` Tokens, 4 Domains) | `66.457 ms` | `43.902 ms` | `16.453 ms` | **`16.875 ms` (`15.85 ms` Min)** | **`8.623 ms` (`1.91x` Bun)** |
| **29** | `CS-03: 03-multi-tenant-saas-plugin-mesh` (12 Tenants + Hot-Swap) | `85.829 ms` | `48.875 ms` | `16.897 ms` | **`11.400 ms` (`1.48x` Bun)** | **`5.625 ms` (`3.00x` Bun)** |
| **30** | `CS-04: 04-wasm-zero-trust-cdr-pipeline` (Wasm SimHash CDR + Vault) | `67.193 ms` | `45.358 ms` | `19.225 ms` | **`13.108 ms` (`1.47x` Bun)** | **`1.907 ms` (`10.1x` Bun)** |
| **31** | `CS-05: 05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | `82.822 ms` | `63.327 ms` | `19.707 ms` | **`18.270 ms` (`1.08x` Bun)** | **`12.275 ms` (`1.61x` Bun)** |

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
* **[Chapter 00: Master Documentation Index](file:///p:/opensource/slice/docs/general/00_index.md)**
* **[Chapter 01: Getting Started, Unified 4-Layer Configuration & Complete CLI Reference](file:///p:/opensource/slice/docs/general/01_getting_started_and_cli.md)**
* **[Chapter 02: Hybrid Three-Tier Execution Engine & Default `SLICEBC\x01` Precompiled CAS (`slice-engine`)](file:///p:/opensource/slice/docs/general/02_execution_engine_and_tiers.md)**
* **[Chapter 03: Zero-Stat CAS Archive, Oxc Bundler & Standalone Binaries (`slice-cas`)](file:///p:/opensource/slice/docs/general/03_cas_bundler_and_standalone_binaries.md)**
* **[Chapter 04: Security Architecture — OCap, SES Lockdown, Fuel Cage & Hardware MPK (`slice-sandbox`)](file:///p:/opensource/slice/docs/general/04_security_ocap_ses_and_hardware_mpk.md)**
* **[Chapter 05: Multi-Tenant HTTP/1.1 Server, WinterTC Bridge & Sharded Edge KV (`slice serve`)](file:///p:/opensource/slice/docs/general/05_http_server_wintertc_and_edge_kv.md)**
* **[Chapter 06: AI Agent "Code Mode" Sandbox, `< 5 us` Checkpoints & Native MCP Server (`slice mcp`)](file:///p:/opensource/slice/docs/general/06_ai_agent_sandbox_checkpoints_and_mcp.md)**
* **[Chapter 07: Node.js Compatibility (`node:*`), Native Intrinsics & External NPM Packages](file:///p:/opensource/slice/docs/general/07_node_compat_intrinsics_and_npm.md)**
* **[Chapter 08: Kernel Completion Rings (`slice-io`) & Embedding Slice in Rust](file:///p:/opensource/slice/docs/general/08_kernel_io_rings_and_rust_embedding.md)**
* **[Chapter 09: Benchmarks, Time-Travel Testing & 31-Workload Docker Verification Suite](file:///p:/opensource/slice/docs/general/09_benchmarks_testing_and_docker_suite.md)**
* **[Runtime Compatibility Matrix (`docs/general/COMPATIBILITY.md`)](file:///p:/opensource/slice/docs/general/COMPATIBILITY.md)**

### Developer How-To Guides (`docs/guides/`)
* **[Developer Guides & Recipes Index (`docs/guides/README.md`)](file:///p:/opensource/slice/docs/guides/README.md)**
* **[Guide 01: Building & Deploying Hono (`v4`) and UnJS Edge Microservices (`docs/guides/01_hono_and_unjs_edge_microservices.md`)](file:///p:/opensource/slice/docs/guides/01_hono_and_unjs_edge_microservices.md)**
* **[Guide 02: TypeScript IDE Setup (`slice.d.ts`), Linting (`oxlint` / `Biome`), & `.wasm` Modules (`docs/guides/02_typescript_ide_linting_and_wasm.md`)](file:///p:/opensource/slice/docs/guides/02_typescript_ide_linting_and_wasm.md)**

### System Engineering Specifications (`docs/system/`, `vendor/`, & `tests/`)
* **[00: System Architecture & Internal Specification Index (`docs/system/00_index.md`)](file:///p:/opensource/slice/docs/system/00_index.md)**
* **[01: `slice-cas` — Zero-Stat CAS, Oxc Compiler & AST Tree-Shaker (`docs/system/01_cas_bundler_and_oxc_compiler.md`)](file:///p:/opensource/slice/docs/system/01_cas_bundler_and_oxc_compiler.md)**
* **[02: `slice-engine` — Hybrid Three-Tier Execution Pipeline, `SLICEBC\x01` & `WarmBoaRealm` (`docs/system/02_tiered_execution_and_boa_vm.md`)](file:///p:/opensource/slice/docs/system/02_tiered_execution_and_boa_vm.md)**
* **[03: `slice-sandbox` — 5-Ring Zero-Trust OCap, Dynamic SES & Hardware MPK (`docs/system/03_security_sandbox_ocap_and_mpk.md`)](file:///p:/opensource/slice/docs/system/03_security_sandbox_ocap_and_mpk.md)**
* **[04: Networking, WinterTC Web Platform & Async Event Loop (`docs/system/04_networking_wintertc_and_event_loop.md`)](file:///p:/opensource/slice/docs/system/04_networking_wintertc_and_event_loop.md)**
* **[05: Pure-Rust WebAssembly, Web Workers & Direct Kernel I/O (`docs/system/05_wasm_workers_and_kernel_io.md`)](file:///p:/opensource/slice/docs/system/05_wasm_workers_and_kernel_io.md)**
* **[06: Serverless Edge KV, AI Agent "Code Mode" & MCP Server (`docs/system/06_edge_kv_ai_sandbox_and_mcp.md`)](file:///p:/opensource/slice/docs/system/06_edge_kv_ai_sandbox_and_mcp.md)**
* **[07: Production Tooling & Deployment Guide (`docs/system/07_production_tooling_guide.md`)](file:///p:/opensource/slice/docs/system/07_production_tooling_guide.md)**
* **[08: Empirical Benchmarks & 31-Workload Comparisons (`docs/system/08_benchmarks_and_comparisons.md`)](file:///p:/opensource/slice/docs/system/08_benchmarks_and_comparisons.md)**
* **[09: Cutting-Edge 2026 Research Deep Dive (`docs/system/09_research_deep_dive.md`)](file:///p:/opensource/slice/docs/system/09_research_deep_dive.md)**
* **[Vendored `boa_engine` `SLICEBC\x01` Patch Specification (`vendor/boa_engine/SLICE_PATCH.md`)](file:///p:/opensource/slice/vendor/boa_engine/SLICE_PATCH.md)**
* **[Latest Full 31-Application Benchmark Report (`bench_latest.md`)](file:///p:/opensource/slice/bench_latest.md)**
* **[Docker 31-Workload Production Benchmarks (`tests/README.md`)](file:///p:/opensource/slice/tests/README.md)**
* **[Framework Integration Benchmarks (`tests/frameworks/README.md`)](file:///p:/opensource/slice/tests/frameworks/README.md)**
* **[Interactive Production Case Studies & Visual Cockpit (`tests/case_studies/README.md`)](file:///p:/opensource/slice/tests/case_studies/README.md)**
