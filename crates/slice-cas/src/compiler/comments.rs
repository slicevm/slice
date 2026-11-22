//! Lexical JavaScript/TypeScript Comment & Blank-Line Stripper (`strip_js_comments`).

use crate::linker::rewrite_buffer_fast_paths_ast;

/// Strips JSDoc/license block comments (`/* ... */`), `//` line comments, and blank lines
/// at `slice pack` compile time so `boa_engine` does not spend cold-start CPU cycles lexing comments,
/// while preserving string literals, RegExp literals, and multi-line / nested `${...}` template literals.
pub fn strip_js_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut raw_out = Vec::with_capacity(len);
    let mut i = 0usize;

    let mut in_template = false;
    let mut brace_depth = 0usize;
    let mut tpl_brace_stack: Vec<usize> = Vec::new();
    let mut paren_stack: Vec<bool> = Vec::new();
    let mut after_cond_close_paren = false;
    let mut prev_sig_byte = 0u8;
    let mut prev_was_regex_kw = false;
    let mut prev_was_cond_kw = false;

    let is_id_start = |b: u8| b.is_ascii_alphabetic() || b == b'_' || b == b'$';
    let is_id_cont = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';

    while i < len {
        if in_template {
            let b = bytes[i];
            raw_out.push(b);
            i += 1;
            if b == b'\\' && i < len {
                raw_out.push(bytes[i]);
                i += 1;
            } else if b == b'`' {
                in_template = false;
                prev_sig_byte = b'`';
                prev_was_regex_kw = false;
                prev_was_cond_kw = false;
                after_cond_close_paren = false;
            } else if b == b'$' && i < len && bytes[i] == b'{' {
                raw_out.push(b'{');
                i += 1;
                tpl_brace_stack.push(brace_depth);
                in_template = false;
                prev_sig_byte = b'{';
                prev_was_regex_kw = false;
                prev_was_cond_kw = false;
                after_cond_close_paren = false;
            }
            continue;
        }

        let b = bytes[i];

        if b == b' ' || b == b'\t' || b == b'\r' || b == b'\n' {
            if b != b'\r' {
                raw_out.push(b);
            }
            i += 1;
            continue;
        }

        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            let mut had_newline = false;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                if bytes[i] == b'\n' {
                    had_newline = true;
                }
                i += 1;
            }
            i = (i + 2).min(len);
            if had_newline {
                raw_out.push(b'\n');
            } else if raw_out.last().is_some_and(|&p| !p.is_ascii_whitespace())
                && bytes.get(i).is_some_and(|&n| !n.is_ascii_whitespace())
            {
                raw_out.push(b' ');
            }
            continue;
        }

        if b == b'\'' || b == b'"' {
            let q = b;
            raw_out.push(q);
            i += 1;
            while i < len {
                let c = bytes[i];
                raw_out.push(c);
                i += 1;
                if c == b'\\' && i < len {
                    raw_out.push(bytes[i]);
                    i += 1;
                } else if c == q {
                    break;
                }
            }
            prev_sig_byte = q;
            prev_was_regex_kw = false;
            prev_was_cond_kw = false;
            after_cond_close_paren = false;
            continue;
        }

        if b == b'`' {
            raw_out.push(b'`');
            i += 1;
            in_template = true;
            continue;
        }

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
                raw_out.push(b'/');
                i += 1;
                let mut in_class = false;
                while i < len {
                    let rc = bytes[i];
                    if rc == b'\n' {
                        break;
                    }
                    raw_out.push(rc);
                    i += 1;
                    if rc == b'\\' && i < len {
                        raw_out.push(bytes[i]);
                        i += 1;
                    } else if rc == b'[' {
                        in_class = true;
                    } else if rc == b']' {
                        in_class = false;
                    } else if rc == b'/' && !in_class {
                        while i < len && bytes[i].is_ascii_alphabetic() {
                            raw_out.push(bytes[i]);
                            i += 1;
                        }
                        break;
                    }
                }
                prev_sig_byte = b'/';
                prev_was_regex_kw = false;
                prev_was_cond_kw = false;
                after_cond_close_paren = false;
                continue;
            }
        }

        if b == b'(' {
            paren_stack.push(prev_was_cond_kw);
            raw_out.push(b'(');
            prev_sig_byte = b'(';
            prev_was_regex_kw = false;
            prev_was_cond_kw = false;
            after_cond_close_paren = false;
            i += 1;
            continue;
        }
        if b == b')' {
            after_cond_close_paren = paren_stack.pop().unwrap_or(false);
            raw_out.push(b')');
            prev_sig_byte = b')';
            prev_was_regex_kw = false;
            prev_was_cond_kw = false;
            i += 1;
            continue;
        }
        if b == b'{' {
            brace_depth += 1;
            raw_out.push(b'{');
            prev_sig_byte = b'{';
            prev_was_regex_kw = false;
            prev_was_cond_kw = false;
            after_cond_close_paren = false;
            i += 1;
            continue;
        }
        if b == b'}' {
            raw_out.push(b'}');
            prev_sig_byte = b'}';
            prev_was_regex_kw = false;
            prev_was_cond_kw = false;
            after_cond_close_paren = false;
            i += 1;
            if tpl_brace_stack.last() == Some(&brace_depth) {
                tpl_brace_stack.pop();
                in_template = true;
            } else {
                brace_depth = brace_depth.saturating_sub(1);
            }
            continue;
        }

        if is_id_start(b) {
            let start = i;
            i += 1;
            while i < len && is_id_cont(bytes[i]) {
                i += 1;
            }
            raw_out.extend_from_slice(&bytes[start..i]);
            let ident = &source[start..i];
            let is_dot_prop =
                prev_sig_byte == b'.' && !(start >= 3 && &bytes[start - 3..start] == b"...");
            prev_was_regex_kw = !is_dot_prop
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
            prev_was_cond_kw = !is_dot_prop && matches!(ident, "if" | "while" | "for" | "with");
            after_cond_close_paren = false;
            prev_sig_byte = bytes[i - 1];
            continue;
        }

        raw_out.push(b);
        prev_sig_byte = b;
        prev_was_regex_kw = false;
        prev_was_cond_kw = false;
        after_cond_close_paren = false;
        i += 1;
    }

    let stripped_str = String::from_utf8(raw_out).unwrap_or_else(|_| source.to_string());
    let mut out = String::with_capacity(stripped_str.len());
    let mut tpl_state = false;
    let mut tpl_braces: Vec<usize> = Vec::new();
    let mut b_depth = 0usize;
    for line in stripped_str.split('\n') {
        if !tpl_state && line.trim().is_empty() {
            continue;
        }
        out.push_str(line);
        out.push('\n');
        let lb = line.as_bytes();
        let mut li = 0usize;
        while li < lb.len() {
            if tpl_state {
                if lb[li] == b'\\' {
                    li = (li + 2).min(lb.len());
                    continue;
                }
                if lb[li] == b'`' {
                    tpl_state = false;
                } else if lb[li] == b'$' && li + 1 < lb.len() && lb[li + 1] == b'{' {
                    tpl_braces.push(b_depth);
                    tpl_state = false;
                    li += 2;
                    continue;
                }
                li += 1;
                continue;
            }
            let c = lb[li];
            if c == b'\'' || c == b'"' {
                let q = c;
                li += 1;
                while li < lb.len() {
                    if lb[li] == b'\\' {
                        li = (li + 2).min(lb.len());
                        continue;
                    }
                    if lb[li] == q {
                        li += 1;
                        break;
                    }
                    li += 1;
                }
                continue;
            }
            if c == b'`' {
                tpl_state = true;
            } else if c == b'{' {
                b_depth += 1;
            } else if c == b'}' {
                if tpl_braces.last() == Some(&b_depth) {
                    tpl_braces.pop();
                    tpl_state = true;
                } else {
                    b_depth = b_depth.saturating_sub(1);
                }
            }
            li += 1;
        }
    }

    if out.contains("Buffer.") {
        rewrite_buffer_fast_paths_ast(&out)
    } else {
        out
    }
}
