# Chapter 07: Node.js Compatibility (`node:*`), NPM Packages & Migration Guide (Node / Bun / Deno to Slice)

Moving an existing TypeScript or JavaScript codebase to Slice does not require rewriting your business logic or giving up your favorite `npm` packages. Slice provides a zero-cold-start **Node.js built-in module layer (`node:*`)** backed by native Rust intrinsics, bundles standard CommonJS and ESM packages from `node_modules/`, and supports the standard WinterTC Web APIs (`fetch`, `Request`, `Response`, `Headers`, `URL`, `URLSearchParams`, `ReadableStream`).

This chapter provides the complete API reference for every built-in `node:*` module, explains how to use external `npm` libraries, and gives you step-by-step migration guides from **Node.js (Express/Fastify)**, **Bun**, **Deno**, and **Cloudflare Workers**.

---

## 1. Complete `node:*` Built-In Module Reference

All built-in modules can be imported with or without the `node:` prefix (`import * as fs from "node:fs"` or `import * as fs from "fs"`, as well as CommonJS `require("node:fs")` or `require("fs")`). They are loaded lazily on first use (`0.00 ns` cost if unused).

### 1.1 `node:fs` / `fs` (OCap-Guarded In-Memory VFS Slab + OS Disk Fallback)
Every `node:fs` operation checks your `allow_read` / `allow_write` prefixes, blocks `..` traversal, serves hot files from Slice's in-memory VFS slab (`BOA_VFS_CACHE`), and automatically snapshots/restores during `Slice.checkpoint()` / `Slice.restore()` and `slice test`!

```typescript
import * as fs from "node:fs";

// Write & append files (requires allow_write prefix)
fs.writeFileSync("/tmp/report.json", JSON.stringify({ ok: true }));
fs.appendFileSync("/tmp/access.log", "2026-10-25 GET /api/v1\n");

// Read files & check existence (requires allow_read prefix)
const exists: boolean = fs.existsSync("/tmp/report.json");
const content: string = fs.readFileSync("/tmp/report.json", "utf8");

// Directories, metadata & cleanup
fs.mkdirSync("/tmp/shards");
const files: string[] = fs.readdirSync("/tmp");
const stats: { size: number } = fs.statSync("/tmp/report.json");
fs.unlinkSync("/tmp/report.json");
fs.rmSync("/tmp/shards");
```

### 1.2 `node:crypto` / `crypto` (Hardware Intel/AMD `SHA-NI` Accelerated)
```typescript
import * as crypto from "node:crypto";

// FIPS-180-4 SHA-256 hex digest
const sha256Hex: string = crypto
    .createHash("sha256")
    .update("order-payload-1001")
    .digest("hex");

// RFC-2104 HMAC-SHA256 hex signature
const hmacHex: string = crypto
    .createHmac("sha256", "whsec_signing_key")
    .update("webhook-body")
    .digest("hex");
```

### 1.3 `node:path` / `path`
```typescript
import * as path from "node:path";

const fullPath = path.join("/workspace", "tenants", "alpha", "config.json");
const resolved = path.resolve("/workspace", "src/index.ts");
const dir = path.dirname(fullPath);              // "/workspace/tenants/alpha"
const base = path.basename(fullPath);            // "config.json"
const nameOnly = path.basename(fullPath, ".json"); // "config"
const ext = path.extname(fullPath);              // ".json"
const sep = path.sep;                            // "/"
```

### 1.4 `node:buffer` / `buffer` (and Global `Buffer`)
```typescript
import { Buffer } from "node:buffer";

// UTF-8 -> Base64 and Base64URL encoding
const b64: string = Buffer.from("hello:world", "utf8").toString("base64");
const b64Url: string = Buffer.from("{\"alg\":\"HS256\"}", "utf8").toString("base64url");

// Base64 / Base64URL -> UTF-8 decoding
const decoded: string = Buffer.from(b64Url, "base64url").toString("utf8");

// Byte length calculation
const bytes: number = Buffer.byteLength("Slice MicroVM");
```

### 1.5 `node:url` / `url` (and Global `URL` & `URLSearchParams`)
```typescript
import { URL, URLSearchParams } from "node:url";

const u = new URL("/v2/search?q=slice&tag=edge", "https://api.slice.dev");
u.searchParams.append("tag", "rust");
u.searchParams.sort();
console.log(u.href); // "https://api.slice.dev/v2/search?q=slice&tag=edge&tag=rust"
```

### 1.6 `node:os` / `os`, `node:util` / `util`, & `node:process` / `process`
```typescript
import * as os from "node:os";
import * as util from "node:util";
import process from "node:process";

const tmp: string = os.tmpdir();      // e.g. "/tmp"
const plat: string = os.platform();   // "linux"
const arch: string = os.arch();       // "x64"

const str: string = util.inspect({ a: 1, b: [2, 3] });
const stage: string = process.env.NODE_ENV || "production";
```

### 1.7 `slice:test` / `node:test` / `bun:test`
```typescript
import { describe, test, it, beforeEach, afterEach, expect } from "slice:test";
// Also works with:
// import { describe, test, expect } from "node:test";
// import { describe, test, expect } from "bun:test";
```

---

## 2. Using External `npm` Packages (`node_modules/`)

You can install packages into your project folder using `npm install`, `pnpm add`, or `bun install`, and then run or bundle your project with `slice run`, `slice pack`, or `slice compile`.

```bash
npm init -y
npm install semver ms ini cookie bytes dlv dset mitt fast-querystring flatted
slice run . --warm --metrics
```

### Verified Production NPM Packages in Slice's 16-App Suite
All 16 production applications in [`docker-tests/apps/`](file:///p:/opensource/slice/docker-tests/README.md) bundle and execute real third-party `node_modules` libraries and verify **100% byte-for-byte identical output** against Node.js, Deno, and Bun:

| NPM Package | Common Use Case | Slice Optimization Applied |
| :--- | :--- | :--- |
| **`semver`** (`@7.6.3`) | Semantic version parsing, coercion & range matching | Barrel tree-shaking (prunes 30+ unused files) + Native Rust `__slice_semver_*` fast path (`0.27 ms` warm!) |
| **`ms`** (`@2.1.3`) | Human-readable time window parsing/formatting (`"14d"`, `"2h"`) | Native Rust `__slice_fast_ms` zero-regex fast path |
| **`bytes`** (`@3.1.2`) | Byte unit parsing/formatting (`"64KB"`, `"4.56MB"`) | Native Rust `__slice_fast_bytes_*` + compile-time constant folding |
| **`cookie`** (`@0.6.0`) | RFC 6265 HTTP `Cookie` parsing & `Set-Cookie` serialization | Fast-path RFC 6265 serializer/parser lowering |
| **`ini`** (`@4.1.3`) | `.ini` configuration topology parsing | Fast-path section/key-value decoder |
| **`fast-querystring`** (`@1.1.2`) | Fastify URL query string parser | Memoized query cache + lazy `fast-decode-uri-component` |
| **`flatted`** (`@3.3.1`) | Circular object graph serialization (OpenTelemetry spans) | Map-indexed single-pass cycle serializer/deserializer |
| **`dlv`** (`@1.1.3`) & **`dset`** (`@3.1.3`) | Deep nested object property lookup and mutation | Direct CommonJS/ESM bundling |
| **`mitt`** (`@3.0.1`) | Event emitter & CQRS pub/sub bus | Direct CommonJS/ESM bundling |

---

## 3. Step-by-Step Migration Guides

### 3.1 Migrating from Node.js (`Express` / `Fastify` / `http.createServer`) to `slice serve`

**Before (Node.js + Express):**
```typescript
import express from "express";
const app = express();
app.use(express.json());

app.get("/api/users/:id", (req, res) => {
    res.json({ id: req.params.id, env: process.env.STAGE });
});

app.post("/api/orders", (req, res) => {
    res.status(201).json({ received: req.body });
});

app.listen(3000);
```

**After (Slice Standard WinterTC `export default { fetch }`):**
```typescript
export default {
    async fetch(req: Request, env: Record<string, string>): Promise<Response> {
        const url = new URL(req.url);

        if (req.method === "GET" && url.pathname.startsWith("/api/users/")) {
            const id = url.pathname.slice("/api/users/".length);
            return Response.json({ id, env: env.STAGE });
        }

        if (req.method === "POST" && url.pathname === "/api/orders") {
            const body = await req.json();
            return Response.json({ received: body }, { status: 201 });
        }

        return Response.json({ error: "Not Found" }, { status: 404 });
    },
};
```
**What you gain immediately:**
- `2.7x` higher HTTP throughput than Node.js (`152k req/s` Tier-0 / `107k req/s` Tier-1 vs `55k req/s` Node.js).
- `10x` lower memory footprint (`~10 MB` vs `~70 MB`).
- Every request runs inside an isolated compartment with CPU fuel limits (`--fuel`) and Egress DLP (`--env-file=.env`).

---

### 3.2 Migrating from Bun (`Bun.serve` & `bun test`) to Slice

1. **HTTP Server (`Bun.serve` -> `export default { fetch }`):**
   - Bun already supports `export default { fetch(req) }`. Your existing `fetch(req)` handler runs unchanged under `slice serve`!
   - If your project serves static files, pass `--static=./public` to `slice serve` or `slice compile`.
2. **Test Suite (`import { describe, test, expect } from "bun:test"`):**
   - Slice natively recognizes `"bun:test"`! You don't even have to change your import statement — just run `slice test` and your tests automatically gain `< 5 us` VFS + KV state rollback after every `test()` block.

---

### 3.3 Migrating from Deno (`Deno.serve` & `Deno.openKv`) to Slice

1. **Relative TypeScript Imports (`.ts` extensions):**
   - Like Deno, Slice supports explicit `.ts` extensions in imports (`import { foo } from "./foo.ts"`) as well as extensionless (`"./foo"`) and `.js` (`"./foo.js"`) imports.
2. **CLI Permission Flags:**
   - Deno's `--allow-read=/path`, `--allow-write=/path`, `--allow-net=host`, and `--allow-env=KEY=VAL` work with the **exact same flag names** in `slice run` and `slice serve` — plus you can persist them cleanly in `slice.toml` so you don't have to repeat long CLI flags!
3. **Key-Value Store (`Deno.openKv` -> `Slice.kv`):**
   - Replace async `await kv.get(["rate", id])` / `await kv.set(["rate", id], val)` with synchronous, `< 0.6 us` sharded `Slice.kv.get("rate:" + id)`, `Slice.kv.set("rate:" + id, val, ttlMs)`, and `Slice.kv.incr("rate:" + id, 1)`.
