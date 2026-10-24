# Chapter 02: Hybrid Three-Tier Execution Engine (`slice-engine`)

Implemented in [`crates/slice-engine/src/lib.rs`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs), Slice's execution engine is designed around a single engineering rule: **execute every workload on the lightest possible tier, eliminate runtime parsing via zero-copy `mmap` slices and persistent realm closures, and lower typed loops and hot standard-library calls directly into native Rust/SIMD instructions**.

---

## 1. Overview of the Three Execution Tiers ([`ExecutionTier`](file:///p:/opensource/slice/crates/slice-sandbox/src/lib.rs#L17-L26))

```text
+-------------------------------------------------------------------------------------------------+
|                            Slice Hybrid Three-Tier Execution Pipeline                           |
|                                                                                                 |
|  TypeScript / JavaScript / .cas Archive                                                         |
|         |                                                                                       |
|         +---> [Tier 0: SLICESNP Snapshot Interpreter]                                           |
|         |       - 0.98 us Cold Start | 4.0 KB Memory | 16-Byte Cache-Line JsValue               |
|         |       - Fast scalar math, env(), WinterTC.route(), static CAS imports                 |
|         |                                                                                       |
|         +---> [Tier 1: Zero-Copy SLICEBOA + Pre-Warmed Boa ES2024+ Pool]                        |
|         |       - 0.02 us - 0.21 us mmap Instantiation | Full ES2024+ Classes, Closures, Map    |
|         |       - WarmBoaRealm::loaded_bundles (__slice_rc) direct JsObject::call (0 parsing!)  |
|         |                                                                                       |
|         +---> [Tier 2: Peak Native SIMD & Monomorphic Kernels]                                  |
|                 - Oxc Type-Directed MonomorphicForLoop (: number -> 64-bit register loop)       |
|                 - Hardware Intel/AMD SHA-NI SHA-256 & RFC-2104 HMAC-SHA256                      |
|                 - FibKernel, NumericLoopKernel, BLAKE3 CryptoDigest                             |
+-------------------------------------------------------------------------------------------------+
```

---

## 2. Tier 0: `SLICESNP` Snapshot VM & 16-Byte `JsValue`

For lightweight serverless routing, scalar expressions, and static module graphs, [`compile_source_to_bytecode`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L1435-L1637) emits a binary `SLICESNP` snapshot (`SNAPSHOT_MAGIC = b"SLICESNP"`):

- **16-Byte Cache-Line Aligned [`JsValue`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L24-L40):**
  - `JsValue` wraps heap types (`Rc<String>`, `Rc<RefCell<Vec<JsValue>>>`, `Rc<RefCell<FxHashMap<String, JsValue>>>`, `Rc<JsFunction>`) so `std::mem::size_of::<JsValue>() == 16`. Exactly **4 `JsValue`s fit into a single 64-byte CPU L1 cache line**.
- **Thread-Local Decoded Snapshot Cache ([`SNP_MODULE_CACHE`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L3229-L3252)):**
  - `decode_slicesnp_cached` hashes the `SLICESNP` payload with `FxHasher` and caches the decoded `Arc<BytecodeModule>` per worker thread, eliminating repeated deserialization in `slice serve` and multi-tenant swarms.
- **Zero-Allocation Isolate Spawn ([`SliceIsolate::spawn`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L1709-L1731)):**
  - `TenantCompartment::new` uses a static `OnceLock<IntrinsicRegistry>` reference instead of cloning 7 strings per tenant, spawning a fresh hardware-protected isolate in **`0.98 microseconds` (`980 ns`)**.

### Runnable Tier-0 Example

Create `tier0_demo.ts`:
```typescript
const basePrice: number = 1400;
const tax: number = 125;
const routeStatus: string = WinterTC.route("GET", "https://edge.slice.local/api/v1/status");
console.log(basePrice + tax);
console.log(routeStatus);
```

Run and inspect the generated `SLICESNP` opcodes:
```bash
slice inspect tier0_demo.ts
slice run tier0_demo.ts --metrics
```

---

## 3. Tier 1: Zero-Copy `SLICEBOA`, Pre-Warmed `BOA_CONTEXT_POOL` & Direct `JsObject::call`

When an application uses full ECMAScript 2024+ features (`class`, `new Map`, `new Set`, `RegExp`, `try/catch`, `node:*` built-ins, `Slice.*` host APIs, or external `node_modules` packages), `compile_source_to_bytecode` emits a `b"SLICEBOA"` frame:

1. **Zero-Copy `mmap` String Slicing (`0.02 us - 0.21 us` Instantiation):**
   - `SLICEBOA` stores the Oxc-transpiled and block-wrapped (`{\n...\n}`) UTF-8 JavaScript directly after the 8-byte `b"SLICEBOA"` header.
   - At runtime, [`SliceIsolate::execute_bytecode`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L1946-L1975) slices `&bytecode[8..]` directly from the memory-mapped `.cas` file with zero heap copies or JSON unescaping.
2. **Thread-Local Pre-Warmed Realm Pool ([`BOA_CONTEXT_POOL`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L3562-L3590)):**
   - Pre-initializes up to 8 `WarmBoaRealm` instances per worker thread (`SliceIsolate::prewarm_boa_pool(count)`), wrapped in `ManuallyDrop` for deterministic TLS teardown safety.
   - Uses a compact `< 1 KB` `CORE_PRELUDE` and loads `node:crypto`, `node:fs`, `node:path`, `node:os`, `node:util`, `node:buffer`, and `node:process` **lazily on first `require()`** via `__slice_load_builtin`.
3. **Persistent Realm Module Cache (`__slice_rc`) & Direct `JsObject::call` ([`exec_boa_full_es_vm`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L3108-L3135)):**
   - Every bundled application includes a 16-hex-char BLAKE3/FxHash fingerprint (`//__SLICE_RC:<16_hex_chars>`) and registers its entrypoint function in `globalThis.__slice_rc["<key>"]`.
   - On the first execution (or during `SliceIsolate::prewarm_boa_bundle`), [`WarmBoaRealm::loaded_bundles`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L3223-L3226) caches the `boa_engine::JsObject` function handle in a Rust `FxHashMap<u64, JsObject>`.
   - On every subsequent execution (`slice serve` or `slice run --warm`), Rust invokes `fn_obj.call(&JsValue::undefined(), &[], &mut realm.ctx)` **directly without running the JavaScript lexer, parser, or bytecompiler at all**.

### Runnable Tier-1 Example (Cold vs. Warm Pool)

```bash
slice pack ./docker-tests/apps/02-npm-ecosystem-suite ./npm_suite.cas
# Cold Realm Execution
slice run ./npm_suite.cas --metrics
# Pre-Warmed Realm + Cached __slice_rc JsObject Execution (0.27 ms!)
slice run ./npm_suite.cas --warm --metrics
```

---

## 4. Tier 2: Peak Native SIMD & Oxc Type-Directed Lowering

Slice uses TypeScript type annotations at compile time (`slice pack` / `slice compile`) to unlock native Tier-2 optimizations before types are erased:

1. **Oxc Type-Directed Monomorphic Loop Promotion ([`BytecodeOp::MonomorphicForLoop`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L334-L340)):**
   - [`analyze_typescript_semantics`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs#L88-L195) walks the Oxc AST and symbol table to identify variables explicitly annotated with `: number`.
   - Standard `for (let i: number = 0; i < N; i++) { acc += i * k; }` loops over `: number` bindings are promoted directly to `BytecodeOp::MonomorphicForLoop`, executing in a tight native 64-bit CPU register loop instead of interpreting per-iteration AST nodes.
2. **Schema-Driven `JSON.stringify` Specialization ([`build_fast_json_template_for_schema`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs#L197-L230)):**
   - When a variable is typed with a flat TypeScript `interface` (such as `const rec: AccessLogRecord = ...`), `strip_typescript_raw` replaces dynamic `JSON.stringify(rec)` calls with direct monomorphic string template concatenations (`'{"ts":' + rec.ts + ',"service":"' + rec.service + ...`).
3. **Hardware `SHA-NI` Intrinsics ([`Sha256State::compress_block_sha_ni`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L3711-L3807)):**
   - Uses hardware Intel/AMD `SHA-NI` instructions (`_mm_sha256rnds2_epu32`, `_mm_sha256msg1_epu32`, `_mm_sha256msg2_epu32`) for streaming 64-byte FIPS-180-4 SHA-256 and RFC-2104 HMAC-SHA256 blocks with zero intermediate heap allocations.

### Runnable Tier-2 Typed Loop Example

Create `tier2_loop.ts`:
```typescript
let acc: number = 0;
for (let i: number = 0; i < 2000; i++) {
    acc += i * 3;
}
console.log(acc);
```

Inspect the promoted `TIER2_MONO_LOOP` instruction and execute it:
```bash
slice inspect tier2_loop.ts
slice run tier2_loop.ts --metrics
```
Output:
```text
=========================================================================
  SLICESNP BYTECODE DISASSEMBLY: tier2_loop.ts (294 bytes)
=========================================================================
  0000  CONST_NUM       %acc <- 0
  0001  MONO_FOR_T2     %acc += (%i in 0..2000) * 3
  0002  AST_EXEC        ExprStmt(Call { callee: Member { object: Ident("console"), property: "log" }, args: [Ident("acc")] })
=========================================================================
5997000
[Slice Telemetry] Tier: Tier2PeakNative | Instantiate: 16.40 us | Exec: 1.50 us | Total: 17.90 us | Memory: 4096 B | Stat Syscalls: 0
```

