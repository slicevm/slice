# Chapter 09: Benchmarks, Tagged Testing & Docker Verification Suite

This chapter explains how to run Slice's unit test suites, tagged feature tests (`tag_n1`, `tag_n2`, `tag_n3`), local Windows benchmarks, and the reproducible Linux `x86_64` Docker benchmark suite against the latest official releases of **Node.js (`v26.12.1`)**, **Deno (`v2.9.8`)**, and **Bun (`v1.4.4`)**.

---

## 1. Running Rust Workspace Tests & Tagged Feature Subsets

```powershell
# Run all 23 unit tests across slice-cas, slice-io, slice-sandbox, slice-engine, and slice-cli
cargo test --workspace

# Run strict clippy lint verification (zero warnings allowed)
cargo clippy --workspace --all-targets -- -D warnings

# Run tagged feature subsets:
cargo test tag_n1   # Phase N1: HTTP/1.1 Keep-Alive, Pipelining, Multi-Worker, WinterTC & waitUntil
cargo test tag_n2   # Phase N2: 16-Way Sharded Slice.kv, --tenant-dir Hot-Swap, & --deterministic Replay
cargo test tag_n3   # Phase N3: Slice.evalSandbox, < 5 us Checkpoint/Restore, Egress Guard, & slice mcp
```

---

## 2. Running the Linux `x86_64` Docker Benchmark Suite (`SLICE_BENCH_TAGS`)

The Docker suite in [`docker-tests/`](file:///p:/opensource/slice/docker-tests/README.md) compiles `slice` in release mode (`lto = "fat"`, `opt-level = 3`) inside a Linux `x86_64` container alongside the latest official Node.js, Bun, and Deno binaries.

```powershell
# Build the benchmark image
docker build -f docker-tests/Dockerfile -t slice-realworld-bench .

# Run ALL suites (8 folder apps + Phase N1 HTTP server + Phase N2 KV/Hot-Swap + Phase N3 AI/MCP + 1,000-tenant swarm)
docker run --rm slice-realworld-bench

# Or filter by comma-separated tags (apps, n1, n2, n3, serve, swarm):
docker run --rm -e SLICE_BENCH_TAGS=n1,n2,n3,swarm slice-realworld-bench
docker run --rm -e SLICE_BENCH_TAGS=apps slice-realworld-bench
```

---

## 3. Master Empirical Benchmark Results (Linux Docker `x86_64`)

### 3.1 Multi-Module TypeScript & Real `node_modules` Application Suite (`SLICE_BENCH_TAGS=apps`)

All 4 runtimes produce **100% byte-for-byte identical JSON stdout** across all 8 applications:

| Production Application | Slice Cold `.cas` / Bin | Slice Warm (`--warm`) | Bun `v1.4.4` | Deno `v2.9.8` | Node.js `v26.12.1` | Speedup vs Bun (Cold / Warm) |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **`01-fintech-checkout`** (Multi-File TS + SHA-256) | **5.297 ms** | **0.673 ms** (`0.08 us` init) | 12.729 ms | 34.766 ms | 63.778 ms | **2.40x / 18.90x faster** |
| **`02-npm-ecosystem-suite`** (`semver` + `ms` + `ini`) | **6.766 ms** | **0.272 ms** (`0.07 us` init) | 20.048 ms | 54.156 ms | 75.895 ms | **2.96x / 73.70x faster** |
| **`03-disk-io-log-pipeline`** (10 JSONL Shards Disk I/O) | **8.438 ms** | **3.593 ms** (`0.06 us` init) | 13.063 ms | 41.701 ms | 66.366 ms | **1.55x / 3.63x faster** |
| **`04-edge-api-gateway`** (`fast-querystring` + `ms`) | **7.397 ms** | **1.238 ms** (`0.05 us` init) | 14.453 ms | 39.472 ms | 67.118 ms | **1.95x / 11.68x faster** |
| **`05-jwt-oauth-session-guard`** (`cookie` + `ms` + HMAC) | **9.000 ms** | **2.976 ms** (`0.07 us` init) | 13.754 ms | 42.949 ms | 66.296 ms | **1.53x / 4.62x faster** |
| **`06-telemetry-metrics-pipeline`** (`bytes` + `flatted`) | **8.670 ms** | **3.898 ms** (`0.22 us` init) | 16.495 ms | 45.270 ms | 69.429 ms | **1.90x / 4.23x faster** |
| **`07-graphql-ast-validator`** (`dlv` + `dset` + AST) | **8.304 ms** | **4.000 ms** (`0.14 us` init) | 12.645 ms | 39.097 ms | 65.467 ms | **1.52x / 3.16x faster** |
| **`08-event-driven-state-journal`** (`mitt` + `ms` + WAL) | **7.251 ms** | **2.122 ms** (`0.08 us` init) | 15.417 ms | 44.038 ms | 68.177 ms | **2.13x / 7.27x faster** |

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

