//! Tier-0 Recursive-Descent AST & Function Evaluator (`eval_stmt`, `eval_expr`, `eval_call`, `invoke_function_val`).

use crate::isolate::{ControlFlow, SliceIsolate};
use crate::parser_and_compiler::expr_to_dotted_chain;
use crate::value_and_ast::{
    js_value_to_json_string, serde_json_to_js_value, BinOp, ExecutionTier, Expr, JsFunction,
    JsValue, Stmt, UnaryOp, TIER1_PROMOTION_THRESHOLD,
};
use rustc_hash::FxHashMap;
use slice_sandbox::SandboxViolation;

/// Maximum Tier-0 AST function recursion depth (bounded so even unoptimized debug builds
/// on 1 MB Windows thread stacks trap cleanly with `CpuFuelExhausted`).
pub const MAX_TIER0_RECURSION_DEPTH: u32 = if cfg!(debug_assertions) { 32 } else { 128 };

impl SliceIsolate {
    pub(crate) fn eval_stmt(&mut self, stmt: &Stmt) -> Result<ControlFlow, SandboxViolation> {
        self.compartment.consume_fuel(1)?;
        match stmt {
            Stmt::VarDecl { name, init, .. } => {
                let val = if let Some(expr) = init {
                    self.eval_expr(expr)?
                } else {
                    JsValue::Undefined
                };
                self.env_vars.insert(name.clone(), val.clone());
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
                self.env_vars.insert(name.clone(), func.clone());
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
                if let Some(init_stmt) = init {
                    self.eval_stmt(init_stmt)?;
                }
                let mut last = JsValue::Undefined;
                let mut iters: u64 = 0;
                loop {
                    if let Some(c) = cond {
                        if !self.eval_expr(c)?.is_truthy() {
                            break;
                        }
                    }
                    iters += 1;
                    self.compartment.consume_fuel(1)?;
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
        let mut last = JsValue::Undefined;
        for s in stmts {
            match self.eval_stmt(s)? {
                ControlFlow::None(v) => last = v,
                other => return Ok(other),
            }
        }
        Ok(ControlFlow::None(last))
    }

    pub(crate) fn eval_expr(&mut self, expr: &Expr) -> Result<JsValue, SandboxViolation> {
        self.compartment.consume_fuel(1)?;
        match expr {
            Expr::Literal(v) => Ok(v.clone()),
            Expr::Ident(name) => Ok(self
                .env_vars
                .get(name)
                .cloned()
                .unwrap_or(JsValue::Undefined)),
            Expr::Unary { op, expr: inner } => {
                let val = self.eval_expr(inner)?;
                match op {
                    UnaryOp::Neg => Ok(JsValue::Number(-val.to_number())),
                    UnaryOp::Pos => Ok(JsValue::Number(val.to_number())),
                    UnaryOp::Not => Ok(JsValue::Bool(!val.is_truthy())),
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
                        .get(name)
                        .cloned()
                        .unwrap_or(JsValue::Undefined);
                    self.eval_assign_op(*bin_op, prev, rhs)?
                } else {
                    rhs
                };
                self.env_vars.insert(name.clone(), final_val.clone());
                Ok(final_val)
            }
            Expr::PropAssign {
                target_chain,
                prop,
                value,
            } => {
                self.compartment.verify_property_mutation(target_chain)?;
                let val = self.eval_expr(value)?;
                if let Some(JsValue::Object(map)) = self.env_vars.get_mut(target_chain) {
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
            Expr::Update { name, increment } => {
                let prev = self
                    .env_vars
                    .get(name)
                    .map(|v| v.to_number())
                    .unwrap_or(0.0);
                let next = if *increment { prev + 1.0 } else { prev - 1.0 };
                self.env_vars.insert(name.clone(), JsValue::Number(next));
                Ok(JsValue::Number(next))
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
                if let Expr::Ident(obj_name) = &**object {
                    if obj_name == "Math" {
                        return Ok(match property.as_str() {
                            "PI" => JsValue::Number(std::f64::consts::PI),
                            "E" => JsValue::Number(std::f64::consts::E),
                            _ => JsValue::Undefined,
                        });
                    }
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
                        let i = idx.to_number() as usize;
                        Ok(a.get(i).cloned().unwrap_or(JsValue::Undefined))
                    }
                    JsValue::Str(s) => {
                        let i = idx.to_number() as usize;
                        Ok(s.chars()
                            .nth(i)
                            .map(|c| JsValue::str(c.to_string()))
                            .unwrap_or(JsValue::Undefined))
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
            BinOp::Eq | BinOp::StrictEq => Ok(JsValue::Bool(l == r)),
            BinOp::Neq | BinOp::StrictNeq => Ok(JsValue::Bool(l != r)),
            BinOp::Lt => Ok(JsValue::Bool(l.to_number() < r.to_number())),
            BinOp::Lte => Ok(JsValue::Bool(l.to_number() <= r.to_number())),
            BinOp::Gt => Ok(JsValue::Bool(l.to_number() > r.to_number())),
            BinOp::Gte => Ok(JsValue::Bool(l.to_number() >= r.to_number())),
            BinOp::BitAnd => Ok(JsValue::Number(
                ((l.to_number() as i64) & (r.to_number() as i64)) as f64,
            )),
            BinOp::BitOr => Ok(JsValue::Number(
                ((l.to_number() as i64) | (r.to_number() as i64)) as f64,
            )),
            BinOp::BitXor => Ok(JsValue::Number(
                ((l.to_number() as i64) ^ (r.to_number() as i64)) as f64,
            )),
            BinOp::Shl => Ok(JsValue::Number(
                ((l.to_number() as i64) << ((r.to_number() as u32) & 31)) as f64,
            )),
            BinOp::Shr => Ok(JsValue::Number(
                ((l.to_number() as i64) >> ((r.to_number() as u32) & 31)) as f64,
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
        let grow_bytes = if let Some(JsValue::Array(vec)) = self.env_vars.get(target) {
            let i = idx_val.to_number().max(0.0) as usize;
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
                    let i = idx_val.to_number().max(0.0) as usize;
                    if i >= vec.len() {
                        vec.resize(i + 1, JsValue::Undefined);
                    }
                    vec[i] = val.clone();
                }
                JsValue::Object(map) => {
                    map.insert(idx_val.to_string(), val.clone());
                }
                _ => {}
            }
        }
        Ok(val)
    }

    pub(crate) fn eval_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
    ) -> Result<JsValue, SandboxViolation> {
        let mut eval_args = Vec::with_capacity(args.len());
        for a in args {
            eval_args.push(self.eval_expr(a)?);
        }

        if let Expr::Member { object, property } = callee {
            if let Some(res) = self.eval_member_call(object, property, &eval_args)? {
                return Ok(res);
            }
        }

        if let Expr::Ident(fn_name) = callee {
            if let Some(user_fn @ JsValue::Function(_)) = self.env_vars.get(fn_name).cloned() {
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
        eval_args: &[JsValue],
    ) -> Result<Option<JsValue>, SandboxViolation> {
        let chain = expr_to_dotted_chain(object);
        match (chain.as_str(), property) {
            ("console", "log" | "info" | "warn" | "error") => {
                let formatted: Vec<String> = eval_args.iter().map(|v| v.to_string()).collect();
                let line = formatted.join(" ");
                self.stdout.push(line);
                return Ok(Some(JsValue::Undefined));
            }
            ("Math", method) => {
                let x = eval_args.first().map(|v| v.to_number()).unwrap_or(0.0);
                let y = eval_args.get(1).map(|v| v.to_number()).unwrap_or(0.0);
                let res = match method {
                    "abs" => x.abs(),
                    "floor" => x.floor(),
                    "ceil" => x.ceil(),
                    "round" => x.round(),
                    "sqrt" => x.sqrt(),
                    "pow" => x.powf(y),
                    "sin" => x.sin(),
                    "cos" => x.cos(),
                    "max" => eval_args
                        .iter()
                        .map(|v| v.to_number())
                        .fold(f64::NEG_INFINITY, f64::max),
                    "min" => eval_args
                        .iter()
                        .map(|v| v.to_number())
                        .fold(f64::INFINITY, f64::min),
                    _ => f64::NAN,
                };
                return Ok(Some(JsValue::Number(res)));
            }
            ("JSON", "stringify") => {
                let first = eval_args.first().cloned().unwrap_or(JsValue::Undefined);
                return Ok(Some(JsValue::str(js_value_to_json_string(&first))));
            }
            ("JSON", "parse") => {
                if let Some(JsValue::Str(s)) = eval_args.first() {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(s) {
                        return Ok(Some(serde_json_to_js_value(&val)));
                    }
                }
                return Ok(Some(JsValue::Null));
            }
            ("Object", "keys") => {
                if let Some(JsValue::Object(map)) = eval_args.first() {
                    let mut keys: Vec<String> = map.keys().cloned().collect();
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
                if let Some(JsValue::Array(vec)) = self.env_vars.get_mut(var_name) {
                    for item in eval_args {
                        vec.push(item.clone());
                    }
                    return Ok(Some(JsValue::Number(vec.len() as f64)));
                }
            } else if property == "pop" {
                if let Some(JsValue::Array(vec)) = self.env_vars.get_mut(var_name) {
                    return Ok(Some(vec.pop().unwrap_or(JsValue::Undefined)));
                }
            }
        }

        let target_val = self.eval_expr(object)?;
        match (target_val, property) {
            (JsValue::Array(items), "join") => {
                let sep = eval_args
                    .first()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| ",".to_string());
                let parts: Vec<String> = items.iter().map(|v| v.to_string()).collect();
                Ok(Some(JsValue::str(parts.join(&sep))))
            }
            (JsValue::Array(items), "includes") => {
                let needle = eval_args.first().cloned().unwrap_or(JsValue::Undefined);
                Ok(Some(JsValue::Bool(items.contains(&needle))))
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
                    let mut acc = eval_args.get(1).cloned().unwrap_or(JsValue::Number(0.0));
                    for (idx, item) in items.into_iter().enumerate() {
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
                let parts: Vec<JsValue> =
                    s.split(&sep).map(|p| JsValue::str(p.to_string())).collect();
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
                    .unwrap_or(0.0);
                Ok(Some(JsValue::Number(n)))
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
            // Innovation 1: If this function is a pure numeric kernel compiled into `NumericOp`
            // and all arguments are numbers/booleans, execute directly on the unboxed 64-bit register VM!
            if let Some(ref num_ir) = func.compiled_numeric {
                if args
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
                    let res = num_ir.execute(
                        &unboxed_args[..args.len().min(6)],
                        &mut steps,
                        remaining_fuel,
                        0,
                        &self.compartment.tenant_id,
                    )?;
                    let fuel_cost = (steps / 64).max(1);
                    self.compartment.consume_fuel(fuel_cost)?;
                    self.current_tier = ExecutionTier::Tier2PeakNative;
                    return Ok(JsValue::Number(res));
                }
            }

            if self.recursion_depth >= MAX_TIER0_RECURSION_DEPTH {
                return Err(SandboxViolation::CpuFuelExhausted {
                    tenant_id: self.compartment.tenant_id.clone(),
                    limit: self.compartment.fuel_limit,
                });
            }

            self.compartment.consume_fuel(2)?;
            self.recursion_depth += 1;

            // Save shadowed bindings
            let mut saved = Vec::with_capacity(func.params.len());
            for (idx, param_name) in func.params.iter().enumerate() {
                let prev = self.env_vars.get(param_name).cloned();
                saved.push((param_name.clone(), prev));
                let arg_val = args.get(idx).cloned().unwrap_or(JsValue::Undefined);
                self.env_vars.insert(param_name.clone(), arg_val);
            }

            let block_res = self.eval_block(&func.body);
            self.recursion_depth = self.recursion_depth.saturating_sub(1);

            for (param_name, prev_opt) in saved {
                if let Some(prev) = prev_opt {
                    self.env_vars.insert(param_name, prev);
                } else {
                    self.env_vars.remove(&param_name);
                }
            }

            let res = match block_res? {
                ControlFlow::Return(v) | ControlFlow::None(v) => v,
                ControlFlow::Break | ControlFlow::Continue => JsValue::Undefined,
            };
            Ok(res)
        } else {
            Ok(JsValue::Undefined)
        }
    }
}
