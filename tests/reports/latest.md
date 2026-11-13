# Slice (Nexus-JS) - Full 33-Workload & MicroVM Benchmark Report

* **Slice Version:** `v0.10.12` (Commit `46b9717`)
* **Generated Timestamp (UTC):** `2026-11-13T17:04:52Z`
* **Archive File:** [`tests/reports/bench_v0.10.12_20261113_170452Z_46b9717.md`](./bench_v0.10.12_20261113_170452Z_46b9717.md)
* **Canonical Latest Report:** [`tests/reports/latest.md`](./latest.md)
* **Environment:** `Linux x86_64 Docker (io_uring SQPOLL + WRPKRU)`
* **Compared Runtimes:**
  * **Slice:** `v0.10.12` (Default `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS + Monomorphic Inline Caching `get_fast` + Iterative ECMA-404 `JSON.parse`)
  * **Bun:** `v1.4.4` (JavaScriptCore)
  * **Deno:** `v2.9.8` (V8)
  * **Node.js:** `v27.0.0-alpha.0.0.0` (V8 native TypeScript execution)

---

## 1. Executive Summary

Across all **33 real-world multi-file TypeScript applications, framework suites, and case studies**, Slice beats **Deno (`v2.9.8`)** on **33/33** workloads, beats **Node.js (`v27.0.0-alpha.0.0.0`)** on **33/33** workloads, beats **Bun (`v1.4.4`)** on **27/33** workloads in cold one-shot execution, and beats **all 3 runtimes on 33/33 workloads in `--warm` pooled isolate execution**.

| Metric | Linux (`x86_64` Docker) | Windows (`x86_64` Host) | Comparison vs Runtimes & MicroVMs |
| :--- | :---: | :---: | :--- |
| **Single Isolate Cold-Start (Min / P50 / P99)** | **`0.99 us` / `1.25 us` / `1.97 us`** | **`1.40 us` / `1.50 us` / `2.00 us`** | **~120,000x faster than 125ms Firecracker VM** |
| **1,000-Tenant MicroVM Swarm (Boot + Exec)** | **`1.42 ms`** (`7.88 MB` RSS) | **`2.08 ms`** (`6.26 MB` RSS) | **> 10,000x faster than container/VM swarms** |
| **500-Module Monorepo Cold Boot (`0 stat()`)** | **`2.714 ms`** (`8.14 us` in-mem) | **`5.618 ms`** (`13.10 us` in-mem) | **25.9x faster than Node.js (`1,500+ stat()`)** |
| **10,000 Direct Kernel Ring I/O Completions** | **`0.01 ms`** | **`5.06 ms`** | **Zero worker-thread lock contention** |
| **5-Ring Security Containment Verification** | **`5 / 5 PASS`** | **`5 / 5 PASS`** | **OCap + SES + Fuel + SSRF/DLP + MPK** |

---

## 2. `slice serve` Multi-Tenant HTTP/1.1 Keep-Alive Throughput (`16,000` Reqs, 8 Sockets x 8 Pipeline)

| Server / Runtime | Throughput (`req/s`) | Avg Latency / Req | Total Time (`16k` Reqs) | Relative Speedup vs Runtimes |
| :--- | :---: | :---: | :---: | :--- |
| **`slice serve` (Tier-0 CAS, `--workers=8`)** | **`162,482 req/s`** | **`6.15 us/req`** | **`98.5 ms`** | **`1.64x` faster than `Bun.serve` / `2.84x` Node.js** |
| **`slice serve` (Tier-1 WinterTC, `--workers=8`)** | **`150,927 req/s`** | **`6.63 us/req`** | **`106.0 ms`** | **`1.52x` faster than `Bun.serve` / `2.63x` Node.js** |
| **`Deno.serve` (`v2.9.8`, V8, No Per-Req Sandbox)** | `94,376 req/s` | `10.60 us/req` | `169.5 ms` | `0.95x` vs Bun |
| **`Bun.serve` (`v1.4.4`, JSC, No Per-Req Sandbox)** | `99,361 req/s` | `10.06 us/req` | `161.0 ms` | `1.00x` (Baseline) |
| **`Node.js http.createServer` (`v27.0.0-alpha.0.0.0`, V8)** | `57,280 req/s` | `17.46 us/req` | `279.3 ms` | `0.58x` vs Bun |

---

## 3. Full 33-Workload Latency Comparison (20 Runs Each, Linux `x86_64` Docker)

| # | Workload (`tests/apps/`, `tests/frameworks/`, `tests/case_studies/`) | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm (`--warm`) | Previous `.cas` (`Delta`) | Bun `v1.4.4` | Deno `v2.9.8` | Node.js `v27.0.0-alpha.0.0.0` | Slice Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`6.265 ms`** (`5.82 ms` Min) | **`6.643 ms`** | **`1.132 ms`** | `6.367 ms` (`-1.6%`) | `13.325 ms` | `35.297 ms` | `60.494 ms` | **`2.13x Bun` (`11.77x` Warm) / `9.66x Node`** |
| **02** | `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | **`13.172 ms`** (`12.63 ms` Min) | **`14.045 ms`** | **`3.912 ms`** | `14.656 ms` (`-10.1%`) | `21.983 ms` | `52.961 ms` | `68.935 ms` | **`1.67x Bun` (`5.62x` Warm) / `5.23x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`7.480 ms`** (`7.00 ms` Min) | **`7.931 ms`** | **`1.222 ms`** | `7.464 ms` (`0.2%`) | `14.051 ms` | `42.896 ms` | `61.289 ms` | **`1.88x Bun` (`11.50x` Warm) / `8.19x Node`** |
| **04** | `04-edge-api-gateway` (`fast-querystring` + `ms`) | **`7.545 ms`** (`6.95 ms` Min) | **`8.042 ms`** | **`1.663 ms`** | `7.652 ms` (`-1.4%`) | `14.954 ms` | `45.010 ms` | `63.131 ms` | **`1.98x Bun` (`8.99x` Warm) / `8.37x Node`** |
| **05** | `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | **`10.676 ms`** (`9.81 ms` Min) | **`10.619 ms`** | **`4.932 ms`** | `10.426 ms` (`2.4%`) | `14.411 ms` | `44.409 ms` | `62.412 ms` | **`1.36x Bun` (`2.92x` Warm) / `5.88x Node`** |
| **06** | `06-telemetry-metrics-pipeline` (`bytes` + `flatted` + WAL) | **`12.734 ms`** (`12.37 ms` Min) | **`13.514 ms`** | **`5.105 ms`** | `12.875 ms` (`-1.1%`) | `17.036 ms` | `47.760 ms` | `64.167 ms` | **`1.34x Bun` (`3.34x` Warm) / `5.04x Node`** |
| **07** | `07-graphql-ast-validator` (`dlv` + `dset` + AST Cost) | **`9.513 ms`** (`8.36 ms` Min) | **`9.277 ms`** | **`3.872 ms`** | `9.182 ms` (`3.6%`) | `13.298 ms` | `50.338 ms` | `60.925 ms` | **`1.43x Bun` (`3.43x` Warm) / `6.57x Node`** |
| **08** | `08-event-driven-state-journal` (`mitt` + `ms` + JSONL Replay) | **`11.202 ms`** (`10.51 ms` Min) | **`11.899 ms`** | **`5.181 ms`** | `12.722 ms` (`-11.9%`) | `15.870 ms` | `52.342 ms` | `62.841 ms` | **`1.42x Bun` (`3.06x` Warm) / `5.61x Node`** |
| **09** | `09-wintertc-web-router` (`Request` + `Response` + `URL`, 80 reqs) | **`17.056 ms`** (`16.14 ms` Min) | **`17.984 ms`** | **`10.137 ms`** | `15.646 ms` (`9.0%`) | `13.618 ms` | `50.700 ms` | `84.158 ms` | **`1.34x Bun Warm` / `4.93x Node`** |
| **10** | `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **`14.664 ms`** (`14.09 ms` Min) | **`15.660 ms`** | **`7.125 ms`** | `13.388 ms` (`9.5%`) | `14.273 ms` | `47.818 ms` | `84.390 ms` | **`2.00x Bun Warm` / `5.75x Node`** |
| **11** | `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **`11.775 ms`** (`11.16 ms` Min) | **`12.215 ms`** | **`5.146 ms`** | `10.637 ms` (`10.7%`) | `13.446 ms` | `49.029 ms` | `88.647 ms` | **`1.14x Bun` (`2.61x` Warm) / `7.53x Node`** |
| **12** | `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP, 320 frames) | **`17.129 ms`** (`15.47 ms` Min) | **`16.526 ms`** | **`7.634 ms`** | `15.938 ms` (`7.5%`) | `15.523 ms` | `50.096 ms` | `89.167 ms` | **`2.03x Bun Warm` / `5.40x Node`** |
| **13** | `13-webhook-orchestrator-ssrf` (`fast-querystring` + `ms` + Live `fetch`) | **`16.514 ms`** (`15.57 ms` Min) | **`16.733 ms`** | **`6.647 ms`** | `16.015 ms` (`3.1%`) | `19.423 ms` | `51.641 ms` | `111.561 ms` | **`1.18x Bun` (`2.92x` Warm) / `6.76x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`16.525 ms`** (`15.75 ms` Min) | **`17.097 ms`** | **`7.595 ms`** | `15.932 ms` (`3.7%`) | `20.681 ms` | `52.803 ms` | `107.237 ms` | **`1.25x Bun` (`2.72x` Warm) / `6.49x Node`** |
| **15** | `15-time-travel-test-suite` (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **`14.839 ms`** (`14.07 ms` Min) | **`15.058 ms`** | **`5.109 ms`** | `15.649 ms` (`-5.2%`) | `24.649 ms` | `68.997 ms` | `83.813 ms` | **`1.66x Bun` (`4.82x` Warm) / `5.65x Node`** |
| **16** | `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **`7.555 ms`** (`7.08 ms` Min) | **`8.404 ms`** | **`1.332 ms`** | `7.912 ms` (`-4.5%`) | `14.729 ms` | `45.416 ms` | `65.129 ms` | **`1.95x Bun` (`11.06x` Warm) / `8.62x Node`** |
| **17** | `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive/307/SSRF/DLP) | **`18.096 ms`** (`17.27 ms` Min) | **`18.833 ms`** | **`8.683 ms`** | `17.820 ms` (`1.5%`) | `21.923 ms` | `56.133 ms` | `111.297 ms` | **`1.21x Bun` (`2.52x` Warm) / `6.15x Node`** |
| **18** | `18-async-event-loop-fanout-scheduler` (Timer Wheel/`Promise.all`/`AbortSignal`) | **`30.874 ms`** (`29.33 ms` Min) | **`31.591 ms`** | **`17.171 ms`** | `31.783 ms` (`-2.9%`) | `32.483 ms` | `71.346 ms` | `80.321 ms` | **`1.05x Bun` (`1.89x` Warm) / `2.60x Node`** |
| **19** | `19-wintertc-webcrypto-streams-edge` (WebCrypto/Streams/FormData/Clone) | **`8.593 ms`** (`8.03 ms` Min) | **`9.189 ms`** | **`1.460 ms`** | `8.765 ms` (`-2.0%`) | `14.744 ms` | `45.665 ms` | `83.735 ms` | **`1.72x Bun` (`10.10x` Warm) / `9.74x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`10.668 ms`** (`10.17 ms` Min) | **`11.181 ms`** | **`0.470 ms`** | `11.177 ms` (`-4.6%`) | `22.026 ms` | `54.814 ms` | `70.851 ms` | **`2.06x Bun` (`46.86x` Warm) / `6.64x Node`** |
| **21** | `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket` + 5-Ring Guard) | **`10.806 ms`** (`10.23 ms` Min) | **`11.540 ms`** | **`2.327 ms`** | `10.644 ms` (`1.5%`) | `20.083 ms` | `53.996 ms` | `116.501 ms` | **`1.86x Bun` (`8.63x` Warm) / `10.78x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`21.844 ms`** (`21.20 ms` Min) | **`22.222 ms`** | **`3.933 ms`** | `21.875 ms` (`-0.1%`) | `56.947 ms` | `112.291 ms` | `103.688 ms` | **`2.61x Bun` (`14.48x` Warm) / `4.75x Node`** |
| **23** | `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **`16.288 ms`** (`15.72 ms` Min) | **`16.794 ms`** | **`0.970 ms`** | `16.758 ms` (`-2.8%`) | `21.613 ms` | `41.861 ms` | `59.946 ms` | **`1.33x Bun` (`22.28x` Warm) / `3.68x Node`** |
| **24** | `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` SLA Scorer + SSR Pipeline) | **`8.843 ms`** (`8.25 ms` Min) | **`8.946 ms`** | **`1.543 ms`** | `8.732 ms` (`1.3%`) | `18.540 ms` | `43.602 ms` | `66.576 ms` | **`2.10x Bun` (`12.02x` Warm) / `7.53x Node`** |
| **FW-01** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`15.516 ms`** (`14.65 ms` Min) | **`15.883 ms`** | **`6.322 ms`** | `17.826 ms` (`-13.0%`) | `19.770 ms` | `49.178 ms` | `94.014 ms` | **`1.27x Bun` (`3.13x` Warm) / `6.06x Node`** |
| **FW-02** | `FW-02: frameworks/02-itty-router-unjs-nitro-stack` (`itty-router` + UnJS) | **`17.620 ms`** (`17.06 ms` Min) | **`18.179 ms`** | **`9.085 ms`** | `17.649 ms` (`-0.2%`) | `19.159 ms` | `45.023 ms` | `84.569 ms` | **`1.09x Bun` (`2.11x` Warm) / `4.80x Node`** |
| **FW-03** | `FW-03: frameworks/03-node-http-express-compat` (`node:http` + `node:stream` Express/Polka) | **`18.561 ms`** (`16.52 ms` Min) | **`18.268 ms`** | **`1.172 ms`** | `17.078 ms` (`8.7%`) | `34.159 ms` | `50.834 ms` | `86.573 ms` | **`1.87x Bun` (`29.15x` Warm) / `4.74x Node`** |
| **FW-04** | `FW-04: frameworks/04-sveltekit-edge-ssr` (`@sveltejs/kit@3.1.1` + `svelte@5.57.3` + `devalue@6.0.3`) | **`33.795 ms`** (`32.40 ms` Min) | **`37.038 ms`** | **`14.830 ms`** | `33.319 ms` (`1.4%`) | `46.698 ms` | `71.072 ms` | `111.478 ms` | **`1.38x Bun` (`3.15x` Warm) / `3.30x Node`** |
| **CS-01** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`37.938 ms`** (`32.22 ms` Min) | **`34.029 ms`** | **`12.148 ms`** | `33.907 ms` (`11.9%`) | `28.323 ms` | `71.329 ms` | `87.953 ms` | **`2.33x Bun Warm` / `2.58x Node`** |
| **CS-02** | `CS-02: case_studies/02-mcp-code-mode-orchestrator` (`-98.4%` Tokens) | **`19.490 ms`** (`18.61 ms` Min) | **`19.739 ms`** | **`3.535 ms`** | `19.465 ms` (`0.1%`) | `16.446 ms` | `43.091 ms` | `67.460 ms` | **`4.65x Bun Warm` / `3.46x Node`** |
| **CS-03** | `CS-03: case_studies/03-multi-tenant-saas-plugin-mesh` (12 Tenants + Hot-Swap) | **`14.633 ms`** (`13.76 ms` Min) | **`15.502 ms`** | **`6.000 ms`** | `14.453 ms` (`1.2%`) | `16.291 ms` | `47.488 ms` | `82.742 ms` | **`1.11x Bun` (`2.72x` Warm) / `5.65x Node`** |
| **CS-04** | `CS-04: case_studies/04-wasm-zero-trust-cdr-pipeline` (Wasm SimHash CDR) | **`20.253 ms`** (`19.48 ms` Min) | **`20.787 ms`** | **`1.412 ms`** | `20.159 ms` (`0.5%`) | `18.499 ms` | `43.307 ms` | `68.165 ms` | **`13.10x Bun Warm` / `3.37x Node`** |
| **CS-05** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`17.054 ms`** (`16.31 ms` Min) | **`17.670 ms`** | **`8.826 ms`** | `17.288 ms` (`-1.4%`) | `19.789 ms` | `62.273 ms` | `83.390 ms` | **`1.16x Bun` (`2.24x` Warm) / `4.89x Node`** |

---

## 4. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Workload | Slice (`SLICEBC\x01` Default) | Bun (`v1.4.4`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha.0.0.0`) | Slice Memory Advantage |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **App 2: NPM semver+ms+ini** | **13,824 KB (13.50 MB)** | 30,336 KB (29.63 MB) | 47,360 KB (46.25 MB) | 71,328 KB (69.66 MB) | **5.16x less than Node / 2.19x less than Bun** |
| **App 6: Telemetry + Flatted** | **14,336 KB (14.00 MB)** | 30,080 KB (29.38 MB) | 50,932 KB (49.74 MB) | 73,088 KB (71.38 MB) | **5.10x less than Node / 2.10x less than Bun** |
| **App 21: RFC 6455 WS + Sec** | **15,104 KB (14.75 MB)** | 31,572 KB (30.83 MB) | 50,604 KB (49.42 MB) | 94,976 KB (92.75 MB) | **6.29x less than Node / 2.09x less than Bun** |
| **App 22: Workers + Sockets** | **22,912 KB (22.38 MB)** | 41,788 KB (40.81 MB) | 63,244 KB (61.76 MB) | 93,308 KB (91.12 MB) | **4.07x less than Node / 1.82x less than Bun** |
| **App 23: Pure-Rust WASM** | **19,456 KB (19.00 MB)** | 30,848 KB (30.13 MB) | 49,164 KB (48.01 MB) | 80,276 KB (78.39 MB) | **4.13x less than Node / 1.59x less than Bun** |
| **FW 1: Hono v4 Microservice** | **16,128 KB (15.75 MB)** | 30,972 KB (30.25 MB) | 49,724 KB (48.56 MB) | 89,356 KB (87.26 MB) | **5.54x less than Node / 1.92x less than Bun** |
| **FW 2: itty-router + UnJS** | **14,976 KB (14.63 MB)** | 31,744 KB (31.00 MB) | 49,136 KB (47.98 MB) | 87,176 KB (85.13 MB) | **5.82x less than Node / 2.12x less than Bun** |
| **App 24: WASM + SSR** | **14,208 KB (13.88 MB)** | 30,592 KB (29.88 MB) | 48,636 KB (47.50 MB) | 81,152 KB (79.25 MB) | **5.71x less than Node / 2.15x less than Bun** |
| **FW 3: Express node:http** | **15,744 KB (15.38 MB)** | 33,024 KB (32.25 MB) | 46,576 KB (45.48 MB) | 80,396 KB (78.51 MB) | **5.11x less than Node / 2.10x less than Bun** |
| **01-tot-autonomous-sre-agent** | **21,248 KB (20.75 MB)** | 32,000 KB (31.25 MB) | 53,560 KB (52.30 MB) | 83,972 KB (82.00 MB) | **3.95x less than Node / 1.51x less than Bun** |
| **02-mcp-code-mode-orchestrator** | **18,940 KB (18.50 MB)** | 28,032 KB (27.38 MB) | 46,940 KB (45.84 MB) | 80,464 KB (78.58 MB) | **4.25x less than Node / 1.48x less than Bun** |
| **03-multi-tenant-saas-plugin-mesh** | **15,744 KB (15.38 MB)** | 28,544 KB (27.88 MB) | 50,012 KB (48.84 MB) | 88,200 KB (86.13 MB) | **5.60x less than Node / 1.81x less than Bun** |
| **04-wasm-zero-trust-cdr-pipeline** | **21,120 KB (20.63 MB)** | 30,848 KB (30.13 MB) | 49,912 KB (48.74 MB) | 80,388 KB (78.50 MB) | **3.81x less than Node / 1.46x less than Bun** |
| **05-time-travel-chaos-fuzzer-payments** | **16,000 KB (15.63 MB)** | 29,156 KB (28.47 MB) | 48,760 KB (47.62 MB) | 81,568 KB (79.66 MB) | **5.10x less than Node / 1.82x less than Bun** |
| **FW 4: SvelteKit 3 + Svelte 5** | **22,912 KB (22.38 MB)** | 42,624 KB (41.63 MB) | 56,260 KB (54.94 MB) | 85,920 KB (83.91 MB) | **3.75x less than Node / 1.86x less than Bun** |

---

## 5. Multi-Tenant Edge KV, AI Agent Checkpoints & 5-Ring Security Telemetry

| Subsystem / Benchmark | Measured Telemetry | Status |
| :--- | :--- | :---: |
| **Sharded `Slice.kv` (`1,001` Get/Set/Incr Ops in Isolate)** | **`714.69 us`** total (`--warm`, `8,466 B` memory cage) | `PASS` |
| **Multi-Tenant `--tenant-dir` Zero-Downtime Hot-Swap** | Live atomic replacement of `tenant-alpha.cas` (`v1 -> v2-hot-swapped`) with isolated `Slice.kv` state | `PASS` |
| **Deterministic Time-Travel Replay (`--deterministic=20261019`)** | 100% bit-for-bit identical PRNG (`Math.random`) & virtual clock (`Date.now`, `performance.now`) | `PASS` |
| **Tree-of-Thought AI Agent (`Slice.checkpoint` + 3 `evalSandbox` + `restore`)** | **`43.15 us`** total (`1.80 us` child sandbox) | `PASS` |
| **Cloud Metadata SSRF (`169.254.169.254`) & Base64 Egress DLP Guard** | `SSRF_METADATA_BLOCKED` & `EGRESS_SECRET_LEAK_BLOCKED` deterministically trapped | `PASS` |
| **Built-In Model Context Protocol Server (`slice mcp --smoke-test`)** | JSON-RPC 2.0 `initialize`, `tools/list`, `tools/call` verified over `stdio` | `PASS` |
