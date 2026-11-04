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
| **FW 1: `01-hono-edge-microservice`** (`hono@4.6.20` + `cookie` + `ms`) | **28.212 ms / 29.229 ms** | **28.499 ms / 29.332 ms** | **6.560 ms** (`0.06 us` init) | 18.391 ms / 19.633 ms | 48.183 ms / 51.357 ms | 90.885 ms / 97.127 ms |
| **FW 2: `02-itty-router-unjs-nitro-stack`** (`itty-router` + `ufo` + `destr` + `ohash` + `radix3`) | **27.422 ms / 28.482 ms** | **27.554 ms / 29.443 ms** | **8.922 ms** (`0.05 us` init) | 17.925 ms / 19.030 ms | 43.899 ms / 47.855 ms | 82.748 ms / 87.404 ms |

### 4. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Framework Suite | Slice (`v0.9.8`) | Bun (`v1.4.4`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha`) | Slice Memory Advantage |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **FW 1: `01-hono-edge-microservice`** (`hono` v4) | **15,488 KB (15.12 MB)** | 30,720 KB (30.00 MB) | 49,008 KB (47.86 MB) | 89,116 KB (87.03 MB) | **5.75x less than Node / 1.98x less than Bun** |
| **FW 2: `02-itty-router-unjs-nitro-stack`** (UnJS / Nitro) | **14,464 KB (14.12 MB)** | 31,744 KB (31.00 MB) | 48,760 KB (47.62 MB) | 86,924 KB (84.89 MB) | **6.01x less than Node / 2.19x less than Bun** |

---

## Running the Framework Benchmark Suite in Docker

```bash
docker build -f tests/Dockerfile -t slice-real-stack-bench .
docker run --rm -e SLICE_BENCH_TAGS=frameworks slice-real-stack-bench
```


