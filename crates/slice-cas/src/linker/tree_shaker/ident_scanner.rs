//! Lexer-Aware Free Identifier Scanner (`extract_referenced_identifiers_with_offsets`).

use rustc_hash::FxHashSet;

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
