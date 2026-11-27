<div align="center">

# SLICE

**The Microsecond, Zero-Trust JavaScript & TypeScript Runtime Built in Rust**

Run untrusted AI agent code, multi-tenant SaaS plugins, and full-stack edge services in `< 1 us` cold starts and `13 - 22 MB` RSS -- with capability sandboxing, egress DLP, and `< 5 us` state rollbacks built into the runtime.

[![CI](https://github.com/slicevm/slice/actions/workflows/ci.yml/badge.svg)](https://github.com/slicevm/slice/actions/workflows/ci.yml)
[![Version](https://img.shields.io/badge/version-v0.14.6-blue)](https://github.com/slicevm/slice/releases)
[![Rust](https://img.shields.io/badge/rust-edition%202024-orange)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-green)](./LICENSE-MIT)
[![Benchmarks](https://img.shields.io/badge/benchmarks-33%20workloads-brightgreen)](./tests/reports/latest.md)
[![Docs](https://img.shields.io/badge/docs-index-informational)](./docs/README.md)

[Quickstart](#30-second-quickstart) | [Use Cases](#built-for-three-workloads) | [Benchmarks](#benchmarks-at-a-glance) | [Security Model](#5-ring-zero-trust-security) | [Documentation](#documentation)

</div>

---

## Why Teams Choose Slice

Modern JS runtimes were built to run trusted code in long-lived processes. When you need to execute **LLM-generated code**, **customer-authored plugins**, or **thousands of isolated tenants** on a single node, process-based runtimes pay a massive tax in cold starts, memory, and container orchestration.

**Slice removes that tax.** Built from scratch in 100% safe-boundary Rust, Slice combines sub-microsecond isolate boot, deterministic resource cages, and instant copy-on-write state rollbacks into a single ~7 MB binary.

| Capability | Slice (`v0.14.6`) | Bun / Deno / Node.js | Containers / MicroVMs |
| :--- | :--- | :--- | :--- |
| **Cold Start Latency** | **`0.85 us` P50** (`1.61 us` P99) | `15 - 80 ms` process boot | `100 - 500+ ms` VM boot |
| **1,000-Tenant Density** | **`1.04 ms` total (`8.25 MB` RSS)** | `5,000 - 30,000+ ms` | Gigabytes of RAM |
| **Zero-Trust Sandbox** | Built-in SES + OCap + Fuel/Heap Cage | Process / flag-level | External hypervisor |
| **State Rollbacks** | **`< 5 us` CoW VFS + KV snapshots** | Not built-in | Slow disk/VM snapshots |
| **Secret Leak Defense** | **Streaming Egress DLP** (raw/hex/B64) | External proxy required | External proxy required |
| **Deployment Unit** | Zero-`stat()` `.cas` or single binary | `node_modules` / bundle | OCI image + runtime |

---

## Built for Three Workloads

### 1. AI Agent "Code Mode" & Safe Tool Sandboxes
Give LLMs a real TypeScript runtime without risking your host or secrets. Fork memory-resident VFS and KV state in `< 5 us` via `Slice.checkpoint()`, evaluate untrusted code with deterministic CPU fuel and byte-level heap caps via `Slice.evalSandbox()`, roll back cleanly on failure, or connect directly to any MCP client with `slice mcp`.

### 2. Multi-Tenant SaaS Plugin Engines & Customer Workflows
Run customer-authored TypeScript hooks, webhooks, and automations in-process with zero ambient authority. Every tenant gets an attenuated `CapabilityToken` (`allow_read`, `allow_write`, `allow_net`, `allow_env`, `allow_exec`), DNS-rebinding & cloud-metadata SSRF blocking, and zero cross-tenant state leakage across pooled `--warm` executions.

### 3. High-Throughput Edge Services & Single-Binary Deploys
Serve `Hono`, `UnJS`, `Express`, or `SvelteKit 3` at **`159,866 req/s`** on shared-nothing worker threads (`slice serve`), hot-swap multi-tenant bundles via `--tenant-dir`, or ship a zero-dependency native executable via `slice compile`.

---

## 30-Second Quickstart

### Install

```bash
# Linux / macOS
curl -fsSL https://raw.githubusercontent.com/slicevm/slice/main/scripts/install.sh | sh

# Or build from source with Cargo
cargo build --release
```

```powershell
# Windows (PowerShell)
irm https://raw.githubusercontent.com/slicevm/slice/main/scripts/install.ps1 | iex
```

### Run, Pack & Ship

```bash
# Scaffold a TypeScript edge service (or --template=sveltekit for SvelteKit 3 + Svelte 5 SSR)
slice init my-app && cd my-app
slice run --metrics

# Run any .ts file, npm project, or inline expression with explicit capabilities
slice run ./src/index.ts --allow-env=STAGE=prod --allow-net=api.example.com:443
slice -p "crypto.randomUUID()"

# Pack into a zero-stat() .cas archive or compile into a standalone native executable
slice pack . ./app.cas
slice run ./app.cas --warm --metrics
slice compile . -o ./my-app
```

---

## See Slice in Action

### AI Agent Sandbox with `< 5 us` Time-Travel Rollback

Snapshot both the virtual filesystem (`node:fs`) and sharded KV store (`Slice.kv`), run untrusted TypeScript inside an attenuated child isolate, and roll back instantly if anything fails:

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

### Portable WinterTC Edge Server & Sharded KV (`slice serve`)

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
# Serve on 8 shared-nothing worker threads (or hot-swap tenants from a directory)
slice serve ./app.cas --port=3000 --workers=8
slice serve --port=3000 --workers=8 --tenant-dir=./tenants
```

### Declarative Policy-as-Code (`slice.toml`)

Configuration resolves deterministically: **Defaults < `slice.toml` < `// @slice-*` inline directives (attenuation-only) < `SLICE_*` env vars < CLI flags**.

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

## 5-Ring Zero-Trust Security

Every isolate executes inside five layered defense rings -- verify them yourself in under a second with `slice verify-security`:

1. **Ring 1 - Object-Capability (OCap) Isolation**: Zero ambient authority by default. Filesystem (`allow_read`/`allow_write` with symlink containment), network (`allow_net` with monotonic child attenuation), environment (`allow_env`), and subprocess (`allow_exec`) calls fail closed with structured `ERR_SLICE_*` errors.
2. **Ring 2 - Hardened JavaScript (SES Lockdown)**: 29 built-in prototypes and constructors are frozen across every execution tier, eliminating prototype pollution and cross-tenant realm tampering.
3. **Ring 3 - Deterministic CPU Fuel & Byte-Level Heap Cage**: Enforces strict instruction budgets, per-isolate heap ceilings, and wall-clock timeouts -- stopping infinite loops, zip bombs, and memory DoS cold.
4. **Ring 4 - Resolved-IP SSRF Firewall & Streaming Egress DLP**: Blocks cloud metadata endpoints unconditionally and blocks DNS rebinding to loopback, RFC-1918 private, link-local, CGNAT, and IPv4-mapped IPv6 addresses across `fetch`, redirects, `WebSocket`, TCP/TLS, and UDP. Scans outbound traffic for registered secrets in raw, hex, URL-encoded, and Base64/Base64URL encodings.
5. **Ring 5 - Pure-Rust Memory Safety & Modern Crypto**: Zero-dependency Rust implementation with `ring`, `ed25519-dalek`, `x25519-dalek`, and `blake3` powering WebCrypto and `node:crypto`.

See [`SECURITY.md`](./SECURITY.md) for the full threat model and private vulnerability reporting policy.

---

## Benchmarks at a Glance

<!-- BENCHMARK_SUMMARY_START -->
Across **33 real-world TypeScript apps, frameworks (`Hono`, `UnJS`, `Express`, `SvelteKit 3`), and agent case studies**, Slice (`v0.14.6`) beats **Deno (`v2.9.8`)** on **33/33**, **Node.js (`v27.0.0-alpha.0.1.1`)** on **33/33**, **Bun (`v1.4.5`)** on **33/33** cold one-shot runs, and **all 3 runtimes on 33/33 workloads in `--warm` pooled execution**.

| Metric (`Linux x86_64`) | Slice (`v0.14.6`) | Bun (`v1.4.5`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha.0.1.1`) |
| :--- | :---: | :---: | :---: | :---: |
| **Isolate Cold Start (`P50` / `P99`)** | **`0.85 us` / `1.61 us`** | Process / VM | Process / VM | Process / VM |
| **1,000-Tenant Swarm (Boot + Exec)** | **`1.04 ms` (`8.25 MB` RSS)** | `> 5,000 ms` | `> 15,000 ms` | `> 30,000 ms` |
| **HTTP/1.1 Keep-Alive (`16k` Reqs)** | **`159,866 req/s`** (Tier-0) / **`156,501 req/s`** (Tier-1) | `98,564 req/s` | `89,842 req/s` | `55,136 req/s` |
| **33-Workload Cold `.cas` Total** | **`419.6 ms`** (`1.73x` Bun / `5.92x` Node) | `724.2 ms` | `1,720.8 ms` | `2,486.2 ms` |
| **33-Workload `--warm` Pooled Total** | **`135.1 ms`** (`5.36x` Bun / `18.41x` Node) | `724.2 ms` | `1,720.8 ms` | `2,486.2 ms` |
| **Peak Memory (`Max RSS` Range)** | **`13.63 - 22.00 MB`** | `36.38 - 48.38 MB` | `45.47 - 62.75 MB` | `68.82 - 91.35 MB` |

Full 33-workload breakdown, per-app latency and RSS tables, and run provenance (`144857c`, `2026-11-26T18:56:00Z`): **[`tests/reports/latest.md`](./tests/reports/latest.md)** (archive: [`bench_v0.14.6_20261126_185600Z_144857c.md`](./tests/reports/bench_v0.14.6_20261126_185600Z_144857c.md)).
<!-- BENCHMARK_SUMMARY_END -->

Reproduce the full Linux `x86_64` Docker benchmark suite on your own hardware:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\run_benchmarks.ps1
```

---

## CLI Command Reference

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

## Documentation

Full guides and architecture references live under [`docs/`](./docs/README.md):

* **[Developer Handbook (`docs/general/`)](./docs/general/00_index.md)** -- CLI configuration, execution tiers, `.cas` bundling, OCap/DLP security, `slice serve`, AI agent checkpoints/MCP, `node:*` compatibility, Rust embedding, and time-travel testing.
* **[Framework & Tooling Guides (`docs/guides/`)](./docs/guides/README.md)** -- Deploying Hono, UnJS, and SvelteKit 3, TypeScript IDE setup (`slice.d.ts`), and WebAssembly bundling.
* **[API Compatibility Matrix (`docs/general/COMPATIBILITY.md`)](./docs/general/COMPATIBILITY.md)** -- Supported WinterTC Web APIs, `node:*` modules, `Buffer`, and `Slice.*` globals.
* **[System Architecture (`docs/system/`)](./docs/system/00_index.md)** -- Internal engineering specs for `slice-cas`, `slice-engine`, `slice-sandbox`, `slice-io`, and the vendored `boa_engine` bytecode pipeline.

---

## Get Involved

* **Try it in 60 seconds**: Run `slice verify-security` and `slice microvm-bench` to test containment and density locally.
* **Report bugs or request features**: Open an issue on [GitHub Issues](https://github.com/slicevm/slice/issues).
* **Report security findings privately**: Use [GitHub Private Vulnerability Reporting](https://github.com/slicevm/slice/security/advisories/new) -- see [`SECURITY.md`](./SECURITY.md).
* **Follow releases**: Star the repository to track upcoming milestones (`v0.15.0` async C10K reactor + HTTP/2, `v0.16.0` Durable Objects + Landlock/seccomp-bpf).

## License

Licensed under either of [Apache License, Version 2.0](./LICENSE-APACHE) or [MIT License](./LICENSE-MIT) at your option.
