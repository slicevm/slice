//! Polyglot Tiered Execution Core & WinterTC Native Engine (`slice-engine`)
//!
//! Implements the three-phase execution pipeline from the Nexus-JS blueprint:
//! - Tier 0: Direct Binary Bytecode & Snapshot Deserialization (`SLICESNP`), `< 50 us` cold start.
//! - Tier 1: Adaptive Baseline Promotion after hot-loop threshold (`> 1,000` iterations).
//! - Tier 2: Native SIMD / JIT Intrinsics for compute-heavy kernels, JSON, Crypto, and WinterTC routing.
//!
//! ## Domain Architecture
//! - [`bindings`]: Modular native Rust host functions (`__slice_*`) registered into `boa_engine::Context`.
//! - [`intrinsics`]: Native hardware `SHA-NI` + FIPS-180-4 SHA-256, HMAC, AES-CBC/GCM, PBKDF2/HKDF/scrypt,
//!   Ed25519/ECDSA, Base64/Hex, WHATWG URL/URLSearchParams, DEFLATE/Zlib/Gzip, SemVer, `ms`, and `bytes`.
//! - [`isolate`]: [`SliceIsolate`] lifecycle, Tier-0/Tier-1/Tier-2 execution, [`HttpRequestContext`],
//!   [`SandboxEvalOptions`], and 16-way sharded [`EDGE_KV_SLAB`](state_and_kv::EDGE_KV_SLAB) (`Slice.kv`).
//! - [`vm`]: 16-byte [`JsValue`] layout, AST/Bytecode compiler ([`parser_and_compiler`]), unboxed [`numeric_vm`],
//!   and precompiled `SLICEBC\x01` [`boacas`] container.
//! - [`runtime`]: Warm `boa_engine` realm pool ([`boa_realm`]), virtual-time [`event_loop`], Web [`workers`],
//!   and sandboxed `wasmi` [`wasm`] engine.
//! - [`net`]: Outbound `fetch()` HTTP/1.1 & HTTPS engine ([`outbound_http`]), OCap TCP/TLS/UDP [`sockets`],
//!   and RFC 6455 [`websocket`] codec.
//! - [`preludes`]: Sharded JavaScript micro-preludes (`00_ultra_core.js` .. `18_intl.js`), Aho-Corasick shard
//!   detector ([`js_preludes`]), compile-time minifier ([`js_minifier`]), and lazy `node:*` built-ins ([`node_builtins`]).

pub mod bindings;
pub mod intrinsics;
pub mod isolate;
pub mod net;
pub mod preludes;
pub mod runtime;
pub mod vm;

// Backward-compatible flat module aliases:
pub use bindings as host_bindings;
pub use intrinsics::url_and_usp as web_primitives;
pub use isolate::state_and_kv;
pub use net::{outbound_http, sockets, websocket};
pub use preludes::{js_minifier, js_preludes, node_builtins};
pub use runtime::{boa_realm, event_loop, wasm, workers};
pub use vm::{boacas, numeric_vm, parser_and_compiler, value_and_ast};

#[cfg(test)]
#[path = "../tests/mod.rs"]
mod tests;

pub use boa_realm::ChromeTraceEvent;
pub use boacas::{
    active_tier_override, compute_boacas_metadata, is_exper_boacas_build_enabled,
    is_exper_boacas_runtime_enabled, pack_boacas_container, set_exper_boacas_override,
    set_thread_exper_boacas_override, set_thread_tier_override, set_tier_override, BoaCasHeader,
    BOACAS_FLAG_HAS_HTTP_BRIDGE, BOACAS_FLAG_HAS_KV_SANDBOX, BOACAS_FLAG_HAS_NEXT_TICK,
    BOACAS_FLAG_HAS_SEMVER_FAST, BOACAS_FLAG_HAS_WORKER, BOACAS_FLAG_IS_RC_BUNDLE,
    BOACAS_FLAG_NEEDS_LAZY_GETTERS, BOACAS_FLAG_NEEDS_SES_CHECK, BOACAS_FLAG_STARTS_WITH_BRACE,
    BOACAS_HEADER_LEN, BOACAS_MAGIC,
};
pub use event_loop::{
    fetch_reactor_active_threads, fetch_reactor_pool_reuse_count, reset_fetch_reactor_telemetry,
    reset_virtual_time_warped_ms, virtual_time_warped_ms, MAX_FETCH_REACTOR_WORKERS,
};
pub use isolate::{
    run_source_isolated, ExecutionMetrics, HttpExecutionOutcome, HttpRequestContext,
    SandboxEvalOptions, SandboxEvalOutcome, SliceIsolate,
};
pub use outbound_http::{
    clear_outbound_conn_pool, fetch_reactor_slab_acquire_count,
    fetch_reactor_slab_available_count, fetch_reactor_slab_in_use_count, is_valid_http_header_name,
    is_valid_http_header_value, is_valid_request_target, outbound_pool_reuse_count,
    parse_chunked_body, register_custom_root_cert_der, reset_fetch_slab_telemetry,
    MAX_OUTBOUND_BODY_BYTES, MAX_OUTBOUND_HEADER_BYTES,
};
pub use parser_and_compiler::{
    compile_source_to_bytecode, disassemble_bytecode, is_tier0_slicesnp_compatible_ast,
};
pub use value_and_ast::{
    BinOp, BytecodeModule, BytecodeOp, ExecutionTier, Expr, JsFunction, JsValue, Stmt, UnaryOp,
    SNAPSHOT_MAGIC, TIER1_PROMOTION_THRESHOLD,
};
pub use wasm::{
    clear_thread_wasm_state, reset_wasm_telemetry, wasm_module_cache_hits, MAX_WASM_BINARY_BYTES,
    MAX_WASM_MEMORY_PAGES, WASM_PAGE_BYTES,
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
