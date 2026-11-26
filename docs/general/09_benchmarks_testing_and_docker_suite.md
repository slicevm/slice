# Chapter 09: Time-Travel Testing (`slice test`), Deterministic Replay & Production Benchmarks

In traditional test runners (`jest`, `vitest`, `bun test`, `deno test`), if Test #1 writes a temporary file to disk or mutates a shared key-value store and fails mid-test, Test #2 inherits dirty state - leading to flaky tests that pass in isolation and fail in CI. Furthermore, no standard test runner lets you assert that a function stays within a strict **CPU instruction fuel budget**, **memory ceiling**, or **Object-Capability security policy**.

Slice solves both problems with **`slice test`: the Time-Travel Test Runner**.

This chapter shows you how to write unit, integration, and security tests with `slice test`, how to eliminate flaky time/randomness bugs with `--deterministic`, and how to run Slice's 33-workload Docker benchmark suite against Node.js, Deno, and Bun.

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
3. **`setTimeout(fn, delayMs)` and `setInterval(fn, delayMs)`** execute via **Deterministic Virtual Time-Warp (`0 ns` wall-clock sleep)**: a `setTimeout(fn, 5000)` advances the virtual clock by `+5000 ms` and runs `fn` immediately without sleeping the OS thread!
4. **Outbound `fetch()`** automatically records upstream HTTP responses into `Slice.kv` (`fetch_cassette:<METHOD>:<URL>`) and replays them deterministically.

---

## 4. Running the 33-Workload Docker Benchmark Suite (`tests/`)

Slice includes a comprehensive Linux `x86_64` Docker benchmark suite ([`tests/`](../../tests/README.md)) that tests **33 realistic multi-file TypeScript + `node_modules` applications, full-stack edge frameworks (`Hono`, `itty-router + UnJS`, `Express/Polka`, `SvelteKit 3 + Svelte 5`), and architectural case studies** against the latest official releases of **Bun**, **Deno**, and **Node.js**, verifying **100% byte-for-byte identical JSON output** across all 4 runtimes and automatically writing timestamped reports to [`tests/reports/`](../../tests/reports/latest.md).

```bash
# Run the automated benchmark suite & doc sync (PowerShell or Bash)
./scripts/run_benchmarks.sh

# Or run specific tagged suites:
./scripts/run_benchmarks.sh "apps,frameworks,case_studies,serve,swarm"
```

### 4.1 Verified Results Across All 33 Production Applications, Frameworks & Case Studies ([`tests/reports/latest.md`](../../tests/reports/latest.md))

<!-- BENCHMARK_SUMMARY_START -->
> **Latest Verified Benchmark Report:** **[`tests/reports/latest.md`](../../tests/reports/latest.md)** (Archive: [`bench_v0.14.4_20261126_003136Z_91d9e60.md`](../../tests/reports/bench_v0.14.4_20261126_003136Z_91d9e60.md))  
> **Provenance:** Slice `v0.14.4` (`91d9e60`) | Generated `2026-11-26T00:31:36Z` | Environment: `Linux x86_64 Docker (io_uring SQPOLL + WRPKRU)`  
> **Scorecard Across All 33 Production Workloads:** Beats **Deno (`v2.9.8`)** on **33/33**, **Node.js (`v27.0.0-alpha.0.1.1`)** on **33/33**, **Bun (`v1.4.5`)** on **33/33** cold one-shot, and **all 3 runtimes on 33/33 workloads in `--warm` pooled execution**.

#### 1. Core Architecture, HTTP Throughput & 1,000-Tenant MicroVM Density

| Metric | Slice (`v0.14.4` `SLICEBC\x01`) | Bun (`v1.4.5`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha.0.1.1`) | Slice Advantage |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **In-Process Isolate Cold Start (`P50` / `P99`)** | **`0.86 us` / `1.21 us`** | N/A (Process/VM) | N/A (Process/VM) | N/A (Process/VM) | **~120,000x faster than 125ms Firecracker VM** |
| **1,000-Tenant MicroVM Swarm (Boot + Exec)** | **`1.10 ms` (`8.12 MB` RSS)** | > 5,000 ms | > 15,000 ms | > 30,000 ms | **> 3,900x faster / ~32,000x denser** |
| **500-Module Monorepo Cold Boot (`stat()` Calls)** | **`3.07 ms` (`0 stat()`)** | `12.72 ms` (`500+ stat`) | `41.80 ms` (`500+ stat`) | `70.38 ms` (`1,500+ stat`) | **`4.1x` faster than Bun / `22.9x` Node.js** |
| **HTTP/1.1 Keep-Alive Throughput (`16k` Reqs)** | **`158,118 req/s`** (Tier-0)<br>**`153,531 req/s`** (Tier-1) | `99,946 req/s` | `95,072 req/s` | `54,802 req/s` | **`1.58x` (`Tier-0`) / `1.54x` (`Tier-1`) vs `Bun.serve`** |
| **Peak Process Memory (`Max RSS` Across Suite)** | **`13.63 - 21.85 MB`** | `34.88 - 49.63 MB` | `46.17 - 62.13 MB` | `68.72 - 91.35 MB` | **`1.5x-2.4x` less than Bun / `3.9x-6.8x` less than Node** |
| **Multi-Tenant Security Containment** | **`5 / 5 PASS`** (OCap+SES+Fuel+DLP) | `0 / 5` (Ambient) | `2 / 5` (Coarse CLI) | `0 / 5` (Ambient) | **Hardware & Language Zero-Trust Isolation** |

#### 2. Real-Life Workload Summary by Domain (All 33 Workloads Aggregated)

| Workload Domain & Stack Coverage | Suites | Slice Cold `.cas` Avg (Range) | Slice Warm (`--warm`) Avg | Bun `v1.4.5` Avg | Deno `v2.9.8` Avg | Node.js `v27.0.0-alpha.0.1.1` Avg | Category Speedup vs Bun / Node.js |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **Core Edge, Auth & Crypto (`Apps 01, 04, 05, 16`)**<br><sub>Multi-file TS, SHA-256/HMAC, JWT/OAuth2, `.env` Auto-DLP</sub> | `4` | **`6.78 ms`** (`5.22-8.72 ms`) | **`1.94 ms`** | `15.85 ms` | `42.46 ms` | `59.01 ms` | **`2.34x Bun` (`8.17x` Warm) / `8.70x Node`** |
| **NPM Ecosystem, AST & Disk I/O (`Apps 02, 03, 06, 07, 08, 20`)**<br><sub>`semver`, `flatted`, `dlv`/`dset`, `mitt`, 10-Shard JSONL I/O, `Buffer`</sub> | `6` | **`8.60 ms`** (`6.17-11.69 ms`) | **`2.50 ms`** | `19.02 ms` | `47.86 ms` | `60.69 ms` | **`2.21x Bun` (`7.59x` Warm) / `7.05x Node`** |
| **WinterTC Web Standards, Fetch Mesh & DLP (`Apps 09-14, 17-19`)**<br><sub>`Request`/`Response`, Static CDN `304`, SSE, Live `fetch`, WebCrypto</sub> | `9` | **`13.35 ms`** (`8.47-29.45 ms`) | **`6.35 ms`** | `19.80 ms` | `53.90 ms` | `91.07 ms` | **`1.48x Bun` (`3.12x` Warm) / `6.82x Node`** |
| **Realtime WebSockets, Workers & Wasm/TSX (`Apps 15, 21-24`)**<br><sub>RFC 6455 WS, `worker_threads` + TCP/UDP, Pure-Rust `wasmi` Wasm, TSX SSR</sub> | `5` | **`14.39 ms`** (`7.13-28.54 ms`) | **`2.26 ms`** | `30.64 ms` | `65.11 ms` | `82.08 ms` | **`2.13x Bun` (`13.57x` Warm) / `5.71x Node`** |
| **Full-Stack Edge & Node Frameworks (`FW 01-04`)**<br><sub>`Hono v4.6.20`, `itty-router + UnJS Nitro`, `Express/Polka node:http`, `SvelteKit 3.1.1 + Svelte 5.57.3`</sub> | `4` | **`18.16 ms`** (`13.08-28.90 ms`) | **`7.26 ms`** | `30.59 ms` | `58.12 ms` | `90.77 ms` | **`1.68x Bun` (`4.21x` Warm) / `5.00x Node`** |
| **AI Agent, Multi-Tenant SaaS & Chaos Case Studies (`CS 01-05`)**<br><sub>Depth-4 ToT Agent, MCP Code Mode, 12-Tenant Mesh, Wasm CDR, 48 Payment Sagas</sub> | `5` | **`15.73 ms`** (`9.25-26.78 ms`) | **`4.76 ms`** | `21.52 ms` | `55.60 ms` | `73.49 ms` | **`1.37x Bun` (`4.52x` Warm) / `4.67x Node`** |

#### 3. Flagship Real-World Representative Workloads (9 of 33 Highlights)

| # | Flagship Workload | Slice Cold `.cas` | Slice Warm (`--warm`) | Bun `v1.4.5` | Deno `v2.9.8` | Node.js `v27.0.0-alpha.0.1.1` | Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`5.22 ms`** | **`0.92 ms`** | `14.78 ms` | `36.48 ms` | `54.46 ms` | **`2.83x Bun` (`16.00x` Warm) / `10.44x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`6.18 ms`** | **`1.15 ms`** | `16.33 ms` | `43.93 ms` | `55.84 ms` | **`2.64x Bun` (`14.23x` Warm) / `9.03x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`12.99 ms`** | **`7.34 ms`** | `22.37 ms` | `53.56 ms` | `109.00 ms` | **`1.72x Bun` (`3.05x` Warm) / `8.39x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`9.05 ms`** | **`0.43 ms`** | `23.33 ms` | `54.46 ms` | `67.84 ms` | **`2.58x Bun` (`54.14x` Warm) / `7.49x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`28.54 ms`** | **`2.54 ms`** | `60.44 ms` | `119.87 ms` | `95.21 ms` | **`2.12x Bun` (`23.82x` Warm) / `3.34x Node`** |
| **FW-01** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`13.08 ms`** | **`5.60 ms`** | `20.85 ms` | `54.13 ms` | `93.26 ms` | **`1.59x Bun` (`3.72x` Warm) / `7.13x Node`** |
| **FW-04** | `FW-04: frameworks/04-sveltekit-edge-ssr` (`@sveltejs/kit@3.1.1` + `svelte@5.57.3` + `devalue@6.0.3`) | **`28.90 ms`** | **`13.60 ms`** | `45.68 ms` | `69.87 ms` | `107.65 ms` | **`1.58x Bun` (`3.36x` Warm) / `3.72x Node`** |
| **CS-01** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`26.78 ms`** | **`12.04 ms`** | `29.77 ms` | `73.20 ms` | `83.48 ms` | **`1.11x Bun` (`2.47x` Warm) / `3.12x Node`** |
| **CS-05** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`9.25 ms`** | **`2.79 ms`** | `22.53 ms` | `64.58 ms` | `79.56 ms` | **`2.44x Bun` (`8.06x` Warm) / `8.60x Node`** |

*Full 33-workload latency breakdown, standalone binary metrics, peak RSS per app, and historical reports: **[`tests/reports/latest.md`](../../tests/reports/latest.md)**.*
<!-- BENCHMARK_SUMMARY_END -->

---

## 5. Running Slice's Rust Workspace Test Suite (For Contributors)

If you are building Slice from source and want to verify the internal Rust unit test suites:
```bash
# Run all workspace unit and integration tests across all crates
cargo test --workspace
```
