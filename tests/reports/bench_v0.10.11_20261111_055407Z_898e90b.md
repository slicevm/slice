# Slice (Nexus-JS) — Full 33-Workload & MicroVM Benchmark Report

* **Slice Version:** `v0.10.11` (Commit `898e90b`)
* **Generated Timestamp (UTC):** `2026-11-11T05:54:07Z`
* **Archive File:** [`tests/reports/bench_v0.10.11_20261111_055407Z_898e90b.md`](./bench_v0.10.11_20261111_055407Z_898e90b.md)
* **Canonical Latest Report:** [`tests/reports/latest.md`](./latest.md)
* **Environment:** `Linux x86_64 Docker (io_uring SQPOLL + WRPKRU)`
* **Compared Runtimes:**
  * **Slice:** `v0.10.11` (Default `SLICEBC\x01` Precompiled Boa `CodeBlock` CAS + Monomorphic Inline Caching `get_fast` + Iterative ECMA-404 `JSON.parse`)
  * **Bun:** `v1.4.4` (JavaScriptCore)
  * **Deno:** `v2.9.8` (V8)
  * **Node.js:** `v27.0.0-alpha.0.0.0` (V8 native TypeScript execution)

---

## 1. Executive Summary

Across all **33 real-world multi-file TypeScript applications, framework suites, and case studies**, Slice beats **Deno (`v2.9.8`)** on **33/33** workloads, beats **Node.js (`v27.0.0-alpha.0.0.0`)** on **33/33** workloads, beats **Bun (`v1.4.4`)** on **28/33** workloads in cold one-shot execution, and beats **all 3 runtimes on 33/33 workloads in `--warm` pooled isolate execution**.

| Metric | Linux (`x86_64` Docker) | Windows (`x86_64` Host) | Comparison vs Runtimes & MicroVMs |
| :--- | :---: | :---: | :--- |
| **Single Isolate Cold-Start (Min / P50 / P99)** | **`0.98 us` / `1.23 us` / `2.03 us`** | **`1.40 us` / `1.60 us` / `2.30 us`** | **~120,000x faster than 125ms Firecracker VM** |
| **1,000-Tenant MicroVM Swarm (Boot + Exec)** | **`1.47 ms`** (`9.70 MB` RSS) | **`2.24 ms`** (`11.82 MB` RSS) | **> 10,000x faster than container/VM swarms** |
| **500-Module Monorepo Cold Boot (`0 stat()`)** | **`3.408 ms`** (`372.98 us` in-mem) | **`5.637 ms`** (`640.00 us` in-mem) | **20.6x faster than Node.js (`1,500+ stat()`)** |
| **10,000 Direct Kernel Ring I/O Completions** | **`0.02 ms`** | **`5.11 ms`** | **Zero worker-thread lock contention** |
| **5-Ring Security Containment Verification** | **`5 / 5 PASS`** | **`5 / 5 PASS`** | **OCap + SES + Fuel + SSRF/DLP + MPK** |

---

## 2. `slice serve` Multi-Tenant HTTP/1.1 Keep-Alive Throughput (`16,000` Reqs, 8 Sockets x 8 Pipeline)

| Server / Runtime | Throughput (`req/s`) | Avg Latency / Req | Total Time (`16k` Reqs) | Relative Speedup vs Runtimes |
| :--- | :---: | :---: | :---: | :--- |
| **`slice serve` (Tier-0 CAS, `--workers=8`)** | **`153,174 req/s`** | **`6.53 us/req`** | **`104.5 ms`** | **`1.52x` faster than `Bun.serve` / `2.80x` Node.js** |
| **`slice serve` (Tier-1 WinterTC, `--workers=8`)** | **`145,192 req/s`** | **`6.89 us/req`** | **`110.2 ms`** | **`1.44x` faster than `Bun.serve` / `2.66x` Node.js** |
| **`Deno.serve` (`v2.9.8`, V8, No Per-Req Sandbox)** | `99,307 req/s` | `10.07 us/req` | `161.1 ms` | `0.98x` vs Bun |
| **`Bun.serve` (`v1.4.4`, JSC, No Per-Req Sandbox)** | `101,093 req/s` | `9.89 us/req` | `158.3 ms` | `1.00x` (Baseline) |
| **`Node.js http.createServer` (`v27.0.0-alpha.0.0.0`, V8)** | `54,637 req/s` | `18.30 us/req` | `292.8 ms` | `0.54x` vs Bun |

---

## 3. Full 33-Workload Latency Comparison (20 Runs Each, Linux `x86_64` Docker)

| # | Workload (`tests/apps/`, `tests/frameworks/`, `tests/case_studies/`) | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm (`--warm`) | Previous `.cas` (`Delta`) | Bun `v1.4.4` | Deno `v2.9.8` | Node.js `v27.0.0-alpha.0.0.0` | Slice Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`6.698 ms`** (`5.89 ms` Min) | **`6.942 ms`** | **`1.679 ms`** | `5.659 ms` (`18.4%`) | `13.445 ms` | `35.698 ms` | `59.152 ms` | **`2.01x Bun` (`8.01x` Warm) / `8.83x Node`** |
| **02** | `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | **`14.785 ms`** (`13.71 ms` Min) | **`14.951 ms`** | **`6.431 ms`** | `12.659 ms` (`16.8%`) | `21.505 ms` | `52.831 ms` | `69.235 ms` | **`1.45x Bun` (`3.34x` Warm) / `4.68x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`7.617 ms`** (`7.24 ms` Min) | **`7.855 ms`** | **`1.723 ms`** | `6.727 ms` (`13.2%`) | `13.792 ms` | `45.409 ms` | `61.734 ms` | **`1.81x Bun` (`8.00x` Warm) / `8.10x Node`** |
| **04** | `04-edge-api-gateway` (`fast-querystring` + `ms`) | **`7.559 ms`** (`7.10 ms` Min) | **`8.159 ms`** | **`3.956 ms`** | `6.624 ms` (`14.1%`) | `14.699 ms` | `41.787 ms` | `62.835 ms` | **`1.94x Bun` (`3.72x` Warm) / `8.31x Node`** |
| **05** | `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | **`10.511 ms`** (`10.00 ms` Min) | **`11.042 ms`** | **`7.876 ms`** | `9.604 ms` (`9.4%`) | `14.601 ms` | `44.318 ms` | `62.788 ms` | **`1.39x Bun` (`1.85x` Warm) / `5.97x Node`** |
| **06** | `06-telemetry-metrics-pipeline` (`bytes` + `flatted` + WAL) | **`14.045 ms`** (`13.21 ms` Min) | **`13.994 ms`** | **`6.433 ms`** | `13.023 ms` (`7.8%`) | `17.194 ms` | `48.119 ms` | `65.404 ms` | **`1.23x Bun` (`2.67x` Warm) / `4.67x Node`** |
| **07** | `07-graphql-ast-validator` (`dlv` + `dset` + AST Cost) | **`9.193 ms`** (`8.44 ms` Min) | **`9.804 ms`** | **`5.195 ms`** | `8.178 ms` (`12.4%`) | `13.052 ms` | `40.817 ms` | `61.714 ms` | **`1.42x Bun` (`2.51x` Warm) / `6.71x Node`** |
| **08** | `08-event-driven-state-journal` (`mitt` + `ms` + JSONL Replay) | **`11.667 ms`** (`11.04 ms` Min) | **`11.989 ms`** | **`5.807 ms`** | `10.036 ms` (`16.3%`) | `16.467 ms` | `46.579 ms` | `66.675 ms` | **`1.41x Bun` (`2.84x` Warm) / `5.71x Node`** |
| **09** | `09-wintertc-web-router` (`Request` + `Response` + `URL`, 80 reqs) | **`16.323 ms`** (`15.03 ms` Min) | **`16.184 ms`** | **`9.689 ms`** | `13.781 ms` (`18.4%`) | `13.954 ms` | `47.109 ms` | `86.236 ms` | **`1.44x Bun Warm` / `5.33x Node`** |
| **10** | `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **`13.778 ms`** (`12.70 ms` Min) | **`13.793 ms`** | **`7.468 ms`** | `12.008 ms` (`14.7%`) | `14.611 ms` | `47.719 ms` | `85.928 ms` | **`1.06x Bun` (`1.96x` Warm) / `6.24x Node`** |
| **11** | `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **`10.366 ms`** (`9.73 ms` Min) | **`10.611 ms`** | **`4.123 ms`** | `9.199 ms` (`12.7%`) | `13.422 ms` | `49.132 ms` | `86.083 ms` | **`1.29x Bun` (`3.26x` Warm) / `8.30x Node`** |
| **12** | `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP, 320 frames) | **`15.430 ms`** (`14.88 ms` Min) | **`15.783 ms`** | **`9.011 ms`** | `14.114 ms` (`9.3%`) | `15.047 ms` | `49.863 ms` | `89.926 ms` | **`1.67x Bun Warm` / `5.83x Node`** |
| **13** | `13-webhook-orchestrator-ssrf` (`fast-querystring` + `ms` + Live `fetch`) | **`17.563 ms`** (`15.40 ms` Min) | **`15.976 ms`** | **`6.678 ms`** | `12.492 ms` (`40.6%`) | `19.819 ms` | `51.823 ms` | `113.097 ms` | **`1.24x Bun` (`2.97x` Warm) / `7.08x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`16.692 ms`** (`15.44 ms` Min) | **`16.840 ms`** | **`7.968 ms`** | `14.838 ms` (`12.5%`) | `21.633 ms` | `53.171 ms` | `112.883 ms` | **`1.30x Bun` (`2.71x` Warm) / `6.76x Node`** |
| **15** | `15-time-travel-test-suite` (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **`16.035 ms`** (`15.36 ms` Min) | **`15.777 ms`** | **`4.855 ms`** | `14.887 ms` (`7.7%`) | `25.404 ms` | `67.449 ms` | `84.002 ms` | **`1.61x Bun` (`5.23x` Warm) / `5.32x Node`** |
| **16** | `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **`8.079 ms`** (`7.46 ms` Min) | **`8.455 ms`** | **`1.385 ms`** | `7.467 ms` (`8.2%`) | `14.281 ms` | `44.477 ms` | `63.767 ms` | **`1.77x Bun` (`10.31x` Warm) / `7.89x Node`** |
| **17** | `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive/307/SSRF/DLP) | **`17.376 ms`** (`16.67 ms` Min) | **`19.974 ms`** | **`9.042 ms`** | `15.779 ms` (`10.1%`) | `22.206 ms` | `55.655 ms` | `112.300 ms` | **`1.28x Bun` (`2.46x` Warm) / `6.46x Node`** |
| **18** | `18-async-event-loop-fanout-scheduler` (Timer Wheel/`Promise.all`/`AbortSignal`) | **`30.841 ms`** (`28.84 ms` Min) | **`31.316 ms`** | **`17.473 ms`** | `29.409 ms` (`4.9%`) | `32.649 ms` | `73.881 ms` | `81.441 ms` | **`1.06x Bun` (`1.87x` Warm) / `2.64x Node`** |
| **19** | `19-wintertc-webcrypto-streams-edge` (WebCrypto/Streams/FormData/Clone) | **`8.515 ms`** (`8.12 ms` Min) | **`8.959 ms`** | **`1.493 ms`** | `8.407 ms` (`1.3%`) | `14.021 ms` | `45.631 ms` | `87.941 ms` | **`1.65x Bun` (`9.39x` Warm) / `10.33x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`11.298 ms`** (`10.48 ms` Min) | **`11.547 ms`** | **`0.612 ms`** | `10.148 ms` (`11.3%`) | `22.400 ms` | `54.998 ms` | `76.295 ms` | **`1.98x Bun` (`36.60x` Warm) / `6.75x Node`** |
| **21** | `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket` + 5-Ring Guard) | **`11.073 ms`** (`10.66 ms` Min) | **`11.583 ms`** | **`3.777 ms`** | `10.288 ms` (`7.6%`) | `20.349 ms` | `52.160 ms` | `117.179 ms` | **`1.84x Bun` (`5.39x` Warm) / `10.58x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`20.159 ms`** (`19.42 ms` Min) | **`20.547 ms`** | **`2.228 ms`** | `19.101 ms` (`5.5%`) | `55.925 ms` | `112.632 ms` | `101.841 ms` | **`2.77x Bun` (`25.10x` Warm) / `5.05x Node`** |
| **23** | `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **`15.430 ms`** (`14.75 ms` Min) | **`16.036 ms`** | **`1.954 ms`** | `13.643 ms` (`13.1%`) | `21.292 ms` | `42.492 ms` | `61.274 ms` | **`1.38x Bun` (`10.90x` Warm) / `3.97x Node`** |
| **24** | `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` SLA Scorer + SSR Pipeline) | **`8.744 ms`** (`8.23 ms` Min) | **`9.198 ms`** | **`1.838 ms`** | `8.336 ms` (`4.9%`) | `20.232 ms` | `43.778 ms` | `68.091 ms` | **`2.31x Bun` (`11.01x` Warm) / `7.79x Node`** |
| **FW-01** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`15.219 ms`** (`14.46 ms` Min) | **`15.812 ms`** | **`5.631 ms`** | `13.701 ms` (`11.1%`) | `19.633 ms` | `52.596 ms` | `94.610 ms` | **`1.29x Bun` (`3.49x` Warm) / `6.22x Node`** |
| **FW-02** | `FW-02: frameworks/02-itty-router-unjs-nitro-stack` (`itty-router` + UnJS) | **`17.819 ms`** (`17.14 ms` Min) | **`18.045 ms`** | **`9.665 ms`** | `16.349 ms` (`9.0%`) | `20.945 ms` | `46.351 ms` | `87.095 ms` | **`1.18x Bun` (`2.17x` Warm) / `4.89x Node`** |
| **FW-03** | `FW-03: frameworks/03-node-http-express-compat` (`node:http` + `node:stream` Express/Polka) | **`17.267 ms`** (`16.45 ms` Min) | **`17.638 ms`** | **`1.311 ms`** | `14.809 ms` (`16.6%`) | `34.805 ms` | `51.001 ms` | `82.310 ms` | **`2.02x Bun` (`26.55x` Warm) / `4.77x Node`** |
| **FW-04** | `FW-04: frameworks/04-sveltekit-edge-ssr` (`@sveltejs/kit@3.1.1` + `svelte@5.57.3` + `devalue@6.0.3`) | **`32.075 ms`** (`31.12 ms` Min) | **`32.819 ms`** | **`16.953 ms`** | `33.452 ms` (`-4.1%`) | `43.525 ms` | `74.247 ms` | `112.271 ms` | **`1.36x Bun` (`2.57x` Warm) / `3.50x Node`** |
| **CS-01** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`34.706 ms`** (`30.76 ms` Min) | **`32.179 ms`** | **`13.019 ms`** | `29.806 ms` (`16.4%`) | `28.387 ms` | `72.119 ms` | `88.960 ms` | **`2.18x Bun Warm` / `2.76x Node`** |
| **CS-02** | `CS-02: case_studies/02-mcp-code-mode-orchestrator` (`-98.4%` Tokens) | **`18.871 ms`** (`18.21 ms` Min) | **`19.492 ms`** | **`5.409 ms`** | `16.342 ms` (`15.5%`) | `16.618 ms` | `43.050 ms` | `68.969 ms` | **`3.07x Bun Warm` / `3.65x Node`** |
| **CS-03** | `CS-03: case_studies/03-multi-tenant-saas-plugin-mesh` (12 Tenants + Hot-Swap) | **`13.687 ms`** (`13.07 ms` Min) | **`14.343 ms`** | **`5.404 ms`** | `12.120 ms` (`12.9%`) | `16.388 ms` | `48.628 ms` | `85.608 ms` | **`1.20x Bun` (`3.03x` Warm) / `6.25x Node`** |
| **CS-04** | `CS-04: case_studies/04-wasm-zero-trust-cdr-pipeline` (Wasm SimHash CDR) | **`19.960 ms`** (`19.14 ms` Min) | **`20.818 ms`** | **`2.104 ms`** | `15.992 ms` (`24.8%`) | `19.218 ms` | `44.875 ms` | `68.073 ms` | **`9.13x Bun Warm` / `3.41x Node`** |
| **CS-05** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`18.213 ms`** (`17.27 ms` Min) | **`19.284 ms`** | **`7.924 ms`** | `18.391 ms` (`-1.0%`) | `20.307 ms` | `62.714 ms` | `84.144 ms` | **`1.11x Bun` (`2.56x` Warm) / `4.62x Node`** |

---

## 4. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Workload | Slice (`SLICEBC\x01` Default) | Bun (`v1.4.4`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha.0.0.0`) | Slice Memory Advantage |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **App 2: NPM semver+ms+ini** | **14,592 KB (14.25 MB)** | 29,952 KB (29.25 MB) | 47,344 KB (46.23 MB) | 71,584 KB (69.91 MB) | **4.91x less than Node / 2.05x less than Bun** |
| **App 6: Telemetry + Flatted** | **15,104 KB (14.75 MB)** | 30,464 KB (29.75 MB) | 50,928 KB (49.73 MB) | 72,860 KB (71.15 MB) | **4.82x less than Node / 2.02x less than Bun** |
| **App 21: RFC 6455 WS + Sec** | **15,104 KB (14.75 MB)** | 31,732 KB (30.99 MB) | 50,756 KB (49.57 MB) | 95,356 KB (93.12 MB) | **6.31x less than Node / 2.10x less than Bun** |
| **App 22: Workers + Sockets** | **22,272 KB (21.75 MB)** | 41,688 KB (40.71 MB) | 63,236 KB (61.75 MB) | 94,180 KB (91.97 MB) | **4.23x less than Node / 1.87x less than Bun** |
| **App 23: Pure-Rust WASM** | **19,200 KB (18.75 MB)** | 30,720 KB (30.00 MB) | 48,784 KB (47.64 MB) | 80,660 KB (78.77 MB) | **4.20x less than Node / 1.60x less than Bun** |
| **FW 1: Hono v4 Microservice** | **15,872 KB (15.50 MB)** | 30,976 KB (30.25 MB) | 49,144 KB (47.99 MB) | 88,816 KB (86.73 MB) | **5.60x less than Node / 1.95x less than Bun** |
| **FW 2: itty-router + UnJS** | **15,232 KB (14.88 MB)** | 32,128 KB (31.38 MB) | 48,640 KB (47.50 MB) | 87,308 KB (85.26 MB) | **5.73x less than Node / 2.11x less than Bun** |
| **App 24: WASM + SSR** | **14,592 KB (14.25 MB)** | 30,592 KB (29.88 MB) | 49,332 KB (48.18 MB) | 81,028 KB (79.13 MB) | **5.55x less than Node / 2.10x less than Bun** |
| **FW 3: Express node:http** | **15,616 KB (15.25 MB)** | 33,152 KB (32.38 MB) | 47,340 KB (46.23 MB) | 80,140 KB (78.26 MB) | **5.13x less than Node / 2.12x less than Bun** |
| **01-tot-autonomous-sre-agent** | **20,736 KB (20.25 MB)** | 32,128 KB (31.38 MB) | 53,360 KB (52.11 MB) | 83,128 KB (81.18 MB) | **4.01x less than Node / 1.55x less than Bun** |
| **02-mcp-code-mode-orchestrator** | **18,668 KB (18.23 MB)** | 28,288 KB (27.63 MB) | 46,624 KB (45.53 MB) | 82,692 KB (80.75 MB) | **4.43x less than Node / 1.52x less than Bun** |
| **03-multi-tenant-saas-plugin-mesh** | **16,000 KB (15.63 MB)** | 26,752 KB (26.13 MB) | 49,776 KB (48.61 MB) | 88,100 KB (86.04 MB) | **5.51x less than Node / 1.67x less than Bun** |
| **04-wasm-zero-trust-cdr-pipeline** | **20,608 KB (20.13 MB)** | 31,232 KB (30.50 MB) | 49,748 KB (48.58 MB) | 80,264 KB (78.38 MB) | **3.89x less than Node / 1.52x less than Bun** |
| **05-time-travel-chaos-fuzzer-payments** | **17,024 KB (16.63 MB)** | 29,184 KB (28.50 MB) | 49,524 KB (48.36 MB) | 81,696 KB (79.78 MB) | **4.80x less than Node / 1.71x less than Bun** |
| **FW 4: SvelteKit 3 + Svelte 5** | **22,400 KB (21.88 MB)** | 42,112 KB (41.13 MB) | 56,616 KB (55.29 MB) | 85,792 KB (83.78 MB) | **3.83x less than Node / 1.88x less than Bun** |

---

## 5. Multi-Tenant Edge KV, AI Agent Checkpoints & 5-Ring Security Telemetry

| Subsystem / Benchmark | Measured Telemetry | Status |
| :--- | :--- | :---: |
| **Sharded `Slice.kv` (`1,001` Get/Set/Incr Ops in Isolate)** | **`706.00 us`** total (`--warm`, `8,431 B` memory cage) | `PASS` |
| **Multi-Tenant `--tenant-dir` Zero-Downtime Hot-Swap** | Live atomic replacement of `tenant-alpha.cas` (`v1 -> v2-hot-swapped`) with isolated `Slice.kv` state | `PASS` |
| **Deterministic Time-Travel Replay (`--deterministic=20261019`)** | 100% bit-for-bit identical PRNG (`Math.random`) & virtual clock (`Date.now`, `performance.now`) | `PASS` |
| **Tree-of-Thought AI Agent (`Slice.checkpoint` + 3 `evalSandbox` + `restore`)** | **`129.86 us`** total (`2.80 us` child sandbox) | `PASS` |
| **Cloud Metadata SSRF (`169.254.169.254`) & Base64 Egress DLP Guard** | `SSRF_METADATA_BLOCKED` & `EGRESS_SECRET_LEAK_BLOCKED` deterministically trapped | `PASS` |
| **Built-In Model Context Protocol Server (`slice mcp --smoke-test`)** | JSON-RPC 2.0 `initialize`, `tools/list`, `tools/call` verified over `stdio` | `PASS` |
