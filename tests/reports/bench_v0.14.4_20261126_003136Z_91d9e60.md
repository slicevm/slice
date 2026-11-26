# Slice (Nexus-JS) - Full 33-Workload & MicroVM Benchmark Report

* **Slice Version:** `v0.14.4` (Commit `91d9e60`)
* **Generated Timestamp (UTC):** `2026-11-26T00:31:36Z`
* **Archive File:** [`tests/reports/bench_v0.14.4_20261126_003136Z_91d9e60.md`](./bench_v0.14.4_20261126_003136Z_91d9e60.md)
* **Canonical Latest Report:** [`tests/reports/latest.md`](./latest.md)
* **Environment:** `Linux x86_64 Docker (io_uring SQPOLL + WRPKRU)`
* **Compared Runtimes:**
  * **Slice:** `v0.14.4` (Default `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS + Monomorphic Inline Caching `get_fast` + Iterative ECMA-404 `JSON.parse`)
  * **Bun:** `v1.4.5` (JavaScriptCore)
  * **Deno:** `v2.9.8` (V8)
  * **Node.js:** `v27.0.0-alpha.0.1.1` (V8 native TypeScript execution)

---

## 1. Executive Summary

Across all **33 real-world multi-file TypeScript applications, framework suites, and case studies**, Slice beats **Deno (`v2.9.8`)** on **33/33** workloads, beats **Node.js (`v27.0.0-alpha.0.1.1`)** on **33/33** workloads, beats **Bun (`v1.4.5`)** on **33/33** workloads in cold one-shot execution, and beats **all 3 runtimes on 33/33 workloads in `--warm` pooled isolate execution**.

| Metric | Linux (`x86_64` Docker) | Windows (`x86_64` Host) | Comparison vs Runtimes & MicroVMs |
| :--- | :---: | :---: | :--- |
| **Single Isolate Cold-Start (Min / P50 / P99)** | **`0.78 us` / `0.86 us` / `1.21 us`** | **`0.80 us` / `0.90 us` / `3.40 us`** | **~120,000x faster than 125ms Firecracker VM** |
| **1,000-Tenant MicroVM Swarm (Boot + Exec)** | **`1.10 ms`** (`8.12 MB` RSS) | **`1.28 ms`** (`6.71 MB` RSS) | **> 10,000x faster than container/VM swarms** |
| **500-Module Monorepo Cold Boot (`0 stat()`)** | **`3.067 ms`** (`119.81 us` in-mem) | **`9.107 ms`** (`174.20 us` in-mem) | **22.9x faster than Node.js (`1,500+ stat()`)** |
| **10,000 Direct Kernel Ring I/O Completions** | **`0.01 ms`** | **`2.80 ms`** | **Zero worker-thread lock contention** |
| **5-Ring Security Containment Verification** | **`5 / 5 PASS`** | **`5 / 5 PASS`** | **OCap + SES + Fuel + SSRF/DLP + MPK** |

---

## 2. `slice serve` Multi-Tenant HTTP/1.1 Keep-Alive Throughput (`16,000` Reqs, 8 Sockets x 8 Pipeline)

| Server / Runtime | Throughput (`req/s`) | Avg Latency / Req | Total Time (`16k` Reqs) | Relative Speedup vs Runtimes |
| :--- | :---: | :---: | :---: | :--- |
| **`slice serve` (Tier-0 CAS, `--workers=8`)** | **`158,118 req/s`** | **`6.32 us/req`** | **`101.2 ms`** | **`1.58x` faster than `Bun.serve` / `2.89x` Node.js** |
| **`slice serve` (Tier-1 WinterTC, `--workers=8`)** | **`153,531 req/s`** | **`6.51 us/req`** | **`104.2 ms`** | **`1.54x` faster than `Bun.serve` / `2.80x` Node.js** |
| **`Deno.serve` (`v2.9.8`, V8, No Per-Req Sandbox)** | `95,072 req/s` | `10.52 us/req` | `168.3 ms` | `0.95x` vs Bun |
| **`Bun.serve` (`v1.4.5`, JSC, No Per-Req Sandbox)** | `99,946 req/s` | `10.01 us/req` | `160.1 ms` | `1.00x` (Baseline) |
| **`Node.js http.createServer` (`v27.0.0-alpha.0.1.1`, V8)** | `54,802 req/s` | `18.25 us/req` | `292.0 ms` | `0.55x` vs Bun |

---

## 3. Full 33-Workload Latency Comparison (20 Runs Each, Linux `x86_64` Docker)

| # | Workload (`tests/apps/`, `tests/frameworks/`, `tests/case_studies/`) | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm (`--warm`) | Previous `.cas` (`Delta`) | Bun `v1.4.5` | Deno `v2.9.8` | Node.js `v27.0.0-alpha.0.1.1` | Slice Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`5.217 ms`** (`4.85 ms` Min) | **`5.826 ms`** | **`0.924 ms`** | `5.118 ms` (`1.9%`) | `14.782 ms` | `36.479 ms` | `54.459 ms` | **`2.83x Bun` (`16.00x` Warm) / `10.44x Node`** |
| **02** | `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | **`11.686 ms`** (`11.07 ms` Min) | **`12.387 ms`** | **`3.619 ms`** | `11.667 ms` (`0.2%`) | `23.339 ms` | `52.934 ms` | `65.306 ms` | **`2.00x Bun` (`6.45x` Warm) / `5.59x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`6.183 ms`** (`5.66 ms` Min) | **`6.525 ms`** | **`1.147 ms`** | `6.059 ms` (`2.0%`) | `16.327 ms` | `43.930 ms` | `55.841 ms` | **`2.64x Bun` (`14.23x` Warm) / `9.03x Node`** |
| **04** | `04-edge-api-gateway` (`fast-querystring` + `ms`) | **`6.534 ms`** (`5.70 ms` Min) | **`6.770 ms`** | **`1.338 ms`** | `6.398 ms` (`2.1%`) | `16.022 ms` | `42.597 ms` | `63.844 ms` | **`2.45x Bun` (`11.97x` Warm) / `9.77x Node`** |
| **05** | `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | **`8.725 ms`** (`8.32 ms` Min) | **`9.347 ms`** | **`4.254 ms`** | `8.601 ms` (`1.4%`) | `16.115 ms` | `45.432 ms` | `57.966 ms` | **`1.85x Bun` (`3.79x` Warm) / `6.64x Node`** |
| **06** | `06-telemetry-metrics-pipeline` (`bytes` + `flatted` + WAL) | **`11.006 ms`** (`10.40 ms` Min) | **`12.248 ms`** | **`5.079 ms`** | `10.946 ms` (`0.5%`) | `19.442 ms` | `47.711 ms` | `59.805 ms` | **`1.77x Bun` (`3.83x` Warm) / `5.43x Node`** |
| **07** | `07-graphql-ast-validator` (`dlv` + `dset` + AST Cost) | **`7.533 ms`** (`6.95 ms` Min) | **`8.381 ms`** | **`3.585 ms`** | `7.741 ms` (`-2.7%`) | `14.445 ms` | `41.629 ms` | `56.404 ms` | **`1.92x Bun` (`4.03x` Warm) / `7.49x Node`** |
| **08** | `08-event-driven-state-journal` (`mitt` + `ms` + JSONL Replay) | **`6.169 ms`** (`5.53 ms` Min) | **`6.769 ms`** | **`1.168 ms`** | `6.225 ms` (`-0.9%`) | `17.254 ms` | `46.514 ms` | `58.953 ms` | **`2.80x Bun` (`14.77x` Warm) / `9.56x Node`** |
| **09** | `09-wintertc-web-router` (`Request` + `Response` + `URL`, 80 reqs) | **`13.614 ms`** (`12.60 ms` Min) | **`13.807 ms`** | **`7.588 ms`** | `12.646 ms` (`7.7%`) | `15.387 ms` | `49.047 ms` | `82.811 ms` | **`1.13x Bun` (`2.03x` Warm) / `6.08x Node`** |
| **10** | `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **`10.123 ms`** (`9.55 ms` Min) | **`10.848 ms`** | **`4.031 ms`** | `10.563 ms` (`-4.2%`) | `15.744 ms` | `47.901 ms` | `81.751 ms` | **`1.56x Bun` (`3.91x` Warm) / `8.08x Node`** |
| **11** | `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **`8.847 ms`** (`8.14 ms` Min) | **`9.606 ms`** | **`3.072 ms`** | `8.717 ms` (`1.5%`) | `14.745 ms` | `49.738 ms` | `82.470 ms` | **`1.67x Bun` (`4.80x` Warm) / `9.32x Node`** |
| **12** | `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP, 320 frames) | **`13.656 ms`** (`12.51 ms` Min) | **`13.809 ms`** | **`6.763 ms`** | `13.181 ms` (`3.6%`) | `16.615 ms` | `50.726 ms` | `86.540 ms` | **`1.22x Bun` (`2.46x` Warm) / `6.34x Node`** |
| **13** | `13-webhook-orchestrator-ssrf` (`fast-querystring` + `ms` + Live `fetch`) | **`10.186 ms`** (`9.50 ms` Min) | **`10.814 ms`** | **`3.824 ms`** | `9.999 ms` (`1.9%`) | `20.557 ms` | `55.106 ms` | `107.148 ms` | **`2.02x Bun` (`5.38x` Warm) / `10.52x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`12.991 ms`** (`12.09 ms` Min) | **`13.199 ms`** | **`7.339 ms`** | `13.592 ms` (`-4.4%`) | `22.374 ms` | `53.565 ms` | `109.004 ms` | **`1.72x Bun` (`3.05x` Warm) / `8.39x Node`** |
| **15** | `15-time-travel-test-suite` (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **`12.809 ms`** (`11.73 ms` Min) | **`14.770 ms`** | **`3.728 ms`** | `11.587 ms` (`10.5%`) | `26.521 ms` | `70.688 ms` | `82.059 ms` | **`2.07x Bun` (`7.11x` Warm) / `6.41x Node`** |
| **16** | `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **`6.647 ms`** (`6.14 ms` Min) | **`7.108 ms`** | **`1.246 ms`** | `6.360 ms` (`4.5%`) | `16.465 ms` | `45.329 ms` | `59.785 ms` | **`2.48x Bun` (`13.21x` Warm) / `8.99x Node`** |
| **17** | `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive/307/SSRF/DLP) | **`12.831 ms`** (`12.16 ms` Min) | **`13.746 ms`** | **`5.803 ms`** | `13.222 ms` (`-3.0%`) | `22.701 ms` | `55.748 ms` | `110.498 ms` | **`1.77x Bun` (`3.91x` Warm) / `8.61x Node`** |
| **18** | `18-async-event-loop-fanout-scheduler` (Timer Wheel/`Promise.all`/`AbortSignal`) | **`29.449 ms`** (`27.79 ms` Min) | **`30.396 ms`** | **`17.288 ms`** | `28.018 ms` (`5.1%`) | `35.289 ms` | `75.668 ms` | `76.208 ms` | **`1.20x Bun` (`2.04x` Warm) / `2.59x Node`** |
| **19** | `19-wintertc-webcrypto-streams-edge` (WebCrypto/Streams/FormData/Clone) | **`8.471 ms`** (`7.93 ms` Min) | **`9.378 ms`** | **`1.438 ms`** | `8.558 ms` (`-1.0%`) | `14.755 ms` | `47.589 ms` | `83.194 ms` | **`1.74x Bun` (`10.26x` Warm) / `9.82x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`9.053 ms`** (`8.51 ms` Min) | **`9.681 ms`** | **`0.431 ms`** | `9.025 ms` (`0.3%`) | `23.333 ms` | `54.459 ms` | `67.838 ms` | **`2.58x Bun` (`54.14x` Warm) / `7.49x Node`** |
| **21** | `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket` + 5-Ring Guard) | **`9.802 ms`** (`9.13 ms` Min) | **`10.565 ms`** | **`2.165 ms`** | `9.141 ms` (`7.2%`) | `21.434 ms` | `52.085 ms` | `113.405 ms` | **`2.19x Bun` (`9.90x` Warm) / `11.57x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`28.539 ms`** (`27.18 ms` Min) | **`29.059 ms`** | **`2.537 ms`** | `28.718 ms` (`-0.6%`) | `60.443 ms` | `119.867 ms` | `95.207 ms` | **`2.12x Bun` (`23.82x` Warm) / `3.34x Node`** |
| **23** | `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **`15.050 ms`** (`12.98 ms` Min) | **`13.662 ms`** | **`1.466 ms`** | `13.047 ms` (`15.4%`) | `24.197 ms` | `40.244 ms` | `58.995 ms` | **`1.77x Bun` (`16.51x` Warm) / `4.32x Node`** |
| **24** | `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` SLA Scorer + SSR Pipeline) | **`7.127 ms`** (`6.67 ms` Min) | **`7.451 ms`** | **`1.394 ms`** | `7.311 ms` (`-2.5%`) | `20.611 ms` | `42.682 ms` | `60.752 ms` | **`2.89x Bun` (`14.79x` Warm) / `8.52x Node`** |
| **FW-01** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`13.081 ms`** (`12.48 ms` Min) | **`14.019 ms`** | **`5.600 ms`** | `13.621 ms` (`-4.0%`) | `20.851 ms` | `54.130 ms` | `93.260 ms` | **`1.59x Bun` (`3.72x` Warm) / `7.13x Node`** |
| **FW-02** | `FW-02: frameworks/02-itty-router-unjs-nitro-stack` (`itty-router` + UnJS) | **`16.392 ms`** (`15.47 ms` Min) | **`17.352 ms`** | **`8.949 ms`** | `15.677 ms` (`4.6%`) | `19.955 ms` | `47.915 ms` | `82.028 ms` | **`1.22x Bun` (`2.23x` Warm) / `5.00x Node`** |
| **FW-03** | `FW-03: frameworks/03-node-http-express-compat` (`node:http` + `node:stream` Express/Polka) | **`14.261 ms`** (`13.42 ms` Min) | **`14.489 ms`** | **`0.899 ms`** | `13.816 ms` (`3.2%`) | `35.867 ms` | `60.555 ms` | `80.153 ms` | **`2.52x Bun` (`39.90x` Warm) / `5.62x Node`** |
| **FW-04** | `FW-04: frameworks/04-sveltekit-edge-ssr` (`@sveltejs/kit@3.1.1` + `svelte@5.57.3` + `devalue@6.0.3`) | **`28.901 ms`** (`5.62 ms` Min) | **`30.822 ms`** | **`13.599 ms`** | `28.510 ms` (`1.4%`) | `45.676 ms` | `69.871 ms` | `107.648 ms` | **`1.58x Bun` (`3.36x` Warm) / `3.72x Node`** |
| **CS-01** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`26.779 ms`** (`3.70 ms` Min) | **`30.040 ms`** | **`12.037 ms`** | `27.549 ms` (`-2.8%`) | `29.766 ms` | `73.200 ms` | `83.478 ms` | **`1.11x Bun` (`2.47x` Warm) / `3.12x Node`** |
| **CS-02** | `CS-02: case_studies/02-mcp-code-mode-orchestrator` (`-98.4%` Tokens) | **`15.910 ms`** (`14.79 ms` Min) | **`16.190 ms`** | **`2.691 ms`** | `16.641 ms` (`-4.4%`) | `17.634 ms` | `46.078 ms` | `63.410 ms` | **`1.11x Bun` (`6.55x` Warm) / `3.99x Node`** |
| **CS-03** | `CS-03: case_studies/03-multi-tenant-saas-plugin-mesh` (12 Tenants + Hot-Swap) | **`12.858 ms`** (`11.66 ms` Min) | **`12.866 ms`** | **`4.433 ms`** | `12.254 ms` (`4.9%`) | `17.434 ms` | `48.581 ms` | `79.804 ms` | **`1.36x Bun` (`3.93x` Warm) / `6.21x Node`** |
| **CS-04** | `CS-04: case_studies/04-wasm-zero-trust-cdr-pipeline` (Wasm SimHash CDR) | **`13.843 ms`** (`13.46 ms` Min) | **`14.894 ms`** | **`1.830 ms`** | `14.355 ms` (`-3.6%`) | `20.253 ms` | `45.565 ms` | `61.222 ms` | **`1.46x Bun` (`11.07x` Warm) / `4.42x Node`** |
| **CS-05** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`9.252 ms`** (`8.57 ms` Min) | **`9.542 ms`** | **`2.794 ms`** | `8.774 ms` (`5.4%`) | `22.530 ms` | `64.576 ms` | `79.556 ms` | **`2.44x Bun` (`8.06x` Warm) / `8.60x Node`** |

---

## 4. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Workload | Slice (`SLICEBC\x01` Default) | Bun (`v1.4.5`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha.0.1.1`) | Slice Memory Advantage |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **App 2: NPM semver+ms+ini** | **13,952 KB (13.63 MB)** | 38,528 KB (37.63 MB) | 47,460 KB (46.35 MB) | 70,372 KB (68.72 MB) | **5.04x less than Node / 2.76x less than Bun** |
| **App 6: Telemetry + Flatted** | **14,336 KB (14.00 MB)** | 39,936 KB (39.00 MB) | 50,804 KB (49.61 MB) | 71,652 KB (69.97 MB) | **5.00x less than Node / 2.79x less than Bun** |
| **App 21: RFC 6455 WS + Sec** | **14,720 KB (14.38 MB)** | 40,052 KB (39.11 MB) | 50,832 KB (49.64 MB) | 93,544 KB (91.35 MB) | **6.35x less than Node / 2.72x less than Bun** |
| **App 22: Workers + Sockets** | **22,144 KB (21.63 MB)** | 46,936 KB (45.84 MB) | 63,616 KB (62.13 MB) | 90,860 KB (88.73 MB) | **4.10x less than Node / 2.12x less than Bun** |
| **App 23: Pure-Rust WASM** | **18,560 KB (18.13 MB)** | 37,760 KB (36.88 MB) | 48,928 KB (47.78 MB) | 76,740 KB (74.94 MB) | **4.13x less than Node / 2.03x less than Bun** |
| **FW 1: Hono v4 Microservice** | **16,000 KB (15.63 MB)** | 38,352 KB (37.45 MB) | 49,780 KB (48.61 MB) | 86,124 KB (84.11 MB) | **5.38x less than Node / 2.40x less than Bun** |
| **FW 2: itty-router + UnJS** | **15,104 KB (14.75 MB)** | 41,088 KB (40.13 MB) | 48,492 KB (47.36 MB) | 86,248 KB (84.23 MB) | **5.71x less than Node / 2.72x less than Bun** |
| **App 24: WASM + SSR** | **14,464 KB (14.13 MB)** | 40,448 KB (39.50 MB) | 48,720 KB (47.58 MB) | 78,532 KB (76.69 MB) | **5.43x less than Node / 2.80x less than Bun** |
| **FW 3: Express node:http** | **15,488 KB (15.13 MB)** | 41,472 KB (40.50 MB) | 47,464 KB (46.35 MB) | 79,696 KB (77.83 MB) | **5.15x less than Node / 2.68x less than Bun** |
| **01-tot-autonomous-sre-agent** | **20,352 KB (19.88 MB)** | 40,576 KB (39.63 MB) | 52,976 KB (51.73 MB) | 80,232 KB (78.35 MB) | **3.94x less than Node / 1.99x less than Bun** |
| **02-mcp-code-mode-orchestrator** | **17,380 KB (16.97 MB)** | 37,476 KB (36.60 MB) | 47,276 KB (46.17 MB) | 78,184 KB (76.35 MB) | **4.50x less than Node / 2.16x less than Bun** |
| **03-multi-tenant-saas-plugin-mesh** | **14,848 KB (14.50 MB)** | 35,712 KB (34.88 MB) | 49,656 KB (48.49 MB) | 87,116 KB (85.07 MB) | **5.87x less than Node / 2.41x less than Bun** |
| **04-wasm-zero-trust-cdr-pipeline** | **19,072 KB (18.63 MB)** | 39,168 KB (38.25 MB) | 49,696 KB (48.53 MB) | 78,188 KB (76.36 MB) | **4.10x less than Node / 2.05x less than Bun** |
| **05-time-travel-chaos-fuzzer-payments** | **14,336 KB (14.00 MB)** | 38,784 KB (37.88 MB) | 49,012 KB (47.86 MB) | 79,336 KB (77.48 MB) | **5.53x less than Node / 2.71x less than Bun** |
| **FW 4: SvelteKit 3 + Svelte 5** | **22,372 KB (21.85 MB)** | 50,816 KB (49.63 MB) | 56,556 KB (55.23 MB) | 85,100 KB (83.11 MB) | **3.80x less than Node / 2.27x less than Bun** |

---

## 5. Multi-Tenant Edge KV, AI Agent Checkpoints & 5-Ring Security Telemetry

| Subsystem / Benchmark | Measured Telemetry | Status |
| :--- | :--- | :---: |
| **Sharded `Slice.kv` (`1,001` Get/Set/Incr Ops in Isolate)** | **`561.01 us`** total (`--warm`, `288 B` memory cage) | `PASS` |
| **Multi-Tenant `--tenant-dir` Zero-Downtime Hot-Swap** | Live atomic replacement of `tenant-alpha.cas` (`v1 -> v2-hot-swapped`) with isolated `Slice.kv` state | `PASS` |
| **Deterministic Time-Travel Replay (`--deterministic=20261019`)** | 100% bit-for-bit identical PRNG (`Math.random`) & virtual clock (`Date.now`, `performance.now`) | `PASS` |
| **Tree-of-Thought AI Agent (`Slice.checkpoint` + 3 `evalSandbox` + `restore`)** | **`69.09 us`** total (`2.43 us` child sandbox) | `PASS` |
| **Cloud Metadata SSRF (`169.254.169.254`) & Base64 Egress DLP Guard** | `SSRF_METADATA_BLOCKED` & `EGRESS_SECRET_LEAK_BLOCKED` deterministically trapped | `PASS` |
| **Built-In Model Context Protocol Server (`slice mcp --smoke-test`)** | JSON-RPC 2.0 `initialize`, `tools/list`, `tools/call` verified over `stdio` | `PASS` |
