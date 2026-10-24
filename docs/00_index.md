# Slice (Nexus-JS) v0.2.0 — Complete System Documentation

Welcome to the official chapter-by-chapter documentation for **Slice (Nexus-JS)** (`v0.2.0`), a next-generation, microVM-grade JavaScript and TypeScript edge runtime engineered in Rust.

---

## Chapter Index

1. **[Chapter 01: Getting Started, Project Configuration & Complete CLI Reference](file:///p:/opensource/slice/docs/01_getting_started_and_cli.md)**
   - Building & installing on Windows, Linux, and macOS
   - Scaffolding projects with `slice init` and configuring `slice.toml`
   - Complete CLI reference for all 12 subcommands (`init`, `run`, `pack`, `compile`, `serve`, `mcp`, `inspect`, `repl`, `doctor`, `microvm-bench`, `verify-security`, `info`) and all runtime flags

2. **[Chapter 02: Hybrid Three-Tier Execution Engine (`slice-engine`)](file:///p:/opensource/slice/docs/02_execution_engine_and_tiers.md)**
   - Tier-0 `SLICESNP` Snapshot VM (`0.98 us` cold start, `16-byte` cache-line `JsValue`)
   - Tier-1 Zero-Copy `SLICEBOA` ECMAScript 2024+ VM, Pre-Warmed `BOA_CONTEXT_POOL` (`--warm`), and Persistent Realm Cache (`__slice_rc` direct `JsObject::call`)
   - Tier-2 Peak Native SIMD Kernels (`MonomorphicForLoop`, Intel/AMD `SHA-NI`, `FibKernel`, `NumericLoopKernel`)
   - Oxc Type-Directed Semantic Lowering (`interface` schema `JSON.stringify` specialization)

3. **[Chapter 03: Zero-Stat CAS Archive, Oxc Bundler & Standalone Binaries (`slice-cas`)](file:///p:/opensource/slice/docs/03_cas_bundler_and_standalone_binaries.md)**
   - Oxc `v0.152.0` AOT TypeScript compilation and comment stripping
   - Transitive ESM (`import`/`export`), CommonJS (`require`/`module.exports`), and `.json` bundling
   - Static barrel tree-shaking (`pruned_req_ids`)
   - 64-byte cache-line aligned `SLICE_CAS` v1 binary format (`mmap` with `0` `stat()` syscalls)
   - Compiling standalone zero-dependency executables with `slice compile` (`SLICEEXE`) and disassembling with `slice inspect`

4. **[Chapter 04: Security Architecture — OCap, SES Lockdown, Fuel Cage & Hardware MPK (`slice-sandbox`)](file:///p:/opensource/slice/docs/04_security_ocap_ses_and_hardware_mpk.md)**
   - Secure ECMAScript (`lockdown()`) intrinsic prototype freezing
   - Object-Capability (`CapabilityToken`) path-prefix VFS (`FsCap`), outbound network (`NetCap`), and environment (`EnvCap`) enforcement
   - Deterministic CPU instruction fuel preemption (`CpuFuelExhausted`) and byte-level memory ceiling (`MemoryCeilingExceeded`)
   - Linux `x86_64` `SYS_pkey_alloc` + `WRPKRU` Hardware Memory Protection Keys (`< 20` CPU cycles)
   - Egress Secret Guard (`--audit-egress=<secret>`) blocking raw, Base64, and Base64URL exfiltration

5. **[Chapter 05: Multi-Tenant HTTP/1.1 Server, WinterTC Bridge & Sharded Edge KV (`slice serve`)](file:///p:/opensource/slice/docs/05_http_server_wintertc_and_edge_kv.md)**
   - Shared-nothing Thread-Per-Core (`--workers=N`) HTTP/1.1 `Keep-Alive` and pipelined server (`152,376 req/s` Tier-0, `107,274 req/s` Tier-1 WinterTC)
   - WinterTC Request/Response Bridge (`Slice.request()`, `Slice.respond()`, and post-response `Slice.waitUntil(fn)`)
   - 16-way sharded in-memory Edge KV & atomic counter slab (`Slice.kv.get`, `set`, `del`, `incr`)
   - Multi-tenant dynamic `.cas` routing and zero-downtime atomic hot-swap (`--tenant-dir=<dir>`)
   - Opt-in deterministic replay mode (`--deterministic=<seed>`)

6. **[Chapter 06: AI Agent "Code Mode" Sandbox, `< 5 us` Checkpoints & Native MCP Server (`slice mcp`)](file:///p:/opensource/slice/docs/06_ai_agent_sandbox_checkpoints_and_mcp.md)**
   - Spawning nested sub-sandboxes with `Slice.evalSandbox(code, opts)` and strict capability attenuation
   - `< 5 us` in-memory VFS + KV state snapshots (`Slice.checkpoint()` and `Slice.restore(id)`) for Tree-of-Thought (ToT) agents
   - Built-in JSON-RPC 2.0 Model Context Protocol server (`slice mcp`) for Claude Code, Cursor, and autonomous LLM agents

7. **[Chapter 07: Node.js Compatibility (`node:*`), Native Intrinsics & External NPM Packages](file:///p:/opensource/slice/docs/07_node_compat_intrinsics_and_npm.md)**
   - Supported `node:*` built-in modules (`node:crypto`, `node:fs`, `node:path`, `node:os`, `node:util`, `node:buffer`, `node:process`)
   - Native Rust fast-path intrinsics (`SHA-256`, `HMAC-SHA256`, `Base64`/`Base64URL`, `SemVer`, `ms`, `bytes`, In-Memory VFS Slab)
   - Bundling and running real external `node_modules` libraries (`semver`, `ms`, `ini`, `fast-querystring`, `cookie`, `bytes`, `flatted`, `dlv`, `dset`, `mitt`)

8. **[Chapter 08: Kernel Completion Rings (`slice-io`) & Embedding Slice in Rust](file:///p:/opensource/slice/docs/08_kernel_io_rings_and_rust_embedding.md)**
   - Linux `io_uring` (`IORING_SETUP_SQPOLL` + Registered Fixed Buffers) and Windows NT `IOCP` completion rings
   - Embedding `slice-engine`, `slice-cas`, `slice-sandbox`, and `slice-io` directly in Rust applications

9. **[Chapter 09: Benchmarks, Tagged Testing & Docker Verification Suite](file:///p:/opensource/slice/docs/09_benchmarks_testing_and_docker_suite.md)**
   - Running workspace tests and tagged unit tests (`tag_n1`, `tag_n2`, `tag_n3`)
   - Running the Linux Docker benchmark suite and filtering with `SLICE_BENCH_TAGS`
   - Complete empirical benchmark tables vs Node.js (`v26.12.1`), Deno (`v2.9.8`), and Bun (`v1.4.4`)

---

## System Architecture Documents (`sys-docs/`)

In-depth internal engineering specifications and empirical research notes are also maintained in [`sys-docs/`](file:///p:/opensource/slice/sys-docs/architecture.md):
- [`sys-docs/architecture.md`](file:///p:/opensource/slice/sys-docs/architecture.md)
- [`sys-docs/benchmarks_and_comparisons.md`](file:///p:/opensource/slice/sys-docs/benchmarks_and_comparisons.md)
- [`sys-docs/production_tooling_guide.md`](file:///p:/opensource/slice/sys-docs/production_tooling_guide.md)
- [`sys-docs/research_deep_dive.md`](file:///p:/opensource/slice/sys-docs/research_deep_dive.md)

