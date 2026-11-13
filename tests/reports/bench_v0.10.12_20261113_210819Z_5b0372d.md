# Slice (Nexus-JS) - Full 33-Workload & MicroVM Benchmark Report

* **Slice Version:** `v0.10.12` (Commit `5b0372d`)
* **Generated Timestamp (UTC):** `2026-11-13T21:08:19Z`
* **Archive File:** [`tests/reports/bench_v0.10.12_20261113_210819Z_5b0372d.md`](./bench_v0.10.12_20261113_210819Z_5b0372d.md)
* **Canonical Latest Report:** [`tests/reports/latest.md`](./latest.md)
* **Environment:** `Linux x86_64 Docker (io_uring SQPOLL + WRPKRU)`
* **Compared Runtimes:**
  * **Slice:** `v0.10.12` (Default `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS + Monomorphic Inline Caching `get_fast` + Iterative ECMA-404 `JSON.parse`)
  * **Bun:** `v1.4.4` (JavaScriptCore)
  * **Deno:** `v2.9.8` (V8)
  * **Node.js:** `v27.0.0-alpha.0.0.0` (V8 native TypeScript execution)

---

## 1. Executive Summary

Across all **33 real-world multi-file TypeScript applications, framework suites, and case studies**, Slice beats **Deno (`v2.9.8`)** on **33/33** workloads, beats **Node.js (`v27.0.0-alpha.0.0.0`)** on **33/33** workloads, beats **Bun (`v1.4.4`)** on **28/33** workloads in cold one-shot execution, and beats **all 3 runtimes on 33/33 workloads in `--warm` pooled isolate execution**.

| Metric | Linux (`x86_64` Docker) | Windows (`x86_64` Host) | Comparison vs Runtimes & MicroVMs |
| :--- | :---: | :---: | :--- |
| **Single Isolate Cold-Start (Min / P50 / P99)** | **`1.00 us` / `1.30 us` / `1.69 us`** | **`1.30 us` / `1.50 us` / `1.70 us`** | **~120,000x faster than 125ms Firecracker VM** |
| **1,000-Tenant MicroVM Swarm (Boot + Exec)** | **`1.56 ms`** (`7.75 MB` RSS) | **`1.90 ms`** (`6.23 MB` RSS) | **> 10,000x faster than container/VM swarms** |
| **500-Module Monorepo Cold Boot (`0 stat()`)** | **`2.625 ms`** (`8.52 us` in-mem) | **`5.992 ms`** (`12.20 us` in-mem) | **26.8x faster than Node.js (`1,500+ stat()`)** |
| **10,000 Direct Kernel Ring I/O Completions** | **`0.01 ms`** | **`3.05 ms`** | **Zero worker-thread lock contention** |
| **5-Ring Security Containment Verification** | **`5 / 5 PASS`** | **`5 / 5 PASS`** | **OCap + SES + Fuel + SSRF/DLP + MPK** |

---

## 2. `slice serve` Multi-Tenant HTTP/1.1 Keep-Alive Throughput (`16,000` Reqs, 8 Sockets x 8 Pipeline)

| Server / Runtime | Throughput (`req/s`) | Avg Latency / Req | Total Time (`16k` Reqs) | Relative Speedup vs Runtimes |
| :--- | :---: | :---: | :---: | :--- |
| **`slice serve` (Tier-0 CAS, `--workers=8`)** | **`157,690 req/s`** | **`6.34 us/req`** | **`101.5 ms`** | **`1.66x` faster than `Bun.serve` / `2.73x` Node.js** |
| **`slice serve` (Tier-1 WinterTC, `--workers=8`)** | **`147,426 req/s`** | **`6.78 us/req`** | **`108.5 ms`** | **`1.55x` faster than `Bun.serve` / `2.55x` Node.js** |
| **`Deno.serve` (`v2.9.8`, V8, No Per-Req Sandbox)** | `104,430 req/s` | `9.58 us/req` | `153.2 ms` | `1.10x` vs Bun |
| **`Bun.serve` (`v1.4.4`, JSC, No Per-Req Sandbox)** | `95,067 req/s` | `10.52 us/req` | `168.3 ms` | `1.00x` (Baseline) |
| **`Node.js http.createServer` (`v27.0.0-alpha.0.0.0`, V8)** | `57,770 req/s` | `17.31 us/req` | `277.0 ms` | `0.61x` vs Bun |

---

## 3. Full 33-Workload Latency Comparison (20 Runs Each, Linux `x86_64` Docker)

| # | Workload (`tests/apps/`, `tests/frameworks/`, `tests/case_studies/`) | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm (`--warm`) | Previous `.cas` (`Delta`) | Bun `v1.4.4` | Deno `v2.9.8` | Node.js `v27.0.0-alpha.0.0.0` | Slice Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`6.478 ms`** (`6.03 ms` Min) | **`7.158 ms`** | **`1.191 ms`** | `6.265 ms` (`3.4%`) | `13.398 ms` | `37.329 ms` | `61.973 ms` | **`2.07x Bun` (`11.25x` Warm) / `9.57x Node`** |
| **02** | `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | **`14.197 ms`** (`13.08 ms` Min) | **`14.456 ms`** | **`4.023 ms`** | `13.172 ms` (`7.8%`) | `21.481 ms` | `53.345 ms` | `70.165 ms` | **`1.51x Bun` (`5.34x` Warm) / `4.94x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`7.513 ms`** (`6.92 ms` Min) | **`8.499 ms`** | **`1.641 ms`** | `7.480 ms` (`0.4%`) | `14.088 ms` | `44.362 ms` | `63.883 ms` | **`1.88x Bun` (`8.59x` Warm) / `8.50x Node`** |
| **04** | `04-edge-api-gateway` (`fast-querystring` + `ms`) | **`7.700 ms`** (`7.17 ms` Min) | **`8.233 ms`** | **`1.744 ms`** | `7.545 ms` (`2.1%`) | `16.438 ms` | `44.531 ms` | `64.907 ms` | **`2.13x Bun` (`9.43x` Warm) / `8.43x Node`** |
| **05** | `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | **`10.297 ms`** (`9.67 ms` Min) | **`11.044 ms`** | **`4.925 ms`** | `10.676 ms` (`-3.6%`) | `14.663 ms` | `45.557 ms` | `63.061 ms` | **`1.42x Bun` (`2.98x` Warm) / `6.12x Node`** |
| **06** | `06-telemetry-metrics-pipeline` (`bytes` + `flatted` + WAL) | **`12.948 ms`** (`12.17 ms` Min) | **`13.570 ms`** | **`5.581 ms`** | `12.734 ms` (`1.7%`) | `17.501 ms` | `47.943 ms` | `67.545 ms` | **`1.35x Bun` (`3.14x` Warm) / `5.22x Node`** |
| **07** | `07-graphql-ast-validator` (`dlv` + `dset` + AST Cost) | **`9.174 ms`** (`8.38 ms` Min) | **`9.712 ms`** | **`4.005 ms`** | `9.513 ms` (`-3.6%`) | `13.058 ms` | `41.001 ms` | `60.840 ms` | **`1.42x Bun` (`3.26x` Warm) / `6.63x Node`** |
| **08** | `08-event-driven-state-journal` (`mitt` + `ms` + JSONL Replay) | **`11.459 ms`** (`10.71 ms` Min) | **`12.158 ms`** | **`4.893 ms`** | `11.202 ms` (`2.3%`) | `16.037 ms` | `46.164 ms` | `63.867 ms` | **`1.40x Bun` (`3.28x` Warm) / `5.57x Node`** |
| **09** | `09-wintertc-web-router` (`Request` + `Response` + `URL`, 80 reqs) | **`17.704 ms`** (`16.69 ms` Min) | **`18.005 ms`** | **`10.443 ms`** | `17.056 ms` (`3.8%`) | `13.937 ms` | `51.906 ms` | `85.996 ms` | **`1.33x Bun Warm` / `4.86x Node`** |
| **10** | `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **`14.734 ms`** (`14.04 ms` Min) | **`15.248 ms`** | **`7.656 ms`** | `14.664 ms` (`0.5%`) | `14.863 ms` | `49.920 ms` | `85.091 ms` | **`1.01x Bun` (`1.94x` Warm) / `5.78x Node`** |
| **11** | `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **`11.798 ms`** (`11.15 ms` Min) | **`12.429 ms`** | **`5.882 ms`** | `11.775 ms` (`0.2%`) | `13.773 ms` | `53.545 ms` | `89.018 ms` | **`1.17x Bun` (`2.34x` Warm) / `7.55x Node`** |
| **12** | `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP, 320 frames) | **`16.951 ms`** (`15.97 ms` Min) | **`16.961 ms`** | **`9.493 ms`** | `17.129 ms` (`-1.0%`) | `15.561 ms` | `50.617 ms` | `90.480 ms` | **`1.64x Bun Warm` / `5.34x Node`** |
| **13** | `13-webhook-orchestrator-ssrf` (`fast-querystring` + `ms` + Live `fetch`) | **`15.810 ms`** (`15.18 ms` Min) | **`16.891 ms`** | **`6.472 ms`** | `16.514 ms` (`-4.3%`) | `19.735 ms` | `53.880 ms` | `112.550 ms` | **`1.25x Bun` (`3.05x` Warm) / `7.12x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`16.602 ms`** (`15.61 ms` Min) | **`16.764 ms`** | **`7.367 ms`** | `16.525 ms` (`0.5%`) | `21.864 ms` | `53.695 ms` | `109.217 ms` | **`1.32x Bun` (`2.97x` Warm) / `6.58x Node`** |
| **15** | `15-time-travel-test-suite` (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **`15.196 ms`** (`14.40 ms` Min) | **`16.018 ms`** | **`5.062 ms`** | `14.839 ms` (`2.4%`) | `27.322 ms` | `68.752 ms` | `84.303 ms` | **`1.80x Bun` (`5.40x` Warm) / `5.55x Node`** |
| **16** | `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **`7.900 ms`** (`7.05 ms` Min) | **`8.597 ms`** | **`1.329 ms`** | `7.555 ms` (`4.6%`) | `14.645 ms` | `45.738 ms` | `66.435 ms` | **`1.85x Bun` (`11.02x` Warm) / `8.41x Node`** |
| **17** | `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive/307/SSRF/DLP) | **`18.359 ms`** (`17.35 ms` Min) | **`19.235 ms`** | **`8.509 ms`** | `18.096 ms` (`1.5%`) | `22.656 ms` | `58.167 ms` | `112.144 ms` | **`1.23x Bun` (`2.66x` Warm) / `6.11x Node`** |
| **18** | `18-async-event-loop-fanout-scheduler` (Timer Wheel/`Promise.all`/`AbortSignal`) | **`31.587 ms`** (`29.77 ms` Min) | **`32.800 ms`** | **`17.882 ms`** | `30.874 ms` (`2.3%`) | `33.329 ms` | `70.994 ms` | `81.521 ms` | **`1.06x Bun` (`1.86x` Warm) / `2.58x Node`** |
| **19** | `19-wintertc-webcrypto-streams-edge` (WebCrypto/Streams/FormData/Clone) | **`8.890 ms`** (`8.32 ms` Min) | **`10.550 ms`** | **`1.313 ms`** | `8.593 ms` (`3.5%`) | `14.004 ms` | `46.081 ms` | `87.335 ms` | **`1.58x Bun` (`10.67x` Warm) / `9.82x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`10.934 ms`** (`10.37 ms` Min) | **`11.357 ms`** | **`0.555 ms`** | `10.668 ms` (`2.5%`) | `22.478 ms` | `53.558 ms` | `72.236 ms` | **`2.06x Bun` (`40.50x` Warm) / `6.61x Node`** |
| **21** | `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket` + 5-Ring Guard) | **`10.636 ms`** (`9.99 ms` Min) | **`11.655 ms`** | **`2.240 ms`** | `10.806 ms` (`-1.6%`) | `20.298 ms` | `52.817 ms` | `118.633 ms` | **`1.91x Bun` (`9.06x` Warm) / `11.15x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`22.051 ms`** (`21.43 ms` Min) | **`23.287 ms`** | **`4.249 ms`** | `21.844 ms` (`0.9%`) | `56.612 ms` | `116.458 ms` | `103.764 ms` | **`2.57x Bun` (`13.32x` Warm) / `4.71x Node`** |
| **23** | `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **`16.480 ms`** (`15.87 ms` Min) | **`17.338 ms`** | **`1.020 ms`** | `16.288 ms` (`1.2%`) | `21.464 ms` | `42.201 ms` | `61.815 ms` | **`1.30x Bun` (`21.04x` Warm) / `3.75x Node`** |
| **24** | `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` SLA Scorer + SSR Pipeline) | **`8.738 ms`** (`8.10 ms` Min) | **`9.087 ms`** | **`1.702 ms`** | `8.843 ms` (`-1.2%`) | `19.040 ms` | `42.919 ms` | `69.974 ms` | **`2.18x Bun` (`11.19x` Warm) / `8.01x Node`** |
| **FW-01** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`16.344 ms`** (`15.42 ms` Min) | **`16.607 ms`** | **`6.363 ms`** | `15.516 ms` (`5.3%`) | `19.684 ms` | `53.742 ms` | `95.240 ms` | **`1.20x Bun` (`3.09x` Warm) / `5.83x Node`** |
| **FW-02** | `FW-02: frameworks/02-itty-router-unjs-nitro-stack` (`itty-router` + UnJS) | **`18.518 ms`** (`17.37 ms` Min) | **`18.677 ms`** | **`9.533 ms`** | `17.620 ms` (`5.1%`) | `18.726 ms` | `47.560 ms` | `90.995 ms` | **`1.01x Bun` (`1.96x` Warm) / `4.91x Node`** |
| **FW-03** | `FW-03: frameworks/03-node-http-express-compat` (`node:http` + `node:stream` Express/Polka) | **`17.759 ms`** (`16.72 ms` Min) | **`18.895 ms`** | **`0.961 ms`** | `18.561 ms` (`-4.3%`) | `35.540 ms` | `52.294 ms` | `84.895 ms` | **`2.00x Bun` (`36.98x` Warm) / `4.78x Node`** |
| **FW-04** | `FW-04: frameworks/04-sveltekit-edge-ssr` (`@sveltejs/kit@3.1.1` + `svelte@5.57.3` + `devalue@6.0.3`) | **`34.414 ms`** (`32.91 ms` Min) | **`35.209 ms`** | **`15.092 ms`** | `33.795 ms` (`1.8%`) | `44.573 ms` | `70.457 ms` | `114.150 ms` | **`1.30x Bun` (`2.95x` Warm) / `3.32x Node`** |
| **CS-01** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`34.615 ms`** (`32.55 ms` Min) | **`36.304 ms`** | **`12.604 ms`** | `37.938 ms` (`-8.8%`) | `28.673 ms` | `73.219 ms` | `89.933 ms` | **`2.27x Bun Warm` / `2.60x Node`** |
| **CS-02** | `CS-02: case_studies/02-mcp-code-mode-orchestrator` (`-98.4%` Tokens) | **`19.795 ms`** (`18.82 ms` Min) | **`20.143 ms`** | **`3.545 ms`** | `19.490 ms` (`1.6%`) | `16.173 ms` | `43.014 ms` | `68.816 ms` | **`4.56x Bun Warm` / `3.48x Node`** |
| **CS-03** | `CS-03: case_studies/03-multi-tenant-saas-plugin-mesh` (12 Tenants + Hot-Swap) | **`14.494 ms`** (`13.58 ms` Min) | **`15.443 ms`** | **`6.201 ms`** | `14.633 ms` (`-0.9%`) | `16.034 ms` | `48.235 ms` | `84.985 ms` | **`1.11x Bun` (`2.59x` Warm) / `5.86x Node`** |
| **CS-04** | `CS-04: case_studies/04-wasm-zero-trust-cdr-pipeline` (Wasm SimHash CDR) | **`20.976 ms`** (`20.11 ms` Min) | **`23.846 ms`** | **`1.535 ms`** | `20.253 ms` (`3.6%`) | `19.273 ms` | `45.209 ms` | `67.223 ms` | **`12.56x Bun Warm` / `3.20x Node`** |
| **CS-05** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`17.793 ms`** (`16.95 ms` Min) | **`18.421 ms`** | **`8.641 ms`** | `17.054 ms` (`4.3%`) | `19.880 ms` | `62.584 ms` | `85.425 ms` | **`1.12x Bun` (`2.30x` Warm) / `4.80x Node`** |

---

## 4. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Workload | Slice (`SLICEBC\x01` Default) | Bun (`v1.4.4`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha.0.0.0`) | Slice Memory Advantage |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **App 2: NPM semver+ms+ini** | **13,884 KB (13.56 MB)** | 29,280 KB (28.59 MB) | 47,432 KB (46.32 MB) | 71,580 KB (69.90 MB) | **5.16x less than Node / 2.11x less than Bun** |
| **App 6: Telemetry + Flatted** | **14,464 KB (14.13 MB)** | 30,080 KB (29.38 MB) | 50,816 KB (49.63 MB) | 72,732 KB (71.03 MB) | **5.03x less than Node / 2.08x less than Bun** |
| **App 21: RFC 6455 WS + Sec** | **14,720 KB (14.38 MB)** | 32,104 KB (31.35 MB) | 50,840 KB (49.65 MB) | 95,616 KB (93.38 MB) | **6.50x less than Node / 2.18x less than Bun** |
| **App 22: Workers + Sockets** | **23,296 KB (22.75 MB)** | 41,164 KB (40.20 MB) | 64,108 KB (62.61 MB) | 92,624 KB (90.45 MB) | **3.98x less than Node / 1.77x less than Bun** |
| **App 23: Pure-Rust WASM** | **19,328 KB (18.88 MB)** | 31,080 KB (30.35 MB) | 48,264 KB (47.13 MB) | 79,460 KB (77.60 MB) | **4.11x less than Node / 1.61x less than Bun** |
| **FW 1: Hono v4 Microservice** | **16,256 KB (15.88 MB)** | 30,884 KB (30.16 MB) | 49,140 KB (47.99 MB) | 89,244 KB (87.15 MB) | **5.49x less than Node / 1.90x less than Bun** |
| **FW 2: itty-router + UnJS** | **15,232 KB (14.88 MB)** | 32,000 KB (31.25 MB) | 48,368 KB (47.23 MB) | 86,916 KB (84.88 MB) | **5.71x less than Node / 2.10x less than Bun** |
| **App 24: WASM + SSR** | **14,352 KB (14.02 MB)** | 31,872 KB (31.13 MB) | 48,972 KB (47.82 MB) | 80,724 KB (78.83 MB) | **5.62x less than Node / 2.22x less than Bun** |
| **FW 3: Express node:http** | **15,872 KB (15.50 MB)** | 33,152 KB (32.38 MB) | 47,208 KB (46.10 MB) | 80,524 KB (78.64 MB) | **5.07x less than Node / 2.09x less than Bun** |
| **01-tot-autonomous-sre-agent** | **21,504 KB (21.00 MB)** | 31,232 KB (30.50 MB) | 53,460 KB (52.21 MB) | 83,420 KB (81.46 MB) | **3.88x less than Node / 1.45x less than Bun** |
| **02-mcp-code-mode-orchestrator** | **18,680 KB (18.24 MB)** | 27,776 KB (27.13 MB) | 46,472 KB (45.38 MB) | 82,692 KB (80.75 MB) | **4.43x less than Node / 1.49x less than Bun** |
| **03-multi-tenant-saas-plugin-mesh** | **15,744 KB (15.38 MB)** | 28,288 KB (27.63 MB) | 49,992 KB (48.82 MB) | 87,684 KB (85.63 MB) | **5.57x less than Node / 1.80x less than Bun** |
| **04-wasm-zero-trust-cdr-pipeline** | **21,168 KB (20.67 MB)** | 31,232 KB (30.50 MB) | 49,708 KB (48.54 MB) | 80,132 KB (78.25 MB) | **3.79x less than Node / 1.48x less than Bun** |
| **05-time-travel-chaos-fuzzer-payments** | **16,256 KB (15.88 MB)** | 29,312 KB (28.63 MB) | 49,528 KB (48.37 MB) | 81,432 KB (79.52 MB) | **5.01x less than Node / 1.80x less than Bun** |
| **FW 4: SvelteKit 3 + Svelte 5** | **22,912 KB (22.38 MB)** | 41,984 KB (41.00 MB) | 56,204 KB (54.89 MB) | 85,724 KB (83.71 MB) | **3.74x less than Node / 1.83x less than Bun** |

---

## 5. Multi-Tenant Edge KV, AI Agent Checkpoints & 5-Ring Security Telemetry

| Subsystem / Benchmark | Measured Telemetry | Status |
| :--- | :--- | :---: |
| **Sharded `Slice.kv` (`1,001` Get/Set/Incr Ops in Isolate)** | **`514.81 us`** total (`--warm`, `8,466 B` memory cage) | `PASS` |
| **Multi-Tenant `--tenant-dir` Zero-Downtime Hot-Swap** | Live atomic replacement of `tenant-alpha.cas` (`v1 -> v2-hot-swapped`) with isolated `Slice.kv` state | `PASS` |
| **Deterministic Time-Travel Replay (`--deterministic=20261019`)** | 100% bit-for-bit identical PRNG (`Math.random`) & virtual clock (`Date.now`, `performance.now`) | `PASS` |
| **Tree-of-Thought AI Agent (`Slice.checkpoint` + 3 `evalSandbox` + `restore`)** | **`41.76 us`** total (`1.85 us` child sandbox) | `PASS` |
| **Cloud Metadata SSRF (`169.254.169.254`) & Base64 Egress DLP Guard** | `SSRF_METADATA_BLOCKED` & `EGRESS_SECRET_LEAK_BLOCKED` deterministically trapped | `PASS` |
| **Built-In Model Context Protocol Server (`slice mcp --smoke-test`)** | JSON-RPC 2.0 `initialize`, `tools/list`, `tools/call` verified over `stdio` | `PASS` |
