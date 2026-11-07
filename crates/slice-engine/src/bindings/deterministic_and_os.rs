//! Deterministic Replay (`__slice_det_random`, `__slice_det_now`), `os.tmpdir`, and `AsyncLocalStorage` Host Bindings.

use crate::boa_realm::{BOA_ALS_ACTIVE, BOA_ALS_SNAP, BOA_DET_STATE};
use crate::state_and_kv::splitmix64_next_f64;

pub(crate) fn register_deterministic_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsValue as BoaVal, NativeFunction};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_det_random"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_det_random"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let val = BOA_DET_STATE.with(|s| {
                if let Some((mut prng, clk)) = s.get() {
                    let r = splitmix64_next_f64(&mut prng);
                    s.set(Some((prng, clk)));
                    r
                } else {
                    -1.0
                }
            });
            Ok(BoaVal::from(val))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_det_now"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let val = BOA_DET_STATE.with(|s| {
                if let Some((prng, clk)) = s.get() {
                    let next_clk = clk.wrapping_add(1);
                    s.set(Some((prng, next_clk)));
                    next_clk as f64
                } else {
                    -1.0
                }
            });
            Ok(BoaVal::from(val))
        }),
    );
}

pub(crate) fn register_os_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsValue as BoaVal, NativeFunction};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_os_tmpdir"), ctx)
        .unwrap_or(false)
    {
        return;
    }
    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_os_tmpdir"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let tmp = std::env::temp_dir().to_string_lossy().replace('\\', "/");
            Ok(BoaVal::from(js_string!(tmp)))
        }),
    );
}

pub(crate) fn register_async_hooks_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsValue as BoaVal, NativeFunction};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_als_get"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_als_get"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            Ok(BOA_ALS_SNAP.with(|s| s.borrow().clone()))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_als_set"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let next = args.first().cloned().unwrap_or_else(BoaVal::undefined);
            BOA_ALS_ACTIVE.with(|a| a.set(true));
            let prev = BOA_ALS_SNAP.with(|s| s.replace(next));
            Ok(prev)
        }),
    );
}
