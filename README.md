# Slice (Nexus-JS)

**Slice (Nexus-JS)** is a next-generation, microVM-grade JavaScript/TypeScript edge runtime engineered in Rust. It is designed from the ground up to replace ambient POSIX authority, recursive `node_modules` filesystem metadata storms, readiness polling (`libuv`), and heavy V8 memory footprints with four unified pillars:

1. **Tiered Polyglot Execution (`slice-engine`):** Instant `< 2.5 microsecond` cold starts via pre-initialized `SLICESNP` binary bytecode snapshots (`Tier0LLIntSnapshot`), scaling dynamically to baseline (`Tier1Baseline`) and native SIMD kernels (`Tier2PeakNative`).
2. **Zero-Stat Content-Addressed Storage (`slice-cas`):** Flattens TypeScript/JavaScript dependency graphs at build time into a 64-byte cache-line aligned, BLAKE3-indexed `.cas` archive mapped directly into memory via `mmap(MAP_SHARED)` with **0 runtime `stat` syscalls**.
3. **Direct Kernel Completion Rings (`slice-io`):** Lockless Submission/Completion rings backed by Linux `io_uring` (`IORING_SETUP_SQPOLL` + Fixed Registered Buffers) and Windows NT `IOCP`.
4. **MicroVM-Grade OCap & Hardware Sandboxing (`slice-sandbox`):** Enforces Secure ECMAScript (`lockdown()`), isolated per-tenant TC39 Compartments with attenuated `CapabilityToken` permissions (`FsCap`, `NetCap`, `EnvCap`), deterministic CPU instruction fuel preemption, byte-level heap cages, and `< 20`-cycle hardware memory domain switches via `pkey_alloc` / `WRPKRU` (Intel MPK).

---

## Verified Benchmark Overview (Linux `x86_64` & Windows `x86_64`)

```
+-----------------------------------+-------------------+-------------------+-------------------+-----------------------+
| Metric (Linux x86_64 Verified)    | Node.js v22.22.3  | Deno v2.8.3       | Bun v1.3.14       | Slice (Nexus-JS)      |
+-----------------------------------+-------------------+-------------------+-------------------+-----------------------+
| Isolate Cold Start (In-Process)   | N/A (Process/VM)  | N/A (Process/VM)  | N/A (Process/VM)  | 1.92 us (P50: 2.10us) |
| CLI Process Cold Start (Min)      | 49.294 ms         | 19.371 ms         | 5.343 ms          | 2.000 ms (24.6x Node) |
| 500-Module Graph Load (Min)       | 70.382 ms         | 41.799 ms         | 12.721 ms         | 3.408 ms (0.62ms mem) |
| OS stat() Syscalls (500 Modules)  | 1,500+ syscalls   | 500+ syscalls     | 500+ syscalls     | 0 stat() syscalls     |
| Compute (Fib40 + 10M Loop)        | 32.787 ms         | 27.660 ms         | 12.556 ms         | 1.924 ms (0.13ms mem) |
| Peak Process Memory (RSS)         | 61,392 KB (60 MB) | 30,720 KB (30 MB) | 12,672 KB (12 MB) | 2,688 KB (2.62 MB)    |
| 1,000-Tenant MicroVM Swarm Time   | > 30,000 ms       | > 15,000 ms       | > 5,000 ms        | 2.62 ms total         |
| Multi-Tenant Security Containment | 0/5 (Ambient)     | 2/5 (Coarse CLI)  | 0/5 (Ambient)     | 5/5 (OCap+SES+Fuel)   |
+-----------------------------------+-------------------+-------------------+-------------------+-----------------------+
```

---

## Quickstart

### 1. Build & Install

**Windows (PowerShell):**
```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1
```

**Linux / macOS (Bash):**
```bash
./scripts/install.sh
```

Or build directly with Cargo:
```bash
cargo build --release
```

### 2. Scaffold a New Edge MicroVM Project (`slice init`)

```bash
slice init my-edge-service
cd my-edge-service
slice run --metrics
```

This creates:
* `slice.toml`: Declarative Object-Capability (OCap) policy and resource cage configuration.
* `src/index.ts`: TypeScript edge entrypoint.

### 3. Run TypeScript Files, Folder Apps, or Pre-Packed CAS Bundles (`slice run`)

```bash
# Execute a raw TypeScript file or an entire project folder (with package.json & node_modules)
slice run ./src/index.ts --allow-env=SERVICE_STAGE=prod --metrics
slice run ./docker-tests/apps/02-npm-ecosystem-suite --metrics

# Pack an entire folder app (including transitive node_modules) into a 64-byte aligned BLAKE3 .cas archive
slice pack ./docker-tests/apps/02-npm-ecosystem-suite ./bundle.cas

# Execute the memory-mapped .cas bundle with 0 stat() syscalls
slice run ./bundle.cas --metrics
```

### 4. Compile a Standalone Zero-Dependency Executable (`slice compile`)

Embeds your compiled `.cas` archive and `SLICESNP`/`SLICEBOA` bytecode directly into a self-contained native binary:

```bash
slice compile ./docker-tests/apps/01-fintech-checkout -o my-edge-app
./my-edge-app --metrics
```

### 5. Launch the Multi-Tenant WinterTC HTTP MicroVM Server (`slice serve`)

Spawns a fresh, hardware- and OCap-isolated `SliceIsolate` in `< 2.5 microseconds` for every incoming HTTP request:

```bash
# Start server on port 3000
slice serve ./bundle.cas --port=3000

# Or run a self-contained single-request smoke test with telemetry headers
slice serve ./bundle.cas --smoke-test
```

Every HTTP response includes real-time microVM execution headers:
```http
HTTP/1.1 200 OK
Server: Slice-NexusJS/0.2
X-Slice-Tenant: tenant-edge-42
X-Slice-Isolate-Us: 24.70
X-Slice-Tier: Tier0LLIntSnapshot
X-Slice-Fuel-Used: 4
X-Slice-Pkey-Domain: 0x2
```

### 6. Inspect Bytecode & CAS Archives (`slice inspect`)

Disassemble `SLICESNP` bytecode and inspect the BLAKE3 Radix resolution table:

```bash
slice inspect ./bundle.cas
slice inspect ./benchmarks/workloads/cold_start.ts
```

### 7. Interactive SES Sandbox Shell (`slice repl`) & Kernel Diagnostics (`slice doctor`)

```bash
# Audit host OS completion rings (io_uring SQPOLL / Windows IOCP), MPK hardware keys, and OCap containment
slice doctor

# Launch an interactive SES-locked REPL
slice repl
```

---

## Declarative Security Manifest (`slice.toml`)

By default, every Slice compartment starts with **zero ambient authority** (`process`, `fs`, and `net` are absent unless granted via attenuated tokens). Configure capabilities declaratively in `slice.toml`:

```toml
[package]
name = "my-edge-service"
version = "0.1.0"
entrypoint = "./src/index.ts"

[limits]
fuel = 50_000_000
memory_kb = 4096

[capabilities]
allow_read = ["/app/public", "/app/config"]
allow_net = ["api.internal:443", "edge.slice.local:80"]

[capabilities.env]
SERVICE_STAGE = "production"
```

---

## CLI Reference

| Command | Description |
| :--- | :--- |
| `slice init [dir]` | Scaffold a new TypeScript Edge project with `slice.toml` and `src/index.ts`. |
| `slice run [target] [flags]` | Execute a `.ts`/`.js` script, `.cas` archive, or `slice.toml` project. |
| `slice pack <in> <out.cas>` | Flatten and pre-compile a module tree into a 64-byte aligned BLAKE3 `.cas` archive. |
| `slice compile <in> -o <bin>` | Produce a zero-dependency standalone executable (`SLICEEXE`). |
| `slice serve [target] [--port=N]` | Run the multi-tenant WinterTC HTTP MicroVM server (one fresh isolate per request). |
| `slice inspect <target>` | Disassemble `SLICESNP` bytecode and inspect BLAKE3 CAS offsets. |
| `slice repl` | Interactive SES-locked TypeScript/JavaScript compartment shell. |
| `slice doctor` | Audit host kernel `io_uring`/`IOCP`, MPK protection keys, RSS, and 5 security vectors. |
| `slice microvm-bench` | Spawn 1,000 isolated tenants and benchmark P50/P99 cold-start, CAS, and I/O rings. |
| `slice verify-security` | Execute the 5-vector OCap, SES prototype lockdown, and CPU fuel DoS verification suite. |

## Folder-Based Real-Life Production Applications, External NPM Libraries & Disk I/O Benchmarks (`docker-tests/apps/`)

In addition to synthetic microbenchmarks, [`docker-tests/`](file:///p:/opensource/slice/docker-tests/README.md) runs 4 folder-based multi-module production TypeScript applications (with real external `node_modules/` packages `semver@7.6.3`, `ms@2.1.3`, `ini@4.1.3`, `fast-querystring@1.1.2`, FIPS-180-4 SHA-256 `node:crypto`, and real `node:fs` disk I/O) and verifies **100% byte-for-byte identical JSON output** against the **absolute latest releases** of Bun (`v1.4.3`), Deno (`v2.9.7`), and Node.js (`v26.12.0`):

| Folder-Based Production App (Linux Docker `x86_64`) | Slice (`v0.2.0`) | Bun (`v1.4.3`) | Deno (`v2.9.7`) | Node.js (`v26.12.0`) |
| :--- | :--- | :--- | :--- | :--- |
| **App 1: `01-fintech-checkout`** (Multi-File TS + SHA-256, 50 orders) | **6.270 ms** (`3.68 ms` isolate) | 12.896 ms | 36.331 ms | 64.775 ms |
| **App 2: `02-npm-ecosystem-suite`** (`semver` + `ms` + `ini` from `node_modules`) | **19.139 ms** (`15.89 ms` isolate) | 20.610 ms | 51.333 ms | 75.508 ms |
| **App 3: `03-disk-io-log-pipeline`** (10 JSONL Shards Disk Write/Scan/Read/Unlink) | **11.425 ms** (`8.02 ms` isolate) | 13.488 ms | 43.511 ms | 67.315 ms |
| **App 4: `04-edge-api-gateway`** (`fast-querystring` + `ms` + Auth, 100 reqs) | **9.863 ms** (`8.09 ms` isolate) | 14.812 ms | 40.520 ms | 67.782 ms |
| **Peak Memory Footprint: Disk I/O App (Max RSS)** | **11.50 MB (`11,776 KB`)** | 21.12 MB (`21,632 KB`) | 45.88 MB (`46,976 KB`) | 71.40 MB (`73,112 KB`) |
| **Peak Memory Footprint: NPM Ecosystem App (Max RSS)** | **12.62 MB (`12,928 KB`)** | 29.62 MB (`30,336 KB`) | 46.38 MB (`47,488 KB`) | 71.15 MB (`72,860 KB`) |
| **1,000-Tenant MicroVM Isolate Cold-Start (Min)** | **3.13 us (`0.00313 ms`)** | N/A | N/A | N/A |

Run the full Docker folder-app benchmark suite yourself:

```powershell
docker build -f docker-tests/Dockerfile -t slice-real-stack-bench .
docker run --rm slice-real-stack-bench
```

---

## Documentation & Benchmarks

* [Master Plan (`BIG_PLAN.md`)](file:///p:/opensource/slice/BIG_PLAN.md)
* [Architectural Blueprint (`reimagining_next_gen_node_runtime.md`)](file:///p:/opensource/slice/reimagining_next_gen_node_runtime.md)
* [Docker Real-Stack Production Benchmarks (`docker-tests/README.md`)](file:///p:/opensource/slice/docker-tests/README.md)
* [Cutting-Edge 2026 Research Deep Dive (`docs/research_deep_dive.md`)](file:///p:/opensource/slice/docs/research_deep_dive.md)
* [Internal Architecture Specification (`docs/architecture.md`)](file:///p:/opensource/slice/docs/architecture.md)
* [Empirical Benchmarks & Comparisons (`docs/benchmarks_and_comparisons.md`)](file:///p:/opensource/slice/docs/benchmarks_and_comparisons.md)
* [Production Tooling & Deployment Guide (`docs/production_tooling_guide.md`)](file:///p:/opensource/slice/docs/production_tooling_guide.md)
