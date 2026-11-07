//! Experimental Precompiled Boa CAS Container (`SLICEBC\x01`, `--exper-boacas`).
//!
//! Provides the shared wire format, build-time feature/shard metadata precomputation,
//! runtime experiment flag controls (`--exper-boacas` / `--no-exper-boacas` / `SLICE_EXPER_BOACAS`),
//! and execution hooks for the `EXPER_BOACAS` architecture.

use crate::boa_realm::{ensure_realm_shards, WarmBoaRealm, BOA_VIOLATION};
use crate::js_preludes::detect_shards_and_boacas_flags;
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

/// Process-wide override set via CLI flags (`--boacas` / `--exper-boacas` -> `1`, `--no-boacas` / `--no-exper-boacas` -> `2`, unset -> `0`).
static EXPER_BOACAS_OVERRIDE: AtomicU8 = AtomicU8::new(0);

/// Process-wide execution tier override (`--tier=1` -> `1`, `--tier=0` -> `2`, unset/auto -> `0`).
static TIER_OVERRIDE: AtomicU8 = AtomicU8::new(0);

thread_local! {
    static THREAD_EXPER_BOACAS_OVERRIDE: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
    static THREAD_TIER_OVERRIDE: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

/// Sets or clears the process-wide `--boacas` / `--no-boacas` (`--exper-boacas` / `--no-exper-boacas`) override.
#[inline]
pub fn set_exper_boacas_override(enabled: Option<bool>) {
    let code = match enabled {
        None => 0,
        Some(true) => 1,
        Some(false) => 2,
    };
    EXPER_BOACAS_OVERRIDE.store(code, Ordering::Relaxed);
}

/// Sets or clears a thread-local `--boacas` / `--no-boacas` override (safe for parallel tests and one-shot overrides).
#[inline]
pub fn set_thread_exper_boacas_override(enabled: Option<bool>) {
    let code = match enabled {
        None => 0,
        Some(true) => 1,
        Some(false) => 2,
    };
    THREAD_EXPER_BOACAS_OVERRIDE.with(|c| c.set(code));
}

/// Sets or clears the process-wide execution tier override (`Some(1)` = Tier 1 Boa, `Some(0)` = Tier 0 SLICESNP, `None` = default/auto).
#[inline]
pub fn set_tier_override(tier: Option<u8>) {
    let code = match tier {
        Some(1) => 1,
        Some(0) => 2,
        _ => 0,
    };
    TIER_OVERRIDE.store(code, Ordering::Relaxed);
}

/// Sets or clears a thread-local execution tier override (`Some(1)` = Tier 1 Boa, `Some(0)` = Tier 0 SLICESNP, `None` = default/auto).
#[inline]
pub fn set_thread_tier_override(tier: Option<u8>) {
    let code = match tier {
        Some(1) => 1,
        Some(0) => 2,
        _ => 0,
    };
    THREAD_TIER_OVERRIDE.with(|c| c.set(code));
}

/// Returns the active execution tier override (`Some(1)` = force Tier 1 Boa, `Some(0)` = force Tier 0 SLICESNP, `None` = default optimal Tier 1 with Tier 0/2 kernels).
#[inline]
pub fn active_tier_override() -> Option<u8> {
    let tl = THREAD_TIER_OVERRIDE.with(|c| c.get());
    if tl == 1 {
        return Some(1);
    } else if tl == 2 {
        return Some(0);
    }
    match TIER_OVERRIDE.load(Ordering::Relaxed) {
        1 => return Some(1),
        2 => return Some(0),
        _ => {}
    }
    if let Ok(val) = std::env::var("SLICE_TIER") {
        if let Some(parsed) = slice_sandbox::parse_tier_setting(&val) {
            return parsed;
        }
    }
    None
}

/// Returns `true` when `slice pack` / `slice compile` / `compile_source_to_bytecode` should emit
/// `SLICEBC\x01` precompiled Boa `CodeBlock` containers (enabled by default unless disabled via
/// `--no-boacas`, `--no-exper-boacas`, `SLICE_BOACAS=0`, `SLICE_EXPER_BOACAS=0`, or `boacas = false`).
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
    if let Ok(val) = std::env::var("SLICE_BOACAS").or_else(|_| std::env::var("SLICE_EXPER_BOACAS")) {
        if let Some(b) = slice_sandbox::parse_bool_setting(&val) {
            return b;
        }
    }
    true
}

/// Returns `true` when `slice run` / `slice serve` should execute the precompiled `CodeBlock` fast-path
/// inside a `SLICEBC\x01` archive (enabled by default unless disabled via `--no-boacas`,
/// `--no-exper-boacas`, `SLICE_BOACAS=0`, or `SLICE_EXPER_BOACAS=0`).
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
    if let Ok(val) = std::env::var("SLICE_BOACAS").or_else(|_| std::env::var("SLICE_EXPER_BOACAS")) {
        if let Some(b) = slice_sandbox::parse_bool_setting(&val) {
            return b;
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

/// Computes `(feature_flags, shards_mask, rc_key)` once at build time in a single `O(N)`
/// Aho-Corasick pass over `js_code` (zero separate `.contains()` linear substring scans).
pub fn compute_boacas_metadata(js_code: &str) -> (u16, u32, u64) {
    let (rc_key, is_rc_bundle) = match extract_slice_rc_key(js_code) {
        Some((k, _)) => (k, true),
        None => (0, false),
    };
    let (mut flags, shards_mask) = detect_shards_and_boacas_flags(js_code, false, is_rc_bundle);
    if is_rc_bundle {
        flags |= BOACAS_FLAG_IS_RC_BUNDLE;
    }
    if js_code.starts_with('{') {
        flags |= BOACAS_FLAG_STARTS_WITH_BRACE;
    }
    (flags, shards_mask, rc_key)
}

/// Packs a compacted JavaScript bundle into a `SLICEBC\x01` binary container (`Approach C`: `approach_kind = 3`).
///
/// Fuses `ULTRA_CORE_PRELUDE` + topological `shards_mask` shards + `fallback_js`, parses and compiles
/// the entire program into a `boa_engine::vm::CodeBlock` tree at build time (`Script::compile_to_slicebc`),
/// and embeds the serialized bytecode tree into `precompiled`.
/// At cold runtime, `boa_lexer`, `boa_parser`, `Scope::analyze_scope`, and `ByteCompiler` are 100% bypassed.
pub fn pack_boacas_container(compacted_js: &str) -> Vec<u8> {
    let mut fallback_js = String::with_capacity(compacted_js.len() + 4);
    fallback_js.push_str("{\n");
    fallback_js.push_str(compacted_js.trim_end());
    fallback_js.push_str("\n}");

    let (feature_flags, shards_mask, rc_key) = compute_boacas_metadata(&fallback_js);
    let init_lazy = (feature_flags & BOACAS_FLAG_NEEDS_LAZY_GETTERS) != 0;
    let has_next_tick = (feature_flags & BOACAS_FLAG_HAS_NEXT_TICK) != 0;
    let fused_bytes = crate::js_preludes::build_fused_prelude_and_bundle(
        &fallback_js,
        shards_mask,
        init_lazy,
        has_next_tick,
    );

    let mut compiler_ctx = boa_engine::Context::default();
    let precompiled = if let Ok(script) = boa_engine::Script::parse(
        boa_engine::Source::from_bytes(&fused_bytes),
        None,
        &mut compiler_ctx,
    ) {
        script
            .compile_to_slicebc(&mut compiler_ctx)
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let header = BoaCasHeader {
        approach_kind: 3,
        feature_flags,
        shards_mask,
        rc_key,
        precompiled_len: precompiled.len() as u32,
        fallback_js_len: fallback_js.len() as u32,
    };

    let mut out = Vec::with_capacity(BOACAS_HEADER_LEN + precompiled.len() + fallback_js.len());
    header.encode_into(&mut out);
    out.extend_from_slice(&precompiled);
    out.extend_from_slice(fallback_js.as_bytes());
    out
}

/// Evaluates a decoded `SLICEBC\x01` (`Approach C`) payload on `realm`.
///
/// When `is_cold_bare_realm` is `true` and `header.approach_kind == 3`, reconstructs the precompiled
/// `Gc<CodeBlock>` tree directly from `precompiled_slice` via `boa_engine::Script::from_slicebc_bytes`
/// and calls `script.evaluate(&mut realm.ctx)` — skipping `boa_lexer`, `boa_parser`, `Scope::analyze_scope`,
/// and `ByteCompiler` at runtime.
pub(crate) fn eval_boacas_on_realm(
    realm: &mut WarmBoaRealm,
    header: &BoaCasHeader,
    precompiled_slice: &[u8],
    fallback_js: &str,
    _full_contiguous_payload: &[u8],
    is_http_req: bool,
    is_cold_bare_realm: bool,
) -> (boa_engine::JsResult<boa_engine::JsValue>, bool) {
    let effective_shards = if is_http_req {
        header.shards_mask | (1 << 11)
    } else {
        header.shards_mask
    };
    let init_lazy = (header.feature_flags & BOACAS_FLAG_NEEDS_LAZY_GETTERS) != 0;

    if is_cold_bare_realm && header.approach_kind == 3 && !precompiled_slice.is_empty() {
        crate::js_preludes::register_shard_host_functions_for_mask(
            effective_shards,
            &mut realm.ctx,
        );
        if let Some(script) =
            boa_engine::Script::from_slicebc_bytes(precompiled_slice, &mut realm.ctx)
        {
            let res = script.evaluate(&mut realm.ctx);
            realm.loaded_shards_mask = effective_shards | if init_lazy { 1 } else { 0 };
            crate::boa_realm::populate_realm_cached_handles(realm);
            if (effective_shards & !header.shards_mask) != 0 {
                ensure_realm_shards(realm, effective_shards & !header.shards_mask, false);
            }
            if (effective_shards & (1 << 8)) != 0 && realm.init_worker_self_fn.is_none() {
                realm.init_worker_self_fn = realm
                    .ctx
                    .global_object()
                    .get(
                        boa_engine::js_string!("__slice_init_worker_self_env"),
                        &mut realm.ctx,
                    )
                    .ok()
                    .and_then(|v| v.as_object());
            }
            if (header.feature_flags & BOACAS_FLAG_IS_RC_BUNDLE) != 0
                && res.is_ok()
                && BOA_VIOLATION.with(|v| v.borrow().is_none())
            {
                let mut hex_buf = [0u8; 16];
                let _ = std::io::Write::write_fmt(
                    &mut (&mut hex_buf[..]),
                    format_args!("{:016x}", header.rc_key),
                );
                if let Ok(hex_str) = std::str::from_utf8(&hex_buf) {
                    let global = realm.ctx.global_object();
                    if let Ok(rc_val) =
                        global.get(boa_engine::js_string!("__slice_rc"), &mut realm.ctx)
                    {
                        if let Some(rc_obj) = rc_val.as_object() {
                            if let Ok(val) =
                                rc_obj.get(boa_engine::js_string!(hex_str), &mut realm.ctx)
                            {
                                if let Some(obj) = val.as_object() {
                                    realm.loaded_bundles.insert(header.rc_key, obj.clone());
                                }
                            }
                        }
                    }
                }
            }
            return (res, true);
        }
        // Fallback if CodeBlock deserialization fails
        let _ = realm.ctx.eval(boa_engine::Source::from_bytes(
            crate::js_preludes::ULTRA_CORE_PRELUDE,
        ));
        crate::boa_realm::populate_realm_cached_handles(realm);
    }

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

        let (hdr, pre, fb_js) = BoaCasHeader::decode(&bc).expect("Valid SLICEBCv1 header");
        assert_eq!(
            hdr.approach_kind, 3,
            "Expected Approach C (precompiled CodeBlock bytecode)"
        );
        assert!(
            !pre.is_empty(),
            "Expected non-empty precompiled CodeBlock bytecode"
        );
        assert_ne!(hdr.shards_mask & (1 << 1), 0, "Expected URL shard bit set");
        assert_ne!(
            hdr.shards_mask & (1 << 2),
            0,
            "Expected TextEncoder shard bit set"
        );
        assert!(fb_js.contains("URLSearchParams") || fb_js.contains("new URL"));

        // Clear thread-local pool so iso1 takes the cold bare realm precompiled CodeBlock path
        crate::boa_realm::BOA_CONTEXT_POOL.with(|pool| pool.borrow_mut().clear());

        // Execute with --exper-boacas enabled (cold precompiled CodeBlock)
        let mut iso1 = SliceIsolate::spawn(
            "boacas-c-t1",
            1,
            CapabilityToken::pure_compute(),
            100_000,
            4 * 1024 * 1024,
        );
        let m1 = iso1.execute_bytecode(&bc, None).unwrap();
        assert_eq!(m1.stdout, vec!["10:9"]);

        // Execute a second time on the warm pooled realm
        let mut iso_warm = SliceIsolate::spawn(
            "boacas-c-warm",
            1,
            CapabilityToken::pure_compute(),
            100_000,
            4 * 1024 * 1024,
        );
        let m_warm = iso_warm.execute_bytecode(&bc, None).unwrap();
        assert_eq!(m_warm.stdout, vec!["10:9"]);

        // Execute the exact same SLICEBC\x01 buffer with --no-exper-boacas (fallback path)
        set_thread_exper_boacas_override(Some(false));
        let mut iso2 = SliceIsolate::spawn(
            "boacas-c-t2",
            1,
            CapabilityToken::pure_compute(),
            100_000,
            4 * 1024 * 1024,
        );
        let m2 = iso2.execute_bytecode(&bc, None).unwrap();
        assert_eq!(m2.stdout, vec!["10:9"]);

        set_thread_exper_boacas_override(None);
    }

    #[test]
    fn test_boacas_codeblock_edge_cases_bigint_utf16_closures_handlers_and_corruption_fallback() {
        set_thread_exper_boacas_override(Some(true));
        let edge_src = r#"
            const big = 9007199254740993n * 2n;
            const utf16 = "αβγδ-λ-Ω";
            function makeAdder(base) {
                let acc = base;
                return function(step) {
                    try {
                        if (step < 0) throw new Error("neg:" + step);
                        acc += step;
                        return acc;
                    } catch (e) {
                        return -1;
                    } finally {
                        acc += 1;
                    }
                };
            }
            class Boxed {
                constructor(v) { this._v = v; }
                get val() { return this._v; }
            }
            const add = makeAdder(10);
            const r1 = add(5);
            const r2 = add(-3);
            const r3 = new Boxed(add(0)).val;
            Promise.resolve(utf16.length).then((uLen) => {
                console.log(big.toString() + "|" + utf16 + "|" + uLen + "|" + r1 + "," + r2 + "," + r3);
            });
        "#;
        let bc = compile_source_to_bytecode(edge_src);
        assert_eq!(&bc[0..8], BOACAS_MAGIC);

        // 1. Cold bare-realm execution of precompiled CodeBlock with BigInt, UTF-16, closures, and try/catch/finally
        crate::boa_realm::BOA_CONTEXT_POOL.with(|pool| pool.borrow_mut().clear());
        let mut iso_cold = SliceIsolate::spawn(
            "boacas-edge-cold",
            1,
            CapabilityToken::pure_compute(),
            100_000,
            8 * 1024 * 1024,
        );
        let m_cold = iso_cold.execute_bytecode(&bc, None).unwrap();
        assert_eq!(
            m_cold.stdout,
            vec!["18014398509481986|αβγδ-λ-Ω|8|15,-1,17"]
        );

        // 2. Corrupt the inner SLCB\x01 magic inside precompiled_slice and verify graceful fallback to fallback_js
        let mut corrupted_bc = bc.clone();
        corrupted_bc[BOACAS_HEADER_LEN] ^= 0xFF;
        crate::boa_realm::BOA_CONTEXT_POOL.with(|pool| pool.borrow_mut().clear());
        let mut iso_corrupt = SliceIsolate::spawn(
            "boacas-edge-corrupt",
            1,
            CapabilityToken::pure_compute(),
            100_000,
            8 * 1024 * 1024,
        );
        let m_corrupt = iso_corrupt.execute_bytecode(&corrupted_bc, None).unwrap();
        assert_eq!(
            m_corrupt.stdout,
            vec!["18014398509481986|αβγδ-λ-Ω|8|15,-1,17"]
        );

        // 3. Truncated header returns None from BoaCasHeader::decode
        assert!(BoaCasHeader::decode(&bc[..16]).is_none());

        set_thread_exper_boacas_override(None);
    }

    #[test]
    fn test_boacas_prefused_node_builtins_and_deferred_module_spans() {
        set_thread_exper_boacas_override(Some(true));

        let tmp_dir = std::env::temp_dir().join("slice_boacas_edge_node_modules");
        let _ = std::fs::remove_dir_all(&tmp_dir);
        let nm_dir = tmp_dir.join("node_modules");
        std::fs::create_dir_all(nm_dir.join("hot-pkg")).unwrap();
        std::fs::create_dir_all(nm_dir.join("cold-pkg")).unwrap();
        std::fs::write(
            nm_dir.join("hot-pkg").join("index.js"),
            "const path = require('node:path');\nexports.tag = function(a, b) { return path.posix.join(a, b); };\n",
        )
        .unwrap();
        std::fs::write(
            nm_dir.join("cold-pkg").join("index.js"),
            "exports.neverCalled = function() { return 'COLD_PKG_LOADED'; };\n",
        )
        .unwrap();
        let entry_path = tmp_dir.join("index.js");
        std::fs::write(
            &entry_path,
            r#"
            const hot = require("hot-pkg");
            const crypto = require("node:crypto");
            const assert = require("node:assert");
            function unusedHelper() {
                return require("cold-pkg").neverCalled();
            }
            const joined = hot.tag("edge", "v1");
            assert.strictEqual(joined, "edge/v1");
            const digest = crypto.createHash("sha256").update(joined).digest("hex").slice(0, 8);
            console.log(joined + ":" + digest);
            "#,
        )
        .unwrap();

        let bundled = slice_cas::bundle_entrypoint_with_deps(&entry_path).unwrap();
        let _ = std::fs::remove_dir_all(&tmp_dir);

        // Verify Branch A's Oxc AST deferred function span optimization turned cold-pkg into a lazy string literal
        assert!(
            bundled.contains("= \"exports.neverCalled"),
            "Expected cold-pkg inside unusedHelper() to be deferred as a lazy string literal"
        );

        let bc = compile_source_to_bytecode(&bundled);
        let (hdr, _, _) = BoaCasHeader::decode(&bc).expect("Valid SLICEBCv1 header");
        assert_ne!(hdr.shards_mask & (1 << 19), 0, "Expected node:crypto bit 19");
        assert_ne!(hdr.shards_mask & (1 << 21), 0, "Expected node:path bit 21");
        assert_ne!(hdr.shards_mask & (1 << 25), 0, "Expected node:assert bit 25");
        assert_ne!(hdr.feature_flags & BOACAS_FLAG_IS_RC_BUNDLE, 0);

        crate::boa_realm::BOA_CONTEXT_POOL.with(|pool| pool.borrow_mut().clear());
        let mut iso_cold = SliceIsolate::spawn(
            "boacas-node-cold",
            1,
            CapabilityToken::pure_compute(),
            100_000,
            8 * 1024 * 1024,
        );
        let m_cold = iso_cold.execute_bytecode(&bc, None).unwrap();
        assert_eq!(m_cold.stdout.len(), 1);
        assert!(m_cold.stdout[0].starts_with("edge/v1:"));

        // Second execution hits the warm __slice_rc cached function handle
        let mut iso_warm = SliceIsolate::spawn(
            "boacas-node-warm",
            1,
            CapabilityToken::pure_compute(),
            100_000,
            8 * 1024 * 1024,
        );
        let m_warm = iso_warm.execute_bytecode(&bc, None).unwrap();
        assert_eq!(m_warm.stdout, m_cold.stdout);

        set_thread_exper_boacas_override(None);
    }
}
