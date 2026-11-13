# Slice (Nexus-JS) - Full 33-Workload & MicroVM Benchmark Report

* **Slice Version:** `v0.10.12` (Commit `4d021ab`)
* **Generated Timestamp (UTC):** `2026-11-13T02:14:57Z`
* **Archive File:** [`tests/reports/bench_v0.10.12_20261113_021457Z_4d021ab.md`](./bench_v0.10.12_20261113_021457Z_4d021ab.md)
* **Canonical Latest Report:** [`tests/reports/latest.md`](./latest.md)
* **Environment:** `Linux x86_64 Docker (io_uring SQPOLL + WRPKRU)`
* **Compared Runtimes:**
  * **Slice:** `v0.10.12` (Default `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS + Monomorphic Inline Caching `get_fast` + Iterative ECMA-404 `JSON.parse`)
  * **Bun:** `v1.4.4` (JavaScriptCore)
  * **Deno:** `v2.9.8` (V8)
  * **Node.js:** `v27.0.0-alpha.0.0.0` (V8 native TypeScript execution)

---

## 1. Executive Summary

Across all **33 real-world multi-file TypeScript applications, framework suites, and case studies**, Slice beats **Deno (`v2.9.8`)** on **33/33** workloads, beats **Node.js (`v27.0.0-alpha.0.0.0`)** on **33/33** workloads, beats **Bun (`v1.4.4`)** on **28/33** workloads in cold one-shot execution, and beats **all 3 runtimes on 33/33 workloads in `--warm` pooled isolate execution**.

| Metric | Linux (`x86_64` Docker) | Windows (`x86_64` Host) | Comparison vs Runtimes & MicroVMs |
| :--- | :---: | :---: | :--- |
| **Single Isolate Cold-Start (Min / P50 / P99)** | **`0.98 us` / `1.28 us` / `1.38 us`** | **`1.20 us` / `1.50 us` / `2.40 us`** | **~120,000x faster than 125ms Firecracker VM** |
| **1,000-Tenant MicroVM Swarm (Boot + Exec)** | **`1.49 ms`** (`7.75 MB` RSS) | **`1.82 ms`** (`6.20 MB` RSS) | **> 10,000x faster than container/VM swarms** |
| **500-Module Monorepo Cold Boot (`0 stat()`)** | **`2.648 ms`** (`7.90 us` in-mem) | **`5.754 ms`** (`15.00 us` in-mem) | **26.6x faster than Node.js (`1,500+ stat()`)** |
| **10,000 Direct Kernel Ring I/O Completions** | **`0.01 ms`** | **`5.03 ms`** | **Zero worker-thread lock contention** |
| **5-Ring Security Containment Verification** | **`5 / 5 PASS`** | **`5 / 5 PASS`** | **OCap + SES + Fuel + SSRF/DLP + MPK** |

---

## 2. `slice serve` Multi-Tenant HTTP/1.1 Keep-Alive Throughput (`16,000` Reqs, 8 Sockets x 8 Pipeline)

| Server / Runtime | Throughput (`req/s`) | Avg Latency / Req | Total Time (`16k` Reqs) | Relative Speedup vs Runtimes |
| :--- | :---: | :---: | :---: | :--- |
| **`slice serve` (Tier-0 CAS, `--workers=8`)** | **`156,688 req/s`** | **`6.38 us/req`** | **`102.1 ms`** | **`1.62x` faster than `Bun.serve` / `2.86x` Node.js** |
| **`slice serve` (Tier-1 WinterTC, `--workers=8`)** | **`146,827 req/s`** | **`6.81 us/req`** | **`109.0 ms`** | **`1.52x` faster than `Bun.serve` / `2.68x` Node.js** |
| **`Deno.serve` (`v2.9.8`, V8, No Per-Req Sandbox)** | `102,218 req/s` | `9.78 us/req` | `156.5 ms` | `1.06x` vs Bun |
| **`Bun.serve` (`v1.4.4`, JSC, No Per-Req Sandbox)** | `96,828 req/s` | `10.33 us/req` | `165.2 ms` | `1.00x` (Baseline) |
| **`Node.js http.createServer` (`v27.0.0-alpha.0.0.0`, V8)** | `54,845 req/s` | `18.23 us/req` | `291.7 ms` | `0.57x` vs Bun |

---

## 3. Full 33-Workload Latency Comparison (20 Runs Each, Linux `x86_64` Docker)

| # | Workload (`tests/apps/`, `tests/frameworks/`, `tests/case_studies/`) | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm (`--warm`) | Previous `.cas` (`Delta`) | Bun `v1.4.4` | Deno `v2.9.8` | Node.js `v27.0.0-alpha.0.0.0` | Slice Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`6.367 ms`** (`5.98 ms` Min) | **`6.815 ms`** | **`1.111 ms`** | `6.197 ms` (`2.7%`) | `13.151 ms` | `36.019 ms` | `61.456 ms` | **`2.07x Bun` (`11.84x` Warm) / `9.65x Node`** |
| **02** | `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | **`14.656 ms`** (`13.80 ms` Min) | **`15.060 ms`** | **`3.978 ms`** | `14.246 ms` (`2.9%`) | `21.271 ms` | `53.419 ms` | `69.285 ms` | **`1.45x Bun` (`5.35x` Warm) / `4.73x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`7.464 ms`** (`7.01 ms` Min) | **`7.950 ms`** | **`1.192 ms`** | `7.197 ms` (`3.7%`) | `13.918 ms` | `42.505 ms` | `61.936 ms` | **`1.86x Bun` (`11.68x` Warm) / `8.30x Node`** |
| **04** | `04-edge-api-gateway` (`fast-querystring` + `ms`) | **`7.652 ms`** (`7.23 ms` Min) | **`8.314 ms`** | **`1.716 ms`** | `7.332 ms` (`4.4%`) | `15.111 ms` | `45.470 ms` | `63.502 ms` | **`1.97x Bun` (`8.81x` Warm) / `8.30x Node`** |
| **05** | `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | **`10.426 ms`** (`9.76 ms` Min) | **`10.852 ms`** | **`4.626 ms`** | `10.285 ms` (`1.4%`) | `14.372 ms` | `45.691 ms` | `63.396 ms` | **`1.38x Bun` (`3.11x` Warm) / `6.08x Node`** |
| **06** | `06-telemetry-metrics-pipeline` (`bytes` + `flatted` + WAL) | **`12.875 ms`** (`12.10 ms` Min) | **`13.584 ms`** | **`5.120 ms`** | `12.826 ms` (`0.4%`) | `17.092 ms` | `46.806 ms` | `67.400 ms` | **`1.33x Bun` (`3.34x` Warm) / `5.23x Node`** |
| **07** | `07-graphql-ast-validator` (`dlv` + `dset` + AST Cost) | **`9.182 ms`** (`8.58 ms` Min) | **`10.659 ms`** | **`3.848 ms`** | `10.054 ms` (`-8.7%`) | `13.442 ms` | `40.724 ms` | `62.665 ms` | **`1.46x Bun` (`3.49x` Warm) / `6.82x Node`** |
| **08** | `08-event-driven-state-journal` (`mitt` + `ms` + JSONL Replay) | **`12.722 ms`** (`11.29 ms` Min) | **`13.634 ms`** | **`5.138 ms`** | `11.443 ms` (`11.2%`) | `16.174 ms` | `47.641 ms` | `64.724 ms` | **`1.27x Bun` (`3.15x` Warm) / `5.09x Node`** |
| **09** | `09-wintertc-web-router` (`Request` + `Response` + `URL`, 80 reqs) | **`15.646 ms`** (`15.00 ms` Min) | **`16.393 ms`** | **`9.505 ms`** | `15.519 ms` (`0.8%`) | `13.836 ms` | `53.711 ms` | `86.602 ms` | **`1.46x Bun Warm` / `5.54x Node`** |
| **10** | `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **`13.388 ms`** (`12.71 ms` Min) | **`14.436 ms`** | **`6.581 ms`** | `13.388 ms` (`0.0%`) | `15.021 ms` | `47.287 ms` | `84.640 ms` | **`1.12x Bun` (`2.28x` Warm) / `6.32x Node`** |
| **11** | `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **`10.637 ms`** (`10.17 ms` Min) | **`11.459 ms`** | **`4.104 ms`** | `10.930 ms` (`-2.7%`) | `13.425 ms` | `56.648 ms` | `89.877 ms` | **`1.26x Bun` (`3.27x` Warm) / `8.45x Node`** |
| **12** | `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP, 320 frames) | **`15.938 ms`** (`15.04 ms` Min) | **`16.973 ms`** | **`7.055 ms`** | `15.296 ms` (`4.2%`) | `15.034 ms` | `50.798 ms` | `90.286 ms` | **`2.13x Bun Warm` / `5.66x Node`** |
| **13** | `13-webhook-orchestrator-ssrf` (`fast-querystring` + `ms` + Live `fetch`) | **`16.015 ms`** (`15.21 ms` Min) | **`16.538 ms`** | **`6.443 ms`** | `15.922 ms` (`0.6%`) | `19.705 ms` | `51.302 ms` | `114.422 ms` | **`1.23x Bun` (`3.06x` Warm) / `7.14x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`15.932 ms`** (`15.34 ms` Min) | **`16.551 ms`** | **`7.109 ms`** | `15.601 ms` (`2.1%`) | `21.438 ms` | `54.070 ms` | `108.982 ms` | **`1.35x Bun` (`3.02x` Warm) / `6.84x Node`** |
| **15** | `15-time-travel-test-suite` (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **`15.649 ms`** (`14.87 ms` Min) | **`16.417 ms`** | **`5.133 ms`** | `14.828 ms` (`5.5%`) | `24.751 ms` | `69.440 ms` | `83.051 ms` | **`1.58x Bun` (`4.82x` Warm) / `5.31x Node`** |
| **16** | `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **`7.912 ms`** (`7.39 ms` Min) | **`8.426 ms`** | **`1.305 ms`** | `7.737 ms` (`2.3%`) | `14.970 ms` | `46.388 ms` | `65.102 ms` | **`1.89x Bun` (`11.47x` Warm) / `8.23x Node`** |
| **17** | `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive/307/SSRF/DLP) | **`17.820 ms`** (`17.09 ms` Min) | **`18.407 ms`** | **`8.162 ms`** | `17.608 ms` (`1.2%`) | `22.076 ms` | `58.391 ms` | `110.944 ms` | **`1.24x Bun` (`2.70x` Warm) / `6.23x Node`** |
| **18** | `18-async-event-loop-fanout-scheduler` (Timer Wheel/`Promise.all`/`AbortSignal`) | **`31.783 ms`** (`29.46 ms` Min) | **`31.972 ms`** | **`17.503 ms`** | `29.524 ms` (`7.7%`) | `32.715 ms` | `71.993 ms` | `81.722 ms` | **`1.03x Bun` (`1.87x` Warm) / `2.57x Node`** |
| **19** | `19-wintertc-webcrypto-streams-edge` (WebCrypto/Streams/FormData/Clone) | **`8.765 ms`** (`7.89 ms` Min) | **`10.563 ms`** | **`1.329 ms`** | `8.378 ms` (`4.6%`) | `13.633 ms` | `47.001 ms` | `86.667 ms` | **`1.56x Bun` (`10.26x` Warm) / `9.89x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`11.177 ms`** (`10.64 ms` Min) | **`12.701 ms`** | **`0.518 ms`** | `10.830 ms` (`3.2%`) | `22.378 ms` | `55.920 ms` | `72.306 ms` | **`2.00x Bun` (`43.20x` Warm) / `6.47x Node`** |
| **21** | `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket` + 5-Ring Guard) | **`10.644 ms`** (`10.03 ms` Min) | **`11.544 ms`** | **`2.426 ms`** | `11.083 ms` (`-4.0%`) | `21.815 ms` | `50.551 ms` | `115.905 ms` | **`2.05x Bun` (`8.99x` Warm) / `10.89x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`21.875 ms`** (`20.77 ms` Min) | **`22.261 ms`** | **`4.254 ms`** | `21.431 ms` (`2.1%`) | `57.618 ms` | `111.828 ms` | `104.780 ms` | **`2.63x Bun` (`13.54x` Warm) / `4.79x Node`** |
| **23** | `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **`16.758 ms`** (`15.65 ms` Min) | **`16.776 ms`** | **`1.262 ms`** | `15.455 ms` (`8.4%`) | `20.967 ms` | `39.722 ms` | `62.128 ms` | **`1.25x Bun` (`16.61x` Warm) / `3.71x Node`** |
| **24** | `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` SLA Scorer + SSR Pipeline) | **`8.732 ms`** (`8.26 ms` Min) | **`9.255 ms`** | **`1.627 ms`** | `8.681 ms` (`0.6%`) | `18.398 ms` | `42.237 ms` | `67.453 ms` | **`2.11x Bun` (`11.31x` Warm) / `7.72x Node`** |
| **FW-01** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`17.826 ms`** (`14.77 ms` Min) | **`16.001 ms`** | **`8.907 ms`** | `16.171 ms` (`10.2%`) | `20.640 ms` | `52.170 ms` | `94.872 ms` | **`1.29x Bun` (`2.32x` Warm) / `5.93x Node`** |
| **FW-02** | `FW-02: frameworks/02-itty-router-unjs-nitro-stack` (`itty-router` + UnJS) | **`17.649 ms`** (`16.81 ms` Min) | **`18.336 ms`** | **`8.996 ms`** | `17.377 ms` (`1.6%`) | `18.994 ms` | `46.621 ms` | `89.547 ms` | **`1.08x Bun` (`2.11x` Warm) / `5.07x Node`** |
| **FW-03** | `FW-03: frameworks/03-node-http-express-compat` (`node:http` + `node:stream` Express/Polka) | **`17.078 ms`** (`16.60 ms` Min) | **`18.121 ms`** | **`1.051 ms`** | `17.811 ms` (`-4.1%`) | `35.129 ms` | `51.978 ms` | `82.965 ms` | **`2.06x Bun` (`33.42x` Warm) / `4.86x Node`** |
| **FW-04** | `FW-04: frameworks/04-sveltekit-edge-ssr` (`@sveltejs/kit@3.1.1` + `svelte@5.57.3` + `devalue@6.0.3`) | **`33.319 ms`** (`32.01 ms` Min) | **`34.421 ms`** | **`14.332 ms`** | `33.504 ms` (`-0.6%`) | `44.672 ms` | `69.125 ms` | `111.024 ms` | **`1.34x Bun` (`3.12x` Warm) / `3.33x Node`** |
| **CS-01** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`33.907 ms`** (`32.67 ms` Min) | **`37.050 ms`** | **`13.111 ms`** | `33.655 ms` (`0.7%`) | `28.394 ms` | `72.424 ms` | `90.770 ms` | **`2.17x Bun Warm` / `2.68x Node`** |
| **CS-02** | `CS-02: case_studies/02-mcp-code-mode-orchestrator` (`-98.4%` Tokens) | **`19.465 ms`** (`18.76 ms` Min) | **`20.102 ms`** | **`3.599 ms`** | `19.384 ms` (`0.4%`) | `16.673 ms` | `43.522 ms` | `67.117 ms` | **`4.63x Bun Warm` / `3.45x Node`** |
| **CS-03** | `CS-03: case_studies/03-multi-tenant-saas-plugin-mesh` (12 Tenants + Hot-Swap) | **`14.453 ms`** (`13.43 ms` Min) | **`14.822 ms`** | **`5.688 ms`** | `13.717 ms` (`5.4%`) | `16.318 ms` | `48.223 ms` | `84.079 ms` | **`1.13x Bun` (`2.87x` Warm) / `5.82x Node`** |
| **CS-04** | `CS-04: case_studies/04-wasm-zero-trust-cdr-pipeline` (Wasm SimHash CDR) | **`20.159 ms`** (`19.34 ms` Min) | **`21.700 ms`** | **`1.362 ms`** | `19.798 ms` (`1.8%`) | `18.895 ms` | `43.462 ms` | `68.052 ms` | **`13.87x Bun Warm` / `3.38x Node`** |
| **CS-05** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`17.288 ms`** (`16.11 ms` Min) | **`18.080 ms`** | **`8.703 ms`** | `17.561 ms` (`-1.6%`) | `19.516 ms` | `60.673 ms` | `81.498 ms` | **`1.13x Bun` (`2.24x` Warm) / `4.71x Node`** |

---

## 4. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Workload | Slice (`SLICEBC\x01` Default) | Bun (`v1.4.4`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha.0.0.0`) | Slice Memory Advantage |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **App 2: NPM semver+ms+ini** | **14,720 KB (14.38 MB)** | 29,952 KB (29.25 MB) | 46,916 KB (45.82 MB) | 71,580 KB (69.90 MB) | **4.86x less than Node / 2.03x less than Bun** |
| **App 6: Telemetry + Flatted** | **14,464 KB (14.13 MB)** | 30,592 KB (29.88 MB) | 50,924 KB (49.73 MB) | 72,856 KB (71.15 MB) | **5.04x less than Node / 2.12x less than Bun** |
| **App 21: RFC 6455 WS + Sec** | **14,976 KB (14.63 MB)** | 31,872 KB (31.13 MB) | 50,704 KB (49.52 MB) | 95,240 KB (93.01 MB) | **6.36x less than Node / 2.13x less than Bun** |
| **App 22: Workers + Sockets** | **22,784 KB (22.25 MB)** | 41,408 KB (40.44 MB) | 63,808 KB (62.31 MB) | 93,532 KB (91.34 MB) | **4.11x less than Node / 1.82x less than Bun** |
| **App 23: Pure-Rust WASM** | **19,456 KB (19.00 MB)** | 30,080 KB (29.38 MB) | 48,796 KB (47.65 MB) | 80,528 KB (78.64 MB) | **4.14x less than Node / 1.55x less than Bun** |
| **FW 1: Hono v4 Microservice** | **16,128 KB (15.75 MB)** | 30,680 KB (29.96 MB) | 49,780 KB (48.61 MB) | 89,376 KB (87.28 MB) | **5.54x less than Node / 1.90x less than Bun** |
| **FW 2: itty-router + UnJS** | **15,104 KB (14.75 MB)** | 31,872 KB (31.13 MB) | 49,192 KB (48.04 MB) | 86,920 KB (84.88 MB) | **5.75x less than Node / 2.11x less than Bun** |
| **App 24: WASM + SSR** | **14,336 KB (14.00 MB)** | 30,464 KB (29.75 MB) | 49,040 KB (47.89 MB) | 81,016 KB (79.12 MB) | **5.65x less than Node / 2.13x less than Bun** |
| **FW 3: Express node:http** | **15,872 KB (15.50 MB)** | 33,280 KB (32.50 MB) | 47,064 KB (45.96 MB) | 79,892 KB (78.02 MB) | **5.03x less than Node / 2.10x less than Bun** |
| **01-tot-autonomous-sre-agent** | **21,360 KB (20.86 MB)** | 31,872 KB (31.13 MB) | 53,196 KB (51.95 MB) | 83,332 KB (81.38 MB) | **3.90x less than Node / 1.49x less than Bun** |
| **02-mcp-code-mode-orchestrator** | **18,808 KB (18.37 MB)** | 27,776 KB (27.13 MB) | 47,012 KB (45.91 MB) | 80,512 KB (78.63 MB) | **4.28x less than Node / 1.48x less than Bun** |
| **03-multi-tenant-saas-plugin-mesh** | **15,616 KB (15.25 MB)** | 26,752 KB (26.13 MB) | 50,020 KB (48.85 MB) | 88,072 KB (86.01 MB) | **5.64x less than Node / 1.71x less than Bun** |
| **04-wasm-zero-trust-cdr-pipeline** | **20,736 KB (20.25 MB)** | 31,488 KB (30.75 MB) | 50,104 KB (48.93 MB) | 82,956 KB (81.01 MB) | **4.00x less than Node / 1.52x less than Bun** |
| **05-time-travel-chaos-fuzzer-payments** | **16,128 KB (15.75 MB)** | 29,184 KB (28.50 MB) | 49,400 KB (48.24 MB) | 81,564 KB (79.65 MB) | **5.06x less than Node / 1.81x less than Bun** |
| **FW 4: SvelteKit 3 + Svelte 5** | **22,784 KB (22.25 MB)** | 42,752 KB (41.75 MB) | 56,460 KB (55.14 MB) | 85,924 KB (83.91 MB) | **3.77x less than Node / 1.88x less than Bun** |

---

## 5. Multi-Tenant Edge KV, AI Agent Checkpoints & 5-Ring Security Telemetry

| Subsystem / Benchmark | Measured Telemetry | Status |
| :--- | :--- | :---: |
| **Sharded `Slice.kv` (`1,001` Get/Set/Incr Ops in Isolate)** | **`504.90 us`** total (`--warm`, `8,466 B` memory cage) | `PASS` |
| **Multi-Tenant `--tenant-dir` Zero-Downtime Hot-Swap** | Live atomic replacement of `tenant-alpha.cas` (`v1 -> v2-hot-swapped`) with isolated `Slice.kv` state | `PASS` |
| **Deterministic Time-Travel Replay (`--deterministic=20261019`)** | 100% bit-for-bit identical PRNG (`Math.random`) & virtual clock (`Date.now`, `performance.now`) | `PASS` |
| **Tree-of-Thought AI Agent (`Slice.checkpoint` + 3 `evalSandbox` + `restore`)** | **`44.40 us`** total (`1.77 us` child sandbox) | `PASS` |
| **Cloud Metadata SSRF (`169.254.169.254`) & Base64 Egress DLP Guard** | `SSRF_METADATA_BLOCKED` & `EGRESS_SECRET_LEAK_BLOCKED` deterministically trapped | `PASS` |
| **Built-In Model Context Protocol Server (`slice mcp --smoke-test`)** | JSON-RPC 2.0 `initialize`, `tools/list`, `tools/call` verified over `stdio` | `PASS` |
