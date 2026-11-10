# Slice (Nexus-JS) — Developer How-To Guides (`docs/guides/`)

This directory contains practical step-by-step developer guides, production deployment how-tos, and ecosystem integration recipes for Slice (`v0.10.7`):

## Available Guides

* **[Guide 01: Building & Deploying Hono (`v4`), UnJS, and SvelteKit (`v3`) Edge Microservices](./01_hono_and_unjs_edge_microservices.md)**: Scaffolding (`slice init --template=sveltekit`), serving (`slice serve`), and compiling (`slice compile`) `hono`, `itty-router`, UnJS (`ufo`, `destr`, `ohash`, `radix3`), and full-stack SvelteKit 3 + Svelte 5 SSR applications.
* **[Guide 02: TypeScript IDE Setup (`slice.d.ts`), Linting (`oxlint` / `Biome`), & `.wasm` Modules](./02_typescript_ide_linting_and_wasm.md)**: Configuring `tsc --noEmit`, `oxlint`, `Biome`, and replacing `.node` C++ addons with 5-Ring sandboxed `.wasm` modules.
* **[Production Tooling & Deployment Guide](../system/07_production_tooling_guide.md)**: End-to-end guide for `slice pack` (Zero-Stat `.cas` bundles), `slice compile` (self-contained `SLICEEXE` binaries), `slice serve` (multi-tenant HTTP/1.1 & WebSocket MicroVM hosting), `slice mcp` (AI Agent "Code Mode" MCP server), and `slice doctor`.
* **[Getting Started, `slice.toml` & CLI Reference](../general/01_getting_started_and_cli.md)**: Quickstart, project scaffolding (`slice init`), `.env` Auto-DLP configuration, and complete CLI command reference.
* **[Node.js Compatibility (`node:*`) & External NPM Packages](../general/07_node_compat_intrinsics_and_npm.md)**: Using npm packages (`node_modules/`), package managers (`npm`, `pnpm`, `yarn`, `bun`), CommonJS/ESM interop, and built-in `node:*` modules.
* **[Runtime Compatibility Matrix](../general/COMPATIBILITY.md)**: Full WinterTC Web Standards, `node:*` module support, and WebAssembly (`wasmi`) reference.
