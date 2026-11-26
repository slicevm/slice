# Slice (Nexus-JS) - Full 33-Workload & MicroVM Benchmark Report

* **Slice Version:** `v0.14.5` (Commit `ca3459d`)
* **Generated Timestamp (UTC):** `2026-11-26T17:49:00Z`
* **Archive File:** [`tests/reports/bench_v0.14.5_20261126_174900Z_ca3459d.md`](./bench_v0.14.5_20261126_174900Z_ca3459d.md)
* **Canonical Latest Report:** [`tests/reports/latest.md`](./latest.md)
* **Environment:** `Linux x86_64 Docker (io_uring SQPOLL + WRPKRU)`
* **Compared Runtimes:**
  * **Slice:** `v0.14.5` (Default `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS + Monomorphic Inline Caching `get_fast` + Iterative ECMA-404 `JSON.parse`)
  * **Bun:** `v1.4.5` (JavaScriptCore)
  * **Deno:** `v2.9.8` (V8)
  * **Node.js:** `v27.0.0-alpha.0.1.1` (V8 native TypeScript execution)

---

## 1. Executive Summary

Across all **33 real-world multi-file TypeScript applications, framework suites, and case studies**, Slice beats **Deno (`v2.9.8`)** on **33/33** workloads, beats **Node.js (`v27.0.0-alpha.0.1.1`)** on **33/33** workloads, beats **Bun (`v1.4.5`)** on **33/33** workloads in cold one-shot execution, and beats **all 3 runtimes on 33/33 workloads in `--warm` pooled isolate execution**.

| Metric | Linux (`x86_64` Docker) | Windows (`x86_64` Host) | Comparison vs Runtimes & MicroVMs |
| :--- | :---: | :---: | :--- |
| **Single Isolate Cold-Start (Min / P50 / P99)** | **`0.81 us` / `1.01 us` / `1.55 us`** | **`0.80 us` / `1.60 us` / `4.50 us`** | **~120,000x faster than 125ms Firecracker VM** |
| **1,000-Tenant MicroVM Swarm (Boot + Exec)** | **`1.28 ms`** (`8.25 MB` RSS) | **`1.94 ms`** (`6.71 MB` RSS) | **> 10,000x faster than container/VM swarms** |
| **500-Module Monorepo Cold Boot (`0 stat()`)** | **`3.030 ms`** (`156.34 us` in-mem) | **`7.585 ms`** (`206.00 us` in-mem) | **23.2x faster than Node.js (`1,500+ stat()`)** |
| **10,000 Direct Kernel Ring I/O Completions** | **`0.01 ms`** | **`3.17 ms`** | **Zero worker-thread lock contention** |
| **5-Ring Security Containment Verification** | **`5 / 5 PASS`** | **`5 / 5 PASS`** | **OCap + SES + Fuel + SSRF/DLP + MPK** |

---

## 2. `slice serve` Multi-Tenant HTTP/1.1 Keep-Alive Throughput (`16,000` Reqs, 8 Sockets x 8 Pipeline)

| Server / Runtime | Throughput (`req/s`) | Avg Latency / Req | Total Time (`16k` Reqs) | Relative Speedup vs Runtimes |
| :--- | :---: | :---: | :---: | :--- |
| **`slice serve` (Tier-0 CAS, `--workers=8`)** | **`158,337 req/s`** | **`6.32 us/req`** | **`101.1 ms`** | **`1.68x` faster than `Bun.serve` / `2.88x` Node.js** |
| **`slice serve` (Tier-1 WinterTC, `--workers=8`)** | **`148,887 req/s`** | **`6.72 us/req`** | **`107.5 ms`** | **`1.58x` faster than `Bun.serve` / `2.71x` Node.js** |
| **`Deno.serve` (`v2.9.8`, V8, No Per-Req Sandbox)** | `94,372 req/s` | `10.60 us/req` | `169.5 ms` | `1.00x` vs Bun |
| **`Bun.serve` (`v1.4.5`, JSC, No Per-Req Sandbox)** | `94,360 req/s` | `10.60 us/req` | `169.6 ms` | `1.00x` (Baseline) |
| **`Node.js http.createServer` (`v27.0.0-alpha.0.1.1`, V8)** | `54,883 req/s` | `18.22 us/req` | `291.5 ms` | `0.58x` vs Bun |

---

## 3. Full 33-Workload Latency Comparison (20 Runs Each, Linux `x86_64` Docker)

| # | Workload (`tests/apps/`, `tests/frameworks/`, `tests/case_studies/`) | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm (`--warm`) | Previous `.cas` (`Delta`) | Bun `v1.4.5` | Deno `v2.9.8` | Node.js `v27.0.0-alpha.0.1.1` | Slice Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`5.292 ms`** (`4.83 ms` Min) | **`5.871 ms`** | **`0.993 ms`** | `5.217 ms` (`1.4%`) | `14.388 ms` | `34.991 ms` | `53.124 ms` | **`2.72x Bun` (`14.49x` Warm) / `10.04x Node`** |
| **02** | `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | **`11.424 ms`** (`10.74 ms` Min) | **`12.236 ms`** | **`3.731 ms`** | `11.686 ms` (`-2.2%`) | `22.670 ms` | `53.674 ms` | `64.104 ms` | **`1.98x Bun` (`6.08x` Warm) / `5.61x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`6.061 ms`** (`5.79 ms` Min) | **`6.656 ms`** | **`1.218 ms`** | `6.183 ms` (`-2.0%`) | `15.118 ms` | `42.976 ms` | `57.534 ms` | **`2.49x Bun` (`12.41x` Warm) / `9.49x Node`** |
| **04** | `04-edge-api-gateway` (`fast-querystring` + `ms`) | **`6.472 ms`** (`6.04 ms` Min) | **`6.871 ms`** | **`1.455 ms`** | `6.534 ms` (`-0.9%`) | `16.099 ms` | `41.321 ms` | `57.157 ms` | **`2.49x Bun` (`11.06x` Warm) / `8.83x Node`** |
| **05** | `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | **`8.843 ms`** (`8.26 ms` Min) | **`9.294 ms`** | **`4.227 ms`** | `8.725 ms` (`1.4%`) | `15.874 ms` | `44.243 ms` | `56.743 ms` | **`1.80x Bun` (`3.76x` Warm) / `6.42x Node`** |
| **06** | `06-telemetry-metrics-pipeline` (`bytes` + `flatted` + WAL) | **`10.578 ms`** (`9.82 ms` Min) | **`11.213 ms`** | **`4.366 ms`** | `11.006 ms` (`-3.9%`) | `18.329 ms` | `46.372 ms` | `59.952 ms` | **`1.73x Bun` (`4.20x` Warm) / `5.67x Node`** |
| **07** | `07-graphql-ast-validator` (`dlv` + `dset` + AST Cost) | **`7.795 ms`** (`7.06 ms` Min) | **`8.224 ms`** | **`3.566 ms`** | `7.533 ms` (`3.5%`) | `14.626 ms` | `41.850 ms` | `55.356 ms` | **`1.88x Bun` (`4.10x` Warm) / `7.10x Node`** |
| **08** | `08-event-driven-state-journal` (`mitt` + `ms` + JSONL Replay) | **`6.291 ms`** (`5.96 ms` Min) | **`6.681 ms`** | **`1.182 ms`** | `6.169 ms` (`2.0%`) | `17.134 ms` | `48.592 ms` | `56.499 ms` | **`2.72x Bun` (`14.50x` Warm) / `8.98x Node`** |
| **09** | `09-wintertc-web-router` (`Request` + `Response` + `URL`, 80 reqs) | **`12.965 ms`** (`12.31 ms` Min) | **`13.194 ms`** | **`6.742 ms`** | `13.614 ms` (`-4.8%`) | `14.934 ms` | `49.282 ms` | `83.060 ms` | **`1.15x Bun` (`2.22x` Warm) / `6.41x Node`** |
| **10** | `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **`10.143 ms`** (`9.30 ms` Min) | **`11.523 ms`** | **`3.965 ms`** | `10.123 ms` (`0.2%`) | `16.609 ms` | `48.660 ms` | `80.823 ms` | **`1.64x Bun` (`4.19x` Warm) / `7.97x Node`** |
| **11** | `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **`8.624 ms`** (`8.31 ms` Min) | **`9.414 ms`** | **`2.897 ms`** | `8.847 ms` (`-2.5%`) | `14.541 ms` | `49.189 ms` | `82.435 ms` | **`1.69x Bun` (`5.02x` Warm) / `9.56x Node`** |
| **12** | `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP, 320 frames) | **`13.562 ms`** (`12.63 ms` Min) | **`14.618 ms`** | **`6.437 ms`** | `13.656 ms` (`-0.7%`) | `16.514 ms` | `50.509 ms` | `85.529 ms` | **`1.22x Bun` (`2.57x` Warm) / `6.31x Node`** |
| **13** | `13-webhook-orchestrator-ssrf` (`fast-querystring` + `ms` + Live `fetch`) | **`10.215 ms`** (`9.74 ms` Min) | **`10.565 ms`** | **`3.569 ms`** | `10.186 ms` (`0.3%`) | `23.808 ms` | `51.799 ms` | `108.274 ms` | **`2.33x Bun` (`6.67x` Warm) / `10.60x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`13.098 ms`** (`12.19 ms` Min) | **`13.053 ms`** | **`6.931 ms`** | `12.991 ms` (`0.8%`) | `23.416 ms` | `53.173 ms` | `107.293 ms` | **`1.79x Bun` (`3.38x` Warm) / `8.22x Node`** |
| **15** | `15-time-travel-test-suite` (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **`11.819 ms`** (`11.14 ms` Min) | **`12.287 ms`** | **`3.598 ms`** | `12.809 ms` (`-7.7%`) | `26.063 ms` | `66.448 ms` | `80.038 ms` | **`2.21x Bun` (`7.24x` Warm) / `6.77x Node`** |
| **16** | `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **`6.913 ms`** (`6.41 ms` Min) | **`6.981 ms`** | **`1.306 ms`** | `6.647 ms` (`4.0%`) | `17.023 ms` | `50.412 ms` | `63.746 ms` | **`2.46x Bun` (`13.03x` Warm) / `9.22x Node`** |
| **17** | `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive/307/SSRF/DLP) | **`13.947 ms`** (`12.91 ms` Min) | **`14.754 ms`** | **`5.842 ms`** | `12.831 ms` (`8.7%`) | `24.023 ms` | `56.667 ms` | `111.382 ms` | **`1.72x Bun` (`4.11x` Warm) / `7.99x Node`** |
| **18** | `18-async-event-loop-fanout-scheduler` (Timer Wheel/`Promise.all`/`AbortSignal`) | **`30.138 ms`** (`27.18 ms` Min) | **`29.144 ms`** | **`17.060 ms`** | `29.449 ms` (`2.3%`) | `33.718 ms` | `72.776 ms` | `77.565 ms` | **`1.16x Bun` (`1.98x` Warm) / `2.66x Node`** |
| **19** | `19-wintertc-webcrypto-streams-edge` (WebCrypto/Streams/FormData/Clone) | **`8.497 ms`** (`7.88 ms` Min) | **`8.914 ms`** | **`1.441 ms`** | `8.471 ms` (`0.3%`) | `15.040 ms` | `44.876 ms` | `81.944 ms` | **`1.77x Bun` (`10.44x` Warm) / `9.64x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`8.905 ms`** (`8.58 ms` Min) | **`9.240 ms`** | **`0.438 ms`** | `9.053 ms` (`-1.6%`) | `23.831 ms` | `54.577 ms` | `68.346 ms` | **`2.68x Bun` (`54.41x` Warm) / `7.68x Node`** |
| **21** | `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket` + 5-Ring Guard) | **`9.588 ms`** (`8.78 ms` Min) | **`9.894 ms`** | **`1.948 ms`** | `9.802 ms` (`-2.2%`) | `20.934 ms` | `50.916 ms` | `117.927 ms` | **`2.18x Bun` (`10.75x` Warm) / `12.30x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`29.742 ms`** (`27.54 ms` Min) | **`28.529 ms`** | **`2.612 ms`** | `28.539 ms` (`4.2%`) | `57.189 ms` | `112.017 ms` | `94.677 ms` | **`2.00x Bun` (`21.89x` Warm) / `3.32x Node`** |
| **23** | `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **`13.516 ms`** (`12.38 ms` Min) | **`13.771 ms`** | **`1.427 ms`** | `15.050 ms` (`-10.2%`) | `22.351 ms` | `42.408 ms` | `60.309 ms` | **`1.65x Bun` (`15.66x` Warm) / `4.46x Node`** |
| **24** | `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` SLA Scorer + SSR Pipeline) | **`7.165 ms`** (`6.58 ms` Min) | **`7.623 ms`** | **`1.375 ms`** | `7.127 ms` (`0.5%`) | `19.683 ms` | `42.454 ms` | `62.986 ms` | **`2.75x Bun` (`14.31x` Warm) / `8.79x Node`** |
| **FW-01** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`17.465 ms`** (`16.36 ms` Min) | **`17.896 ms`** | **`4.889 ms`** | `13.081 ms` (`33.5%`) | `21.225 ms` | `51.898 ms` | `87.896 ms` | **`1.22x Bun` (`4.34x` Warm) / `5.03x Node`** |
| **FW-02** | `FW-02: frameworks/02-itty-router-unjs-nitro-stack` (`itty-router` + UnJS) | **`16.108 ms`** (`15.29 ms` Min) | **`16.563 ms`** | **`8.562 ms`** | `16.392 ms` (`-1.7%`) | `20.278 ms` | `45.713 ms` | `81.309 ms` | **`1.26x Bun` (`2.37x` Warm) / `5.05x Node`** |
| **FW-03** | `FW-03: frameworks/03-node-http-express-compat` (`node:http` + `node:stream` Express/Polka) | **`14.060 ms`** (`13.52 ms` Min) | **`14.916 ms`** | **`0.812 ms`** | `14.261 ms` (`-1.4%`) | `36.206 ms` | `52.120 ms` | `80.911 ms` | **`2.58x Bun` (`44.59x` Warm) / `5.75x Node`** |
| **FW-04** | `FW-04: frameworks/04-sveltekit-edge-ssr` (`@sveltejs/kit@3.1.1` + `svelte@5.57.3` + `devalue@6.0.3`) | **`30.209 ms`** (`28.19 ms` Min) | **`29.128 ms`** | **`13.126 ms`** | `28.901 ms` (`4.5%`) | `45.421 ms` | `69.908 ms` | `106.023 ms` | **`1.56x Bun` (`3.46x` Warm) / `3.64x Node`** |
| **CS-01** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`26.057 ms`** (`3.60 ms` Min) | **`28.378 ms`** | **`11.765 ms`** | `26.779 ms` (`-2.7%`) | `29.446 ms` | `72.134 ms` | `82.315 ms` | **`1.13x Bun` (`2.50x` Warm) / `3.16x Node`** |
| **CS-02** | `CS-02: case_studies/02-mcp-code-mode-orchestrator` (`-98.4%` Tokens) | **`15.434 ms`** (`14.67 ms` Min) | **`16.040 ms`** | **`1.963 ms`** | `15.910 ms` (`-3.0%`) | `17.326 ms` | `42.348 ms` | `60.007 ms` | **`1.12x Bun` (`8.83x` Warm) / `3.89x Node`** |
| **CS-03** | `CS-03: case_studies/03-multi-tenant-saas-plugin-mesh` (12 Tenants + Hot-Swap) | **`12.562 ms`** (`11.65 ms` Min) | **`12.981 ms`** | **`4.287 ms`** | `12.858 ms` (`-2.3%`) | `17.613 ms` | `49.408 ms` | `78.170 ms` | **`1.40x Bun` (`4.11x` Warm) / `6.22x Node`** |
| **CS-04** | `CS-04: case_studies/04-wasm-zero-trust-cdr-pipeline` (Wasm SimHash CDR) | **`13.785 ms`** (`13.05 ms` Min) | **`14.091 ms`** | **`1.888 ms`** | `13.843 ms` (`-0.4%`) | `20.539 ms` | `44.615 ms` | `60.154 ms` | **`1.49x Bun` (`10.88x` Warm) / `4.36x Node`** |
| **CS-05** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`8.893 ms`** (`8.36 ms` Min) | **`9.433 ms`** | **`2.428 ms`** | `9.252 ms` (`-3.9%`) | `21.341 ms` | `62.011 ms` | `79.661 ms` | **`2.40x Bun` (`8.79x` Warm) / `8.96x Node`** |

---

## 4. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Workload | Slice (`SLICEBC\x01` Default) | Bun (`v1.4.5`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha.0.1.1`) | Slice Memory Advantage |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **App 2: NPM semver+ms+ini** | **14,336 KB (14.00 MB)** | 38,912 KB (38.00 MB) | 46,968 KB (45.87 MB) | 70,632 KB (68.98 MB) | **4.93x less than Node / 2.71x less than Bun** |
| **App 6: Telemetry + Flatted** | **14,080 KB (13.75 MB)** | 40,192 KB (39.25 MB) | 51,064 KB (49.87 MB) | 71,784 KB (70.10 MB) | **5.10x less than Node / 2.85x less than Bun** |
| **App 21: RFC 6455 WS + Sec** | **14,848 KB (14.50 MB)** | 37,840 KB (36.95 MB) | 50,892 KB (49.70 MB) | 93,540 KB (91.35 MB) | **6.30x less than Node / 2.55x less than Bun** |
| **App 22: Workers + Sockets** | **22,272 KB (21.75 MB)** | 47,436 KB (46.32 MB) | 63,460 KB (61.97 MB) | 90,048 KB (87.94 MB) | **4.04x less than Node / 2.13x less than Bun** |
| **App 23: Pure-Rust WASM** | **18,944 KB (18.50 MB)** | 37,992 KB (37.10 MB) | 48,432 KB (47.30 MB) | 77,560 KB (75.74 MB) | **4.09x less than Node / 2.01x less than Bun** |
| **FW 1: Hono v4 Microservice** | **17,408 KB (17.00 MB)** | 40,448 KB (39.50 MB) | 50,516 KB (49.33 MB) | 87,116 KB (85.07 MB) | **5.00x less than Node / 2.32x less than Bun** |
| **FW 2: itty-router + UnJS** | **14,592 KB (14.25 MB)** | 40,832 KB (39.88 MB) | 48,752 KB (47.61 MB) | 86,252 KB (84.23 MB) | **5.91x less than Node / 2.80x less than Bun** |
| **App 24: WASM + SSR** | **14,208 KB (13.88 MB)** | 37,888 KB (37.00 MB) | 49,280 KB (48.13 MB) | 78,304 KB (76.47 MB) | **5.51x less than Node / 2.67x less than Bun** |
| **FW 3: Express node:http** | **15,360 KB (15.00 MB)** | 41,344 KB (40.38 MB) | 47,084 KB (45.98 MB) | 80,848 KB (78.95 MB) | **5.26x less than Node / 2.69x less than Bun** |
| **01-tot-autonomous-sre-agent** | **20,352 KB (19.88 MB)** | 40,576 KB (39.63 MB) | 53,100 KB (51.86 MB) | 80,612 KB (78.72 MB) | **3.96x less than Node / 1.99x less than Bun** |
| **02-mcp-code-mode-orchestrator** | **17,760 KB (17.34 MB)** | 37,248 KB (36.38 MB) | 46,736 KB (45.64 MB) | 78,564 KB (76.72 MB) | **4.42x less than Node / 2.10x less than Bun** |
| **03-multi-tenant-saas-plugin-mesh** | **14,976 KB (14.63 MB)** | 37,888 KB (37.00 MB) | 50,032 KB (48.86 MB) | 86,888 KB (84.85 MB) | **5.80x less than Node / 2.53x less than Bun** |
| **04-wasm-zero-trust-cdr-pipeline** | **18,816 KB (18.38 MB)** | 39,140 KB (38.22 MB) | 50,048 KB (48.88 MB) | 78,440 KB (76.60 MB) | **4.17x less than Node / 2.08x less than Bun** |
| **05-time-travel-chaos-fuzzer-payments** | **14,080 KB (13.75 MB)** | 38,912 KB (38.00 MB) | 49,664 KB (48.50 MB) | 77,920 KB (76.09 MB) | **5.53x less than Node / 2.76x less than Bun** |
| **FW 4: SvelteKit 3 + Svelte 5** | **22,492 KB (21.96 MB)** | 49,792 KB (48.63 MB) | 56,724 KB (55.39 MB) | 84,336 KB (82.36 MB) | **3.75x less than Node / 2.21x less than Bun** |

---

## 5. Multi-Tenant Edge KV, AI Agent Checkpoints & 5-Ring Security Telemetry

| Subsystem / Benchmark | Measured Telemetry | Status |
| :--- | :--- | :---: |
| **Sharded `Slice.kv` (`1,001` Get/Set/Incr Ops in Isolate)** | **`541.58 us`** total (`--warm`, `288 B` memory cage) | `PASS` |
| **Multi-Tenant `--tenant-dir` Zero-Downtime Hot-Swap** | Live atomic replacement of `tenant-alpha.cas` (`v1 -> v2-hot-swapped`) with isolated `Slice.kv` state | `PASS` |
| **Deterministic Time-Travel Replay (`--deterministic=20261019`)** | 100% bit-for-bit identical PRNG (`Math.random`) & virtual clock (`Date.now`, `performance.now`) | `PASS` |
| **Tree-of-Thought AI Agent (`Slice.checkpoint` + 3 `evalSandbox` + `restore`)** | **`54.00 us`** total (`1.75 us` child sandbox) | `PASS` |
| **Cloud Metadata SSRF (`169.254.169.254`) & Base64 Egress DLP Guard** | `SSRF_METADATA_BLOCKED` & `EGRESS_SECRET_LEAK_BLOCKED` deterministically trapped | `PASS` |
| **Built-In Model Context Protocol Server (`slice mcp --smoke-test`)** | JSON-RPC 2.0 `initialize`, `tools/list`, `tools/call` verified over `stdio` | `PASS` |
