# Slice (Nexus-JS) — Full 32-Workload & MicroVM Benchmark Report

* **Slice Version:** `v0.10.4` (Commit `7a86dd6`)
* **Generated Timestamp (UTC):** `2026-11-09T06:36:35Z`
* **Archive File:** [`tests/reports/bench_v0.10.4_20261109_063635Z_7a86dd6.md`](./bench_v0.10.4_20261109_063635Z_7a86dd6.md)
* **Canonical Latest Report:** [`tests/reports/latest.md`](./latest.md)
* **Environment:** `Linux x86_64 Docker (io_uring SQPOLL + WRPKRU)`
* **Compared Runtimes:**
  * **Slice:** `v0.10.4` (Default `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS + Monomorphic Inline Caching `get_fast` + Streaming `JsonSeed` `JSON.parse`)
  * **Bun:** `v1.4.4` (JavaScriptCore)
  * **Deno:** `v2.9.8` (V8)
  * **Node.js:** `v27.0.0-alpha.0.0.0` (V8 native TypeScript execution)

---

## 1. Executive Summary

Across all **32 real-world multi-file TypeScript applications, framework suites, and case studies**, Slice beats **Deno (`v2.9.8`)** on **32/32** workloads, beats **Node.js (`v27.0.0-alpha.0.0.0`)** on **32/32** workloads, beats **Bun (`v1.4.4`)** on **29/32** workloads in cold one-shot execution, and beats **all 3 runtimes on 32/32 workloads in `--warm` pooled isolate execution**.

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
| **`Node.js http.createServer` (`v27.0.0-alpha.0.0.0`, V8)** | `45,123 req/s` | `22.16 us/req` | `354.6 ms` | `0.49x` vs Bun |

---

## 3. Full 32-Workload Latency Comparison (20 Runs Each, Linux `x86_64` Docker)

| # | Workload (`tests/apps/`, `tests/frameworks/`, `tests/case_studies/`) | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm (`--warm`) | Previous `.cas` (`Delta`) | Bun `v1.4.4` | Deno `v2.9.8` | Node.js `v27.0.0-alpha.0.0.0` | Slice Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`6.209 ms`** (`5.75 ms` Min) | **`7.344 ms`** | **`1.024 ms`** | `7.118 ms` (`-12.8%`) | `13.479 ms` | `36.367 ms` | `59.948 ms` | **`2.17x Bun` (`13.16x` Warm) / `9.66x Node`** |
| **02** | `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | **`14.111 ms`** (`13.30 ms` Min) | **`14.730 ms`** | **`4.394 ms`** | `13.553 ms` (`4.1%`) | `22.320 ms` | `52.435 ms` | `70.680 ms` | **`1.58x Bun` (`5.08x` Warm) / `5.01x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`7.819 ms`** (`7.29 ms` Min) | **`8.724 ms`** | **`2.186 ms`** | `9.811 ms` (`-20.3%`) | `14.486 ms` | `43.481 ms` | `64.151 ms` | **`1.85x Bun` (`6.63x` Warm) / `8.20x Node`** |
| **04** | `04-edge-api-gateway` (`fast-querystring` + `ms`) | **`7.940 ms`** (`7.21 ms` Min) | **`8.555 ms`** | **`1.987 ms`** | `8.301 ms` (`-4.3%`) | `15.310 ms` | `42.365 ms` | `62.641 ms` | **`1.93x Bun` (`7.71x` Warm) / `7.89x Node`** |
| **05** | `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | **`10.820 ms`** (`10.08 ms` Min) | **`11.534 ms`** | **`5.355 ms`** | `12.164 ms` (`-11.0%`) | `14.716 ms` | `45.525 ms` | `62.690 ms` | **`1.36x Bun` (`2.75x` Warm) / `5.79x Node`** |
| **06** | `06-telemetry-metrics-pipeline` (`bytes` + `flatted` + WAL) | **`14.531 ms`** (`13.71 ms` Min) | **`15.036 ms`** | **`5.938 ms`** | `19.654 ms` (`-26.1%`) | `17.100 ms` | `48.266 ms` | `64.145 ms` | **`1.18x Bun` (`2.88x` Warm) / `4.41x Node`** |
| **07** | `07-graphql-ast-validator` (`dlv` + `dset` + AST Cost) | **`9.236 ms`** (`8.83 ms` Min) | **`10.105 ms`** | **`5.973 ms`** | `9.869 ms` (`-6.4%`) | `13.228 ms` | `47.641 ms` | `59.910 ms` | **`1.43x Bun` (`2.21x` Warm) / `6.49x Node`** |
| **08** | `08-event-driven-state-journal` (`mitt` + `ms` + JSONL Replay) | **`11.756 ms`** (`11.01 ms` Min) | **`12.479 ms`** | **`5.614 ms`** | `13.575 ms` (`-13.4%`) | `15.682 ms` | `45.016 ms` | `64.216 ms` | **`1.33x Bun` (`2.79x` Warm) / `5.46x Node`** |
| **09** | `09-wintertc-web-router` (`Request` + `Response` + `URL`, 80 reqs) | **`14.792 ms`** (`14.12 ms` Min) | **`15.994 ms`** | **`10.539 ms`** | `18.674 ms` (`-20.8%`) | `13.800 ms` | `47.165 ms` | `85.390 ms` | **`1.31x Bun Warm` / `5.77x Node`** |
| **10** | `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **`14.266 ms`** (`12.84 ms` Min) | **`14.089 ms`** | **`6.663 ms`** | `15.654 ms` (`-8.9%`) | `14.328 ms` | `48.401 ms` | `85.338 ms` | **`1.02x Bun` (`2.15x` Warm) / `6.06x Node`** |
| **11** | `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **`9.957 ms`** (`9.62 ms` Min) | **`10.786 ms`** | **`3.725 ms`** | `10.818 ms` (`-8.0%`) | `13.501 ms` | `50.629 ms` | `86.572 ms` | **`1.36x Bun` (`3.62x` Warm) / `8.69x Node`** |
| **12** | `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP, 320 frames) | **`16.313 ms`** (`15.16 ms` Min) | **`17.165 ms`** | **`8.330 ms`** | `17.560 ms` (`-7.1%`) | `15.590 ms` | `51.956 ms` | `91.041 ms` | **`1.87x Bun Warm` / `5.58x Node`** |
| **13** | `13-webhook-orchestrator-ssrf` (`fast-querystring` + `ms` + Live `fetch`) | **`13.841 ms`** (`13.17 ms` Min) | **`14.638 ms`** | **`6.802 ms`** | `15.462 ms` (`-10.5%`) | `19.498 ms` | `52.729 ms` | `112.184 ms` | **`1.41x Bun` (`2.87x` Warm) / `8.11x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`14.809 ms`** (`13.65 ms` Min) | **`16.292 ms`** | **`7.722 ms`** | `18.234 ms` (`-18.8%`) | `19.741 ms` | `54.666 ms` | `108.006 ms` | **`1.33x Bun` (`2.56x` Warm) / `7.29x Node`** |
| **15** | `15-time-travel-test-suite` (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **`15.122 ms`** (`14.37 ms` Min) | **`15.547 ms`** | **`5.539 ms`** | `14.151 ms` (`6.9%`) | `25.002 ms` | `66.696 ms` | `81.374 ms` | **`1.65x Bun` (`4.51x` Warm) / `5.38x Node`** |
| **16** | `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **`7.469 ms`** (`6.96 ms` Min) | **`8.078 ms`** | **`1.483 ms`** | `7.647 ms` (`-2.3%`) | `14.923 ms` | `47.585 ms` | `66.587 ms` | **`2.00x Bun` (`10.06x` Warm) / `8.92x Node`** |
| **17** | `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive/307/SSRF/DLP) | **`16.200 ms`** (`14.97 ms` Min) | **`16.438 ms`** | **`8.109 ms`** | `18.550 ms` (`-12.7%`) | `22.112 ms` | `55.772 ms` | `110.881 ms` | **`1.36x Bun` (`2.73x` Warm) / `6.84x Node`** |
| **18** | `18-async-event-loop-fanout-scheduler` (Timer Wheel/`Promise.all`/`AbortSignal`) | **`29.421 ms`** (`27.87 ms` Min) | **`30.273 ms`** | **`17.177 ms`** | `30.892 ms` (`-4.8%`) | `33.023 ms` | `70.776 ms` | `81.795 ms` | **`1.12x Bun` (`1.92x` Warm) / `2.78x Node`** |
| **19** | `19-wintertc-webcrypto-streams-edge` (WebCrypto/Streams/FormData/Clone) | **`8.191 ms`** (`7.58 ms` Min) | **`8.981 ms`** | **`1.615 ms`** | `7.772 ms` (`5.4%`) | `13.902 ms` | `46.583 ms` | `85.521 ms` | **`1.70x Bun` (`8.61x` Warm) / `10.44x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`11.296 ms`** (`10.35 ms` Min) | **`11.578 ms`** | **`0.794 ms`** | `10.752 ms` (`5.1%`) | `22.263 ms` | `53.889 ms` | `72.381 ms` | **`1.97x Bun` (`28.04x` Warm) / `6.41x Node`** |
| **21** | `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket` + 5-Ring Guard) | **`10.553 ms`** (`10.09 ms` Min) | **`11.457 ms`** | **`2.443 ms`** | `10.935 ms` (`-3.5%`) | `20.439 ms` | `52.042 ms` | `116.667 ms` | **`1.94x Bun` (`8.37x` Warm) / `11.06x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`20.938 ms`** (`19.19 ms` Min) | **`20.688 ms`** | **`2.569 ms`** | `20.495 ms` (`2.2%`) | `56.452 ms` | `113.360 ms` | `104.122 ms` | **`2.73x Bun` (`21.97x` Warm) / `5.03x Node`** |
| **23** | `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **`15.069 ms`** (`14.36 ms` Min) | **`15.833 ms`** | **`1.987 ms`** | `16.299 ms` (`-7.5%`) | `21.904 ms` | `41.046 ms` | `62.540 ms` | **`1.45x Bun` (`11.02x` Warm) / `4.15x Node`** |
| **24** | `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` SLA Scorer + SSR Pipeline) | **`8.419 ms`** (`7.87 ms` Min) | **`9.078 ms`** | **`1.868 ms`** | `8.534 ms` (`-1.3%`) | `19.236 ms` | `43.101 ms` | `67.248 ms` | **`2.28x Bun` (`10.30x` Warm) / `7.99x Node`** |
| **FW-01** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`15.140 ms`** (`14.48 ms` Min) | **`15.815 ms`** | **`5.463 ms`** | `15.963 ms` (`-5.2%`) | `20.201 ms` | `53.327 ms` | `92.313 ms` | **`1.33x Bun` (`3.70x` Warm) / `6.10x Node`** |
| **FW-02** | `FW-02: frameworks/02-itty-router-unjs-nitro-stack` (`itty-router` + UnJS) | **`17.873 ms`** (`16.93 ms` Min) | **`19.018 ms`** | **`9.594 ms`** | `17.697 ms` (`1.0%`) | `18.893 ms` | `47.054 ms` | `86.423 ms` | **`1.06x Bun` (`1.97x` Warm) / `4.84x Node`** |
| **FW-03** | `FW-03: frameworks/03-node-http-express-compat` (`node:http` + `node:stream` Express/Polka) | **`16.410 ms`** (`15.72 ms` Min) | **`17.571 ms`** | **`1.269 ms`** | `9.450 ms` (`73.7%`) | `35.721 ms` | `51.056 ms` | `85.952 ms` | **`2.18x Bun` (`28.15x` Warm) / `5.24x Node`** |
| **CS-01** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`26.690 ms`** (`25.75 ms` Min) | **`28.421 ms`** | **`15.689 ms`** | `25.995 ms` (`2.7%`) | `28.069 ms` | `72.993 ms` | `108.395 ms` | **`1.05x Bun` (`1.79x` Warm) / `4.06x Node`** |
| **CS-02** | `CS-02: case_studies/02-mcp-code-mode-orchestrator` (`-98.4%` Tokens) | **`19.966 ms`** (`19.19 ms` Min) | **`20.462 ms`** | **`8.205 ms`** | `16.875 ms` (`18.3%`) | `16.265 ms` | `42.627 ms` | `66.058 ms` | **`1.98x Bun Warm` / `3.31x Node`** |
| **CS-03** | `CS-03: case_studies/03-multi-tenant-saas-plugin-mesh` (12 Tenants + Hot-Swap) | **`13.726 ms`** (`12.61 ms` Min) | **`14.029 ms`** | **`5.814 ms`** | `11.400 ms` (`20.4%`) | `16.271 ms` | `48.349 ms` | `83.635 ms` | **`1.19x Bun` (`2.80x` Warm) / `6.09x Node`** |
| **CS-04** | `CS-04: case_studies/04-wasm-zero-trust-cdr-pipeline` (Wasm SimHash CDR) | **`17.723 ms`** (`16.89 ms` Min) | **`18.207 ms`** | **`1.742 ms`** | `13.108 ms` (`35.2%`) | `19.522 ms` | `44.998 ms` | `64.845 ms` | **`1.10x Bun` (`11.21x` Warm) / `3.66x Node`** |
| **CS-05** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`18.002 ms`** (`16.73 ms` Min) | **`18.840 ms`** | **`11.847 ms`** | `18.270 ms` (`-1.5%`) | `19.388 ms` | `62.440 ms` | `82.156 ms` | **`1.08x Bun` (`1.64x` Warm) / `4.56x Node`** |

---

## 4. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Workload | Slice (`SLICEBC\x01` Default) | Bun (`v1.4.4`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha.0.0.0`) | Slice Memory Advantage |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **App 2: NPM semver+ms+ini** | **13,952 KB (13.63 MB)** | 29,952 KB (29.25 MB) | 47,444 KB (46.33 MB) | 71,448 KB (69.77 MB) | **5.12x less than Node / 2.15x less than Bun** |
| **App 6: Telemetry + Flatted** | **14,416 KB (14.08 MB)** | 29,824 KB (29.13 MB) | 50,784 KB (49.59 MB) | 72,748 KB (71.04 MB) | **5.05x less than Node / 2.07x less than Bun** |
| **App 21: RFC 6455 WS + Sec** | **13,824 KB (13.50 MB)** | 31,848 KB (31.10 MB) | 50,672 KB (49.48 MB) | 94,980 KB (92.75 MB) | **6.87x less than Node / 2.30x less than Bun** |
| **App 22: Workers + Sockets** | **22,144 KB (21.63 MB)** | 41,788 KB (40.81 MB) | 63,440 KB (61.95 MB) | 93,836 KB (91.64 MB) | **4.24x less than Node / 1.89x less than Bun** |
| **App 23: Pure-Rust WASM** | **18,816 KB (18.38 MB)** | 30,592 KB (29.88 MB) | 48,788 KB (47.64 MB) | 80,404 KB (78.52 MB) | **4.27x less than Node / 1.63x less than Bun** |
| **FW 1: Hono v4 Microservice** | **14,976 KB (14.63 MB)** | 30,556 KB (29.84 MB) | 49,392 KB (48.23 MB) | 89,372 KB (87.28 MB) | **5.97x less than Node / 2.04x less than Bun** |
| **FW 2: itty-router + UnJS** | **14,592 KB (14.25 MB)** | 31,872 KB (31.13 MB) | 49,056 KB (47.91 MB) | 87,048 KB (85.01 MB) | **5.97x less than Node / 2.18x less than Bun** |
| **App 24: WASM + SSR** | **13,824 KB (13.50 MB)** | 30,720 KB (30.00 MB) | 48,944 KB (47.80 MB) | 81,148 KB (79.25 MB) | **5.87x less than Node / 2.22x less than Bun** |
| **FW 3: Express node:http** | **15,232 KB (14.88 MB)** | 33,408 KB (32.63 MB) | 46,832 KB (45.73 MB) | 80,652 KB (78.76 MB) | **5.29x less than Node / 2.19x less than Bun** |
| **01-tot-autonomous-sre-agent** | **18,432 KB (18.00 MB)** | 32,104 KB (31.35 MB) | 52,968 KB (51.73 MB) | 83,720 KB (81.76 MB) | **4.54x less than Node / 1.74x less than Bun** |
| **02-mcp-code-mode-orchestrator** | **18,028 KB (17.61 MB)** | 28,032 KB (27.38 MB) | 46,108 KB (45.03 MB) | 80,644 KB (78.75 MB) | **4.47x less than Node / 1.55x less than Bun** |
| **03-multi-tenant-saas-plugin-mesh** | **15,360 KB (15.00 MB)** | 26,624 KB (26.00 MB) | 49,624 KB (48.46 MB) | 88,068 KB (86.00 MB) | **5.73x less than Node / 1.73x less than Bun** |
| **04-wasm-zero-trust-cdr-pipeline** | **19,968 KB (19.50 MB)** | 31,360 KB (30.63 MB) | 49,808 KB (48.64 MB) | 80,260 KB (78.38 MB) | **4.02x less than Node / 1.57x less than Bun** |
| **05-time-travel-chaos-fuzzer-payments** | **16,000 KB (15.63 MB)** | 29,184 KB (28.50 MB) | 49,272 KB (48.12 MB) | 81,572 KB (79.66 MB) | **5.10x less than Node / 1.82x less than Bun** |

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
