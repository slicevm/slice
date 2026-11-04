# Slice (Nexus-JS) — Real-World Framework Test & Benchmark Suite (`tests/frameworks/`)

This directory houses full-stack framework integration tests and cross-runtime benchmarks comparing **Slice**, **Bun**, **Deno**, and **Node.js** on unmodified production Edge and WinterTC frameworks installed from `node_modules/`.

General multi-file application and security benchmarks (`01-fintech-checkout` through `24-tsx-ssr-and-wasm-edge-pipeline`) are located in [`../apps/`](../apps/).

---

## Framework Suites

| ID | Framework Suite | Real External `npm` Packages (`node_modules/`) | Exercised Capabilities |
| :--- | :--- | :--- | :--- |
| **FW 1 (`App 25`)** | [`01-hono-edge-microservice`](./01-hono-edge-microservice) | `hono@4.6.20`, `hono/cookie`, `cookie@0.6.0`, `ms@2.1.3` | Unmodified **Hono v4** `Hono` router, `app.use("*")` security/request-ID middleware, `app.use("/api/v1/tenants/:tenantId/*")` Bearer HMAC-SHA256 tenant guard (`401`), parameterized `GET /api/v1/tenants/:tenantId/orders/:orderId` (`200`), `POST /api/v1/tenants/:tenantId/checkout` (`201`), `getCookie`/`setCookie`, `app.notFound` (`404`), and `slice test` (`tests/hono.test.ts`). |
| **FW 2 (`App 26`)** | [`02-itty-router-unjs-nitro-stack`](./02-itty-router-unjs-nitro-stack) | `itty-router@5.0.18`, `ufo@1.5.4`, `destr@2.0.3`, `ohash@1.1.4`, `radix3@1.1.2` | Unmodified **itty-router v5** (`AutoRouter`) + **UnJS / Nitro** toolkit (`ufo` URL/query normalization, `radix3` route policy matcher, `destr` prototype-pollution-safe JSON parser, `ohash` deterministic object hash & SHA-256 Base64 ETag generator, `If-None-Match` -> `304 Not Modified`), and `slice test` (`tests/unjs.test.ts`). |

---

## Verified Linux Docker (`x86_64`) Benchmark Results (`SLICE_BENCH_TAGS=frameworks`)

### 1. [`01-hono-edge-microservice`](./01-hono-edge-microservice/src/index.ts) — Hono v4 Edge Microservice (`hono@4.6.20`, `hono/cookie`, `cookie@0.6.0`, `ms@2.1.3`)
- **Identical Output Across All 4 Runtimes (`Slice`, `Bun v1.4.4`, `Deno v2.9.8`, `Node.js v27.0.0-alpha`):**
  ```json
  {"framework":"01-hono-edge-microservice","libraries":["cookie@0.6.0","hono@4.6.20","ms@2.1.3"],"totalRequestsDispatched":12,"status200Count":7,"status201Count":3,"status401Count":1,"status404Count":1,"cookiesIssued":9,"totalRevenueCents":77470,"totalTaxCents":9994,"middlewareHeadersVerified":true,"healthStatus":"healthy:Hono-v4","auditDigest":"499c81f97f4a1f33"}
  ```
- **Time-Travel `slice test` Runner (`tests/hono.test.ts`):** `2 passed, 0 failed` (`2 VFS+KV rollbacks in 14.89 ms`)

### 2. [`02-itty-router-unjs-nitro-stack`](./02-itty-router-unjs-nitro-stack/src/index.ts) — itty-router v5 + UnJS / Nitro Stack (`itty-router@5.0.18`, `ufo@1.5.4`, `destr@2.0.3`, `ohash@1.1.4`, `radix3@1.1.2`)
- **Identical Output Across All 4 Runtimes (`Slice`, `Bun v1.4.4`, `Deno v2.9.8`, `Node.js v27.0.0-alpha`):**
  ```json
  {"framework":"02-itty-router-unjs-nitro-stack","libraries":["destr@2.0.3","itty-router@5.0.18","ohash@1.1.4","radix3@1.1.2","ufo@1.5.4"],"totalRequestsDispatched":12,"status200Count":6,"status202Count":3,"status304Count":2,"status404Count":1,"radixPolicyMatches":11,"normalizedUrlsCount":12,"destrPayloadsParsed":3,"uniqueEtagsGenerated":5,"totalCatalogValueCents":24553135,"totalWebhookAmountCents":7500,"sampleCanonicalSearchUrl":"/v1/search?category=gpu&page=2&limit=25&sort=price_asc","auditDigest":"40f28432a68c05ac"}
  ```
- **Time-Travel `slice test` Runner (`tests/unjs.test.ts`):** `2 passed, 0 failed` (`2 VFS+KV rollbacks in 13.98 ms`)

---

### 3. Cold CLI Latency (20 Runs) & Warm Isolate Latency (`--warm`)

| Framework Suite | Slice Cold `.cas` (Min / Avg) | Slice Standalone Bin (Min / Avg) | Slice Warm Isolate (`--warm`) | Bun `v1.4.4` (Min / Avg) | Deno `v2.9.8` (Min / Avg) | Node.js `v27.0.0-alpha` (Min / Avg) |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **FW 1: `01-hono-edge-microservice`** (`hono@4.6.20` + `cookie` + `ms`) | **17.534 ms / 18.599 ms** | **17.594 ms / 18.607 ms** | **2.312 ms** (`0.08 us` init) | 18.829 ms / 19.871 ms | 50.341 ms / 54.317 ms | 94.402 ms / 97.324 ms |
| **FW 2: `02-itty-router-unjs-nitro-stack`** (`itty-router` + `ufo` + `destr` + `ohash` + `radix3`) | **16.680 ms / 17.373 ms** | **17.033 ms / 17.930 ms** | **2.073 ms** (`0.03 us` init) | 18.018 ms / 19.310 ms | 44.211 ms / 47.543 ms | 84.550 ms / 88.654 ms |

### 4. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Framework Suite | Slice (`v0.9.3`) | Bun (`v1.4.4`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha`) | Slice Memory Advantage |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **FW 1: `01-hono-edge-microservice`** (`hono` v4) | **13,672 KB (13.35 MB)** | 30,336 KB (29.62 MB) | 49,520 KB (48.36 MB) | 89,112 KB (87.02 MB) | **6.52x less than Node / 2.22x less than Bun** |
| **FW 2: `02-itty-router-unjs-nitro-stack`** (UnJS / Nitro) | **13,672 KB (13.35 MB)** | 31,232 KB (30.50 MB) | 47,860 KB (46.74 MB) | 86,920 KB (84.88 MB) | **6.36x less than Node / 2.28x less than Bun** |

---

## Running the Framework Benchmark Suite in Docker

```bash
docker build -f tests/Dockerfile -t slice-real-stack-bench .
docker run --rm -e SLICE_BENCH_TAGS=frameworks slice-real-stack-bench
```


