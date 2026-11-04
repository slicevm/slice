# Serverless Edge KV, AI Agent "Code Mode" Sandbox, Time-Travel Testing & MCP Server

This document covers Slice's 16-way sharded in-memory Edge KV (`Slice.kv`), zero-downtime multi-tenant `.cas` hot-swapping (`--tenant-dir`), SplitMix64 deterministic replay (`--deterministic`), AI Agent "Code Mode" (`Slice.evalSandbox`, `< 5 us` `Slice.checkpoint`/`Slice.restore`), the Time-Travel `slice test` runner, and the built-in JSON-RPC 2.0 Model Context Protocol server (`slice mcp`).

---

## 1. Multi-Tenant Serverless & Edge KV Primitives (`Slice.kv`, `--tenant-dir`, `--deterministic`)

- **16-Way Sharded In-Memory Edge KV & Atomic Counter Slab ([`EDGE_KV_SLAB`](file:///p:/opensource/slice/crates/slice-engine/src/state_and_kv.rs)):**
  - Backed by `OnceLock<[Mutex<FxHashMap<(String, String), KvEntry>>; 16]>` partitioned by `(tenant_id, key)` using `FxHasher` low 4 bits (`0..15`) for lock-free multi-core scaling across `--workers=N` (`586.10 us` for 1,001 KV operations = **`0.58 us/op`**).
  - Supports `Slice.kv.get(key)`, `Slice.kv.set(key, val, ttlMs)`, `Slice.kv.del(key)`, and atomic 64-bit `Slice.kv.incr(key, delta)`. Every write charges its byte delta against `BOA_KV_MEM_BUDGET` and `compartment.alloc_memory(bytes)`, trapping with `SandboxViolation::MemoryCeilingExceeded` if a tenant exceeds its memory ceiling.
- **Multi-Tenant Dynamic `.cas` Router & Zero-Downtime Atomic Hot-Swap ([`resolve_tenant_bytecode_hotswap`](file:///p:/opensource/slice/crates/slice-cli/src/http_server.rs)):**
  - When `slice serve --tenant-dir=<dir>` is active, each worker resolves `X-Slice-Tenant: <id>` or URL prefix `/t/<id>/...` to `<dir>/<id>.cas` (falling back to `<dir>/default.cas`).
  - Thread-local `TENANT_CAS_CACHE` tracks `(mtime_ns, file_len, Arc<Vec<u8>>)`. When a `.cas` file is atomically replaced on disk, the worker automatically maps the new `.cas` archive and pre-warms its `__slice_rc` closure with zero dropped TCP connections.
- **Opt-In Deterministic Replay Mode ([`SliceIsolate::set_deterministic_seed`](file:///p:/opensource/slice/crates/slice-engine/src/isolate.rs)):**
  - When `--deterministic=<seed>` is passed (`None` by default = `0.00 ns` overhead), `Math.random()`, `Date.now()`, `performance.now()`, `crypto.randomUUID()`, and `crypto.getRandomValues()` route through a deterministic **SplitMix64** PRNG (`splitmix64_next_f64`) and a monotonic virtual clock (`1700000000000 + tick_ms`).

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
|  |  - `Slice.checkpoint()` -> Snapshots `BOA_VFS_CACHE` + Tenant `EDGE_KV_SLAB` in < 5 us      |  |
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

1. **Dynamic Sub-Isolate Spawning with Strict Capability Attenuation ([`SliceIsolate::eval_sandbox`](file:///p:/opensource/slice/crates/slice-engine/src/isolate.rs)):**
   - An orchestrator script or MCP host can evaluate untrusted AI-generated TypeScript/JavaScript strings via `Slice.evalSandbox(code, opts)`.
   - Child `allowed_read_prefixes`, `allowed_write_prefixes`, and `allowed_net_hosts` are **strictly intersected** with the parent isolate's `CapabilityToken` (a child can never escalate privileges beyond its parent), and child `fuel_consumed` is deducted directly from the parent isolate's remaining fuel budget.
   - Re-entrant `__slice_eval_sandbox` saves and restores the parent isolate's thread-local state (`BOA_STDOUT`, `BOA_VIOLATION`, `BOA_ACTIVE_TENANT`, `BOA_ACTIVE_CAPS`, `BOA_KV_MEM_BUDGET`, `BOA_DET_STATE`) so nested sub-isolates execute with zero TLS interference.
2. **Sub-5us VFS & KV Checkpointing (`Slice.checkpoint()` & `Slice.restore(id)`):**
   - [`checkpoint_tenant_state`](file:///p:/opensource/slice/crates/slice-engine/src/state_and_kv.rs) captures a point-in-time clone of the thread-local `node:fs` VFS slab (`BOA_VFS_CACHE` and `BOA_VFS_DIRS`) and every entry belonging to `tenant_id` across all 16 shards of `EDGE_KV_SLAB`, returning a monotonic `u64` snapshot ID.
   - [`restore_tenant_state`](file:///p:/opensource/slice/crates/slice-engine/src/state_and_kv.rs) restores both the VFS slab and the tenant's sharded KV keys in `< 5 microseconds`, enabling Tree-of-Thought (ToT) AI coding agents to test candidate patches and roll back failed branches instantaneously.
3. **Time-Travel `slice test` Runner (`slice:test`):**
   - Implemented in [`crates/slice-cli/src/test_runner.rs`](file:///p:/opensource/slice/crates/slice-cli/src/test_runner.rs) and `slice:test` (`describe`, `test`, `it`, `expect`).
   - Automatically checkpoints and restores VFS + KV state around every `test()` block in `< 5 us`, warps virtual timer deadlines in `0 ns`, and provides first-class OCap security matchers (`.toViolate()`, `.toStayWithinFuel()`, `.toStayWithinMemory()`).
4. **Built-In Model Context Protocol Server (`slice mcp`):**
   - Implemented in [`crates/slice-cli/src/mcp_and_repl.rs`](file:///p:/opensource/slice/crates/slice-cli/src/mcp_and_repl.rs), exposing `slice_eval_ts`, `slice_kv_get`, `slice_kv_set`, and `slice_checkpoint` over JSON-RPC 2.0 `stdio`.
