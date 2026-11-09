# Slice (Nexus-JS) — Full 33-Workload & MicroVM Benchmark Report

* **Slice Version:** `v0.10.5` (Commit `47bfa6b`)
* **Generated Timestamp (UTC):** `2026-11-09T22:52:49Z`
* **Archive File:** [`tests/reports/bench_v0.10.5_20261109_225249Z_47bfa6b.md`](./bench_v0.10.5_20261109_225249Z_47bfa6b.md)
* **Canonical Latest Report:** [`tests/reports/latest.md`](./latest.md)
* **Environment:** `Linux x86_64 Docker (io_uring SQPOLL + WRPKRU)`
* **Compared Runtimes:**
  * **Slice:** `v0.10.5` (Default `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS + Monomorphic Inline Caching `get_fast` + Streaming `JsonSeed` `JSON.parse`)
  * **Bun:** `v1.4.4` (JavaScriptCore)
  * **Deno:** `v2.9.8` (V8)
  * **Node.js:** `v27.0.0-alpha.0.0.0` (V8 native TypeScript execution)

---

## 1. Executive Summary

Across all **33 real-world multi-file TypeScript applications, framework suites, and case studies**, Slice beats **Deno (`v2.9.8`)** on **33/33** workloads, beats **Node.js (`v27.0.0-alpha.0.0.0`)** on **33/33** workloads, beats **Bun (`v1.4.4`)** on **30/33** workloads in cold one-shot execution, and beats **all 3 runtimes on 33/33 workloads in `--warm` pooled isolate execution**.

| Metric | Linux (`x86_64` Docker) | Windows (`x86_64` Host) | Comparison vs Runtimes & MicroVMs |
| :--- | :---: | :---: | :--- |
| **Single Isolate Cold-Start (Min / P50 / P99)** | **`0.96 us` / `1.05 us` / `1.80 us`** | **`1.40 us` / `1.60 us` / `2.30 us`** | **~120,000x faster than 125ms Firecracker VM** |
| **1,000-Tenant MicroVM Swarm (Boot + Exec)** | **`1.37 ms`** (`9.70 MB` RSS) | **`2.24 ms`** (`11.82 MB` RSS) | **> 10,000x faster than container/VM swarms** |
| **500-Module Monorepo Cold Boot (`0 stat()`)** | **`3.408 ms`** (`542.85 us` in-mem) | **`5.637 ms`** (`640.00 us` in-mem) | **20.6x faster than Node.js (`1,500+ stat()`)** |
| **10,000 Direct Kernel Ring I/O Completions** | **`0.02 ms`** | **`5.11 ms`** | **Zero worker-thread lock contention** |
| **5-Ring Security Containment Verification** | **`5 / 5 PASS`** | **`5 / 5 PASS`** | **OCap + SES + Fuel + SSRF/DLP + MPK** |

---

## 2. `slice serve` Multi-Tenant HTTP/1.1 Keep-Alive Throughput (`16,000` Reqs, 8 Sockets x 8 Pipeline)

| Server / Runtime | Throughput (`req/s`) | Avg Latency / Req | Total Time (`16k` Reqs) | Relative Speedup vs Runtimes |
| :--- | :---: | :---: | :---: | :--- |
| **`slice serve` (Tier-0 CAS, `--workers=8`)** | **`137,020 req/s`** | **`7.30 us/req`** | **`116.8 ms`** | **`1.88x` faster than `Bun.serve` / `2.99x` Node.js** |
| **`slice serve` (Tier-1 WinterTC, `--workers=8`)** | **`125,831 req/s`** | **`7.95 us/req`** | **`127.2 ms`** | **`1.73x` faster than `Bun.serve` / `2.74x` Node.js** |
| **`Deno.serve` (`v2.9.8`, V8, No Per-Req Sandbox)** | `87,455 req/s` | `11.43 us/req` | `183.0 ms` | `1.20x` vs Bun |
| **`Bun.serve` (`v1.4.4`, JSC, No Per-Req Sandbox)** | `72,924 req/s` | `13.71 us/req` | `219.4 ms` | `1.00x` (Baseline) |
| **`Node.js http.createServer` (`v27.0.0-alpha.0.0.0`, V8)** | `45,868 req/s` | `21.80 us/req` | `348.8 ms` | `0.63x` vs Bun |

---

## 3. Full 33-Workload Latency Comparison (20 Runs Each, Linux `x86_64` Docker)

| # | Workload (`tests/apps/`, `tests/frameworks/`, `tests/case_studies/`) | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm (`--warm`) | Previous `.cas` (`Delta`) | Bun `v1.4.4` | Deno `v2.9.8` | Node.js `v27.0.0-alpha.0.0.0` | Slice Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`6.549 ms`** (`5.96 ms` Min) | **`8.093 ms`** | **`1.934 ms`** | `6.206 ms` (`5.5%`) | `15.167 ms` | `42.607 ms` | `74.220 ms` | **`2.32x Bun` (`7.84x` Warm) / `11.33x Node`** |
| **02** | `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | **`16.553 ms`** (`14.49 ms` Min) | **`16.721 ms`** | **`4.790 ms`** | `14.659 ms` (`12.9%`) | `24.251 ms` | `59.826 ms` | `81.827 ms` | **`1.47x Bun` (`5.06x` Warm) / `4.94x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`7.941 ms`** (`7.02 ms` Min) | **`8.422 ms`** | **`1.613 ms`** | `8.036 ms` (`-1.2%`) | `17.735 ms` | `50.096 ms` | `70.811 ms` | **`2.23x Bun` (`11.00x` Warm) / `8.92x Node`** |
| **04** | `04-edge-api-gateway` (`fast-querystring` + `ms`) | **`7.827 ms`** (`7.15 ms` Min) | **`8.669 ms`** | **`1.769 ms`** | `7.850 ms` (`-0.3%`) | `16.480 ms` | `49.015 ms` | `72.202 ms` | **`2.11x Bun` (`9.32x` Warm) / `9.22x Node`** |
| **05** | `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | **`11.143 ms`** (`10.11 ms` Min) | **`11.926 ms`** | **`8.203 ms`** | `10.654 ms` (`4.6%`) | `15.814 ms` | `51.773 ms` | `76.002 ms` | **`1.42x Bun` (`1.93x` Warm) / `6.82x Node`** |
| **06** | `06-telemetry-metrics-pipeline` (`bytes` + `flatted` + WAL) | **`16.076 ms`** (`14.79 ms` Min) | **`17.028 ms`** | **`6.338 ms`** | `14.519 ms` (`10.7%`) | `19.737 ms` | `56.963 ms` | `71.608 ms` | **`1.23x Bun` (`3.11x` Warm) / `4.45x Node`** |
| **07** | `07-graphql-ast-validator` (`dlv` + `dset` + AST Cost) | **`11.115 ms`** (`8.79 ms` Min) | **`10.370 ms`** | **`4.102 ms`** | `9.992 ms` (`11.2%`) | `15.024 ms` | `46.462 ms` | `68.656 ms` | **`1.45x Bun` (`3.66x` Warm) / `6.62x Node`** |
| **08** | `08-event-driven-state-journal` (`mitt` + `ms` + JSONL Replay) | **`12.471 ms`** (`11.00 ms` Min) | **`12.646 ms`** | **`5.602 ms`** | `13.568 ms` (`-8.1%`) | `17.725 ms` | `53.924 ms` | `71.640 ms` | **`1.42x Bun` (`3.16x` Warm) / `5.74x Node`** |
| **09** | `09-wintertc-web-router` (`Request` + `Response` + `URL`, 80 reqs) | **`16.900 ms`** (`15.01 ms` Min) | **`17.946 ms`** | **`11.562 ms`** | `15.476 ms` (`9.2%`) | `14.796 ms` | `57.896 ms` | `96.743 ms` | **`1.28x Bun Warm` / `5.72x Node`** |
| **10** | `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **`14.257 ms`** (`12.89 ms` Min) | **`16.402 ms`** | **`11.179 ms`** | `13.998 ms` (`1.9%`) | `18.103 ms` | `64.257 ms` | `101.196 ms` | **`1.27x Bun` (`1.62x` Warm) / `7.10x Node`** |
| **11** | `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **`11.158 ms`** (`10.52 ms` Min) | **`12.072 ms`** | **`4.154 ms`** | `10.268 ms` (`8.7%`) | `15.225 ms` | `56.518 ms` | `101.741 ms` | **`1.36x Bun` (`3.67x` Warm) / `9.12x Node`** |
| **12** | `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP, 320 frames) | **`18.478 ms`** (`15.37 ms` Min) | **`19.011 ms`** | **`8.424 ms`** | `16.665 ms` (`10.9%`) | `17.424 ms` | `56.409 ms` | `111.849 ms` | **`2.07x Bun Warm` / `6.05x Node`** |
| **13** | `13-webhook-orchestrator-ssrf` (`fast-querystring` + `ms` + Live `fetch`) | **`15.312 ms`** (`13.94 ms` Min) | **`15.857 ms`** | **`7.041 ms`** | `14.135 ms` (`8.3%`) | `25.142 ms` | `60.842 ms` | `132.603 ms` | **`1.64x Bun` (`3.57x` Warm) / `8.66x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`15.929 ms`** (`14.68 ms` Min) | **`16.196 ms`** | **`7.802 ms`** | `15.515 ms` (`2.7%`) | `24.460 ms` | `61.587 ms` | `122.583 ms` | **`1.54x Bun` (`3.14x` Warm) / `7.70x Node`** |
| **15** | `15-time-travel-test-suite` (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **`16.968 ms`** (`15.10 ms` Min) | **`17.549 ms`** | **`5.987 ms`** | `15.345 ms` (`10.6%`) | `28.355 ms` | `77.460 ms` | `95.857 ms` | **`1.67x Bun` (`4.74x` Warm) / `5.65x Node`** |
| **16** | `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **`8.364 ms`** (`7.37 ms` Min) | **`9.346 ms`** | **`1.280 ms`** | `7.716 ms` (`8.4%`) | `16.161 ms` | `54.266 ms` | `75.761 ms` | **`1.93x Bun` (`12.63x` Warm) / `9.06x Node`** |
| **17** | `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive/307/SSRF/DLP) | **`17.319 ms`** (`16.27 ms` Min) | **`18.538 ms`** | **`9.241 ms`** | `16.359 ms` (`5.9%`) | `25.921 ms` | `65.348 ms` | `132.340 ms` | **`1.50x Bun` (`2.80x` Warm) / `7.64x Node`** |
| **18** | `18-async-event-loop-fanout-scheduler` (Timer Wheel/`Promise.all`/`AbortSignal`) | **`30.986 ms`** (`28.49 ms` Min) | **`33.993 ms`** | **`17.717 ms`** | `31.543 ms` (`-1.8%`) | `37.518 ms` | `81.154 ms` | `94.107 ms` | **`1.21x Bun` (`2.12x` Warm) / `3.04x Node`** |
| **19** | `19-wintertc-webcrypto-streams-edge` (WebCrypto/Streams/FormData/Clone) | **`9.453 ms`** (`8.34 ms` Min) | **`9.903 ms`** | **`1.477 ms`** | `8.353 ms` (`13.2%`) | `15.328 ms` | `53.271 ms` | `102.717 ms` | **`1.62x Bun` (`10.38x` Warm) / `10.87x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`12.304 ms`** (`10.57 ms` Min) | **`12.468 ms`** | **`0.888 ms`** | `11.502 ms` (`7.0%`) | `25.969 ms` | `64.068 ms` | `88.273 ms` | **`2.11x Bun` (`29.24x` Warm) / `7.17x Node`** |
| **21** | `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket` + 5-Ring Guard) | **`11.666 ms`** (`10.39 ms` Min) | **`12.224 ms`** | **`2.803 ms`** | `10.803 ms` (`8.0%`) | `24.293 ms` | `60.684 ms` | `135.299 ms` | **`2.08x Bun` (`8.67x` Warm) / `11.60x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`24.541 ms`** (`21.21 ms` Min) | **`23.822 ms`** | **`2.377 ms`** | `21.520 ms` (`14.0%`) | `68.043 ms` | `134.578 ms` | `116.660 ms` | **`2.86x Bun` (`28.63x` Warm) / `4.90x Node`** |
| **23** | `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **`17.830 ms`** (`15.16 ms` Min) | **`18.181 ms`** | **`2.088 ms`** | `15.912 ms` (`12.1%`) | `24.671 ms` | `49.186 ms` | `78.258 ms` | **`1.38x Bun` (`11.82x` Warm) / `4.39x Node`** |
| **24** | `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` SLA Scorer + SSR Pipeline) | **`9.438 ms`** (`8.21 ms` Min) | **`9.628 ms`** | **`1.751 ms`** | `8.927 ms` (`5.7%`) | `21.746 ms` | `51.375 ms` | `78.308 ms` | **`2.30x Bun` (`12.42x` Warm) / `8.30x Node`** |
| **FW-01** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`16.903 ms`** (`14.97 ms` Min) | **`17.879 ms`** | **`7.535 ms`** | `16.071 ms` (`5.2%`) | `22.254 ms` | `61.183 ms` | `110.858 ms` | **`1.32x Bun` (`2.95x` Warm) / `6.56x Node`** |
| **FW-02** | `FW-02: frameworks/02-itty-router-unjs-nitro-stack` (`itty-router` + UnJS) | **`19.999 ms`** (`17.74 ms` Min) | **`20.653 ms`** | **`10.506 ms`** | `19.468 ms` (`2.7%`) | `21.391 ms` | `53.565 ms` | `100.970 ms` | **`1.07x Bun` (`2.04x` Warm) / `5.05x Node`** |
| **FW-03** | `FW-03: frameworks/03-node-http-express-compat` (`node:http` + `node:stream` Express/Polka) | **`18.850 ms`** (`16.78 ms` Min) | **`19.326 ms`** | **`1.399 ms`** | `16.830 ms` (`12.0%`) | `41.711 ms` | `60.165 ms` | `99.962 ms` | **`2.21x Bun` (`29.81x` Warm) / `5.30x Node`** |
| **FW-04** | `FW-04: frameworks/04-sveltekit-edge-ssr` (`@sveltejs/kit@3.1.1` + `svelte@5.57.3` + `devalue@6.0.3`) | **`39.319 ms`** (`34.46 ms` Min) | **`40.783 ms`** | **`19.712 ms`** | `36.160 ms` (`8.7%`) | `52.502 ms` | `84.299 ms` | `133.012 ms` | **`1.34x Bun` (`2.66x` Warm) / `3.38x Node`** |
| **CS-01** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`31.107 ms`** (`26.75 ms` Min) | **`31.878 ms`** | **`13.126 ms`** | `26.485 ms` (`17.5%`) | `32.776 ms` | `86.411 ms` | `105.353 ms` | **`1.05x Bun` (`2.50x` Warm) / `3.39x Node`** |
| **CS-02** | `CS-02: case_studies/02-mcp-code-mode-orchestrator` (`-98.4%` Tokens) | **`20.659 ms`** (`18.43 ms` Min) | **`21.371 ms`** | **`6.569 ms`** | `20.362 ms` (`1.5%`) | `18.019 ms` | `52.128 ms` | `79.359 ms` | **`2.74x Bun Warm` / `3.84x Node`** |
| **CS-03** | `CS-03: case_studies/03-multi-tenant-saas-plugin-mesh` (12 Tenants + Hot-Swap) | **`15.332 ms`** (`13.56 ms` Min) | **`15.796 ms`** | **`5.721 ms`** | `13.984 ms` (`9.6%`) | `18.869 ms` | `57.047 ms` | `98.424 ms` | **`1.23x Bun` (`3.30x` Warm) / `6.42x Node`** |
| **CS-04** | `CS-04: case_studies/04-wasm-zero-trust-cdr-pipeline` (Wasm SimHash CDR) | **`19.955 ms`** (`17.49 ms` Min) | **`21.163 ms`** | **`1.656 ms`** | `17.514 ms` (`13.9%`) | `24.621 ms` | `50.990 ms` | `78.359 ms` | **`1.23x Bun` (`14.87x` Warm) / `3.93x Node`** |
| **CS-05** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`20.765 ms`** (`17.12 ms` Min) | **`21.730 ms`** | **`9.381 ms`** | `18.127 ms` (`14.6%`) | `23.262 ms` | `71.208 ms` | `96.481 ms` | **`1.12x Bun` (`2.48x` Warm) / `4.65x Node`** |

---

## 4. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Workload | Slice (`SLICEBC\x01` Default) | Bun (`v1.4.4`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha.0.0.0`) | Slice Memory Advantage |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **App 2: NPM semver+ms+ini** | **13,568 KB (13.25 MB)** | 29,948 KB (29.25 MB) | 46,560 KB (45.47 MB) | 71,708 KB (70.03 MB) | **5.29x less than Node / 2.21x less than Bun** |
| **App 6: Telemetry + Flatted** | **14,612 KB (14.27 MB)** | 30,080 KB (29.38 MB) | 50,548 KB (49.36 MB) | 73,240 KB (71.52 MB) | **5.01x less than Node / 2.06x less than Bun** |
| **App 21: RFC 6455 WS + Sec** | **13,952 KB (13.63 MB)** | 31,844 KB (31.10 MB) | 50,936 KB (49.74 MB) | 95,104 KB (92.88 MB) | **6.82x less than Node / 2.28x less than Bun** |
| **App 22: Workers + Sockets** | **22,272 KB (21.75 MB)** | 41,808 KB (40.83 MB) | 63,732 KB (62.24 MB) | 93,304 KB (91.12 MB) | **4.19x less than Node / 1.88x less than Bun** |
| **App 23: Pure-Rust WASM** | **19,072 KB (18.63 MB)** | 30,848 KB (30.13 MB) | 49,160 KB (48.01 MB) | 80,528 KB (78.64 MB) | **4.22x less than Node / 1.62x less than Bun** |
| **FW 1: Hono v4 Microservice** | **14,976 KB (14.63 MB)** | 31,176 KB (30.45 MB) | 48,724 KB (47.58 MB) | 89,112 KB (87.02 MB) | **5.95x less than Node / 2.08x less than Bun** |
| **FW 2: itty-router + UnJS** | **14,336 KB (14.00 MB)** | 31,872 KB (31.13 MB) | 49,084 KB (47.93 MB) | 87,300 KB (85.25 MB) | **6.09x less than Node / 2.22x less than Bun** |
| **App 24: WASM + SSR** | **13,864 KB (13.54 MB)** | 30,464 KB (29.75 MB) | 49,016 KB (47.87 MB) | 81,024 KB (79.13 MB) | **5.84x less than Node / 2.20x less than Bun** |
| **FW 3: Express node:http** | **15,360 KB (15.00 MB)** | 33,024 KB (32.25 MB) | 47,032 KB (45.93 MB) | 80,400 KB (78.52 MB) | **5.23x less than Node / 2.15x less than Bun** |
| **01-tot-autonomous-sre-agent** | **18,944 KB (18.50 MB)** | 31,744 KB (31.00 MB) | 53,352 KB (52.10 MB) | 82,952 KB (81.01 MB) | **4.38x less than Node / 1.68x less than Bun** |
| **02-mcp-code-mode-orchestrator** | **18,028 KB (17.61 MB)** | 28,416 KB (27.75 MB) | 46,656 KB (45.56 MB) | 82,820 KB (80.88 MB) | **4.59x less than Node / 1.58x less than Bun** |
| **03-multi-tenant-saas-plugin-mesh** | **15,744 KB (15.38 MB)** | 26,496 KB (25.88 MB) | 49,800 KB (48.63 MB) | 88,580 KB (86.50 MB) | **5.63x less than Node / 1.68x less than Bun** |
| **04-wasm-zero-trust-cdr-pipeline** | **19,712 KB (19.25 MB)** | 31,616 KB (30.88 MB) | 49,748 KB (48.58 MB) | 80,392 KB (78.51 MB) | **4.08x less than Node / 1.60x less than Bun** |
| **05-time-travel-chaos-fuzzer-payments** | **15,616 KB (15.25 MB)** | 29,056 KB (28.38 MB) | 49,524 KB (48.36 MB) | 80,928 KB (79.03 MB) | **5.18x less than Node / 1.86x less than Bun** |
| **FW 4: SvelteKit 3 + Svelte 5** | **21,120 KB (20.63 MB)** | 41,728 KB (40.75 MB) | 56,476 KB (55.15 MB) | 85,664 KB (83.66 MB) | **4.06x less than Node / 1.98x less than Bun** |

---

## 5. Multi-Tenant Edge KV, AI Agent Checkpoints & 5-Ring Security Telemetry

| Subsystem / Benchmark | Measured Telemetry | Status |
| :--- | :--- | :---: |
| **Sharded `Slice.kv` (`1,001` Get/Set/Incr Ops in Isolate)** | **`500.30 us`** total (`--warm`, `8,431 B` memory cage) | `PASS` |
| **Multi-Tenant `--tenant-dir` Zero-Downtime Hot-Swap** | Live atomic replacement of `tenant-alpha.cas` (`v1 -> v2-hot-swapped`) with isolated `Slice.kv` state | `PASS` |
| **Deterministic Time-Travel Replay (`--deterministic=20261019`)** | 100% bit-for-bit identical PRNG (`Math.random`) & virtual clock (`Date.now`, `performance.now`) | `PASS` |
| **Tree-of-Thought AI Agent (`Slice.checkpoint` + 3 `evalSandbox` + `restore`)** | **`50.17 us`** total (`1.87 us` child sandbox) | `PASS` |
| **Cloud Metadata SSRF (`169.254.169.254`) & Base64 Egress DLP Guard** | `SSRF_METADATA_BLOCKED` & `EGRESS_SECRET_LEAK_BLOCKED` deterministically trapped | `PASS` |
| **Built-In Model Context Protocol Server (`slice mcp --smoke-test`)** | JSON-RPC 2.0 `initialize`, `tools/list`, `tools/call` verified over `stdio` | `PASS` |
