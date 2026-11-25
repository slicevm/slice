//! 5-Ring OCap-Jailed Subprocess Execution (`node:child_process`) Host Bindings.

mod portable_shell;
mod runner;

use crate::bindings::helpers::{
    active_tenant_id_or_default, consume_crypto_kdf_fuel, ocap_denied_js_error,
    trip_isolate_oom_limits, try_consume_isolate_memory_ctx,
};
use crate::boa_realm::{BOA_EGRESS_SECRETS, BOA_ENV_MAP, BOA_EXEC_BINS, check_boa_fs_read_ocap};
use crate::intrinsics::{base64_decode, base64_encode};
use crate::state_and_kv::verify_egress_secrets_slice;
use runner::execute_sanitized_subprocess_sync;
use slice_sandbox::SandboxViolation;

pub(super) type SubprocessSyncResult = Result<
    (
        Option<i32>,
        Vec<u8>,
        Vec<u8>,
        Option<String>,
        Option<(String, i32)>,
    ),
    String,
>;

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

/// Returns `true` if `key` is a dynamic-linker, interpreter startup, or `PATH` override variable
/// (`LD_PRELOAD`, `DYLD_INSERT_LIBRARIES`, `NODE_OPTIONS`, `PYTHONPATH`, `BASH_ENV`, `PATH`, etc.)
/// that must not be injected by untrusted scripts unless explicitly granted in `BOA_ENV_MAP`.
#[inline]
fn is_restricted_loader_env_key(key: &str) -> bool {
    let upper = key.trim().to_ascii_uppercase();
    upper.starts_with("LD_")
        || upper.starts_with("DYLD_")
        || upper.starts_with("BASH_FUNC_")
        || matches!(
            upper.as_str(),
            "PATH"
                | "IFS"
                | "BASH_ENV"
                | "ENV"
                | "GLIBC_TUNABLES"
                | "PROMPT_COMMAND"
                | "CDPATH"
                | "NODE_OPTIONS"
                | "PYTHONPATH"
                | "PYTHONSTARTUP"
                | "PYTHONHOME"
                | "PERL5OPT"
                | "PERL5LIB"
                | "RUBYOPT"
                | "RUBYLIB"
                | "JAVA_TOOL_OPTIONS"
                | "_JAVA_OPTIONS"
                | "DOTNET_STARTUP_HOOKS"
        )
}

pub(crate) fn register_child_process_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{JsNativeError, JsValue as BoaVal, NativeFunction, js_string};
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

            let (shell_opt, is_custom_shell): (Option<&str>, bool) = match &opts.shell {
                None | Some(serde_json::Value::Null) | Some(serde_json::Value::Bool(false)) => {
                    (None, false)
                }
                Some(serde_json::Value::String(s)) if s.is_empty() => (None, false),
                Some(serde_json::Value::Number(n)) if n.as_f64() == Some(0.0) => (None, false),
                Some(serde_json::Value::String(s)) => (Some(s.as_str()), true),
                Some(_) => (Some("/bin/sh"), false),
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
                        || (!is_custom_shell
                            && sh_bin == "/bin/sh"
                            && slice_sandbox::exec_binary_matches("sh", allowed))
                });
                if shell_allowed {
                    true
                } else if is_custom_shell {
                    false
                } else {
                    let full_cmd = &exec_args[1];
                    if !slice_sandbox::has_exec_shell_metachar(full_cmd) {
                        let first_tok = full_cmd.split_whitespace().next().unwrap_or("");
                        !first_tok.is_empty()
                            && !first_tok.contains('=')
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
                crate::boa_realm::set_child_proc_violation(violation);
                return Err(ocap_denied_js_error("ERR_SLICE_EXEC_DENIED", &msg, context));
            }

            // Verify `opts.cwd` (and any VFS/host symlink hops) against the tenant's `FsRead` capability.
            let resolved_cwd: Option<String> = if let Some(ref cwd_dir) = opts.cwd
                && !cwd_dir.trim().is_empty()
            {
                if crate::bindings::fs::is_host_unc_blocked(cwd_dir)
                    || !check_boa_fs_read_ocap(cwd_dir)
                {
                    let violation = SandboxViolation::FsReadDenied {
                        tenant_id,
                        path: cwd_dir.clone(),
                    };
                    let msg = violation.to_string();
                    crate::boa_realm::set_child_proc_violation(violation);
                    return Err(ocap_denied_js_error("ERR_SLICE_FS_DENIED", &msg, context));
                }
                match crate::bindings::fs::resolve_effective_vfs_path(cwd_dir, true, true) {
                    Ok(res) => {
                        if !check_boa_fs_read_ocap(&res.norm) {
                            let violation = SandboxViolation::FsReadDenied {
                                tenant_id,
                                path: res.norm,
                            };
                            let msg = violation.to_string();
                            crate::boa_realm::set_child_proc_violation(violation);
                            return Err(ocap_denied_js_error("ERR_SLICE_FS_DENIED", &msg, context));
                        }
                        Some(res.norm)
                    }
                    Err(crate::bindings::fs::SymlinkResolveError::ReadOcapDenied(denied)) => {
                        let _ = check_boa_fs_read_ocap(&denied);
                        let violation = SandboxViolation::FsReadDenied {
                            tenant_id,
                            path: denied,
                        };
                        let msg = violation.to_string();
                        crate::boa_realm::set_child_proc_violation(violation);
                        return Err(ocap_denied_js_error("ERR_SLICE_FS_DENIED", &msg, context));
                    }
                    Err(crate::bindings::fs::SymlinkResolveError::Loop) => {
                        return Err(JsNativeError::error()
                            .with_message(format!(
                                "ELOOP: too many symbolic links encountered, chdir '{}'",
                                cwd_dir
                            ))
                            .into());
                    }
                }
            } else {
                None
            };

            // Ring 5: Base CPU Fuel charge for subprocess invocation
            if let Err(fuel_err) = consume_crypto_kdf_fuel(500) {
                trip_isolate_oom_limits(context);
                return Err(JsNativeError::range().with_message(fuel_err).into());
            }

            // Ring 2: Environment Sanitization - construct child environment strictly from
            // the tenant's attenuated `BOA_ENV_MAP` (never host `std::env::vars()`), blocking
            // dynamic-linker, interpreter startup, and `PATH` injection variables.
            let tenant_env = BOA_ENV_MAP.with(|m| m.borrow().clone());
            let mut child_env = std::collections::BTreeMap::new();
            if let Some(custom_env) = opts.env {
                for (k, v) in custom_env {
                    if is_restricted_loader_env_key(&k)
                        && !tenant_env.keys().any(|tk| tk.eq_ignore_ascii_case(&k))
                    {
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
                    resolved_cwd.as_deref(),
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
                    trip_isolate_oom_limits(context);
                    return Err(JsNativeError::range().with_message(fuel_err).into());
                }
                if let Err(msg) = try_consume_isolate_memory_ctx(total_out_bytes, context) {
                    return Err(JsNativeError::range().with_message(msg).into());
                }
            }

            let stdout_str = String::from_utf8_lossy(&stdout_bytes).into_owned();
            let stderr_str = String::from_utf8_lossy(&stderr_bytes).into_owned();

            // Ring 3: Output Egress DLP - scan both `stdout` and `stderr` against `BOA_EGRESS_SECRETS`.
            let egress_secrets = BOA_EGRESS_SECRETS.with(|s| s.borrow().clone());
            if !egress_secrets.is_empty()
                && let Err(violation) =
                    verify_egress_secrets_slice(&tenant_id, &egress_secrets, &stdout_str).and_then(
                        |()| verify_egress_secrets_slice(&tenant_id, &egress_secrets, &stderr_str),
                    )
            {
                let msg = violation.to_string();
                crate::boa_realm::set_child_proc_violation(violation);
                return Err(JsNativeError::error().with_message(msg).into());
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
