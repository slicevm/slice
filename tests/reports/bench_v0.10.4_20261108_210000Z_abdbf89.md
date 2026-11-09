# Slice (Nexus-JS) — Full 32-Workload & MicroVM Benchmark Report

* **Slice Version:** `v0.10.4` (Commit `abdbf89`)
* **Generated Timestamp (UTC):** `2026-11-08T21:00:00Z`
* **Archive File:** [`tests/reports/bench_v0.10.4_20261108_210000Z_abdbf89.md`](./bench_v0.10.4_20261108_210000Z_abdbf89.md)
* **Canonical Latest Report:** [`tests/reports/latest.md`](./latest.md)
* **Environment:** `Linux x86_64 Docker (io_uring SQPOLL + WRPKRU)`
* **Compared Runtimes:**
  * **Slice:** `v0.10.4` (Default `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS + Monomorphic Inline Caching `get_fast` + Streaming `JsonSeed` `JSON.parse`)
  * **Bun:** `v1.4.4` (JavaScriptCore)
  * **Deno:** `v2.9.8` (V8)
  * **Node.js:** `v27.0.0-alpha` (V8 native TypeScript execution)

---

## 1. Executive Summary

Across all **32 real-world multi-file TypeScript applications, framework suites, and case studies**, Slice beats **Deno (`v2.9.8`)** on **32/32** workloads, beats **Node.js (`v27.0.0-alpha`)** on **32/32** workloads, beats **Bun (`v1.4.4`)** on **29/32** workloads in cold one-shot execution, and beats **all 3 runtimes on 32/32 workloads in `--warm` pooled isolate execution**.

| Metric | Linux (`x86_64` Docker) | Windows (`x86_64` Host) | Comparison vs Runtimes & MicroVMs |
| :--- | :---: | :---: | :--- |
| **Single Isolate Cold-Start (Min / P50 / P99)** | **`0.98 us` / `1.03 us` / `2.42 us`** | **`1.40 us` / `1.60 us` / `2.30 us`** | **~120,000x faster than 125ms Firecracker VM** |
| **1,000-Tenant MicroVM Swarm (Boot + Exec)** | **`1.28 ms`** (`10.06 MB` RSS) | **`2.24 ms`** (`11.82 MB` RSS) | **> 10,000x faster than container/VM swarms** |
| **500-Module Monorepo Cold Boot (`0 stat()`)** | **`3.408 ms`** (`619.39 us` in-mem) | **`5.637 ms`** (`640.00 us` in-mem) | **20.6x faster than Node.js (`1,500+ stat()`)** |
| **10,000 Direct Kernel Ring I/O Completions** | **`0.02 ms`** | **`5.11 ms`** | **Zero worker-thread lock contention** |
| **5-Ring Security Containment Verification** | **`5 / 5 PASS`** | **`5 / 5 PASS`** | **OCap + SES + Fuel + SSRF/DLP + MPK** |

---

## 2. `slice serve` Multi-Tenant HTTP/1.1 Keep-Alive Throughput (`16,000` Reqs, 8 Sockets x 8 Pipeline)

| Server / Runtime | Throughput (`req/s`) | Avg Latency / Req | Total Time (`16k` Reqs) | Relative Speedup vs Runtimes |
| :--- | :---: | :---: | :---: | :--- |
| **`slice serve` (Tier-0 CAS, `--workers=8`)** | **`158,534 req/s`** | **`6.31 us/req`** | **`100.9 ms`** | **`1.74x` faster than `Bun.serve` / `3.51x` Node.js** |
| **`slice serve` (Tier-1 WinterTC, `--workers=8`)** | **`120,333 req/s`** | **`8.31 us/req`** | **`133.0 ms`** | **`1.32x` faster than `Bun.serve` / `2.67x` Node.js** |
| **`Deno.serve` (`v2.9.8`, V8, No Per-Req Sandbox)** | `98,377 req/s` | `10.16 us/req` | `162.6 ms` | `1.08x` vs Bun |
| **`Bun.serve` (`v1.4.4`, JSC, No Per-Req Sandbox)** | `91,234 req/s` | `10.96 us/req` | `175.4 ms` | `1.00x` (Baseline) |
| **`Node.js http.createServer` (`v27.0.0-alpha`, V8)** | `45,123 req/s` | `22.16 us/req` | `354.6 ms` | `0.49x` vs Bun |

---

## 3. Full 32-Workload Latency Comparison (20 Runs Each, Linux `x86_64` Docker)

| # | Workload (`tests/apps/`, `tests/frameworks/`, `tests/case_studies/`) | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm (`--warm`) | Previous `.cas` (`Delta`) | Bun `v1.4.4` | Deno `v2.9.8` | Node.js `v27.0.0-alpha` | Slice Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`6.462 ms`** (`5.81 ms` Min) | **`6.915 ms`** | **`0.970 ms`** | `7.118 ms` (`-9.2%`) | `14.357 ms` | `40.515 ms` | `64.388 ms` | **`2.22x Bun` (`14.80x` Warm) / `9.96x Node`** |
| **02** | `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | **`14.045 ms`** (`13.14 ms` Min) | **`14.795 ms`** | **`4.101 ms`** | `13.553 ms` (`3.6%`) | `21.324 ms` | `52.462 ms` | `70.900 ms` | **`1.52x Bun` (`5.20x` Warm) / `5.05x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`7.891 ms`** (`7.32 ms` Min) | **`8.480 ms`** | **`2.044 ms`** | `9.811 ms` (`-19.6%`) | `14.361 ms` | `42.519 ms` | `63.433 ms` | **`1.82x Bun` (`7.03x` Warm) / `8.04x Node`** |
| **04** | `04-edge-api-gateway` (`fast-querystring` + `ms`) | **`7.793 ms`** (`7.36 ms` Min) | **`8.387 ms`** | **`1.949 ms`** | `8.301 ms` (`-6.1%`) | `14.829 ms` | `41.315 ms` | `63.293 ms` | **`1.90x Bun` (`7.61x` Warm) / `8.12x Node`** |
| **05** | `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | **`10.875 ms`** (`10.36 ms` Min) | **`11.395 ms`** | **`5.157 ms`** | `12.164 ms` (`-10.6%`) | `15.178 ms` | `49.432 ms` | `65.323 ms` | **`1.40x Bun` (`2.94x` Warm) / `6.01x Node`** |
| **06** | `06-telemetry-metrics-pipeline` (`bytes` + `flatted` + WAL) | **`14.513 ms`** (`13.80 ms` Min) | **`15.101 ms`** | **`5.815 ms`** | `19.654 ms` (`-26.2%`) | `16.831 ms` | `45.490 ms` | `65.159 ms` | **`1.16x Bun` (`2.89x` Warm) / `4.49x Node`** |
| **07** | `07-graphql-ast-validator` (`dlv` + `dset` + AST Cost) | **`9.282 ms`** (`8.86 ms` Min) | **`9.871 ms`** | **`4.160 ms`** | `9.869 ms` (`-5.9%`) | `13.126 ms` | `46.870 ms` | `61.297 ms` | **`1.41x Bun` (`3.16x` Warm) / `6.60x Node`** |
| **08** | `08-event-driven-state-journal` (`mitt` + `ms` + JSONL Replay) | **`11.336 ms`** (`10.72 ms` Min) | **`12.380 ms`** | **`5.362 ms`** | `13.575 ms` (`-16.5%`) | `16.258 ms` | `46.551 ms` | `62.180 ms` | **`1.43x Bun` (`3.03x` Warm) / `5.49x Node`** |
| **09** | `09-wintertc-web-router` (`Request` + `Response` + `URL`, 80 reqs) | **`15.435 ms`** (`14.09 ms` Min) | **`15.418 ms`** | **`9.402 ms`** | `18.674 ms` (`-17.3%`) | `13.766 ms` | `47.303 ms` | `84.676 ms` | **`1.46x Bun Warm` / `5.49x Node`** |
| **10** | `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **`12.988 ms`** (`12.16 ms` Min) | **`13.701 ms`** | **`6.387 ms`** | `15.654 ms` (`-17.0%`) | `14.099 ms` | `46.932 ms` | `83.650 ms` | **`1.09x Bun` (`2.21x` Warm) / `6.44x Node`** |
| **11** | `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **`9.831 ms`** (`9.21 ms` Min) | **`10.431 ms`** | **`3.575 ms`** | `10.818 ms` (`-9.1%`) | `13.202 ms` | `52.120 ms` | `86.887 ms` | **`1.34x Bun` (`3.69x` Warm) / `8.84x Node`** |
| **12** | `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP, 320 frames) | **`16.268 ms`** (`15.38 ms` Min) | **`17.053 ms`** | **`8.443 ms`** | `17.560 ms` (`-7.4%`) | `15.734 ms` | `49.942 ms` | `89.397 ms` | **`1.86x Bun Warm` / `5.50x Node`** |
| **13** | `13-webhook-orchestrator-ssrf` (`fast-querystring` + `ms` + Live `fetch`) | **`13.596 ms`** (`12.97 ms` Min) | **`14.157 ms`** | **`6.651 ms`** | `15.462 ms` (`-12.1%`) | `19.532 ms` | `51.076 ms` | `111.596 ms` | **`1.44x Bun` (`2.94x` Warm) / `8.21x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`14.463 ms`** (`13.86 ms` Min) | **`15.599 ms`** | **`8.065 ms`** | `18.234 ms` (`-20.7%`) | `21.703 ms` | `52.931 ms` | `109.095 ms` | **`1.50x Bun` (`2.69x` Warm) / `7.54x Node`** |
| **15** | `15-time-travel-test-suite` (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **`14.976 ms`** (`14.43 ms` Min) | **`15.350 ms`** | **`5.118 ms`** | `14.151 ms` (`5.8%`) | `24.472 ms` | `65.626 ms` | `82.490 ms` | **`1.63x Bun` (`4.78x` Warm) / `5.51x Node`** |
| **16** | `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **`7.580 ms`** (`7.13 ms` Min) | **`8.043 ms`** | **`1.664 ms`** | `7.647 ms` (`-0.9%`) | `14.572 ms` | `43.637 ms` | `65.300 ms` | **`1.92x Bun` (`8.76x` Warm) / `8.61x Node`** |
| **17** | `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive/307/SSRF/DLP) | **`15.683 ms`** (`14.99 ms` Min) | **`16.255 ms`** | **`8.442 ms`** | `18.550 ms` (`-15.5%`) | `21.812 ms` | `54.938 ms` | `110.915 ms` | **`1.39x Bun` (`2.58x` Warm) / `7.07x Node`** |
| **18** | `18-async-event-loop-fanout-scheduler` (Timer Wheel/`Promise.all`/`AbortSignal`) | **`29.056 ms`** (`27.75 ms` Min) | **`29.790 ms`** | **`17.136 ms`** | `30.892 ms` (`-5.9%`) | `32.800 ms` | `70.636 ms` | `81.020 ms` | **`1.13x Bun` (`1.91x` Warm) / `2.79x Node`** |
| **19** | `19-wintertc-webcrypto-streams-edge` (WebCrypto/Streams/FormData/Clone) | **`8.116 ms`** (`7.44 ms` Min) | **`8.786 ms`** | **`1.722 ms`** | `7.772 ms` (`4.4%`) | `13.853 ms` | `45.039 ms` | `85.490 ms` | **`1.71x Bun` (`8.04x` Warm) / `10.53x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`10.578 ms`** (`10.13 ms` Min) | **`11.315 ms`** | **`0.606 ms`** | `10.752 ms` (`-1.6%`) | `22.162 ms` | `54.876 ms` | `71.146 ms` | **`2.10x Bun` (`36.57x` Warm) / `6.73x Node`** |
| **21** | `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket` + 5-Ring Guard) | **`10.727 ms`** (`10.07 ms` Min) | **`10.911 ms`** | **`2.504 ms`** | `10.935 ms` (`-1.9%`) | `19.980 ms` | `51.863 ms` | `114.930 ms` | **`1.86x Bun` (`7.98x` Warm) / `10.71x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`19.960 ms`** (`19.31 ms` Min) | **`20.390 ms`** | **`2.160 ms`** | `20.495 ms` (`-2.6%`) | `54.184 ms` | `110.814 ms` | `99.212 ms` | **`2.71x Bun` (`25.09x` Warm) / `4.97x Node`** |
| **23** | `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **`16.511 ms`** (`15.77 ms` Min) | **`16.966 ms`** | **`1.449 ms`** | `16.299 ms` (`1.3%`) | `20.779 ms` | `40.072 ms` | `59.115 ms` | **`1.26x Bun` (`14.34x` Warm) / `3.58x Node`** |
| **24** | `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` SLA Scorer + SSR Pipeline) | **`8.646 ms`** (`7.79 ms` Min) | **`8.948 ms`** | **`1.747 ms`** | `8.534 ms` (`1.3%`) | `18.867 ms` | `42.100 ms` | `65.406 ms` | **`2.18x Bun` (`10.80x` Warm) / `7.56x Node`** |
| **FW-01** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`14.577 ms`** (`13.82 ms` Min) | **`15.145 ms`** | **`5.652 ms`** | `15.963 ms` (`-8.7%`) | `19.203 ms` | `51.451 ms` | `93.418 ms` | **`1.32x Bun` (`3.40x` Warm) / `6.41x Node`** |
| **FW-02** | `FW-02: frameworks/02-itty-router-unjs-nitro-stack` (`itty-router` + UnJS) | **`17.079 ms`** (`16.30 ms` Min) | **`17.628 ms`** | **`9.078 ms`** | `17.697 ms` (`-3.5%`) | `19.205 ms` | `46.065 ms` | `89.889 ms` | **`1.12x Bun` (`2.12x` Warm) / `5.26x Node`** |
| **FW-03** | `FW-03: frameworks/03-node-http-express-compat` (`node:http` + `node:stream` Express/Polka) | **`9.120 ms`** (`8.42 ms` Min) | **`9.640 ms`** | **`1.810 ms`** | `9.450 ms` (`-3.5%`) | `15.840 ms` | `44.120 ms` | `68.450 ms` | **`1.74x Bun` (`8.75x` Warm) / `7.51x Node`** |
| **CS-01** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`26.682 ms`** (`25.20 ms` Min) | **`27.212 ms`** | **`12.714 ms`** | `25.995 ms` (`2.6%`) | `28.814 ms` | `68.862 ms` | `86.661 ms` | **`1.08x Bun` (`2.27x` Warm) / `3.25x Node`** |
| **CS-02** | `CS-02: case_studies/02-mcp-code-mode-orchestrator` (`-98.4%` Tokens) | **`19.081 ms`** (`18.48 ms` Min) | **`19.823 ms`** | **`8.094 ms`** | `16.875 ms` (`13.1%`) | `16.056 ms` | `42.538 ms` | `64.045 ms` | **`1.98x Bun Warm` / `3.36x Node`** |
| **CS-03** | `CS-03: case_studies/03-multi-tenant-saas-plugin-mesh` (12 Tenants + Hot-Swap) | **`13.198 ms`** (`12.54 ms` Min) | **`13.986 ms`** | **`5.702 ms`** | `11.400 ms` (`15.8%`) | `16.135 ms` | `47.442 ms` | `83.575 ms` | **`1.22x Bun` (`2.83x` Warm) / `6.33x Node`** |
| **CS-04** | `CS-04: case_studies/04-wasm-zero-trust-cdr-pipeline` (Wasm SimHash CDR) | **`17.055 ms`** (`16.22 ms` Min) | **`17.557 ms`** | **`2.019 ms`** | `13.108 ms` (`30.1%`) | `18.790 ms` | `43.061 ms` | `66.076 ms` | **`1.10x Bun` (`9.31x` Warm) / `3.87x Node`** |
| **CS-05** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`17.484 ms`** (`16.92 ms` Min) | **`18.490 ms`** | **`9.455 ms`** | `18.270 ms` (`-4.3%`) | `19.619 ms` | `61.474 ms` | `82.259 ms` | **`1.12x Bun` (`2.07x` Warm) / `4.70x Node`** |

---

## 4. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Workload | Slice (`SLICEBC\x01` Default) | Bun (`v1.4.4`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha`) | Slice Memory Advantage |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **App 02 (`02-npm-ecosystem-suite`: `semver`+`ms`+`ini`)** | **13,824 KB (13.50 MB)** | 30,200 KB (29.49 MB) | 47,100 KB (46.00 MB) | 71,368 KB (69.70 MB) | **5.16x less than Node / 2.18x less than Bun** |
| **App 06 (`06-telemetry-metrics-pipeline`: `bytes`+`flatted`)** | **14,328 KB (13.99 MB)** | 30,592 KB (29.88 MB) | 51,184 KB (49.98 MB) | 72,732 KB (71.03 MB) | **5.08x less than Node / 2.14x less than Bun** |
| **App 21 (`21-websocket-realtime-and-security-guard`: RFC 6455 WS)** | **13,952 KB (13.63 MB)** | 31,700 KB (30.96 MB) | 50,788 KB (49.60 MB) | 94,976 KB (92.75 MB) | **6.81x less than Node / 2.27x less than Bun** |
| **App 22 (`22-worker-threads-and-raw-sockets`: 3 Workers + TCP/UDP)** | **21,888 KB (21.38 MB)** | 41,188 KB (40.22 MB) | 63,308 KB (61.82 MB) | 93,320 KB (91.13 MB) | **4.26x less than Node / 1.88x less than Bun** |
| **App 23 (`23-wasm-crypto-and-simd-kernel`: Pure-Rust WASM)** | **18,944 KB (18.50 MB)** | 31,104 KB (30.38 MB) | 49,048 KB (47.90 MB) | 80,276 KB (78.39 MB) | **4.24x less than Node / 1.64x less than Bun** |
| **FW 01 (`frameworks/01-hono-edge-microservice`: `hono` v4)** | **14,848 KB (14.50 MB)** | 28,408 KB (27.74 MB) | 48,996 KB (47.85 MB) | 89,628 KB (87.53 MB) | **6.04x less than Node / 1.91x less than Bun** |
| **FW 02 (`frameworks/02-itty-router-unjs-nitro-stack`: UnJS / Nitro)** | **14,080 KB (13.75 MB)** | 32,000 KB (31.25 MB) | 48,724 KB (47.58 MB) | 87,176 KB (85.13 MB) | **6.19x less than Node / 2.27x less than Bun** |
| **CS 01 (`case_studies/01-tot-autonomous-sre-agent`: 20 ToT Sandboxes)** | **19,096 KB (18.65 MB)** | 30,428 KB (29.71 MB) | 50,820 KB (49.63 MB) | 81,548 KB (79.64 MB) | **4.27x less than Node / 1.59x less than Bun** |
| **CS 02 (`case_studies/02-mcp-code-mode-orchestrator`: MCP Code Mode)** | **17,664 KB (17.25 MB)** | 31,068 KB (30.34 MB) | 46,380 KB (45.29 MB) | 79,872 KB (78.00 MB) | **4.52x less than Node / 1.76x less than Bun** |
| **CS 03 (`case_studies/03-multi-tenant-saas-plugin-mesh`: 12 Tenants)** | **15,360 KB (15.00 MB)** | 26,624 KB (26.00 MB) | 48,324 KB (47.19 MB) | 85,000 KB (83.01 MB) | **5.53x less than Node / 1.73x less than Bun** |
| **CS 04 (`case_studies/04-wasm-zero-trust-cdr-pipeline`: Wasm SimHash CDR)** | **19,456 KB (19.00 MB)** | 31,616 KB (30.88 MB) | 48,752 KB (47.61 MB) | 78,868 KB (77.02 MB) | **4.05x less than Node / 1.63x less than Bun** |
| **CS 05 (`case_studies/05-time-travel-chaos-fuzzer-payments`: 48 Sagas)** | **15,616 KB (15.25 MB)** | 28,868 KB (28.19 MB) | 52,848 KB (51.61 MB) | 79,608 KB (77.74 MB) | **5.10x less than Node / 1.85x less than Bun** |

---

## 5. Multi-Tenant Edge KV, AI Agent Checkpoints & 5-Ring Security Telemetry

| Subsystem / Benchmark | Measured Telemetry | Status |
| :--- | :--- | :---: |
| **Sharded `Slice.kv` (`1,001` Get/Set/Incr Ops in Isolate)** | **`596.53 us`** total (`--warm`, `12,362 B` memory cage) | `PASS` |
| **Multi-Tenant `--tenant-dir` Zero-Downtime Hot-Swap** | Live atomic replacement of `tenant-alpha.cas` (`v1 -> v2-hot-swapped`) with isolated `Slice.kv` state | `PASS` |
| **Deterministic Time-Travel Replay (`--deterministic=20261019`)** | 100% bit-for-bit identical PRNG (`Math.random`) & virtual clock (`Date.now`, `performance.now`) | `PASS` |
| **Tree-of-Thought AI Agent (`Slice.checkpoint` + 3 `evalSandbox` + `restore`)** | **`107.06 us`** total (`15.22 us` child sandbox) | `PASS` |
| **Cloud Metadata SSRF (`169.254.169.254`) & Base64 Egress DLP Guard** | `SSRF_METADATA_BLOCKED` & `EGRESS_SECRET_LEAK_BLOCKED` deterministically trapped | `PASS` |
| **Built-In Model Context Protocol Server (`slice mcp --smoke-test`)** | JSON-RPC 2.0 `initialize`, `tools/list`, `tools/call` verified over `stdio` | `PASS` |
