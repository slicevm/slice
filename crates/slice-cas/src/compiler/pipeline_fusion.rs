//! Opt-In Hygienic Oxc AST Array Pipeline Fusion Pass (`fuse_array_pipelines_ast`).
//!
//! Note: Standard TypeScript/JavaScript compilation (`strip_typescript_raw_with_ext`) delegates
//! higher-order iteration directly to `boa_engine`'s spec-compliant `Array.prototype` and
//! `Iterator.prototype` intrinsics so arbitrary getters, `Proxy` traps, and custom receivers
//! preserve exact ECMA-262 observable semantics. Callers invoking `fuse_array_pipelines_ast`
//! explicitly receive hygienic temporary prefixes, sparse-hole guards, frozen loop bounds,
//! runtime `Array.isArray` + own-method guards, and multi-stage side-effect bailouts.

fn choose_hygienic_fusion_prefix(code: &str) -> String {
    if !code.contains("__sl_") {
        return "__sl_".to_string();
    }
    let mut idx = 0usize;
    loop {
        let candidate = format!("__sl{idx}_");
        if !code.contains(&candidate) {
            return candidate;
        }
        idx += 1;
    }
}

/// Opt-in Oxc AST Array Pipeline Fusion Pass:
/// Inspects the Oxc AST of `code` and fuses pure higher-order array expressions into a single
/// allocation-free `for` loop IIFE with collision-free temporaries, `Array.isArray` dispatch
/// fallback, initial `.length >>> 0` capture, and sparse-hole (`i in arr`) skipping:
/// - `<arr_expr>.map((<x>) => <map_body>).reduce((<acc>, <curr>) => <red_body>, <init>)`
/// - `<arr_expr>.filter((<x>) => <pred>).map((<y>) => <map_body>).reduce((<acc>, <curr>) => <red_body>, <init>)`
/// - `<arr_expr>.some((<x>) => <pred>)`
/// - `<arr_expr>.every((<x>) => <pred>)`
pub fn fuse_array_pipelines_ast(code: &str) -> String {
    if !code.contains(".reduce(") && !code.contains(".some(") && !code.contains(".every(") {
        return code.to_string();
    }

    let allocator = oxc::allocator::Allocator::default();
    let source_type = oxc::span::SourceType::mjs();
    let parser_ret = oxc::parser::Parser::new(&allocator, code, source_type).parse();
    if !parser_ret.diagnostics.is_empty() {
        return code.to_string();
    }

    use crate::linker::{collect_expr_identifier_edits, slice_with_edits};
    use oxc::ast::ast::{Argument, BindingPattern, Expression, Statement, UnaryOperator};
    use oxc::span::GetSpan;
    use rustc_hash::FxHashMap;

    let pfx = choose_hygienic_fusion_prefix(code);

    struct Replacement {
        start: usize,
        end: usize,
        text: String,
    }

    fn expr_contains_fusion_hazard(expr: &Expression<'_>, src: &str) -> bool {
        use oxc::ast::ast::{ArrayExpressionElement, ChainElement, ObjectPropertyKind};
        match expr {
            Expression::AwaitExpression(_)
            | Expression::YieldExpression(_)
            | Expression::ThisExpression(_)
            | Expression::Super(_) => true,
            Expression::Identifier(id) => id.name.as_str() == "arguments",
            Expression::ParenthesizedExpression(p) => {
                expr_contains_fusion_hazard(&p.expression, src)
            }
            Expression::CallExpression(c) => {
                expr_contains_fusion_hazard(&c.callee, src)
                    || c.arguments.iter().any(|a| match a {
                        Argument::SpreadElement(sp) => {
                            expr_contains_fusion_hazard(&sp.argument, src)
                        }
                        other => other
                            .as_expression()
                            .is_some_and(|e| expr_contains_fusion_hazard(e, src)),
                    })
            }
            Expression::NewExpression(n) => {
                expr_contains_fusion_hazard(&n.callee, src)
                    || n.arguments.iter().any(|a| match a {
                        Argument::SpreadElement(sp) => {
                            expr_contains_fusion_hazard(&sp.argument, src)
                        }
                        other => other
                            .as_expression()
                            .is_some_and(|e| expr_contains_fusion_hazard(e, src)),
                    })
            }
            Expression::StaticMemberExpression(m) => expr_contains_fusion_hazard(&m.object, src),
            Expression::ComputedMemberExpression(m) => {
                expr_contains_fusion_hazard(&m.object, src)
                    || expr_contains_fusion_hazard(&m.expression, src)
            }
            Expression::PrivateFieldExpression(m) => expr_contains_fusion_hazard(&m.object, src),
            Expression::BinaryExpression(b) => {
                expr_contains_fusion_hazard(&b.left, src)
                    || expr_contains_fusion_hazard(&b.right, src)
            }
            Expression::LogicalExpression(l) => {
                expr_contains_fusion_hazard(&l.left, src)
                    || expr_contains_fusion_hazard(&l.right, src)
            }
            Expression::UnaryExpression(u) => expr_contains_fusion_hazard(&u.argument, src),
            Expression::UpdateExpression(u) => match &u.argument {
                oxc::ast::ast::SimpleAssignmentTarget::StaticMemberExpression(m) => {
                    expr_contains_fusion_hazard(&m.object, src)
                }
                oxc::ast::ast::SimpleAssignmentTarget::ComputedMemberExpression(m) => {
                    expr_contains_fusion_hazard(&m.object, src)
                        || expr_contains_fusion_hazard(&m.expression, src)
                }
                oxc::ast::ast::SimpleAssignmentTarget::PrivateFieldExpression(m) => {
                    expr_contains_fusion_hazard(&m.object, src)
                }
                _ => false,
            },
            Expression::ConditionalExpression(c) => {
                expr_contains_fusion_hazard(&c.test, src)
                    || expr_contains_fusion_hazard(&c.consequent, src)
                    || expr_contains_fusion_hazard(&c.alternate, src)
            }
            Expression::AssignmentExpression(a) => {
                let left_hazard = match &a.left {
                    oxc::ast::ast::AssignmentTarget::StaticMemberExpression(m) => {
                        expr_contains_fusion_hazard(&m.object, src)
                    }
                    oxc::ast::ast::AssignmentTarget::ComputedMemberExpression(m) => {
                        expr_contains_fusion_hazard(&m.object, src)
                            || expr_contains_fusion_hazard(&m.expression, src)
                    }
                    oxc::ast::ast::AssignmentTarget::PrivateFieldExpression(m) => {
                        expr_contains_fusion_hazard(&m.object, src)
                    }
                    _ => false,
                };
                left_hazard || expr_contains_fusion_hazard(&a.right, src)
            }
            Expression::SequenceExpression(seq) => seq
                .expressions
                .iter()
                .any(|e| expr_contains_fusion_hazard(e, src)),
            Expression::ObjectExpression(obj) => obj.properties.iter().any(|prop| match prop {
                ObjectPropertyKind::ObjectProperty(p) => {
                    (p.computed
                        && p.key
                            .as_expression()
                            .is_some_and(|e| expr_contains_fusion_hazard(e, src)))
                        || expr_contains_fusion_hazard(&p.value, src)
                }
                ObjectPropertyKind::SpreadProperty(sp) => {
                    expr_contains_fusion_hazard(&sp.argument, src)
                }
            }),
            Expression::ArrayExpression(arr) => arr.elements.iter().any(|el| match el {
                ArrayExpressionElement::SpreadElement(sp) => {
                    expr_contains_fusion_hazard(&sp.argument, src)
                }
                other => other
                    .as_expression()
                    .is_some_and(|e| expr_contains_fusion_hazard(e, src)),
            }),
            Expression::TemplateLiteral(tpl) => tpl
                .expressions
                .iter()
                .any(|e| expr_contains_fusion_hazard(e, src)),
            Expression::TaggedTemplateExpression(tt) => {
                expr_contains_fusion_hazard(&tt.tag, src)
                    || tt
                        .quasi
                        .expressions
                        .iter()
                        .any(|e| expr_contains_fusion_hazard(e, src))
            }
            Expression::ChainExpression(ch) => match &ch.expression {
                ChainElement::CallExpression(c) => {
                    expr_contains_fusion_hazard(&c.callee, src)
                        || c.arguments.iter().any(|a| {
                            a.as_expression()
                                .is_some_and(|e| expr_contains_fusion_hazard(e, src))
                        })
                }
                ChainElement::StaticMemberExpression(m) => {
                    expr_contains_fusion_hazard(&m.object, src)
                }
                ChainElement::ComputedMemberExpression(m) => {
                    expr_contains_fusion_hazard(&m.object, src)
                        || expr_contains_fusion_hazard(&m.expression, src)
                }
                ChainElement::PrivateFieldExpression(m) => {
                    expr_contains_fusion_hazard(&m.object, src)
                }
                ChainElement::TSNonNullExpression(n) => {
                    expr_contains_fusion_hazard(&n.expression, src)
                }
            },
            Expression::ArrowFunctionExpression(arrow) => {
                if arrow.r#async {
                    return true;
                }
                match &arrow.body {
                    oxc::ast::ast::ArrowFunctionBody::FunctionBody(_) => true,
                    _ => arrow
                        .body
                        .as_expression()
                        .is_some_and(|e| expr_contains_fusion_hazard(e, src)),
                }
            }
            Expression::FunctionExpression(_) | Expression::ClassExpression(_) => false,
            other => {
                let sp = other.span();
                matches!(
                    src.get(sp.start as usize..sp.end as usize),
                    Some("new.target" | "import.meta")
                )
            }
        }
    }

    /// Rejects expressions that may perform observable side effects across stages when fusing
    /// multi-stage pipelines (`map`+`reduce`, `filter`+`map`+`reduce`), preventing callback or
    /// `<init>` side-effect reordering relative to ECMA-262 eager stage execution.
    fn expr_has_observable_side_effect(expr: &Expression<'_>) -> bool {
        use oxc::ast::ast::{
            ArrayExpressionElement, ChainElement, ObjectPropertyKind, PropertyKind,
        };
        match expr {
            Expression::CallExpression(_)
            | Expression::NewExpression(_)
            | Expression::AssignmentExpression(_)
            | Expression::UpdateExpression(_)
            | Expression::TaggedTemplateExpression(_)
            | Expression::AwaitExpression(_)
            | Expression::YieldExpression(_)
            | Expression::ImportExpression(_) => true,
            Expression::UnaryExpression(u) => {
                u.operator == UnaryOperator::Delete || expr_has_observable_side_effect(&u.argument)
            }
            Expression::SequenceExpression(seq) => {
                seq.expressions.iter().any(expr_has_observable_side_effect)
            }
            Expression::ParenthesizedExpression(p) => {
                expr_has_observable_side_effect(&p.expression)
            }
            Expression::StaticMemberExpression(m) => expr_has_observable_side_effect(&m.object),
            Expression::ComputedMemberExpression(m) => {
                expr_has_observable_side_effect(&m.object)
                    || expr_has_observable_side_effect(&m.expression)
            }
            Expression::PrivateFieldExpression(m) => expr_has_observable_side_effect(&m.object),
            Expression::BinaryExpression(b) => {
                expr_has_observable_side_effect(&b.left)
                    || expr_has_observable_side_effect(&b.right)
            }
            Expression::LogicalExpression(l) => {
                expr_has_observable_side_effect(&l.left)
                    || expr_has_observable_side_effect(&l.right)
            }
            Expression::ConditionalExpression(c) => {
                expr_has_observable_side_effect(&c.test)
                    || expr_has_observable_side_effect(&c.consequent)
                    || expr_has_observable_side_effect(&c.alternate)
            }
            Expression::ObjectExpression(obj) => obj.properties.iter().any(|prop| match prop {
                ObjectPropertyKind::ObjectProperty(p) => {
                    p.kind != PropertyKind::Init
                        || (p.computed
                            && p.key
                                .as_expression()
                                .is_some_and(expr_has_observable_side_effect))
                        || expr_has_observable_side_effect(&p.value)
                }
                ObjectPropertyKind::SpreadProperty(sp) => {
                    expr_has_observable_side_effect(&sp.argument)
                }
            }),
            Expression::ArrayExpression(arr) => arr.elements.iter().any(|el| match el {
                ArrayExpressionElement::SpreadElement(sp) => {
                    expr_has_observable_side_effect(&sp.argument)
                }
                other => other
                    .as_expression()
                    .is_some_and(expr_has_observable_side_effect),
            }),
            Expression::TemplateLiteral(tpl) => {
                tpl.expressions.iter().any(expr_has_observable_side_effect)
            }
            Expression::ChainExpression(ch) => match &ch.expression {
                ChainElement::CallExpression(_) => true,
                ChainElement::StaticMemberExpression(m) => {
                    expr_has_observable_side_effect(&m.object)
                }
                ChainElement::ComputedMemberExpression(m) => {
                    expr_has_observable_side_effect(&m.object)
                        || expr_has_observable_side_effect(&m.expression)
                }
                ChainElement::PrivateFieldExpression(m) => {
                    expr_has_observable_side_effect(&m.object)
                }
                ChainElement::TSNonNullExpression(n) => {
                    expr_has_observable_side_effect(&n.expression)
                }
            },
            _ => false,
        }
    }

    fn is_definitely_non_array_receiver(expr: &Expression<'_>) -> bool {
        matches!(
            expr.without_parentheses(),
            Expression::ObjectExpression(_)
                | Expression::FunctionExpression(_)
                | Expression::ArrowFunctionExpression(_)
                | Expression::ClassExpression(_)
                | Expression::StringLiteral(_)
                | Expression::NumericLiteral(_)
                | Expression::BooleanLiteral(_)
                | Expression::NullLiteral(_)
                | Expression::BigIntLiteral(_)
                | Expression::RegExpLiteral(_)
        )
    }

    fn rewrite_arrow_expr_body(
        body_expr: &Expression<'_>,
        src: &str,
        rename_map: &FxHashMap<String, String>,
    ) -> String {
        let sp = body_expr.span();
        let mut edits = Vec::new();
        collect_expr_identifier_edits(body_expr, rename_map, &mut edits);
        edits.sort_by_key(|e| e.0);
        slice_with_edits(src, sp.start as usize, sp.end as usize, &edits)
            .trim()
            .to_string()
    }

    fn extract_single_param_arrow<'a>(
        arg: &'a Argument<'a>,
        src: &'a str,
    ) -> Option<(&'a str, &'a Expression<'a>, &'a str)> {
        let expr = arg.as_expression()?;
        let Expression::ArrowFunctionExpression(arrow) = expr.without_parentheses() else {
            return None;
        };
        if arrow.r#async || arrow.params.items.len() != 1 || arrow.params.rest.is_some() {
            return None;
        }
        let param = &arrow.params.items[0];
        if param.initializer.is_some() {
            return None;
        }
        let BindingPattern::BindingIdentifier(ident) = &param.pattern else {
            return None;
        };
        let body_expr = arrow.body.as_expression()?;
        if expr_contains_fusion_hazard(body_expr, src) {
            return None;
        }
        let sp = expr.span();
        Some((
            ident.name.as_str(),
            body_expr,
            &src[sp.start as usize..sp.end as usize],
        ))
    }

    fn extract_two_param_arrow<'a>(
        arg: &'a Argument<'a>,
        src: &'a str,
    ) -> Option<(&'a str, &'a str, &'a Expression<'a>, &'a str)> {
        let expr = arg.as_expression()?;
        let Expression::ArrowFunctionExpression(arrow) = expr.without_parentheses() else {
            return None;
        };
        if arrow.r#async || arrow.params.items.len() != 2 || arrow.params.rest.is_some() {
            return None;
        }
        let p0 = &arrow.params.items[0];
        let p1 = &arrow.params.items[1];
        if p0.initializer.is_some() || p1.initializer.is_some() {
            return None;
        }
        let BindingPattern::BindingIdentifier(id0) = &p0.pattern else {
            return None;
        };
        let BindingPattern::BindingIdentifier(id1) = &p1.pattern else {
            return None;
        };
        let body_expr = arrow.body.as_expression()?;
        if expr_contains_fusion_hazard(body_expr, src) {
            return None;
        }
        let sp = expr.span();
        Some((
            id0.name.as_str(),
            id1.name.as_str(),
            body_expr,
            &src[sp.start as usize..sp.end as usize],
        ))
    }

    fn try_fuse_call<'a>(
        call: &'a oxc::ast::ast::CallExpression<'a>,
        src: &'a str,
        pfx: &str,
        counter: &mut usize,
    ) -> Option<Replacement> {
        let Expression::StaticMemberExpression(outer_mem) = call.callee.without_parentheses()
        else {
            return None;
        };
        let outer_method = outer_mem.property.name.as_str();

        // Case 1: `<arr>.some((x) => <pred>)` or `<arr>.every((x) => <pred>)`
        if (outer_method == "some" || outer_method == "every") && call.arguments.len() == 1 {
            let (param, pred_expr, cb_src) = extract_single_param_arrow(&call.arguments[0], src)?;
            if is_definitely_non_array_receiver(&outer_mem.object)
                || expr_contains_fusion_hazard(&outer_mem.object, src)
            {
                return None;
            }
            let arr_sp = outer_mem.object.span();
            let arr_src = &src[arr_sp.start as usize..arr_sp.end as usize];
            *counter += 1;
            let id = *counter;
            let a_tmp = format!("{pfx}a{id}");
            let len_tmp = format!("{pfx}len{id}");
            let i_tmp = format!("{pfx}i{id}");
            let p_tmp = format!("{pfx}v{id}");
            let mut rmap = FxHashMap::default();
            rmap.insert(param.to_string(), p_tmp.clone());
            let pred_src = rewrite_arrow_expr_body(pred_expr, src, &rmap);
            let (early_cond, final_ret) = if outer_method == "some" {
                (pred_src, "false")
            } else {
                (format!("!({pred_src})"), "true")
            };
            let early_ret = if outer_method == "some" {
                "true"
            } else {
                "false"
            };
            let fused = format!(
                "(() => {{ const {a} = {arr}; if (!Array.isArray({a}) || {a}[{m:?}] !== Array.prototype[{m:?}]) return {a}[{m:?}]({cb}); const {len} = {a}.length >>> 0; for (let {i} = 0; {i} < {len}; {i}++) {{ if (!({i} in {a})) continue; let {param} = {a}[{i}]; if ({cond}) return {early}; }} return {fin}; }})()",
                a = a_tmp,
                arr = arr_src,
                m = outer_method,
                cb = cb_src,
                len = len_tmp,
                i = i_tmp,
                param = p_tmp,
                cond = early_cond,
                early = early_ret,
                fin = final_ret
            );
            return Some(Replacement {
                start: call.span.start as usize,
                end: call.span.end as usize,
                text: fused,
            });
        }

        // Case 2: `<inner>.reduce((acc, curr) => <red_body>, <init>)`
        if outer_method == "reduce" && call.arguments.len() == 2 {
            let (acc_param, curr_param, red_expr, red_cb_src) =
                extract_two_param_arrow(&call.arguments[0], src)?;
            let init_expr = call.arguments[1].as_expression()?;
            if expr_contains_fusion_hazard(init_expr, src)
                || expr_has_observable_side_effect(red_expr)
                || expr_has_observable_side_effect(init_expr)
            {
                return None;
            }
            let init_sp = init_expr.span();
            let init_src = &src[init_sp.start as usize..init_sp.end as usize];

            // Check if `<inner>` is `<arr>.map((x) => <map_body>)`
            if let Expression::CallExpression(inner_call) = outer_mem.object.without_parentheses() {
                if let Expression::StaticMemberExpression(inner_mem) =
                    inner_call.callee.without_parentheses()
                {
                    if inner_mem.property.name.as_str() == "map" && inner_call.arguments.len() == 1
                    {
                        let (map_param, map_expr, map_cb_src) =
                            extract_single_param_arrow(&inner_call.arguments[0], src)?;
                        if expr_has_observable_side_effect(map_expr) {
                            return None;
                        }

                        // Check if `<arr>` is `<base>.filter((f) => <filt_body>)`
                        if let Expression::CallExpression(filt_call) =
                            inner_mem.object.without_parentheses()
                        {
                            if let Expression::StaticMemberExpression(filt_mem) =
                                filt_call.callee.without_parentheses()
                            {
                                if filt_mem.property.name.as_str() == "filter"
                                    && filt_call.arguments.len() == 1
                                {
                                    if let Some((filt_param, filt_expr, filt_cb_src)) =
                                        extract_single_param_arrow(&filt_call.arguments[0], src)
                                    {
                                        if is_definitely_non_array_receiver(&filt_mem.object)
                                            || expr_contains_fusion_hazard(&filt_mem.object, src)
                                            || expr_has_observable_side_effect(filt_expr)
                                        {
                                            return None;
                                        }
                                        let base_sp = filt_mem.object.span();
                                        let base_src =
                                            &src[base_sp.start as usize..base_sp.end as usize];
                                        *counter += 1;
                                        let id = *counter;
                                        let a_tmp = format!("{pfx}a{id}");
                                        let len_tmp = format!("{pfx}len{id}");
                                        let i_tmp = format!("{pfx}i{id}");
                                        let acc_tmp = format!("{pfx}acc{id}");
                                        let fp_tmp = format!("{pfx}f{id}");
                                        let mp_tmp = format!("{pfx}m{id}");
                                        let cp_tmp = format!("{pfx}c{id}");

                                        let mut fmap = FxHashMap::default();
                                        fmap.insert(filt_param.to_string(), fp_tmp.clone());
                                        let filt_body =
                                            rewrite_arrow_expr_body(filt_expr, src, &fmap);

                                        let mut mmap = FxHashMap::default();
                                        mmap.insert(map_param.to_string(), mp_tmp.clone());
                                        let map_body =
                                            rewrite_arrow_expr_body(map_expr, src, &mmap);

                                        let mut rmap = FxHashMap::default();
                                        rmap.insert(acc_param.to_string(), acc_tmp.clone());
                                        rmap.insert(curr_param.to_string(), cp_tmp.clone());
                                        let red_body =
                                            rewrite_arrow_expr_body(red_expr, src, &rmap);

                                        let fused = format!(
                                            "(() => {{ const {a} = {base}; if (!Array.isArray({a}) || {a}[\"filter\"] !== Array.prototype[\"filter\"] || {a}[\"map\"] !== Array.prototype[\"map\"] || {a}[\"reduce\"] !== Array.prototype[\"reduce\"]) return {a}[\"filter\"]({fcb})[\"map\"]({mcb})[\"reduce\"]({rcb}, {init}); let {acc} = {init}; const {len} = {a}.length >>> 0; for (let {i} = 0; {i} < {len}; {i}++) {{ if (!({i} in {a})) continue; let {fp} = {a}[{i}]; if (!({fb})) continue; let {mp} = {fp}; let {cp} = ({mb}); {acc} = ({rb}); }} return {acc}; }})()",
                                            a = a_tmp,
                                            base = base_src,
                                            fcb = filt_cb_src,
                                            mcb = map_cb_src,
                                            rcb = red_cb_src,
                                            acc = acc_tmp,
                                            init = init_src,
                                            len = len_tmp,
                                            i = i_tmp,
                                            fp = fp_tmp,
                                            fb = filt_body,
                                            mp = mp_tmp,
                                            cp = cp_tmp,
                                            mb = map_body,
                                            rb = red_body
                                        );
                                        return Some(Replacement {
                                            start: call.span.start as usize,
                                            end: call.span.end as usize,
                                            text: fused,
                                        });
                                    }
                                }
                            }
                        }

                        if is_definitely_non_array_receiver(&inner_mem.object)
                            || expr_contains_fusion_hazard(&inner_mem.object, src)
                        {
                            return None;
                        }
                        let arr_sp = inner_mem.object.span();
                        let arr_src = &src[arr_sp.start as usize..arr_sp.end as usize];
                        *counter += 1;
                        let id = *counter;
                        let a_tmp = format!("{pfx}a{id}");
                        let len_tmp = format!("{pfx}len{id}");
                        let i_tmp = format!("{pfx}i{id}");
                        let acc_tmp = format!("{pfx}acc{id}");
                        let mp_tmp = format!("{pfx}m{id}");
                        let cp_tmp = format!("{pfx}c{id}");

                        let mut mmap = FxHashMap::default();
                        mmap.insert(map_param.to_string(), mp_tmp.clone());
                        let map_body = rewrite_arrow_expr_body(map_expr, src, &mmap);

                        let mut rmap = FxHashMap::default();
                        rmap.insert(acc_param.to_string(), acc_tmp.clone());
                        rmap.insert(curr_param.to_string(), cp_tmp.clone());
                        let red_body = rewrite_arrow_expr_body(red_expr, src, &rmap);

                        let fused = format!(
                            "(() => {{ const {a} = {arr}; if (!Array.isArray({a}) || {a}[\"map\"] !== Array.prototype[\"map\"] || {a}[\"reduce\"] !== Array.prototype[\"reduce\"]) return {a}[\"map\"]({mcb})[\"reduce\"]({rcb}, {init}); let {acc} = {init}; const {len} = {a}.length >>> 0; for (let {i} = 0; {i} < {len}; {i}++) {{ if (!({i} in {a})) continue; let {mp} = {a}[{i}]; let {cp} = ({mb}); {acc} = ({rb}); }} return {acc}; }})()",
                            a = a_tmp,
                            arr = arr_src,
                            mcb = map_cb_src,
                            rcb = red_cb_src,
                            acc = acc_tmp,
                            init = init_src,
                            len = len_tmp,
                            i = i_tmp,
                            mp = mp_tmp,
                            cp = cp_tmp,
                            mb = map_body,
                            rb = red_body
                        );
                        return Some(Replacement {
                            start: call.span.start as usize,
                            end: call.span.end as usize,
                            text: fused,
                        });
                    }
                }
            }
        }
        None
    }

    fn walk_expr<'a>(
        expr: &'a Expression<'a>,
        src: &'a str,
        pfx: &str,
        counter: &mut usize,
        out: &mut Vec<Replacement>,
    ) {
        match expr {
            Expression::CallExpression(call) => {
                if let Some(rep) = try_fuse_call(call, src, pfx, counter) {
                    out.push(rep);
                    return;
                }
                walk_expr(&call.callee, src, pfx, counter, out);
                for arg in &call.arguments {
                    if let Some(e) = arg.as_expression() {
                        walk_expr(e, src, pfx, counter, out);
                    }
                }
            }
            Expression::BinaryExpression(b) => {
                walk_expr(&b.left, src, pfx, counter, out);
                walk_expr(&b.right, src, pfx, counter, out);
            }
            Expression::LogicalExpression(l) => {
                walk_expr(&l.left, src, pfx, counter, out);
                walk_expr(&l.right, src, pfx, counter, out);
            }
            Expression::ConditionalExpression(c) => {
                walk_expr(&c.test, src, pfx, counter, out);
                walk_expr(&c.consequent, src, pfx, counter, out);
                walk_expr(&c.alternate, src, pfx, counter, out);
            }
            Expression::AssignmentExpression(a) => {
                walk_expr(&a.right, src, pfx, counter, out);
            }
            Expression::ParenthesizedExpression(p) => {
                walk_expr(&p.expression, src, pfx, counter, out);
            }
            Expression::ArrowFunctionExpression(arrow) => {
                if let oxc::ast::ast::ArrowFunctionBody::FunctionBody(fb) = &arrow.body {
                    for s in &fb.statements {
                        walk_stmt(s, src, pfx, counter, out);
                    }
                } else if let Some(e) = arrow.body.as_expression() {
                    walk_expr(e, src, pfx, counter, out);
                }
            }
            Expression::FunctionExpression(fe) => {
                if let Some(body) = &fe.body {
                    for s in &body.statements {
                        walk_stmt(s, src, pfx, counter, out);
                    }
                }
            }
            Expression::ObjectExpression(obj) => {
                for prop in &obj.properties {
                    if let oxc::ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop {
                        walk_expr(&p.value, src, pfx, counter, out);
                    }
                }
            }
            Expression::ArrayExpression(arr) => {
                for el in &arr.elements {
                    if let Some(e) = el.as_expression() {
                        walk_expr(e, src, pfx, counter, out);
                    }
                }
            }
            _ => {}
        }
    }

    fn walk_stmt<'a>(
        stmt: &'a Statement<'a>,
        src: &'a str,
        pfx: &str,
        counter: &mut usize,
        out: &mut Vec<Replacement>,
    ) {
        match stmt {
            Statement::VariableDeclaration(vd) => {
                for decl in &vd.declarations {
                    if let Some(init) = &decl.init {
                        walk_expr(init, src, pfx, counter, out);
                    }
                }
            }
            Statement::ExpressionStatement(es) => {
                walk_expr(&es.expression, src, pfx, counter, out);
            }
            Statement::ReturnStatement(rs) => {
                if let Some(arg) = &rs.argument {
                    walk_expr(arg, src, pfx, counter, out);
                }
            }
            Statement::BlockStatement(bs) => {
                for s in &bs.body {
                    walk_stmt(s, src, pfx, counter, out);
                }
            }
            Statement::IfStatement(ifs) => {
                walk_expr(&ifs.test, src, pfx, counter, out);
                walk_stmt(&ifs.consequent, src, pfx, counter, out);
                if let Some(alt) = &ifs.alternate {
                    walk_stmt(alt, src, pfx, counter, out);
                }
            }
            Statement::FunctionDeclaration(fd) => {
                if let Some(body) = &fd.body {
                    for s in &body.statements {
                        walk_stmt(s, src, pfx, counter, out);
                    }
                }
            }
            Statement::ClassDeclaration(cd) => {
                for el in &cd.body.body {
                    if let oxc::ast::ast::ClassElement::MethodDefinition(md) = el {
                        if let Some(body) = &md.value.body {
                            for s in &body.statements {
                                walk_stmt(s, src, pfx, counter, out);
                            }
                        }
                    }
                }
            }
            Statement::ExportDefaultDeclaration(edd) => {
                if let Some(expr) = edd.declaration.as_expression() {
                    walk_expr(expr, src, pfx, counter, out);
                }
            }
            _ => {}
        }
    }

    let mut replacements = Vec::new();
    let mut counter = 0usize;
    for stmt in &parser_ret.program.body {
        walk_stmt(stmt, code, &pfx, &mut counter, &mut replacements);
    }

    if replacements.is_empty() {
        return code.to_string();
    }

    replacements.sort_by_key(|r| r.start);
    let mut result = String::with_capacity(code.len() + replacements.len() * 128);
    let mut cursor = 0usize;
    for r in replacements {
        if r.start < cursor {
            continue;
        }
        result.push_str(&code[cursor..r.start]);
        result.push_str(&r.text);
        cursor = r.end;
    }
    result.push_str(&code[cursor..]);
    result
}
