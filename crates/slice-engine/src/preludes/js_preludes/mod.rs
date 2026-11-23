//! WinterTC, WHATWG, Node.js `Buffer`, Event-Loop Microtask/Immediate, `console.*`, and `WebSocket`
//! Micro-Preludes.
//!
//! All JavaScript preludes are maintained as clean, readable, multi-line `.js` source files under
//! `crates/slice-engine/src/preludes/*.js` and automatically minified at compile time by
//! `crates/slice-engine/build.rs` into `$OUT_DIR/minified_preludes.rs`.
//!
//! Only `ULTRA_CORE_PRELUDE` (`~2 KB` minified) is evaluated eagerly when creating a `boa_engine`
//! realm; Shards `1..=18` and `29` are registered via self-replacing lazy property getters on
//! `globalThis` (`__defLazy`) or pre-loaded on demand by `detect_needed_shards` with zero unused
//! parse/eval cost.

mod shard_detector;
mod slicebc_cache;

pub(crate) use shard_detector::{
    detect_needed_shards, detect_shards_and_boacas_flags, needs_lazy_global_getters,
};
pub(crate) use slicebc_cache::{
    compile_static_js_to_slicebc_with_min_line, eval_lazy_getters_init_prelude,
    eval_lazy_prelude_shard, eval_static_js_with_2hit_slicebc_cache, eval_ultra_core_prelude,
};

include!(concat!(env!("OUT_DIR"), "/minified_preludes.rs"));

/// Human-readable source for `ULTRA_CORE_PRELUDE` (`src/preludes/00_ultra_core.js`).
pub const ULTRA_CORE_PRELUDE_SRC: &str = include_str!("../00_ultra_core.js");
/// Human-readable source for Shard 1: `WEB_FETCH_URL_PRELUDE` (`01a_url_search_params.js` + `01b_url.js`).
pub const WEB_FETCH_URL_PRELUDE_SRC: &str = concat!(
    include_str!("../01a_url_search_params.js"),
    "\n",
    include_str!("../01b_url.js")
);
/// Human-readable source for Shard 2: `ENCODING_EVENTS_PRELUDE` (`src/preludes/02_encoding_events.js`).
pub const ENCODING_EVENTS_PRELUDE_SRC: &str = include_str!("../02_encoding_events.js");
/// Human-readable source for Shard 3: `STREAMS_FORMDATA_PRELUDE` (`src/preludes/03_streams_formdata.js`).
pub const STREAMS_FORMDATA_PRELUDE_SRC: &str = include_str!("../03_streams_formdata.js");
/// Human-readable source for Shard 4: `WEBCRYPTO_CLONE_PRELUDE` (`src/preludes/04_webcrypto_clone.js`).
pub const WEBCRYPTO_CLONE_PRELUDE_SRC: &str = include_str!("../04_webcrypto_clone.js");
/// Human-readable source for Shard 5: `FULL_BUFFER_PRELUDE` (`src/preludes/05_full_buffer.js`).
pub const FULL_BUFFER_PRELUDE_SRC: &str = include_str!("../05_full_buffer.js");
/// Human-readable source for Shard 6: `CONSOLE_EXT_PRELUDE` (`src/preludes/06_console_ext.js`).
pub const CONSOLE_EXT_PRELUDE_SRC: &str = include_str!("../06_console_ext.js");
/// Human-readable source for Shard 7: `WEBSOCKET_PRELUDE` (`src/preludes/07_websocket.js`).
pub const WEBSOCKET_PRELUDE_SRC: &str = include_str!("../07_websocket.js");
/// Human-readable source for Shard 8: `WORKERS_SOCKETS_PRELUDE` (`src/preludes/08_workers_sockets.js`).
pub const WORKERS_SOCKETS_PRELUDE_SRC: &str = include_str!("../08_workers_sockets.js");
/// Human-readable source for Shard 9: `WASM_PRELUDE` (`src/preludes/09_wasm.js`).
pub const WASM_PRELUDE_SRC: &str = include_str!("../09_wasm.js");
/// Human-readable source for Shard 10: `OUTBOUND_FETCH_SSE_PRELUDE` (`src/preludes/10_outbound_fetch_sse.js`).
pub const OUTBOUND_FETCH_SSE_PRELUDE_SRC: &str = include_str!("../10_outbound_fetch_sse.js");
/// Human-readable source for Shard 11: `FETCH_HEADERS_REQ_RES_PRELUDE` (`11a_headers_and_body.js` + `11b_request_and_response.js`).
pub const FETCH_HEADERS_REQ_RES_PRELUDE_SRC: &str = concat!(
    include_str!("../11a_headers_and_body.js"),
    "\n",
    include_str!("../11b_request_and_response.js")
);
/// Human-readable source for Shard 12: `URL_PATTERN_PRELUDE` (`src/preludes/12_url_pattern.js`).
pub const URL_PATTERN_PRELUDE_SRC: &str = include_str!("../12_url_pattern.js");
/// Human-readable source for Shard 13: `DOM_EVENTS_PRELUDE` (`src/preludes/13_dom_events.js`).
pub const DOM_EVENTS_PRELUDE_SRC: &str = include_str!("../13_dom_events.js");
/// Human-readable source for Shard 14: `TRANSFORM_COMPRESSION_STREAMS_PRELUDE` (`src/preludes/14_transform_compression_streams.js`).
pub const TRANSFORM_COMPRESSION_STREAMS_PRELUDE_SRC: &str =
    include_str!("../14_transform_compression_streams.js");
/// Human-readable source for Shard 15: `BLOB_FORMDATA_PRELUDE` (`src/preludes/15_blob_formdata.js`).
pub const BLOB_FORMDATA_PRELUDE_SRC: &str = include_str!("../15_blob_formdata.js");
/// Human-readable source for Shard 16: `STRUCTURED_CLONE_PRELUDE` (`src/preludes/16_structured_clone.js`).
pub const STRUCTURED_CLONE_PRELUDE_SRC: &str = include_str!("../16_structured_clone.js");
/// Human-readable source for Shard 17: `BUFFER_BINARY_RW_PRELUDE` (`src/preludes/17_buffer_binary_rw.js`).
pub const BUFFER_BINARY_RW_PRELUDE_SRC: &str = include_str!("../17_buffer_binary_rw.js");
/// Human-readable source for Shard 18: `INTL_PRELUDE` (`src/preludes/18_intl.js`).
pub const INTL_PRELUDE_SRC: &str = include_str!("../18_intl.js");
/// Human-readable source for Shard 29: `PROCESS_CLI_PRELUDE` (`src/preludes/19_process_cli.js`).
pub const PROCESS_CLI_PRELUDE_SRC: &str = include_str!("../19_process_cli.js");

/// On-demand self-replacing lazy property getter registration (`__defLazy`) evaluated ONLY when
/// `needs_lazy_global_getters(js_code)` is true (`globalThis.*` dynamic probes).
pub(crate) const LAZY_GETTERS_INIT_PRELUDE: &[u8] = br#"(function(){var __loadShard=globalThis.__slice_ensure_shard,__shardProps=globalThis.__shardProps,__shMask=globalThis.__shMask;globalThis.__slice_mark_lazy_inited();function __defLazy(names,shardId){__shardProps[shardId]=names;for(var i=0;i<names.length;i++){(function(prop){Object.defineProperty(globalThis,prop,{configurable:true,enumerable:false,get:function(){__loadShard(shardId);return globalThis[prop];},set:function(val){Object.defineProperty(globalThis,prop,{value:val,writable:true,enumerable:false,configurable:true});}});})(names[i]);}}if((__shMask&2)===0)__defLazy(["URLSearchParams","URL"],1);if((__shMask&2048)===0)__defLazy(["Headers","Request","Response","__slice_make_web_request"],11);if((__shMask&4096)===0)__defLazy(["URLPattern"],12);if((__shMask&1024)===0)__defLazy(["AbortSignal","AbortController","fetch"],10);if((__shMask&4)===0)__defLazy(["btoa","atob","TextEncoder","TextDecoder"],2);if((__shMask&8192)===0)__defLazy(["Event","CustomEvent","EventTarget","DOMException"],13);if((__shMask&8)===0)__defLazy(["ReadableStream"],3);if((__shMask&16384)===0)__defLazy(["WritableStream","TransformStream","TextEncoderStream","TextDecoderStream","CompressionStream","DecompressionStream"],14);if((__shMask&32768)===0)__defLazy(["Blob","File","FormData"],15);if((__shMask&16)===0)__defLazy(["crypto","CryptoKey"],4);if((__shMask&65536)===0)__defLazy(["structuredClone"],16);if((__shMask&32)===0)__defLazy(["Buffer"],5);if((__shMask&64)===0){var __cMethods=["dir","dirxml","table","trace","assert","clear","count","countReset","group","groupCollapsed","groupEnd","time","timeLog","timeEnd"];for(var c=0;c<__cMethods.length;c++){(function(m){globalThis.console[m]=function(){__loadShard(6);return globalThis.console[m].apply(globalThis.console,arguments);};})(__cMethods[c]);}}if((__shMask&128)===0)__defLazy(["WebSocket","MessageEvent","CloseEvent","__slice_dispatch_server_ws"],7);if((__shMask&256)===0)__defLazy(["Worker","MessageChannel","MessagePort","__slice_receiveMessageOnPort","__slice_wintertc_connect","__slice_init_worker_self_env","__slice_worker_host_deliver","__slice_worker_self_deliver"],8);if((__shMask&512)===0)__defLazy(["WebAssembly"],9);if((__shMask&262144)===0)__defLazy(["Intl"],18);if((__shMask&536870912)===0)__defLazy(["navigator","location"],29);})();"#;

/// Canonical topological order for loading prelude shards (`1..=18` and `29`) so base dependencies
/// (`TextEncoder`, `EventTarget`, `URL`, `Request`/`Response`, `ReadableStream`, `Buffer`) are
/// always initialized before dependent shards (`fetch`, `TransformStream`, `WebSocket`, `Worker`, `Intl`).
pub(crate) const SHARD_LOAD_ORDER: [u32; 19] = [
    2, 13, 1, 11, 12, 10, 3, 14, 15, 4, 16, 5, 17, 6, 7, 8, 9, 29, 18,
];

/// Expands cross-shard prerequisites so any fused or warm-realm batch load evaluates base classes
/// (`EventTarget` in Shard 13, `Response` in Shard 11, `ReadableStream` in Shard 3, `TextEncoder`
/// in Shard 2) before subclasses (`AbortSignal`, `WebSocket`, `MessagePort`/`Worker`, `TransformStream`).
#[inline]
pub(crate) const fn expand_shard_prerequisites(mut mask: u32) -> u32 {
    if (mask & (1 << 10)) != 0 {
        mask |= (1 << 13) | (1 << 11);
    }
    if (mask & (1 << 7)) != 0 {
        mask |= 1 << 13;
    }
    if (mask & (1 << 8)) != 0 {
        mask |= (1 << 13) | (1 << 2);
    }
    if (mask & (1 << 14)) != 0 {
        mask |= 1 << 3;
    }
    if (mask & (1 << 30)) != 0 {
        mask |= 1 << 5;
    }
    if (mask & (1 << 31)) != 0 {
        mask |= (1 << 19) | (1 << 5);
    }
    mask
}

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
    if (shards_mask & ((1 << 2) | (1 << 5) | (1 << 14) | (1 << 15) | (1 << 16) | (1 << 30))) != 0 {
        crate::host_bindings::register_wintertc_phase_n3_host_functions(context);
    }
    if (shards_mask & ((1 << 4) | (1 << 31))) != 0 {
        crate::host_bindings::register_webcrypto_host_functions(context);
    }
    if (shards_mask & ((1 << 5) | (1 << 7) | (1 << 30) | (1 << 31))) != 0 {
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

const PREFUSED_BUILTIN_BITS: [u32; 12] = [19, 31, 20, 21, 22, 23, 24, 25, 26, 27, 28, 30];

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
    let shards_mask = expand_shard_prerequisites(shards_mask);
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
    if (shards_mask & (1u32 << 31)) != 0 {
        active_mask |= 1u32 << 31;
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
    for builtin_bit in PREFUSED_BUILTIN_BITS {
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
    let shards_mask = expand_shard_prerequisites(shards_mask);
    let mut nl = ULTRA_CORE_PRELUDE.iter().filter(|&&b| b == b'\n').count();
    for shard_id in SHARD_LOAD_ORDER {
        let bit = 1u32 << shard_id;
        if (shards_mask & bit) != 0 {
            let shard_bytes = shard_prelude_bytes(shard_id);
            nl += shard_bytes.iter().filter(|&&b| b == b'\n').count();
        }
    }
    for builtin_bit in PREFUSED_BUILTIN_BITS {
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
