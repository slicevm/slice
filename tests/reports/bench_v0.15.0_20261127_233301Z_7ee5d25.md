# Slice (Nexus-JS) - Full 33-Workload & MicroVM Benchmark Report

* **Slice Version:** `v0.15.0` (Commit `7ee5d25`)
* **Generated Timestamp (UTC):** `2026-11-27T23:33:01Z`
* **Archive File:** [`tests/reports/bench_v0.15.0_20261127_233301Z_7ee5d25.md`](./bench_v0.15.0_20261127_233301Z_7ee5d25.md)
* **Canonical Latest Report:** [`tests/reports/latest.md`](./latest.md)
* **Environment:** `Linux x86_64 Docker (io_uring SQPOLL + WRPKRU)`
* **Compared Runtimes:**
  * **Slice:** `v0.15.0` (Default `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS + Monomorphic Inline Caching `get_fast` + Iterative ECMA-404 `JSON.parse`)
  * **Bun:** `v1.4.5` (JavaScriptCore)
  * **Deno:** `v2.9.8` (V8)
  * **Node.js:** `v27.0.0-alpha.0.1.1` (V8 native TypeScript execution)

---

## 1. Executive Summary

Across all **33 real-world multi-file TypeScript applications, framework suites, and case studies**, Slice beats **Deno (`v2.9.8`)** on **33/33** workloads, beats **Node.js (`v27.0.0-alpha.0.1.1`)** on **33/33** workloads, beats **Bun (`v1.4.5`)** on **33/33** workloads in cold one-shot execution, and beats **all 3 runtimes on 33/33 workloads in `--warm` pooled isolate execution**.

| Metric | Linux (`x86_64` Docker) | Windows (`x86_64` Host) | Comparison vs Runtimes & MicroVMs |
| :--- | :---: | :---: | :--- |
| **Single Isolate Cold-Start (Min / P50 / P99)** | **`0.80 us` / `0.85 us` / `1.49 us`** | **`0.80 us` / `0.90 us` / `1.00 us`** | **~120,000x faster than 125ms Firecracker VM** |
| **1,000-Tenant MicroVM Swarm (Boot + Exec)** | **`1.03 ms`** (`8.62 MB` RSS) | **`1.14 ms`** (`6.79 MB` RSS) | **> 10,000x faster than container/VM swarms** |
| **500-Module Monorepo Cold Boot (`0 stat()`)** | **`3.154 ms`** (`112.28 us` in-mem) | **`8.472 ms`** (`182.10 us` in-mem) | **22.3x faster than Node.js (`1,500+ stat()`)** |
| **10,000 Direct Kernel Ring I/O Completions** | **`0.01 ms`** | **`3.24 ms`** | **Zero worker-thread lock contention** |
| **5-Ring Security Containment Verification** | **`5 / 5 PASS`** | **`5 / 5 PASS`** | **OCap + SES + Fuel + SSRF/DLP + MPK** |

---

## 2. `slice serve` Multi-Tenant HTTP/1.1 Keep-Alive Throughput (`16,000` Reqs, 8 Sockets x 8 Pipeline)

| Server / Runtime | Throughput (`req/s`) | Avg Latency / Req | Total Time (`16k` Reqs) | Relative Speedup vs Runtimes |
| :--- | :---: | :---: | :---: | :--- |
| **`slice serve` (Tier-0 CAS, `--workers=8`)** | **`159,746 req/s`** | **`6.26 us/req`** | **`100.2 ms`** | **`1.64x` faster than `Bun.serve` / `3.24x` Node.js** |
| **`slice serve` (Tier-1 WinterTC, `--workers=8`)** | **`140,405 req/s`** | **`7.12 us/req`** | **`114.0 ms`** | **`1.44x` faster than `Bun.serve` / `2.84x` Node.js** |
| **`Deno.serve` (`v2.9.8`, V8, No Per-Req Sandbox)** | `98,900 req/s` | `10.11 us/req` | `161.8 ms` | `1.01x` vs Bun |
| **`Bun.serve` (`v1.4.5`, JSC, No Per-Req Sandbox)** | `97,553 req/s` | `10.25 us/req` | `164.0 ms` | `1.00x` (Baseline) |
| **`Node.js http.createServer` (`v27.0.0-alpha.0.1.1`, V8)** | `49,367 req/s` | `20.26 us/req` | `324.1 ms` | `0.51x` vs Bun |

---

## 3. Full 33-Workload Latency Comparison (20 Runs Each, Linux `x86_64` Docker)

| # | Workload (`tests/apps/`, `tests/frameworks/`, `tests/case_studies/`) | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm (`--warm`) | Previous `.cas` (`Delta`) | Bun `v1.4.5` | Deno `v2.9.8` | Node.js `v27.0.0-alpha.0.1.1` | Slice Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`5.588 ms`** (`5.00 ms` Min) | **`5.696 ms`** | **`0.861 ms`** | `5.234 ms` (`6.8%`) | `14.730 ms` | `37.379 ms` | `57.370 ms` | **`2.64x Bun` (`17.11x` Warm) / `10.27x Node`** |
| **02** | `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | **`12.055 ms`** (`10.95 ms` Min) | **`12.118 ms`** | **`3.505 ms`** | `11.456 ms` (`5.2%`) | `22.986 ms` | `53.296 ms` | `65.331 ms` | **`1.91x Bun` (`6.56x` Warm) / `5.42x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`6.168 ms`** (`5.67 ms` Min) | **`6.780 ms`** | **`1.104 ms`** | `6.174 ms` (`-0.1%`) | `15.215 ms` | `44.596 ms` | `57.770 ms` | **`2.47x Bun` (`13.78x` Warm) / `9.37x Node`** |
| **04** | `04-edge-api-gateway` (`fast-querystring` + `ms`) | **`6.654 ms`** (`6.02 ms` Min) | **`7.525 ms`** | **`1.359 ms`** | `6.435 ms` (`3.4%`) | `17.345 ms` | `46.228 ms` | `59.137 ms` | **`2.61x Bun` (`12.76x` Warm) / `8.89x Node`** |
| **05** | `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | **`8.960 ms`** (`8.18 ms` Min) | **`9.452 ms`** | **`4.509 ms`** | `9.585 ms` (`-6.5%`) | `17.082 ms` | `45.106 ms` | `59.342 ms` | **`1.91x Bun` (`3.79x` Warm) / `6.62x Node`** |
| **06** | `06-telemetry-metrics-pipeline` (`bytes` + `flatted` + WAL) | **`11.272 ms`** (`10.14 ms` Min) | **`12.430 ms`** | **`4.318 ms`** | `10.744 ms` (`4.9%`) | `18.799 ms` | `49.780 ms` | `61.864 ms` | **`1.67x Bun` (`4.35x` Warm) / `5.49x Node`** |
| **07** | `07-graphql-ast-validator` (`dlv` + `dset` + AST Cost) | **`8.325 ms`** (`7.58 ms` Min) | **`8.474 ms`** | **`3.667 ms`** | `7.961 ms` (`4.6%`) | `15.741 ms` | `41.884 ms` | `57.496 ms` | **`1.89x Bun` (`4.29x` Warm) / `6.91x Node`** |
| **08** | `08-event-driven-state-journal` (`mitt` + `ms` + JSONL Replay) | **`6.503 ms`** (`6.00 ms` Min) | **`7.329 ms`** | **`1.224 ms`** | `6.366 ms` (`2.2%`) | `17.391 ms` | `47.328 ms` | `57.729 ms` | **`2.67x Bun` (`14.21x` Warm) / `8.88x Node`** |
| **09** | `09-wintertc-web-router` (`Request` + `Response` + `URL`, 80 reqs) | **`13.381 ms`** (`12.02 ms` Min) | **`13.745 ms`** | **`6.947 ms`** | `12.989 ms` (`3.0%`) | `15.059 ms` | `48.965 ms` | `82.789 ms` | **`1.13x Bun` (`2.17x` Warm) / `6.19x Node`** |
| **10** | `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **`10.268 ms`** (`9.65 ms` Min) | **`10.791 ms`** | **`3.827 ms`** | `10.224 ms` (`0.4%`) | `16.225 ms` | `49.254 ms` | `79.908 ms` | **`1.58x Bun` (`4.24x` Warm) / `7.78x Node`** |
| **11** | `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **`9.186 ms`** (`8.37 ms` Min) | **`9.555 ms`** | **`3.101 ms`** | `10.606 ms` (`-13.4%`) | `14.613 ms` | `50.919 ms` | `83.315 ms` | **`1.59x Bun` (`4.71x` Warm) / `9.07x Node`** |
| **12** | `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP, 320 frames) | **`13.683 ms`** (`12.99 ms` Min) | **`14.138 ms`** | **`6.744 ms`** | `13.776 ms` (`-0.7%`) | `16.930 ms` | `50.027 ms` | `85.496 ms` | **`1.24x Bun` (`2.51x` Warm) / `6.25x Node`** |
| **13** | `13-webhook-orchestrator-ssrf` (`fast-querystring` + `ms` + Live `fetch`) | **`10.327 ms`** (`9.77 ms` Min) | **`12.058 ms`** | **`3.403 ms`** | `10.616 ms` (`-2.7%`) | `20.778 ms` | `53.133 ms` | `107.621 ms` | **`2.01x Bun` (`6.11x` Warm) / `10.42x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`12.995 ms`** (`12.00 ms` Min) | **`13.992 ms`** | **`7.155 ms`** | `12.976 ms` (`0.1%`) | `24.708 ms` | `53.841 ms` | `105.459 ms` | **`1.90x Bun` (`3.45x` Warm) / `8.12x Node`** |
| **15** | `15-time-travel-test-suite` (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **`11.650 ms`** (`10.90 ms` Min) | **`12.301 ms`** | **`3.847 ms`** | `11.639 ms` (`0.1%`) | `26.846 ms` | `68.115 ms` | `80.995 ms` | **`2.30x Bun` (`6.98x` Warm) / `6.95x Node`** |
| **16** | `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **`6.900 ms`** (`6.14 ms` Min) | **`7.073 ms`** | **`1.260 ms`** | `6.491 ms` (`6.3%`) | `16.418 ms` | `48.539 ms` | `61.477 ms` | **`2.38x Bun` (`13.03x` Warm) / `8.91x Node`** |
| **17** | `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive/307/SSRF/DLP) | **`13.794 ms`** (`13.36 ms` Min) | **`14.331 ms`** | **`5.955 ms`** | `13.134 ms` (`5.0%`) | `24.800 ms` | `56.499 ms` | `108.310 ms` | **`1.80x Bun` (`4.16x` Warm) / `7.85x Node`** |
| **18** | `18-async-event-loop-fanout-scheduler` (Timer Wheel/`Promise.all`/`AbortSignal`) | **`28.513 ms`** (`27.34 ms` Min) | **`29.470 ms`** | **`17.640 ms`** | `28.536 ms` (`-0.1%`) | `35.451 ms` | `73.005 ms` | `75.458 ms` | **`1.24x Bun` (`2.01x` Warm) / `2.65x Node`** |
| **19** | `19-wintertc-webcrypto-streams-edge` (WebCrypto/Streams/FormData/Clone) | **`8.514 ms`** (`7.85 ms` Min) | **`8.948 ms`** | **`1.470 ms`** | `8.366 ms` (`1.8%`) | `14.703 ms` | `46.544 ms` | `83.776 ms` | **`1.73x Bun` (`10.00x` Warm) / `9.84x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`9.396 ms`** (`8.92 ms` Min) | **`9.909 ms`** | **`0.482 ms`** | `9.025 ms` (`4.1%`) | `24.258 ms` | `55.536 ms` | `68.572 ms` | **`2.58x Bun` (`50.33x` Warm) / `7.30x Node`** |
| **21** | `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket` + 5-Ring Guard) | **`9.527 ms`** (`8.83 ms` Min) | **`9.930 ms`** | **`2.136 ms`** | `9.398 ms` (`1.4%`) | `21.866 ms` | `52.730 ms` | `111.741 ms` | **`2.30x Bun` (`10.24x` Warm) / `11.73x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`29.817 ms`** (`28.07 ms` Min) | **`29.843 ms`** | **`2.593 ms`** | `28.728 ms` (`3.8%`) | `58.265 ms` | `116.338 ms` | `96.511 ms` | **`1.95x Bun` (`22.47x` Warm) / `3.24x Node`** |
| **23** | `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **`13.272 ms`** (`12.56 ms` Min) | **`14.256 ms`** | **`1.622 ms`** | `13.036 ms` (`1.8%`) | `23.196 ms` | `42.013 ms` | `59.859 ms` | **`1.75x Bun` (`14.30x` Warm) / `4.51x Node`** |
| **24** | `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` SLA Scorer + SSR Pipeline) | **`7.144 ms`** (`6.67 ms` Min) | **`7.523 ms`** | **`1.412 ms`** | `7.135 ms` (`0.1%`) | `20.667 ms` | `43.930 ms` | `63.529 ms` | **`2.89x Bun` (`14.64x` Warm) / `8.89x Node`** |
| **FW-01** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`17.976 ms`** (`16.68 ms` Min) | **`18.242 ms`** | **`5.293 ms`** | `17.441 ms` (`3.1%`) | `22.869 ms` | `57.191 ms` | `91.676 ms` | **`1.27x Bun` (`4.32x` Warm) / `5.10x Node`** |
| **FW-02** | `FW-02: frameworks/02-itty-router-unjs-nitro-stack` (`itty-router` + UnJS) | **`17.648 ms`** (`15.42 ms` Min) | **`18.369 ms`** | **`8.629 ms`** | `16.555 ms` (`6.6%`) | `20.469 ms` | `48.881 ms` | `83.240 ms` | **`1.16x Bun` (`2.37x` Warm) / `4.72x Node`** |
| **FW-03** | `FW-03: frameworks/03-node-http-express-compat` (`node:http` + `node:stream` Express/Polka) | **`14.308 ms`** (`13.68 ms` Min) | **`14.802 ms`** | **`0.882 ms`** | `13.902 ms` (`2.9%`) | `45.741 ms` | `53.179 ms` | `83.301 ms` | **`3.20x Bun` (`51.86x` Warm) / `5.82x Node`** |
| **FW-04** | `FW-04: frameworks/04-sveltekit-edge-ssr` (`@sveltejs/kit@3.1.1` + `svelte@5.57.3` + `devalue@6.0.3`) | **`31.568 ms`** (`28.99 ms` Min) | **`30.294 ms`** | **`13.190 ms`** | `29.812 ms` (`5.9%`) | `46.892 ms` | `71.807 ms` | `107.255 ms` | **`1.55x Bun` (`3.56x` Warm) / `3.54x Node`** |
| **CS-01** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`29.012 ms`** (`27.37 ms` Min) | **`29.155 ms`** | **`11.989 ms`** | `27.772 ms` (`4.5%`) | `29.762 ms` | `73.084 ms` | `82.967 ms` | **`1.03x Bun` (`2.48x` Warm) / `2.86x Node`** |
| **CS-02** | `CS-02: case_studies/02-mcp-code-mode-orchestrator` (`-98.4%` Tokens) | **`16.178 ms`** (`15.48 ms` Min) | **`17.021 ms`** | **`2.106 ms`** | `15.956 ms` (`1.4%`) | `17.525 ms` | `43.987 ms` | `61.158 ms` | **`1.08x Bun` (`8.32x` Warm) / `3.78x Node`** |
| **CS-03** | `CS-03: case_studies/03-multi-tenant-saas-plugin-mesh` (12 Tenants + Hot-Swap) | **`12.752 ms`** (`11.76 ms` Min) | **`13.454 ms`** | **`4.533 ms`** | `12.342 ms` (`3.3%`) | `17.948 ms` | `48.507 ms` | `80.388 ms` | **`1.41x Bun` (`3.96x` Warm) / `6.30x Node`** |
| **CS-04** | `CS-04: case_studies/04-wasm-zero-trust-cdr-pipeline` (Wasm SimHash CDR) | **`15.182 ms`** (`13.85 ms` Min) | **`15.484 ms`** | **`1.865 ms`** | `14.076 ms` (`7.9%`) | `20.309 ms` | `44.538 ms` | `60.336 ms` | **`1.34x Bun` (`10.89x` Warm) / `3.97x Node`** |
| **CS-05** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`8.845 ms`** (`8.18 ms` Min) | **`9.438 ms`** | **`2.349 ms`** | `8.804 ms` (`0.5%`) | `21.113 ms` | `63.957 ms` | `78.555 ms` | **`2.39x Bun` (`8.99x` Warm) / `8.88x Node`** |

---

## 4. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Workload | Slice (`SLICEBC\x01` Default) | Bun (`v1.4.5`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha.0.1.1`) | Slice Memory Advantage |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **App 2: NPM semver+ms+ini** | **14,208 KB (13.88 MB)** | 38,656 KB (37.75 MB) | 46,560 KB (45.47 MB) | 70,096 KB (68.45 MB) | **4.93x less than Node / 2.72x less than Bun** |
| **App 6: Telemetry + Flatted** | **14,464 KB (14.13 MB)** | 40,064 KB (39.13 MB) | 50,776 KB (49.59 MB) | 71,272 KB (69.60 MB) | **4.93x less than Node / 2.77x less than Bun** |
| **App 21: RFC 6455 WS + Sec** | **15,232 KB (14.88 MB)** | 37,628 KB (36.75 MB) | 50,596 KB (49.41 MB) | 92,768 KB (90.59 MB) | **6.09x less than Node / 2.47x less than Bun** |
| **App 22: Workers + Sockets** | **22,528 KB (22.00 MB)** | 47,272 KB (46.16 MB) | 63,492 KB (62.00 MB) | 91,372 KB (89.23 MB) | **4.06x less than Node / 2.10x less than Bun** |
| **App 23: Pure-Rust WASM** | **19,456 KB (19.00 MB)** | 37,760 KB (36.88 MB) | 48,516 KB (47.38 MB) | 77,916 KB (76.09 MB) | **4.00x less than Node / 1.94x less than Bun** |
| **FW 1: Hono v4 Microservice** | **17,792 KB (17.38 MB)** | 40,916 KB (39.96 MB) | 50,296 KB (49.12 MB) | 87,504 KB (85.45 MB) | **4.92x less than Node / 2.30x less than Bun** |
| **FW 2: itty-router + UnJS** | **15,360 KB (15.00 MB)** | 40,704 KB (39.75 MB) | 48,756 KB (47.61 MB) | 85,860 KB (83.85 MB) | **5.59x less than Node / 2.65x less than Bun** |
| **App 24: WASM + SSR** | **15,104 KB (14.75 MB)** | 38,016 KB (37.13 MB) | 48,940 KB (47.79 MB) | 79,468 KB (77.61 MB) | **5.26x less than Node / 2.52x less than Bun** |
| **FW 3: Express node:http** | **15,488 KB (15.13 MB)** | 41,728 KB (40.75 MB) | 47,188 KB (46.08 MB) | 79,312 KB (77.45 MB) | **5.12x less than Node / 2.69x less than Bun** |
| **01-tot-autonomous-sre-agent** | **20,864 KB (20.38 MB)** | 41,728 KB (40.75 MB) | 53,488 KB (52.23 MB) | 80,360 KB (78.48 MB) | **3.85x less than Node / 2.00x less than Bun** |
| **02-mcp-code-mode-orchestrator** | **17,916 KB (17.50 MB)** | 37,632 KB (36.75 MB) | 46,476 KB (45.39 MB) | 77,900 KB (76.07 MB) | **4.35x less than Node / 2.10x less than Bun** |
| **03-multi-tenant-saas-plugin-mesh** | **15,104 KB (14.75 MB)** | 37,760 KB (36.88 MB) | 49,984 KB (48.81 MB) | 87,016 KB (84.98 MB) | **5.76x less than Node / 2.50x less than Bun** |
| **04-wasm-zero-trust-cdr-pipeline** | **19,584 KB (19.13 MB)** | 39,168 KB (38.25 MB) | 49,772 KB (48.61 MB) | 78,568 KB (76.73 MB) | **4.01x less than Node / 2.00x less than Bun** |
| **05-time-travel-chaos-fuzzer-payments** | **14,592 KB (14.25 MB)** | 39,040 KB (38.13 MB) | 49,400 KB (48.24 MB) | 78,152 KB (76.32 MB) | **5.36x less than Node / 2.68x less than Bun** |
| **FW 4: SvelteKit 3 + Svelte 5** | **22,904 KB (22.37 MB)** | 49,408 KB (48.25 MB) | 56,252 KB (54.93 MB) | 84,592 KB (82.61 MB) | **3.69x less than Node / 2.16x less than Bun** |

---

## 5. Multi-Tenant Edge KV, AI Agent Checkpoints & 5-Ring Security Telemetry

| Subsystem / Benchmark | Measured Telemetry | Status |
| :--- | :--- | :---: |
| **Sharded `Slice.kv` (`1,001` Get/Set/Incr Ops in Isolate)** | **`579.35 us`** total (`--warm`, `288 B` memory cage) | `PASS` |
| **Multi-Tenant `--tenant-dir` Zero-Downtime Hot-Swap** | Live atomic replacement of `tenant-alpha.cas` (`v1 -> v2-hot-swapped`) with isolated `Slice.kv` state | `PASS` |
| **Deterministic Time-Travel Replay (`--deterministic=20261019`)** | 100% bit-for-bit identical PRNG (`Math.random`) & virtual clock (`Date.now`, `performance.now`) | `PASS` |
| **Tree-of-Thought AI Agent (`Slice.checkpoint` + 3 `evalSandbox` + `restore`)** | **`55.55 us`** total (`1.92 us` child sandbox) | `PASS` |
| **Cloud Metadata SSRF (`169.254.169.254`) & Base64 Egress DLP Guard** | `SSRF_METADATA_BLOCKED` & `EGRESS_SECRET_LEAK_BLOCKED` deterministically trapped | `PASS` |
| **Built-In Model Context Protocol Server (`slice mcp --smoke-test`)** | JSON-RPC 2.0 `initialize`, `tools/list`, `tools/call` verified over `stdio` | `PASS` |
