# Chapter 01: Getting Started, Project Configuration & Complete CLI Reference

This chapter covers building and installing **Slice (Nexus-JS)** from source, scaffolding your first TypeScript edge service, configuring declarative capabilities in `slice.toml`, and using all 13 CLI subcommands.

---

## 1. Prerequisites & Installation

Slice is written in 100% pure Rust (requiring zero external C/C++ compilers or V8 prebuilts) and compiles to a single native executable (`slice` on Linux/macOS, `slice.exe` on Windows).

### 1.1 Automated Install Scripts

**Windows (PowerShell):**
```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1
```

**Linux / macOS (Bash):**
```bash
./scripts/install.sh
```

### 1.2 Building Directly with Cargo

```bash
cargo build --release
```

The release profile in [`Cargo.toml`](file:///p:/opensource/slice/Cargo.toml) enables full Link-Time Optimization (`lto = "fat"`), `opt-level = 3`, `codegen-units = 1`, `panic = "abort"`, and symbol stripping, producing an optimized binary at `./target/release/slice` (or `.\target\release\slice.exe`).

### 1.3 Verifying Host Kernel & Hardware Support (`slice doctor` & `slice info`)

Run `slice doctor` and `slice info` to inspect your OS completion rings (`io_uring` on Linux, `IOCP` on Windows), hardware memory protection keys (`MPK/PKU`), pre-warmed `Boa` ES2024 realm pool, and 5-vector security containment:

```bash
slice info
slice doctor
```

Example output:
```text
=========================================================================
  SLICE (NEXUS-JS) PRODUCTION ENVIRONMENT & KERNEL DOCTOR
=========================================================================
  [OK] Host OS / Architecture     : windows / x86_64
  [OK] Direct Completion Ring     : Windows NT I/O Completion Port (IOCP Direct Kernel Ring)
  [OK] Kernel SQPOLL Ring Mode    : Standard Completion Ring (IOCP / Direct Ring)
  [OK] Registered Fixed Slab Bufs : 128 x 64 KB (8 MB Pre-Registered)
  [OK] Memory Protection Domain   : PKEY 0x2 (PKRU Mask 0xffffffcf, Hardware MPK: false)
  [OK] CAS Cache-Line Alignment   : 64 bytes (Zero-Copy mmap ready)
  [OK] Pre-Warmed Boa ES2024 Pool : 2 Ready Realms (Zero-Init SLICEBOA Fast Path)
  [OK] Current Host Process RSS   : 15812 KB (15.44 MB)
  [OK] OCap / SES Security Audit  : 5/5 Attack Vectors Blocked
=========================================================================
```

---

## 2. Scaffolding a New Edge Project (`slice init`)

Create a new project directory with a declarative `slice.toml` security manifest and TypeScript entrypoint:

```bash
slice init ./my-edge-service
cd ./my-edge-service
slice run --metrics
```

This generates two files:
1. `slice.toml`: Declarative Object-Capability (OCap) permissions, CPU fuel limit, and memory ceiling.
2. `src/index.ts`: Starter TypeScript edge script.

---

## 3. Declarative Manifest Reference (`slice.toml`)

When `slice run` or `slice serve` executes in a directory containing `slice.toml` (parsed by [`SliceManifest::parse_toml`](file:///p:/opensource/slice/crates/slice-sandbox/src/lib.rs#L316-L382)), Slice automatically loads the entrypoint, resource limits, and capability tokens:

```toml
[package]
name = "my-edge-service"
version = "0.1.0"
entrypoint = "./src/index.ts"

[limits]
# Maximum deterministic CPU instruction fuel per isolate invocation
fuel = 50_000_000
# Maximum per-tenant memory ceiling in kilobytes (4096 KB = 4 MB)
memory_kb = 4096

[capabilities]
# Allowed filesystem read path prefixes (normalized, blocks '..' traversal)
allow_read = ["/app/public", "/app/config"]
# Allowed filesystem write path prefixes
allow_write = ["/tmp/staging"]
# Allowed outbound network host:port endpoints
allow_net = ["api.internal:443", "edge.slice.local:80"]
# Optional .env file automatically loaded into capabilities.env with Auto-DLP secret registration
env_file = ".env"

[capabilities.env]
# Explicit environment variables injected into the tenant compartment
SERVICE_STAGE = "production"
REGION = "us-east-1"
```

---

## 4. Complete CLI Command & Flag Reference

Implemented in [`crates/slice-cli/src/main.rs`](file:///p:/opensource/slice/crates/slice-cli/src/main.rs):

### 4.1 `slice init [dir]`
Scaffolds a new TypeScript project with `slice.toml` and `src/index.ts` in `[dir]` (defaults to `.`).

### 4.2 `slice run [target] [flags]`
Executes a single `.ts`/`.js` script, a folder application (containing `package.json`, `slice.toml`, or `src/index.ts` + `node_modules/`), or a pre-packed `.cas` archive. If `[target]` is omitted, reads `entrypoint` from `./slice.toml`.

| Flag | Description |
| :--- | :--- |
| `--allow-read=<prefix>` | Grant read access to paths starting with `<prefix>`. Can be specified multiple times. |
| `--allow-write=<prefix>` | Grant write/append/mkdir/unlink access to paths starting with `<prefix>`. |
| `--allow-net=<host:port>` | Grant outbound network access to `<host:port>`. |
| `--allow-env=<KEY=VAL>` | Inject an explicit `KEY=VAL` environment variable into the isolate. |
| `--env-file=<path>` | Load `KEY=VALUE` pairs from a `.env` file into `process.env` / `env()` and automatically register sensitive keys (`*_SECRET*`, `*_TOKEN*`, `*_KEY*`, `*_PASSWORD*`, `*_PRIVATE*`) into the Egress DLP guard. |
| `--watch` / `--watch-once` | Poll source files (`mtime` + size) and live-reload via Oxc in `< 3 ms` when a `.ts`/`.js`/`slice.toml`/`.env` file changes (`--watch-once` exits after 1 reload). |
| `--fuel=<u64>` | Override the maximum CPU instruction fuel budget (default: `50,000,000`). |
| `--memory=<bytes>` | Override the maximum per-isolate memory ceiling in bytes (default: `16,777,216`). |
| `--warm` | Pre-warm the thread-local `BOA_CONTEXT_POOL` and `__slice_rc` bundle closure before measurement. |
| `--deterministic[=<seed>]` | Enable deterministic replay mode (`Math.random()`, `Date.now()`, `performance.now()`) using SplitMix64 with `<seed>` (default: `42`). |
| `--audit-egress=<secret>` | Inspect `stdout`, `returnValue`, and outbound `fetch()` for raw, Base64, or Base64URL leaks of `<secret>` and trap on match. |
| `--metrics` | Print microsecond instantiation, execution, total time, tier, memory bytes, and `stat` syscall count. |
| `--quiet` / `-q` | Suppress script `stdout` output (useful when benchmarking with `--metrics`). |

### 4.3 `slice test [target] [flags]`
Runs the built-in Time-Travel Test Runner (`"slice:test"` / `"node:test"` / `"bun:test"`), discovering `*.test.ts`, `*.spec.ts`, `*.test.js`, and `*.spec.js` files across `[target]` (or running a single test file). Before every `test(...)` / `it(...)` block, Slice automatically takes a `< 5 us` VFS (`node:fs`) and `Slice.kv` state checkpoint (`Slice.checkpoint()`) and restores it (`Slice.restore(snap)`) in `finally`, guaranteeing 100% clean state isolation between tests.

| Flag | Description |
| :--- | :--- |
| `--filter=<substring>` | Only execute tests whose `"Suite > Test Name"` matches `<substring>`. |
| `--deterministic[=<seed>]` | Run all tests with a deterministic SplitMix64 PRNG and monotonic clock (default seed: `20261024`). |
| `--env-file=<path>` | Load `.env` variables and auto-register sensitive keys into the Egress DLP guard. |
| `--json` | Emit structured JSON test results (`files`, `total`, `passed`, `failed`, `skipped`, `checkpointsRestored`). |

Supported assertions on `expect(actual)`:
- Standard matchers: `.toBe(v)`, `.toEqual(v)`, `.toContain(sub)`, `.toBeTruthy()`, `.toBeFalsy()`, `.toBeNull()`, `.toBeDefined()`, `.toBeUndefined()`, `.toBeGreaterThan(n)`, `.toBeGreaterThanOrEqual(n)`, `.toBeLessThan(n)`, `.toBeLessThanOrEqual(n)`, `.toHaveLength(n)`, `.toHaveProperty(k, v?)`, `.toThrow(sub?)`, and `.not.*`.
- Slice OCap & Resource matchers (on `Slice.evalSandbox(...)` outcomes or thunks): `.toViolate(kindOrSubstring?)`, `.toStayWithinFuel(maxFuel)`, and `.toStayWithinMemory(maxBytes)`.

### 4.3 `slice pack <input_file_or_dir> <output.cas> [--entry=./index.ts]`
Compiles TypeScript via Oxc, bundles transitive ESM/CommonJS/JSON dependencies from `src/` and `node_modules/`, tree-shakes unused barrel submodules, hashes blobs with BLAKE3-256, and writes a 64-byte cache-line aligned `.cas` archive.

```bash
# Pack a single TypeScript file
slice pack ./src/index.ts ./bundle.cas

# Pack an entire folder application with package.json and node_modules/
slice pack ./docker-tests/apps/02-npm-ecosystem-suite ./npm-app.cas
```

### 4.4 `slice compile <input> [-o <output_binary>] [--entry=./index.ts]`
Embeds a `.ts` script, project folder, or `.cas` archive directly into a standalone, zero-dependency native executable (`SLICEEXE` trailer format).

```bash
slice compile ./docker-tests/apps/01-fintech-checkout -o ./checkout-bin
./checkout-bin --warm --metrics
```

### 4.5 `slice serve [target] [flags]`
Launches the multi-tenant WinterTC HTTP/1.1 `Keep-Alive` and pipelined MicroVM server.

| Flag | Description |
| :--- | :--- |
| `--port=<u16>` | TCP listening port (default: `3000`). |
| `--workers=<usize>` | Number of shared-nothing worker threads (default: `clamp(num_cpus, 1, 8)`). |
| `--tenant-dir=<path>` | Dynamically route `X-Slice-Tenant: <id>` or `/t/<id>/...` to `<path>/<id>.cas` with zero-downtime live hot-swapping. |
| `--deterministic[=<seed>]` | Enable deterministic PRNG and monotonic virtual clock for all served requests. |
| `--max-requests=<usize>` | Exit cleanly after serving `<usize>` requests (useful for automated benchmarks). |
| `--smoke-test` / `--once` | Bind an ephemeral port, send 2 pipelined HTTP/1.1 `Keep-Alive` requests, print headers/body, and exit. |

### 4.6 `slice mcp [flags]`
Starts the built-in JSON-RPC 2.0 Model Context Protocol (MCP) server over `stdio`, exposing `slice_eval_ts`, `slice_kv_get`, `slice_kv_set`, and `slice_checkpoint`.

| Flag | Description |
| :--- | :--- |
| `--allow-read=<prefix>` | Grant VFS read prefix to the MCP host compartment. |
| `--allow-write=<prefix>` | Grant VFS write prefix to the MCP host compartment. |
| `--allow-net=<host:port>` | Grant outbound network host to the MCP host compartment. |
| `--audit-egress=<secret>` | Block raw, Base64, or Base64URL exfiltration of `<secret>` across all MCP tool calls. |
| `--smoke-test` | Run a built-in 6-step JSON-RPC 2.0 self-test and print all responses. |

### 4.7 `slice inspect <archive.cas | script.ts | folder>`
Disassembles `SLICESNP`/`SLICEBOA` bytecode instructions and prints the BLAKE3 Radix resolution table and 64-byte aligned offsets for `.cas` archives.

### 4.8 `slice repl`
Starts an interactive SES-locked TypeScript/JavaScript compartment shell supporting `:help`, `:metrics`, `:caps`, `:reset`, and `:quit`.

### 4.9 `slice microvm-bench [--tenants=1000] [--json]`
Spawns `N` isolated tenant compartments (`default: 1000`), benchmarks Min/P50/P99 cold-start latency, 500-module Zero-Stat CAS resolution, 10,000 completion ring operations, Tier-2 compute kernels, and 5-vector security containment.

### 4.10 `slice verify-security`
Executes all 5 multi-tenant attack vectors (prototype pollution, `/etc/shadow` read, unauthorized network fetch, `AWS_SECRET_ACCESS_KEY` harvest, and `while(true)` CPU DoS) and verifies 100% deterministic containment.

