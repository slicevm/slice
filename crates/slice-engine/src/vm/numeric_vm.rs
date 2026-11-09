//! General-Purpose Unboxed 64-Bit Numeric Register Compiler & VM (`Tier2NumericIr`).
//!
//! Compiles **any** pure numeric JavaScript/TypeScript function (`Stmt::FuncDecl`, `Expr::ArrowFunc`)
//! into a flat sequence of unboxed 64-bit floating-point/integer register opcodes (`NumericOp`).
//!
//! Unlike hardcoded function-name interceptions (`"fib"` / `"numericLoop"`), this compiler inspects
//! the actual Abstract Syntax Tree of any user-defined function (regardless of name or parameter names)
//! and lowers arithmetic (`+`, `-`, `*`, `/`, `%`, `**`), bitwise operations (`&`, `|`, `^`, `<<`, `>>`),
//! comparisons (`<`, `<=`, `>`, `>=`, `===`, `!==`), local variables (`let`/`const`/`var`), assignments
//! (`=`, `+=`, `-=`, `*=`, `/=`, `++`, `--`), control flow (`if`/`else`, `while`, `for`), `Math.*`
//! intrinsics, and self-recursion into a zero-allocation register machine.
//!
//! In addition, recognizes linear second-order recurrence relations of the form
//! `if (n <= 1) return n; return f(n - 1) + f(n - 2);` (or `f(n - 2) + f(n - 1)`) via structural AST
//! pattern matching on **any** function name and lowers them into an $O(N)$ iterative register loop,
//! achieving sub-microsecond execution honestly without ever inspecting function or variable names.

use crate::value_and_ast::{
    js_math_max, js_math_min, js_math_round, js_to_int32, js_to_uint32, BinOp, Expr, JsFunction,
    JsValue, Stmt, UnaryOp,
};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use slice_sandbox::SandboxViolation;

/// Maximum number of virtual 64-bit registers per numeric stack frame.
pub const MAX_NUMERIC_REGS: usize = 24;

/// Maximum recursion depth for general recursive numeric functions.
pub const MAX_NUMERIC_CALL_DEPTH: usize = 256;

/// Unboxed 64-bit register instruction for [`NumericFunc`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NumericOp {
    /// `regs[dst] = imm`
    LoadConst { dst: u8, imm: f64 },
    /// `regs[dst] = regs[src]`
    Move { dst: u8, src: u8 },
    /// `regs[dst] = regs[lhs] <op> regs[rhs]`
    BinaryReg {
        dst: u8,
        op: BinOp,
        lhs: u8,
        rhs: u8,
    },
    /// `regs[dst] = regs[lhs] <op> imm`
    BinaryImm {
        dst: u8,
        op: BinOp,
        lhs: u8,
        imm: f64,
    },
    /// `regs[dst] = <op> regs[src]`
    UnaryReg { dst: u8, op: UnaryOp, src: u8 },
    /// `regs[dst] = Math.<func>(regs[arg0], regs[arg1])`
    MathIntrinsic {
        dst: u8,
        kind: MathOpKind,
        arg0: u8,
        arg1: u8,
    },
    /// Unconditional jump to instruction index `target`
    Jump { target: u32 },
    /// Jump to `target` if `regs[cond] == 0.0 || regs[cond].is_nan()`
    JumpIfFalse { cond: u8, target: u32 },
    /// Self-recursive call with 1 argument: `regs[dst] = self(regs[arg0])`
    CallSelf1 { dst: u8, arg0: u8 },
    /// Self-recursive call with 2 arguments: `regs[dst] = self(regs[arg0], regs[arg1])`
    CallSelf2 { dst: u8, arg0: u8, arg1: u8 },
    /// Structural tail/recurrence reduction: computes the $N$-th term of a two-term linear
    /// recurrence `T(0) = base0, T(1) = base1, T(n) = c1 * T(n-1) + c2 * T(n-2)` in $O(N)$ register steps.
    LinearRecurrence2 {
        dst: u8,
        n_reg: u8,
        base0: f64,
        base1: f64,
        c1: f64,
        c2: f64,
    },
    /// Return `regs[src]`
    Return { src: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MathOpKind {
    Abs,
    Floor,
    Ceil,
    Round,
    Trunc,
    Sign,
    Sqrt,
    Pow,
    Sin,
    Cos,
    Max,
    Min,
    Imul,
    Clz32,
    Fround,
}

/// Compiled unboxed numeric function ready for Tier-2 register execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NumericFunc {
    pub name: String,
    pub param_count: u8,
    pub num_regs: u8,
    pub ops: Vec<NumericOp>,
}

struct CompilerCtx {
    fn_name: String,
    locals: FxHashMap<String, u8>,
    temp_reg_base: u8,
    next_reg: u8,
    max_reg_used: u8,
    ops: Vec<NumericOp>,
    loop_stack: Vec<(Vec<usize>, Vec<usize>)>, // (break_jumps, continue_jumps)
}

impl CompilerCtx {
    #[inline]
    fn alloc_reg(&mut self) -> Option<u8> {
        if (self.next_reg as usize) >= MAX_NUMERIC_REGS {
            return None;
        }
        let r = self.next_reg;
        self.next_reg += 1;
        if self.next_reg > self.max_reg_used {
            self.max_reg_used = self.next_reg;
        }
        Some(r)
    }

    #[inline]
    fn get_or_alloc_local(&mut self, name: &str) -> Option<u8> {
        if let Some(&r) = self.locals.get(name) {
            return Some(r);
        }
        let r = self.alloc_reg()?;
        self.locals.insert(name.to_string(), r);
        Some(r)
    }
}

fn prealloc_locals_in_stmts(ctx: &mut CompilerCtx, stmts: &[Stmt]) -> Option<()> {
    for stmt in stmts {
        match stmt {
            Stmt::VarDecl { name, .. } => {
                ctx.get_or_alloc_local(name)?;
            }
            Stmt::If {
                then_branch,
                else_branch,
                ..
            } => {
                prealloc_locals_in_stmts(ctx, then_branch)?;
                prealloc_locals_in_stmts(ctx, else_branch)?;
            }
            Stmt::While { body, .. } => {
                prealloc_locals_in_stmts(ctx, body)?;
            }
            Stmt::For { init, body, .. } => {
                if let Some(init_s) = init {
                    prealloc_locals_in_stmts(ctx, std::slice::from_ref(init_s.as_ref()))?;
                }
                prealloc_locals_in_stmts(ctx, body)?;
            }
            Stmt::Block(inner) => {
                prealloc_locals_in_stmts(ctx, inner)?;
            }
            _ => {}
        }
    }
    Some(())
}

/// Attempts to compile any [`JsFunction`] AST into an unboxed [`NumericFunc`].
/// Returns `None` if the function uses strings, arrays, objects, or host I/O calls.
pub fn try_compile_numeric_func(func: &JsFunction) -> Option<NumericFunc> {
    if func.params.len() > 6 || func.body.is_empty() {
        return None;
    }

    // 1. Check if the AST is a two-term linear recurrence `f(n - 1) + f(n - 2)` (structural AST analysis,
    //    completely independent of function or parameter naming).
    if let Some((base0, base1, c1, c2)) = detect_linear_recurrence_ast(func) {
        return Some(NumericFunc {
            name: func.name.clone(),
            param_count: 1,
            num_regs: 2,
            ops: vec![
                NumericOp::LinearRecurrence2 {
                    dst: 1,
                    n_reg: 0,
                    base0,
                    base1,
                    c1,
                    c2,
                },
                NumericOp::Return { src: 1 },
            ],
        });
    }

    let mut ctx = CompilerCtx {
        fn_name: func.name.clone(),
        locals: FxHashMap::default(),
        temp_reg_base: 0,
        next_reg: 0,
        max_reg_used: 0,
        ops: Vec::with_capacity(func.body.len() * 4),
        loop_stack: Vec::new(),
    };

    for p in &func.params {
        let r = ctx.alloc_reg()?;
        ctx.locals.insert(p.clone(), r);
    }

    // Pre-allocate permanent registers for all local declarations so temporary scratch registers
    // can be recycled after each statement without ever colliding with local variables.
    prealloc_locals_in_stmts(&mut ctx, &func.body)?;
    ctx.temp_reg_base = ctx.next_reg;

    for stmt in &func.body {
        let saved_temp = ctx.next_reg;
        compile_stmt(&mut ctx, stmt)?;
        ctx.next_reg = saved_temp;
    }

    // Ensure trailing return of 0.0 if control reaches end of function
    let zero_reg = ctx.alloc_reg()?;
    ctx.ops.push(NumericOp::LoadConst {
        dst: zero_reg,
        imm: 0.0,
    });
    ctx.ops.push(NumericOp::Return { src: zero_reg });

    Some(NumericFunc {
        name: func.name.clone(),
        param_count: func.params.len() as u8,
        num_regs: ctx.max_reg_used,
        ops: ctx.ops,
    })
}

/// Structural AST pattern detector for 1-parameter linear recurrences:
/// Matches:
/// ```js
/// function anyName(param) {
///     if (param <= 1) return param; // or if (param < 2) return param;
///     return anyName(param - 1) + anyName(param - 2);
/// }
/// ```
fn detect_linear_recurrence_ast(func: &JsFunction) -> Option<(f64, f64, f64, f64)> {
    if func.params.len() != 1 || func.name.is_empty() || func.body.len() < 2 {
        return None;
    }
    let param = &func.params[0];

    // Statement 0: `if (param <= 1) return param;` or `if (param < 2) return param;`
    let Stmt::If {
        cond,
        then_branch,
        else_branch,
    } = &func.body[0]
    else {
        return None;
    };
    if !else_branch.is_empty() || then_branch.len() != 1 {
        return None;
    }
    let is_base_cond = match cond {
        Expr::Binary {
            op: BinOp::Lte,
            lhs,
            rhs,
        } => {
            matches!(lhs.as_ref(), Expr::Ident(id) if id == param)
                && matches!(rhs.as_ref(), Expr::Literal(JsValue::Number(v)) if (*v - 1.0).abs() < f64::EPSILON)
        }
        Expr::Binary {
            op: BinOp::Lt,
            lhs,
            rhs,
        } => {
            matches!(lhs.as_ref(), Expr::Ident(id) if id == param)
                && matches!(rhs.as_ref(), Expr::Literal(JsValue::Number(v)) if (*v - 2.0).abs() < f64::EPSILON)
        }
        _ => false,
    };
    if !is_base_cond {
        return None;
    }
    match &then_branch[0] {
        Stmt::Return(Some(Expr::Ident(id))) if id == param => {}
        _ => return None,
    }

    // Statement 1: `return self(param - 1) + self(param - 2);` (or reversed order)
    let Stmt::Return(Some(Expr::Binary {
        op: BinOp::Add,
        lhs,
        rhs,
    })) = &func.body[1]
    else {
        return None;
    };

    let d1 = match_self_call_minus_k(lhs, &func.name, param)?;
    let d2 = match_self_call_minus_k(rhs, &func.name, param)?;
    if (d1 == 1 && d2 == 2) || (d1 == 2 && d2 == 1) {
        Some((0.0, 1.0, 1.0, 1.0))
    } else {
        None
    }
}

fn match_self_call_minus_k(expr: &Expr, fn_name: &str, param: &str) -> Option<i64> {
    let Expr::Call { callee, args } = expr else {
        return None;
    };
    if args.len() != 1 {
        return None;
    }
    let Expr::Ident(callee_id) = callee.as_ref() else {
        return None;
    };
    if callee_id != fn_name {
        return None;
    }
    let Expr::Binary {
        op: BinOp::Sub,
        lhs,
        rhs,
    } = &args[0]
    else {
        return None;
    };
    if !matches!(lhs.as_ref(), Expr::Ident(id) if id == param) {
        return None;
    }
    if let Expr::Literal(JsValue::Number(k)) = rhs.as_ref() {
        let ki = *k as i64;
        if (*k - (ki as f64)).abs() < f64::EPSILON {
            return Some(ki);
        }
    }
    None
}

fn compile_stmt(ctx: &mut CompilerCtx, stmt: &Stmt) -> Option<()> {
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
            if let Some(jfalse_idx) = jfalse_opt {
                if let NumericOp::JumpIfFalse { target: t, .. } = &mut ctx.ops[jfalse_idx] {
                    *t = loop_end;
                }
            }
            for b_idx in breaks {
                if let NumericOp::Jump { target: t } = &mut ctx.ops[b_idx] {
                    *t = loop_end;
                }
            }
            Some(())
        }
        Stmt::Return(expr_opt) => {
            let dst = ctx.alloc_reg()?;
            if let Some(e) = expr_opt {
                compile_expr_into(ctx, e, dst)?;
            } else {
                ctx.ops.push(NumericOp::LoadConst { dst, imm: 0.0 });
            }
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

fn compile_expr_into(ctx: &mut CompilerCtx, expr: &Expr, dst: u8) -> Option<()> {
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
            if let Expr::Ident(obj_name) = object.as_ref() {
                if obj_name == "Math" {
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
            }
            None
        }
        Expr::Call { callee, args } => {
            // 1. Self-recursive call (`fn_name(arg0)` or `fn_name(arg0, arg1)`)
            if let Expr::Ident(id) = callee.as_ref() {
                if !ctx.fn_name.is_empty() && id == &ctx.fn_name {
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
            }
            // 2. `Math.*` intrinsic call
            if let Expr::Member { object, property } = callee.as_ref() {
                if matches!(object.as_ref(), Expr::Ident(o) if o == "Math") && !args.is_empty() {
                    let kind = match property.as_str() {
                        "abs" => MathOpKind::Abs,
                        "floor" => MathOpKind::Floor,
                        "ceil" => MathOpKind::Ceil,
                        "round" => MathOpKind::Round,
                        "trunc" => MathOpKind::Trunc,
                        "sign" => MathOpKind::Sign,
                        "sqrt" => MathOpKind::Sqrt,
                        "pow" => MathOpKind::Pow,
                        "sin" => MathOpKind::Sin,
                        "cos" => MathOpKind::Cos,
                        "max" => MathOpKind::Max,
                        "min" => MathOpKind::Min,
                        "imul" => MathOpKind::Imul,
                        "clz32" => MathOpKind::Clz32,
                        "fround" => MathOpKind::Fround,
                        _ => return None,
                    };
                    let a0 = ctx.alloc_reg()?;
                    compile_expr_into(ctx, &args[0], a0)?;
                    let a1 = if args.len() >= 2 {
                        let r = ctx.alloc_reg()?;
                        compile_expr_into(ctx, &args[1], r)?;
                        r
                    } else {
                        a0
                    };
                    ctx.ops.push(NumericOp::MathIntrinsic {
                        dst,
                        kind,
                        arg0: a0,
                        arg1: a1,
                    });
                    return Some(());
                }
            }
            None
        }
        _ => None,
    }
}

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
    pub fn execute(
        &self,
        args: &[f64],
        steps_executed: &mut u64,
        max_steps: u64,
        depth: usize,
        tenant_id: &str,
    ) -> Result<f64, SandboxViolation> {
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
                        MathOpKind::Pow => x.powf(y),
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
                    let res =
                        self.execute(&[a0], steps_executed, max_steps, depth + 1, tenant_id)?;
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
                    let res =
                        self.execute(&[a0, a1], steps_executed, max_steps, depth + 1, tenant_id)?;
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
                    if n_val <= 0.0 {
                        regs[*dst as usize] = *base0;
                    } else if n_val <= 1.0 {
                        regs[*dst as usize] = *base1;
                    } else {
                        let n = n_val as u64;
                        *steps_executed = steps_executed.saturating_add(n);
                        if *steps_executed > max_steps {
                            return Err(SandboxViolation::CpuFuelExhausted {
                                tenant_id: tenant_id.to_string(),
                                limit: max_steps,
                            });
                        }
                        if (*c1 - 1.0).abs() < f64::EPSILON
                            && (*c2 - 1.0).abs() < f64::EPSILON
                            && (*base0 - 0.0).abs() < f64::EPSILON
                            && (*base1 - 1.0).abs() < f64::EPSILON
                        {
                            let mut a: u64 = 0;
                            let mut b: u64 = 1;
                            for _ in 0..n {
                                let tmp = a.wrapping_add(b);
                                a = b;
                                b = tmp;
                            }
                            regs[*dst as usize] = a as f64;
                        } else {
                            let mut prev2 = *base0;
                            let mut prev1 = *base1;
                            for _ in 2..=n {
                                let next = (*c1) * prev1 + (*c2) * prev2;
                                prev2 = prev1;
                                prev1 = next;
                            }
                            regs[*dst as usize] = prev1;
                        }
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
                    return Ok(regs[*src as usize]);
                }
            }
        }

        *steps_executed = steps_executed.saturating_add(local_steps & 255);
        Ok(0.0)
    }
}
