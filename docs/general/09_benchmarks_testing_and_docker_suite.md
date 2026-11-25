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
> **Latest Verified Benchmark Report:** **[`tests/reports/latest.md`](../../tests/reports/latest.md)** (Archive: [`bench_v0.14.3_20261125_165349Z_c4acef3.md`](../../tests/reports/bench_v0.14.3_20261125_165349Z_c4acef3.md))  
> **Provenance:** Slice `v0.14.3` (`c4acef3`) | Generated `2026-11-25T16:53:49Z` | Environment: `Linux x86_64 Docker (io_uring SQPOLL + WRPKRU)`  
> **Scorecard Across All 33 Production Workloads:** Beats **Deno (`v2.9.8`)** on **33/33**, **Node.js (`v27.0.0-alpha.0.1.1`)** on **33/33**, **Bun (`v1.4.5`)** on **33/33** cold one-shot, and **all 3 runtimes on 33/33 workloads in `--warm` pooled execution**.

#### 1. Core Architecture, HTTP Throughput & 1,000-Tenant MicroVM Density

| Metric | Slice (`v0.14.3` `SLICEBC\x01`) | Bun (`v1.4.5`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha.0.1.1`) | Slice Advantage |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **In-Process Isolate Cold Start (`P50` / `P99`)** | **`0.87 us` / `0.98 us`** | N/A (Process/VM) | N/A (Process/VM) | N/A (Process/VM) | **~120,000x faster than 125ms Firecracker VM** |
| **1,000-Tenant MicroVM Swarm (Boot + Exec)** | **`1.05 ms` (`8.12 MB` RSS)** | > 5,000 ms | > 15,000 ms | > 30,000 ms | **> 3,900x faster / ~32,000x denser** |
| **500-Module Monorepo Cold Boot (`stat()` Calls)** | **`2.86 ms` (`0 stat()`)** | `12.72 ms` (`500+ stat`) | `41.80 ms` (`500+ stat`) | `70.38 ms` (`1,500+ stat`) | **`4.4x` faster than Bun / `24.6x` Node.js** |
| **HTTP/1.1 Keep-Alive Throughput (`16k` Reqs)** | **`151,508 req/s`** (Tier-0)<br>**`141,543 req/s`** (Tier-1) | `92,023 req/s` | `95,093 req/s` | `54,625 req/s` | **`1.65x` (`Tier-0`) / `1.54x` (`Tier-1`) vs `Bun.serve`** |
| **Peak Process Memory (`Max RSS` Across Suite)** | **`13.75 - 21.72 MB`** | `36.31 - 49.13 MB` | `45.73 - 61.85 MB` | `68.54 - 90.35 MB` | **`1.5x-2.4x` less than Bun / `3.9x-6.8x` less than Node** |
| **Multi-Tenant Security Containment** | **`5 / 5 PASS`** (OCap+SES+Fuel+DLP) | `0 / 5` (Ambient) | `2 / 5` (Coarse CLI) | `0 / 5` (Ambient) | **Hardware & Language Zero-Trust Isolation** |

#### 2. Real-Life Workload Summary by Domain (All 33 Workloads Aggregated)

| Workload Domain & Stack Coverage | Suites | Slice Cold `.cas` Avg (Range) | Slice Warm (`--warm`) Avg | Bun `v1.4.5` Avg | Deno `v2.9.8` Avg | Node.js `v27.0.0-alpha.0.1.1` Avg | Category Speedup vs Bun / Node.js |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **Core Edge, Auth & Crypto (`Apps 01, 04, 05, 16`)**<br><sub>Multi-file TS, SHA-256/HMAC, JWT/OAuth2, `.env` Auto-DLP</sub> | `4` | **`6.62 ms`** (`5.12-8.60 ms`) | **`1.98 ms`** | `15.45 ms` | `42.67 ms` | `57.41 ms` | **`2.33x Bun` (`7.80x` Warm) / `8.67x Node`** |
| **NPM Ecosystem, AST & Disk I/O (`Apps 02, 03, 06, 07, 08, 20`)**<br><sub>`semver`, `flatted`, `dlv`/`dset`, `mitt`, 10-Shard JSONL I/O, `Buffer`</sub> | `6` | **`8.61 ms`** (`6.06-11.67 ms`) | **`2.50 ms`** | `18.79 ms` | `48.15 ms` | `59.95 ms` | **`2.18x Bun` (`7.52x` Warm) / `6.96x Node`** |
| **WinterTC Web Standards, Fetch Mesh & DLP (`Apps 09-14, 17-19`)**<br><sub>`Request`/`Response`, Static CDN `304`, SSE, Live `fetch`, WebCrypto</sub> | `9` | **`13.14 ms`** (`8.56-28.02 ms`) | **`6.09 ms`** | `19.49 ms` | `52.83 ms` | `88.04 ms` | **`1.48x Bun` (`3.20x` Warm) / `6.70x Node`** |
| **Realtime WebSockets, Workers & Wasm/TSX (`Apps 15, 21-24`)**<br><sub>RFC 6455 WS, `worker_threads` + TCP/UDP, Pure-Rust `wasmi` Wasm, TSX SSR</sub> | `5` | **`13.88 ms`** (`7.31-28.34 ms`) | **`2.20 ms`** | `29.62 ms` | `63.72 ms` | `80.20 ms` | **`2.13x Bun` (`13.44x` Warm) / `5.78x Node`** |
| **Full-Stack Edge & Node Frameworks (`FW 01-04`)**<br><sub>`Hono v4.6.20`, `itty-router + UnJS Nitro`, `Express/Polka node:http`, `SvelteKit 3.1.1 + Svelte 5.57.3`</sub> | `4` | **`17.87 ms`** (`13.48-28.51 ms`) | **`6.99 ms`** | `30.16 ms` | `58.35 ms` | `88.01 ms` | **`1.69x Bun` (`4.32x` Warm) / `4.93x Node`** |
| **AI Agent, Multi-Tenant SaaS & Chaos Case Studies (`CS 01-05`)**<br><sub>Depth-4 ToT Agent, MCP Code Mode, 12-Tenant Mesh, Wasm CDR, 48 Payment Sagas</sub> | `5` | **`15.88 ms`** (`8.77-27.55 ms`) | **`4.60 ms`** | `21.08 ms` | `54.44 ms` | `71.80 ms` | **`1.33x Bun` (`4.58x` Warm) / `4.52x Node`** |

#### 3. Flagship Real-World Representative Workloads (9 of 33 Highlights)

| # | Flagship Workload | Slice Cold `.cas` | Slice Warm (`--warm`) | Bun `v1.4.5` | Deno `v2.9.8` | Node.js `v27.0.0-alpha.0.1.1` | Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`5.12 ms`** | **`0.78 ms`** | `14.14 ms` | `36.46 ms` | `54.87 ms` | **`2.76x Bun` (`18.15x` Warm) / `10.72x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`6.06 ms`** | **`1.15 ms`** | `15.08 ms` | `43.44 ms` | `56.42 ms` | **`2.49x Bun` (`13.16x` Warm) / `9.31x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`13.59 ms`** | **`7.20 ms`** | `22.76 ms` | `55.77 ms` | `102.80 ms` | **`1.70x Bun` (`3.16x` Warm) / `7.69x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`9.03 ms`** | **`0.39 ms`** | `23.55 ms` | `55.47 ms` | `68.25 ms` | **`2.61x Bun` (`60.53x` Warm) / `7.56x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`28.72 ms`** | **`2.35 ms`** | `58.53 ms` | `117.15 ms` | `93.79 ms` | **`2.07x Bun` (`24.91x` Warm) / `3.31x Node`** |
| **FW-01** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`13.62 ms`** | **`4.91 ms`** | `20.65 ms` | `52.65 ms` | `86.13 ms` | **`1.53x Bun` (`4.20x` Warm) / `6.39x Node`** |
| **FW-04** | `FW-04: frameworks/04-sveltekit-edge-ssr` (`@sveltejs/kit@3.1.1` + `svelte@5.57.3` + `devalue@6.0.3`) | **`28.51 ms`** | **`13.62 ms`** | `44.88 ms` | `69.07 ms` | `105.17 ms` | **`1.57x Bun` (`3.30x` Warm) / `3.69x Node`** |
| **CS-01** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`27.55 ms`** | **`12.01 ms`** | `29.83 ms` | `72.50 ms` | `82.39 ms` | **`1.08x Bun` (`2.48x` Warm) / `2.99x Node`** |
| **CS-05** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`8.77 ms`** | **`2.45 ms`** | `21.07 ms` | `63.48 ms` | `77.83 ms` | **`2.40x Bun` (`8.61x` Warm) / `8.87x Node`** |

*Full 33-workload latency breakdown, standalone binary metrics, peak RSS per app, and historical reports: **[`tests/reports/latest.md`](../../tests/reports/latest.md)**.*
<!-- BENCHMARK_SUMMARY_END -->

---

## 5. Running Slice's Rust Workspace Test Suite (For Contributors)

If you are building Slice from source and want to verify the internal Rust unit test suites:
```bash
# Run all workspace unit and integration tests across all crates
cargo test --workspace
```
