# Slice (Nexus-JS): Developer How-To Guides (`docs/guides/`)

This directory contains practical step-by-step developer guides, production deployment how-tos, and ecosystem integration recipes for Slice (`v0.10.12`):

## Available Guides

* **[Guide 01: Building & Deploying Hono (`v4`), UnJS, Express, and SvelteKit (`v3`) Edge Microservices](./01_hono_and_unjs_edge_microservices.md)**: Scaffolding (`slice init --template=sveltekit`), serving (`slice serve`), and compiling (`slice compile`) `hono`, `itty-router`, UnJS (`ufo`, `destr`, `ohash`, `radix3`), `Express`/`Polka` (`node:http`), and full-stack SvelteKit 3 + Svelte 5 SSR applications.
* **[Guide 02: TypeScript IDE Setup (`slice.d.ts`), Linting (`oxlint` / `Biome`), & `.wasm` Modules](./02_typescript_ide_linting_and_wasm.md)**: Configuring `slice check`, `tsc --noEmit`, `oxlint`, `Biome`, and replacing `.node` C++ addons with 5-Ring sandboxed `.wasm` modules.
* **[Getting Started, `slice.toml` & CLI Reference](../general/01_getting_started_and_cli.md)**: Quickstart, project scaffolding (`slice init`), `.env` Auto-DLP configuration, and complete 14-command CLI reference.
* **[Bundling (`.cas`) & Single-Binary Compilation (`slice compile`)](../general/03_cas_bundler_and_standalone_binaries.md)**: Zero-stat `.cas` bundles, self-contained `SLICEEXE` binaries, and static asset embedding (`--static`).
* **[HTTP Server (`slice serve`), TLS 1.3, WinterTC & Edge KV](../general/05_http_server_wintertc_and_edge_kv.md)**: Multi-tenant HTTP/1.1 & WebSocket MicroVM hosting, native TLS termination, SSE, and sharded `Slice.kv`.
* **[AI Agent Sandboxes (`Slice.evalSandbox`), Checkpoints & `slice mcp`](../general/06_ai_agent_sandbox_checkpoints_and_mcp.md)**: Sub-millisecond LLM code execution, `< 5 us` time-travel checkpoints, and Claude Code / Cursor MCP server setup.
* **[Node.js Compatibility (`node:*`) & External NPM Packages](../general/07_node_compat_intrinsics_and_npm.md)**: Using npm packages (`node_modules/`), package managers (`npm`, `pnpm`, `yarn`, `bun`), CommonJS/ESM interop, and built-in `node:*` modules.
* **[Runtime Compatibility Matrix](../general/COMPATIBILITY.md)**: Full WinterTC Web Standards, `node:*` module support, and WebAssembly (`wasmi`) reference.
* **[Top-Level Documentation Index (`docs/README.md`)](../README.md)**: Complete navigation across User Documentation, Guides, System Architecture Deep Dives, and Live Benchmark Reports.
