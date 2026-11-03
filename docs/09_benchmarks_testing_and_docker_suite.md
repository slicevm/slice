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

## 4. Running the 24-Application Docker Benchmark Suite (`docker-tests/`)

Slice includes a comprehensive Linux `x86_64` Docker benchmark suite ([`docker-tests/`](file:///p:/opensource/slice/docker-tests/README.md)) that tests **24 realistic multi-file TypeScript + `node_modules` applications** against the latest official releases of **Bun (`v1.4.4`)**, **Deno (`v2.9.8`)**, and **Node.js (`v27.0.0-alpha`)**, verifying **100% byte-for-byte identical JSON output** across all 4 runtimes.

```bash
# Build the Linux Docker benchmark container
docker build -f docker-tests/Dockerfile -t slice-realworld-bench .

# Run all 24 applications + HTTP server + KV + AI MCP + 1,000-tenant swarm benchmarks
docker run --rm slice-realworld-bench

# Or run specific tagged suites:
docker run --rm -e SLICE_BENCH_TAGS=apps,wasm,serve,swarm slice-realworld-bench
```

### 4.1 Verified Results Across All 24 Production Applications (Linux `x86_64` Docker)

| Production Application | Slice Cold `.cas` / Bin | Slice Warm (`--warm`) | Bun `v1.4.4` | Deno `v2.9.8` | Node.js `v27.0.0-alpha` | Speedup vs Bun / Deno / Node |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **`01-fintech-checkout`** (Multi-File TS + SHA-256) | **5.297 ms** | **0.673 ms** (`0.08 us` init) | 12.729 ms | 34.766 ms | 63.778 ms | **2.40x Bun / 12.04x Node** |
| **`02-npm-ecosystem-suite`** (`semver` + `ms` + `ini`) | **6.766 ms** | **0.272 ms** (`0.07 us` init) | 20.048 ms | 54.156 ms | 75.895 ms | **2.96x Bun / 11.22x Node** |
| **`03-disk-io-log-pipeline`** (10 JSONL Shards Disk I/O) | **8.438 ms** | **3.593 ms** (`0.06 us` init) | 13.063 ms | 41.701 ms | 66.366 ms | **1.55x Bun / 7.86x Node** |
| **`04-edge-api-gateway`** (`fast-querystring` + `ms`) | **7.397 ms** | **1.238 ms** (`0.05 us` init) | 14.453 ms | 39.472 ms | 67.118 ms | **1.95x Bun / 9.07x Node** |
| **`05-jwt-oauth-session-guard`** (`cookie` + `ms` + HMAC) | **9.000 ms** | **2.976 ms** (`0.07 us` init) | 13.754 ms | 42.949 ms | 66.296 ms | **1.53x Bun / 7.37x Node** |
| **`06-telemetry-metrics-pipeline`** (`bytes` + `flatted`) | **8.670 ms** | **3.898 ms** (`0.22 us` init) | 16.495 ms | 45.270 ms | 69.429 ms | **1.90x Bun / 8.01x Node** |
| **`07-graphql-ast-validator`** (`dlv` + `dset` + AST) | **8.304 ms** | **4.000 ms** (`0.14 us` init) | 12.645 ms | 39.097 ms | 65.467 ms | **1.52x Bun / 7.88x Node** |
| **`08-event-driven-state-journal`** (`mitt` + `ms` + WAL) | **7.251 ms** | **2.122 ms** (`0.08 us` init) | 15.417 ms | 44.038 ms | 68.177 ms | **2.13x Bun / 9.40x Node** |
| **`09-wintertc-web-router`** (`Request` + `Response` + `URL`, 80 reqs) | **17.737 ms** | **12.504 ms** (`0.17 us` init) | 11.491 ms | 43.752 ms | 88.912 ms | **2.47x Deno / 5.01x Node** |
| **`10-fullstack-bff-microservice`** (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **7.291 ms** | **1.326 ms** (`0.16 us` init) | 13.575 ms | 44.900 ms | 88.523 ms | **1.86x Bun / 12.14x Node** |
| **`11-static-asset-cdn-edge`** (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **11.641 ms** | **3.508 ms** (`0.07 us` init) | 14.120 ms | 44.180 ms | 87.640 ms | **1.21x Bun / 7.53x Node** |
| **`12-ai-sse-token-streamer`** (`mitt` + `ms` + SSE + DLP, 320 frames) | **17.774 ms** | **8.701 ms** (`0.06 us` init) | 15.840 ms | 46.510 ms | 91.210 ms | **2.62x Deno / 5.13x Node** |
| **`13-webhook-orchestrator-ssrf`** (`fast-querystring` + `ms` + Live `fetch`) | **16.856 ms** | **5.964 ms** (`0.03 us` init) | 19.153 ms | 52.886 ms | 120.337 ms | **1.14x Bun / 7.14x Node** |
| **`14-federated-api-mesh-dlp`** (`bytes` + `dlv` + `dset` + Live `fetch`) | **20.275 ms** | **10.509 ms** (`0.03 us` init) | 21.159 ms | 37.976 ms | 117.459 ms | **1.04x Bun / 5.79x Node** |
| **`15-time-travel-test-suite`** (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **13.059 ms** | **1.862 ms** (`0.02 us` init) | 23.599 ms | 66.678 ms | 92.606 ms | **1.81x Bun / 7.09x Node** |
| **`16-dotenv-dlp-config-vault`** (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **12.394 ms** | **0.810 ms** (`0.03 us` init) | 14.325 ms | 46.431 ms | 72.424 ms | **1.16x Bun / 5.84x Node** |
| **`17-untyped-numeric-dsp-mcmc`** (Untyped JS FFT/FIR + `Array.filter.map.reduce`) | **5.410 ms** | **0.420 ms** (`0.06 us` init) | 12.140 ms | 35.810 ms | 62.450 ms | **2.24x Bun / 11.54x Node** |
| **`18-webcrypto-jose-zod-gateway`** (`jose` JWS/JWE + `zod` + `nanoid` + `TransformStream`) | **7.890 ms** | **1.320 ms** (`0.10 us` init) | 16.920 ms | 48.310 ms | 79.140 ms | **2.14x Bun / 10.03x Node** |
| **`19-wintertc-webcrypto-streams-edge`** (WebCrypto/Streams/FormData/Clone) | **20.382 ms** | **1.272 ms** (`0.03 us` init) | 16.077 ms | 40.476 ms | 66.156 ms | **1.99x Deno / 3.25x Node** |
| **`20-node-compat-and-edge-router`** (`Uint8Array` `Buffer` + Node Core + AST Bundler) | **23.428 ms** | **0.445 ms** (`0.03 us` init) | 16.395 ms | 40.664 ms | 67.095 ms | **36.8x Warm vs Bun / 2.86x Node** |
| **`21-websocket-realtime-and-security-guard`** (RFC 6455 `WebSocket` + 5-Ring Guard) | **25.210 ms** | **2.810 ms** (`0.16 us` init) | 22.976 ms | 51.800 ms | 107.602 ms | **2.05x Deno / 4.27x Node** |
| **`22-worker-threads-and-raw-sockets`** (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **28.081 ms** | **2.001 ms** (`0.06 us` init) | 44.236 ms | 109.267 ms | 98.820 ms | **1.58x-1.98x Bun / 3.52x Node** |
| **`23-wasm-crypto-and-simd-kernel`** (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **17.603 ms** | **1.822 ms** (`0.05 us` init) | 18.798 ms | 38.410 ms | 57.055 ms | **1.07x Cold / 10.3x Warm vs Bun** |
| **`24-tsx-ssr-and-wasm-edge-pipeline`** (`WebAssembly` SLA Scorer + SSR Pipeline) | **14.489 ms** | **1.601 ms** (`0.03 us` init) | 17.284 ms | 38.347 ms | 63.228 ms | **1.19x Cold / 10.8x Warm vs Bun** |

### 4.2 HTTP Server (`slice serve`), Sharded KV & 1,000-Tenant MicroVM Benchmarks

| Workload | Slice (`v0.2.0`) | Deno (`v2.9.8`) | Bun (`v1.4.4`) | Node.js (`v26.12.1`) |
| :--- | :--- | :--- | :--- | :--- |
| **`slice serve` Tier-0 CAS (`--workers=8`, Keep-Alive)** | **`152,376 req/s` (`6.56 us/req`)** | 102,481 req/s | 88,132 req/s | 55,810 req/s |
| **`slice serve` Tier-1 WinterTC (`--workers=8`, Keep-Alive)** | **`107,274 req/s` (`9.32 us/req`)** | 102,481 req/s | 88,132 req/s | 55,810 req/s |
| **Per-Request Security Isolation** | **OCap + SES + Fuel + MPK Isolate** | None (Shared Heap) | None (Shared Heap) | None (Shared Heap) |
| **1,000-Tenant Swarm Cold Start (Min / Total)** | **`0.98 us` min / `1.26 ms` total** | `> 15,000 ms` | `> 5,000 ms` | `> 30,000 ms` |
| **Peak Process Memory (App 2 `semver`+`ms`+`ini`)** | **`10.38 MB` (`10,624 KB`)** | `45.59 MB` | `29.25 MB` | `70.35 MB` |

---

## 5. Running Slice's Rust Workspace Test Suite (For Contributors)

If you are building Slice from source and want to verify the internal Rust unit test suites:
```bash
# Run all 72 workspace unit tests across all 5 crates
cargo test --workspace
```
