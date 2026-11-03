# Slice Runtime Compatibility Matrix

Slice is a capability-based, deterministic edge and serverless JavaScript/TypeScript runtime engineered for sub-microsecond cold starts (`< 1.5 us`), `4.0 KB` per-tenant isolate memory overhead, and deterministic time-travel replay.

Instead of embedding a heavyweight OS process runtime like libuv/V8, Slice targets the **WinterTC Minimum Common Web API** specification alongside **OCap-Sandboxed `node:*` Core Tier-1** modules. This document provides an explicit, verifiable compatibility matrix modeled on Cloudflare `workerd` and Deno/Bun compatibility tables.

---

## 1. WinterTC Minimum Common Web API (100% Supported)

All WinterTC globals are available in every isolate out of the box with zero polyfill overhead:

| Category | Global / API | Support Status | Implementation Notes |
| :--- | :--- | :--- | :--- |
| **HTTP & Fetch** | `fetch`, `Request`, `Response`, `Headers` | **100% Supported** | Native Rust HTTP/1.1 & TLS 1.3 (`rustls` + `webpki-roots`), Keep-Alive socket pool, chunked streaming, hop-by-hop Zero-Trust SSRF redirect validation, Egress DLP redaction, `.arrayBuffer()`, `.bytes()`, `.blob()`, `.formData()`, `.json()`, `.text()`, `Response.json()`, `Response.redirect()`. |
| **URL Routing** | `URL`, `URLSearchParams` | **100% Supported** | Full query mutation (`append`, `set`, `delete`, `getAll`, `sort`), iterator protocols, and relative URL resolution. |
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
| `node:child_process`, `node:cluster`, `node:worker_threads` | **Unsupported by Design** | Spawning OS processes or threads from inside a tenant violates multi-tenant OCap containment and deterministic replay. Use `Slice.evalSandbox(code, caps)` for sub-isolate execution or host-orchestrated worker pools. |
| Native C/C++ Addons (`.node` / N-API / `dlopen`) | **Unsupported by Design** | Arbitrary native machine code bypasses memory caps, fuel metering, and capability checks. Use pure JS/TS packages or WebAssembly / built-in native Rust intrinsics (`crypto`, `Slice.kv`). |
| `node:vm` | **Unsupported by Design** | Use `Slice.evalSandbox(code, { fuel, memoryKb, allowNet })` which enforces SES prototype lockdown, fuel metering, and automatic CoW rollback on error. |
| Raw OS Sockets (`node:net`, `node:tls`, `node:dgram`, `node:dns`) | **Unsupported by Design** | Direct TCP/UDP sockets bypass HTTP Egress DLP and hop-by-hop SSRF guards. Use WinterTC `fetch()` (with Keep-Alive pooling and TLS 1.3) for outbound I/O and `export default { fetch(req, env, ctx) }` for inbound HTTP serving. |

---

## 5. Slice-Exclusive Extensions (`globalThis.Slice`)

In addition to standard WinterTC and `node:*` APIs, Slice provides deterministic and zero-trust primitives on the frozen `Slice` namespace:

- `Slice.checkpoint()` / `Slice.restore(id)`: $O(1)$ Copy-on-Write snapshot and rollback of both the Virtual File System and Sharded Edge KV store.
- `Slice.kv`: 64-shard concurrent in-memory Edge KV store (`get`, `put` with TTL, `delete`, `list`) with zero serialization overhead.
- `Slice.evalSandbox(code, options)`: Spawn an isolated, fuel-metered sub-sandbox with independent capability bounds for safely executing untrusted AI/LLM-generated code.
- `Slice.mockFetch(pattern, response)` / `Slice.clearMocks()`: Deterministic HTTP cassette mocking combined with virtual time-warp (`0 ns` timer fast-forwarding).
- `Slice.setSeed(u64)`: Switches `Math.random()`, `crypto.randomUUID()`, `crypto.getRandomValues()`, and `crypto.randomBytes()` into deterministic xorshift64* replay mode.

