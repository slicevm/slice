//! Oxc AST Dead-Declaration & Dead-Export Tree-Shaker (`prune_and_minify_cjs_module_ast`),
//! Used-Exports Analyzer (`analyze_module_used_exports`), and Free Identifier Scanner.

use rustc_hash::{FxHashMap, FxHashSet};

#[derive(Debug, Clone)]
pub enum UsedExports {
    All,
    Unreferenced,
    Subset(FxHashSet<String>),
}

/// Cached per-module `require("<target>")` usage summary (`CAS-PERF-02`) so Pass 2 in
/// `bundle_entrypoint_with_deps` parses each module at most once instead of $O(M^2)$ times.
#[derive(Debug, Clone, Default)]
pub struct ConsumerTargetUsage {
    pub occurrences: usize,
    pub accounted: usize,
    pub forces_all: bool,
    pub used_props: FxHashSet<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ModuleRequireSummary {
    pub parse_failed: bool,
    pub targets: FxHashMap<String, ConsumerTargetUsage>,
}

#[inline]
pub fn is_js_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$'
}

#[inline]
pub fn is_js_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

#[inline]
pub fn is_js_keyword(s: &str) -> bool {
    matches!(
        s,
        "const"
            | "let"
            | "var"
            | "function"
            | "return"
            | "if"
            | "else"
            | "for"
            | "while"
            | "do"
            | "switch"
            | "case"
            | "default"
            | "break"
            | "continue"
            | "try"
            | "catch"
            | "finally"
            | "throw"
            | "new"
            | "class"
            | "extends"
            | "super"
            | "this"
            | "typeof"
            | "instanceof"
            | "in"
            | "void"
            | "delete"
            | "await"
            | "yield"
            | "true"
            | "false"
            | "null"
            | "undefined"
    )
}

/// Scans `snippet` for all referenced free identifier names, skipping comments, string literals,
/// static template literal segments (while scanning `${...}` interpolations), RegExp literals,
/// and `.prop` member access names.
pub fn extract_referenced_identifiers_with_offsets(
    snippet: &str,
    base_offset: u32,
) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    let bytes = snippet.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let mut prev_sig_byte = 0u8;
    let mut prev_was_regex_kw = false;
    let mut prev_was_cond_kw = false;
    let mut after_cond_close_paren = false;
    let mut paren_stack: Vec<bool> = Vec::new();
    let mut in_template = false;
    let mut brace_depth = 0usize;
    let mut tpl_brace_stack: Vec<usize> = Vec::new();

    while i < len {
        if in_template {
            let b = bytes[i];
            if b == b'\\' {
                i = (i + 2).min(len);
                continue;
            }
            if b == b'`' {
                in_template = false;
                prev_sig_byte = b'`';
                prev_was_regex_kw = false;
                prev_was_cond_kw = false;
                after_cond_close_paren = false;
                i += 1;
                continue;
            }
            if b == b'$' && i + 1 < len && bytes[i + 1] == b'{' {
                tpl_brace_stack.push(brace_depth);
                in_template = false;
                prev_sig_byte = b'{';
                prev_was_regex_kw = false;
                prev_was_cond_kw = false;
                after_cond_close_paren = false;
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }

        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Line comment
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // Block comment
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(len);
            continue;
        }

        // RegExp literal
        if b == b'/' {
            let can_start_regex = prev_was_regex_kw
                || after_cond_close_paren
                || prev_sig_byte == 0
                || matches!(
                    prev_sig_byte,
                    b'(' | b','
                        | b'='
                        | b':'
                        | b'['
                        | b'!'
                        | b'&'
                        | b'|'
                        | b'?'
                        | b'{'
                        | b'}'
                        | b';'
                        | b'+'
                        | b'-'
                        | b'*'
                        | b'%'
                        | b'^'
                        | b'~'
                        | b'<'
                        | b'>'
                );
            if can_start_regex {
                i += 1;
                let mut in_class = false;
                while i < len {
                    let rb = bytes[i];
                    if rb == b'\\' {
                        i = (i + 2).min(len);
                        continue;
                    }
                    if rb == b'[' {
                        in_class = true;
                    } else if rb == b']' {
                        in_class = false;
                    } else if rb == b'/' && !in_class {
                        i += 1;
                        while i < len && bytes[i].is_ascii_alphabetic() {
                            i += 1;
                        }
                        break;
                    } else if rb == b'\n' {
                        break;
                    }
                    i += 1;
                }
                prev_sig_byte = b'/';
                prev_was_regex_kw = false;
                prev_was_cond_kw = false;
                after_cond_close_paren = false;
                continue;
            }
        }

        // Quote literal
        if b == b'\'' || b == b'"' {
            let q = b;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(len);
                    continue;
                }
                if bytes[i] == q {
                    i += 1;
                    break;
                }
                i += 1;
            }
            prev_sig_byte = q;
            prev_was_regex_kw = false;
            prev_was_cond_kw = false;
            after_cond_close_paren = false;
            continue;
        }

        if b == b'`' {
            in_template = true;
            i += 1;
            continue;
        }

        if b == b'(' {
            paren_stack.push(prev_was_cond_kw);
            prev_sig_byte = b'(';
            prev_was_regex_kw = false;
            prev_was_cond_kw = false;
            after_cond_close_paren = false;
            i += 1;
            continue;
        }

        if b == b')' {
            after_cond_close_paren = paren_stack.pop().unwrap_or(false);
            prev_sig_byte = b')';
            prev_was_regex_kw = false;
            prev_was_cond_kw = false;
            i += 1;
            continue;
        }

        if b == b'{' {
            brace_depth += 1;
            prev_sig_byte = b'{';
            prev_was_regex_kw = false;
            prev_was_cond_kw = false;
            after_cond_close_paren = false;
            i += 1;
            continue;
        }

        if b == b'}' {
            if tpl_brace_stack.last() == Some(&brace_depth) {
                tpl_brace_stack.pop();
                in_template = true;
            } else {
                brace_depth = brace_depth.saturating_sub(1);
            }
            prev_sig_byte = b'}';
            prev_was_regex_kw = false;
            prev_was_cond_kw = false;
            after_cond_close_paren = false;
            i += 1;
            continue;
        }

        if is_js_ident_start(b) {
            let start = i;
            i += 1;
            while i < len && is_js_ident_continue(bytes[i]) {
                i += 1;
            }
            let ident = &snippet[start..i];
            let is_member_prop =
                prev_sig_byte == b'.' && !(start >= 3 && &bytes[start - 3..start] == b"...");
            prev_was_regex_kw = !is_member_prop
                && matches!(
                    ident,
                    "return"
                        | "typeof"
                        | "void"
                        | "delete"
                        | "in"
                        | "of"
                        | "instanceof"
                        | "case"
                        | "throw"
                        | "else"
                        | "do"
                        | "new"
                        | "yield"
                        | "await"
                );
            prev_was_cond_kw = !is_member_prop && matches!(ident, "if" | "while" | "for" | "with");
            after_cond_close_paren = false;
            if !is_member_prop && !is_js_keyword(ident) {
                out.push((ident.to_string(), base_offset + start as u32));
            }
            prev_sig_byte = bytes[i - 1];
            continue;
        }

        prev_sig_byte = b;
        prev_was_regex_kw = false;
        prev_was_cond_kw = false;
        after_cond_close_paren = false;
        i += 1;
    }

    out
}

pub fn extract_referenced_identifiers(snippet: &str) -> FxHashSet<String> {
    extract_referenced_identifiers_with_offsets(snippet, 0)
        .into_iter()
        .map(|(n, _)| n)
        .collect()
}

fn is_require_call_for_target(expr: &oxc::ast::ast::Expression<'_>, target_mod_id: &str) -> bool {
    is_any_require_call(expr).is_some_and(|id| id == target_mod_id)
}

pub(crate) fn is_any_require_call(expr: &oxc::ast::ast::Expression<'_>) -> Option<String> {
    use oxc::ast::ast::{Argument, Expression};
    let Expression::CallExpression(call) = expr.without_parentheses() else {
        return None;
    };
    let Expression::Identifier(callee) = call.callee.without_parentheses() else {
        return None;
    };
    if callee.name.as_str() != "require" || call.arguments.len() != 1 {
        return None;
    }
    let Some(Argument::StringLiteral(str_lit)) = call.arguments.first() else {
        return None;
    };
    Some(str_lit.value.to_string())
}

fn has_ident_outside_span(code: &str, var_name: &str, allowed_span: (usize, usize)) -> bool {
    let bytes = code.as_bytes();
    let vbytes = var_name.as_bytes();
    let vlen = vbytes.len();
    if vlen == 0 {
        return false;
    }
    let finder = memchr::memmem::Finder::new(vbytes);
    let mut pos = 0usize;
    while let Some(rel) = finder.find(&bytes[pos..]) {
        let idx = pos + rel;
        let end = idx + vlen;
        pos = end;
        if idx >= allowed_span.0 && end <= allowed_span.1 {
            continue;
        }
        let valid_start =
            idx == 0 || (!is_js_ident_continue(bytes[idx - 1]) && bytes[idx - 1] != b'.');
        let valid_end = end == bytes.len() || !is_js_ident_continue(bytes[end]);
        if valid_start && valid_end {
            return true;
        }
    }
    false
}

fn try_collect_esm_default_interop_props(
    stmts: &[oxc::ast::ast::Statement<'_>],
    stmt_idx: usize,
    imp_var: &str,
    imp_stmt_start: usize,
    consumer_code: &str,
) -> Option<FxHashSet<String>> {
    use oxc::ast::ast::{BindingPattern, Expression, Statement};

    let Statement::VariableDeclaration(def_vd) = stmts.get(stmt_idx + 1)? else {
        return None;
    };
    if def_vd.declarations.len() != 1 {
        return None;
    }
    let def_decl = &def_vd.declarations[0];
    let BindingPattern::BindingIdentifier(def_ident) = &def_decl.id else {
        return None;
    };
    let def_init = def_decl.init.as_ref()?;
    let def_init_text = &consumer_code[def_vd.span.start as usize..def_vd.span.end as usize];
    if !def_init_text.contains(imp_var) || expr_contains_assignment(def_init) {
        return None;
    }

    let mut group_end = def_vd.span.end as usize;
    let mut props = FxHashSet::default();

    if let Some(Statement::VariableDeclaration(destr_vd)) = stmts.get(stmt_idx + 2) {
        if destr_vd.declarations.len() == 1 {
            let destr_decl = &destr_vd.declarations[0];
            if let Some(Expression::Identifier(id_expr)) = &destr_decl.init {
                if id_expr.name.as_str() == imp_var {
                    group_end = destr_vd.span.end as usize;
                    let BindingPattern::ObjectPattern(obj_pat) = &destr_decl.id else {
                        return None;
                    };
                    if obj_pat.rest.is_some() {
                        return None;
                    }
                    for prop in &obj_pat.properties {
                        let key_name = prop.key.name()?;
                        props.insert(key_name.to_string());
                    }
                }
            }
        }
    }

    if has_ident_outside_span(consumer_code, imp_var, (imp_stmt_start, group_end)) {
        return None;
    }

    if !collect_member_accesses_on_var(
        consumer_code,
        def_ident.name.as_str(),
        (imp_stmt_start, group_end),
        &mut props,
    ) {
        return None;
    }

    props.insert("default".to_string());
    props.insert("__esModule".to_string());
    Some(props)
}

fn collect_member_accesses_on_var(
    code: &str,
    var_name: &str,
    skip_span: (usize, usize),
    used_out: &mut FxHashSet<String>,
) -> bool {
    let bytes = code.as_bytes();
    let len = bytes.len();
    let vbytes = var_name.as_bytes();
    let vlen = vbytes.len();
    if vlen == 0 {
        return false;
    }
    let mut tpl_brace_stack: Vec<usize> = Vec::new();
    let mut i = 0usize;
    while i < len {
        let b = bytes[i];
        if b == b'/' && i + 1 < len {
            if bytes[i + 1] == b'/' {
                i += 2;
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if bytes[i + 1] == b'*' {
                i += 2;
                while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(len);
                continue;
            }
        }
        if b == b'\'' || b == b'"' {
            let q = b;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(len);
                    continue;
                }
                if bytes[i] == q {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if b == b'`' || (b == b'}' && tpl_brace_stack.last() == Some(&0)) {
            if b == b'}' {
                tpl_brace_stack.pop();
            }
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(len);
                    continue;
                }
                if bytes[i] == b'`' {
                    i += 1;
                    break;
                }
                if bytes[i] == b'$' && i + 1 < len && bytes[i + 1] == b'{' {
                    tpl_brace_stack.push(0);
                    i += 2;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if b == b'{' {
            if let Some(depth) = tpl_brace_stack.last_mut() {
                *depth += 1;
            }
            i += 1;
            continue;
        }
        if b == b'}' {
            if let Some(depth) = tpl_brace_stack.last_mut() {
                *depth = depth.saturating_sub(1);
            }
            i += 1;
            continue;
        }
        if is_js_ident_start(b) {
            let start = i;
            i += 1;
            while i < len && is_js_ident_continue(bytes[i]) {
                i += 1;
            }
            let end = i;
            if end - start == vlen && &bytes[start..end] == vbytes {
                if start >= skip_span.0 && end <= skip_span.1 {
                    continue;
                }
                if start > 0 && bytes[start - 1] == b'.' {
                    continue;
                }
                let after = code[end..].trim_start();
                let Some(after_dot) = after.strip_prefix('.') else {
                    return false;
                };
                let prop_bytes = after_dot.as_bytes();
                let prop_len = prop_bytes
                    .iter()
                    .take_while(|&&pb| is_js_ident_continue(pb))
                    .count();
                if prop_len == 0 {
                    return false;
                }
                used_out.insert(after_dot[..prop_len].to_string());
            }
            continue;
        }
        i += 1;
    }
    true
}

/// Extracts a single-pass `ModuleRequireSummary` for `consumer_code` (`CAS-PERF-02`) by parsing
/// `consumer_code` once with Oxc and indexing all `require("<target>")` bindings and inline
/// `require("<target>").Prop` member accesses.
pub fn extract_module_require_summary(consumer_code: &str) -> ModuleRequireSummary {
    use oxc::ast::ast::{Argument, BindingPattern, Expression, Statement};

    let mut summary = ModuleRequireSummary::default();
    if !consumer_code.contains("require(") {
        return summary;
    }

    let allocator = oxc::allocator::Allocator::default();
    let source_type = oxc::span::SourceType::mjs().with_module(false);
    let parser_ret = oxc::parser::Parser::new(&allocator, consumer_code, source_type).parse();
    if !parser_ret.diagnostics.is_empty() {
        summary.parse_failed = true;
        return summary;
    }

    // 1. Count all `require("<str>")` calls across the entire AST
    crate::linker::walk_ast_call_expressions(&parser_ret.program, &mut |call| {
        if let Expression::Identifier(callee) = call.callee.without_parentheses() {
            if callee.name.as_str() == "require" && call.arguments.len() == 1 {
                if let Some(Argument::StringLiteral(str_lit)) = call.arguments.first() {
                    let target = str_lit.value.as_str();
                    summary
                        .targets
                        .entry(target.to_string())
                        .or_default()
                        .occurrences += 1;
                }
            }
        }
    });

    if summary.targets.is_empty() {
        return summary;
    }

    // 2. Inspect top-level `const { ... } = require("<target>")` and `const x = require("<target>")`
    for (stmt_idx, stmt) in parser_ret.program.body.iter().enumerate() {
        let Statement::VariableDeclaration(vd) = stmt else {
            continue;
        };
        for decl in &vd.declarations {
            let Some(init) = &decl.init else {
                continue;
            };
            let Some(target_id) = is_any_require_call(init) else {
                continue;
            };
            let entry = summary.targets.entry(target_id).or_default();
            match &decl.id {
                BindingPattern::ObjectPattern(obj_pat) => {
                    if obj_pat.rest.is_some() {
                        entry.forces_all = true;
                    } else {
                        for prop in &obj_pat.properties {
                            if let Some(key_name) = prop.key.name() {
                                entry.used_props.insert(key_name.to_string());
                            } else {
                                entry.forces_all = true;
                            }
                        }
                        entry.accounted += 1;
                    }
                }
                BindingPattern::BindingIdentifier(ident) => {
                    let var_name = ident.name.as_str();
                    if var_name.starts_with("__imp_") {
                        if vd.declarations.len() == 1 {
                            if let Some(props) = try_collect_esm_default_interop_props(
                                &parser_ret.program.body,
                                stmt_idx,
                                var_name,
                                vd.span.start as usize,
                                consumer_code,
                            ) {
                                entry.used_props.extend(props);
                                entry.accounted += 1;
                            } else {
                                entry.forces_all = true;
                            }
                        } else {
                            entry.forces_all = true;
                        }
                    } else if collect_member_accesses_on_var(
                        consumer_code,
                        var_name,
                        (decl.span.start as usize, decl.span.end as usize),
                        &mut entry.used_props,
                    ) {
                        entry.accounted += 1;
                    } else {
                        entry.forces_all = true;
                    }
                }
                _ => {
                    entry.forces_all = true;
                }
            }
        }
    }

    // 3. Also check direct `require("<target>").Prop` inline member accesses
    crate::linker::walk_ast_expressions(&parser_ret.program, &mut |expr| {
        if let Expression::StaticMemberExpression(mem) = expr {
            if let Some(target_id) = is_any_require_call(&mem.object) {
                if let Some(entry) = summary.targets.get_mut(&target_id) {
                    entry
                        .used_props
                        .insert(mem.property.name.as_str().to_string());
                    entry.accounted += 1;
                }
            }
        }
    });

    summary
}

/// Evaluates `UsedExports` for `target_mod_id` from precomputed `ModuleRequireSummary` entries
/// in $O(M)$ time with zero redundant Oxc AST parses (`CAS-PERF-02`).
pub fn analyze_module_used_exports_from_summaries(
    target_mod_id: &str,
    summaries: &[ModuleRequireSummary],
    skip_idx: usize,
) -> UsedExports {
    let mut total_occurrences = 0usize;
    let mut used_set = FxHashSet::default();

    for (idx, summary) in summaries.iter().enumerate() {
        if idx == skip_idx {
            continue;
        }
        if summary.parse_failed {
            return UsedExports::All;
        }
        let Some(usage) = summary.targets.get(target_mod_id) else {
            continue;
        };
        if usage.occurrences == 0 {
            continue;
        }
        total_occurrences += usage.occurrences;
        if usage.forces_all || usage.accounted != usage.occurrences {
            return UsedExports::All;
        }
        used_set.extend(usage.used_props.iter().cloned());
    }

    if total_occurrences == 0 {
        UsedExports::Unreferenced
    } else {
        UsedExports::Subset(used_set)
    }
}

/// Inspects all other modules in the bundle to determine whether `target_mod_id` is:
/// - `UsedExports::Unreferenced` (0 `require(target_mod_id)` calls remain in the bundle),
/// - `UsedExports::Subset(names)` (every consumer accesses a statically known set of named exports), or
/// - `UsedExports::All` (consumed via default import, namespace value, or dynamic access).
pub fn analyze_module_used_exports(target_mod_id: &str, other_modules: &[&str]) -> UsedExports {
    use oxc::ast::ast::{BindingPattern, Statement};

    let mut total_occurrences = 0usize;
    let mut used_set = FxHashSet::default();

    for &consumer_code in other_modules {
        if !consumer_code.contains(target_mod_id) {
            continue;
        }
        let occurrences_in_consumer = consumer_code.matches(target_mod_id).count();
        total_occurrences += occurrences_in_consumer;

        let allocator = oxc::allocator::Allocator::default();
        let source_type = oxc::span::SourceType::mjs().with_module(false);
        let parser_ret = oxc::parser::Parser::new(&allocator, consumer_code, source_type).parse();
        if !parser_ret.diagnostics.is_empty() {
            return UsedExports::All;
        }

        let mut accounted = 0usize;
        for (stmt_idx, stmt) in parser_ret.program.body.iter().enumerate() {
            let Statement::VariableDeclaration(vd) = stmt else {
                continue;
            };
            for decl in &vd.declarations {
                let Some(init) = &decl.init else {
                    continue;
                };
                if !is_require_call_for_target(init, target_mod_id) {
                    continue;
                }
                match &decl.id {
                    BindingPattern::ObjectPattern(obj_pat) => {
                        if obj_pat.rest.is_some() {
                            return UsedExports::All;
                        }
                        for prop in &obj_pat.properties {
                            let Some(key_name) = prop.key.name() else {
                                return UsedExports::All;
                            };
                            used_set.insert(key_name.to_string());
                        }
                        accounted += 1;
                    }
                    BindingPattern::BindingIdentifier(ident) => {
                        let var_name = ident.name.as_str();
                        if var_name.starts_with("__imp_") {
                            if vd.declarations.len() != 1 {
                                return UsedExports::All;
                            }
                            let Some(props) = try_collect_esm_default_interop_props(
                                &parser_ret.program.body,
                                stmt_idx,
                                var_name,
                                vd.span.start as usize,
                                consumer_code,
                            ) else {
                                return UsedExports::All;
                            };
                            used_set.extend(props);
                            accounted += 1;
                        } else {
                            if !collect_member_accesses_on_var(
                                consumer_code,
                                var_name,
                                (decl.span.start as usize, decl.span.end as usize),
                                &mut used_set,
                            ) {
                                return UsedExports::All;
                            }
                            accounted += 1;
                        }
                    }
                    _ => return UsedExports::All,
                }
            }
        }

        for pat in [
            format!("require({:?}).", target_mod_id),
            format!("require('{}').", target_mod_id),
        ] {
            let mut search_pos = 0usize;
            while let Some(rel) = consumer_code[search_pos..].find(&pat) {
                let after_dot = &consumer_code[search_pos + rel + pat.len()..];
                let prop_len = after_dot
                    .as_bytes()
                    .iter()
                    .take_while(|&&b| is_js_ident_continue(b))
                    .count();
                if prop_len > 0 {
                    used_set.insert(after_dot[..prop_len].to_string());
                    accounted += 1;
                }
                search_pos += rel + pat.len();
            }
        }

        if accounted != occurrences_in_consumer {
            return UsedExports::All;
        }
    }

    if total_occurrences == 0 {
        UsedExports::Unreferenced
    } else {
        UsedExports::Subset(used_set)
    }
}

fn pattern_bound_names(pat: &oxc::ast::ast::BindingPattern<'_>, out: &mut Vec<String>) {
    use oxc::ast::ast::BindingPattern;
    match pat {
        BindingPattern::BindingIdentifier(id) => out.push(id.name.to_string()),
        BindingPattern::ObjectPattern(obj) => {
            for p in &obj.properties {
                pattern_bound_names(&p.value, out);
            }
            if let Some(r) = &obj.rest {
                pattern_bound_names(&r.argument, out);
            }
        }
        BindingPattern::ArrayPattern(arr) => {
            for el in arr.elements.iter().flatten() {
                pattern_bound_names(el, out);
            }
            if let Some(r) = &arr.rest {
                pattern_bound_names(&r.argument, out);
            }
        }
        BindingPattern::AssignmentPattern(assign) => {
            pattern_bound_names(&assign.left, out);
        }
    }
}

fn pattern_has_live(pat: &oxc::ast::ast::BindingPattern<'_>, live: &FxHashSet<String>) -> bool {
    let mut names = Vec::new();
    pattern_bound_names(pat, &mut names);
    names.iter().any(|n| live.contains(n))
}

fn expr_contains_assignment(expr: &oxc::ast::ast::Expression<'_>) -> bool {
    use oxc::ast::ast::{
        Argument, ArrayExpressionElement, ChainElement, Expression, ObjectPropertyKind,
    };
    match expr {
        Expression::AssignmentExpression(_) | Expression::UpdateExpression(_) => true,
        Expression::ParenthesizedExpression(p) => expr_contains_assignment(&p.expression),
        Expression::SequenceExpression(seq) => seq.expressions.iter().any(expr_contains_assignment),
        Expression::BinaryExpression(b) => {
            expr_contains_assignment(&b.left) || expr_contains_assignment(&b.right)
        }
        Expression::LogicalExpression(l) => {
            expr_contains_assignment(&l.left) || expr_contains_assignment(&l.right)
        }
        Expression::UnaryExpression(u) => expr_contains_assignment(&u.argument),
        Expression::ConditionalExpression(c) => {
            expr_contains_assignment(&c.test)
                || expr_contains_assignment(&c.consequent)
                || expr_contains_assignment(&c.alternate)
        }
        Expression::CallExpression(c) => {
            expr_contains_assignment(&c.callee)
                || c.arguments.iter().any(|a| match a {
                    Argument::SpreadElement(sp) => expr_contains_assignment(&sp.argument),
                    other => other.as_expression().is_some_and(expr_contains_assignment),
                })
        }
        Expression::NewExpression(n) => {
            expr_contains_assignment(&n.callee)
                || n.arguments.iter().any(|a| match a {
                    Argument::SpreadElement(sp) => expr_contains_assignment(&sp.argument),
                    other => other.as_expression().is_some_and(expr_contains_assignment),
                })
        }
        Expression::StaticMemberExpression(m) => expr_contains_assignment(&m.object),
        Expression::ComputedMemberExpression(m) => {
            expr_contains_assignment(&m.object) || expr_contains_assignment(&m.expression)
        }
        Expression::ObjectExpression(obj) => obj.properties.iter().any(|p| match p {
            ObjectPropertyKind::ObjectProperty(op) => {
                (op.computed && op.key.as_expression().is_some_and(expr_contains_assignment))
                    || expr_contains_assignment(&op.value)
            }
            ObjectPropertyKind::SpreadProperty(sp) => expr_contains_assignment(&sp.argument),
        }),
        Expression::ArrayExpression(arr) => arr.elements.iter().any(|el| match el {
            ArrayExpressionElement::SpreadElement(sp) => expr_contains_assignment(&sp.argument),
            other => other.as_expression().is_some_and(expr_contains_assignment),
        }),
        Expression::ChainExpression(ch) => match &ch.expression {
            ChainElement::CallExpression(c) => {
                expr_contains_assignment(&c.callee)
                    || c.arguments
                        .iter()
                        .any(|a| a.as_expression().is_some_and(expr_contains_assignment))
            }
            ChainElement::StaticMemberExpression(m) => expr_contains_assignment(&m.object),
            ChainElement::ComputedMemberExpression(m) => {
                expr_contains_assignment(&m.object) || expr_contains_assignment(&m.expression)
            }
            _ => false,
        },
        _ => false,
    }
}

fn match_exports_assignment_stmt(
    stmt: &oxc::ast::ast::Statement<'_>,
    src: &str,
) -> Option<(String, FxHashSet<String>)> {
    use oxc::ast::ast::Statement;
    use oxc::span::GetSpan;
    let Statement::ExpressionStatement(_) = stmt else {
        return None;
    };
    let sp = stmt.span();
    let text = src[sp.start as usize..sp.end as usize].trim();
    let after_exp = text.strip_prefix("exports.")?;
    let eq_idx = after_exp.find('=')?;
    if after_exp[eq_idx + 1..].starts_with('=') {
        return None;
    }
    let exp_name = after_exp[..eq_idx].trim();
    if exp_name.is_empty()
        || !exp_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
    {
        return None;
    }
    let rhs = after_exp[eq_idx + 1..].trim().trim_end_matches(';').trim();
    if rhs.contains("exports.") {
        return None;
    }
    Some((exp_name.to_string(), extract_referenced_identifiers(rhs)))
}

/// General-Purpose Oxc AST Dead-Declaration & Dead-Export Tree-Shaker (`prune_and_minify_cjs_module_ast`):
/// - Computes the transitive closure of top-level declarations (`function`, `class`, and per-`VariableDeclarator`
///   `const`/`let`/`var` bindings) reachable from `used_exports` and top-level side effects.
/// - Prunes unreferenced `exports.<Unused> = ...` assignments, unreferenced top-level functions/classes/variables,
///   and unused destructured properties in `const { a, b } = require("...")`.
/// - Emits clean, comment-free (and optionally minified) JavaScript via `oxc::codegen::Codegen`.
pub fn prune_and_minify_cjs_module_ast(
    cjs_code: &str,
    used_exports: Option<&FxHashSet<String>>,
    minify: bool,
) -> String {
    use oxc::ast::ast::{BindingPattern, Statement};
    use oxc::span::GetSpan;

    let allocator = oxc::allocator::Allocator::default();
    let source_type = oxc::span::SourceType::mjs().with_module(false);
    let parser_ret = oxc::parser::Parser::new(&allocator, cjs_code, source_type).parse();
    if !parser_ret.diagnostics.is_empty() {
        return cjs_code.to_string();
    }

    let mut program = parser_ret.program;

    if let Some(used_set) = used_exports {
        let has_named_export_stmts = program.body.iter().any(|s| {
            match_exports_assignment_stmt(s, cjs_code).is_some_and(|(name, _)| name != "__esModule")
        });

        let mut pruned_module_exports_obj_span: Option<(u32, Vec<String>)> = None;
        if !has_named_export_stmts && cjs_code.matches("exports").count() == 1 {
            for stmt in program.body.iter_mut() {
                let Statement::ExpressionStatement(es) = stmt else {
                    continue;
                };
                let oxc::ast::ast::Expression::AssignmentExpression(assign) =
                    es.expression.get_inner_expression_mut()
                else {
                    continue;
                };
                if assign.operator != oxc::ast::ast::AssignmentOperator::Assign {
                    continue;
                }
                let lhs_sp = assign.left.span();
                if cjs_code[lhs_sp.start as usize..lhs_sp.end as usize].trim() != "module.exports" {
                    continue;
                }
                let oxc::ast::ast::Expression::ObjectExpression(obj) = &mut assign.right else {
                    continue;
                };
                let obj_text = &cjs_code[obj.span.start as usize..obj.span.end as usize];
                if obj_text.contains("this.") {
                    break;
                }
                let all_static_named = obj.properties.iter().all(|p| match p {
                    oxc::ast::ast::ObjectPropertyKind::ObjectProperty(op) => {
                        !op.computed && op.key.name().is_some()
                    }
                    oxc::ast::ast::ObjectPropertyKind::SpreadProperty(_) => false,
                });
                if !all_static_named {
                    break;
                }
                let before_len = obj.properties.len();
                obj.properties.retain(|p| {
                    let oxc::ast::ast::ObjectPropertyKind::ObjectProperty(op) = p else {
                        return true;
                    };
                    let Some(k_name) = op.key.name() else {
                        return true;
                    };
                    if used_set.contains(k_name.as_ref()) {
                        return true;
                    }
                    let is_pure_droppable = op.kind == oxc::ast::ast::PropertyKind::Get
                        || matches!(
                            op.value.without_parentheses(),
                            oxc::ast::ast::Expression::Identifier(_)
                                | oxc::ast::ast::Expression::FunctionExpression(_)
                                | oxc::ast::ast::Expression::ArrowFunctionExpression(_)
                                | oxc::ast::ast::Expression::ClassExpression(_)
                                | oxc::ast::ast::Expression::NumericLiteral(_)
                                | oxc::ast::ast::Expression::StringLiteral(_)
                                | oxc::ast::ast::Expression::BooleanLiteral(_)
                                | oxc::ast::ast::Expression::NullLiteral(_)
                        );
                    !is_pure_droppable
                });
                if obj.properties.len() != before_len {
                    let mut kept_refs = Vec::new();
                    for p in &obj.properties {
                        let p_sp = p.span();
                        let p_txt = &cjs_code[p_sp.start as usize..p_sp.end as usize];
                        kept_refs.extend(extract_referenced_identifiers(p_txt));
                    }
                    pruned_module_exports_obj_span = Some((es.span.start, kept_refs));
                }
                break;
            }
        }

        if has_named_export_stmts || pruned_module_exports_obj_span.is_some() {
            // Step 1: Remove unused `exports.<Unused> = ...` and unused `module.exports = __slice_default`
            if has_named_export_stmts {
                program.body.retain(|stmt| {
                    if let Some((exp_name, _)) = match_exports_assignment_stmt(stmt, cjs_code) {
                        exp_name == "__esModule" || used_set.contains(&exp_name)
                    } else {
                        let sp = stmt.span();
                        let text = cjs_code[sp.start as usize..sp.end as usize].trim();
                        !text.starts_with("module.exports = __slice_default;")
                            || used_set.contains("default")
                    }
                });
            }

            // Step 2: Iterative Oxc Semantic Scope-Resolved Zero-Reference Pruning.
            for _ in 0..8 {
                let scoping = oxc::semantic::SemanticBuilder::new()
                    .build(&program)
                    .semantic
                    .into_scoping();
                let root_scope = scoping.root_scope_id();
                let mut referenced_root_syms: FxHashSet<String> = FxHashSet::default();
                for sym_id in scoping.symbol_ids() {
                    if scoping.symbol_scope_id(sym_id) == root_scope
                        && !scoping.get_resolved_reference_ids(sym_id).is_empty()
                    {
                        referenced_root_syms.insert(scoping.symbol_name(sym_id).to_string());
                    }
                }

                let mut changed = false;
                for stmt in program.body.iter_mut() {
                    if let Statement::VariableDeclaration(vd) = stmt {
                        for decl in vd.declarations.iter_mut() {
                            let is_req = decl
                                .init
                                .as_ref()
                                .is_some_and(|e| is_any_require_call(e).is_some());
                            if is_req {
                                if let BindingPattern::ObjectPattern(obj_pat) = &mut decl.id {
                                    if obj_pat.rest.is_none() {
                                        let before_len = obj_pat.properties.len();
                                        obj_pat.properties.retain(|prop| {
                                            pattern_has_live(&prop.value, &referenced_root_syms)
                                        });
                                        if obj_pat.properties.len() != before_len {
                                            changed = true;
                                        }
                                    }
                                }
                            }
                        }
                        let before_decls = vd.declarations.len();
                        vd.declarations.retain(|decl| {
                            pattern_has_live(&decl.id, &referenced_root_syms)
                                || decl.init.as_ref().is_some_and(expr_contains_assignment)
                        });
                        if vd.declarations.len() != before_decls {
                            changed = true;
                        }
                    }
                }

                let before_stmts = program.body.len();
                program.body.retain(|stmt| match stmt {
                    Statement::FunctionDeclaration(fd) => fd
                        .id
                        .as_ref()
                        .is_none_or(|id| referenced_root_syms.contains(id.name.as_str())),
                    Statement::ClassDeclaration(cd) => cd
                        .id
                        .as_ref()
                        .is_none_or(|id| referenced_root_syms.contains(id.name.as_str())),
                    Statement::VariableDeclaration(vd) => !vd.declarations.is_empty(),
                    _ => true,
                });
                if program.body.len() != before_stmts {
                    changed = true;
                }
                if !changed {
                    break;
                }
            }

            // Step 3: Transitive reachability closure to prune self-recursive dead functions
            let mut decl_dep_map: FxHashMap<String, Vec<String>> = FxHashMap::default();
            for stmt in &program.body {
                match stmt {
                    Statement::FunctionDeclaration(fd) => {
                        if let Some(id) = &fd.id {
                            let name = id.name.to_string();
                            let refs = extract_referenced_identifiers(
                                &cjs_code[fd.span.start as usize..fd.span.end as usize],
                            );
                            decl_dep_map.entry(name).or_default().extend(refs);
                        }
                    }
                    Statement::ClassDeclaration(cd) => {
                        if let Some(id) = &cd.id {
                            let name = id.name.to_string();
                            let refs = extract_referenced_identifiers(
                                &cjs_code[cd.span.start as usize..cd.span.end as usize],
                            );
                            decl_dep_map.entry(name).or_default().extend(refs);
                        }
                    }
                    Statement::VariableDeclaration(vd) => {
                        for decl in &vd.declarations {
                            let mut bound = Vec::new();
                            pattern_bound_names(&decl.id, &mut bound);
                            let is_destructured_req = matches!(&decl.id, BindingPattern::ObjectPattern(obj) if obj.rest.is_none())
                                && decl
                                    .init
                                    .as_ref()
                                    .is_some_and(|e| is_any_require_call(e).is_some());
                            let refs = if is_destructured_req {
                                FxHashSet::default()
                            } else {
                                extract_referenced_identifiers(
                                    &cjs_code[decl.span.start as usize..decl.span.end as usize],
                                )
                            };
                            for b_name in bound {
                                decl_dep_map
                                    .entry(b_name)
                                    .or_default()
                                    .extend(refs.iter().cloned());
                            }
                        }
                    }
                    _ => {}
                }
            }

            let mut worklist: Vec<String> = Vec::new();
            for stmt in &program.body {
                match stmt {
                    Statement::FunctionDeclaration(_) | Statement::ClassDeclaration(_) => {}
                    Statement::VariableDeclaration(vd) => {
                        for decl in &vd.declarations {
                            if decl.init.as_ref().is_some_and(expr_contains_assignment) {
                                let sp = decl.span();
                                let text = cjs_code[sp.start as usize..sp.end as usize].trim();
                                worklist.extend(extract_referenced_identifiers(text));
                            }
                        }
                    }
                    _ => {
                        let sp = stmt.span();
                        if let Some((obj_stmt_start, ref kept_refs)) =
                            pruned_module_exports_obj_span
                        {
                            if sp.start == obj_stmt_start {
                                worklist.extend(kept_refs.iter().cloned());
                                continue;
                            }
                        }
                        let text = cjs_code[sp.start as usize..sp.end as usize].trim();
                        worklist.extend(extract_referenced_identifiers(text));
                    }
                }
            }

            let mut live_idents: FxHashSet<String> = FxHashSet::default();
            while let Some(ident) = worklist.pop() {
                if live_idents.insert(ident.clone()) {
                    if let Some(deps) = decl_dep_map.get(&ident) {
                        for d in deps {
                            if !live_idents.contains(d) {
                                worklist.push(d.clone());
                            }
                        }
                    }
                }
            }

            for stmt in program.body.iter_mut() {
                if let Statement::VariableDeclaration(vd) = stmt {
                    for decl in vd.declarations.iter_mut() {
                        let is_req = decl
                            .init
                            .as_ref()
                            .is_some_and(|e| is_any_require_call(e).is_some());
                        if is_req {
                            if let BindingPattern::ObjectPattern(obj_pat) = &mut decl.id {
                                if obj_pat.rest.is_none() {
                                    obj_pat
                                        .properties
                                        .retain(|prop| pattern_has_live(&prop.value, &live_idents));
                                }
                            }
                        }
                    }
                    vd.declarations.retain(|decl| {
                        pattern_has_live(&decl.id, &live_idents)
                            || decl.init.as_ref().is_some_and(expr_contains_assignment)
                    });
                }
            }

            program.body.retain(|stmt| match stmt {
                Statement::FunctionDeclaration(fd) => fd
                    .id
                    .as_ref()
                    .is_none_or(|id| live_idents.contains(id.name.as_str())),
                Statement::ClassDeclaration(cd) => cd
                    .id
                    .as_ref()
                    .is_none_or(|id| live_idents.contains(id.name.as_str())),
                Statement::VariableDeclaration(vd) => !vd.declarations.is_empty(),
                _ => {
                    if let Some((exp_name, _)) = match_exports_assignment_stmt(stmt, cjs_code) {
                        exp_name == "__esModule" || used_set.contains(&exp_name)
                    } else {
                        let sp = stmt.span();
                        let text = cjs_code[sp.start as usize..sp.end as usize].trim();
                        !text.starts_with("module.exports = __slice_default;")
                            || used_set.contains("default")
                    }
                }
            });
        }
    }

    oxc::codegen::Codegen::new()
        .with_options(oxc::codegen::CodegenOptions {
            minify,
            comments: oxc::codegen::CommentOptions::disabled(),
            ..Default::default()
        })
        .build(&program)
        .code
}
