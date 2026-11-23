//! Single-Pass Aho-Corasick Prelude Shard & `BoaCAS` Feature-Flag Detector.

use super::expand_shard_prerequisites;

#[inline]
fn is_inside_quoted_string_on_line(bytes: &[u8], idx: usize) -> bool {
    let line_start = memchr::memrchr(b'\n', &bytes[..idx]).map_or(0, |i| i + 1);
    let mut i = line_start;
    let mut quote: u8 = 0;
    while i < idx {
        let b = bytes[i];
        if quote != 0 {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == quote {
                quote = 0;
            }
        } else if b == b'"' || b == b'\'' {
            quote = b;
        } else if b == b'/' && i + 1 < idx && bytes[i + 1] == b'/' {
            return true;
        }
        i += 1;
    }
    quote != 0
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
    !is_member
        && !is_async_method
        && !is_ident_suffix
        && !is_sync_method
        && !is_inside_quoted_string_on_line(bytes, idx)
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
    CryptoPki,
    CryptoPkiAndBuf,
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
    ("pbkdf2", ShardTriggerAction::CryptoPkiAndBuf),
    ("scrypt", ShardTriggerAction::CryptoPkiAndBuf),
    ("hkdf", ShardTriggerAction::CryptoPki),
    ("KeyObject", ShardTriggerAction::CryptoPki),
    ("createCipheriv", ShardTriggerAction::CryptoPkiAndBuf),
    ("createDecipheriv", ShardTriggerAction::CryptoPkiAndBuf),
    ("generateKeyPair", ShardTriggerAction::CryptoPkiAndBuf),
    ("diffieHellman", ShardTriggerAction::CryptoPkiAndBuf),
    ("createPrivateKey", ShardTriggerAction::CryptoPki),
    ("createPublicKey", ShardTriggerAction::CryptoPki),
    ("createSecretKey", ShardTriggerAction::CryptoPkiAndBuf),
    ("createSign", ShardTriggerAction::CryptoPkiAndBuf),
    ("createVerify", ShardTriggerAction::CryptoPkiAndBuf),
    ("crypto.sign(", ShardTriggerAction::CryptoPkiAndBuf),
    ("crypto.verify(", ShardTriggerAction::CryptoPki),
    ("-----BEGIN ", ShardTriggerAction::CryptoPki),
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
    // Shard 18: WinterTC `Intl` (`1 << 18`); `toLocale*` / `localeCompare` lazy-load Shard 18 on first call via `00_ultra_core.js`
    ("Intl", ShardTriggerAction::Mask(1 << 18)),
    // Shard 29: `navigator`, `location`, & Complete `process` CLI Surface (`1 << 29`)
    ("navigator", ShardTriggerAction::Mask(1 << 29)),
    ("location.", ShardTriggerAction::Mask(1 << 29)),
    ("globalThis.location", ShardTriggerAction::Mask(1 << 29)),
    ("process.stdout", ShardTriggerAction::Mask(1 << 29)),
    ("process.stderr", ShardTriggerAction::Mask(1 << 29)),
    ("process.stdin", ShardTriggerAction::Mask(1 << 29)),
    ("stdout.write", ShardTriggerAction::Mask(1 << 29)),
    ("stderr.write", ShardTriggerAction::Mask(1 << 29)),
    ("stdin.on", ShardTriggerAction::Mask(1 << 29)),
    ("stdin.read", ShardTriggerAction::Mask(1 << 29)),
    ("{ stdout", ShardTriggerAction::Mask(1 << 29)),
    ("{stdout", ShardTriggerAction::Mask(1 << 29)),
    (", stdout", ShardTriggerAction::Mask(1 << 29)),
    (",stdout", ShardTriggerAction::Mask(1 << 29)),
    ("{ stderr", ShardTriggerAction::Mask(1 << 29)),
    ("{stderr", ShardTriggerAction::Mask(1 << 29)),
    (", stderr", ShardTriggerAction::Mask(1 << 29)),
    (",stderr", ShardTriggerAction::Mask(1 << 29)),
    ("{ stdin", ShardTriggerAction::Mask(1 << 29)),
    ("{stdin", ShardTriggerAction::Mask(1 << 29)),
    (", stdin", ShardTriggerAction::Mask(1 << 29)),
    (",stdin", ShardTriggerAction::Mask(1 << 29)),
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
    ("node:zlib", ShardTriggerAction::BuiltinMask(1 << 30)),
    ("\"zlib\"", ShardTriggerAction::BuiltinMask(1 << 30)),
    ("'zlib'", ShardTriggerAction::BuiltinMask(1 << 30)),
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
        "defineProperties",
        ShardTriggerAction::Flags(crate::boacas::BOACAS_FLAG_NEEDS_SES_CHECK),
    ),
    (
        "setPrototypeOf",
        ShardTriggerAction::Flags(crate::boacas::BOACAS_FLAG_NEEDS_SES_CHECK),
    ),
    (
        "getPrototypeOf",
        ShardTriggerAction::Flags(crate::boacas::BOACAS_FLAG_NEEDS_SES_CHECK),
    ),
    (
        "constructor",
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
    let mut has_crypto_pki = false;

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
            ShardTriggerAction::CryptoPki => {
                has_crypto_pki = true;
            }
            ShardTriggerAction::CryptoPkiAndBuf => {
                mask |= 1 << 5;
                has_crypto_pki = true;
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

    if has_crypto_pki && (mask & (1 << 19)) != 0 {
        mask |= 1 << 31;
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

    (flags, expand_shard_prerequisites(mask))
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
