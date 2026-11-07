//! Native Semver (`slice:semver`) and Fast String/Number Formatting Intrinsics (`slice:ms`, `bytes`).

use crate::intrinsics::{
    fast_format_bytes, fast_format_ms, fast_parse_bytes, fast_parse_ms, semver_coerce_xyz,
    semver_parse_strict_xyz, semver_satisfies_xyz,
};

pub(crate) fn register_semver_and_fast_intrinsics_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsValue as BoaVal, NativeFunction};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_semver_valid"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_semver_valid"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::null());
            };
            if first.is_null() || first.is_undefined() {
                return Ok(BoaVal::null());
            }
            let s = if let Some(obj) = first.as_object() {
                if let Ok(v) = obj.get(js_string!("version"), context) {
                    v.to_string(context)?.to_std_string_escaped()
                } else {
                    first.to_string(context)?.to_std_string_escaped()
                }
            } else {
                first.to_string(context)?.to_std_string_escaped()
            };
            match semver_parse_strict_xyz(&s) {
                Some((maj, min, pat)) => {
                    Ok(BoaVal::from(js_string!(format!("{}.{}.{}", maj, min, pat))))
                }
                None => Ok(BoaVal::null()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_semver_coerce"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::null());
            };
            let s = first.to_string(context)?.to_std_string_escaped();
            match semver_coerce_xyz(&s) {
                Some((maj, min, pat)) => {
                    Ok(BoaVal::from(js_string!(format!("{}.{}.{}", maj, min, pat))))
                }
                None => Ok(BoaVal::null()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_semver_gt"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let a = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let b = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if let (Some(va), Some(vb)) = (semver_parse_strict_xyz(&a), semver_parse_strict_xyz(&b))
            {
                Ok(BoaVal::from(va > vb))
            } else {
                Ok(BoaVal::from(false))
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_semver_satisfies"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let ver = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let rng = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            match semver_satisfies_xyz(&ver, &rng) {
                Some(true) => Ok(BoaVal::from(1)),
                Some(false) => Ok(BoaVal::from(0)),
                None => Ok(BoaVal::from(-1)),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fast_ms"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::undefined());
            };
            if let Some(s) = first.as_string() {
                let raw = s.to_std_string_escaped();
                if let Some(ms_val) = fast_parse_ms(&raw) {
                    return Ok(BoaVal::from(ms_val));
                }
            } else if let Some(n) = first.as_number() {
                let long = args.get(1).and_then(|v| v.as_boolean()).unwrap_or(false);
                if let Some(formatted) = fast_format_ms(n, long) {
                    return Ok(BoaVal::from(js_string!(formatted)));
                }
            }
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fast_bytes_parse"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            let Some(first) = args.first() else {
                return Ok(BoaVal::undefined());
            };
            if let Some(n) = first.as_number() {
                return Ok(BoaVal::from(n));
            }
            if let Some(s) = first.as_string() {
                let raw = s.to_std_string_escaped();
                if let Some(b_val) = fast_parse_bytes(&raw) {
                    return Ok(BoaVal::from(b_val));
                }
            }
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fast_bytes_format"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, _context| {
            if args
                .get(1)
                .is_some_and(|v| !v.is_undefined() && !v.is_null())
            {
                return Ok(BoaVal::undefined());
            }
            let Some(n) = args.first().and_then(|v| v.as_number()) else {
                return Ok(BoaVal::undefined());
            };
            if let Some(formatted) = fast_format_bytes(n) {
                return Ok(BoaVal::from(js_string!(formatted)));
            }
            Ok(BoaVal::undefined())
        }),
    );
}
