# Slice (Nexus-JS) — Full 33-Workload & MicroVM Benchmark Report

* **Slice Version:** `v0.10.8` (Commit `8e712ae`)
* **Generated Timestamp (UTC):** `2026-11-11T00:04:08Z`
* **Archive File:** [`tests/reports/bench_v0.10.8_20261111_000408Z_8e712ae.md`](./bench_v0.10.8_20261111_000408Z_8e712ae.md)
* **Canonical Latest Report:** [`tests/reports/latest.md`](./latest.md)
* **Environment:** `Linux x86_64 Docker (io_uring SQPOLL + WRPKRU)`
* **Compared Runtimes:**
  * **Slice:** `v0.10.8` (Default `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS + Monomorphic Inline Caching `get_fast` + Streaming `JsonSeed` `JSON.parse`)
  * **Bun:** `v1.4.4` (JavaScriptCore)
  * **Deno:** `v2.9.8` (V8)
  * **Node.js:** `v27.0.0-alpha.0.0.0` (V8 native TypeScript execution)

---

## 1. Executive Summary

Across all **33 real-world multi-file TypeScript applications, framework suites, and case studies**, Slice beats **Deno (`v2.9.8`)** on **33/33** workloads, beats **Node.js (`v27.0.0-alpha.0.0.0`)** on **33/33** workloads, beats **Bun (`v1.4.4`)** on **30/33** workloads in cold one-shot execution, and beats **all 3 runtimes on 33/33 workloads in `--warm` pooled isolate execution**.

| Metric | Linux (`x86_64` Docker) | Windows (`x86_64` Host) | Comparison vs Runtimes & MicroVMs |
| :--- | :---: | :---: | :--- |
| **Single Isolate Cold-Start (Min / P50 / P99)** | **`1.09 us` / `1.16 us` / `2.30 us`** | **`1.40 us` / `1.60 us` / `2.30 us`** | **~120,000x faster than 125ms Firecracker VM** |
| **1,000-Tenant MicroVM Swarm (Boot + Exec)** | **`1.42 ms`** (`9.82 MB` RSS) | **`2.24 ms`** (`11.82 MB` RSS) | **> 10,000x faster than container/VM swarms** |
| **500-Module Monorepo Cold Boot (`0 stat()`)** | **`3.408 ms`** (`366.35 us` in-mem) | **`5.637 ms`** (`640.00 us` in-mem) | **20.6x faster than Node.js (`1,500+ stat()`)** |
| **10,000 Direct Kernel Ring I/O Completions** | **`0.02 ms`** | **`5.11 ms`** | **Zero worker-thread lock contention** |
| **5-Ring Security Containment Verification** | **`5 / 5 PASS`** | **`5 / 5 PASS`** | **OCap + SES + Fuel + SSRF/DLP + MPK** |

---

## 2. `slice serve` Multi-Tenant HTTP/1.1 Keep-Alive Throughput (`16,000` Reqs, 8 Sockets x 8 Pipeline)

| Server / Runtime | Throughput (`req/s`) | Avg Latency / Req | Total Time (`16k` Reqs) | Relative Speedup vs Runtimes |
| :--- | :---: | :---: | :---: | :--- |
| **`slice serve` (Tier-0 CAS, `--workers=8`)** | **`172,255 req/s`** | **`5.81 us/req`** | **`92.9 ms`** | **`1.63x` faster than `Bun.serve` / `2.88x` Node.js** |
| **`slice serve` (Tier-1 WinterTC, `--workers=8`)** | **`158,151 req/s`** | **`6.32 us/req`** | **`101.2 ms`** | **`1.50x` faster than `Bun.serve` / `2.64x` Node.js** |
| **`Deno.serve` (`v2.9.8`, V8, No Per-Req Sandbox)** | `108,954 req/s` | `9.18 us/req` | `146.9 ms` | `1.03x` vs Bun |
| **`Bun.serve` (`v1.4.4`, JSC, No Per-Req Sandbox)** | `105,661 req/s` | `9.46 us/req` | `151.4 ms` | `1.00x` (Baseline) |
| **`Node.js http.createServer` (`v27.0.0-alpha.0.0.0`, V8)** | `59,817 req/s` | `16.72 us/req` | `267.5 ms` | `0.57x` vs Bun |

---

## 3. Full 33-Workload Latency Comparison (20 Runs Each, Linux `x86_64` Docker)

| # | Workload (`tests/apps/`, `tests/frameworks/`, `tests/case_studies/`) | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm (`--warm`) | Previous `.cas` (`Delta`) | Bun `v1.4.4` | Deno `v2.9.8` | Node.js `v27.0.0-alpha.0.0.0` | Slice Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`5.659 ms`** (`5.21 ms` Min) | **`6.347 ms`** | **`1.073 ms`** | `7.502 ms` (`-24.6%`) | `12.223 ms` | `33.173 ms` | `52.961 ms` | **`2.16x Bun` (`11.39x` Warm) / `9.36x Node`** |
| **02** | `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | **`12.659 ms`** (`12.21 ms` Min) | **`13.536 ms`** | **`3.503 ms`** | `16.522 ms` (`-23.4%`) | `19.475 ms` | `48.522 ms` | `63.585 ms` | **`1.54x Bun` (`5.56x` Warm) / `5.02x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`6.727 ms`** (`6.34 ms` Min) | **`7.389 ms`** | **`1.302 ms`** | `8.552 ms` (`-21.3%`) | `12.647 ms` | `39.249 ms` | `635.437 ms` | **`1.88x Bun` (`9.71x` Warm) / `94.46x Node`** |
| **04** | `04-edge-api-gateway` (`fast-querystring` + `ms`) | **`6.624 ms`** (`6.30 ms` Min) | **`7.275 ms`** | **`1.737 ms`** | `8.133 ms` (`-18.6%`) | `13.809 ms` | `38.380 ms` | `57.957 ms` | **`2.08x Bun` (`7.95x` Warm) / `8.75x Node`** |
| **05** | `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | **`9.604 ms`** (`8.87 ms` Min) | **`9.980 ms`** | **`4.556 ms`** | `11.369 ms` (`-15.5%`) | `13.104 ms` | `40.856 ms` | `61.242 ms` | **`1.36x Bun` (`2.88x` Warm) / `6.38x Node`** |
| **06** | `06-telemetry-metrics-pipeline` (`bytes` + `flatted` + WAL) | **`13.023 ms`** (`12.44 ms` Min) | **`13.877 ms`** | **`5.325 ms`** | `17.681 ms` (`-26.3%`) | `15.642 ms` | `43.250 ms` | `59.804 ms` | **`1.20x Bun` (`2.94x` Warm) / `4.59x Node`** |
| **07** | `07-graphql-ast-validator` (`dlv` + `dset` + AST Cost) | **`8.178 ms`** (`7.79 ms` Min) | **`8.494 ms`** | **`3.457 ms`** | `9.215 ms` (`-11.3%`) | `11.949 ms` | `37.640 ms` | `56.046 ms` | **`1.46x Bun` (`3.46x` Warm) / `6.85x Node`** |
| **08** | `08-event-driven-state-journal` (`mitt` + `ms` + JSONL Replay) | **`10.036 ms`** (`9.43 ms` Min) | **`10.937 ms`** | **`4.249 ms`** | `12.649 ms` (`-20.7%`) | `14.450 ms` | `45.277 ms` | `57.812 ms` | **`1.44x Bun` (`3.40x` Warm) / `5.76x Node`** |
| **09** | `09-wintertc-web-router` (`Request` + `Response` + `URL`, 80 reqs) | **`13.781 ms`** (`13.20 ms` Min) | **`14.300 ms`** | **`8.171 ms`** | `16.762 ms` (`-17.8%`) | `13.030 ms` | `46.647 ms` | `78.330 ms` | **`1.59x Bun Warm` / `5.68x Node`** |
| **10** | `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **`12.008 ms`** (`11.42 ms` Min) | **`12.732 ms`** | **`6.202 ms`** | `16.690 ms` (`-28.1%`) | `13.606 ms` | `45.578 ms` | `80.153 ms` | **`1.13x Bun` (`2.19x` Warm) / `6.67x Node`** |
| **11** | `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **`9.199 ms`** (`8.70 ms` Min) | **`9.914 ms`** | **`4.031 ms`** | `12.593 ms` (`-27.0%`) | `12.505 ms` | `46.068 ms` | `660.703 ms` | **`1.36x Bun` (`3.10x` Warm) / `71.82x Node`** |
| **12** | `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP, 320 frames) | **`14.114 ms`** (`13.60 ms` Min) | **`14.741 ms`** | **`6.607 ms`** | `19.401 ms` (`-27.3%`) | `17.513 ms` | `47.514 ms` | `85.664 ms` | **`1.24x Bun` (`2.65x` Warm) / `6.07x Node`** |
| **13** | `13-webhook-orchestrator-ssrf` (`fast-querystring` + `ms` + Live `fetch`) | **`12.492 ms`** (`11.81 ms` Min) | **`13.272 ms`** | **`5.661 ms`** | `17.325 ms` (`-27.9%`) | `18.551 ms` | `47.436 ms` | `108.460 ms` | **`1.49x Bun` (`3.28x` Warm) / `8.68x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`14.838 ms`** (`13.69 ms` Min) | **`15.021 ms`** | **`6.241 ms`** | `16.318 ms` (`-9.1%`) | `21.797 ms` | `52.647 ms` | `111.245 ms` | **`1.47x Bun` (`3.49x` Warm) / `7.50x Node`** |
| **15** | `15-time-travel-test-suite` (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **`14.887 ms`** (`14.05 ms` Min) | **`15.515 ms`** | **`4.818 ms`** | `19.162 ms` (`-22.3%`) | `25.034 ms` | `66.213 ms` | `82.159 ms` | **`1.68x Bun` (`5.20x` Warm) / `5.52x Node`** |
| **16** | `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **`7.467 ms`** (`6.92 ms` Min) | **`7.962 ms`** | **`1.234 ms`** | `8.122 ms` (`-8.1%`) | `15.072 ms` | `45.563 ms` | `68.271 ms` | **`2.02x Bun` (`12.21x` Warm) / `9.14x Node`** |
| **17** | `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive/307/SSRF/DLP) | **`15.779 ms`** (`15.31 ms` Min) | **`16.332 ms`** | **`7.396 ms`** | `19.128 ms` (`-17.5%`) | `21.898 ms` | `55.024 ms` | `689.733 ms` | **`1.39x Bun` (`2.96x` Warm) / `43.71x Node`** |
| **18** | `18-async-event-loop-fanout-scheduler` (Timer Wheel/`Promise.all`/`AbortSignal`) | **`29.409 ms`** (`28.15 ms` Min) | **`30.078 ms`** | **`17.266 ms`** | `33.520 ms` (`-12.3%`) | `32.388 ms` | `73.706 ms` | `82.001 ms` | **`1.10x Bun` (`1.88x` Warm) / `2.79x Node`** |
| **19** | `19-wintertc-webcrypto-streams-edge` (WebCrypto/Streams/FormData/Clone) | **`8.407 ms`** (`7.74 ms` Min) | **`8.873 ms`** | **`1.408 ms`** | `9.791 ms` (`-14.1%`) | `14.009 ms` | `46.314 ms` | `82.776 ms` | **`1.67x Bun` (`9.95x` Warm) / `9.85x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`10.148 ms`** (`9.51 ms` Min) | **`10.741 ms`** | **`0.486 ms`** | `12.907 ms` (`-21.4%`) | `20.973 ms` | `55.022 ms` | `68.289 ms` | **`2.07x Bun` (`43.15x` Warm) / `6.73x Node`** |
| **21** | `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket` + 5-Ring Guard) | **`10.288 ms`** (`9.63 ms` Min) | **`10.822 ms`** | **`2.100 ms`** | `12.063 ms` (`-14.7%`) | `19.085 ms` | `49.357 ms` | `112.308 ms` | **`1.86x Bun` (`9.09x` Warm) / `10.92x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`19.101 ms`** (`17.91 ms` Min) | **`19.150 ms`** | **`1.953 ms`** | `25.795 ms` (`-26.0%`) | `55.881 ms` | `109.353 ms` | `97.310 ms` | **`2.93x Bun` (`28.61x` Warm) / `5.09x Node`** |
| **23** | `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **`13.643 ms`** (`13.16 ms` Min) | **`14.450 ms`** | **`1.710 ms`** | `18.567 ms` (`-26.5%`) | `19.465 ms` | `615.643 ms` | `57.529 ms` | **`1.43x Bun` (`11.38x` Warm) / `4.22x Node`** |
| **24** | `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` SLA Scorer + SSR Pipeline) | **`8.336 ms`** (`7.30 ms` Min) | **`8.233 ms`** | **`1.532 ms`** | `10.224 ms` (`-18.5%`) | `17.231 ms` | `40.464 ms` | `62.182 ms` | **`2.09x Bun` (`11.25x` Warm) / `7.55x Node`** |
| **FW-01** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`13.701 ms`** (`12.96 ms` Min) | **`14.206 ms`** | **`5.134 ms`** | `19.113 ms` (`-28.3%`) | `18.285 ms` | `47.806 ms` | `89.673 ms` | **`1.33x Bun` (`3.56x` Warm) / `6.54x Node`** |
| **FW-02** | `FW-02: frameworks/02-itty-router-unjs-nitro-stack` (`itty-router` + UnJS) | **`16.349 ms`** (`15.38 ms` Min) | **`16.582 ms`** | **`7.909 ms`** | `21.035 ms` (`-22.3%`) | `17.317 ms` | `43.041 ms` | `79.615 ms` | **`1.06x Bun` (`2.19x` Warm) / `4.87x Node`** |
| **FW-03** | `FW-03: frameworks/03-node-http-express-compat` (`node:http` + `node:stream` Express/Polka) | **`14.809 ms`** (`14.47 ms` Min) | **`15.869 ms`** | **`0.953 ms`** | `20.431 ms` (`-27.5%`) | `31.347 ms` | `46.931 ms` | `77.305 ms` | **`2.12x Bun` (`32.89x` Warm) / `5.22x Node`** |
| **FW-04** | `FW-04: frameworks/04-sveltekit-edge-ssr` (`@sveltejs/kit@3.1.1` + `svelte@5.57.3` + `devalue@6.0.3`) | **`33.452 ms`** (`28.87 ms` Min) | **`31.535 ms`** | **`14.186 ms`** | `40.143 ms` (`-16.7%`) | `40.133 ms` | `63.010 ms` | `101.684 ms` | **`1.27x Bun` (`2.83x` Warm) / `3.22x Node`** |
| **CS-01** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`29.806 ms`** (`26.82 ms` Min) | **`28.177 ms`** | **`11.591 ms`** | `34.358 ms` (`-13.2%`) | `26.160 ms` | `67.158 ms` | `81.722 ms` | **`2.26x Bun Warm` / `2.90x Node`** |
| **CS-02** | `CS-02: case_studies/02-mcp-code-mode-orchestrator` (`-98.4%` Tokens) | **`16.342 ms`** (`15.74 ms` Min) | **`16.838 ms`** | **`5.353 ms`** | `21.390 ms` (`-23.6%`) | `14.630 ms` | `39.249 ms` | `63.272 ms` | **`2.73x Bun Warm` / `3.87x Node`** |
| **CS-03** | `CS-03: case_studies/03-multi-tenant-saas-plugin-mesh` (12 Tenants + Hot-Swap) | **`12.120 ms`** (`11.57 ms` Min) | **`12.932 ms`** | **`4.893 ms`** | `16.666 ms` (`-27.3%`) | `14.818 ms` | `44.511 ms` | `77.045 ms` | **`1.22x Bun` (`3.03x` Warm) / `6.36x Node`** |
| **CS-04** | `CS-04: case_studies/04-wasm-zero-trust-cdr-pipeline` (Wasm SimHash CDR) | **`15.992 ms`** (`15.35 ms` Min) | **`17.273 ms`** | **`1.441 ms`** | `21.343 ms` (`-25.1%`) | `17.222 ms` | `39.827 ms` | `639.815 ms` | **`1.08x Bun` (`11.95x` Warm) / `40.01x Node`** |
| **CS-05** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`18.391 ms`** (`14.82 ms` Min) | **`16.127 ms`** | **`7.698 ms`** | `21.318 ms` (`-13.7%`) | `18.104 ms` | `57.787 ms` | `76.716 ms` | **`1.12x Bun` (`2.35x` Warm) / `4.76x Node`** |

---

## 4. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Workload | Slice (`SLICEBC\x01` Default) | Bun (`v1.4.4`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha.0.0.0`) | Slice Memory Advantage |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **App 2: NPM semver+ms+ini** | **13,888 KB (13.56 MB)** | 30,280 KB (29.57 MB) | 46,832 KB (45.73 MB) | 71,580 KB (69.90 MB) | **5.15x less than Node / 2.18x less than Bun** |
| **App 6: Telemetry + Flatted** | **14,080 KB (13.75 MB)** | 29,952 KB (29.25 MB) | 50,688 KB (49.50 MB) | 73,112 KB (71.40 MB) | **5.19x less than Node / 2.13x less than Bun** |
| **App 21: RFC 6455 WS + Sec** | **14,080 KB (13.75 MB)** | 31,704 KB (30.96 MB) | 50,656 KB (49.47 MB) | 96,128 KB (93.88 MB) | **6.83x less than Node / 2.25x less than Bun** |
| **App 22: Workers + Sockets** | **22,144 KB (21.63 MB)** | 41,300 KB (40.33 MB) | 63,544 KB (62.05 MB) | 92,816 KB (90.64 MB) | **4.19x less than Node / 1.87x less than Bun** |
| **App 23: Pure-Rust WASM** | **18,560 KB (18.13 MB)** | 30,592 KB (29.88 MB) | 49,192 KB (48.04 MB) | 80,784 KB (78.89 MB) | **4.35x less than Node / 1.65x less than Bun** |
| **FW 1: Hono v4 Microservice** | **15,488 KB (15.13 MB)** | 29,300 KB (28.61 MB) | 49,640 KB (48.48 MB) | 88,604 KB (86.53 MB) | **5.72x less than Node / 1.89x less than Bun** |
| **FW 2: itty-router + UnJS** | **14,592 KB (14.25 MB)** | 32,128 KB (31.38 MB) | 48,624 KB (47.48 MB) | 86,796 KB (84.76 MB) | **5.95x less than Node / 2.20x less than Bun** |
| **App 24: WASM + SSR** | **13,740 KB (13.42 MB)** | 30,336 KB (29.63 MB) | 49,212 KB (48.06 MB) | 81,024 KB (79.13 MB) | **5.90x less than Node / 2.21x less than Bun** |
| **FW 3: Express node:http** | **14,976 KB (14.63 MB)** | 32,896 KB (32.13 MB) | 47,040 KB (45.94 MB) | 80,140 KB (78.26 MB) | **5.35x less than Node / 2.20x less than Bun** |
| **01-tot-autonomous-sre-agent** | **19,840 KB (19.38 MB)** | 32,256 KB (31.50 MB) | 53,336 KB (52.09 MB) | 83,848 KB (81.88 MB) | **4.23x less than Node / 1.63x less than Bun** |
| **02-mcp-code-mode-orchestrator** | **17,896 KB (17.48 MB)** | 28,160 KB (27.50 MB) | 46,864 KB (45.77 MB) | 80,776 KB (78.88 MB) | **4.51x less than Node / 1.57x less than Bun** |
| **03-multi-tenant-saas-plugin-mesh** | **15,616 KB (15.25 MB)** | 28,160 KB (27.50 MB) | 49,924 KB (48.75 MB) | 87,812 KB (85.75 MB) | **5.62x less than Node / 1.80x less than Bun** |
| **04-wasm-zero-trust-cdr-pipeline** | **19,968 KB (19.50 MB)** | 31,488 KB (30.75 MB) | 49,956 KB (48.79 MB) | 80,008 KB (78.13 MB) | **4.01x less than Node / 1.58x less than Bun** |
| **05-time-travel-chaos-fuzzer-payments** | **16,128 KB (15.75 MB)** | 29,056 KB (28.38 MB) | 49,908 KB (48.74 MB) | 80,932 KB (79.04 MB) | **5.02x less than Node / 1.80x less than Bun** |
| **FW 4: SvelteKit 3 + Svelte 5** | **21,120 KB (20.63 MB)** | 41,984 KB (41.00 MB) | 56,372 KB (55.05 MB) | 85,796 KB (83.79 MB) | **4.06x less than Node / 1.99x less than Bun** |

---

## 5. Multi-Tenant Edge KV, AI Agent Checkpoints & 5-Ring Security Telemetry

| Subsystem / Benchmark | Measured Telemetry | Status |
| :--- | :--- | :---: |
| **Sharded `Slice.kv` (`1,001` Get/Set/Incr Ops in Isolate)** | **`492.86 us`** total (`--warm`, `8,431 B` memory cage) | `PASS` |
| **Multi-Tenant `--tenant-dir` Zero-Downtime Hot-Swap** | Live atomic replacement of `tenant-alpha.cas` (`v1 -> v2-hot-swapped`) with isolated `Slice.kv` state | `PASS` |
| **Deterministic Time-Travel Replay (`--deterministic=20261019`)** | 100% bit-for-bit identical PRNG (`Math.random`) & virtual clock (`Date.now`, `performance.now`) | `PASS` |
| **Tree-of-Thought AI Agent (`Slice.checkpoint` + 3 `evalSandbox` + `restore`)** | **`44.91 us`** total (`3.70 us` child sandbox) | `PASS` |
| **Cloud Metadata SSRF (`169.254.169.254`) & Base64 Egress DLP Guard** | `SSRF_METADATA_BLOCKED` & `EGRESS_SECRET_LEAK_BLOCKED` deterministically trapped | `PASS` |
| **Built-In Model Context Protocol Server (`slice mcp --smoke-test`)** | JSON-RPC 2.0 `initialize`, `tools/list`, `tools/call` verified over `stdio` | `PASS` |
