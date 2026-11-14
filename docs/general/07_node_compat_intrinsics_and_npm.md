# Chapter 07: Node.js Compatibility (`node:*`), NPM Packages & Migration Guide (Node / Bun / Deno to Slice)

Moving an existing TypeScript or JavaScript codebase to Slice does not require rewriting your business logic or giving up your favorite `npm` packages. Slice provides a zero-cold-start **Node.js built-in module layer (`node:*`)** backed by native Rust intrinsics, bundles standard CommonJS and ESM packages from `node_modules/`, and supports the standard WinterTC Web APIs (`fetch`, `Request`, `Response`, `Headers`, `URL`, `URLSearchParams`, `ReadableStream`).

This chapter provides the complete API reference for every built-in `node:*` module, explains how to use external `npm` libraries, and gives you step-by-step migration guides from **Node.js (Express/Fastify)**, **Bun**, **Deno**, and **Cloudflare Workers**.

---

## 1. Complete `node:*` Built-In Module Reference

All built-in modules can be imported with or without the `node:` prefix (`import * as fs from "node:fs"` or `import * as fs from "fs"`, as well as CommonJS `require("node:fs")` or `require("fs")`). They are loaded lazily on first use (`0.00 ns` cost if unused).

### 1.1 `node:fs` / `fs` & `node:fs/promises` (OCap-Guarded Copy-on-Write VFS Slab + Read-Only Host Fallback)
Every `node:fs` and `node:fs/promises` operation checks your `allow_read` / `allow_write` prefixes, blocks `..` traversal, NUL bytes, and device/UNC paths, anchors relative paths to the canonical workspace CWD (`process.cwd()` / `realpathSync`), serves hot files and open file descriptors from Slice's inode-backed copy-on-write VFS slab (`BOA_VFS_STATE`), masks unlinked paths with VFS tombstones without deleting host OS files, and snapshots/restores state in `< 5 us` during `Slice.checkpoint()` / `Slice.restore()` and `slice test`.

```typescript
import * as fs from "node:fs";
import * as fsp from "node:fs/promises";

// Write, append, copy, and rename files (requires allow_write prefix)
fs.writeFileSync("/tmp/report.json", JSON.stringify({ ok: true }));
fs.appendFileSync("/tmp/access.log", "2026-10-25 GET /api/v1\n");
fs.copyFileSync("/tmp/report.json", "/tmp/report.backup.json");
fs.renameSync("/tmp/report.backup.json", "/tmp/report.v2.json");

// Read files (as UTF-8 string or Uint8Array Buffer) & check existence (requires allow_read prefix)
const exists: boolean = fs.existsSync("/tmp/report.json");
const content: string = fs.readFileSync("/tmp/report.json", "utf8");
const rawBuf: Buffer = fs.readFileSync("/tmp/report.json");

// Directories, metadata, symlinks, temp dirs & cleanup
fs.mkdirSync("/tmp/shards", { recursive: true });
const tmpDir: string = fs.mkdtempSync("/tmp/job-"); // CoW VFS-aware - rolls back in < 5 us on Slice.restore()!
fs.symlinkSync("/tmp/report.json", `${tmpDir}/latest.json`);
const realTarget: string = fs.realpathSync(`${tmpDir}/latest.json`);
const lstats = fs.lstatSync(`${tmpDir}/latest.json`); // lstats.isSymbolicLink() === true
const files: string[] = fs.readdirSync("/tmp");
const stats = fs.statSync("/tmp/report.json"); // { size, isFile(), isDirectory(), mtimeMs, mtime }
fs.chmodSync("/tmp/report.json", 0o644);
fs.unlinkSync("/tmp/report.v2.json");
fs.rmSync("/tmp/shards", { recursive: true, force: true });

// 64 KB Streaming File I/O (fs.createReadStream & fs.createWriteStream)
const rs = fs.createReadStream("/tmp/report.json", { highWaterMark: 65536 });
const ws = fs.createWriteStream(`${tmpDir}/streamed.json`);
rs.pipe(ws);

// Promise-based API (node:fs/promises or fs.promises)
await fsp.writeFile("/tmp/async.txt", "hello");
const asyncText = await fsp.readFile("/tmp/async.txt", "utf8");
```

### 1.2 `node:crypto` / `crypto` (Hardware Intel/AMD `SHA-NI` + Native `ring` AEAD, KDF, & Asymmetric Suite)
```typescript
import * as crypto from "node:crypto";

// Multi-chunk streaming Hash ("sha256", "sha384", "sha512", "sha1", "md5")
// Calling .digest() without encoding returns a real Uint8Array-backed Buffer!
const sha256Hex: string = crypto
    .createHash("sha256")
    .update("order-payload-")
    .update("1001")
    .digest("hex");

// Multi-chunk RFC-2104 HMAC ("sha256", "sha384", "sha512", "sha1", "md5")
const hmacBuf: Buffer = crypto
    .createHmac("sha512", "whsec_signing_key")
    .update("header.")
    .update("payload")
    .digest();

// Cryptographic random generation, randomFillSync, & constant-time comparison
const nonce: Buffer = crypto.randomBytes(32);
crypto.randomFillSync(nonce, 0, 16);
const id: string = crypto.randomUUID();
const match: boolean = crypto.timingSafeEqual(hmacBuf, hmacBuf);

// Fuel-metered Key Derivation Functions (PBKDF2, HKDF, scrypt)
const derivedPbkdf2: Buffer = crypto.pbkdf2Sync("password", "salt", 10_000, 32, "sha256");
const derivedHkdf: ArrayBuffer = crypto.hkdfSync("sha256", "ikm", "salt", "info", 32);
const derivedScrypt: Buffer = crypto.scryptSync("password", "salt", 32, { N: 1024, r: 8, p: 1 });

// Symmetric Ciphers (aes-128-gcm, aes-256-gcm, aes-128-cbc, aes-256-cbc)
const gcmKey = crypto.randomBytes(32);
const gcmIv = crypto.randomBytes(12);
const cipher = crypto.createCipheriv("aes-256-gcm", gcmKey, gcmIv);
cipher.setAAD(Buffer.from("aad-header"));
const encHex = cipher.update("confidential-payload", "utf8", "hex") + cipher.final("hex");
const authTag = cipher.getAuthTag();

// Asymmetric Key Generation, Key Agreement & Signing (Ed25519, X25519, ECDSA P-256 & P-384, and imported RSA 2048..8192 PKCS#1 v1.5 / RSA-PSS)
const { publicKey, privateKey } = crypto.generateKeyPairSync("ed25519");
const sig = crypto.sign(null, Buffer.from("token-body"), privateKey);
const verified = crypto.verify(null, Buffer.from("token-body"), publicKey, sig);

const aliceX = crypto.generateKeyPairSync("x25519");
const bobX = crypto.generateKeyPairSync("x25519");
const sharedSecret: Buffer = crypto.diffieHellman({ privateKey: aliceX.privateKey, publicKey: bobX.publicKey });

// Import pre-provisioned RSA keys (PEM / PKCS#8 / PKCS#1 / SPKI / JWK, 2048 to 8192 bits) for RS256/384/512 & PS256/384/512:
const rsaPriv = crypto.createPrivateKey(process.env.JWT_RSA_PRIVATE_KEY_PEM!);
const rsaPub = crypto.createPublicKey(rsaPriv);
const rsaSig = crypto.sign("sha256", Buffer.from("jwt-header.payload"), rsaPriv);
const rsaOk = crypto.verify("sha256", Buffer.from("jwt-header.payload"), rsaPub, rsaSig);

// WebCrypto subtle (Ed25519, X25519 deriveBits/deriveKey, ECDSA P-256/P-384, and imported RSASSA-PKCS1-v1_5 / RSA-PSS) is also exposed as crypto.subtle:
const x25519Pair = await crypto.subtle.generateKey({ name: "X25519" }, true, ["deriveBits", "deriveKey"]);
```

### 1.3 `node:path` / `path` (and `path.posix` / `path.win32`)
```typescript
import * as path from "node:path";

const fullPath = path.join("/workspace", "tenants", "alpha", "config.json");
const norm = path.normalize("/workspace/tenants/../tenants/./alpha/config.json");
const rel = path.relative("/workspace/tenants", fullPath); // "alpha/config.json"
const parsed = path.parse(fullPath); // { root: "/", dir: "/workspace/tenants/alpha", base: "config.json", ext: ".json", name: "config" }
const formatted = path.format(parsed);
const isAbs = path.isAbsolute(fullPath); // true
```

### 1.4 `node:buffer` / `buffer` (Real `Uint8Array`-Backed `Buffer`)
In Slice, every `Buffer` is a real **`Uint8Array` instance** (`buf instanceof Uint8Array === true`, `ArrayBuffer.isView(buf) === true`, `Buffer.isBuffer(buf) === true`) with direct zero-copy `buffer.subarray()` views over `ArrayBuffer` memory and full big/little-endian wire-protocol integer readers and writers:
```typescript
import { Buffer } from "node:buffer";

// Allocation & encoding ("utf8", "hex", "base64", "base64url", "latin1", "ascii")
const b64Url: string = Buffer.from("{\"alg\":\"HS256\"}", "utf8").toString("base64url");
const decoded: string = Buffer.from(b64Url, "base64url").toString("utf8");

// Binary wire-protocol packing (PostgreSQL / Redis / JWT / Protobuf)
const packet = Buffer.alloc(16);
packet.writeUInt32BE(0xdeadbeef, 0);
packet.writeInt32LE(-42000, 4);
packet.writeBigUInt64BE(0x0102030405060708n, 8);

console.log(packet.readUInt32BE(0) === 0xdeadbeef); // true
console.log(packet.subarray(0, 4).toString("hex")); // "deadbeef"
```

### 1.5 `node:events` / `events` (`EventEmitter`)
```typescript
import { EventEmitter } from "node:events";

const bus = new EventEmitter();
bus.on("order", (id: string, cents: number) => console.log(id, cents));
bus.once("init", () => console.log("initialized"));
bus.emit("order", "ORD-1", 4900);
```

### 1.6 `node:url` / `url` (and Global `URL` & `URLSearchParams`)
```typescript
import { URL, URLSearchParams, fileURLToPath, pathToFileURL } from "node:url";

const u = new URL("/v2/search?q=slice&tag=edge", "https://api.slice.dev");
u.searchParams.append("tag", "rust");
u.searchParams.sort();
console.log(u.href); // "https://api.slice.dev/v2/search?q=slice&tag=edge&tag=rust"
```

### 1.7 `node:util`, `node:util/types`, `node:assert`, `node:os`, `node:process`, & Secondary `node:*` Modules (`perf_hooks`, `diagnostics_channel`, `readline`, `dns`, `tls`, `http2`, `tty`, `module`, `vm`, `v8`, `wasi`, `inspector`, `cluster`, `console`, `querystring`)
```typescript
import * as os from "node:os";
import * as util from "node:util";
import * as assert from "node:assert/strict";
import process from "node:process";
import { performance, PerformanceObserver } from "node:perf_hooks";
import * as diagnostics_channel from "node:diagnostics_channel";
import { createRequire, isBuiltin } from "node:module";
import * as vm from "node:vm";

// CLI flag parsing, deep equality, ANSI styling, & MIME parsing in node:util:
const { values, positionals } = util.parseArgs({
    args: ["-v", "--port=8080", "app.ts"],
    options: { verbose: { type: "boolean", short: "v" }, port: { type: "string" } },
    allowPositionals: true,
});
assert.strictEqual(util.isDeepStrictEqual(values, { verbose: true, port: "8080" }), true);
const colored = util.styleText(["bold", "green"], "READY");
assert.strictEqual(util.stripVTControlCharacters(colored), "READY");
const mime = new util.MIMEType("application/json; charset=utf-8");

// Diagnostics channels & full process CLI surface (stdout/stderr/stdin, memoryUsage, hrtime.bigint, uptime, events):
const ch = diagnostics_channel.channel("http.server.request");
process.stdout.write(`Stage: ${process.env.NODE_ENV || "production"}\n`);
const mem = process.memoryUsage(); // { rss, heapTotal, heapUsed, external, arrayBuffers }
const t0: bigint = process.hrtime.bigint();

// Sandboxed node:vm backed by Slice.evalSandbox (5-Ring OCap + fuel metered):
const ctx = vm.createContext({ count: 10 });
const vmOut = vm.runInContext("count += 32; count", ctx, { timeout: 50 }); // 42
```

### 1.7a `node:async_hooks`, `node:zlib`, & `node:string_decoder`
Slice includes full asynchronous context propagation, zero-dependency native Rust compression, and multibyte streaming string decoding:
- **`node:async_hooks` (`AsyncLocalStorage` & `AsyncResource`)**: Supports `run`, `getStore`, `enterWith`, `disable`, `exit`, `AsyncLocalStorage.bind`, `AsyncLocalStorage.snapshot`, and `AsyncResource` (`runInAsyncScope`, `bind`) with automatic context propagation across `Promise.then`/`catch`/`finally`, `queueMicrotask`, `process.nextTick`, `setTimeout`, `setInterval`, and `setImmediate`.
- **`node:zlib` (Pure-Rust Zero-Dependency DEFLATE / GZIP / Zlib)**: Backed by Slice's self-contained RFC 1950/1951/1952 compression engine (`gzipSync`, `gunzipSync`, `deflateSync`, `inflateSync`, `deflateRawSync`, `inflateRawSync`, `unzipSync`, async callback equivalents, and `zlib.constants`) with decompression bomb protection enforced against the tenant's memory ceiling.
- **`node:string_decoder` (`StringDecoder`)**: Stateful multibyte chunk streaming (`.write(buf)`, `.end(buf?)`) preserving split UTF-8 sequences, UTF-16LE surrogate pairs, and Base64 / Base64URL byte triplets across chunk boundaries.

```typescript
import { AsyncLocalStorage } from "node:async_hooks";
import * as zlib from "node:zlib";
import { StringDecoder } from "node:string_decoder";

const requestContext = new AsyncLocalStorage<{ traceId: string }>();
await requestContext.run({ traceId: "req-9001" }, async () => {
    await Promise.resolve();
    console.log(requestContext.getStore()?.traceId); // "req-9001"
});

const gz = zlib.gzipSync(Buffer.from("edge-payload-v1"));
const plain = zlib.gunzipSync(gz).toString("utf8"); // "edge-payload-v1"

const decoder = new StringDecoder("utf8");
const part1 = decoder.write(Buffer.from([0xf0, 0x9f]));       // "" (holds incomplete 4-byte sequence)
const part2 = decoder.end(Buffer.from([0x9a, 0x80]));         // completes multibyte codepoint
```

### 1.7b `node:stream`, `node:stream/promises`, `node:stream/web`, `node:http`, `node:https`, & `Slice.serve`
Slice provides a full `EventEmitter`-based `node:stream` hierarchy (`Readable`, `Writable`, `Duplex`, `Transform`, `PassThrough`, `pipeline`, `finished`, `Readable.fromWeb` / `Readable.toWeb`, `Writable.fromWeb` / `Writable.toWeb`) and `node:http` / `node:https` (`IncomingMessage`, `ServerResponse`, `Server`, `createServer`, `ClientRequest`, `request`, `get`, `Agent`, `STATUS_CODES`, `METHODS`), plus `Slice.serve` (`Deno.serve` / `Bun.serve` aliases). Incoming HTTP requests in `slice serve` bridge directly in-memory to `http.createServer` or `Slice.serve` handlers with zero loopback socket overhead:
```typescript
import * as http from "node:http";
import { Readable, Transform } from "node:stream";
import { pipeline } from "node:stream/promises";

// Stream pipeline + WHATWG ReadableStream interoperability:
const upper = new Transform({
    transform(chunk, _enc, cb) {
        cb(null, Buffer.from(chunk).toString("utf8").toUpperCase());
    },
});
const webStream = Readable.toWeb(Readable.from(["slice-", "streams"]));

// Express / Polka / Node http.createServer works in both `slice run` and `slice serve`:
const server = http.createServer((req, res) => {
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify({ method: req.method, url: req.url }));
});
server.listen(3000);
```

### 1.8 `slice:test` / `node:test` / `bun:test`
```typescript
import { describe, test, it, beforeEach, afterEach, expect } from "slice:test";
// Also works with:
// import { describe, test, expect } from "node:test";
// import { describe, test, expect } from "bun:test";
```

### 1.9 `node:timers`, `node:timers/promises`, `setImmediate`, & `process.nextTick`
Slice enforces the exact Node.js / WinterTC task and microtask priority hierarchy (`synchronous code -> process.nextTick -> Promise.then / queueMicrotask -> setImmediate -> setTimeout(0)`) and supports Promise-based timers with `AbortSignal` cancellation and `0 ns` deterministic virtual time-warp:
```typescript
import { setTimeout as sleep, setImmediate as nextCheck } from "node:timers/promises";

process.nextTick(() => console.log("1. nextTick queue"));
queueMicrotask(() => console.log("2. microtask queue"));
setImmediate(() => console.log("3. check-phase setImmediate"));

const res = await sleep(25, "warped-in-0ns-under-deterministic-mode");
const imm = await nextCheck("immediate-resolved");
```

### 1.10 Full WHATWG `console.*` Specification (Protected by Egress DLP)
Every `console.*` method (`log`, `info`, `warn`, `error`, `debug`, `table`, `time`, `timeLog`, `timeEnd`, `group`, `groupCollapsed`, `groupEnd`, `dir`, `dirxml`, `count`, `countReset`, `assert`, `trace`, `clear`) formats structured output with automatic group indentation, deterministic `console.time`/`timeEnd` deltas under `--deterministic`, and full Outbound Egress Secret DLP scanning:
```typescript
console.group("Edge Request Diagnostics");
console.time("db-query");
console.table([
    { shard: "iad-01", latencyMs: 1.2, healthy: true },
    { shard: "fra-02", latencyMs: 2.4, healthy: true },
]);
console.timeEnd("db-query");
console.groupEnd();
```

### 1.11 Built-In Native JSX / TSX & Edge SSR (`slice:jsx`)
Slice compiles `.tsx` and `.jsx` files natively in Rust using Oxc (`v0.152.0`) and includes a built-in zero-dependency JSX/SSR runtime (`"slice:jsx"`, plus automatic fallback for `"react/jsx-runtime"`, `"preact/jsx-runtime"`, and `"hono/jsx"` when not installed in `node_modules/`):
```tsx
import { renderToString, raw, Fragment } from "slice:jsx";

function Page(props: { user: string; items: string[] }) {
    return (
        <>
            <header className="hero" style={{ backgroundColor: "#0b0f19", paddingTop: "16px" }}>
                <h1>Welcome, {props.user}</h1>
            </header>
            <ul>
                {props.items.map((item) => <li key={item}>{item}</li>)}
            </ul>
            {raw("<!-- pre-sanitized static footer -->")}
        </>
    );
}

export default {
    fetch(req: Request): Response {
        const html = renderToString(<Page user="Alice <Admin>" items={["Fast", "Secure"]} />);
        return new Response("<!doctype html>" + html, {
            headers: { "content-type": "text/html; charset=utf-8" },
        });
    },
};
```

### 1.12 MicroVM Web Workers (`new Worker()`) & `node:worker_threads`
Slice supports both WHATWG `globalThis.Worker` (`new Worker(url, options)`, `URL.createObjectURL(blob)`) and Node.js `node:worker_threads` (`Worker`, `isMainThread`, `workerData`, `parentPort`, `threadId`, `MessageChannel`, `MessagePort`, `receiveMessageOnPort`). Every spawned worker executes inside an isolated child `SliceIsolate` compartment backed by a persistent warm worker pool (`IDLE_WORKER_POOL`) with speculative background pre-warming and cached `__SLICE_RC` worker closures (`WORKER_SCRIPT_CACHE`), and supports strict capability attenuation via `slice: { allowRead, allowWrite, allowNet, fuel, memoryBytes }`:
```typescript
import { Worker, isMainThread, MessageChannel, receiveMessageOnPort } from "node:worker_threads";

const worker = new Worker(
    `
    const { workerData, parentPort } = require("node:worker_threads");
    parentPort.once("message", (msg) => {
        parentPort.postMessage({ total: msg.values.reduce((a, b) => a + b, 0) * workerData.scale });
    });
    `,
    {
        eval: true,
        workerData: { scale: 10 },
        slice: { allowRead: [], fuel: 250_000, memoryBytes: 4 * 1024 * 1024 },
    }
);

worker.on("message", (res) => console.log("Worker result:", res.total));
worker.postMessage({ values: [1, 2, 3, 4] });
```

### 1.13 Zero-Trust TCP/TLS/UDP Sockets (`cloudflare:sockets`, `node:net`, `node:dgram`)
Slice implements the WinterTC / Cloudflare `connect(address, options)` Streams Socket API (`"cloudflare:sockets"` / `"slice:sockets"` / `Slice.connect`), `node:net` (`Socket`, `connect`, `createConnection`, `isIP`, `isIPv4`, `isIPv6` with coalesced non-blocking `_schedulePump()` read microtasks), and `node:dgram` (`createSocket("udp4" | "udp6")`). Every raw TCP, TLS 1.3 (`secureTransport: "on" | "starttls"`), and UDP socket is protected by 5-Ring Security (OCap `allow_net` verification, privileged port `< 1024` blocking under wildcard `*`, Pre-Connect DNS Resolved-IP SSRF filtering, Outbound Egress Secret DLP on every `socket.write()` and `udp.send()`, and deterministic `Slice.mockSocket` testing):
```typescript
import { connect } from "cloudflare:sockets";

Slice.mockSocket("redis.internal:6379", "redis");
const sock = connect({ hostname: "redis.internal", port: 6379 });
await sock.opened;

const writer = sock.writable.getWriter();
await writer.write("PING\r\n");

const reader = sock.readable.getReader();
const { value } = await reader.read();
console.log(new TextDecoder().decode(value).trim()); // "+PONG"
await sock.close();
```

### 1.14 5-Ring OCap `node:child_process` (`ExecCap`, Environment Sanitization & Output Egress DLP)
Slice provides `node:child_process` (`execFileSync`, `spawnSync`, `execSync`, `spawn`, `exec`, `execFile`, `ChildProcess`) gated by 5-Ring `ExecCap` security (`--allow-exec`, `SLICE_ALLOW_EXEC`, `[capabilities] allow_exec`):
- **Ring 1 (Deny-All by Default):** Unlisted binaries immediately trap with `[Slice Sandbox Violation] Subprocess execution denied`.
- **Ring 2 (Environment Sanitization):** Child processes run with `env_clear()` and receive **only** the tenant's explicitly allowed environment variables (`--allow-env` / `.env`), never host ambient `std::env::vars()`.
- **Ring 3 (Child Stdio Egress DLP):** Child `stdout` and `stderr` are scanned against `--audit-egress` and `.env` Auto-DLP secrets before JavaScript receives them.
- **Ring 4 & 5 (`maxBuffer`, Timeout, & Fuel/Memory Accounting):** Enforces `maxBuffer` and execution timeouts while charging fuel and memory budgets.
```typescript
import { execFileSync, spawnSync } from "node:child_process";

// Requires: slice run app.ts --allow-exec=/bin/echo,/usr/bin/env
const out = execFileSync("/bin/echo", ["hello", "from", "ExecCap"], { encoding: "utf8" }).trim();
const envRes = spawnSync("/usr/bin/env", [], { encoding: "utf8" });
```

---

## 2. Using External `npm` Packages (`node_modules/`), Package Managers, & `.wasm` vs `.node` Addons

### 2.1 Package Manager Workflow (`npm`, `pnpm`, `yarn`, `bun`)
By design, Slice keeps its runtime binary small, deterministic, and air-gapped by delegating registry resolution and lockfile management to standard package managers (`npm`, `pnpm`, `yarn`, or `bun`). Install your dependencies into `node_modules/` with any package manager, and Slice's Oxc AST bundler (`slice run`, `slice pack`, `slice compile`) resolves ESM/CommonJS conditional `"exports"`, tree-shakes unused exports via `oxc::semantic::SemanticBuilder`, and links your entire dependency graph into a single 64-byte aligned `.cas` archive or standalone executable:

```bash
# Using npm, pnpm, yarn, or bun:
npm install hono itty-router ufo destr ohash radix3 jose zod nanoid superjson ulid semver ms ini cookie bytes dlv dset mitt fast-querystring flatted
slice run . --warm --metrics
```

### 2.2 Why `.node` C++ Addons Are Excluded - and How to Use `.wasm` Instead
Traditional Node.js C/C++ native addons (`.node` shared libraries loaded via `process.dlopen` / N-API) execute arbitrary native machine code with full ambient OS authority, bypassing CPU fuel metering, per-tenant memory ceilings, and hardware `WRPKRU` Memory Protection Keys.

In Slice, native performance extensions are written as **WebAssembly (`.wasm`)** modules and executed inside Slice's **5-Ring Pure-Rust `wasmi` (`v2.0.0`) Cage**:
- Import `.wasm` files directly (`import wasmBytes from "./kernel.wasm"`), which `slice-cas` embeds as zero-stat `Uint8Array` slices inside `.cas` archives and compiled binaries.
- Every WASM instruction and `memory.grow()` call is metered against the tenant's deterministic CPU fuel and memory budget (`< 15 us` warm instantiation via `GLOBAL_WASM_MODULE_CACHE`).

### 2.3 Verified Production NPM Packages & Edge Frameworks in Slice's 33-Workload Suite
All **33 production workloads** - `24` applications in [`tests/apps/`](../../tests/README.md) (`01..24`), `4` full-stack framework suites in [`tests/frameworks/`](../../tests/frameworks/README.md) (`Hono`, `itty-router + UnJS`, `Express/Polka` over `node:http`, and `SvelteKit 3 + Svelte 5` SSR), and `5` architectural differentiation case studies in [`tests/case_studies/`](../../tests/case_studies/README.md) (`01..05`) - bundle and execute unmodified third-party `node_modules` libraries and verify **100% byte-for-byte identical output** against Node.js, Deno, and Bun (see [`tests/reports/latest.md`](../../tests/reports/latest.md) for live cold and warm latency numbers):

| NPM Package / Framework | Common Use Case | General-Purpose Slice Compiler / Runtime Mechanism Applied |
| :--- | :--- | :--- |
| **`hono`** (`@4.6.20`, `hono/cookie`) | Ultrafast WinterTC Edge Web Framework (`RegExpRouter`, middleware, cookies) | `SLICEBC\x01` Precompiled `CodeBlock` CAS + Oxc AST Deferred Module Spans |
| **`@sveltejs/kit`** (`@3.1.1`) + **`svelte`** (`@5.57.3`) + **`devalue`** (`@6.0.3`) | Full-stack SvelteKit 3 + Svelte 5 Edge SSR, form actions, `hooks.server.js`, and hydration | `adapter-slice.js` WinterTC entry + `SLICEBC\x01` Precompiled CAS + zero-stat static asset table |
| **`express` / `@polka`** (`node:http`) | Node.js `http.createServer(app)` middleware & REST routing | Zero-copy in-memory `node:http` <-> `slice serve` request/response bridge + `node:stream` |
| **`itty-router`** (`@5.0.18`) | Compact WinterTC `AutoRouter` & middleware pipeline | `SLICEBC\x01` Precompiled `CodeBlock` CAS + Oxc AST Deferred Module Spans |
| **`ufo`** (`@1.5.4`), **`destr`** (`@2.0.3`), **`ohash`** (`@1.1.4`) | UnJS / Nitro URL normalization, safe JSON parsing, & MurmurHash3 ETag hashing | Oxc Semantic Scope-Resolved Dead-Export Pruning (prunes unused SHA-256 class in `ohash` when only `murmurHash` is imported) |
| **`jose`** (`@5.9.6`) | Edge JWS (`HS256`), JWE (`A256GCM`), & JWKS cryptography | Conditional `"exports"` subpath resolution (`jose/jwt/sign`, `jose/flattened/encrypt`) + Native Rust `crypto.subtle` |
| **`zod`** (`@3.23.8`) | TypeScript-first schema validation & transformation | Oxc AST ESM-to-CJS bundling + warm realm closure caching |
| **`radix3`** (`@1.1.2`) | High-speed Radix-tree HTTP route matching (`UnJS` / `Nitro`) | Conditional `"exports"` ESM entry resolution + Oxc AST ESM-to-CJS transformation |
| **`superjson`** (`@2.2.1`) | Rich TypeScript serialization (`Date`, `Map`, `Set`, `BigInt`, `RegExp`) | Transitive ESM graph resolution (`superjson` + `copy-anything` + `is-what`) |
| **`nanoid`** (`@5.0.7`) & **`ulid`** (`@2.3.0`) | Collision-resistant URL-safe IDs & lexicographically sortable ULIDs | Pure ESM bundling + `crypto.getRandomValues` hardware CSPRNG |
| **`semver`** (`@7.6.3`) | Semantic version parsing, coercion & range matching | Barrel tree-shaking (prunes 30+ unused files) + Native Rust `__slice_semver_*` fast path |
| **`ms`** (`@2.1.3`) | Human-readable time window parsing/formatting (`"14d"`, `"2h"`) | Oxc AST CJS bundling + warm `__slice_rc` closure cache |
| **`bytes`** (`@3.1.2`) | Byte unit parsing/formatting (`"64KB"`, `"4.56MB"`) | Oxc AST CJS bundling + warm `__slice_rc` closure cache |
| **`cookie`** (`@0.6.0` / `@0.7.2`) | RFC 6265 HTTP `Cookie` parsing & `Set-Cookie` serialization | Oxc Semantic Dead-Export Pruning + warm `__slice_rc` closure cache |
| **`ini`** (`@4.1.3`) | `.ini` configuration topology parsing | Oxc AST ESM-to-CJS transformation |
| **`fast-querystring`** (`@1.1.2`) | Fastify URL query string parser | Oxc AST CJS bundling + Eager-Edge BFS Lazy Stringification |
| **`flatted`** (`@3.3.1`) | Circular object graph serialization (OpenTelemetry spans) | ESM `"exports"` resolution + Oxc AST ESM-to-CJS transformation |
| **`dlv`** (`@1.1.3`) & **`dset`** (`@3.1.3`) | Deep nested object property lookup and mutation | Direct CommonJS/ESM bundling |
| **`mitt`** (`@3.0.1`) | Event emitter & CQRS pub/sub bus | Direct CommonJS/ESM bundling |

> For the complete API matrix of supported WinterTC globals, `Buffer` methods, and `node:*` modules, see [`docs/general/COMPATIBILITY.md`](./COMPATIBILITY.md).

---

## 3. Step-by-Step Migration Guides

### 3.1 Migrating from Node.js (`Express` / `Fastify` / `http.createServer`) to `slice serve`

> **Zero-Rewrite Option:** Existing `http.createServer(app)` / Express / Polka servers already run directly under `slice serve` via Slice's zero-copy in-memory `node:http` bridge (see [`tests/frameworks/03-node-http-express-compat`](../../tests/frameworks/03-node-http-express-compat/src/index.ts)). When you want maximum Tier-0/Tier-1 WinterTC throughput, migrate your handler to `export default { fetch }`:

**Before (Node.js + Express):**
```typescript
import express from "express";
const app = express();
app.use(express.json());

app.get("/api/users/:id", (req, res) => {
    res.json({ id: req.params.id, env: process.env.STAGE });
});

app.post("/api/orders", (req, res) => {
    res.status(201).json({ received: req.body });
});

app.listen(3000);
```

**After (Slice Standard WinterTC `export default { fetch }`):**
```typescript
export default {
    async fetch(req: Request, env: Record<string, string>): Promise<Response> {
        const url = new URL(req.url);

        if (req.method === "GET" && url.pathname.startsWith("/api/users/")) {
            const id = url.pathname.slice("/api/users/".length);
            return Response.json({ id, env: env.STAGE });
        }

        if (req.method === "POST" && url.pathname === "/api/orders") {
            const body = await req.json();
            return Response.json({ received: body }, { status: 201 });
        }

        return Response.json({ error: "Not Found" }, { status: 404 });
    },
};
```
**What you gain immediately:**
- Multi-worker HTTP/1.1 Keep-Alive throughput exceeding `145k-154k req/s` (`> 2.5x` faster than Node.js; see [`tests/reports/latest.md`](../../tests/reports/latest.md)).
- Dramatically lower memory footprint (`~13-21 MB` RSS vs `70-93 MB` in Node.js).
- Every request runs inside an isolated compartment with CPU fuel limits (`--fuel`) and Egress DLP (`--env-file=.env`).

---

### 3.2 Migrating from Bun (`Bun.serve` & `bun test`) to Slice

1. **HTTP Server (`Bun.serve` -> `export default { fetch }`):**
   - Bun already supports `export default { fetch(req) }`, and Slice also aliases `Bun.serve` to `Slice.serve`. Your existing `fetch(req)` handler runs unchanged under `slice serve`!
   - If your project serves static files, pass `--static=./public` to `slice serve` or `slice compile`.
2. **Test Suite (`import { describe, test, expect } from "bun:test"`):**
   - Slice natively recognizes `"bun:test"`! You don't even have to change your import statement - just run `slice test` and your tests automatically gain `< 5 us` VFS + KV state rollback after every `test()` block.

---

### 3.3 Migrating from Deno (`Deno.serve` & `Deno.openKv`) to Slice

1. **Relative TypeScript Imports (`.ts` extensions):**
   - Like Deno, Slice supports explicit `.ts` extensions in imports (`import { foo } from "./foo.ts"`) as well as extensionless (`"./foo"`) and `.js` (`"./foo.js"`) imports.
2. **CLI Permission Flags:**
   - Deno's `--allow-read=/path`, `--allow-write=/path`, `--allow-net=host`, `--allow-exec=/bin/tool`, and `--allow-env=KEY=VAL` work with the **exact same flag names** in `slice run` and `slice serve` - plus you can persist them cleanly in `slice.toml` so you don't have to repeat long CLI flags!
3. **Key-Value Store (`Deno.openKv` -> `Slice.kv`):**
   - Replace async `await kv.get(["rate", id])` / `await kv.set(["rate", id], val)` with synchronous, `< 0.6 us` sharded `Slice.kv.get("rate:" + id)`, `Slice.kv.set("rate:" + id, val, ttlMs)`, and `Slice.kv.incr("rate:" + id, 1)`.
