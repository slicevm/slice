# Slice (Nexus-JS) - Full 33-Workload & MicroVM Benchmark Report

* **Slice Version:** `v0.14.6` (Commit `144857c`)
* **Generated Timestamp (UTC):** `2026-11-26T18:56:00Z`
* **Archive File:** [`tests/reports/bench_v0.14.6_20261126_185600Z_144857c.md`](./bench_v0.14.6_20261126_185600Z_144857c.md)
* **Canonical Latest Report:** [`tests/reports/latest.md`](./latest.md)
* **Environment:** `Linux x86_64 Docker (io_uring SQPOLL + WRPKRU)`
* **Compared Runtimes:**
  * **Slice:** `v0.14.6` (Default `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS + Monomorphic Inline Caching `get_fast` + Iterative ECMA-404 `JSON.parse`)
  * **Bun:** `v1.4.5` (JavaScriptCore)
  * **Deno:** `v2.9.8` (V8)
  * **Node.js:** `v27.0.0-alpha.0.1.1` (V8 native TypeScript execution)

---

## 1. Executive Summary

Across all **33 real-world multi-file TypeScript applications, framework suites, and case studies**, Slice beats **Deno (`v2.9.8`)** on **33/33** workloads, beats **Node.js (`v27.0.0-alpha.0.1.1`)** on **33/33** workloads, beats **Bun (`v1.4.5`)** on **33/33** workloads in cold one-shot execution, and beats **all 3 runtimes on 33/33 workloads in `--warm` pooled isolate execution**.

| Metric | Linux (`x86_64` Docker) | Windows (`x86_64` Host) | Comparison vs Runtimes & MicroVMs |
| :--- | :---: | :---: | :--- |
| **Single Isolate Cold-Start (Min / P50 / P99)** | **`0.81 us` / `0.85 us` / `1.61 us`** | **`0.80 us` / `0.90 us` / `3.10 us`** | **~120,000x faster than 125ms Firecracker VM** |
| **1,000-Tenant MicroVM Swarm (Boot + Exec)** | **`1.04 ms`** (`8.25 MB` RSS) | **`1.26 ms`** (`6.84 MB` RSS) | **> 10,000x faster than container/VM swarms** |
| **500-Module Monorepo Cold Boot (`0 stat()`)** | **`2.923 ms`** (`107.08 us` in-mem) | **`7.262 ms`** (`221.40 us` in-mem) | **24.1x faster than Node.js (`1,500+ stat()`)** |
| **10,000 Direct Kernel Ring I/O Completions** | **`0.01 ms`** | **`3.39 ms`** | **Zero worker-thread lock contention** |
| **5-Ring Security Containment Verification** | **`5 / 5 PASS`** | **`5 / 5 PASS`** | **OCap + SES + Fuel + SSRF/DLP + MPK** |

---

## 2. `slice serve` Multi-Tenant HTTP/1.1 Keep-Alive Throughput (`16,000` Reqs, 8 Sockets x 8 Pipeline)

| Server / Runtime | Throughput (`req/s`) | Avg Latency / Req | Total Time (`16k` Reqs) | Relative Speedup vs Runtimes |
| :--- | :---: | :---: | :---: | :--- |
| **`slice serve` (Tier-0 CAS, `--workers=8`)** | **`159,866 req/s`** | **`6.26 us/req`** | **`100.1 ms`** | **`1.62x` faster than `Bun.serve` / `2.90x` Node.js** |
| **`slice serve` (Tier-1 WinterTC, `--workers=8`)** | **`156,501 req/s`** | **`6.39 us/req`** | **`102.2 ms`** | **`1.59x` faster than `Bun.serve` / `2.84x` Node.js** |
| **`Deno.serve` (`v2.9.8`, V8, No Per-Req Sandbox)** | `89,842 req/s` | `11.13 us/req` | `178.1 ms` | `0.91x` vs Bun |
| **`Bun.serve` (`v1.4.5`, JSC, No Per-Req Sandbox)** | `98,564 req/s` | `10.15 us/req` | `162.3 ms` | `1.00x` (Baseline) |
| **`Node.js http.createServer` (`v27.0.0-alpha.0.1.1`, V8)** | `55,136 req/s` | `18.14 us/req` | `290.2 ms` | `0.56x` vs Bun |

---

## 3. Full 33-Workload Latency Comparison (20 Runs Each, Linux `x86_64` Docker)

| # | Workload (`tests/apps/`, `tests/frameworks/`, `tests/case_studies/`) | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm (`--warm`) | Previous `.cas` (`Delta`) | Bun `v1.4.5` | Deno `v2.9.8` | Node.js `v27.0.0-alpha.0.1.1` | Slice Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`5.309 ms`** (`4.74 ms` Min) | **`5.730 ms`** | **`0.925 ms`** | `5.292 ms` (`0.3%`) | `14.754 ms` | `35.291 ms` | `52.941 ms` | **`2.78x Bun` (`15.95x` Warm) / `9.97x Node`** |
| **02** | `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | **`11.325 ms`** (`10.82 ms` Min) | **`12.113 ms`** | **`3.278 ms`** | `11.424 ms` (`-0.9%`) | `22.970 ms` | `51.314 ms` | `64.329 ms` | **`2.03x Bun` (`7.01x` Warm) / `5.68x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`5.989 ms`** (`5.58 ms` Min) | **`7.096 ms`** | **`1.099 ms`** | `6.061 ms` (`-1.2%`) | `15.386 ms` | `42.391 ms` | `55.536 ms` | **`2.57x Bun` (`14.00x` Warm) / `9.27x Node`** |
| **04** | `04-edge-api-gateway` (`fast-querystring` + `ms`) | **`6.456 ms`** (`5.91 ms` Min) | **`6.872 ms`** | **`1.296 ms`** | `6.472 ms` (`-0.2%`) | `16.197 ms` | `41.651 ms` | `56.101 ms` | **`2.51x Bun` (`12.50x` Warm) / `8.69x Node`** |
| **05** | `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | **`8.795 ms`** (`8.27 ms` Min) | **`9.317 ms`** | **`4.688 ms`** | `8.843 ms` (`-0.5%`) | `15.896 ms` | `44.479 ms` | `57.490 ms` | **`1.81x Bun` (`3.39x` Warm) / `6.54x Node`** |
| **06** | `06-telemetry-metrics-pipeline` (`bytes` + `flatted` + WAL) | **`10.828 ms`** (`10.03 ms` Min) | **`11.183 ms`** | **`4.062 ms`** | `10.578 ms` (`2.4%`) | `18.195 ms` | `46.365 ms` | `59.253 ms` | **`1.68x Bun` (`4.48x` Warm) / `5.47x Node`** |
| **07** | `07-graphql-ast-validator` (`dlv` + `dset` + AST Cost) | **`7.808 ms`** (`7.26 ms` Min) | **`8.978 ms`** | **`3.481 ms`** | `7.795 ms` (`0.2%`) | `14.447 ms` | `42.009 ms` | `54.979 ms` | **`1.85x Bun` (`4.15x` Warm) / `7.04x Node`** |
| **08** | `08-event-driven-state-journal` (`mitt` + `ms` + JSONL Replay) | **`6.282 ms`** (`5.83 ms` Min) | **`6.704 ms`** | **`1.120 ms`** | `6.291 ms` (`-0.1%`) | `17.403 ms` | `44.680 ms` | `58.871 ms` | **`2.77x Bun` (`15.54x` Warm) / `9.37x Node`** |
| **09** | `09-wintertc-web-router` (`Request` + `Response` + `URL`, 80 reqs) | **`13.109 ms`** (`12.24 ms` Min) | **`13.832 ms`** | **`6.701 ms`** | `12.965 ms` (`1.1%`) | `15.096 ms` | `46.919 ms` | `79.259 ms` | **`1.15x Bun` (`2.25x` Warm) / `6.05x Node`** |
| **10** | `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **`10.354 ms`** (`9.72 ms` Min) | **`10.572 ms`** | **`3.806 ms`** | `10.143 ms` (`2.1%`) | `15.909 ms` | `46.790 ms` | `78.439 ms` | **`1.54x Bun` (`4.18x` Warm) / `7.58x Node`** |
| **11** | `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **`8.784 ms`** (`8.37 ms` Min) | **`9.181 ms`** | **`2.900 ms`** | `8.624 ms` (`1.9%`) | `14.335 ms` | `48.313 ms` | `80.760 ms` | **`1.63x Bun` (`4.94x` Warm) / `9.19x Node`** |
| **12** | `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP, 320 frames) | **`13.355 ms`** (`12.73 ms` Min) | **`13.777 ms`** | **`6.273 ms`** | `13.562 ms` (`-1.5%`) | `16.317 ms` | `49.385 ms` | `84.284 ms` | **`1.22x Bun` (`2.60x` Warm) / `6.31x Node`** |
| **13** | `13-webhook-orchestrator-ssrf` (`fast-querystring` + `ms` + Live `fetch`) | **`9.779 ms`** (`9.36 ms` Min) | **`10.511 ms`** | **`3.232 ms`** | `10.215 ms` (`-4.3%`) | `20.395 ms` | `52.247 ms` | `104.498 ms` | **`2.09x Bun` (`6.31x` Warm) / `10.69x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`12.976 ms`** (`12.26 ms` Min) | **`13.412 ms`** | **`7.169 ms`** | `13.098 ms` (`-0.9%`) | `22.343 ms` | `52.364 ms` | `102.578 ms` | **`1.72x Bun` (`3.12x` Warm) / `7.91x Node`** |
| **15** | `15-time-travel-test-suite` (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **`11.575 ms`** (`10.97 ms` Min) | **`11.723 ms`** | **`3.643 ms`** | `11.819 ms` (`-2.1%`) | `26.038 ms` | `67.719 ms` | `78.595 ms` | **`2.25x Bun` (`7.15x` Warm) / `6.79x Node`** |
| **16** | `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **`6.486 ms`** (`6.21 ms` Min) | **`7.108 ms`** | **`1.185 ms`** | `6.913 ms` (`-6.2%`) | `15.972 ms` | `45.702 ms` | `59.666 ms` | **`2.46x Bun` (`13.48x` Warm) / `9.20x Node`** |
| **17** | `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive/307/SSRF/DLP) | **`13.172 ms`** (`12.48 ms` Min) | **`13.559 ms`** | **`5.602 ms`** | `13.947 ms` (`-5.6%`) | `23.018 ms` | `54.551 ms` | `105.544 ms` | **`1.75x Bun` (`4.11x` Warm) / `8.01x Node`** |
| **18** | `18-async-event-loop-fanout-scheduler` (Timer Wheel/`Promise.all`/`AbortSignal`) | **`27.840 ms`** (`26.85 ms` Min) | **`28.140 ms`** | **`15.778 ms`** | `30.138 ms` (`-7.6%`) | `33.952 ms` | `71.501 ms` | `73.376 ms` | **`1.22x Bun` (`2.15x` Warm) / `2.64x Node`** |
| **19** | `19-wintertc-webcrypto-streams-edge` (WebCrypto/Streams/FormData/Clone) | **`8.354 ms`** (`7.68 ms` Min) | **`8.960 ms`** | **`1.431 ms`** | `8.497 ms` (`-1.7%`) | `14.775 ms` | `45.837 ms` | `81.956 ms` | **`1.77x Bun` (`10.32x` Warm) / `9.81x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`8.977 ms`** (`8.52 ms` Min) | **`9.422 ms`** | **`0.430 ms`** | `8.905 ms` (`0.8%`) | `23.503 ms` | `54.191 ms` | `66.277 ms` | **`2.62x Bun` (`54.66x` Warm) / `7.38x Node`** |
| **21** | `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket` + 5-Ring Guard) | **`9.092 ms`** (`8.59 ms` Min) | **`9.669 ms`** | **`2.077 ms`** | `9.588 ms` (`-5.2%`) | `21.290 ms` | `51.360 ms` | `112.639 ms` | **`2.34x Bun` (`10.25x` Warm) / `12.39x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`28.412 ms`** (`26.66 ms` Min) | **`29.244 ms`** | **`2.438 ms`** | `29.742 ms` (`-4.5%`) | `57.597 ms` | `111.442 ms` | `93.675 ms` | **`2.03x Bun` (`23.62x` Warm) / `3.30x Node`** |
| **23** | `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **`12.887 ms`** (`12.48 ms` Min) | **`13.678 ms`** | **`1.381 ms`** | `13.516 ms` (`-4.7%`) | `21.713 ms` | `40.437 ms` | `58.056 ms` | **`1.68x Bun` (`15.72x` Warm) / `4.51x Node`** |
| **24** | `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` SLA Scorer + SSR Pipeline) | **`7.127 ms`** (`6.71 ms` Min) | **`7.621 ms`** | **`1.371 ms`** | `7.165 ms` (`-0.5%`) | `19.477 ms` | `42.539 ms` | `60.273 ms` | **`2.73x Bun` (`14.21x` Warm) / `8.46x Node`** |
| **FW-01** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`17.414 ms`** (`16.53 ms` Min) | **`17.939 ms`** | **`4.967 ms`** | `17.465 ms` (`-0.3%`) | `21.334 ms` | `53.278 ms` | `88.381 ms` | **`1.23x Bun` (`4.30x` Warm) / `5.08x Node`** |
| **FW-02** | `FW-02: frameworks/02-itty-router-unjs-nitro-stack` (`itty-router` + UnJS) | **`15.704 ms`** (`14.94 ms` Min) | **`16.144 ms`** | **`8.458 ms`** | `16.108 ms` (`-2.5%`) | `20.146 ms` | `45.945 ms` | `81.950 ms` | **`1.28x Bun` (`2.38x` Warm) / `5.22x Node`** |
| **FW-03** | `FW-03: frameworks/03-node-http-express-compat` (`node:http` + `node:stream` Express/Polka) | **`13.988 ms`** (`13.27 ms` Min) | **`14.349 ms`** | **`0.881 ms`** | `14.060 ms` (`-0.5%`) | `35.934 ms` | `51.221 ms` | `78.065 ms` | **`2.57x Bun` (`40.79x` Warm) / `5.58x Node`** |
| **FW-04** | `FW-04: frameworks/04-sveltekit-edge-ssr` (`@sveltejs/kit@3.1.1` + `svelte@5.57.3` + `devalue@6.0.3`) | **`29.093 ms`** (`27.88 ms` Min) | **`29.067 ms`** | **`13.070 ms`** | `30.209 ms` (`-3.7%`) | `44.898 ms` | `70.284 ms` | `105.098 ms` | **`1.54x Bun` (`3.44x` Warm) / `3.62x Node`** |
| **CS-01** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`27.305 ms`** (`26.59 ms` Min) | **`28.318 ms`** | **`11.865 ms`** | `26.057 ms` (`4.8%`) | `29.303 ms` | `71.907 ms` | `79.738 ms` | **`1.07x Bun` (`2.47x` Warm) / `2.92x Node`** |
| **CS-02** | `CS-02: case_studies/02-mcp-code-mode-orchestrator` (`-98.4%` Tokens) | **`15.878 ms`** (`14.82 ms` Min) | **`16.915 ms`** | **`1.995 ms`** | `15.434 ms` (`2.9%`) | `17.065 ms` | `43.260 ms` | `59.229 ms` | **`1.07x Bun` (`8.55x` Warm) / `3.73x Node`** |
| **CS-03** | `CS-03: case_studies/03-multi-tenant-saas-plugin-mesh` (12 Tenants + Hot-Swap) | **`12.631 ms`** (`11.70 ms` Min) | **`13.079 ms`** | **`4.447 ms`** | `12.562 ms` (`0.5%`) | `17.434 ms` | `47.701 ms` | `77.629 ms` | **`1.38x Bun` (`3.92x` Warm) / `6.15x Node`** |
| **CS-04** | `CS-04: case_studies/04-wasm-zero-trust-cdr-pipeline` (Wasm SimHash CDR) | **`13.582 ms`** (`12.95 ms` Min) | **`14.521 ms`** | **`1.738 ms`** | `13.785 ms` (`-1.5%`) | `20.225 ms` | `43.464 ms` | `60.160 ms` | **`1.49x Bun` (`11.64x` Warm) / `4.43x Node`** |
| **CS-05** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`8.982 ms`** (`8.20 ms` Min) | **`9.104 ms`** | **`2.274 ms`** | `8.893 ms` (`1.0%`) | `20.860 ms` | `64.286 ms` | `76.571 ms` | **`2.32x Bun` (`9.17x` Warm) / `8.52x Node`** |

---

## 4. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Workload | Slice (`SLICEBC\x01` Default) | Bun (`v1.4.5`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha.0.1.1`) | Slice Memory Advantage |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **App 2: NPM semver+ms+ini** | **13,952 KB (13.63 MB)** | 38,388 KB (37.49 MB) | 47,096 KB (45.99 MB) | 70,476 KB (68.82 MB) | **5.05x less than Node / 2.75x less than Bun** |
| **App 6: Telemetry + Flatted** | **14,208 KB (13.88 MB)** | 39,680 KB (38.75 MB) | 50,552 KB (49.37 MB) | 71,784 KB (70.10 MB) | **5.05x less than Node / 2.79x less than Bun** |
| **App 21: RFC 6455 WS + Sec** | **14,592 KB (14.25 MB)** | 37,872 KB (36.98 MB) | 50,580 KB (49.39 MB) | 93,540 KB (91.35 MB) | **6.41x less than Node / 2.60x less than Bun** |
| **App 22: Workers + Sockets** | **22,528 KB (22.00 MB)** | 47,160 KB (46.05 MB) | 64,256 KB (62.75 MB) | 91,128 KB (88.99 MB) | **4.05x less than Node / 2.09x less than Bun** |
| **App 23: Pure-Rust WASM** | **19,072 KB (18.63 MB)** | 38,016 KB (37.13 MB) | 48,412 KB (47.28 MB) | 77,916 KB (76.09 MB) | **4.09x less than Node / 1.99x less than Bun** |
| **FW 1: Hono v4 Microservice** | **17,152 KB (16.75 MB)** | 40,440 KB (39.49 MB) | 50,164 KB (48.99 MB) | 87,272 KB (85.23 MB) | **5.09x less than Node / 2.36x less than Bun** |
| **FW 2: itty-router + UnJS** | **14,848 KB (14.50 MB)** | 40,448 KB (39.50 MB) | 48,940 KB (47.79 MB) | 86,380 KB (84.36 MB) | **5.82x less than Node / 2.72x less than Bun** |
| **App 24: WASM + SSR** | **14,208 KB (13.88 MB)** | 38,016 KB (37.13 MB) | 48,456 KB (47.32 MB) | 78,576 KB (76.73 MB) | **5.53x less than Node / 2.68x less than Bun** |
| **FW 3: Express node:http** | **15,232 KB (14.88 MB)** | 41,600 KB (40.63 MB) | 46,560 KB (45.47 MB) | 80,232 KB (78.35 MB) | **5.27x less than Node / 2.73x less than Bun** |
| **01-tot-autonomous-sre-agent** | **20,864 KB (20.38 MB)** | 41,728 KB (40.75 MB) | 52,736 KB (51.50 MB) | 79,976 KB (78.10 MB) | **3.83x less than Node / 2.00x less than Bun** |
| **02-mcp-code-mode-orchestrator** | **18,176 KB (17.75 MB)** | 37,248 KB (36.38 MB) | 46,740 KB (45.64 MB) | 78,280 KB (76.45 MB) | **4.31x less than Node / 2.05x less than Bun** |
| **03-multi-tenant-saas-plugin-mesh** | **14,848 KB (14.50 MB)** | 37,632 KB (36.75 MB) | 50,176 KB (49.00 MB) | 86,248 KB (84.23 MB) | **5.81x less than Node / 2.53x less than Bun** |
| **04-wasm-zero-trust-cdr-pipeline** | **18,944 KB (18.50 MB)** | 39,396 KB (38.47 MB) | 49,880 KB (48.71 MB) | 78,824 KB (76.98 MB) | **4.16x less than Node / 2.08x less than Bun** |
| **05-time-travel-chaos-fuzzer-payments** | **13,952 KB (13.63 MB)** | 38,912 KB (38.00 MB) | 49,400 KB (48.24 MB) | 79,328 KB (77.47 MB) | **5.69x less than Node / 2.79x less than Bun** |
| **FW 4: SvelteKit 3 + Svelte 5** | **22,368 KB (21.84 MB)** | 49,536 KB (48.38 MB) | 56,824 KB (55.49 MB) | 84,976 KB (82.98 MB) | **3.80x less than Node / 2.21x less than Bun** |

---

## 5. Multi-Tenant Edge KV, AI Agent Checkpoints & 5-Ring Security Telemetry

| Subsystem / Benchmark | Measured Telemetry | Status |
| :--- | :--- | :---: |
| **Sharded `Slice.kv` (`1,001` Get/Set/Incr Ops in Isolate)** | **`520.11 us`** total (`--warm`, `288 B` memory cage) | `PASS` |
| **Multi-Tenant `--tenant-dir` Zero-Downtime Hot-Swap** | Live atomic replacement of `tenant-alpha.cas` (`v1 -> v2-hot-swapped`) with isolated `Slice.kv` state | `PASS` |
| **Deterministic Time-Travel Replay (`--deterministic=20261019`)** | 100% bit-for-bit identical PRNG (`Math.random`) & virtual clock (`Date.now`, `performance.now`) | `PASS` |
| **Tree-of-Thought AI Agent (`Slice.checkpoint` + 3 `evalSandbox` + `restore`)** | **`55.16 us`** total (`1.79 us` child sandbox) | `PASS` |
| **Cloud Metadata SSRF (`169.254.169.254`) & Base64 Egress DLP Guard** | `SSRF_METADATA_BLOCKED` & `EGRESS_SECRET_LEAK_BLOCKED` deterministically trapped | `PASS` |
| **Built-In Model Context Protocol Server (`slice mcp --smoke-test`)** | JSON-RPC 2.0 `initialize`, `tools/list`, `tools/call` verified over `stdio` | `PASS` |
