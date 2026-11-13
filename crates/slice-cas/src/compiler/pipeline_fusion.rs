//! General-Purpose Oxc AST Array Pipeline Fusion Pass (`fuse_array_pipelines_ast`).

/// General-Purpose Oxc AST Array Pipeline Fusion Pass:
/// Inspects the Oxc AST of `code` and fuses chained higher-order array expressions on **any**
/// receiver `<arr_expr>` into a single allocation-free `for` loop IIFE:
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
    use oxc::ast::ast::{Argument, BindingPattern, Expression, Statement};
    use oxc::span::GetSpan;
    use rustc_hash::FxHashMap;

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
    ) -> Option<(&'a str, &'a Expression<'a>)> {
        let expr = arg.as_expression()?;
        let Expression::ArrowFunctionExpression(arrow) = expr.without_parentheses() else {
            return None;
        };
        if arrow.r#async || arrow.params.items.len() != 1 || arrow.params.rest.is_some() {
            return None;
        }
        let BindingPattern::BindingIdentifier(ident) = &arrow.params.items[0].pattern else {
            return None;
        };
        let body_expr = arrow.body.as_expression()?;
        if expr_contains_fusion_hazard(body_expr, src) {
            return None;
        }
        Some((ident.name.as_str(), body_expr))
    }

    fn extract_two_param_arrow<'a>(
        arg: &'a Argument<'a>,
        src: &'a str,
    ) -> Option<(&'a str, &'a str, &'a Expression<'a>)> {
        let expr = arg.as_expression()?;
        let Expression::ArrowFunctionExpression(arrow) = expr.without_parentheses() else {
            return None;
        };
        if arrow.r#async || arrow.params.items.len() != 2 || arrow.params.rest.is_some() {
            return None;
        }
        let BindingPattern::BindingIdentifier(id0) = &arrow.params.items[0].pattern else {
            return None;
        };
        let BindingPattern::BindingIdentifier(id1) = &arrow.params.items[1].pattern else {
            return None;
        };
        let body_expr = arrow.body.as_expression()?;
        if expr_contains_fusion_hazard(body_expr, src) {
            return None;
        }
        Some((id0.name.as_str(), id1.name.as_str(), body_expr))
    }

    fn try_fuse_call<'a>(
        call: &'a oxc::ast::ast::CallExpression<'a>,
        src: &'a str,
        counter: &mut usize,
    ) -> Option<Replacement> {
        let Expression::StaticMemberExpression(outer_mem) = call.callee.without_parentheses()
        else {
            return None;
        };
        let outer_method = outer_mem.property.name.as_str();

        // Case 1: `<arr>.some((x) => <pred>)` or `<arr>.every((x) => <pred>)`
        if (outer_method == "some" || outer_method == "every") && call.arguments.len() == 1 {
            let (param, pred_expr) = extract_single_param_arrow(&call.arguments[0], src)?;
            if expr_contains_fusion_hazard(&outer_mem.object, src) {
                return None;
            }
            let arr_sp = outer_mem.object.span();
            let arr_src = &src[arr_sp.start as usize..arr_sp.end as usize];
            *counter += 1;
            let id = *counter;
            let p_tmp = format!("__sl_v{id}");
            let mut rmap = FxHashMap::default();
            rmap.insert(param.to_string(), p_tmp.clone());
            let pred_src = rewrite_arrow_expr_body(pred_expr, src, &rmap);
            let fused = if outer_method == "some" {
                format!(
                    "(() => {{ const __sl_a{id} = {arr}; for (let __sl_i{id} = 0; __sl_i{id} < __sl_a{id}.length; __sl_i{id}++) {{ let {param} = __sl_a{id}[__sl_i{id}]; if ({pred}) return true; }} return false; }})()",
                    id = id,
                    arr = arr_src,
                    param = p_tmp,
                    pred = pred_src
                )
            } else {
                format!(
                    "(() => {{ const __sl_a{id} = {arr}; for (let __sl_i{id} = 0; __sl_i{id} < __sl_a{id}.length; __sl_i{id}++) {{ let {param} = __sl_a{id}[__sl_i{id}]; if (!({pred})) return false; }} return true; }})()",
                    id = id,
                    arr = arr_src,
                    param = p_tmp,
                    pred = pred_src
                )
            };
            return Some(Replacement {
                start: call.span.start as usize,
                end: call.span.end as usize,
                text: fused,
            });
        }

        // Case 2: `<inner>.reduce((acc, curr) => <red_body>, <init>)`
        if outer_method == "reduce" && call.arguments.len() == 2 {
            let (acc_param, curr_param, red_expr) =
                extract_two_param_arrow(&call.arguments[0], src)?;
            let init_expr = call.arguments[1].as_expression()?;
            if expr_contains_fusion_hazard(init_expr, src) {
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
                        let (map_param, map_expr) =
                            extract_single_param_arrow(&inner_call.arguments[0], src)?;

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
                                    if let Some((filt_param, filt_expr)) =
                                        extract_single_param_arrow(&filt_call.arguments[0], src)
                                    {
                                        if expr_contains_fusion_hazard(&filt_mem.object, src) {
                                            return None;
                                        }
                                        let base_sp = filt_mem.object.span();
                                        let base_src =
                                            &src[base_sp.start as usize..base_sp.end as usize];
                                        *counter += 1;
                                        let id = *counter;
                                        let acc_tmp = format!("__sl_acc{id}");
                                        let fp_tmp = format!("__sl_f{id}");
                                        let mp_tmp = format!("__sl_m{id}");
                                        let cp_tmp = format!("__sl_c{id}");

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
                                            "(() => {{ const __sl_a{id} = {base}; let {acc} = {init}; for (let __sl_i{id} = 0; __sl_i{id} < __sl_a{id}.length; __sl_i{id}++) {{ let {fp} = __sl_a{id}[__sl_i{id}]; if (!({fb})) continue; let {mp} = {fp}; let {cp} = ({mb}); {acc} = ({rb}); }} return {acc}; }})()",
                                            id = id,
                                            base = base_src,
                                            acc = acc_tmp,
                                            init = init_src,
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

                        if expr_contains_fusion_hazard(&inner_mem.object, src) {
                            return None;
                        }
                        let arr_sp = inner_mem.object.span();
                        let arr_src = &src[arr_sp.start as usize..arr_sp.end as usize];
                        *counter += 1;
                        let id = *counter;
                        let acc_tmp = format!("__sl_acc{id}");
                        let mp_tmp = format!("__sl_m{id}");
                        let cp_tmp = format!("__sl_c{id}");

                        let mut mmap = FxHashMap::default();
                        mmap.insert(map_param.to_string(), mp_tmp.clone());
                        let map_body = rewrite_arrow_expr_body(map_expr, src, &mmap);

                        let mut rmap = FxHashMap::default();
                        rmap.insert(acc_param.to_string(), acc_tmp.clone());
                        rmap.insert(curr_param.to_string(), cp_tmp.clone());
                        let red_body = rewrite_arrow_expr_body(red_expr, src, &rmap);

                        let fused = format!(
                            "(() => {{ const __sl_a{id} = {arr}; let {acc} = {init}; for (let __sl_i{id} = 0; __sl_i{id} < __sl_a{id}.length; __sl_i{id}++) {{ let {mp} = __sl_a{id}[__sl_i{id}]; let {cp} = ({mb}); {acc} = ({rb}); }} return {acc}; }})()",
                            id = id,
                            arr = arr_src,
                            acc = acc_tmp,
                            init = init_src,
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
        counter: &mut usize,
        out: &mut Vec<Replacement>,
    ) {
        match expr {
            Expression::CallExpression(call) => {
                if let Some(rep) = try_fuse_call(call, src, counter) {
                    out.push(rep);
                    return;
                }
                walk_expr(&call.callee, src, counter, out);
                for arg in &call.arguments {
                    if let Some(e) = arg.as_expression() {
                        walk_expr(e, src, counter, out);
                    }
                }
            }
            Expression::BinaryExpression(b) => {
                walk_expr(&b.left, src, counter, out);
                walk_expr(&b.right, src, counter, out);
            }
            Expression::LogicalExpression(l) => {
                walk_expr(&l.left, src, counter, out);
                walk_expr(&l.right, src, counter, out);
            }
            Expression::ConditionalExpression(c) => {
                walk_expr(&c.test, src, counter, out);
                walk_expr(&c.consequent, src, counter, out);
                walk_expr(&c.alternate, src, counter, out);
            }
            Expression::AssignmentExpression(a) => {
                walk_expr(&a.right, src, counter, out);
            }
            Expression::ParenthesizedExpression(p) => {
                walk_expr(&p.expression, src, counter, out);
            }
            Expression::FunctionExpression(fe) => {
                if let Some(body) = &fe.body {
                    for s in &body.statements {
                        walk_stmt(s, src, counter, out);
                    }
                }
            }
            Expression::ObjectExpression(obj) => {
                for prop in &obj.properties {
                    if let oxc::ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop {
                        walk_expr(&p.value, src, counter, out);
                    }
                }
            }
            Expression::ArrayExpression(arr) => {
                for el in &arr.elements {
                    if let Some(e) = el.as_expression() {
                        walk_expr(e, src, counter, out);
                    }
                }
            }
            _ => {}
        }
    }

    fn walk_stmt<'a>(
        stmt: &'a Statement<'a>,
        src: &'a str,
        counter: &mut usize,
        out: &mut Vec<Replacement>,
    ) {
        match stmt {
            Statement::VariableDeclaration(vd) => {
                for decl in &vd.declarations {
                    if let Some(init) = &decl.init {
                        walk_expr(init, src, counter, out);
                    }
                }
            }
            Statement::ExpressionStatement(es) => {
                walk_expr(&es.expression, src, counter, out);
            }
            Statement::ReturnStatement(rs) => {
                if let Some(arg) = &rs.argument {
                    walk_expr(arg, src, counter, out);
                }
            }
            Statement::BlockStatement(bs) => {
                for s in &bs.body {
                    walk_stmt(s, src, counter, out);
                }
            }
            Statement::IfStatement(ifs) => {
                walk_expr(&ifs.test, src, counter, out);
                walk_stmt(&ifs.consequent, src, counter, out);
                if let Some(alt) = &ifs.alternate {
                    walk_stmt(alt, src, counter, out);
                }
            }
            Statement::FunctionDeclaration(fd) => {
                if let Some(body) = &fd.body {
                    for s in &body.statements {
                        walk_stmt(s, src, counter, out);
                    }
                }
            }
            Statement::ClassDeclaration(cd) => {
                for el in &cd.body.body {
                    if let oxc::ast::ast::ClassElement::MethodDefinition(md) = el {
                        if let Some(body) = &md.value.body {
                            for s in &body.statements {
                                walk_stmt(s, src, counter, out);
                            }
                        }
                    }
                }
            }
            Statement::ExportDefaultDeclaration(edd) => {
                if let Some(expr) = edd.declaration.as_expression() {
                    walk_expr(expr, src, counter, out);
                }
            }
            _ => {}
        }
    }

    let mut replacements = Vec::new();
    let mut counter = 0usize;
    for stmt in &parser_ret.program.body {
        walk_stmt(stmt, code, &mut counter, &mut replacements);
    }

    if replacements.is_empty() {
        return code.to_string();
    }

    replacements.sort_by_key(|r| r.start);
    let mut result = String::with_capacity(code.len() + replacements.len() * 64);
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
