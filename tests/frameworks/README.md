# Slice (Nexus-JS) — Real-World Framework Test & Benchmark Suite (`tests/frameworks/`)

This directory houses full-stack framework integration tests and cross-runtime benchmarks comparing **Slice**, **Bun**, **Deno**, and **Node.js** on unmodified production Edge and WinterTC frameworks installed from `node_modules/`.

General multi-file application and security benchmarks (`01-fintech-checkout` through `24-tsx-ssr-and-wasm-edge-pipeline`) are located in [`../apps/`](../apps/).

---

## Framework Suites

| ID | Framework Suite | Real External `npm` Packages (`node_modules/`) | Exercised Capabilities |
| :--- | :--- | :--- | :--- |
| **FW 1 (`App 25`)** | [`01-hono-edge-microservice`](./01-hono-edge-microservice) | `hono@4.6.20`, `hono/cookie`, `cookie@0.6.0`, `ms@2.1.3` | Unmodified **Hono v4** `Hono` router, `app.use("*")` security/request-ID middleware, `app.use("/api/v1/tenants/:tenantId/*")` Bearer HMAC-SHA256 tenant guard (`401`), parameterized `GET /api/v1/tenants/:tenantId/orders/:orderId` (`200`), `POST /api/v1/tenants/:tenantId/checkout` (`201`), `getCookie`/`setCookie`, `app.notFound` (`404`), and `slice test` (`tests/hono.test.ts`). |
| **FW 2 (`App 26`)** | [`02-itty-router-unjs-nitro-stack`](./02-itty-router-unjs-nitro-stack) | `itty-router@5.0.18`, `ufo@1.5.4`, `destr@2.0.3`, `ohash@1.1.4`, `radix3@1.1.2` | Unmodified **itty-router v5** (`AutoRouter`) + **UnJS / Nitro** toolkit (`ufo` URL/query normalization, `radix3` route policy matcher, `destr` prototype-pollution-safe JSON parser, `ohash` deterministic object hash & SHA-256 Base64 ETag generator, `If-None-Match` -> `304 Not Modified`), and `slice test` (`tests/unjs.test.ts`). |
| **FW 3 (`App 27`)** | [`03-node-http-express-compat`](./03-node-http-express-compat) | `node:http`, `node:stream`, `node:stream/promises`, `node:crypto` | Express / Polka-compatible `http.createServer` + `app.use()` middleware chain (`req.params`, `req.query`, `res.status().json()`), `stream.Transform` + `stream/promises` `pipeline()`, and `http.request` / `http.get` client roundtrips + direct `slice serve` bridge (`export default { fetch }`). |

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

### 3. [`03-node-http-express-compat`](./03-node-http-express-compat/src/index.ts) — `node:http` + `node:stream` Express / Polka Middleware & Pipeline Suite
- **Identical Output Across All 4 Runtimes (`Slice`, `Bun v1.4.4`, `Deno v2.9.8`, `Node.js v27.0.0-alpha`):**
  ```json
  {"framework":"03-node-http-express-compat","totalRequestsDispatched":5,"status200Count":2,"status201Count":2,"status404Count":1,"streamedEventsCount":3,"totalOrderCents":9500,"middlewareHeadersVerified":true,"auditDigest":"eb7f023c67427dc4"}
  ```

---

### 3. Cold CLI Latency (20 Runs) & Warm Isolate Latency (`--warm`) — `SLICEBC\x01` Precompiled `CodeBlock` CAS Default

| Framework Suite | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm Isolate (`--warm`) | Bun `v1.4.4` (Avg) | Deno `v2.9.8` (Avg) | Node.js `v27.0.0-alpha` (Avg) | Slice Speedup vs Bun |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **FW 1: `01-hono-edge-microservice`** (`hono@4.6.20` + `cookie` + `ms`) | **`13.107 ms`** | **`13.478 ms`** | **`6.719 ms` (`0.56 ms` P50)** | `20.227 ms` (`20.933 ms` cold) | `52.293 ms` | `93.408 ms` | **`1.60x faster than Bun` (`-57.4%` via `SLICEBC\x01`)** |
| **FW 2: `02-itty-router-unjs-nitro-stack`** (`itty-router` + `ufo` + `destr` + `ohash` + `radix3`) | **`15.521 ms`** | **`16.177 ms`** | **`9.361 ms` (`0.55 ms` P50)** | `19.550 ms` (`19.585 ms` cold) | `46.454 ms` | `86.203 ms` | **`1.26x faster than Bun` (`-48.2%` via `SLICEBC\x01`)** |

### 4. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Framework Suite | Slice (`SLICEBC\x01` Default) | Bun (`v1.4.4`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha`) | Slice Memory Advantage |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **FW 1: `01-hono-edge-microservice`** (`hono` v4) | **14,336 KB (14.00 MB)** | 30,832 KB (30.11 MB) | 49,388 KB (48.23 MB) | 88,728 KB (86.65 MB) | **6.19x less than Node / 2.15x less than Bun** |
| **FW 2: `02-itty-router-unjs-nitro-stack`** (UnJS / Nitro) | **13,824 KB (13.50 MB)** | 31,868 KB (31.12 MB) | 48,884 KB (47.74 MB) | 87,052 KB (85.01 MB) | **6.30x less than Node / 2.31x less than Bun** |

---

## Running the Framework Benchmark Suite in Docker

```bash
docker build -f tests/Dockerfile -t slice-real-stack-bench .
docker run --rm -e SLICE_BENCH_TAGS=frameworks slice-real-stack-bench
```


