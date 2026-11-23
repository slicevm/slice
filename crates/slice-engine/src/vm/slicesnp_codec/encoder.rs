//! Deterministic Binary `SLICESNP\x02` Snapshot Encoder (`BytecodeModule` -> `Vec<u8>`).

use super::{
    SLICESNP_FLAG_FLAT_LLINT, SLICESNP_FLAG_PURE_STATIC_EXPORTS, SLICESNP_V2_VERSION, bin_op_to_u8,
    unary_op_to_u8,
};
use crate::value_and_ast::{BytecodeModule, BytecodeOp, Expr, JsValue, SNAPSHOT_MAGIC, Stmt};

#[inline]
fn is_pure_static_exports_module(module: &BytecodeModule) -> bool {
    !module.static_exports.is_empty()
        && module.ops.len() == module.static_exports.len()
        && module.ops.iter().all(|op| match op {
            BytecodeOp::ConstNum { dst, val } => module
                .static_exports
                .get(dst)
                .is_some_and(|exp_v| exp_v.to_bits() == val.to_bits()),
            _ => false,
        })
}

#[inline]
fn is_flat_llint_module(module: &BytecodeModule) -> bool {
    !module.ops.is_empty()
        && module
            .ops
            .iter()
            .all(|op| !matches!(op, BytecodeOp::EvalStmt(_) | BytecodeOp::BoaFullEsVm { .. }))
}

/// Encodes a [`BytecodeModule`] into a compact deterministic `SLICESNP\x02` binary snapshot.
pub fn encode_slicesnp_v2(module: &BytecodeModule) -> Vec<u8> {
    let pure_static = is_pure_static_exports_module(module);
    let flat_llint = is_flat_llint_module(module);
    let mut flags: u8 = 0;
    if pure_static {
        flags |= SLICESNP_FLAG_PURE_STATIC_EXPORTS;
    }
    if flat_llint {
        flags |= SLICESNP_FLAG_FLAT_LLINT;
    }

    let est_cap = if pure_static {
        32 + module.static_exports.len() * 20
    } else {
        32 + module.ops.len() * 20
    };
    let mut out = Vec::with_capacity(est_cap);
    out.extend_from_slice(SNAPSHOT_MAGIC);
    out.push(SLICESNP_V2_VERSION);
    out.push(flags);

    write_u32(&mut out, module.static_exports.len() as u32);
    if module.static_exports.len() <= 1 {
        for (k, v) in &module.static_exports {
            write_str(&mut out, k);
            write_f64(&mut out, *v);
        }
    } else {
        let mut ordered_exports: Vec<(&String, f64)> =
            module.static_exports.iter().map(|(k, v)| (k, *v)).collect();
        ordered_exports.sort_unstable_by(|a, b| a.0.cmp(b.0));
        for &(k, v) in &ordered_exports {
            write_str(&mut out, k);
            write_f64(&mut out, v);
        }
    }

    if pure_static {
        write_u32(&mut out, 0);
    } else {
        write_u32(&mut out, module.ops.len() as u32);
        for op in &module.ops {
            encode_op(&mut out, op);
        }
    }
    out
}

#[inline]
fn write_u8(out: &mut Vec<u8>, v: u8) {
    out.push(v);
}

#[inline]
fn write_bool(out: &mut Vec<u8>, v: bool) {
    out.push(u8::from(v));
}

#[inline]
fn write_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

#[inline]
fn write_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

#[inline]
fn write_i64(out: &mut Vec<u8>, v: i64) {
    out.extend_from_slice(&v.to_le_bytes());
}

#[inline]
fn write_f64(out: &mut Vec<u8>, v: f64) {
    out.extend_from_slice(&v.to_le_bytes());
}

#[inline]
fn write_str(out: &mut Vec<u8>, s: &str) {
    write_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

#[inline]
fn write_str_slice(out: &mut Vec<u8>, items: &[String]) {
    write_u32(out, items.len() as u32);
    for s in items {
        write_str(out, s);
    }
}

fn encode_op(out: &mut Vec<u8>, op: &BytecodeOp) {
    match op {
        BytecodeOp::ConstNum { dst, val } => {
            write_u8(out, 0);
            write_str(out, dst);
            write_f64(out, *val);
        }
        BytecodeOp::ConstStr { dst, val } => {
            write_u8(out, 1);
            write_str(out, dst);
            write_str(out, val);
        }
        BytecodeOp::ImportCas { dst, specifier } => {
            write_u8(out, 2);
            write_str(out, dst);
            write_str(out, specifier);
        }
        BytecodeOp::AddNum { dst, lhs, rhs } => {
            write_u8(out, 3);
            write_str(out, dst);
            write_str(out, lhs);
            write_str(out, rhs);
        }
        BytecodeOp::NumericLoopKernel { dst, iterations } => {
            write_u8(out, 4);
            write_str(out, dst);
            write_u64(out, *iterations);
        }
        BytecodeOp::FibKernel { dst, n } => {
            write_u8(out, 5);
            write_str(out, dst);
            write_u64(out, *n);
        }
        BytecodeOp::CryptoDigest {
            dst,
            payload,
            rounds,
        } => {
            write_u8(out, 6);
            write_str(out, dst);
            write_str(out, payload);
            write_u64(out, *rounds);
        }
        BytecodeOp::JsonTransform {
            dst,
            json_payload,
            iterations,
        } => {
            write_u8(out, 7);
            write_str(out, dst);
            write_str(out, json_payload);
            write_u64(out, *iterations);
        }
        BytecodeOp::WinterTcRoute { dst, method, url } => {
            write_u8(out, 8);
            write_str(out, dst);
            write_str(out, method);
            write_str(out, url);
        }
        BytecodeOp::MutateIntrinsic { target } => {
            write_u8(out, 9);
            write_str(out, target);
        }
        BytecodeOp::VfsRead { dst, path } => {
            write_u8(out, 10);
            write_str(out, dst);
            write_str(out, path);
        }
        BytecodeOp::NetFetch { dst, host, path } => {
            write_u8(out, 11);
            write_str(out, dst);
            write_str(out, host);
            write_str(out, path);
        }
        BytecodeOp::EnvGet { dst, key } => {
            write_u8(out, 12);
            write_str(out, dst);
            write_str(out, key);
        }
        BytecodeOp::ConsoleLog { expr } => {
            write_u8(out, 13);
            write_str(out, expr);
        }
        BytecodeOp::ReturnVar { var } => {
            write_u8(out, 14);
            write_str(out, var);
        }
        BytecodeOp::MonomorphicForLoop {
            acc_var,
            idx_var,
            start,
            limit,
            scale,
        } => {
            write_u8(out, 15);
            write_str(out, acc_var);
            write_str(out, idx_var);
            write_i64(out, *start);
            write_i64(out, *limit);
            write_f64(out, *scale);
        }
        BytecodeOp::EvalStmt(stmt) => {
            write_u8(out, 16);
            encode_stmt(out, stmt);
        }
        BytecodeOp::BoaFullEsVm { js_code } => {
            write_u8(out, 17);
            write_str(out, js_code);
        }
        BytecodeOp::AddAssignVar { dst, src } => {
            write_u8(out, 18);
            write_str(out, dst);
            write_str(out, src);
        }
        BytecodeOp::ConsoleLogVar { var } => {
            write_u8(out, 19);
            write_str(out, var);
        }
    }
}

fn encode_stmt_slice(out: &mut Vec<u8>, stmts: &[Stmt]) {
    write_u32(out, stmts.len() as u32);
    for s in stmts {
        encode_stmt(out, s);
    }
}

fn encode_stmt(out: &mut Vec<u8>, stmt: &Stmt) {
    match stmt {
        Stmt::VarDecl {
            name,
            init,
            is_export,
        } => {
            write_u8(out, 0);
            write_str(out, name);
            write_bool(out, *is_export);
            match init {
                Some(e) => {
                    write_u8(out, 1);
                    encode_expr(out, e);
                }
                None => write_u8(out, 0),
            }
        }
        Stmt::ImportCas {
            bindings,
            specifier,
        } => {
            write_u8(out, 1);
            write_str_slice(out, bindings);
            write_str(out, specifier);
        }
        Stmt::FuncDecl { name, params, body } => {
            write_u8(out, 2);
            write_str(out, name);
            write_str_slice(out, params);
            encode_stmt_slice(out, body);
        }
        Stmt::If {
            cond,
            then_branch,
            else_branch,
        } => {
            write_u8(out, 3);
            encode_expr(out, cond);
            encode_stmt_slice(out, then_branch);
            encode_stmt_slice(out, else_branch);
        }
        Stmt::While { cond, body } => {
            write_u8(out, 4);
            encode_expr(out, cond);
            encode_stmt_slice(out, body);
        }
        Stmt::For {
            init,
            cond,
            step,
            body,
        } => {
            write_u8(out, 5);
            match init {
                Some(s) => {
                    write_u8(out, 1);
                    encode_stmt(out, s);
                }
                None => write_u8(out, 0),
            }
            match cond {
                Some(e) => {
                    write_u8(out, 1);
                    encode_expr(out, e);
                }
                None => write_u8(out, 0),
            }
            match step {
                Some(e) => {
                    write_u8(out, 1);
                    encode_expr(out, e);
                }
                None => write_u8(out, 0),
            }
            encode_stmt_slice(out, body);
        }
        Stmt::Return(opt) => {
            write_u8(out, 6);
            match opt {
                Some(e) => {
                    write_u8(out, 1);
                    encode_expr(out, e);
                }
                None => write_u8(out, 0),
            }
        }
        Stmt::Break => write_u8(out, 7),
        Stmt::Continue => write_u8(out, 8),
        Stmt::Block(stmts) => {
            write_u8(out, 9);
            encode_stmt_slice(out, stmts);
        }
        Stmt::ExprStmt(expr) => {
            write_u8(out, 10);
            encode_expr(out, expr);
        }
    }
}

fn encode_js_value(out: &mut Vec<u8>, val: &JsValue) {
    match val {
        JsValue::Undefined => write_u8(out, 0),
        JsValue::Null => write_u8(out, 1),
        JsValue::Bool(b) => {
            write_u8(out, 2);
            write_bool(out, *b);
        }
        JsValue::Number(n) => {
            write_u8(out, 3);
            write_f64(out, *n);
        }
        JsValue::Str(s) => {
            write_u8(out, 4);
            write_str(out, s);
        }
        JsValue::Array(items) => {
            write_u8(out, 5);
            write_u32(out, items.len() as u32);
            for item in items.iter() {
                encode_js_value(out, item);
            }
        }
        JsValue::Object(map) => {
            write_u8(out, 6);
            write_u32(out, map.len() as u32);
            if map.len() <= 1 {
                for (k, v) in map.iter() {
                    write_str(out, k);
                    encode_js_value(out, v);
                }
            } else {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort_unstable();
                for k in keys {
                    write_str(out, k);
                    encode_js_value(out, &map[k]);
                }
            }
        }
        JsValue::Function(func) => {
            write_u8(out, 7);
            write_str(out, &func.name);
            write_str_slice(out, &func.params);
            encode_stmt_slice(out, &func.body);
        }
    }
}

fn encode_expr(out: &mut Vec<u8>, expr: &Expr) {
    match expr {
        Expr::Literal(val) => {
            write_u8(out, 0);
            encode_js_value(out, val);
        }
        Expr::Ident(name) => {
            write_u8(out, 1);
            write_str(out, name);
        }
        Expr::Binary { op, lhs, rhs } => {
            write_u8(out, 2);
            write_u8(out, bin_op_to_u8(*op));
            encode_expr(out, lhs);
            encode_expr(out, rhs);
        }
        Expr::Unary { op, expr: inner } => {
            write_u8(out, 3);
            write_u8(out, unary_op_to_u8(*op));
            encode_expr(out, inner);
        }
        Expr::Assign { name, op, value } => {
            write_u8(out, 4);
            write_str(out, name);
            match op {
                Some(bop) => {
                    write_u8(out, 1);
                    write_u8(out, bin_op_to_u8(*bop));
                }
                None => write_u8(out, 0),
            }
            encode_expr(out, value);
        }
        Expr::PropAssign {
            target_chain,
            prop,
            value,
        } => {
            write_u8(out, 5);
            write_str(out, target_chain);
            write_str(out, prop);
            encode_expr(out, value);
        }
        Expr::IndexAssign {
            target,
            index,
            value,
        } => {
            write_u8(out, 6);
            write_str(out, target);
            encode_expr(out, index);
            encode_expr(out, value);
        }
        Expr::Update {
            name,
            increment,
            prefix,
        } => {
            write_u8(out, 7);
            write_str(out, name);
            write_bool(out, *increment);
            write_bool(out, *prefix);
        }
        Expr::Ternary {
            cond,
            then_expr,
            else_expr,
        } => {
            write_u8(out, 8);
            encode_expr(out, cond);
            encode_expr(out, then_expr);
            encode_expr(out, else_expr);
        }
        Expr::Call { callee, args } => {
            write_u8(out, 9);
            encode_expr(out, callee);
            write_u32(out, args.len() as u32);
            for arg in args {
                encode_expr(out, arg);
            }
        }
        Expr::Member { object, property } => {
            write_u8(out, 10);
            encode_expr(out, object);
            write_str(out, property);
        }
        Expr::Index { object, index } => {
            write_u8(out, 11);
            encode_expr(out, object);
            encode_expr(out, index);
        }
        Expr::ArrayLit(items) => {
            write_u8(out, 12);
            write_u32(out, items.len() as u32);
            for item in items {
                encode_expr(out, item);
            }
        }
        Expr::ObjectLit(entries) => {
            write_u8(out, 13);
            write_u32(out, entries.len() as u32);
            for (k, v) in entries {
                write_str(out, k);
                encode_expr(out, v);
            }
        }
        Expr::ArrowFunc { params, body } => {
            write_u8(out, 14);
            write_str_slice(out, params);
            encode_stmt_slice(out, body);
        }
    }
}
