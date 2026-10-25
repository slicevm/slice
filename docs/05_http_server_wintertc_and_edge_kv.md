# Chapter 05: Multi-Tenant HTTP/1.1 Server, WinterTC Bridge & Sharded Edge KV (`slice serve`)

Implemented in [`crates/slice-cli/src/main.rs`](file:///p:/opensource/slice/crates/slice-cli/src/main.rs#L489-L930) and [`crates/slice-engine/src/lib.rs`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L2020-L2126), `slice serve` is a high-throughput, multi-tenant edge HTTP/1.1 server that executes **every incoming HTTP request inside an isolated, fuel-metered `SliceIsolate`** while outperforming single-process `Bun.serve`, `Deno.serve`, and `Node.js http.createServer`.

---

## 1. Architecture of `slice serve` (`152,376 req/s` Tier-0 / `107,274 req/s` Tier-1)

1. **Shared-Nothing Thread-Per-Core Pool (`--workers=N`):**
   - Spawns `N` worker threads (default: `clamp(available_parallelism, 1, 8)`). Each worker pre-warms its own thread-local `BOA_CONTEXT_POOL` and `__slice_rc` bundle closure (`SliceIsolate::prewarm_boa_bundle`) and pre-allocates a reusable `64 KB` socket read buffer (`read_buf`) and `4 KB` write buffer (`write_buf`).
2. **Zero-Allocation Stack HTTP/1.1 Parser (`find_http_header_end`):**
   - Scans `read_buf[..filled]` for `\r\n\r\n`, splits `METHOD /path?query HTTP/1.1` in place as `&str` slices over the stack buffer, and stores up to 16 headers in a stack array `[(&str, &str); 16]` with **0 heap `String` allocations**.
3. **In-Place Isolate Reset ([`SliceIsolate::reset_for_request`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L1908-L1934)):**
   - Resets `fuel_consumed = 0`, `memory_used_bytes = 0`, and enters a fresh `HardwareProtectionDomain` (`WRPKRU`) in nanoseconds without reallocating `CapabilityToken` vectors.
4. **Vectorized Pipelined Flushing:**
   - When multiple pipelined HTTP/1.1 requests are buffered on a persistent `Keep-Alive` connection (`has_more_pipelined`), responses are appended to `write_buf` and flushed in a single `stream.write_all(write_buf)` syscall.

---

## 2. WinterTC Web Standards (`export default { fetch }`, `Request`, `Response`, `Headers`, `URL`, `URLSearchParams`) & Native `Slice` API

Slice supports **both** the standard WinterTC / Cloudflare Workers / Bun / Deno `export default { fetch(req, env, ctx) }` handler contract **and** the low-level `Slice.request()` / `Slice.respond()` API:

### Standard WinterTC Web APIs (Phase F1)
- **`export default { fetch(req, env, ctx) }`**:
  - Top-level module state is initialized **once** per worker realm (including during `prewarm_boa_bundle`) and cached in the `__slice_rc` closure.
  - Every incoming HTTP request invokes `handler.fetch(req, env, ctx)` with:
    - `req: Request`: Zero-copy `Request` instance backed by a lazy live `Headers` view (`__SLICE_LIVE_REQ__`) over the worker's stack HTTP buffer (`req.headers.get(name)` reads directly from Rust without allocating a JS header map!).
    - `env`: `globalThis.process.env`.
    - `ctx`: `{ waitUntil(fn), passThroughOnException() }`.
  - Supports both synchronous `fetch(req): Response` (zero Promise allocation overhead) and `async fetch(req): Promise<Response>`.
- **`URL` & `URLSearchParams`** (also available via `import { URL, URLSearchParams } from "node:url"`):
  - Full relative URL resolution (`new URL("/v2/items/../orders?tag=gpu#top", "https://api.slice.dev")`), path dot-segment normalization, default port stripping (`:80` / `:443`), and live two-way `url.searchParams` synchronization (`.get()`, `.getAll()`, `.has()`, `.set()`, `.append()`, `.delete()`, `.sort()`, `.entries()`, `.size`, `Symbol.iterator`).
- **`Headers`**:
  - Case-insensitive HTTP header map supporting object, `[key, value][]`, or `Headers` initialization, `.get()`, `.set()`, `.append()`, `.has()`, `.delete()`, `.entries()`, `.forEach()`, and `Symbol.iterator`.
- **`Request` & `Response`**:
  - `new Request(input, init)` and `new Response(body, init)` with `.status`, `.statusText`, `.ok`, `.headers`, `.json()`, `.text()`, `.jsonSync()`, `.textSync()`, `.clone()`, `Response.json(data, init)`, and `Response.redirect(url, status)`.

### Runnable Standard `export default { fetch }` Example

Create `wintertc_worker.ts`:
```typescript
export default {
    fetch(req: Request, env: Record<string, string>, ctx: { waitUntil(fn: () => void): void }): Response {
        const url = new URL(req.url);
        const tenant = req.headers.get("x-slice-tenant") || "default";
        const hits = Slice.kv.incr("worker:hits", 1);

        ctx.waitUntil(() => {
            Slice.kv.set("worker:last_path", url.pathname, 60_000);
        });

        if (url.pathname === "/legacy") {
            return Response.redirect(new URL("/v2/catalog?migrated=1", url.origin).href, 308);
        }

        return Response.json(
            {
                ok: true,
                method: req.method,
                path: url.pathname,
                category: url.searchParams.get("category") || "all",
                tenant,
                hits,
                nodeEnv: env.NODE_ENV
            },
            { status: 200, headers: { "x-powered-by": "Slice-NexusJS" } }
        );
    }
};
```

### Low-Level Zero-Allocation `Slice.request()` & `Slice.respond()` API

Inside any TypeScript/JavaScript handler served by `slice serve`:

- **`Slice.request()`**: Returns the current HTTP request object:
  - `req.method`: `"GET"`, `"POST"`, etc.
  - `req.path`: Normalized URL path (e.g., `"/api/v1/checkout"`).
  - `req.query`: Raw query string after `?` (e.g., `"sku=edge&qty=2"`).
  - `req.tenant` / `req.tenantId`: Active tenant ID resolved from the `X-Slice-Tenant` header or `/t/<tenant_id>/...` URL prefix.
  - `req.body`: UTF-8 request body string (`Content-Length` framed).
  - `req.header(name)`: Case-insensitive lookup of any HTTP request header (e.g., `req.header("authorization")`).
- **`Slice.respond(status, body, contentType?)`**: Sets the HTTP status code (`200`, `201`, `400`, `401`, `403`, `404`, `429`, `500`), response body (string or JSON-serializable object), and `Content-Type` (defaults to `"application/json"`).
- **`Slice.waitUntil(callback)`**: Queues a callback function to execute **after** the HTTP response bytes have been flushed to the client socket (`drain_wait_until_tasks`), while still metering against the tenant's remaining CPU fuel budget.

---

## 3. 16-Way Sharded In-Memory Edge KV & Atomic Counter Slab (`Slice.kv`)

Implemented in [`EDGE_KV_SLAB`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L3381-L3470), `Slice.kv` provides an in-process, 16-way sharded key-value store and atomic 64-bit integer counter slab (`0.57 us` per operation) that is **strictly partitioned by `tenant_id`** and **charged against the tenant's memory cage**:

- **`Slice.kv.get(key: string): string | null`**: Reads a key from the active tenant's KV namespace (automatically evicting expired keys).
- **`Slice.kv.set(key: string, value: string | object, ttlMs?: number): boolean`**: Writes a key-value pair with optional TTL in milliseconds (`0` = no expiry). Charges `key.length + value.length` against the tenant's memory limit.
- **`Slice.kv.del(key: string): boolean`**: Deletes a key from the active tenant's KV namespace.
- **`Slice.kv.incr(key: string, delta?: number): number`**: Atomically increments a 64-bit integer counter (`default delta = 1`) and returns the new value.

### Runnable WinterTC + `Slice.kv` Rate-Limited Gateway Example

Create `edge_gateway.ts`:
```typescript
class EdgeRateLimitedGateway {
    public serve(): void {
        const req = Slice.request();
        const hits = Slice.kv.incr("rate:window", 1);

        if (hits > 100) {
            Slice.respond(429, { error: "Too Many Requests", tenant: req.tenant, hits }, "application/json");
            return;
        }

        Slice.kv.set("last:SeenPath", req.path, 60_000);

        Slice.waitUntil(() => {
            Slice.kv.incr("audit:flushed_responses", 1);
        });

        Slice.respond(
            200,
            {
                ok: true,
                method: req.method,
                path: req.path,
                query: req.query,
                tenant: req.tenant,
                hits,
                userAgent: req.header("user-agent")
            },
            "application/json"
        );
    }
}

new EdgeRateLimitedGateway().serve();
```

Pack and test with `--smoke-test` or run live:
```bash
slice pack edge_gateway.ts edge_gateway.cas
slice serve edge_gateway.cas --smoke-test
```

---

## 4. Multi-Tenant Dynamic `.cas` Router & Zero-Downtime Hot-Swap (`--tenant-dir`)

When `slice serve --tenant-dir=./tenants` is passed (implemented in [`resolve_tenant_bytecode_hotswap`](file:///p:/opensource/slice/crates/slice-cli/src/main.rs#L663-L716)):
1. Incoming requests with header `X-Slice-Tenant: tenant-alpha` or URL `/t/tenant-alpha/orders` automatically load `./tenants/tenant-alpha.cas` (or fall back to `./tenants/default.cas`).
2. Each worker caches `(mtime_ns, file_len, Arc<Vec<u8>>)` in thread-local `TENANT_CAS_CACHE`.
3. When you deploy a new version by overwriting `./tenants/tenant-alpha.cas`, the very next request automatically detects the updated `mtime_ns`/`file_len`, memory-maps the new `.cas` archive, and pre-warms its `__slice_rc` closure with **zero dropped connections and zero server restarts**.

---

## 5. Opt-In Deterministic Replay Mode (`--deterministic=<seed>`)

Pass `--deterministic=<seed>` to `slice run` or `slice serve` to make `Math.random()`, `Date.now()`, and `performance.now()` 100% deterministic across runs using a **SplitMix64** PRNG and monotonic virtual clock (`1700000000000 + tick_ms`), with `0.00 ns` overhead when disabled:

```bash
slice run ./my_script.ts --deterministic=20261019
```

---

## 6. Zero-Stat CAS Static Asset Server (`--static`) & SSE Token Streaming (`Slice.streamSSE` / `Response.sse`)

### 6.1 Embedding & Serving Static Web Assets with Cryptographic BLAKE3 `ETag` (`Phase F2`)
Implemented in [`StaticAssetTable`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs#L1314-L1416) and [`serve_connection_keep_alive`](file:///p:/opensource/slice/crates/slice-cli/src/main.rs#L940-L993):
- Pass `--static=./public` to **`slice pack`**, **`slice compile`**, or **`slice serve`**:
  ```bash
  # Pack your TypeScript API + entire ./public frontend directory into one .cas archive
  slice pack ./src/index.ts ./fullstack.cas --static=./public

  # Or compile your Frontend UI + Backend API into a single zero-dependency executable
  slice compile ./src/index.ts -o ./my-webapp --static=./public

  # Serve with zero-stat mmap static asset caching (< 2 us per hit, 0 JS VM spinup)
  slice serve ./fullstack.cas --port=3000 --workers=8
  ```
- Every static file (`.html`, `.css`, `.js`, `.json`, `.svg`, `.wasm`, `.png`, `.ico`, `.woff2`, etc.) is indexed in O(1) by URL path (including directory index aliases `/` -> `/index.html` and `/docs/` -> `/docs/index.html`) with a pre-computed **16-hex-character BLAKE3 `ETag`**.
- When a browser sends `If-None-Match: "<blake3_16>"`, `slice serve` responds with `HTTP/1.1 304 Not Modified` (`X-Slice-Static: HIT-304`, `X-Slice-Isolate-Us: 0`) in **`< 2 microseconds`** without spinning up a JS isolate or touching the disk.

### 6.2 Server-Sent Events (SSE) & `ReadableStream` with Outbound Egress DLP (`Slice.streamSSE` / `Response.sse`)
- Format and stream WHATWG Server-Sent Events (`id:`, `event:`, `retry:`, multi-line `data:`) using `Response.sse(eventsOrCallback, init?)`, `Slice.streamSSE(eventsOrCallback, init?)`, or `new ReadableStream({ start(controller) { ... } })`:
  ```typescript
  export default {
      fetch(req: Request): Response {
          const url = new URL(req.url);
          if (url.pathname === "/api/v1/chat/stream") {
              return Response.sse((emit: (ev: any) => void) => {
                  emit({ id: "1", event: "token", data: { delta: "Hello " }, retry: 1500 });
                  emit({ id: "2", event: "token", data: { delta: "from Slice Edge!" } });
                  emit({ id: "3", event: "done", data: "[DONE]" });
              });
          }
          return Response.json({ status: "ok" });
      }
  };
  ```
- Every SSE response is automatically scanned by `--audit-egress` (`verify_egress_secrets_slice`), blocking raw, Base64, or Base64URL secret exfiltration inside AI token streams before any bytes leave the host.

---

## 7. OCap-Guarded Outbound `fetch()`, Cloud Metadata SSRF Shield, Outbound Egress DLP & Deterministic Cassette Replay (`Phase F3`)

Implemented in [`crates/slice-engine/src/lib.rs`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs) (`check_boa_outbound_fetch_ocap_and_dlp`, `execute_outbound_http11`, `__slice_fetch_sync`) and [`crates/slice-sandbox/src/lib.rs`](file:///p:/opensource/slice/crates/slice-sandbox/src/lib.rs#L162-L231):

### 7.1 Standard `await fetch(url, init)` & `Slice.fetchSync(url, init)`
- Supports `GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `HEAD`, custom `Headers`, request bodies, chunked transfer decoding, and standard `Response` methods (`await res.json()`, `await res.text()`, `res.jsonSync()`, `res.textSync()`, `res.status`, `res.ok`, `res.headers.get(...)`).
- By default (**Zero Ambient Authority**), any outbound `fetch()` call without an explicit `allow_net` capability traps immediately with `SandboxViolation::NetAccessDenied`.
- Grant outbound network capabilities via `slice.toml` (`allow_net = ["api.stripe.com:443", "127.0.0.1:18095"]` — automatically embedded into `.cas` archives and standalone `SLICEEXE` binaries under `manifest:/slice.toml`), CLI `--allow-net=host1,host2`, or `Slice.evalSandbox(code, { allowNet: [...] })`.

### 7.2 Built-In Cloud Metadata & Private CIDR SSRF Shield
- **Unconditional Cloud Metadata Protection**: Even when wildcard `allow_net = ["*"]` (`--allow-net=*`) is granted, Slice unconditionally blocks Server-Side Request Forgery (SSRF) against cloud instance metadata endpoints (`169.254.169.254`, `169.254.170.2`, `100.100.100.200`, `fd00:ec2::254`, `metadata.google.internal`, `metadata.goog`).
- **Loopback & RFC1918 Private Network Protection**: Under wildcard `allow_net = ["*"]`, private and loopback networks (`localhost`, `127.0.0.0/8`, `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`) are blocked unless that exact `host:port` is explicitly allowlisted (e.g., `--allow-net=127.0.0.1:18095`).

### 7.3 Outbound Egress Secret DLP (`--audit-egress` on `fetch()`)
- Before any outbound `fetch(url, { headers, body })` packet is transmitted over TCP, Slice scans the URL query string, outbound request headers, and request body for any secret registered via `--audit-egress=<SECRET>` (in raw plaintext, standard Base64, or URL-safe Base64URL).
- If a secret is detected, Slice aborts the socket connection before a single packet leaves the machine and traps with `SandboxViolation::EnvAccessDenied { key: "EGRESS_SECRET_LEAK_BLOCKED" }`.

### 7.4 Deterministic `fetch()` Cassette Recording & Replay (`--deterministic=<seed>` & `Slice.mockFetch`)
- When `--deterministic=<seed>` is active, live outbound `fetch()` responses are automatically recorded into the tenant's isolated `Slice.kv` partition under `fetch_cassette:<METHOD>:<URL>` and replayed in `< 1 us` on subsequent runs.
- Unit tests can also register in-memory responses directly via `Slice.mockFetch(url, { status: 200, headers: { ... }, body: { ... } })`.



