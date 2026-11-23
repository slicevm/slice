//! Tier-1 Multi-Blob `MappedCasArchive` Module Loader (`__slice_cas_require`).
//!
//! Resolves relative, bare, stem-stripped, and `/index` module specifiers directly inside the
//! active memory-mapped `.cas` archive (`BOA_ACTIVE_CAS`) with zero OS `stat()` syscalls,
//! supporting pure Tier-0 `SLICESNP` exports, Tier-1 `SLICEBC\x01` / `SLICEBOA` modules,
//! diamond caching, and CommonJS circular `require()` cycles.

use crate::bindings::helpers::{
    consume_crypto_kdf_fuel, trip_isolate_oom_limits, try_consume_isolate_memory_ctx,
};
use crate::boa_realm::{BOA_ACTIVE_CAS, BOA_HTTP_REQ, REALM_CTL_SET_NEXT_TICK};
use crate::boacas::{
    BOACAS_FLAG_HAS_HTTP_BRIDGE, BOACAS_FLAG_HAS_KV_SANDBOX, BOACAS_FLAG_HAS_NEXT_TICK,
    BOACAS_FLAG_HAS_SEMVER_FAST, BOACAS_MAGIC, BoaCasHeader,
};
use crate::slicesnp_codec::{decode_slicesnp_import_cached, visit_pure_static_exports};
use crate::value_and_ast::{BytecodeOp, SNAPSHOT_MAGIC};
use boa_engine::property::PropertyDescriptor;
use boa_engine::{JsNativeError, JsObject, JsResult, JsValue as BoaVal, Source, js_string};
use compact_str::CompactString;
use std::borrow::Cow;

struct CasReferrerPopGuard {
    active: bool,
}

impl Drop for CasReferrerPopGuard {
    #[inline]
    fn drop(&mut self) {
        if self.active {
            BOA_ACTIVE_CAS.with(|slot| {
                if let Some(state) = slot.borrow_mut().as_mut() {
                    let _ = state.referrer_stack.pop();
                }
            });
        }
    }
}

#[inline]
fn cas_miss_sentinel(context: &mut boa_engine::Context) -> JsResult<BoaVal> {
    context
        .global_object()
        .get(js_string!("__slice_cas_miss"), context)
}

pub(crate) fn eval_cas_require_module(
    spec: &str,
    context: &mut boa_engine::Context,
) -> JsResult<BoaVal> {
    if spec.is_empty()
        || slice_cas::is_builtin_module(spec)
        || slice_cas::is_jsx_runtime_builtin(spec)
    {
        return cas_miss_sentinel(context);
    }

    let (archive, referrer) = match BOA_ACTIVE_CAS.with(|slot| {
        let borrow = slot.borrow();
        let state = borrow.as_ref()?;
        Some((state.archive.clone(), state.referrer_stack.last().cloned()))
    }) {
        Some(pair) => pair,
        None => return cas_miss_sentinel(context),
    };

    let Some(entry) = archive.resolve_specifier_from(referrer.as_deref(), spec) else {
        return cas_miss_sentinel(context);
    };

    if let Err(msg) = consume_crypto_kdf_fuel(2) {
        trip_isolate_oom_limits(context);
        return Err(JsNativeError::range().with_message(msg).into());
    }

    let cached_mod = BOA_ACTIVE_CAS.with(|slot| {
        let mut borrow = slot.borrow_mut();
        let state = borrow.as_mut()?;
        state.resolved_count += 1;
        state
            .module_cache
            .iter()
            .find(|(h, _)| h.as_str() == entry.hash_hex.as_str())
            .map(|(_, obj)| obj.clone())
    });
    if let Some(mod_obj) = cached_mod {
        return mod_obj.get(js_string!("exports"), context);
    }

    let exports_obj = JsObject::with_object_proto(context.intrinsics());
    let module_obj = JsObject::with_object_proto(context.intrinsics());
    module_obj.set(js_string!("exports"), exports_obj.clone(), false, context)?;

    let child_bc = archive.get_bytecode_slice(entry);

    // Zero-parse fast path when importing a pure numeric Tier-0 `SLICESNP` module from Tier-1 Boa
    if let Some(exp_iter) = visit_pure_static_exports(child_bc) {
        let _ = exports_obj.define_property_or_throw(
            js_string!("__esModule"),
            PropertyDescriptor::builder()
                .value(true)
                .writable(false)
                .enumerable(false)
                .configurable(false),
            context,
        );
        for (k, v) in exp_iter {
            exports_obj.set(js_string!(k), BoaVal::from(v), false, context)?;
        }
        BOA_ACTIVE_CAS.with(|slot| {
            if let Some(state) = slot.borrow_mut().as_mut() {
                state
                    .module_cache
                    .push((CompactString::new(&entry.hash_hex), module_obj));
            }
        });
        return Ok(BoaVal::from(exports_obj));
    }
    if child_bc.len() >= 9
        && &child_bc[0..8] == SNAPSHOT_MAGIC
        && let Some(child_mod) = decode_slicesnp_import_cached(child_bc)
        && child_mod.ops.len() == child_mod.static_exports.len()
        && child_mod
            .ops
            .iter()
            .all(|op| matches!(op, BytecodeOp::ConstNum { .. }))
    {
        let _ = exports_obj.define_property_or_throw(
            js_string!("__esModule"),
            PropertyDescriptor::builder()
                .value(true)
                .writable(false)
                .enumerable(false)
                .configurable(false),
            context,
        );
        for (k, v) in &child_mod.static_exports {
            exports_obj.set(js_string!(k.as_str()), BoaVal::from(*v), false, context)?;
        }
        BOA_ACTIVE_CAS.with(|slot| {
            if let Some(state) = slot.borrow_mut().as_mut() {
                state
                    .module_cache
                    .push((CompactString::new(&entry.hash_hex), module_obj));
            }
        });
        return Ok(BoaVal::from(exports_obj));
    }

    let raw_js: Cow<'_, str> = if child_bc.len() >= 8 && &child_bc[0..8] == BOACAS_MAGIC {
        if let Some((_, _, fallback_js)) = BoaCasHeader::decode(child_bc) {
            Cow::Borrowed(fallback_js)
        } else {
            Cow::Borrowed(archive.get_source_slice(entry))
        }
    } else if child_bc.len() >= 8 && &child_bc[0..8] == b"SLICEBOA" {
        let s = simdutf8::basic::from_utf8(&child_bc[8..]).unwrap_or_default();
        let unwrapped = s
            .strip_prefix("{\n")
            .and_then(|r| r.strip_suffix("\n}"))
            .unwrap_or(s);
        Cow::Borrowed(unwrapped)
    } else {
        Cow::Borrowed(archive.get_source_slice(entry))
    };

    let js_body: Cow<'_, str> = if slice_cas::needs_unbundled_esm_lowering(&raw_js) {
        Cow::Owned(slice_cas::lower_unbundled_esm(&raw_js))
    } else {
        raw_js
    };

    if let Err(msg) = try_consume_isolate_memory_ctx((js_body.len() / 2).max(32), context) {
        return Err(JsNativeError::range().with_message(msg).into());
    }

    ensure_cas_module_prerequisites(&js_body, context)?;

    BOA_ACTIVE_CAS.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            state
                .module_cache
                .push((CompactString::new(&entry.hash_hex), module_obj.clone()));
            state.referrer_stack.push(entry.original_specifier.clone());
        }
    });
    let _ref_guard = CasReferrerPopGuard { active: true };

    let global = context.global_object();
    let prev_cas_exports = global
        .get(js_string!("__slice_cas_exports"), context)
        .unwrap_or(BoaVal::undefined());
    let prev_cas_module = global
        .get(js_string!("__slice_cas_module"), context)
        .unwrap_or(BoaVal::undefined());
    global.set(
        js_string!("__slice_cas_exports"),
        exports_obj.clone(),
        false,
        context,
    )?;
    global.set(
        js_string!("__slice_cas_module"),
        module_obj.clone(),
        false,
        context,
    )?;

    let mut wrapped = String::with_capacity(js_body.len() + 44);
    wrapped.push_str("(function(exports, module) {\n");
    wrapped.push_str(&js_body);
    wrapped.push_str("\n})");

    let eval_res = context
        .eval(Source::from_bytes(wrapped.as_bytes()))
        .and_then(|factory_val| {
            let Some(factory_obj) = factory_val.as_object() else {
                return Ok(BoaVal::undefined());
            };
            factory_obj.call(
                &BoaVal::undefined(),
                &[
                    BoaVal::from(exports_obj.clone()),
                    BoaVal::from(module_obj.clone()),
                ],
                context,
            )
        });

    let _ = global.set(
        js_string!("__slice_cas_exports"),
        prev_cas_exports,
        false,
        context,
    );
    let _ = global.set(
        js_string!("__slice_cas_module"),
        prev_cas_module,
        false,
        context,
    );

    eval_res?;
    module_obj.get(js_string!("exports"), context)
}

fn ensure_cas_module_prerequisites(
    js_body: &str,
    context: &mut boa_engine::Context,
) -> JsResult<()> {
    let is_http = BOA_HTTP_REQ.with(|r| r.borrow().active);
    let (flags, full_mask) =
        crate::js_preludes::detect_shards_and_boacas_flags(js_body, is_http, false);
    if (flags & BOACAS_FLAG_HAS_KV_SANDBOX) != 0 {
        crate::host_bindings::register_kv_and_sandbox_host_functions(context);
    }
    if (flags & BOACAS_FLAG_HAS_SEMVER_FAST) != 0 {
        crate::host_bindings::register_semver_and_fast_intrinsics_host_functions(context);
    }
    if (flags & BOACAS_FLAG_HAS_HTTP_BRIDGE) != 0 {
        crate::host_bindings::register_http_bridge_host_functions(context);
    }
    let shard_bits = (full_mask & ((1u32 << 19) - 1)) | (full_mask & (1u32 << 29));
    let global = context.global_object();
    if shard_bits != 0
        && let Ok(ensure_val) = global.get(js_string!("__slice_ensure_shard"), context)
        && let Some(ensure_fn) = ensure_val.as_object()
    {
        for shard_id in 1u32..=18 {
            if (shard_bits & (1u32 << shard_id)) != 0 {
                ensure_fn.call(&BoaVal::undefined(), &[BoaVal::from(shard_id)], context)?;
            }
        }
        if (shard_bits & (1u32 << 29)) != 0 {
            ensure_fn.call(&BoaVal::undefined(), &[BoaVal::from(29u32)], context)?;
        }
    }
    if (flags & BOACAS_FLAG_HAS_NEXT_TICK) != 0
        && let Ok(ctl_val) = global.get(js_string!("__slice_ctl"), context)
        && let Some(ctl_fn) = ctl_val.as_object()
    {
        let _ = ctl_fn.call(
            &BoaVal::undefined(),
            &[BoaVal::from(REALM_CTL_SET_NEXT_TICK), BoaVal::from(true)],
            context,
        );
    }
    Ok(())
}
