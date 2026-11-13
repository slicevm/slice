//! Deterministic, zero-dependency JavaScript source minifier used at compile time (`build.rs`)
//! and runtime (`slice-engine`) to compact readable `.js` prelude and built-in module sources
//! into dense single-line byte slices without sacrificing source readability in the repository.

#[inline]
fn is_id_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

#[inline]
fn is_regex_preceding_keyword(word: &[u8]) -> bool {
    matches!(
        word,
        b"return"
            | b"throw"
            | b"case"
            | b"typeof"
            | b"void"
            | b"delete"
            | b"in"
            | b"of"
            | b"instanceof"
            | b"new"
            | b"yield"
            | b"await"
            | b"else"
            | b"do"
    )
}

#[inline]
fn is_regex_preceding_punct(b: u8) -> bool {
    matches!(
        b,
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
            | b'<'
            | b'>'
            | b'^'
            | b'~'
    )
}

/// Minifies readable JavaScript source code while preserving string literals (`'...'`, `"..."`),
/// template literals (`` `...` `` including nested `${...}` interpolations), regular expression
/// literals (`/pattern/flags`), required keyword/identifier spaces, and Automatic Semicolon
/// Insertion (ASI) boundaries.
pub fn minify_js_source(src: &str) -> String {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut out = Vec::with_capacity(len);
    let mut i = 0;

    let mut pending_space = false;
    let mut pending_newline = false;
    let mut last_word_start: Option<usize> = None;
    let mut in_template = false;
    let mut brace_depth = 0usize;
    let mut tpl_brace_stack: Vec<usize> = Vec::new();
    let mut paren_stack: Vec<bool> = Vec::new();
    let mut after_cond_close_paren = false;

    while i < len {
        // Inside static segment of a template literal (`` `...` `` or `}...`): copy verbatim!
        if in_template {
            let ch = bytes[i];
            out.push(ch);
            i += 1;
            if ch == b'\\' && i < len {
                out.push(bytes[i]);
                i += 1;
            } else if ch == b'`' {
                in_template = false;
                last_word_start = None;
                after_cond_close_paren = false;
            } else if ch == b'$' && i < len && bytes[i] == b'{' {
                out.push(b'{');
                i += 1;
                tpl_brace_stack.push(brace_depth);
                in_template = false;
                last_word_start = None;
                after_cond_close_paren = false;
            }
            continue;
        }

        let b = bytes[i];

        // 1. Whitespace handling
        if b == b' ' || b == b'\t' || b == b'\r' || b == b'\n' {
            pending_space = true;
            if b == b'\n' {
                pending_newline = true;
            }
            i += 1;
            continue;
        }

        // 2. Single-line comment `// ...`
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            pending_space = true;
            pending_newline = true;
            continue;
        }

        // 3. Multi-line block comment `/* ... */`
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                if bytes[i] == b'\n' {
                    pending_newline = true;
                }
                i += 1;
            }
            i = (i + 2).min(len);
            pending_space = true;
            continue;
        }

        // 4. Emit required separator if whitespace was skipped
        if pending_space {
            if let Some(&prev_b) = out.last() {
                let need_id_space = is_id_char(prev_b) && is_id_char(b);
                let need_op_space = (prev_b == b'+' && b == b'+')
                    || (prev_b == b'-' && b == b'-')
                    || (prev_b == b'/' && (b == b'/' || b == b'*'));
                let need_asi_newline = pending_newline
                    && (is_id_char(prev_b)
                        || prev_b == b')'
                        || prev_b == b']'
                        || prev_b == b'"'
                        || prev_b == b'\''
                        || prev_b == b'`')
                    && (is_id_char(b) || b == b'`');

                if need_asi_newline {
                    out.push(b'\n');
                } else if need_id_space || need_op_space {
                    out.push(b' ');
                }
            }
            pending_space = false;
            pending_newline = false;
        }

        // 5a. Quoted string literals (`'...'` or `"..."`)
        if b == b'\'' || b == b'"' {
            let quote = b;
            last_word_start = None;
            after_cond_close_paren = false;
            out.push(quote);
            i += 1;
            while i < len {
                let ch = bytes[i];
                out.push(ch);
                i += 1;
                if ch == b'\\' && i < len {
                    out.push(bytes[i]);
                    i += 1;
                } else if ch == quote {
                    break;
                }
            }
            continue;
        }

        // 5b. Template literal opener (`` ` ``)
        if b == b'`' {
            last_word_start = None;
            after_cond_close_paren = false;
            out.push(b'`');
            i += 1;
            in_template = true;
            continue;
        }

        // 6. Regular expression literal (`/pattern/flags`) vs division operator (`/`, `/=`)
        if b == b'/' {
            let prev_sig = out
                .iter()
                .rev()
                .copied()
                .find(|&c| c != b' ' && c != b'\n')
                .unwrap_or(0);
            let prev_word = match last_word_start {
                Some(ws) if ws <= out.len() && is_id_char(prev_sig) => {
                    let end = out[ws..]
                        .iter()
                        .position(|&c| !is_id_char(c))
                        .map(|p| ws + p)
                        .unwrap_or(out.len());
                    &out[ws..end]
                }
                _ => &[][..],
            };
            let is_regex = prev_sig == 0
                || after_cond_close_paren
                || is_regex_preceding_punct(prev_sig)
                || is_regex_preceding_keyword(prev_word);

            if is_regex {
                last_word_start = None;
                after_cond_close_paren = false;
                out.push(b'/');
                i += 1;
                let mut in_char_class = false;
                while i < len {
                    let rc = bytes[i];
                    out.push(rc);
                    i += 1;
                    if rc == b'\\' && i < len {
                        out.push(bytes[i]);
                        i += 1;
                    } else if rc == b'[' {
                        in_char_class = true;
                    } else if rc == b']' {
                        in_char_class = false;
                    } else if rc == b'/' && !in_char_class {
                        break;
                    }
                }
                while i < len && bytes[i].is_ascii_alphabetic() {
                    out.push(bytes[i]);
                    i += 1;
                }
                continue;
            }
        }

        // 7. Parentheses & template interpolation braces
        if b == b'(' {
            let prev_sig = out
                .iter()
                .rev()
                .copied()
                .find(|&c| c != b' ' && c != b'\n')
                .unwrap_or(0);
            let prev_word = match last_word_start {
                Some(ws) if ws <= out.len() && is_id_char(prev_sig) => {
                    let end = out[ws..]
                        .iter()
                        .position(|&c| !is_id_char(c))
                        .map(|p| ws + p)
                        .unwrap_or(out.len());
                    &out[ws..end]
                }
                _ => &[][..],
            };
            let is_cond = matches!(prev_word, b"if" | b"while" | b"for" | b"with");
            paren_stack.push(is_cond);
            last_word_start = None;
            after_cond_close_paren = false;
            out.push(b'(');
            i += 1;
            continue;
        }
        if b == b')' {
            after_cond_close_paren = paren_stack.pop().unwrap_or(false);
            last_word_start = None;
            out.push(b')');
            i += 1;
            continue;
        }
        if b == b'{' {
            brace_depth += 1;
            last_word_start = None;
            after_cond_close_paren = false;
            out.push(b'{');
            i += 1;
            continue;
        }
        if b == b'}' {
            last_word_start = None;
            after_cond_close_paren = false;
            out.push(b'}');
            i += 1;
            if tpl_brace_stack.last() == Some(&brace_depth) {
                tpl_brace_stack.pop();
                in_template = true;
            } else {
                brace_depth = brace_depth.saturating_sub(1);
            }
            continue;
        }

        // 8. Identifiers / numbers / punctuation
        if is_id_char(b) {
            let prev_b = out.last().copied().unwrap_or(0);
            let prev_is_id = is_id_char(prev_b);
            if !prev_is_id {
                let prev_sig = out
                    .iter()
                    .rev()
                    .copied()
                    .find(|&c| c != b' ' && c != b'\n')
                    .unwrap_or(0);
                // Do not record member properties (`obj.return`) as standalone keywords
                let is_dot_prop =
                    prev_sig == b'.' && !(out.len() >= 3 && &out[out.len() - 3..] == b"...");
                last_word_start = if is_dot_prop { None } else { Some(out.len()) };
            }
        } else {
            last_word_start = None;
        }
        after_cond_close_paren = false;

        out.push(b);
        i += 1;
    }

    // Input `src` is valid UTF-8 and we only stripped ASCII whitespace/comments.
    String::from_utf8(out).unwrap_or_else(|_| src.to_string())
}

#[cfg(test)]
mod tests {
    use super::minify_js_source;

    #[test]
    fn test_minify_nested_template_literals_and_regex_contexts() {
        let src = r#"
function parse(ok, obj, url) {
  const tpl = `head ${`nested // keep ${1 + 2}`} // tail`;
  if (ok) return /https?:\/\/host\/a/.test(url) ? tpl : "";
  return obj.return / 2;
}
"#;
        let minified = minify_js_source(src);
        assert!(minified.contains("`head ${`nested // keep ${1+2}`} // tail`"));
        assert!(minified.contains("if(ok)return/https?:\\/\\/host\\/a/.test(url)"));
        assert!(minified.contains("obj.return/2"));
    }
}
