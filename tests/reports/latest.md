# Slice (Nexus-JS) - Full 33-Workload & MicroVM Benchmark Report

* **Slice Version:** `v0.14.2` (Commit `d9b9f44`)
* **Generated Timestamp (UTC):** `2026-11-25T05:43:42Z`
* **Archive File:** [`tests/reports/bench_v0.14.2_20261125_054342Z_d9b9f44.md`](./bench_v0.14.2_20261125_054342Z_d9b9f44.md)
* **Canonical Latest Report:** [`tests/reports/latest.md`](./latest.md)
* **Environment:** `Linux x86_64 Docker (io_uring SQPOLL + WRPKRU)`
* **Compared Runtimes:**
  * **Slice:** `v0.14.2` (Default `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS + Monomorphic Inline Caching `get_fast` + Iterative ECMA-404 `JSON.parse`)
  * **Bun:** `v1.4.5` (JavaScriptCore)
  * **Deno:** `v2.9.8` (V8)
  * **Node.js:** `v27.0.0-alpha.0.1.1` (V8 native TypeScript execution)

---

## 1. Executive Summary

Across all **33 real-world multi-file TypeScript applications, framework suites, and case studies**, Slice beats **Deno (`v2.9.8`)** on **33/33** workloads, beats **Node.js (`v27.0.0-alpha.0.1.1`)** on **33/33** workloads, beats **Bun (`v1.4.5`)** on **33/33** workloads in cold one-shot execution, and beats **all 3 runtimes on 33/33 workloads in `--warm` pooled isolate execution**.

| Metric | Linux (`x86_64` Docker) | Windows (`x86_64` Host) | Comparison vs Runtimes & MicroVMs |
| :--- | :---: | :---: | :--- |
| **Single Isolate Cold-Start (Min / P50 / P99)** | **`0.76 us` / `0.81 us` / `1.48 us`** | **`0.80 us` / `0.90 us` / `3.80 us`** | **~120,000x faster than 125ms Firecracker VM** |
| **1,000-Tenant MicroVM Swarm (Boot + Exec)** | **`1.02 ms`** (`8.25 MB` RSS) | **`1.36 ms`** (`6.69 MB` RSS) | **> 10,000x faster than container/VM swarms** |
| **500-Module Monorepo Cold Boot (`0 stat()`)** | **`2.902 ms`** (`107.49 us` in-mem) | **`7.412 ms`** (`130.20 us` in-mem) | **24.3x faster than Node.js (`1,500+ stat()`)** |
| **10,000 Direct Kernel Ring I/O Completions** | **`0.01 ms`** | **`3.04 ms`** | **Zero worker-thread lock contention** |
| **5-Ring Security Containment Verification** | **`5 / 5 PASS`** | **`5 / 5 PASS`** | **OCap + SES + Fuel + SSRF/DLP + MPK** |

---

## 2. `slice serve` Multi-Tenant HTTP/1.1 Keep-Alive Throughput (`16,000` Reqs, 8 Sockets x 8 Pipeline)

| Server / Runtime | Throughput (`req/s`) | Avg Latency / Req | Total Time (`16k` Reqs) | Relative Speedup vs Runtimes |
| :--- | :---: | :---: | :---: | :--- |
| **`slice serve` (Tier-0 CAS, `--workers=8`)** | **`163,475 req/s`** | **`6.12 us/req`** | **`97.9 ms`** | **`1.69x` faster than `Bun.serve` / `2.91x` Node.js** |
| **`slice serve` (Tier-1 WinterTC, `--workers=8`)** | **`154,290 req/s`** | **`6.48 us/req`** | **`103.7 ms`** | **`1.60x` faster than `Bun.serve` / `2.75x` Node.js** |
| **`Deno.serve` (`v2.9.8`, V8, No Per-Req Sandbox)** | `96,672 req/s` | `10.34 us/req` | `165.5 ms` | `1.00x` vs Bun |
| **`Bun.serve` (`v1.4.5`, JSC, No Per-Req Sandbox)** | `96,504 req/s` | `10.36 us/req` | `165.8 ms` | `1.00x` (Baseline) |
| **`Node.js http.createServer` (`v27.0.0-alpha.0.1.1`, V8)** | `56,207 req/s` | `17.79 us/req` | `284.7 ms` | `0.58x` vs Bun |

---

## 3. Full 33-Workload Latency Comparison (20 Runs Each, Linux `x86_64` Docker)

| # | Workload (`tests/apps/`, `tests/frameworks/`, `tests/case_studies/`) | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm (`--warm`) | Previous `.cas` (`Delta`) | Bun `v1.4.5` | Deno `v2.9.8` | Node.js `v27.0.0-alpha.0.1.1` | Slice Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`5.141 ms`** (`4.61 ms` Min) | **`5.544 ms`** | **`0.829 ms`** | `5.289 ms` (`-2.8%`) | `14.594 ms` | `34.673 ms` | `52.101 ms` | **`2.84x Bun` (`17.60x` Warm) / `10.13x Node`** |
| **02** | `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | **`11.282 ms`** (`10.67 ms` Min) | **`11.907 ms`** | **`3.215 ms`** | `11.749 ms` (`-4.0%`) | `22.573 ms` | `52.979 ms` | `64.220 ms` | **`2.00x Bun` (`7.02x` Warm) / `5.69x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`6.087 ms`** (`5.58 ms` Min) | **`6.473 ms`** | **`1.147 ms`** | `6.179 ms` (`-1.5%`) | `15.372 ms` | `41.712 ms` | `54.883 ms` | **`2.53x Bun` (`13.40x` Warm) / `9.02x Node`** |
| **04** | `04-edge-api-gateway` (`fast-querystring` + `ms`) | **`6.235 ms`** (`5.81 ms` Min) | **`6.646 ms`** | **`1.454 ms`** | `6.354 ms` (`-1.9%`) | `15.848 ms` | `42.060 ms` | `56.357 ms` | **`2.54x Bun` (`10.90x` Warm) / `9.04x Node`** |
| **05** | `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | **`8.691 ms`** (`8.18 ms` Min) | **`9.279 ms`** | **`4.229 ms`** | `8.593 ms` (`1.1%`) | `15.789 ms` | `42.422 ms` | `56.841 ms` | **`1.82x Bun` (`3.73x` Warm) / `6.54x Node`** |
| **06** | `06-telemetry-metrics-pipeline` (`bytes` + `flatted` + WAL) | **`10.784 ms`** (`10.29 ms` Min) | **`11.441 ms`** | **`5.024 ms`** | `10.719 ms` (`0.6%`) | `18.372 ms` | `47.640 ms` | `58.420 ms` | **`1.70x Bun` (`3.66x` Warm) / `5.42x Node`** |
| **07** | `07-graphql-ast-validator` (`dlv` + `dset` + AST Cost) | **`7.417 ms`** (`6.73 ms` Min) | **`7.751 ms`** | **`3.472 ms`** | `7.451 ms` (`-0.5%`) | `14.020 ms` | `38.448 ms` | `54.210 ms` | **`1.89x Bun` (`4.04x` Warm) / `7.31x Node`** |
| **08** | `08-event-driven-state-journal` (`mitt` + `ms` + JSONL Replay) | **`6.235 ms`** (`5.73 ms` Min) | **`6.461 ms`** | **`1.169 ms`** | `6.200 ms` (`0.6%`) | `16.783 ms` | `46.563 ms` | `59.748 ms` | **`2.69x Bun` (`14.36x` Warm) / `9.58x Node`** |
| **09** | `09-wintertc-web-router` (`Request` + `Response` + `URL`, 80 reqs) | **`12.782 ms`** (`11.99 ms` Min) | **`13.247 ms`** | **`6.562 ms`** | `13.224 ms` (`-3.3%`) | `15.293 ms` | `47.171 ms` | `80.673 ms` | **`1.20x Bun` (`2.33x` Warm) / `6.31x Node`** |
| **10** | `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **`9.581 ms`** (`9.11 ms` Min) | **`10.369 ms`** | **`4.261 ms`** | `10.145 ms` (`-5.6%`) | `15.384 ms` | `48.368 ms` | `77.571 ms` | **`1.61x Bun` (`3.61x` Warm) / `8.10x Node`** |
| **11** | `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **`8.725 ms`** (`8.23 ms` Min) | **`9.333 ms`** | **`3.138 ms`** | `8.939 ms` (`-2.4%`) | `14.379 ms` | `56.379 ms` | `90.736 ms` | **`1.65x Bun` (`4.58x` Warm) / `10.40x Node`** |
| **12** | `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP, 320 frames) | **`14.380 ms`** (`12.80 ms` Min) | **`14.154 ms`** | **`6.450 ms`** | `13.153 ms` (`9.3%`) | `18.161 ms` | `58.674 ms` | `87.194 ms` | **`1.28x Bun` (`2.82x` Warm) / `6.16x Node`** |
| **13** | `13-webhook-orchestrator-ssrf` (`fast-querystring` + `ms` + Live `fetch`) | **`10.003 ms`** (`9.40 ms` Min) | **`10.570 ms`** | **`3.348 ms`** | `10.002 ms` (`0.0%`) | `22.380 ms` | `52.807 ms` | `106.287 ms` | **`2.24x Bun` (`6.68x` Warm) / `10.63x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`13.079 ms`** (`11.97 ms` Min) | **`12.926 ms`** | **`6.593 ms`** | `12.834 ms` (`1.9%`) | `22.649 ms` | `51.308 ms` | `101.802 ms` | **`1.75x Bun` (`3.44x` Warm) / `7.88x Node`** |
| **15** | `15-time-travel-test-suite` (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **`11.097 ms`** (`10.80 ms` Min) | **`12.483 ms`** | **`3.725 ms`** | `12.121 ms` (`-8.4%`) | `26.767 ms` | `65.877 ms` | `76.805 ms` | **`2.41x Bun` (`7.19x` Warm) / `6.92x Node`** |
| **16** | `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **`6.312 ms`** (`5.90 ms` Min) | **`6.895 ms`** | **`1.240 ms`** | `6.487 ms` (`-2.7%`) | `15.650 ms` | `46.816 ms` | `58.083 ms` | **`2.48x Bun` (`12.62x` Warm) / `9.20x Node`** |
| **17** | `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive/307/SSRF/DLP) | **`13.043 ms`** (`12.50 ms` Min) | **`13.232 ms`** | **`5.548 ms`** | `13.047 ms` (`-0.0%`) | `22.301 ms` | `53.673 ms` | `103.658 ms` | **`1.71x Bun` (`4.02x` Warm) / `7.95x Node`** |
| **18** | `18-async-event-loop-fanout-scheduler` (Timer Wheel/`Promise.all`/`AbortSignal`) | **`27.728 ms`** (`26.14 ms` Min) | **`28.716 ms`** | **`17.200 ms`** | `28.102 ms` (`-1.3%`) | `34.348 ms` | `72.899 ms` | `73.220 ms` | **`1.24x Bun` (`2.00x` Warm) / `2.64x Node`** |
| **19** | `19-wintertc-webcrypto-streams-edge` (WebCrypto/Streams/FormData/Clone) | **`8.350 ms`** (`7.80 ms` Min) | **`8.810 ms`** | **`1.430 ms`** | `8.315 ms` (`0.4%`) | `14.495 ms` | `45.453 ms` | `81.405 ms` | **`1.74x Bun` (`10.14x` Warm) / `9.75x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`8.929 ms`** (`8.33 ms` Min) | **`9.476 ms`** | **`0.455 ms`** | `9.148 ms` (`-2.4%`) | `23.419 ms` | `53.464 ms` | `67.436 ms` | **`2.62x Bun` (`51.47x` Warm) / `7.55x Node`** |
| **21** | `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket` + 5-Ring Guard) | **`9.089 ms`** (`8.55 ms` Min) | **`9.810 ms`** | **`2.015 ms`** | `9.202 ms` (`-1.2%`) | `20.743 ms` | `51.854 ms` | `114.249 ms` | **`2.28x Bun` (`10.29x` Warm) / `12.57x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`28.457 ms`** (`27.04 ms` Min) | **`28.385 ms`** | **`2.436 ms`** | `29.712 ms` (`-4.2%`) | `57.232 ms` | `110.518 ms` | `93.970 ms` | **`2.02x Bun` (`23.49x` Warm) / `3.31x Node`** |
| **23** | `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **`13.909 ms`** (`13.00 ms` Min) | **`13.595 ms`** | **`1.433 ms`** | `13.876 ms` (`0.2%`) | `23.826 ms` | `40.621 ms` | `58.284 ms` | **`1.75x Bun` (`16.63x` Warm) / `4.29x Node`** |
| **24** | `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` SLA Scorer + SSR Pipeline) | **`7.182 ms`** (`6.61 ms` Min) | **`7.765 ms`** | **`1.362 ms`** | `7.438 ms` (`-3.4%`) | `19.869 ms` | `42.866 ms` | `61.439 ms` | **`2.77x Bun` (`14.59x` Warm) / `8.55x Node`** |
| **FW-01** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`13.715 ms`** (`12.96 ms` Min) | **`14.075 ms`** | **`5.717 ms`** | `13.266 ms` (`3.4%`) | `21.640 ms` | `52.086 ms` | `85.707 ms` | **`1.58x Bun` (`3.79x` Warm) / `6.25x Node`** |
| **FW-02** | `FW-02: frameworks/02-itty-router-unjs-nitro-stack` (`itty-router` + UnJS) | **`16.557 ms`** (`15.54 ms` Min) | **`16.792 ms`** | **`8.813 ms`** | `15.817 ms` (`4.7%`) | `19.586 ms` | `46.271 ms` | `81.021 ms` | **`1.18x Bun` (`2.22x` Warm) / `4.89x Node`** |
| **FW-03** | `FW-03: frameworks/03-node-http-express-compat` (`node:http` + `node:stream` Express/Polka) | **`14.182 ms`** (`13.22 ms` Min) | **`15.792 ms`** | **`0.897 ms`** | `13.806 ms` (`2.7%`) | `35.587 ms` | `51.115 ms` | `78.965 ms` | **`2.51x Bun` (`39.67x` Warm) / `5.57x Node`** |
| **FW-04** | `FW-04: frameworks/04-sveltekit-edge-ssr` (`@sveltejs/kit@3.1.1` + `svelte@5.57.3` + `devalue@6.0.3`) | **`30.776 ms`** (`28.80 ms` Min) | **`30.019 ms`** | **`13.456 ms`** | `29.551 ms` (`4.1%`) | `44.821 ms` | `68.258 ms` | `105.211 ms` | **`1.49x Bun` (`3.33x` Warm) / `3.50x Node`** |
| **CS-01** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`27.555 ms`** (`26.10 ms` Min) | **`27.063 ms`** | **`12.056 ms`** | `27.058 ms` (`1.8%`) | `31.057 ms` | `70.686 ms` | `79.436 ms` | **`1.15x Bun` (`2.58x` Warm) / `2.94x Node`** |
| **CS-02** | `CS-02: case_studies/02-mcp-code-mode-orchestrator` (`-98.4%` Tokens) | **`15.535 ms`** (`14.77 ms` Min) | **`15.797 ms`** | **`2.010 ms`** | `15.358 ms` (`1.2%`) | `17.330 ms` | `43.659 ms` | `58.832 ms` | **`1.12x Bun` (`8.62x` Warm) / `3.79x Node`** |
| **CS-03** | `CS-03: case_studies/03-multi-tenant-saas-plugin-mesh` (12 Tenants + Hot-Swap) | **`12.094 ms`** (`11.47 ms` Min) | **`12.420 ms`** | **`4.148 ms`** | `11.933 ms` (`1.3%`) | `17.380 ms` | `46.793 ms` | `78.040 ms` | **`1.44x Bun` (`4.19x` Warm) / `6.45x Node`** |
| **CS-04** | `CS-04: case_studies/04-wasm-zero-trust-cdr-pipeline` (Wasm SimHash CDR) | **`13.882 ms`** (`13.29 ms` Min) | **`14.015 ms`** | **`1.742 ms`** | `13.687 ms` (`1.4%`) | `19.738 ms` | `45.897 ms` | `60.279 ms` | **`1.42x Bun` (`11.33x` Warm) / `4.34x Node`** |
| **CS-05** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`8.374 ms`** (`7.86 ms` Min) | **`9.089 ms`** | **`2.388 ms`** | `8.811 ms` (`-5.0%`) | `21.074 ms` | `64.811 ms` | `83.166 ms` | **`2.52x Bun` (`8.82x` Warm) / `9.93x Node`** |

---

## 4. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Workload | Slice (`SLICEBC\x01` Default) | Bun (`v1.4.5`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha.0.1.1`) | Slice Memory Advantage |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **App 2: NPM semver+ms+ini** | **14,208 KB (13.88 MB)** | 39,296 KB (38.38 MB) | 47,496 KB (46.38 MB) | 71,140 KB (69.47 MB) | **5.01x less than Node / 2.77x less than Bun** |
| **App 6: Telemetry + Flatted** | **14,592 KB (14.25 MB)** | 41,856 KB (40.88 MB) | 51,956 KB (50.74 MB) | 72,656 KB (70.95 MB) | **4.98x less than Node / 2.87x less than Bun** |
| **App 21: RFC 6455 WS + Sec** | **14,848 KB (14.50 MB)** | 38,752 KB (37.84 MB) | 51,372 KB (50.17 MB) | 94,308 KB (92.10 MB) | **6.35x less than Node / 2.61x less than Bun** |
| **App 22: Workers + Sockets** | **21,888 KB (21.38 MB)** | 48,324 KB (47.19 MB) | 64,408 KB (62.90 MB) | 92,216 KB (90.05 MB) | **4.21x less than Node / 2.21x less than Bun** |
| **App 23: Pure-Rust WASM** | **18,560 KB (18.13 MB)** | 38,912 KB (38.00 MB) | 49,948 KB (48.78 MB) | 78,948 KB (77.10 MB) | **4.25x less than Node / 2.10x less than Bun** |
| **FW 1: Hono v4 Microservice** | **16,128 KB (15.75 MB)** | 41,120 KB (40.16 MB) | 50,292 KB (49.11 MB) | 87,764 KB (85.71 MB) | **5.44x less than Node / 2.55x less than Bun** |
| **FW 2: itty-router + UnJS** | **15,104 KB (14.75 MB)** | 41,856 KB (40.88 MB) | 49,024 KB (47.88 MB) | 88,168 KB (86.10 MB) | **5.84x less than Node / 2.77x less than Bun** |
| **App 24: WASM + SSR** | **14,464 KB (14.13 MB)** | 41,344 KB (40.38 MB) | 49,068 KB (47.92 MB) | 79,216 KB (77.36 MB) | **5.48x less than Node / 2.86x less than Bun** |
| **FW 3: Express node:http** | **15,232 KB (14.88 MB)** | 42,368 KB (41.38 MB) | 47,860 KB (46.74 MB) | 81,644 KB (79.73 MB) | **5.36x less than Node / 2.78x less than Bun** |
| **01-tot-autonomous-sre-agent** | **20,224 KB (19.75 MB)** | 42,368 KB (41.38 MB) | 54,248 KB (52.98 MB) | 81,992 KB (80.07 MB) | **4.05x less than Node / 2.09x less than Bun** |
| **02-mcp-code-mode-orchestrator** | **17,504 KB (17.09 MB)** | 38,528 KB (37.63 MB) | 47,336 KB (46.23 MB) | 79,304 KB (77.45 MB) | **4.53x less than Node / 2.20x less than Bun** |
| **03-multi-tenant-saas-plugin-mesh** | **14,592 KB (14.25 MB)** | 36,864 KB (36.00 MB) | 50,808 KB (49.62 MB) | 88,552 KB (86.48 MB) | **6.07x less than Node / 2.53x less than Bun** |
| **04-wasm-zero-trust-cdr-pipeline** | **18,816 KB (18.38 MB)** | 40,448 KB (39.50 MB) | 50,472 KB (49.29 MB) | 79,844 KB (77.97 MB) | **4.24x less than Node / 2.15x less than Bun** |
| **05-time-travel-chaos-fuzzer-payments** | **14,336 KB (14.00 MB)** | 39,936 KB (39.00 MB) | 50,292 KB (49.11 MB) | 82,404 KB (80.47 MB) | **5.75x less than Node / 2.79x less than Bun** |
| **FW 4: SvelteKit 3 + Svelte 5** | **22,360 KB (21.84 MB)** | 50,816 KB (49.63 MB) | 56,712 KB (55.38 MB) | 85,992 KB (83.98 MB) | **3.85x less than Node / 2.27x less than Bun** |

---

## 5. Multi-Tenant Edge KV, AI Agent Checkpoints & 5-Ring Security Telemetry

| Subsystem / Benchmark | Measured Telemetry | Status |
| :--- | :--- | :---: |
| **Sharded `Slice.kv` (`1,001` Get/Set/Incr Ops in Isolate)** | **`617.30 us`** total (`--warm`, `93 B` memory cage) | `PASS` |
| **Multi-Tenant `--tenant-dir` Zero-Downtime Hot-Swap** | Live atomic replacement of `tenant-alpha.cas` (`v1 -> v2-hot-swapped`) with isolated `Slice.kv` state | `PASS` |
| **Deterministic Time-Travel Replay (`--deterministic=20261019`)** | 100% bit-for-bit identical PRNG (`Math.random`) & virtual clock (`Date.now`, `performance.now`) | `PASS` |
| **Tree-of-Thought AI Agent (`Slice.checkpoint` + 3 `evalSandbox` + `restore`)** | **`53.41 us`** total (`4.04 us` child sandbox) | `PASS` |
| **Cloud Metadata SSRF (`169.254.169.254`) & Base64 Egress DLP Guard** | `SSRF_METADATA_BLOCKED` & `EGRESS_SECRET_LEAK_BLOCKED` deterministically trapped | `PASS` |
| **Built-In Model Context Protocol Server (`slice mcp --smoke-test`)** | JSON-RPC 2.0 `initialize`, `tools/list`, `tools/call` verified over `stdio` | `PASS` |
