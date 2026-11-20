# Slice (Nexus-JS) - Full 33-Workload & MicroVM Benchmark Report

* **Slice Version:** `v0.13.5` (Commit `0adaae5`)
* **Generated Timestamp (UTC):** `2026-11-20T15:49:56Z`
* **Archive File:** [`tests/reports/bench_v0.13.5_20261120_154956Z_0adaae5.md`](./bench_v0.13.5_20261120_154956Z_0adaae5.md)
* **Canonical Latest Report:** [`tests/reports/latest.md`](./latest.md)
* **Environment:** `Linux x86_64 Docker (io_uring SQPOLL + WRPKRU)`
* **Compared Runtimes:**
  * **Slice:** `v0.13.5` (Default `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS + Monomorphic Inline Caching `get_fast` + Iterative ECMA-404 `JSON.parse`)
  * **Bun:** `v1.4.4` (JavaScriptCore)
  * **Deno:** `v2.9.8` (V8)
  * **Node.js:** `v27.0.0-alpha.0.0.0` (V8 native TypeScript execution)

---

## 1. Executive Summary

Across all **33 real-world multi-file TypeScript applications, framework suites, and case studies**, Slice beats **Deno (`v2.9.8`)** on **33/33** workloads, beats **Node.js (`v27.0.0-alpha.0.0.0`)** on **33/33** workloads, beats **Bun (`v1.4.4`)** on **33/33** workloads in cold one-shot execution, and beats **all 3 runtimes on 33/33 workloads in `--warm` pooled isolate execution**.

| Metric | Linux (`x86_64` Docker) | Windows (`x86_64` Host) | Comparison vs Runtimes & MicroVMs |
| :--- | :---: | :---: | :--- |
| **Single Isolate Cold-Start (Min / P50 / P99)** | **`0.97 us` / `1.05 us` / `3.19 us`** | **`0.70 us` / `0.80 us` / `2.10 us`** | **~120,000x faster than 125ms Firecracker VM** |
| **1,000-Tenant MicroVM Swarm (Boot + Exec)** | **`1.32 ms`** (`8.00 MB` RSS) | **`1.09 ms`** (`6.72 MB` RSS) | **> 10,000x faster than container/VM swarms** |
| **500-Module Monorepo Cold Boot (`0 stat()`)** | **`4.719 ms`** (`4379.68 us` in-mem) | **`10.625 ms`** (`3937.20 us` in-mem) | **14.9x faster than Node.js (`1,500+ stat()`)** |
| **10,000 Direct Kernel Ring I/O Completions** | **`0.01 ms`** | **`3.19 ms`** | **Zero worker-thread lock contention** |
| **5-Ring Security Containment Verification** | **`5 / 5 PASS`** | **`5 / 5 PASS`** | **OCap + SES + Fuel + SSRF/DLP + MPK** |

---

## 2. `slice serve` Multi-Tenant HTTP/1.1 Keep-Alive Throughput (`16,000` Reqs, 8 Sockets x 8 Pipeline)

| Server / Runtime | Throughput (`req/s`) | Avg Latency / Req | Total Time (`16k` Reqs) | Relative Speedup vs Runtimes |
| :--- | :---: | :---: | :---: | :--- |
| **`slice serve` (Tier-0 CAS, `--workers=8`)** | **`161,440 req/s`** | **`6.19 us/req`** | **`99.1 ms`** | **`1.51x` faster than `Bun.serve` / `2.82x` Node.js** |
| **`slice serve` (Tier-1 WinterTC, `--workers=8`)** | **`158,435 req/s`** | **`6.31 us/req`** | **`101.0 ms`** | **`1.48x` faster than `Bun.serve` / `2.77x` Node.js** |
| **`Deno.serve` (`v2.9.8`, V8, No Per-Req Sandbox)** | `107,744 req/s` | `9.28 us/req` | `148.5 ms` | `1.01x` vs Bun |
| **`Bun.serve` (`v1.4.4`, JSC, No Per-Req Sandbox)** | `106,893 req/s` | `9.36 us/req` | `149.7 ms` | `1.00x` (Baseline) |
| **`Node.js http.createServer` (`v27.0.0-alpha.0.0.0`, V8)** | `57,221 req/s` | `17.48 us/req` | `279.6 ms` | `0.54x` vs Bun |

---

## 3. Full 33-Workload Latency Comparison (20 Runs Each, Linux `x86_64` Docker)

| # | Workload (`tests/apps/`, `tests/frameworks/`, `tests/case_studies/`) | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm (`--warm`) | Previous `.cas` (`Delta`) | Bun `v1.4.4` | Deno `v2.9.8` | Node.js `v27.0.0-alpha.0.0.0` | Slice Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`5.383 ms`** (`4.82 ms` Min) | **`5.480 ms`** | **`0.786 ms`** | `5.837 ms` (`-7.8%`) | `13.187 ms` | `36.224 ms` | `59.242 ms` | **`2.45x Bun` (`16.78x` Warm) / `11.01x Node`** |
| **02** | `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | **`11.157 ms`** (`10.73 ms` Min) | **`11.892 ms`** | **`3.335 ms`** | `12.388 ms` (`-9.9%`) | `21.101 ms` | `52.650 ms` | `67.710 ms` | **`1.89x Bun` (`6.33x` Warm) / `6.07x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`6.010 ms`** (`5.63 ms` Min) | **`6.718 ms`** | **`1.081 ms`** | `6.744 ms` (`-10.9%`) | `13.621 ms` | `41.176 ms` | `61.071 ms` | **`2.27x Bun` (`12.60x` Warm) / `10.16x Node`** |
| **04** | `04-edge-api-gateway` (`fast-querystring` + `ms`) | **`6.049 ms`** (`5.45 ms` Min) | **`6.562 ms`** | **`1.439 ms`** | `6.966 ms` (`-13.2%`) | `14.938 ms` | `42.373 ms` | `60.823 ms` | **`2.47x Bun` (`10.38x` Warm) / `10.06x Node`** |
| **05** | `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | **`8.291 ms`** (`7.56 ms` Min) | **`9.108 ms`** | **`4.420 ms`** | `9.480 ms` (`-12.5%`) | `14.175 ms` | `43.076 ms` | `60.689 ms` | **`1.71x Bun` (`3.21x` Warm) / `7.32x Node`** |
| **06** | `06-telemetry-metrics-pipeline` (`bytes` + `flatted` + WAL) | **`10.835 ms`** (`10.09 ms` Min) | **`10.834 ms`** | **`5.000 ms`** | `11.607 ms` (`-6.7%`) | `16.606 ms` | `45.515 ms` | `63.514 ms` | **`1.53x Bun` (`3.32x` Warm) / `5.86x Node`** |
| **07** | `07-graphql-ast-validator` (`dlv` + `dset` + AST Cost) | **`7.342 ms`** (`6.61 ms` Min) | **`7.675 ms`** | **`3.582 ms`** | `8.503 ms` (`-13.7%`) | `12.872 ms` | `42.425 ms` | `59.083 ms` | **`1.75x Bun` (`3.59x` Warm) / `8.05x Node`** |
| **08** | `08-event-driven-state-journal` (`mitt` + `ms` + JSONL Replay) | **`6.189 ms`** (`5.61 ms` Min) | **`6.594 ms`** | **`1.108 ms`** | `10.352 ms` (`-40.2%`) | `15.559 ms` | `44.381 ms` | `62.514 ms` | **`2.51x Bun` (`14.04x` Warm) / `10.10x Node`** |
| **09** | `09-wintertc-web-router` (`Request` + `Response` + `URL`, 80 reqs) | **`12.816 ms`** (`12.07 ms` Min) | **`12.895 ms`** | **`7.035 ms`** | `15.986 ms` (`-19.8%`) | `13.997 ms` | `46.306 ms` | `84.018 ms` | **`1.09x Bun` (`1.99x` Warm) / `6.56x Node`** |
| **10** | `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **`9.423 ms`** (`8.85 ms` Min) | **`10.095 ms`** | **`3.659 ms`** | `13.588 ms` (`-30.7%`) | `14.144 ms` | `48.186 ms` | `83.155 ms` | **`1.50x Bun` (`3.87x` Warm) / `8.82x Node`** |
| **11** | `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **`8.489 ms`** (`8.14 ms` Min) | **`9.171 ms`** | **`2.933 ms`** | `11.268 ms` (`-24.7%`) | `13.153 ms` | `47.860 ms` | `85.345 ms` | **`1.55x Bun` (`4.48x` Warm) / `10.05x Node`** |
| **12** | `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP, 320 frames) | **`13.394 ms`** (`12.28 ms` Min) | **`13.688 ms`** | **`6.645 ms`** | `15.806 ms` (`-15.3%`) | `15.194 ms` | `50.006 ms` | `87.218 ms` | **`1.13x Bun` (`2.29x` Warm) / `6.51x Node`** |
| **13** | `13-webhook-orchestrator-ssrf` (`fast-querystring` + `ms` + Live `fetch`) | **`9.606 ms`** (`9.00 ms` Min) | **`9.949 ms`** | **`3.171 ms`** | `13.490 ms` (`-28.8%`) | `18.784 ms` | `50.686 ms` | `109.354 ms` | **`1.96x Bun` (`5.92x` Warm) / `11.38x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`12.176 ms`** (`11.57 ms` Min) | **`12.796 ms`** | **`7.036 ms`** | `13.296 ms` (`-8.4%`) | `20.583 ms` | `52.495 ms` | `106.385 ms` | **`1.69x Bun` (`2.93x` Warm) / `8.74x Node`** |
| **15** | `15-time-travel-test-suite` (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **`11.454 ms`** (`10.84 ms` Min) | **`11.681 ms`** | **`3.646 ms`** | `14.040 ms` (`-18.4%`) | `24.488 ms` | `66.216 ms` | `81.512 ms` | **`2.14x Bun` (`6.72x` Warm) / `7.12x Node`** |
| **16** | `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **`6.375 ms`** (`5.75 ms` Min) | **`6.708 ms`** | **`1.136 ms`** | `7.113 ms` (`-10.4%`) | `14.778 ms` | `45.783 ms` | `66.435 ms` | **`2.32x Bun` (`13.01x` Warm) / `10.42x Node`** |
| **17** | `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive/307/SSRF/DLP) | **`12.720 ms`** (`11.97 ms` Min) | **`13.137 ms`** | **`5.572 ms`** | `15.165 ms` (`-16.1%`) | `21.465 ms` | `53.988 ms` | `109.449 ms` | **`1.69x Bun` (`3.85x` Warm) / `8.60x Node`** |
| **18** | `18-async-event-loop-fanout-scheduler` (Timer Wheel/`Promise.all`/`AbortSignal`) | **`27.555 ms`** (`25.98 ms` Min) | **`27.931 ms`** | **`17.266 ms`** | `30.025 ms` (`-8.2%`) | `32.239 ms` | `69.723 ms` | `79.312 ms` | **`1.17x Bun` (`1.87x` Warm) / `2.88x Node`** |
| **19** | `19-wintertc-webcrypto-streams-edge` (WebCrypto/Streams/FormData/Clone) | **`8.173 ms`** (`7.59 ms` Min) | **`8.479 ms`** | **`1.482 ms`** | `9.961 ms` (`-18.0%`) | `13.641 ms` | `43.183 ms` | `84.763 ms` | **`1.67x Bun` (`9.20x` Warm) / `10.37x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`8.765 ms`** (`8.20 ms` Min) | **`9.091 ms`** | **`0.406 ms`** | `9.961 ms` (`-12.0%`) | `21.745 ms` | `53.602 ms` | `73.345 ms` | **`2.48x Bun` (`53.56x` Warm) / `8.37x Node`** |
| **21** | `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket` + 5-Ring Guard) | **`8.922 ms`** (`8.29 ms` Min) | **`9.383 ms`** | **`1.993 ms`** | `9.783 ms` (`-8.8%`) | `19.270 ms` | `50.475 ms` | `114.344 ms` | **`2.16x Bun` (`9.67x` Warm) / `12.82x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`19.573 ms`** (`18.50 ms` Min) | **`19.789 ms`** | **`11.420 ms`** | `20.849 ms` (`-6.1%`) | `55.821 ms` | `113.721 ms` | `100.168 ms` | **`2.85x Bun` (`4.89x` Warm) / `5.12x Node`** |
| **23** | `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **`12.765 ms`** (`12.38 ms` Min) | **`13.543 ms`** | **`1.407 ms`** | `13.970 ms` (`-8.6%`) | `21.033 ms` | `40.151 ms` | `59.822 ms` | **`1.65x Bun` (`14.95x` Warm) / `4.69x Node`** |
| **24** | `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` SLA Scorer + SSR Pipeline) | **`6.882 ms`** (`6.45 ms` Min) | **`7.228 ms`** | **`1.331 ms`** | `7.643 ms` (`-10.0%`) | `18.506 ms` | `41.675 ms` | `68.394 ms` | **`2.69x Bun` (`13.90x` Warm) / `9.94x Node`** |
| **FW-01** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`12.648 ms`** (`11.99 ms` Min) | **`13.163 ms`** | **`5.047 ms`** | `14.734 ms` (`-14.2%`) | `19.501 ms` | `50.450 ms` | `92.255 ms` | **`1.54x Bun` (`3.86x` Warm) / `7.29x Node`** |
| **FW-02** | `FW-02: frameworks/02-itty-router-unjs-nitro-stack` (`itty-router` + UnJS) | **`15.193 ms`** (`14.60 ms` Min) | **`15.887 ms`** | **`8.865 ms`** | `16.339 ms` (`-7.0%`) | `18.693 ms` | `46.488 ms` | `87.381 ms` | **`1.23x Bun` (`2.11x` Warm) / `5.75x Node`** |
| **FW-03** | `FW-03: frameworks/03-node-http-express-compat` (`node:http` + `node:stream` Express/Polka) | **`13.516 ms`** (`12.85 ms` Min) | **`13.924 ms`** | **`0.758 ms`** | `16.248 ms` (`-16.8%`) | `34.490 ms` | `49.163 ms` | `80.879 ms` | **`2.55x Bun` (`45.50x` Warm) / `5.98x Node`** |
| **FW-04** | `FW-04: frameworks/04-sveltekit-edge-ssr` (`@sveltejs/kit@3.1.1` + `svelte@5.57.3` + `devalue@6.0.3`) | **`27.950 ms`** (`26.67 ms` Min) | **`27.847 ms`** | **`14.375 ms`** | `31.769 ms` (`-12.0%`) | `42.762 ms` | `66.959 ms` | `108.190 ms` | **`1.54x Bun` (`2.97x` Warm) / `3.89x Node`** |
| **CS-01** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`26.251 ms`** (`25.42 ms` Min) | **`26.733 ms`** | **`12.365 ms`** | `32.434 ms` (`-19.1%`) | `27.452 ms` | `70.894 ms` | `90.197 ms` | **`1.05x Bun` (`2.22x` Warm) / `3.44x Node`** |
| **CS-02** | `CS-02: case_studies/02-mcp-code-mode-orchestrator` (`-98.4%` Tokens) | **`15.304 ms`** (`14.16 ms` Min) | **`15.758 ms`** | **`2.040 ms`** | `23.230 ms` (`-34.1%`) | `16.054 ms` | `43.604 ms` | `65.154 ms` | **`1.05x Bun` (`7.87x` Warm) / `4.26x Node`** |
| **CS-03** | `CS-03: case_studies/03-multi-tenant-saas-plugin-mesh` (12 Tenants + Hot-Swap) | **`11.970 ms`** (`11.37 ms` Min) | **`12.337 ms`** | **`4.655 ms`** | `14.112 ms` (`-15.2%`) | `16.176 ms` | `47.986 ms` | `81.774 ms` | **`1.35x Bun` (`3.47x` Warm) / `6.83x Node`** |
| **CS-04** | `CS-04: case_studies/04-wasm-zero-trust-cdr-pipeline` (Wasm SimHash CDR) | **`13.688 ms`** (`12.90 ms` Min) | **`13.784 ms`** | **`1.904 ms`** | `17.170 ms` (`-20.3%`) | `18.809 ms` | `42.385 ms` | `64.760 ms` | **`1.37x Bun` (`9.88x` Warm) / `4.73x Node`** |
| **CS-05** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`8.524 ms`** (`7.95 ms` Min) | **`8.892 ms`** | **`2.519 ms`** | `16.488 ms` (`-48.3%`) | `18.971 ms` | `61.540 ms` | `80.990 ms` | **`2.23x Bun` (`7.53x` Warm) / `9.50x Node`** |

---

## 4. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Workload | Slice (`SLICEBC\x01` Default) | Bun (`v1.4.4`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha.0.0.0`) | Slice Memory Advantage |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **App 2: NPM semver+ms+ini** | **13,312 KB (13.00 MB)** | 30,204 KB (29.50 MB) | 46,944 KB (45.84 MB) | 71,580 KB (69.90 MB) | **5.38x less than Node / 2.27x less than Bun** |
| **App 6: Telemetry + Flatted** | **13,952 KB (13.63 MB)** | 30,208 KB (29.50 MB) | 50,796 KB (49.61 MB) | 72,988 KB (71.28 MB) | **5.23x less than Node / 2.17x less than Bun** |
| **App 21: RFC 6455 WS + Sec** | **14,080 KB (13.75 MB)** | 31,996 KB (31.25 MB) | 50,684 KB (49.50 MB) | 94,848 KB (92.63 MB) | **6.74x less than Node / 2.27x less than Bun** |
| **App 22: Workers + Sockets** | **20,096 KB (19.63 MB)** | 41,300 KB (40.33 MB) | 63,708 KB (62.21 MB) | 92,752 KB (90.58 MB) | **4.62x less than Node / 2.06x less than Bun** |
| **App 23: Pure-Rust WASM** | **17,792 KB (17.38 MB)** | 30,848 KB (30.13 MB) | 48,540 KB (47.40 MB) | 80,784 KB (78.89 MB) | **4.54x less than Node / 1.73x less than Bun** |
| **FW 1: Hono v4 Microservice** | **15,360 KB (15.00 MB)** | 31,136 KB (30.41 MB) | 49,392 KB (48.23 MB) | 88,740 KB (86.66 MB) | **5.78x less than Node / 2.03x less than Bun** |
| **FW 2: itty-router + UnJS** | **14,336 KB (14.00 MB)** | 31,872 KB (31.13 MB) | 48,884 KB (47.74 MB) | 87,308 KB (85.26 MB) | **6.09x less than Node / 2.22x less than Bun** |
| **App 24: WASM + SSR** | **14,208 KB (13.88 MB)** | 30,336 KB (29.63 MB) | 48,600 KB (47.46 MB) | 80,896 KB (79.00 MB) | **5.69x less than Node / 2.14x less than Bun** |
| **FW 3: Express node:http** | **14,592 KB (14.25 MB)** | 33,152 KB (32.38 MB) | 47,200 KB (46.09 MB) | 80,268 KB (78.39 MB) | **5.50x less than Node / 2.27x less than Bun** |
| **01-tot-autonomous-sre-agent** | **19,456 KB (19.00 MB)** | 31,720 KB (30.98 MB) | 53,100 KB (51.86 MB) | 83,592 KB (81.63 MB) | **4.30x less than Node / 1.63x less than Bun** |
| **02-mcp-code-mode-orchestrator** | **16,708 KB (16.32 MB)** | 27,776 KB (27.13 MB) | 46,800 KB (45.70 MB) | 80,904 KB (79.01 MB) | **4.84x less than Node / 1.66x less than Bun** |
| **03-multi-tenant-saas-plugin-mesh** | **14,464 KB (14.13 MB)** | 26,368 KB (25.75 MB) | 49,664 KB (48.50 MB) | 88,200 KB (86.13 MB) | **6.10x less than Node / 1.82x less than Bun** |
| **04-wasm-zero-trust-cdr-pipeline** | **18,176 KB (17.75 MB)** | 31,232 KB (30.50 MB) | 49,360 KB (48.20 MB) | 80,516 KB (78.63 MB) | **4.43x less than Node / 1.72x less than Bun** |
| **05-time-travel-chaos-fuzzer-payments** | **13,440 KB (13.13 MB)** | 29,056 KB (28.38 MB) | 49,792 KB (48.63 MB) | 81,696 KB (79.78 MB) | **6.08x less than Node / 2.16x less than Bun** |
| **FW 4: SvelteKit 3 + Svelte 5** | **21,760 KB (21.25 MB)** | 41,728 KB (40.75 MB) | 56,652 KB (55.32 MB) | 85,796 KB (83.79 MB) | **3.94x less than Node / 1.92x less than Bun** |

---

## 5. Multi-Tenant Edge KV, AI Agent Checkpoints & 5-Ring Security Telemetry

| Subsystem / Benchmark | Measured Telemetry | Status |
| :--- | :--- | :---: |
| **Sharded `Slice.kv` (`1,001` Get/Set/Incr Ops in Isolate)** | **`511.38 us`** total (`--warm`, `93 B` memory cage) | `PASS` |
| **Multi-Tenant `--tenant-dir` Zero-Downtime Hot-Swap** | Live atomic replacement of `tenant-alpha.cas` (`v1 -> v2-hot-swapped`) with isolated `Slice.kv` state | `PASS` |
| **Deterministic Time-Travel Replay (`--deterministic=20261019`)** | 100% bit-for-bit identical PRNG (`Math.random`) & virtual clock (`Date.now`, `performance.now`) | `PASS` |
| **Tree-of-Thought AI Agent (`Slice.checkpoint` + 3 `evalSandbox` + `restore`)** | **`47.55 us`** total (`4.00 us` child sandbox) | `PASS` |
| **Cloud Metadata SSRF (`169.254.169.254`) & Base64 Egress DLP Guard** | `SSRF_METADATA_BLOCKED` & `EGRESS_SECRET_LEAK_BLOCKED` deterministically trapped | `PASS` |
| **Built-In Model Context Protocol Server (`slice mcp --smoke-test`)** | JSON-RPC 2.0 `initialize`, `tools/list`, `tools/call` verified over `stdio` | `PASS` |
