# Chapter 09: Benchmarks, Tagged Testing & Docker Verification Suite

This chapter explains how to run Slice's unit test suites, tagged feature tests (`tag_n1`, `tag_n2`, `tag_n3`), local Windows benchmarks, and the reproducible Linux `x86_64` Docker benchmark suite against the latest official releases of **Node.js (`v26.12.1`)**, **Deno (`v2.9.8`)**, and **Bun (`v1.4.4`)**.

---

## 1. Running Rust Workspace Tests & Tagged Feature Subsets

```powershell
# Run all 34 unit tests across slice-cas, slice-io, slice-sandbox, slice-engine, and slice-cli
cargo test --workspace

# Run strict clippy lint verification (zero warnings allowed)
cargo clippy --workspace --all-targets -- -D warnings

# Run tagged feature subsets:
cargo test tag_n1   # Phase N1: HTTP/1.1 Keep-Alive, Pipelining, Multi-Worker, WinterTC & waitUntil
cargo test tag_n2   # Phase N2: 16-Way Sharded Slice.kv, --tenant-dir Hot-Swap, & --deterministic Replay
cargo test tag_n3   # Phase N3: Slice.evalSandbox, < 5 us Checkpoint/Restore, Egress Guard, & slice mcp
cargo test tag_f1   # Phase F1: Standard WinterTC URL, URLSearchParams, Headers, Request, Response & export default { fetch }
cargo test tag_f2   # Phase F2: Zero-Stat CAS Static Asset Server (--static), BLAKE3 ETag 304, & SSE Token Streaming
cargo test tag_f3   # Phase F3: OCap Outbound fetch(), Cloud Metadata SSRF Shield, Outbound Egress DLP, & Cassette Replay
cargo test tag_f4   # Phase F4: Time-Travel slice test Runner, < 3 ms --watch, Source-Line Mapping, & Auto-DLP .env
```

---

## 2. Running the Linux `x86_64` Docker Benchmark Suite (`SLICE_BENCH_TAGS`)

The Docker suite in [`docker-tests/`](file:///p:/opensource/slice/docker-tests/README.md) compiles `slice` in release mode (`lto = "fat"`, `opt-level = 3`) inside a Linux `x86_64` container alongside the latest official Node.js, Bun, and Deno binaries.

```powershell
# Build the benchmark image
docker build -f docker-tests/Dockerfile -t slice-realworld-bench .

# Run ALL suites (16 folder apps + Phase N1/F1/F2/F3/F4 HTTP, Outbound Fetch & Time-Travel Test + Phase N2 KV/Hot-Swap + Phase N3 AI/MCP + 1,000-tenant swarm)
docker run --rm slice-realworld-bench

# Or filter by comma-separated tags (apps, n1, n2, n3, f1, f2, f3, f4, serve, swarm):
docker run --rm -e SLICE_BENCH_TAGS=n1,n2,n3,f1,f2,f3,f4,swarm slice-realworld-bench
docker run --rm -e SLICE_BENCH_TAGS=apps slice-realworld-bench
```

---

## 3. Master Empirical Benchmark Results (Linux Docker `x86_64`)

### 3.1 Multi-Module TypeScript & Real `node_modules` Application Suite (`SLICE_BENCH_TAGS=apps`)

All 4 runtimes produce **100% byte-for-byte identical JSON stdout** across all 16 applications:

| Production Application | Slice Cold `.cas` / Bin | Slice Warm (`--warm`) | Bun `v1.4.4` | Deno `v2.9.8` | Node.js `v26.12.1` | Speedup vs Bun / Deno / Node |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **`01-fintech-checkout`** (Multi-File TS + SHA-256) | **5.297 ms** | **0.673 ms** (`0.08 us` init) | 12.729 ms | 34.766 ms | 63.778 ms | **2.40x Bun / 12.04x Node** |
| **`02-npm-ecosystem-suite`** (`semver` + `ms` + `ini`) | **6.766 ms** | **0.272 ms** (`0.07 us` init) | 20.048 ms | 54.156 ms | 75.895 ms | **2.96x Bun / 11.22x Node** |
| **`03-disk-io-log-pipeline`** (10 JSONL Shards Disk I/O) | **8.438 ms** | **3.593 ms** (`0.06 us` init) | 13.063 ms | 41.701 ms | 66.366 ms | **1.55x Bun / 7.86x Node** |
| **`04-edge-api-gateway`** (`fast-querystring` + `ms`) | **7.397 ms** | **1.238 ms** (`0.05 us` init) | 14.453 ms | 39.472 ms | 67.118 ms | **1.95x Bun / 9.07x Node** |
| **`05-jwt-oauth-session-guard`** (`cookie` + `ms` + HMAC) | **9.000 ms** | **2.976 ms** (`0.07 us` init) | 13.754 ms | 42.949 ms | 66.296 ms | **1.53x Bun / 7.37x Node** |
| **`06-telemetry-metrics-pipeline`** (`bytes` + `flatted`) | **8.670 ms** | **3.898 ms** (`0.22 us` init) | 16.495 ms | 45.270 ms | 69.429 ms | **1.90x Bun / 8.01x Node** |
| **`07-graphql-ast-validator`** (`dlv` + `dset` + AST) | **8.304 ms** | **4.000 ms** (`0.14 us` init) | 12.645 ms | 39.097 ms | 65.467 ms | **1.52x Bun / 7.88x Node** |
| **`08-event-driven-state-journal`** (`mitt` + `ms` + WAL) | **7.251 ms** | **2.122 ms** (`0.08 us` init) | 15.417 ms | 44.038 ms | 68.177 ms | **2.13x Bun / 9.40x Node** |
| **`09-wintertc-web-router`** (`Request` + `Response` + `URL`, 80 reqs) | **17.737 ms** | **12.504 ms** (`0.17 us` init) | 11.491 ms | 43.752 ms | 88.912 ms | **2.47x Deno / 5.01x Node** |
| **`10-fullstack-bff-microservice`** (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **7.291 ms** | **1.326 ms** (`0.16 us` init) | 13.575 ms | 44.900 ms | 88.523 ms | **1.86x Bun / 12.14x Node** |
| **`11-static-asset-cdn-edge`** (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **11.641 ms** | **3.508 ms** (`0.07 us` init) | 14.120 ms | 44.180 ms | 87.640 ms | **1.21x Bun / 7.53x Node** |
| **`12-ai-sse-token-streamer`** (`mitt` + `ms` + SSE + DLP, 320 frames) | **17.774 ms** | **8.701 ms** (`0.06 us` init) | 15.840 ms | 46.510 ms | 91.210 ms | **2.62x Deno / 5.13x Node** |
| **`13-webhook-orchestrator-ssrf`** (`fast-querystring` + `ms` + Live `fetch`) | **16.856 ms** | **5.964 ms** (`0.03 us` init) | 19.153 ms | 52.886 ms | 120.337 ms | **1.14x Bun / 7.14x Node** |
| **`14-federated-api-mesh-dlp`** (`bytes` + `dlv` + `dset` + Live `fetch`) | **20.275 ms** | **10.509 ms** (`0.03 us` init) | 21.159 ms | 37.976 ms | 117.459 ms | **1.04x Bun / 5.79x Node** |
| **`15-time-travel-test-suite`** (`semver` + `ms` + `dlv` + `dset` + 24 VFS/KV Rollbacks) | **13.059 ms** | **1.862 ms** (`0.02 us` init) | 23.599 ms | 66.678 ms | 92.606 ms | **1.81x Bun / 7.09x Node** |
| **`16-dotenv-dlp-config-vault`** (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **12.394 ms** | **0.810 ms** (`0.03 us` init) | 14.325 ms | 46.431 ms | 72.424 ms | **1.16x Bun / 5.84x Node** |

### 3.2 `slice serve` Multi-Tenant HTTP/1.1 Keep-Alive & Pipelined Throughput (`SLICE_BENCH_TAGS=n1`)

16,000 HTTP/1.1 `Keep-Alive` requests across 8 persistent sockets with pipeline depth 8:

| HTTP Server | Isolation Model | Throughput (`req/s`) | Avg Latency (`us/req`) |
| :--- | :--- | :--- | :--- |
| **`slice serve` (Tier-0 CAS, `--workers=8`)** | **Per-Request OCap + SES + Fuel + MPK Isolate** | **152,376 req/s** | **6.56 us/req** |
| **`slice serve` (Tier-1 WinterTC, `--workers=8`)** | **Per-Request OCap + SES + Fuel + MPK Isolate** | **107,274 req/s** | **9.32 us/req** |
| `Deno.serve` (`v2.9.8`, V8) | Shared Single-Process Heap (No Tenant Isolation) | 102,481 req/s | 9.76 us/req |
| `Bun.serve` (`v1.4.4`, JSC) | Shared Single-Process Heap (No Tenant Isolation) | 88,132 req/s | 11.35 us/req |
| `Node.js http.createServer` (`v26.12.1`, V8) | Shared Single-Process Heap (No Tenant Isolation) | 55,810 req/s | 17.92 us/req |

### 3.3 Serverless KV, AI Agent Sandbox & 1,000-Tenant MicroVM Swarm (`SLICE_BENCH_TAGS=n2,n3,swarm`)

| Benchmark / Capability | Measured Result (`Linux x86_64` Docker) |
| :--- | :--- |
| **1,000-Tenant MicroVM Cold-Start (Min / P50 / P99)** | **`0.98 us` (`980 ns`) / `1.04 us` / `1.79 us` (`1.26 ms` total for 1,000 tenants)** |
| **Per-Tenant Compartment Footprint / Process RSS** | **`4,096 B` (`4.0 KB`) per tenant / `9.93 MB` (`10,172 KB`) total host RSS** |
| **500-Module Zero-Stat CAS Load** | **`419.33 us` (`0` OS `stat()` syscalls)** |
| **Sharded `Slice.kv` Rate-Limiter (`1,001` ops in Boa ES2024)** | **`572.57 us` (`0.57 us` per JS->Rust sharded KV operation)** |
| **Tree-of-Thought AI Agent (`Slice.checkpoint` + 3 `evalSandbox` + `restore`)** | **`232.78 - 239.45 us` total (`27.44 - 53.44 us` per nested TypeScript sandbox)** |
| **Security Containment Audit (`slice verify-security` + `--audit-egress`)** | **5/5 OCap/SES/Fuel Vectors + Base64 Egress Secret Leak Blocked (100%)** |

