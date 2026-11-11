# Slice (Nexus-JS) — Full 33-Workload & MicroVM Benchmark Report

* **Slice Version:** `v0.10.12` (Commit `dc31f74`)
* **Generated Timestamp (UTC):** `2026-11-11T17:58:36Z`
* **Archive File:** [`tests/reports/bench_v0.10.12_20261111_175836Z_dc31f74.md`](./bench_v0.10.12_20261111_175836Z_dc31f74.md)
* **Canonical Latest Report:** [`tests/reports/latest.md`](./latest.md)
* **Environment:** `Linux x86_64 Docker (io_uring SQPOLL + WRPKRU)`
* **Compared Runtimes:**
  * **Slice:** `v0.10.12` (Default `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS + Monomorphic Inline Caching `get_fast` + Iterative ECMA-404 `JSON.parse`)
  * **Bun:** `v1.4.4` (JavaScriptCore)
  * **Deno:** `v2.9.8` (V8)
  * **Node.js:** `v27.0.0-alpha.0.0.0` (V8 native TypeScript execution)

---

## 1. Executive Summary

Across all **33 real-world multi-file TypeScript applications, framework suites, and case studies**, Slice beats **Deno (`v2.9.8`)** on **33/33** workloads, beats **Node.js (`v27.0.0-alpha.0.0.0`)** on **33/33** workloads, beats **Bun (`v1.4.4`)** on **29/33** workloads in cold one-shot execution, and beats **all 3 runtimes on 33/33 workloads in `--warm` pooled isolate execution**.

| Metric | Linux (`x86_64` Docker) | Windows (`x86_64` Host) | Comparison vs Runtimes & MicroVMs |
| :--- | :---: | :---: | :--- |
| **Single Isolate Cold-Start (Min / P50 / P99)** | **`0.94 us` / `1.02 us` / `4.27 us`** | **`1.40 us` / `1.60 us` / `2.30 us`** | **~120,000x faster than 125ms Firecracker VM** |
| **1,000-Tenant MicroVM Swarm (Boot + Exec)** | **`1.74 ms`** (`9.82 MB` RSS) | **`2.24 ms`** (`11.82 MB` RSS) | **> 10,000x faster than container/VM swarms** |
| **500-Module Monorepo Cold Boot (`0 stat()`)** | **`3.408 ms`** (`467.08 us` in-mem) | **`5.637 ms`** (`640.00 us` in-mem) | **20.6x faster than Node.js (`1,500+ stat()`)** |
| **10,000 Direct Kernel Ring I/O Completions** | **`0.02 ms`** | **`5.11 ms`** | **Zero worker-thread lock contention** |
| **5-Ring Security Containment Verification** | **`5 / 5 PASS`** | **`5 / 5 PASS`** | **OCap + SES + Fuel + SSRF/DLP + MPK** |

---

## 2. `slice serve` Multi-Tenant HTTP/1.1 Keep-Alive Throughput (`16,000` Reqs, 8 Sockets x 8 Pipeline)

| Server / Runtime | Throughput (`req/s`) | Avg Latency / Req | Total Time (`16k` Reqs) | Relative Speedup vs Runtimes |
| :--- | :---: | :---: | :---: | :--- |
| **`slice serve` (Tier-0 CAS, `--workers=8`)** | **`154,794 req/s`** | **`6.46 us/req`** | **`103.4 ms`** | **`1.54x` faster than `Bun.serve` / `2.74x` Node.js** |
| **`slice serve` (Tier-1 WinterTC, `--workers=8`)** | **`147,564 req/s`** | **`6.78 us/req`** | **`108.4 ms`** | **`1.47x` faster than `Bun.serve` / `2.61x` Node.js** |
| **`Deno.serve` (`v2.9.8`, V8, No Per-Req Sandbox)** | `99,082 req/s` | `10.09 us/req` | `161.5 ms` | `0.99x` vs Bun |
| **`Bun.serve` (`v1.4.4`, JSC, No Per-Req Sandbox)** | `100,562 req/s` | `9.94 us/req` | `159.1 ms` | `1.00x` (Baseline) |
| **`Node.js http.createServer` (`v27.0.0-alpha.0.0.0`, V8)** | `56,447 req/s` | `17.72 us/req` | `283.5 ms` | `0.56x` vs Bun |

---

## 3. Full 33-Workload Latency Comparison (20 Runs Each, Linux `x86_64` Docker)

| # | Workload (`tests/apps/`, `tests/frameworks/`, `tests/case_studies/`) | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm (`--warm`) | Previous `.cas` (`Delta`) | Bun `v1.4.4` | Deno `v2.9.8` | Node.js `v27.0.0-alpha.0.0.0` | Slice Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`6.278 ms`** (`5.88 ms` Min) | **`6.621 ms`** | **`1.050 ms`** | `6.698 ms` (`-6.3%`) | `13.377 ms` | `35.217 ms` | `58.453 ms` | **`2.13x Bun` (`12.74x` Warm) / `9.31x Node`** |
| **02** | `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | **`14.228 ms`** (`13.36 ms` Min) | **`14.717 ms`** | **`3.817 ms`** | `14.785 ms` (`-3.8%`) | `21.069 ms` | `55.166 ms` | `69.341 ms` | **`1.48x Bun` (`5.52x` Warm) / `4.87x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`7.250 ms`** (`6.81 ms` Min) | **`7.800 ms`** | **`1.289 ms`** | `7.617 ms` (`-4.8%`) | `14.082 ms` | `43.228 ms` | `61.792 ms` | **`1.94x Bun` (`10.92x` Warm) / `8.52x Node`** |
| **04** | `04-edge-api-gateway` (`fast-querystring` + `ms`) | **`7.408 ms`** (`6.98 ms` Min) | **`8.033 ms`** | **`1.736 ms`** | `7.559 ms` (`-2.0%`) | `15.082 ms` | `41.232 ms` | `59.873 ms` | **`2.04x Bun` (`8.69x` Warm) / `8.08x Node`** |
| **05** | `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | **`10.074 ms`** (`9.45 ms` Min) | **`10.543 ms`** | **`4.654 ms`** | `10.511 ms` (`-4.2%`) | `14.766 ms` | `45.411 ms` | `62.437 ms` | **`1.47x Bun` (`3.17x` Warm) / `6.20x Node`** |
| **06** | `06-telemetry-metrics-pipeline` (`bytes` + `flatted` + WAL) | **`12.738 ms`** (`12.19 ms` Min) | **`13.970 ms`** | **`4.330 ms`** | `14.045 ms` (`-9.3%`) | `16.697 ms` | `46.630 ms` | `65.983 ms` | **`1.31x Bun` (`3.86x` Warm) / `5.18x Node`** |
| **07** | `07-graphql-ast-validator` (`dlv` + `dset` + AST Cost) | **`8.937 ms`** (`8.40 ms` Min) | **`9.786 ms`** | **`3.758 ms`** | `9.193 ms` (`-2.8%`) | `13.020 ms` | `41.561 ms` | `61.914 ms` | **`1.46x Bun` (`3.46x` Warm) / `6.93x Node`** |
| **08** | `08-event-driven-state-journal` (`mitt` + `ms` + JSONL Replay) | **`11.229 ms`** (`10.45 ms` Min) | **`14.547 ms`** | **`5.258 ms`** | `11.667 ms` (`-3.8%`) | `15.902 ms` | `46.355 ms` | `62.965 ms` | **`1.42x Bun` (`3.02x` Warm) / `5.61x Node`** |
| **09** | `09-wintertc-web-router` (`Request` + `Response` + `URL`, 80 reqs) | **`14.771 ms`** (`14.22 ms` Min) | **`15.429 ms`** | **`9.085 ms`** | `16.323 ms` (`-9.5%`) | `13.919 ms` | `47.276 ms` | `85.354 ms` | **`1.53x Bun Warm` / `5.78x Node`** |
| **10** | `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **`12.802 ms`** (`12.16 ms` Min) | **`13.616 ms`** | **`6.308 ms`** | `13.778 ms` (`-7.1%`) | `14.467 ms` | `50.395 ms` | `84.816 ms` | **`1.13x Bun` (`2.29x` Warm) / `6.63x Node`** |
| **11** | `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **`10.042 ms`** (`9.54 ms` Min) | **`10.698 ms`** | **`3.656 ms`** | `10.366 ms` (`-3.1%`) | `13.499 ms` | `47.729 ms` | `87.639 ms` | **`1.34x Bun` (`3.69x` Warm) / `8.73x Node`** |
| **12** | `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP, 320 frames) | **`15.593 ms`** (`14.85 ms` Min) | **`15.893 ms`** | **`7.429 ms`** | `15.430 ms` (`1.1%`) | `15.213 ms` | `48.694 ms` | `90.253 ms` | **`2.05x Bun Warm` / `5.79x Node`** |
| **13** | `13-webhook-orchestrator-ssrf` (`fast-querystring` + `ms` + Live `fetch`) | **`15.199 ms`** (`14.71 ms` Min) | **`16.293 ms`** | **`6.388 ms`** | `17.563 ms` (`-13.5%`) | `18.942 ms` | `51.379 ms` | `111.110 ms` | **`1.25x Bun` (`2.97x` Warm) / `7.31x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`15.828 ms`** (`15.01 ms` Min) | **`16.384 ms`** | **`6.697 ms`** | `16.692 ms` (`-5.2%`) | `21.200 ms` | `56.402 ms` | `108.617 ms` | **`1.34x Bun` (`3.17x` Warm) / `6.86x Node`** |
| **15** | `15-time-travel-test-suite` (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **`15.273 ms`** (`14.53 ms` Min) | **`15.908 ms`** | **`4.757 ms`** | `16.035 ms` (`-4.8%`) | `24.939 ms` | `66.357 ms` | `84.077 ms` | **`1.63x Bun` (`5.24x` Warm) / `5.50x Node`** |
| **16** | `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **`7.598 ms`** (`6.83 ms` Min) | **`8.351 ms`** | **`1.234 ms`** | `8.079 ms` (`-6.0%`) | `14.844 ms` | `48.547 ms` | `67.533 ms` | **`1.95x Bun` (`12.03x` Warm) / `8.89x Node`** |
| **17** | `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive/307/SSRF/DLP) | **`17.333 ms`** (`16.70 ms` Min) | **`18.916 ms`** | **`7.896 ms`** | `17.376 ms` (`-0.2%`) | `22.634 ms` | `55.132 ms` | `109.514 ms` | **`1.31x Bun` (`2.87x` Warm) / `6.32x Node`** |
| **18** | `18-async-event-loop-fanout-scheduler` (Timer Wheel/`Promise.all`/`AbortSignal`) | **`29.326 ms`** (`27.79 ms` Min) | **`30.533 ms`** | **`16.971 ms`** | `30.841 ms` (`-4.9%`) | `32.616 ms` | `70.809 ms` | `81.452 ms` | **`1.11x Bun` (`1.92x` Warm) / `2.78x Node`** |
| **19** | `19-wintertc-webcrypto-streams-edge` (WebCrypto/Streams/FormData/Clone) | **`8.118 ms`** (`7.68 ms` Min) | **`9.079 ms`** | **`1.396 ms`** | `8.515 ms` (`-4.7%`) | `13.794 ms` | `44.403 ms` | `86.921 ms` | **`1.70x Bun` (`9.88x` Warm) / `10.71x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`11.261 ms`** (`10.21 ms` Min) | **`11.623 ms`** | **`0.496 ms`** | `11.298 ms` (`-0.3%`) | `25.080 ms` | `55.776 ms` | `73.350 ms` | **`2.23x Bun` (`50.56x` Warm) / `6.51x Node`** |
| **21** | `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket` + 5-Ring Guard) | **`10.714 ms`** (`10.08 ms` Min) | **`11.463 ms`** | **`2.166 ms`** | `11.073 ms` (`-3.2%`) | `20.494 ms` | `51.042 ms` | `116.007 ms` | **`1.91x Bun` (`9.46x` Warm) / `10.83x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`20.988 ms`** (`20.33 ms` Min) | **`21.637 ms`** | **`1.739 ms`** | `20.159 ms` (`4.1%`) | `57.152 ms` | `114.016 ms` | `101.723 ms` | **`2.72x Bun` (`32.86x` Warm) / `4.85x Node`** |
| **23** | `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **`15.548 ms`** (`14.73 ms` Min) | **`16.009 ms`** | **`1.970 ms`** | `15.430 ms` (`0.8%`) | `21.684 ms` | `40.550 ms` | `63.213 ms` | **`1.39x Bun` (`11.01x` Warm) / `4.07x Node`** |
| **24** | `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` SLA Scorer + SSR Pipeline) | **`8.719 ms`** (`7.84 ms` Min) | **`8.994 ms`** | **`1.758 ms`** | `8.744 ms` (`-0.3%`) | `19.118 ms` | `42.405 ms` | `67.199 ms` | **`2.19x Bun` (`10.87x` Warm) / `7.71x Node`** |
| **FW-01** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`14.653 ms`** (`14.06 ms` Min) | **`15.350 ms`** | **`5.593 ms`** | `15.219 ms` (`-3.7%`) | `19.858 ms` | `51.149 ms` | `94.150 ms` | **`1.36x Bun` (`3.55x` Warm) / `6.43x Node`** |
| **FW-02** | `FW-02: frameworks/02-itty-router-unjs-nitro-stack` (`itty-router` + UnJS) | **`17.364 ms`** (`16.63 ms` Min) | **`17.789 ms`** | **`9.113 ms`** | `17.819 ms` (`-2.6%`) | `18.943 ms` | `46.132 ms` | `86.398 ms` | **`1.09x Bun` (`2.08x` Warm) / `4.98x Node`** |
| **FW-03** | `FW-03: frameworks/03-node-http-express-compat` (`node:http` + `node:stream` Express/Polka) | **`16.742 ms`** (`16.24 ms` Min) | **`18.053 ms`** | **`0.927 ms`** | `17.267 ms` (`-3.0%`) | `34.374 ms` | `51.235 ms` | `84.492 ms` | **`2.05x Bun` (`37.08x` Warm) / `5.05x Node`** |
| **FW-04** | `FW-04: frameworks/04-sveltekit-edge-ssr` (`@sveltejs/kit@3.1.1` + `svelte@5.57.3` + `devalue@6.0.3`) | **`34.372 ms`** (`32.03 ms` Min) | **`34.188 ms`** | **`14.260 ms`** | `32.075 ms` (`7.2%`) | `43.709 ms` | `68.042 ms` | `108.544 ms` | **`1.28x Bun` (`3.07x` Warm) / `3.17x Node`** |
| **CS-01** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`34.837 ms`** (`32.96 ms` Min) | **`36.000 ms`** | **`12.991 ms`** | `34.706 ms` (`0.4%`) | `28.855 ms` | `73.549 ms` | `91.045 ms` | **`2.22x Bun Warm` / `2.61x Node`** |
| **CS-02** | `CS-02: case_studies/02-mcp-code-mode-orchestrator` (`-98.4%` Tokens) | **`19.490 ms`** (`17.87 ms` Min) | **`19.711 ms`** | **`3.241 ms`** | `18.871 ms` (`3.3%`) | `16.986 ms` | `44.877 ms` | `68.991 ms` | **`5.24x Bun Warm` / `3.54x Node`** |
| **CS-03** | `CS-03: case_studies/03-multi-tenant-saas-plugin-mesh` (12 Tenants + Hot-Swap) | **`13.171 ms`** (`12.59 ms` Min) | **`14.508 ms`** | **`5.274 ms`** | `13.687 ms` (`-3.8%`) | `16.965 ms` | `49.996 ms` | `87.777 ms` | **`1.29x Bun` (`3.22x` Warm) / `6.66x Node`** |
| **CS-04** | `CS-04: case_studies/04-wasm-zero-trust-cdr-pipeline` (Wasm SimHash CDR) | **`20.235 ms`** (`18.73 ms` Min) | **`21.128 ms`** | **`1.501 ms`** | `19.960 ms` (`1.4%`) | `20.878 ms` | `48.725 ms` | `70.429 ms` | **`1.03x Bun` (`13.91x` Warm) / `3.48x Node`** |
| **CS-05** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`18.139 ms`** (`16.44 ms` Min) | **`19.258 ms`** | **`8.323 ms`** | `18.213 ms` (`-0.4%`) | `21.102 ms` | `65.542 ms` | `86.649 ms` | **`1.16x Bun` (`2.54x` Warm) / `4.78x Node`** |

---

## 4. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Workload | Slice (`SLICEBC\x01` Default) | Bun (`v1.4.4`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha.0.0.0`) | Slice Memory Advantage |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **App 2: NPM semver+ms+ini** | **14,464 KB (14.13 MB)** | 30,336 KB (29.63 MB) | 47,068 KB (45.96 MB) | 71,836 KB (70.15 MB) | **4.97x less than Node / 2.10x less than Bun** |
| **App 6: Telemetry + Flatted** | **13,776 KB (13.45 MB)** | 29,952 KB (29.25 MB) | 50,800 KB (49.61 MB) | 72,724 KB (71.02 MB) | **5.28x less than Node / 2.17x less than Bun** |
| **App 21: RFC 6455 WS + Sec** | **14,080 KB (13.75 MB)** | 31,860 KB (31.11 MB) | 50,564 KB (49.38 MB) | 95,236 KB (93.00 MB) | **6.76x less than Node / 2.26x less than Bun** |
| **App 22: Workers + Sockets** | **22,144 KB (21.63 MB)** | 40,480 KB (39.53 MB) | 63,880 KB (62.38 MB) | 92,416 KB (90.25 MB) | **4.17x less than Node / 1.83x less than Bun** |
| **App 23: Pure-Rust WASM** | **19,072 KB (18.63 MB)** | 30,848 KB (30.13 MB) | 48,792 KB (47.65 MB) | 80,528 KB (78.64 MB) | **4.22x less than Node / 1.62x less than Bun** |
| **FW 1: Hono v4 Microservice** | **15,360 KB (15.00 MB)** | 31,084 KB (30.36 MB) | 49,664 KB (48.50 MB) | 89,376 KB (87.28 MB) | **5.82x less than Node / 2.02x less than Bun** |
| **FW 2: itty-router + UnJS** | **14,336 KB (14.00 MB)** | 31,872 KB (31.13 MB) | 48,288 KB (47.16 MB) | 87,436 KB (85.39 MB) | **6.10x less than Node / 2.22x less than Bun** |
| **App 24: WASM + SSR** | **13,832 KB (13.51 MB)** | 32,512 KB (31.75 MB) | 48,528 KB (47.39 MB) | 81,024 KB (79.13 MB) | **5.86x less than Node / 2.35x less than Bun** |
| **FW 3: Express node:http** | **15,488 KB (15.13 MB)** | 33,152 KB (32.38 MB) | 47,088 KB (45.98 MB) | 80,144 KB (78.27 MB) | **5.17x less than Node / 2.14x less than Bun** |
| **01-tot-autonomous-sre-agent** | **20,992 KB (20.50 MB)** | 31,104 KB (30.38 MB) | 53,228 KB (51.98 MB) | 83,588 KB (81.63 MB) | **3.98x less than Node / 1.48x less than Bun** |
| **02-mcp-code-mode-orchestrator** | **18,552 KB (18.12 MB)** | 28,288 KB (27.63 MB) | 46,784 KB (45.69 MB) | 80,776 KB (78.88 MB) | **4.35x less than Node / 1.52x less than Bun** |
| **03-multi-tenant-saas-plugin-mesh** | **15,104 KB (14.75 MB)** | 26,752 KB (26.13 MB) | 49,484 KB (48.32 MB) | 88,456 KB (86.38 MB) | **5.86x less than Node / 1.77x less than Bun** |
| **04-wasm-zero-trust-cdr-pipeline** | **20,608 KB (20.13 MB)** | 31,360 KB (30.63 MB) | 49,912 KB (48.74 MB) | 80,260 KB (78.38 MB) | **3.89x less than Node / 1.52x less than Bun** |
| **05-time-travel-chaos-fuzzer-payments** | **15,232 KB (14.88 MB)** | 29,440 KB (28.75 MB) | 49,488 KB (48.33 MB) | 81,444 KB (79.54 MB) | **5.35x less than Node / 1.93x less than Bun** |
| **FW 4: SvelteKit 3 + Svelte 5** | **21,632 KB (21.13 MB)** | 41,216 KB (40.25 MB) | 56,580 KB (55.25 MB) | 85,536 KB (83.53 MB) | **3.95x less than Node / 1.91x less than Bun** |

---

## 5. Multi-Tenant Edge KV, AI Agent Checkpoints & 5-Ring Security Telemetry

| Subsystem / Benchmark | Measured Telemetry | Status |
| :--- | :--- | :---: |
| **Sharded `Slice.kv` (`1,001` Get/Set/Incr Ops in Isolate)** | **`564.84 us`** total (`--warm`, `8,431 B` memory cage) | `PASS` |
| **Multi-Tenant `--tenant-dir` Zero-Downtime Hot-Swap** | Live atomic replacement of `tenant-alpha.cas` (`v1 -> v2-hot-swapped`) with isolated `Slice.kv` state | `PASS` |
| **Deterministic Time-Travel Replay (`--deterministic=20261019`)** | 100% bit-for-bit identical PRNG (`Math.random`) & virtual clock (`Date.now`, `performance.now`) | `PASS` |
| **Tree-of-Thought AI Agent (`Slice.checkpoint` + 3 `evalSandbox` + `restore`)** | **`74.64 us`** total (`3.50 us` child sandbox) | `PASS` |
| **Cloud Metadata SSRF (`169.254.169.254`) & Base64 Egress DLP Guard** | `SSRF_METADATA_BLOCKED` & `EGRESS_SECRET_LEAK_BLOCKED` deterministically trapped | `PASS` |
| **Built-In Model Context Protocol Server (`slice mcp --smoke-test`)** | JSON-RPC 2.0 `initialize`, `tools/list`, `tools/call` verified over `stdio` | `PASS` |
