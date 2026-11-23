//! Sanitized OS Subprocess Spawner with Bounded `maxBuffer` Pipes and Wall-Clock Timeout.

use super::SubprocessSyncResult;
use super::portable_shell::{try_eval_portable_builtin_argv, try_eval_portable_shell_script};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub(super) fn execute_sanitized_subprocess_sync(
    file: &str,
    args: &[String],
    child_env: &BTreeMap<String, String>,
    cwd: Option<&str>,
    stdin_input: Option<&[u8]>,
    timeout_ms: u64,
    max_buffer: usize,
) -> SubprocessSyncResult {
    let norm_file = file.trim().replace('\\', "/");
    let base_raw = norm_file.rsplit('/').next().unwrap_or(norm_file.as_str());
    let base_cmd = base_raw
        .strip_suffix(".exe")
        .or_else(|| base_raw.strip_suffix(".EXE"))
        .unwrap_or(base_raw)
        .to_ascii_lowercase();

    // Portable built-in handling for standard POSIX utilities (`echo`, `cat`, `tr`, `exit`, `env`,
    // `printenv`, `true`, `false`, `sh -c`) after Ring 1 (`ExecCap`) and Ring 2 (`BOA_ENV_MAP`).
    if base_cmd == "sh"
        && args.len() == 2
        && args[0] == "-c"
        && let Some(res) = try_eval_portable_shell_script(
            &args[1],
            child_env,
            stdin_input.unwrap_or(&[]),
            max_buffer,
        )
    {
        return res;
    }

    if matches!(
        base_cmd.as_str(),
        "echo" | "cat" | "exit" | "env" | "printenv" | "true" | "false"
    ) && let Some((code, out_bytes, err_bytes)) = try_eval_portable_builtin_argv(
        &base_cmd,
        args,
        child_env,
        stdin_input.unwrap_or(&[]),
        false,
    ) {
        if out_bytes.len().saturating_add(err_bytes.len()) > max_buffer {
            return Err(
                "RangeError [ERR_CHILD_PROCESS_STDIO_MAXBUFFER]: stdout maxBuffer length exceeded"
                    .to_string(),
            );
        }
        return Ok((Some(code), out_bytes, err_bytes, None, None));
    }

    let mut cmd = Command::new(file);
    cmd.args(args);
    // Ring 2: Strictly clear host ambient environment and inject ONLY tenant-attenuated variables
    cmd.env_clear();
    cmd.envs(child_env);
    if let Some(dir) = cwd
        && !dir.is_empty()
        && std::path::Path::new(dir).is_dir()
    {
        cmd.current_dir(dir);
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

    if let Some(input_data) = stdin_input
        && let Some(mut stdin_pipe) = child.stdin.take()
    {
        let _ = stdin_pipe.write_all(input_data);
    }

    let pipe_cap = (max_buffer as u64).saturating_add(1);
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = stdout_pipe {
            let _ = p.take(pipe_cap).read_to_end(&mut buf);
        }
        buf
    });
    let stderr_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = stderr_pipe {
            let _ = p.take(pipe_cap).read_to_end(&mut buf);
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

    // Ring 4: Output byte ceiling (`maxBuffer`), enforced even on timeout
    if stdout_bytes.len().saturating_add(stderr_bytes.len()) > max_buffer {
        return Err(
            "RangeError [ERR_CHILD_PROCESS_STDIO_MAXBUFFER]: stdout/stderr maxBuffer length exceeded"
                .to_string(),
        );
    }

    if timed_out {
        return Ok((
            None,
            stdout_bytes,
            stderr_bytes,
            Some("SIGTERM".to_string()),
            Some(("ETIMEDOUT".to_string(), -110)),
        ));
    }

    Ok((Some(exit_code), stdout_bytes, stderr_bytes, None, None))
}
