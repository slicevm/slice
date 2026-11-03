//! Polyglot Tiered Execution Core & WinterTC Native Engine (`slice-engine`)
//!
//! Implements the three-phase execution pipeline from the Nexus-JS blueprint:
//! - Tier 0: Direct Binary Bytecode & Snapshot Deserialization (`SLICESNP`), `< 50 us` cold start.
//! - Tier 1: Adaptive Baseline Promotion after hot-loop threshold (`> 1,000` iterations).
//! - Tier 2: Native SIMD / JIT Intrinsics for compute-heavy kernels, JSON, Crypto, and WinterTC routing.
//!
//! Includes a complete recursive-descent ECMAScript/TypeScript AST & Bytecode VM supporting:
//! - `var`, `let`, `const`, assignments (`=`, `+=`, `-=`, `*=`, `/=`), `++`, `--`
//! - Full operator precedence (`||`, `&&`, `===`, `!==`, `==`, `!=`, `<`, `<=`, `>`, `>=`, `+`, `-`, `*`, `/`, `%`, `**`, `!`, unary `-`/`+`, ternary `? :`)
//! - Arrays `[1, 2, 3]`, Objects `{ a: 1 }`, member access `obj.prop` / `arr[i]`, methods (`.map`, `.filter`, `.reduce`, `.push`, `.join`, `.slice`, `.includes`, `.length`)
//! - Functions (`function f(a, b) {}`, arrow functions `(x) => x * 2`, closures, recursion)
//! - Control flow (`if / else`, `while`, `for`, `return`, `break`, `continue`, `typeof`)
//! - Built-in globals (`console.log`, `print`, `Math.*`, `JSON.*`, `Object.freeze`, `Array.isArray`, `Number`, `String`, `parseInt`, `parseFloat`)
//! - Native Tier-2 SIMD intrinsics (`fib`, `numericLoop`, `cryptoDigest`, `WinterTC.route`, `fetch`, `fs.readFileSync`, `env`)
//! - SES `lockdown()` prototype freeze enforcement and OCap capability token traps.
//!
//! ## Module Architecture
//! - [`value_and_ast`]: 16-byte cache-line [`JsValue`] layout, [`ExecutionTier`], AST nodes ([`Stmt`], [`Expr`]), and [`BytecodeOp`].
//! - [`parser_and_compiler`]: Recursive-descent JS/TS lexer/parser, [`compile_source_to_bytecode`], and [`disassemble_bytecode`].
//! - [`isolate`]: [`SliceIsolate`] lifecycle, Tier-0/Tier-1/Tier-2 execution, [`HttpRequestContext`], and [`SandboxEvalOptions`].
//! - [`boa_realm`]: Warm `boa_engine` realm pool, thread-local compartment state, `CORE_PRELUDE` WinterTC/SSE/Fetch bootstrap, and lazy `node:*` / `slice:test` loaders.
//! - [`intrinsics`]: Native hardware `SHA-NI` + FIPS-180-4 SHA-256, HMAC-SHA256, Base64, SemVer, `ms`, and `bytes` fast-paths.
//! - [`state_and_kv`]: 16-way sharded [`EDGE_KV_SLAB`](state_and_kv::EDGE_KV_SLAB) (`Slice.kv`), `< 5 us` VFS+KV checkpoints, SplitMix64 PRNG, and Egress DLP guard.
//! - [`outbound_http`]: Outbound `fetch()` HTTP/1.1 socket engine, OCap host allowlist, and Cloud Metadata SSRF shield.

pub mod boa_realm;
pub mod event_loop;
pub mod host_bindings;
pub mod intrinsics;
pub mod isolate;
pub mod js_minifier;
pub mod js_preludes;
pub mod node_builtins;
pub mod numeric_vm;
pub mod outbound_http;
pub mod parser_and_compiler;
pub mod sockets;
pub mod state_and_kv;
pub mod value_and_ast;
pub mod wasm;
pub mod websocket;
pub mod workers;

#[cfg(test)]
mod tests;

pub use event_loop::{reset_virtual_time_warped_ms, virtual_time_warped_ms};
pub use isolate::{
    run_source_isolated, ExecutionMetrics, HttpExecutionOutcome, HttpRequestContext,
    SandboxEvalOptions, SandboxEvalOutcome, SliceIsolate,
};
pub use outbound_http::{
    clear_outbound_conn_pool, is_valid_http_header_name, is_valid_http_header_value,
    is_valid_request_target, outbound_pool_reuse_count, parse_chunked_body,
    register_custom_root_cert_der, MAX_OUTBOUND_BODY_BYTES, MAX_OUTBOUND_HEADER_BYTES,
};
pub use parser_and_compiler::{compile_source_to_bytecode, disassemble_bytecode};
pub use value_and_ast::{
    BinOp, BytecodeModule, BytecodeOp, ExecutionTier, Expr, JsFunction, JsValue, Stmt, UnaryOp,
    SNAPSHOT_MAGIC, TIER1_PROMOTION_THRESHOLD,
};
pub use websocket::{
    apply_websocket_mask_in_place, build_close_frame_payload, clear_thread_websocket_state,
    compute_websocket_accept_key, decode_websocket_frame, drain_server_ws_outbox,
    encode_websocket_frame, parse_close_frame_payload, read_websocket_message_from_stream,
    DecodedWsFrame, ServerWsOutFrame, MAX_WS_FRAME_BYTES, WS_GUID, WS_OPCODE_BINARY,
    WS_OPCODE_CLOSE, WS_OPCODE_CONTINUATION, WS_OPCODE_PING, WS_OPCODE_PONG, WS_OPCODE_TEXT,
};
pub use workers::{
    prewarm_worker_pool_sync, reset_worker_pool_telemetry, speculative_prewarm_worker_pool,
    worker_pool_reuse_count, worker_script_cache_hits,
};
pub use wasm::{
    clear_thread_wasm_state, reset_wasm_telemetry, wasm_module_cache_hits, MAX_WASM_BINARY_BYTES,
    MAX_WASM_MEMORY_PAGES, WASM_PAGE_BYTES,
};
