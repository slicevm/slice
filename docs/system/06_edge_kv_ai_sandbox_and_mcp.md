# Serverless Edge KV, AI Agent "Code Mode" Sandbox, Time-Travel Testing & MCP Server

This document covers Slice's 16-way sharded in-memory Edge KV (`Slice.kv`), zero-downtime multi-tenant `.cas` hot-swapping (`--tenant-dir`), SplitMix64 deterministic replay (`--deterministic`), AI Agent "Code Mode" (`Slice.evalSandbox`, `< 5 us` `Slice.checkpoint`/`Slice.restore`), the Time-Travel `slice test` runner, and the built-in JSON-RPC 2.0 Model Context Protocol server (`slice mcp`).

---

## 1. Multi-Tenant Serverless & Edge KV Primitives (`Slice.kv`, `--tenant-dir`, `--deterministic`)

- **16-Way Sharded In-Memory Edge KV & Atomic Counter Slab ([`EDGE_KV_SLAB`](../../crates/slice-engine/src/isolate/state_and_kv.rs)):**
  - Backed by `OnceLock<[Mutex<FxHashMap<(String, String), KvEntry>>; 16]>` partitioned by `(tenant_id, key)` using `FxHasher` low 4 bits (`0..15`) for lock-free multi-core scaling across `--workers=N` (`< 1 us/op`; see **[`tests/reports/latest.md`](../../tests/reports/latest.md)**).
  - Supports `Slice.kv.get(key)`, `Slice.kv.set(key, val, ttlMs)`, `Slice.kv.del(key)`, and atomic 64-bit `Slice.kv.incr(key, delta)`. Every write computes its exact signed retained-byte delta (`new_bytes - old_bytes`) atomically inside the shard lock (`edge_kv_set_checked` / `edge_kv_incr_checked`), reclaiming expired keys under memory pressure and adjusting `BOA_KV_MEM_BUDGET` (`try_adjust_isolate_memory_ctx`) while immediately preempting guest loops with uncatchable `SandboxViolation::MemoryCeilingExceeded` if a tenant exceeds its memory ceiling.
- **Multi-Tenant Dynamic `.cas` Router & Zero-Downtime Atomic Hot-Swap ([`resolve_tenant_bytecode_hotswap`](../../crates/slice-cli/src/server/mod.rs)):**
  - When `slice serve --tenant-dir=<dir>` is active, each worker resolves `X-Slice-Tenant: <id>` or URL prefix `/t/<id>/...` to `<dir>/<id>.cas` (falling back to `<dir>/default.cas`).
  - Thread-local `TENANT_CAS_CACHE` tracks `(mtime_ns, file_len, Arc<Vec<u8>>)`. When a `.cas` file is atomically replaced on disk, the worker automatically maps the new `.cas` archive and pre-warms its `__slice_rc` closure with zero dropped TCP connections.
- **Opt-In Deterministic Replay Mode ([`SliceIsolate::set_deterministic_seed`](../../crates/slice-engine/src/isolate/mod.rs)):**
  - When `--deterministic=<seed>` is passed (`None` by default = `0.00 ns` overhead), `Math.random()`, `Date.now()`, `performance.now()`, `crypto.randomUUID()`, `crypto.randomInt()`, and `crypto.getRandomValues()` route through a deterministic **SplitMix64** PRNG (`splitmix64_next_f64` / `sample_unbiased_u64_below`) and a monotonic virtual clock (`1700000000000 + tick_ms`).

---

## 2. AI Agent "Code Mode" Sandbox, `< 5 us` VFS/KV Checkpoints, & Native MCP Server (`slice mcp`)

```
+---------------------------------------------------------------------------------------------------+
|                            AI Agent "Code Mode" & Native MCP Architecture                         |
|                                                                                                   |
|  Claude Code / Cursor / LLM Agent                                                                 |
|         |  JSON-RPC 2.0 over stdio (`slice mcp [--audit-egress=SECRET]`)                          |
|         v                                                                                         |
|  +---------------------------------------------------------------------------------------------+  |
|  | Parent Orchestrator Isolate (`mcp-agent-host` or `slice run tot_agent.ts`)                  |  |
|  |  - `Slice.checkpoint()` -> Snapshots `BOA_VFS_STATE` + Tenant `EDGE_KV_SLAB` in < 5 us      |  |
|  |  - `Slice.evalSandbox(code, opts)` -> Saves Parent TLS -> Spawns Attenuated Child Isolate   |  |
|  |      |                                                                                      |  |
|  |      +---> [Child SliceIsolate]                                                             |  |
|  |              * Capabilities = Parent Caps INTERSECT Child `opts.allowRead` / `allowWrite`   |  |
|  |              * Fuel & Memory = min(Parent Remaining, Child `opts.fuel` / `memoryBytes`)     |  |
|  |              * Egress Guard = Scans stdout & returnValue for raw/base64/base64url secrets   |  |
|  |  - If `!branch.ok` -> `Slice.restore(snapId)` rolls back VFS + KV state in < 5 us!          |  |
|  +---------------------------------------------------------------------------------------------+  |
+---------------------------------------------------------------------------------------------------+
```

1. **Dynamic Sub-Isolate Spawning with Strict Capability Attenuation ([`SliceIsolate::eval_sandbox`](../../crates/slice-engine/src/isolate/kv_and_sandbox.rs)):**
   - An orchestrator script or MCP host can evaluate untrusted AI-generated TypeScript/JavaScript strings via `Slice.evalSandbox(code, opts)`.
   - Child `allowed_read_prefixes`, `allowed_write_prefixes`, and `allowed_net_hosts` are **strictly intersected** with the parent isolate's `CapabilityToken` (a child can never escalate privileges beyond its parent), and child `fuel_consumed` is deducted directly from the parent isolate's remaining fuel budget.
   - Inline source directives (`// @slice-allow-*`, `// @slice-fuel`, `// @slice-memory-kb`) inside `Slice.evalSandbox` and `slice mcp` (`slice_eval_ts`) are ignored for capabilities and limits, and default CLI runs (`trust_directives = false`) enforce monotonic attenuation in [`ResolvedExecutionConfig::apply_inline_directives`](../../crates/slice-cli/src/config/mod.rs) so AI-generated entrypoints cannot self-grant permissions.
   - Re-entrant [`SavedIsolateContextGuard`](../../crates/slice-engine/src/runtime/boa_realm.rs) uses zero-allocation `std::mem::take` swaps for `BOA_ACTIVE_TENANT` and `BOA_ENV_MAP`, `Option<SavedEventLoopSnapshot>` fast-path skipping when no parent async timers/fetches are pending, and a per-subsystem `BOA_DIRTY_SUBSYSTEMS` bitmask (`DIRTY_EVENT_LOOP`, `DIRTY_ALS`, `DIRTY_FETCH_MOCK`, `DIRTY_WEBSOCKET`, `DIRTY_SOCKETS`, `DIRTY_WORKERS`, `DIRTY_WASM`), while [`compile_source_for_sandbox_eval`](../../crates/slice-engine/src/vm/parser_and_compiler.rs) caches compiled bytecode in a 64-entry thread-local `SANDBOX_EVAL_BC_CACHE` (`FxHashMap<u64, Rc<[u8]>>`) so nested sub-isolates execute in microseconds with zero TLS interference.
2. **Sub-5us VFS & KV Checkpointing (`Slice.checkpoint()` & `Slice.restore(id)`):**
   - [`checkpoint_tenant_state`](../../crates/slice-engine/src/isolate/state_and_kv.rs) captures a point-in-time `Arc` CoW snapshot (`VfsSnapshot`) of the thread-local inode-backed `node:fs` VFS & open FD slab (`BOA_VFS_STATE`) and every entry belonging to `tenant_id` across all 16 shards of `EDGE_KV_SLAB`, recording exact `state_bytes` and returning a monotonic `u64` snapshot ID.
   - [`restore_tenant_state_checked`](../../crates/slice-engine/src/isolate/state_and_kv.rs) restores the VFS slab, open FD cursors, the tenant's sharded KV keys, and the signed `BOA_KV_MEM_BUDGET` delta (`snap.state_bytes - pre_restore_bytes`) in `< 5 microseconds` (skipping empty shard hash-table probes when `shard_snap.is_none()`), and [`prewarm_boa_bundle`](../../crates/slice-engine/src/isolate/exec_boa.rs) purges `CHECKPOINT_SLAB` after pre-warming so warm pools start with zero orphaned snapshots.
3. **Time-Travel `slice test` Runner (`slice:test`):**
   - Implemented in [`crates/slice-cli/src/commands/test_runner.rs`](../../crates/slice-cli/src/commands/test_runner.rs) and `slice:test` (`describe`, `test`, `it`, `expect`).
   - Automatically checkpoints and restores VFS + KV state around every `test()` block in `< 5 us`, warps virtual timer deadlines in `0 ns`, and provides first-class OCap security matchers (`.toViolate()`, `.toStayWithinFuel()`, `.toStayWithinMemory()`).
4. **Built-In Model Context Protocol Server (`slice mcp`) & Interactive REPL (`slice repl`):**
   - Implemented in [`crates/slice-cli/src/diagnostics/mcp.rs`](../../crates/slice-cli/src/diagnostics/mcp.rs) and [`crates/slice-cli/src/diagnostics/repl.rs`](../../crates/slice-cli/src/diagnostics/repl.rs), exposing `slice_eval_ts`, `slice_kv_get`, `slice_kv_set`, and `slice_checkpoint` over JSON-RPC 2.0 `stdio`, with monotonic session-wide `--fuel`, `--memory`, and `--max-checkpoints` enforcement across both `slice_eval_ts` and `slice_checkpoint`.
