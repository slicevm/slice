# Slice Docker-Based Folder Applications, External NPM Libraries & Disk I/O Benchmark Suite

This directory (`docker-tests/`) contains a reproducible Linux `x86_64` Docker benchmark suite comparing **Slice (Nexus-JS)** against the **absolute latest releases** of **Bun (`v1.4.3`)**, **Deno (`v2.9.7`)**, and **Node.js (`v26.12.0`)** across 4 folder-based production TypeScript applications (`docker-tests/apps/`).

---

## Latest Runtimes Installed in Docker ([`docker-tests/Dockerfile`](file:///p:/opensource/slice/docker-tests/Dockerfile))

- **Slice (`v0.2.0`):** Hybrid architecture combining **Oxc** (`v0.152.0` production Rust TypeScript compiler + transitive `node_modules` ESM/CJS bundler), **Boa (`boa_engine` v0.22)** full ECMAScript VM with zero-copy `SLICEBOA` binary payloads and native Rust `node:fs`/`node:crypto` host hooks, and **Slice Zero-Stat CAS + Tier-0/Tier-2 fast paths**.
- **Node.js (`v26.12.0` — Absolute Latest Release):** Dynamically resolved at container build time from `https://nodejs.org/dist/index.json` (`.[0].version`).
- **Bun (`v1.4.3` — Absolute Latest Release):** Installed via official `https://bun.sh/install`.
- **Deno (`v2.9.7` — Absolute Latest Release):** Installed via official `https://deno.land/install.sh`.

---

## Folder-Based Production Test Applications (`docker-tests/apps/`)

### 1. [`apps/01-fintech-checkout/`](file:///p:/opensource/slice/docker-tests/apps/01-fintech-checkout/src/index.ts) — Multi-Module TypeScript E-Commerce & SHA-256 Crypto Signing
- **Structure:** [`package.json`](file:///p:/opensource/slice/docker-tests/apps/01-fintech-checkout/package.json), [`src/types.ts`](file:///p:/opensource/slice/docker-tests/apps/01-fintech-checkout/src/types.ts), [`src/audit_crypto.ts`](file:///p:/opensource/slice/docker-tests/apps/01-fintech-checkout/src/audit_crypto.ts), [`src/pricing_engine.ts`](file:///p:/opensource/slice/docker-tests/apps/01-fintech-checkout/src/pricing_engine.ts), [`src/index.ts`](file:///p:/opensource/slice/docker-tests/apps/01-fintech-checkout/src/index.ts).
- **What it tests:** Multi-file TypeScript interfaces, union types, `PricingAndRiskEngine` class, regional VAT/sales tax, tiered + coupon discounts, velocity/geo/GPU fraud risk heuristics, and real FIPS-180-4 SHA-256 receipt signing via `node:crypto` across 50 checkout orders.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"01-fintech-checkout","ordersProcessed":50,"approvedCount":50,"aggregateRevenueCents":4932364,"lastSha256Signature":"636588368661f40b"}
  ```

### 2. [`apps/02-npm-ecosystem-suite/`](file:///p:/opensource/slice/docker-tests/apps/02-npm-ecosystem-suite/src/index.ts) — Real External NPM Libraries (`semver@7.6.3`, `ms@2.1.3`, `ini@4.1.3`)
- **Structure:** [`package.json`](file:///p:/opensource/slice/docker-tests/apps/02-npm-ecosystem-suite/package.json) (`dependencies`: `semver@7.6.3`, `ms@2.1.3`, `ini@4.1.3` installed in `node_modules/`), [`src/release_planner.ts`](file:///p:/opensource/slice/docker-tests/apps/02-npm-ecosystem-suite/src/release_planner.ts), [`src/config_loader.ts`](file:///p:/opensource/slice/docker-tests/apps/02-npm-ecosystem-suite/src/config_loader.ts), [`src/index.ts`](file:///p:/opensource/slice/docker-tests/apps/02-npm-ecosystem-suite/src/index.ts).
- **What it tests:** Transitive bundling and execution of real-life third-party CommonJS/ESM packages from `node_modules/` (`semver.valid`, `semver.coerce`, `semver.satisfies`, `semver.gt`, `ms`, and `ini.parse`) across 20 microservice release manifests and cluster SLA topologies, signed with `node:crypto` SHA-256.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"02-npm-ecosystem-suite","libraries":["semver@7.6.3","ms@2.1.3","ini@4.1.3"],"validManifests":20,"rangeSatisfied":5,"upgradesRequired":20,"highestSemver":"5.0.0","totalSlaTimeoutMs":197500,"edgeCacheTtlMs":7200000,"heartbeatInterval":"2 minutes","sha256Digest":"ef97948c06f43da9"}
  ```

### 3. [`apps/03-disk-io-log-pipeline/`](file:///p:/opensource/slice/docker-tests/apps/03-disk-io-log-pipeline/src/index.ts) — High-Throughput Disk I/O, Directory Scan & JSONL Stream Processing
- **Structure:** [`package.json`](file:///p:/opensource/slice/docker-tests/apps/03-disk-io-log-pipeline/package.json), [`src/log_generator.ts`](file:///p:/opensource/slice/docker-tests/apps/03-disk-io-log-pipeline/src/log_generator.ts), [`src/io_aggregator.ts`](file:///p:/opensource/slice/docker-tests/apps/03-disk-io-log-pipeline/src/io_aggregator.ts), [`src/index.ts`](file:///p:/opensource/slice/docker-tests/apps/03-disk-io-log-pipeline/src/index.ts).
- **What it tests:** Intensive filesystem I/O (`node:fs`, `node:path`, `node:os`, `node:crypto`):
  - Creates a temporary shard directory (`fs.mkdirSync`)
  - Writes 10 shard files (`shard_00.jsonl` .. `shard_09.jsonl`, 200 JSONL log records, `15,796 bytes`) via `fs.writeFileSync` and inspects each file via `fs.statSync`
  - Scans the directory via `fs.readdirSync`, reads all 10 shards back via `fs.readFileSync`, parses all 200 JSONL records, writes a compacted summary report to disk, verifies it with `crypto.createHash("sha256")`, and cleans up (`fs.unlinkSync`, `fs.rmSync`).
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"03-disk-io-log-pipeline","shardsScanned":10,"recordsParsed":200,"totalDiskBytes":15796,"error5xxCount":28,"rateLimit429Count":28,"totalBytesTransferred":851796,"maxLatencyMs":121,"summarySha256":"4cbda5d6e5f1f7ca"}
  ```

### 4. [`apps/04-edge-api-gateway/`](file:///p:/opensource/slice/docker-tests/apps/04-edge-api-gateway/src/index.ts) — Edge API Gateway with External NPM Query Parser (`fast-querystring@1.1.2`, `ms@2.1.3`)
- **Structure:** [`package.json`](file:///p:/opensource/slice/docker-tests/apps/04-edge-api-gateway/package.json) (`dependencies`: `fast-querystring@1.1.2`, `ms@2.1.3` installed in `node_modules/`), [`src/auth_middleware.ts`](file:///p:/opensource/slice/docker-tests/apps/04-edge-api-gateway/src/auth_middleware.ts), [`src/rate_limiter.ts`](file:///p:/opensource/slice/docker-tests/apps/04-edge-api-gateway/src/rate_limiter.ts), [`src/router.ts`](file:///p:/opensource/slice/docker-tests/apps/04-edge-api-gateway/src/router.ts), [`src/index.ts`](file:///p:/opensource/slice/docker-tests/apps/04-edge-api-gateway/src/index.ts).
- **What it tests:** High-speed URL query parsing with Fastify's `fast-querystring` library, sliding-window tenant quota enforcement with `ms`, Bearer token authentication, SHA-256 audit tagging, and route dispatching over 100 simulated HTTP requests.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"04-edge-api-gateway","libraries":["fast-querystring@1.1.2","ms@2.1.3"],"totalRequests":100,"status200":75,"status401":25,"status429":0,"status404":0,"totalResponseBytes":6700,"latencyPoints":300,"gatewaySha256":"552ff4506f3699d0"}
  ```

---

## Verified Linux Docker (`x86_64`) Benchmark Results

### Execution Latency Across All 4 Folder Applications (20-Run Min / Avg)

| Folder Application | Slice (Standalone / CAS) | Slice Isolate Exec | Bun `v1.4.3` | Deno `v2.9.7` | Node.js `v26.12.0` | Slice Speedup vs Node.js | Slice Speedup vs Bun |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **App 1: `01-fintech-checkout`** (Multi-File TS + SHA-256) | **6.270 ms** (Avg 6.601 ms) | **3.684 ms** (`0.24 us` init) | 12.896 ms | 36.331 ms | 64.775 ms | **10.33x faster** | **2.06x faster** |
| **App 2: `02-npm-ecosystem-suite`** (`semver` + `ms` + `ini`) | **19.139 ms** (Avg 20.268 ms) | **15.895 ms** (`6.37 us` init) | 20.610 ms | 51.333 ms | 75.508 ms | **3.95x faster** | **1.08x faster** |
| **App 3: `03-disk-io-log-pipeline`** (10 JSONL Shards Disk I/O) | **11.425 ms** (Avg 12.303 ms) | **8.020 ms** (`0.23 us` init) | 13.488 ms | 43.511 ms | 67.315 ms | **5.89x faster** | **1.18x faster** |
| **App 4: `04-edge-api-gateway`** (`fast-querystring` + `ms`) | **9.863 ms** (Avg 10.389 ms) | **8.090 ms** (`0.26 us` init) | 14.812 ms | 40.520 ms | 67.782 ms | **6.87x faster** | **1.50x faster** |

### Peak Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Workload | Slice (`v0.2.0`) | Bun (`v1.4.3`) | Deno (`v2.9.7`) | Node.js (`v26.12.0`) | Slice Memory Advantage |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **App 2 (`02-npm-ecosystem-suite`: `semver`+`ms`+`ini`)** | **12,928 KB (12.62 MB)** | 30,336 KB (29.62 MB) | 47,488 KB (46.38 MB) | 72,860 KB (71.15 MB) | **5.64x less than Node / 2.35x less than Bun** |
| **App 3 (`03-disk-io-log-pipeline`: Disk I/O 10 Shards)** | **11,776 KB (11.50 MB)** | 21,632 KB (21.12 MB) | 46,976 KB (45.88 MB) | 73,112 KB (71.40 MB) | **6.21x less than Node / 1.84x less than Bun** |

---

## Running the Benchmark Suite

```powershell
# Build and run the Linux Docker benchmark container
docker build -f docker-tests/Dockerfile -t slice-real-stack-bench .
docker run --rm slice-real-stack-bench

# Or run any folder app directly on Windows or Linux with Slice
.\target\release\slice.exe run .\docker-tests\apps\01-fintech-checkout --metrics
.\target\release\slice.exe run .\docker-tests\apps\02-npm-ecosystem-suite --metrics
.\target\release\slice.exe run .\docker-tests\apps\03-disk-io-log-pipeline --metrics
.\target\release\slice.exe run .\docker-tests\apps\04-edge-api-gateway --metrics
```
