# Chapter 02: Writing TypeScript & JavaScript for Maximum Performance

In Node.js, Deno, and Bun, TypeScript type annotations (`: number`, `interface Order { ... }`) are simply erased before your code runs — they do nothing to make your program faster at runtime.

In **Slice**, TypeScript is a first-class performance tool. Before Slice's Oxc compiler strips your TypeScript types, it analyzes your `: number` variables and `interface` definitions to unlock **compile-time native optimizations** alongside a **three-tier execution pipeline**.

This chapter explains how Slice executes your code across its three tiers, how to read `--metrics` telemetry, and practical patterns you can use in your TypeScript code to get up to **10x-70x faster execution**.

---

## 1. How Slice Runs Your Code: The Three Execution Tiers & Default `SLICEBC\x01` Precompiled CAS

Whenever you run a script or bundle (`slice run`, `slice serve`, `slice test`, or a compiled binary), Slice defaults to **Tier 1 (`Tier1Baseline`) with Precompiled Boa `CodeBlock` (`SLICEBC\x01`) enabled** (`boacas = true`), while automatically engaging Tier-0 Snapshot fast paths (`SLICESNP`) on scalar scripts and Tier-2 Native SIMD kernels (`Tier2PeakNative`) on typed compute loops:

```text
+-------------------------------------------------------------------------------------------------+
|                       How Slice Selects Your Execution Tier Automatically                       |
|                                                                                                 |
|  Your TypeScript / JavaScript / .cas Bundle                                                     |
|         |                                                                                       |
|         +---> [Tier 1 (Default Optimal Path): Precompiled SLICEBC\x01 + Warm Realm Pool]        |
|         |       - Used for: Full TypeScript/JS classes, closures, async/await, Map/Set,         |
|         |         Request/Response/URL, node:* modules, Slice.kv, and npm packages              |
|         |       - Build Time: Oxc AST deferred module spans + node:* pre-fusion +               |
|         |         32-byte BoaCasHeader + SLICEBC1 recursive CodeBlock serialization             |
|         |       - Cold Start: ~3.26 - 6.95 ms (0 runtime JS lexing/parsing!)                    |
|         |       - Warm Pool (--warm / slice serve): ~0.18 - 0.56 ms (0.02 - 0.25 us instantiate)|
|         |                                                                                       |
|         +---> [Tier 0: SLICESNP Snapshot VM] (`Tier0LLIntSnapshot`)                             |
|         |       - Used for: Lightweight scalar routing, arithmetic, env(), static exports       |
|         |       - Cold Start: ~1.27 microseconds P50 | Memory: 4.0 KB                           |
|         |                                                                                       |
|         +---> [Tier 2: Peak Native SIMD & Typed Kernels] (`Tier2PeakNative`)                    |
|                 - Used when: Your code runs typed `: number` loops, SHA-256 / HMAC-SHA256,      |
|                   interface-typed JSON.stringify, or native compute kernels                     |
+-------------------------------------------------------------------------------------------------+
```

You never have to manually configure which tier to use — Slice runs on its optimal **Tier 1 + `SLICEBC\x01`** path out of the box. If you ever want to override the tier or toggle `SLICEBC\x01` precompilation for a specific script or project, you can set it via `slice.toml` (`[engine] tier = "tier1"`, `boacas = true`), top-of-file inline directives (`// @slice-tier: tier1`, `// @slice-boacas: false`), environment variables (`SLICE_TIER`, `SLICE_BOACAS`), or CLI flags (`--tier=`, `--boacas` / `--no-boacas`).

---

## 2. Reading `--metrics` Telemetry

Pass `--metrics` to any `slice run` command (or standalone compiled binary) to see exactly how your code executed:

```bash
slice run ./tests/apps/02-npm-ecosystem-suite --warm --metrics
```
Output:
```text
[Slice Telemetry] Tier: Tier1Baseline | Instantiate: 0.07 us | Exec: 272.10 us | Total: 272.17 us | Memory: 4096 B | Stat Syscalls: 0
```

| Telemetry Field | What It Means for You |
| :--- | :--- |
| **`Tier`** | Highest execution tier reached (`Tier0LLIntSnapshot`, `Tier1Baseline`, or `Tier2PeakNative`). |
| **`Instantiate`** | Time spent spawning the isolated compartment and slicing bytecode from memory (`0.02 - 2.0 us`). |
| **`Exec`** | Time spent executing your TypeScript/JavaScript logic inside the isolate. |
| **`Total`** | `Instantiate + Exec` total isolate wall time in microseconds. |
| **`Memory`** | Per-tenant memory bytes charged against your `memory_kb` ceiling (base isolate + `Slice.kv` bytes). |
| **`Stat Syscalls`** | Number of OS `stat()` filesystem metadata syscalls performed at runtime (`0` when running `.cas` or compiled binaries!). |

---

## 3. Developer Optimization #1: Use `--warm` (and `slice serve` Automatic Pre-Warming)

When running full ES2024+ applications with classes and external `node_modules` packages (`Tier1Baseline`), Slice embeds a 16-character content fingerprint (`//__SLICE_RC:<hash>`) into the compiled bundle.

- **Without `--warm` (Cold Process):** Slice initializes a fresh ECMAScript realm, evaluates your `node_modules` CommonJS factory wrappers once, and runs your entrypoint (`~5 - 15 ms`).
- **With `--warm` (or automatically inside `slice serve` workers):** Slice pre-warms its thread-local realm pool (`BOA_CONTEXT_POOL`) and caches the compiled entrypoint function handle (`JsObject`). Subsequent executions invoke your entrypoint **directly in Rust without lexing, parsing, or re-initializing `node_modules`**, dropping execution time from `6.76 ms` down to **`0.27 ms`** (`25x` faster!).

```bash
# Pack your app once
slice pack ./my-app ./my-app.cas

# Run with --warm to use the pre-warmed realm pool
slice run ./my-app.cas --warm --metrics
```
> **Note:** In `slice serve`, every worker thread automatically pre-warms both the realm pool and your `.cas` bundle at server startup — you get `--warm` performance on every HTTP request automatically.

---

## 4. Developer Optimization #2: Type-Directed `: number` Loops (`Tier2PeakNative`)

When you annotate loop counters and accumulators with TypeScript's `: number` type, Slice's Oxc semantic analyzer (`analyze_typescript_semantics`) proves at compile time that the variables are 64-bit numbers and promotes the loop into a native **`MONO_FOR_T2` (`Tier2PeakNative`)** register loop.

### Example: Writing a Tier-2 Promoted Numeric Loop

Create `typed_loop.ts`:
```typescript
// Annotating `acc: number` and `i: number` allows Slice to lower this loop
// directly into a Tier-2 native 64-bit register loop!
let acc: number = 0;
for (let i: number = 0; i < 10_000; i++) {
    acc += i * 3;
}
console.log(acc);
```

Verify the promotion using `slice inspect` and `slice run --metrics`:
```bash
slice inspect typed_loop.ts
slice run typed_loop.ts --metrics
```
Output:
```text
=========================================================================
  SLICESNP BYTECODE DISASSEMBLY: typed_loop.ts (296 bytes)
=========================================================================
  0000  CONST_NUM       %acc <- 0
  0001  MONO_FOR_T2     %acc += (%i in 0..10000) * 3
  0002  AST_EXEC        ExprStmt(Call { callee: Member { object: Ident("console"), property: "log" }, args: [Ident("acc")] })
=========================================================================
149985000
[Slice Telemetry] Tier: Tier2PeakNative | Instantiate: 14.20 us | Exec: 1.20 us | Total: 15.40 us | Memory: 4096 B | Stat Syscalls: 0
```

---

## 5. Developer Optimization #3: Type-Directed `interface` `JSON.stringify` Specialization

Serializing structured objects with `JSON.stringify(obj)` is one of the hottest paths in REST APIs, BFF microservices, and JSONL loggers. Normally, `JSON.stringify` must dynamically inspect every property key, type, and prototype at runtime.

In Slice, if you:
1. Declare a flat TypeScript `interface` with primitive fields (`string`, `number`, `boolean`), and
2. Annotate your variable with that `interface` (`const record: AuditEvent = ...`),

Slice's compiler automatically rewrites `JSON.stringify(record)` at build time into a **direct monomorphic string concatenation template**!

### Example: Schema-Accelerated `JSON.stringify`

```typescript
interface CheckoutReceipt {
    orderId: string;
    tenantId: string;
    totalCents: number;
    approved: boolean;
}

export function serializeReceipt(id: string, tenant: string, cents: number): string {
    // Because `receipt` is explicitly typed as `: CheckoutReceipt`,
    // Slice compiles `JSON.stringify(receipt)` into a direct, zero-reflection string builder!
    const receipt: CheckoutReceipt = {
        orderId: id,
        tenantId: tenant,
        totalCents: cents,
        approved: true,
    };
    return JSON.stringify(receipt);
}

console.log(serializeReceipt("ORD-501", "tenant-eu", 19900));
```
> **Safety Rule:** Slice only applies this optimization when your `interface` fields are plain `string`, `number`, or `boolean` primitives (and when string fields do not contain characters requiring JSON escaping). For nested or dynamic objects, Slice uses the standard `JSON.stringify` engine automatically.

---

## 6. Developer Optimization #4: Unboxed 64-Bit Numeric Register VM & Oxc AST Array Pipeline Fusion

Beyond explicit `: number` loop annotations, Slice includes two general-purpose ahead-of-time compiler optimizations that accelerate standard JavaScript and TypeScript functions without requiring any special annotations:

1. **Unboxed 64-Bit Numeric Register VM ([`numeric_vm.rs`](file:///p:/opensource/slice/crates/slice-engine/src/numeric_vm.rs)):**
   - Any pure numeric JS/TS function (`function fn(a, b) { ... }`) using arithmetic (`+`, `-`, `*`, `/`, `%`, `**`), bitwise operators (`|`, `&`, `^`, `<<`, `>>`, `>>>`), `while`/`for` loops, `if`/`else` branches, `Math.*` intrinsics (`Math.abs`, `Math.min`, `Math.max`, `Math.floor`, `Math.ceil`, `Math.round`, `Math.trunc`, `Math.sqrt`, `Math.imul`), or recursive calls is automatically compiled into unboxed 64-bit register instructions (`NumericOp`) on a 32-register stack file (`[f64; 32]`).
   - 2-term linear recurrences (`return f(n - 1) + f(n - 2)`) are automatically recognized and lowered to $O(n)$ register accumulation while charging deterministic CPU fuel per step.
2. **Oxc AST Array Pipeline Fusion ([`fuse_array_pipelines_ast`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs#L231-L575)):**
   - During `slice pack` and `slice compile`, Slice's Oxc AST pass detects chained functional array pipelines (such as `arr.filter(...).map(...).reduce(...)`, `arr.map(...).reduce(...)`, `arr.some(...)`, and `arr.every(...)`) on any arbitrary receiver expression and fuses them into a single-pass, zero-allocation `for` loop IIFE — eliminating intermediate array allocations on hot paths.

---

## 7. Developer Optimization #5: Hardware-Accelerated Crypto, Encoding & Ecosystem Fast Paths

When writing applications in Slice, prefer standard `node:crypto`, WebCrypto (`crypto.subtle`), `Buffer`, and supported ecosystem packages — Slice intercepts the hottest operations and runs them in **native Rust and hardware SIMD**:

| Standard TypeScript / JS Code You Write | How Slice Executes It Under the Hood |
| :--- | :--- |
| `crypto.createHash("sha256" \| "sha384" \| "sha512" \| "sha1").update(data).digest("hex")` | Lowered directly to native Rust cryptographic intrinsics (`__slice_crypto_hash`) using **Intel/AMD `SHA-NI` hardware instructions** for SHA-256 and pure-Rust `sha2`/`sha1` for SHA-384/SHA-512/SHA-1 (`Buffer` or string output). |
| `crypto.createHmac("sha256" \| "sha384" \| "sha512" \| "sha1", key).update(msg).digest("hex")` | Lowered directly to native Rust RFC-2104 HMAC (`__slice_crypto_hmac` / `__slice_crypto_hmac_sha256`). |
| `await crypto.subtle.encrypt("AES-GCM", key, data)`<br>`await crypto.subtle.sign("Ed25519", key, data)` | Executed in native Rust (`aes-gcm`, `ed25519-dalek`, `pbkdf2`, `hkdf`, `sha2`) with zero OpenSSL dependency. |
| `Buffer.from(str, "utf8").toString("base64url")`<br>`buf.readUInt32BE(0)` / `buf.writeBigUInt64BE(val, 0)` | Backed by a real `Uint8Array` instance (`Object.setPrototypeOf(u8, Buffer.prototype)`) with native Rust Base64/Base64URL/Hex codecs and full big/little-endian wire-protocol readers/writers. |
| `semver.valid(v)`, `semver.coerce(v)`, `semver.gt(a, b)`, `semver.satisfies(v, range)` | Executed in `~150 ns` by Slice's zero-regex Rust SemVer engine (`__slice_semver_*`), with automatic fallback for complex ranges. |
| `ms("15m")`, `ms(60000, { long: true })`<br>`bytes.parse("64KB")`, `bytes.format(4781506)` | Executed by native Rust zero-regex parsers/formatters (`__slice_fast_ms`, `__slice_fast_bytes_*`) + compile-time constant folding for literals like `bytes.parse("64KB")` -> `65536`. |
| `fs.writeFileSync("/tmp/a.json", data)`<br>`fs.readFileSync("/tmp/a.json", "utf8")` | Served out of Slice's thread-local **In-Memory VFS Slab** (`BOA_VFS_CACHE`) at memory speed, with OCap prefix security and `< 5 us` `Slice.checkpoint()` support. |

---

## 8. Built-In Global Functions & Objects Available in Every Script

Every Slice script has immediate access to the following globals without needing any imports:

- **Standard Console & Output:** `console.log(...)`, `console.info(...)`, `console.warn(...)`, `console.error(...)`, `print(...)`
- **WinterTC Web Standards:** `fetch(input, init)`, `Request`, `Response` (`Response.json`, `Response.redirect`, `Response.sse`), `Headers`, `URL`, `URLSearchParams`, `ReadableStream`, `WritableStream`, `TransformStream`, `TextEncoder`, `TextDecoder`, `TextEncoderStream`, `TextDecoderStream`, `Blob`, `File`, `FormData`, `Event`, `CustomEvent`, `EventTarget`, `AbortController`, `AbortSignal` (`AbortSignal.abort`, `AbortSignal.timeout`, `AbortSignal.any`), `structuredClone`, `atob`, `btoa`, `crypto` (`crypto.randomUUID()`, `crypto.getRandomValues(typedArray)`, `crypto.subtle`)
- **Timers & Microtasks:** `setTimeout(fn, delayMs, ...args)`, `clearTimeout(id)`, `setInterval(fn, delayMs, ...args)`, `clearInterval(id)`, `queueMicrotask(fn)` (powered by a $O(1)$ slab timer wheel with `0 ns` virtual time-warp in `--deterministic` mode)
- **Environment & Node Globals:** `process` (`process.env`, `process.cwd()`, `process.platform`, `process.arch`, `process.version`, `process.argv`, `process.hrtime.bigint()`, `process.nextTick()`), `env(key)`, `Buffer` (real `Uint8Array` subclass), `require(id)`
- **Timing & Determinism:** `Date.now()`, `performance.now()`, `Math.random()` (all automatically deterministic when `--deterministic=<seed>` is enabled!)
- **Test Runner Globals (Zero-Cost Lazy Loaded):** `describe(name, fn)`, `test(name, fn, opts)`, `it(name, fn, opts)`, `beforeEach(fn)`, `afterEach(fn)`, `expect(actual)`
- **The `Slice` Host Namespace:**
  - `Slice.request()`, `Slice.respond(status, body, contentType?)`, `Slice.streamSSE(source, init?)`, `Slice.waitUntil(fn)`
  - `Slice.fetchSync(input, init?)`, `Slice.mockFetch(methodOrUrl, urlOrOpts, opts?)`
  - `Slice.kv.get(key)`, `Slice.kv.set(key, val, ttlMs?)`, `Slice.kv.del(key)`, `Slice.kv.incr(key, delta?)`
  - `Slice.checkpoint()`, `Slice.restore(snapshotId)`, `Slice.evalSandbox(code, opts?)`
  - `Slice.testSummary()`, `Slice.resetTests()`
