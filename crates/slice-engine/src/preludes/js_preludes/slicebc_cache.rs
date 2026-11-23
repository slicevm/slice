//! 2-Hit `SLCB\x01` Precompiled Bytecode Cache for Static JS Preludes & Shards.

use super::{
    LAZY_GETTERS_INIT_PRELUDE, ULTRA_CORE_PRELUDE, register_shard_host_functions_for_mask,
    shard_prelude_bytes,
};
use std::sync::OnceLock;

static ULTRA_CORE_HITS: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
static ULTRA_CORE_SLICEBC: OnceLock<Vec<u8>> = OnceLock::new();
static LAZY_GETTERS_HITS: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
static LAZY_GETTERS_SLICEBC: OnceLock<Vec<u8>> = OnceLock::new();
static SHARD_HITS: [std::sync::atomic::AtomicU8; 30] =
    [const { std::sync::atomic::AtomicU8::new(0) }; 30];
static SHARD_SLICEBC: [OnceLock<Vec<u8>>; 30] = [const { OnceLock::new() }; 30];

#[inline]
pub(crate) fn compile_static_js_to_slicebc(bytes: &[u8]) -> Vec<u8> {
    compile_static_js_to_slicebc_with_min_line(bytes, u32::MAX)
}

#[inline]
pub(crate) fn compile_static_js_to_slicebc_with_min_line(
    bytes: &[u8],
    min_sourcemap_line: u32,
) -> Vec<u8> {
    if cfg!(debug_assertions) {
        let owned = bytes.to_vec();
        return std::thread::Builder::new()
            .name("slicebc-static-compiler".to_string())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let mut compiler_ctx = crate::boa_realm::build_compiler_boa_context();
                if let Ok(script) = boa_engine::Script::parse(
                    boa_engine::Source::from_bytes(&owned),
                    None,
                    &mut compiler_ctx,
                ) {
                    script
                        .compile_to_slicebc_with_min_line(&mut compiler_ctx, min_sourcemap_line)
                        .unwrap_or_default()
                } else {
                    Vec::new()
                }
            })
            .expect("failed to spawn slicebc-static-compiler thread")
            .join()
            .expect("slicebc-static-compiler thread panicked");
    }
    let mut compiler_ctx = crate::boa_realm::build_compiler_boa_context();
    if let Ok(script) = boa_engine::Script::parse(
        boa_engine::Source::from_bytes(bytes),
        None,
        &mut compiler_ctx,
    ) {
        script
            .compile_to_slicebc_with_min_line(&mut compiler_ctx, min_sourcemap_line)
            .unwrap_or_default()
    } else {
        Vec::new()
    }
}

/// Evaluates a static JS prelude/builtin snippet using a 2-hit `AtomicU8` gate:
/// - **1st evaluation in a process (`hit == 0`)**: Evaluates `Source::from_bytes(bytes)` directly
///   on `context`, avoiding any throwaway `Context` construction or `SLCB\x01` serialization on cold CLI runs.
/// - **2nd+ evaluation in a process (`hit >= 1`)**: Compiles and caches `SLCB\x01` bytecode in `cache`
///   (reusing `context` directly when `scope().num_bindings() == 0`) and evaluates from `SLCB\x01` thereafter.
#[inline]
pub(crate) fn eval_static_js_with_2hit_slicebc_cache(
    hit_counter: &std::sync::atomic::AtomicU8,
    cache: &OnceLock<Vec<u8>>,
    bytes: &'static [u8],
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<boa_engine::JsValue> {
    if let Some(precompiled) = cache.get()
        && !precompiled.is_empty()
        && let Some(script) = boa_engine::Script::from_slicebc_bytes(precompiled, context)
    {
        return script.evaluate(context);
    }
    if !cfg!(debug_assertions) {
        if hit_counter
            .compare_exchange(
                0,
                1,
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
            )
            .is_ok()
        {
            return context.eval(boa_engine::Source::from_bytes(bytes));
        }
        if cache.get().is_none() && context.realm().scope().num_bindings() == 0 {
            let script =
                boa_engine::Script::parse(boa_engine::Source::from_bytes(bytes), None, context)?;
            if let Ok(compiled) = script.compile_to_slicebc_with_min_line(context, u32::MAX)
                && !compiled.is_empty()
            {
                let _ = cache.set(compiled);
            }
            return script.evaluate(context);
        }
    }
    let precompiled = cache.get_or_init(|| compile_static_js_to_slicebc(bytes));
    if !precompiled.is_empty()
        && let Some(script) = boa_engine::Script::from_slicebc_bytes(precompiled, context)
    {
        return script.evaluate(context);
    }
    context.eval(boa_engine::Source::from_bytes(bytes))
}

/// Evaluates `ULTRA_CORE_PRELUDE` inside `context` without cold throwaway `Context` allocation.
#[inline]
pub(crate) fn eval_ultra_core_prelude(
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<boa_engine::JsValue> {
    eval_static_js_with_2hit_slicebc_cache(
        &ULTRA_CORE_HITS,
        &ULTRA_CORE_SLICEBC,
        ULTRA_CORE_PRELUDE,
        context,
    )
}

/// Evaluates `LAZY_GETTERS_INIT_PRELUDE` inside `context` without cold throwaway `Context` allocation.
#[inline]
pub(crate) fn eval_lazy_getters_init_prelude(
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<boa_engine::JsValue> {
    eval_static_js_with_2hit_slicebc_cache(
        &LAZY_GETTERS_HITS,
        &LAZY_GETTERS_SLICEBC,
        LAZY_GETTERS_INIT_PRELUDE,
        context,
    )
}

/// Evaluates a compile-time minified lazy prelude shard (`1..=18` or `29`) on demand inside `context`
/// without cold throwaway `Context` allocation.
#[inline]
pub(crate) fn eval_lazy_prelude_shard(
    shard_id: u32,
    context: &mut boa_engine::Context,
) -> boa_engine::JsResult<boa_engine::JsValue> {
    if !((1..=18).contains(&shard_id) || shard_id == 29) {
        return Ok(boa_engine::JsValue::undefined());
    }
    register_shard_host_functions_for_mask(1u32 << shard_id, context);
    let idx = shard_id as usize;
    let bytes = shard_prelude_bytes(shard_id);
    eval_static_js_with_2hit_slicebc_cache(&SHARD_HITS[idx], &SHARD_SLICEBC[idx], bytes, context)
}
