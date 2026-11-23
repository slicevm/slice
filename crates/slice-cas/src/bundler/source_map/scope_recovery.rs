//! Lexical Scope and Enclosing Class/Method/Function Name Recovery for Stack Frames.

use super::BundledSourceSection;

#[inline]
fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

fn take_first_ident(s: &str) -> Option<&str> {
    let trimmed = s.trim_start();
    let len = trimmed.bytes().take_while(|&b| is_ident_char(b)).count();
    if len > 0 && !trimmed.as_bytes()[0].is_ascii_digit() {
        Some(&trimmed[..len])
    } else {
        None
    }
}

fn take_trailing_lhs_name(lhs: &str) -> Option<&str> {
    let trimmed = lhs.trim();
    let end = trimmed.len();
    let start = trimmed
        .bytes()
        .rev()
        .take_while(|&b| is_ident_char(b) || b == b'.')
        .count();
    if start == 0 {
        return None;
    }
    let cand = trimmed[end - start..].trim_matches('.');
    if cand.is_empty() || cand.as_bytes()[0].is_ascii_digit() {
        return None;
    }
    if let Some(stripped) = cand.strip_prefix("exports.")
        && !stripped.is_empty()
    {
        return Some(stripped);
    }
    if let Some(stripped) = cand.strip_prefix("module.exports.")
        && !stripped.is_empty()
    {
        return Some(stripped);
    }
    if let Some((cls, method)) = cand.split_once(".prototype.")
        && !cls.is_empty()
        && !method.is_empty()
    {
        return Some(cand);
    }
    Some(cand)
}

#[derive(Debug, Clone)]
enum LexicalScope {
    Block,
    Class(String),
    Callable(String),
    AnonymousCallable,
}

fn classify_brace_header(header: &str, enclosing_class: Option<&str>) -> LexicalScope {
    let h = header.trim();
    if h.is_empty() {
        return LexicalScope::Block;
    }

    // 1. Class declaration or assigned class expression (`class Ledger`, `export class Ledger`, `const L = class`)
    if let Some(after_cls) = h
        .strip_prefix("class ")
        .or_else(|| h.strip_prefix("export class "))
        .or_else(|| h.strip_prefix("export default class "))
    {
        if let Some(cls_name) = take_first_ident(after_cls)
            && cls_name != "extends"
        {
            return LexicalScope::Class(cls_name.to_string());
        }
        return LexicalScope::Class("<anonymous>".to_string());
    }
    if let Some((lhs, rhs)) = h.rsplit_once('=') {
        let rhs_t = rhs.trim();
        if !lhs.ends_with('=')
            && !lhs.ends_with('!')
            && !lhs.ends_with('<')
            && !lhs.ends_with('>')
            && (rhs_t == "class" || rhs_t.starts_with("class "))
        {
            let explicit = rhs_t
                .strip_prefix("class")
                .and_then(take_first_ident)
                .filter(|&id| id != "extends");
            if let Some(c) = explicit.or_else(|| take_trailing_lhs_name(lhs)) {
                return LexicalScope::Class(c.to_string());
            }
        }
    }

    // 2. Control-flow blocks and object literals (`if (...)`, `try`, `catch (e)`, `const x =`, `foo(`)
    if let Some(kw) = take_first_ident(h)
        && matches!(
            kw,
            "if" | "else"
                | "for"
                | "while"
                | "do"
                | "switch"
                | "try"
                | "catch"
                | "finally"
                | "with"
                | "return"
                | "throw"
        )
    {
        return LexicalScope::Block;
    }
    if h.ends_with('=')
        || h.ends_with('(')
        || h.ends_with(',')
        || h.ends_with(':')
        || h.ends_with('[')
    {
        return LexicalScope::Block;
    }

    // 3. Arrow functions (`(...) =>`)
    if let Some(before_arrow) = h.strip_suffix("=>") {
        if let Some((lhs, _params)) = before_arrow.rsplit_once('=') {
            let lhs_trim = lhs.trim_end();
            if !lhs_trim.ends_with('=')
                && !lhs_trim.ends_with('!')
                && !lhs_trim.ends_with('<')
                && !lhs_trim.ends_with('>')
                && let Some(name) = take_trailing_lhs_name(lhs_trim)
            {
                return LexicalScope::Callable(name.replace(".prototype.", "."));
            }
        }
        return LexicalScope::AnonymousCallable;
    }

    // 4. `function` declarations and expressions
    if let Some(fn_pos) = h.rfind("function") {
        let before_ok = fn_pos == 0 || !is_ident_char(h.as_bytes()[fn_pos - 1]);
        let after_fn = &h[fn_pos + 8..];
        let after_ok = after_fn.is_empty()
            || after_fn.starts_with(' ')
            || after_fn.starts_with('*')
            || after_fn.starts_with('(');
        if before_ok && after_ok {
            let after_star = after_fn.trim_start().trim_start_matches('*').trim_start();
            if let Some(explicit_fn) = take_first_ident(after_star) {
                return LexicalScope::Callable(explicit_fn.to_string());
            }
            let prefix = h[..fn_pos].trim_end().trim_end_matches("async").trim_end();
            if let Some(lhs) = prefix.strip_suffix('=')
                && let Some(name) = take_trailing_lhs_name(lhs)
            {
                return LexicalScope::Callable(name.replace(".prototype.", "."));
            }
            return LexicalScope::AnonymousCallable;
        }
    }

    // 5. Concise ES6 class methods and object methods (`post(o)`, `async post(o)`, `static create()`, `get x()`)
    if h.ends_with(')') {
        let mut rest = h;
        loop {
            let next = rest
                .strip_prefix("public ")
                .or_else(|| rest.strip_prefix("private "))
                .or_else(|| rest.strip_prefix("protected "))
                .or_else(|| rest.strip_prefix("readonly "))
                .or_else(|| rest.strip_prefix("override "))
                .or_else(|| rest.strip_prefix("static "))
                .or_else(|| rest.strip_prefix("async "))
                .or_else(|| rest.strip_prefix("get "))
                .or_else(|| rest.strip_prefix("set "))
                .map(str::trim_start);
            match next {
                Some(n) => rest = n,
                None => break,
            }
        }
        rest = rest.trim_start_matches('*').trim_start();
        if let Some(method_name) = take_first_ident(rest) {
            let after_name = rest[method_name.len()..].trim_start();
            if after_name.starts_with('(') {
                if let Some(cls) = enclosing_class
                    && cls != "<anonymous>"
                {
                    if method_name == "constructor" {
                        return LexicalScope::Callable(format!("new {}", cls));
                    }
                    return LexicalScope::Callable(format!("{}.{}", cls, method_name));
                }
                return LexicalScope::Callable(method_name.to_string());
            }
        }
    }

    LexicalScope::Block
}

pub(super) fn recover_enclosing_symbol_name(
    lines: &[&str],
    sec: &BundledSourceSection<'_>,
    target_line_1: usize,
    target_col_1: usize,
) -> Option<String> {
    let end_l = target_line_1.min(sec.end_line_1).min(lines.len());
    if end_l <= sec.marker_line_1 {
        return None;
    }

    let mut scope_stack: Vec<LexicalScope> = Vec::with_capacity(8);
    let mut in_str: Option<u8> = None;

    for l_1 in (sec.marker_line_1 + 1)..=end_l {
        let line = lines[l_1 - 1];
        let bytes = line.as_bytes();
        let is_target_line = l_1 == target_line_1;
        let mut seg_start = 0usize;
        let mut i = 0usize;
        let mut opened_callable_on_target = false;

        while i < bytes.len() {
            if is_target_line && target_col_1 > 1 && i >= target_col_1 {
                break;
            }
            let b = bytes[i];
            if let Some(q) = in_str {
                if b == b'\\' {
                    i += 2;
                    continue;
                }
                if b == q {
                    in_str = None;
                }
                i += 1;
                continue;
            }
            if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                break;
            }
            if b == b'\'' || b == b'"' || b == b'`' {
                in_str = Some(b);
                i += 1;
                continue;
            }
            if b == b';' {
                seg_start = i + 1;
            } else if b == b'{' {
                let header = if seg_start <= i {
                    &line[seg_start..i]
                } else {
                    ""
                };
                let direct_class = match scope_stack.last() {
                    Some(LexicalScope::Class(cls)) => Some(cls.as_str()),
                    _ => None,
                };
                let scope = classify_brace_header(header, direct_class);
                if is_target_line
                    && matches!(
                        scope,
                        LexicalScope::Callable(_) | LexicalScope::AnonymousCallable
                    )
                {
                    opened_callable_on_target = true;
                }
                scope_stack.push(scope);
                seg_start = i + 1;
            } else if b == b'}' {
                if is_target_line && target_col_1 <= 1 && opened_callable_on_target {
                    break;
                }
                scope_stack.pop();
                seg_start = i + 1;
            }
            i += 1;
        }
    }

    for scope in scope_stack.iter().rev() {
        match scope {
            LexicalScope::Callable(name) => return Some(name.clone()),
            LexicalScope::AnonymousCallable => return None,
            LexicalScope::Block | LexicalScope::Class(_) => {}
        }
    }
    None
}
