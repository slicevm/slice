//! Expression Lowering (`Expr` -> `NumericOp`) for `NumericFunc`.

use super::{CompilerCtx, MathOpKind, NumericOp};
use crate::value_and_ast::{BinOp, Expr, JsValue};

pub(super) fn compile_expr_into(ctx: &mut CompilerCtx, expr: &Expr, dst: u8) -> Option<()> {
    match expr {
        Expr::Literal(JsValue::Number(n)) => {
            ctx.ops.push(NumericOp::LoadConst { dst, imm: *n });
            Some(())
        }
        Expr::Literal(JsValue::Bool(b)) => {
            ctx.ops.push(NumericOp::LoadConst {
                dst,
                imm: if *b { 1.0 } else { 0.0 },
            });
            Some(())
        }
        Expr::Ident(name) => {
            let src = *ctx.locals.get(name)?;
            if src != dst {
                ctx.ops.push(NumericOp::Move { dst, src });
            }
            Some(())
        }
        Expr::Unary { op, expr: inner } => {
            compile_expr_into(ctx, inner, dst)?;
            ctx.ops.push(NumericOp::UnaryReg {
                dst,
                op: *op,
                src: dst,
            });
            Some(())
        }
        Expr::Binary { op, lhs, rhs } => {
            // Short-circuit logical operators `&&` and `||`
            if *op == BinOp::And {
                let out_reg = if dst < ctx.temp_reg_base {
                    ctx.alloc_reg()?
                } else {
                    dst
                };
                compile_expr_into(ctx, lhs, out_reg)?;
                let jfalse_idx = ctx.ops.len();
                ctx.ops.push(NumericOp::JumpIfFalse {
                    cond: out_reg,
                    target: 0,
                });
                compile_expr_into(ctx, rhs, out_reg)?;
                let end_target = ctx.ops.len() as u32;
                if let NumericOp::JumpIfFalse { target: t, .. } = &mut ctx.ops[jfalse_idx] {
                    *t = end_target;
                }
                if out_reg != dst {
                    ctx.ops.push(NumericOp::Move { dst, src: out_reg });
                }
                return Some(());
            }
            if *op == BinOp::Or {
                let out_reg = if dst < ctx.temp_reg_base {
                    ctx.alloc_reg()?
                } else {
                    dst
                };
                compile_expr_into(ctx, lhs, out_reg)?;
                let jfalse_idx = ctx.ops.len();
                ctx.ops.push(NumericOp::JumpIfFalse {
                    cond: out_reg,
                    target: 0,
                });
                let jend_idx = ctx.ops.len();
                ctx.ops.push(NumericOp::Jump { target: 0 });
                let rhs_start = ctx.ops.len() as u32;
                if let NumericOp::JumpIfFalse { target: t, .. } = &mut ctx.ops[jfalse_idx] {
                    *t = rhs_start;
                }
                compile_expr_into(ctx, rhs, out_reg)?;
                let end_target = ctx.ops.len() as u32;
                if let NumericOp::Jump { target: t } = &mut ctx.ops[jend_idx] {
                    *t = end_target;
                }
                if out_reg != dst {
                    ctx.ops.push(NumericOp::Move { dst, src: out_reg });
                }
                return Some(());
            }
            if let Expr::Literal(JsValue::Number(imm)) = rhs.as_ref() {
                let l_reg = if let Expr::Ident(id) = lhs.as_ref() {
                    *ctx.locals.get(id)?
                } else {
                    compile_expr_into(ctx, lhs, dst)?;
                    dst
                };
                ctx.ops.push(NumericOp::BinaryImm {
                    dst,
                    op: *op,
                    lhs: l_reg,
                    imm: *imm,
                });
                return Some(());
            }
            let l_reg = if let Expr::Ident(id) = lhs.as_ref() {
                *ctx.locals.get(id)?
            } else {
                let r = ctx.alloc_reg()?;
                compile_expr_into(ctx, lhs, r)?;
                r
            };
            let r_reg = if let Expr::Ident(id) = rhs.as_ref() {
                *ctx.locals.get(id)?
            } else {
                let r = ctx.alloc_reg()?;
                compile_expr_into(ctx, rhs, r)?;
                r
            };
            ctx.ops.push(NumericOp::BinaryReg {
                dst,
                op: *op,
                lhs: l_reg,
                rhs: r_reg,
            });
            Some(())
        }
        Expr::Assign { name, op, value } => {
            let var_reg = *ctx.locals.get(name)?;
            if let Some(bin_op) = op {
                if let Expr::Literal(JsValue::Number(imm)) = value.as_ref() {
                    ctx.ops.push(NumericOp::BinaryImm {
                        dst: var_reg,
                        op: *bin_op,
                        lhs: var_reg,
                        imm: *imm,
                    });
                } else {
                    let rhs_reg = ctx.alloc_reg()?;
                    compile_expr_into(ctx, value, rhs_reg)?;
                    ctx.ops.push(NumericOp::BinaryReg {
                        dst: var_reg,
                        op: *bin_op,
                        lhs: var_reg,
                        rhs: rhs_reg,
                    });
                }
            } else {
                compile_expr_into(ctx, value, var_reg)?;
            }
            if dst != var_reg {
                ctx.ops.push(NumericOp::Move { dst, src: var_reg });
            }
            Some(())
        }
        Expr::Update {
            name,
            increment,
            prefix,
        } => {
            let var_reg = *ctx.locals.get(name)?;
            let op = if *increment { BinOp::Add } else { BinOp::Sub };
            if *prefix || dst == var_reg {
                ctx.ops.push(NumericOp::BinaryImm {
                    dst: var_reg,
                    op,
                    lhs: var_reg,
                    imm: 1.0,
                });
                if dst != var_reg {
                    ctx.ops.push(NumericOp::Move { dst, src: var_reg });
                }
            } else {
                ctx.ops.push(NumericOp::Move { dst, src: var_reg });
                ctx.ops.push(NumericOp::BinaryImm {
                    dst: var_reg,
                    op,
                    lhs: var_reg,
                    imm: 1.0,
                });
            }
            Some(())
        }
        Expr::Ternary {
            cond,
            then_expr,
            else_expr,
        } => {
            let cond_reg = ctx.alloc_reg()?;
            compile_expr_into(ctx, cond, cond_reg)?;
            let jfalse_idx = ctx.ops.len();
            ctx.ops.push(NumericOp::JumpIfFalse {
                cond: cond_reg,
                target: 0,
            });
            compile_expr_into(ctx, then_expr, dst)?;
            let jend_idx = ctx.ops.len();
            ctx.ops.push(NumericOp::Jump { target: 0 });
            let else_start = ctx.ops.len() as u32;
            if let NumericOp::JumpIfFalse { target: t, .. } = &mut ctx.ops[jfalse_idx] {
                *t = else_start;
            }
            compile_expr_into(ctx, else_expr, dst)?;
            let end_target = ctx.ops.len() as u32;
            if let NumericOp::Jump { target: t } = &mut ctx.ops[jend_idx] {
                *t = end_target;
            }
            Some(())
        }
        Expr::Member { object, property } => {
            if let Expr::Ident(obj_name) = object.as_ref()
                && obj_name == "Math"
            {
                let imm = match property.as_str() {
                    "PI" => std::f64::consts::PI,
                    "E" => std::f64::consts::E,
                    "SQRT2" => std::f64::consts::SQRT_2,
                    "LN2" => std::f64::consts::LN_2,
                    "LN10" => std::f64::consts::LN_10,
                    _ => return None,
                };
                ctx.ops.push(NumericOp::LoadConst { dst, imm });
                return Some(());
            }
            None
        }
        Expr::Call { callee, args } => {
            // 1. Self-recursive call (`fn_name(arg0)` or `fn_name(arg0, arg1)`)
            if let Expr::Ident(id) = callee.as_ref()
                && !ctx.fn_name.is_empty()
                && id == &ctx.fn_name
            {
                match args.len() {
                    1 => {
                        let a0 = ctx.alloc_reg()?;
                        compile_expr_into(ctx, &args[0], a0)?;
                        ctx.ops.push(NumericOp::CallSelf1 { dst, arg0: a0 });
                        return Some(());
                    }
                    2 => {
                        let a0 = ctx.alloc_reg()?;
                        let a1 = ctx.alloc_reg()?;
                        compile_expr_into(ctx, &args[0], a0)?;
                        compile_expr_into(ctx, &args[1], a1)?;
                        ctx.ops.push(NumericOp::CallSelf2 {
                            dst,
                            arg0: a0,
                            arg1: a1,
                        });
                        return Some(());
                    }
                    _ => return None,
                }
            }
            // 2. `Math.*` intrinsic call
            if let Expr::Member { object, property } = callee.as_ref()
                && matches!(object.as_ref(), Expr::Ident(o) if o == "Math")
            {
                match property.as_str() {
                    "max" | "min" => {
                        let is_max = property == "max";
                        let kind = if is_max {
                            MathOpKind::Max
                        } else {
                            MathOpKind::Min
                        };
                        let identity = if is_max {
                            f64::NEG_INFINITY
                        } else {
                            f64::INFINITY
                        };
                        if args.is_empty() {
                            ctx.ops.push(NumericOp::LoadConst { dst, imm: identity });
                            return Some(());
                        }
                        let acc = ctx.alloc_reg()?;
                        compile_expr_into(ctx, &args[0], acc)?;
                        if args.len() == 1 {
                            let id_reg = ctx.alloc_reg()?;
                            ctx.ops.push(NumericOp::LoadConst {
                                dst: id_reg,
                                imm: identity,
                            });
                            ctx.ops.push(NumericOp::MathIntrinsic {
                                dst,
                                kind,
                                arg0: acc,
                                arg1: id_reg,
                            });
                            return Some(());
                        }
                        let scratch = ctx.alloc_reg()?;
                        for extra_arg in &args[1..] {
                            compile_expr_into(ctx, extra_arg, scratch)?;
                            ctx.ops.push(NumericOp::MathIntrinsic {
                                dst: acc,
                                kind,
                                arg0: acc,
                                arg1: scratch,
                            });
                        }
                        if acc != dst {
                            ctx.ops.push(NumericOp::Move { dst, src: acc });
                        }
                        return Some(());
                    }
                    "pow" | "imul" => {
                        if args.len() != 2 {
                            return None;
                        }
                        let kind = if property == "pow" {
                            MathOpKind::Pow
                        } else {
                            MathOpKind::Imul
                        };
                        let a0 = ctx.alloc_reg()?;
                        let a1 = ctx.alloc_reg()?;
                        compile_expr_into(ctx, &args[0], a0)?;
                        compile_expr_into(ctx, &args[1], a1)?;
                        ctx.ops.push(NumericOp::MathIntrinsic {
                            dst,
                            kind,
                            arg0: a0,
                            arg1: a1,
                        });
                        return Some(());
                    }
                    _ => {
                        if args.len() != 1 {
                            return None;
                        }
                        let kind = match property.as_str() {
                            "abs" => MathOpKind::Abs,
                            "floor" => MathOpKind::Floor,
                            "ceil" => MathOpKind::Ceil,
                            "round" => MathOpKind::Round,
                            "trunc" => MathOpKind::Trunc,
                            "sign" => MathOpKind::Sign,
                            "sqrt" => MathOpKind::Sqrt,
                            "sin" => MathOpKind::Sin,
                            "cos" => MathOpKind::Cos,
                            "clz32" => MathOpKind::Clz32,
                            "fround" => MathOpKind::Fround,
                            _ => return None,
                        };
                        let a0 = ctx.alloc_reg()?;
                        compile_expr_into(ctx, &args[0], a0)?;
                        ctx.ops.push(NumericOp::MathIntrinsic {
                            dst,
                            kind,
                            arg0: a0,
                            arg1: a0,
                        });
                        return Some(());
                    }
                }
            }
            None
        }
        _ => None,
    }
}
