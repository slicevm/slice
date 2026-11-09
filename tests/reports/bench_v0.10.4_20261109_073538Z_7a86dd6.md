# Slice (Nexus-JS) — Full 32-Workload & MicroVM Benchmark Report

* **Slice Version:** `v0.10.4` (Commit `7a86dd6`)
* **Generated Timestamp (UTC):** `2026-11-09T07:35:38Z`
* **Archive File:** [`tests/reports/bench_v0.10.4_20261109_073538Z_7a86dd6.md`](./bench_v0.10.4_20261109_073538Z_7a86dd6.md)
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
| **Single Isolate Cold-Start (Min / P50 / P99)** | **`1.17 us` / `1.24 us` / `1.61 us`** | **`1.40 us` / `1.60 us` / `2.30 us`** | **~120,000x faster than 125ms Firecracker VM** |
| **1,000-Tenant MicroVM Swarm (Boot + Exec)** | **`1.51 ms`** (`10.07 MB` RSS) | **`2.24 ms`** (`11.82 MB` RSS) | **> 10,000x faster than container/VM swarms** |
| **500-Module Monorepo Cold Boot (`0 stat()`)** | **`3.408 ms`** (`394.90 us` in-mem) | **`5.637 ms`** (`640.00 us` in-mem) | **20.6x faster than Node.js (`1,500+ stat()`)** |
| **10,000 Direct Kernel Ring I/O Completions** | **`0.02 ms`** | **`5.11 ms`** | **Zero worker-thread lock contention** |
| **5-Ring Security Containment Verification** | **`5 / 5 PASS`** | **`5 / 5 PASS`** | **OCap + SES + Fuel + SSRF/DLP + MPK** |

---

## 2. `slice serve` Multi-Tenant HTTP/1.1 Keep-Alive Throughput (`16,000` Reqs, 8 Sockets x 8 Pipeline)

| Server / Runtime | Throughput (`req/s`) | Avg Latency / Req | Total Time (`16k` Reqs) | Relative Speedup vs Runtimes |
| :--- | :---: | :---: | :---: | :--- |
| **`slice serve` (Tier-0 CAS, `--workers=8`)** | **`148,407 req/s`** | **`6.74 us/req`** | **`107.8 ms`** | **`1.51x` faster than `Bun.serve` / `2.65x` Node.js** |
| **`slice serve` (Tier-1 WinterTC, `--workers=8`)** | **`145,839 req/s`** | **`6.86 us/req`** | **`109.7 ms`** | **`1.49x` faster than `Bun.serve` / `2.61x` Node.js** |
| **`Deno.serve` (`v2.9.8`, V8, No Per-Req Sandbox)** | `100,099 req/s` | `9.99 us/req` | `159.8 ms` | `1.02x` vs Bun |
| **`Bun.serve` (`v1.4.4`, JSC, No Per-Req Sandbox)** | `97,987 req/s` | `10.21 us/req` | `163.3 ms` | `1.00x` (Baseline) |
| **`Node.js http.createServer` (`v27.0.0-alpha.0.0.0`, V8)** | `55,977 req/s` | `17.86 us/req` | `285.8 ms` | `0.57x` vs Bun |

---

## 3. Full 32-Workload Latency Comparison (20 Runs Each, Linux `x86_64` Docker)

| # | Workload (`tests/apps/`, `tests/frameworks/`, `tests/case_studies/`) | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm (`--warm`) | Previous `.cas` (`Delta`) | Bun `v1.4.4` | Deno `v2.9.8` | Node.js `v27.0.0-alpha.0.0.0` | Slice Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`6.206 ms`** (`5.74 ms` Min) | **`6.836 ms`** | **`1.168 ms`** | `6.209 ms` (`-0.0%`) | `14.399 ms` | `38.716 ms` | `59.167 ms` | **`2.32x Bun` (`12.33x` Warm) / `9.53x Node`** |
| **02** | `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | **`14.659 ms`** (`13.67 ms` Min) | **`15.257 ms`** | **`3.989 ms`** | `14.111 ms` (`3.9%`) | `21.031 ms` | `54.896 ms` | `72.423 ms` | **`1.43x Bun` (`5.27x` Warm) / `4.94x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`8.036 ms`** (`7.42 ms` Min) | **`8.745 ms`** | **`2.175 ms`** | `7.819 ms` (`2.8%`) | `14.427 ms` | `45.774 ms` | `63.282 ms` | **`1.80x Bun` (`6.63x` Warm) / `7.87x Node`** |
| **04** | `04-edge-api-gateway` (`fast-querystring` + `ms`) | **`7.850 ms`** (`7.22 ms` Min) | **`8.498 ms`** | **`2.193 ms`** | `7.940 ms` (`-1.1%`) | `15.712 ms` | `45.705 ms` | `65.196 ms` | **`2.00x Bun` (`7.16x` Warm) / `8.31x Node`** |
| **05** | `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | **`10.654 ms`** (`10.03 ms` Min) | **`11.471 ms`** | **`5.686 ms`** | `10.820 ms` (`-1.5%`) | `15.477 ms` | `45.984 ms` | `65.979 ms` | **`1.45x Bun` (`2.72x` Warm) / `6.19x Node`** |
| **06** | `06-telemetry-metrics-pipeline` (`bytes` + `flatted` + WAL) | **`14.519 ms`** (`13.70 ms` Min) | **`15.347 ms`** | **`5.943 ms`** | `14.531 ms` (`-0.1%`) | `17.760 ms` | `48.005 ms` | `66.981 ms` | **`1.22x Bun` (`2.99x` Warm) / `4.61x Node`** |
| **07** | `07-graphql-ast-validator` (`dlv` + `dset` + AST Cost) | **`9.992 ms`** (`8.98 ms` Min) | **`10.286 ms`** | **`4.429 ms`** | `9.236 ms` (`8.2%`) | `13.532 ms` | `53.519 ms` | `62.876 ms` | **`1.35x Bun` (`3.06x` Warm) / `6.29x Node`** |
| **08** | `08-event-driven-state-journal` (`mitt` + `ms` + JSONL Replay) | **`13.568 ms`** (`11.39 ms` Min) | **`12.399 ms`** | **`6.140 ms`** | `11.756 ms` (`15.4%`) | `16.462 ms` | `49.933 ms` | `66.841 ms` | **`1.33x Bun` (`2.68x` Warm) / `5.39x Node`** |
| **09** | `09-wintertc-web-router` (`Request` + `Response` + `URL`, 80 reqs) | **`15.476 ms`** (`14.26 ms` Min) | **`15.679 ms`** | **`10.881 ms`** | `14.792 ms` (`4.6%`) | `14.242 ms` | `49.197 ms` | `88.955 ms` | **`1.31x Bun Warm` / `5.75x Node`** |
| **10** | `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **`13.998 ms`** (`13.03 ms` Min) | **`13.960 ms`** | **`7.199 ms`** | `14.266 ms` (`-1.9%`) | `14.723 ms` | `50.512 ms` | `87.655 ms` | **`1.05x Bun` (`2.05x` Warm) / `6.28x Node`** |
| **11** | `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **`10.268 ms`** (`9.54 ms` Min) | **`10.693 ms`** | **`4.430 ms`** | `9.957 ms` (`3.1%`) | `14.581 ms` | `54.186 ms` | `88.489 ms` | **`1.42x Bun` (`3.29x` Warm) / `8.62x Node`** |
| **12** | `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP, 320 frames) | **`16.665 ms`** (`15.51 ms` Min) | **`17.562 ms`** | **`9.427 ms`** | `16.313 ms` (`2.2%`) | `15.962 ms` | `52.085 ms` | `92.752 ms` | **`1.69x Bun Warm` / `5.57x Node`** |
| **13** | `13-webhook-orchestrator-ssrf` (`fast-querystring` + `ms` + Live `fetch`) | **`14.135 ms`** (`13.59 ms` Min) | **`15.028 ms`** | **`6.475 ms`** | `13.841 ms` (`2.1%`) | `21.100 ms` | `53.692 ms` | `115.809 ms` | **`1.49x Bun` (`3.26x` Warm) / `8.19x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`15.515 ms`** (`14.69 ms` Min) | **`16.745 ms`** | **`7.867 ms`** | `14.809 ms` (`4.8%`) | `22.376 ms` | `56.663 ms` | `112.022 ms` | **`1.44x Bun` (`2.84x` Warm) / `7.22x Node`** |
| **15** | `15-time-travel-test-suite` (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **`15.345 ms`** (`14.57 ms` Min) | **`16.621 ms`** | **`5.088 ms`** | `15.122 ms` (`1.5%`) | `25.308 ms` | `68.122 ms` | `84.543 ms` | **`1.65x Bun` (`4.97x` Warm) / `5.51x Node`** |
| **16** | `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **`7.716 ms`** (`7.21 ms` Min) | **`8.187 ms`** | **`1.686 ms`** | `7.469 ms` (`3.3%`) | `15.118 ms` | `48.508 ms` | `66.922 ms` | **`1.96x Bun` (`8.97x` Warm) / `8.67x Node`** |
| **17** | `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive/307/SSRF/DLP) | **`16.359 ms`** (`15.52 ms` Min) | **`16.874 ms`** | **`8.301 ms`** | `16.200 ms` (`1.0%`) | `22.977 ms` | `60.584 ms` | `118.344 ms` | **`1.40x Bun` (`2.77x` Warm) / `7.23x Node`** |
| **18** | `18-async-event-loop-fanout-scheduler` (Timer Wheel/`Promise.all`/`AbortSignal`) | **`31.543 ms`** (`29.50 ms` Min) | **`30.834 ms`** | **`17.928 ms`** | `29.421 ms` (`7.2%`) | `32.957 ms` | `72.222 ms` | `84.990 ms` | **`1.07x Bun` (`1.84x` Warm) / `2.76x Node`** |
| **19** | `19-wintertc-webcrypto-streams-edge` (WebCrypto/Streams/FormData/Clone) | **`8.353 ms`** (`7.81 ms` Min) | **`9.008 ms`** | **`1.744 ms`** | `8.191 ms` (`2.0%`) | `14.072 ms` | `46.331 ms` | `89.299 ms` | **`1.68x Bun` (`8.07x` Warm) / `10.69x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`11.502 ms`** (`10.73 ms` Min) | **`11.919 ms`** | **`0.545 ms`** | `11.296 ms` (`1.8%`) | `22.558 ms` | `55.771 ms` | `71.858 ms` | **`1.96x Bun` (`41.39x` Warm) / `6.25x Node`** |
| **21** | `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket` + 5-Ring Guard) | **`10.803 ms`** (`10.15 ms` Min) | **`11.980 ms`** | **`2.872 ms`** | `10.553 ms` (`2.4%`) | `20.404 ms` | `53.926 ms` | `119.323 ms` | **`1.89x Bun` (`7.10x` Warm) / `11.05x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`21.520 ms`** (`19.88 ms` Min) | **`21.161 ms`** | **`2.217 ms`** | `20.938 ms` (`2.8%`) | `57.852 ms` | `113.393 ms` | `103.236 ms` | **`2.73x Bun` (`26.09x` Warm) / `4.88x Node`** |
| **23** | `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **`15.912 ms`** (`14.82 ms` Min) | **`15.965 ms`** | **`1.955 ms`** | `15.069 ms` (`5.6%`) | `21.598 ms` | `42.081 ms` | `61.413 ms` | **`1.36x Bun` (`11.05x` Warm) / `3.86x Node`** |
| **24** | `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` SLA Scorer + SSR Pipeline) | **`8.927 ms`** (`8.48 ms` Min) | **`9.257 ms`** | **`1.824 ms`** | `8.419 ms` (`6.0%`) | `19.246 ms` | `43.391 ms` | `69.066 ms` | **`2.16x Bun` (`10.55x` Warm) / `7.74x Node`** |
| **FW-01** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`15.318 ms`** (`14.30 ms` Min) | **`16.199 ms`** | **`6.483 ms`** | `15.140 ms` (`1.2%`) | `20.415 ms` | `54.052 ms` | `96.960 ms` | **`1.33x Bun` (`3.15x` Warm) / `6.33x Node`** |
| **FW-02** | `FW-02: frameworks/02-itty-router-unjs-nitro-stack` (`itty-router` + UnJS) | **`18.321 ms`** (`17.05 ms` Min) | **`18.916 ms`** | **`8.790 ms`** | `17.873 ms` (`2.5%`) | `20.104 ms` | `47.450 ms` | `89.334 ms` | **`1.10x Bun` (`2.29x` Warm) / `4.88x Node`** |
| **FW-03** | `FW-03: frameworks/03-node-http-express-compat` (`node:http` + `node:stream` Express/Polka) | **`16.913 ms`** (`15.81 ms` Min) | **`17.379 ms`** | **`1.070 ms`** | `16.410 ms` (`3.1%`) | `35.105 ms` | `50.537 ms` | `86.593 ms` | **`2.08x Bun` (`32.81x` Warm) / `5.12x Node`** |
| **CS-01** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`26.485 ms`** (`25.52 ms` Min) | **`28.096 ms`** | **`13.120 ms`** | `26.690 ms` (`-0.8%`) | `28.256 ms` | `72.647 ms` | `89.711 ms` | **`1.07x Bun` (`2.15x` Warm) / `3.39x Node`** |
| **CS-02** | `CS-02: case_studies/02-mcp-code-mode-orchestrator` (`-98.4%` Tokens) | **`20.362 ms`** (`19.29 ms` Min) | **`21.082 ms`** | **`7.854 ms`** | `19.966 ms` (`2.0%`) | `16.230 ms` | `44.344 ms` | `66.894 ms` | **`2.07x Bun Warm` / `3.29x Node`** |
| **CS-03** | `CS-03: case_studies/03-multi-tenant-saas-plugin-mesh` (12 Tenants + Hot-Swap) | **`13.984 ms`** (`12.95 ms` Min) | **`14.000 ms`** | **`6.270 ms`** | `13.726 ms` (`1.9%`) | `16.305 ms` | `49.236 ms` | `84.935 ms` | **`1.17x Bun` (`2.60x` Warm) / `6.07x Node`** |
| **CS-04** | `CS-04: case_studies/04-wasm-zero-trust-cdr-pipeline` (Wasm SimHash CDR) | **`17.514 ms`** (`16.97 ms` Min) | **`18.387 ms`** | **`1.807 ms`** | `17.723 ms` (`-1.2%`) | `19.807 ms` | `45.755 ms` | `68.820 ms` | **`1.13x Bun` (`10.96x` Warm) / `3.93x Node`** |
| **CS-05** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`18.127 ms`** (`17.30 ms` Min) | **`18.959 ms`** | **`10.313 ms`** | `18.002 ms` (`0.7%`) | `19.616 ms` | `62.942 ms` | `83.703 ms` | **`1.08x Bun` (`1.90x` Warm) / `4.62x Node`** |

---

## 4. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Workload | Slice (`SLICEBC\x01` Default) | Bun (`v1.4.4`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha.0.0.0`) | Slice Memory Advantage |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **App 2: NPM semver+ms+ini** | **14,208 KB (13.88 MB)** | 30,080 KB (29.38 MB) | 47,100 KB (46.00 MB) | 71,576 KB (69.90 MB) | **5.04x less than Node / 2.12x less than Bun** |
| **App 6: Telemetry + Flatted** | **14,592 KB (14.25 MB)** | 30,592 KB (29.88 MB) | 50,780 KB (49.59 MB) | 72,988 KB (71.28 MB) | **5.00x less than Node / 2.10x less than Bun** |
| **App 21: RFC 6455 WS + Sec** | **13,952 KB (13.63 MB)** | 32,088 KB (31.34 MB) | 50,848 KB (49.66 MB) | 94,724 KB (92.50 MB) | **6.79x less than Node / 2.30x less than Bun** |
| **App 22: Workers + Sockets** | **22,016 KB (21.50 MB)** | 41,568 KB (40.59 MB) | 62,868 KB (61.39 MB) | 94,028 KB (91.82 MB) | **4.27x less than Node / 1.89x less than Bun** |
| **App 23: Pure-Rust WASM** | **18,944 KB (18.50 MB)** | 30,976 KB (30.25 MB) | 48,924 KB (47.78 MB) | 80,660 KB (78.77 MB) | **4.26x less than Node / 1.64x less than Bun** |
| **FW 1: Hono v4 Microservice** | **15,232 KB (14.88 MB)** | 31,224 KB (30.49 MB) | 49,436 KB (48.28 MB) | 89,116 KB (87.03 MB) | **5.85x less than Node / 2.05x less than Bun** |
| **FW 2: itty-router + UnJS** | **14,592 KB (14.25 MB)** | 32,128 KB (31.38 MB) | 48,544 KB (47.41 MB) | 87,180 KB (85.14 MB) | **5.97x less than Node / 2.20x less than Bun** |
| **App 24: WASM + SSR** | **13,696 KB (13.38 MB)** | 32,256 KB (31.50 MB) | 48,544 KB (47.41 MB) | 81,152 KB (79.25 MB) | **5.93x less than Node / 2.36x less than Bun** |
| **FW 3: Express node:http** | **14,976 KB (14.63 MB)** | 33,152 KB (32.38 MB) | 46,956 KB (45.86 MB) | 80,136 KB (78.26 MB) | **5.35x less than Node / 2.21x less than Bun** |
| **01-tot-autonomous-sre-agent** | **18,560 KB (18.13 MB)** | 31,744 KB (31.00 MB) | 52,980 KB (51.74 MB) | 86,016 KB (84.00 MB) | **4.63x less than Node / 1.71x less than Bun** |
| **02-mcp-code-mode-orchestrator** | **18,112 KB (17.69 MB)** | 27,904 KB (27.25 MB) | 47,068 KB (45.96 MB) | 80,648 KB (78.76 MB) | **4.45x less than Node / 1.54x less than Bun** |
| **03-multi-tenant-saas-plugin-mesh** | **15,744 KB (15.38 MB)** | 28,672 KB (28.00 MB) | 49,480 KB (48.32 MB) | 87,940 KB (85.88 MB) | **5.59x less than Node / 1.82x less than Bun** |
| **04-wasm-zero-trust-cdr-pipeline** | **19,968 KB (19.50 MB)** | 31,104 KB (30.38 MB) | 49,692 KB (48.53 MB) | 80,136 KB (78.26 MB) | **4.01x less than Node / 1.56x less than Bun** |
| **05-time-travel-chaos-fuzzer-payments** | **15,744 KB (15.38 MB)** | 29,184 KB (28.50 MB) | 49,780 KB (48.61 MB) | 81,440 KB (79.53 MB) | **5.17x less than Node / 1.85x less than Bun** |

---

## 5. Multi-Tenant Edge KV, AI Agent Checkpoints & 5-Ring Security Telemetry

| Subsystem / Benchmark | Measured Telemetry | Status |
| :--- | :--- | :---: |
| **Sharded `Slice.kv` (`1,001` Get/Set/Incr Ops in Isolate)** | **`644.66 us`** total (`--warm`, `8,431 B` memory cage) | `PASS` |
| **Multi-Tenant `--tenant-dir` Zero-Downtime Hot-Swap** | Live atomic replacement of `tenant-alpha.cas` (`v1 -> v2-hot-swapped`) with isolated `Slice.kv` state | `PASS` |
| **Deterministic Time-Travel Replay (`--deterministic=20261019`)** | 100% bit-for-bit identical PRNG (`Math.random`) & virtual clock (`Date.now`, `performance.now`) | `PASS` |
| **Tree-of-Thought AI Agent (`Slice.checkpoint` + 3 `evalSandbox` + `restore`)** | **`55.02 us`** total (`1.77 us` child sandbox) | `PASS` |
| **Cloud Metadata SSRF (`169.254.169.254`) & Base64 Egress DLP Guard** | `SSRF_METADATA_BLOCKED` & `EGRESS_SECRET_LEAK_BLOCKED` deterministically trapped | `PASS` |
| **Built-In Model Context Protocol Server (`slice mcp --smoke-test`)** | JSON-RPC 2.0 `initialize`, `tools/list`, `tools/call` verified over `stdio` | `PASS` |
