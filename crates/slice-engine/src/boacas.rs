//! Experimental Precompiled Boa CAS Container (`SLICEBC\x01`, `--exper-boacas`).
//!
//! Provides the shared wire format, build-time feature/shard metadata precomputation,
//! runtime experiment flag controls (`--exper-boacas` / `--no-exper-boacas` / `SLICE_EXPER_BOACAS`),
//! and execution hooks for the `EXPER_BOACAS` architecture.

use crate::boa_realm::{ensure_realm_shards, WarmBoaRealm, BOA_VIOLATION};
use crate::js_preludes::{detect_needed_shards, needs_lazy_global_getters};
use crate::parser_and_compiler::extract_slice_rc_key;
use std::sync::atomic::{AtomicU8, Ordering};

/// 8-byte magic header identifying an experimental `SLICEBC\x01` precompiled Boa container.
pub const BOACAS_MAGIC: &[u8; 8] = b"SLICEBC\x01";

/// Fixed byte length of [`BoaCasHeader`] (`32` bytes).
pub const BOACAS_HEADER_LEN: usize = 32;

/// Precomputed feature flag: bundle references `Worker` or `worker_threads`.
pub const BOACAS_FLAG_HAS_WORKER: u16 = 1 << 0;
/// Precomputed feature flag: bundle references `nextTick`.
pub const BOACAS_FLAG_HAS_NEXT_TICK: u16 = 1 << 1;
/// Precomputed feature flag: bundle references `Slice.request`, `Slice.respond`, or `__slice_http_`.
pub const BOACAS_FLAG_HAS_HTTP_BRIDGE: u16 = 1 << 2;
/// Precomputed feature flag: bundle references `__slice_semver_` or `__slice_fast_`.
pub const BOACAS_FLAG_HAS_SEMVER_FAST: u16 = 1 << 3;
/// Precomputed feature flag: bundle references `Slice.kv`, `Slice.checkpoint`, `Slice.restore`, or `Slice.evalSandbox`.
pub const BOACAS_FLAG_HAS_KV_SANDBOX: u16 = 1 << 4;
/// Precomputed feature flag: bundle requires `LAZY_GETTERS_INIT_PRELUDE` (`needs_lazy_global_getters`).
pub const BOACAS_FLAG_NEEDS_LAZY_GETTERS: u16 = 1 << 5;
/// Precomputed feature flag: bundle references `prototype`, `__proto__`, `Reflect`, or `Object[`.
pub const BOACAS_FLAG_NEEDS_SES_CHECK: u16 = 1 << 6;
/// Precomputed feature flag: bundle is a multi-module or single-module `//__SLICE_RC:` cached bundle.
pub const BOACAS_FLAG_IS_RC_BUNDLE: u16 = 1 << 7;
/// Precomputed feature flag: `fallback_js` starts with `{`.
pub const BOACAS_FLAG_STARTS_WITH_BRACE: u16 = 1 << 8;

/// Process-wide override set via CLI flags (`--exper-boacas` -> `1`, `--no-exper-boacas` -> `2`, unset -> `0`).
static EXPER_BOACAS_OVERRIDE: AtomicU8 = AtomicU8::new(0);

thread_local! {
    static THREAD_EXPER_BOACAS_OVERRIDE: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

/// Sets or clears the process-wide `--exper-boacas` / `--no-exper-boacas` override.
#[inline]
pub fn set_exper_boacas_override(enabled: Option<bool>) {
    let code = match enabled {
        None => 0,
        Some(true) => 1,
        Some(false) => 2,
    };
    EXPER_BOACAS_OVERRIDE.store(code, Ordering::Relaxed);
}

/// Sets or clears a thread-local `--exper-boacas` / `--no-exper-boacas` override (safe for parallel tests).
#[inline]
pub fn set_thread_exper_boacas_override(enabled: Option<bool>) {
    let code = match enabled {
        None => 0,
        Some(true) => 1,
        Some(false) => 2,
    };
    THREAD_EXPER_BOACAS_OVERRIDE.with(|c| c.set(code));
}

/// Returns `true` when `slice pack` / `slice compile` should emit `SLICEBC\x01` precompiled containers
/// instead of raw `SLICEBOA` source blobs.
#[inline]
pub fn is_exper_boacas_build_enabled() -> bool {
    let tl = THREAD_EXPER_BOACAS_OVERRIDE.with(|c| c.get());
    if tl == 1 {
        return true;
    } else if tl == 2 {
        return false;
    }
    match EXPER_BOACAS_OVERRIDE.load(Ordering::Relaxed) {
        1 => return true,
        2 => return false,
        _ => {}
    }
    if let Ok(val) = std::env::var("SLICE_EXPER_BOACAS") {
        let norm = val.trim();
        return matches!(norm, "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON");
    }
    false
}

/// Returns `true` when `slice run` should execute the precompiled fast-path inside a `SLICEBC\x01`
/// archive (enabled by default whenever a `SLICEBC\x01` archive is loaded, unless explicitly
/// disabled via `--no-exper-boacas` or `SLICE_EXPER_BOACAS=0`).
#[inline]
pub fn is_exper_boacas_runtime_enabled() -> bool {
    let tl = THREAD_EXPER_BOACAS_OVERRIDE.with(|c| c.get());
    if tl == 1 {
        return true;
    } else if tl == 2 {
        return false;
    }
    match EXPER_BOACAS_OVERRIDE.load(Ordering::Relaxed) {
        1 => return true,
        2 => return false,
        _ => {}
    }
    if let Ok(val) = std::env::var("SLICE_EXPER_BOACAS") {
        let norm = val.trim();
        if matches!(norm, "0" | "false" | "FALSE" | "no" | "NO" | "off" | "OFF") {
            return false;
        }
    }
    true
}

/// 32-byte binary header at the start of a `SLICEBC\x01` bytecode blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoaCasHeader {
    pub approach_kind: u8,
    pub feature_flags: u16,
    pub shards_mask: u32,
    pub rc_key: u64,
    pub precompiled_len: u32,
    pub fallback_js_len: u32,
}

impl BoaCasHeader {
    /// Encodes this 32-byte header into `dst`.
    #[inline]
    pub fn encode_into(&self, dst: &mut Vec<u8>) {
        dst.extend_from_slice(BOACAS_MAGIC);
        dst.push(self.approach_kind);
        dst.push(0u8); // reserved
        dst.extend_from_slice(&self.feature_flags.to_le_bytes());
        dst.extend_from_slice(&self.shards_mask.to_le_bytes());
        dst.extend_from_slice(&self.rc_key.to_le_bytes());
        dst.extend_from_slice(&self.precompiled_len.to_le_bytes());
        dst.extend_from_slice(&self.fallback_js_len.to_le_bytes());
    }

    /// Decodes a `SLICEBC\x01` buffer into `(BoaCasHeader, precompiled_slice, fallback_js_str)`.
    #[inline]
    pub fn decode(bytecode: &[u8]) -> Option<(Self, &[u8], &str)> {
        if bytecode.len() < BOACAS_HEADER_LEN || &bytecode[0..8] != BOACAS_MAGIC {
            return None;
        }
        let approach_kind = bytecode[8];
        let feature_flags = u16::from_le_bytes([bytecode[10], bytecode[11]]);
        let shards_mask =
            u32::from_le_bytes([bytecode[12], bytecode[13], bytecode[14], bytecode[15]]);
        let rc_key = u64::from_le_bytes([
            bytecode[16],
            bytecode[17],
            bytecode[18],
            bytecode[19],
            bytecode[20],
            bytecode[21],
            bytecode[22],
            bytecode[23],
        ]);
        let precompiled_len =
            u32::from_le_bytes([bytecode[24], bytecode[25], bytecode[26], bytecode[27]]) as usize;
        let fallback_js_len =
            u32::from_le_bytes([bytecode[28], bytecode[29], bytecode[30], bytecode[31]]) as usize;

        let pre_end = BOACAS_HEADER_LEN.checked_add(precompiled_len)?;
        let total_end = pre_end.checked_add(fallback_js_len)?;
        if total_end > bytecode.len() {
            return None;
        }
        let precompiled_slice = &bytecode[BOACAS_HEADER_LEN..pre_end];
        let fallback_js = std::str::from_utf8(&bytecode[pre_end..total_end]).ok()?;
        Some((
            Self {
                approach_kind,
                feature_flags,
                shards_mask,
                rc_key,
                precompiled_len: precompiled_len as u32,
                fallback_js_len: fallback_js_len as u32,
            },
            precompiled_slice,
            fallback_js,
        ))
    }
}

/// Computes `(feature_flags, shards_mask, rc_key)` once at build time so cold runtime execution
/// performs zero linear substring scans or Aho-Corasick passes over `js_code`.
pub fn compute_boacas_metadata(js_code: &str) -> (u16, u32, u64) {
    let mut flags: u16 = 0;
    if js_code.contains("Worker") || js_code.contains("worker_threads") {
        flags |= BOACAS_FLAG_HAS_WORKER;
    }
    if js_code.contains("nextTick") {
        flags |= BOACAS_FLAG_HAS_NEXT_TICK;
    }
    if js_code.contains("Slice.request")
        || js_code.contains("Slice.respond")
        || js_code.contains("__slice_http_")
    {
        flags |= BOACAS_FLAG_HAS_HTTP_BRIDGE;
    }
    if js_code.contains("__slice_semver_") || js_code.contains("__slice_fast_") {
        flags |= BOACAS_FLAG_HAS_SEMVER_FAST;
    }
    if js_code.contains("Slice.kv")
        || js_code.contains("Slice.checkpoint")
        || js_code.contains("Slice.restore")
        || js_code.contains("Slice.evalSandbox")
    {
        flags |= BOACAS_FLAG_HAS_KV_SANDBOX;
    }
    if needs_lazy_global_getters(js_code) {
        flags |= BOACAS_FLAG_NEEDS_LAZY_GETTERS;
    }
    if js_code.contains("prototype")
        || js_code.contains("__proto__")
        || js_code.contains("Reflect")
        || js_code.contains("Object[")
    {
        flags |= BOACAS_FLAG_NEEDS_SES_CHECK;
    }
    let rc_key = if let Some((k, _)) = extract_slice_rc_key(js_code) {
        flags |= BOACAS_FLAG_IS_RC_BUNDLE;
        k
    } else {
        0
    };
    if js_code.starts_with('{') {
        flags |= BOACAS_FLAG_STARTS_WITH_BRACE;
    }
    let shards_mask = detect_needed_shards(js_code, false);
    (flags, shards_mask, rc_key)
}

/// Packs a compacted JavaScript bundle into a `SLICEBC\x01` binary container.
///
/// On `main` (`approach_kind = 0`), this embeds the precomputed 32-byte `BoaCasHeader`
/// (`feature_flags`, `shards_mask`, `rc_key`) with an empty `precompiled` section and
/// the `{\n<compacted_js>\n}` fallback payload. Experimental branches (`A`, `B`, `C`)
/// populate the branch-specific `precompiled` payload.
pub fn pack_boacas_container(compacted_js: &str) -> Vec<u8> {
    let mut fallback_js = String::with_capacity(compacted_js.len() + 4);
    fallback_js.push_str("{\n");
    fallback_js.push_str(compacted_js.trim_end());
    fallback_js.push_str("\n}");

    let (feature_flags, shards_mask, rc_key) = compute_boacas_metadata(&fallback_js);
    let precompiled: &[u8] = &[];

    let header = BoaCasHeader {
        approach_kind: 0,
        feature_flags,
        shards_mask,
        rc_key,
        precompiled_len: precompiled.len() as u32,
        fallback_js_len: fallback_js.len() as u32,
    };

    let mut out = Vec::with_capacity(BOACAS_HEADER_LEN + precompiled.len() + fallback_js.len());
    header.encode_into(&mut out);
    out.extend_from_slice(precompiled);
    out.extend_from_slice(fallback_js.as_bytes());
    out
}

/// Evaluates a decoded `SLICEBC\x01` payload on `realm` using the precomputed `BoaCasHeader`
/// metadata (skipping all 18 runtime substring scans and runtime Aho-Corasick shard detection).
pub(crate) fn eval_boacas_on_realm(
    realm: &mut WarmBoaRealm,
    header: &BoaCasHeader,
    _precompiled_slice: &[u8],
    fallback_js: &str,
    is_http_req: bool,
) -> (boa_engine::JsResult<boa_engine::JsValue>, bool) {
    let effective_shards = if is_http_req {
        header.shards_mask | (1 << 11)
    } else {
        header.shards_mask
    };
    let init_lazy = (header.feature_flags & BOACAS_FLAG_NEEDS_LAZY_GETTERS) != 0;

    if (header.feature_flags & BOACAS_FLAG_IS_RC_BUNDLE) != 0 {
        if let Some(fn_obj) = realm.loaded_bundles.get(&header.rc_key).cloned() {
            if is_http_req && (realm.loaded_shards_mask & (1 << 11)) == 0 {
                ensure_realm_shards(realm, 1 << 11, false);
            }
            let res = fn_obj.call(&boa_engine::JsValue::undefined(), &[], &mut realm.ctx);
            return (res, false);
        }
        ensure_realm_shards(realm, effective_shards, init_lazy);
        let res = realm
            .ctx
            .eval(boa_engine::Source::from_bytes(fallback_js.as_bytes()));
        if res.is_ok() && BOA_VIOLATION.with(|v| v.borrow().is_none()) {
            let mut hex_buf = [0u8; 16];
            let _ = std::io::Write::write_fmt(
                &mut (&mut hex_buf[..]),
                format_args!("{:016x}", header.rc_key),
            );
            if let Ok(hex_str) = std::str::from_utf8(&hex_buf) {
                let global = realm.ctx.global_object();
                if let Ok(rc_val) = global.get(boa_engine::js_string!("__slice_rc"), &mut realm.ctx)
                {
                    if let Some(rc_obj) = rc_val.as_object() {
                        if let Ok(val) = rc_obj.get(boa_engine::js_string!(hex_str), &mut realm.ctx)
                        {
                            if let Some(obj) = val.as_object() {
                                realm.loaded_bundles.insert(header.rc_key, obj.clone());
                            }
                        }
                    }
                }
            }
        }
        (res, true)
    } else {
        ensure_realm_shards(realm, effective_shards, init_lazy);
        let res = if (header.feature_flags & BOACAS_FLAG_STARTS_WITH_BRACE) != 0 {
            realm
                .ctx
                .eval(boa_engine::Source::from_bytes(fallback_js.as_bytes()))
        } else {
            let scoped = format!("{{\n{}\n}}", fallback_js);
            realm
                .ctx
                .eval(boa_engine::Source::from_bytes(scoped.as_bytes()))
        };
        (res, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser_and_compiler::compile_source_to_bytecode;
    use crate::SliceIsolate;
    use slice_sandbox::CapabilityToken;

    #[test]
    fn test_boacas_header_roundtrip_and_runtime_toggle() {
        set_thread_exper_boacas_override(Some(true));
        let ts_src = r#"
            const u = new URL("https://edge.slice.local/v1/items?limit=10");
            const enc = new TextEncoder().encode(u.pathname);
            console.log(u.searchParams.get("limit") + ":" + enc.length);
        "#;
        let bc = compile_source_to_bytecode(ts_src);
        assert_eq!(&bc[0..8], BOACAS_MAGIC);

        let (hdr, _pre, fb_js) = BoaCasHeader::decode(&bc).expect("Valid SLICEBCv1 header");
        assert_ne!(hdr.shards_mask & (1 << 1), 0, "Expected URL shard bit set");
        assert_ne!(
            hdr.shards_mask & (1 << 2),
            0,
            "Expected TextEncoder shard bit set"
        );
        assert!(fb_js.contains("URLSearchParams") || fb_js.contains("new URL"));

        // Execute with --exper-boacas enabled
        let mut iso1 = SliceIsolate::spawn(
            "boacas-t1",
            1,
            CapabilityToken::pure_compute(),
            100_000,
            4 * 1024 * 1024,
        );
        let m1 = iso1.execute_bytecode(&bc, None).unwrap();
        assert_eq!(m1.stdout, vec!["10:9"]);

        // Execute the exact same SLICEBC\x01 buffer with --no-exper-boacas (fallback path)
        set_thread_exper_boacas_override(Some(false));
        let mut iso2 = SliceIsolate::spawn(
            "boacas-t2",
            1,
            CapabilityToken::pure_compute(),
            100_000,
            4 * 1024 * 1024,
        );
        let m2 = iso2.execute_bytecode(&bc, None).unwrap();
        assert_eq!(m2.stdout, vec!["10:9"]);

        set_thread_exper_boacas_override(None);
    }
}

