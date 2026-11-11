# Slice (Nexus-JS) - Real-World Framework Test & Benchmark Suite (`tests/frameworks/`)

This directory houses full-stack framework integration tests and cross-runtime benchmarks comparing **Slice**, **Bun**, **Deno**, and **Node.js** on unmodified production Edge and WinterTC frameworks installed from `node_modules/`.

General multi-file application and security benchmarks (`01-fintech-checkout` through `24-tsx-ssr-and-wasm-edge-pipeline`) are located in [`../apps/`](../apps/). For the complete suite overview and synced summary tables, see **[`tests/README.md`](../README.md)** and **[`tests/reports/latest.md`](../reports/latest.md)**.

---

## Framework Suites

| ID | Framework Suite | Real External `npm` Packages (`node_modules/`) | Exercised Capabilities |
| :--- | :--- | :--- | :--- |
| **FW 1 (`App 25`)** | [`01-hono-edge-microservice`](./01-hono-edge-microservice) | `hono@4.6.20`, `hono/cookie`, `cookie@0.6.0`, `ms@2.1.3` | Unmodified **Hono v4** `Hono` router, `app.use("*")` security/request-ID middleware, `app.use("/api/v1/tenants/:tenantId/*")` Bearer HMAC-SHA256 tenant guard (`401`), parameterized `GET /api/v1/tenants/:tenantId/orders/:orderId` (`200`), `POST /api/v1/tenants/:tenantId/checkout` (`201`), `getCookie`/`setCookie`, `app.notFound` (`404`), and `slice test` ([`tests/hono.test.ts`](./01-hono-edge-microservice/tests/hono.test.ts)). |
| **FW 2 (`App 26`)** | [`02-itty-router-unjs-nitro-stack`](./02-itty-router-unjs-nitro-stack) | `itty-router@5.0.18`, `ufo@1.5.4`, `destr@2.0.3`, `ohash@1.1.4`, `radix3@1.1.2` | Unmodified **itty-router v5** (`AutoRouter`) + **UnJS / Nitro** toolkit (`ufo` URL/query normalization, `radix3` route policy matcher, `destr` prototype-pollution-safe JSON parser, `ohash` deterministic object hash & SHA-256 Base64 ETag generator, `If-None-Match` -> `304 Not Modified`), and `slice test` ([`tests/unjs.test.ts`](./02-itty-router-unjs-nitro-stack/tests/unjs.test.ts)). |
| **FW 3 (`App 32`)** | [`03-node-http-express-compat`](./03-node-http-express-compat) | `node:http`, `node:stream`, `node:stream/promises`, `node:crypto` | Express / Polka-compatible `http.createServer` + `app.use()` middleware chain (`req.params`, `req.query`, `res.status().json()`), `stream.Transform` + `stream/promises` `pipeline()`, and `http.request` / `http.get` client roundtrips + direct `slice serve` bridge (`export default { fetch }`). |
| **FW 4 (`App 33`)** | [`04-sveltekit-edge-ssr`](./04-sveltekit-edge-ssr) | `@sveltejs/kit@3.1.1`, `svelte@5.57.3`, `devalue@6.0.3`, `cookie@2.0.2`, `set-cookie-parser@3.2.2`, `clsx@2.1.1` | Full-stack **SvelteKit 3 + Svelte 5** SSR (`$props()`), `hooks.server.js` (`event.locals` + custom response headers), `+page.server.js` `load()` with `Slice.kv` page counters & `Cookie` parsing, parameterized `[slug]` SSR routes, Form Actions (`POST /?/setTheme` with `request.formData()` + `Set-Cookie` + CSRF `Origin` validation), `/__data.json` `devalue` client navigation, `+server.js` JSON API (`GET`/`POST`), `+error.svelte` `404` boundary, and `slice test` ([`tests/sveltekit.test.ts`](./04-sveltekit-edge-ssr/tests/sveltekit.test.ts)). |

---

## Verified Cross-Runtime Parity (`SLICE_BENCH_TAGS=frameworks`)

### 1. [`01-hono-edge-microservice`](./01-hono-edge-microservice/src/index.ts) - Hono v4 Edge Microservice (`hono@4.6.20`, `hono/cookie`, `cookie@0.6.0`, `ms@2.1.3`)
- **Identical Output Across All 4 Runtimes (`Slice`, `Bun`, `Deno`, `Node.js`):**
  ```json
  {"framework":"01-hono-edge-microservice","libraries":["cookie@0.6.0","hono@4.6.20","ms@2.1.3"],"totalRequestsDispatched":12,"status200Count":7,"status201Count":3,"status401Count":1,"status404Count":1,"cookiesIssued":9,"totalRevenueCents":77470,"totalTaxCents":9994,"middlewareHeadersVerified":true,"healthStatus":"healthy:Hono-v4","auditDigest":"499c81f97f4a1f33"}
  ```
- **Time-Travel `slice test` Runner ([`tests/hono.test.ts`](./01-hono-edge-microservice/tests/hono.test.ts)):** `2 passed, 0 failed` (`2 VFS+KV rollbacks`)

### 2. [`02-itty-router-unjs-nitro-stack`](./02-itty-router-unjs-nitro-stack/src/index.ts) - itty-router v5 + UnJS / Nitro Stack (`itty-router@5.0.18`, `ufo@1.5.4`, `destr@2.0.3`, `ohash@1.1.4`, `radix3@1.1.2`)
- **Identical Output Across All 4 Runtimes (`Slice`, `Bun`, `Deno`, `Node.js`):**
  ```json
  {"framework":"02-itty-router-unjs-nitro-stack","libraries":["destr@2.0.3","itty-router@5.0.18","ohash@1.1.4","radix3@1.1.2","ufo@1.5.4"],"totalRequestsDispatched":12,"status200Count":6,"status202Count":3,"status304Count":2,"status404Count":1,"radixPolicyMatches":11,"normalizedUrlsCount":12,"destrPayloadsParsed":3,"uniqueEtagsGenerated":5,"totalCatalogValueCents":24553135,"totalWebhookAmountCents":7500,"sampleCanonicalSearchUrl":"/v1/search?category=gpu&page=2&limit=25&sort=price_asc","auditDigest":"40f28432a68c05ac"}
  ```
- **Time-Travel `slice test` Runner ([`tests/unjs.test.ts`](./02-itty-router-unjs-nitro-stack/tests/unjs.test.ts)):** `2 passed, 0 failed` (`2 VFS+KV rollbacks`)

### 3. [`03-node-http-express-compat`](./03-node-http-express-compat/src/index.ts) - `node:http` + `node:stream` Express / Polka Middleware & Pipeline Suite
- **Identical Output Across All 4 Runtimes (`Slice`, `Bun`, `Deno`, `Node.js`):**
  ```json
  {"framework":"03-node-http-express-compat","totalRequestsDispatched":3,"health":{"status":"healthy:Node-HTTP-Express-Compat","hasMethods":true},"poweredBy":"Slice-Node-Http","echoStatus":201,"echoUpper":"HELLO SLICE STREAM","notFoundStatus":404}
  ```

### 4. [`04-sveltekit-edge-ssr`](./04-sveltekit-edge-ssr/src/index.ts) - SvelteKit 3 + Svelte 5 Full-Stack Edge SSR & Actions Suite (`@sveltejs/kit@3.1.1`, `svelte@5.57.3`, `devalue@6.0.3`)
- **Identical Output Across All 4 Runtimes (`Slice`, `Bun`, `Deno`, `Node.js`):**
  ```json
  {"framework":"04-sveltekit-edge-ssr","libraries":["@sveltejs/kit@3.1.1","clsx@2.1.1","cookie@2.0.2","devalue@6.0.3","set-cookie-parser@3.2.2","svelte@5.57.3"],"totalRequestsDispatched":12,"status200Count":10,"status201Count":1,"status404Count":1,"ssrPagesRendered":4,"dynamicSlugPagesRendered":2,"formActionsExecuted":2,"clientDataLoads":1,"cookiesIssued":2,"hooksHeadersVerified":true,"auditDigest":"1d85e85c9c954480"}
  ```
- **Time-Travel `slice test` Runner ([`tests/sveltekit.test.ts`](./04-sveltekit-edge-ssr/tests/sveltekit.test.ts)):** `2 passed, 0 failed` (`2 VFS+KV rollbacks`)

---

## Latest Verified Benchmark Results

All cross-engine latency (`cold .cas`, standalone binary, and `--warm` pooled execution) and peak process memory (`Max RSS`) measurements for `FW 1` through `FW 4` are maintained automatically in **[`tests/reports/latest.md`](../reports/latest.md)** (summary in **[`tests/README.md`](../README.md)**).

---

## Running the Framework Benchmark Suite in Docker

```bash
docker build -f tests/Dockerfile -t slice-real-stack-bench .
docker run --rm -e SLICE_BENCH_TAGS=frameworks slice-real-stack-bench
```
