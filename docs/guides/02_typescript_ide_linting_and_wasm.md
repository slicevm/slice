# Guide 02: TypeScript IDE Setup (`slice.d.ts`), Linting (`oxlint` / `Biome`), & Replacing `.node` Addons with `.wasm`

This guide covers day-to-day TypeScript IDE tooling, sub-millisecond syntax checking (`slice check`), static type-checking (`tsc --noEmit`), fast Rust-based linting and formatting (`oxlint` and `Biome`), and migrating native C/C++ `.node` addons to zero-stat `.wasm` modules.

---

## 1. Automatic IDE IntelliSense, `slice check`, & `tsc --noEmit` (`slice.d.ts` + `tsconfig.json`)

Running `slice init` generates both `slice.d.ts` and `tsconfig.json` in your project root (and `slice init [dir] --template=sveltekit` scaffolds a full SvelteKit 3 + Svelte 5 project):
- **`slice.d.ts`** provides ambient TypeScript types for `Slice.*` (`Slice.kv`, `Slice.evalSandbox`, `Slice.checkpoint`, `Slice.restore`, `Slice.releaseCheckpoint`, `Slice.connect`, `Slice.streamSSE`, `Slice.waitUntil`), `WinterTC.route`, `env()`, `slice:test`, `slice:jsx`, and `cloudflare:sockets`.
- **`tsconfig.json`** configures `"moduleResolution": "bundler"`, `"jsx": "react-jsx"`, `"jsxImportSource": "slice:jsx"`, and `"noEmit": true`.

Run sub-millisecond Oxc AST + module-graph syntax validation or full TypeScript type-checking in CI and pre-commit hooks without emitting `.js` files:
```bash
# Fast Oxc AST + module dependency graph syntax validation (zero code execution):
slice check .

# Full TypeScript type-checking via tsc:
npx tsc --noEmit
```

---

## 2. Linting & Formatting with `oxlint` and `Biome`

Because Slice uses the **Oxc (`v0.152.0`)** parser and compiler in `slice-cas`, pairing Slice with `oxlint` or `Biome` gives you an all-Rust toolchain from linter to bundler to runtime:

```bash
# Lint TypeScript/TSX with Oxc's linter (50-100x faster than ESLint):
npx oxlint ./src ./tests

# Format & check with Biome:
npx @biomejs/biome check --write ./src ./tests
```

---

## 3. Replacing `.node` Native Addons with Pure-Rust `WebAssembly` (`.wasm`)

Node.js `.node` addons (`process.dlopen`) load unsandboxed native shared libraries directly into the host process address space, bypassing OCap filesystem/network controls, CPU fuel limits, and per-tenant memory ceilings.

In Slice, compile performance-critical C, C++, Rust, or Zig kernels to `wasm32-unknown-unknown` and import the `.wasm` binary directly:

```typescript
import wasmBytes from "./crypto_kernel.wasm";

const { instance } = await WebAssembly.instantiate(wasmBytes, {
    env: {
        host_log: (code: number) => console.log("WASM status:", code),
    },
});

const exports = instance.exports as {
    fnv1a_32: (ptr: number, len: number) => number;
    memory: WebAssembly.Memory;
};
```
See [`tests/apps/23-wasm-crypto-and-simd-kernel`](../../tests/apps/23-wasm-crypto-and-simd-kernel/src/index.ts) and [`tests/apps/24-tsx-ssr-and-wasm-edge-pipeline`](../../tests/apps/24-tsx-ssr-and-wasm-edge-pipeline/src/index.ts) for working examples.
