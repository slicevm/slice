//! Tier-0 Expression, Binary/Assignment Operator, and Property/Index Mutation Evaluator.

use super::as_canonical_array_index;
use crate::isolate::SliceIsolate;
use crate::value_and_ast::{BinOp, Expr, JsFunction, JsValue, UnaryOp, js_to_int32, js_to_uint32};
use rustc_hash::FxHashMap;
use slice_sandbox::SandboxViolation;

impl SliceIsolate {
    pub(crate) fn eval_expr(&mut self, expr: &Expr) -> Result<JsValue, SandboxViolation> {
        self.compartment.consume_fuel(1)?;
        if self.pending_tier0_throw.is_some() {
            return Ok(JsValue::Undefined);
        }
        match expr {
            Expr::Literal(v) => Ok(v.clone()),
            Expr::Ident(name) => Ok(self
                .env_vars
                .get(name.as_str())
                .cloned()
                .unwrap_or(JsValue::Undefined)),
            Expr::Unary { op, expr: inner } => {
                let val = self.eval_expr(inner)?;
                match op {
                    UnaryOp::Neg => Ok(JsValue::Number(-val.to_number())),
                    UnaryOp::Pos => Ok(JsValue::Number(val.to_number())),
                    UnaryOp::Not => Ok(JsValue::Bool(!val.is_truthy())),
                    UnaryOp::BitNot => Ok(JsValue::Number((!js_to_int32(val.to_number())) as f64)),
                    UnaryOp::TypeOf => Ok(JsValue::str(val.type_of().to_string())),
                }
            }
            Expr::Binary { op, lhs, rhs } => {
                if *op == BinOp::And {
                    let l = self.eval_expr(lhs)?;
                    if !l.is_truthy() {
                        return Ok(l);
                    }
                    return self.eval_expr(rhs);
                }
                if *op == BinOp::Or {
                    let l = self.eval_expr(lhs)?;
                    if l.is_truthy() {
                        return Ok(l);
                    }
                    return self.eval_expr(rhs);
                }
                let l = self.eval_expr(lhs)?;
                let r = self.eval_expr(rhs)?;
                self.eval_binary_op(*op, l, r)
            }
            Expr::Assign { name, op, value } => {
                let rhs = self.eval_expr(value)?;
                let final_val = if let Some(bin_op) = op {
                    let prev = self
                        .env_vars
                        .get(name.as_str())
                        .cloned()
                        .unwrap_or(JsValue::Undefined);
                    self.eval_assign_op(*bin_op, prev, rhs)?
                } else {
                    rhs
                };
                self.set_env_var_str(name, final_val.clone());
                Ok(final_val)
            }
            Expr::PropAssign {
                target_chain,
                prop,
                value,
            } => {
                self.compartment.verify_property_mutation(target_chain)?;
                let val = self.eval_expr(value)?;
                if let Some(JsValue::Object(map)) = self.env_vars.get_mut(target_chain.as_str()) {
                    map.insert(prop.clone(), val.clone());
                }
                Ok(val)
            }
            Expr::IndexAssign {
                target,
                index,
                value,
            } => {
                let idx_val = self.eval_expr(index)?;
                let val = self.eval_expr(value)?;
                self.apply_index_assign(target, idx_val, val)
            }
            Expr::Update {
                name,
                increment,
                prefix,
            } => {
                let prev = self
                    .env_vars
                    .get(name.as_str())
                    .map(|v| v.to_number())
                    .unwrap_or(f64::NAN);
                let next = if *increment { prev + 1.0 } else { prev - 1.0 };
                self.set_env_var_str(name, JsValue::Number(next));
                Ok(JsValue::Number(if *prefix { next } else { prev }))
            }
            Expr::Ternary {
                cond,
                then_expr,
                else_expr,
            } => {
                if self.eval_expr(cond)?.is_truthy() {
                    self.eval_expr(then_expr)
                } else {
                    self.eval_expr(else_expr)
                }
            }
            Expr::ArrayLit(items) => {
                let mut vec = Vec::with_capacity(items.len());
                for item in items {
                    vec.push(self.eval_expr(item)?);
                }
                self.compartment.alloc_memory(vec.len() * 16)?;
                Ok(JsValue::array(vec))
            }
            Expr::ObjectLit(props) => {
                let mut map = FxHashMap::with_capacity_and_hasher(props.len(), Default::default());
                for (k, v) in props {
                    map.insert(k.clone(), self.eval_expr(v)?);
                }
                self.compartment.alloc_memory(map.len() * 32)?;
                Ok(JsValue::Object(Box::new(map)))
            }
            Expr::ArrowFunc { params, body } => {
                let mut js_fn = JsFunction {
                    name: String::new(),
                    params: params.clone(),
                    body: body.clone(),
                    compiled_numeric: None,
                };
                js_fn.compiled_numeric = crate::numeric_vm::try_compile_numeric_func(&js_fn);
                Ok(JsValue::Function(Box::new(js_fn)))
            }
            Expr::Member { object, property } => {
                if let Expr::Ident(obj_name) = &**object
                    && obj_name == "Math"
                {
                    return Ok(match property.as_str() {
                        "PI" => JsValue::Number(std::f64::consts::PI),
                        "E" => JsValue::Number(std::f64::consts::E),
                        "SQRT2" => JsValue::Number(std::f64::consts::SQRT_2),
                        "LN2" => JsValue::Number(std::f64::consts::LN_2),
                        "LN10" => JsValue::Number(std::f64::consts::LN_10),
                        _ => JsValue::Undefined,
                    });
                }
                let target = self.eval_expr(object)?;
                match target {
                    JsValue::Array(a) if property == "length" => {
                        Ok(JsValue::Number(a.len() as f64))
                    }
                    JsValue::Str(s) if property == "length" => Ok(JsValue::Number(s.len() as f64)),
                    JsValue::Object(map) => {
                        Ok(map.get(property).cloned().unwrap_or(JsValue::Undefined))
                    }
                    _ => Ok(JsValue::Undefined),
                }
            }
            Expr::Index { object, index } => {
                let target = self.eval_expr(object)?;
                let idx = self.eval_expr(index)?;
                match target {
                    JsValue::Array(a) => {
                        if let Some(i) = as_canonical_array_index(&idx) {
                            Ok(a.get(i).cloned().unwrap_or(JsValue::Undefined))
                        } else {
                            Ok(JsValue::Undefined)
                        }
                    }
                    JsValue::Str(s) => {
                        let idx_f = idx.to_number();
                        if idx_f >= 0.0 && idx_f.fract() == 0.0 {
                            Ok(s.chars()
                                .nth(idx_f as usize)
                                .map(|c| JsValue::str(c.to_string()))
                                .unwrap_or(JsValue::Undefined))
                        } else {
                            Ok(JsValue::Undefined)
                        }
                    }
                    JsValue::Object(map) => Ok(map
                        .get(&idx.to_string())
                        .cloned()
                        .unwrap_or(JsValue::Undefined)),
                    _ => Ok(JsValue::Undefined),
                }
            }
            Expr::Call { callee, args } => self.eval_call(callee, args),
        }
    }

    #[inline(never)]
    fn eval_binary_op(
        &mut self,
        op: BinOp,
        l: JsValue,
        r: JsValue,
    ) -> Result<JsValue, SandboxViolation> {
        match op {
            BinOp::Add => match (&l, &r) {
                (JsValue::Str(sa), _) => {
                    let s = format!("{}{}", sa, r);
                    self.compartment.alloc_memory(s.len())?;
                    Ok(JsValue::str(s))
                }
                (_, JsValue::Str(sb)) => {
                    let s = format!("{}{}", l, sb);
                    self.compartment.alloc_memory(s.len())?;
                    Ok(JsValue::str(s))
                }
                _ => Ok(JsValue::Number(l.to_number() + r.to_number())),
            },
            BinOp::Sub => Ok(JsValue::Number(l.to_number() - r.to_number())),
            BinOp::Mul => Ok(JsValue::Number(l.to_number() * r.to_number())),
            BinOp::Div => Ok(JsValue::Number(l.to_number() / r.to_number())),
            BinOp::Rem => Ok(JsValue::Number(l.to_number() % r.to_number())),
            BinOp::Pow => Ok(JsValue::Number(l.to_number().powf(r.to_number()))),
            BinOp::Eq => Ok(JsValue::Bool(l.abstract_eq(&r))),
            BinOp::Neq => Ok(JsValue::Bool(!l.abstract_eq(&r))),
            BinOp::StrictEq => Ok(JsValue::Bool(l == r)),
            BinOp::StrictNeq => Ok(JsValue::Bool(l != r)),
            BinOp::Lt => Ok(JsValue::Bool(l.to_number() < r.to_number())),
            BinOp::Lte => Ok(JsValue::Bool(l.to_number() <= r.to_number())),
            BinOp::Gt => Ok(JsValue::Bool(l.to_number() > r.to_number())),
            BinOp::Gte => Ok(JsValue::Bool(l.to_number() >= r.to_number())),
            BinOp::BitAnd => Ok(JsValue::Number(
                (js_to_int32(l.to_number()) & js_to_int32(r.to_number())) as f64,
            )),
            BinOp::BitOr => Ok(JsValue::Number(
                (js_to_int32(l.to_number()) | js_to_int32(r.to_number())) as f64,
            )),
            BinOp::BitXor => Ok(JsValue::Number(
                (js_to_int32(l.to_number()) ^ js_to_int32(r.to_number())) as f64,
            )),
            BinOp::Shl => Ok(JsValue::Number(
                (js_to_int32(l.to_number()).wrapping_shl(js_to_uint32(r.to_number()) & 31)) as f64,
            )),
            BinOp::Shr => Ok(JsValue::Number(
                (js_to_int32(l.to_number()).wrapping_shr(js_to_uint32(r.to_number()) & 31)) as f64,
            )),
            BinOp::UShr => Ok(JsValue::Number(
                (js_to_uint32(l.to_number()).wrapping_shr(js_to_uint32(r.to_number()) & 31)) as f64,
            )),
            BinOp::And | BinOp::Or => unreachable!(),
        }
    }

    #[inline(never)]
    fn eval_assign_op(
        &mut self,
        bin_op: BinOp,
        prev: JsValue,
        rhs: JsValue,
    ) -> Result<JsValue, SandboxViolation> {
        Ok(match bin_op {
            BinOp::Add => match (&prev, &rhs) {
                (JsValue::Str(sa), _) => {
                    let s = format!("{}{}", sa, rhs);
                    self.compartment.alloc_memory(s.len())?;
                    JsValue::str(s)
                }
                (_, JsValue::Str(sb)) => {
                    let s = format!("{}{}", prev, sb);
                    self.compartment.alloc_memory(s.len())?;
                    JsValue::str(s)
                }
                _ => JsValue::Number(prev.to_number() + rhs.to_number()),
            },
            BinOp::Sub => JsValue::Number(prev.to_number() - rhs.to_number()),
            BinOp::Mul => JsValue::Number(prev.to_number() * rhs.to_number()),
            BinOp::Div => JsValue::Number(prev.to_number() / rhs.to_number()),
            BinOp::Rem => JsValue::Number(prev.to_number() % rhs.to_number()),
            _ => rhs,
        })
    }

    #[inline(never)]
    fn apply_index_assign(
        &mut self,
        target: &str,
        idx_val: JsValue,
        val: JsValue,
    ) -> Result<JsValue, SandboxViolation> {
        self.compartment.verify_property_mutation(target)?;
        let arr_idx = as_canonical_array_index(&idx_val);
        let grow_bytes =
            if let (Some(JsValue::Array(vec)), Some(i)) = (self.env_vars.get(target), arr_idx) {
                if i >= vec.len() {
                    (i.saturating_add(1).saturating_sub(vec.len())).saturating_mul(16)
                } else {
                    0
                }
            } else {
                0
            };
        if grow_bytes > 0 {
            self.compartment.alloc_memory(grow_bytes)?;
        }
        if let Some(container) = self.env_vars.get_mut(target) {
            match container {
                JsValue::Array(vec) => {
                    if let Some(i) = arr_idx {
                        if i >= vec.len() {
                            vec.resize(i + 1, JsValue::Undefined);
                        }
                        vec[i] = val.clone();
                    }
                }
                JsValue::Object(map) => {
                    map.insert(idx_val.to_string(), val.clone());
                }
                _ => {}
            }
        }
        Ok(val)
    }
}
