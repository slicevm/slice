//! Portable In-Process POSIX Shell Builtin & Pipeline Evaluator (`echo`, `cat`, `tr`, `exit`, `env`, `printenv`, `true`, `false`).

use super::SubprocessSyncResult;
use std::collections::BTreeMap;

fn expand_tr_set(spec: &str) -> Vec<u8> {
    let bytes = spec.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if i + 2 < bytes.len() && bytes[i + 1] == b'-' && bytes[i] <= bytes[i + 2] {
            for b in bytes[i]..=bytes[i + 2] {
                out.push(b);
            }
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

fn split_shell_words(cmd: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in cmd.chars() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else {
                cur.push(ch);
            }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
        } else if ch.is_whitespace() {
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

fn format_env_map(child_env: &BTreeMap<String, String>) -> Vec<u8> {
    let mut out = String::new();
    for (k, v) in child_env {
        out.push_str(k);
        out.push('=');
        out.push_str(v);
        out.push('\n');
    }
    out.into_bytes()
}

pub(super) fn try_eval_portable_builtin_argv(
    base: &str,
    rest: &[String],
    child_env: &BTreeMap<String, String>,
    stdin_bytes: &[u8],
    expand_shell_vars: bool,
) -> Option<(i32, Vec<u8>, Vec<u8>)> {
    match base {
        "echo" => {
            let mut out = String::new();
            let (no_nl, slice) = if rest.first().map(String::as_str) == Some("-n") {
                (true, &rest[1..])
            } else {
                (false, rest)
            };
            for (i, a) in slice.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                if expand_shell_vars && let Some(var_name) = a.strip_prefix('$') {
                    let clean = var_name.trim_matches(|c: char| c == '{' || c == '}');
                    out.push_str(child_env.get(clean).map(String::as_str).unwrap_or(""));
                } else {
                    out.push_str(a);
                }
            }
            if !no_nl {
                out.push('\n');
            }
            Some((0, out.into_bytes(), Vec::new()))
        }
        "cat" if rest.is_empty() => Some((0, stdin_bytes.to_vec(), Vec::new())),
        "tr" if expand_shell_vars && rest.len() == 2 => {
            let set1 = expand_tr_set(&rest[0]);
            let set2 = expand_tr_set(&rest[1]);
            let mut out = stdin_bytes.to_vec();
            if !set1.is_empty() && !set2.is_empty() {
                for b in &mut out {
                    if let Some(pos) = set1.iter().position(|&x| x == *b) {
                        let mapped_idx = pos.min(set2.len() - 1);
                        *b = set2[mapped_idx];
                    }
                }
            }
            Some((0, out, Vec::new()))
        }
        "exit" => {
            let code = rest
                .first()
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(0);
            Some((code & 0xFF, Vec::new(), Vec::new()))
        }
        "true" => Some((0, Vec::new(), Vec::new())),
        "false" => Some((1, Vec::new(), Vec::new())),
        "env" if rest.is_empty() => Some((0, format_env_map(child_env), Vec::new())),
        "printenv" => {
            if rest.is_empty() {
                return Some((0, format_env_map(child_env), Vec::new()));
            }
            if let Some(val) = child_env.get(&rest[0]) {
                Some((0, format!("{}\n", val).into_bytes(), Vec::new()))
            } else {
                Some((1, Vec::new(), Vec::new()))
            }
        }
        _ => None,
    }
}

pub(super) fn try_eval_portable_shell_atom(
    cmd: &str,
    child_env: &BTreeMap<String, String>,
    stdin_bytes: &[u8],
) -> Option<(i32, Vec<u8>, Vec<u8>)> {
    let words = split_shell_words(cmd.trim());
    let first = words.first()?;
    let norm = first.replace('\\', "/");
    let base_raw = norm.rsplit('/').next().unwrap_or(norm.as_str());
    let base = base_raw
        .strip_suffix(".exe")
        .or_else(|| base_raw.strip_suffix(".EXE"))
        .unwrap_or(base_raw)
        .to_ascii_lowercase();
    try_eval_portable_builtin_argv(&base, &words[1..], child_env, stdin_bytes, true)
}

pub(super) fn try_eval_portable_shell_script(
    script: &str,
    child_env: &BTreeMap<String, String>,
    initial_stdin: &[u8],
    max_buffer: usize,
) -> Option<SubprocessSyncResult> {
    if script
        .bytes()
        .any(|b| matches!(b, b'`' | b'>' | b'<' | b'(' | b')'))
    {
        return None;
    }
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ChainOp {
        Always,
        And,
        Or,
    }
    let mut clauses: Vec<(ChainOp, &str)> = Vec::new();
    let bytes = script.as_bytes();
    let mut start = 0;
    let mut next_op = ChainOp::Always;
    let mut i = 0;
    let mut in_quote: Option<u8> = None;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = in_quote {
            if b == q {
                in_quote = None;
            }
            i += 1;
            continue;
        }
        if b == b'\'' || b == b'"' {
            in_quote = Some(b);
            i += 1;
            continue;
        }
        if b == b';' {
            clauses.push((next_op, &script[start..i]));
            next_op = ChainOp::Always;
            i += 1;
            start = i;
        } else if i + 1 < bytes.len() && b == b'&' && bytes[i + 1] == b'&' {
            clauses.push((next_op, &script[start..i]));
            next_op = ChainOp::And;
            i += 2;
            start = i;
        } else if i + 1 < bytes.len() && b == b'|' && bytes[i + 1] == b'|' {
            clauses.push((next_op, &script[start..i]));
            next_op = ChainOp::Or;
            i += 2;
            start = i;
        } else {
            i += 1;
        }
    }
    if start < bytes.len() {
        clauses.push((next_op, &script[start..]));
    }
    if clauses.is_empty() {
        return Some(Ok((Some(0), Vec::new(), Vec::new(), None, None)));
    }

    let mut total_stdout = Vec::new();
    let mut total_stderr = Vec::new();
    let mut last_status = 0;

    for (idx, (op, clause_raw)) in clauses.into_iter().enumerate() {
        let clause = clause_raw.trim();
        if clause.is_empty() {
            continue;
        }
        match op {
            ChainOp::Always => {}
            ChainOp::And if last_status != 0 => continue,
            ChainOp::Or if last_status == 0 => continue,
            _ => {}
        }
        let stages: Vec<&str> = clause.split('|').collect();
        let mut pipe_data = if idx == 0 {
            initial_stdin.to_vec()
        } else {
            Vec::new()
        };
        let mut clause_status = 0;
        let mut exited_early = false;
        for (s_idx, stage) in stages.iter().enumerate() {
            let trimmed = stage.trim();
            let (st, out_b, err_b) = try_eval_portable_shell_atom(trimmed, child_env, &pipe_data)?;
            clause_status = st;
            total_stderr.extend_from_slice(&err_b);
            if s_idx + 1 < stages.len() {
                pipe_data = out_b;
            } else {
                total_stdout.extend_from_slice(&out_b);
            }
            if trimmed == "exit" || trimmed.starts_with("exit ") {
                exited_early = true;
                break;
            }
        }
        last_status = clause_status;
        if total_stdout.len().saturating_add(total_stderr.len()) > max_buffer {
            return Some(Err(
                "RangeError [ERR_CHILD_PROCESS_STDIO_MAXBUFFER]: stdout/stderr maxBuffer length exceeded"
                    .to_string(),
            ));
        }
        if exited_early {
            break;
        }
    }
    Some(Ok((
        Some(last_status),
        total_stdout,
        total_stderr,
        None,
        None,
    )))
}
