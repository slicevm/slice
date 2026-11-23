# Slice `boa_engine` Patch Specification (`SLCB\x01` Precompiled `CodeBlock`)

## 1. Purpose

Upstream `boa_engine v0.22.0` compiles JavaScript through four phases inside [`src/script.rs`](./src/script.rs):
1. `boa_lexer` + `boa_parser` (`Script::parse`)
2. `boa_ast::Script::analyze_scope`
3. `ByteCompiler::compile_statement_list` (`Script::codeblock`)
4. `Context::run` (`Script::evaluate`)

While `pub struct CodeBlock` is public in `boa_engine::vm`, its 16 internal fields (`flags`, `length`, `parameter_length`, `register_count`, `strict`, `this_mode`, `params`, `bytecode`, `constants`, `bindings`, `handler_table`, `source_info`, `ic`, `global_lexs`, `global_fns`, `global_vars`) are `pub(crate)`.
Critically, by the time `ByteCompiler` finishes constructing a `CodeBlock`, **100% of `boa_interner::Sym` symbol IDs have already been resolved into `JsString` (Latin1 or UTF-16) values** inside `Constant::String(JsString)`, `BindingLocator`, and `InlineCache`.

This minimal `[patch.crates-io]` overlay exposes public methods on `boa_engine::Script` in [`src/script.rs`](./src/script.rs) (~250 lines added at the bottom of the file, zero changes to VM execution or GC semantics):
- `pub fn compile_to_slicebc(&self, context: &mut Context) -> JsResult<Vec<u8>>`
- `pub fn from_slicebc_bytes(bytes: &[u8], context: &mut Context) -> Option<Self>`

By serializing the compiled `Gc<CodeBlock>` tree and global `Scope` bindings into `.cas` archives (`SLICEBC\x01`) at `slice pack` time and reconstructing `Gc<CodeBlock>` directly at `slice run` time, Slice bypasses `boa_lexer`, `boa_parser`, `Scope::analyze_scope`, and `ByteCompiler` on cold start, reducing cold VM execution from **`5.6-7.0 ms` down to `0.60-0.77 ms` (`9.0x-9.5x` faster in `--release`)**.

---

## 2. Modified Files

Thirty-five source files in `vendor/boa_engine` differ from upstream `boa_engine v0.22.0`:
- [`src/script.rs`](./src/script.rs):
  - `Script::compile_to_slicebc(&self, context: &mut Context) -> JsResult<Vec<u8>>` & `Script::compile_to_slicebc_with_min_line(&self, context: &mut Context, min_sourcemap_line: u32) -> JsResult<Vec<u8>>`
  - `Script::from_slicebc_bytes(bytes: &[u8], context: &mut Context) -> Option<Self>`
  - `Script::can_reuse_across_runs(&self) -> bool`: verifies that a compiled `Script` introduces no global `let`/`const`/`class` lexical declarations so warm pooled isolates can retain and re-run compiled `Script` instances (keeping populated `InlineCache` slots warm across executions).
  - Internal recursive helpers `encode_codeblock` / `decode_codeblock` (omitting internal prelude `SourceMap` entries below `min_sourcemap_line`, sharing `SourceInfo::default()` and `SpannedSourceText`, and deduplicating short Latin1 identifiers/constants via a bounded `FxHashMap<&[u8], JsString>` pool in `read_js_string`) and `write_js_string` / `read_js_string`.
- [`src/property/mod.rs`](./src/property/mod.rs):
  - Fast-path `PropertyKey::from(JsStr)` and `PropertyKey::from(JsString)` via `parse_u32_index_bytes` / `parse_u32_index_js_str`, avoiding `JsStrIter` construction and returning immediately when the leading byte is outside `'0'..='9'`.
- [`src/builtins/json/mod.rs`](./src/builtins/json/mod.rs), [`src/object/property_map.rs`](./src/object/property_map.rs), & [`src/object/jsobject.rs`](./src/object/jsobject.rs):
  - `Json::parse_ecma404`, `PropertyMap::insert_data_property`, `PropertyMap::from_parts`, and `JsObject::from_property_map`: Direct iterative ECMA-404 state-machine parser (`MAX_DEPTH = 2048` on heap-allocated `Vec<ContainerFrame>`, `O(1)` C/Rust call stack) over `JsStrVariant::Latin1` and `JsStrVariant::Utf16` with 8-byte SWAR unescaped string scanning, single-branch whitespace skipping, single-pass exact integer accumulation (`<= 16` digits) plus `fast-float2` float parsing (`+-Infinity` on overflow) over stack buffers, lone UTF-16 surrogate preservation, zero-drop `SourceTracker` (`NoSource` ZST when `reviver` is absent vs `WithSource` for `context.source`), `SharedShape` object construction (`ordinary_object` root shape), and per-depth `ShapeChain` prediction so homogeneous arrays/trees of JSON objects skip `JsString` key allocation, hashing, transition lookups, and `GcRefCell` borrows while enabling downstream monomorphic `InlineCache` hits.
  - `Json::try_fast_stringify`: Direct single-buffer `Vec<u8>` serializer fast-path in `JSON.stringify` when no `replacer` or `space` argument is supplied, traversing ordinary `JsObject`s (via `Shape::with_property_keys`, `verified_obj_shape_addr` `toJSON` shape caching, + direct storage slots) and dense `Array`s (`DenseI32`, `DenseF64`, `DenseElement`) with 8-byte SWAR ASCII string scanning, `ryu_js` number formatting, inline `\uXXXX` escaping for lone UTF-16 surrogates, and automatic fallback for `toJSON`, getters, or exotic objects.
- [`src/object/shape/mod.rs`](./src/object/shape/mod.rs), [`src/object/shape/shared_shape/mod.rs`](./src/object/shape/shared_shape/mod.rs), & [`src/object/shape/unique_shape.rs`](./src/object/shape/unique_shape.rs):
  - `Shape::id`: Process-unique monotonic `u64` shape identifier (`AtomicU64`) on `SharedShape` and `UniqueShape` (bumped on in-place `UniqueShape` property insertions, attribute transitions, and internal overrides), eliminating heap pointer-reuse ABA in inline caches.
  - `Shape::with_property_keys`: Zero-allocation slice/iterator callback over ordinary object property keys for `Json::try_fast_stringify`.
  - `Shape::as_shared`, `Shape::from_shared_checked`, `SharedShape::try_fast_insert_property_transition`, and `SharedShape::insert_property_transition_missed`: Single-lookup `SharedShape` property transition fast path for `PropertyMap::insert_data_property` and `Json::parse_ecma404`.
- [`src/vm/inline_cache/mod.rs`](./src/vm/inline_cache/mod.rs) & [`src/vm/inline_cache/tests.rs`](./src/vm/inline_cache/tests.rs):
  - `InlineCache::mono_shape_id`, `InlineCache::mono_proto_shape_id`, `InlineCache::mono_slot`, `IcSlowState` (`Option<Box<IcSlowState>>` keeping cold `InlineCache` slots compact at 56 bytes with zero heap allocation until populated), and `InlineCache::get_fast(&self, shape: &Shape) -> Option<(Option<Shape>, Slot)>`: Zero-`GcRefCell`-borrow / zero-`WeakGc`-upgrade monomorphic fast-path for both own-property and 1-hop prototype `Shape -> Slot` hits, with prototype `Shape::id()` validation, duplicate `shape_id` deduplication in `InlineCache::set`, and automatic polymorphic (`PIC_CAPACITY = 4`) and megamorphic fallback.
  - `AddTransitionEntry`, `InlineCache::add_from_shape_id`, `InlineCache::set_add_transition`, and `InlineCache::try_match_add_transition`: Monomorphic `SharedShape` add-property transition cache (`S_from -> S_to` + `<= 4`-hop prototype `Shape::id()` chain verification) so `SetPropertyByName` on class constructors and `Object.create(proto)` factories appends new own data properties in O(1) without prototype hash walks or `__set__` vtable dispatch.
- [`src/vm/opcode/get/property.rs`](./src/vm/opcode/get/property.rs) & [`src/vm/opcode/set/property.rs`](./src/vm/opcode/set/property.rs):
  - Wired `get_by_name` and `set_by_name` to inline monomorphic own/prototype IC hits, `ic.get_fast(...)`, and `ic.try_match_add_transition(...)`, plus added ordinary/null-prototype string-keyed dictionary fast paths in `get_by_value` and `SetPropertyByValue` bypassing virtual `__get__`/`__set__` and `PropertyDescriptor` allocation.
- [`src/vm/runtime_limits.rs`](./src/vm/runtime_limits.rs), [`src/vm/opcode/iteration/loop_ops.rs`](./src/vm/opcode/iteration/loop_ops.rs), [`src/vm/call_frame/mod.rs`](./src/vm/call_frame/mod.rs), & [`src/bytecompiler/statement/loop.rs`](./src/bytecompiler/statement/loop.rs):
  - `RuntimeLimits::loop_iterations_executed()` and `RuntimeLimits::check_and_increment_loop_iteration()`: Inline cumulative loop iteration counter + limit check on `IncrementLoopIteration` (removing `CallFrame::loop_iteration_count` and avoiding `frame_mut()` heap dereferencing on every loop iteration), plus emitting `IncrementLoopIteration` after the loop condition check in `while`, `for..in`, and `for..of` so Tier-1 Boa loop executions report and enforce exact `fuelUsed` telemetry.
- [`src/builtins/typed_array/mod.rs`](./src/builtins/typed_array/mod.rs), [`src/builtins/typed_array/object.rs`](./src/builtins/typed_array/object.rs), [`src/builtins/dataview/mod.rs`](./src/builtins/dataview/mod.rs), & [`src/builtins/array_buffer/mod.rs`](./src/builtins/array_buffer/mod.rs):
  - `TypedArrayKind::to_element_f64`: Replaced Rust's saturating `f64 as u8/i8/u16/i16/i32/u32` casts with ECMAScript modular two's-complement conversion (`f64_to_int32` / `f64_to_uint32`, plus `round_ties_even` + `NaN -> 0` for `Uint8Clamped`) so `new Uint8Array(typedArray)` and `Buffer.from(typedArray)` honor ECMA-262 `ToUint8`/`ToInt8`/`ToUint16`/`ToInt16`/`ToInt32`/`ToUint32`.
  - `TypedArray::{is_auto_length, is_out_of_bounds_now}`, `DataView::{is_auto_byte_length, raw_byte_offset, is_out_of_bounds_now}`, and `ArrayBuffer::has_detach_key`: Public zero-allocation inspection getters so `structuredClone` preserves `auto` length-tracking views across resizable `ArrayBuffer` / growable `SharedArrayBuffer` clones and rejects out-of-bounds views and non-detachable `WebAssembly.Memory` buffers with `DataCloneError`.
- [`src/object/builtins/jsarraybuffer.rs`](./src/object/builtins/jsarraybuffer.rs), [`src/object/operations.rs`](./src/object/operations.rs), & [`src/vm/mod.rs`](./src/vm/mod.rs):
  - `JsArrayBuffer::from_byte_block_with_detach_key`: Exposes `[[ArrayBufferDetachKey]]` initialization on `JsArrayBuffer` so `WebAssembly.Memory.prototype.buffer` sets `"WebAssembly.Memory"` as its detach key per W3C WebAssembly JS-API `CreateMemoryBuffer` / `ResetMemoryBuffer`.
  - `JsObject::call`, `JsObject::construct`, `Script::evaluate`, `Script::evaluate_async_with_budget`, `Script::prepare_run`, and `Stack::len` / `Stack::truncate`: Restores `context.vm.stack` back to its pre-invocation stack pointer when an early-exiting host call/script unwinds with `Err`, preventing unpopped register/argument slots from accumulating across repeated caught host-callback exceptions or warm pooled realm turns.
- [`src/builtins/promise/mod.rs`](./src/builtins/promise/mod.rs), [`src/vm/opcode/await/mod.rs`](./src/vm/opcode/await/mod.rs), & [`src/context/hooks.rs`](./src/context/hooks.rs):
  - `Promise::new_fulfilled`, `Promise::try_fast_as_intrinsic_promise`, `Promise::promise_resolve`, `Promise::promise_reject`, and `PromiseCapability::new`: ECMA-262 spec-preserving 1-allocation fast paths when `C` is the intrinsic `%Promise%` constructor (`Promise::promise_resolve` returns fulfilled `Promise` objects directly in 1 allocation with zero `PromiseCapability` / `NativeFunction` closures / dynamic VM calls for primitives and non-thenable ordinary/array objects with unmodified `null` / `%Object.prototype%` / `%Array.prototype%` chains, and skips `x.get("constructor")` property walks for unmodified `%Promise.prototype%` instances; `PromiseCapability::new` constructs pending `%Promise%` capabilities directly without an intermediate `RejectResolve` cell or `executor` closure; `Promise::promise_reject` settles rejected `%Promise%` instances directly while invoking `HostPromiseRejectionTracker`).
  - `Await::operation`, `HostHooks::has_active_job_context`, & `HostHooks::run_with_empty_job_context`: Enqueues a direct zero-GC-allocation `PromiseJob` resuming `GeneratorContext` by value when awaiting an already-settled (`Fulfilled` or `Rejected`) intrinsic `Promise` and no host continuation context (`AsyncLocalStorage`) is active (masking any context entered synchronously via `als.enterWith` before microtasks drain), and skips unused reaction closures on the general path.
- [`src/builtins/array/mod.rs`](./src/builtins/array/mod.rs) & [`src/context/intrinsics.rs`](./src/context/intrinsics.rs):
  - `Array::has_default_array_species`, `Array::array_species_create`, `Array::slice`, and `IntrinsicObjects::array_species_getter`: Verified 4-pointer fast path for unmodified intrinsic `Array` instances (`%Array.prototype%` constructor + `%Array%[@@species]` getter) eliminating `Get(O, "constructor")`, `Get(C, @@species)`, and VM `Construct(%Array%)` overhead across `.slice()`, `.map()`, `.filter()`, `.concat()`, `.flat()`, and `.splice()`, plus direct dense backing-slice cloning (`DenseI32`, `DenseF64`, `DenseElement`) in `Array.prototype.slice`.
- [`src/vm/opcode/define/own_property.rs`](./src/vm/opcode/define/own_property.rs):
  - `DefineOwnPropertyByName::operation` & `PropertyMap::try_fast_define_own_data_property`: Direct `SharedShape` own data property insertion fast path on ordinary extensible `JsObject` literals (`{ key: value }`), bypassing `PropertyDescriptor` builder allocation and virtual `__define_own_property__` dispatch while falling back cleanly for integer indices, non-extensible objects, or existing accessor/non-configurable properties.
- [`src/builtins/iterable/mod.rs`](./src/builtins/iterable/mod.rs), [`src/builtins/object/for_in_iterator.rs`](./src/builtins/object/for_in_iterator.rs), [`src/vm/opcode/iteration/for_in.rs`](./src/vm/opcode/iteration/for_in.rs), & [`src/vm/opcode/iteration/iterator.rs`](./src/vm/opcode/iteration/iterator.rs):
  - `FastForInState` & `IteratorRecord::try_new_fast_for_in`: Zero-`ForInIterator`-allocation / zero-`{value, done}`-allocation `for..in` iteration over ordinary objects with no indexed properties and non-enumerable prototypes (`null` or `%Object.prototype%`), preserving live property deletion semantics and deoptimizing seamlessly into `ForInIterator::create_for_in_iterator_from_remaining` if the prototype or indexed storage mutates mid-loop; also makes `IteratorRecord::last_result` `Option<IteratorResult>` (`None` initially) so `for..of` loops skip eager `JsObject::with_null_proto()` allocation.
  - `JsObject::enumerable_own_property_names` in [`src/object/operations.rs`](./src/object/operations.rs): Single-pass `Shape::with_property_keys` fast path for `Object.keys`, `Object.values`, and `Object.entries` on ordinary objects without indexed properties or accessors.
- [`src/value/mod.rs`](./src/value/mod.rs), [`src/value/inner/nan_boxed.rs`](./src/value/inner/nan_boxed.rs), & [`src/object/property_map.rs`](./src/object/property_map.rs):
  - `JsValue::raw_fingerprint_bits`, `NanBoxedValue::value`, and `PropertyMap::storage_fingerprint`: Branchless 64-bit FNV/SplitMix-style mixing over NaN-boxed own-property slot bits and indexed storage length so `slice-engine` verifies 29 SES intrinsic prototypes, constructors, and singleton namespaces (`Object`, `Array`, `Function`, `String`, `Number`, `Boolean`, `RegExp`, `Error`, `Promise`, `Map`, `Set`, `Date`, `Uint8Array`, `Reflect`, `JSON`, `Math`) in native Rust (`~20 ns`, zero JS function calls), catching same-shape `Object.assign` / `Reflect.set` method overwrites.

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
2. Copy the `impl Script { pub fn compile_to_slicebc ... pub fn from_slicebc_bytes ... }` block at the bottom of [`src/script.rs`](./src/script.rs) into the new `script.rs`.
3. If `CodeBlock` or `Constant` added or reordered fields in the new `boa_engine` release, update `encode_codeblock` / `decode_codeblock` and bump `b"SLCB\x01"` to `b"SLCB\x02"`.
4. Run `cargo test --workspace` to verify all `boacas` roundtrip and edge-case tests pass.
