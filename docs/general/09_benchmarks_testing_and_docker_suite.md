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

### 4.1 Verified Results Across All 31 Production Applications (`SLICEBC\x01` Default, Linux `x86_64` Docker)

With **`SLICEBC\x01` Precompiled Boa `CodeBlock` CAS** enabled by default, Slice beats **Deno** and **Node.js** on **31 out of 31** workloads and beats **Bun (`v1.3.10`)** on **30 out of 31** workloads in cold one-shot execution (and **31 out of 31** in `--warm` pooled execution):

| Production Application | Slice Cold `.cas` / Bin | Slice Warm (`--warm`) | Bun `v1.3.10` | Deno `v2.7.5` | Node.js `v25.8.1` | Speedup vs Bun / Deno / Node |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **`01-fintech-checkout`** (Multi-File TS + SHA-256) | **5.11 ms** | **0.21 ms** (`0.09 us` init) | 25.19 ms | 57.62 ms | 65.56 ms | **4.93x Bun / 12.83x Node** |
| **`02-npm-ecosystem-suite`** (`semver` + `ms` + `ini`) | **5.41 ms** | **0.22 ms** (`0.06 us` init) | 29.88 ms | 75.59 ms | 115.65 ms | **5.52x Bun / 21.38x Node** |
| **`03-disk-io-log-pipeline`** (10 JSONL Shards Disk I/O) | **8.09 ms** | **1.44 ms** (`0.06 us` init) | 30.88 ms | 70.11 ms | 81.98 ms | **3.82x Bun / 10.13x Node** |
| **`04-edge-api-gateway`** (`fast-querystring` + `ms`) | **5.30 ms** | **0.18 ms** (`0.05 us` init) | 23.09 ms | 62.66 ms | 93.47 ms | **4.36x Bun / 17.64x Node** |
| **`05-jwt-oauth-session-guard`** (`cookie` + `ms` + HMAC) | **5.09 ms** | **0.20 ms** (`0.06 us` init) | 23.79 ms | 63.36 ms | 111.79 ms | **4.67x Bun / 21.96x Node** |
| **`06-telemetry-metrics-pipeline`** (`bytes` + `flatted`) | **6.10 ms** | **0.20 ms** (`0.06 us` init) | 27.97 ms | 55.34 ms | 109.12 ms | **4.59x Bun / 17.89x Node** |
| **`07-graphql-ast-validator`** (`dlv` + `dset` + AST) | **6.34 ms** | **0.22 ms** (`0.05 us` init) | 24.47 ms | 71.15 ms | 106.87 ms | **3.86x Bun / 16.86x Node** |
| **`08-event-driven-state-journal`** (`mitt` + `ms` + WAL) | **6.65 ms** | **0.22 ms** (`0.05 us` init) | 25.04 ms | 80.52 ms | 104.22 ms | **3.77x Bun / 15.67x Node** |
| **`09-wintertc-web-router`** (`Request` + `Response` + `URL`, 80 reqs) | **6.95 ms** | **0.20 ms** (`0.06 us` init) | 21.84 ms | 60.41 ms | 91.61 ms | **3.14x Bun / 13.18x Node** |
| **`10-fullstack-bff-microservice`** (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **6.62 ms** | **0.24 ms** (`0.05 us` init) | 28.44 ms | 57.51 ms | 127.60 ms | **4.30x Bun / 19.27x Node** |
| **`11-static-asset-cdn-edge`** (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **6.73 ms** | **0.26 ms** (`0.05 us` init) | 21.13 ms | 66.22 ms | 96.42 ms | **3.14x Bun / 14.33x Node** |
| **`12-ai-sse-token-streamer`** (`mitt` + `ms` + SSE + DLP, 320 frames) | **6.57 ms** | **0.22 ms** (`0.05 us` init) | 25.19 ms | 56.92 ms | 103.65 ms | **3.83x Bun / 15.78x Node** |
| **`13-webhook-orchestrator-ssrf`** (`fast-querystring` + `ms` + Live `fetch`) | **14.07 ms** | **4.96 ms** (`0.05 us` init) | 30.89 ms | 75.04 ms | 140.48 ms | **2.20x Bun / 9.98x Node** |
| **`14-federated-api-mesh-dlp`** (`bytes` + `dlv` + `dset` + Live `fetch`) | **20.89 ms** | **8.49 ms** (`0.05 us` init) | 27.99 ms | 74.08 ms | 150.74 ms | **1.34x Bun / 7.22x Node** |
| **`15-time-travel-test-suite`** (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **9.45 ms** | **0.98 ms** (`0.05 us` init) | 34.48 ms | 75.56 ms | 112.90 ms | **3.65x Bun / 11.95x Node** |
| **`16-dotenv-dlp-config-vault`** (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **7.74 ms** | **0.51 ms** (`0.05 us` init) | 19.65 ms | 56.60 ms | 88.24 ms | **2.54x Bun / 11.40x Node** |
| **`17-untyped-numeric-dsp-mcmc`** (Untyped JS FFT/FIR + `Array.filter.map.reduce`) | **5.55 ms** | **0.28 ms** (`0.05 us` init) | 20.82 ms | 49.06 ms | 81.90 ms | **3.75x Bun / 14.76x Node** |
| **`18-webcrypto-jose-zod-gateway`** (`jose` + `zod` + `nanoid` + `radix3` + `superjson`) | **42.73 ms** | **0.46 ms** (`0.05 us` init) | 38.79 ms | 96.46 ms | 127.71 ms | **84.3x Warm vs Bun / 2.99x Node** |
| **`19-wintertc-webcrypto-streams-edge`** (WebCrypto/Streams/FormData/Clone) | **11.17 ms** | **1.00 ms** (`0.05 us` init) | 26.86 ms | 57.76 ms | 84.99 ms | **2.40x Bun / 7.61x Node** |
| **`20-node-compat-and-edge-router`** (`Uint8Array` `Buffer` + Node Core + AST Bundler) | **9.66 ms** | **0.35 ms** (`0.05 us` init) | 24.47 ms | 56.97 ms | 82.89 ms | **2.53x Bun / 8.58x Node** |
| **`21-websocket-realtime-and-security-guard`** (RFC 6455 `WebSocket` + 5-Ring Guard) | **13.50 ms** | **2.15 ms** (`0.05 us` init) | 35.12 ms | 75.56 ms | 122.41 ms | **2.60x Bun / 9.07x Node** |
| **`22-worker-threads-and-raw-sockets`** (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **13.94 ms** | **1.66 ms** (`0.05 us` init) | 59.35 ms | 129.01 ms | 115.05 ms | **4.26x Bun / 8.25x Node** |
| **`23-wasm-crypto-and-simd-kernel`** (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **9.96 ms** | **1.46 ms** (`0.05 us` init) | 25.02 ms | 53.75 ms | 72.14 ms | **2.51x Bun / 7.24x Node** |
| **`24-tsx-ssr-and-wasm-edge-pipeline`** (`WebAssembly` SLA Scorer + SSR Pipeline) | **7.98 ms** | **1.20 ms** (`0.05 us` init) | 25.49 ms | 53.68 ms | 76.19 ms | **3.19x Bun / 9.55x Node** |
| **`25-wintertc-async-context-compression-urlpattern`** (`AsyncLocalStorage` + `zlib` + `URLPattern`) | **7.02 ms** | **0.34 ms** (`0.05 us` init) | 24.82 ms | 56.71 ms | 81.45 ms | **3.54x Bun / 11.60x Node** |
| **`frameworks/01-hono-edge-microservice`** (`hono@4.6.20` + `hono/cookie` + `ms`) | **13.11 ms** | **0.56 ms** (`0.05 us` init) | 33.24 ms | 77.79 ms | 96.49 ms | **2.54x Bun / 7.36x Node** |
| **`frameworks/02-itty-router-unjs-nitro-stack`** (`itty-router` + `ufo` + `destr` + `ohash` + `radix3`) | **15.52 ms** | **0.55 ms** (`0.05 us` init) | 41.10 ms | 85.94 ms | 114.66 ms | **2.65x Bun / 7.39x Node** |
| **`case_studies/01-ai-agent-tot-sandbox-branching`** (16 ToT Branches + CoW Checkpoints) | **7.25 ms** | **0.33 ms** (`0.05 us` init) | 31.11 ms | 71.13 ms | 127.00 ms | **4.29x Bun / 17.52x Node** |
| **`case_studies/02-mcp-code-mode-synthesis`** (8 Multi-Step MCP Tool Orchestrations) | **7.35 ms** | **0.39 ms** (`0.05 us` init) | 29.85 ms | 76.23 ms | 109.25 ms | **4.06x Bun / 14.86x Node** |
| **`case_studies/03-zero-trust-plugin-marketplace`** (20 Tenants x 5 Untrusted Plugins) | **8.09 ms** | **0.46 ms** (`0.05 us` init) | 41.00 ms | 80.17 ms | 113.09 ms | **5.07x Bun / 13.98x Node** |
| **`case_studies/04-100-tenant-saas-edge-mesh`** (100 Tenants x 5 Stateful Edge Reqs) | **11.12 ms** | **2.16 ms** (`0.05 us` init) | 26.86 ms | 57.66 ms | 91.69 ms | **2.42x Bun / 8.25x Node** |

### 4.2 HTTP Server (`slice serve`), Sharded KV & 1,000-Tenant MicroVM Benchmarks

| Workload | Slice (`v0.10.0`) | Deno (`v2.7.5`) | Bun (`v1.3.10`) | Node.js (`v25.8.1`) |
| :--- | :--- | :--- | :--- | :--- |
| **`slice serve` Tier-0 CAS (`--workers=8`, Keep-Alive)** | **`163,766 req/s` (`6.11 us/req`)** | 94,185 req/s | 105,077 req/s | 58,607 req/s |
| **`slice serve` Tier-1 WinterTC (`--workers=8`, Keep-Alive)** | **`126,566 req/s` (`7.90 us/req`)** | 94,185 req/s | 105,077 req/s | 58,607 req/s |
| **Per-Request Security Isolation** | **OCap + SES + Fuel + MPK Isolate** | None (Shared Heap) | None (Shared Heap) | None (Shared Heap) |
| **1,000-Tenant Swarm Cold Start (P50 / Total / RSS)** | **`1.27 us` P50 / `1.43 ms` total (`9.68 MB` RSS)** | `> 15,000 ms` | `> 5,000 ms` | `> 30,000 ms` |
| **Peak Process Memory (Across All 31 Apps)** | **`12.38 MB – 18.25 MB`** | `45.29 MB – 62.12 MB` | `26.00 MB – 40.31 MB` | `69.77 MB – 94.58 MB` |

---

## 5. Running Slice's Rust Workspace Test Suite (For Contributors)

If you are building Slice from source and want to verify the internal Rust unit test suites:
```bash
# Run all 97 workspace unit tests across all crates
cargo test --workspace
```
