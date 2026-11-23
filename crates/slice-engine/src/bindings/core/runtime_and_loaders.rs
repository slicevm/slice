//! Core Telemetry, Console Output, Memory/Fuel Accounting, Lazy Builtins/Preludes, and SES Signatures.

use crate::bindings::helpers::{
    consume_crypto_kdf_fuel, trip_isolate_oom_limits, try_consume_isolate_memory_ctx,
};
use crate::boa_realm::BOA_STDOUT;
use crate::js_preludes::eval_lazy_prelude_shard;
use crate::node_builtins::eval_lazy_builtin_module;

pub(super) fn register_runtime_and_loader_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{JsNativeError, JsValue as BoaVal, NativeFunction, js_string};

    static PERF_ORIGIN: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_perf_now"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let origin = PERF_ORIGIN.get_or_init(std::time::Instant::now);
            let elapsed_ms = origin.elapsed().as_secs_f64() * 1000.0
                + (crate::event_loop::virtual_time_warped_ms() as f64);
            Ok(BoaVal::from(elapsed_ms))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_console_log"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let line = if args.len() == 1 {
                let first = &args[0];
                if let Some(s) = first.as_string() {
                    s.to_std_string_escaped()
                } else {
                    first.to_string(context)?.to_std_string_escaped()
                }
            } else if args.is_empty() {
                String::new()
            } else {
                let mut joined = String::with_capacity(64);
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        joined.push(' ');
                    }
                    if let Some(s) = arg.as_string() {
                        joined.push_str(&s.to_std_string_escaped());
                    } else {
                        joined.push_str(&arg.to_string(context)?.to_std_string_escaped());
                    }
                }
                joined
            };
            if let Err(msg) = try_consume_isolate_memory_ctx(line.len().max(8), context) {
                return Err(JsNativeError::range().with_message(msg).into());
            }
            if let Err(msg) = consume_crypto_kdf_fuel(((line.len() as u64) / 32).saturating_add(2))
            {
                trip_isolate_oom_limits(context);
                return Err(JsNativeError::range().with_message(msg).into());
            }
            BOA_STDOUT.with(|out| out.borrow_mut().push(line));
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_charge_mem"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let bytes = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as usize)
                .unwrap_or(0);
            let fuel = args
                .get(1)
                .and_then(|v| v.as_number())
                .map(|n| n.max(0.0) as u64)
                .unwrap_or_else(|| ((bytes as u64) / 64).max(1));
            if let Err(msg) = try_consume_isolate_memory_ctx(bytes, context) {
                return Err(JsNativeError::range().with_message(msg).into());
            }
            if let Err(msg) = consume_crypto_kdf_fuel(fuel) {
                trip_isolate_oom_limits(context);
                return Err(JsNativeError::range().with_message(msg).into());
            }
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_map_error_stack"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let raw = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let mapped = crate::boa_realm::map_stack_with_active_source(&raw);
            Ok(BoaVal::from(js_string!(mapped)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_load_builtin"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let id = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            eval_lazy_builtin_module(&id, context)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_cas_require"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let id = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            crate::bindings::cas_loader::eval_cas_require_module(&id, context)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_lazy_prelude"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let shard_id = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u32)
                .unwrap_or(0);
            if shard_id == 18 {
                let c = context.intrinsics().constructors();
                let pre_str = crate::boa_realm::proto_sig(&c.string().prototype());
                let pre_num = crate::boa_realm::proto_sig(&c.number().prototype());
                let pre_date = crate::boa_realm::proto_sig(&c.date().prototype());
                let res = eval_lazy_prelude_shard(shard_id, context)?;
                let c = context.intrinsics().constructors();
                let post_str = crate::boa_realm::proto_sig(&c.string().prototype());
                let post_num = crate::boa_realm::proto_sig(&c.number().prototype());
                let post_date = crate::boa_realm::proto_sig(&c.date().prototype());
                crate::boa_realm::BOA_SES_INTL_SIG_PAIR.with(|p| {
                    p.set(Some([
                        (pre_str, post_str),
                        (pre_num, post_num),
                        (pre_date, post_date),
                    ]))
                });
                return Ok(res);
            }
            eval_lazy_prelude_shard(shard_id, context)
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_capture_ses_sigs"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let mode = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as i32)
                .unwrap_or(0);
            match mode {
                0 => {
                    let sigs = crate::boa_realm::capture_ses_proto_sigs(context);
                    crate::boa_realm::BOA_SES_BASELINE_SIGS.with(|s| s.set(Some(sigs)));
                    Ok(BoaVal::undefined())
                }
                1 => {
                    let c = context.intrinsics().constructors();
                    let cur_str = crate::boa_realm::proto_sig(&c.string().prototype());
                    let cur_num = crate::boa_realm::proto_sig(&c.number().prototype());
                    let cur_date = crate::boa_realm::proto_sig(&c.date().prototype());
                    let mask = crate::boa_realm::BOA_SES_BASELINE_SIGS.with(|s| {
                        s.get().map_or(0i32, |sigs| {
                            (if sigs[3] == cur_str { 1 } else { 0 })
                                | (if sigs[6] == cur_num { 2 } else { 0 })
                                | (if sigs[12] == cur_date { 4 } else { 0 })
                        })
                    });
                    Ok(BoaVal::from(mask))
                }
                m if (m & 16) != 0 => {
                    let c = context.intrinsics().constructors();
                    let cur_str = crate::boa_realm::proto_sig(&c.string().prototype());
                    let cur_num = crate::boa_realm::proto_sig(&c.number().prototype());
                    let cur_date = crate::boa_realm::proto_sig(&c.date().prototype());
                    crate::boa_realm::BOA_SES_BASELINE_SIGS.with(|s| {
                        if let Some(mut sigs) = s.get() {
                            if (m & 1) != 0 {
                                sigs[3] = cur_str;
                            }
                            if (m & 2) != 0 {
                                sigs[6] = cur_num;
                            }
                            if (m & 4) != 0 {
                                sigs[12] = cur_date;
                            }
                            s.set(Some(sigs));
                        }
                    });
                    Ok(BoaVal::undefined())
                }
                _ => Ok(BoaVal::undefined()),
            }
        }),
    );
}
