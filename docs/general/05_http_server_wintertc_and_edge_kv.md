# Chapter 05: Building Full-Stack Web Apps, APIs, Static Sites & SSE Streams (`slice serve`)

`slice serve` is a high-throughput, multi-tenant HTTP/1.1 `Keep-Alive` and pipelined web server that executes **every incoming HTTP request inside an isolated, fuel-metered `SliceIsolate`** - sustaining **`> 145k-154k req/s`** across Tier-1 WinterTC and Tier-0 CAS modes and outperforming `Deno.serve`, `Bun.serve`, and `Node.js http.createServer` (see [`tests/reports/latest.md`](../../tests/reports/latest.md) for live benchmark numbers).

This chapter is a hands-on guide to building full-stack web applications, REST/BFF APIs, static frontend CDNs, Server-Sent Events (SSE) token streams, outbound `fetch()` webhooks, and multi-tenant hot-swapped SaaS platforms with `slice serve`.

---

## 1. Writing Standard WinterTC Web Handlers (`export default { fetch }`)

Slice implements the industry-standard **WinterTC / WHATWG Fetch Server contract** (`export default { fetch(req, env, ctx) }`) used by Cloudflare Workers, Bun, Deno, and Hono.

### 1.1 Complete REST API & Router Recipe

Create `server.ts`:
```typescript
import * as crypto from "node:crypto";

export default {
    async fetch(
        req: Request,
        env: Record<string, string>,
        ctx: { waitUntil(fn: () => void): void; passThroughOnException(): void }
    ): Promise<Response> {
        const url = new URL(req.url);
        const tenant = req.headers.get("x-slice-tenant") || "public-tenant";

        // 1. Per-tenant atomic rate limiting in < 0.6 us via Slice.kv
        const hits = Slice.kv.incr("rate:minute", 1);
        if (hits > 1000) {
            return Response.json({ error: "Rate limit exceeded", tenant, hits }, { status: 429 });
        }

        // 2. Permanent 308 redirect for legacy routes
        if (url.pathname === "/v1/items") {
            return Response.redirect(new URL("/api/v2/items" + url.search, url.origin).href, 308);
        }

        // 3. GET /api/v2/items?category=gpu&sort=asc
        if (req.method === "GET" && url.pathname === "/api/v2/items") {
            const category = url.searchParams.get("category") || "all";
            url.searchParams.sort(); // Canonicalize query params

            return Response.json(
                {
                    ok: true,
                    tenant,
                    category,
                    canonicalQuery: url.searchParams.toString(),
                    hits,
                    stage: env.NODE_ENV,
                },
                {
                    status: 200,
                    headers: {
                        "cache-control": "public, max-age=60",
                        "x-powered-by": "Slice-NexusJS",
                    },
                }
            );
        }

        // 4. POST /api/v2/orders with JSON body & background waitUntil task
        if (req.method === "POST" && url.pathname === "/api/v2/orders") {
            const body: any = await req.json();
            const receiptId = crypto
                .createHash("sha256")
                .update(`${tenant}:${body.sku}:${body.qty}`)
                .digest("hex")
                .slice(0, 16);

            // Schedule post-response background work (runs AFTER HTTP response is flushed to client!)
            ctx.waitUntil(() => {
                Slice.kv.set(`order:${receiptId}`, JSON.stringify(body), 3600_000);
            });

            return Response.json({ created: true, receiptId, tenant }, { status: 201 });
        }

        return Response.json({ error: "Not Found", path: url.pathname }, { status: 404 });
    },
};
```

### 1.2 Running and Smoke-Testing Your Server
```bash
# Quick self-contained smoke test (binds ephemeral port, sends 2 pipelined requests, prints output & exits)
slice serve ./server.ts --smoke-test

# Start production server on port 3000 with 8 worker threads
slice serve ./server.ts --port=3000 --workers=8
```

### 1.3 Built-In HTTP Response Telemetry Headers
Every response from `slice serve` includes real-time microVM execution headers so you can monitor per-request isolate latency and CPU fuel usage in production:
```http
HTTP/1.1 200 OK
Content-Type: application/json
Content-Length: 118
Connection: keep-alive
Server: Slice-NexusJS/0.2
X-Slice-Tenant: tenant-alpha
X-Slice-Isolate-Us: 8
X-Slice-Total-Us: 10
X-Slice-Fuel-Used: 120
```

---

## 2. WinterTC Web API Reference (`Request`, `Response`, `Headers`, `URL`, `URLPattern`, `WebCrypto`, `Streams`, `Intl`, `Timers`)

| Web API Class / Global | Supported Constructors, Properties & Methods |
| :--- | :--- |
| **`URL` & `URLPattern`** | `new URL(input, base?)` (`URL.canParse`, `URL.createObjectURL`, `URL.revokeObjectURL`, `.href`, `.origin`, `.protocol`, `.username`, `.password`, `.host`, `.hostname`, `.port`, `.pathname`, `.search`, `.hash`, `.searchParams`, `.toString()`, `.toJSON()`) and **`new URLPattern(input, baseURL?, opts?)`** (`.test()`, `.exec()`, named `:param` groups, wildcards). |
| **`URLSearchParams`** | `new URLSearchParams(init?)` - `.get(k)`, `.getAll(k)`, `.has(k)`, `.set(k, v)`, `.append(k, v)`, `.delete(k)`, `.sort()`, `.forEach(cb)`, `.entries()`, `.keys()`, `.values()`, `.toString()`, `.size`, `for (const [k, v] of params)`. Live two-way synced with `URL`. |
| **`Headers`** | `new Headers(init?)` - Case-insensitive map supporting `.get(k)`, `.set(k, v)`, `.append(k, v)`, `.has(k)`, `.delete(k)`, **`getSetCookie()`**, `.entries()`, `.keys()`, `.values()`, `.forEach(cb)`, `for (const [k, v] of headers)`. Incoming `req.headers` uses a **zero-copy lazy view** (`__SLICE_LIVE_REQ__`) over the Rust socket buffer! |
| **`Request`** | `new Request(input, init?)` - `.method`, `.url`, `.headers`, `.signal`, `.body`, `.bodyUsed`, `await req.text()`, `await req.json()`, **`await req.arrayBuffer()`**, **`await req.bytes()`**, **`await req.blob()`**, **`await req.formData()`**, `req.textSync()`, `req.jsonSync()`, `req.clone()`. |
| **`Response`** | `new Response(body?, init?)` - `.status`, `.statusText`, `.ok`, `.redirected`, `.url`, `.headers`, `.body`, `.bodyUsed`, `await res.text()`, `await res.json()`, **`await res.arrayBuffer()`**, **`await res.bytes()`**, **`await res.blob()`**, **`await res.formData()`**, `res.textSync()`, `res.jsonSync()`, `res.clone()`, **`Response.json(data, init?)`**, **`Response.redirect(url, status?)`**, **`Response.sse(source, init?)`**. |
| **`crypto` & `crypto.subtle`** | `crypto.randomUUID()`, `crypto.getRandomValues(typedArray)`, and native Rust `SubtleCrypto`: `digest("SHA-256"\|"SHA-384"\|"SHA-512"\|"SHA-1")`, `sign`/`verify` (`HMAC`, `Ed25519`, `ECDSA` `P-256`/`P-384`, imported `RSASSA-PKCS1-v1_5` & `RSA-PSS`), `encrypt`/`decrypt` (`AES-GCM` & `AES-CBC` 128/256-bit), `deriveBits`/`deriveKey` (`PBKDF2`, `HKDF`, RFC 7748 `X25519`), `generateKey` (`AES-GCM`, `AES-CBC`, `HMAC`, `Ed25519`, `X25519`, `ECDSA`), `importKey`, `exportKey` (`"raw"`, `"jwk"`, `"pkcs8"`, `"spki"`). |
| **`ReadableStream` / `WritableStream` / `TransformStream`** | Full WHATWG Streams pipeline with `.getReader()`, `.getWriter()`, `.pipeThrough(transform)`, `.pipeTo(dest)`, `.tee()`, `for await (const chunk of stream)`, `TextEncoderStream`, and `TextDecoderStream`. |
| **`CompressionStream` & `DecompressionStream`** | Pure-Rust zero-dependency RFC 1950/1951/1952 streaming compression and decompression supporting `"gzip"`, `"deflate"`, and `"deflate-raw"` via `.pipeThrough()`. |
| **`TextEncoder` & `TextDecoder`** | `new TextEncoder()` (`.encode(str)`, `.encodeInto(str, u8)`), `new TextDecoder(label?, opts?)` (`.decode(buf)`). |
| **`Blob`, `File`, & `FormData`** | `new Blob(parts, opts)` (`.size`, `.type`, `.text()`, `.arrayBuffer()`, `.bytes()`, `.slice()`, `.stream()`), `new File(parts, name, opts)`, `new FormData()` (`.append()`, `.set()`, `.get()`, `.getAll()`, `multipart/form-data` serialization & parsing). |
| **`AbortController`, `AbortSignal` & `DOMException`** | `new AbortController()`, `AbortSignal.abort(reason?)`, `AbortSignal.timeout(ms)`, `AbortSignal.any(signals)`, `.throwIfAborted()`, and standard **`DOMException`** (`name`, `message`, `code`). Integrated with `fetch(url, { signal })` and `ReadableStream.pipeTo`. |
| **`Intl`, `navigator`, & `location`** | Deterministic zero-ICU **`Intl`** (`DateTimeFormat`, `NumberFormat`, `Collator`, `PluralRules`, `Segmenter`, `getCanonicalLocales`, `supportedValuesOf`), plus WinterTC `navigator` and `location` globals. |
| **`setTimeout` / `setInterval` / `queueMicrotask`** | $O(1)$ slab timer wheel (`setTimeout`, `clearTimeout`, `setInterval`, `clearInterval`, `setImmediate`, `clearImmediate`, `queueMicrotask`) with **`0 ns` deterministic virtual time-warp** when `--deterministic` is active. |
| **`EventTarget`, `Event`, `CustomEvent`, `structuredClone`** | Standard DOM-compatible `EventTarget` (`addEventListener`, `removeEventListener`, `dispatchEvent`), `CustomEvent`, `structuredClone`, `atob`, and `btoa`. |

> **Performance Tip (`jsonSync` & Synchronous `fetch`):**
> In `export default { fetch(req) }`, you can return a `Response` synchronously (without `async`) and call `req.jsonSync()` or `req.textSync()` to avoid allocating JavaScript `Promise` microtasks on the hot path!

---

## 3. Ultra-Low-Overhead Native Bridge (`Slice.request()` & `Slice.respond()`)

If you want maximum raw speed without constructing `new URL()` or `new Response()` objects, you can also use Slice's direct synchronous bridge inside any script served by `slice serve`:

```typescript
const req = Slice.request();
// req.method   -> "GET" | "POST" | ...
// req.path     -> "/api/v1/health"
// req.query    -> "region=us-east-1"
// req.tenant   -> "tenant-alpha"
// req.body     -> raw UTF-8 request body
// req.header("authorization") -> case-insensitive header lookup

Slice.waitUntil(() => {
    Slice.kv.incr("total_requests_served", 1);
});

Slice.respond(200, { status: "healthy", tenant: req.tenant }, "application/json");
```

---

## 4. Using the 16-Way Sharded Edge KV Store (`Slice.kv`)

Every `slice` process includes a built-in, lock-free-sharded (`16` shards) in-memory key-value and atomic counter store (`Slice.kv`) that operates in **`< 1 microsecond` per call**.

Crucially, **`Slice.kv` is automatically isolated per tenant**:
- When `Tenant A` (`X-Slice-Tenant: tenant-a`) calls `Slice.kv.set("config", "dark")`, `Tenant B` (`X-Slice-Tenant: tenant-b`) calling `Slice.kv.get("config")` receives `null`.
- Every byte stored in `Slice.kv` by a tenant is charged against that tenant's `memory_kb` ceiling.

### `Slice.kv` API Reference

```typescript
// 1. Store a string or JSON object with an optional TTL in milliseconds (0 or omitted = no expiry)
Slice.kv.set("session:user-42", { role: "admin", tier: "pro" }, 60_000);

// 2. Read a key (returns string | null; automatically evicts expired TTL entries)
const raw = Slice.kv.get("session:user-42");

// 3. Atomically increment/decrement a 64-bit integer counter (default delta = 1)
const count: number = Slice.kv.incr("ratelimit:ip:203.0.113.5", 1);
const remaining: number = Slice.kv.incr("credits:balance", -50);

// 4. Delete a key (returns true if the key existed) or clear all keys for the active tenant
const deleted: boolean = Slice.kv.del("session:user-42");
Slice.kv.clear();
```

---

## 5. Serving Static Frontend Assets (`--static`) with `< 2 us` BLAKE3 `ETag` `304`

You don't need Nginx or a separate CDN container to serve your React, Vite, Vue, SvelteKit, or static HTML/CSS/JS frontend alongside your TypeScript API.

Pass `--static=./public` to `slice serve`, `slice pack`, or `slice compile`:

```bash
# Serve directly during development
slice serve ./src/server.ts --static=./public --port=3000

# Or pack your backend API + ./public frontend into one .cas archive
slice pack ./src/server.ts ./fullstack.cas --static=./public
slice serve ./fullstack.cas --port=3000 --workers=8

# Or compile your entire Frontend + Backend into a single native executable!
slice compile ./src/server.ts -o ./my-fullstack-app --static=./public
```

### How `--static` Works
1. **Build-Time BLAKE3 Indexing:** Every file in `./public` (`.html`, `.css`, `.js`, `.mjs`, `.json`, `.svg`, `.png`, `.jpg`, `.webp`, `.gif`, `.ico`, `.woff2`, `.wasm`, `.txt`, `.xml`) is embedded into the `.cas` archive and indexed by URL path (including automatic directory index aliases: `/` -> `/index.html` and `/docs/` -> `/docs/index.html`) with a pre-computed **16-hex-character BLAKE3 `ETag`**.
2. **Zero-VM Static Hits (`< 2 microseconds`):**
   - When a `GET` or `HEAD` request matches a static asset path, `slice serve` serves the bytes directly from the memory-mapped `.cas` archive (`X-Slice-Static: HIT-200`, `X-Slice-Isolate-Us: 0`) **without spinning up a JavaScript isolate or touching the disk**.
   - When the browser sends `If-None-Match: "<etag>"`, `slice serve` returns `HTTP/1.1 304 Not Modified` (`X-Slice-Static: HIT-304`) in under 2 microseconds.
3. **Dynamic API Fallback:** Any request path that does not match a static file (such as `/api/v1/...` or any `POST`/`PUT`/`DELETE` request) is routed straight to your TypeScript `fetch(req)` handler.

---

## 6. Streaming Server-Sent Events (`Response.sse` & `Slice.streamSSE`)

Building an AI chat interface, real-time notification feed, or live telemetry dashboard? Slice has first-class support for **WHATWG Server-Sent Events (SSE)** and **`ReadableStream`** - with automatic Egress DLP scanning on every emitted frame.

### 6.1 Using `Response.sse(source, init?)` inside `export default { fetch }`
`Response.sse` accepts an array of event objects/strings, a callback function `emit(event)`, or a generator/iterator:

```typescript
export default {
    fetch(req: Request): Response {
        const url = new URL(req.url);

        if (url.pathname === "/api/ai/stream") {
            return Response.sse(
                (emit: (ev: any) => void) => {
                    emit({ id: "msg-1", event: "token", data: { text: "Slice " }, retry: 2000 });
                    emit({ id: "msg-2", event: "token", data: { text: "streams SSE " } });
                    emit({ id: "msg-3", event: "token", data: { text: "with Egress DLP!" } });
                    emit({ id: "msg-4", event: "done", data: "[DONE]" });
                },
                { status: 200, headers: { "x-stream-model": "slice-agent-v1" } }
            );
        }

        return Response.json({ ok: true });
    },
};
```
`slice serve` automatically sets `Content-Type: text/event-stream; charset=utf-8`, `Cache-Control: no-cache`, and `X-Accel-Buffering: no`, and formats each event as standard `id:`, `event:`, `retry:`, and `data:` SSE frames.

### 6.2 Using `ReadableStream`
You can also construct and read standard WHATWG `ReadableStream` objects directly:
```typescript
const stream = new ReadableStream({
    start(controller) {
        controller.enqueue("id: 1\nevent: update\ndata: {\"step\":1}\n\n");
        controller.enqueue("id: 2\nevent: done\ndata: [DONE]\n\n");
        controller.close();
    },
});

// Pass directly as a Response body:
const res = new Response(stream, {
    headers: { "content-type": "text/event-stream; charset=utf-8" },
});

// Or consume chunks via getReader():
const reader = stream.getReader();
while (true) {
    const { value, done } = await reader.read();
    if (done) break;
    console.log("Chunk:", value);
}
```

---

## 7. Making Outbound HTTP/HTTPS Requests (`fetch()` & `Slice.fetchSync()`)

Inside your scripts or HTTP handlers, you can call upstream HTTP/1.1 and **HTTPS (TLS 1.3 / 1.2 via pure-Rust `rustls` + Mozilla WebPKI root certificates)** APIs and webhooks using standard `await fetch(url, init)` or synchronous `Slice.fetchSync(url, init)`.

### 7.1 Outbound `fetch()` Architecture: Connection Pooling, Concurrent Fan-Out & Redirects
- **Thread-Local Keep-Alive Connection Pool (`OUTBOUND_CONN_POOL`):** Idle TCP and TLS 1.3 streams are pooled per `(tenant_id, scheme, host, port)` (up to 4 streams per origin), eliminating repeated TCP + TLS handshakes on sequential webhook calls.
- **Concurrent `Promise.all([fetch(...), ...])` Fan-Out (`OUTBOUND_FETCH_POOL`):** Multiple `fetch()` calls initiated in the same microtask tick are dispatched in parallel across a background OS thread pool so `await Promise.all([fetch(a), fetch(b), fetch(c)])` runs in $\max(T_a, T_b, T_c)$ wall time instead of $\sum T_i$.
- **HTTP Chunked Transfer-Encoding & Binary Bodies:** Automatically decodes `Transfer-Encoding: chunked` upstream responses and preserves raw bytes (`latin1` 1:1 byte string) for `await res.arrayBuffer()`, `await res.bytes()`, and `await res.blob()`.
- **Hop-by-Hop Redirect Validation (`redirect: "follow" | "manual" | "error"`):** Follows up to 5 `301`/`302`/`303`/`307`/`308` redirects, re-verifying each hop's `Location` URL and DNS-resolved IP against `allow_net` and the SSRF firewall while stripping `Authorization` and `Cookie` headers on cross-origin hops.

### 7.2 Outbound `fetch()` Example with HMAC Signing
```typescript
import * as crypto from "node:crypto";

async function dispatchWebhook(orderId: string, amountCents: number): Promise<number> {
    const payload = JSON.stringify({ orderId, amountCents });
    const signature = crypto
        .createHmac("sha256", process.env.WEBHOOK_SECRET || "dev_key")
        .update(payload)
        .digest("hex");

    const response = await fetch("http://127.0.0.1:18095/upstream/webhook", {
        method: "POST",
        headers: {
            "content-type": "application/json",
            "x-slice-signature": `sha256=${signature}`,
        },
        body: payload,
    });

    const data: any = await response.json();
    return response.status;
}
```
Remember to allowlist the upstream host in `slice.toml` (`allow_net = ["127.0.0.1:18095"]`) or on the command line (`--allow-net=127.0.0.1:18095`).

### 7.3 Mocking Outbound `fetch()` in Tests (`Slice.mockFetch`) & Cassette Replay
- **In Unit Tests (`Slice.mockFetch`):** Mock any upstream endpoint in memory without starting a mock server:
  ```typescript
  Slice.mockFetch("POST", "https://api.stripe.com/v1/charges", {
      status: 200,
      body: { id: "ch_mock_123", paid: true },
  });

  const res = await fetch("https://api.stripe.com/v1/charges", { method: "POST" });
  const json = await res.json(); // -> { id: "ch_mock_123", paid: true }
  ```
- **Deterministic Cassette Recording (`--deterministic=<seed>`):** When `--deterministic` is enabled, the first live `fetch()` call automatically caches the upstream HTTP response in `Slice.kv` under `fetch_cassette:<METHOD>:<URL>`, and subsequent calls replay the exact response from memory in `< 1 microsecond`!

---

## 8. Multi-Tenant SaaS Routing & Zero-Downtime Hot-Swap (`--tenant-dir`)

Suppose you run a SaaS platform where each customer (`tenant-alpha`, `tenant-beta`, `tenant-gamma`) has their own custom TypeScript workflow or storefront compiled into a `.cas` bundle:
```text
./tenants/
  ├── default.cas
  ├── tenant-alpha.cas
  └── tenant-beta.cas
```

Start `slice serve` with `--tenant-dir=./tenants`:
```bash
slice serve --port=3000 --workers=8 --tenant-dir=./tenants
```

1. **Automatic Tenant Resolution:**
   - A request with header `X-Slice-Tenant: tenant-alpha` **or** URL path `/t/tenant-alpha/checkout` automatically executes `./tenants/tenant-alpha.cas` inside `tenant-alpha`'s isolated `SliceIsolate` and `Slice.kv` partition.
   - Unknown tenants automatically fall back to `./tenants/default.cas` (if present).
2. **Zero-Downtime Atomic Hot-Swapping:**
   - When `tenant-alpha` deploys new code, simply run `slice pack ./alpha-src ./tenants/tenant-alpha.cas`.
   - On the very next HTTP request for `tenant-alpha`, `slice serve` detects the new file modification timestamp (`mtime_ns`), memory-maps the updated `.cas` archive, pre-warms its realm closure, and serves the new version with **0 dropped connections and 0 server restarts**.

---

## 9. Real-Time RFC 6455 WebSockets (Client `new WebSocket` & Server `Slice.upgradeWebSocket`)

Slice includes a zero-dependency **RFC 6455 WebSocket engine** supporting both outbound client connections (`ws://` and `wss://` over TLS 1.3) and inbound server connection upgrades (`HTTP/1.1 101 Switching Protocols`).

### 9.1 Outbound Client `new WebSocket(url, protocols?)`
Client WebSockets enforce the exact same **OCap `allow_net` host allowlist**, **Pre-Connect DNS Resolved-IP SSRF Firewall**, **Tenant Memory Ceiling**, and **Outbound Egress Secret DLP** (on every `ws.send()` frame) as `fetch()`:

```typescript
const ws = new WebSocket("wss://echo.example.com/realtime", ["chat.v1"]);
ws.binaryType = "arraybuffer";

ws.onopen = () => {
    ws.send("hello from Slice");
    ws.send(new Uint8Array([1, 2, 3, 4]));
    ws.close(1000, "done");
};

ws.onmessage = (ev: MessageEvent) => {
    console.log("Received frame:", ev.data);
};
```

For offline unit testing and deterministic replay, mock any WebSocket endpoint with `Slice.mockWebSocket(urlPattern, modeOrHandler)`:
```typescript
Slice.mockWebSocket("wss://echo.example.com/realtime", "echo");
```

### 9.2 Server-Side WebSocket Upgrade (`Slice.upgradeWebSocket`)
Inside `export default { fetch(req) }`, upgrade incoming WebSocket connections (`req.headers.get("upgrade") === "websocket"`) using `Slice.upgradeWebSocket({ open, message, close })`:

```typescript
export default {
    fetch(req: Request): Response {
        if (req.headers.get("upgrade") === "websocket") {
            return Slice.upgradeWebSocket({
                open(ws) {
                    ws.send("connected");
                },
                message(ws, msg) {
                    ws.send(typeof msg === "string" ? `echo:${msg}` : msg);
                },
                close(ws, code, reason) {
                    // Clean disconnect
                },
            });
        }
        return Response.json({ status: "http-ok" });
    },
};
```
