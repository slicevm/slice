# Networking, WinterTC Web Platform & Async Event Loop Architecture

This document details Slice's multi-worker HTTP/1.1 server (`slice serve` in [`crates/slice-cli/src/server/mod.rs`](../../crates/slice-cli/src/server/mod.rs)), 3-level asynchronous concurrency model, outbound HTTPS/TLS 1.3 client ([`outbound_http.rs`](../../crates/slice-engine/src/net/outbound_http.rs)), deterministic time-travel event loop ([`event_loop.rs`](../../crates/slice-engine/src/runtime/event_loop.rs)), RFC 6455 WebSocket engine ([`websocket.rs`](../../crates/slice-engine/src/net/websocket.rs)), raw TCP/TLS/UDP sockets ([`sockets.rs`](../../crates/slice-engine/src/net/sockets.rs)), and WinterTC + Node Core Tier-1 API surface.

---

## 1. Multi-Tenant `slice serve` & 3-Level Asynchronous Concurrency Architecture

```
+---------------------------------------------------------------------------------------------------+
|                           Slice (Nexus-JS) 3-Level Concurrency Architecture                       |
|                                                                                                   |
|  Level 1: Multi-Worker Shared-Nothing HTTP Server (`slice serve --workers=N`)                     |
|  +-----------------------------+  +-----------------------------+  +---------------------------+  |
|  | Worker Thread #0            |  | Worker Thread #1            |  | Worker Thread #N-1        |  |
|  | - Pre-allocated 64KB/4KB    |  | - Pre-allocated 64KB/4KB    |  | - Pre-allocated 64KB/4KB  |  |
|  | - WarmBoaRealm (__slice_rc) |  | - WarmBoaRealm (__slice_rc) |  | - WarmBoaRealm            |  |
|  | - In-place reset_for_request|  | - In-place reset_for_request|  | - In-place reset          |  |
|  +--------------+--------------+  +--------------+--------------+  +-------------+-------------+  |
|                 |                                |                               |                |
|  Level 2: Per-Isolate Deterministic Event Loop (`crates/slice-engine/src/runtime/event_loop.rs`)  |
|  +--------------+--------------------------------+-------------------------------+-------------+  |
|  | - Zero-Cost Sync Fast-Path (`ACTIVE_ASYNC_ITEMS == 0` -> 0.00 ns scheduler overhead)        |  |
|  | - Priority Queue: Sync -> `process.nextTick` -> `Promise.then`/`queueMicrotask`             |  |
|  |                        -> `setImmediate` -> `setTimeout`/`setInterval` Slab Timer Wheel     |  |
|  | - Virtual Time-Warp (`--deterministic` / `slice test` warps timer deadlines in 0 ns)        |  |
|  +--------------+--------------------------------+-------------------------------+-------------+  |
|                 |                                                                                 |
|  Level 3: Background I/O & Compute Fan-Out Pools                                                  |
|  +--------------+-----------------------------------------+------------------------------------+  |
|  | - `Promise.all([fetch(...)])` Multiplexer -> `SHARED_OUTBOUND_CONN_POOL` (TLS 1.3 KeepAlive)|  |
|  | - `IDLE_WORKER_POOL` (`workers.rs`)       -> Attenuated MicroVM `Worker` / `worker_threads` |  |
|  +---------------------------------------------------------------------------------------------+  |
+---------------------------------------------------------------------------------------------------+
```

- **Zero-Allocation Stack HTTP/1.1 Parser & Vectorized Pipelined Flushing ([`server/mod.rs`](../../crates/slice-cli/src/server/mod.rs) & [`serve_connection_keep_alive`](../../crates/slice-cli/src/server/connection.rs)):**
  - Each worker thread in the `--workers=N` shared-nothing pool pre-allocates a reusable `64 KB` socket read buffer (`read_buf`) and `4 KB` write buffer (`write_buf`).
  - Incoming HTTP/1.1 requests are scanned in-place via [`find_http_header_end`](../../crates/slice-cli/src/server/connection.rs) with zero heap `String` allocations, extracting up to 16 headers into a stack-allocated `[(&str, &str); 16]` array (`headers_stack`).
  - When multiple pipelined HTTP/1.1 requests arrive in a single TCP read (`has_more_pipelined`), responses are accumulated into `write_buf` and flushed in a single vectorized `stream.write_all(write_buf)` system call.
- **In-Place Isolate Reset ([`SliceIsolate::reset_for_request`](../../crates/slice-engine/src/isolate/mod.rs)) & Direct `JsObject::call`:**
  - Each worker resets its `SliceIsolate` in place, entering a fresh `HardwareProtectionDomain` (`WRPKRU`) and resetting CPU fuel and memory counters in nanoseconds.
  - On warm `SLICEBC\x01`/`SLICEBOA` hits, [`WarmBoaRealm::loaded_bundles`](../../crates/slice-engine/src/runtime/boa_realm.rs) stores the compiled `boa_engine::JsObject` entrypoint function handle and invokes `fn_obj.call(...)` directly in Rust, completely bypassing JS source lexing and `ctx.eval`.
- **WinterTC Request/Response, Static Assets (`ETag`), `Response.sse`, & Post-Response `Slice.waitUntil(fn)`:**
  - Supports both `export default { async fetch(request, env, ctx) }` and `Slice.request()` / `Slice.respond(...)`.
  - In-memory static asset table serves `304 Not Modified` `BLAKE3` `ETag` hits in `< 2 us`.
  - Deferred `ctx.waitUntil` / `Slice.waitUntil` callbacks are drained via [`drain_wait_until_tasks`](../../crates/slice-engine/src/isolate/http_and_ws.rs) **after** the HTTP response bytes have been flushed to the client socket, while still charging CPU fuel to the tenant's compartment budget.

---

## 2. Outbound HTTP/1.1 & HTTPS TLS 1.3 (`outbound_http.rs`)

- **Zero-Cost Lazy TLS 1.3 Engine ([`get_tls_client_config`](../../crates/slice-engine/src/net/outbound_http.rs)):** Uses `rustls` (`v0.23`, `ring` crypto provider) and Mozilla's `webpki-roots` wrapped in a `OnceLock<Arc<rustls::ClientConfig>>` with 256-session in-memory TLS 1.3 session ticket resumption. Pure-compute and HTTP workloads pay `0 bytes` of TLS root-store memory and `0.00 ns` cold-start overhead.
- **Keep-Alive Connection Pool (`OUTBOUND_CONN_POOL` & `SHARED_OUTBOUND_CONN_POOL`), `slice-io` 64 KB Slab Staging (`SHARED_FETCH_SLAB_POOL`), & Exact HTTP/1.1 Framing ([`send_and_read_framed_http11`](../../crates/slice-engine/src/net/outbound_http.rs)):** Caches warm TCP or TLS 1.3 connections partitioned by `(tenant_id, scheme, host, port)` (`30s` idle eviction), stages socket reads through 64 pre-allocated `64 KB` pinned slabs in `slice-io::RegisteredBufferPool` (`FetchSlabSlotGuard`, releasing the slot on `Drop` with zero slab leaks), and frames responses via `Content-Length` or `Transfer-Encoding: chunked` (`parse_chunked_body`) without waiting for TCP EOF, while enforcing `MAX_OUTBOUND_HEADER_BYTES` (`64 KB`), `MAX_OUTBOUND_BODY_BYTES` (`16 MB`), and RFC 9110 `tchar`/CRLF header validation.
- **Zero-Trust Hop-by-Hop Redirect Guard ([`execute_outbound_fetch_with_redirects`](../../crates/slice-engine/src/net/outbound_http.rs)):** Re-evaluates OCap host allowlist, Pre-Connect DNS Resolved-IP SSRF, and Egress DLP on every `301`/`302`/`303`/`307`/`308` redirect hop (up to 5 hops), automatically strips `Authorization`, `Cookie`, and `Proxy-Authorization` headers on cross-origin redirects, and supports `redirect: "follow" | "manual" | "error"`, `res.redirected`, and `res.url`.

---

## 3. Deterministic Time-Travel Event Loop (`event_loop.rs`)

- **Zero-Cost Synchronous Fast-Path (`ACTIVE_ASYNC_ITEMS`):** Synchronous scripts and synchronous HTTP handlers bypass the event loop scheduler completely (`0.00 ns` overhead, `4.0 KB` per-tenant compartment footprint).
- **Slab Timer Wheel, `setImmediate`, & Speculative On-Demand `Promise.prototype.then` Microtask Priority Hierarchy:**
  - Enforces strict Node.js / WinterTC priority ordering (`sync -> process.nextTick -> Promise.then / queueMicrotask -> setImmediate -> setTimeout(0)`), fires macrotask timers in deterministic `(deadline_ms, !is_immediate, id)` order with live cancellation/deadline re-verification before each callback, drains `nextTick` and microtasks after every macrotask turn, and enforces `MAX_ACTIVE_TIMERS = 1024` and `MAX_EVENT_LOOP_TICKS = 4096` (`SandboxViolation::CpuFuelExhausted`).
  - By default, `Promise.prototype.then` remains bound directly to `Boa`'s native Rust implementation (`__origThen`) with zero JS closure allocation overhead per `await` / `.then()`, and is only wrapped via `__slice_set_next_tick_hook(true)` when a script references or invokes `process.nextTick`.
- **Virtual Time-Warp (`virtual_time_warped_ms`):** When `BOA_DET_STATE` is active (`--deterministic` or `slice test`) and no live network sockets are in flight, the event loop warps the deterministic clock directly to the next timer's `deadline_ms` in **`0 ns`** without calling `std::thread::sleep`, executing 66-second exponential backoff test suites in `< 50 ms`.
- **Concurrent `Promise.all([fetch(...), ...])` Fan-Out via Bounded `FetchReactorPool` (`MAX_FETCH_REACTOR_WORKERS = 16`) & `AbortSignal.timeout(ms)`:** Executes single un-raced `fetch()` calls inline on the isolate thread, and dispatches concurrent `Promise.all([fetch(...), ...])` batches onto a lazily-initialized, bounded `FetchReactorPool` (`OnceLock<Arc<FetchReactorPool>>`, up to 16 reusable worker threads sharing `SHARED_OUTBOUND_CONN_POOL` and `slice-io::RegisteredBufferPool` 64 KB slabs) while racing timer deadlines (`AbortController`, `AbortSignal.abort()`, `AbortSignal.timeout(ms)`).

---

## 4. RFC 6455 WebSocket Engine (`websocket.rs`) & 5-Ring Raw TCP/TLS/UDP Sockets (`sockets.rs`)

- **RFC 6455 WebSockets ([`crates/slice-engine/src/net/websocket.rs`](../../crates/slice-engine/src/net/websocket.rs)):**
  - Zero-dependency RFC 6455 engine (`compute_websocket_accept_key`, 4-byte `u32` XOR masking `apply_websocket_mask_in_place`, `encode_websocket_frame`, `decode_websocket_frame`, `read_websocket_message_from_stream` with fragmented continuation reassembly and automatic `Ping`/`Pong` heartbeats, client `ws://`/`wss://` sessions, server upgrade `Response.websocket`, `Slice.mockWebSocket`, and per-frame Outbound Egress Secret DLP).
- **Zero-Trust OCap TCP/TLS/UDP Sockets ([`crates/slice-engine/src/net/sockets.rs`](../../crates/slice-engine/src/net/sockets.rs)):**
  - Powers WinterTC `Slice.connect` / `import { connect } from "cloudflare:sockets"`, `node:net`, `node:tls` (including `startTls()`), and `node:dgram` under all 5 security rings (`NetCap`, Pre-Connect DNS SSRF, Egress Secret DLP, CPU Fuel, and Memory Cage), plus deterministic `Slice.mockSocket` / `Slice.mockUdpSocket`.

---

## 5. WinterTC Minimum Common Web Platform API & `Uint8Array`-Backed `Buffer` + Node Core Tier-1

- **WinterTC Web Platform API ([`crates/slice-engine/src/preludes/js_preludes.rs`](../../crates/slice-engine/src/preludes/js_preludes.rs)):** `URL` (`canParse`), `URLPattern`, `URLSearchParams`, `Headers`, `Request`, `Response` (`Response.sse`), `fetch`, `EventSource`, `TextEncoder` (`encode`, `encodeInto`), `TextDecoder` (`utf-8`, `ascii`, `iso-8859-1`, `fatal: true`), `btoa`/`atob`, `crypto.randomUUID`, `crypto.getRandomValues`, `crypto.subtle` (`SHA-1/256/384/512`, `HMAC`, `AES-GCM`/`AES-CBC`, `Ed25519`, `ECDSA`, `RSASSA-PKCS1-v1_5`/`RSA-PSS`, `X25519`, `PBKDF2`/`HKDF`, `timingSafeEqual`, JWK/PKCS#8/SPKI/raw), `DOMException`, `Event`, `CustomEvent`, `EventTarget`, `AbortController`, `AbortSignal` (`timeout`, `any`, `abort`), `Blob`, `File`, `FormData`, `structuredClone`, `ReadableStream`, `WritableStream`, `TransformStream`, `TextEncoderStream`, `TextDecoderStream`, `CompressionStream`, `DecompressionStream`, `Intl` (`DateTimeFormat`, `NumberFormat`, `Collator`, `PluralRules`, `Segmenter`), `navigator`, `location`, and `WebAssembly`.
- **Real `Uint8Array`-Backed `Buffer` (`Object.setPrototypeOf(Buffer.prototype, Uint8Array.prototype)`):** True `Uint8Array` subclass (`b instanceof Uint8Array === true`, `Buffer.isBuffer(b) === true`) with `DataView`-accelerated wire-protocol integer readers/writers (`readUInt8/16/32BE/LE`, `readBigUInt64BE/LE`, `writeUInt8/16/32BE/LE`, `writeBigUInt64BE/LE`).
- **Lazy IIFE-Isolated `node:*` Core Modules ([`crates/slice-engine/src/preludes/node_builtins.rs`](../../crates/slice-engine/src/preludes/node_builtins.rs)):** `node:events`, `node:path` (`posix`/`win32`), `node:crypto`, `node:fs` & `node:fs/promises`, `node:util` & `node:util/types`, `node:assert` / `node:assert/strict`, `node:timers` & `node:timers/promises`, `node:os`, `node:url`, `node:buffer`, `node:stream` & `node:stream/web`, `node:http` & `node:https`, `node:zlib`, `node:async_hooks`, `node:string_decoder`, `node:querystring`, `node:net`, `node:tls`, `node:dgram`, `node:dns`, `node:vm`, `node:child_process`, and `node:worker_threads`.
