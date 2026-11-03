//! WinterTC, WHATWG, Node.js `Buffer`, Event-Loop Microtask/Immediate, `console.*`, and `WebSocket`
//! Micro-Preludes.
//!
//! All JavaScript preludes are maintained as clean, readable, multi-line `.js` source files under
//! `crates/slice-engine/src/preludes/*.js` and automatically minified at compile time by
//! `crates/slice-engine/build.rs` into `$OUT_DIR/minified_preludes.rs`.
//!
//! Only `ULTRA_CORE_PRELUDE` (`~2 KB` minified) is evaluated eagerly when creating a `boa_engine`
//! realm; Shards `1..=7` are registered via self-replacing lazy property getters on `globalThis`
//! (`__defLazy`) and evaluated on first access with zero subsequent getter overhead.

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

/// Evaluates a compile-time minified lazy prelude shard (`1..=8`) on demand inside `context`.
#[inline]
pub(crate) fn eval_lazy_prelude_shard(
    shard_id: u32,
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<boa_engine::JsValue> {
    let bytes: &[u8] = match shard_id {
        1 => WEB_FETCH_URL_PRELUDE,
        2 => ENCODING_EVENTS_PRELUDE,
        3 => STREAMS_FORMDATA_PRELUDE,
        4 => WEBCRYPTO_CLONE_PRELUDE,
        5 => FULL_BUFFER_PRELUDE,
        6 => CONSOLE_EXT_PRELUDE,
        7 => WEBSOCKET_PRELUDE,
        8 => WORKERS_SOCKETS_PRELUDE,
        _ => return Ok(boa_engine::JsValue::undefined()),
    };
    context.eval(boa_engine::Source::from_bytes(bytes))
}

/// Computes a bitmask of lazy prelude shards (`1..=8`) referenced by `js_code` so top-level
/// execution can pre-load only the shards actually used by a script before entering async/generator
/// frames, while scripts that don't use those globals pay 0 bytes of prelude evaluation cost.
#[inline]
pub(crate) fn detect_needed_shards(js_code: &str, is_http: bool) -> u16 {
    let mut mask: u16 = 0;
    if is_http
        || js_code.contains("fetch(")
        || js_code.contains("fetchSync")
        || js_code.contains("mockFetch")
        || js_code.contains("streamSSE")
        || js_code.contains("new URL(")
        || js_code.contains("new URLSearchParams(")
        || js_code.contains("URLSearchParams.")
        || js_code.contains("node:url")
        || js_code.contains("new Request(")
        || js_code.contains("new Response(")
        || js_code.contains("Response.")
        || js_code.contains("new Headers(")
        || js_code.contains("AbortController")
        || js_code.contains("AbortSignal")
    {
        mask |= (1 << 1) | (1 << 2);
    }
    if js_code.contains("TextEncoder")
        || js_code.contains("TextDecoder")
        || js_code.contains("btoa(")
        || js_code.contains("atob(")
        || js_code.contains("new Event(")
        || js_code.contains("new CustomEvent(")
        || js_code.contains("EventTarget")
    {
        mask |= 1 << 2;
    }
    if js_code.contains("ReadableStream")
        || js_code.contains("WritableStream")
        || js_code.contains("TransformStream")
        || js_code.contains("TextEncoderStream")
        || js_code.contains("TextDecoderStream")
        || js_code.contains("new Blob(")
        || js_code.contains("new File(")
        || js_code.contains("new FormData(")
        || js_code.contains("Response.sse")
        || js_code.contains("streamSSE")
        || js_code.contains(".formData(")
        || js_code.contains(".blob(")
    {
        mask |= (1 << 3) | (1 << 2);
    }
    if js_code.contains("subtle")
        || js_code.contains("CryptoKey")
        || js_code.contains("structuredClone")
        || js_code.contains("getRandomValues")
        || js_code.contains("randomUUID")
        || js_code.contains("webcrypto")
        || js_code.contains("timingSafeEqual")
    {
        mask |= 1 << 4;
    }
    if js_code.contains("Buffer.")
        || js_code.contains("Buffer(")
        || js_code.contains("new Buffer(")
        || js_code.contains("node:buffer")
        || js_code.contains("randomBytes")
        || js_code.contains("createHash(\"sha512\"")
        || js_code.contains("createHash('sha512'")
        || js_code.contains("createHash(\"sha384\"")
        || js_code.contains("createHash('sha384'")
        || js_code.contains("createHash(\"sha1\"")
        || js_code.contains("createHash('sha1'")
    {
        mask |= 1 << 5;
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
        mask |= (1 << 7) | (1 << 2);
    }
    if js_code.contains("Worker")
        || js_code.contains("worker_threads")
        || js_code.contains("MessageChannel")
        || js_code.contains("MessagePort")
        || js_code.contains("parentPort")
        || js_code.contains("workerData")
        || js_code.contains("cloudflare:sockets")
        || js_code.contains("slice:sockets")
        || js_code.contains("Slice.connect")
        || js_code.contains("mockSocket")
        || js_code.contains("createObjectURL")
        || js_code.contains("canParse")
    {
        mask |= (1 << 8) | (1 << 3) | (1 << 2) | (1 << 1);
    }
    mask
}
