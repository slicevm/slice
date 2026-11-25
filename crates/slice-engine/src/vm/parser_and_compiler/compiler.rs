//! Polyglot JS/TS Source Compiler (`compile_source_to_bytecode`, `SLICEBC\x01`, `SLICEBOA`, `SLICESNP\x02`).

use super::lexer::{Token, tokenize_js};
use super::parser_stmt::{Parser, is_tier0_guarded_proto_target};
use super::tier0_ast_gate::{
    analyze_tier0_slicesnp_ast, analyze_tier0_slicesnp_program, can_attempt_tier0_slicesnp,
};
use super::tier0_scope_and_hazards::{
    program_has_tier0_unsupported_mutation_syntax, source_has_tier0_unsupported_mutation_syntax,
};
use crate::slicesnp_codec::encode_slicesnp_v2;
use crate::value_and_ast::{BinOp, BytecodeModule, BytecodeOp, Expr, JsValue, Stmt};
use rustc_hash::FxHashMap;

/// Magic header for deferred compile-time syntax errors (`SLICEERR` + UTF-8 error text).
pub const SLICEERR_MAGIC: &[u8; 8] = b"SLICEERR";

pub(crate) use slice_cas::{compact_js_for_boa_bytecode, extract_slice_rc_key};

/// Compiles JavaScript/TypeScript source into a compact binary `SLICEBC\x01`, `SLICEBOA`, or `SLICESNP` bytecode buffer.
/// Combines **Oxc** production TypeScript transpilation, **Tier-1 `SLICEBC\x01` Precompiled Boa `CodeBlock`**
/// as the default optimal execution path, and **Tier-0/Tier-2** fast-path SIMD intrinsics
/// (selected via AST-verified `is_tier0_slicesnp_compatible_ast` unless `--tier=1` forces pure Tier-1 Boa).
pub fn compile_source_to_bytecode(raw_source: &str) -> Vec<u8> {
    compile_source_to_bytecode_inner(raw_source, true)
}

struct SandboxEvalCacheEntry {
    source: String,
    tier_override: Option<u8>,
    bytecode: std::rc::Rc<[u8]>,
}

thread_local! {
    static SANDBOX_EVAL_BC_CACHE: std::cell::RefCell<FxHashMap<(u64, usize), SandboxEvalCacheEntry>> =
        std::cell::RefCell::new(FxHashMap::default());
}

/// Compiles an inline `Slice.evalSandbox(code)` snippet for immediate in-memory execution,
/// using Tier-0 `SLICESNP` when compatible or lightweight `SLICEBOA` on a pooled `WarmBoaRealm`
/// without constructing a throwaway AOT `SLICEBC\x01` compiler context, backed by a bounded
/// 64-entry thread-local bytecode cache for repeated template/snippet evaluations.
#[inline]
pub(crate) fn compile_source_for_sandbox_eval(raw_source: &str) -> std::rc::Rc<[u8]> {
    if raw_source.len() <= 131_072 {
        use std::hash::{Hash, Hasher};
        let mut hasher = rustc_hash::FxHasher::default();
        raw_source.hash(&mut hasher);
        let key = (hasher.finish(), raw_source.len());
        let active_tier = crate::boacas::active_tier_override();
        let hit = SANDBOX_EVAL_BC_CACHE.with(|cache| {
            cache
                .borrow()
                .get(&key)
                .filter(|e| e.tier_override == active_tier && e.source == raw_source)
                .map(|e| e.bytecode.clone())
        });
        if let Some(bc) = hit {
            return bc;
        }
        let compiled: std::rc::Rc<[u8]> =
            compile_source_to_bytecode_inner(raw_source, false).into();
        SANDBOX_EVAL_BC_CACHE.with(|cache| {
            let mut map = cache.borrow_mut();
            if map.len() >= 64 {
                map.clear();
            }
            map.insert(
                key,
                SandboxEvalCacheEntry {
                    source: raw_source.to_string(),
                    tier_override: active_tier,
                    bytecode: compiled.clone(),
                },
            );
        });
        return compiled;
    }
    compile_source_to_bytecode_inner(raw_source, false).into()
}

fn compile_source_to_bytecode_inner(raw_source: &str, allow_boacas_aot: bool) -> Vec<u8> {
    let inline_dirs = if raw_source.starts_with("//") || raw_source.starts_with("#!") {
        slice_sandbox::parse_inline_directives(raw_source)
    } else {
        slice_sandbox::InlineDirectives::default()
    };
    let use_boacas = allow_boacas_aot
        && inline_dirs
            .boacas
            .unwrap_or_else(crate::boacas::is_exper_boacas_build_enabled);
    let tier_ov = inline_dirs
        .tier
        .unwrap_or_else(crate::boacas::active_tier_override);

    if raw_source.starts_with("var __slice_modules =")
        || memchr::memmem::find(raw_source.as_bytes(), b"\nvar __slice_modules =").is_some()
    {
        let compacted = compact_js_for_boa_bytecode(raw_source);
        if use_boacas {
            return crate::boacas::pack_boacas_container(&compacted);
        }
        let mut bytes = Vec::with_capacity(12 + compacted.len());
        bytes.extend_from_slice(b"SLICEBOA{\n");
        bytes.extend_from_slice(compacted.trim_end().as_bytes());
        bytes.extend_from_slice(b"\n}");
        return bytes;
    }
    let mut cached_tier0: Option<(bool, bool)> = None;
    let mut forced_tier0_needs_boa_ast: Option<bool> = None;
    let mut ast_has_top_level_return: Option<bool> = None;
    let source = match slice_cas::compile_ts_or_js_checked_with_ast_hook(
        raw_source,
        true,
        false,
        |program| {
            ast_has_top_level_return = Some(slice_cas::program_has_top_level_return(program));
            if tier_ov == Some(0) {
                forced_tier0_needs_boa_ast =
                    Some(program_has_tier0_unsupported_mutation_syntax(program));
            } else if can_attempt_tier0_slicesnp(raw_source) {
                cached_tier0 = Some(analyze_tier0_slicesnp_program(program));
            }
        },
    ) {
        Ok(src) => src,
        Err(diag) => {
            let err_str = diag.to_uncaught_error_string();
            let mut out = Vec::with_capacity(8 + err_str.len());
            out.extend_from_slice(SLICEERR_MAGIC);
            out.extend_from_slice(err_str.as_bytes());
            return out;
        }
    };
    let src_bytes = source.as_bytes();

    let has_while_true = memchr::memmem::find(src_bytes, b"while (true)").is_some()
        || memchr::memmem::find(src_bytes, b"while(true)").is_some();
    let has_proto_candidate = memchr::memmem::find(src_bytes, b"prototype").is_some();
    let (tier0_ast_ok, has_proto_mutation) = match tier_ov {
        Some(0) => {
            let needs_boa = memchr::memmem::find(src_bytes, b"?.").is_some()
                || forced_tier0_needs_boa_ast
                    .unwrap_or_else(|| source_has_tier0_unsupported_mutation_syntax(&source));
            (!needs_boa, false)
        }
        Some(1) if !has_while_true && !has_proto_candidate => (false, false),
        _ => {
            if !can_attempt_tier0_slicesnp(&source) {
                (false, false)
            } else if let Some(res) = cached_tier0 {
                res
            } else {
                analyze_tier0_slicesnp_ast(&source)
            }
        }
    };
    let has_direct_intrinsic_attack = (has_proto_mutation || (has_while_true && tier0_ast_ok))
        && memchr::memmem::find(src_bytes, b"Slice.").is_none()
        && memchr::memmem::find(src_bytes, b"Worker").is_none();

    let route_to_tier1_boa = match tier_ov {
        Some(1) => !has_direct_intrinsic_attack,
        Some(0) => !tier0_ast_ok,
        _ => !has_direct_intrinsic_attack && !tier0_ast_ok,
    };

    let tokens_opt = if route_to_tier1_boa {
        None
    } else {
        tokenize_js(&source)
    };

    if route_to_tier1_boa || (tokens_opt.is_none() && !has_direct_intrinsic_attack) {
        let esm_lowered: std::borrow::Cow<'_, str> =
            if slice_cas::needs_unbundled_esm_lowering(&source) {
                std::borrow::Cow::Owned(slice_cas::lower_unbundled_esm(&source))
            } else {
                source
            };
        // If `lower_unbundled_esm` produced a self-contained `var __slice_modules =` RC bundle
        // (`export default ...`), return it directly without wrapping in an outer TLA IIFE.
        if esm_lowered.starts_with("var __slice_modules =") {
            let compacted = compact_js_for_boa_bytecode(&esm_lowered);
            if use_boacas {
                return crate::boacas::pack_boacas_container(&compacted);
            }
            let mut bytes = Vec::with_capacity(12 + compacted.len());
            bytes.extend_from_slice(b"SLICEBOA{\n");
            bytes.extend_from_slice(compacted.trim_end().as_bytes());
            bytes.extend_from_slice(b"\n}");
            return bytes;
        }
        let boa_source: std::borrow::Cow<'_, str> =
            if let Some(wrapped) = slice_cas::wrap_top_level_await_script(&esm_lowered) {
                std::borrow::Cow::Owned(wrapped)
            } else if ast_has_top_level_return != Some(false)
                && let Some(wrapped) = slice_cas::wrap_top_level_return_script(&esm_lowered)
            {
                std::borrow::Cow::Owned(wrapped)
            } else {
                esm_lowered
            };
        if use_boacas {
            return crate::boacas::pack_boacas_container(&boa_source);
        }
        let mut bytes = Vec::with_capacity(12 + boa_source.len());
        bytes.extend_from_slice(b"SLICEBOA{\n");
        bytes.extend_from_slice(boa_source.as_bytes());
        bytes.extend_from_slice(b"\n}");
        return bytes;
    }

    let mut known_numeric_vars = if memchr::memchr(b':', raw_source.as_bytes()).is_some() {
        slice_cas::analyze_typescript_semantics(raw_source).numeric_vars
    } else {
        rustc_hash::FxHashSet::default()
    };
    let tokens = tokens_opt.unwrap_or_else(|| vec![Token::Eof]);
    let mut parser = Parser::new(tokens);
    let stmts = parser.parse_program();

    let mut ops = Vec::with_capacity(stmts.len());
    let mut static_exports = FxHashMap::default();

    for stmt in stmts {
        // Lower common top-level patterns into specialized Tier-0/Tier-2 fast-path opcodes
        match &stmt {
            Stmt::VarDecl {
                name,
                init: Some(Expr::Literal(JsValue::Number(n))),
                is_export,
            } => {
                known_numeric_vars.insert(name.clone());
                if *is_export {
                    static_exports.insert(name.clone(), *n);
                }
                ops.push(BytecodeOp::ConstNum {
                    dst: name.clone(),
                    val: *n,
                });
                continue;
            }
            Stmt::For {
                init: Some(init_stmt),
                cond:
                    Some(Expr::Binary {
                        op: BinOp::Lt,
                        lhs: cond_lhs,
                        rhs: cond_rhs,
                    }),
                step: Some(step_expr),
                body,
            } if body.len() == 1 => {
                if let (
                    Stmt::VarDecl {
                        name: idx_var,
                        init: Some(Expr::Literal(JsValue::Number(start_n))),
                        ..
                    },
                    Expr::Ident(cond_ident),
                    Expr::Literal(JsValue::Number(limit_n)),
                ) = (init_stmt.as_ref(), cond_lhs.as_ref(), cond_rhs.as_ref())
                {
                    let step_is_inc_1 = match step_expr {
                        Expr::Update {
                            name: s_name,
                            increment: true,
                            ..
                        } => s_name == idx_var,
                        Expr::Assign {
                            name: s_name,
                            op: Some(BinOp::Add),
                            value,
                        } => {
                            s_name == idx_var
                                && matches!(value.as_ref(), Expr::Literal(JsValue::Number(v)) if (*v - 1.0).abs() < f64::EPSILON)
                        }
                        _ => false,
                    };
                    if cond_ident == idx_var
                        && step_is_inc_1
                        && let Stmt::ExprStmt(Expr::Assign {
                            name: acc_var,
                            op: Some(BinOp::Add),
                            value: rhs_val,
                        }) = &body[0]
                        && (known_numeric_vars.contains(acc_var)
                            || known_numeric_vars.contains(idx_var))
                    {
                        let scale_opt = match rhs_val.as_ref() {
                            Expr::Ident(id) if id == idx_var => Some(1.0),
                            Expr::Binary {
                                op: BinOp::Mul,
                                lhs,
                                rhs,
                            } => match (lhs.as_ref(), rhs.as_ref()) {
                                (Expr::Ident(id), Expr::Literal(JsValue::Number(k)))
                                | (Expr::Literal(JsValue::Number(k)), Expr::Ident(id))
                                    if id == idx_var =>
                                {
                                    Some(*k)
                                }
                                _ => None,
                            },
                            _ => None,
                        };
                        if let Some(scale) = scale_opt {
                            ops.push(BytecodeOp::MonomorphicForLoop {
                                acc_var: acc_var.clone(),
                                idx_var: idx_var.clone(),
                                start: *start_n as i64,
                                limit: *limit_n as i64,
                                scale,
                            });
                            continue;
                        }
                    }
                }
            }
            Stmt::ImportCas {
                bindings,
                specifier,
            } => {
                let dst = bindings
                    .first()
                    .cloned()
                    .unwrap_or_else(|| specifier.clone());
                ops.push(BytecodeOp::ImportCas {
                    dst,
                    specifier: specifier.clone(),
                });
                continue;
            }
            Stmt::ExprStmt(Expr::PropAssign { target_chain, .. })
                if is_tier0_guarded_proto_target(target_chain) =>
            {
                ops.push(BytecodeOp::MutateIntrinsic {
                    target: target_chain.clone(),
                });
                continue;
            }
            Stmt::ExprStmt(Expr::IndexAssign { target, .. })
                if is_tier0_guarded_proto_target(target) =>
            {
                ops.push(BytecodeOp::MutateIntrinsic {
                    target: target.clone(),
                });
                continue;
            }
            Stmt::ExprStmt(Expr::Assign {
                name,
                op: Some(BinOp::Add),
                value,
            }) if !is_tier0_guarded_proto_target(name) => {
                if let Expr::Ident(src) = value.as_ref() {
                    ops.push(BytecodeOp::AddAssignVar {
                        dst: name.clone(),
                        src: src.clone(),
                    });
                    continue;
                }
            }
            Stmt::ExprStmt(Expr::Call { callee, args }) if args.len() == 1 => {
                if let (Expr::Member { object, property }, Expr::Ident(var)) =
                    (callee.as_ref(), &args[0])
                    && matches!(object.as_ref(), Expr::Ident(obj) if obj == "console")
                    && matches!(property.as_str(), "log" | "info" | "warn" | "error")
                {
                    ops.push(BytecodeOp::ConsoleLogVar { var: var.clone() });
                    continue;
                }
            }
            Stmt::While {
                cond: Expr::Literal(JsValue::Bool(true)),
                body,
            } if body.is_empty() => {
                ops.push(BytecodeOp::NumericLoopKernel {
                    dst: "__infinite".to_string(),
                    iterations: u64::MAX / 2,
                });
                continue;
            }
            _ => {}
        }

        ops.push(BytecodeOp::EvalStmt(stmt));
    }

    let module = BytecodeModule {
        ops,
        static_exports,
    };
    encode_slicesnp_v2(&module)
}
