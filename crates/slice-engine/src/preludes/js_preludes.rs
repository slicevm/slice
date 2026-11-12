//! WinterTC, WHATWG, Node.js `Buffer`, Event-Loop Microtask/Immediate, `console.*`, and `WebSocket`
//! Micro-Preludes.
//!
//! All JavaScript preludes are maintained as clean, readable, multi-line `.js` source files under
//! `crates/slice-engine/src/preludes/*.js` and automatically minified at compile time by
//! `crates/slice-engine/build.rs` into `$OUT_DIR/minified_preludes.rs`.
//!
//! Only `ULTRA_CORE_PRELUDE` (`~2 KB` minified) is evaluated eagerly when creating a `boa_engine`
//! realm; Shards `1..=17` are registered via self-replacing lazy property getters on `globalThis`
//! (`__defLazy`) or pre-loaded on demand by `detect_needed_shards` with zero unused parse/eval cost.

use std::sync::OnceLock;

include!(concat!(env!("OUT_DIR"), "/minified_preludes.rs"));

/// Human-readable source for `ULTRA_CORE_PRELUDE` (`src/preludes/00_ultra_core.js`).
pub const ULTRA_CORE_PRELUDE_SRC: &str = include_str!("00_ultra_core.js");
/// Human-readable source for Shard 1: `WEB_FETCH_URL_PRELUDE` (`src/preludes/01_web_fetch_url.js`).
pub const WEB_FETCH_URL_PRELUDE_SRC: &str = include_str!("01_web_fetch_url.js");
/// Human-readable source for Shard 2: `ENCODING_EVENTS_PRELUDE` (`src/preludes/02_encoding_events.js`).
pub const ENCODING_EVENTS_PRELUDE_SRC: &str = include_str!("02_encoding_events.js");
/// Human-readable source for Shard 3: `STREAMS_FORMDATA_PRELUDE` (`src/preludes/03_streams_formdata.js`).
pub const STREAMS_FORMDATA_PRELUDE_SRC: &str = include_str!("03_streams_formdata.js");
/// Human-readable source for Shard 4: `WEBCRYPTO_CLONE_PRELUDE` (`src/preludes/04_webcrypto_clone.js`).
pub const WEBCRYPTO_CLONE_PRELUDE_SRC: &str = include_str!("04_webcrypto_clone.js");
/// Human-readable source for Shard 5: `FULL_BUFFER_PRELUDE` (`src/preludes/05_full_buffer.js`).
pub const FULL_BUFFER_PRELUDE_SRC: &str = include_str!("05_full_buffer.js");
/// Human-readable source for Shard 6: `CONSOLE_EXT_PRELUDE` (`src/preludes/06_console_ext.js`).
pub const CONSOLE_EXT_PRELUDE_SRC: &str = include_str!("06_console_ext.js");
/// Human-readable source for Shard 7: `WEBSOCKET_PRELUDE` (`src/preludes/07_websocket.js`).
pub const WEBSOCKET_PRELUDE_SRC: &str = include_str!("07_websocket.js");
/// Human-readable source for Shard 8: `WORKERS_SOCKETS_PRELUDE` (`src/preludes/08_workers_sockets.js`).
pub const WORKERS_SOCKETS_PRELUDE_SRC: &str = include_str!("08_workers_sockets.js");
/// Human-readable source for Shard 9: `WASM_PRELUDE` (`src/preludes/09_wasm.js`).
pub const WASM_PRELUDE_SRC: &str = include_str!("09_wasm.js");
/// Human-readable source for Shard 10: `OUTBOUND_FETCH_SSE_PRELUDE` (`src/preludes/10_outbound_fetch_sse.js`).
pub const OUTBOUND_FETCH_SSE_PRELUDE_SRC: &str = include_str!("10_outbound_fetch_sse.js");
/// Human-readable source for Shard 11: `FETCH_HEADERS_REQ_RES_PRELUDE` (`src/preludes/11_fetch_headers_req_res.js`).
pub const FETCH_HEADERS_REQ_RES_PRELUDE_SRC: &str = include_str!("11_fetch_headers_req_res.js");
/// Human-readable source for Shard 12: `URL_PATTERN_PRELUDE` (`src/preludes/12_url_pattern.js`).
pub const URL_PATTERN_PRELUDE_SRC: &str = include_str!("12_url_pattern.js");
/// Human-readable source for Shard 13: `DOM_EVENTS_PRELUDE` (`src/preludes/13_dom_events.js`).
pub const DOM_EVENTS_PRELUDE_SRC: &str = include_str!("13_dom_events.js");
/// Human-readable source for Shard 14: `TRANSFORM_COMPRESSION_STREAMS_PRELUDE` (`src/preludes/14_transform_compression_streams.js`).
pub const TRANSFORM_COMPRESSION_STREAMS_PRELUDE_SRC: &str =
    include_str!("14_transform_compression_streams.js");
/// Human-readable source for Shard 15: `BLOB_FORMDATA_PRELUDE` (`src/preludes/15_blob_formdata.js`).
pub const BLOB_FORMDATA_PRELUDE_SRC: &str = include_str!("15_blob_formdata.js");
/// Human-readable source for Shard 16: `STRUCTURED_CLONE_PRELUDE` (`src/preludes/16_structured_clone.js`).
pub const STRUCTURED_CLONE_PRELUDE_SRC: &str = include_str!("16_structured_clone.js");
/// Human-readable source for Shard 17: `BUFFER_BINARY_RW_PRELUDE` (`src/preludes/17_buffer_binary_rw.js`).
pub const BUFFER_BINARY_RW_PRELUDE_SRC: &str = include_str!("17_buffer_binary_rw.js");
/// Human-readable source for Shard 18: `INTL_PRELUDE` (`src/preludes/18_intl.js`).
pub const INTL_PRELUDE_SRC: &str = include_str!("18_intl.js");
/// Human-readable source for Shard 29: `PROCESS_CLI_PRELUDE` (`src/preludes/19_process_cli.js`).
pub const PROCESS_CLI_PRELUDE_SRC: &str = include_str!("19_process_cli.js");

/// On-demand self-replacing lazy property getter registration (`__defLazy`) evaluated ONLY when
/// `needs_lazy_global_getters(js_code)` is true (`globalThis.*` dynamic probes).
pub(crate) const LAZY_GETTERS_INIT_PRELUDE: &[u8] = br#"(function(){var __loadShard=globalThis.__slice_ensure_shard,__shardProps=globalThis.__shardProps,__shMask=globalThis.__shMask;globalThis.__slice_mark_lazy_inited();function __defLazy(names,shardId){__shardProps[shardId]=names;for(var i=0;i<names.length;i++){(function(prop){Object.defineProperty(globalThis,prop,{configurable:true,enumerable:false,get:function(){__loadShard(shardId);return globalThis[prop];},set:function(val){Object.defineProperty(globalThis,prop,{value:val,writable:true,enumerable:false,configurable:true});}});})(names[i]);}}if((__shMask&2)===0)__defLazy(["URLSearchParams","URL"],1);if((__shMask&2048)===0)__defLazy(["Headers","Request","Response","__slice_make_web_request"],11);if((__shMask&4096)===0)__defLazy(["URLPattern"],12);if((__shMask&1024)===0)__defLazy(["AbortSignal","AbortController","fetch"],10);if((__shMask&4)===0)__defLazy(["btoa","atob","TextEncoder","TextDecoder"],2);if((__shMask&8192)===0)__defLazy(["Event","CustomEvent","EventTarget","DOMException"],13);if((__shMask&8)===0)__defLazy(["ReadableStream"],3);if((__shMask&16384)===0)__defLazy(["WritableStream","TransformStream","TextEncoderStream","TextDecoderStream","CompressionStream","DecompressionStream"],14);if((__shMask&32768)===0)__defLazy(["Blob","File","FormData"],15);if((__shMask&16)===0)__defLazy(["crypto","CryptoKey"],4);if((__shMask&65536)===0)__defLazy(["structuredClone"],16);if((__shMask&32)===0)__defLazy(["Buffer"],5);if((__shMask&64)===0){var __cMethods=["dir","dirxml","table","trace","assert","clear","count","countReset","group","groupCollapsed","groupEnd","time","timeLog","timeEnd"];for(var c=0;c<__cMethods.length;c++){(function(m){globalThis.console[m]=function(){__loadShard(6);return globalThis.console[m].apply(globalThis.console,arguments);};})(__cMethods[c]);}}if((__shMask&128)===0)__defLazy(["WebSocket","MessageEvent","CloseEvent","__slice_dispatch_server_ws"],7);if((__shMask&256)===0)__defLazy(["Worker","MessageChannel","MessagePort","__slice_receiveMessageOnPort","__slice_wintertc_connect","__slice_init_worker_self_env","__slice_worker_host_deliver","__slice_worker_self_deliver"],8);if((__shMask&512)===0)__defLazy(["WebAssembly"],9);if((__shMask&262144)===0)__defLazy(["Intl"],18);if((__shMask&536870912)===0)__defLazy(["navigator","location"],29);})();"#;

/// Canonical topological order for loading prelude shards (`1..=18` and `29`) so base dependencies
/// (`TextEncoder`, `EventTarget`, `URL`, `Request`/`Response`, `ReadableStream`, `Buffer`) are
/// always initialized before dependent shards (`fetch`, `TransformStream`, `WebSocket`, `Worker`, `Intl`).
#[allow(dead_code)]
pub(crate) const SHARD_LOAD_ORDER: [u32; 19] = [
    2, 13, 1, 11, 12, 10, 3, 14, 15, 4, 16, 5, 17, 6, 7, 8, 9, 29, 18,
];

/// Returns the compile-time minified JS source bytes for `shard_id` (`1..=18` or `29`).
#[inline]
pub(crate) fn shard_prelude_bytes(shard_id: u32) -> &'static [u8] {
    match shard_id {
        1 => WEB_FETCH_URL_PRELUDE,
        2 => ENCODING_EVENTS_PRELUDE,
        3 => STREAMS_FORMDATA_PRELUDE,
        4 => WEBCRYPTO_CLONE_PRELUDE,
        5 => FULL_BUFFER_PRELUDE,
        6 => CONSOLE_EXT_PRELUDE,
        7 => WEBSOCKET_PRELUDE,
        8 => WORKERS_SOCKETS_PRELUDE,
        9 => WASM_PRELUDE,
        10 => OUTBOUND_FETCH_SSE_PRELUDE,
        11 => FETCH_HEADERS_REQ_RES_PRELUDE,
        12 => URL_PATTERN_PRELUDE,
        13 => DOM_EVENTS_PRELUDE,
        14 => TRANSFORM_COMPRESSION_STREAMS_PRELUDE,
        15 => BLOB_FORMDATA_PRELUDE,
        16 => STRUCTURED_CLONE_PRELUDE,
        17 => BUFFER_BINARY_RW_PRELUDE,
        18 => INTL_PRELUDE,
        29 => PROCESS_CLI_PRELUDE,
        _ => b"",
    }
}

/// Registers all native Rust host bindings required by `shards_mask` onto `context`,
/// deduplicating shared host function groups (`wintertc_phase_n3`, `buffer`) so each native
/// group is registered at most once per realm.
pub(crate) fn register_shard_host_functions_for_mask(
    shards_mask: u32,
    context: &mut boa_engine::Context,
) {
    if shards_mask == 0 {
        return;
    }
    if (shards_mask & (1 << 1)) != 0 {
        crate::host_bindings::register_url_and_usp_host_functions(context);
    }
    if (shards_mask & ((1 << 2) | (1 << 5) | (1 << 14) | (1 << 15))) != 0 {
        crate::host_bindings::register_wintertc_phase_n3_host_functions(context);
    }
    if (shards_mask & (1 << 4)) != 0 {
        crate::host_bindings::register_webcrypto_host_functions(context);
    }
    if (shards_mask & ((1 << 5) | (1 << 7))) != 0 {
        crate::host_bindings::register_buffer_host_functions(context);
    }
    if (shards_mask & (1 << 7)) != 0 {
        crate::host_bindings::register_shard7_websocket_host_functions(context);
    }
    if (shards_mask & (1 << 8)) != 0 {
        crate::host_bindings::register_shard8_workers_sockets_host_functions(context);
    }
    if (shards_mask & (1 << 9)) != 0 {
        crate::host_bindings::register_shard9_wasm_host_functions(context);
    }
    if (shards_mask & (1 << 10)) != 0 {
        crate::host_bindings::register_shard10_outbound_fetch_host_functions(context);
    }
    if (shards_mask & (1 << 11)) != 0 {
        crate::host_bindings::register_http_bridge_host_functions(context);
    }
    if (shards_mask & (1 << 19)) != 0 {
        crate::host_bindings::register_node_crypto_bootstrap_host_function(context);
    }
    if (shards_mask & (1 << 20)) != 0 {
        crate::host_bindings::register_fs_host_functions(context);
    }
    if (shards_mask & (1 << 23)) != 0 {
        crate::host_bindings::register_os_host_functions(context);
    }
}

/// Builds the single-pass fused prelude prefix (`ULTRA_CORE_PRELUDE`, all set shards in
/// `shards_mask` in topological `SHARD_LOAD_ORDER`, `globalThis.__shMask`, optional
/// `LAZY_GETTERS_INIT_PRELUDE`, and boot-flag hooks) so that when placed immediately before
/// `fallback_js` in `SLICEBC\x01`, the entire `[precompiled_prefix .. fallback_js]` region is a
/// single contiguous zero-copy ECMAScript program in memory.
pub(crate) fn build_fused_prelude_prefix(
    shards_mask: u32,
    init_lazy_getters: bool,
    has_next_tick: bool,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(ULTRA_CORE_PRELUDE.len() + 8192);
    out.extend_from_slice(ULTRA_CORE_PRELUDE);
    let mut active_mask: u32 = 0;
    for shard_id in SHARD_LOAD_ORDER {
        let bit = 1u32 << shard_id;
        if (shards_mask & bit) != 0 {
            let shard_bytes = shard_prelude_bytes(shard_id);
            if !shard_bytes.is_empty() {
                out.extend_from_slice(shard_bytes);
                active_mask |= bit;
            }
        }
    }
    if init_lazy_getters {
        active_mask |= 1;
    }
    if active_mask != 0 {
        let mut mask_buf = itoa::Buffer::new();
        out.extend_from_slice(b"globalThis.__shMask|=");
        out.extend_from_slice(mask_buf.format(active_mask).as_bytes());
        out.push(b';');
    }
    for builtin_bit in 19..=28u32 {
        if (shards_mask & (1u32 << builtin_bit)) != 0 {
            let b_bytes = crate::node_builtins::builtin_snippet_for_bit(builtin_bit);
            if !b_bytes.is_empty() {
                out.extend_from_slice(b_bytes);
                if !b_bytes.ends_with(b";") {
                    out.push(b';');
                }
            }
        }
    }
    if init_lazy_getters {
        out.extend_from_slice(LAZY_GETTERS_INIT_PRELUDE);
    }
    if has_next_tick {
        out.extend_from_slice(b"__slice_set_next_tick_hook(true);");
    }
    out.extend_from_slice(
        b"if(globalThis.__slice_bf){if(globalThis.__slice_bf&1)__slice_sync_process_env(true);if(globalThis.__slice_bf&2)__slice_set_det_mode(true);if(globalThis.__slice_bf&4)globalThis.__slice_init_worker_self_env(true);}",
    );
    out
}

/// Computes the exact newline count emitted by [`build_fused_prelude_prefix`] so runtime stack
/// traces from precompiled `SLICEBC\x01` `CodeBlock`s subtract the prelude prefix offset accurately.
#[cold]
pub(crate) fn count_fused_prelude_newlines(shards_mask: u32, init_lazy_getters: bool) -> usize {
    let mut nl = ULTRA_CORE_PRELUDE.iter().filter(|&&b| b == b'\n').count();
    for shard_id in SHARD_LOAD_ORDER {
        let bit = 1u32 << shard_id;
        if (shards_mask & bit) != 0 {
            let shard_bytes = shard_prelude_bytes(shard_id);
            nl += shard_bytes.iter().filter(|&&b| b == b'\n').count();
        }
    }
    for builtin_bit in 19..=28u32 {
        if (shards_mask & (1u32 << builtin_bit)) != 0 {
            let b_bytes = crate::node_builtins::builtin_snippet_for_bit(builtin_bit);
            nl += b_bytes.iter().filter(|&&b| b == b'\n').count();
        }
    }
    if init_lazy_getters {
        nl += LAZY_GETTERS_INIT_PRELUDE
            .iter()
            .filter(|&&b| b == b'\n')
            .count();
    }
    nl
}

/// Concatenates `build_fused_prelude_prefix` and `js_code` into a single contiguous
/// ECMAScript compilation unit so `boa_engine` parses and compiles everything in one pass.
#[allow(dead_code)]
pub(crate) fn build_fused_prelude_and_bundle(
    js_code: &str,
    shards_mask: u32,
    init_lazy_getters: bool,
    has_next_tick: bool,
) -> Vec<u8> {
    let mut out = build_fused_prelude_prefix(shards_mask, init_lazy_getters, has_next_tick);
    out.reserve(js_code.len() + 4);
    if js_code.starts_with('{') {
        out.extend_from_slice(js_code.as_bytes());
    } else {
        out.extend_from_slice(b"{\n");
        out.extend_from_slice(js_code.as_bytes());
        out.extend_from_slice(b"\n}");
    }
    out
}

static ULTRA_CORE_SLICEBC: OnceLock<Vec<u8>> = OnceLock::new();
static LAZY_GETTERS_SLICEBC: OnceLock<Vec<u8>> = OnceLock::new();
static SHARD_SLICEBC: [OnceLock<Vec<u8>>; 30] = [const { OnceLock::new() }; 30];

#[inline]
pub(crate) fn compile_static_js_to_slicebc(bytes: &[u8]) -> Vec<u8> {
    if cfg!(debug_assertions) {
        let owned = bytes.to_vec();
        return std::thread::Builder::new()
            .name("slicebc-static-compiler".to_string())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let mut compiler_ctx = boa_engine::Context::default();
                if let Ok(script) = boa_engine::Script::parse(
                    boa_engine::Source::from_bytes(&owned),
                    None,
                    &mut compiler_ctx,
                ) {
                    script
                        .compile_to_slicebc(&mut compiler_ctx)
                        .unwrap_or_default()
                } else {
                    Vec::new()
                }
            })
            .expect("failed to spawn slicebc-static-compiler thread")
            .join()
            .expect("slicebc-static-compiler thread panicked");
    }
    let mut compiler_ctx = boa_engine::Context::default();
    if let Ok(script) = boa_engine::Script::parse(
        boa_engine::Source::from_bytes(bytes),
        None,
        &mut compiler_ctx,
    ) {
        script
            .compile_to_slicebc(&mut compiler_ctx)
            .unwrap_or_default()
    } else {
        Vec::new()
    }
}

/// Evaluates `ULTRA_CORE_PRELUDE` inside `context` using cached precompiled `SLICEBC\x01` `CodeBlock` bytecode.
#[inline]
pub(crate) fn eval_ultra_core_prelude(
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<boa_engine::JsValue> {
    let precompiled =
        ULTRA_CORE_SLICEBC.get_or_init(|| compile_static_js_to_slicebc(ULTRA_CORE_PRELUDE));
    if !precompiled.is_empty() {
        if let Some(script) = boa_engine::Script::from_slicebc_bytes(precompiled, context) {
            return script.evaluate(context);
        }
    }
    context.eval(boa_engine::Source::from_bytes(ULTRA_CORE_PRELUDE))
}

/// Evaluates `LAZY_GETTERS_INIT_PRELUDE` inside `context` using cached precompiled `SLICEBC\x01` `CodeBlock` bytecode.
#[inline]
pub(crate) fn eval_lazy_getters_init_prelude(
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<boa_engine::JsValue> {
    let precompiled = LAZY_GETTERS_SLICEBC
        .get_or_init(|| compile_static_js_to_slicebc(LAZY_GETTERS_INIT_PRELUDE));
    if !precompiled.is_empty() {
        if let Some(script) = boa_engine::Script::from_slicebc_bytes(precompiled, context) {
            return script.evaluate(context);
        }
    }
    context.eval(boa_engine::Source::from_bytes(LAZY_GETTERS_INIT_PRELUDE))
}

/// Evaluates a compile-time minified lazy prelude shard (`1..=18` or `29`) on demand inside `context`
/// using cached precompiled `SLICEBC\x01` `CodeBlock` bytecode.
#[inline]
pub(crate) fn eval_lazy_prelude_shard(
    shard_id: u32,
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<boa_engine::JsValue> {
    if !((1..=18).contains(&shard_id) || shard_id == 29) {
        return Ok(boa_engine::JsValue::undefined());
    }
    register_shard_host_functions_for_mask(1u32 << shard_id, context);
    let bytes = shard_prelude_bytes(shard_id);
    let precompiled =
        SHARD_SLICEBC[shard_id as usize].get_or_init(|| compile_static_js_to_slicebc(bytes));
    if !precompiled.is_empty() {
        if let Some(script) = boa_engine::Script::from_slicebc_bytes(precompiled, context) {
            return script.evaluate(context);
        }
    }
    context.eval(boa_engine::Source::from_bytes(bytes))
}

#[inline]
fn is_outbound_fetch_at(js_code: &str, idx: usize) -> bool {
    let bytes = js_code.as_bytes();
    let is_member = idx > 0 && bytes[idx - 1] == b'.';
    let is_async_method = idx >= 6 && &bytes[idx - 6..idx] == b"async ";
    let is_ident_suffix =
        idx > 0 && (bytes[idx - 1].is_ascii_alphanumeric() || bytes[idx - 1] == b'_');
    let is_sync_method = js_code[idx + 6..]
        .find(')')
        .is_some_and(|c| js_code[idx + 6 + c + 1..].trim_start().starts_with('{'));
    !is_member && !is_async_method && !is_ident_suffix && !is_sync_method
}

#[inline]
fn is_in_lazy_string_module_line(bytes: &[u8], pos: usize) -> bool {
    let line_start = memchr::memrchr(b'\n', &bytes[..pos]).map_or(0, |i| i + 1);
    let line_rem = &bytes[line_start..];
    if !line_rem.starts_with(b"__slice_modules[") {
        return false;
    }
    let after_prefix = &line_rem[b"__slice_modules[".len()..];
    let Some(close_bracket) = memchr::memchr(b']', after_prefix) else {
        return false;
    };
    let after_bracket = &after_prefix[close_bracket + 1..];
    after_bracket.starts_with(b" = \"") || after_bracket.starts_with(b"=\"")
}

#[derive(Clone, Copy)]
enum ShardTriggerAction {
    Mask(u32),
    Flags(u16),
    MaskAndFlags(u32, u16),
    BuiltinMask(u32),
    FetchCall,
    BufBin,
    HasBinRwMarker,
    GlobalThisRef,
    GetOwnPropDescRef,
}

const SHARD_PATTERNS: &[(&str, ShardTriggerAction)] = &[
    // Outbound Fetch & SSE (`(1 << 10) | (1 << 11)`)
    ("fetch(", ShardTriggerAction::FetchCall),
    ("fetchSync", ShardTriggerAction::Mask((1 << 10) | (1 << 11))),
    ("mockFetch", ShardTriggerAction::Mask((1 << 10) | (1 << 11))),
    (
        "streamSSE",
        ShardTriggerAction::Mask((1 << 10) | (1 << 11) | (1 << 3)),
    ),
    (
        "Response.sse",
        ShardTriggerAction::Mask((1 << 10) | (1 << 11) | (1 << 3)),
    ),
    (
        "AbortController",
        ShardTriggerAction::Mask((1 << 10) | (1 << 11) | (1 << 13)),
    ),
    (
        "AbortSignal",
        ShardTriggerAction::Mask((1 << 10) | (1 << 11) | (1 << 13)),
    ),
    // HTTP Request / Response / Headers (`1 << 11`)
    (".fetch(", ShardTriggerAction::Mask(1 << 11)),
    ("new Request(", ShardTriggerAction::Mask(1 << 11)),
    ("new Response(", ShardTriggerAction::Mask(1 << 11)),
    ("Response.", ShardTriggerAction::Mask(1 << 11)),
    ("new Headers(", ShardTriggerAction::Mask(1 << 11)),
    ("Headers.", ShardTriggerAction::Mask(1 << 11)),
    ("Request.", ShardTriggerAction::Mask(1 << 11)),
    (
        "__slice_dispatch_fetch",
        ShardTriggerAction::BuiltinMask(1 << 11),
    ),
    // URL & URLSearchParams (`1 << 1` & builtin `1 << 27`)
    ("new URL(", ShardTriggerAction::Mask(1 << 1)),
    ("URL.", ShardTriggerAction::Mask(1 << 1)),
    ("new URLSearchParams(", ShardTriggerAction::Mask(1 << 1)),
    ("URLSearchParams.", ShardTriggerAction::Mask(1 << 1)),
    ("node:url", ShardTriggerAction::Mask((1 << 1) | (1 << 27))),
    (
        "\"url\"",
        ShardTriggerAction::BuiltinMask((1 << 1) | (1 << 27)),
    ),
    (
        "'url'",
        ShardTriggerAction::BuiltinMask((1 << 1) | (1 << 27)),
    ),
    ("createObjectURL", ShardTriggerAction::Mask(1 << 1)),
    ("canParse", ShardTriggerAction::Mask(1 << 1)),
    // URLPattern (`1 << 12`)
    ("URLPattern", ShardTriggerAction::Mask(1 << 12)),
    // TextEncoder / TextDecoder / Base64 (`1 << 2`)
    ("TextEncoder", ShardTriggerAction::Mask(1 << 2)),
    ("TextDecoder", ShardTriggerAction::Mask(1 << 2)),
    ("btoa(", ShardTriggerAction::Mask(1 << 2)),
    ("atob(", ShardTriggerAction::Mask(1 << 2)),
    // Event / EventTarget / DOMException (`1 << 13`)
    ("new Event(", ShardTriggerAction::Mask(1 << 13)),
    ("typeof Event", ShardTriggerAction::Mask(1 << 13)),
    ("CustomEvent", ShardTriggerAction::Mask(1 << 13)),
    ("EventTarget", ShardTriggerAction::Mask(1 << 13)),
    ("DOMException", ShardTriggerAction::Mask(1 << 13)),
    // Streams (`(1 << 14) | (1 << 3)` and `1 << 3`)
    (
        "WritableStream",
        ShardTriggerAction::Mask((1 << 14) | (1 << 3)),
    ),
    (
        "TransformStream",
        ShardTriggerAction::Mask((1 << 14) | (1 << 3)),
    ),
    (
        "TextEncoderStream",
        ShardTriggerAction::Mask((1 << 14) | (1 << 3) | (1 << 2)),
    ),
    (
        "TextDecoderStream",
        ShardTriggerAction::Mask((1 << 14) | (1 << 3) | (1 << 2)),
    ),
    (
        "CompressionStream",
        ShardTriggerAction::Mask((1 << 14) | (1 << 3)),
    ),
    (
        "DecompressionStream",
        ShardTriggerAction::Mask((1 << 14) | (1 << 3)),
    ),
    ("ReadableStream", ShardTriggerAction::Mask(1 << 3)),
    // Blob / File / FormData (`1 << 15`)
    ("new Blob(", ShardTriggerAction::Mask(1 << 15)),
    ("new File(", ShardTriggerAction::Mask(1 << 15)),
    ("new FormData(", ShardTriggerAction::Mask(1 << 15)),
    ("typeof FormData", ShardTriggerAction::Mask(1 << 15)),
    ("typeof Blob", ShardTriggerAction::Mask(1 << 15)),
    ("typeof File", ShardTriggerAction::Mask(1 << 15)),
    ("FormData.", ShardTriggerAction::Mask(1 << 15)),
    ("Blob.", ShardTriggerAction::Mask(1 << 15)),
    ("File.", ShardTriggerAction::Mask(1 << 15)),
    // WebCrypto (`1 << 4`)
    ("subtle", ShardTriggerAction::Mask(1 << 4)),
    ("getRandomValues", ShardTriggerAction::Mask(1 << 4)),
    ("randomUUID", ShardTriggerAction::Mask(1 << 4)),
    ("webcrypto", ShardTriggerAction::Mask(1 << 4)),
    ("CryptoKey", ShardTriggerAction::Mask(1 << 4)),
    ("SubtleCrypto", ShardTriggerAction::Mask(1 << 4)),
    ("typeof crypto", ShardTriggerAction::Mask(1 << 4)),
    ("globalThis.crypto", ShardTriggerAction::Mask(1 << 4)),
    ("window.crypto", ShardTriggerAction::Mask(1 << 4)),
    ("self.crypto", ShardTriggerAction::Mask(1 << 4)),
    // structuredClone (`1 << 16`)
    ("structuredClone", ShardTriggerAction::Mask(1 << 16)),
    // Buffer & Binary R/W (`1 << 5`, `1 << 17`, & builtin `1 << 26`)
    ("writeUInt", ShardTriggerAction::BufBin),
    ("readUInt", ShardTriggerAction::BufBin),
    ("writeInt", ShardTriggerAction::BufBin),
    ("readInt", ShardTriggerAction::BufBin),
    ("writeBig", ShardTriggerAction::BufBin),
    ("readBig", ShardTriggerAction::BufBin),
    ("_hasBinRW", ShardTriggerAction::HasBinRwMarker),
    ("Buffer.from", ShardTriggerAction::Mask(1 << 5)),
    ("Buffer.alloc", ShardTriggerAction::Mask(1 << 5)),
    ("globalThis.Buffer", ShardTriggerAction::Mask(1 << 5)),
    ("Buffer.byteLength", ShardTriggerAction::Mask(1 << 5)),
    ("Buffer.isBuffer", ShardTriggerAction::Mask(1 << 5)),
    ("Buffer.isEncoding", ShardTriggerAction::Mask(1 << 5)),
    ("Buffer(", ShardTriggerAction::Mask(1 << 5)),
    ("Buffer.concat", ShardTriggerAction::Mask(1 << 5)),
    ("Buffer.compare", ShardTriggerAction::Mask(1 << 5)),
    ("new Buffer(", ShardTriggerAction::Mask(1 << 5)),
    (
        "node:buffer",
        ShardTriggerAction::Mask((1 << 5) | (1 << 26)),
    ),
    (
        "\"buffer\"",
        ShardTriggerAction::BuiltinMask((1 << 5) | (1 << 26)),
    ),
    (
        "'buffer'",
        ShardTriggerAction::BuiltinMask((1 << 5) | (1 << 26)),
    ),
    ("instanceof Buffer", ShardTriggerAction::Mask(1 << 5)),
    ("randomBytes", ShardTriggerAction::Mask(1 << 5)),
    ("pbkdf2", ShardTriggerAction::Mask(1 << 5)),
    ("scrypt", ShardTriggerAction::Mask(1 << 5)),
    ("createCipheriv", ShardTriggerAction::Mask(1 << 5)),
    ("createDecipheriv", ShardTriggerAction::Mask(1 << 5)),
    ("generateKeyPair", ShardTriggerAction::Mask(1 << 5)),
    ("createSign", ShardTriggerAction::Mask(1 << 5)),
    ("createVerify", ShardTriggerAction::Mask(1 << 5)),
    ("createHash(\"md5\"", ShardTriggerAction::Mask(1 << 5)),
    ("createHash('md5'", ShardTriggerAction::Mask(1 << 5)),
    ("createHash(\"sha512\"", ShardTriggerAction::Mask(1 << 5)),
    ("createHash('sha512'", ShardTriggerAction::Mask(1 << 5)),
    ("createHash(\"sha384\"", ShardTriggerAction::Mask(1 << 5)),
    ("createHash('sha384'", ShardTriggerAction::Mask(1 << 5)),
    ("createHash(\"sha1\"", ShardTriggerAction::Mask(1 << 5)),
    ("createHash('sha1'", ShardTriggerAction::Mask(1 << 5)),
    ("createHmac(\"sha512\"", ShardTriggerAction::Mask(1 << 5)),
    ("createHmac('sha512'", ShardTriggerAction::Mask(1 << 5)),
    // Console Extended (`1 << 6`)
    ("console.table", ShardTriggerAction::Mask(1 << 6)),
    ("console.dir", ShardTriggerAction::Mask(1 << 6)),
    ("console.group", ShardTriggerAction::Mask(1 << 6)),
    ("console.time", ShardTriggerAction::Mask(1 << 6)),
    ("console.trace", ShardTriggerAction::Mask(1 << 6)),
    ("console.count", ShardTriggerAction::Mask(1 << 6)),
    ("console.assert", ShardTriggerAction::Mask(1 << 6)),
    ("console.clear", ShardTriggerAction::Mask(1 << 6)),
    // WebSocket (`(1 << 7) | (1 << 13)`)
    ("WebSocket", ShardTriggerAction::Mask((1 << 7) | (1 << 13))),
    (
        "MessageEvent",
        ShardTriggerAction::Mask((1 << 7) | (1 << 13)),
    ),
    ("CloseEvent", ShardTriggerAction::Mask((1 << 7) | (1 << 13))),
    (
        "mockWebSocket",
        ShardTriggerAction::Mask((1 << 7) | (1 << 13)),
    ),
    (
        "onWebSocket",
        ShardTriggerAction::Mask((1 << 7) | (1 << 13)),
    ),
    (
        "upgradeWebSocket",
        ShardTriggerAction::Mask((1 << 7) | (1 << 13)),
    ),
    // WinterTC Sockets & Workers (`1 << 8` etc. + `BOACAS_FLAG_HAS_WORKER`)
    (
        "cloudflare:sockets",
        ShardTriggerAction::Mask((1 << 8) | (1 << 14) | (1 << 3) | (1 << 13) | (1 << 2)),
    ),
    (
        "slice:sockets",
        ShardTriggerAction::Mask((1 << 8) | (1 << 14) | (1 << 3) | (1 << 13) | (1 << 2)),
    ),
    (
        "Slice.connect",
        ShardTriggerAction::Mask((1 << 8) | (1 << 14) | (1 << 3) | (1 << 13) | (1 << 2)),
    ),
    (
        "Worker",
        ShardTriggerAction::MaskAndFlags(
            (1 << 8) | (1 << 13) | (1 << 2),
            crate::boacas::BOACAS_FLAG_HAS_WORKER,
        ),
    ),
    (
        "worker_threads",
        ShardTriggerAction::MaskAndFlags(
            (1 << 8) | (1 << 13) | (1 << 2),
            crate::boacas::BOACAS_FLAG_HAS_WORKER,
        ),
    ),
    (
        "MessageChannel",
        ShardTriggerAction::Mask((1 << 8) | (1 << 13) | (1 << 2)),
    ),
    (
        "MessagePort",
        ShardTriggerAction::Mask((1 << 8) | (1 << 13) | (1 << 2)),
    ),
    (
        "parentPort",
        ShardTriggerAction::Mask((1 << 8) | (1 << 13) | (1 << 2)),
    ),
    (
        "workerData",
        ShardTriggerAction::Mask((1 << 8) | (1 << 13) | (1 << 2)),
    ),
    (
        "mockSocket",
        ShardTriggerAction::Mask((1 << 8) | (1 << 13) | (1 << 2)),
    ),
    // WebAssembly (`1 << 9`)
    ("WebAssembly", ShardTriggerAction::Mask(1 << 9)),
    // Shard 18: WinterTC `Intl` (`1 << 18`)
    ("Intl", ShardTriggerAction::Mask(1 << 18)),
    ("toLocaleString", ShardTriggerAction::Mask(1 << 18)),
    ("toLocaleDateString", ShardTriggerAction::Mask(1 << 18)),
    ("toLocaleTimeString", ShardTriggerAction::Mask(1 << 18)),
    ("localeCompare", ShardTriggerAction::Mask(1 << 18)),
    // Shard 29: `navigator`, `location`, & Complete `process` CLI Surface (`1 << 29`)
    ("navigator", ShardTriggerAction::Mask(1 << 29)),
    ("location.", ShardTriggerAction::Mask(1 << 29)),
    ("globalThis.location", ShardTriggerAction::Mask(1 << 29)),
    ("stdout", ShardTriggerAction::Mask(1 << 29)),
    ("stderr", ShardTriggerAction::Mask(1 << 29)),
    ("stdin", ShardTriggerAction::Mask(1 << 29)),
    ("memoryUsage", ShardTriggerAction::Mask(1 << 29)),
    ("hrtime", ShardTriggerAction::Mask(1 << 29)),
    ("uptime", ShardTriggerAction::Mask(1 << 29)),
    ("emitWarning", ShardTriggerAction::Mask(1 << 29)),
    ("argv0", ShardTriggerAction::Mask(1 << 29)),
    ("execArgv", ShardTriggerAction::Mask(1 << 29)),
    ("execPath", ShardTriggerAction::Mask(1 << 29)),
    ("ppid", ShardTriggerAction::Mask(1 << 29)),
    ("process.on", ShardTriggerAction::Mask(1 << 29)),
    ("process.once", ShardTriggerAction::Mask(1 << 29)),
    ("process.off", ShardTriggerAction::Mask(1 << 29)),
    ("process.emit", ShardTriggerAction::Mask(1 << 29)),
    ("process.listeners", ShardTriggerAction::Mask(1 << 29)),
    ("process.removeListener", ShardTriggerAction::Mask(1 << 29)),
    (
        "process.removeAllListeners",
        ShardTriggerAction::Mask(1 << 29),
    ),
    ("process.addListener", ShardTriggerAction::Mask(1 << 29)),
    ("process.listenerCount", ShardTriggerAction::Mask(1 << 29)),
    ("unhandledRejection", ShardTriggerAction::Mask(1 << 29)),
    ("uncaughtException", ShardTriggerAction::Mask(1 << 29)),
    ("beforeExit", ShardTriggerAction::Mask(1 << 29)),
    ("node:process", ShardTriggerAction::Mask(1 << 29)),
    ("\"process\"", ShardTriggerAction::BuiltinMask(1 << 29)),
    ("'process'", ShardTriggerAction::BuiltinMask(1 << 29)),
    // Pre-fused `node:*` builtin module triggers (`19..=28`)
    ("node:crypto", ShardTriggerAction::BuiltinMask(1 << 19)),
    ("\"crypto\"", ShardTriggerAction::BuiltinMask(1 << 19)),
    ("'crypto'", ShardTriggerAction::BuiltinMask(1 << 19)),
    ("node:fs", ShardTriggerAction::BuiltinMask(1 << 20)),
    ("\"fs\"", ShardTriggerAction::BuiltinMask(1 << 20)),
    ("'fs'", ShardTriggerAction::BuiltinMask(1 << 20)),
    ("fs/promises", ShardTriggerAction::BuiltinMask(1 << 20)),
    ("node:path", ShardTriggerAction::BuiltinMask(1 << 21)),
    ("\"path\"", ShardTriggerAction::BuiltinMask(1 << 21)),
    ("'path'", ShardTriggerAction::BuiltinMask(1 << 21)),
    ("node:events", ShardTriggerAction::BuiltinMask(1 << 22)),
    ("\"events\"", ShardTriggerAction::BuiltinMask(1 << 22)),
    ("'events'", ShardTriggerAction::BuiltinMask(1 << 22)),
    ("node:os", ShardTriggerAction::BuiltinMask(1 << 23)),
    ("\"os\"", ShardTriggerAction::BuiltinMask(1 << 23)),
    ("'os'", ShardTriggerAction::BuiltinMask(1 << 23)),
    ("node:util", ShardTriggerAction::BuiltinMask(1 << 24)),
    ("\"util\"", ShardTriggerAction::BuiltinMask(1 << 24)),
    ("'util'", ShardTriggerAction::BuiltinMask(1 << 24)),
    ("node:assert", ShardTriggerAction::BuiltinMask(1 << 25)),
    ("\"assert\"", ShardTriggerAction::BuiltinMask(1 << 25)),
    ("'assert'", ShardTriggerAction::BuiltinMask(1 << 25)),
    ("node:timers", ShardTriggerAction::BuiltinMask(1 << 28)),
    ("\"timers\"", ShardTriggerAction::BuiltinMask(1 << 28)),
    ("'timers'", ShardTriggerAction::BuiltinMask(1 << 28)),
    // `SLICEBC\x01` feature flags (`BOACAS_FLAG_*`) folded into single-pass automaton
    (
        "nextTick",
        ShardTriggerAction::Flags(crate::boacas::BOACAS_FLAG_HAS_NEXT_TICK),
    ),
    (
        "Slice.request",
        ShardTriggerAction::MaskAndFlags(1 << 11, crate::boacas::BOACAS_FLAG_HAS_HTTP_BRIDGE),
    ),
    (
        "Slice.respond",
        ShardTriggerAction::MaskAndFlags(1 << 11, crate::boacas::BOACAS_FLAG_HAS_HTTP_BRIDGE),
    ),
    (
        "__slice_http_",
        ShardTriggerAction::MaskAndFlags(1 << 11, crate::boacas::BOACAS_FLAG_HAS_HTTP_BRIDGE),
    ),
    (
        "__slice_semver_",
        ShardTriggerAction::Flags(crate::boacas::BOACAS_FLAG_HAS_SEMVER_FAST),
    ),
    (
        "__slice_fast_",
        ShardTriggerAction::Flags(crate::boacas::BOACAS_FLAG_HAS_SEMVER_FAST),
    ),
    (
        "Slice.kv",
        ShardTriggerAction::Flags(crate::boacas::BOACAS_FLAG_HAS_KV_SANDBOX),
    ),
    (
        "Slice.checkpoint",
        ShardTriggerAction::Flags(crate::boacas::BOACAS_FLAG_HAS_KV_SANDBOX),
    ),
    (
        "Slice.restore",
        ShardTriggerAction::Flags(crate::boacas::BOACAS_FLAG_HAS_KV_SANDBOX),
    ),
    (
        "Slice.releaseCheckpoint",
        ShardTriggerAction::Flags(crate::boacas::BOACAS_FLAG_HAS_KV_SANDBOX),
    ),
    (
        "Slice.evalSandbox",
        ShardTriggerAction::Flags(crate::boacas::BOACAS_FLAG_HAS_KV_SANDBOX),
    ),
    (
        "node:vm",
        ShardTriggerAction::Flags(crate::boacas::BOACAS_FLAG_HAS_KV_SANDBOX),
    ),
    (
        "\"vm\"",
        ShardTriggerAction::Flags(crate::boacas::BOACAS_FLAG_HAS_KV_SANDBOX),
    ),
    (
        "'vm'",
        ShardTriggerAction::Flags(crate::boacas::BOACAS_FLAG_HAS_KV_SANDBOX),
    ),
    (
        "prototype",
        ShardTriggerAction::Flags(crate::boacas::BOACAS_FLAG_NEEDS_SES_CHECK),
    ),
    (
        "__proto__",
        ShardTriggerAction::Flags(crate::boacas::BOACAS_FLAG_NEEDS_SES_CHECK),
    ),
    (
        "Reflect",
        ShardTriggerAction::Flags(crate::boacas::BOACAS_FLAG_NEEDS_SES_CHECK),
    ),
    (
        "Object[",
        ShardTriggerAction::Flags(crate::boacas::BOACAS_FLAG_NEEDS_SES_CHECK),
    ),
    (
        "defineProperty",
        ShardTriggerAction::Flags(crate::boacas::BOACAS_FLAG_NEEDS_SES_CHECK),
    ),
    (
        "][",
        ShardTriggerAction::Flags(crate::boacas::BOACAS_FLAG_NEEDS_SES_CHECK),
    ),
    ("globalThis", ShardTriggerAction::GlobalThisRef),
    (
        "getOwnPropertyDescriptor",
        ShardTriggerAction::GetOwnPropDescRef,
    ),
];

#[inline]
fn shard_detector_automaton() -> &'static aho_corasick::AhoCorasick {
    static AC: std::sync::OnceLock<aho_corasick::AhoCorasick> = std::sync::OnceLock::new();
    AC.get_or_init(|| {
        let patterns: Vec<&str> = SHARD_PATTERNS.iter().map(|(p, _)| *p).collect();
        aho_corasick::AhoCorasick::builder()
            .match_kind(aho_corasick::MatchKind::Standard)
            .build(&patterns)
            .expect("Valid SHARD_PATTERNS AhoCorasick automaton")
    })
}

/// Computes `(feature_flags, shards_mask)` in a single zero-allocation `O(N)` SIMD multi-pattern pass
/// over `raw_js_code`, replacing 38 separate `.contains(...)` linear scans in `compute_boacas_metadata`.
#[inline]
pub(crate) fn detect_shards_and_boacas_flags(
    raw_js_code: &str,
    is_http: bool,
    is_rc_bundle: bool,
) -> (u16, u32) {
    let bytes = raw_js_code.as_bytes();
    let has_lazy_string_modules = memchr::memmem::find(bytes, b"__slice_modules[").is_some()
        && (memchr::memmem::find(bytes, b"] = \"").is_some()
            || memchr::memmem::find(bytes, b"]=\"").is_some());

    let mut flags: u16 = 0;
    let mut mask: u32 = if is_http { 1 << 11 } else { 0 };
    let mut needs_buf_bin = false;
    let mut has_bin_rw_marker = false;
    let mut has_global_this = false;
    let mut has_get_own_prop_desc = false;

    let ac = shard_detector_automaton();
    for mat in ac.find_overlapping_iter(raw_js_code) {
        let action = SHARD_PATTERNS[mat.pattern().as_usize()].1;
        match action {
            ShardTriggerAction::Flags(f) => {
                flags |= f;
                continue;
            }
            ShardTriggerAction::BuiltinMask(b_bits) => {
                mask |= b_bits;
                continue;
            }
            ShardTriggerAction::GlobalThisRef => {
                has_global_this = true;
                continue;
            }
            ShardTriggerAction::GetOwnPropDescRef => {
                has_get_own_prop_desc = true;
                continue;
            }
            ShardTriggerAction::MaskAndFlags(_, f) => {
                flags |= f;
            }
            _ => {}
        }
        if has_lazy_string_modules && is_in_lazy_string_module_line(bytes, mat.start()) {
            continue;
        }
        match action {
            ShardTriggerAction::Mask(bits) | ShardTriggerAction::MaskAndFlags(bits, _) => {
                mask |= bits;
            }
            ShardTriggerAction::FetchCall => {
                if is_outbound_fetch_at(raw_js_code, mat.start()) {
                    mask |= (1 << 10) | (1 << 11);
                }
            }
            ShardTriggerAction::BufBin => {
                mask |= 1 << 5;
                needs_buf_bin = true;
            }
            ShardTriggerAction::HasBinRwMarker => {
                has_bin_rw_marker = true;
            }
            _ => {}
        }
    }

    if needs_buf_bin && !has_bin_rw_marker {
        mask |= 1 << 17;
    }
    let needs_lazy = if is_rc_bundle {
        has_get_own_prop_desc
    } else {
        has_global_this
    };
    if needs_lazy {
        flags |= crate::boacas::BOACAS_FLAG_NEEDS_LAZY_GETTERS;
    }

    (flags, mask)
}

/// Computes a bitmask of lazy prelude shards (`1..=18` and `29`) referenced by `raw_js_code` in a single
/// zero-allocation `O(N)` SIMD multi-pattern pass so top-level execution pre-loads only the shards
/// actually used by a script before entering async/generator frames.
#[inline]
pub(crate) fn detect_needed_shards(raw_js_code: &str, is_http: bool) -> u32 {
    let (_, mask) = detect_shards_and_boacas_flags(raw_js_code, is_http, false);
    (mask & ((1 << 19) - 1)) | (mask & (1 << 29))
}

/// Returns `true` only when `js_code` may dynamically probe unreferenced `globalThis` shard properties
/// without mentioning their global names in `detect_needed_shards`. Multi-file `//__SLICE_RC:` bundles
/// always have their required shards pre-loaded by `detect_needed_shards`, avoiding the 84-closure
/// `__slice_init_lazy_getters()` overhead on cold start.
#[inline]
pub(crate) fn needs_lazy_global_getters(js_code: &str) -> bool {
    if js_code.contains("//__SLICE_RC:") {
        return js_code.contains("getOwnPropertyDescriptor");
    }
    js_code.contains("globalThis")
}
