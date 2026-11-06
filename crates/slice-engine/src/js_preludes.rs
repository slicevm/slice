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

include!(concat!(env!("OUT_DIR"), "/minified_preludes.rs"));

/// Human-readable source for `ULTRA_CORE_PRELUDE` (`src/preludes/00_ultra_core.js`).
pub const ULTRA_CORE_PRELUDE_SRC: &str = include_str!("preludes/00_ultra_core.js");
/// Human-readable source for Shard 1: `WEB_FETCH_URL_PRELUDE` (`src/preludes/01_web_fetch_url.js`).
pub const WEB_FETCH_URL_PRELUDE_SRC: &str = include_str!("preludes/01_web_fetch_url.js");
/// Human-readable source for Shard 2: `ENCODING_EVENTS_PRELUDE` (`src/preludes/02_encoding_events.js`).
pub const ENCODING_EVENTS_PRELUDE_SRC: &str = include_str!("preludes/02_encoding_events.js");
/// Human-readable source for Shard 3: `STREAMS_FORMDATA_PRELUDE` (`src/preludes/03_streams_formdata.js`).
pub const STREAMS_FORMDATA_PRELUDE_SRC: &str = include_str!("preludes/03_streams_formdata.js");
/// Human-readable source for Shard 4: `WEBCRYPTO_CLONE_PRELUDE` (`src/preludes/04_webcrypto_clone.js`).
pub const WEBCRYPTO_CLONE_PRELUDE_SRC: &str = include_str!("preludes/04_webcrypto_clone.js");
/// Human-readable source for Shard 5: `FULL_BUFFER_PRELUDE` (`src/preludes/05_full_buffer.js`).
pub const FULL_BUFFER_PRELUDE_SRC: &str = include_str!("preludes/05_full_buffer.js");
/// Human-readable source for Shard 6: `CONSOLE_EXT_PRELUDE` (`src/preludes/06_console_ext.js`).
pub const CONSOLE_EXT_PRELUDE_SRC: &str = include_str!("preludes/06_console_ext.js");
/// Human-readable source for Shard 7: `WEBSOCKET_PRELUDE` (`src/preludes/07_websocket.js`).
pub const WEBSOCKET_PRELUDE_SRC: &str = include_str!("preludes/07_websocket.js");
/// Human-readable source for Shard 8: `WORKERS_SOCKETS_PRELUDE` (`src/preludes/08_workers_sockets.js`).
pub const WORKERS_SOCKETS_PRELUDE_SRC: &str = include_str!("preludes/08_workers_sockets.js");
/// Human-readable source for Shard 9: `WASM_PRELUDE` (`src/preludes/09_wasm.js`).
pub const WASM_PRELUDE_SRC: &str = include_str!("preludes/09_wasm.js");
/// Human-readable source for Shard 10: `OUTBOUND_FETCH_SSE_PRELUDE` (`src/preludes/10_outbound_fetch_sse.js`).
pub const OUTBOUND_FETCH_SSE_PRELUDE_SRC: &str = include_str!("preludes/10_outbound_fetch_sse.js");
/// Human-readable source for Shard 11: `FETCH_HEADERS_REQ_RES_PRELUDE` (`src/preludes/11_fetch_headers_req_res.js`).
pub const FETCH_HEADERS_REQ_RES_PRELUDE_SRC: &str =
    include_str!("preludes/11_fetch_headers_req_res.js");
/// Human-readable source for Shard 12: `URL_PATTERN_PRELUDE` (`src/preludes/12_url_pattern.js`).
pub const URL_PATTERN_PRELUDE_SRC: &str = include_str!("preludes/12_url_pattern.js");
/// Human-readable source for Shard 13: `DOM_EVENTS_PRELUDE` (`src/preludes/13_dom_events.js`).
pub const DOM_EVENTS_PRELUDE_SRC: &str = include_str!("preludes/13_dom_events.js");
/// Human-readable source for Shard 14: `TRANSFORM_COMPRESSION_STREAMS_PRELUDE` (`src/preludes/14_transform_compression_streams.js`).
pub const TRANSFORM_COMPRESSION_STREAMS_PRELUDE_SRC: &str =
    include_str!("preludes/14_transform_compression_streams.js");
/// Human-readable source for Shard 15: `BLOB_FORMDATA_PRELUDE` (`src/preludes/15_blob_formdata.js`).
pub const BLOB_FORMDATA_PRELUDE_SRC: &str = include_str!("preludes/15_blob_formdata.js");
/// Human-readable source for Shard 16: `STRUCTURED_CLONE_PRELUDE` (`src/preludes/16_structured_clone.js`).
pub const STRUCTURED_CLONE_PRELUDE_SRC: &str = include_str!("preludes/16_structured_clone.js");
/// Human-readable source for Shard 17: `BUFFER_BINARY_RW_PRELUDE` (`src/preludes/17_buffer_binary_rw.js`).
pub const BUFFER_BINARY_RW_PRELUDE_SRC: &str = include_str!("preludes/17_buffer_binary_rw.js");

/// On-demand self-replacing lazy property getter registration (`__defLazy`) evaluated ONLY when
/// `needs_lazy_global_getters(js_code)` is true (`globalThis.*` dynamic probes).
pub(crate) const LAZY_GETTERS_INIT_PRELUDE: &[u8] = br#"(function(){var __loadShard=globalThis.__slice_ensure_shard,__shardProps=globalThis.__shardProps,__shMask=globalThis.__shMask;globalThis.__slice_mark_lazy_inited();function __defLazy(names,shardId){__shardProps[shardId]=names;for(var i=0;i<names.length;i++){(function(prop){Object.defineProperty(globalThis,prop,{configurable:true,enumerable:false,get:function(){__loadShard(shardId);return globalThis[prop];},set:function(val){Object.defineProperty(globalThis,prop,{value:val,writable:true,enumerable:false,configurable:true});}});})(names[i]);}}if((__shMask&2)===0)__defLazy(["URLSearchParams","URL"],1);if((__shMask&2048)===0)__defLazy(["Headers","Request","Response","__slice_make_web_request"],11);if((__shMask&4096)===0)__defLazy(["URLPattern"],12);if((__shMask&1024)===0)__defLazy(["AbortSignal","AbortController","fetch"],10);if((__shMask&4)===0)__defLazy(["btoa","atob","TextEncoder","TextDecoder"],2);if((__shMask&8192)===0)__defLazy(["Event","CustomEvent","EventTarget"],13);if((__shMask&8)===0)__defLazy(["ReadableStream"],3);if((__shMask&16384)===0)__defLazy(["WritableStream","TransformStream","TextEncoderStream","TextDecoderStream","CompressionStream","DecompressionStream"],14);if((__shMask&32768)===0)__defLazy(["Blob","File","FormData"],15);if((__shMask&16)===0)__defLazy(["crypto","CryptoKey"],4);if((__shMask&65536)===0)__defLazy(["structuredClone"],16);if((__shMask&32)===0)__defLazy(["Buffer"],5);if((__shMask&64)===0){var __cMethods=["dir","dirxml","table","trace","assert","clear","count","countReset","group","groupCollapsed","groupEnd","time","timeLog","timeEnd"];for(var c=0;c<__cMethods.length;c++){(function(m){globalThis.console[m]=function(){__loadShard(6);return globalThis.console[m].apply(globalThis.console,arguments);};})(__cMethods[c]);}}if((__shMask&128)===0)__defLazy(["WebSocket","MessageEvent","CloseEvent","__slice_dispatch_server_ws"],7);if((__shMask&256)===0)__defLazy(["Worker","MessageChannel","MessagePort","__slice_receiveMessageOnPort","__slice_wintertc_connect","__slice_init_worker_self_env","__slice_worker_host_deliver","__slice_worker_self_deliver"],8);if((__shMask&512)===0)__defLazy(["WebAssembly"],9);})();"#;

/// Evaluates a compile-time minified lazy prelude shard (`1..=17`) on demand inside `context`.
#[inline]
pub(crate) fn eval_lazy_prelude_shard(
    shard_id: u32,
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<boa_engine::JsValue> {
    let bytes: &[u8] = match shard_id {
        1 => {
            crate::web_primitives::register_url_and_usp_host_functions(context);
            WEB_FETCH_URL_PRELUDE
        }
        2 => {
            crate::host_bindings::register_wintertc_phase_n3_host_functions(context);
            ENCODING_EVENTS_PRELUDE
        }
        3 => STREAMS_FORMDATA_PRELUDE,
        4 => {
            crate::host_bindings::register_webcrypto_host_functions(context);
            WEBCRYPTO_CLONE_PRELUDE
        }
        5 => {
            crate::host_bindings::register_wintertc_phase_n3_host_functions(context);
            crate::host_bindings::register_buffer_host_functions(context);
            FULL_BUFFER_PRELUDE
        }
        6 => CONSOLE_EXT_PRELUDE,
        7 => {
            crate::host_bindings::register_buffer_host_functions(context);
            crate::host_bindings::register_shard7_websocket_host_functions(context);
            WEBSOCKET_PRELUDE
        }
        8 => {
            crate::host_bindings::register_shard8_workers_sockets_host_functions(context);
            WORKERS_SOCKETS_PRELUDE
        }
        9 => {
            crate::host_bindings::register_shard9_wasm_host_functions(context);
            WASM_PRELUDE
        }
        10 => {
            crate::host_bindings::register_shard10_outbound_fetch_host_functions(context);
            OUTBOUND_FETCH_SSE_PRELUDE
        }
        11 => {
            crate::host_bindings::register_http_bridge_host_functions(context);
            FETCH_HEADERS_REQ_RES_PRELUDE
        }
        12 => URL_PATTERN_PRELUDE,
        13 => DOM_EVENTS_PRELUDE,
        14 => {
            crate::host_bindings::register_wintertc_phase_n3_host_functions(context);
            TRANSFORM_COMPRESSION_STREAMS_PRELUDE
        }
        15 => {
            crate::host_bindings::register_wintertc_phase_n3_host_functions(context);
            BLOB_FORMDATA_PRELUDE
        }
        16 => STRUCTURED_CLONE_PRELUDE,
        17 => BUFFER_BINARY_RW_PRELUDE,
        _ => return Ok(boa_engine::JsValue::undefined()),
    };
    context.eval(boa_engine::Source::from_bytes(bytes))
}

#[inline]
fn has_outbound_fetch_call(js_code: &str) -> bool {
    let bytes = js_code.as_bytes();
    let mut pos = 0;
    while let Some(rel) = js_code[pos..].find("fetch(") {
        let idx = pos + rel;
        let is_member = idx > 0 && bytes[idx - 1] == b'.';
        let is_async_method = idx >= 6 && &bytes[idx - 6..idx] == b"async ";
        let is_ident_suffix =
            idx > 0 && (bytes[idx - 1].is_ascii_alphanumeric() || bytes[idx - 1] == b'_');
        let is_sync_method = js_code[idx + 6..]
            .find(')')
            .is_some_and(|c| js_code[idx + 6 + c + 1..].trim_start().starts_with('{'));
        if !is_member && !is_async_method && !is_ident_suffix && !is_sync_method {
            return true;
        }
        pos = idx + 6;
    }
    false
}

/// Computes a bitmask of lazy prelude shards (`1..=17`) referenced by `js_code` so top-level
/// execution can pre-load only the shards actually used by a script before entering async/generator
/// frames, while scripts that don't use those globals pay 0 bytes of prelude evaluation cost.
#[inline]
pub(crate) fn detect_needed_shards(raw_js_code: &str, is_http: bool) -> u32 {
    let active_storage: String;
    let js_code: &str = if raw_js_code.contains("] = \"") || raw_js_code.contains("]=\"") {
        active_storage = raw_js_code
            .lines()
            .filter(|l| {
                !(l.starts_with("__slice_modules[") && (l.contains("] = \"") || l.contains("]=\"")))
            })
            .collect::<Vec<_>>()
            .join("\n");
        &active_storage
    } else {
        raw_js_code
    };
    let mut mask: u32 = 0;

    let needs_outbound_fetch_sse = has_outbound_fetch_call(js_code)
        || js_code.contains("fetchSync")
        || js_code.contains("mockFetch")
        || js_code.contains("streamSSE")
        || js_code.contains("Response.sse")
        || js_code.contains("AbortController")
        || js_code.contains("AbortSignal");
    if needs_outbound_fetch_sse {
        mask |= (1 << 10) | (1 << 11);
    } else if is_http
        || js_code.contains(".fetch(")
        || js_code.contains("new Request(")
        || js_code.contains("new Response(")
        || js_code.contains("Response.")
        || js_code.contains("new Headers(")
        || js_code.contains("Headers.")
        || js_code.contains("Request.")
    {
        mask |= 1 << 11;
    }

    if js_code.contains("new URL(")
        || js_code.contains("URL.")
        || js_code.contains("new URLSearchParams(")
        || js_code.contains("URLSearchParams.")
        || js_code.contains("node:url")
        || js_code.contains("createObjectURL")
        || js_code.contains("canParse")
    {
        mask |= 1 << 1;
    }

    if js_code.contains("URLPattern") {
        mask |= 1 << 12;
    }

    if js_code.contains("TextEncoder")
        || js_code.contains("TextDecoder")
        || js_code.contains("btoa(")
        || js_code.contains("atob(")
    {
        mask |= 1 << 2;
    }

    if js_code.contains("new Event(")
        || js_code.contains("new CustomEvent(")
        || js_code.contains("EventTarget")
    {
        mask |= 1 << 13;
    }

    if js_code.contains("WritableStream")
        || js_code.contains("TransformStream")
        || js_code.contains("TextEncoderStream")
        || js_code.contains("TextDecoderStream")
        || js_code.contains("CompressionStream")
        || js_code.contains("DecompressionStream")
    {
        mask |= (1 << 14) | (1 << 3);
    } else if js_code.contains("ReadableStream")
        || js_code.contains("Response.sse")
        || js_code.contains("streamSSE")
    {
        mask |= 1 << 3;
    }

    if js_code.contains("new Blob(")
        || js_code.contains("new File(")
        || js_code.contains("new FormData(")
        || js_code.contains("FormData.")
        || js_code.contains("Blob.")
        || js_code.contains("File.")
    {
        mask |= 1 << 15;
    }

    if js_code.contains("subtle")
        || js_code.contains("getRandomValues")
        || js_code.contains("randomUUID")
        || js_code.contains("webcrypto")
        || js_code.contains("CryptoKey")
        || js_code.contains("crypto.")
    {
        mask |= 1 << 4;
    }

    if js_code.contains("structuredClone") {
        mask |= 1 << 16;
    }

    let needs_buf_bin = js_code.contains("writeUInt")
        || js_code.contains("readUInt")
        || js_code.contains("writeInt")
        || js_code.contains("readInt")
        || js_code.contains("writeBig")
        || js_code.contains("readBig");
    if needs_buf_bin
        || js_code.contains("Buffer.from")
        || js_code.contains("Buffer.alloc")
        || js_code.contains("Buffer.byteLength")
        || js_code.contains("Buffer.isBuffer")
        || js_code.contains("Buffer.isEncoding")
        || js_code.contains("Buffer(")
        || js_code.contains("Buffer.concat")
        || js_code.contains("Buffer.compare")
        || js_code.contains("new Buffer(")
        || js_code.contains("node:buffer")
        || js_code.contains("instanceof Buffer")
        || js_code.contains("randomBytes")
        || js_code.contains("createHash(\"sha512\"")
        || js_code.contains("createHash('sha512'")
        || js_code.contains("createHash(\"sha384\"")
        || js_code.contains("createHash('sha384'")
        || js_code.contains("createHash(\"sha1\"")
        || js_code.contains("createHash('sha1'")
        || js_code.contains("createHmac(\"sha512\"")
        || js_code.contains("createHmac('sha512'")
    {
        mask |= 1 << 5;
        if needs_buf_bin && !js_code.contains("_hasBinRW") {
            mask |= 1 << 17;
        }
    }

    if js_code.contains("console.table")
        || js_code.contains("console.dir")
        || js_code.contains("console.group")
        || js_code.contains("console.time")
        || js_code.contains("console.trace")
        || js_code.contains("console.count")
        || js_code.contains("console.assert")
        || js_code.contains("console.clear")
    {
        mask |= 1 << 6;
    }

    if js_code.contains("WebSocket")
        || js_code.contains("MessageEvent")
        || js_code.contains("CloseEvent")
        || js_code.contains("mockWebSocket")
        || js_code.contains("onWebSocket")
        || js_code.contains("upgradeWebSocket")
    {
        mask |= (1 << 7) | (1 << 13);
    }

    if js_code.contains("cloudflare:sockets")
        || js_code.contains("slice:sockets")
        || js_code.contains("Slice.connect")
    {
        mask |= (1 << 8) | (1 << 14) | (1 << 3) | (1 << 13) | (1 << 2);
    } else if js_code.contains("Worker")
        || js_code.contains("worker_threads")
        || js_code.contains("MessageChannel")
        || js_code.contains("MessagePort")
        || js_code.contains("parentPort")
        || js_code.contains("workerData")
        || js_code.contains("mockSocket")
    {
        mask |= (1 << 8) | (1 << 13) | (1 << 2);
    }

    if js_code.contains("WebAssembly") {
        mask |= 1 << 9;
    }
    mask
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
