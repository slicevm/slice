# Slice Runtime Compatibility Matrix

Slice is a capability-based, deterministic edge and serverless JavaScript/TypeScript runtime engineered for sub-microsecond cold starts (`< 1.5 us`), `4.0 KB` per-tenant isolate memory overhead, and deterministic time-travel replay.

Instead of embedding a heavyweight OS process runtime like libuv/V8, Slice targets the **WinterTC Minimum Common Web API** specification alongside **OCap-Sandboxed `node:*` Core Tier-1** modules. This document provides an explicit, verifiable compatibility matrix modeled on Cloudflare `workerd` and Deno/Bun compatibility tables.

---

## 1. WinterTC Minimum Common Web API (100% Supported)

All WinterTC globals are available in every isolate out of the box. Under the hood, Slice maintains human-readable JavaScript prelude sources in `crates/slice-engine/src/preludes/*.js`, compiles and minifies them deterministically at Rust build time (`crates/slice-engine/build.rs`), and evaluates only a ~6 KB ultra-core bootstrap (`00_ultra_core.js`) when creating a realm. All 9 feature shards (`fetch`/`URL`, `TextEncoder`/`EventTarget`, `ReadableStream`/`FormData`, `WebCrypto`/`structuredClone`, `Buffer`, `console.*` extensions, `WebSocket`, `Worker`/Sockets, and `WebAssembly`) are registered via self-replacing lazy property getters on `globalThis` so scripts that do not touch a subsystem pay `0 bytes` of parse or evaluation cost:

| Category | Global / API | Support Status | Implementation Notes |
| :--- | :--- | :--- | :--- |
| **HTTP & Fetch** | `fetch`, `Request`, `Response`, `Headers` | **100% Supported** | Native Rust HTTP/1.1 & TLS 1.3 (`rustls` + `webpki-roots`), tenant-partitioned Keep-Alive socket pool, checked chunked streaming, hop-by-hop Zero-Trust SSRF redirect validation, Egress DLP redaction, `.arrayBuffer()`, `.bytes()`, `.blob()`, `.formData()`, `.json()`, `.text()`, `Response.json()`, `Response.redirect()`. |
| **WebSockets (RFC 6455)** | `WebSocket`, `MessageEvent`, `CloseEvent`, `Slice.upgradeWebSocket`, `Slice.mockWebSocket` | **100% Supported** | Zero-dependency RFC 6455 Client (`ws://` and `wss://` over TLS 1.3) and Server Upgrade (`HTTP/1.1 101 Switching Protocols` + `Sec-WebSocket-Accept`). Supports text and binary (`Uint8Array` / `ArrayBuffer` / `Blob`) frames, 4-byte XOR masking, fragmented frame reassembly, automatic `Ping`/`Pong` heartbeats, OCap `allow_net` + DNS SSRF + per-frame Outbound Egress Secret DLP, and deterministic `Slice.mockWebSocket`. |
| **Pure-Rust WebAssembly** | `WebAssembly` (`validate`, `compile`, `instantiate`, `Module`, `Instance`, `Memory`, `Global`, `Table`, `CompileError`, `LinkError`, `RuntimeError`) | **100% Supported** | Pure-Rust `wasmi` (`v0.40`) engine with `GLOBAL_WASM_MODULE_CACHE` (`< 15 us` warm instantiation), zero-stat `.wasm` bundling (`import wasmBytes from "./kernel.wasm"`), 8-byte word-aligned `ArrayBuffer` linear memory sync, and 5-Ring WASM Security Cage (binary size/magic guard, unified CPU fuel metering with `SliceIsolate`, unified linear memory page/byte cage, zero ambient WASI authority + fuel-metered JS host imports, and Outbound Egress Secret DLP). |
| **MicroVM Web Workers** | `Worker`, `MessageChannel`, `MessagePort`, `URL.createObjectURL`, `URL.revokeObjectURL`, `URL.canParse` | **100% Supported** | Spawns isolated child `SliceIsolate` workers on background threads with strict capability attenuation (`child_caps = parent_caps ∩ opts.slice`), inherited Outbound Egress Secret DLP, independent fuel & memory ceilings, lock-free `mpsc` message channels, and support for Blob/data/VFS URLs. |
| **WinterTC Raw Sockets** | `cloudflare:sockets` / `slice:sockets` (`connect`), `Slice.connect`, `Slice.mockSocket` | **100% Supported** | Streams-based TCP & TLS 1.3 (`secureTransport: "off" \| "on" \| "starttls"`) sockets returning `{ readable, writable, opened, closed, close, startTls }` protected by 5-Ring Security (OCap `allow_net`, privileged port blocking under wildcard `*`, Pre-Connect DNS Resolved-IP SSRF guard, Outbound Egress Secret DLP on every `writable` chunk, and deterministic `Slice.mockSocket`). |
| **URL Routing** | `URL`, `URLSearchParams`, `URLPattern` | **100% Supported** | Full query mutation (`append`, `set`, `delete`, `getAll`, `sort`), iterator protocols, `URL.canParse()`, `URL.createObjectURL(blob)`, relative URL resolution, and WHATWG `URLPattern` (`test`, `exec`, named `:param` groups, wildcards). |
| **Compression Streams** | `CompressionStream`, `DecompressionStream` | **100% Supported** | Pure-Rust `flate2` (`miniz_oxide`) streaming compression and decompression supporting `"gzip"`, `"deflate"`, and `"deflate-raw"` via `.pipeThrough()`. |
| **WebCrypto** | `crypto`, `crypto.subtle`, `CryptoKey` | **100% Supported** | Native Rust `ring` / `sha2` / `hmac` backend: `getRandomValues`, `randomUUID`, `subtle.digest` (`SHA-1`, `SHA-256`, `SHA-384`, `SHA-512`), `subtle.importKey` / `exportKey` (`raw`, `jwk`), `subtle.sign` / `verify` (`HMAC`). Deterministic PRNG mode when `Slice.setSeed` is active. |
| **Streams** | `ReadableStream`, `WritableStream`, `TransformStream`, `TextEncoderStream`, `TextDecoderStream` | **100% Supported** | Pull/cancel lifecycle, `getReader()`, `getWriter()`, `pipeThrough()`, `pipeTo()`, `tee()`, `for await...of` async iteration, and SSE token streaming (`text/event-stream`). |
| **Binary & Encoding** | `TextEncoder`, `TextDecoder`, `atob`, `btoa` | **100% Supported** | Native Rust UTF-8 codec (`encode`, `encodeInto` returning `{ read, written }`, `decode` with `fatal` and `ignoreBOM` options) and Base64 helpers. |
| **Multipart & Blobs** | `Blob`, `File`, `FormData` | **100% Supported** | Binary-safe `Blob`/`File` (`slice`, `text`, `arrayBuffer`, `bytes`, `stream`) and `FormData` (`append`, `set`, `get`, `getAll`, `entries`) integrated with `Request`/`Response`. |
| **Events & Signals** | `Event`, `CustomEvent`, `EventTarget`, `AbortController`, `AbortSignal` | **100% Supported** | Listener registration (`once`, `passive`, `signal`), `AbortSignal.abort()`, `AbortSignal.timeout(ms)`, `AbortSignal.any(signals)`, and `DOMException`. |
| **Timers & Microtasks** | `setTimeout`, `clearTimeout`, `setInterval`, `clearInterval`, `setImmediate`, `clearImmediate`, `queueMicrotask`, `process.nextTick` | **100% Supported** | $O(1)$ Slab-Allocated Hierarchical Timer Wheel with strict priority ordering (`sync -> process.nextTick -> Promise.then / queueMicrotask -> setImmediate -> setTimeout(0)`), deterministic virtual time-warp (`0 ns` wall-clock wait during cassette tests), and real non-blocking concurrency on live sockets. |
| **Cloning & Diagnostics** | `structuredClone`, `performance.now()`, `console.*` (`log`, `info`, `warn`, `error`, `debug`, `table`, `time`, `timeLog`, `timeEnd`, `group`, `groupCollapsed`, `groupEnd`, `dir`, `dirxml`, `count`, `countReset`, `assert`, `trace`, `clear`) | **100% Supported** | Graph-preserving cyclic `structuredClone` (`Map`, `Set`, `Date`, `RegExp`, `ArrayBuffer`, `Uint8Array`, `Error`, `transfer` list) and full WHATWG `console.*` specification routed through the Outbound Egress Secret DLP Firewall (with deterministic elapsed timing under `--deterministic`). |
| **JSX / TSX & Edge SSR** | Native `.tsx` / `.jsx` compilation + `slice:jsx` (`react/jsx-runtime`, `preact/jsx-runtime`, `hono/jsx`) | **100% Supported** | Native Rust Oxc JSX/TSX compiler (`runtime: automatic`) paired with a built-in zero-dependency SSR engine (`renderToString`, `jsx`, `jsxs`, `jsxDEV`, `Fragment`, `createElement`/`h`, `raw`) featuring automatic HTML & attribute XSS escaping, void tag handling, and camelCase `style` object serialization. |

---

## 2. Node.js Core Modules (`node:*`) Compatibility

Slice supports `node:*` built-in specifiers (both `"node:<module>"` and bare `"<module>"`) via lazy per-module initialization and Object-Capability (`OCap`) host enforcement.

| Module | Support Status | Supported APIs & Sandbox Semantics |
| :--- | :--- | :--- |
| `node:buffer` / Global `Buffer` | **Supported (Full Binary)** | Real `Uint8Array` subclass (`Buffer.prototype instanceof Uint8Array === true`, `Buffer.isBuffer(b) === true`). Supports `Buffer.from` (`utf8`, `hex`, `base64`, `base64url`, `latin1`, `ascii`), `alloc`, `allocUnsafe` (zero-filled for cross-tenant safety), `concat`, `byteLength`, `compare`, `.subarray()`, `.slice()`, `.copy()`, `.equals()`, `.fill()`, `.write()`, and integer wire readers/writers (`readUInt8`, `readUInt16BE/LE`, `readUInt32BE/LE`, `readBigUInt64BE/LE`, `writeUInt8`, `writeUInt16BE/LE`, `writeUInt32BE/LE`, `writeBigUInt64BE/LE`, plus signed variants). |
| `node:crypto` | **Supported (Native Rust)** | `randomBytes(size, cb?)`, `randomUUID()`, `getRandomValues(ta)`, `timingSafeEqual(a, b)`, streaming multi-chunk `createHash("sha256" \| "sha384" \| "sha512" \| "sha1")` and `createHmac(alg, key)` returning `Buffer` (when called with no encoding) or encoded strings (`hex`, `base64`, `base64url`), plus `crypto.subtle` and `crypto.webcrypto`. |
| `node:events` | **Supported** | Full `EventEmitter` class (`on`, `addListener`, `once`, `off`, `removeListener`, `prependListener`, `prependOnceListener`, `removeAllListeners`, `emit`, `listeners`, `rawListeners`, `listenerCount`, `eventNames`, `setMaxListeners`, `getMaxListeners`, static `EventEmitter.once`). |
| `node:path` | **Supported** | `normalize`, `isAbsolute`, `join`, `resolve`, `dirname`, `basename`, `extname`, `relative`, `parse`, `format`, `sep`, `delimiter`, `path.posix`, `path.win32`. |
| `node:fs` & `node:fs/promises` | **Supported (OCap + VFS)** | `readFileSync`, `writeFileSync`, `appendFileSync`, `existsSync`, `mkdirSync`, `readdirSync`, `statSync` (`isFile()`, `isDirectory()`, `size`, `mtimeMs`), `unlinkSync`, `rmSync`, `copyFileSync`, `renameSync`, and `fs.promises` async equivalents (`readFile`, `writeFile`, `appendFile`, `stat`, `mkdir`, `readdir`, `rm`, `unlink`, `copyFile`, `rename`, `access`). Backed by CoW Virtual File System (`/tmp`, `/vfs`) with O(1) `Slice.checkpoint()` / `Slice.restore()` and gated by `--allow-read` / `--allow-write` OCap tokens for host paths. |
| `node:util` & `node:util/types` | **Supported** | `util.format` (`%s`, `%d`, `%i`, `%f`, `%j`, `%o`, `%O`, `%%`), `util.inspect` (cycle-safe), `util.promisify`, `util.callbackify`, `util.deprecate`, `util.inherits`, `TextEncoder`, `TextDecoder`, and `util.types` (`isUint8Array`, `isTypedArray`, `isArrayBuffer`, `isPromise`, `isDate`, `isRegExp`, `isNativeError`, `isMap`, `isSet`). |
| `node:assert` & `node:assert/strict` | **Supported** | `assert`, `assert.ok`, `strictEqual`, `notStrictEqual`, `deepEqual`, `deepStrictEqual`, `notDeepStrictEqual`, `throws`, `doesNotThrow`, `rejects`, `match`, `doesNotMatch`, `fail`, `AssertionError`. |
| `node:url` | **Supported** | `URL`, `URLSearchParams`, `fileURLToPath`, `pathToFileURL`. |
| `node:timers` & `node:timers/promises` | **Supported** | Callback timers (`setTimeout`, `clearTimeout`, `setInterval`, `clearInterval`, `setImmediate`, `clearImmediate`) and Promise-based `timers/promises` (`setTimeout(delay, value?, { signal? })`, `setImmediate(value?, { signal? })`, `scheduler.wait`, `scheduler.yield`) with `AbortSignal` cancellation and `0 ns` virtual time-warp in `--deterministic` mode. |
| `node:os` & `process` | **Supported (Sandboxed)** | `os.tmpdir()`, `os.platform()`, `os.arch()`, `os.EOL`, `process.env` (Proxy-gated by `--allow-env` OCap token), `process.cwd()`, `process.argv`, `process.version`, `process.nextTick`. |
| `node:worker_threads` | **Supported (OCap Attenuated)** | `Worker`, `isMainThread`, `parentPort`, `workerData`, `threadId`, `MessageChannel`, `MessagePort`, `receiveMessageOnPort`, `getEnvironmentData`, `setEnvironmentData`, `SHARE_ENV`. Child workers run inside dedicated `SliceIsolate` compartments with optional `slice: { allowRead, allowWrite, allowNet, fuel, memoryBytes }` capability attenuation. |
| `node:net` & `cloudflare:sockets` | **Supported (5-Ring OCap + DLP)** | `net.Socket`, `net.connect`, `net.createConnection`, `net.isIP`, `net.isIPv4`, `net.isIPv6`, and WinterTC `connect(address, options)` (`cloudflare:sockets` / `slice:sockets`). Every TCP/TLS connection enforces OCap `allow_net`, Pre-Connect DNS Resolved-IP SSRF filtering, and Outbound Egress Secret DLP on every `socket.write()`. |
| `node:dgram` | **Supported (5-Ring OCap + DLP)** | `dgram.Socket`, `dgram.createSocket("udp4" \| "udp6")`, `.bind()`, `.send()`, `.address()`, `.close()` with OCap `allow_net`, Pre-Connect DNS SSRF filtering, and Outbound Egress Secret DLP on every datagram. |
| `node:async_hooks` | **Supported** | `AsyncLocalStorage` (`run`, `getStore`, `enterWith`, `disable`, `exit`, `bind`, `snapshot`) and `AsyncResource` (`runInAsyncScope`, `bind`) with automatic store propagation across `Promise.then`/`catch`/`finally`, `queueMicrotask`, `process.nextTick`, `setTimeout`, `setInterval`, and `setImmediate`. |
| `node:zlib` | **Supported (Native Rust)** | Pure-Rust `flate2` (`miniz_oxide`) `gzipSync`, `gunzipSync`, `deflateSync`, `inflateSync`, `deflateRawSync`, `inflateRawSync` (plus callback/async variants) returning real `Uint8Array`-backed `Buffer` instances with decompression bomb protection against the tenant's memory ceiling. |
| `slice:jsx` (`react/jsx-runtime`, `preact/jsx-runtime`, `hono/jsx`) | **Supported (Built-In SSR)** | Zero-dependency JSX/TSX VNode runtime (`jsx`, `jsxs`, `jsxDEV`, `Fragment`, `createElement`, `h`, `raw`) and synchronous HTML serializer (`renderToString(vnode)`) with automatic XSS escaping. |

---

## 3. Module Bundling & npm Package Compatibility (`slice-cas`)

Slice compiles multi-file TypeScript/JavaScript projects and `node_modules` dependencies ahead-of-time into a content-addressed `.cas` archive (`SLC3`):

- **Zero-Config TypeScript (`oxc`)**: Native TypeScript (`interface`, `type`, generics, `enum`, access modifiers) stripped via `oxc` at build time and runtime; `.js` -> `.ts` / `.tsx` relative import resolution works automatically.
- **Oxc AST ESM-to-CJS Transformer**: Rewrites multiline `import`, `export const/function/class`, `export { a as b } from "..."`, `export default`, and `export * from "..."` statements into deterministic CommonJS closures without fragile regexes.
- **Full `package.json` `"exports"` Resolution**: Supports nested conditional exports (`"workerd"`, `"worker"`, `"edge-light"`, `"import"`, `"require"`, `"module"`, `"node"`, `"default"`), wildcard subpath patterns (`"./*"`), and array fallbacks.
- **Verified Unmodified npm Ecosystem**: Edge web routers (`hono`, `itty-router`), schema validators (`zod`, `valibot`), data utilities (`semver`, `cookie`, `ms`, `bytes`, `flatted`, `mitt`, `uuid`), and JWT/OAuth libraries run unmodified inside Slice isolates.

---

## 4. Intentionally Unsupported by Design (Security Sandbox Boundary)

To guarantee `< 1.5 us` isolate spawn times, `4.0 KB` per-tenant memory footprint, and zero-trust multi-tenant containment without hardware virtualization overhead, the following Node.js OS-level modules are **intentionally unsupported**:

| Module / Feature | Status | Architectural Reason & Recommended Alternative |
| :--- | :--- | :--- |
| `node:child_process`, `node:cluster` | **Unsupported by Design** | Spawning arbitrary host OS binaries (`exec`, `spawn`, `fork`) violates multi-tenant OCap containment. Use `new Worker()` / `node:worker_threads` for parallel JS/TS isolate execution or `Slice.evalSandbox(code, caps)` for synchronous sub-isolate evaluation. |
| Native C/C++ Addons (`.node` / N-API / `dlopen`) | **Unsupported by Design** | Arbitrary native machine code bypasses memory caps, fuel metering, and capability checks. Use pure JS/TS packages or WebAssembly / built-in native Rust intrinsics (`crypto`, `Slice.kv`). |
| `node:vm` | **Unsupported by Design** | Use `Slice.evalSandbox(code, { fuel, memoryKb, allowNet })` which enforces SES prototype lockdown, fuel metering, and automatic CoW rollback on error. |

---

## 5. Slice-Exclusive Extensions (`globalThis.Slice`)

In addition to standard WinterTC and `node:*` APIs, Slice provides deterministic and zero-trust primitives on the frozen `Slice` namespace:

- `Slice.checkpoint()` / `Slice.restore(id)`: $O(1)$ Copy-on-Write snapshot and rollback of both the Virtual File System and Sharded Edge KV store.
- `Slice.kv`: 64-shard concurrent in-memory Edge KV store (`get`, `put` with TTL, `delete`, `list`) with zero serialization overhead.
- `Slice.evalSandbox(code, options)`: Spawn an isolated, fuel-metered sub-sandbox with independent capability bounds for safely executing untrusted AI/LLM-generated code.
- `Slice.mockFetch(pattern, response)` / `Slice.clearMocks()`: Deterministic HTTP cassette mocking combined with virtual time-warp (`0 ns` timer fast-forwarding).
- `Slice.setSeed(u64)`: Switches `Math.random()`, `crypto.randomUUID()`, `crypto.getRandomValues()`, and `crypto.randomBytes()` into deterministic xorshift64* replay mode.

