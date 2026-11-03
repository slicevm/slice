# Chapter 03: Bundling Apps (`slice pack`), Single-File Executables (`slice compile`) & Bytecode Inspection

When you deploy a Node.js, Deno, or Bun application to production, you typically have to ship hundreds of source files or a massive `node_modules/` directory — and on every cold start, the runtime has to issue hundreds of OS `stat()`, `open()`, and `read()` syscalls just to locate your modules on disk.

Slice solves this with **Zero-Stat Content-Addressed Storage (`.cas`)** and **Self-Contained Native Executables (`SLICEEXE`)**.

This chapter shows you how to structure multi-file TypeScript projects, bundle your code and `node_modules/` with `slice pack`, embed frontend static assets (`--static`), compile everything into a single zero-dependency binary with `slice compile`, and inspect your bundles with `slice inspect`.

---

## 1. How Slice Resolves Your Project Structure Automatically

You can pass a single `.ts`/`.js` file or an entire project directory to `slice run`, `slice pack`, `slice compile`, or `slice serve`.

When you pass a **project directory** (e.g., `slice pack ./my-app ./my-app.cas`), Slice resolves your entrypoint in the following priority order:
1. **Explicit `--entry` CLI flag:** `slice pack ./my-app ./my-app.cas --entry=./src/worker.ts`
2. **`slice.toml` `[package] entrypoint`:** Reads `entrypoint = "./src/index.ts"` from `./my-app/slice.toml`.
3. **`package.json`:** Reads `"exports": { ".": "..." }`, `"module"`, or `"main"`.
4. **Conventional Fallbacks:** Checks `src/index.ts`, `src/index.tsx`, `index.ts`, `index.tsx`, `src/index.js`, `src/index.jsx`, `index.js`, `index.jsx`, `src/main.ts`, and `main.ts`.

### Supported Module Formats & Imports
Inside your project, you can freely mix and match:
- **TypeScript, TSX/JSX & ESM (`import` / `export`):**
  ```tsx
  import { PricingEngine } from "./pricing_engine.ts"; // or "./pricing_engine" or "./pricing_engine.js"
  import type { OrderReceipt } from "./types.ts";
  import { renderToString } from "slice:jsx";
  import semver from "semver";
  import * as crypto from "node:crypto";

  export function ReceiptCard(props: { id: string; total: number }) {
      return <article className="receipt"><h2>Order {props.id}</h2><p>{props.total}</p></article>;
  }
  ```
- **CommonJS (`require` / `module.exports` / `exports`):**
  ```javascript
  const ms = require("ms");
  const { parse } = require("fast-querystring");
  ```
- **JSON Files (`.json`):**
  ```typescript
  import config from "./config/default.json";
  ```
- **Node & Slice Built-Ins:**
  - `"node:crypto"`, `"node:fs"`, `"node:fs/promises"`, `"node:path"`, `"node:os"`, `"node:buffer"`, `"node:events"`, `"node:timers"`, `"node:timers/promises"`, `"node:url"`, `"node:util"`, `"node:util/types"`, `"node:assert"`, `"node:assert/strict"`, `"node:process"`, `"node:test"`, `"slice:test"`, `"bun:test"`, `"slice:jsx"`, `"react/jsx-runtime"`, `"preact/jsx-runtime"`, `"hono/jsx"`

### Modern ESM NPM Packages & Conditional `"exports"` Resolution
Slice's zero-stat bundler ([`crates/slice-cas/src/lib.rs`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs)) includes a full **Oxc AST-driven ESM-to-CJS module linker** (`try_transform_esm_to_cjs_ast`) and **Node.js `package.json` `"exports"` resolver** (`resolve_package_exports_value`):
- **Conditional `"exports"` Priority:** Evaluates nested condition maps in edge-first priority order: `"workerd"` -> `"worker"` -> `"edge-light"` -> `"browser"` -> `"import"` -> `"require"` -> `"node"` -> `"default"` (automatically skipping `.d.ts` type declarations).
- **Subpath & Wildcard `"exports"`:** Resolves exact subpaths (such as `import { jwtVerify } from "jose/jwt/verify"` or `import { createRouter } from "radix3"`) as well as wildcard patterns (`"./*": "./dist/*.mjs"`).
- **TypeScript `.js` -> `.ts` Specifier Resolution:** Supports standard TypeScript ESM imports written with `.js` extensions (`import { x } from "./foo.js"` automatically resolves `./foo.ts` when `./foo.js` is not on disk).
- **Full ES2024 Import/Export Lowering:** Rewrites multi-line `import { a, b as c } from "..."`, namespace imports (`import * as ns`), default + named imports, live-getter re-exports (`export { x } from "./sub"`, `export * from "./sub"`, `export * as ns from "./sub"`), and exported declarations (`export class`, `export async function`, `export const { a, b } = ...`) via Oxc AST span analysis without fragile regexes.

---

## 2. Packing a Zero-Stat `.cas` Archive (`slice pack`)

`slice pack` compiles your TypeScript code using **Oxc (`v0.152.0`)**, bundles all transitive local modules and `node_modules/` dependencies, tree-shakes unused barrel submodules, hashes every artifact with **BLAKE3-256**, and writes a single **64-byte cache-line aligned `.cas` file**.

### 2.1 Basic Usage
```bash
# Pack a single TypeScript file
slice pack ./src/index.ts ./bundle.cas

# Pack an entire folder project (with package.json, slice.toml, src/, and node_modules/)
slice pack ./tests/apps/02-npm-ecosystem-suite ./npm-app.cas

# Pack with a custom entrypoint and embedded static frontend assets
slice pack ./my-webapp ./webapp.cas --entry=./src/server.ts --static=./public
```

### 2.2 What Happens Inside `slice pack` (And Why It Makes Cold Starts So Fast)

1. **Oxc Ahead-of-Time TypeScript Compilation & Comment Stripping:**
   - All TypeScript types, interfaces, and JSDoc/license comments are stripped at build time.
2. **Dense Integer Module IDs (`0..N-1`):**
   - Instead of resolving string paths like `"./node_modules/semver/classes/semver.js"` at runtime, Slice assigns every module a dense integer ID (`0`, `1`, `2`, ...) so internal `require()` calls are O(1) array lookups.
3. **Static Barrel Tree-Shaking (`50%+` Smaller Bundles):**
   - Many popular npm packages (like `semver`) have an `index.js` barrel file that `require()`s 35+ submodules even if your app only calls `semver.satisfies()`.
   - Slice's bundler statically analyzes which exports your application actually uses and **prunes unused barrel submodules before reading them from disk**, cutting `.cas` bundle size in half.
4. **Embedded `slice.toml` Security Manifest (`manifest:/slice.toml`):**
   - If your project folder contains a `slice.toml` file, `slice pack` embeds it directly inside the `.cas` archive. When you run `slice run ./app.cas` or `slice serve ./app.cas`, Slice automatically applies your `allow_read`, `allow_write`, `allow_net`, `allow_env`, `fuel`, and `memory_kb` settings!
5. **Memory-Mapped Execution (`0` `stat()` Syscalls):**
   - At runtime, `slice run ./app.cas` maps the `.cas` file directly into virtual memory via `mmap(MAP_SHARED)` and executes with **zero filesystem `stat()` calls**.

---

## 3. Compiling Single-File Native Executables (`slice compile`)

Want to distribute a CLI tool, microservice, or edge worker to servers or customers without installing `slice`, `node`, or any source files on the target machine?

Use **`slice compile`** to fuse the native `slice` runtime binary and your `.cas` bundle into a **single, zero-dependency executable (`SLICEEXE`)**:

```bash
# Compile a folder app (including node_modules/ and slice.toml) into a standalone executable
slice compile ./tests/apps/01-fintech-checkout -o ./fintech-checkout

# Or compile a full-stack web app (Backend API + ./public Frontend UI) into one binary
slice compile ./my-webapp -o ./my-webapp-bin --static=./public
```

### Running Your Compiled Binary
The generated binary (`./fintech-checkout` on Linux/macOS, `.\fintech-checkout.exe` on Windows) runs directly and accepts `--warm`, `--metrics`, and `--quiet`:

```bash
./fintech-checkout --warm --metrics
```
Output:
```text
{"app":"01-fintech-checkout","ordersProcessed":50,"approvedCount":50,"aggregateRevenueCents":4932364,"lastSha256Signature":"636588368661f40b"}
[Slice Standalone] Tier: Tier1Baseline | Instantiate: 0.08 us | Exec: 673.00 us | Total: 673.08 us | Memory: 4096 B | Stat Syscalls: 0
```

> **How `SLICEEXE` Works Without Temp-File Extraction:**
> Unlike Node `sea` or `pkg` tools that unpack megabytes of files into `/tmp` on startup, a `SLICEEXE` binary memory-maps its own executable file at the exact 64-byte aligned offset where the embedded `.cas` archive begins. Cold start takes microseconds and writes **0 bytes** to `/tmp`.

---

## 4. Inspecting Bundles, Offsets & Bytecode (`slice inspect`)

Use `slice inspect` whenever you want to verify what modules were bundled into a `.cas` archive, check 64-byte cache-line alignment, or see the exact `SLICESNP` / `SLICEBOA` bytecode generated for a `.ts` script.

### 4.1 Inspecting a `.cas` Archive
```bash
slice pack ./tests/apps/01-fintech-checkout ./fintech.cas
slice inspect ./fintech.cas
```
Example output:
```text
=========================================================================
  SLICE ZERO-STAT CAS ARCHIVE INSPECTOR: ./fintech.cas
=========================================================================
  CAS Format Version : v1
  Total Mapped Size  : 8192 bytes
  Entrypoint Hash    : 4a8e9c1b7d3f2056...
  Static Web Assets  : 0 embedded files
  Modules Indexed    : 1
-------------------------------------------------------------------------
  [000] Specifier : ./tests/apps/01-fintech-checkout
        BLAKE3    : 4a8e9c1b7d3f2056...
        Source    : offset=448 (64B aligned=true), len=3412 bytes
        Bytecode  : offset=3904 (64B aligned=true), len=3420 bytes
=========================================================================
```

### 4.2 Inspecting a TypeScript Script's Bytecode
```bash
slice inspect ./src/index.ts
```
- For **Tier-0 / Tier-2 (`SLICESNP`)** scripts, `slice inspect` prints the disassembled instruction table (`CONST_NUM`, `CONST_STR`, `MONO_FOR_T2`, `AST_EXEC`, `EXPORT_SYM`).
- For **Tier-1 (`SLICEBOA`)** bundles, `slice inspect` prints the bundled JavaScript payload, `// #slice-source:` error-mapping markers, and the `//__SLICE_RC:<hash>` persistent realm cache key.

---

## 5. Source-Line Error Mapping in Multi-File Bundles

When `slice-cas` bundles multiple TypeScript files from `src/` into a single `.cas` archive, it automatically inserts lightweight `// #slice-source: <short_path>|<full_path>` markers before each user module.

If an uncaught error is thrown inside one of your bundled TypeScript modules at runtime, Slice's error mapper (`slice_cas::map_bundled_error_location`) automatically translates the bundled line number back to your **original TypeScript file and line number** (e.g., `src/pricing_engine.ts:42`), so you never have to hunt through a concatenated bundle to debug an exception.
