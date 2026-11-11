//! 5-Ring OCap-Jailed Subprocess Execution (`node:child_process`) Host Bindings.

use crate::bindings::helpers::{active_tenant_id_or_default, consume_crypto_kdf_fuel};
use crate::boa_realm::{
    check_boa_fs_read_ocap, BOA_EGRESS_SECRETS, BOA_ENV_MAP, BOA_EXEC_BINS, BOA_KV_MEM_BUDGET,
    BOA_LAST_CHILD_PROC_VIOLATION, BOA_VIOLATION,
};
use crate::intrinsics::{base64_decode, base64_encode};
use crate::state_and_kv::verify_egress_secrets_slice;
use slice_sandbox::SandboxViolation;

#[derive(Default, serde::Deserialize)]
struct ChildProcessSpawnOpts {
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: Option<std::collections::BTreeMap<String, serde_json::Value>>,
    #[serde(default)]
    input: Option<String>,
    #[serde(default, rename = "inputBase64")]
    input_base64: Option<String>,
    #[serde(default)]
    timeout: Option<u64>,
    #[serde(default, rename = "maxBuffer")]
    max_buffer: Option<usize>,
    #[serde(default)]
    shell: Option<serde_json::Value>,
}

/// Returns `true` if `key` is a dynamic-linker or shell startup injection environment variable
/// (`LD_PRELOAD`, `LD_LIBRARY_PATH`, `DYLD_INSERT_LIBRARIES`, `BASH_ENV`, etc.) that must not be
/// injected by untrusted scripts unless explicitly granted in the parent isolate's `BOA_ENV_MAP`.
#[inline]
fn is_restricted_loader_env_key(key: &str) -> bool {
    let upper = key.trim().to_ascii_uppercase();
    upper.starts_with("LD_")
        || upper.starts_with("DYLD_")
        || matches!(
            upper.as_str(),
            "BASH_ENV" | "ENV" | "GLIBC_TUNABLES" | "PROMPT_COMMAND" | "CDPATH"
        )
}

type SubprocessSyncResult = Result<
    (
        Option<i32>,
        Vec<u8>,
        Vec<u8>,
        Option<String>,
        Option<(String, i32)>,
    ),
    String,
>;

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

fn try_eval_portable_shell_atom(
    cmd: &str,
    child_env: &std::collections::BTreeMap<String, String>,
    stdin_bytes: &[u8],
) -> Option<(i32, Vec<u8>, Vec<u8>)> {
    let words = split_shell_words(cmd.trim());
    let first = words.first()?;
    let norm = first.replace('\\', "/");
    let base = norm
        .rsplit('/')
        .next()
        .unwrap_or(norm.as_str())
        .to_ascii_lowercase();
    let rest = &words[1..];
    match base.as_str() {
        "echo" => {
            let mut out = String::new();
            let (no_nl, slice) = if rest.first().map(|s| s.as_str()) == Some("-n") {
                (true, &rest[1..])
            } else {
                (false, rest)
            };
            for (i, a) in slice.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                if let Some(var_name) = a.strip_prefix('$') {
                    let clean = var_name.trim_matches(|c: char| c == '{' || c == '}');
                    out.push_str(child_env.get(clean).map(|s| s.as_str()).unwrap_or(""));
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
        "tr" if rest.len() == 2 => {
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
        "env" if rest.is_empty() => {
            let mut out = String::new();
            for (k, v) in child_env {
                out.push_str(k);
                out.push('=');
                out.push_str(v);
                out.push('\n');
            }
            Some((0, out.into_bytes(), Vec::new()))
        }
        "printenv" => {
            if rest.is_empty() {
                let mut out = String::new();
                for (k, v) in child_env {
                    out.push_str(k);
                    out.push('=');
                    out.push_str(v);
                    out.push('\n');
                }
                return Some((0, out.into_bytes(), Vec::new()));
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

fn try_eval_portable_shell_script(
    script: &str,
    child_env: &std::collections::BTreeMap<String, String>,
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

fn execute_sanitized_subprocess_sync(
    file: &str,
    args: &[String],
    child_env: &std::collections::BTreeMap<String, String>,
    cwd: Option<&str>,
    stdin_input: Option<&[u8]>,
    timeout_ms: u64,
    max_buffer: usize,
) -> SubprocessSyncResult {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let norm_file = file.trim().replace('\\', "/");
    let base_raw = norm_file.rsplit('/').next().unwrap_or(norm_file.as_str());
    let base_cmd = base_raw
        .strip_suffix(".exe")
        .or_else(|| base_raw.strip_suffix(".EXE"))
        .unwrap_or(base_raw)
        .to_ascii_lowercase();

    // Portable built-in handling for standard POSIX utilities (`echo`, `cat`, `tr`, `exit`, `env`, `printenv`, `true`, `false`, `sh -c`)
    // after Ring 1 (`ExecCap`) and Ring 2 (`BOA_ENV_MAP` sanitization) have verified authority.
    match base_cmd.as_str() {
        "sh" if args.len() == 2 && args[0] == "-c" => {
            if let Some(res) = try_eval_portable_shell_script(
                &args[1],
                child_env,
                stdin_input.unwrap_or(&[]),
                max_buffer,
            ) {
                return res;
            }
        }
        "cat" if args.is_empty() => {
            let out_bytes = stdin_input.unwrap_or(&[]).to_vec();
            if out_bytes.len() > max_buffer {
                return Err(
                    "RangeError [ERR_CHILD_PROCESS_STDIO_MAXBUFFER]: stdout maxBuffer length exceeded"
                        .to_string(),
                );
            }
            return Ok((Some(0), out_bytes, Vec::new(), None, None));
        }
        "exit" => {
            let code = args
                .first()
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(0);
            return Ok((Some(code & 0xFF), Vec::new(), Vec::new(), None, None));
        }
        "echo" => {
            let mut out = String::new();
            let (no_newline, slice) = if args.first().map(|s| s.as_str()) == Some("-n") {
                (true, &args[1..])
            } else {
                (false, args)
            };
            for (i, a) in slice.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                out.push_str(a);
            }
            if !no_newline {
                out.push('\n');
            }
            let out_bytes = out.into_bytes();
            if out_bytes.len() > max_buffer {
                return Err(
                    "RangeError [ERR_CHILD_PROCESS_STDIO_MAXBUFFER]: stdout maxBuffer length exceeded"
                        .to_string(),
                );
            }
            return Ok((Some(0), out_bytes, Vec::new(), None, None));
        }
        "env" if args.is_empty() => {
            let mut out = String::new();
            for (k, v) in child_env {
                out.push_str(k);
                out.push('=');
                out.push_str(v);
                out.push('\n');
            }
            let out_bytes = out.into_bytes();
            if out_bytes.len() > max_buffer {
                return Err(
                    "RangeError [ERR_CHILD_PROCESS_STDIO_MAXBUFFER]: stdout maxBuffer length exceeded"
                        .to_string(),
                );
            }
            return Ok((Some(0), out_bytes, Vec::new(), None, None));
        }
        "printenv" => {
            if args.is_empty() {
                let mut out = String::new();
                for (k, v) in child_env {
                    out.push_str(k);
                    out.push('=');
                    out.push_str(v);
                    out.push('\n');
                }
                let out_bytes = out.into_bytes();
                if out_bytes.len() > max_buffer {
                    return Err(
                        "RangeError [ERR_CHILD_PROCESS_STDIO_MAXBUFFER]: stdout maxBuffer length exceeded"
                            .to_string(),
                    );
                }
                return Ok((Some(0), out_bytes, Vec::new(), None, None));
            }
            let key = &args[0];
            if let Some(val) = child_env.get(key) {
                let out_bytes = format!("{}\n", val).into_bytes();
                if out_bytes.len() > max_buffer {
                    return Err(
                        "RangeError [ERR_CHILD_PROCESS_STDIO_MAXBUFFER]: stdout maxBuffer length exceeded"
                            .to_string(),
                    );
                }
                return Ok((Some(0), out_bytes, Vec::new(), None, None));
            }
            return Ok((Some(1), Vec::new(), Vec::new(), None, None));
        }
        "true" => return Ok((Some(0), Vec::new(), Vec::new(), None, None)),
        "false" => return Ok((Some(1), Vec::new(), Vec::new(), None, None)),
        _ => {}
    }

    let mut cmd = Command::new(file);
    cmd.args(args);
    // Ring 2: Strictly clear host ambient environment and inject ONLY tenant-attenuated variables
    cmd.env_clear();
    cmd.envs(child_env);
    if let Some(dir) = cwd {
        if !dir.is_empty() && std::path::Path::new(dir).is_dir() {
            cmd.current_dir(dir);
        }
    }
    cmd.stdin(if stdin_input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let (err_code, err_no) = match e.kind() {
                std::io::ErrorKind::NotFound => ("ENOENT", -2),
                std::io::ErrorKind::PermissionDenied => ("EACCES", -13),
                _ => ("ENOENT", -2),
            };
            return Ok((
                None,
                Vec::new(),
                Vec::new(),
                None,
                Some((err_code.to_string(), err_no)),
            ));
        }
    };

    if let Some(input_data) = stdin_input {
        if let Some(mut stdin_pipe) = child.stdin.take() {
            let _ = stdin_pipe.write_all(input_data);
        }
    }

    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let stdout_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(ref mut p) = stdout_pipe {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });
    let stderr_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(ref mut p) = stderr_pipe {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });

    let deadline = Instant::now() + Duration::from_millis(timeout_ms.max(1));
    let mut timed_out = false;
    let exit_code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(1),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break 1;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(e) => {
                return Err(format!("Error waiting for subprocess {}: {}", file, e));
            }
        }
    };

    let stdout_bytes = stdout_handle.join().unwrap_or_default();
    let stderr_bytes = stderr_handle.join().unwrap_or_default();

    if timed_out {
        return Ok((
            None,
            stdout_bytes,
            stderr_bytes,
            Some("SIGTERM".to_string()),
            Some(("ETIMEDOUT".to_string(), -110)),
        ));
    }

    // Ring 4: Output byte ceiling (`maxBuffer`)
    if stdout_bytes.len().saturating_add(stderr_bytes.len()) > max_buffer {
        return Err(
            "RangeError [ERR_CHILD_PROCESS_STDIO_MAXBUFFER]: stdout/stderr maxBuffer length exceeded"
                .to_string(),
        );
    }

    Ok((Some(exit_code), stdout_bytes, stderr_bytes, None, None))
}

pub(crate) fn register_child_process_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsNativeError, JsValue as BoaVal, NativeFunction};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_child_process_spawn_sync"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_child_process_spawn_sync"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let file = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let args_json = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "[]".to_string());
            let opts_json = args
                .get(2)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "{}".to_string());

            let cmd_args: Vec<String> = serde_json::from_str(&args_json).unwrap_or_default();
            let opts: ChildProcessSpawnOpts = serde_json::from_str(&opts_json).unwrap_or_default();

            let tenant_id = active_tenant_id_or_default();

            let shell_opt: Option<&str> = match &opts.shell {
                Some(serde_json::Value::Bool(true)) => Some("/bin/sh"),
                Some(serde_json::Value::String(s)) if !s.is_empty() => Some(s.as_str()),
                _ => None,
            };

            let (exec_file, exec_args) = if let Some(sh_bin) = shell_opt {
                let full_cmd = if cmd_args.is_empty() {
                    file.clone()
                } else {
                    format!("{} {}", file, cmd_args.join(" "))
                };
                (sh_bin.to_string(), vec!["-c".to_string(), full_cmd])
            } else {
                (file.clone(), cmd_args)
            };

            // Ring 1: OCap `ExecCap` Allowlist Verification (Deny-All by default)
            let allowed_bins = BOA_EXEC_BINS.with(|eb| eb.borrow().clone());
            let is_allowed = if let Some(sh_bin) = shell_opt {
                let shell_allowed = allowed_bins.iter().any(|allowed| {
                    slice_sandbox::exec_binary_matches(sh_bin, allowed)
                        || (sh_bin == "/bin/sh"
                            && slice_sandbox::exec_binary_matches("sh", allowed))
                });
                if shell_allowed {
                    true
                } else {
                    let full_cmd = &exec_args[1];
                    let has_meta = full_cmd.bytes().any(|b| {
                        matches!(
                            b,
                            b'&' | b'|' | b';' | b'`' | b'$' | b'>' | b'<' | b'(' | b')'
                        )
                    });
                    if !has_meta {
                        let first_tok = full_cmd.split_whitespace().next().unwrap_or("");
                        !first_tok.is_empty()
                            && allowed_bins.iter().any(|allowed| {
                                slice_sandbox::exec_binary_matches(first_tok, allowed)
                            })
                    } else {
                        false
                    }
                }
            } else {
                !exec_file.trim().is_empty()
                    && allowed_bins
                        .iter()
                        .any(|allowed| slice_sandbox::exec_binary_matches(&exec_file, allowed))
            };
            if !is_allowed {
                let violation = SandboxViolation::ExecDenied {
                    tenant_id,
                    binary: exec_file,
                };
                let msg = violation.to_string();
                BOA_LAST_CHILD_PROC_VIOLATION.with(|v| {
                    *v.borrow_mut() = Some(violation);
                });
                return Err(JsNativeError::error().with_message(msg).into());
            }

            // Verify `opts.cwd` against the tenant's `FsRead` capability before entering directory.
            if let Some(ref cwd_dir) = opts.cwd {
                if !cwd_dir.trim().is_empty() && !check_boa_fs_read_ocap(cwd_dir) {
                    let violation = SandboxViolation::FsReadDenied {
                        tenant_id,
                        path: cwd_dir.clone(),
                    };
                    let msg = violation.to_string();
                    BOA_LAST_CHILD_PROC_VIOLATION.with(|v| {
                        *v.borrow_mut() = Some(violation);
                    });
                    return Err(JsNativeError::error().with_message(msg).into());
                }
            }

            // Ring 5: Base CPU Fuel charge for subprocess invocation
            if let Err(fuel_err) = consume_crypto_kdf_fuel(500) {
                return Err(JsNativeError::range().with_message(fuel_err).into());
            }

            // Ring 2: Environment Sanitization — construct child environment strictly from
            // the tenant's attenuated `BOA_ENV_MAP` (never host `std::env::vars()`), blocking
            // dynamic-linker injection variables (`LD_PRELOAD`, `DYLD_INSERT_LIBRARIES`, etc.).
            let tenant_env = BOA_ENV_MAP.with(|m| m.borrow().clone());
            let mut child_env = std::collections::BTreeMap::new();
            if let Some(custom_env) = opts.env {
                for (k, v) in custom_env {
                    if is_restricted_loader_env_key(&k) && !tenant_env.contains_key(&k) {
                        continue;
                    }
                    match v {
                        serde_json::Value::String(s) => {
                            child_env.insert(k, s);
                        }
                        serde_json::Value::Number(n) => {
                            child_env.insert(k, n.to_string());
                        }
                        serde_json::Value::Bool(b) => {
                            child_env.insert(k, b.to_string());
                        }
                        _ => {}
                    }
                }
            } else {
                for (k, v) in &tenant_env {
                    child_env.insert(k.clone(), v.clone());
                }
            }
            if shell_opt.is_some() {
                child_env
                    .entry("PATH".to_string())
                    .or_insert_with(|| "/usr/local/bin:/usr/bin:/bin".to_string());
            }

            let timeout_ms = opts.timeout.unwrap_or(30_000).min(120_000);
            let max_buffer = opts.max_buffer.unwrap_or(1024 * 1024).min(16 * 1024 * 1024);
            let stdin_bytes: Option<Vec<u8>> = if let Some(ref b64) = opts.input_base64 {
                Some(base64_decode(b64))
            } else {
                opts.input.as_ref().map(|s| s.as_bytes().to_vec())
            };

            let (status_code, stdout_bytes, stderr_bytes, signal, spawn_err) =
                match execute_sanitized_subprocess_sync(
                    &exec_file,
                    &exec_args,
                    &child_env,
                    opts.cwd.as_deref(),
                    stdin_bytes.as_deref(),
                    timeout_ms,
                    max_buffer,
                ) {
                    Ok(res) => res,
                    Err(err_msg) => {
                        return Err(JsNativeError::error().with_message(err_msg).into());
                    }
                };

            // Ring 5: Charge fuel & memory ceiling proportional to child stdio output
            let total_out_bytes = stdout_bytes.len().saturating_add(stderr_bytes.len());
            if total_out_bytes > 0 {
                let extra_fuel = (total_out_bytes as u64 / 64).max(1);
                if let Err(fuel_err) = consume_crypto_kdf_fuel(extra_fuel) {
                    return Err(JsNativeError::range().with_message(fuel_err).into());
                }
                let (used_mem, limit_mem) = BOA_KV_MEM_BUDGET.with(|m| m.get());
                let next_mem = used_mem.saturating_add(total_out_bytes);
                if next_mem > limit_mem {
                    let violation = SandboxViolation::MemoryCeilingExceeded {
                        tenant_id: tenant_id.clone(),
                        requested: next_mem,
                        limit: limit_mem,
                    };
                    let msg = violation.to_string();
                    BOA_VIOLATION.with(|v| {
                        if v.borrow().is_none() {
                            *v.borrow_mut() = Some(violation);
                        }
                    });
                    return Err(JsNativeError::range().with_message(msg).into());
                }
                BOA_KV_MEM_BUDGET.with(|m| m.set((next_mem, limit_mem)));
            }

            let stdout_str = String::from_utf8_lossy(&stdout_bytes).into_owned();
            let stderr_str = String::from_utf8_lossy(&stderr_bytes).into_owned();

            // Ring 3: Output Egress DLP — scan both `stdout` and `stderr` against the tenant's
            // Egress DLP automaton (`BOA_EGRESS_SECRETS`, including `.env` Auto-DLP secrets).
            let egress_secrets = BOA_EGRESS_SECRETS.with(|s| s.borrow().clone());
            if !egress_secrets.is_empty() {
                if let Err(violation) =
                    verify_egress_secrets_slice(&tenant_id, &egress_secrets, &stdout_str).and_then(
                        |()| verify_egress_secrets_slice(&tenant_id, &egress_secrets, &stderr_str),
                    )
                {
                    let msg = violation.to_string();
                    BOA_LAST_CHILD_PROC_VIOLATION.with(|v| {
                        *v.borrow_mut() = Some(violation);
                    });
                    return Err(JsNativeError::error().with_message(msg).into());
                }
            }

            let resp = if let Some((err_code, err_no)) = spawn_err {
                serde_json::json!({
                    "status": status_code,
                    "signal": signal,
                    "stdoutBase64": base64_encode(&stdout_bytes, false),
                    "stderrBase64": base64_encode(&stderr_bytes, false),
                    "stdout": stdout_str,
                    "stderr": stderr_str,
                    "pid": 0,
                    "errorCode": err_code,
                    "errorErrno": err_no,
                })
            } else {
                serde_json::json!({
                    "status": status_code,
                    "signal": signal,
                    "stdoutBase64": base64_encode(&stdout_bytes, false),
                    "stderrBase64": base64_encode(&stderr_bytes, false),
                    "stdout": stdout_str,
                    "stderr": stderr_str,
                    "pid": 4200,
                })
            };
            Ok(BoaVal::from(js_string!(resp.to_string())))
        }),
    );
}
