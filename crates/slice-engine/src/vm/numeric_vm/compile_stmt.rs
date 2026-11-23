//! Statement Lowering (`Stmt` -> `NumericOp`) for `NumericFunc`.

use super::compile_expr::compile_expr_into;
use super::{CompilerCtx, NumericOp};
use crate::value_and_ast::{Expr, Stmt};

pub(super) fn compile_stmt(ctx: &mut CompilerCtx, stmt: &Stmt) -> Option<()> {
    let saved_temp = ctx.next_reg;
    let res = match stmt {
        Stmt::VarDecl { name, init, .. } => {
            let dst = ctx.get_or_alloc_local(name)?;
            if let Some(expr) = init {
                compile_expr_into(ctx, expr, dst)?;
            } else {
                ctx.ops.push(NumericOp::LoadConst { dst, imm: 0.0 });
            }
            Some(())
        }
        Stmt::If {
            cond,
            then_branch,
            else_branch,
        } => {
            let cond_reg = ctx.alloc_reg()?;
            compile_expr_into(ctx, cond, cond_reg)?;
            let jfalse_idx = ctx.ops.len();
            ctx.ops.push(NumericOp::JumpIfFalse {
                cond: cond_reg,
                target: 0,
            });
            for s in then_branch {
                compile_stmt(ctx, s)?;
            }
            if else_branch.is_empty() {
                let target = ctx.ops.len() as u32;
                if let NumericOp::JumpIfFalse { target: t, .. } = &mut ctx.ops[jfalse_idx] {
                    *t = target;
                }
            } else {
                let jend_idx = ctx.ops.len();
                ctx.ops.push(NumericOp::Jump { target: 0 });
                let else_start = ctx.ops.len() as u32;
                if let NumericOp::JumpIfFalse { target: t, .. } = &mut ctx.ops[jfalse_idx] {
                    *t = else_start;
                }
                for s in else_branch {
                    compile_stmt(ctx, s)?;
                }
                let end_target = ctx.ops.len() as u32;
                if let NumericOp::Jump { target: t } = &mut ctx.ops[jend_idx] {
                    *t = end_target;
                }
            }
            Some(())
        }
        Stmt::While { cond, body } => {
            let loop_start = ctx.ops.len() as u32;
            let cond_reg = ctx.alloc_reg()?;
            compile_expr_into(ctx, cond, cond_reg)?;
            let jfalse_idx = ctx.ops.len();
            ctx.ops.push(NumericOp::JumpIfFalse {
                cond: cond_reg,
                target: 0,
            });
            ctx.next_reg = saved_temp;
            ctx.loop_stack.push((Vec::new(), Vec::new()));
            for s in body {
                compile_stmt(ctx, s)?;
            }
            let (breaks, continues) = ctx.loop_stack.pop()?;
            for c_idx in continues {
                if let NumericOp::Jump { target: t } = &mut ctx.ops[c_idx] {
                    *t = loop_start;
                }
            }
            ctx.ops.push(NumericOp::Jump { target: loop_start });
            let loop_end = ctx.ops.len() as u32;
            if let NumericOp::JumpIfFalse { target: t, .. } = &mut ctx.ops[jfalse_idx] {
                *t = loop_end;
            }
            for b_idx in breaks {
                if let NumericOp::Jump { target: t } = &mut ctx.ops[b_idx] {
                    *t = loop_end;
                }
            }
            Some(())
        }
        Stmt::For {
            init,
            cond,
            step,
            body,
        } => {
            if let Some(init_s) = init {
                compile_stmt(ctx, init_s)?;
            }
            let loop_start = ctx.ops.len() as u32;
            let jfalse_opt = if let Some(c_expr) = cond {
                let cond_reg = ctx.alloc_reg()?;
                compile_expr_into(ctx, c_expr, cond_reg)?;
                let idx = ctx.ops.len();
                ctx.ops.push(NumericOp::JumpIfFalse {
                    cond: cond_reg,
                    target: 0,
                });
                ctx.next_reg = saved_temp;
                Some(idx)
            } else {
                None
            };
            ctx.loop_stack.push((Vec::new(), Vec::new()));
            for s in body {
                compile_stmt(ctx, s)?;
            }
            let (breaks, continues) = ctx.loop_stack.pop()?;
            let step_start = ctx.ops.len() as u32;
            for c_idx in continues {
                if let NumericOp::Jump { target: t } = &mut ctx.ops[c_idx] {
                    *t = step_start;
                }
            }
            if let Some(step_e) = step {
                if let Expr::Update { name, .. } = step_e {
                    let var_reg = *ctx.locals.get(name)?;
                    compile_expr_into(ctx, step_e, var_reg)?;
                } else {
                    let scratch = ctx.alloc_reg()?;
                    compile_expr_into(ctx, step_e, scratch)?;
                    ctx.next_reg = saved_temp;
                }
            }
            ctx.ops.push(NumericOp::Jump { target: loop_start });
            let loop_end = ctx.ops.len() as u32;
            if let Some(jfalse_idx) = jfalse_opt
                && let NumericOp::JumpIfFalse { target: t, .. } = &mut ctx.ops[jfalse_idx]
            {
                *t = loop_end;
            }
            for b_idx in breaks {
                if let NumericOp::Jump { target: t } = &mut ctx.ops[b_idx] {
                    *t = loop_end;
                }
            }
            Some(())
        }
        Stmt::Return(expr_opt) => {
            let e = expr_opt.as_ref()?;
            let dst = ctx.alloc_reg()?;
            compile_expr_into(ctx, e, dst)?;
            ctx.ops.push(NumericOp::Return { src: dst });
            Some(())
        }
        Stmt::Break => {
            let idx = ctx.ops.len();
            ctx.ops.push(NumericOp::Jump { target: 0 });
            ctx.loop_stack.last_mut()?.0.push(idx);
            Some(())
        }
        Stmt::Continue => {
            let idx = ctx.ops.len();
            ctx.ops.push(NumericOp::Jump { target: 0 });
            ctx.loop_stack.last_mut()?.1.push(idx);
            Some(())
        }
        Stmt::Block(stmts) => {
            for s in stmts {
                compile_stmt(ctx, s)?;
            }
            Some(())
        }
        Stmt::ExprStmt(expr) => {
            if let Expr::Update { name, .. } = expr {
                let var_reg = *ctx.locals.get(name)?;
                compile_expr_into(ctx, expr, var_reg)
            } else {
                let scratch = ctx.alloc_reg()?;
                compile_expr_into(ctx, expr, scratch)
            }
        }
        Stmt::ImportCas { .. } | Stmt::FuncDecl { .. } => None,
    };
    ctx.next_reg = saved_temp;
    res
}
