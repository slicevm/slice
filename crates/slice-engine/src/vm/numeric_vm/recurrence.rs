//! Structural AST Pattern Detector for Second-Order Linear Recurrences (`T(n) = c1 * T(n - 1) + c2 * T(n - 2)`).

use crate::value_and_ast::{BinOp, Expr, JsFunction, JsValue, Stmt};

/// Structural AST pattern detector for 1-parameter linear recurrences:
/// Matches:
/// ```js
/// function anyName(param) {
///     if (param <= 1) return param; // or if (param < 2) return param;
///     return anyName(param - 1) + anyName(param - 2);
/// }
/// ```
pub(super) fn detect_linear_recurrence_ast(func: &JsFunction) -> Option<(f64, f64, f64, f64)> {
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
