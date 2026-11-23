//! Unboxed 64-Bit Register Interpreter for `NumericFunc` with Fuel & Recursion Guards.

use super::{MAX_NUMERIC_CALL_DEPTH, MAX_NUMERIC_REGS, MathOpKind, NumericFunc, NumericOp};
use crate::value_and_ast::{
    BinOp, UnaryOp, js_math_max, js_math_min, js_math_round, js_to_int32, js_to_uint32,
};
use slice_sandbox::SandboxViolation;

#[inline(always)]
fn eval_bin_f64(op: BinOp, a: f64, b: f64) -> f64 {
    match op {
        BinOp::Add => a + b,
        BinOp::Sub => a - b,
        BinOp::Mul => a * b,
        BinOp::Div => a / b,
        BinOp::Rem => a % b,
        BinOp::Pow => a.powf(b),
        BinOp::Eq | BinOp::StrictEq => {
            if a == b {
                1.0
            } else {
                0.0
            }
        }
        BinOp::Neq | BinOp::StrictNeq => {
            if a != b {
                1.0
            } else {
                0.0
            }
        }
        BinOp::Lt => {
            if a < b {
                1.0
            } else {
                0.0
            }
        }
        BinOp::Lte => {
            if a <= b {
                1.0
            } else {
                0.0
            }
        }
        BinOp::Gt => {
            if a > b {
                1.0
            } else {
                0.0
            }
        }
        BinOp::Gte => {
            if a >= b {
                1.0
            } else {
                0.0
            }
        }
        BinOp::And => {
            if a != 0.0 && !a.is_nan() {
                b
            } else {
                a
            }
        }
        BinOp::Or => {
            if a != 0.0 && !a.is_nan() {
                a
            } else {
                b
            }
        }
        BinOp::BitAnd => (js_to_int32(a) & js_to_int32(b)) as f64,
        BinOp::BitOr => (js_to_int32(a) | js_to_int32(b)) as f64,
        BinOp::BitXor => (js_to_int32(a) ^ js_to_int32(b)) as f64,
        BinOp::Shl => (js_to_int32(a).wrapping_shl(js_to_uint32(b) & 31)) as f64,
        BinOp::Shr => (js_to_int32(a).wrapping_shr(js_to_uint32(b) & 31)) as f64,
        BinOp::UShr => (js_to_uint32(a).wrapping_shr(js_to_uint32(b) & 31)) as f64,
    }
}

impl NumericFunc {
    /// Executes this compiled numeric function on an unboxed register file `[f64; MAX_NUMERIC_REGS]`,
    /// tracking instruction steps in `steps_executed` and enforcing `max_steps` fuel limits.
    /// Returns `Ok(None)` if a specialized op encounters a runtime value outside its exact fast-path domain,
    /// allowing the caller to fall back cleanly to general evaluation.
    pub fn execute(
        &self,
        args: &[f64],
        steps_executed: &mut u64,
        max_steps: u64,
        depth: usize,
        tenant_id: &str,
    ) -> Result<Option<f64>, SandboxViolation> {
        if depth > MAX_NUMERIC_CALL_DEPTH {
            return Err(SandboxViolation::CpuFuelExhausted {
                tenant_id: tenant_id.to_string(),
                limit: max_steps,
            });
        }
        let mut regs = [0.0f64; MAX_NUMERIC_REGS];
        for (i, &val) in args.iter().take(self.param_count as usize).enumerate() {
            regs[i] = val;
        }

        let ops = &self.ops;
        let mut pc: usize = 0;
        let mut local_steps: u64 = 0;

        while pc < ops.len() {
            local_steps += 1;
            if (local_steps & 255) == 0 {
                *steps_executed = steps_executed.saturating_add(256);
                if *steps_executed > max_steps {
                    return Err(SandboxViolation::CpuFuelExhausted {
                        tenant_id: tenant_id.to_string(),
                        limit: max_steps,
                    });
                }
            }

            match &ops[pc] {
                NumericOp::LoadConst { dst, imm } => {
                    regs[*dst as usize] = *imm;
                    pc += 1;
                }
                NumericOp::Move { dst, src } => {
                    regs[*dst as usize] = regs[*src as usize];
                    pc += 1;
                }
                NumericOp::BinaryReg { dst, op, lhs, rhs } => {
                    let a = regs[*lhs as usize];
                    let b = regs[*rhs as usize];
                    regs[*dst as usize] = eval_bin_f64(*op, a, b);
                    pc += 1;
                }
                NumericOp::BinaryImm { dst, op, lhs, imm } => {
                    let a = regs[*lhs as usize];
                    regs[*dst as usize] = eval_bin_f64(*op, a, *imm);
                    pc += 1;
                }
                NumericOp::UnaryReg { dst, op, src } => {
                    let v = regs[*src as usize];
                    regs[*dst as usize] = match op {
                        UnaryOp::Neg => -v,
                        UnaryOp::Pos => v,
                        UnaryOp::Not => {
                            if v == 0.0 || v.is_nan() {
                                1.0
                            } else {
                                0.0
                            }
                        }
                        UnaryOp::BitNot => (!js_to_int32(v)) as f64,
                        UnaryOp::TypeOf => f64::NAN,
                    };
                    pc += 1;
                }
                NumericOp::MathIntrinsic {
                    dst,
                    kind,
                    arg0,
                    arg1,
                } => {
                    let x = regs[*arg0 as usize];
                    let y = regs[*arg1 as usize];
                    regs[*dst as usize] = match kind {
                        MathOpKind::Abs => x.abs(),
                        MathOpKind::Floor => x.floor(),
                        MathOpKind::Ceil => x.ceil(),
                        MathOpKind::Round => js_math_round(x),
                        MathOpKind::Trunc => x.trunc(),
                        MathOpKind::Sign => {
                            if x == 0.0 || x.is_nan() {
                                x
                            } else if x > 0.0 {
                                1.0
                            } else {
                                -1.0
                            }
                        }
                        MathOpKind::Sqrt => x.sqrt(),
                        MathOpKind::Pow => {
                            if y.is_nan() || (y.is_infinite() && x.abs() == 1.0) {
                                f64::NAN
                            } else {
                                x.powf(y)
                            }
                        }
                        MathOpKind::Sin => x.sin(),
                        MathOpKind::Cos => x.cos(),
                        MathOpKind::Max => js_math_max(x, y),
                        MathOpKind::Min => js_math_min(x, y),
                        MathOpKind::Imul => (js_to_int32(x).wrapping_mul(js_to_int32(y))) as f64,
                        MathOpKind::Clz32 => (js_to_uint32(x).leading_zeros()) as f64,
                        MathOpKind::Fround => (x as f32) as f64,
                    };
                    pc += 1;
                }
                NumericOp::Jump { target } => {
                    pc = *target as usize;
                }
                NumericOp::JumpIfFalse { cond, target } => {
                    let c = regs[*cond as usize];
                    if c == 0.0 || c.is_nan() {
                        pc = *target as usize;
                    } else {
                        pc += 1;
                    }
                }
                NumericOp::CallSelf1 { dst, arg0 } => {
                    *steps_executed = steps_executed.saturating_add(local_steps & 255);
                    local_steps = 0;
                    if *steps_executed > max_steps {
                        return Err(SandboxViolation::CpuFuelExhausted {
                            tenant_id: tenant_id.to_string(),
                            limit: max_steps,
                        });
                    }
                    let a0 = regs[*arg0 as usize];
                    let Some(res) =
                        self.execute(&[a0], steps_executed, max_steps, depth + 1, tenant_id)?
                    else {
                        return Ok(None);
                    };
                    regs[*dst as usize] = res;
                    pc += 1;
                }
                NumericOp::CallSelf2 { dst, arg0, arg1 } => {
                    *steps_executed = steps_executed.saturating_add(local_steps & 255);
                    local_steps = 0;
                    if *steps_executed > max_steps {
                        return Err(SandboxViolation::CpuFuelExhausted {
                            tenant_id: tenant_id.to_string(),
                            limit: max_steps,
                        });
                    }
                    let a0 = regs[*arg0 as usize];
                    let a1 = regs[*arg1 as usize];
                    let Some(res) =
                        self.execute(&[a0, a1], steps_executed, max_steps, depth + 1, tenant_id)?
                    else {
                        return Ok(None);
                    };
                    regs[*dst as usize] = res;
                    pc += 1;
                }
                NumericOp::LinearRecurrence2 {
                    dst,
                    n_reg,
                    base0,
                    base1,
                    c1,
                    c2,
                } => {
                    let n_val = regs[*n_reg as usize];
                    if n_val < 0.0 || !n_val.is_finite() || n_val.fract() != 0.0 || n_val > 78.0 {
                        return Ok(None);
                    }
                    let n = n_val as u64;
                    if n == 0 {
                        regs[*dst as usize] = *base0;
                    } else if n == 1 {
                        regs[*dst as usize] = *base1;
                    } else {
                        *steps_executed = steps_executed.saturating_add(n);
                        if *steps_executed > max_steps {
                            return Err(SandboxViolation::CpuFuelExhausted {
                                tenant_id: tenant_id.to_string(),
                                limit: max_steps,
                            });
                        }
                        let mut prev2 = *base0;
                        let mut prev1 = *base1;
                        for _ in 2..=n {
                            let next = (*c1) * prev1 + (*c2) * prev2;
                            prev2 = prev1;
                            prev1 = next;
                        }
                        regs[*dst as usize] = prev1;
                    }
                    pc += 1;
                }
                NumericOp::Return { src } => {
                    *steps_executed = steps_executed.saturating_add(local_steps & 255);
                    if *steps_executed > max_steps {
                        return Err(SandboxViolation::CpuFuelExhausted {
                            tenant_id: tenant_id.to_string(),
                            limit: max_steps,
                        });
                    }
                    return Ok(Some(regs[*src as usize]));
                }
            }
        }

        *steps_executed = steps_executed.saturating_add(local_steps & 255);
        Ok(None)
    }
}
