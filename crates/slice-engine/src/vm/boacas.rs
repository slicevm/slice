//! Experimental Precompiled Boa CAS Container (`SLICEBC\x01`, `--exper-boacas`).
//!
//! Provides the shared wire format, build-time feature/shard metadata precomputation,
//! runtime experiment flag controls (`--exper-boacas` / `--no-exper-boacas` / `SLICE_EXPER_BOACAS`),
//! and execution hooks for the `EXPER_BOACAS` architecture.

use crate::boa_realm::{ensure_realm_shards, WarmBoaRealm, BOA_VIOLATION};
use crate::js_preludes::detect_shards_and_boacas_flags;
use crate::parser_and_compiler::extract_slice_rc_key;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

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

static ENV_TIER_OVERRIDE: OnceLock<Option<u8>> = OnceLock::new();
static ENV_EXPER_BOACAS_ENABLED: OnceLock<bool> = OnceLock::new();

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
    *ENV_TIER_OVERRIDE.get_or_init(|| {
        std::env::var("SLICE_TIER")
            .ok()
            .and_then(|val| slice_sandbox::parse_tier_setting(&val))
            .flatten()
    })
}

#[inline]
fn env_exper_boacas_default() -> bool {
    *ENV_EXPER_BOACAS_ENABLED.get_or_init(|| {
        std::env::var("SLICE_BOACAS")
            .or_else(|_| std::env::var("SLICE_EXPER_BOACAS"))
            .ok()
            .and_then(|val| slice_sandbox::parse_bool_setting(&val))
            .unwrap_or(true)
    })
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
    env_exper_boacas_default()
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
    env_exper_boacas_default()
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

    let precompiled = crate::js_preludes::compile_static_js_to_slicebc(&fused_bytes);

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
            let fused_spec =
                crate::boa_realm::encode_fused_source_spec(header.shards_mask, init_lazy);
            crate::boa_realm::set_active_source_fused_spec(fused_spec);
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
                                    realm
                                        .loaded_bundle_fused_specs
                                        .insert(header.rc_key, fused_spec);
                                }
                            }
                        }
                    }
                }
            }
            return (res, true);
        }
        // Fallback if CodeBlock deserialization fails
        crate::boa_realm::set_active_source_fused_spec(0);
        let _ = crate::js_preludes::eval_ultra_core_prelude(&mut realm.ctx);
        crate::boa_realm::populate_realm_cached_handles(realm);
    }

    if (header.feature_flags & BOACAS_FLAG_IS_RC_BUNDLE) != 0 {
        if let Some(fn_obj) = realm.loaded_bundles.get(&header.rc_key).cloned() {
            let fused_spec = realm
                .loaded_bundle_fused_specs
                .get(&header.rc_key)
                .copied()
                .unwrap_or(0);
            crate::boa_realm::set_active_source_fused_spec(fused_spec);
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
            crate::boa_realm::eval_or_reuse_cached_script(realm, fallback_js)
        } else {
            let scoped = format!("{{\n{}\n}}", fallback_js);
            crate::boa_realm::eval_or_reuse_cached_script(realm, &scoped)
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
        assert_eq!(m_cold.stdout, vec!["18014398509481986|αβγδ-λ-Ω|8|15,-1,17"]);

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
        assert_ne!(
            hdr.shards_mask & (1 << 19),
            0,
            "Expected node:crypto bit 19"
        );
        assert_ne!(hdr.shards_mask & (1 << 21), 0, "Expected node:path bit 21");
        assert_ne!(
            hdr.shards_mask & (1 << 25),
            0,
            "Expected node:assert bit 25"
        );
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

    #[test]
    fn test_slicebc_adversarial_deserialization_bounds() {
        let mut ctx = boa_engine::Context::default();
        let valid_script =
            boa_engine::Script::parse(boa_engine::Source::from_bytes(b"1 + 2"), None, &mut ctx)
                .expect("valid parse");
        let valid_bytes = valid_script
            .compile_to_slicebc(&mut ctx)
            .expect("valid slicebc");
        assert_eq!(&valid_bytes[..5], b"SLCB\x01");
        assert!(boa_engine::Script::from_slicebc_bytes(&valid_bytes, &mut ctx).is_some());

        // 1. Bad magic header must fail closed immediately
        let mut bad_magic = valid_bytes.clone();
        bad_magic[0] = b'X';
        assert!(boa_engine::Script::from_slicebc_bytes(&bad_magic, &mut ctx).is_none());

        // 2. Trailing garbage bytes must be rejected
        let mut with_trailing = valid_bytes.clone();
        with_trailing.push(0xAA);
        assert!(boa_engine::Script::from_slicebc_bytes(&with_trailing, &mut ctx).is_none());

        // 3. Adversarial global_bindings = u32::MAX (zero-byte loop hazard)
        let t0 = std::time::Instant::now();
        let mut huge_globals = valid_bytes.clone();
        huge_globals[5..9].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(boa_engine::Script::from_slicebc_bytes(&huge_globals, &mut ctx).is_none());

        // 4. Adversarial register_count = u32::MAX & mapped_len = u32::MAX (OOM hazard)
        let mut huge_regs = Vec::new();
        huge_regs.extend_from_slice(b"SLCB\x01");
        huge_regs.extend_from_slice(&0u32.to_le_bytes()); // global_bindings
        huge_regs.extend_from_slice(&0u16.to_le_bytes()); // flags
        huge_regs.extend_from_slice(&0u32.to_le_bytes()); // length
        huge_regs.extend_from_slice(&0u32.to_le_bytes()); // parameter_length
        huge_regs.extend_from_slice(&u32::MAX.to_le_bytes()); // register_count
        huge_regs.push(1u8); // strict
        assert!(boa_engine::Script::from_slicebc_bytes(&huge_regs, &mut ctx).is_none());

        let mut huge_mapped = Vec::new();
        huge_mapped.extend_from_slice(b"SLCB\x01");
        huge_mapped.extend_from_slice(&0u32.to_le_bytes()); // global_bindings
        huge_mapped.extend_from_slice(&0u16.to_le_bytes()); // flags
        huge_mapped.extend_from_slice(&0u32.to_le_bytes()); // length
        huge_mapped.extend_from_slice(&0u32.to_le_bytes()); // parameter_length
        huge_mapped.extend_from_slice(&4u32.to_le_bytes()); // register_count
        huge_mapped.push(1u8); // strict
        huge_mapped.extend_from_slice(&u32::MAX.to_le_bytes()); // mapped_len = u32::MAX
        assert!(boa_engine::Script::from_slicebc_bytes(&huge_mapped, &mut ctx).is_none());

        // 5. Adversarial Constant::Scope binding count = u32::MAX - 1 (34 GB scope_cache resize hazard)
        let mut huge_scope = Vec::new();
        huge_scope.extend_from_slice(b"SLCB\x01");
        huge_scope.extend_from_slice(&0u32.to_le_bytes()); // global_bindings
        huge_scope.extend_from_slice(&0u16.to_le_bytes()); // flags
        huge_scope.extend_from_slice(&0u32.to_le_bytes()); // length
        huge_scope.extend_from_slice(&0u32.to_le_bytes()); // parameter_length
        huge_scope.extend_from_slice(&1u32.to_le_bytes()); // register_count
        huge_scope.push(1u8); // strict
        huge_scope.extend_from_slice(&0u32.to_le_bytes()); // mapped_len
        huge_scope.extend_from_slice(&0u32.to_le_bytes()); // bc_len
        huge_scope.extend_from_slice(&1u32.to_le_bytes()); // const_len = 1
        huge_scope.push(3u8); // Constant::Scope
        huge_scope.extend_from_slice(&(u32::MAX - 1).to_le_bytes()); // n = u32::MAX - 1
        assert!(boa_engine::Script::from_slicebc_bytes(&huge_scope, &mut ctx).is_none());

        // 6. 500-deep nested Constant::Function recursion bomb
        let mut deep_fn = Vec::new();
        deep_fn.extend_from_slice(b"SLCB\x01");
        deep_fn.extend_from_slice(&0u32.to_le_bytes()); // global_bindings
        for _ in 0..500 {
            deep_fn.extend_from_slice(&0u16.to_le_bytes()); // flags
            deep_fn.extend_from_slice(&0u32.to_le_bytes()); // length
            deep_fn.extend_from_slice(&0u32.to_le_bytes()); // parameter_length
            deep_fn.extend_from_slice(&1u32.to_le_bytes()); // register_count
            deep_fn.push(1u8); // strict
            deep_fn.extend_from_slice(&0u32.to_le_bytes()); // mapped_len
            deep_fn.extend_from_slice(&0u32.to_le_bytes()); // bc_len
            deep_fn.extend_from_slice(&1u32.to_le_bytes()); // const_len = 1
            deep_fn.push(1u8); // tag = 1 (Constant::Function)
        }
        assert!(boa_engine::Script::from_slicebc_bytes(&deep_fn, &mut ctx).is_none());
        assert!(
            t0.elapsed().as_millis() < 50,
            "adversarial SLICEBC checks must reject in sub-millisecond time"
        );
    }

    #[test]
    fn test_inline_cache_shape_id_gc_churn_and_unique_shape_transitions() {
        let mut ctx = boa_engine::Context::default();

        // Define an IC-hot reader/writer over both SharedShape and UniqueShape objects
        // (Object.create(null) + property deletion transitions into dictionary/UniqueShape mode)
        ctx.eval(boa_engine::Source::from_bytes(
            br#"
            function readTarget(o) { return o.target; }
            function writeTarget(o, v) { o.target = v; }
            function makeUniqueWithSixSlots(val) {
                var u = Object.create(null);
                u.a = 1; u.b = 2; u.c = 3; u.d = 4; u.e = 5; u.target = val;
                delete u.a;
                return u;
            }
            function makeUniqueWithOneSlot(val) {
                var u = Object.create(null);
                u.x = 1; u.only = val;
                delete u.x;
                return u;
            }
            "#,
        ))
        .expect("setup IC helpers");

        // Churn GC between UniqueShape allocations to verify monotonic Shape::id() never hits stale slots
        for round in 0..32 {
            let v = ctx
                .eval(boa_engine::Source::from_bytes(
                    format!("readTarget(makeUniqueWithSixSlots({}))", 1000 + round).as_bytes(),
                ))
                .expect("eval six-slot unique shape");
            assert_eq!(v.as_number(), Some((1000 + round) as f64));
            boa_engine::gc::force_collect();

            let v_small = ctx
                .eval(boa_engine::Source::from_bytes(
                    b"readTarget(makeUniqueWithOneSlot(7))",
                ))
                .expect("eval one-slot unique shape after GC");
            assert!(v_small.is_undefined());
        }

        // Verify UniqueShape in-place defineProperty(writable: false) invalidates cached writable IC slot
        let frozen_prop_res = ctx
            .eval(boa_engine::Source::from_bytes(
                br#"
                function strictWriteTarget(o, v) { "use strict"; o.target = v; }
                var u = Object.create(null);
                u.a = 1; u.target = 10;
                delete u.a;
                strictWriteTarget(u, 20);
                Object.defineProperty(u, "target", { writable: false });
                var threw = false;
                try { strictWriteTarget(u, 99); } catch (_e) { threw = true; }
                threw + ":" + u.target
                "#,
            ))
            .expect("strict non-writable UniqueShape check");
        assert_eq!(
            frozen_prop_res
                .as_string()
                .map(|s| s.to_std_string_escaped()),
            Some("true:20".to_string())
        );

        // Microbenchmark monomorphic IC loop throughput
        let t0 = std::time::Instant::now();
        let sum_val = ctx
            .eval(boa_engine::Source::from_bytes(
                br#"
                (function() {
                    var pt = { x: 3, y: 4, z: 5 };
                    var acc = 0;
                    for (var i = 0; i < 100000; i++) {
                        acc += pt.x + pt.y + pt.z;
                    }
                    return acc;
                })()
                "#,
            ))
            .expect("monomorphic IC loop");
        assert_eq!(sum_val.as_number(), Some(1_200_000.0));
        println!(
            "[IC-PERF] 300k monomorphic Shape::id() IC reads completed in {:.2} ms",
            t0.elapsed().as_secs_f64() * 1_000.0
        );
    }
}
