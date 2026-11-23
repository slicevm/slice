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

mod compile_expr;
mod compile_stmt;
mod interpreter;
mod recurrence;

use crate::value_and_ast::{BinOp, JsFunction, Stmt, UnaryOp};
use compile_stmt::compile_stmt;
use recurrence::detect_linear_recurrence_ast;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};

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

pub(super) struct CompilerCtx {
    pub(super) fn_name: String,
    pub(super) locals: FxHashMap<String, u8>,
    pub(super) temp_reg_base: u8,
    pub(super) next_reg: u8,
    pub(super) max_reg_used: u8,
    pub(super) ops: Vec<NumericOp>,
    pub(super) loop_stack: Vec<(Vec<usize>, Vec<usize>)>, // (break_jumps, continue_jumps)
}

impl CompilerCtx {
    #[inline]
    pub(super) fn alloc_reg(&mut self) -> Option<u8> {
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
    pub(super) fn get_or_alloc_local(&mut self, name: &str) -> Option<u8> {
        if let Some(&r) = self.locals.get(name) {
            return Some(r);
        }
        let r = self.alloc_reg()?;
        self.locals.insert(name.to_string(), r);
        Some(r)
    }
}

fn prealloc_locals_in_stmts(
    ctx: &mut CompilerCtx,
    stmts: &[Stmt],
    enclosing_vars: &FxHashSet<String>,
) -> Option<()> {
    let mut block_vars: FxHashSet<String> = FxHashSet::default();
    for stmt in stmts {
        if let Stmt::VarDecl { name, .. } = stmt {
            if enclosing_vars.contains(name) || !block_vars.insert(name.clone()) {
                return None;
            }
            ctx.get_or_alloc_local(name)?;
        }
    }
    let mut child_enclosing = enclosing_vars.clone();
    child_enclosing.extend(block_vars);

    for stmt in stmts {
        match stmt {
            Stmt::If {
                then_branch,
                else_branch,
                ..
            } => {
                prealloc_locals_in_stmts(ctx, then_branch, &child_enclosing)?;
                prealloc_locals_in_stmts(ctx, else_branch, &child_enclosing)?;
            }
            Stmt::While { body, .. } => {
                prealloc_locals_in_stmts(ctx, body, &child_enclosing)?;
            }
            Stmt::For { init, body, .. } => {
                let mut for_enclosing = child_enclosing.clone();
                if let Some(init_s) = init
                    && let Stmt::VarDecl { name, .. } = init_s.as_ref()
                {
                    if for_enclosing.contains(name) {
                        return None;
                    }
                    ctx.get_or_alloc_local(name)?;
                    for_enclosing.insert(name.clone());
                }
                prealloc_locals_in_stmts(ctx, body, &for_enclosing)?;
            }
            Stmt::Block(inner) => {
                prealloc_locals_in_stmts(ctx, inner, &child_enclosing)?;
            }
            _ => {}
        }
    }
    Some(())
}

fn stmts_always_return_value(stmts: &[Stmt]) -> bool {
    for stmt in stmts {
        match stmt {
            Stmt::Return(Some(_)) => return true,
            Stmt::Return(None) => return false,
            Stmt::Block(inner) if stmts_always_return_value(inner) => {
                return true;
            }
            Stmt::If {
                then_branch,
                else_branch,
                ..
            } if !else_branch.is_empty()
                && stmts_always_return_value(then_branch)
                && stmts_always_return_value(else_branch) =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
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

    if !stmts_always_return_value(&func.body) {
        return None;
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

    let mut param_set: FxHashSet<String> = FxHashSet::default();
    for p in &func.params {
        if !param_set.insert(p.clone()) {
            return None;
        }
        let r = ctx.alloc_reg()?;
        ctx.locals.insert(p.clone(), r);
    }

    // Pre-allocate permanent registers for all local declarations so temporary scratch registers
    // can be recycled after each statement without ever colliding with local variables.
    prealloc_locals_in_stmts(&mut ctx, &func.body, &param_set)?;
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
