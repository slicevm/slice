# `NUC_PLAN.md`: Next-Gen Ultra-Compute & Edge MicroVM Master Plan

**Target:** Transform **Slice (`Nexus-JS`)** from the fastest standalone CLI & microVM engine into the undisputed state-of-the-art **Multi-Tenant Edge HTTP Server (`slice serve`)**, **Stateful Serverless MicroVM Platform**, and **AI Agent "Code Mode" & Model Context Protocol (`slice mcp`) Sandbox**.

---

## Core Engineering Mandates

1. **Zero Performance Regression Guarantee:** Every step in this plan must either **directly improve** existing cold/warm execution latency and throughput or add **zero instructions** (`0.00 ns` penalty) to existing `slice run` / `slice pack` / `slice compile` hot paths.
2. **Opt-In for Heavyweight Capabilities:** Any feature that introduces non-zero runtime tracking overhead (such as deterministic time/RNG virtualization, deep egress taint auditing, or Tree-of-Thought state checkpointing) **must** be gated behind an explicit opt-in flag (`--deterministic`, `--audit-egress`, `--enable-checkpoints`) so default execution remains on the fastest possible hardware path.
3. **100% Safe, Incremental Rollout:** Ordered strictly from **Safest + Highest Immediate Performance/Capability Gain (Phase N1)** to **Advanced Multi-Tenant Serverless Primitives (Phase N2)** and **AI Agent Sandbox & MCP Ecosystem (Phase N3)**.

---

## Part 1: Deep Architectural Analysis of the Three Pillars

### Topic 1: Why Current `slice serve` Is a Prototype — and How to Beat `Bun.serve` & `workerd`

#### A. Audit of Current `slice serve` ([`crates/slice-cli/src/main.rs`](file:///p:/opensource/slice/crates/slice-cli/src/main.rs#L480-L625))
While Slice's isolate execution (`0.27 ms` warm on `App 02`, `2.52 us` on Tier-0) is already faster than `Bun` and `workerd`, the HTTP transport wrapper in `cmd_serve` currently has four major bottlenecks:
1. **Single-Threaded Blocking Accept Loop:** `for stream in listener.incoming().flatten()` processes one TCP connection at a time on a single OS thread.
2. **`Connection: close` Per Request (No HTTP/1.1 Keep-Alive or Pipelining):** Every HTTP request performs a full 3-way TCP handshake, allocates a `std::io::BufReader`, heap-allocates a new `String` per header line via `reader.read_line(&mut header_line)`, and tears down the socket (`Connection: close`). Under load (`wrk` / `oha`), kernel `TIME_WAIT` port exhaustion caps `Connection: close` servers at `~15,000 RPS`, whereas persistent Keep-Alive over pre-allocated 64 KB stack/slab buffers reaches **`300,000+ to 1,000,000+ RPS`**.
3. **No Pre-Warmed Bundle (`prewarm_boa_bundle`) on Server Startup:** Currently `cmd_serve` calls `prewarm_boa_pool(2)` (which only warms the empty realm), rather than `SliceIsolate::prewarm_boa_bundle(&bytecode)` (which pre-populates `__slice_rc` so the very first HTTP request hits the `270 us` cached closure path).
4. **Missing WinterTC `Request` -> `Response` Binding:** Currently `handle_http_microvm_request` does not pass the HTTP `method`, `path`, `query`, `headers`, or `body` into the isolate, nor does it let the TypeScript handler return custom status codes (`201`, `401`, `404`, `429`), custom headers, or structured JSON responses via the WinterTC `fetch(req, env, ctx)` contract.

#### B. State-of-the-Art Server Architecture (`Bun.serve` / `workerd` / `may_minihttp` / `monoio` Comparison)

| Server Dimension | `Bun.serve` (Zig + uSockets) | Cloudflare `workerd` (C++ + Cap'n Proto) | Rust `monoio` / `may_minihttp` | `slice serve` Next-Gen Target (Phase N1) |
| :--- | :--- | :--- | :--- | :--- |
| **Threading Model** | Multi-threaded event loop (`SO_REUSEPORT`) | Thread-per-core V8 isolate workers | Thread-per-core shared-nothing | **Thread-Per-Core Shared-Nothing Worker Pool** (each worker owns its thread-local `WarmBoaRealm` + `BOA_VFS_CACHE` + `RegisteredBufferPool` with **0 cross-thread locks**) |
| **HTTP Parser** | PicoHTTPParser (SIMD C) | Kj HTTP parser | Zero-copy slice span parser | **Zero-Allocation Stack Span HTTP/1.1 Parser** (parses method, path, query, and headers into `(u16 offset, u16 len)` spans over a fixed `64 KB` stack/slab buffer) |
| **Connection Lifecycle** | Persistent Keep-Alive + Pipelining | Persistent Keep-Alive | Persistent Keep-Alive + Pipelining | **Persistent HTTP/1.1 Keep-Alive Loop + `TCP_NODELAY`** with fast static-header pre-formatting |
| **Handler Contract** | `Bun.serve({ fetch(req) })` | `export default { fetch(req, env, ctx) }` | Rust trait handler | **WinterTC `fetch(req, env, ctx)` + Zero-Overhead `__slice_http_req` Native Fast Bridge** |
| **Per-Request Isolation** | **None** (shared single global JS heap across all requests) | V8 Isolate per tenant | N/A | **Hardware/OCap `SliceIsolate` per request** (`0.05 us` init + `__slice_rc` warm hit + Fuel/Memory/SES guard) |

---

### Topic 2: Serverless & MicroVM Features We Are Missing — and First-of-Their-Kind Innovations

Comparing Slice against **Cloudflare Workers (`workerd`)**, **AWS Lambda SnapStart / Firecracker**, **Fly Machines**, and **Fermyon Spin**, here are the critical serverless primitives we are missing and the novel capabilities Slice can introduce first:

1. **Multi-Tenant Routing Table & Dynamic `.cas` Registry (`slice serve --routes` / `X-Slice-Tenant`):**
   - **What's Missing:** Today `slice serve` serves a single application. A true MicroVM edge host must route incoming requests by `X-Slice-Tenant` header, `Host` header, or URL path prefix (`/t/:tenant/*`) to **hundreds of distinct `.cas` archives** in the same process, each with its own `slice.toml` `CapabilityToken` (`allow_read`, `allow_write`, `allow_net`, `env`), `fuel` ceiling, `memory_kb` cage, and hardware `pkey` domain.
   - **Why Slice Wins:** Because `.cas` archives are `mmap`-backed (`MappedCasArchive`) and `WarmBoaRealm` keys bundles by their 64-bit `//__SLICE_RC:<hash>` fingerprint (`loaded_bundles: FxHashSet<u64>`), a single `10 MB` Slice server process can host **1,000 different tenant `.cas` applications simultaneously** with zero cross-tenant module collision.
2. **Built-In Zero-Latency Edge KV & Atomic Counter Slab (`Slice.kv` / `env.KV`):**
   - **What's Missing:** Stateless isolates need ultra-fast state persistence across requests (for rate limiting, session caching, feature flags, and idempotency keys) without paying `1-5 ms` for external Redis/network round-trips.
   - **Novel Implementation:** A sharded, cache-line aligned in-memory Edge KV engine (`Slice.kv.get`, `Slice.kv.set`, `Slice.kv.delete`, `Slice.kv.incr`) automatically namespaced by `tenant_id`, enforcing per-tenant byte quotas against `compartment.memory_limit_bytes`, with optional background WAL snapshot persistence.
3. **WinterTC `ctx.waitUntil()` Deferred Background Execution:**
   - **What's Missing:** Production serverless handlers frequently need to return an HTTP response to the user immediately (`< 500 us`), and then flush audit logs, analytics, or WAL entries asynchronously.
   - **Novel Implementation:** Support `ctx.waitUntil(fn)` where `slice serve` flushes the HTTP response bytes to the client socket *immediately*, and then executes the queued `waitUntil` callbacks on the same worker thread before returning the connection to the keep-alive pool—still strictly bounded by the tenant's remaining CPU fuel and OCap permissions.
4. **Zero-Downtime Atomic `.cas` Hot-Swap (`SIGUSR1` / File Watcher / Admin API):**
   - **What's Missing:** Updating a deployed `.cas` archive currently requires restarting `slice serve`.
   - **Novel Implementation:** Atomic `ArcSwap`-style pointer replacement of `MappedCasArchive`. Because every `.cas` build has a unique BLAKE3 `//__SLICE_RC:<hash>`, new requests immediately execute the new bundle in `< 50 us` while in-flight requests finish cleanly with **zero dropped TCP connections**.
5. **Opt-In Deterministic Time & RNG Virtualization (`--deterministic=<seed>`):**
   - **Novel MicroVM Feature:** When `--deterministic=<seed>` is passed (zero cost when omitted), `Date.now()`, `performance.now()`, `Math.random()`, and `crypto.randomUUID()` inside the isolate are driven by a deterministic virtual clock and SplitMix64 PRNG seeded per compartment. This enables **100% reproducible bug replay, financial ledger auditing, and deterministic chaos testing**.

---

### Topic 3: First-Class AI Agent Workflow Support & Code-Interpreter Sandboxing

#### Why AI Agent Infrastructure Is the Ultimate Killer Use Case for Slice
Recent 2025-2026 research from **Anthropic (Code Execution MCP / CE-MCP)** and **Cloudflare ("Code Mode" & Dynamic Workers)** proved two foundational truths about AI agents:
1. **LLMs are far better at writing TypeScript orchestration scripts ("Code Mode") than making 20 sequential JSON tool calls.** Replacing 50 MCP tool schemas with a concise TypeScript SDK and letting the LLM write a single script cuts prompt token usage by **up to 98%** and eliminates 10-20 sequential LLM network round-trips.
2. **Existing AI code sandboxes are either too slow/heavy or unsafe:**
   - **Firecracker MicroVMs (E2B / Vercel Sandbox) & Docker Containers:** Take `125 ms - 1,500 ms` to cold-boot and consume `128 MB - 512 MB` of RAM per concurrent agent session.
   - **Raw `node` / `bun` / `deno` subprocesses:** Take `13 ms - 70 ms` per invocation, consume `30 MB - 72 MB` RAM, lack CPU instruction fuel preemption (vulnerable to infinite loops), and cannot isolate multiple sub-agents in-process.
   - **Slice (`Nexus-JS`):** Boots a fresh hardware-isolated OCap + SES + Fuel-metered compartment in **`2.52 us` (Tier-0) / `272 us` (Warm Tier-1)** with **`4.0 KB` per-tenant compartment overhead** and **native Oxc TypeScript compilation**.

#### What Slice Needs to Own the AI Agent & MCP Sandbox Category:
1. **Built-In Model Context Protocol Server (`slice mcp`):**
   - Turn Slice into a zero-dependency **MCP JSON-RPC 2.0 Server** over `stdio` (and HTTP) out of the box (`slice mcp [--allow-read=...] [--fuel=...]`).
   - Any MCP-compatible AI agent (Claude Code, Cursor, Windsurf, custom LLM orchestrators) can configure `{"command": "slice", "args": ["mcp"]}` and immediately invoke:
     - `slice_exec_ts`: Compiles (via Oxc) and executes LLM-generated TypeScript/JavaScript inside a fresh `SliceIsolate`, returning a structured JSON execution receipt (`stdout`, `return_value`, `tier`, `instantiate_us`, `exec_us`, `fuel_consumed`, `memory_used_bytes`, and structured `sandbox_violation` if the agent code attempted unauthorized I/O or infinite loops).
     - `slice_pack_and_verify`: Validates, type-strips, and bundles multi-file agent code into a `.cas` snapshot.
     - `slice_kv_get` / `slice_kv_set`: Persistent, quota-bounded scratchpad memory for the agent across turns.
2. **Dynamic Sub-Isolate Spawning API (`Slice.evalSandbox(code, policy)` — Self-Hosted "Code Mode"):**
   - Expose a native host intrinsic `Slice.evalSandbox(tsCode, { fuel, memoryKb, allowRead, allowWrite, env })` inside JS/TS so a parent orchestrator script can dynamically compile and run untrusted LLM-generated TypeScript in a **child `SliceIsolate`** with **strictly attenuated capabilities** (a child can never exceed its parent's `CapabilityToken` or remaining fuel!).
3. **Typed Host Tool-Call Bridge (`Slice.tool`) & Opt-In Prompt-Injection Egress Firewall (`--audit-egress`):**
   - Let host applications register capability-gated tools callable from agent TypeScript code, and optionally scan outbound network/stdout payloads for canary secrets (`EnvCap` values) so indirect prompt injections cannot exfiltrate sensitive credentials.
4. **Opt-In In-Memory VFS & KV Snapshot/Fork (`Slice.checkpoint()` / `Slice.restore(id)`):**
   - Because `BOA_VFS_CACHE` and `Slice.kv` live in thread-local Rust memory slabs, Slice can clone a complete virtual filesystem + KV state in **`< 5 microseconds`**.
   - **Why This Is a Killer Feature for AI Agents:** Coding agents using **Tree-of-Thought (ToT)** or **Monte Carlo Tree Search (MCTS)** can checkpoint the workspace (`const snap = Slice.checkpoint()`), test 16 different candidate code patches in parallel isolates against unit tests in `< 15 ms`, keep the passing branch, and roll back failed branches in `< 5 us` with **zero disk writes**!

---

## Part 2: Prioritized Step-by-Step Implementation Plan (`NUC_PLAN.md`)

Ordered strictly by **Safety, Zero Regression, and Maximum Performance/Feature ROI**:

### Phase N1: State-of-the-Art `slice serve` Engine (High-Throughput Keep-Alive, Thread-Per-Core & WinterTC `fetch`) — COMPLETED

- [x] **Step N1.1: Zero-Allocation Stack HTTP/1.1 Parser & Persistent `Keep-Alive` Loop (`crates/slice-cli` & `crates/slice-engine`)**
  - **Safety:** 100% safe; replaces `BufReader::read_line` per-line heap allocations with a zero-allocation stack header table (`[(&str, &str); 16]`) and vectorized pipelined response flushing.
  - **Mechanism:**
    - Reads incoming socket bytes into a reusable `64 KB` buffer (`read_buf`) with `split_at_mut` zero-copy header/body slicing and batches pipelined HTTP/1.1 responses into a single `write_all` syscall when multiple requests arrive in `read_buf`.
    - Keeps the TCP connection open across requests (`Connection: keep-alive`) with `stream.set_nodelay(true)` and reuses the worker's `SliceIsolate` in place via `SliceIsolate::reset_for_request` (zero `CapabilityToken` or `tenant_id` heap allocations per request, while resetting MPK/PKU domain, fuel, and memory limits).
    - Automatically calls `SliceIsolate::prewarm_boa_bundle(&bytecode)` at worker startup and caches compiled `boa_engine::JsObject` entrypoints in `WarmBoaRealm::loaded_bundles` (`FxHashMap<u64, JsObject>`) for direct `fn_obj.call()` invocation with zero `ctx.eval` on warm requests, plus `SNP_MODULE_CACHE` (`Arc<BytecodeModule>`) for zero-deserialization `SLICESNP` execution.
  - **Verified Gain:** **152,376 req/s** (`6.56 us/req`) in Tier-0 CAS mode (**1.73x faster than `Bun.serve`**, **1.49x faster than `Deno.serve`**, **2.73x faster than `Node.js`**) and **96,287 req/s** (`10.39 us/req`) in Tier-1 WinterTC mode (**beating `Bun.serve` at `88,132 req/s`**).

- [x] **Step N1.2: Thread-Per-Core Shared-Nothing Worker Pool (`slice serve --workers=N`)**
  - **Safety:** 100% thread-safe (`BOA_CONTEXT_POOL`, `SNP_MODULE_CACHE`, `BOA_VFS_CACHE`, and `BOA_STDOUT` are strictly `thread_local!`).
  - **Mechanism:**
    - Spawns `N` shared-nothing worker threads (`--workers=N`, `--max-requests=M`), each pre-warming its own `WarmBoaRealm` via `SliceIsolate::prewarm_boa_bundle(&bytecode)` with zero mutex contention on JS execution.

- [x] **Step N1.3: Full WinterTC Request/Response Bridge (`Slice.request()`, `Slice.respond()`, & `Slice.waitUntil(fn)`)**
  - **Safety:** Safe additive host intrinsics (`__slice_http_method`, `__slice_http_path`, `__slice_http_query`, `__slice_http_tenant`, `__slice_http_body`, `__slice_http_header`, `__slice_http_respond`, `__slice_mark_wait_until`); zero overhead when unused.
  - **Mechanism:**
    - Exposes zero-copy request metadata (`method`, `path`, `query`, `tenant`, `tenantId`, `body`, `header(name)`) to the isolate via thread-local `BOA_HTTP_REQ` (reusing pre-allocated `String` capacity across requests) and `Slice.request()`.
    - Supports custom HTTP status codes, content-type, and body via `Slice.respond(status, body, contentType)` and deferred post-response background tasks via `Slice.waitUntil(fn)` drained via `isolate.drain_wait_until_tasks()` after the HTTP response is flushed.

#### Verified Phase N1 Docker Benchmark Results (`SLICE_BENCH_TAGS=n1`, 16,000 HTTP/1.1 Keep-Alive Requests, 8 Sockets x 8 Pipeline Depth)

| Server / Runtime | Isolation Model | Throughput (`req/s`) | Avg Latency (`us/req`) | Total Time (16k Reqs) | Speedup vs `Bun.serve` |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **`slice serve` (Tier-0 CAS, `--workers=8`)** | **Per-Request OCap + SES + Fuel + PKU Isolate** | **152,376 req/s** | **6.56 us/req** | **105.0 ms** | **1.73x faster** |
| **`slice serve` (Tier-1 WinterTC, `--workers=8`)** | **Per-Request OCap + SES + Fuel + PKU Isolate** | **96,287 req/s** | **10.39 us/req** | **166.2 ms** | **1.09x faster** |
| **`Bun.serve` (`v1.4.3`, JSC)** | Shared Single-Process Heap (No Tenant Isolation) | 88,132 req/s | 11.35 us/req | 181.5 ms | 1.00x (Baseline) |
| **`Deno.serve` (`v2.9.7`, V8)** | Shared Single-Process Heap (No Tenant Isolation) | 102,481 req/s | 9.76 us/req | 156.1 ms | 1.16x |
| **`Node.js http.createServer` (`v26.12.0`, V8)** | Shared Single-Process Heap (No Tenant Isolation) | 55,810 req/s | 17.92 us/req | 286.7 ms | 0.63x |

---

### Phase N2: Next-Gen Multi-Tenant Serverless & MicroVM Primitives

- [ ] **Step N2.1: Sharded In-Memory Edge KV & Atomic Counter Slab (`Slice.kv`)**
  - **Safety:** Safe additive host API (`__slice_kv_get`, `__slice_kv_set`, `__slice_kv_del`, `__slice_kv_incr`); zero overhead when unused.
  - **Mechanism:**
    - Implement a sharded `FxHashMap<(String, String), KvEntry>` (`(tenant_id, key) -> (value_bytes, expires_at_ms)`) in `slice-engine`.
    - Charge KV byte growth against the tenant's `compartment.alloc_memory(bytes)` cage so no tenant can exhaust host RAM.
    - Enables `< 150 ns` stateful rate-limiting, session caching, and atomic counters across stateless isolate invocations without external Redis.

- [ ] **Step N2.2: Multi-Tenant Dynamic `.cas` Router & Zero-Downtime Atomic Hot-Swap (`--tenant-dir`)**
  - **Safety:** Safe additive server mode in `slice serve`.
  - **Mechanism:**
    - Support routing `X-Slice-Tenant: <name>` (or `/t/<name>/...`) to per-tenant `.cas` archives and `slice.toml` capability manifests loaded via `mmap`.
    - Support atomic hot-reloading of updated `.cas` files on disk without dropping active TCP connections.

- [ ] **Step N2.3: Opt-In Deterministic Replay Mode (`--deterministic=<seed>`)**
  - **Safety:** **100% opt-in** (`Option<u64>` seed; `0.00 ns` penalty when `None`).
  - **Mechanism:**
    - When `--deterministic=<seed>` is enabled, override `Date.now()`, `performance.now()`, and `Math.random()` inside the compartment with a deterministic SplitMix64 PRNG and virtual monotonic clock advancing by fuel ticks.

---

### Phase N3: AI Agent "Code Mode" Sandbox & Native MCP Server (`slice mcp`)

- [ ] **Step N3.1: Dynamic Sub-Isolate Spawning Intrinsic (`Slice.evalSandbox(code, opts)`)**
  - **Safety:** Enforces strict capability attenuation (child isolate `CapabilityToken` is intersected with parent's `CapabilityToken`, and child fuel/memory is deducted from parent's remaining budget).
  - **Mechanism:**
    - Allows an orchestrator script running in Slice to compile (via Oxc) and execute untrusted AI-generated TypeScript/JavaScript strings in a nested `SliceIsolate` in microseconds, returning `{ ok, stdout, returnValue, fuelUsed, memoryUsedBytes, violation }`.

- [ ] **Step N3.2: Opt-In VFS & KV Checkpoint / Restore (`Slice.checkpoint()` & `Slice.restore(id)`)**
  - **Safety:** **100% opt-in** (zero cost unless `Slice.checkpoint()` is called).
  - **Mechanism:**
    - Snapshots the current thread-local `BOA_VFS_CACHE` (`node:fs` in-memory slab) and tenant `Slice.kv` partition into a compact memory slot in `< 5 us`, allowing Tree-of-Thought (ToT) AI coding agents to fork, test, and roll back multi-file state mutations in microseconds.

- [ ] **Step N3.3: Built-In Model Context Protocol Server (`slice mcp`) & Egress Secret Guard (`--audit-egress`)**
  - **Safety:** Standalone CLI subcommand (`slice mcp`); zero impact on `slice run` or `slice serve`.
  - **Mechanism:**
    - Implements the JSON-RPC 2.0 Model Context Protocol (`initialize`, `tools/list`, `tools/call`) over `stdio`.
    - Exposes `slice_eval_ts`, `slice_kv_get`, `slice_kv_set`, and `slice_vfs_snapshot` as ready-to-use MCP tools for Claude Code, Cursor, and autonomous LLM agents, complete with an optional `--audit-egress` guard that prevents indirect prompt injections from leaking `EnvCap` secrets into `stdout` or network calls.

