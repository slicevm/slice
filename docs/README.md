# Slice Documentation Index

Welcome to the documentation hub for **Slice** (`v0.10.12`), a zero-trust JavaScript and TypeScript runtime built in Rust for multi-tenant edge services, SaaS plugin engines, and AI agent sandboxes.

Documentation is organized into four tracks depending on what you are building:

| Track | Audience | Entrypoint |
| :--- | :--- | :--- |
| **1. Developer Handbook (`docs/general/`)** | JS/TS application developers, DevOps, and Rust embedders | [`docs/general/00_index.md`](./general/00_index.md) |
| **2. Framework & Tooling Guides (`docs/guides/`)** | End-to-end recipes for web frameworks, IDE setup, and WebAssembly | [`docs/guides/README.md`](./guides/README.md) |
| **3. API Compatibility Matrix** | Supported WinterTC Web APIs, `node:*` modules, and `Slice.*` globals | [`docs/general/COMPATIBILITY.md`](./general/COMPATIBILITY.md) |
| **4. Architecture & Crate Internals (`docs/system/`)** | Runtime contributors and systems engineers | [`docs/system/00_index.md`](./system/00_index.md) |

---

## 1. User & Developer Handbook (`docs/general/`)

* **[00: Handbook Overview & Common Workflows](./general/00_index.md)**
* **[01: Getting Started, Configuration & Complete CLI Reference](./general/01_getting_started_and_cli.md)** - Install scripts, `slice init`, `slice.toml`, inline `@slice-*` directives, `SLICE_*` env vars, and all CLI commands/flags.
* **[02: Execution Engine & Tiers (`slice-engine`)](./general/02_execution_engine_and_tiers.md)** - Precompiled `SLICEBC` bytecode, `--warm` isolate pooling, TypeScript type-guided optimizations, and Tier-0/Tier-1/Tier-2 modes.
* **[03: Zero-Stat CAS Bundler & Standalone Binaries (`slice-cas`)](./general/03_cas_bundler_and_standalone_binaries.md)** - `slice pack`, `slice compile`, static asset embedding (`--static`), and `slice inspect`.
* **[04: Security Architecture: OCap, SES, Fuel & DLP (`slice-sandbox`)](./general/04_security_ocap_ses_and_hardware_mpk.md)** - File/network/env/exec capability rules, CPU fuel and memory cages, SSRF blocking, and outbound secret redaction.
* **[05: HTTP Edge Server, WinterTC APIs & Sharded KV (`slice serve`)](./general/05_http_server_wintertc_and_edge_kv.md)** - Multi-worker HTTP/1.1 keep-alive, TLS 1.3, SSE, WebSockets, `Slice.kv`, and `--tenant-dir` hot-swapping.
* **[06: AI Agent Sandboxes, `< 5 us` Checkpoints & MCP (`slice mcp`)](./general/06_ai_agent_sandbox_checkpoints_and_mcp.md)** - `Slice.evalSandbox()`, `Slice.checkpoint()` / `Slice.restore()`, deterministic replay, and stdio MCP integration.
* **[07: Node.js Compatibility (`node:*`), Intrinsics & NPM Packages](./general/07_node_compat_intrinsics_and_npm.md)** - Built-in `node:*` modules (`fs`, `crypto`, `stream`, `http`, `async_hooks`, `zlib`, `worker_threads`), `Buffer`, and migration recipes.
* **[08: Kernel Completion Rings (`slice-io`) & Rust Embedding](./general/08_kernel_io_rings_and_rust_embedding.md)** - Linux `io_uring` / Windows `IOCP` reactor and embedding `SliceIsolate` inside Rust host applications.
* **[09: Time-Travel Testing (`slice test`) & Docker Benchmark Suite](./general/09_benchmarks_testing_and_docker_suite.md)** - Writing tests with automatic `< 5 us` VFS+KV rollback and running the reproducible 33-workload benchmark suite.
* **[Runtime Compatibility Matrix (`COMPATIBILITY.md`)](./general/COMPATIBILITY.md)** - Reference table of WinterTC standards, `node:*` APIs, `Buffer`, and `Slice.*` runtime extensions.

---

## 2. Framework & Tooling Guides (`docs/guides/`)

* **[Guides Overview](./guides/README.md)**
* **[Guide 01: Hono (`v4`), UnJS/Nitro & SvelteKit (`v3`) Microservices](./guides/01_hono_and_unjs_edge_microservices.md)**
* **[Guide 02: TypeScript IDE Setup (`slice.d.ts`), Linting & WebAssembly](./guides/02_typescript_ide_linting_and_wasm.md)**

---

## 3. System Architecture & Internals (`docs/system/`)

* **[00: System Architecture Overview & Crate Map](./system/00_index.md)**
* **[01: `slice-cas` - Zero-Stat CAS Archive, Oxc Compiler & Linker](./system/01_cas_bundler_and_oxc_compiler.md)**
* **[02: `slice-engine` - Bytecode Pipeline, Prelude Shards & Native Intrinsics](./system/02_tiered_execution_and_boa_vm.md)**
* **[03: `slice-sandbox` - 5-Ring OCap Compartments, Dynamic SES & Hardware MPK](./system/03_security_sandbox_ocap_and_mpk.md)**
* **[04: Networking, WinterTC Web Platform & Event Loop](./system/04_networking_wintertc_and_event_loop.md)**
* **[05: Pure-Rust WebAssembly (`wasmi`), Workers & Kernel I/O](./system/05_wasm_workers_and_kernel_io.md)**
* **[06: Sharded Edge KV, Copy-on-Write Checkpoints & MCP Server](./system/06_edge_kv_ai_sandbox_and_mcp.md)**
* **[Vendored `boa_engine` Patch Specification (`SLICE_PATCH.md`)](../vendor/boa_engine/SLICE_PATCH.md)**

---

## 4. Benchmarks & Verification Suites (`tests/`)

* **[Latest Verified 33-Workload Benchmark Report (`tests/reports/latest.md`)](../tests/reports/latest.md)**
* **[24 Production Application Suites (`tests/README.md`)](../tests/README.md)**
* **[Full-Stack Framework Suites (`tests/frameworks/README.md`)](../tests/frameworks/README.md)** - Hono v4, itty-router + UnJS, Express/Polka `node:http`, and SvelteKit 3 + Svelte 5 SSR.
* **[Interactive Case Studies & Visual Cockpit (`tests/case_studies/README.md`)](../tests/case_studies/README.md)** - Tree-of-Thought SRE agent, MCP code-mode orchestrator, multi-tenant SaaS plugin mesh, Wasm CDR pipeline, and payment saga chaos fuzzer.
