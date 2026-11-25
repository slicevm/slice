# Slice (Nexus-JS) - Full 33-Workload & MicroVM Benchmark Report

* **Slice Version:** `v0.14.3` (Commit `c4acef3`)
* **Generated Timestamp (UTC):** `2026-11-25T16:53:49Z`
* **Archive File:** [`tests/reports/bench_v0.14.3_20261125_165349Z_c4acef3.md`](./bench_v0.14.3_20261125_165349Z_c4acef3.md)
* **Canonical Latest Report:** [`tests/reports/latest.md`](./latest.md)
* **Environment:** `Linux x86_64 Docker (io_uring SQPOLL + WRPKRU)`
* **Compared Runtimes:**
  * **Slice:** `v0.14.3` (Default `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS + Monomorphic Inline Caching `get_fast` + Iterative ECMA-404 `JSON.parse`)
  * **Bun:** `v1.4.5` (JavaScriptCore)
  * **Deno:** `v2.9.8` (V8)
  * **Node.js:** `v27.0.0-alpha.0.1.1` (V8 native TypeScript execution)

---

## 1. Executive Summary

Across all **33 real-world multi-file TypeScript applications, framework suites, and case studies**, Slice beats **Deno (`v2.9.8`)** on **33/33** workloads, beats **Node.js (`v27.0.0-alpha.0.1.1`)** on **33/33** workloads, beats **Bun (`v1.4.5`)** on **33/33** workloads in cold one-shot execution, and beats **all 3 runtimes on 33/33 workloads in `--warm` pooled isolate execution**.

| Metric | Linux (`x86_64` Docker) | Windows (`x86_64` Host) | Comparison vs Runtimes & MicroVMs |
| :--- | :---: | :---: | :--- |
| **Single Isolate Cold-Start (Min / P50 / P99)** | **`0.82 us` / `0.87 us` / `0.98 us`** | **`0.80 us` / `1.40 us` / `4.70 us`** | **~120,000x faster than 125ms Firecracker VM** |
| **1,000-Tenant MicroVM Swarm (Boot + Exec)** | **`1.05 ms`** (`8.12 MB` RSS) | **`1.74 ms`** (`6.67 MB` RSS) | **> 10,000x faster than container/VM swarms** |
| **500-Module Monorepo Cold Boot (`0 stat()`)** | **`2.861 ms`** (`106.85 us` in-mem) | **`9.033 ms`** (`155.30 us` in-mem) | **24.6x faster than Node.js (`1,500+ stat()`)** |
| **10,000 Direct Kernel Ring I/O Completions** | **`0.01 ms`** | **`3.21 ms`** | **Zero worker-thread lock contention** |
| **5-Ring Security Containment Verification** | **`5 / 5 PASS`** | **`5 / 5 PASS`** | **OCap + SES + Fuel + SSRF/DLP + MPK** |

---

## 2. `slice serve` Multi-Tenant HTTP/1.1 Keep-Alive Throughput (`16,000` Reqs, 8 Sockets x 8 Pipeline)

| Server / Runtime | Throughput (`req/s`) | Avg Latency / Req | Total Time (`16k` Reqs) | Relative Speedup vs Runtimes |
| :--- | :---: | :---: | :---: | :--- |
| **`slice serve` (Tier-0 CAS, `--workers=8`)** | **`151,508 req/s`** | **`6.60 us/req`** | **`105.6 ms`** | **`1.65x` faster than `Bun.serve` / `2.77x` Node.js** |
| **`slice serve` (Tier-1 WinterTC, `--workers=8`)** | **`141,543 req/s`** | **`7.06 us/req`** | **`113.0 ms`** | **`1.54x` faster than `Bun.serve` / `2.59x` Node.js** |
| **`Deno.serve` (`v2.9.8`, V8, No Per-Req Sandbox)** | `95,093 req/s` | `10.52 us/req` | `168.3 ms` | `1.03x` vs Bun |
| **`Bun.serve` (`v1.4.5`, JSC, No Per-Req Sandbox)** | `92,023 req/s` | `10.87 us/req` | `173.9 ms` | `1.00x` (Baseline) |
| **`Node.js http.createServer` (`v27.0.0-alpha.0.1.1`, V8)** | `54,625 req/s` | `18.31 us/req` | `292.9 ms` | `0.59x` vs Bun |

---

## 3. Full 33-Workload Latency Comparison (20 Runs Each, Linux `x86_64` Docker)

| # | Workload (`tests/apps/`, `tests/frameworks/`, `tests/case_studies/`) | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm (`--warm`) | Previous `.cas` (`Delta`) | Bun `v1.4.5` | Deno `v2.9.8` | Node.js `v27.0.0-alpha.0.1.1` | Slice Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`5.118 ms`** (`4.72 ms` Min) | **`5.671 ms`** | **`0.779 ms`** | `5.141 ms` (`-0.4%`) | `14.139 ms` | `36.462 ms` | `54.867 ms` | **`2.76x Bun` (`18.15x` Warm) / `10.72x Node`** |
| **02** | `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | **`11.667 ms`** (`10.75 ms` Min) | **`12.365 ms`** | **`3.248 ms`** | `11.282 ms` (`3.4%`) | `22.748 ms` | `52.603 ms` | `64.013 ms` | **`1.95x Bun` (`7.00x` Warm) / `5.49x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`6.059 ms`** (`5.62 ms` Min) | **`6.519 ms`** | **`1.146 ms`** | `6.087 ms` (`-0.5%`) | `15.081 ms` | `43.440 ms` | `56.422 ms` | **`2.49x Bun` (`13.16x` Warm) / `9.31x Node`** |
| **04** | `04-edge-api-gateway` (`fast-querystring` + `ms`) | **`6.398 ms`** (`5.91 ms` Min) | **`6.862 ms`** | **`1.323 ms`** | `6.235 ms` (`2.6%`) | `16.388 ms` | `43.218 ms` | `56.786 ms` | **`2.56x Bun` (`12.39x` Warm) / `8.88x Node`** |
| **05** | `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | **`8.601 ms`** (`7.99 ms` Min) | **`9.287 ms`** | **`4.587 ms`** | `8.691 ms` (`-1.0%`) | `15.672 ms` | `45.377 ms` | `57.464 ms` | **`1.82x Bun` (`3.42x` Warm) / `6.68x Node`** |
| **06** | `06-telemetry-metrics-pipeline` (`bytes` + `flatted` + WAL) | **`10.946 ms`** (`10.17 ms` Min) | **`11.689 ms`** | **`5.373 ms`** | `10.784 ms` (`1.5%`) | `18.376 ms` | `49.069 ms` | `58.897 ms` | **`1.68x Bun` (`3.42x` Warm) / `5.38x Node`** |
| **07** | `07-graphql-ast-validator` (`dlv` + `dset` + AST Cost) | **`7.741 ms`** (`7.07 ms` Min) | **`8.071 ms`** | **`3.679 ms`** | `7.417 ms` (`4.4%`) | `15.851 ms` | `41.978 ms` | `56.223 ms` | **`2.05x Bun` (`4.31x` Warm) / `7.26x Node`** |
| **08** | `08-event-driven-state-journal` (`mitt` + `ms` + JSONL Replay) | **`6.225 ms`** (`5.74 ms` Min) | **`6.717 ms`** | **`1.166 ms`** | `6.235 ms` (`-0.2%`) | `17.149 ms` | `46.349 ms` | `55.906 ms` | **`2.75x Bun` (`14.71x` Warm) / `8.98x Node`** |
| **09** | `09-wintertc-web-router` (`Request` + `Response` + `URL`, 80 reqs) | **`12.646 ms`** (`12.02 ms` Min) | **`13.548 ms`** | **`6.792 ms`** | `12.782 ms` (`-1.1%`) | `14.796 ms` | `47.056 ms` | `79.749 ms` | **`1.17x Bun` (`2.18x` Warm) / `6.31x Node`** |
| **10** | `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **`10.563 ms`** (`9.40 ms` Min) | **`10.824 ms`** | **`3.587 ms`** | `9.581 ms` (`10.2%`) | `15.320 ms` | `48.291 ms` | `78.872 ms` | **`1.45x Bun` (`4.27x` Warm) / `7.47x Node`** |
| **11** | `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **`8.717 ms`** (`8.11 ms` Min) | **`9.158 ms`** | **`2.907 ms`** | `8.725 ms` (`-0.1%`) | `14.203 ms` | `48.366 ms` | `80.595 ms` | **`1.63x Bun` (`4.89x` Warm) / `9.25x Node`** |
| **12** | `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP, 320 frames) | **`13.181 ms`** (`12.67 ms` Min) | **`13.563 ms`** | **`6.305 ms`** | `14.380 ms` (`-8.3%`) | `16.255 ms` | `50.970 ms` | `85.921 ms` | **`1.23x Bun` (`2.58x` Warm) / `6.52x Node`** |
| **13** | `13-webhook-orchestrator-ssrf` (`fast-querystring` + `ms` + Live `fetch`) | **`9.999 ms`** (`9.30 ms` Min) | **`10.633 ms`** | **`3.593 ms`** | `10.003 ms` (`-0.0%`) | `20.789 ms` | `52.801 ms` | `104.723 ms` | **`2.08x Bun` (`5.79x` Warm) / `10.47x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`13.592 ms`** (`12.76 ms` Min) | **`13.374 ms`** | **`7.197 ms`** | `13.079 ms` (`3.9%`) | `22.763 ms` | `55.774 ms` | `102.804 ms` | **`1.70x Bun` (`3.16x` Warm) / `7.69x Node`** |
| **15** | `15-time-travel-test-suite` (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **`11.587 ms`** (`10.75 ms` Min) | **`12.243 ms`** | **`3.751 ms`** | `11.097 ms` (`4.4%`) | `26.398 ms` | `67.160 ms` | `77.699 ms` | **`2.28x Bun` (`7.04x` Warm) / `6.71x Node`** |
| **16** | `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **`6.360 ms`** (`5.93 ms` Min) | **`6.926 ms`** | **`1.232 ms`** | `6.312 ms` (`0.8%`) | `15.611 ms` | `45.631 ms` | `60.531 ms` | **`2.45x Bun` (`12.67x` Warm) / `9.52x Node`** |
| **17** | `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive/307/SSRF/DLP) | **`13.222 ms`** (`12.28 ms` Min) | **`13.869 ms`** | **`5.835 ms`** | `13.043 ms` (`1.4%`) | `22.608 ms` | `54.102 ms` | `104.539 ms` | **`1.71x Bun` (`3.87x` Warm) / `7.91x Node`** |
| **18** | `18-async-event-loop-fanout-scheduler` (Timer Wheel/`Promise.all`/`AbortSignal`) | **`28.018 ms`** (`26.63 ms` Min) | **`28.229 ms`** | **`17.164 ms`** | `27.728 ms` (`1.0%`) | `33.936 ms` | `72.180 ms` | `73.727 ms` | **`1.21x Bun` (`1.98x` Warm) / `2.63x Node`** |
| **19** | `19-wintertc-webcrypto-streams-edge` (WebCrypto/Streams/FormData/Clone) | **`8.558 ms`** (`7.78 ms` Min) | **`8.945 ms`** | **`1.443 ms`** | `8.350 ms` (`2.5%`) | `14.701 ms` | `45.960 ms` | `81.393 ms` | **`1.72x Bun` (`10.19x` Warm) / `9.51x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`9.025 ms`** (`8.38 ms` Min) | **`9.402 ms`** | **`0.389 ms`** | `8.929 ms` (`1.1%`) | `23.546 ms` | `55.467 ms` | `68.255 ms` | **`2.61x Bun` (`60.53x` Warm) / `7.56x Node`** |
| **21** | `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket` + 5-Ring Guard) | **`9.141 ms`** (`8.37 ms` Min) | **`9.788 ms`** | **`2.119 ms`** | `9.089 ms` (`0.6%`) | `21.153 ms` | `49.611 ms` | `109.727 ms` | **`2.31x Bun` (`9.98x` Warm) / `12.00x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`28.718 ms`** (`26.98 ms` Min) | **`28.336 ms`** | **`2.350 ms`** | `28.457 ms` (`0.9%`) | `58.534 ms` | `117.150 ms` | `93.791 ms` | **`2.07x Bun` (`24.91x` Warm) / `3.31x Node`** |
| **23** | `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **`13.047 ms`** (`12.41 ms` Min) | **`13.825 ms`** | **`1.429 ms`** | `13.909 ms` (`-6.2%`) | `22.261 ms` | `40.941 ms` | `58.861 ms` | **`1.71x Bun` (`15.58x` Warm) / `4.51x Node`** |
| **24** | `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` SLA Scorer + SSR Pipeline) | **`7.311 ms`** (`6.59 ms` Min) | **`7.619 ms`** | **`1.372 ms`** | `7.182 ms` (`1.8%`) | `19.755 ms` | `43.735 ms` | `60.910 ms` | **`2.70x Bun` (`14.40x` Warm) / `8.33x Node`** |
| **FW-01** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`13.621 ms`** (`12.52 ms` Min) | **`13.478 ms`** | **`4.913 ms`** | `13.715 ms` (`-0.7%`) | `20.649 ms` | `52.653 ms` | `86.125 ms` | **`1.53x Bun` (`4.20x` Warm) / `6.39x Node`** |
| **FW-02** | `FW-02: frameworks/02-itty-router-unjs-nitro-stack` (`itty-router` + UnJS) | **`15.677 ms`** (`14.96 ms` Min) | **`16.450 ms`** | **`8.535 ms`** | `16.557 ms` (`-5.3%`) | `19.686 ms` | `54.609 ms` | `83.149 ms` | **`1.26x Bun` (`2.31x` Warm) / `5.30x Node`** |
| **FW-03** | `FW-03: frameworks/03-node-http-express-compat` (`node:http` + `node:stream` Express/Polka) | **`13.816 ms`** (`13.05 ms` Min) | **`14.247 ms`** | **`0.880 ms`** | `14.182 ms` (`-2.6%`) | `35.435 ms` | `57.075 ms` | `77.605 ms` | **`2.56x Bun` (`40.27x` Warm) / `5.62x Node`** |
| **FW-04** | `FW-04: frameworks/04-sveltekit-edge-ssr` (`@sveltejs/kit@3.1.1` + `svelte@5.57.3` + `devalue@6.0.3`) | **`28.510 ms`** (`27.73 ms` Min) | **`28.825 ms`** | **`13.618 ms`** | `30.776 ms` (`-7.4%`) | `44.885 ms` | `69.070 ms` | `105.166 ms` | **`1.57x Bun` (`3.30x` Warm) / `3.69x Node`** |
| **CS-01** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`27.549 ms`** (`26.07 ms` Min) | **`27.771 ms`** | **`12.010 ms`** | `27.555 ms` (`-0.0%`) | `29.828 ms` | `72.503 ms` | `82.387 ms` | **`1.08x Bun` (`2.48x` Warm) / `2.99x Node`** |
| **CS-02** | `CS-02: case_studies/02-mcp-code-mode-orchestrator` (`-98.4%` Tokens) | **`16.641 ms`** (`15.42 ms` Min) | **`16.592 ms`** | **`2.005 ms`** | `15.535 ms` (`7.1%`) | `17.175 ms` | `43.312 ms` | `60.153 ms` | **`1.04x Bun` (`8.57x` Warm) / `3.63x Node`** |
| **CS-03** | `CS-03: case_studies/03-multi-tenant-saas-plugin-mesh` (12 Tenants + Hot-Swap) | **`12.254 ms`** (`11.34 ms` Min) | **`12.663 ms`** | **`4.720 ms`** | `12.094 ms` (`1.3%`) | `17.277 ms` | `47.958 ms` | `78.845 ms` | **`1.41x Bun` (`3.66x` Warm) / `6.43x Node`** |
| **CS-04** | `CS-04: case_studies/04-wasm-zero-trust-cdr-pipeline` (Wasm SimHash CDR) | **`14.355 ms`** (`13.21 ms` Min) | **`14.224 ms`** | **`1.805 ms`** | `13.882 ms` (`3.4%`) | `20.028 ms` | `44.960 ms` | `59.806 ms` | **`1.41x Bun` (`11.10x` Warm) / `4.20x Node`** |
| **CS-05** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`8.774 ms`** (`8.09 ms` Min) | **`9.122 ms`** | **`2.447 ms`** | `8.374 ms` (`4.8%`) | `21.075 ms` | `63.483 ms` | `77.832 ms` | **`2.40x Bun` (`8.61x` Warm) / `8.87x Node`** |

---

## 4. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Workload | Slice (`SLICEBC\x01` Default) | Bun (`v1.4.5`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha.0.1.1`) | Slice Memory Advantage |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **App 2: NPM semver+ms+ini** | **14,208 KB (13.88 MB)** | 38,656 KB (37.75 MB) | 46,824 KB (45.73 MB) | 70,188 KB (68.54 MB) | **4.94x less than Node / 2.72x less than Bun** |
| **App 6: Telemetry + Flatted** | **14,336 KB (14.00 MB)** | 40,832 KB (39.88 MB) | 50,692 KB (49.50 MB) | 71,752 KB (70.07 MB) | **5.01x less than Node / 2.85x less than Bun** |
| **App 21: RFC 6455 WS + Sec** | **14,976 KB (14.63 MB)** | 37,180 KB (36.31 MB) | 51,104 KB (49.91 MB) | 92,520 KB (90.35 MB) | **6.18x less than Node / 2.48x less than Bun** |
| **App 22: Workers + Sockets** | **22,144 KB (21.63 MB)** | 47,444 KB (46.33 MB) | 63,332 KB (61.85 MB) | 92,028 KB (89.87 MB) | **4.16x less than Node / 2.14x less than Bun** |
| **App 23: Pure-Rust WASM** | **18,688 KB (18.25 MB)** | 37,760 KB (36.88 MB) | 48,848 KB (47.70 MB) | 78,044 KB (76.21 MB) | **4.18x less than Node / 2.02x less than Bun** |
| **FW 1: Hono v4 Microservice** | **15,744 KB (15.38 MB)** | 39,992 KB (39.05 MB) | 49,780 KB (48.61 MB) | 86,348 KB (84.32 MB) | **5.48x less than Node / 2.54x less than Bun** |
| **FW 2: itty-router + UnJS** | **15,104 KB (14.75 MB)** | 40,832 KB (39.88 MB) | 48,880 KB (47.73 MB) | 85,612 KB (83.61 MB) | **5.67x less than Node / 2.70x less than Bun** |
| **App 24: WASM + SSR** | **14,464 KB (14.13 MB)** | 38,144 KB (37.25 MB) | 49,120 KB (47.97 MB) | 78,284 KB (76.45 MB) | **5.41x less than Node / 2.64x less than Bun** |
| **FW 3: Express node:http** | **15,232 KB (14.88 MB)** | 41,472 KB (40.50 MB) | 46,904 KB (45.80 MB) | 79,212 KB (77.36 MB) | **5.20x less than Node / 2.72x less than Bun** |
| **01-tot-autonomous-sre-agent** | **20,096 KB (19.63 MB)** | 41,728 KB (40.75 MB) | 53,108 KB (51.86 MB) | 79,716 KB (77.85 MB) | **3.97x less than Node / 2.08x less than Bun** |
| **02-mcp-code-mode-orchestrator** | **17,760 KB (17.34 MB)** | 37,248 KB (36.38 MB) | 46,912 KB (45.81 MB) | 78,568 KB (76.73 MB) | **4.42x less than Node / 2.10x less than Bun** |
| **03-multi-tenant-saas-plugin-mesh** | **14,848 KB (14.50 MB)** | 38,016 KB (37.13 MB) | 50,116 KB (48.94 MB) | 87,116 KB (85.07 MB) | **5.87x less than Node / 2.56x less than Bun** |
| **04-wasm-zero-trust-cdr-pipeline** | **18,688 KB (18.25 MB)** | 39,040 KB (38.13 MB) | 50,140 KB (48.96 MB) | 78,700 KB (76.86 MB) | **4.21x less than Node / 2.09x less than Bun** |
| **05-time-travel-chaos-fuzzer-payments** | **14,080 KB (13.75 MB)** | 38,784 KB (37.88 MB) | 49,400 KB (48.24 MB) | 77,792 KB (75.97 MB) | **5.53x less than Node / 2.75x less than Bun** |
| **FW 4: SvelteKit 3 + Svelte 5** | **22,240 KB (21.72 MB)** | 50,304 KB (49.13 MB) | 56,720 KB (55.39 MB) | 83,948 KB (81.98 MB) | **3.77x less than Node / 2.26x less than Bun** |

---

## 5. Multi-Tenant Edge KV, AI Agent Checkpoints & 5-Ring Security Telemetry

| Subsystem / Benchmark | Measured Telemetry | Status |
| :--- | :--- | :---: |
| **Sharded `Slice.kv` (`1,001` Get/Set/Incr Ops in Isolate)** | **`671.98 us`** total (`--warm`, `288 B` memory cage) | `PASS` |
| **Multi-Tenant `--tenant-dir` Zero-Downtime Hot-Swap** | Live atomic replacement of `tenant-alpha.cas` (`v1 -> v2-hot-swapped`) with isolated `Slice.kv` state | `PASS` |
| **Deterministic Time-Travel Replay (`--deterministic=20261019`)** | 100% bit-for-bit identical PRNG (`Math.random`) & virtual clock (`Date.now`, `performance.now`) | `PASS` |
| **Tree-of-Thought AI Agent (`Slice.checkpoint` + 3 `evalSandbox` + `restore`)** | **`94.54 us`** total (`2.23 us` child sandbox) | `PASS` |
| **Cloud Metadata SSRF (`169.254.169.254`) & Base64 Egress DLP Guard** | `SSRF_METADATA_BLOCKED` & `EGRESS_SECRET_LEAK_BLOCKED` deterministically trapped | `PASS` |
| **Built-In Model Context Protocol Server (`slice mcp --smoke-test`)** | JSON-RPC 2.0 `initialize`, `tools/list`, `tools/call` verified over `stdio` | `PASS` |
