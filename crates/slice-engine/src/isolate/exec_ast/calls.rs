//! Tier-0 Built-In Member/Global Calls (`Math`, `JSON`, `Array`, `String`, `WinterTC`)
//! and Unboxed `NumericOp` Function Invocation (`invoke_function_val`).

use super::{MAX_TIER0_RECURSION_DEPTH, collect_local_var_names};
use crate::isolate::{ControlFlow, SliceIsolate};
use crate::parser_and_compiler::expr_to_dotted_chain;
use crate::value_and_ast::{
    ExecutionTier, Expr, JsValue, js_math_max, js_math_min, js_math_round, js_to_int32,
    js_to_uint32, js_value_to_json_string, serde_json_to_js_value,
};
use slice_sandbox::SandboxViolation;

impl SliceIsolate {
    pub(crate) fn eval_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
    ) -> Result<JsValue, SandboxViolation> {
        let mut eval_args = Vec::with_capacity(args.len());
        for a in args {
            eval_args.push(self.eval_expr(a)?);
        }

        if let Expr::Member { object, property } = callee
            && let Some(res) = self.eval_member_call(object, property, args, &eval_args)?
        {
            return Ok(res);
        }

        if let Expr::Ident(fn_name) = callee {
            if let Some(user_fn @ JsValue::Function(_)) =
                self.env_vars.get(fn_name.as_str()).cloned()
            {
                return self.invoke_function_val(&user_fn, &eval_args);
            }
            if let Some(res) = self.eval_ident_builtin_call(fn_name, &eval_args)? {
                return Ok(res);
            }
        }

        let func_val = self.eval_expr(callee)?;
        self.invoke_function_val(&func_val, &eval_args)
    }

    #[inline(never)]
    fn eval_member_call(
        &mut self,
        object: &Expr,
        property: &str,
        raw_args: &[Expr],
        eval_args: &[JsValue],
    ) -> Result<Option<JsValue>, SandboxViolation> {
        let chain = expr_to_dotted_chain(object);
        let clean_chain = chain.strip_prefix("globalThis.").unwrap_or(chain.as_str());
        match (clean_chain, property) {
            ("Object", "assign" | "defineProperty" | "defineProperties" | "setPrototypeOf")
            | ("Reflect", "set" | "defineProperty" | "deleteProperty" | "setPrototypeOf") => {
                if let Some(first_raw) = raw_args.first() {
                    let tgt = expr_to_dotted_chain(first_raw);
                    if !tgt.is_empty() {
                        self.compartment.verify_property_mutation(&tgt)?;
                    }
                }
            }
            _ => {}
        }
        match (clean_chain, property) {
            ("console", "log" | "info" | "warn" | "error") => {
                let formatted: Vec<String> = eval_args.iter().map(|v| v.to_string()).collect();
                let line = formatted.join(" ");
                self.stdout.push(line);
                return Ok(Some(JsValue::Undefined));
            }
            ("Math", method) => {
                let x = eval_args.first().map(|v| v.to_number()).unwrap_or(f64::NAN);
                let y = eval_args.get(1).map(|v| v.to_number()).unwrap_or(f64::NAN);
                let res = match method {
                    "abs" => x.abs(),
                    "floor" => x.floor(),
                    "ceil" => x.ceil(),
                    "round" => js_math_round(x),
                    "trunc" => x.trunc(),
                    "sign" => {
                        if x == 0.0 || x.is_nan() {
                            x
                        } else if x > 0.0 {
                            1.0
                        } else {
                            -1.0
                        }
                    }
                    "sqrt" => x.sqrt(),
                    "pow" => {
                        if eval_args.len() < 2 || y.is_nan() || (y.is_infinite() && x.abs() == 1.0)
                        {
                            f64::NAN
                        } else {
                            x.powf(y)
                        }
                    }
                    "sin" => x.sin(),
                    "cos" => x.cos(),
                    "max" => eval_args
                        .iter()
                        .map(|v| v.to_number())
                        .fold(f64::NEG_INFINITY, js_math_max),
                    "min" => eval_args
                        .iter()
                        .map(|v| v.to_number())
                        .fold(f64::INFINITY, js_math_min),
                    "imul" => {
                        (js_to_int32(x).wrapping_mul(js_to_int32(
                            eval_args.get(1).map(|v| v.to_number()).unwrap_or(0.0),
                        ))) as f64
                    }
                    "clz32" => {
                        (js_to_uint32(eval_args.first().map(|v| v.to_number()).unwrap_or(0.0))
                            .leading_zeros()) as f64
                    }
                    "fround" => (x as f32) as f64,
                    _ => f64::NAN,
                };
                return Ok(Some(JsValue::Number(res)));
            }
            ("JSON", "stringify") => {
                let first = eval_args.first().cloned().unwrap_or(JsValue::Undefined);
                let s = js_value_to_json_string(&first);
                self.compartment.alloc_memory(s.len())?;
                return Ok(Some(JsValue::str(s)));
            }
            ("JSON", "parse") => {
                if let Some(JsValue::Str(s)) = eval_args.first() {
                    match serde_json::from_str::<serde_json::Value>(s) {
                        Ok(val) => {
                            self.compartment.alloc_memory(s.len())?;
                            return Ok(Some(serde_json_to_js_value(&val)));
                        }
                        Err(e) => {
                            self.pending_tier0_throw = Some(format!(
                                "Uncaught SyntaxError: {}\n    at <main> (unknown at :1:12)",
                                e
                            ));
                            self.last_exit_code = Some(1);
                            return Ok(Some(JsValue::Undefined));
                        }
                    }
                }
                self.pending_tier0_throw = Some(
                    "Uncaught SyntaxError: Unexpected token in JSON\n    at <main> (unknown at :1:12)"
                        .to_string(),
                );
                self.last_exit_code = Some(1);
                return Ok(Some(JsValue::Undefined));
            }
            ("Object", "keys") => {
                if let Some(JsValue::Object(map)) = eval_args.first() {
                    let mut keys: Vec<String> = map.keys().cloned().collect();
                    let bytes: usize = keys.iter().map(|k| k.len().saturating_add(16)).sum();
                    self.compartment.alloc_memory(bytes)?;
                    keys.sort();
                    return Ok(Some(JsValue::array(
                        keys.into_iter().map(JsValue::str).collect(),
                    )));
                }
                return Ok(Some(JsValue::array(Vec::new())));
            }
            ("Object", "freeze") => {
                return Ok(Some(
                    eval_args.first().cloned().unwrap_or(JsValue::Undefined),
                ));
            }
            ("Array", "isArray") => {
                return Ok(Some(JsValue::Bool(matches!(
                    eval_args.first(),
                    Some(JsValue::Array(_))
                ))));
            }
            ("fs", "readFileSync" | "read") => {
                let path = eval_args.first().map(|v| v.to_string()).unwrap_or_default();
                self.compartment.verify_fs_read(&path)?;
                return Ok(Some(JsValue::str(format!("CAS_VIRTUAL_FILE[{}]", path))));
            }
            ("WinterTC", "route") => {
                let method = eval_args
                    .first()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "GET".to_string());
                let url = eval_args
                    .get(1)
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "https://edge.slice.local/".to_string());
                return self.exec_wintertc_route(&method, &url).map(Some);
            }
            ("Slice", "numericLoop") => {
                let n = eval_args.first().map(|v| v.to_number() as u64).unwrap_or(0);
                return self.exec_numeric_loop_kernel(n).map(Some);
            }
            ("crypto.subtle", "digest") => {
                let payload = eval_args
                    .first()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "payload".to_string());
                return self.exec_crypto_digest(&payload, 1).map(Some);
            }
            _ => {}
        }

        if let Expr::Ident(var_name) = object {
            if property == "push" {
                let added_bytes = eval_args.len().saturating_mul(16);
                if added_bytes > 0 {
                    self.compartment.alloc_memory(added_bytes)?;
                }
                if let Some(JsValue::Array(vec)) = self.env_vars.get_mut(var_name.as_str()) {
                    for item in eval_args {
                        vec.push(item.clone());
                    }
                    return Ok(Some(JsValue::Number(vec.len() as f64)));
                }
            } else if property == "pop"
                && let Some(JsValue::Array(vec)) = self.env_vars.get_mut(var_name.as_str())
            {
                return Ok(Some(vec.pop().unwrap_or(JsValue::Undefined)));
            }
        }

        let target_val = self.eval_expr(object)?;
        match (target_val, property) {
            (JsValue::Array(items), "join") => {
                let sep = match eval_args.first() {
                    None | Some(JsValue::Undefined) => ",".to_string(),
                    Some(v) => v.to_string(),
                };
                let mut parts = Vec::with_capacity(items.len());
                let mut total_bytes = items.len().saturating_sub(1).saturating_mul(sep.len());
                for v in items.iter() {
                    let s = match v {
                        JsValue::Null | JsValue::Undefined => String::new(),
                        other => other.to_string(),
                    };
                    total_bytes = total_bytes.saturating_add(s.len());
                    parts.push(s);
                }
                self.compartment.alloc_memory(total_bytes)?;
                Ok(Some(JsValue::str(parts.join(&sep))))
            }
            (JsValue::Array(items), "includes") => {
                let needle = eval_args.first().cloned().unwrap_or(JsValue::Undefined);
                Ok(Some(JsValue::Bool(
                    items.iter().any(|item| item.same_value_zero(&needle)),
                )))
            }
            (JsValue::Array(items), "map") => {
                if let Some(cb) = eval_args.first() {
                    let mut out = Vec::with_capacity(items.len());
                    for (idx, item) in items.into_iter().enumerate() {
                        out.push(
                            self.invoke_function_val(cb, &[item, JsValue::Number(idx as f64)])?,
                        );
                    }
                    return Ok(Some(JsValue::array(out)));
                }
                Ok(None)
            }
            (JsValue::Array(items), "filter") => {
                if let Some(cb) = eval_args.first() {
                    let mut out = Vec::new();
                    for (idx, item) in items.into_iter().enumerate() {
                        let keep = self.invoke_function_val(
                            cb,
                            &[item.clone(), JsValue::Number(idx as f64)],
                        )?;
                        if keep.is_truthy() {
                            out.push(item);
                        }
                    }
                    return Ok(Some(JsValue::array(out)));
                }
                Ok(None)
            }
            (JsValue::Array(items), "reduce") => {
                if let Some(cb) = eval_args.first() {
                    let mut iter = items.into_iter().enumerate();
                    let mut acc = if let Some(init_val) = eval_args.get(1) {
                        init_val.clone()
                    } else if let Some((_, first_item)) = iter.next() {
                        first_item
                    } else {
                        return Ok(Some(JsValue::Undefined));
                    };
                    for (idx, item) in iter {
                        acc = self
                            .invoke_function_val(cb, &[acc, item, JsValue::Number(idx as f64)])?;
                    }
                    return Ok(Some(acc));
                }
                Ok(None)
            }
            (JsValue::Str(s), "toUpperCase") => Ok(Some(JsValue::str(s.to_uppercase()))),
            (JsValue::Str(s), "toLowerCase") => Ok(Some(JsValue::str(s.to_lowercase()))),
            (JsValue::Str(s), "trim") => Ok(Some(JsValue::str(s.trim().to_string()))),
            (JsValue::Str(s), "includes") => {
                let needle = eval_args.first().map(|v| v.to_string()).unwrap_or_default();
                Ok(Some(JsValue::Bool(s.contains(&needle))))
            }
            (JsValue::Str(s), "split") => {
                let sep = eval_args.first().map(|v| v.to_string()).unwrap_or_default();
                let mut parts = Vec::new();
                for p in s.split(&sep) {
                    self.compartment.alloc_memory(p.len().saturating_add(16))?;
                    parts.push(JsValue::str(p.to_string()));
                }
                Ok(Some(JsValue::array(parts)))
            }
            _ => Ok(None),
        }
    }

    #[inline(never)]
    fn eval_ident_builtin_call(
        &mut self,
        fn_name: &str,
        eval_args: &[JsValue],
    ) -> Result<Option<JsValue>, SandboxViolation> {
        match fn_name {
            "print" => {
                let formatted: Vec<String> = eval_args.iter().map(|v| v.to_string()).collect();
                let line = formatted.join(" ");
                self.stdout.push(line);
                Ok(Some(JsValue::Undefined))
            }
            "fetch" => {
                let url = eval_args.first().map(|v| v.to_string()).unwrap_or_default();
                let stripped = url
                    .trim_start_matches("https://")
                    .trim_start_matches("http://");
                let (host, path) = stripped.split_once('/').unwrap_or((stripped, ""));
                self.compartment.verify_net_connect(host)?;
                self.verify_egress_not_leaked(&url)?;
                if let Some(arg2) = eval_args.get(1) {
                    self.verify_egress_not_leaked(&arg2.to_string())?;
                }
                Ok(Some(JsValue::str(format!(
                    "HTTP/2 200 OK from {}/{}",
                    host, path
                ))))
            }
            "env" => {
                let key = eval_args.first().map(|v| v.to_string()).unwrap_or_default();
                let val = self.compartment.verify_env_read(&key)?.to_string();
                Ok(Some(JsValue::str(val)))
            }
            "Number" | "parseFloat" => {
                let n = eval_args.first().map(|v| v.to_number()).unwrap_or(0.0);
                Ok(Some(JsValue::Number(n)))
            }
            "parseInt" => {
                let n = eval_args
                    .first()
                    .map(|v| v.to_number().trunc())
                    .unwrap_or(f64::NAN);
                Ok(Some(JsValue::Number(n)))
            }
            "isNaN" => {
                let n = eval_args.first().map(|v| v.to_number()).unwrap_or(f64::NAN);
                Ok(Some(JsValue::Bool(n.is_nan())))
            }
            "isFinite" => {
                let n = eval_args.first().map(|v| v.to_number()).unwrap_or(f64::NAN);
                Ok(Some(JsValue::Bool(n.is_finite())))
            }
            "String" => {
                let s = eval_args.first().map(|v| v.to_string()).unwrap_or_default();
                Ok(Some(JsValue::str(s)))
            }
            "Boolean" => {
                let b = eval_args.first().map(|v| v.is_truthy()).unwrap_or(false);
                Ok(Some(JsValue::Bool(b)))
            }
            _ => Ok(None),
        }
    }

    pub(crate) fn invoke_function_val(
        &mut self,
        func_val: &JsValue,
        args: &[JsValue],
    ) -> Result<JsValue, SandboxViolation> {
        if let JsValue::Function(func) = func_val {
            // If this function is a pure numeric kernel compiled into `NumericOp` and all arguments
            // are numbers/booleans, execute directly on the unboxed 64-bit register VM!
            if let Some(ref num_ir) = func.compiled_numeric
                && args.len() >= func.params.len()
                && args
                    .iter()
                    .all(|a| matches!(a, JsValue::Number(_) | JsValue::Bool(_)))
            {
                let mut unboxed_args = [0.0f64; 6];
                for (idx, a) in args.iter().take(6).enumerate() {
                    unboxed_args[idx] = a.to_number();
                }
                let remaining_fuel = self
                    .compartment
                    .fuel_limit
                    .saturating_sub(self.compartment.fuel_consumed);
                let mut steps: u64 = 0;
                match num_ir.execute(
                    &unboxed_args[..args.len().min(6)],
                    &mut steps,
                    remaining_fuel,
                    0,
                    &self.compartment.tenant_id,
                ) {
                    Ok(Some(val)) => {
                        let fuel_cost = (steps / 64).max(1);
                        self.compartment.consume_fuel(fuel_cost)?;
                        self.current_tier = ExecutionTier::Tier2PeakNative;
                        return Ok(JsValue::Number(val));
                    }
                    Ok(None) => {}
                    Err(err) => {
                        let _ = self
                            .compartment
                            .consume_fuel(remaining_fuel.saturating_add(1));
                        return Err(err);
                    }
                }
            }

            if self.recursion_depth >= MAX_TIER0_RECURSION_DEPTH {
                let remaining = self
                    .compartment
                    .fuel_limit
                    .saturating_sub(self.compartment.fuel_consumed);
                let _ = self.compartment.consume_fuel(remaining.saturating_add(1));
                return Err(SandboxViolation::CpuFuelExhausted {
                    tenant_id: self.compartment.tenant_id.clone(),
                    limit: self.compartment.fuel_limit,
                });
            }

            self.compartment.consume_fuel(2)?;
            self.recursion_depth += 1;

            // Save shadowed parameter and local variable bindings so recursive/helper calls
            // never clobber caller frame variables of the same name.
            let mut local_names = Vec::new();
            collect_local_var_names(&func.body, &func.params, &mut local_names);
            let mut saved = Vec::with_capacity(func.params.len() + local_names.len());
            for (idx, param_name) in func.params.iter().enumerate() {
                let prev = self.env_vars.get(param_name.as_str()).cloned();
                saved.push((compact_str::CompactString::new(param_name), prev));
                let arg_val = args.get(idx).cloned().unwrap_or(JsValue::Undefined);
                self.set_env_var_str(param_name, arg_val);
            }
            for local_name in local_names {
                let prev = self.env_vars.get(local_name.as_str()).cloned();
                saved.push((compact_str::CompactString::new(&local_name), prev));
            }

            let block_res = self.eval_block(&func.body);
            self.recursion_depth = self.recursion_depth.saturating_sub(1);

            for (var_name, prev_opt) in saved {
                if let Some(prev) = prev_opt {
                    self.env_vars.insert(var_name, prev);
                } else {
                    self.env_vars.remove(var_name.as_str());
                }
            }

            let res = match block_res? {
                ControlFlow::Return(v) => v,
                ControlFlow::None(_) | ControlFlow::Break | ControlFlow::Continue => {
                    JsValue::Undefined
                }
            };
            Ok(res)
        } else {
            Ok(JsValue::Undefined)
        }
    }
}
