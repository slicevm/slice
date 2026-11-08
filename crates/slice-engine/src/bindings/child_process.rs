//! 5-Ring OCap-Jailed Subprocess Execution (`node:child_process`) Host Bindings.

use crate::bindings::helpers::{active_tenant_id_or_default, consume_crypto_kdf_fuel};
use crate::boa_realm::{
    check_boa_fs_read_ocap, BOA_EGRESS_SECRETS, BOA_ENV_MAP, BOA_EXEC_BINS, BOA_KV_MEM_BUDGET,
    BOA_LAST_CHILD_PROC_VIOLATION, BOA_VIOLATION,
};
use crate::intrinsics::base64_encode;
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

type SubprocessSyncResult = Result<(i32, Vec<u8>, Vec<u8>, Option<String>), String>;

fn execute_sanitized_subprocess_sync(
    file: &str,
    args: &[String],
    child_env: &std::collections::BTreeMap<String, String>,
    cwd: Option<&str>,
    stdin_input: Option<&str>,
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

    // Portable built-in handling for standard POSIX utilities (`echo`, `env`, `printenv`, `true`, `false`)
    // after Ring 1 (`ExecCap`) and Ring 2 (`BOA_ENV_MAP` sanitization) have verified authority.
    match base_cmd.as_str() {
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
            return Ok((0, out_bytes, Vec::new(), None));
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
            return Ok((0, out_bytes, Vec::new(), None));
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
                return Ok((0, out_bytes, Vec::new(), None));
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
                return Ok((0, out_bytes, Vec::new(), None));
            }
            return Ok((1, Vec::new(), Vec::new(), None));
        }
        "true" => return Ok((0, Vec::new(), Vec::new(), None)),
        "false" => return Ok((1, Vec::new(), Vec::new(), None)),
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

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Error: spawn {} ENOENT ({})", file, e))?;

    if let Some(input_data) = stdin_input {
        if let Some(mut stdin_pipe) = child.stdin.take() {
            let _ = stdin_pipe.write_all(input_data.as_bytes());
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
        return Err(format!(
            "Error: spawnSync {} ETIMEDOUT (exceeded {} ms)",
            file, timeout_ms
        ));
    }

    // Ring 4: Output byte ceiling (`maxBuffer`)
    if stdout_bytes.len().saturating_add(stderr_bytes.len()) > max_buffer {
        return Err(
            "RangeError [ERR_CHILD_PROCESS_STDIO_MAXBUFFER]: stdout/stderr maxBuffer length exceeded"
                .to_string(),
        );
    }

    Ok((exit_code, stdout_bytes, stderr_bytes, None))
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

            // Ring 1: OCap `ExecCap` Allowlist Verification (Deny-All by default)
            let allowed_bins = BOA_EXEC_BINS.with(|eb| eb.borrow().clone());
            let is_allowed = !file.trim().is_empty()
                && allowed_bins
                    .iter()
                    .any(|allowed| slice_sandbox::exec_binary_matches(&file, allowed));
            if !is_allowed {
                let violation = SandboxViolation::ExecDenied {
                    tenant_id,
                    binary: file,
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

            // If `shell` option is explicitly requested as a string path or boolean `true`,
            // also verify that the shell binary itself is authorized in `allowed_exec_binaries`.
            if let Some(ref sh_val) = opts.shell {
                let shell_bin = match sh_val {
                    serde_json::Value::Bool(true) => Some("/bin/sh"),
                    serde_json::Value::String(s) if !s.is_empty() => Some(s.as_str()),
                    _ => None,
                };
                if let Some(sb) = shell_bin {
                    let shell_allowed = allowed_bins
                        .iter()
                        .any(|allowed| slice_sandbox::exec_binary_matches(sb, allowed));
                    if !shell_allowed {
                        let violation = SandboxViolation::ExecDenied {
                            tenant_id,
                            binary: sb.to_string(),
                        };
                        let msg = violation.to_string();
                        BOA_LAST_CHILD_PROC_VIOLATION.with(|v| {
                            *v.borrow_mut() = Some(violation);
                        });
                        return Err(JsNativeError::error().with_message(msg).into());
                    }
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

            let timeout_ms = opts.timeout.unwrap_or(30_000).min(120_000);
            let max_buffer = opts.max_buffer.unwrap_or(1024 * 1024).min(16 * 1024 * 1024);

            let (status_code, stdout_bytes, stderr_bytes, signal) =
                match execute_sanitized_subprocess_sync(
                    &file,
                    &cmd_args,
                    &child_env,
                    opts.cwd.as_deref(),
                    opts.input.as_deref(),
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

            let resp = serde_json::json!({
                "status": status_code,
                "signal": signal,
                "stdoutBase64": base64_encode(&stdout_bytes, false),
                "stderrBase64": base64_encode(&stderr_bytes, false),
                "stdout": stdout_str,
                "stderr": stderr_str,
                "pid": 4200,
            });
            Ok(BoaVal::from(js_string!(resp.to_string())))
        }),
    );
}
