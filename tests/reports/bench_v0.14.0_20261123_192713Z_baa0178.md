# Slice (Nexus-JS) - Full 33-Workload & MicroVM Benchmark Report

* **Slice Version:** `v0.14.0` (Commit `baa0178`)
* **Generated Timestamp (UTC):** `2026-11-23T19:27:13Z`
* **Archive File:** [`tests/reports/bench_v0.14.0_20261123_192713Z_baa0178.md`](./bench_v0.14.0_20261123_192713Z_baa0178.md)
* **Canonical Latest Report:** [`tests/reports/latest.md`](./latest.md)
* **Environment:** `Linux x86_64 Docker (io_uring SQPOLL + WRPKRU)`
* **Compared Runtimes:**
  * **Slice:** `v0.14.0` (Default `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS + Monomorphic Inline Caching `get_fast` + Iterative ECMA-404 `JSON.parse`)
  * **Bun:** `v1.4.4` (JavaScriptCore)
  * **Deno:** `v2.9.8` (V8)
  * **Node.js:** `v27.0.0-alpha.0.0.0` (V8 native TypeScript execution)

---

## 1. Executive Summary

Across all **33 real-world multi-file TypeScript applications, framework suites, and case studies**, Slice beats **Deno (`v2.9.8`)** on **33/33** workloads, beats **Node.js (`v27.0.0-alpha.0.0.0`)** on **33/33** workloads, beats **Bun (`v1.4.4`)** on **33/33** workloads in cold one-shot execution, and beats **all 3 runtimes on 33/33 workloads in `--warm` pooled isolate execution**.

| Metric | Linux (`x86_64` Docker) | Windows (`x86_64` Host) | Comparison vs Runtimes & MicroVMs |
| :--- | :---: | :---: | :--- |
| **Single Isolate Cold-Start (Min / P50 / P99)** | **`0.80 us` / `0.84 us` / `0.92 us`** | **`0.80 us` / `1.50 us` / `4.50 us`** | **~120,000x faster than 125ms Firecracker VM** |
| **1,000-Tenant MicroVM Swarm (Boot + Exec)** | **`1.02 ms`** (`8.38 MB` RSS) | **`1.79 ms`** (`6.66 MB` RSS) | **> 10,000x faster than container/VM swarms** |
| **500-Module Monorepo Cold Boot (`0 stat()`)** | **`2.799 ms`** (`107.40 us` in-mem) | **`8.119 ms`** (`193.50 us` in-mem) | **25.1x faster than Node.js (`1,500+ stat()`)** |
| **10,000 Direct Kernel Ring I/O Completions** | **`0.01 ms`** | **`3.61 ms`** | **Zero worker-thread lock contention** |
| **5-Ring Security Containment Verification** | **`5 / 5 PASS`** | **`5 / 5 PASS`** | **OCap + SES + Fuel + SSRF/DLP + MPK** |

---

## 2. `slice serve` Multi-Tenant HTTP/1.1 Keep-Alive Throughput (`16,000` Reqs, 8 Sockets x 8 Pipeline)

| Server / Runtime | Throughput (`req/s`) | Avg Latency / Req | Total Time (`16k` Reqs) | Relative Speedup vs Runtimes |
| :--- | :---: | :---: | :---: | :--- |
| **`slice serve` (Tier-0 CAS, `--workers=8`)** | **`161,478 req/s`** | **`6.19 us/req`** | **`99.1 ms`** | **`1.60x` faster than `Bun.serve` / `2.81x` Node.js** |
| **`slice serve` (Tier-1 WinterTC, `--workers=8`)** | **`157,571 req/s`** | **`6.35 us/req`** | **`101.5 ms`** | **`1.56x` faster than `Bun.serve` / `2.74x` Node.js** |
| **`Deno.serve` (`v2.9.8`, V8, No Per-Req Sandbox)** | `101,131 req/s` | `9.89 us/req` | `158.2 ms` | `1.00x` vs Bun |
| **`Bun.serve` (`v1.4.4`, JSC, No Per-Req Sandbox)** | `100,858 req/s` | `9.91 us/req` | `158.6 ms` | `1.00x` (Baseline) |
| **`Node.js http.createServer` (`v27.0.0-alpha.0.0.0`, V8)** | `57,467 req/s` | `17.40 us/req` | `278.4 ms` | `0.57x` vs Bun |

---

## 3. Full 33-Workload Latency Comparison (20 Runs Each, Linux `x86_64` Docker)

| # | Workload (`tests/apps/`, `tests/frameworks/`, `tests/case_studies/`) | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm (`--warm`) | Previous `.cas` (`Delta`) | Bun `v1.4.4` | Deno `v2.9.8` | Node.js `v27.0.0-alpha.0.0.0` | Slice Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`5.289 ms`** (`4.75 ms` Min) | **`5.719 ms`** | **`0.775 ms`** | `5.383 ms` (`-1.7%`) | `13.714 ms` | `34.646 ms` | `60.459 ms` | **`2.59x Bun` (`17.70x` Warm) / `11.43x Node`** |
| **02** | `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | **`11.749 ms`** (`11.10 ms` Min) | **`16.431 ms`** | **`3.451 ms`** | `11.157 ms` (`5.3%`) | `21.174 ms` | `53.336 ms` | `70.391 ms` | **`1.80x Bun` (`6.14x` Warm) / `5.99x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`6.179 ms`** (`5.61 ms` Min) | **`6.680 ms`** | **`1.480 ms`** | `6.010 ms` (`2.8%`) | `13.886 ms` | `43.454 ms` | `62.070 ms` | **`2.25x Bun` (`9.38x` Warm) / `10.05x Node`** |
| **04** | `04-edge-api-gateway` (`fast-querystring` + `ms`) | **`6.354 ms`** (`5.97 ms` Min) | **`6.974 ms`** | **`1.483 ms`** | `6.049 ms` (`5.0%`) | `15.056 ms` | `42.370 ms` | `64.476 ms` | **`2.37x Bun` (`10.15x` Warm) / `10.15x Node`** |
| **05** | `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | **`8.593 ms`** (`8.08 ms` Min) | **`8.977 ms`** | **`4.526 ms`** | `8.291 ms` (`3.6%`) | `14.320 ms` | `45.111 ms` | `62.449 ms` | **`1.67x Bun` (`3.16x` Warm) / `7.27x Node`** |
| **06** | `06-telemetry-metrics-pipeline` (`bytes` + `flatted` + WAL) | **`10.719 ms`** (`10.24 ms` Min) | **`11.145 ms`** | **`5.255 ms`** | `10.835 ms` (`-1.1%`) | `17.076 ms` | `50.378 ms` | `67.675 ms` | **`1.59x Bun` (`3.25x` Warm) / `6.31x Node`** |
| **07** | `07-graphql-ast-validator` (`dlv` + `dset` + AST Cost) | **`7.451 ms`** (`6.96 ms` Min) | **`7.986 ms`** | **`3.277 ms`** | `7.342 ms` (`1.5%`) | `13.984 ms` | `46.707 ms` | `62.072 ms` | **`1.88x Bun` (`4.27x` Warm) / `8.33x Node`** |
| **08** | `08-event-driven-state-journal` (`mitt` + `ms` + JSONL Replay) | **`6.200 ms`** (`5.75 ms` Min) | **`6.580 ms`** | **`1.231 ms`** | `6.189 ms` (`0.2%`) | `15.726 ms` | `46.207 ms` | `63.441 ms` | **`2.54x Bun` (`12.77x` Warm) / `10.23x Node`** |
| **09** | `09-wintertc-web-router` (`Request` + `Response` + `URL`, 80 reqs) | **`13.224 ms`** (`12.54 ms` Min) | **`13.433 ms`** | **`6.633 ms`** | `12.816 ms` (`3.2%`) | `14.037 ms` | `47.911 ms` | `87.990 ms` | **`1.06x Bun` (`2.12x` Warm) / `6.65x Node`** |
| **10** | `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **`10.145 ms`** (`9.07 ms` Min) | **`10.861 ms`** | **`3.613 ms`** | `9.423 ms` (`7.7%`) | `14.639 ms` | `62.650 ms` | `85.783 ms` | **`1.44x Bun` (`4.05x` Warm) / `8.46x Node`** |
| **11** | `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **`8.939 ms`** (`8.44 ms` Min) | **`9.601 ms`** | **`2.948 ms`** | `8.489 ms` (`5.3%`) | `13.322 ms` | `49.017 ms` | `86.450 ms` | **`1.49x Bun` (`4.52x` Warm) / `9.67x Node`** |
| **12** | `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP, 320 frames) | **`13.153 ms`** (`12.34 ms` Min) | **`13.655 ms`** | **`6.290 ms`** | `13.394 ms` (`-1.8%`) | `15.968 ms` | `50.044 ms` | `89.697 ms` | **`1.21x Bun` (`2.54x` Warm) / `6.82x Node`** |
| **13** | `13-webhook-orchestrator-ssrf` (`fast-querystring` + `ms` + Live `fetch`) | **`10.002 ms`** (`9.47 ms` Min) | **`10.535 ms`** | **`3.218 ms`** | `9.606 ms` (`4.1%`) | `19.336 ms` | `51.324 ms` | `113.465 ms` | **`1.93x Bun` (`6.01x` Warm) / `11.34x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`12.834 ms`** (`11.86 ms` Min) | **`12.996 ms`** | **`7.063 ms`** | `12.176 ms` (`5.4%`) | `20.735 ms` | `52.966 ms` | `110.174 ms` | **`1.62x Bun` (`2.94x` Warm) / `8.58x Node`** |
| **15** | `15-time-travel-test-suite` (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **`12.121 ms`** (`10.87 ms` Min) | **`12.237 ms`** | **`3.452 ms`** | `11.454 ms` (`5.8%`) | `25.046 ms` | `65.406 ms` | `83.804 ms` | **`2.07x Bun` (`7.26x` Warm) / `6.91x Node`** |
| **16** | `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **`6.487 ms`** (`6.05 ms` Min) | **`7.120 ms`** | **`1.235 ms`** | `6.375 ms` (`1.8%`) | `14.737 ms` | `46.037 ms` | `65.973 ms` | **`2.27x Bun` (`11.93x` Warm) / `10.17x Node`** |
| **17** | `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive/307/SSRF/DLP) | **`13.047 ms`** (`12.25 ms` Min) | **`14.005 ms`** | **`5.873 ms`** | `12.720 ms` (`2.6%`) | `21.574 ms` | `54.880 ms` | `113.117 ms` | **`1.65x Bun` (`3.67x` Warm) / `8.67x Node`** |
| **18** | `18-async-event-loop-fanout-scheduler` (Timer Wheel/`Promise.all`/`AbortSignal`) | **`28.102 ms`** (`26.26 ms` Min) | **`28.768 ms`** | **`17.199 ms`** | `27.555 ms` (`2.0%`) | `32.819 ms` | `70.327 ms` | `82.473 ms` | **`1.17x Bun` (`1.91x` Warm) / `2.93x Node`** |
| **19** | `19-wintertc-webcrypto-streams-edge` (WebCrypto/Streams/FormData/Clone) | **`8.315 ms`** (`8.03 ms` Min) | **`9.005 ms`** | **`1.426 ms`** | `8.173 ms` (`1.7%`) | `13.951 ms` | `44.739 ms` | `88.289 ms` | **`1.68x Bun` (`9.78x` Warm) / `10.62x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`9.148 ms`** (`8.65 ms` Min) | **`9.462 ms`** | **`0.429 ms`** | `8.765 ms` (`4.4%`) | `22.649 ms` | `53.999 ms` | `73.570 ms` | **`2.48x Bun` (`52.79x` Warm) / `8.04x Node`** |
| **21** | `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket` + 5-Ring Guard) | **`9.202 ms`** (`8.67 ms` Min) | **`9.671 ms`** | **`2.048 ms`** | `8.922 ms` (`3.1%`) | `19.625 ms` | `50.541 ms` | `117.840 ms` | **`2.13x Bun` (`9.58x` Warm) / `12.81x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`29.712 ms`** (`27.53 ms` Min) | **`29.280 ms`** | **`2.228 ms`** | `19.573 ms` (`51.8%`) | `56.581 ms` | `114.836 ms` | `103.869 ms` | **`1.93x Bun` (`25.40x` Warm) / `3.55x Node`** |
| **23** | `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **`13.876 ms`** (`13.04 ms` Min) | **`14.116 ms`** | **`1.416 ms`** | `12.765 ms` (`8.7%`) | `21.831 ms` | `42.355 ms` | `64.389 ms` | **`1.57x Bun` (`15.42x` Warm) / `4.64x Node`** |
| **24** | `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` SLA Scorer + SSR Pipeline) | **`7.438 ms`** (`7.01 ms` Min) | **`8.153 ms`** | **`1.371 ms`** | `6.882 ms` (`8.1%`) | `18.847 ms` | `42.763 ms` | `66.656 ms` | **`2.53x Bun` (`13.75x` Warm) / `8.96x Node`** |
| **FW-01** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`13.266 ms`** (`12.33 ms` Min) | **`13.682 ms`** | **`4.962 ms`** | `12.648 ms` (`4.9%`) | `19.550 ms` | `52.380 ms` | `96.618 ms` | **`1.47x Bun` (`3.94x` Warm) / `7.28x Node`** |
| **FW-02** | `FW-02: frameworks/02-itty-router-unjs-nitro-stack` (`itty-router` + UnJS) | **`15.817 ms`** (`15.04 ms` Min) | **`16.525 ms`** | **`8.456 ms`** | `15.193 ms` (`4.1%`) | `19.248 ms` | `45.549 ms` | `85.485 ms` | **`1.22x Bun` (`2.28x` Warm) / `5.40x Node`** |
| **FW-03** | `FW-03: frameworks/03-node-http-express-compat` (`node:http` + `node:stream` Express/Polka) | **`13.806 ms`** (`13.10 ms` Min) | **`14.398 ms`** | **`0.856 ms`** | `13.516 ms` (`2.1%`) | `34.284 ms` | `50.708 ms` | `85.303 ms` | **`2.48x Bun` (`40.05x` Warm) / `6.18x Node`** |
| **FW-04** | `FW-04: frameworks/04-sveltekit-edge-ssr` (`@sveltejs/kit@3.1.1` + `svelte@5.57.3` + `devalue@6.0.3`) | **`29.551 ms`** (`27.99 ms` Min) | **`30.058 ms`** | **`13.231 ms`** | `27.950 ms` (`5.7%`) | `44.283 ms` | `68.127 ms` | `110.048 ms` | **`1.50x Bun` (`3.35x` Warm) / `3.72x Node`** |
| **CS-01** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`27.058 ms`** (`25.96 ms` Min) | **`28.050 ms`** | **`11.846 ms`** | `26.251 ms` (`3.1%`) | `28.216 ms` | `71.471 ms` | `88.080 ms` | **`1.04x Bun` (`2.38x` Warm) / `3.26x Node`** |
| **CS-02** | `CS-02: case_studies/02-mcp-code-mode-orchestrator` (`-98.4%` Tokens) | **`15.358 ms`** (`5.50 ms` Min) | **`16.181 ms`** | **`1.987 ms`** | `15.304 ms` (`0.4%`) | `16.319 ms` | `43.940 ms` | `72.287 ms` | **`1.06x Bun` (`8.21x` Warm) / `4.71x Node`** |
| **CS-03** | `CS-03: case_studies/03-multi-tenant-saas-plugin-mesh` (12 Tenants + Hot-Swap) | **`11.933 ms`** (`11.33 ms` Min) | **`12.600 ms`** | **`4.246 ms`** | `11.970 ms` (`-0.3%`) | `16.195 ms` | `47.223 ms` | `85.094 ms` | **`1.36x Bun` (`3.81x` Warm) / `7.13x Node`** |
| **CS-04** | `CS-04: case_studies/04-wasm-zero-trust-cdr-pipeline` (Wasm SimHash CDR) | **`13.687 ms`** (`12.95 ms` Min) | **`14.353 ms`** | **`1.754 ms`** | `13.688 ms` (`-0.0%`) | `19.040 ms` | `43.086 ms` | `65.260 ms` | **`1.39x Bun` (`10.86x` Warm) / `4.77x Node`** |
| **CS-05** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`8.811 ms`** (`7.99 ms` Min) | **`11.545 ms`** | **`2.372 ms`** | `8.524 ms` (`3.4%`) | `20.305 ms` | `62.154 ms` | `81.506 ms` | **`2.30x Bun` (`8.56x` Warm) / `9.25x Node`** |

---

## 4. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Workload | Slice (`SLICEBC\x01` Default) | Bun (`v1.4.4`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha.0.0.0`) | Slice Memory Advantage |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **App 2: NPM semver+ms+ini** | **13,696 KB (13.38 MB)** | 29,952 KB (29.25 MB) | 47,164 KB (46.06 MB) | 71,452 KB (69.78 MB) | **5.22x less than Node / 2.19x less than Bun** |
| **App 6: Telemetry + Flatted** | **14,080 KB (13.75 MB)** | 30,848 KB (30.13 MB) | 50,616 KB (49.43 MB) | 72,988 KB (71.28 MB) | **5.18x less than Node / 2.19x less than Bun** |
| **App 21: RFC 6455 WS + Sec** | **14,464 KB (14.13 MB)** | 32,084 KB (31.33 MB) | 51,012 KB (49.82 MB) | 95,748 KB (93.50 MB) | **6.62x less than Node / 2.22x less than Bun** |
| **App 22: Workers + Sockets** | **22,016 KB (21.50 MB)** | 41,400 KB (40.43 MB) | 63,976 KB (62.48 MB) | 92,848 KB (90.67 MB) | **4.22x less than Node / 1.88x less than Bun** |
| **App 23: Pure-Rust WASM** | **18,560 KB (18.13 MB)** | 30,592 KB (29.88 MB) | 49,296 KB (48.14 MB) | 80,916 KB (79.02 MB) | **4.36x less than Node / 1.65x less than Bun** |
| **FW 1: Hono v4 Microservice** | **15,488 KB (15.13 MB)** | 30,672 KB (29.95 MB) | 50,164 KB (48.99 MB) | 89,112 KB (87.02 MB) | **5.75x less than Node / 1.98x less than Bun** |
| **FW 2: itty-router + UnJS** | **14,464 KB (14.13 MB)** | 32,128 KB (31.38 MB) | 48,748 KB (47.61 MB) | 86,916 KB (84.88 MB) | **6.01x less than Node / 2.22x less than Bun** |
| **App 24: WASM + SSR** | **14,336 KB (14.00 MB)** | 32,256 KB (31.50 MB) | 48,904 KB (47.76 MB) | 80,896 KB (79.00 MB) | **5.64x less than Node / 2.25x less than Bun** |
| **FW 3: Express node:http** | **15,232 KB (14.88 MB)** | 33,280 KB (32.50 MB) | 47,316 KB (46.21 MB) | 80,268 KB (78.39 MB) | **5.27x less than Node / 2.18x less than Bun** |
| **01-tot-autonomous-sre-agent** | **20,608 KB (20.13 MB)** | 31,744 KB (31.00 MB) | 53,104 KB (51.86 MB) | 85,640 KB (83.63 MB) | **4.16x less than Node / 1.54x less than Bun** |
| **02-mcp-code-mode-orchestrator** | **17,632 KB (17.22 MB)** | 27,776 KB (27.13 MB) | 46,996 KB (45.89 MB) | 82,948 KB (81.00 MB) | **4.70x less than Node / 1.58x less than Bun** |
| **03-multi-tenant-saas-plugin-mesh** | **14,720 KB (14.38 MB)** | 26,468 KB (25.85 MB) | 49,992 KB (48.82 MB) | 87,432 KB (85.38 MB) | **5.94x less than Node / 1.80x less than Bun** |
| **04-wasm-zero-trust-cdr-pipeline** | **18,816 KB (18.38 MB)** | 31,360 KB (30.63 MB) | 49,576 KB (48.41 MB) | 80,392 KB (78.51 MB) | **4.27x less than Node / 1.67x less than Bun** |
| **05-time-travel-chaos-fuzzer-payments** | **13,952 KB (13.63 MB)** | 29,312 KB (28.63 MB) | 49,400 KB (48.24 MB) | 81,696 KB (79.78 MB) | **5.86x less than Node / 2.10x less than Bun** |
| **FW 4: SvelteKit 3 + Svelte 5** | **22,104 KB (21.59 MB)** | 41,984 KB (41.00 MB) | 56,624 KB (55.30 MB) | 85,540 KB (83.54 MB) | **3.87x less than Node / 1.90x less than Bun** |

---

## 5. Multi-Tenant Edge KV, AI Agent Checkpoints & 5-Ring Security Telemetry

| Subsystem / Benchmark | Measured Telemetry | Status |
| :--- | :--- | :---: |
| **Sharded `Slice.kv` (`1,001` Get/Set/Incr Ops in Isolate)** | **`643.16 us`** total (`--warm`, `93 B` memory cage) | `PASS` |
| **Multi-Tenant `--tenant-dir` Zero-Downtime Hot-Swap** | Live atomic replacement of `tenant-alpha.cas` (`v1 -> v2-hot-swapped`) with isolated `Slice.kv` state | `PASS` |
| **Deterministic Time-Travel Replay (`--deterministic=20261019`)** | 100% bit-for-bit identical PRNG (`Math.random`) & virtual clock (`Date.now`, `performance.now`) | `PASS` |
| **Tree-of-Thought AI Agent (`Slice.checkpoint` + 3 `evalSandbox` + `restore`)** | **`74.91 us`** total (`4.13 us` child sandbox) | `PASS` |
| **Cloud Metadata SSRF (`169.254.169.254`) & Base64 Egress DLP Guard** | `SSRF_METADATA_BLOCKED` & `EGRESS_SECRET_LEAK_BLOCKED` deterministically trapped | `PASS` |
| **Built-In Model Context Protocol Server (`slice mcp --smoke-test`)** | JSON-RPC 2.0 `initialize`, `tools/list`, `tools/call` verified over `stdio` | `PASS` |
