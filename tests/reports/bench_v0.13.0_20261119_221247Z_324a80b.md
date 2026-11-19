# Slice (Nexus-JS) - Full 33-Workload & MicroVM Benchmark Report

* **Slice Version:** `v0.13.0` (Commit `324a80b`)
* **Generated Timestamp (UTC):** `2026-11-19T22:12:47Z`
* **Archive File:** [`tests/reports/bench_v0.13.0_20261119_221247Z_324a80b.md`](./bench_v0.13.0_20261119_221247Z_324a80b.md)
* **Canonical Latest Report:** [`tests/reports/latest.md`](./latest.md)
* **Environment:** `Linux x86_64 Docker (io_uring SQPOLL + WRPKRU)`
* **Compared Runtimes:**
  * **Slice:** `v0.13.0` (Default `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS + Monomorphic Inline Caching `get_fast` + Iterative ECMA-404 `JSON.parse`)
  * **Bun:** `v1.4.4` (JavaScriptCore)
  * **Deno:** `v2.9.8` (V8)
  * **Node.js:** `v27.0.0-alpha.0.0.0` (V8 native TypeScript execution)

---

## 1. Executive Summary

Across all **33 real-world multi-file TypeScript applications, framework suites, and case studies**, Slice beats **Deno (`v2.9.8`)** on **33/33** workloads, beats **Node.js (`v27.0.0-alpha.0.0.0`)** on **33/33** workloads, beats **Bun (`v1.4.4`)** on **27/33** workloads in cold one-shot execution, and beats **all 3 runtimes on 33/33 workloads in `--warm` pooled isolate execution**.

| Metric | Linux (`x86_64` Docker) | Windows (`x86_64` Host) | Comparison vs Runtimes & MicroVMs |
| :--- | :---: | :---: | :--- |
| **Single Isolate Cold-Start (Min / P50 / P99)** | **`0.89 us` / `0.94 us` / `1.14 us`** | **`1.30 us` / `1.40 us` / `2.70 us`** | **~120,000x faster than 125ms Firecracker VM** |
| **1,000-Tenant MicroVM Swarm (Boot + Exec)** | **`1.12 ms`** (`7.75 MB` RSS) | **`1.74 ms`** (`6.29 MB` RSS) | **> 10,000x faster than container/VM swarms** |
| **500-Module Monorepo Cold Boot (`0 stat()`)** | **`4.522 ms`** (`4163.56 us` in-mem) | **`19.752 ms`** (`10507.10 us` in-mem) | **15.6x faster than Node.js (`1,500+ stat()`)** |
| **10,000 Direct Kernel Ring I/O Completions** | **`0.01 ms`** | **`3.89 ms`** | **Zero worker-thread lock contention** |
| **5-Ring Security Containment Verification** | **`5 / 5 PASS`** | **`5 / 5 PASS`** | **OCap + SES + Fuel + SSRF/DLP + MPK** |

---

## 2. `slice serve` Multi-Tenant HTTP/1.1 Keep-Alive Throughput (`16,000` Reqs, 8 Sockets x 8 Pipeline)

| Server / Runtime | Throughput (`req/s`) | Avg Latency / Req | Total Time (`16k` Reqs) | Relative Speedup vs Runtimes |
| :--- | :---: | :---: | :---: | :--- |
| **`slice serve` (Tier-0 CAS, `--workers=8`)** | **`176,831 req/s`** | **`5.66 us/req`** | **`90.5 ms`** | **`1.60x` faster than `Bun.serve` / `2.80x` Node.js** |
| **`slice serve` (Tier-1 WinterTC, `--workers=8`)** | **`166,496 req/s`** | **`6.01 us/req`** | **`96.1 ms`** | **`1.51x` faster than `Bun.serve` / `2.64x` Node.js** |
| **`Deno.serve` (`v2.9.8`, V8, No Per-Req Sandbox)** | `116,123 req/s` | `8.61 us/req` | `137.8 ms` | `1.05x` vs Bun |
| **`Bun.serve` (`v1.4.4`, JSC, No Per-Req Sandbox)** | `110,295 req/s` | `9.07 us/req` | `145.1 ms` | `1.00x` (Baseline) |
| **`Node.js http.createServer` (`v27.0.0-alpha.0.0.0`, V8)** | `63,079 req/s` | `15.85 us/req` | `253.6 ms` | `0.57x` vs Bun |

---

## 3. Full 33-Workload Latency Comparison (20 Runs Each, Linux `x86_64` Docker)

| # | Workload (`tests/apps/`, `tests/frameworks/`, `tests/case_studies/`) | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm (`--warm`) | Previous `.cas` (`Delta`) | Bun `v1.4.4` | Deno `v2.9.8` | Node.js `v27.0.0-alpha.0.0.0` | Slice Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`5.837 ms`** (`5.17 ms` Min) | **`6.174 ms`** | **`1.008 ms`** | `6.594 ms` (`-11.5%`) | `12.272 ms` | `31.478 ms` | `56.145 ms` | **`2.10x Bun` (`12.17x` Warm) / `9.62x Node`** |
| **02** | `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | **`12.388 ms`** (`11.67 ms` Min) | **`12.930 ms`** | **`3.884 ms`** | `14.991 ms` (`-17.4%`) | `19.628 ms` | `48.256 ms` | `64.151 ms` | **`1.58x Bun` (`5.05x` Warm) / `5.18x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`6.744 ms`** (`6.37 ms` Min) | **`7.148 ms`** | **`1.103 ms`** | `7.733 ms` (`-12.8%`) | `12.764 ms` | `38.532 ms` | `56.307 ms` | **`1.89x Bun` (`11.57x` Warm) / `8.35x Node`** |
| **04** | `04-edge-api-gateway` (`fast-querystring` + `ms`) | **`6.966 ms`** (`6.43 ms` Min) | **`7.236 ms`** | **`1.515 ms`** | `7.752 ms` (`-10.1%`) | `13.860 ms` | `41.132 ms` | `57.500 ms` | **`1.99x Bun` (`9.15x` Warm) / `8.25x Node`** |
| **05** | `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | **`9.480 ms`** (`8.62 ms` Min) | **`9.689 ms`** | **`4.597 ms`** | `10.409 ms` (`-8.9%`) | `13.075 ms` | `40.751 ms` | `609.232 ms` | **`1.38x Bun` (`2.84x` Warm) / `64.26x Node`** |
| **06** | `06-telemetry-metrics-pipeline` (`bytes` + `flatted` + WAL) | **`11.607 ms`** (`11.23 ms` Min) | **`12.134 ms`** | **`4.981 ms`** | `13.504 ms` (`-14.0%`) | `15.486 ms` | `43.154 ms` | `58.673 ms` | **`1.33x Bun` (`3.11x` Warm) / `5.05x Node`** |
| **07** | `07-graphql-ast-validator` (`dlv` + `dset` + AST Cost) | **`8.503 ms`** (`7.65 ms` Min) | **`10.386 ms`** | **`3.349 ms`** | `9.163 ms` (`-7.2%`) | `12.021 ms` | `37.140 ms` | `56.525 ms` | **`1.41x Bun` (`3.59x` Warm) / `6.65x Node`** |
| **08** | `08-event-driven-state-journal` (`mitt` + `ms` + JSONL Replay) | **`10.352 ms`** (`9.82 ms` Min) | **`10.990 ms`** | **`4.340 ms`** | `11.989 ms` (`-13.7%`) | `14.533 ms` | `41.368 ms` | `58.563 ms` | **`1.40x Bun` (`3.35x` Warm) / `5.66x Node`** |
| **09** | `09-wintertc-web-router` (`Request` + `Response` + `URL`, 80 reqs) | **`15.986 ms`** (`15.48 ms` Min) | **`16.434 ms`** | **`9.472 ms`** | `17.471 ms` (`-8.5%`) | `12.602 ms` | `45.774 ms` | `77.889 ms` | **`1.33x Bun Warm` / `4.87x Node`** |
| **10** | `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **`13.588 ms`** (`12.59 ms` Min) | **`14.052 ms`** | **`6.760 ms`** | `14.874 ms` (`-8.6%`) | `13.001 ms` | `43.770 ms` | `76.748 ms` | **`1.92x Bun Warm` / `5.65x Node`** |
| **11** | `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **`11.268 ms`** (`10.36 ms` Min) | **`11.759 ms`** | **`5.418 ms`** | `12.207 ms` (`-7.7%`) | `12.058 ms` | `44.467 ms` | `81.174 ms` | **`1.07x Bun` (`2.23x` Warm) / `7.20x Node`** |
| **12** | `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP, 320 frames) | **`15.806 ms`** (`15.24 ms` Min) | **`16.593 ms`** | **`10.417 ms`** | `17.761 ms` (`-11.0%`) | `13.596 ms` | `45.679 ms` | `81.957 ms` | **`1.31x Bun Warm` / `5.19x Node`** |
| **13** | `13-webhook-orchestrator-ssrf` (`fast-querystring` + `ms` + Live `fetch`) | **`13.490 ms`** (`12.88 ms` Min) | **`13.662 ms`** | **`6.074 ms`** | `14.474 ms` (`-6.8%`) | `17.422 ms` | `47.491 ms` | `105.039 ms` | **`1.29x Bun` (`2.87x` Warm) / `7.79x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`13.296 ms`** (`12.85 ms` Min) | **`13.955 ms`** | **`6.916 ms`** | `15.125 ms` (`-12.1%`) | `19.395 ms` | `48.973 ms` | `98.417 ms` | **`1.46x Bun` (`2.80x` Warm) / `7.40x Node`** |
| **15** | `15-time-travel-test-suite` (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **`14.040 ms`** (`13.10 ms` Min) | **`14.205 ms`** | **`4.710 ms`** | `19.029 ms` (`-26.2%`) | `22.320 ms` | `63.765 ms` | `76.775 ms` | **`1.59x Bun` (`4.74x` Warm) / `5.47x Node`** |
| **16** | `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **`7.113 ms`** (`6.61 ms` Min) | **`7.529 ms`** | **`1.213 ms`** | `8.043 ms` (`-11.6%`) | `13.667 ms` | `41.780 ms` | `59.340 ms` | **`1.92x Bun` (`11.27x` Warm) / `8.34x Node`** |
| **17** | `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive/307/SSRF/DLP) | **`15.165 ms`** (`14.49 ms` Min) | **`15.319 ms`** | **`7.818 ms`** | `19.610 ms` (`-22.7%`) | `19.978 ms` | `52.315 ms` | `103.188 ms` | **`1.32x Bun` (`2.56x` Warm) / `6.80x Node`** |
| **18** | `18-async-event-loop-fanout-scheduler` (Timer Wheel/`Promise.all`/`AbortSignal`) | **`30.025 ms`** (`27.98 ms` Min) | **`30.255 ms`** | **`17.210 ms`** | `32.217 ms` (`-6.8%`) | `31.233 ms` | `67.630 ms` | `75.348 ms` | **`1.04x Bun` (`1.81x` Warm) / `2.51x Node`** |
| **19** | `19-wintertc-webcrypto-streams-edge` (WebCrypto/Streams/FormData/Clone) | **`9.961 ms`** (`9.28 ms` Min) | **`10.114 ms`** | **`2.138 ms`** | `9.839 ms` (`1.2%`) | `12.635 ms` | `42.057 ms` | `79.944 ms` | **`1.27x Bun` (`5.91x` Warm) / `8.03x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`9.961 ms`** (`9.01 ms` Min) | **`10.314 ms`** | **`0.483 ms`** | `11.069 ms` (`-10.0%`) | `19.942 ms` | `48.817 ms` | `65.166 ms` | **`2.00x Bun` (`41.29x` Warm) / `6.54x Node`** |
| **21** | `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket` + 5-Ring Guard) | **`9.783 ms`** (`9.35 ms` Min) | **`10.174 ms`** | **`1.984 ms`** | `11.816 ms` (`-17.2%`) | `18.400 ms` | `48.164 ms` | `105.599 ms` | **`1.88x Bun` (`9.27x` Warm) / `10.79x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`20.849 ms`** (`20.24 ms` Min) | **`21.677 ms`** | **`3.822 ms`** | `23.022 ms` (`-9.4%`) | `50.941 ms` | `101.331 ms` | `94.069 ms` | **`2.44x Bun` (`13.33x` Warm) / `4.51x Node`** |
| **23** | `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **`13.970 ms`** (`13.27 ms` Min) | **`14.657 ms`** | **`0.866 ms`** | `16.806 ms` (`-16.9%`) | `19.090 ms` | `38.106 ms` | `56.880 ms` | **`1.37x Bun` (`22.04x` Warm) / `4.07x Node`** |
| **24** | `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` SLA Scorer + SSR Pipeline) | **`7.643 ms`** (`7.28 ms` Min) | **`8.076 ms`** | **`1.339 ms`** | `9.168 ms` (`-16.6%`) | `16.843 ms` | `39.509 ms` | `60.041 ms` | **`2.20x Bun` (`12.58x` Warm) / `7.86x Node`** |
| **FW-01** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`14.734 ms`** (`14.05 ms` Min) | **`17.714 ms`** | **`5.762 ms`** | `16.505 ms` (`-10.7%`) | `17.813 ms` | `600.375 ms` | `85.789 ms` | **`1.21x Bun` (`3.09x` Warm) / `5.82x Node`** |
| **FW-02** | `FW-02: frameworks/02-itty-router-unjs-nitro-stack` (`itty-router` + UnJS) | **`16.339 ms`** (`15.73 ms` Min) | **`16.685 ms`** | **`8.242 ms`** | `22.222 ms` (`-26.5%`) | `17.121 ms` | `41.702 ms` | `80.966 ms` | **`1.05x Bun` (`2.08x` Warm) / `4.96x Node`** |
| **FW-03** | `FW-03: frameworks/03-node-http-express-compat` (`node:http` + `node:stream` Express/Polka) | **`16.248 ms`** (`15.20 ms` Min) | **`16.386 ms`** | **`1.013 ms`** | `17.430 ms` (`-6.8%`) | `32.277 ms` | `46.781 ms` | `76.700 ms` | **`1.99x Bun` (`31.86x` Warm) / `4.72x Node`** |
| **FW-04** | `FW-04: frameworks/04-sveltekit-edge-ssr` (`@sveltejs/kit@3.1.1` + `svelte@5.57.3` + `devalue@6.0.3`) | **`31.769 ms`** (`30.71 ms` Min) | **`31.973 ms`** | **`14.095 ms`** | `36.096 ms` (`-12.0%`) | `40.573 ms` | `63.187 ms` | `101.244 ms` | **`1.28x Bun` (`2.88x` Warm) / `3.19x Node`** |
| **CS-01** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`32.434 ms`** (`31.63 ms` Min) | **`33.689 ms`** | **`11.529 ms`** | `37.112 ms` (`-12.6%`) | `27.878 ms` | `69.403 ms` | `89.922 ms` | **`2.42x Bun Warm` / `2.77x Node`** |
| **CS-02** | `CS-02: case_studies/02-mcp-code-mode-orchestrator` (`-98.4%` Tokens) | **`23.230 ms`** (`22.13 ms` Min) | **`23.994 ms`** | **`3.301 ms`** | `20.381 ms` (`14.0%`) | `16.095 ms` | `42.548 ms` | `64.883 ms` | **`4.88x Bun Warm` / `2.79x Node`** |
| **CS-03** | `CS-03: case_studies/03-multi-tenant-saas-plugin-mesh` (12 Tenants + Hot-Swap) | **`14.112 ms`** (`13.71 ms` Min) | **`14.755 ms`** | **`5.395 ms`** | `14.811 ms` (`-4.7%`) | `16.264 ms` | `43.717 ms` | `77.790 ms` | **`1.15x Bun` (`3.01x` Warm) / `5.51x Node`** |
| **CS-04** | `CS-04: case_studies/04-wasm-zero-trust-cdr-pipeline` (Wasm SimHash CDR) | **`17.170 ms`** (`16.63 ms` Min) | **`18.091 ms`** | **`1.235 ms`** | `19.402 ms` (`-11.5%`) | `17.165 ms` | `40.002 ms` | `60.005 ms` | **`13.90x Bun Warm` / `3.49x Node`** |
| **CS-05** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`16.488 ms`** (`15.63 ms` Min) | **`16.722 ms`** | **`7.471 ms`** | `19.390 ms` (`-15.0%`) | `17.843 ms` | `56.348 ms` | `76.409 ms` | **`1.08x Bun` (`2.39x` Warm) / `4.63x Node`** |

---

## 4. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Workload | Slice (`SLICEBC\x01` Default) | Bun (`v1.4.4`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha.0.0.0`) | Slice Memory Advantage |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **App 2: NPM semver+ms+ini** | **13,880 KB (13.55 MB)** | 29,696 KB (29.00 MB) | 47,348 KB (46.24 MB) | 71,836 KB (70.15 MB) | **5.18x less than Node / 2.14x less than Bun** |
| **App 6: Telemetry + Flatted** | **14,740 KB (14.39 MB)** | 30,080 KB (29.38 MB) | 50,416 KB (49.23 MB) | 72,992 KB (71.28 MB) | **4.95x less than Node / 2.04x less than Bun** |
| **App 21: RFC 6455 WS + Sec** | **14,976 KB (14.63 MB)** | 31,968 KB (31.22 MB) | 50,852 KB (49.66 MB) | 95,620 KB (93.38 MB) | **6.38x less than Node / 2.13x less than Bun** |
| **App 22: Workers + Sockets** | **24,576 KB (24.00 MB)** | 41,304 KB (40.34 MB) | 63,784 KB (62.29 MB) | 93,524 KB (91.33 MB) | **3.81x less than Node / 1.68x less than Bun** |
| **App 23: Pure-Rust WASM** | **18,944 KB (18.50 MB)** | 30,976 KB (30.25 MB) | 49,036 KB (47.89 MB) | 80,660 KB (78.77 MB) | **4.26x less than Node / 1.64x less than Bun** |
| **FW 1: Hono v4 Microservice** | **16,384 KB (16.00 MB)** | 29,312 KB (28.63 MB) | 48,984 KB (47.84 MB) | 89,248 KB (87.16 MB) | **5.45x less than Node / 1.79x less than Bun** |
| **FW 2: itty-router + UnJS** | **15,232 KB (14.88 MB)** | 31,872 KB (31.13 MB) | 48,756 KB (47.61 MB) | 87,312 KB (85.27 MB) | **5.73x less than Node / 2.09x less than Bun** |
| **App 24: WASM + SSR** | **14,336 KB (14.00 MB)** | 32,256 KB (31.50 MB) | 48,476 KB (47.34 MB) | 81,148 KB (79.25 MB) | **5.66x less than Node / 2.25x less than Bun** |
| **FW 3: Express node:http** | **16,128 KB (15.75 MB)** | 33,152 KB (32.38 MB) | 47,204 KB (46.10 MB) | 80,524 KB (78.64 MB) | **4.99x less than Node / 2.06x less than Bun** |
| **01-tot-autonomous-sre-agent** | **21,760 KB (21.25 MB)** | 31,616 KB (30.88 MB) | 53,024 KB (51.78 MB) | 83,048 KB (81.10 MB) | **3.82x less than Node / 1.45x less than Bun** |
| **02-mcp-code-mode-orchestrator** | **19,388 KB (18.93 MB)** | 28,032 KB (27.38 MB) | 46,528 KB (45.44 MB) | 80,388 KB (78.50 MB) | **4.15x less than Node / 1.45x less than Bun** |
| **03-multi-tenant-saas-plugin-mesh** | **15,616 KB (15.25 MB)** | 26,880 KB (26.25 MB) | 49,780 KB (48.61 MB) | 87,680 KB (85.63 MB) | **5.61x less than Node / 1.72x less than Bun** |
| **04-wasm-zero-trust-cdr-pipeline** | **20,480 KB (20.00 MB)** | 31,360 KB (30.63 MB) | 49,936 KB (48.77 MB) | 80,264 KB (78.38 MB) | **3.92x less than Node / 1.53x less than Bun** |
| **05-time-travel-chaos-fuzzer-payments** | **16,512 KB (16.13 MB)** | 29,184 KB (28.50 MB) | 49,144 KB (47.99 MB) | 81,824 KB (79.91 MB) | **4.96x less than Node / 1.77x less than Bun** |
| **FW 4: SvelteKit 3 + Svelte 5** | **23,552 KB (23.00 MB)** | 42,240 KB (41.25 MB) | 56,780 KB (55.45 MB) | 85,668 KB (83.66 MB) | **3.64x less than Node / 1.79x less than Bun** |

---

## 5. Multi-Tenant Edge KV, AI Agent Checkpoints & 5-Ring Security Telemetry

| Subsystem / Benchmark | Measured Telemetry | Status |
| :--- | :--- | :---: |
| **Sharded `Slice.kv` (`1,001` Get/Set/Incr Ops in Isolate)** | **`575.37 us`** total (`--warm`, `93 B` memory cage) | `PASS` |
| **Multi-Tenant `--tenant-dir` Zero-Downtime Hot-Swap** | Live atomic replacement of `tenant-alpha.cas` (`v1 -> v2-hot-swapped`) with isolated `Slice.kv` state | `PASS` |
| **Deterministic Time-Travel Replay (`--deterministic=20261019`)** | 100% bit-for-bit identical PRNG (`Math.random`) & virtual clock (`Date.now`, `performance.now`) | `PASS` |
| **Tree-of-Thought AI Agent (`Slice.checkpoint` + 3 `evalSandbox` + `restore`)** | **`46.85 us`** total (`1.59 us` child sandbox) | `PASS` |
| **Cloud Metadata SSRF (`169.254.169.254`) & Base64 Egress DLP Guard** | `SSRF_METADATA_BLOCKED` & `EGRESS_SECRET_LEAK_BLOCKED` deterministically trapped | `PASS` |
| **Built-In Model Context Protocol Server (`slice mcp --smoke-test`)** | JSON-RPC 2.0 `initialize`, `tools/list`, `tools/call` verified over `stdio` | `PASS` |
