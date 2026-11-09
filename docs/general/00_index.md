# The Slice (Nexus-JS) Developer Handbook (`v0.10.5`)

Welcome to the complete developer handbook for **Slice (Nexus-JS)** (`v0.10.5`) — a microVM-grade JavaScript and TypeScript runtime, compiler, HTTP server, and AI sandbox engineered in Rust.

Whether you are building a high-throughput full-stack web service, packaging a CLI tool into a single zero-dependency executable, running 1,000 untrusted customer plugins on a single machine, or executing LLM-generated TypeScript inside an AI coding agent, this handbook walks you through every feature, CLI command, API, security capability, and migration pattern from the perspective of an application developer.

---

## Why Developers Choose Slice

Traditional JavaScript runtimes (**Node.js**, **Deno**, and **Bun**) were built around a single-tenant process model with ambient POSIX permissions and runtime `node_modules` filesystem traversal:
- Every cold start pays for hundreds of OS `stat()`/`open()` syscalls and JavaScript parsing.
- Every imported npm package or plugin shares the same mutable `Object.prototype` and can read `~/.ssh`, query cloud metadata (`169.254.169.254`), or exfiltrate `process.env` secrets unless you wrap the entire runtime inside heavy Docker containers or `125 ms` Firecracker VMs.

**Slice replaces that tradeoff with four developer-facing superpowers:**

1. **Instant Cold Starts & Precompiled `SLICEBC\x01` Packaging (`1.27 us` P50 Isolate / `0` `stat()` Syscalls):**
   - Run `.ts`, `.js`, or entire project folders directly (`slice run`), pack them into a 64-byte aligned BLAKE3 `.cas` archive with ahead-of-time **Precompiled Boa `CodeBlock` (`SLICEBC\x01`)** bytecode (`slice pack`), or compile your backend + frontend UI into a **single zero-dependency native executable** (`slice compile`) — beating **Bun (`v1.3.10`)** on **30 out of 31** real-world application benchmarks (`5.11 ms` vs `25.19 ms` on App 01, `13.11 ms` vs `33.24 ms` on Hono v4).
2. **True Multi-Tenant MicroVM Security in Microseconds:**
   - Every script or HTTP request executes inside a frozen Secure ECMAScript (`SES`) compartment with explicit **Object-Capability (OCap)** permissions (`allow_read`, `allow_write`, `allow_net`, `allow_env`), deterministic **CPU instruction fuel limits**, **byte-level memory cages**, a built-in **Cloud Metadata SSRF Firewall**, and **Outbound Egress DLP** that blocks raw, Base64, and Base64URL secret leaks.
3. **Full-Stack Web & Edge Server Out of the Box (`163,766 req/s` Tier-0 / `126,566 req/s` Tier-1):**
   - Write standard WinterTC / Cloudflare Workers / Bun / Deno `export default { fetch(req, env, ctx) }` handlers (`Request`, `Response`, `Headers`, `URL`, `URLSearchParams`, `ReadableStream`), stream **Server-Sent Events (`Response.sse`)**, use the built-in **16-way sharded `Slice.kv` store**, serve static frontend assets (`--static=./public`) with `< 2 us` BLAKE3 `ETag` `304` responses, and hot-swap tenant bundles (`--tenant-dir`) with zero downtime.
4. **Unified 4-Layer Configuration & AI Agent Primitives:**
   - Configure every engine, limit, OCap permission, and server setting declaratively across **`slice.toml`**, **top-of-file inline directives (`// @slice-...`)**, **`SLICE_*` environment variables**, and **CLI flags (`--*`)**.
   - Run unit tests with **`slice test`**, where every `test()` block automatically snapshots (`Slice.checkpoint()`) and rolls back (`Slice.restore()`) both the filesystem (`node:fs`) and `Slice.kv` in `< 5 microseconds`.
   - Spawn nested, capability-attenuated sub-sandboxes in `~27 microseconds` with **`Slice.evalSandbox()`** or connect Claude Code / Cursor directly to **`slice mcp`**.

---

## How This Handbook Is Organized

| Chapter | Title & Developer Focus |
| :--- | :--- |
| **[Chapter 01](./01_getting_started_and_cli.md)** | **Installation, Upgrading, Quickstart, `slice.toml` & Complete CLI Reference**<br>Install and update Slice on Windows, Linux, macOS, and Docker; scaffold your first project with `slice init`; master `slice.toml`, `.env` Auto-DLP, `< 3 ms` `--watch`, `slice repl`, `slice doctor`, and all 13 CLI commands. |
| **[Chapter 02](./02_execution_engine_and_tiers.md)** | **Writing TypeScript & JavaScript for Maximum Performance**<br>Understand how Slice runs your code across Tier-0 Snapshots, Tier-1 Warm Realms (`--warm`), and Tier-2 Native SIMD Kernels — and how simple TypeScript type annotations (`: number`, `interface`) unlock automatic compile-time speedups. |
| **[Chapter 03](./03_cas_bundler_and_standalone_binaries.md)** | **Bundling Apps (`slice pack`), Single-File Executables (`slice compile`) & Bytecode Inspection**<br>Bundle multi-file TypeScript apps and `node_modules` into `.cas` archives, embed frontend static assets (`--static`), ship standalone native binaries (`SLICEEXE`), and inspect bundles with `slice inspect`. |
| **[Chapter 04](./04_security_ocap_ses_and_hardware_mpk.md)** | **Zero-Trust Security Guide: OCap Permissions, Fuel Limits, SSRF Shield & Egress DLP**<br>Configure `allow_read`, `allow_write`, `allow_net`, and `allow_env`; cap CPU fuel and memory per tenant; prevent `169.254.169.254` SSRF attacks; and block accidental or malicious secret leaks with `--audit-egress` and Auto-DLP `.env`. |
| **[Chapter 05](./05_http_server_wintertc_and_edge_kv.md)** | **Building Full-Stack Web Apps, APIs, Static Sites & SSE Streams (`slice serve`)**<br>Build production HTTP services with `export default { fetch }`, `Request`/`Response`/`URL`/`Headers`, `Slice.kv` rate limiters and sessions, `--static` CDN asset serving, `Response.sse` streaming, outbound `fetch()`, and `--tenant-dir` hot-swapping. |
| **[Chapter 06](./06_ai_agent_sandbox_checkpoints_and_mcp.md)** | **AI Agents, Nested MicroVM Sandboxes (`Slice.evalSandbox`), `< 5 us` Checkpoints & `slice mcp`**<br>Execute untrusted LLM or user plugin code safely in microseconds, branch and roll back VFS + KV state with `Slice.checkpoint()` / `Slice.restore()`, and integrate `slice mcp` with Claude Code and Cursor. |
| **[Chapter 07](./07_node_compat_intrinsics_and_npm.md)** | **Node.js Compatibility (`node:*`), NPM Packages & Migration Guide (Node / Bun / Deno to Slice)**<br>Complete API reference for `node:fs`, `node:crypto`, `node:path`, `node:os`, `node:buffer`, `node:url`, `node:util`, `node:process`, and `node:test`, plus recipes for npm packages and step-by-step migration guides from Express/Fastify, Hono, Bun, and Deno. |
| **[Chapter 08](./08_kernel_io_rings_and_rust_embedding.md)** | **Embedding Slice in Rust Applications & High-Density MicroVM Swarms**<br>Embed `slice-engine`, `slice-cas`, `slice-sandbox`, and `slice-io` as a library inside your own Rust services to run thousands of isolated tenant scripts in a single process. |
| **[Chapter 09](./09_benchmarks_testing_and_docker_suite.md)** | **Time-Travel Testing (`slice test`), Deterministic Replay & Production Benchmarks**<br>Write rock-solid unit and security tests with `slice test` (`expect(...).toViolate()`, `.toStayWithinFuel()`, `< 5 us` VFS+KV rollbacks), replay bugs with `--deterministic`, and explore all 33 real-world benchmark applications, frameworks, and case studies. |
| **[Compatibility Matrix](./COMPATIBILITY.md)** | **WinterTC, Node.js (`node:*`), `Buffer` & NPM Bundler Compatibility Matrix**<br>Complete reference of supported WinterTC Web APIs, `Uint8Array`-backed `Buffer` methods, `node:*` core modules, and `package.json` `"exports"` bundler capabilities. |

---

## Choose Your Path: Common Developer Workflows

### 1. "I want to build and ship a Full-Stack Web App or REST/GraphQL API"
1. Scaffold a project: `slice init my-web-api` ([Chapter 01](./01_getting_started_and_cli.md))
2. Write a standard `export default { fetch(req, env, ctx) }` handler using `Request`, `Response.json()`, `URL`, and `Slice.kv` ([Chapter 05](./05_http_server_wintertc_and_edge_kv.md))
3. Attach your frontend build folder with `--static=./public` and compile everything into a single binary:
   ```bash
   slice compile . -o ./my-web-api --static=./public
   ```
   ([Chapter 03](./03_cas_bundler_and_standalone_binaries.md))

### 2. "I want to run untrusted customer plugins or AI-generated code safely"
1. Use `Slice.evalSandbox(untrustedTsCode, { fuel: 250_000, memoryBytes: 1048576, allowRead: ["/tmp/tenant-1"] })` inside TypeScript ([Chapter 06](./06_ai_agent_sandbox_checkpoints_and_mcp.md))
2. Take a `< 5 us` snapshot before running candidate code (`const snap = Slice.checkpoint()`) and roll back (`Slice.restore(snap)`) if the code fails or violates policy ([Chapter 06](./06_ai_agent_sandbox_checkpoints_and_mcp.md))
3. Or wire `slice mcp` directly into your MCP client (`mcp.json`) so your AI coding assistant can evaluate TypeScript and manage state snapshots over `stdio` ([Chapter 06](./06_ai_agent_sandbox_checkpoints_and_mcp.md))

### 3. "I want to migrate an existing Node.js, Bun, or Deno project to Slice"
1. Point `slice run` or `slice test` directly at your existing project folder (`package.json` + `src/index.ts` + `node_modules/`) ([Chapter 07](./07_node_compat_intrinsics_and_npm.md))
2. Add a `slice.toml` manifest with `env_file = ".env"` and explicit `allow_read`, `allow_write`, and `allow_net` rules so your app is protected by default against SSRF and secret exfiltration ([Chapter 04](./04_security_ocap_ses_and_hardware_mpk.md))
3. Convert tests to `slice test` (`import { describe, test, expect } from "slice:test"`) to get automatic `< 5 us` filesystem and KV rollback after every test ([Chapter 09](./09_benchmarks_testing_and_docker_suite.md))

---

## Developer Guides (`docs/guides/`) & System Architecture Docs (`docs/system/`)

- **Developer How-To Guides (`docs/guides/`):**
  - [`docs/guides/README.md`](../guides/README.md)
  - [`docs/guides/01_hono_and_unjs_edge_microservices.md`](../guides/01_hono_and_unjs_edge_microservices.md)
  - [`docs/guides/02_typescript_ide_linting_and_wasm.md`](../guides/02_typescript_ide_linting_and_wasm.md)
- **Core Maintainer & Internal Architecture Specs (`docs/system/`):**
  - [`docs/system/00_index.md`](../system/00_index.md)
  - [`docs/system/01_cas_bundler_and_oxc_compiler.md`](../system/01_cas_bundler_and_oxc_compiler.md)
  - [`docs/system/02_tiered_execution_and_boa_vm.md`](../system/02_tiered_execution_and_boa_vm.md)
  - [`docs/system/03_security_sandbox_ocap_and_mpk.md`](../system/03_security_sandbox_ocap_and_mpk.md)
  - [`docs/system/04_networking_wintertc_and_event_loop.md`](../system/04_networking_wintertc_and_event_loop.md)
  - [`docs/system/05_wasm_workers_and_kernel_io.md`](../system/05_wasm_workers_and_kernel_io.md)
  - [`docs/system/06_edge_kv_ai_sandbox_and_mcp.md`](../system/06_edge_kv_ai_sandbox_and_mcp.md)
  - [`docs/system/07_production_tooling_guide.md`](../system/07_production_tooling_guide.md)
  - [`docs/system/08_benchmarks_and_comparisons.md`](../system/08_benchmarks_and_comparisons.md)
  - [`docs/system/09_research_deep_dive.md`](../system/09_research_deep_dive.md)
  - [`vendor/boa_engine/SLICE_PATCH.md`](../../vendor/boa_engine/SLICE_PATCH.md) (`SLICEBC1` Precompiled `CodeBlock` Vendor Patch Specification)
- **Docker Application, Framework & Case Study Test Suites (`tests/`):**
  - [`tests/README.md`](../../tests/README.md) (`tests/apps/` — 24 Production Apps)
  - [`tests/frameworks/README.md`](../../tests/frameworks/README.md) (`tests/frameworks/` — Hono v4, itty-router + UnJS, Express/Polka, & SvelteKit 3 + Svelte 5)
  - [`tests/case_studies/README.md`](../../tests/case_studies/README.md) (`tests/case_studies/` — 5 Architectural Differentiation Workloads)
