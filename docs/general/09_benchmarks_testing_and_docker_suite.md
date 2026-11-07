# Chapter 09: Time-Travel Testing (`slice test`), Deterministic Replay & Production Benchmarks

In traditional test runners (`jest`, `vitest`, `bun test`, `deno test`), if Test #1 writes a temporary file to disk or mutates a shared key-value store and fails mid-test, Test #2 inherits dirty state — leading to flaky tests that pass in isolation and fail in CI. Furthermore, no standard test runner lets you assert that a function stays within a strict **CPU instruction fuel budget**, **memory ceiling**, or **Object-Capability security policy**.

Slice solves both problems with **`slice test` — the Time-Travel Test Runner**.

This chapter shows you how to write unit, integration, and security tests with `slice test`, how to eliminate flaky time/randomness bugs with `--deterministic`, and how to run Slice's 20-application Docker benchmark suite against Node.js, Deno, and Bun.

---

## 1. Writing Tests with `slice test` (`"slice:test"` / `"node:test"` / `"bun:test"`)

You can import `describe`, `test`, `it`, `beforeEach`, `afterEach`, and `expect` from `"slice:test"`, `"node:test"`, or `"bun:test"` (or use them directly as globals).

### 1.1 How Automatic `< 5 us` Time-Travel Rollback Works

Before executing **every single `test(...)` / `it(...)` block**, `slice test` automatically calls `const snap = Slice.checkpoint()`, runs your `beforeEach` hooks, test body, and `afterEach` hooks, and then calls `Slice.restore(snap)` inside a `finally` block!

That means:
- Every file created, modified, or deleted via `node:fs` inside `test("A", ...)` is **automatically rolled back in `< 5 microseconds`** before `test("B", ...)` starts.
- Every key or counter modified in `Slice.kv` inside `test("A", ...)` is **automatically restored** before `test("B", ...)` starts.
- If you ever want a specific test to intentionally persist its state mutations, pass `{ isolateState: false }` as the third argument: `test("persists state", () => { ... }, { isolateState: false })`.

### 1.2 Complete `slice test` Example

Create `tests/billing.test.ts`:
```typescript
import { describe, test, expect } from "slice:test";
import * as fs from "node:fs";

// Establish baseline state before the suite runs
Slice.kv.set("account:credits", "10000", 0);
fs.writeFileSync("/tmp/ledger.json", JSON.stringify({ balance: 10000 }));

describe("Billing Engine Time-Travel Suite", () => {
    test("debits credits and mutates /tmp/ledger.json in Test 1", () => {
        Slice.kv.set("account:credits", "2500", 0);
        fs.writeFileSync("/tmp/ledger.json", JSON.stringify({ balance: 2500, mutatedBy: "test-1" }));

        expect(Slice.kv.get("account:credits")).toBe("2500");
        expect(JSON.parse(fs.readFileSync("/tmp/ledger.json", "utf8")).balance).toBe(2500);
    });

    test("automatically sees clean baseline VFS and Slice.kv in Test 2 (< 5 us rollback!)", () => {
        // Both Slice.kv and /tmp/ledger.json were automatically restored after Test 1!
        expect(Slice.kv.get("account:credits")).toBe("10000");
        expect(JSON.parse(fs.readFileSync("/tmp/ledger.json", "utf8"))).toEqual({ balance: 10000 });
    });

    test("enforces OCap security, CPU fuel budgets, and memory ceilings", () => {
        // 1. Assert that reading /etc/passwd violates FsReadDenied
        const unauthorizedRead = Slice.evalSandbox(`
            const fs = require("node:fs");
            fs.readFileSync("/etc/passwd", "utf8");
        `, { allowRead: ["/tmp"], fuel: 50_000 });
        expect(unauthorizedRead).toViolate("FsReadDenied");

        // 2. Assert that cloud metadata SSRF is blocked even with wildcard network
        const ssrfAttempt = Slice.evalSandbox(`
            Slice.fetchSync("http://169.254.169.254/latest/meta-data/");
        `, { fuel: 50_000 });
        expect(ssrfAttempt).toViolate("NetAccessDenied");

        // 3. Assert that a hot pricing calculation stays within 1,000 CPU fuel instructions & 16 KB RAM
        const fastCalc = Slice.evalSandbox(`
            const subtotal: number = 450 * 12;
            subtotal
        `, { fuel: 50_000 });
        expect(fastCalc).toStayWithinFuel(1000);
        expect(fastCalc).toStayWithinMemory(16384);
    });
});
```

Run the suite with `slice test`:
```bash
slice test
```
Output:
```text
  [PASS] Billing Engine Time-Travel Suite > debits credits and mutates /tmp/ledger.json in Test 1 (1000 us, VFS+KV Rollback #1)
  [PASS] Billing Engine Time-Travel Suite > automatically sees clean baseline VFS and Slice.kv in Test 2 (< 5 us rollback!) (1000 us, VFS+KV Rollback #2)
  [PASS] Billing Engine Time-Travel Suite > enforces OCap security, CPU fuel budgets, and memory ceilings (1000 us, VFS+KV Rollback #3)
Slice Time-Travel Test Suite: 3 passed, 0 failed, 0 skipped (3 VFS+KV rollbacks in 14.58 ms, seed=Some(20261024))
```

---

## 2. Complete `expect(actual)` Matcher Reference

All matchers support negation via **`.not`** (e.g., `expect(x).not.toBe(0)`, `expect(res).not.toViolate()`).

### 2.1 Standard Value & Structural Matchers
| Matcher | Description |
| :--- | :--- |
| **`.toBe(expected)`** | Strict equality (`===`). |
| **`.toEqual(expected)`** | Deep structural equality for objects and arrays. |
| **`.toContain(itemOrSubstring)`** | Asserts that a string contains `itemOrSubstring` or an array includes `itemOrSubstring`. |
| **`.toBeTruthy()` / `.toBeFalsy()`** | Asserts `Boolean(actual) === true` or `false`. |
| **`.toBeNull()`** | Asserts `actual === null`. |
| **`.toBeDefined()` / `.toBeUndefined()`** | Asserts `actual !== undefined` or `actual === undefined`. |
| **`.toBeGreaterThan(n)` / `.toBeGreaterThanOrEqual(n)`** | Numeric `>` and `>=` comparisons. |
| **`.toBeLessThan(n)` / `.toBeLessThanOrEqual(n)`** | Numeric `<` and `<=` comparisons. |
| **`.toHaveLength(n)`** | Asserts `actual.length === n`. |
| **`.toHaveProperty(key, val?)`** | Asserts `key in actual` (and optionally deep-equals `val`). |
| **`.toThrow(substring?)`** | Calls `actual()` and asserts that it throws an exception (optionally matching `substring`). |

### 2.2 Slice-Exclusive Security & Resource Matchers
Pass either a `SandboxEvalOutcome` returned by `Slice.evalSandbox(...)` or a zero-argument function `() => Slice.evalSandbox(...)`:

| Matcher | Description |
| :--- | :--- |
| **`.toViolate(kindOrSubstring?)`** | Asserts that the sandboxed code trapped (`ok === false`) with a violation matching `kindOrSubstring` (e.g. `"FsReadDenied"`, `"FsWriteDenied"`, `"NetAccessDenied"`, `"EnvAccessDenied"`, `"CpuFuelExhausted"`, `"MemoryCeilingExceeded"`, `"EGRESS_SECRET_LEAK_BLOCKED"`, or any message substring). |
| **`.toStayWithinFuel(maxFuel)`** | Asserts that `ok === true` and `fuelUsed <= maxFuel`. |
| **`.toStayWithinMemory(maxBytes)`** | Asserts that `ok === true` and `memoryUsedBytes <= maxBytes`. |

---

## 3. Reproducible Debugging with `--deterministic=<seed>`

By default, `slice test` runs with a deterministic seed (`--deterministic=20261024`), and you can pass `--deterministic=<seed>` to `slice run`, `slice serve`, or `slice test`:

```bash
slice test --deterministic=998877
slice run ./src/simulation.ts --deterministic=998877
```
When deterministic mode is active:
1. **`Math.random()`, `crypto.randomUUID()`, `crypto.getRandomValues()`, and `crypto.subtle.generateKey()`** use a seeded SplitMix64 PRNG that produces the exact same sequence of values on every run.
2. **`Date.now()` and `performance.now()`** use a monotonic virtual clock (`1700000000000 + tick_ms`) that advances deterministically.
3. **`setTimeout(fn, delayMs)` and `setInterval(fn, delayMs)`** execute via **Deterministic Virtual Time-Warp (`0 ns` wall-clock sleep)** — a `setTimeout(fn, 5000)` advances the virtual clock by `+5000 ms` and runs `fn` immediately without sleeping the OS thread!
4. **Outbound `fetch()`** automatically records upstream HTTP responses into `Slice.kv` (`fetch_cassette:<METHOD>:<URL>`) and replays them deterministically.

---

## 4. Running the 31-Application Docker Benchmark Suite (`tests/`)

Slice includes a comprehensive Linux `x86_64` Docker benchmark suite ([`tests/`](file:///p:/opensource/slice/tests/README.md)) that tests **31 realistic multi-file TypeScript + `node_modules` applications, full-stack edge frameworks, and architectural case studies** against the latest official releases of **Bun (`v1.3.10`)**, **Deno (`v2.7.5`)**, and **Node.js (`v25.8.1`)**, verifying **100% byte-for-byte identical JSON output** across all 4 runtimes.

```bash
# Build the Linux Docker benchmark container
docker build -f tests/Dockerfile -t slice-realworld-bench .

# Run all 31 applications + HTTP server + KV + AI MCP + 1,000-tenant swarm benchmarks
docker run --rm slice-realworld-bench

# Or run specific tagged suites:
docker run --rm -e SLICE_BENCH_TAGS=apps,frameworks,cases,serve,swarm slice-realworld-bench
```

### 4.1 Verified Results Across All 32 Production Applications, Frameworks & Case Studies (`SLICEBC\x01` Default, Linux `x86_64` Docker)

With **`SLICEBC\x01` Precompiled Boa `CodeBlock` CAS**, Monomorphic Inline Caching (`get_fast`), and streaming `JsonSeed` `JSON.parse` enabled by default, Slice beats **Deno (`v2.9.8`)** and **Node.js (`v27.0.0-alpha`)** on **31 out of 31** workloads, beats **Bun (`v1.4.4`)** on **28 out of 31** workloads in cold one-shot execution, and beats **all 3 runtimes on 31 out of 31 workloads in `--warm` pooled execution** (full table in [`bench_latest.md`](file:///p:/opensource/slice/bench_latest.md)):

| Production Application / Framework / Case Study | Slice Cold `.cas` / Bin | Slice Warm (`--warm`) | Bun `v1.4.4` | Deno `v2.9.8` | Node.js `v27.0.0-alpha` | Speedup vs Bun / Node |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **`01-fintech-checkout`** (Multi-File TS + SHA-256) | **6.46 ms** (`5.81 ms` Min) | **0.97 ms** (`0.03 us` init) | 14.36 ms | 40.52 ms | 64.39 ms | **2.22x Bun (14.8x Warm) / 9.96x Node** |
| **`02-npm-ecosystem-suite`** (`semver` + `ms` + `ini`) | **14.05 ms** (`13.14 ms` Min) | **4.10 ms** (`0.04 us` init) | 21.32 ms | 52.46 ms | 70.90 ms | **1.52x Bun (5.20x Warm) / 5.05x Node** |
| **`03-disk-io-log-pipeline`** (10 JSONL Shards Disk I/O) | **7.89 ms** (`7.32 ms` Min) | **2.04 ms** (`0.06 us` init) | 14.36 ms | 42.52 ms | 63.43 ms | **1.82x Bun (7.03x Warm) / 8.04x Node** |
| **`04-edge-api-gateway`** (`fast-querystring` + `ms`) | **7.79 ms** (`7.36 ms` Min) | **1.95 ms** (`0.05 us` init) | 14.83 ms | 41.32 ms | 63.29 ms | **1.90x Bun (7.61x Warm) / 8.12x Node** |
| **`05-jwt-oauth-session-guard`** (`cookie` + `ms` + HMAC) | **10.88 ms** (`10.36 ms` Min) | **5.16 ms** (`0.08 us` init) | 15.18 ms | 49.43 ms | 65.32 ms | **1.40x Bun (2.94x Warm) / 6.01x Node** |
| **`06-telemetry-metrics-pipeline`** (`bytes` + `flatted`) | **14.51 ms** (`13.80 ms` Min) | **5.82 ms** (`0.06 us` init) | 16.83 ms | 45.49 ms | 65.16 ms | **1.16x Bun (2.89x Warm) / 4.49x Node** |
| **`07-graphql-ast-validator`** (`dlv` + `dset` + AST) | **9.28 ms** (`8.86 ms` Min) | **4.16 ms** (`0.03 us` init) | 13.13 ms | 46.87 ms | 61.30 ms | **1.41x Bun (3.16x Warm) / 6.60x Node** |
| **`08-event-driven-state-journal`** (`mitt` + `ms` + WAL) | **11.34 ms** (`10.72 ms` Min) | **5.36 ms** (`0.05 us` init) | 16.26 ms | 46.55 ms | 62.18 ms | **1.43x Bun (3.03x Warm) / 5.49x Node** |
| **`09-wintertc-web-router`** (`Request` + `Response` + `URL`, 80 reqs) | **15.42 ms** (`14.09 ms` Min) | **9.40 ms** (`0.08 us` init) | 13.77 ms | 47.30 ms | 84.68 ms | **1.46x Bun Warm / 5.49x Node** |
| **`10-fullstack-bff-microservice`** (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **12.99 ms** (`12.16 ms` Min) | **6.39 ms** (`0.10 us` init) | 14.10 ms | 46.93 ms | 83.65 ms | **1.09x Bun (2.21x Warm) / 6.44x Node** |
| **`11-static-asset-cdn-edge`** (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **9.83 ms** (`9.21 ms` Min) | **3.58 ms** (`0.03 us` init) | 13.20 ms | 52.12 ms | 86.89 ms | **1.34x Bun (3.69x Warm) / 8.84x Node** |
| **`12-ai-sse-token-streamer`** (`mitt` + `ms` + SSE + DLP, 320 frames) | **16.27 ms** (`15.38 ms` Min) | **8.44 ms** (`0.05 us` init) | 15.73 ms | 49.94 ms | 89.40 ms | **1.86x Bun Warm / 5.50x Node** |
| **`13-webhook-orchestrator-ssrf`** (`fast-querystring` + `ms` + Live `fetch`) | **13.60 ms** (`12.97 ms` Min) | **6.65 ms** (`0.06 us` init) | 19.53 ms | 51.08 ms | 111.60 ms | **1.44x Bun (2.94x Warm) / 8.21x Node** |
| **`14-federated-api-mesh-dlp`** (`bytes` + `dlv` + `dset` + Live `fetch`) | **14.46 ms** (`13.86 ms` Min) | **8.06 ms** (`0.11 us` init) | 21.70 ms | 52.93 ms | 109.10 ms | **1.50x Bun (2.69x Warm) / 7.54x Node** |
| **`15-time-travel-test-suite`** (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **14.98 ms** (`14.43 ms` Min) | **5.12 ms** (`0.10 us` init) | 24.47 ms | 65.63 ms | 82.49 ms | **1.63x Bun (4.78x Warm) / 5.51x Node** |
| **`16-dotenv-dlp-config-vault`** (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **7.58 ms** (`7.13 ms` Min) | **1.66 ms** (`0.03 us` init) | 14.57 ms | 43.64 ms | 65.30 ms | **1.92x Bun (8.76x Warm) / 8.61x Node** |
| **`17-zero-trust-redirect-keepalive-mesh`** (Keep-Alive/307/SSRF/DLP) | **15.68 ms** (`14.99 ms` Min) | **8.44 ms** (`0.09 us` init) | 21.81 ms | 54.94 ms | 110.92 ms | **1.39x Bun (2.58x Warm) / 7.07x Node** |
| **`18-async-event-loop-fanout-scheduler`** (Timer Wheel/`Promise.all`/`AbortSignal`) | **29.06 ms** (`27.75 ms` Min) | **17.14 ms** (`0.02 us` init) | 32.80 ms | 70.64 ms | 81.02 ms | **1.13x Bun (1.91x Warm) / 2.79x Node** |
| **`19-wintertc-webcrypto-streams-edge`** (WebCrypto/Streams/FormData/Clone) | **8.12 ms** (`7.44 ms` Min) | **1.72 ms** (`0.04 us` init) | 13.85 ms | 45.04 ms | 85.49 ms | **1.71x Bun (8.04x Warm) / 10.53x Node** |
| **`20-node-compat-and-edge-router`** (`Uint8Array` `Buffer` + Node Core + AST Bundler) | **10.58 ms** (`10.13 ms` Min) | **0.61 ms** (`0.03 us` init) | 22.16 ms | 54.88 ms | 71.15 ms | **2.10x Bun (36.6x Warm) / 6.73x Node** |
| **`21-websocket-realtime-and-security-guard`** (RFC 6455 `WebSocket` + 5-Ring Guard) | **10.73 ms** (`10.07 ms` Min) | **2.50 ms** (`0.10 us` init) | 19.98 ms | 51.86 ms | 114.93 ms | **1.86x Bun (7.98x Warm) / 10.71x Node** |
| **`22-worker-threads-and-raw-sockets`** (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **19.96 ms** (`19.31 ms` Min) | **2.16 ms** (`0.11 us` init) | 54.18 ms | 110.81 ms | 99.21 ms | **2.71x Bun (25.1x Warm) / 4.97x Node** |
| **`23-wasm-crypto-and-simd-kernel`** (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **16.51 ms** (`15.77 ms` Min) | **1.45 ms** (`0.08 us` init) | 20.78 ms | 40.07 ms | 59.12 ms | **1.26x Bun (14.3x Warm) / 3.58x Node** |
| **`24-tsx-ssr-and-wasm-edge-pipeline`** (`WebAssembly` SLA Scorer + SSR Pipeline) | **8.65 ms** (`7.79 ms` Min) | **1.75 ms** (`0.03 us` init) | 18.87 ms | 42.10 ms | 65.41 ms | **2.18x Bun (10.8x Warm) / 7.56x Node** |
| **`frameworks/01-hono-edge-microservice`** (`hono@4.6.20` + `hono/cookie` + `ms`) | **14.58 ms** (`13.82 ms` Min) | **5.65 ms** (`0.07 us` init) | 19.20 ms | 51.45 ms | 93.42 ms | **1.32x Bun (3.40x Warm) / 6.41x Node** |
| **`frameworks/02-itty-router-unjs-nitro-stack`** (`itty-router` + `ufo` + `destr` + `ohash` + `radix3`) | **17.08 ms** (`16.30 ms` Min) | **9.08 ms** (`0.31 us` init) | 19.21 ms | 46.07 ms | 89.89 ms | **1.12x Bun (2.12x Warm) / 5.26x Node** |
| **`frameworks/03-node-http-express-compat`** (`node:http` + `node:stream` Express/Polka) | **18.32 ms** | **1.81 ms** (`0.00 us` init) | Verified | Verified | Verified | **100% 4-Runtime Parity (`eb7f023c67427dc4`)** |
| **`case_studies/01-tot-autonomous-sre-agent`** (20 ToT Sandboxes + 16 Rollbacks) | **26.68 ms** (`25.20 ms` Min) | **12.71 ms** (`0.05 us` init) | 28.81 ms | 68.86 ms | 86.66 ms | **1.08x Bun (2.27x Warm) / 3.25x Node** |
| **`case_studies/02-mcp-code-mode-orchestrator`** (`-96.7%` Tokens, 4 Domains) | **19.08 ms** (`18.48 ms` Min) | **8.09 ms** (`0.10 us` init) | 16.06 ms | 42.54 ms | 64.05 ms | **1.98x Bun Warm / 3.36x Node** |
| **`case_studies/03-multi-tenant-saas-plugin-mesh`** (12 Tenants + Hot-Swap) | **13.20 ms** (`12.54 ms` Min) | **5.70 ms** (`0.10 us` init) | 16.14 ms | 47.44 ms | 83.58 ms | **1.22x Bun (2.83x Warm) / 6.33x Node** |
| **`case_studies/04-wasm-zero-trust-cdr-pipeline`** (Wasm SimHash CDR + Vault) | **17.06 ms** (`16.22 ms` Min) | **2.02 ms** (`0.09 us` init) | 18.79 ms | 43.06 ms | 66.08 ms | **1.10x Bun (9.31x Warm) / 3.87x Node** |
| **`case_studies/05-time-travel-chaos-fuzzer-payments`** (48 Payment Sagas) | **17.48 ms** (`16.92 ms` Min) | **9.45 ms** (`0.07 us` init) | 19.62 ms | 61.47 ms | 82.26 ms | **1.12x Bun (2.08x Warm) / 4.70x Node** |

### 4.2 HTTP Server (`slice serve`), Sharded KV & 1,000-Tenant MicroVM Benchmarks

| Workload | Slice (`v0.10.3`) | Deno (`v2.9.8`) | Bun (`v1.4.4`) | Node.js (`v27.0.0-alpha`) |
| :--- | :--- | :--- | :--- | :--- |
| **`slice serve` Tier-0 CAS (`--workers=8`, Keep-Alive)** | **`158,534 req/s` (`6.31 us/req`)** | 97,501 req/s | 102,431 req/s | 58,163 req/s |
| **`slice serve` Tier-1 WinterTC (`--workers=8`, Keep-Alive)** | **`120,333 req/s` (`8.31 us/req`)** | 97,501 req/s | 102,431 req/s | 58,163 req/s |
| **Per-Request Security Isolation** | **OCap + SES + Fuel + MPK Isolate** | None (Shared Heap) | None (Shared Heap) | None (Shared Heap) |
| **1,000-Tenant Swarm Cold Start (P50 / Total / RSS)** | **`1.03 us` P50 / `1.28 ms` total (`10.06 MB` RSS)** | `> 15,000 ms` | `> 5,000 ms` | `> 30,000 ms` |
| **Peak Process Memory (Across All 31 Apps)** | **`13.50 MB – 21.38 MB`** | `45.86 MB – 61.82 MB` | `25.88 MB – 40.22 MB` | `69.70 MB – 92.75 MB` |

---

## 5. Running Slice's Rust Workspace Test Suite (For Contributors)

If you are building Slice from source and want to verify the internal Rust unit test suites:
```bash
# Run all 120 workspace unit tests across all crates
cargo test --workspace
```
