# Chapter 03: Zero-Stat CAS Archive, Oxc Bundler & Standalone Binaries (`slice-cas`)

Implemented in [`crates/slice-cas/src/lib.rs`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs), `slice-cas` eliminates the #1 cold-start bottleneck in Node.js, Deno, and Bun: **runtime filesystem metadata storms (`stat`, `lstat`, `readdir`, `open`) and cold source parsing**.

---

## 1. Build-Time AOT TypeScript Compilation & Bundling Pipeline

When you run `slice pack <input> <output.cas>` or `slice compile <input> -o <binary>`, `slice-cas` executes a 5-stage Ahead-of-Time (AOT) pipeline:

1. **Entrypoint Discovery ([`resolve_project_entrypoint`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs#L351-L413)):**
   - Accepts a single `.ts`/`.js`/`.mjs` file or an application folder.
   - For folders, inspects `slice.toml` (`entrypoint`), `package.json` (`exports["."]` -> `module` -> `main`), or falls back to `src/index.ts`, `index.ts`, `src/index.js`, or `index.js`.
2. **Oxc `v0.152.0` TypeScript Transformation & Comment Stripping ([`strip_typescript_raw`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs#L232-L289)):**
   - Parses TypeScript ASTs in a bump-allocated arena (`oxc::allocator::Allocator`), extracts static type hints (`analyze_typescript_semantics`), strips all TypeScript types via `oxc::transformer::Transformer`, and emits compact JavaScript with all JSDoc/license comments removed (`CommentOptions::disabled()`).
3. **Transitive ESM + CommonJS + JSON Graph Bundling ([`bundle_entrypoint_with_deps`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs#L1058-L1245)):**
   - Recursively resolves relative imports (`./pricing_engine.js` -> `./pricing_engine.ts`), `node_modules/` packages (`package.json` `exports`/`main`), and `.json` files.
   - Assigns dense integer IDs (`0..N-1`) to every dependency module so `__slice_require(id)` performs O(1) array indexing instead of string hash-map lookups.
4. **Static Barrel Tree-Shaking ([`transform_module_to_cjs`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs#L566-L1053)):**
   - Detects barrel index files (such as `semver/index.js`) and prunes 30+ unreferenced submodule `require()` statements (`pruned_req_ids`) before they are even read from disk, reducing `.cas` archive size by up to `50.4%`.
5. **BLAKE3-256 Content Addressing & 64-Byte Cache-Line Alignment ([`CasBuilder::write_to_file`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs#L1408-L1498)):**
   - Hashes every module blob with SIMD-accelerated **BLAKE3-256** (`blake3::hash`) and aligns every source and bytecode slice to a strict **64-byte boundary** (`CACHE_LINE_ALIGN = 64`).

---

## 2. Binary Layout of `SLICE_CAS` v1 (`.cas`)

```text
+-----------------------------------------------------------------------+
| Offset 0..8   : Magic Bytes b"SLICECAS"                               |
| Offset 8..12  : Version (u32 LE = 1)                                  |
| Offset 12..16 : Flags (u32 LE = 0)                                    |
| Offset 16..24 : JSON Radix Header Length (u64 LE)                     |
| Offset 24..32 : 64-Byte Aligned Data Payload Start Offset (u64 LE)    |
| Offset 32..N  : CasIndexHeader (Entrypoint + Specifier -> BLAKE3 Map) |
| [Zero Padding to 64-Byte CPU Cache-Line Boundary]                     |
| Data Payload  : [64B-Aligned Source Blob] [64B-Aligned Bytecode Blob] |
+-----------------------------------------------------------------------+
```

At runtime, [`MappedCasArchive::open`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs#L1504-L1536) maps the `.cas` file into virtual memory via a single `mmap(MAP_SHARED)` call (`memmap2::MmapOptions::map`). Module lookups (`resolve_specifier` and `get_bytecode_slice`) return zero-copy `&[u8]` slices directly into the OS page cache with **0 `stat()` or `read()` syscalls**.

---

## 3. Standalone Zero-Dependency Executables (`slice compile` & `SLICEEXE`)

`slice compile` embeds a `.cas` archive directly into a copy of the `slice` binary followed by a 24-byte `SLICEEXE` trailer:

```text
+-----------------------------------------------------------------------+
| [Native Slice Runtime Executable Binary (.exe / ELF / Mach-O)]        |
| [Embedded 64-Byte Aligned SLICE_CAS Archive]                          |
| [u64 LE: cas_start_offset] (8 bytes)                                  |
| [u64 LE: cas_byte_length]  (8 bytes)                                  |
| [Magic Bytes: b"SLICEEXE"] (8 bytes)                                  |
+-----------------------------------------------------------------------+
```

When the compiled binary starts, [`main()`](file:///p:/opensource/slice/crates/slice-cli/src/main.rs#L48-L52) calls [`MappedCasArchive::open_embedded_self()`](file:///p:/opensource/slice/crates/slice-cas/src/lib.rs#L1538-L1583), checks the final 8 bytes of `std::env::current_exe()` for `b"SLICEEXE"`, memory-maps its own executable file, and executes the embedded bytecode in sub-microsecond time without extracting any temporary files to disk.

---

## 4. How-To: Pack, Inspect, and Compile a Multi-File TypeScript Application

```bash
# 1. Pack a multi-file TypeScript + SHA-256 folder application into a .cas archive
slice pack ./docker-tests/apps/01-fintech-checkout ./fintech.cas

# 2. Inspect the BLAKE3 Radix header and 64-byte aligned offsets
slice inspect ./fintech.cas

# 3. Run the memory-mapped .cas archive with telemetry
slice run ./fintech.cas --warm --metrics

# 4. Compile into a standalone zero-dependency native executable
slice compile ./docker-tests/apps/01-fintech-checkout -o ./fintech_app
./fintech_app --warm --metrics
```

