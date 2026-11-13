# Slice (Nexus-JS) - Full 33-Workload & MicroVM Benchmark Report

* **Slice Version:** `v0.11.0` (Commit `1182dae`)
* **Generated Timestamp (UTC):** `2026-11-13T22:10:39Z`
* **Archive File:** [`tests/reports/bench_v0.11.0_20261113_221039Z_1182dae.md`](./bench_v0.11.0_20261113_221039Z_1182dae.md)
* **Canonical Latest Report:** [`tests/reports/latest.md`](./latest.md)
* **Environment:** `Linux x86_64 Docker (io_uring SQPOLL + WRPKRU)`
* **Compared Runtimes:**
  * **Slice:** `v0.11.0` (Default `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS + Monomorphic Inline Caching `get_fast` + Iterative ECMA-404 `JSON.parse`)
  * **Bun:** `v1.4.4` (JavaScriptCore)
  * **Deno:** `v2.9.8` (V8)
  * **Node.js:** `v27.0.0-alpha.0.0.0` (V8 native TypeScript execution)

---

## 1. Executive Summary

Across all **33 real-world multi-file TypeScript applications, framework suites, and case studies**, Slice beats **Deno (`v2.9.8`)** on **33/33** workloads, beats **Node.js (`v27.0.0-alpha.0.0.0`)** on **33/33** workloads, beats **Bun (`v1.4.4`)** on **28/33** workloads in cold one-shot execution, and beats **all 3 runtimes on 33/33 workloads in `--warm` pooled isolate execution**.

| Metric | Linux (`x86_64` Docker) | Windows (`x86_64` Host) | Comparison vs Runtimes & MicroVMs |
| :--- | :---: | :---: | :--- |
| **Single Isolate Cold-Start (Min / P50 / P99)** | **`0.99 us` / `1.26 us` / `1.74 us`** | **`1.20 us` / `1.30 us` / `2.30 us`** | **~120,000x faster than 125ms Firecracker VM** |
| **1,000-Tenant MicroVM Swarm (Boot + Exec)** | **`1.48 ms`** (`7.50 MB` RSS) | **`1.74 ms`** (`6.24 MB` RSS) | **> 10,000x faster than container/VM swarms** |
| **500-Module Monorepo Cold Boot (`0 stat()`)** | **`2.871 ms`** (`8.31 us` in-mem) | **`7.299 ms`** (`18.30 us` in-mem) | **24.5x faster than Node.js (`1,500+ stat()`)** |
| **10,000 Direct Kernel Ring I/O Completions** | **`0.01 ms`** | **`3.52 ms`** | **Zero worker-thread lock contention** |
| **5-Ring Security Containment Verification** | **`5 / 5 PASS`** | **`5 / 5 PASS`** | **OCap + SES + Fuel + SSRF/DLP + MPK** |

---

## 2. `slice serve` Multi-Tenant HTTP/1.1 Keep-Alive Throughput (`16,000` Reqs, 8 Sockets x 8 Pipeline)

| Server / Runtime | Throughput (`req/s`) | Avg Latency / Req | Total Time (`16k` Reqs) | Relative Speedup vs Runtimes |
| :--- | :---: | :---: | :---: | :--- |
| **`slice serve` (Tier-0 CAS, `--workers=8`)** | **`153,678 req/s`** | **`6.51 us/req`** | **`104.1 ms`** | **`1.66x` faster than `Bun.serve` / `2.85x` Node.js** |
| **`slice serve` (Tier-1 WinterTC, `--workers=8`)** | **`139,049 req/s`** | **`7.19 us/req`** | **`115.1 ms`** | **`1.50x` faster than `Bun.serve` / `2.58x` Node.js** |
| **`Deno.serve` (`v2.9.8`, V8, No Per-Req Sandbox)** | `89,111 req/s` | `11.22 us/req` | `179.6 ms` | `0.96x` vs Bun |
| **`Bun.serve` (`v1.4.4`, JSC, No Per-Req Sandbox)** | `92,523 req/s` | `10.81 us/req` | `172.9 ms` | `1.00x` (Baseline) |
| **`Node.js http.createServer` (`v27.0.0-alpha.0.0.0`, V8)** | `53,925 req/s` | `18.54 us/req` | `296.7 ms` | `0.58x` vs Bun |

---

## 3. Full 33-Workload Latency Comparison (20 Runs Each, Linux `x86_64` Docker)

| # | Workload (`tests/apps/`, `tests/frameworks/`, `tests/case_studies/`) | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm (`--warm`) | Previous `.cas` (`Delta`) | Bun `v1.4.4` | Deno `v2.9.8` | Node.js `v27.0.0-alpha.0.0.0` | Slice Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`6.427 ms`** (`5.88 ms` Min) | **`6.811 ms`** | **`1.113 ms`** | `6.197 ms` (`3.7%`) | `13.291 ms` | `36.850 ms` | `62.681 ms` | **`2.07x Bun` (`11.94x` Warm) / `9.75x Node`** |
| **02** | `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | **`13.579 ms`** (`12.70 ms` Min) | **`14.305 ms`** | **`3.923 ms`** | `14.246 ms` (`-4.7%`) | `22.459 ms` | `54.784 ms` | `70.653 ms` | **`1.65x Bun` (`5.72x` Warm) / `5.20x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`7.658 ms`** (`7.10 ms` Min) | **`8.306 ms`** | **`1.286 ms`** | `7.197 ms` (`6.4%`) | `14.446 ms` | `44.422 ms` | `63.233 ms` | **`1.89x Bun` (`11.23x` Warm) / `8.26x Node`** |
| **04** | `04-edge-api-gateway` (`fast-querystring` + `ms`) | **`8.117 ms`** (`7.02 ms` Min) | **`9.900 ms`** | **`1.674 ms`** | `7.332 ms` (`10.7%`) | `15.186 ms` | `42.368 ms` | `64.698 ms` | **`1.87x Bun` (`9.07x` Warm) / `7.97x Node`** |
| **05** | `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | **`10.415 ms`** (`9.79 ms` Min) | **`11.039 ms`** | **`4.553 ms`** | `10.285 ms` (`1.3%`) | `14.918 ms` | `45.405 ms` | `65.242 ms` | **`1.43x Bun` (`3.28x` Warm) / `6.26x Node`** |
| **06** | `06-telemetry-metrics-pipeline` (`bytes` + `flatted` + WAL) | **`13.207 ms`** (`12.61 ms` Min) | **`14.343 ms`** | **`5.345 ms`** | `12.826 ms` (`3.0%`) | `17.334 ms` | `49.253 ms` | `67.924 ms` | **`1.31x Bun` (`3.24x` Warm) / `5.14x Node`** |
| **07** | `07-graphql-ast-validator` (`dlv` + `dset` + AST Cost) | **`8.893 ms`** (`8.41 ms` Min) | **`9.449 ms`** | **`3.752 ms`** | `10.054 ms` (`-11.5%`) | `13.140 ms` | `42.538 ms` | `62.583 ms` | **`1.48x Bun` (`3.50x` Warm) / `7.04x Node`** |
| **08** | `08-event-driven-state-journal` (`mitt` + `ms` + JSONL Replay) | **`11.713 ms`** (`10.77 ms` Min) | **`12.174 ms`** | **`4.810 ms`** | `11.443 ms` (`2.4%`) | `16.328 ms` | `47.015 ms` | `65.736 ms` | **`1.39x Bun` (`3.39x` Warm) / `5.61x Node`** |
| **09** | `09-wintertc-web-router` (`Request` + `Response` + `URL`, 80 reqs) | **`17.402 ms`** (`16.23 ms` Min) | **`18.369 ms`** | **`9.934 ms`** | `15.519 ms` (`12.1%`) | `13.825 ms` | `48.820 ms` | `87.056 ms` | **`1.39x Bun Warm` / `5.00x Node`** |
| **10** | `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **`14.430 ms`** (`13.40 ms` Min) | **`15.319 ms`** | **`7.263 ms`** | `13.388 ms` (`7.8%`) | `14.755 ms` | `48.716 ms` | `86.869 ms` | **`1.02x Bun` (`2.03x` Warm) / `6.02x Node`** |
| **11** | `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **`12.464 ms`** (`11.10 ms` Min) | **`13.038 ms`** | **`5.230 ms`** | `10.930 ms` (`14.0%`) | `13.785 ms` | `52.331 ms` | `89.813 ms` | **`1.11x Bun` (`2.64x` Warm) / `7.21x Node`** |
| **12** | `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP, 320 frames) | **`16.259 ms`** (`15.61 ms` Min) | **`17.610 ms`** | **`7.570 ms`** | `15.296 ms` (`6.3%`) | `15.842 ms` | `51.139 ms` | `92.225 ms` | **`2.09x Bun Warm` / `5.67x Node`** |
| **13** | `13-webhook-orchestrator-ssrf` (`fast-querystring` + `ms` + Live `fetch`) | **`16.939 ms`** (`15.23 ms` Min) | **`17.991 ms`** | **`6.688 ms`** | `15.922 ms` (`6.4%`) | `19.921 ms` | `52.670 ms` | `113.783 ms` | **`1.18x Bun` (`2.98x` Warm) / `6.72x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`16.169 ms`** (`15.39 ms` Min) | **`17.456 ms`** | **`8.232 ms`** | `15.601 ms` (`3.6%`) | `21.851 ms` | `53.889 ms` | `107.692 ms` | **`1.35x Bun` (`2.65x` Warm) / `6.66x Node`** |
| **15** | `15-time-travel-test-suite` (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **`15.037 ms`** (`14.28 ms` Min) | **`18.894 ms`** | **`5.048 ms`** | `14.828 ms` (`1.4%`) | `25.346 ms` | `67.634 ms` | `84.844 ms` | **`1.69x Bun` (`5.02x` Warm) / `5.64x Node`** |
| **16** | `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **`7.998 ms`** (`7.33 ms` Min) | **`8.434 ms`** | **`1.345 ms`** | `7.737 ms` (`3.4%`) | `15.057 ms` | `46.082 ms` | `66.641 ms` | **`1.88x Bun` (`11.19x` Warm) / `8.33x Node`** |
| **17** | `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive/307/SSRF/DLP) | **`19.519 ms`** (`17.29 ms` Min) | **`18.688 ms`** | **`8.726 ms`** | `17.608 ms` (`10.9%`) | `22.249 ms` | `54.840 ms` | `115.385 ms` | **`1.19x Bun` (`2.55x` Warm) / `6.17x Node`** |
| **18** | `18-async-event-loop-fanout-scheduler` (Timer Wheel/`Promise.all`/`AbortSignal`) | **`31.740 ms`** (`29.80 ms` Min) | **`31.987 ms`** | **`17.610 ms`** | `29.524 ms` (`7.5%`) | `32.665 ms` | `72.523 ms` | `84.565 ms` | **`1.03x Bun` (`1.85x` Warm) / `2.66x Node`** |
| **19** | `19-wintertc-webcrypto-streams-edge` (WebCrypto/Streams/FormData/Clone) | **`8.890 ms`** (`8.41 ms` Min) | **`9.585 ms`** | **`1.440 ms`** | `8.378 ms` (`6.1%`) | `13.945 ms` | `47.698 ms` | `87.074 ms` | **`1.57x Bun` (`9.68x` Warm) / `9.79x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`10.805 ms`** (`9.90 ms` Min) | **`11.543 ms`** | **`0.544 ms`** | `10.830 ms` (`-0.2%`) | `22.844 ms` | `56.282 ms` | `76.172 ms` | **`2.11x Bun` (`41.99x` Warm) / `7.05x Node`** |
| **21** | `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket` + 5-Ring Guard) | **`11.324 ms`** (`10.33 ms` Min) | **`11.304 ms`** | **`2.215 ms`** | `11.083 ms` (`2.2%`) | `20.276 ms` | `52.960 ms` | `117.578 ms` | **`1.79x Bun` (`9.15x` Warm) / `10.40x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`22.598 ms`** (`21.39 ms` Min) | **`23.022 ms`** | **`4.042 ms`** | `21.431 ms` (`5.4%`) | `65.097 ms` | `113.759 ms` | `103.449 ms` | **`2.88x Bun` (`16.11x` Warm) / `4.58x Node`** |
| **23** | `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **`16.628 ms`** (`15.66 ms` Min) | **`17.133 ms`** | **`1.017 ms`** | `15.455 ms` (`7.6%`) | `21.303 ms` | `43.637 ms` | `61.759 ms` | **`1.28x Bun` (`20.95x` Warm) / `3.71x Node`** |
| **24** | `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` SLA Scorer + SSR Pipeline) | **`8.838 ms`** (`8.30 ms` Min) | **`9.144 ms`** | **`1.626 ms`** | `8.681 ms` (`1.8%`) | `22.636 ms` | `44.165 ms` | `68.298 ms` | **`2.56x Bun` (`13.92x` Warm) / `7.73x Node`** |
| **FW-01** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`15.751 ms`** (`15.22 ms` Min) | **`17.111 ms`** | **`6.819 ms`** | `16.171 ms` (`-2.6%`) | `19.756 ms` | `52.278 ms` | `93.366 ms` | **`1.25x Bun` (`2.90x` Warm) / `5.93x Node`** |
| **FW-02** | `FW-02: frameworks/02-itty-router-unjs-nitro-stack` (`itty-router` + UnJS) | **`18.206 ms`** (`17.68 ms` Min) | **`19.656 ms`** | **`10.739 ms`** | `17.377 ms` (`4.8%`) | `19.531 ms` | `47.001 ms` | `131.841 ms` | **`1.07x Bun` (`1.82x` Warm) / `7.24x Node`** |
| **FW-03** | `FW-03: frameworks/03-node-http-express-compat` (`node:http` + `node:stream` Express/Polka) | **`17.108 ms`** (`16.42 ms` Min) | **`18.794 ms`** | **`1.038 ms`** | `17.811 ms` (`-3.9%`) | `35.556 ms` | `52.670 ms` | `83.954 ms` | **`2.08x Bun` (`34.25x` Warm) / `4.91x Node`** |
| **FW-04** | `FW-04: frameworks/04-sveltekit-edge-ssr` (`@sveltejs/kit@3.1.1` + `svelte@5.57.3` + `devalue@6.0.3`) | **`34.708 ms`** (`32.44 ms` Min) | **`39.071 ms`** | **`15.472 ms`** | `33.504 ms` (`3.6%`) | `44.279 ms` | `71.176 ms` | `112.306 ms` | **`1.28x Bun` (`2.86x` Warm) / `3.24x Node`** |
| **CS-01** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`33.978 ms`** (`32.65 ms` Min) | **`35.467 ms`** | **`12.950 ms`** | `33.655 ms` (`1.0%`) | `28.247 ms` | `74.207 ms` | `88.475 ms` | **`2.18x Bun Warm` / `2.60x Node`** |
| **CS-02** | `CS-02: case_studies/02-mcp-code-mode-orchestrator` (`-98.4%` Tokens) | **`19.513 ms`** (`18.91 ms` Min) | **`20.283 ms`** | **`3.515 ms`** | `19.384 ms` (`0.7%`) | `16.513 ms` | `44.305 ms` | `66.811 ms` | **`4.70x Bun Warm` / `3.42x Node`** |
| **CS-03** | `CS-03: case_studies/03-multi-tenant-saas-plugin-mesh` (12 Tenants + Hot-Swap) | **`14.436 ms`** (`13.84 ms` Min) | **`15.221 ms`** | **`6.259 ms`** | `13.717 ms` (`5.2%`) | `16.136 ms` | `47.668 ms` | `84.429 ms` | **`1.12x Bun` (`2.58x` Warm) / `5.85x Node`** |
| **CS-04** | `CS-04: case_studies/04-wasm-zero-trust-cdr-pipeline` (Wasm SimHash CDR) | **`21.165 ms`** (`20.09 ms` Min) | **`21.529 ms`** | **`1.414 ms`** | `19.798 ms` (`6.9%`) | `18.966 ms` | `45.231 ms` | `67.024 ms` | **`13.41x Bun Warm` / `3.17x Node`** |
| **CS-05** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`17.401 ms`** (`16.33 ms` Min) | **`19.091 ms`** | **`8.310 ms`** | `17.561 ms` (`-0.9%`) | `20.075 ms` | `63.731 ms` | `82.889 ms` | **`1.15x Bun` (`2.42x` Warm) / `4.76x Node`** |

---

## 4. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Workload | Slice (`SLICEBC\x01` Default) | Bun (`v1.4.4`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha.0.0.0`) | Slice Memory Advantage |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **App 2: NPM semver+ms+ini** | **13,568 KB (13.25 MB)** | 29,504 KB (28.81 MB) | 47,268 KB (46.16 MB) | 71,064 KB (69.40 MB) | **5.24x less than Node / 2.17x less than Bun** |
| **App 6: Telemetry + Flatted** | **14,720 KB (14.38 MB)** | 29,952 KB (29.25 MB) | 51,188 KB (49.99 MB) | 72,472 KB (70.77 MB) | **4.92x less than Node / 2.03x less than Bun** |
| **App 21: RFC 6455 WS + Sec** | **14,720 KB (14.38 MB)** | 31,868 KB (31.12 MB) | 50,812 KB (49.62 MB) | 93,952 KB (91.75 MB) | **6.38x less than Node / 2.16x less than Bun** |
| **App 22: Workers + Sockets** | **23,424 KB (22.88 MB)** | 41,252 KB (40.29 MB) | 63,340 KB (61.86 MB) | 93,096 KB (90.91 MB) | **3.97x less than Node / 1.76x less than Bun** |
| **App 23: Pure-Rust WASM** | **19,328 KB (18.88 MB)** | 30,848 KB (30.13 MB) | 48,912 KB (47.77 MB) | 80,656 KB (78.77 MB) | **4.17x less than Node / 1.60x less than Bun** |
| **FW 1: Hono v4 Microservice** | **15,872 KB (15.50 MB)** | 30,848 KB (30.13 MB) | 49,236 KB (48.08 MB) | 89,376 KB (87.28 MB) | **5.63x less than Node / 1.94x less than Bun** |
| **FW 2: itty-router + UnJS** | **14,976 KB (14.63 MB)** | 31,872 KB (31.13 MB) | 48,500 KB (47.36 MB) | 87,436 KB (85.39 MB) | **5.84x less than Node / 2.13x less than Bun** |
| **App 24: WASM + SSR** | **14,208 KB (13.88 MB)** | 32,128 KB (31.38 MB) | 48,184 KB (47.05 MB) | 81,020 KB (79.12 MB) | **5.70x less than Node / 2.26x less than Bun** |
| **FW 3: Express node:http** | **15,740 KB (15.37 MB)** | 33,408 KB (32.63 MB) | 47,388 KB (46.28 MB) | 80,264 KB (78.38 MB) | **5.10x less than Node / 2.12x less than Bun** |
| **01-tot-autonomous-sre-agent** | **21,504 KB (21.00 MB)** | 31,488 KB (30.75 MB) | 53,324 KB (52.07 MB) | 83,460 KB (81.50 MB) | **3.88x less than Node / 1.46x less than Bun** |
| **02-mcp-code-mode-orchestrator** | **18,936 KB (18.49 MB)** | 27,904 KB (27.25 MB) | 47,064 KB (45.96 MB) | 82,692 KB (80.75 MB) | **4.37x less than Node / 1.47x less than Bun** |
| **03-multi-tenant-saas-plugin-mesh** | **15,616 KB (15.25 MB)** | 26,496 KB (25.88 MB) | 50,124 KB (48.95 MB) | 87,300 KB (85.25 MB) | **5.59x less than Node / 1.70x less than Bun** |
| **04-wasm-zero-trust-cdr-pipeline** | **20,992 KB (20.50 MB)** | 31,232 KB (30.50 MB) | 49,972 KB (48.80 MB) | 80,392 KB (78.51 MB) | **3.83x less than Node / 1.49x less than Bun** |
| **05-time-travel-chaos-fuzzer-payments** | **16,000 KB (15.63 MB)** | 29,056 KB (28.38 MB) | 49,392 KB (48.23 MB) | 81,444 KB (79.54 MB) | **5.09x less than Node / 1.82x less than Bun** |
| **FW 4: SvelteKit 3 + Svelte 5** | **22,912 KB (22.38 MB)** | 41,856 KB (40.88 MB) | 56,540 KB (55.21 MB) | 85,664 KB (83.66 MB) | **3.74x less than Node / 1.83x less than Bun** |

---

## 5. Multi-Tenant Edge KV, AI Agent Checkpoints & 5-Ring Security Telemetry

| Subsystem / Benchmark | Measured Telemetry | Status |
| :--- | :--- | :---: |
| **Sharded `Slice.kv` (`1,001` Get/Set/Incr Ops in Isolate)** | **`573.07 us`** total (`--warm`, `8,466 B` memory cage) | `PASS` |
| **Multi-Tenant `--tenant-dir` Zero-Downtime Hot-Swap** | Live atomic replacement of `tenant-alpha.cas` (`v1 -> v2-hot-swapped`) with isolated `Slice.kv` state | `PASS` |
| **Deterministic Time-Travel Replay (`--deterministic=20261019`)** | 100% bit-for-bit identical PRNG (`Math.random`) & virtual clock (`Date.now`, `performance.now`) | `PASS` |
| **Tree-of-Thought AI Agent (`Slice.checkpoint` + 3 `evalSandbox` + `restore`)** | **`43.78 us`** total (`1.81 us` child sandbox) | `PASS` |
| **Cloud Metadata SSRF (`169.254.169.254`) & Base64 Egress DLP Guard** | `SSRF_METADATA_BLOCKED` & `EGRESS_SECRET_LEAK_BLOCKED` deterministically trapped | `PASS` |
| **Built-In Model Context Protocol Server (`slice mcp --smoke-test`)** | JSON-RPC 2.0 `initialize`, `tools/list`, `tools/call` verified over `stdio` | `PASS` |
