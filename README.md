# Slice

[![CI](https://github.com/slicevm/slice/actions/workflows/ci.yml/badge.svg)](https://github.com/slicevm/slice/actions/workflows/ci.yml)
[![Version](https://img.shields.io/badge/version-v0.11.0-blue)](https://github.com/slicevm/slice/releases)
[![Rust](https://img.shields.io/badge/rust-edition%202021-orange)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-green)](./LICENSE-MIT)
[![Benchmarks](https://img.shields.io/badge/benchmarks-33%20workloads-brightgreen)](./tests/reports/latest.md)
[![Docs](https://img.shields.io/badge/docs-index-informational)](./docs/README.md)

**Slice** is a lightweight, zero-trust JavaScript and TypeScript runtime built in Rust for multi-tenant edge services, SaaS plugin engines, and AI agent sandboxes.

Run `.ts` scripts, npm projects, or full-stack web apps (`Hono`, `UnJS`, `Express`, `SvelteKit 3`) with microsecond isolate startup, a `13 - 22 MB` memory footprint, built-in capability sandboxing, and `< 5 us` stateful rollbacks.

## Why Slice?

* **Microsecond Cold Starts & Single-Binary Deploys** - Pack TypeScript projects and `node_modules` into zero-`stat()` precompiled `.cas` bundles (`slice pack`) or standalone executables (`slice compile`). Isolates boot in `~1 us` without JIT warmup or heavy memory overhead.
* **Zero-Trust Sandbox Built Into the Runtime** - Every isolate runs inside a frozen SES compartment with explicit `allow_read`, `allow_write`, `allow_net`, `allow_env`, and `allow_exec` rules, deterministic CPU fuel and heap cages, cloud-metadata SSRF blocking, and streaming egress secret redaction (DLP).
* **Stateful `< 5 us` Checkpoints for AI Agents & Tests** - Snapshot and roll back both the virtual filesystem (`node:fs`) and built-in KV store (`Slice.kv`) in microseconds via `Slice.checkpoint()` and `Slice.restore()`, run untrusted code via `Slice.evalSandbox()`, or expose an MCP server over stdio (`slice mcp`).
* **WinterTC Web Standards + Pragmatic Node Compatibility** - Write portable `fetch(req, env, ctx)` handlers (`Request`, `Response`, `Streams`, `WebCrypto`, `WebSocket`, `WebAssembly`, `Intl`) alongside core `node:*` modules (`fs`, `crypto`, `http`, `stream`, `async_hooks`, `worker_threads`, `zlib`).

---

## Quickstart

### 1. Install or Build

```bash
# Linux / macOS
./scripts/install.sh

# Or build directly with Cargo
cargo build --release
```

```powershell
# Windows (PowerShell)
powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1
```

### 2. Scaffold, Run & Ship

```bash
# Scaffold a TypeScript edge service (or --template=sveltekit for SvelteKit 3 + Svelte 5 SSR)
slice init my-app
cd my-app
slice run --metrics

# Run a single file, inline snippet, or existing npm project folder
slice run ./src/index.ts --allow-env=STAGE=prod
slice -p "crypto.randomUUID()"

# Bundle into a memory-mapped .cas archive or a zero-dependency native executable
slice pack . ./app.cas
slice run ./app.cas --warm --metrics
slice compile . -o ./my-app
```

---

## Core Usage

### HTTP Edge Server & Sharded KV (`slice serve`)

Write standard WinterTC `fetch` handlers backed by tenant-isolated atomic counters and TTL storage (`Slice.kv`):

```typescript
export default {
  fetch(req: Request, _env: Record<string, string>, ctx: { waitUntil(fn: () => void): void }): Response {
    const url = new URL(req.url);
    const hits = Slice.kv.incr("hits:total", 1);
    Slice.kv.set("last:path", url.pathname, 60_000);

    ctx.waitUntil(() => {
      // Runs after response bytes are flushed to the socket
    });

    return Response.json({ ok: true, path: url.pathname, hits });
  }
};
```

```bash
# Serve on 8 shared-nothing worker threads (or hot-swap multi-tenant bundles via --tenant-dir)
slice serve ./app.cas --port=3000 --workers=8
slice serve --port=3000 --workers=8 --tenant-dir=./tenants
```

### AI Agent Sandbox, `< 5 us` Checkpoints & MCP (`Slice.evalSandbox` / `slice mcp`)

Fork state in memory, evaluate untrusted or LLM-generated TypeScript inside an attenuated child isolate, and roll back cleanly on failure:

```typescript
import fs from "node:fs";

fs.writeFileSync("/workspace/state.json", '{"step":1}');
Slice.kv.set("status", "ready");
const snap = Slice.checkpoint(); // < 5 us copy-on-write VFS + KV snapshot

const result = Slice.evalSandbox("const answer: number = 21 * 2; answer", {
  fuel: 200_000,
  memoryBytes: 1024 * 1024,
  allowRead: ["/workspace"],
  auditSecret: "sk_live_secret"
});

if (!result.ok) {
  Slice.restore(snap); // Instant < 5 us rollback of VFS + KV
}
```

```bash
# Expose Slice over stdio as a Model Context Protocol (MCP) JSON-RPC 2.0 server
slice mcp --allow-read=/workspace --allow-write=/workspace --audit-egress=sk_live_secret

# Run tests with automatic < 5 us VFS + KV rollback after every test() block
slice test ./tests/apps/15-time-travel-test-suite
```

### Declarative Security & Resource Config (`slice.toml`)

Settings resolve in deterministic order: **Defaults < `slice.toml` < `// @slice-*` inline directives < `SLICE_*` env vars < CLI flags**.

```toml
[package]
name = "my-app"
entrypoint = "./src/index.ts"

[limits]
fuel = 50_000_000          # Deterministic CPU instruction budget
memory_kb = 8192           # Per-isolate heap cap (8 MB)

[capabilities]
allow_read = ["./public"]
allow_write = ["/tmp/scratch"]
allow_net = ["api.example.com:443"]
env_file = ".env"          # Auto-registers *SECRET*, *TOKEN*, *KEY*, *PASSWORD* in Egress DLP

[server]
port = 3000
workers = 8
```

---

## CLI Commands

| Command | Purpose |
| :--- | :--- |
| `slice init [dir]` | Scaffold a TypeScript edge service or SvelteKit 3 app (`--template=sveltekit`). |
| `slice run [target]` | Run a `.ts`/`.js`/`.tsx` file, folder, `.cas` bundle, stdin (`-`), or inline code (`-e` / `-p`). |
| `slice check <target>` | Parse and syntax-check TypeScript or JavaScript without executing (`--check` / `-c`). |
| `slice test [target]` | Run `*.test.ts` / `*.spec.ts` suites with `< 5 us` per-test VFS+KV rollback (`--watch`). |
| `slice serve [target]` | Start the multi-worker HTTP/1.1, SSE, WebSocket & TLS 1.3 server (`--tenant-dir`, `--static`). |
| `slice pack <in> <out.cas>` | Bundle a project and `node_modules` into a precompiled zero-`stat()` `.cas` archive. |
| `slice compile <in> -o <bin>` | Compile an application and `.cas` archive into a standalone native binary. |
| `slice inspect <target>` | Disassemble bytecode (`SLICEBC` / `SLICESNP`) and inspect `.cas` module tables. |
| `slice mcp` | Start the built-in JSON-RPC 2.0 Model Context Protocol server over stdio. |
| `slice repl` | Start an interactive SES-locked TypeScript/JavaScript shell. |
| `slice doctor` / `slice info` | Inspect host kernel `io_uring`/`IOCP` rings, hardware isolation, and runtime build info. |
| `slice verify-security` | Run the 5-vector OCap, SES prototype lockdown, and CPU fuel containment checks. |
| `slice microvm-bench` | Benchmark 1,000-tenant isolate cold-start latency, density, and memory footprint. |

---

## Benchmarks at a Glance

<!-- BENCHMARK_SUMMARY_START -->
Across **33 real-world TypeScript apps, frameworks (`Hono`, `UnJS`, `Express`, `SvelteKit 3`), and agent case studies**, Slice (`v0.11.0`) beats **Deno (`v2.9.8`)** on **33/33**, **Node.js (`v27.0.0-alpha.0.0.0`)** on **33/33**, **Bun (`v1.4.4`)** on **28/33** cold one-shot runs, and **all 3 runtimes on 33/33 workloads in `--warm` pooled execution**.

| Metric (`Linux x86_64`) | Slice (`v0.11.0`) | Bun (`v1.4.4`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha.0.0.0`) |
| :--- | :---: | :---: | :---: | :---: |
| **Isolate Cold Start (`P50` / `P99`)** | **`1.26 us` / `1.74 us`** | Process / VM | Process / VM | Process / VM |
| **1,000-Tenant Swarm (Boot + Exec)** | **`1.48 ms` (`7.50 MB` RSS)** | `> 5,000 ms` | `> 15,000 ms` | `> 30,000 ms` |
| **HTTP/1.1 Keep-Alive (`16k` Reqs)** | **`153,678 req/s`** (Tier-0) / **`139,049 req/s`** (Tier-1) | `92,523 req/s` | `89,111 req/s` | `53,925 req/s` |
| **33-Workload Cold `.cas` Total** | **`519.3 ms`** (`1.36x` Bun / `5.35x` Node) | `707.6 ms` | `1,768.0 ms` | `2,777.0 ms` |
| **33-Workload `--warm` Pooled Total** | **`181.5 ms`** (`3.90x` Bun / `15.30x` Node) | `707.6 ms` | `1,768.0 ms` | `2,777.0 ms` |
| **Peak Memory (`Max RSS` Range)** | **`13.25 - 22.88 MB`** | `25.88 - 40.88 MB` | `45.96 - 61.86 MB` | `69.40 - 91.75 MB` |

Full 33-workload breakdown, per-app latency and RSS tables, and run provenance (`1182dae`, `2026-11-13T22:10:39Z`): **[`tests/reports/latest.md`](./tests/reports/latest.md)** (archive: [`bench_v0.11.0_20261113_221039Z_1182dae.md`](./tests/reports/bench_v0.11.0_20261113_221039Z_1182dae.md)).
<!-- BENCHMARK_SUMMARY_END -->

To reproduce the full Linux `x86_64` Docker benchmark suite locally and regenerate [`tests/reports/latest.md`](./tests/reports/latest.md):

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\run_benchmarks.ps1
```

---

## Documentation

Full documentation is organized under [`docs/`](./docs/README.md):

* **Developer Handbook (`docs/general/`)** - Step-by-step guides for CLI configuration, execution tiers, `.cas` bundling, OCap/DLP security, `slice serve`, AI agent checkpoints/MCP, `node:*` compatibility, Rust embedding, and time-travel testing.
* **Framework & Tooling Guides (`docs/guides/`)** - Recipes for deploying Hono, UnJS, and SvelteKit 3, configuring TypeScript IDEs (`slice.d.ts`), and bundling WebAssembly.
* **API Compatibility (`docs/general/COMPATIBILITY.md`)** - Supported WinterTC Web APIs, `node:*` modules, `Buffer`, and `Slice.*` globals.
* **System Architecture (`docs/system/`)** - Internal engineering specs for `slice-cas`, `slice-engine`, `slice-sandbox`, `slice-io`, and the vendored `boa_engine` bytecode pipeline.

Start at the **[Documentation Index (`docs/README.md`)](./docs/README.md)**.

## License

Licensed under either of [Apache License, Version 2.0](./LICENSE-APACHE) or [MIT License](./LICENSE-MIT) at your option.
