# Chapter 07: Node.js Compatibility (`node:*`), Native Intrinsics & External NPM Packages

Slice provides a zero-overhead, lazily loaded Node.js compatibility layer (`node:*` built-in modules) backed by **native Rust fast-path intrinsics** in [`crates/slice-engine/src/lib.rs`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L3593-L4700), plus full AOT bundling for real external `node_modules/` packages.

---

## 1. Supported `node:*` Built-In Modules

To avoid cold-start parser overhead when a tenant script does not use Node built-ins, all 7 `node:*` modules are loaded **on demand** the first time `import ... from "node:<mod>"` or `require("<mod>")` is called (`__slice_load_builtin`):

| Module Specifier | Supported Functions & Properties | Native Rust Fast-Path Backing |
| :--- | :--- | :--- |
| `node:crypto` / `crypto` | `createHash("sha256").update(d).digest("hex")`, `createHmac("sha256", key).update(d).digest("hex")` | Hardware Intel/AMD **`SHA-NI`** (`compress_block_sha_ni`) + Streaming 64-byte FIPS-180-4 & RFC-2104 (`__slice_crypto_sha256` / `__slice_crypto_hmac_sha256`) |
| `node:fs` / `fs` | `readFileSync`, `writeFileSync`, `appendFileSync`, `existsSync`, `mkdirSync`, `readdirSync`, `statSync`, `unlinkSync`, `rmSync` | Thread-local **In-Memory VFS Slab** (`BOA_VFS_CACHE` / `BOA_VFS_DIRS`) with transparent OS `std::fs` fallback + OCap prefix & `..` checks |
| `node:path` / `path` | `join`, `resolve`, `basename`, `dirname`, `extname`, `sep` | Native Rust variadic `__slice_path_join` |
| `node:os` / `os` | `tmpdir()`, `platform()`, `arch()`, `EOL` | Native `__slice_os_tmpdir` |
| `node:buffer` / `buffer` | `Buffer.from(str, "utf8" \| "base64" \| "base64url")`, `.toString("utf8" \| "base64" \| "base64url")`, `Buffer.byteLength`, `Buffer.isBuffer` | Native Rust zero-table `__slice_base64_encode` & `__slice_base64_decode` |
| `node:util` / `util` | `inspect`, `deprecate`, `promisify` | Lightweight inspection & wrapper intrinsics |
| `node:process` / `process` | `process.env`, `process.cwd()`, `process.platform`, `process.arch`, `process.version`, `process.argv` | Pre-populated production `process` object |

---

## 2. Native Rust Ecosystem Fast-Path Intrinsics

In addition to `node:*` built-ins, `slice-engine` accelerates ubiquitous ecosystem primitives directly in Rust:

1. **Zero-Regex Native `SemVer` (`__slice_semver_valid`, `__slice_semver_coerce`, `__slice_semver_gt`, `__slice_semver_satisfies`):**
   - Implemented in [`semver_parse_strict_xyz`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L4641-L4664), [`semver_coerce_xyz`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L4666-L4699), and [`semver_satisfies_xyz`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L4701-L4761).
   - Evaluates `X.Y.Z` versions and `^`, `~`, `>=`, `<=`, `>`, `<`, `=` comparator ranges in ~150 nanoseconds in Rust without compiling or executing JavaScript regular expressions, falling back transparently to the full `semver` JS implementation for complex compound ranges.
2. **Zero-Regex `ms` & `bytes` Parsers/Formatters (`__slice_fast_ms`, `__slice_fast_bytes_parse`, `__slice_fast_bytes_format`):**
   - Implemented in [`fast_parse_ms`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L4449-L4506), [`fast_format_ms`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L4508-L4569), [`fast_parse_bytes`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L4572-L4606), and [`fast_format_bytes`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L4609-L4639).

---

## 3. Runnable `node:crypto` + `node:fs` + `Buffer` Example

Create `crypto_fs_demo.ts`:
```typescript
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";

const filePath = path.join(os.tmpdir(), "slice_demo_receipt.json");
const payload = JSON.stringify({ orderId: "ORD-9001", amountCents: 49900 });

// Write and read back via the OCap-enforced VFS slab
fs.writeFileSync(filePath, payload);
const readBack = fs.readFileSync(filePath, "utf8");
fs.unlinkSync(filePath);

// Compute Hardware SHA-NI SHA-256 and RFC-2104 HMAC-SHA256
const sha256Hex = crypto.createHash("sha256").update(readBack).digest("hex");
const hmacHex = crypto.createHmac("sha256", "edge-signing-key").update(readBack).digest("hex");
const b64Url = Buffer.from(readBack).toString("base64url");

console.log(JSON.stringify({
    bytes: Buffer.byteLength(readBack),
    sha256Prefix: sha256Hex.slice(0, 16),
    hmacPrefix: hmacHex.slice(0, 16),
    b64UrlPrefix: b64Url.slice(0, 16)
}));
```

Run with Slice:
```bash
slice run crypto_fs_demo.ts --warm --metrics
```

---

## 4. Bundling Real External `node_modules` Libraries

Slice bundles and executes real third-party CommonJS and ESM packages installed in `node_modules/` across [`docker-tests/apps/`](file:///p:/opensource/slice/docker-tests/README.md):
- `semver@7.6.3`, `ms@2.1.3`, `ini@4.1.3` (`02-npm-ecosystem-suite`)
- `fast-querystring@1.1.2` (`04-edge-api-gateway`)
- `cookie@0.6.0` (`05-jwt-oauth-session-guard`)
- `bytes@3.1.2`, `flatted@3.3.1` (`06-telemetry-metrics-pipeline`)
- `dlv@1.1.3`, `dset@3.1.3` (`07-graphql-ast-validator`)
- `mitt@3.0.1` (`08-event-driven-state-journal`)

To bundle your own folder project with `node_modules/`:
```bash
npm install semver ms ini
slice pack . ./app.cas
slice run ./app.cas --warm --metrics
```

