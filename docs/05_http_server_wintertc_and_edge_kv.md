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

## 2. WinterTC Request, Response & Post-Response `waitUntil` API

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

