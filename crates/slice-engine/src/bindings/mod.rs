//! Modular Native Rust Host Bindings (`__slice_*`) Registered into `boa_engine::Context`.
//!
//! Organized by capability and runtime domain:
//! - [`helpers`]: Zero-copy `TypedArray`/`ArrayBuffer`/`DataView` extraction, memory & fuel accounting, and WebCrypto digest/HMAC helpers.
//! - [`core`]: Always-on core bindings (`console`, source-mapped stacks, lazy shard/builtin loaders, `os`, SHA-256/HMAC-SHA256, `env`, timers).
//! - [`deterministic_and_os`]: Deterministic PRNG/clock (`--deterministic`), `os.tmpdir`, and `AsyncLocalStorage` snapshot bindings.
//! - [`intrinsics`]: Native `slice:semver`, `ms`, and `bytes` intrinsics.
//! - [`http_and_fetch`]: Inbound HTTP request/response bridge (`Slice.serve`, `node:http`) and outbound `fetch` (`Shard 10`).
//! - [`kv_and_sandbox`]: Multi-tenant Edge KV, transactional state checkpoints, and nested `Slice.evalSandbox` sub-isolates.
//! - [`wintertc_and_buffer`]: WHATWG `btoa`/`atob`, `TextEncoder`/`TextDecoder`, `zlib` compression, and `Buffer` hex/base64 codecs.
//! - [`fs`]: OCap-enforced Virtual Filesystem (`BOA_VFS_CACHE`) and host filesystem bindings (`node:fs`).
//! - [`crypto`]: WHATWG WebCrypto (`crypto.subtle`) and `node:crypto` ciphers, KDFs, and asymmetric signing (`Ed25519`, `ECDSA`).
//! - [`websocket`]: RFC 6455 WebSocket client/server host bindings (`Shard 7`).
//! - [`workers_and_sockets`]: Web `Worker` threads and OCap TCP/TLS/UDP sockets (`Shard 8`).
//! - [`wasm`]: Sandboxed WebAssembly (`wasmi`) compilation, memory, and execution (`Shard 9`).
//! - [`child_process`]: 5-Ring OCap-jailed subprocess execution (`node:child_process`).

pub(crate) mod child_process;
pub(crate) mod core;
pub(crate) mod crypto;
pub(crate) mod deterministic_and_os;
pub(crate) mod fs;
pub(crate) mod helpers;
pub(crate) mod http_and_fetch;
pub(crate) mod intrinsics;
pub(crate) mod kv_and_sandbox;
pub(crate) mod wasm;
pub(crate) mod websocket;
pub(crate) mod wintertc_and_buffer;
pub(crate) mod workers_and_sockets;

pub(crate) use self::child_process::register_child_process_host_functions;
pub(crate) use self::core::register_slice_native_host_functions;
pub(crate) use self::crypto::{
    register_node_crypto_bootstrap_host_function, register_webcrypto_host_functions,
};
pub(crate) use self::deterministic_and_os::{
    register_async_hooks_host_functions, register_deterministic_host_functions,
    register_os_host_functions,
};
pub(crate) use self::fs::register_fs_host_functions;
pub(crate) use self::http_and_fetch::{
    register_http_bridge_host_functions, register_shard10_outbound_fetch_host_functions,
};
pub(crate) use self::intrinsics::register_semver_and_fast_intrinsics_host_functions;
pub(crate) use self::kv_and_sandbox::register_kv_and_sandbox_host_functions;
pub(crate) use self::wasm::register_shard9_wasm_host_functions;
pub(crate) use self::websocket::register_shard7_websocket_host_functions;
pub(crate) use self::wintertc_and_buffer::{
    register_buffer_host_functions, register_url_and_usp_host_functions,
    register_wintertc_phase_n3_host_functions,
};
pub(crate) use self::workers_and_sockets::register_shard8_workers_sockets_host_functions;
