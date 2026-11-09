# Slice (Nexus-JS) — Real-World Framework Test & Benchmark Suite (`tests/frameworks/`)

This directory houses full-stack framework integration tests and cross-runtime benchmarks comparing **Slice**, **Bun**, **Deno**, and **Node.js** on unmodified production Edge and WinterTC frameworks installed from `node_modules/`.

General multi-file application and security benchmarks (`01-fintech-checkout` through `24-tsx-ssr-and-wasm-edge-pipeline`) are located in [`../apps/`](../apps/).

---

## Framework Suites

| ID | Framework Suite | Real External `npm` Packages (`node_modules/`) | Exercised Capabilities |
| :--- | :--- | :--- | :--- |
| **FW 1 (`App 25`)** | [`01-hono-edge-microservice`](./01-hono-edge-microservice) | `hono@4.6.20`, `hono/cookie`, `cookie@0.6.0`, `ms@2.1.3` | Unmodified **Hono v4** `Hono` router, `app.use("*")` security/request-ID middleware, `app.use("/api/v1/tenants/:tenantId/*")` Bearer HMAC-SHA256 tenant guard (`401`), parameterized `GET /api/v1/tenants/:tenantId/orders/:orderId` (`200`), `POST /api/v1/tenants/:tenantId/checkout` (`201`), `getCookie`/`setCookie`, `app.notFound` (`404`), and `slice test` (`tests/hono.test.ts`). |
| **FW 2 (`App 26`)** | [`02-itty-router-unjs-nitro-stack`](./02-itty-router-unjs-nitro-stack) | `itty-router@5.0.18`, `ufo@1.5.4`, `destr@2.0.3`, `ohash@1.1.4`, `radix3@1.1.2` | Unmodified **itty-router v5** (`AutoRouter`) + **UnJS / Nitro** toolkit (`ufo` URL/query normalization, `radix3` route policy matcher, `destr` prototype-pollution-safe JSON parser, `ohash` deterministic object hash & SHA-256 Base64 ETag generator, `If-None-Match` -> `304 Not Modified`), and `slice test` (`tests/unjs.test.ts`). |
| **FW 3 (`App 32`)** | [`03-node-http-express-compat`](./03-node-http-express-compat) | `node:http`, `node:stream`, `node:stream/promises`, `node:crypto` | Express / Polka-compatible `http.createServer` + `app.use()` middleware chain (`req.params`, `req.query`, `res.status().json()`), `stream.Transform` + `stream/promises` `pipeline()`, and `http.request` / `http.get` client roundtrips + direct `slice serve` bridge (`export default { fetch }`). |
| **FW 4 (`App 33`)** | [`04-sveltekit-edge-ssr`](./04-sveltekit-edge-ssr) | `@sveltejs/kit@3.1.1`, `svelte@5.57.3`, `devalue@6.0.3`, `cookie@2.0.2`, `set-cookie-parser@3.2.2`, `clsx@2.1.1` | Full-stack **SvelteKit 3 + Svelte 5** SSR (`$props()`), `hooks.server.js` (`event.locals` + custom response headers), `+page.server.js` `load()` with `Slice.kv` page counters & `Cookie` parsing, parameterized `[slug]` SSR routes, Form Actions (`POST /?/setTheme` with `request.formData()` + `Set-Cookie` + CSRF `Origin` validation), `/__data.json` `devalue` client navigation, `+server.js` JSON API (`GET`/`POST`), `+error.svelte` `404` boundary, and `slice test` (`tests/sveltekit.test.ts`). |

---

## Verified Benchmark & Parity Results (`SLICE_BENCH_TAGS=frameworks`)

### 1. [`01-hono-edge-microservice`](./01-hono-edge-microservice/src/index.ts) — Hono v4 Edge Microservice (`hono@4.6.20`, `hono/cookie`, `cookie@0.6.0`, `ms@2.1.3`)
- **Identical Output Across All 4 Runtimes (`Slice`, `Bun`, `Deno`, `Node.js`):**
  ```json
  {"framework":"01-hono-edge-microservice","libraries":["cookie@0.6.0","hono@4.6.20","ms@2.1.3"],"totalRequestsDispatched":12,"status200Count":7,"status201Count":3,"status401Count":1,"status404Count":1,"cookiesIssued":9,"totalRevenueCents":77470,"totalTaxCents":9994,"middlewareHeadersVerified":true,"healthStatus":"healthy:Hono-v4","auditDigest":"499c81f97f4a1f33"}
  ```
- **Time-Travel `slice test` Runner (`tests/hono.test.ts`):** `2 passed, 0 failed` (`2 VFS+KV rollbacks in 14.89 ms`)

### 2. [`02-itty-router-unjs-nitro-stack`](./02-itty-router-unjs-nitro-stack/src/index.ts) — itty-router v5 + UnJS / Nitro Stack (`itty-router@5.0.18`, `ufo@1.5.4`, `destr@2.0.3`, `ohash@1.1.4`, `radix3@1.1.2`)
- **Identical Output Across All 4 Runtimes (`Slice`, `Bun`, `Deno`, `Node.js`):**
  ```json
  {"framework":"02-itty-router-unjs-nitro-stack","libraries":["destr@2.0.3","itty-router@5.0.18","ohash@1.1.4","radix3@1.1.2","ufo@1.5.4"],"totalRequestsDispatched":12,"status200Count":6,"status202Count":3,"status304Count":2,"status404Count":1,"radixPolicyMatches":11,"normalizedUrlsCount":12,"destrPayloadsParsed":3,"uniqueEtagsGenerated":5,"totalCatalogValueCents":24553135,"totalWebhookAmountCents":7500,"sampleCanonicalSearchUrl":"/v1/search?category=gpu&page=2&limit=25&sort=price_asc","auditDigest":"40f28432a68c05ac"}
  ```
- **Time-Travel `slice test` Runner (`tests/unjs.test.ts`):** `2 passed, 0 failed` (`2 VFS+KV rollbacks in 13.98 ms`)

### 3. [`03-node-http-express-compat`](./03-node-http-express-compat/src/index.ts) — `node:http` + `node:stream` Express / Polka Middleware & Pipeline Suite
- **Identical Output Across All 4 Runtimes (`Slice`, `Bun`, `Deno`, `Node.js`):**
  ```json
  {"framework":"03-node-http-express-compat","totalRequestsDispatched":3,"health":{"status":"healthy:Node-HTTP-Express-Compat","hasMethods":true},"poweredBy":"Slice-Node-Http","echoStatus":201,"echoUpper":"HELLO SLICE STREAM","notFoundStatus":404}
  ```

### 4. [`04-sveltekit-edge-ssr`](./04-sveltekit-edge-ssr/src/index.ts) — SvelteKit 3 + Svelte 5 Full-Stack Edge SSR & Actions Suite (`@sveltejs/kit@3.1.1`, `svelte@5.57.3`, `devalue@6.0.3`)
- **Identical Output Across All 4 Runtimes (`Slice`, `Bun`, `Deno`, `Node.js`):**
  ```json
  {"framework":"04-sveltekit-edge-ssr","libraries":["@sveltejs/kit@3.1.1","clsx@2.1.1","cookie@2.0.2","devalue@6.0.3","set-cookie-parser@3.2.2","svelte@5.57.3"],"totalRequestsDispatched":12,"status200Count":10,"status201Count":1,"status404Count":1,"ssrPagesRendered":4,"dynamicSlugPagesRendered":2,"formActionsExecuted":2,"clientDataLoads":1,"cookiesIssued":2,"hooksHeadersVerified":true,"auditDigest":"1d85e85c9c954480"}
  ```
- **Time-Travel `slice test` Runner (`tests/sveltekit.test.ts`):** `2 passed, 0 failed` (`2 VFS+KV rollbacks`)

---

### 5. Cross-Engine Latency Comparison (`SLICEBC\x01` Precompiled `CodeBlock` CAS Default, Linux `x86_64` Docker)

| Framework Suite | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm Isolate (`--warm`) | Bun `v1.4.4` (Avg) | Deno `v2.9.8` (Avg) | Node.js `v27.0.0-alpha` (Avg) | Slice Speedup vs Bun / Node |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **FW 1: `01-hono-edge-microservice`** (`hono@4.6.20` + `cookie` + `ms`) | **`15.83 ms`** (`14.57 ms` Min) | **`16.53 ms`** | **`5.72 ms`** (`0.48 ms`/req) | `21.21 ms` (`78.06 ms` Win) | `54.81 ms` | `97.92 ms` | **`1.34x` Cold (`3.71x` Warm) vs Bun / `6.19x` vs Node** |
| **FW 2: `02-itty-router-unjs-nitro-stack`** (`itty-router` + `ufo` + `destr` + `ohash` + `radix3`) | **`17.58 ms`** (`16.36 ms` Min) | **`19.16 ms`** | **`9.53 ms`** (`0.79 ms`/req) | `20.23 ms` (`75.51 ms` Win) | `49.25 ms` | `89.38 ms` | **`1.15x` Cold (`2.12x` Warm) vs Bun / `5.08x` vs Node** |
| **FW 3: `03-node-http-express-compat`** (`node:http` + `node:stream` Express/Polka) | **`15.46 ms`** (`14.43 ms` Min) | **`17.07 ms`** | **`1.30 ms`** (`0.43 ms`/req) | `35.67 ms` (`98.20 ms` Win) | `46.51 ms` | `85.62 ms` | **`2.31x` Cold (`27.44x` Warm) vs Bun / `5.54x` vs Node** |
| **FW 4: `04-sveltekit-edge-ssr`** (`@sveltejs/kit@3.1.1` + `svelte@5.57.3` + `devalue@6.0.3`) | **`36.16 ms`** (`33.80 ms` Min) | **`37.80 ms`** | **`17.02 ms`** (`1.42 ms`/req) | `46.58 ms` (`112.82 ms` Win) | `71.05 ms` | `119.71 ms` | **`1.29x` Cold (`2.74x` Warm) vs Bun / `3.31x` vs Node** |

### 6. Peak Process Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Framework Suite | Slice (`SLICEBC\x01` Default) | Bun (`v1.4.4`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha`) | Slice Memory Advantage |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **FW 1: `01-hono-edge-microservice`** (`hono` v4) | **15,104 KB (14.75 MB)** | 30,624 KB (29.91 MB) | 49,008 KB (47.86 MB) | 88,988 KB (86.90 MB) | **5.89x less than Node / 2.03x less than Bun** |
| **FW 2: `02-itty-router-unjs-nitro-stack`** (UnJS / Nitro) | **14,720 KB (14.38 MB)** | 31,616 KB (30.88 MB) | 48,628 KB (47.49 MB) | 86,796 KB (84.76 MB) | **5.90x less than Node / 2.15x less than Bun** |
| **FW 3: `03-node-http-express-compat`** (`Express/Polka node:http`) | **14,976 KB (14.62 MB)** | 33,024 KB (32.25 MB) | 46,564 KB (45.47 MB) | 79,884 KB (78.01 MB) | **5.33x less than Node / 2.21x less than Bun** |
| **FW 4: `04-sveltekit-edge-ssr`** (`SvelteKit 3 + Svelte 5`) | **20,992 KB (20.50 MB)** | 41,984 KB (41.00 MB) | 56,580 KB (55.25 MB) | 85,412 KB (83.41 MB) | **4.07x less than Node / 2.00x less than Bun** |

---

## Running the Framework Benchmark Suite in Docker

```bash
docker build -f tests/Dockerfile -t slice-real-stack-bench .
docker run --rm -e SLICE_BENCH_TAGS=frameworks slice-real-stack-bench
```


