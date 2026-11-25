//! Tier-0 Recursive-Descent AST & Function Evaluator (`eval_stmt`, `eval_block`).

mod calls;
mod expr;

use crate::isolate::{ControlFlow, SliceIsolate};
use crate::value_and_ast::{ExecutionTier, JsFunction, JsValue, Stmt, TIER1_PROMOTION_THRESHOLD};
use slice_sandbox::SandboxViolation;

/// Maximum Tier-0 AST function recursion depth (bounded so even unoptimized debug builds
/// on 1 MB Windows thread stacks trap cleanly with `CpuFuelExhausted`).
pub const MAX_TIER0_RECURSION_DEPTH: u32 = if cfg!(debug_assertions) { 32 } else { 128 };

pub(super) fn collect_local_var_names(stmts: &[Stmt], params: &[String], out: &mut Vec<String>) {
    for stmt in stmts {
        match stmt {
            Stmt::VarDecl { name, .. } => {
                if !params.contains(name) && !out.contains(name) {
                    out.push(name.clone());
                }
            }
            Stmt::If {
                then_branch,
                else_branch,
                ..
            } => {
                collect_local_var_names(then_branch, params, out);
                collect_local_var_names(else_branch, params, out);
            }
            Stmt::While { body, .. } => {
                collect_local_var_names(body, params, out);
            }
            Stmt::For { init, body, .. } => {
                if let Some(init_s) = init {
                    collect_local_var_names(std::slice::from_ref(init_s.as_ref()), params, out);
                }
                collect_local_var_names(body, params, out);
            }
            Stmt::Block(inner) => {
                collect_local_var_names(inner, params, out);
            }
            _ => {}
        }
    }
}

pub(super) fn as_canonical_array_index(val: &JsValue) -> Option<usize> {
    match val {
        JsValue::Number(n) => {
            if n.is_finite() && *n >= 0.0 && n.fract() == 0.0 && *n < (u32::MAX as f64) {
                Some(*n as usize)
            } else {
                None
            }
        }
        JsValue::Str(s) => {
            let u = s.parse::<u32>().ok()?;
            if u < u32::MAX && u.to_string() == s.as_str() {
                Some(u as usize)
            } else {
                None
            }
        }
        _ => None,
    }
}

impl SliceIsolate {
    pub(crate) fn eval_stmt(&mut self, stmt: &Stmt) -> Result<ControlFlow, SandboxViolation> {
        self.compartment.consume_fuel(1)?;
        if self.pending_tier0_throw.is_some() {
            return Ok(ControlFlow::None(JsValue::Undefined));
        }
        match stmt {
            Stmt::VarDecl { name, init, .. } => {
                let val = if let Some(expr) = init {
                    self.eval_expr(expr)?
                } else {
                    JsValue::Undefined
                };
                if self.pending_tier0_throw.is_some() {
                    return Ok(ControlFlow::None(JsValue::Undefined));
                }
                self.set_env_var_str(name, val.clone());
                Ok(ControlFlow::None(val))
            }
            Stmt::ImportCas { .. } => Ok(ControlFlow::None(JsValue::Undefined)),
            Stmt::FuncDecl { name, params, body } => {
                let mut js_fn = JsFunction {
                    name: name.clone(),
                    params: params.clone(),
                    body: body.clone(),
                    compiled_numeric: None,
                };
                js_fn.compiled_numeric = crate::numeric_vm::try_compile_numeric_func(&js_fn);
                let func = JsValue::Function(Box::new(js_fn));
                self.set_env_var_str(name, func.clone());
                Ok(ControlFlow::None(func))
            }
            Stmt::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let c = self.eval_expr(cond)?;
                let branch = if c.is_truthy() {
                    then_branch
                } else {
                    else_branch
                };
                self.eval_block(branch)
            }
            Stmt::While { cond, body } => {
                let mut last = JsValue::Undefined;
                let mut iters: u64 = 0;
                while self.eval_expr(cond)?.is_truthy() {
                    iters += 1;
                    self.compartment.consume_fuel(1)?;
                    self.check_tier0_deadline_and_interrupt(iters)?;
                    if iters == TIER1_PROMOTION_THRESHOLD {
                        self.current_tier = ExecutionTier::Tier1Baseline;
                    } else if iters == 10_000 {
                        self.current_tier = ExecutionTier::Tier2PeakNative;
                    }
                    match self.eval_block(body)? {
                        ControlFlow::None(v) => last = v,
                        ControlFlow::Return(v) => return Ok(ControlFlow::Return(v)),
                        ControlFlow::Break => break,
                        ControlFlow::Continue => continue,
                    }
                }
                Ok(ControlFlow::None(last))
            }
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                let saved_for_init = if let Some(Stmt::VarDecl { name, .. }) = init.as_deref() {
                    Some((
                        compact_str::CompactString::new(name),
                        self.env_vars.get(name.as_str()).cloned(),
                    ))
                } else {
                    None
                };
                let loop_res = (|| -> Result<ControlFlow, SandboxViolation> {
                    if let Some(init_stmt) = init {
                        self.eval_stmt(init_stmt)?;
                    }
                    let mut last = JsValue::Undefined;
                    let mut iters: u64 = 0;
                    loop {
                        if let Some(c) = cond
                            && !self.eval_expr(c)?.is_truthy()
                        {
                            break;
                        }
                        iters += 1;
                        self.compartment.consume_fuel(1)?;
                        self.check_tier0_deadline_and_interrupt(iters)?;
                        if iters == TIER1_PROMOTION_THRESHOLD {
                            self.current_tier = ExecutionTier::Tier1Baseline;
                        } else if iters == 10_000 {
                            self.current_tier = ExecutionTier::Tier2PeakNative;
                        }
                        match self.eval_block(body)? {
                            ControlFlow::None(v) => last = v,
                            ControlFlow::Return(v) => return Ok(ControlFlow::Return(v)),
                            ControlFlow::Break => break,
                            ControlFlow::Continue => {}
                        }
                        if let Some(s) = step {
                            self.eval_expr(s)?;
                        }
                    }
                    Ok(ControlFlow::None(last))
                })();
                if let Some((k, prev_opt)) = saved_for_init {
                    if let Some(prev) = prev_opt {
                        self.env_vars.insert(k, prev);
                    } else {
                        self.env_vars.remove(k.as_str());
                    }
                }
                loop_res
            }
            Stmt::Return(expr_opt) => {
                let val = if let Some(e) = expr_opt {
                    self.eval_expr(e)?
                } else {
                    JsValue::Undefined
                };
                Ok(ControlFlow::Return(val))
            }
            Stmt::Break => Ok(ControlFlow::Break),
            Stmt::Continue => Ok(ControlFlow::Continue),
            Stmt::Block(stmts) => self.eval_block(stmts),
            Stmt::ExprStmt(expr) => {
                let v = self.eval_expr(expr)?;
                Ok(ControlFlow::None(v))
            }
        }
    }

    pub(crate) fn eval_block(&mut self, stmts: &[Stmt]) -> Result<ControlFlow, SandboxViolation> {
        let mut saved_outer: Vec<(compact_str::CompactString, Option<JsValue>)> = Vec::new();
        for s in stmts {
            if let Stmt::VarDecl { name, .. } = s
                && !saved_outer.iter().any(|(k, _)| k.as_str() == name.as_str())
            {
                saved_outer.push((
                    compact_str::CompactString::new(name),
                    self.env_vars.get(name.as_str()).cloned(),
                ));
            }
        }
        let mut last = JsValue::Undefined;
        let mut out: Result<ControlFlow, SandboxViolation> = Ok(ControlFlow::None(last.clone()));
        let mut returned_early = false;
        for s in stmts {
            if self.pending_tier0_throw.is_some() {
                break;
            }
            match self.eval_stmt(s) {
                Ok(ControlFlow::None(v)) => {
                    if self.pending_tier0_throw.is_some() {
                        break;
                    }
                    last = v;
                }
                Ok(other) => {
                    out = Ok(other);
                    returned_early = true;
                    break;
                }
                Err(err) => {
                    out = Err(err);
                    returned_early = true;
                    break;
                }
            }
        }
        for (k, prev_opt) in saved_outer {
            if let Some(prev) = prev_opt {
                self.env_vars.insert(k, prev);
            } else {
                self.env_vars.remove(k.as_str());
            }
        }
        if returned_early {
            out
        } else {
            Ok(ControlFlow::None(last))
        }
    }
}
