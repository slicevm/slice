use crate::commands::test_runner::discover_test_files;
use crate::commands::unique_temp_cas_path;
use crate::config::{build_cas_archive, collect_watch_fingerprint, parse_u64_digits};
use crate::diagnostics::{handle_mcp_jsonrpc_line, MAX_MCP_LINE_BYTES};
use crate::server::connection::{escape_json_string_into, push_u64_ascii};
use crate::server::{
    resolve_tenant_bytecode_hotswap, serve_connection_keep_alive_with_tls_and_secrets,
};
use slice_engine::{compile_source_to_bytecode, SliceIsolate};
use slice_sandbox::CapabilityToken;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Instant;

#[test]
fn test_cli_sec_01_server_audit_secrets_and_json_escaped_violation_response() {
    // 1. Verify `serve_connection_keep_alive_with_tls_and_secrets` enforces explicit `audit_secrets`
    // (`--audit-egress` / `slice.toml` `audit_egress`) even when the secret is not in `allowed_env_keys`.
    let handler_ts = r#"
export default {
    fetch(req: Request): Response {
        const url = new URL(req.url);
        if (url.pathname === "/leak") {
            return new Response("exfiltrated=sk_live_explicit_audit_secret_778899");
        }
        // Trigger a SandboxViolation containing quotes and backslashes in the key
        const s = env("SECRET_\"QUOTE\"_\\BACKSLASH");
        return new Response(s);
    }
};
"#;
    let bc = std::sync::Arc::new(compile_source_to_bytecode(handler_ts));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let bc_srv = std::sync::Arc::clone(&bc);

    let srv_handle = std::thread::spawn(move || {
        let mut read_buf = vec![0u8; 65536];
        let mut write_buf = Vec::with_capacity(4096);
        let secrets = vec!["sk_live_explicit_audit_secret_778899".to_string()];
        if let Ok((stream, _)) = listener.accept() {
            let _ = serve_connection_keep_alive_with_tls_and_secrets(
                stream,
                1,
                &bc_srv,
                None,
                None,
                None,
                &CapabilityToken::pure_compute(),
                &secrets,
                2_000_000,
                4 * 1024 * 1024,
                &mut read_buf,
                &mut write_buf,
                None,
                None,
            );
        }
    });

    let mut client = TcpStream::connect(addr).unwrap();
    client
        .write_all(
            concat!(
                "GET /leak HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\n\r\n",
                "GET /quote HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .unwrap();
    client.flush().unwrap();

    let mut resp = String::new();
    client.read_to_string(&mut resp).unwrap();
    srv_handle.join().unwrap();

    assert!(
        resp.contains("HTTP/1.1 403 Forbidden") && resp.contains("EGRESS_SECRET_LEAK_BLOCKED"),
        "Expected explicit audit_secrets to block egress leak on HTTP server: {}",
        resp
    );

    // Verify every JSON body in the 403 responses parses as valid RFC 8259 JSON
    for part in resp.split("HTTP/1.1 403 Forbidden").skip(1) {
        if let Some((_, body)) = part.split_once("\r\n\r\n") {
            let trimmed = body.trim();
            let parsed: serde_json::Value = serde_json::from_str(trimmed)
                .unwrap_or_else(|e| panic!("Invalid JSON '{}': {}", trimmed, e));
            assert!(parsed["error"].is_string());
        }
    }

    let mut escaped = String::new();
    escape_json_string_into("bad\"quote\\slash\nline", &mut escaped);
    assert_eq!(escaped, r#"bad\"quote\\slash\nline"#);
}

#[test]
fn test_cli_sec_02_unique_temp_cas_staging_paths_never_collide() {
    let p1 = unique_temp_cas_path("slice_compile_staging").unwrap();
    let p2 = unique_temp_cas_path("slice_compile_staging").unwrap();
    assert_ne!(
        p1, p2,
        "Concurrent staging CAS paths must be unique per invocation"
    );
    assert!(p1
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("slice_compile_staging_"));
}

#[test]
fn test_cli_sec_03_watch_fingerprint_and_test_discovery_skip_symlinks() {
    let tmp_dir = std::env::temp_dir().join("slice_cli_sec03_watch_and_tests");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(tmp_dir.join("src")).unwrap();
    std::fs::create_dir_all(tmp_dir.join("node_modules/pkg")).unwrap();

    std::fs::write(tmp_dir.join("src/index.ts"), "export const x = 1;\n").unwrap();
    std::fs::write(
        tmp_dir.join("src/math.test.ts"),
        "export const ok = true;\n",
    )
    .unwrap();
    std::fs::write(
        tmp_dir.join("node_modules/pkg/ignored.test.ts"),
        "export const ignored = true;\n",
    )
    .unwrap();

    let fp1 = collect_watch_fingerprint(&tmp_dir);
    let fp2 = collect_watch_fingerprint(&tmp_dir);
    assert_eq!(fp1, fp2);
    assert_ne!(fp1, 0);

    let tests = discover_test_files(&tmp_dir).unwrap();
    assert_eq!(tests.len(), 1);
    assert!(tests[0].ends_with("math.test.ts"));

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_cli_sec_04_mcp_allow_net_wiring_and_line_size_cap() {
    let caps = CapabilityToken::pure_compute().with_net_host("api.trusted.internal:443");
    let mut isolate = SliceIsolate::spawn("mcp-sec04", 1, caps, 500_000, 1024 * 1024);

    // 1. Verify `allowNet` in `slice_eval_ts` narrows network capability to only the specified host
    let req_denied = r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"slice_eval_ts","arguments":{"code":"const r = fetch(\"https://evil.example.com/pwn\");","allowNet":"api.trusted.internal:443"}}}"#;
    let resp = handle_mcp_jsonrpc_line(req_denied, &mut isolate).unwrap();
    assert!(resp.contains(r#""isError":true"#), "resp={}", resp);
    assert!(
        resp.contains("lacks outbound network authority"),
        "resp={}",
        resp
    );

    // 2. Verify oversized MCP JSON-RPC line (> MAX_MCP_LINE_BYTES) is rejected cleanly with -32600
    let oversized = "x".repeat(MAX_MCP_LINE_BYTES + 64);
    let over_resp = handle_mcp_jsonrpc_line(&oversized, &mut isolate).unwrap();
    assert!(over_resp.contains("-32600"));
    assert!(over_resp.contains("exceeds 4 MB limit"));
}

#[test]
fn test_cli_perf_01_02_tenant_hotswap_and_itoa_header_microbench() {
    assert_eq!(parse_u64_digits("50_000_000"), Some(50_000_000));
    assert_eq!(parse_u64_digits("16384 KB"), Some(16384));
    assert_eq!(parse_u64_digits("no-digits"), None);

    let mut buf = Vec::with_capacity(64);
    push_u64_ascii(&mut buf, 1234567890);
    assert_eq!(&buf, b"1234567890");

    let tmp_dir = std::env::temp_dir().join("slice_cli_perf_tenant_hotswap");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let src_path = tmp_dir.join("t1.ts");
    let cas_path = tmp_dir.join("tenant-fast-1.cas");
    std::fs::write(&src_path, "Slice.respond(200, \"ok\", \"text/plain\");\n").unwrap();
    build_cas_archive(&src_path, &cas_path, "./t1.ts").unwrap();

    // Warm up cache entry
    let initial = resolve_tenant_bytecode_hotswap(&tmp_dir, "tenant-fast-1").unwrap();
    assert!(!initial.is_empty());

    let iters = 1_000usize;
    let t0 = Instant::now();
    for _ in 0..iters {
        let bc = resolve_tenant_bytecode_hotswap(&tmp_dir, "tenant-fast-1").unwrap();
        std::hint::black_box(bc.len());
    }
    let avg_us = t0.elapsed().as_nanos() as f64 / (iters as f64 * 1_000.0);
    println!(
        "[slice-cli perf] 1,000 cached resolve_tenant_bytecode_hotswap lookups avg: {:.2} us/lookup",
        avg_us
    );

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_wave1_cli_eval_print_check_argv_and_exit_codes() {
    use crate::{cmd_check, cmd_run, LAST_CLI_EXIT_CODE};
    use std::sync::atomic::Ordering;

    let tmp_dir = std::env::temp_dir().join("slice_wave1_cli_ergonomics");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();

    // 1. `-c` / `--check` / `cmd_check` on valid TS/JS vs invalid syntax (must NOT execute side effects)
    let valid_script = tmp_dir.join("valid.ts");
    let side_effect_marker = tmp_dir.join("side_effect_ran.txt");
    std::fs::write(
        &valid_script,
        format!(
            "import fs from 'node:fs';\nfs.writeFileSync({:?}, 'ran');\nconst x: number = 40 + 2;\nexport {{ x }};\n",
            side_effect_marker.to_string_lossy()
        ),
    )
    .unwrap();

    cmd_check(&[valid_script.to_string_lossy().to_string()]).unwrap();
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 0);
    assert!(
        !side_effect_marker.exists(),
        "syntax check must not execute script side effects"
    );

    cmd_run(&[
        "--check".to_string(),
        valid_script.to_string_lossy().to_string(),
    ])
    .unwrap();
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 0);

    let invalid_script = tmp_dir.join("invalid.js");
    std::fs::write(&invalid_script, "const x = (\n").unwrap();
    let err1 = cmd_check(&[invalid_script.to_string_lossy().to_string()]).unwrap_err();
    assert!(err1.to_string().contains("SyntaxError"));
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 1);

    let err2 = cmd_run(&[
        "-c".to_string(),
        invalid_script.to_string_lossy().to_string(),
    ])
    .unwrap_err();
    assert!(err2.to_string().contains("SyntaxError"));
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 1);

    // 2. `-e` / `--eval` with `process.argv` and exit code propagation
    let eval_code = r#"
        if (JSON.stringify(process.argv) !== JSON.stringify(["slice", "arg1", "--flag2"])) {
            process.exit(99);
        }
        process.exit(37);
    "#;
    cmd_run(&[
        "-e".to_string(),
        eval_code.to_string(),
        "arg1".to_string(),
        "--flag2".to_string(),
    ])
    .unwrap();
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 37);

    // 3. Script file execution with trailing script arguments in `process.argv` and `process.exitCode`
    let argv_script = tmp_dir.join("script_argv.js");
    std::fs::write(
        &argv_script,
        r#"
        if (JSON.stringify(process.argv.slice(2)) !== JSON.stringify(["--alpha", "beta", "gamma"])) {
            process.exit(98);
        }
        process.exitCode = 11;
        "#,
    )
    .unwrap();
    cmd_run(&[
        argv_script.to_string_lossy().to_string(),
        "--alpha".to_string(),
        "beta".to_string(),
        "gamma".to_string(),
    ])
    .unwrap();
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 11);

    // 4. `-p` / `--print` evaluates expressions and sets exit code 0
    cmd_run(&["-p".to_string(), "({ sum: 20 + 22 })".to_string()]).unwrap();
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 0);

    // 5. Unhandled Promise rejection in `-e` sets LAST_CLI_EXIT_CODE = 1 and returns Uncaught (in promise)
    let rej_err = cmd_run(&[
        "-e".to_string(),
        "Promise.reject(new Error('cli-unhandled'))".to_string(),
    ])
    .unwrap_err();
    assert!(rej_err
        .to_string()
        .contains("Uncaught (in promise) Error: cli-unhandled"));
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 1);

    // Reset LAST_CLI_EXIT_CODE so test harness exits 0
    LAST_CLI_EXIT_CODE.store(0, Ordering::Relaxed);
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_default_cli_zero_fs_containment_across_run_cas_and_serve() {
    use crate::commands::run::cmd_run;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    let tmp_dir =
        std::env::temp_dir().join(format!("slice_cli_default_zero_fs_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let victim_sub = tmp_dir.join("victim").join("sub");
    std::fs::create_dir_all(&victim_sub).unwrap();
    let victim_file = tmp_dir.join("victim").join("secret.txt");
    std::fs::write(&victim_file, "CLI_HOST_SECRET").unwrap();

    let v_file_norm = victim_file.to_string_lossy().replace('\\', "/");
    let v_dir_norm = tmp_dir.join("victim").to_string_lossy().replace('\\', "/");

    let read_script = tmp_dir.join("read_probe.ts");
    std::fs::write(
        &read_script,
        format!(
            "import * as fs from 'node:fs';\nconsole.log(fs.readFileSync({:?}, 'utf8'));\n",
            v_file_norm
        ),
    )
    .unwrap();

    let rm_script = tmp_dir.join("rm_probe.ts");
    std::fs::write(
        &rm_script,
        format!(
            "import * as fs from 'node:fs';\nfs.unlinkSync({:?});\nfs.rmSync({:?}, {{ recursive: true, force: true }});\n",
            v_file_norm, v_dir_norm
        ),
    )
    .unwrap();

    // 1. Default `slice run` (cold and `--warm`) rejects host reads and deletes
    for warm_flag in [false, true] {
        let mut r_args = vec![read_script.to_string_lossy().to_string()];
        if warm_flag {
            r_args.push("--warm".to_string());
        }
        let r_err = cmd_run(&r_args).unwrap_err().to_string();
        assert!(
            r_err.contains("lacks filesystem read authority"),
            "expected FsReadDenied on slice run (warm={}), got: {}",
            warm_flag,
            r_err
        );

        let mut w_args = vec![rm_script.to_string_lossy().to_string()];
        if warm_flag {
            w_args.push("--warm".to_string());
        }
        let w_err = cmd_run(&w_args).unwrap_err().to_string();
        assert!(
            w_err.contains("lacks filesystem write authority"),
            "expected FsWriteDenied on slice run rm (warm={}), got: {}",
            warm_flag,
            w_err
        );
    }
    assert!(
        victim_file.exists() && victim_sub.exists(),
        "host victim files must remain intact on disk"
    );

    // 2. Packed `.cas` archive executed with default flags also blocks host read
    let cas_path = tmp_dir.join("read_probe.cas");
    build_cas_archive(&read_script, &cas_path, "./read_probe.ts").unwrap();
    let cas_err = cmd_run(&[cas_path.to_string_lossy().to_string()])
        .unwrap_err()
        .to_string();
    assert!(cas_err.contains("lacks filesystem read authority"));

    // 3. `slice serve` HTTP handler with default pure_compute() returns HTTP 403 on host FS probe
    let srv_code = format!(
        "import * as fs from 'node:fs';\nSlice.respond(200, fs.readFileSync({:?}, 'utf8'), 'text/plain');\n",
        v_file_norm
    );
    let srv_bc =
        compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(&srv_code, true));
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let srv_handle = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut rbuf = vec![0u8; 8192];
        let mut wbuf = Vec::with_capacity(1024);
        let _ = serve_connection_keep_alive_with_tls_and_secrets(
            stream,
            1,
            &srv_bc,
            None,
            None,
            None,
            &CapabilityToken::pure_compute(),
            &[],
            5_000_000,
            4 * 1024 * 1024,
            &mut rbuf,
            &mut wbuf,
            None,
            None,
        );
    });
    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    client
        .write_all(b"GET /probe HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut http_resp = String::new();
    client.read_to_string(&mut http_resp).unwrap();
    srv_handle.join().unwrap();
    assert!(
        http_resp.starts_with("HTTP/1.1 403")
            && http_resp.contains("lacks filesystem read authority"),
        "expected HTTP 403 OCap violation from slice serve, got: {}",
        http_resp
    );

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_external_cas_isolates_ambient_cwd_manifest_and_honors_explicit_config() {
    use crate::commands::run::{cmd_run, LAST_CLI_EXIT_CODE};
    use std::sync::atomic::Ordering;

    if let Ok(cas_arg) = std::env::var("SLICE_TEST_ISOLATED_CAS_PATH") {
        let explicit_arg = std::env::var("SLICE_TEST_EXPLICIT_CONFIG_PATH").unwrap();

        // 1. Running the external `.cas` inside `ambient_cwd` must NOT inherit `./slice.toml` or `./.env`
        cmd_run(std::slice::from_ref(&cas_arg)).unwrap();
        assert_eq!(
            LAST_CLI_EXIT_CODE.load(Ordering::Relaxed),
            0,
            "external .cas must not inherit ambient CWD slice.toml or .env"
        );

        // 2. Explicit `--config=<path>` grants the configured capabilities cleanly
        cmd_run(&[format!("--config={}", explicit_arg), cas_arg]).unwrap();
        assert_eq!(
            LAST_CLI_EXIT_CODE.load(Ordering::Relaxed),
            42,
            "explicit --config=<path> must apply to external .cas execution"
        );
        LAST_CLI_EXIT_CODE.store(0, Ordering::Relaxed);
        return;
    }

    let tmp_dir =
        std::env::temp_dir().join(format!("slice_cas_cwd_isolation_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let ambient_cwd = tmp_dir.join("ambient_project");
    let build_dir = tmp_dir.join("clean_build");
    std::fs::create_dir_all(&ambient_cwd).unwrap();
    std::fs::create_dir_all(&build_dir).unwrap();

    // Ambient CWD contains a permissive slice.toml and .env
    std::fs::write(
        ambient_cwd.join(".env"),
        "DOTENV_AMBIENT_SECRET=leaked_from_cwd_dotenv\n",
    )
    .unwrap();
    std::fs::write(
        ambient_cwd.join("slice.toml"),
        r#"
[package]
name = "ambient-cwd"
entrypoint = "./unused.ts"

[capabilities]
allow_read = ["/"]
allow_net = ["*"]
allow_env = ["TOML_AMBIENT_VAR=leaked_from_cwd_toml"]
env_file = ".env"
"#,
    )
    .unwrap();

    // Explicit config file for opt-in verification
    let explicit_toml = tmp_dir.join("explicit_host.toml");
    std::fs::write(
        &explicit_toml,
        r#"
[capabilities]
allow_env = ["EXPLICIT_CONFIG_VAR=granted_via_flag"]
"#,
    )
    .unwrap();

    // Build an external .cas archive in `clean_build` (zero embedded capabilities)
    let probe_src = build_dir.join("probe.ts");
    std::fs::write(
        &probe_src,
        r#"
        if (process.env.TOML_AMBIENT_VAR || process.env.DOTENV_AMBIENT_SECRET) {
            process.exit(41);
        }
        if (process.env.EXPLICIT_CONFIG_VAR === "granted_via_flag") {
            process.exit(42);
        }
        process.exit(0);
        "#,
    )
    .unwrap();
    let cas_path = build_dir.join("untrusted_probe.cas");
    build_cas_archive(&probe_src, &cas_path, "./probe.ts").unwrap();

    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .current_dir(&ambient_cwd)
        .env(
            "SLICE_TEST_ISOLATED_CAS_PATH",
            cas_path.to_string_lossy().as_ref(),
        )
        .env(
            "SLICE_TEST_EXPLICIT_CONFIG_PATH",
            explicit_toml.to_string_lossy().as_ref(),
        )
        .arg("--exact")
        .arg("cli_tests::cli_security_and_perf_tests::test_external_cas_isolates_ambient_cwd_manifest_and_honors_explicit_config")
        .arg("--nocapture")
        .status()
        .unwrap();
    assert!(
        status.success(),
        "isolated CWD subprocess verification failed with status: {}",
        status
    );

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_incremental_http_header_cursor_and_single_pass_manifest_dedup() {
    use crate::config::ResolvedExecutionConfig;
    use crate::server::connection::find_http_header_end_from;

    // 1. Incremental header cursor finds `\r\n\r\n` even when split across fragmented TCP reads
    let raw = b"GET /api/v1/items HTTP/1.1\r\nHost: localhost:8787\r\nX-Custom: abcdef\r\n\r\nBODY";
    let mut filled = 0usize;
    let mut cursor = 0usize;
    let mut found = None;
    for step in [7usize, 19, 25, 16, 2, 1, 4] {
        filled = (filled + step).min(raw.len());
        if let Some(pos) = find_http_header_end_from(&raw[..filled], cursor) {
            found = Some(pos);
            break;
        }
        cursor = filled.saturating_sub(3);
    }
    assert_eq!(found, Some((raw.len() - 8, raw.len() - 4)));
    assert_eq!(&raw[found.unwrap().1..], b"BODY");

    // 2. Single-pass manifest loading skips duplicate re-read when target resolves to same slice.toml
    let tmp_dir =
        std::env::temp_dir().join(format!("slice_cli_manifest_dedup_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let manifest_path = tmp_dir.join("slice.toml");
    let script_path = tmp_dir.join("index.ts");
    std::fs::write(
        &manifest_path,
        "[limits]\nfuel = 1234567\n[capabilities]\nallow_env = [\"DEDUP_KEY=v1\"]\n",
    )
    .unwrap();
    std::fs::write(&script_path, "console.log('ok');\n").unwrap();

    let mut cfg = ResolvedExecutionConfig::from_cli_args_with_ambient_cwd(&[], false).unwrap();
    cfg.apply_target_path_manifest(&script_path).unwrap();
    assert_eq!(cfg.fuel_limit, 1_234_567);
    assert_eq!(
        cfg.caps
            .allowed_env_keys
            .get("DEDUP_KEY")
            .map(String::as_str),
        Some("v1")
    );
    // Mutating the file on disk after `loaded_manifest_path` recorded it must be skipped on duplicate target apply
    std::fs::write(
        &manifest_path,
        "[limits]\nfuel = 9999999\n[capabilities]\nallow_env = [\"DEDUP_KEY=v2\"]\n",
    )
    .unwrap();
    cfg.apply_target_path_manifest(&script_path).unwrap();
    assert_eq!(cfg.fuel_limit, 1_234_567);

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_issue_100_fail_closed_security_and_config_flags_across_forms() {
    use crate::commands::run::cmd_run;
    use crate::config::{load_audit_egress_file, ResolvedExecutionConfig};
    use slice_engine::{compile_source_to_bytecode, SliceIsolate};

    let tmp_dir = std::env::temp_dir().join(format!("slice_issue_100_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();

    let missing_file = tmp_dir.join("does_not_exist.txt");
    let missing_str = missing_file.to_string_lossy().to_string();
    let sub_dir = tmp_dir.join("a_directory");
    std::fs::create_dir_all(&sub_dir).unwrap();
    let sub_dir_str = sub_dir.to_string_lossy().to_string();

    let bad_utf8_file = tmp_dir.join("bad_utf8.bin");
    std::fs::write(&bad_utf8_file, [0xFFu8, 0xFE, 0xFD, 0x80]).unwrap();
    let bad_utf8_str = bad_utf8_file.to_string_lossy().to_string();

    // 1. Every invalid/unreadable/empty/dangling security & config CLI flag form must fail closed (`Err`)
    let failing_flag_sets: Vec<Vec<String>> = vec![
        vec![format!("--audit-egress-file={}", missing_str)],
        vec!["--audit-egress-file".to_string(), missing_str.clone()],
        vec![format!("--audit-egress-file={}", sub_dir_str)],
        vec![format!("--audit-egress-file={}", bad_utf8_str)],
        vec!["--audit-egress-file=".to_string()],
        vec!["--audit-egress-file".to_string()],
        vec!["--audit-egress=".to_string()],
        vec!["--audit-egress".to_string()],
        vec![format!("--env-file={}", missing_str)],
        vec!["--env-file".to_string(), missing_str.clone()],
        vec![format!("--env-file={}", bad_utf8_str)],
        vec!["--env-file=".to_string()],
        vec!["--env-file".to_string()],
        vec![format!("--config={}", sub_dir_str)],
        vec!["--config=".to_string()],
        vec!["--config".to_string()],
        vec!["--trace=".to_string()],
        vec!["--trace".to_string()],
    ];

    for flags in &failing_flag_sets {
        let res = ResolvedExecutionConfig::from_cli_args_with_ambient_cwd(flags, false);
        assert!(
            res.is_err(),
            "expected fail-closed Err for CLI flags {:?}, but succeeded!",
            flags
        );
    }

    // 2. Direct `load_audit_egress_file` fails closed on missing path
    let mut dummy_secrets = Vec::new();
    assert!(load_audit_egress_file(&missing_file, &mut dummy_secrets).is_err());

    // 3. Manifest `env_file` and trusted `@slice-env-file` fail closed when missing
    let mut cfg_manifest =
        ResolvedExecutionConfig::from_cli_args_with_ambient_cwd(&[], false).unwrap();
    assert!(cfg_manifest
        .apply_manifest_toml(
            "[capabilities]\nenv_file = \"./missing_manifest.env\"\n",
            Some(&tmp_dir)
        )
        .is_err());

    let mut cfg_trusted = ResolvedExecutionConfig::from_cli_args_with_ambient_cwd(
        &["--trust-directives".to_string()],
        false,
    )
    .unwrap();
    assert!(cfg_trusted
        .apply_inline_directives(
            "// @slice-env-file: ./missing_directive.env\n",
            Some(&tmp_dir)
        )
        .is_err());

    // 4. Unreadable `slice.toml` (e.g. a directory named `slice.toml`) fails closed instead of running with defaults
    let bad_manifest_proj = tmp_dir.join("bad_manifest_proj");
    std::fs::create_dir_all(bad_manifest_proj.join("slice.toml")).unwrap();
    let bad_proj_entry = bad_manifest_proj.join("index.ts");
    std::fs::write(&bad_proj_entry, "console.log('should-not-run');\n").unwrap();
    assert!(cmd_run(&[bad_proj_entry.to_string_lossy().to_string()]).is_err());

    // 5. `--trace=<directory>` fails closed when writing trace output
    let ok_script = tmp_dir.join("ok.ts");
    std::fs::write(&ok_script, "const x = 1 + 1;\n").unwrap();
    assert!(cmd_run(&[
        ok_script.to_string_lossy().to_string(),
        "--quiet".to_string(),
        "--trace".to_string(),
        sub_dir_str,
    ])
    .is_err());

    // 6. Positive verification: space-separated `--audit-egress-file <path>` registers raw tokens,
    //    arbitrary `KEY=VALUE` pairs (even without `SECRET` in key name), and Base64 `=` padded secrets!
    let secrets_file = tmp_dir.join("dlp_secrets.txt");
    std::fs::write(
        &secrets_file,
        "# comment\nsk_live_raw_secret_100\nCUSTOM_DB_CREDS=custom_db_secret_val_100\nZXhmaWwtcGFkZGVkLXNlY3JldA==\n",
    )
    .unwrap();

    let cfg_dlp = ResolvedExecutionConfig::from_cli_args_with_ambient_cwd(
        &[
            "--audit-egress-file".to_string(),
            secrets_file.to_string_lossy().to_string(),
        ],
        false,
    )
    .unwrap();

    for expected in [
        "sk_live_raw_secret_100",
        "custom_db_secret_val_100",
        "ZXhmaWwtcGFkZGVkLXNlY3JldA==",
    ] {
        assert!(
            cfg_dlp.audit_secrets.iter().any(|s| s == expected),
            "expected {:?} in audit_secrets {:?}",
            expected,
            cfg_dlp.audit_secrets
        );
        let bc = compile_source_to_bytecode(&format!("console.log({:?});", expected));
        let mut iso = SliceIsolate::spawn(
            "dlp-verify-100",
            1,
            cfg_dlp.caps.clone(),
            cfg_dlp.fuel_limit,
            cfg_dlp.mem_limit,
        );
        for sec in &cfg_dlp.audit_secrets {
            iso.add_egress_audit_secret(sec.clone());
        }
        let err = iso
            .execute_bytecode(&bc, None)
            .expect_err("expected egress DLP trap");
        assert!(
            format!("{}", err).contains("EGRESS_SECRET_LEAK_BLOCKED"),
            "unexpected err: {}",
            err
        );
    }

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_issue_101_process_exit_code_across_cli_cas_warm_and_test_runner() {
    use crate::commands::pack_and_compile::cmd_pack;
    use crate::commands::run::{cmd_run, LAST_CLI_EXIT_CODE};
    use crate::commands::test_runner::cmd_test;
    use slice_engine::run_source_isolated;
    use slice_sandbox::CapabilityToken;
    use std::sync::atomic::Ordering;

    let tmp_dir = std::env::temp_dir().join(format!("slice_issue_101_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();

    // 1. Direct `.js` and `.ts` files with `process.exitCode = 2` (previously hijacked by Tier-0!)
    let js_file = tmp_dir.join("exit_code.js");
    let ts_file = tmp_dir.join("exit_code.ts");
    std::fs::write(&js_file, "process.exitCode = 2;\n").unwrap();
    std::fs::write(
        &ts_file,
        "const code: number = 3;\nprocess.exitCode = code;\n",
    )
    .unwrap();

    cmd_run(&[js_file.to_string_lossy().to_string(), "--quiet".to_string()]).unwrap();
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 2);

    cmd_run(&[ts_file.to_string_lossy().to_string(), "--quiet".to_string()]).unwrap();
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 3);

    // 2. `-e` and `-p` modes (`-p "process.exitCode = 4"` prints & exits 4; `-p "process.exit(5)"` marks `process_exited`)
    cmd_run(&[
        "--quiet".to_string(),
        "-e".to_string(),
        "process.exitCode = 4".to_string(),
    ])
    .unwrap();
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 4);

    cmd_run(&[
        "--quiet".to_string(),
        "-p".to_string(),
        "process.exitCode = 6".to_string(),
    ])
    .unwrap();
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 6);

    let m_graceful = run_source_isolated(
        "exit-graceful-101",
        "process.exitCode = 6",
        CapabilityToken::pure_compute(),
        500_000,
        4 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(m_graceful.exit_code, Some(6));
    assert!(!m_graceful.process_exited);
    assert_eq!(m_graceful.return_value, "6");

    let m_abrupt = run_source_isolated(
        "exit-abrupt-101",
        "process.exitCode = 9; process.exit(0);",
        CapabilityToken::pure_compute(),
        500_000,
        4 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(m_abrupt.exit_code, Some(0));
    assert!(m_abrupt.process_exited);

    // Verify `BOA_LAST_EXIT_CODE` was drained unconditionally when `process.exit(0)` superseded `process.exitCode = 9`!
    let m_clean_after = run_source_isolated(
        "exit-clean-after-101",
        "const clean = 10 + 20; clean",
        CapabilityToken::pure_compute(),
        500_000,
        4 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(m_clean_after.exit_code, None);
    assert!(!m_clean_after.process_exited);

    // 3. Packed `.cas` cold AND `--warm` execution propagate `process.exitCode = 7`
    let cas_src = tmp_dir.join("cas_exit.ts");
    let cas_out = tmp_dir.join("cas_exit.cas");
    std::fs::write(&cas_src, "process.exitCode = 7;\n").unwrap();
    cmd_pack(&[
        cas_src.to_string_lossy().to_string(),
        cas_out.to_string_lossy().to_string(),
    ])
    .unwrap();

    cmd_run(&[cas_out.to_string_lossy().to_string(), "--quiet".to_string()]).unwrap();
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 7);

    cmd_run(&[
        cas_out.to_string_lossy().to_string(),
        "--warm".to_string(),
        "--quiet".to_string(),
    ])
    .unwrap();
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 7);

    // 4. `slice test` fails when a test file sets non-zero `process.exitCode`
    let test_dir = tmp_dir.join("tests_exit");
    std::fs::create_dir_all(&test_dir).unwrap();
    std::fs::write(
        test_dir.join("bad_exit.test.ts"),
        "import { test, expect } from 'slice:test';\ntest('passes assertion', () => { expect(1).toBe(1); });\nprocess.exitCode = 2;\n",
    )
    .unwrap();
    let test_res = cmd_test(&[test_dir.to_string_lossy().to_string(), "--json".to_string()]);
    assert!(
        test_res.is_err(),
        "expected slice test to fail on non-zero process.exitCode"
    );
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 2);

    LAST_CLI_EXIT_CODE.store(0, Ordering::Relaxed);
    let _ = std::fs::remove_dir_all(&tmp_dir);
}
