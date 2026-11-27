# Slice (Nexus-JS) - Full 33-Workload & MicroVM Benchmark Report

* **Slice Version:** `v0.14.7` (Commit `4aff8ce`)
* **Generated Timestamp (UTC):** `2026-11-27T04:06:24Z`
* **Archive File:** [`tests/reports/bench_v0.14.7_20261127_040624Z_4aff8ce.md`](./bench_v0.14.7_20261127_040624Z_4aff8ce.md)
* **Canonical Latest Report:** [`tests/reports/latest.md`](./latest.md)
* **Environment:** `Linux x86_64 Docker (io_uring SQPOLL + WRPKRU)`
* **Compared Runtimes:**
  * **Slice:** `v0.14.7` (Default `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS + Monomorphic Inline Caching `get_fast` + Iterative ECMA-404 `JSON.parse`)
  * **Bun:** `v1.4.5` (JavaScriptCore)
  * **Deno:** `v2.9.8` (V8)
  * **Node.js:** `v27.0.0-alpha.0.1.1` (V8 native TypeScript execution)

---

## 1. Executive Summary

Across all **33 real-world multi-file TypeScript applications, framework suites, and case studies**, Slice beats **Deno (`v2.9.8`)** on **33/33** workloads, beats **Node.js (`v27.0.0-alpha.0.1.1`)** on **33/33** workloads, beats **Bun (`v1.4.5`)** on **33/33** workloads in cold one-shot execution, and beats **all 3 runtimes on 33/33 workloads in `--warm` pooled isolate execution**.

| Metric | Linux (`x86_64` Docker) | Windows (`x86_64` Host) | Comparison vs Runtimes & MicroVMs |
| :--- | :---: | :---: | :--- |
| **Single Isolate Cold-Start (Min / P50 / P99)** | **`0.84 us` / `0.87 us` / `1.47 us`** | **`0.80 us` / `0.90 us` / `2.10 us`** | **~120,000x faster than 125ms Firecracker VM** |
| **1,000-Tenant MicroVM Swarm (Boot + Exec)** | **`1.04 ms`** (`8.38 MB` RSS) | **`1.19 ms`** (`6.80 MB` RSS) | **> 10,000x faster than container/VM swarms** |
| **500-Module Monorepo Cold Boot (`0 stat()`)** | **`2.834 ms`** (`124.97 us` in-mem) | **`7.385 ms`** (`152.40 us` in-mem) | **24.8x faster than Node.js (`1,500+ stat()`)** |
| **10,000 Direct Kernel Ring I/O Completions** | **`0.01 ms`** | **`3.62 ms`** | **Zero worker-thread lock contention** |
| **5-Ring Security Containment Verification** | **`5 / 5 PASS`** | **`5 / 5 PASS`** | **OCap + SES + Fuel + SSRF/DLP + MPK** |

---

## 2. `slice serve` Multi-Tenant HTTP/1.1 Keep-Alive Throughput (`16,000` Reqs, 8 Sockets x 8 Pipeline)

| Server / Runtime | Throughput (`req/s`) | Avg Latency / Req | Total Time (`16k` Reqs) | Relative Speedup vs Runtimes |
| :--- | :---: | :---: | :---: | :--- |
| **`slice serve` (Tier-0 CAS, `--workers=8`)** | **`146,857 req/s`** | **`6.81 us/req`** | **`108.9 ms`** | **`1.44x` faster than `Bun.serve` / `2.75x` Node.js** |
| **`slice serve` (Tier-1 WinterTC, `--workers=8`)** | **`149,363 req/s`** | **`6.70 us/req`** | **`107.1 ms`** | **`1.46x` faster than `Bun.serve` / `2.79x` Node.js** |
| **`Deno.serve` (`v2.9.8`, V8, No Per-Req Sandbox)** | `97,047 req/s` | `10.30 us/req` | `164.9 ms` | `0.95x` vs Bun |
| **`Bun.serve` (`v1.4.5`, JSC, No Per-Req Sandbox)** | `102,088 req/s` | `9.80 us/req` | `156.7 ms` | `1.00x` (Baseline) |
| **`Node.js http.createServer` (`v27.0.0-alpha.0.1.1`, V8)** | `53,471 req/s` | `18.70 us/req` | `299.2 ms` | `0.52x` vs Bun |

---

## 3. Full 33-Workload Latency Comparison (20 Runs Each, Linux `x86_64` Docker)

| # | Workload (`tests/apps/`, `tests/frameworks/`, `tests/case_studies/`) | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm (`--warm`) | Previous `.cas` (`Delta`) | Bun `v1.4.5` | Deno `v2.9.8` | Node.js `v27.0.0-alpha.0.1.1` | Slice Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`5.234 ms`** (`4.78 ms` Min) | **`5.722 ms`** | **`0.805 ms`** | `5.309 ms` (`-1.4%`) | `14.421 ms` | `39.471 ms` | `53.494 ms` | **`2.76x Bun` (`17.91x` Warm) / `10.22x Node`** |
| **02** | `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | **`11.456 ms`** (`10.82 ms` Min) | **`12.235 ms`** | **`3.298 ms`** | `11.325 ms` (`1.2%`) | `22.578 ms` | `67.007 ms` | `63.499 ms` | **`1.97x Bun` (`6.85x` Warm) / `5.54x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`6.174 ms`** (`5.55 ms` Min) | **`6.703 ms`** | **`1.158 ms`** | `5.989 ms` (`3.1%`) | `15.265 ms` | `43.354 ms` | `54.578 ms` | **`2.47x Bun` (`13.18x` Warm) / `8.84x Node`** |
| **04** | `04-edge-api-gateway` (`fast-querystring` + `ms`) | **`6.435 ms`** (`5.92 ms` Min) | **`6.784 ms`** | **`1.464 ms`** | `6.456 ms` (`-0.3%`) | `16.190 ms` | `41.548 ms` | `57.393 ms` | **`2.52x Bun` (`11.06x` Warm) / `8.92x Node`** |
| **05** | `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | **`9.585 ms`** (`8.77 ms` Min) | **`9.595 ms`** | **`4.261 ms`** | `8.795 ms` (`9.0%`) | `15.834 ms` | `44.539 ms` | `56.429 ms` | **`1.65x Bun` (`3.72x` Warm) / `5.89x Node`** |
| **06** | `06-telemetry-metrics-pipeline` (`bytes` + `flatted` + WAL) | **`10.744 ms`** (`10.10 ms` Min) | **`11.570 ms`** | **`4.177 ms`** | `10.828 ms` (`-0.8%`) | `18.298 ms` | `46.822 ms` | `58.643 ms` | **`1.70x Bun` (`4.38x` Warm) / `5.46x Node`** |
| **07** | `07-graphql-ast-validator` (`dlv` + `dset` + AST Cost) | **`7.961 ms`** (`7.35 ms` Min) | **`8.079 ms`** | **`3.390 ms`** | `7.808 ms` (`2.0%`) | `14.243 ms` | `40.486 ms` | `55.687 ms` | **`1.79x Bun` (`4.20x` Warm) / `6.99x Node`** |
| **08** | `08-event-driven-state-journal` (`mitt` + `ms` + JSONL Replay) | **`6.366 ms`** (`5.83 ms` Min) | **`6.757 ms`** | **`1.158 ms`** | `6.282 ms` (`1.3%`) | `17.378 ms` | `44.783 ms` | `56.842 ms` | **`2.73x Bun` (`15.01x` Warm) / `8.93x Node`** |
| **09** | `09-wintertc-web-router` (`Request` + `Response` + `URL`, 80 reqs) | **`12.989 ms`** (`12.19 ms` Min) | **`13.243 ms`** | **`6.693 ms`** | `13.109 ms` (`-0.9%`) | `15.152 ms` | `50.380 ms` | `80.964 ms` | **`1.17x Bun` (`2.26x` Warm) / `6.23x Node`** |
| **10** | `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **`10.224 ms`** (`9.56 ms` Min) | **`10.963 ms`** | **`3.692 ms`** | `10.354 ms` (`-1.3%`) | `16.253 ms` | `50.542 ms` | `81.744 ms` | **`1.59x Bun` (`4.40x` Warm) / `8.00x Node`** |
| **11** | `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **`10.606 ms`** (`9.65 ms` Min) | **`10.412 ms`** | **`3.167 ms`** | `8.784 ms` (`20.7%`) | `14.624 ms` | `47.456 ms` | `80.422 ms` | **`1.40x Bun` (`4.62x` Warm) / `7.72x Node`** |
| **12** | `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP, 320 frames) | **`13.776 ms`** (`13.03 ms` Min) | **`13.756 ms`** | **`6.533 ms`** | `13.355 ms` (`3.2%`) | `16.284 ms` | `50.068 ms` | `85.200 ms` | **`1.18x Bun` (`2.49x` Warm) / `6.19x Node`** |
| **13** | `13-webhook-orchestrator-ssrf` (`fast-querystring` + `ms` + Live `fetch`) | **`10.616 ms`** (`9.87 ms` Min) | **`10.943 ms`** | **`3.518 ms`** | `9.779 ms` (`8.6%`) | `20.498 ms` | `49.890 ms` | `104.231 ms` | **`1.93x Bun` (`5.83x` Warm) / `9.82x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`12.976 ms`** (`11.93 ms` Min) | **`13.397 ms`** | **`7.036 ms`** | `12.976 ms` (`0.0%`) | `22.133 ms` | `52.589 ms` | `102.818 ms` | **`1.71x Bun` (`3.15x` Warm) / `7.92x Node`** |
| **15** | `15-time-travel-test-suite` (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **`11.639 ms`** (`10.96 ms` Min) | **`11.957 ms`** | **`3.827 ms`** | `11.575 ms` (`0.6%`) | `25.679 ms` | `65.718 ms` | `77.358 ms` | **`2.21x Bun` (`6.71x` Warm) / `6.65x Node`** |
| **16** | `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **`6.491 ms`** (`6.04 ms` Min) | **`6.996 ms`** | **`1.268 ms`** | `6.486 ms` (`0.1%`) | `15.903 ms` | `45.102 ms` | `60.019 ms` | **`2.45x Bun` (`12.54x` Warm) / `9.25x Node`** |
| **17** | `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive/307/SSRF/DLP) | **`13.134 ms`** (`12.20 ms` Min) | **`13.562 ms`** | **`5.672 ms`** | `13.172 ms` (`-0.3%`) | `22.906 ms` | `54.611 ms` | `105.911 ms` | **`1.74x Bun` (`4.04x` Warm) / `8.06x Node`** |
| **18** | `18-async-event-loop-fanout-scheduler` (Timer Wheel/`Promise.all`/`AbortSignal`) | **`28.536 ms`** (`26.92 ms` Min) | **`28.482 ms`** | **`17.490 ms`** | `27.840 ms` (`2.5%`) | `34.006 ms` | `70.858 ms` | `73.832 ms` | **`1.19x Bun` (`1.94x` Warm) / `2.59x Node`** |
| **19** | `19-wintertc-webcrypto-streams-edge` (WebCrypto/Streams/FormData/Clone) | **`8.366 ms`** (`8.01 ms` Min) | **`8.965 ms`** | **`1.443 ms`** | `8.354 ms` (`0.1%`) | `15.100 ms` | `44.882 ms` | `80.924 ms` | **`1.80x Bun` (`10.46x` Warm) / `9.67x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`9.025 ms`** (`8.40 ms` Min) | **`9.619 ms`** | **`0.395 ms`** | `8.977 ms` (`0.5%`) | `23.101 ms` | `53.933 ms` | `67.917 ms` | **`2.56x Bun` (`58.48x` Warm) / `7.53x Node`** |
| **21** | `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket` + 5-Ring Guard) | **`9.398 ms`** (`8.68 ms` Min) | **`9.735 ms`** | **`2.016 ms`** | `9.092 ms` (`3.4%`) | `21.328 ms` | `50.726 ms` | `110.892 ms` | **`2.27x Bun` (`10.58x` Warm) / `11.80x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`28.728 ms`** (`26.93 ms` Min) | **`29.010 ms`** | **`2.411 ms`** | `28.412 ms` (`1.1%`) | `56.786 ms` | `127.223 ms` | `93.370 ms` | **`1.98x Bun` (`23.55x` Warm) / `3.25x Node`** |
| **23** | `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **`13.036 ms`** (`12.14 ms` Min) | **`13.582 ms`** | **`1.512 ms`** | `12.887 ms` (`1.2%`) | `22.056 ms` | `39.684 ms` | `57.926 ms` | **`1.69x Bun` (`14.59x` Warm) / `4.44x Node`** |
| **24** | `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` SLA Scorer + SSR Pipeline) | **`7.135 ms`** (`6.53 ms` Min) | **`7.534 ms`** | **`1.495 ms`** | `7.127 ms` (`0.1%`) | `19.784 ms` | `42.421 ms` | `62.070 ms` | **`2.77x Bun` (`13.23x` Warm) / `8.70x Node`** |
| **FW-01** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`17.441 ms`** (`16.61 ms` Min) | **`18.246 ms`** | **`4.999 ms`** | `17.414 ms` (`0.2%`) | `21.303 ms` | `52.795 ms` | `87.176 ms` | **`1.22x Bun` (`4.26x` Warm) / `5.00x Node`** |
| **FW-02** | `FW-02: frameworks/02-itty-router-unjs-nitro-stack` (`itty-router` + UnJS) | **`16.555 ms`** (`15.09 ms` Min) | **`17.060 ms`** | **`8.482 ms`** | `15.704 ms` (`5.4%`) | `20.142 ms` | `45.569 ms` | `82.731 ms` | **`1.22x Bun` (`2.37x` Warm) / `5.00x Node`** |
| **FW-03** | `FW-03: frameworks/03-node-http-express-compat` (`node:http` + `node:stream` Express/Polka) | **`13.902 ms`** (`13.17 ms` Min) | **`14.445 ms`** | **`1.019 ms`** | `13.988 ms` (`-0.6%`) | `35.177 ms` | `56.226 ms` | `78.453 ms` | **`2.53x Bun` (`34.52x` Warm) / `5.64x Node`** |
| **FW-04** | `FW-04: frameworks/04-sveltekit-edge-ssr` (`@sveltejs/kit@3.1.1` + `svelte@5.57.3` + `devalue@6.0.3`) | **`29.812 ms`** (`27.75 ms` Min) | **`29.457 ms`** | **`13.693 ms`** | `29.093 ms` (`2.5%`) | `44.832 ms` | `69.727 ms` | `105.870 ms` | **`1.52x Bun` (`3.27x` Warm) / `3.59x Node`** |
| **CS-01** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`27.772 ms`** (`26.38 ms` Min) | **`27.657 ms`** | **`11.693 ms`** | `27.305 ms` (`1.7%`) | `29.364 ms` | `71.692 ms` | `80.651 ms` | **`1.06x Bun` (`2.51x` Warm) / `2.92x Node`** |
| **CS-02** | `CS-02: case_studies/02-mcp-code-mode-orchestrator` (`-98.4%` Tokens) | **`15.956 ms`** (`14.90 ms` Min) | **`16.363 ms`** | **`1.977 ms`** | `15.878 ms` (`0.5%`) | `17.316 ms` | `42.888 ms` | `61.914 ms` | **`1.09x Bun` (`8.76x` Warm) / `3.88x Node`** |
| **CS-03** | `CS-03: case_studies/03-multi-tenant-saas-plugin-mesh` (12 Tenants + Hot-Swap) | **`12.342 ms`** (`11.58 ms` Min) | **`12.774 ms`** | **`4.376 ms`** | `12.631 ms` (`-2.3%`) | `17.251 ms` | `47.609 ms` | `79.504 ms` | **`1.40x Bun` (`3.94x` Warm) / `6.44x Node`** |
| **CS-04** | `CS-04: case_studies/04-wasm-zero-trust-cdr-pipeline` (Wasm SimHash CDR) | **`14.076 ms`** (`13.27 ms` Min) | **`14.701 ms`** | **`1.782 ms`** | `13.582 ms` (`3.6%`) | `19.899 ms` | `45.432 ms` | `60.266 ms` | **`1.41x Bun` (`11.17x` Warm) / `4.28x Node`** |
| **CS-05** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`8.804 ms`** (`8.29 ms` Min) | **`9.227 ms`** | **`2.493 ms`** | `8.982 ms` (`-2.0%`) | `20.826 ms` | `62.945 ms` | `77.610 ms` | **`2.37x Bun` (`8.35x` Warm) / `8.82x Node`** |

---

## 4. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Workload | Slice (`SLICEBC\x01` Default) | Bun (`v1.4.5`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha.0.1.1`) | Slice Memory Advantage |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **App 2: NPM semver+ms+ini** | **14,464 KB (14.13 MB)** | 38,528 KB (37.63 MB) | 47,560 KB (46.45 MB) | 70,756 KB (69.10 MB) | **4.89x less than Node / 2.66x less than Bun** |
| **App 6: Telemetry + Flatted** | **14,336 KB (14.00 MB)** | 39,680 KB (38.75 MB) | 50,548 KB (49.36 MB) | 71,368 KB (69.70 MB) | **4.98x less than Node / 2.77x less than Bun** |
| **App 21: RFC 6455 WS + Sec** | **15,232 KB (14.88 MB)** | 37,696 KB (36.81 MB) | 50,796 KB (49.61 MB) | 92,644 KB (90.47 MB) | **6.08x less than Node / 2.47x less than Bun** |
| **App 22: Workers + Sockets** | **22,400 KB (21.88 MB)** | 47,692 KB (46.57 MB) | 64,040 KB (62.54 MB) | 91,840 KB (89.69 MB) | **4.10x less than Node / 2.13x less than Bun** |
| **App 23: Pure-Rust WASM** | **18,816 KB (18.38 MB)** | 38,144 KB (37.25 MB) | 48,760 KB (47.62 MB) | 77,892 KB (76.07 MB) | **4.14x less than Node / 2.03x less than Bun** |
| **FW 1: Hono v4 Microservice** | **17,280 KB (16.88 MB)** | 40,304 KB (39.36 MB) | 49,632 KB (48.47 MB) | 87,144 KB (85.10 MB) | **5.04x less than Node / 2.33x less than Bun** |
| **FW 2: itty-router + UnJS** | **14,464 KB (14.13 MB)** | 40,960 KB (40.00 MB) | 48,880 KB (47.73 MB) | 86,252 KB (84.23 MB) | **5.96x less than Node / 2.83x less than Bun** |
| **App 24: WASM + SSR** | **14,336 KB (14.00 MB)** | 37,888 KB (37.00 MB) | 48,980 KB (47.83 MB) | 78,152 KB (76.32 MB) | **5.45x less than Node / 2.64x less than Bun** |
| **FW 3: Express node:http** | **15,232 KB (14.88 MB)** | 41,344 KB (40.38 MB) | 46,992 KB (45.89 MB) | 79,724 KB (77.86 MB) | **5.23x less than Node / 2.71x less than Bun** |
| **01-tot-autonomous-sre-agent** | **20,736 KB (20.25 MB)** | 41,344 KB (40.38 MB) | 53,484 KB (52.23 MB) | 81,252 KB (79.35 MB) | **3.92x less than Node / 1.99x less than Bun** |
| **02-mcp-code-mode-orchestrator** | **18,028 KB (17.61 MB)** | 37,504 KB (36.63 MB) | 46,184 KB (45.10 MB) | 77,896 KB (76.07 MB) | **4.32x less than Node / 2.08x less than Bun** |
| **03-multi-tenant-saas-plugin-mesh** | **15,232 KB (14.88 MB)** | 37,504 KB (36.63 MB) | 49,768 KB (48.60 MB) | 86,888 KB (84.85 MB) | **5.70x less than Node / 2.46x less than Bun** |
| **04-wasm-zero-trust-cdr-pipeline** | **19,200 KB (18.75 MB)** | 39,268 KB (38.35 MB) | 49,724 KB (48.56 MB) | 78,692 KB (76.85 MB) | **4.10x less than Node / 2.05x less than Bun** |
| **05-time-travel-chaos-fuzzer-payments** | **13,824 KB (13.50 MB)** | 38,912 KB (38.00 MB) | 49,784 KB (48.62 MB) | 78,148 KB (76.32 MB) | **5.65x less than Node / 2.81x less than Bun** |
| **FW 4: SvelteKit 3 + Svelte 5** | **22,240 KB (21.72 MB)** | 50,688 KB (49.50 MB) | 56,348 KB (55.03 MB) | 84,208 KB (82.23 MB) | **3.79x less than Node / 2.28x less than Bun** |

---

## 5. Multi-Tenant Edge KV, AI Agent Checkpoints & 5-Ring Security Telemetry

| Subsystem / Benchmark | Measured Telemetry | Status |
| :--- | :--- | :---: |
| **Sharded `Slice.kv` (`1,001` Get/Set/Incr Ops in Isolate)** | **`545.53 us`** total (`--warm`, `288 B` memory cage) | `PASS` |
| **Multi-Tenant `--tenant-dir` Zero-Downtime Hot-Swap** | Live atomic replacement of `tenant-alpha.cas` (`v1 -> v2-hot-swapped`) with isolated `Slice.kv` state | `PASS` |
| **Deterministic Time-Travel Replay (`--deterministic=20261019`)** | 100% bit-for-bit identical PRNG (`Math.random`) & virtual clock (`Date.now`, `performance.now`) | `PASS` |
| **Tree-of-Thought AI Agent (`Slice.checkpoint` + 3 `evalSandbox` + `restore`)** | **`60.94 us`** total (`1.85 us` child sandbox) | `PASS` |
| **Cloud Metadata SSRF (`169.254.169.254`) & Base64 Egress DLP Guard** | `SSRF_METADATA_BLOCKED` & `EGRESS_SECRET_LEAK_BLOCKED` deterministically trapped | `PASS` |
| **Built-In Model Context Protocol Server (`slice mcp --smoke-test`)** | JSON-RPC 2.0 `initialize`, `tools/list`, `tools/call` verified over `stdio` | `PASS` |
