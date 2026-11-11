# Slice `boa_engine` Patch Specification (`SLCB\x01` Precompiled `CodeBlock`)

## 1. Purpose

Upstream `boa_engine v0.22.0` compiles JavaScript through four phases inside [`src/script.rs`](./src/script.rs):
1. `boa_lexer` + `boa_parser` (`Script::parse`)
2. `boa_ast::Script::analyze_scope`
3. `ByteCompiler::compile_statement_list` (`Script::codeblock`)
4. `Context::run` (`Script::evaluate`)

While `pub struct CodeBlock` is public in `boa_engine::vm`, its 16 internal fields (`flags`, `length`, ` parameter_length`, `register_count`, `strict`, `this_mode`, `params`, `bytecode`, `constants`, `bindings`, `handler_table`, `source_info`, `ic`, `global_lexs`, `global_fns`, `global_vars`) are `pub(crate)`.
Critically, by the time `ByteCompiler` finishes constructing a `CodeBlock`, **100% of `boa_interner::Sym` symbol IDs have already been resolved into `JsString` (Latin1 or UTF-16) values** inside `Constant::String(JsString)`, `BindingLocator`, and `InlineCache`.

This minimal `[patch.crates-io]` overlay exposes two public methods on `boa_engine::Script` in [`src/script.rs`](./src/script.rs) (~250 lines added at the bottom of the file, zero changes to VM execution or GC semantics):
- `pub fn compile_to_slicebc(&self, context: &mut Context) -> JsResult<Vec<u8>>`
- `pub fn from_slicebc_bytes(bytes: &[u8], context: &mut Context) -> Option<Self>`

By serializing the compiled `Gc<CodeBlock>` tree and global `Scope` bindings into `.cas` archives (`SLICEBC\x01`) at `slice pack` time and reconstructing `Gc<CodeBlock>` directly at `slice run` time, Slice bypasses `boa_lexer`, `boa_parser`, `Scope::analyze_scope`, and `ByteCompiler` on cold start — reducing cold VM execution from **`5.6–7.0 ms` down to `0.60–0.77 ms` (`9.0x–9.5x` faster in `--release`)**.

---

## 2. Modified Files

Thirteen files in `vendor/boa_engine` differ from upstream `boa_engine v0.22.0`:
- [`vendor/boa_engine/src/script.rs`](./src/script.rs) & [`vendor/boa_engine/src/slice_precompile.rs`](./src/slice_precompile.rs):
  - `Script::compile_to_slicebc(&self, context: &mut Context) -> JsResult<Vec<u8>>`
  - `Script::from_slicebc_bytes(bytes: &[u8], context: &mut Context) -> Option<Self>`
  - Internal recursive helpers `encode_codeblock` / `decode_codeblock` (including bytecode-to-line `SourceMap` entries and deferred `0x11` function source spans) and `write_js_string` / `read_js_string`.
- [`vendor/boa_engine/src/builtins/json/mod.rs`](./src/builtins/json/mod.rs) & [`vendor/boa_engine/src/object/property_map.rs`](./src/object/property_map.rs):
  - `Json::parse_ecma404` & `PropertyMap::insert_data_property`: Direct iterative ECMA-404 state-machine parser (`MAX_DEPTH = 2048` on heap-allocated `Vec<ContainerFrame>`, `O(1)` C/Rust call stack) over `JsStrVariant::Latin1` and `JsStrVariant::Utf16` with `fast-float2` exact IEEE-754 float parsing (`±Infinity` on overflow), lone UTF-16 surrogate preservation, zero-copy Latin1/UTF-16 string slicing, Latin1 escaped-string fast path, direct `PropertyMap::insert_data_property` slot insertion, and optional `WITH_SOURCE` source-span tracking for `reviver` (`context.source`).
  - `Json::try_fast_stringify`: Direct single-buffer `Vec<u8>` serializer fast-path in `JSON.stringify` when no `replacer` or `space` argument is supplied, traversing ordinary `JsObject`s (via `Shape::with_property_keys` + direct storage slots) and dense `Array`s (`DenseI32`, `DenseF64`, `DenseElement`) with `ryu_js` number formatting, inline `\uXXXX` escaping for lone UTF-16 surrogates, and automatic fallback for `toJSON`, getters, or exotic objects.
- [`vendor/boa_engine/src/object/shape/mod.rs`](./src/object/shape/mod.rs), [`vendor/boa_engine/src/object/shape/shared_shape/mod.rs`](./src/object/shape/shared_shape/mod.rs), & [`vendor/boa_engine/src/object/shape/unique_shape.rs`](./src/object/shape/unique_shape.rs):
  - `Shape::with_property_keys`: Zero-allocation slice/iterator callback over ordinary object property keys for `Json::try_fast_stringify`.
- [`vendor/boa_engine/src/vm/inline_cache/mod.rs`](./src/vm/inline_cache/mod.rs):
  - `InlineCache::mono_shape_addr`, `InlineCache::mono_slot`, and `InlineCache::get_fast(&self, shape: &Shape) -> Option<(Option<Shape>, Slot)>`: Zero-borrow monomorphic fast-path for own-property `Shape -> Slot` hits without locking `GcRefCell` or upgrading `WeakGc` pointers, with automatic polymorphic (`PIC_CAPACITY = 4`) and megamorphic fallback.
- [`vendor/boa_engine/src/vm/opcode/get/property.rs`](./src/vm/opcode/get/property.rs) & [`vendor/boa_engine/src/vm/opcode/set/property.rs`](./src/vm/opcode/set/property.rs):
  - Wired `get_by_name` and `set_by_name` to call `ic.get_fast(object_borrowed.shape())`.
- [`vendor/boa_engine/src/vm/runtime_limits.rs`](./src/vm/runtime_limits.rs), [`vendor/boa_engine/src/vm/opcode/iteration/loop_ops.rs`](./src/vm/opcode/iteration/loop_ops.rs), [`vendor/boa_engine/src/vm/call_frame/mod.rs`](./src/vm/call_frame/mod.rs), & [`vendor/boa_engine/src/bytecompiler/statement/loop.rs`](./src/bytecompiler/statement/loop.rs):
  - `RuntimeLimits::loop_iterations_executed()` and `RuntimeLimits::check_and_increment_loop_iteration()`: Inline cumulative loop iteration counter + limit check on `IncrementLoopIteration` (removing `CallFrame::loop_iteration_count` and avoiding `frame_mut()` heap dereferencing on every loop iteration), plus emitting `IncrementLoopIteration` after the loop condition check in `while`, `for..in`, and `for..of` so Tier-1 Boa loop executions report and enforce exact `fuelUsed` telemetry.

---

## 3. `SLCB\x01` Binary Wire Format

```text
+-----------------------------------------------------------------------+
| Magic: b"SLCB\x01" (5 bytes)                                          |
+-----------------------------------------------------------------------+
| Global Scope Bindings:                                                |
|   binding_count: u32                                                  |
|   repeat binding_count:                                               |
|     name: JsString (tag u8: 0 = Latin1, 1 = UTF-16 + len u32 + bytes) |
|     flags: u8 (bit 0 = mutable, bit 1 = lex, bit 2 = strict)          |
+-----------------------------------------------------------------------+
| Root CodeBlock (recursive):                                           |
|   flags: u16 (CodeBlockFlags::bits())                                 |
|   length: u32, parameter_length: u32, register_count: u32             |
|   this_mode: u8 (0 = Global, 1 = Lexical, 2 = Strict)                 |
|   bytecode_len: u32 + raw Bytecode bytes                              |
|   constants_count: u32 + tagged Constants:                            |
|     0 = Function(nested CodeBlock)                                    |
|     1 = Float(f64)                                                    |
|     2 = Int(i32)                                                      |
|     3 = BigInt(UTF-8 string)                                          |
|     4 = String(JsString Latin1/UTF-16)                                |
|     5 = Scope(index, outer_exists, is_function_var, bindings)         |
|   bindings_count: u32 + BindingLocators (name, env_idx, bind_idx, k)  |
|   handlers_count: u32 + Exception Handlers (start, end, env_count)    |
|   ic_count: u32 + InlineCache slot names (JsString)                   |
|   global_lexs: u32 + [JsString]                                       |
|   global_fns:  u32 + [JsString]                                       |
|   global_vars: u32 + [JsString]                                       |
+-----------------------------------------------------------------------+
```

---

## 4. Upgrade Checklist When Bumping `boa_engine`

1. Extract the new upstream `boa_engine` crate into `vendor/boa_engine`.
2. Copy the `impl Script { pub fn compile_to_slicebc ... pub fn from_slicebc_bytes ... }` block at the bottom of [`vendor/boa_engine/src/script.rs`](./src/script.rs) into the new `script.rs`.
3. If `CodeBlock` or `Constant` added or reordered fields in the new `boa_engine` release, update `encode_codeblock` / `decode_codeblock` and bump `b"SLCB\x01"` to `b"SLCB\x02"`.
4. Run `cargo test --workspace` to verify all `boacas` roundtrip and edge-case tests pass.

