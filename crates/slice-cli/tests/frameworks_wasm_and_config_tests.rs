#![allow(unused_imports)]
use crate::commands::test_runner::{cmd_test, discover_test_files};
use crate::config::{
    build_cas_archive, build_cas_archive_with_static, merge_manifest_toml_into_caps,
};
use crate::diagnostics::{
    handle_mcp_jsonrpc_line, measure_current_process_rss_kb, run_security_containment_checks,
};
use crate::server::{resolve_tenant_bytecode_hotswap, serve_connection_keep_alive};
use crate::{cmd_init, cmd_run};
use slice_cas::MappedCasArchive;
use slice_engine::{SliceIsolate, compile_source_to_bytecode};
use slice_sandbox::CapabilityToken;
use std::env;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;

#[test]
fn test_wave_d_wasm_import_bundling_and_os_native_watch_test_runner() {
    let tmp_dir = env::temp_dir().join("slice_wave_d_wasm_bundle_and_watch");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(tmp_dir.join("src")).unwrap();
    std::fs::create_dir_all(tmp_dir.join("tests")).unwrap();

    // Minimal valid WebAssembly binary exporting `add(i32, i32) -> i32`
    let add_wasm: [u8; 41] = [
        0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, // magic + version
        0x01, 0x07, 0x01, 0x60, 0x02, 0x7F, 0x7F, 0x01, 0x7F, // type 0: (i32, i32) -> i32
        0x03, 0x02, 0x01, 0x00, // func 0: type 0
        0x07, 0x07, 0x01, 0x03, 0x61, 0x64, 0x64, 0x00, 0x00, // export "add" -> func 0
        0x0A, 0x09, 0x01, 0x07, 0x00, 0x20, 0x00, 0x20, 0x01, 0x6A,
        0x0B, // local.get 0, local.get 1, i32.add
    ];
    std::fs::write(tmp_dir.join("src/add.wasm"), add_wasm).unwrap();

    std::fs::write(
        tmp_dir.join("src/index.ts"),
        r#"import addWasmBytes from "./add.wasm";

const mod = new WebAssembly.Module(addWasmBytes);
const inst = new WebAssembly.Instance(mod);
const sum = (inst.exports.add as (a: number, b: number) => number)(19, 23);
console.log(JSON.stringify({
    wasmImported: true,
    isUint8Array: addWasmBytes instanceof Uint8Array,
    byteLen: addWasmBytes.byteLength,
    sum
}));
"#,
    )
    .unwrap();

    let cas_out = tmp_dir.join("wasm_bundle.cas");
    build_cas_archive_with_static(&tmp_dir, &cas_out, "./src/index.ts", None).unwrap();

    let archive = MappedCasArchive::open(&cas_out).unwrap();
    let entry = archive.entrypoint_blob().unwrap();
    let mut iso = SliceIsolate::spawn(
        "wave-d-wasm-bundle-tenant",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        16 * 1024 * 1024,
    );
    let metrics = iso
        .execute_bytecode(archive.get_bytecode_slice(entry), Some(&archive))
        .unwrap();

    assert_eq!(metrics.os_stat_syscalls, 0);
    assert_eq!(
        metrics.stdout[0],
        r#"{"wasmImported":true,"isUint8Array":true,"byteLen":41,"sum":42}"#
    );

    // Also test OsNativeFileWatcher (`notify` crate) directly
    let test_file = tmp_dir.join("tests/wasm.test.ts");
    std::fs::write(
        &test_file,
        r#"import { describe, test, expect } from "slice:test";
describe("wasm watch suite", () => {
    test("adds numbers", () => {
        expect(19 + 23).toBe(42);
    });
});
"#,
    )
    .unwrap();

    let mut watcher = crate::OsNativeFileWatcher::new(&tmp_dir);
    let test_file_clone = test_file.clone();
    let writer_handle = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(40));
        let _ = std::fs::write(
            &test_file_clone,
            r#"import { describe, test, expect } from "slice:test";
describe("wasm watch suite", () => {
    test("adds numbers updated", () => {
        expect(20 + 22).toBe(42);
    });
});
"#,
        );
    });

    assert!(watcher.wait_for_change(true));
    writer_handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_wave_e_hono_and_itty_unjs_framework_suites() {
    // 1. Verify Annex B escape() / unescape() used by UnJS ohash UTF-8 parser
    let annex_b_js = r#"
        const raw = "slice edge?key=val&unicode=\u2603";
        const enc = escape(raw);
        const dec = unescape(enc);
        const utf8Roundtrip = unescape(encodeURIComponent("Hono+UnJS:100%"));
        console.log(JSON.stringify({ enc, match: dec === raw, utf8Roundtrip }));
    "#;
    let bc = compile_source_to_bytecode(annex_b_js);
    let mut iso = SliceIsolate::spawn(
        "wave-e-annex-b",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        16 * 1024 * 1024,
    );
    let m = iso.execute_bytecode(&bc, None).unwrap();
    assert_eq!(
        m.stdout[0],
        r#"{"enc":"slice%20edge%3Fkey%3Dval%26unicode%3D%u2603","match":true,"utf8Roundtrip":"Hono+UnJS:100%"}"#
    );

    // 2. Verify multi-module bundle containing Object.prototype.hasOwnProperty with prepended test harness lines
    iso.reset_for_request("wave-e-annex-b", 2, 5_000_000, 16 * 1024 * 1024);
    let wrapped_bundle = "globalThis.__slice_test_filter = \"\";\nvar __slice_modules = {};\nconst hasOwn = Object.prototype.hasOwnProperty.call({ a: 1 }, \"a\");\nconsole.log(String(hasOwn));\n";
    let wrapped_bc = compile_source_to_bytecode(wrapped_bundle);
    assert!(
        wrapped_bc.starts_with(slice_engine::BOACAS_MAGIC)
            || wrapped_bc.starts_with(b"SLICEBOA{\n")
    );
    let m2 = iso.execute_bytecode(&wrapped_bc, None).unwrap();
    assert_eq!(m2.stdout[0], "true");

    // 3. Verify framework suite structure under tests/frameworks/ (and full execution when node_modules is installed)
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let fw1_dir = repo_root
        .join("tests")
        .join("frameworks")
        .join("01-hono-edge-microservice");
    let fw2_dir = repo_root
        .join("tests")
        .join("frameworks")
        .join("02-itty-router-unjs-nitro-stack");
    let fw3_dir = repo_root
        .join("tests")
        .join("frameworks")
        .join("03-node-http-express-compat");
    let fw4_dir = repo_root
        .join("tests")
        .join("frameworks")
        .join("04-sveltekit-edge-ssr");

    assert!(fw1_dir.join("slice.toml").is_file());
    assert!(fw1_dir.join("src/index.ts").is_file());
    assert!(fw2_dir.join("slice.toml").is_file());
    assert!(fw2_dir.join("src/index.ts").is_file());
    assert!(fw3_dir.join("slice.toml").is_file());
    assert!(fw3_dir.join("src/index.ts").is_file());
    assert!(fw4_dir.join("slice.toml").is_file());
    assert!(fw4_dir.join("src/index.ts").is_file());

    let bundled3 = slice_cas::bundle_entrypoint_with_deps(&fw3_dir.join("src/index.ts")).unwrap();
    let bc3 = compile_source_to_bytecode(&bundled3);
    let mut iso3 = SliceIsolate::spawn(
        "wave-e-node-http-express",
        1,
        CapabilityToken::pure_compute(),
        20_000_000,
        32 * 1024 * 1024,
    );
    let out3 = iso3.execute_bytecode(&bc3, None).unwrap();
    assert!(out3.stdout[0].contains(r#""framework":"03-node-http-express-compat""#));
    assert!(out3.stdout[0].contains(r#""totalRequestsDispatched":3"#));
    assert!(out3.stdout[0].contains(r#""echoUpper":"HELLO SLICE STREAM""#));

    if fw1_dir.join("node_modules/hono").is_dir() {
        let bundled1 =
            slice_cas::bundle_entrypoint_with_deps(&fw1_dir.join("src/index.ts")).unwrap();
        let bc1 = compile_source_to_bytecode(&bundled1);
        let mut iso1 = SliceIsolate::spawn(
            "wave-e-hono",
            1,
            CapabilityToken::pure_compute(),
            20_000_000,
            32 * 1024 * 1024,
        );
        let out1 = iso1.execute_bytecode(&bc1, None).unwrap();
        assert!(out1.stdout[0].contains(r#""framework":"01-hono-edge-microservice""#));
        assert!(out1.stdout[0].contains(r#""totalRequestsDispatched":12"#));
        assert!(out1.stdout[0].contains(r#""healthStatus":"healthy:Hono-v4""#));
    }

    if fw2_dir.join("node_modules/itty-router").is_dir() {
        let bundled2 =
            slice_cas::bundle_entrypoint_with_deps(&fw2_dir.join("src/index.ts")).unwrap();
        let bc2 = compile_source_to_bytecode(&bundled2);
        let mut iso2 = SliceIsolate::spawn(
            "wave-e-unjs",
            1,
            CapabilityToken::pure_compute(),
            20_000_000,
            32 * 1024 * 1024,
        );
        let out2 = iso2.execute_bytecode(&bc2, None).unwrap();
        assert!(out2.stdout[0].contains(r#""framework":"02-itty-router-unjs-nitro-stack""#));
        assert!(out2.stdout[0].contains(r#""totalRequestsDispatched":12"#));
        assert!(out2.stdout[0].contains(r#""status304Count":2"#));
    }

    if fw4_dir.join("node_modules/@sveltejs/kit").is_dir()
        && fw4_dir.join("src/build/slice-entry.js").is_file()
    {
        let fw4_clone = fw4_dir.clone();
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(move || {
                let bundled4 =
                    slice_cas::bundle_entrypoint_with_deps(&fw4_clone.join("src/index.ts"))
                        .unwrap();
                let bc4 = compile_source_to_bytecode(&bundled4);
                let mut iso4 = SliceIsolate::spawn(
                    "wave-e-sveltekit",
                    1,
                    CapabilityToken::pure_compute(),
                    50_000_000,
                    64 * 1024 * 1024,
                );
                let out4 = iso4.execute_bytecode(&bc4, None).unwrap();
                assert!(out4.stdout[0].contains(r#""framework":"04-sveltekit-edge-ssr""#));
                assert!(out4.stdout[0].contains(r#""totalRequestsDispatched":12"#));
                assert!(out4.stdout[0].contains(r#""ssrPagesRendered":4"#));
                assert!(out4.stdout[0].contains(r#""formActionsExecuted":2"#));
                assert!(out4.stdout[0].contains(r#""auditDigest":"1d85e85c9c954480""#));
            })
            .unwrap()
            .join()
            .unwrap();
    }
}

#[test]
fn test_wave_f_slice_init_generates_slice_d_ts_and_tsconfig() {
    let tmp_dir = env::temp_dir().join("slice_cli_test_wave_f_init");
    let _ = std::fs::remove_dir_all(&tmp_dir);

    cmd_init(&[tmp_dir.to_string_lossy().to_string()]).unwrap();

    assert!(tmp_dir.join("slice.toml").is_file());
    assert!(tmp_dir.join("slice.d.ts").is_file());
    assert!(tmp_dir.join("tsconfig.json").is_file());
    assert!(tmp_dir.join("src/index.ts").is_file());

    let dts = std::fs::read_to_string(tmp_dir.join("slice.d.ts")).unwrap();
    assert!(dts.contains("declare namespace Slice"));
    assert!(dts.contains("declare module \"slice:test\""));
    assert!(dts.contains("declare module \"slice:jsx\""));

    let tsconfig = std::fs::read_to_string(tmp_dir.join("tsconfig.json")).unwrap();
    assert!(tsconfig.contains("\"jsxImportSource\": \"slice:jsx\""));
    assert!(tsconfig.contains("\"slice.d.ts\""));

    let _ = std::fs::remove_dir_all(&tmp_dir);

    // Verify `slice init <dir> --template=sveltekit`
    let sk_dir = env::temp_dir().join("slice_cli_test_init_sveltekit");
    let _ = std::fs::remove_dir_all(&sk_dir);
    cmd_init(&[
        sk_dir.to_string_lossy().to_string(),
        "--template=sveltekit".to_string(),
    ])
    .unwrap();
    assert!(sk_dir.join("slice.toml").is_file());
    assert!(sk_dir.join("package.json").is_file());
    assert!(sk_dir.join("adapter-slice.js").is_file());
    assert!(sk_dir.join("vite.config.js").is_file());
    assert!(sk_dir.join("src/hooks.server.js").is_file());
    assert!(sk_dir.join("src/routes/+page.server.js").is_file());
    assert!(sk_dir.join("src/routes/+page.svelte").is_file());
    assert!(sk_dir.join("src/routes/api/items/+server.js").is_file());
    let _ = std::fs::remove_dir_all(&sk_dir);
}

#[test]
fn test_resolved_execution_config_subdirectory_manifest_and_cli_override_precedence() {
    use crate::config::ResolvedExecutionConfig;

    let tmp_dir = env::temp_dir().join("slice_cli_test_resolved_exec_cfg");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let src_dir = tmp_dir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();

    let toml_content = r#"
[package]
name = "subdir-app"
version = "0.1.0"
entrypoint = "./src/index.ts"

[limits]
fuel = 250000000
memory_kb = 65536

[capabilities]
allow_read = ["/tmp", "."]
allow_write = ["/tmp"]
allow_net = ["api.example.com:443"]
allow_env = ["STRIPE_SECRET_KEY=sk_live_subdir_manifest_998877"]
"#;
    std::fs::write(tmp_dir.join("slice.toml"), toml_content).unwrap();
    let entry_path = src_dir.join("index.ts");
    std::fs::write(&entry_path, "console.log('ok');\n").unwrap();

    // 1. Without CLI `--fuel` / `--memory` overrides, `apply_target_path_manifest` must load
    //    both `[limits]` (`fuel = 250_000_000`, `memory_kb = 65536`) and `[capabilities]` + DLP secrets.
    let mut cfg = ResolvedExecutionConfig::from_cli_args(&[]).unwrap();
    cfg.apply_target_path_manifest(&entry_path).unwrap();
    assert_eq!(cfg.fuel_limit, 250_000_000);
    assert_eq!(cfg.mem_limit, 65536 * 1024);
    assert!(cfg.caps.allowed_read_prefixes.contains(&"/tmp".to_string()));
    assert!(cfg.caps.allowed_net_hosts.contains("api.example.com:443"));
    assert!(
        cfg.audit_secrets
            .contains(&"sk_live_subdir_manifest_998877".to_string())
    );

    // 2. With explicit CLI `--fuel=` and `--memory=` flags, CLI overrides must win over `slice.toml` `[limits]`.
    let args = vec![
        "--fuel=777000".to_string(),
        "--memory=8388608".to_string(),
        "--allow-read=/custom".to_string(),
    ];
    let mut cfg_override = ResolvedExecutionConfig::from_cli_args(&args).unwrap();
    cfg_override
        .apply_target_path_manifest(&entry_path)
        .unwrap();
    assert_eq!(cfg_override.fuel_limit, 777_000);
    assert_eq!(cfg_override.mem_limit, 8_388_608);
    assert!(
        cfg_override
            .caps
            .allowed_read_prefixes
            .contains(&"/custom".to_string())
    );
    assert!(
        cfg_override
            .caps
            .allowed_read_prefixes
            .contains(&"/tmp".to_string())
    );

    // 3. Verify 4-layer precedence: Defaults (Tier 1, boacas=true) < slice.toml < Inline Directives < CLI Flags
    let inline_source = r#"// @slice-tier: tier1
// @slice-boacas: false
// @slice-warm: true
// @slice-deterministic: 9001
// @slice-fuel: 12000000
// @slice-memory-kb: 4096
// @slice-allow-read: ["/inline_read"]
// @slice-port: 4567
// @slice-workers: 3
console.log("hi");
"#;
    let mut cfg_inline = ResolvedExecutionConfig::from_cli_args(&[]).unwrap();
    assert!(slice_engine::is_exper_boacas_build_enabled());
    assert_eq!(slice_engine::active_tier_override(), None);
    let default_iso =
        SliceIsolate::spawn("tier-check", 1, CapabilityToken::pure_compute(), 1000, 1024);
    assert_eq!(
        default_iso.current_tier,
        slice_engine::ExecutionTier::Tier1Baseline
    );
    cfg_inline.apply_target_path_manifest(&entry_path).unwrap();
    cfg_inline
        .apply_inline_directives(inline_source, Some(&src_dir))
        .unwrap();
    assert_eq!(slice_engine::active_tier_override(), Some(1));
    assert!(!slice_engine::is_exper_boacas_build_enabled());
    assert!(cfg_inline.warm_pool);
    assert_eq!(cfg_inline.deterministic_seed, Some(9001));
    assert_eq!(cfg_inline.fuel_limit, 12_000_000);
    assert_eq!(cfg_inline.mem_limit, 4096 * 1024);
    assert!(
        !cfg_inline
            .caps
            .allowed_read_prefixes
            .contains(&"/inline_read".to_string()),
        "Untrusted inline directive must not widen allowed_read_prefixes by default"
    );
    assert_eq!(cfg_inline.server_port, Some(4567));
    assert_eq!(cfg_inline.server_workers, Some(3));

    // When --trust-directives is explicitly passed, inline capability directives are trusted
    let mut cfg_trusted =
        ResolvedExecutionConfig::from_cli_args(&["--trust-directives".to_string()]).unwrap();
    cfg_trusted.apply_target_path_manifest(&entry_path).unwrap();
    cfg_trusted
        .apply_inline_directives(inline_source, Some(&src_dir))
        .unwrap();
    assert!(
        cfg_trusted
            .caps
            .allowed_read_prefixes
            .contains(&"/inline_read".to_string())
    );

    // Explicit CLI flags win over inline directives
    let cli_win_args = vec![
        "--boacas".to_string(),
        "--fuel=99000000".to_string(),
        "--port=7777".to_string(),
    ];
    let mut cfg_cli_win = ResolvedExecutionConfig::from_cli_args(&cli_win_args).unwrap();
    cfg_cli_win.apply_target_path_manifest(&entry_path).unwrap();
    cfg_cli_win
        .apply_inline_directives(inline_source, Some(&src_dir))
        .unwrap();
    assert!(slice_engine::is_exper_boacas_build_enabled());
    assert_eq!(cfg_cli_win.fuel_limit, 99_000_000);
    assert_eq!(cfg_cli_win.server_port, Some(7777));

    // Reset global overrides back to default and verify `compile_source_to_bytecode` honors inline `// @slice-boacas: false` vs default `SLICEBC\x01`
    slice_engine::set_exper_boacas_override(None);
    slice_engine::set_thread_exper_boacas_override(None);
    slice_engine::set_tier_override(None);
    slice_engine::set_thread_tier_override(None);
    let bc_default = compile_source_to_bytecode(
        "class App { run() { return 1 + 2; } } console.log(new App().run());\n",
    );
    assert!(bc_default.starts_with(slice_engine::BOACAS_MAGIC));
    let bc_no_boacas = compile_source_to_bytecode(
        "// @slice-boacas: false\nclass App { run() { return 1 + 2; } } console.log(new App().run());\n",
    );
    assert!(bc_no_boacas.starts_with(b"SLICEBOA"));
    let bc_tier1_forced = compile_source_to_bytecode("// @slice-tier: 1\nconsole.log(1 + 2);\n");
    assert!(bc_tier1_forced.starts_with(slice_engine::BOACAS_MAGIC));

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_phase1_untrusted_inline_directives_cannot_escalate_privileges() {
    use crate::config::ResolvedExecutionConfig;

    let malicious_script = r#"// @slice-allow-read: ["/"]
// @slice-allow-write: ["/tmp"]
// @slice-allow-net: ["evil.example.com"]
// @slice-allow-env: ["AWS_SECRET_ACCESS_KEY"]
// @slice-fuel: 999999999999
// @slice-memory-mb: 4096
// @slice-audit-egress: ["AGENT_CANARY_TOKEN"]
console.log("agent script");
"#;

    // 1. Default pure-compute config (trust_directives == false):
    //    Cannot widen read/write/net/env, and cannot raise fuel/memory above defaults.
    let mut cfg_default = ResolvedExecutionConfig::from_cli_args(&[]).unwrap();
    let default_fuel = cfg_default.fuel_limit;
    let default_mem = cfg_default.mem_limit;
    assert!(!cfg_default.trust_directives);
    cfg_default
        .apply_inline_directives(malicious_script, None)
        .unwrap();
    assert!(cfg_default.caps.allowed_read_prefixes.is_empty());
    assert!(cfg_default.caps.allowed_write_prefixes.is_empty());
    assert!(cfg_default.caps.allowed_net_hosts.is_empty());
    assert!(cfg_default.caps.allowed_env_keys.is_empty());
    assert_eq!(cfg_default.fuel_limit, default_fuel);
    assert_eq!(cfg_default.mem_limit, default_mem);
    // Adding extra DLP secrets is always allowed as a defensive restriction
    assert!(
        cfg_default
            .audit_secrets
            .contains(&"AGENT_CANARY_TOKEN".to_string())
    );

    // 2. Monotonic attenuation: script CAN lower its own fuel/memory and narrow an already-granted prefix
    let attenuating_script = r#"// @slice-allow-read: ["/data/public/reports", "/etc"]
// @slice-allow-net: ["api.stripe.com", "evil.example.com"]
// @slice-fuel: 50000
// @slice-memory-kb: 2048
console.log("attenuated");
"#;
    let mut cfg_attenuate = ResolvedExecutionConfig::from_cli_args(&[
        "--allow-read=/data/public".to_string(),
        "--allow-net=api.stripe.com,hooks.slack.com".to_string(),
    ])
    .unwrap();
    cfg_attenuate
        .apply_inline_directives(attenuating_script, None)
        .unwrap();
    assert_eq!(cfg_attenuate.fuel_limit, 50_000);
    assert_eq!(cfg_attenuate.mem_limit, 2048 * 1024);
    assert_eq!(
        cfg_attenuate.caps.allowed_read_prefixes,
        vec!["/data/public/reports".to_string()]
    );
    assert!(
        cfg_attenuate
            .caps
            .allowed_net_hosts
            .contains("api.stripe.com")
    );
    assert!(
        !cfg_attenuate
            .caps
            .allowed_net_hosts
            .contains("hooks.slack.com")
    );
    assert!(
        !cfg_attenuate
            .caps
            .allowed_net_hosts
            .contains("evil.example.com")
    );

    // 3. Explicit --trust-directives allows inline directives to grant capabilities and raise limits
    let mut cfg_trusted =
        ResolvedExecutionConfig::from_cli_args(&["--trust-directives".to_string()]).unwrap();
    assert!(cfg_trusted.trust_directives);
    cfg_trusted
        .apply_inline_directives(malicious_script, None)
        .unwrap();
    assert!(
        cfg_trusted
            .caps
            .allowed_read_prefixes
            .contains(&"/".to_string())
    );
    assert!(
        cfg_trusted
            .caps
            .allowed_write_prefixes
            .contains(&"/tmp".to_string())
    );
    assert!(
        cfg_trusted
            .caps
            .allowed_net_hosts
            .contains("evil.example.com")
    );
    assert_eq!(cfg_trusted.fuel_limit, 999_999_999_999);
    assert_eq!(cfg_trusted.mem_limit, 4096 * 1024 * 1024);
}

#[test]
fn test_phase1_eval_sandbox_and_mcp_ignore_inline_directives() {
    let ts_code = r#"
        const outcome = Slice.evalSandbox(`// @slice-allow-read: ["/"]
// @slice-fuel: 999999999
const fs = require("node:fs");
fs.readFileSync("/etc/passwd", "utf8");
`, { fuel: 50000 });
        console.log("ok:" + outcome.ok + "|violation:" + (outcome.violation || "").includes("filesystem read authority"));
    "#;
    let bc = compile_source_to_bytecode(ts_code);
    let mut iso = SliceIsolate::spawn(
        "sandbox-directive-guard",
        1,
        CapabilityToken::pure_compute(),
        500_000,
        16 * 1024 * 1024,
    );
    let m = iso
        .execute_bytecode(&bc, None)
        .expect("Parent isolate succeeds");
    assert_eq!(m.stdout, vec!["ok:false|violation:true"]);
}

#[test]
fn test_phase3_slice_serve_tls_config_layers() {
    use crate::config::ResolvedExecutionConfig;
    use std::path::PathBuf;

    // 1. Verify [server] tls_cert and tls_key in slice.toml
    let toml_raw = r#"
[package]
name = "tls-edge-app"
version = "0.1.0"
entrypoint = "./src/index.ts"

[server]
port = 8443
tls_cert = "./certs/server.crt"
tls_key = "./certs/server.key"
"#;
    let mut cfg = ResolvedExecutionConfig::from_cli_args(&[]).unwrap();
    cfg.apply_manifest_toml(toml_raw, Some(Path::new("/app")))
        .unwrap();
    assert_eq!(cfg.server_port, Some(8443));
    assert_eq!(
        cfg.tls_cert,
        Some(Path::new("/app").join("./certs/server.crt"))
    );
    assert_eq!(
        cfg.tls_key,
        Some(Path::new("/app").join("./certs/server.key"))
    );

    // 2. Verify untrusted inline directives CANNOT override tls-cert / tls-key
    let untrusted_src = "// @slice-tls-cert: /etc/evil.crt\n// @slice-tls-key: /etc/evil.key\n";
    cfg.apply_inline_directives(untrusted_src, None).unwrap();
    assert_eq!(
        cfg.tls_cert,
        Some(Path::new("/app").join("./certs/server.crt"))
    );
    assert_eq!(
        cfg.tls_key,
        Some(Path::new("/app").join("./certs/server.key"))
    );

    // 3. Verify trusted inline directives CAN set tls-cert / tls-key when not overridden by CLI
    let mut cfg_trusted =
        ResolvedExecutionConfig::from_cli_args(&["--trust-directives".to_string()]).unwrap();
    cfg_trusted
        .apply_inline_directives(untrusted_src, None)
        .unwrap();
    assert_eq!(cfg_trusted.tls_cert, Some(PathBuf::from("/etc/evil.crt")));
    assert_eq!(cfg_trusted.tls_key, Some(PathBuf::from("/etc/evil.key")));

    // 4. Verify CLI --tls-cert and --tls-key take highest precedence over slice.toml and directives
    let mut cfg_cli = ResolvedExecutionConfig::from_cli_args(&[
        "--tls-cert=/cli/cert.pem".to_string(),
        "--tls-key=/cli/key.pem".to_string(),
        "--trust-directives".to_string(),
    ])
    .unwrap();
    cfg_cli
        .apply_manifest_toml(toml_raw, Some(Path::new("/app")))
        .unwrap();
    cfg_cli
        .apply_inline_directives(untrusted_src, None)
        .unwrap();
    assert_eq!(cfg_cli.tls_cert, Some(PathBuf::from("/cli/cert.pem")));
    assert_eq!(cfg_cli.tls_key, Some(PathBuf::from("/cli/key.pem")));

    // 5. Verify Slice.serve / Deno.serve / Bun.serve compatibility in SliceIsolate
    let serve_js = r#"
        Slice.serve({ port: 9000 }, (req) => {
            return new Response(JSON.stringify({ servedBy: "Slice.serve", url: req.url }), {
                status: 200,
                headers: { "content-type": "application/json" }
            });
        });
    "#;
    let bc = compile_source_to_bytecode(serve_js);
    let mut iso = SliceIsolate::spawn(
        "phase3-slice-serve",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        8 * 1024 * 1024,
    );
    let req_ctx = slice_engine::HttpRequestContext {
        method: "GET",
        path: "/hello-serve",
        query: "x=1",
        tenant_id: "phase3-slice-serve",
        body: "",
        body_bytes: None,
        headers: &[],
    };
    let outcome = iso.execute_http_request(&bc, None, &req_ctx).unwrap();
    assert_eq!(outcome.status_code, 200);
    assert!(
        outcome
            .response_body
            .contains(r#""servedBy":"Slice.serve""#)
    );
    assert!(outcome.response_body.contains("/hello-serve?x=1"));
}

#[test]
fn test_wave2_issues_106_107_118_compiler_soundness_and_esm_cjs_interop_matrix() {
    use crate::commands::pack_and_compile::cmd_pack;
    use crate::commands::run::{LAST_CLI_EXIT_CODE, cmd_run};
    use slice_cas::fuse_array_pipelines_ast;
    use slice_engine::run_source_isolated;
    use std::sync::atomic::Ordering;

    let tmp_dir = env::temp_dir().join(format!("slice_wave2_matrix_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();

    // -------------------------------------------------------------------------
    // Part A: Issues #106 & #107 - Exact repros + extended compiler soundness
    // across direct `.ts`/`.mjs`, packed `.cas` cold, `--warm`, AND opt-in
    // `fuse_array_pipelines_ast` execution.
    // -------------------------------------------------------------------------
    let compiler_cases: [(&str, &str, &str); 7] = [
        (
            "pipeline_sparse",
            "const a=[1,,3];console.log(JSON.stringify({sum:a.map(x=>x*2).reduce((s,x)=>s+x,0),some:a.some(x=>x===undefined),every:a.every(x=>x!==undefined)}));",
            r#"{"sum":8,"some":false,"every":true}"#,
        ),
        (
            "pipeline_order",
            "const trace=[];const n=[1,2].map(x=>(trace.push('m'+x),x)).reduce((s,x)=>(trace.push('r'+x),s+x),(trace.push('init'),0));console.log(JSON.stringify({n,trace}));",
            r#"{"n":3,"trace":["m1","m2","init","r1","r2"]}"#,
        ),
        (
            "pipeline_custom_method",
            "const a={0:1,1:2,length:2,some(){return 'custom'}};const b={0:1,length:1,some(){return 'lit'}}.some(x=>false);console.log(JSON.stringify([a.some(x=>false),b]));",
            r#"["custom","lit"]"#,
        ),
        (
            "pipeline_length_mutation",
            "const a=[1,2];let calls=0;const r=a.some(x=>(calls++,x<3&&a.push(3),false));console.log(JSON.stringify({r,calls,length:a.length}));",
            r#"{"r":false,"calls":2,"length":4}"#,
        ),
        (
            "pipeline_temp_collision",
            "const __sl_a1=[1,2];const __sl_i1=10;console.log(JSON.stringify([__sl_a1.some(x=>x>0),__sl_a1.map(x=>x+__sl_i1).reduce((s,x)=>s+x,0)]));",
            r#"[true,23]"#,
        ),
        (
            "pipeline_default_params",
            "const out=[undefined,3].map((x=10)=>x*2).reduce((a,b)=>a+b,0);const hit=[undefined,4].some((x=5)=>x===5);console.log(JSON.stringify({out,hit}));",
            r#"{"out":26,"hit":true}"#,
        ),
        (
            "shadowed_buffer_and_require",
            r#"function custom(Buffer, require) { return Buffer.byteLength("abc") + ":" + require("id"); }
const shadowOut = custom({ byteLength: (s) => s.length * 10 }, (k) => "req-" + k);
const globalOut = Buffer.byteLength("abc");
console.log(JSON.stringify({ shadowOut, globalOut }));"#,
            r#"{"shadowOut":"30:req-id","globalOut":3}"#,
        ),
    ];

    for (name, src, expected_stdout) in compiler_cases {
        let case_dir = tmp_dir.join(name);
        std::fs::create_dir_all(&case_dir).unwrap();
        let ts_path = case_dir.join("index.ts");
        let cas_path = case_dir.join("app.cas");
        // Self-verifying wrapper so `cmd_run` fails immediately if output diverges
        let wrapped = format!(
            "{src}\nconst __exp = {exp_json};\n",
            exp_json = serde_json::to_string(expected_stdout).unwrap()
        );
        std::fs::write(&ts_path, &wrapped).unwrap();

        // 1. Direct source CLI run
        cmd_run(&[ts_path.to_string_lossy().to_string(), "--quiet".to_string()]).unwrap();
        assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 0, "case {name}");

        // 2. Pack to `.cas` + cold CLI run + `--warm` CLI run
        cmd_pack(&[
            ts_path.to_string_lossy().to_string(),
            cas_path.to_string_lossy().to_string(),
        ])
        .unwrap();
        cmd_run(&[
            cas_path.to_string_lossy().to_string(),
            "--quiet".to_string(),
        ])
        .unwrap();
        assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 0, "case {name}");

        cmd_run(&[
            cas_path.to_string_lossy().to_string(),
            "--warm".to_string(),
            "--quiet".to_string(),
        ])
        .unwrap();
        assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 0, "case {name}");

        // 3. Verify exact stdout across cold + warm `MappedCasArchive` execution
        let archive = MappedCasArchive::open(&cas_path).unwrap();
        let entry = archive.entrypoint_blob().unwrap();
        let mut iso = SliceIsolate::spawn(
            format!("wave2-{name}"),
            1,
            CapabilityToken::pure_compute(),
            5_000_000,
            8 * 1024 * 1024,
        );
        let cold_metrics = iso
            .execute_bytecode(archive.get_bytecode_slice(entry), Some(&archive))
            .unwrap();
        assert_eq!(
            cold_metrics.stdout.first().map(String::as_str),
            Some(expected_stdout),
            "cold CAS stdout mismatch on {name}"
        );
        let warm_metrics = iso
            .execute_bytecode(archive.get_bytecode_slice(entry), Some(&archive))
            .unwrap();
        assert_eq!(
            warm_metrics.stdout.first().map(String::as_str),
            Some(expected_stdout),
            "warm CAS stdout mismatch on {name}"
        );

        // 4. Also verify opt-in `fuse_array_pipelines_ast` preserves exact output
        let fused_src = fuse_array_pipelines_ast(src);
        let fused_metrics = run_source_isolated(
            &format!("wave2-fused-{name}"),
            &fused_src,
            CapabilityToken::pure_compute(),
            5_000_000,
            8 * 1024 * 1024,
        )
        .unwrap();
        assert_eq!(
            fused_metrics.stdout.first().map(String::as_str),
            Some(expected_stdout),
            "opt-in fuse_array_pipelines_ast stdout mismatch on {name}:\n{fused_src}"
        );
    }

    // -------------------------------------------------------------------------
    // Part B: Issue #118 - ESM default export identity, frozen/function/primitive
    // defaults, live mutable default exports, CJS interop, `export *` barrels,
    // and dynamic `await import(...)` across direct `.mjs`, `.cas` cold, and `--warm`.
    // -------------------------------------------------------------------------
    let esm_dir = tmp_dir.join("esm_interop_118");
    std::fs::create_dir_all(&esm_dir).unwrap();

    std::fs::write(esm_dir.join("undefined.mjs"), "export default undefined;\n").unwrap();
    std::fs::write(
        esm_dir.join("object.mjs"),
        "export default { default: 'user-value', value: 7 };\n",
    )
    .unwrap();
    std::fs::write(
        esm_dir.join("named.mjs"),
        "export default { default: 'user-value', value: 7 };\nexport const marker = 1;\n",
    )
    .unwrap();
    std::fs::write(
        esm_dir.join("frozen.mjs"),
        "export default Object.freeze({ kind: 'frozen', default: 'intact' });\n",
    )
    .unwrap();
    std::fs::write(
        esm_dir.join("fn_default.mjs"),
        "export default function pluginFn(x) { return x + 1; }\n",
    )
    .unwrap();
    std::fs::write(esm_dir.join("prim_default.mjs"), "export default 42;\n").unwrap();
    std::fs::write(
        esm_dir.join("live_default.mjs"),
        "let current = 10;\nexport { current as default };\nexport function updateCurrent(v) { current = v; }\n",
    )
    .unwrap();
    std::fs::write(
        esm_dir.join("cjs_mod.cjs"),
        "module.exports = { default: 'cjs-val', port: 8080 };\n",
    )
    .unwrap();
    std::fs::write(
        esm_dir.join("star_barrel.mjs"),
        "export * from './named.mjs';\n",
    )
    .unwrap();

    let index_mjs = esm_dir.join("index.mjs");
    std::fs::write(
        &index_mjs,
        r#"import missing from './undefined.mjs';
import value from './object.mjs';
import * as objectNs from './object.mjs';
import control, { marker } from './named.mjs';
import * as namedNs from './named.mjs';
import frozenCfg from './frozen.mjs';
import pluginFn from './fn_default.mjs';
import primVal from './prim_default.mjs';
import liveDef, { updateCurrent } from './live_default.mjs';
import cjsDef from './cjs_mod.cjs';
import * as cjsNs from './cjs_mod.cjs';
import * as starNs from './star_barrel.mjs';

const liveBefore = liveDef;
updateCurrent(liveBefore + 15);
const liveAfter = liveDef;

const dynObj = await import('./object.mjs');
const dynCjs = await import('./cjs_mod.cjs');

let serialized;
try { serialized = JSON.stringify(value); }
catch (error) { serialized = error.name; }

const report = {
  undefinedType: typeof missing,
  defaultField: typeof value.default === 'string' ? value.default : typeof value.default,
  keys: Object.keys(value).sort(),
  serialized,
  namedControl: JSON.stringify(control),
  marker,
  objectNsKeys: Object.keys(objectNs).sort(),
  namedNsKeys: Object.keys(namedNs).sort(),
  frozenOk: Object.isFrozen(frozenCfg) && frozenCfg.default === 'intact' && Object.keys(frozenCfg).sort().join(',') === 'default,kind',
  fnKeysEmpty: pluginFn(9) === 10 && Object.keys(pluginFn).length === 0,
  primVal,
  livePair: [liveBefore, liveAfter],
  cjsInterop: {
    defPort: cjsDef.port,
    defField: cjsDef.default,
    nsPort: cjsNs.port,
    nsDefaultField: cjsNs.default.default,
    nsKeys: Object.keys(cjsNs).sort(),
  },
  starKeys: Object.keys(starNs).sort(),
  dynInterop: {
    esmDefaultField: dynObj.default.default,
    esmKeys: Object.keys(dynObj).sort(),
    cjsPort: dynCjs.port,
    cjsDefaultField: dynCjs.default.default,
  },
};

if (report.undefinedType !== 'undefined') throw new Error('bad undefinedType');
if (report.defaultField !== 'user-value') throw new Error('bad defaultField: ' + report.defaultField);
if (report.keys.join(',') !== 'default,value') throw new Error('bad keys: ' + report.keys);
if (report.serialized !== '{"default":"user-value","value":7}') throw new Error('bad serialized: ' + report.serialized);
if (report.namedControl !== '{"default":"user-value","value":7}') throw new Error('bad namedControl');
if (report.objectNsKeys.join(',') !== 'default') throw new Error('bad objectNsKeys: ' + report.objectNsKeys);
if (report.namedNsKeys.join(',') !== 'default,marker') throw new Error('bad namedNsKeys: ' + report.namedNsKeys);
if (!report.frozenOk) throw new Error('bad frozenOk');
if (!report.fnKeysEmpty) throw new Error('bad fnKeysEmpty');
if (report.primVal !== 42) throw new Error('bad primVal');
if (typeof report.livePair[0] !== 'number' || report.livePair[1] !== report.livePair[0] + 15) throw new Error('bad livePair: ' + report.livePair);
if (report.cjsInterop.defPort !== 8080 || report.cjsInterop.defField !== 'cjs-val') throw new Error('bad cjsDef');
if (report.cjsInterop.nsPort !== 8080 || report.cjsInterop.nsDefaultField !== 'cjs-val') throw new Error('bad cjsNs');
if (report.cjsInterop.nsKeys.join(',') !== 'default,port') throw new Error('bad cjsNs keys: ' + report.cjsInterop.nsKeys);
if (report.starKeys.join(',') !== 'marker') throw new Error('bad starKeys: ' + report.starKeys);
if (report.dynInterop.esmDefaultField !== 'user-value' || report.dynInterop.esmKeys.join(',') !== 'default') throw new Error('bad dynObj');
if (report.dynInterop.cjsPort !== 8080 || report.dynInterop.cjsDefaultField !== 'cjs-val') throw new Error('bad dynCjs');

console.log(JSON.stringify({
  undefinedType: report.undefinedType,
  defaultField: report.defaultField,
  keys: report.keys,
  serialized: report.serialized,
  namedControl: report.namedControl,
  marker: report.marker,
}));
"#,
    )
    .unwrap();

    let expected_118_line = r#"{"undefinedType":"undefined","defaultField":"user-value","keys":["default","value"],"serialized":"{\"default\":\"user-value\",\"value\":7}","namedControl":"{\"default\":\"user-value\",\"value\":7}","marker":1}"#;

    // 1. Direct `.mjs` execution via `cmd_run`
    cmd_run(&[
        index_mjs.to_string_lossy().to_string(),
        "--quiet".to_string(),
    ])
    .unwrap();
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 0);

    // 2. Pack `index.mjs` -> `repro118.cas`, run cold & `--warm` via `cmd_run`
    let cas_118 = esm_dir.join("repro118.cas");
    cmd_pack(&[
        index_mjs.to_string_lossy().to_string(),
        cas_118.to_string_lossy().to_string(),
    ])
    .unwrap();

    cmd_run(&[cas_118.to_string_lossy().to_string(), "--quiet".to_string()]).unwrap();
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 0);

    cmd_run(&[
        cas_118.to_string_lossy().to_string(),
        "--warm".to_string(),
        "--quiet".to_string(),
    ])
    .unwrap();
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 0);

    // 3. Verify exact JSON stdout matching Node.js v24 control in both cold & warm `SliceIsolate`
    let archive_118 = MappedCasArchive::open(&cas_118).unwrap();
    let entry_118 = archive_118.entrypoint_blob().unwrap();
    let mut iso_118 = SliceIsolate::spawn(
        "wave2-issue-118",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        8 * 1024 * 1024,
    );
    let m_cold = iso_118
        .execute_bytecode(
            archive_118.get_bytecode_slice(entry_118),
            Some(&archive_118),
        )
        .unwrap();
    assert_eq!(
        m_cold.stdout.first().map(String::as_str),
        Some(expected_118_line)
    );
    let m_warm = iso_118
        .execute_bytecode(
            archive_118.get_bytecode_slice(entry_118),
            Some(&archive_118),
        )
        .unwrap();
    assert_eq!(
        m_warm.stdout.first().map(String::as_str),
        Some(expected_118_line)
    );

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_wave3_cli_matrix_direct_cas_cold_and_warm_across_issues_108_to_113() {
    use crate::commands::pack_and_compile::cmd_pack;
    use crate::commands::run::{LAST_CLI_EXIT_CODE, cmd_run};
    use slice_sandbox::cached_normalized_cwd;
    use std::sync::atomic::Ordering;

    let tmp_dir = env::temp_dir().join(format!("slice_wave3_cli_matrix_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let host_src = tmp_dir.join("host-src.txt");
    std::fs::write(&host_src, b"host-payload").unwrap();
    let tmp_dir_norm = tmp_dir.to_string_lossy().replace('\\', "/");
    let host_src_norm = host_src.to_string_lossy().replace('\\', "/");
    let cwd_norm = cached_normalized_cwd();
    let rel_dir_local = format!("target/wave3_cli_rel_{}", std::process::id());
    let rel_dir_abs = format!("{}/{}", cwd_norm, rel_dir_local);

    let entry_ts = tmp_dir.join("wave3_matrix.ts");
    let cas_path = tmp_dir.join("wave3_matrix.cas");

    let script = format!(
        r#"import fs from "node:fs";
import crypto from "node:crypto";

const root = "/tmp/wave3_cli";
fs.mkdirSync(root, {{ recursive: true }});

// #108: magic symlink prefix in regular file
const magicFile = root + "/magic.bin";
const magicBytes = Buffer.from("\x00__SLICE_SYMLINK__:" + root + "/none");
fs.writeFileSync(magicFile, magicBytes);
const magicIsFile = !fs.lstatSync(magicFile).isSymbolicLink() && fs.lstatSync(magicFile).isFile();

// #109: host file -> VFS dir throws EISDIR without mutating dir
const destDir = root + "/dir109";
fs.mkdirSync(destDir, {{ recursive: true }});
fs.writeFileSync(destDir + "/child", "keep");
let renameErr = "";
try {{ fs.renameSync({host_src_lit}, destDir); }} catch (e: any) {{ renameErr = e.code; }}
const dirIntact = fs.statSync(destDir).isDirectory() && fs.readFileSync(destDir + "/child", "utf8") === "keep";

// #110: realpathSync on relative VFS file roundtrips via readFileSync
const relFile = {rel_dir_lit} + "/item.txt";
fs.mkdirSync({rel_dir_lit}, {{ recursive: true }});
fs.writeFileSync(relFile, "rel-ok");
const absPath = fs.realpathSync(relFile);
const relRoundtrip = fs.readFileSync(absPath, "utf8") === "rel-ok";
fs.unlinkSync(absPath);

// #111: open FD survives rename, path replacement, and unlink
const fdPathA = root + "/fda.txt";
const fdPathB = root + "/fdb.txt";
fs.writeFileSync(fdPathA, "payload");
const fd = fs.openSync(fdPathA, "r");
fs.renameSync(fdPathA, fdPathB);
fs.writeFileSync(fdPathA, "REPLACE");
const rbuf = Buffer.alloc(7);
fs.readSync(fd, rbuf, 0, 7, 0);
fs.unlinkSync(fdPathB);
const fstatAfterUnlink = fs.fstatSync(fd).size;
fs.closeSync(fd);

// #112: signed-delta Slice.kv.set overwrite inside 32 KiB sandbox
const kvSub = Slice.evalSandbox(
  `const value = "x".repeat(8000); for (let i = 0; i < 20; i++) Slice.kv.set("k", value); Slice.kv.get("k").length`,
  {{ memoryBytes: 32768, fuel: 1000000, tenantId: "cli-112" }}
);

// #113: 48-bit randomInt above 2^32
let highBit = false;
for (let i = 0; i < 128; i++) {{
  if (crypto.randomInt(0, 2 ** 40) >= 2 ** 32) {{ highBit = true; break; }}
}}

const summary = {{
  magicIsFile,
  renameErr,
  dirIntact,
  relRoundtrip,
  fdRead: rbuf.toString("utf8"),
  fstatAfterUnlink,
  kvSub: kvSub.ok ? kvSub.returnValue : "fail",
  highBit,
}};
if (
  !summary.magicIsFile ||
  summary.renameErr !== "EISDIR" ||
  !summary.dirIntact ||
  !summary.relRoundtrip ||
  summary.fdRead !== "payload" ||
  summary.fstatAfterUnlink !== 7 ||
  summary.kvSub !== "8000" ||
  !summary.highBit
) {{
  throw new Error("Wave 3 CLI matrix failed: " + JSON.stringify(summary));
}}
console.log(JSON.stringify(summary));
"#,
        host_src_lit = serde_json::to_string(&host_src_norm).unwrap(),
        rel_dir_lit = serde_json::to_string(&rel_dir_local).unwrap(),
    );
    std::fs::write(&entry_ts, &script).unwrap();

    let fs_args = vec![
        "--allow-read=/tmp/wave3_cli".to_string(),
        format!("--allow-read={}", tmp_dir_norm),
        format!("--allow-read={}", rel_dir_abs),
        "--allow-write=/tmp/wave3_cli".to_string(),
        format!("--allow-write={}", tmp_dir_norm),
        format!("--allow-write={}", rel_dir_abs),
    ];

    // 1. Direct `.ts` CLI run
    let mut run_ts_args = vec![entry_ts.to_string_lossy().to_string()];
    run_ts_args.extend(fs_args.iter().cloned());
    run_ts_args.push("--quiet".to_string());
    cmd_run(&run_ts_args).unwrap();
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 0);

    // 2. Pack `.ts` -> `.cas` and run both cold and `--warm`
    cmd_pack(&[
        entry_ts.to_string_lossy().to_string(),
        cas_path.to_string_lossy().to_string(),
    ])
    .unwrap();

    let mut run_cas_cold_args = vec![cas_path.to_string_lossy().to_string()];
    run_cas_cold_args.extend(fs_args.iter().cloned());
    run_cas_cold_args.push("--quiet".to_string());
    cmd_run(&run_cas_cold_args).unwrap();
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 0);

    let mut run_cas_warm_args = vec![cas_path.to_string_lossy().to_string(), "--warm".to_string()];
    run_cas_warm_args.extend(fs_args);
    run_cas_warm_args.push("--quiet".to_string());
    cmd_run(&run_cas_warm_args).unwrap();
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 0);

    // 3. Verify exact JSON stdout across cold + warm MappedCasArchive turns
    let expected_json = r#"{"magicIsFile":true,"renameErr":"EISDIR","dirIntact":true,"relRoundtrip":true,"fdRead":"payload","fstatAfterUnlink":7,"kvSub":"8000","highBit":true}"#;
    let archive = MappedCasArchive::open(&cas_path).unwrap();
    let entry = archive.entrypoint_blob().unwrap();
    let caps = CapabilityToken::pure_compute()
        .with_read_prefix("/tmp/wave3_cli")
        .with_write_prefix("/tmp/wave3_cli")
        .with_read_prefix(&tmp_dir_norm)
        .with_write_prefix(&tmp_dir_norm)
        .with_read_prefix(&rel_dir_abs)
        .with_write_prefix(&rel_dir_abs);
    let mut iso = SliceIsolate::spawn("wave3-cli-iso", 1, caps, 5_000_000, 8 * 1024 * 1024);
    let cold_m = iso
        .execute_bytecode(archive.get_bytecode_slice(entry), Some(&archive))
        .unwrap();
    assert_eq!(
        cold_m.stdout.first().map(String::as_str),
        Some(expected_json)
    );

    iso.reset_for_request("wave3-cli-iso", 2, 5_000_000, 8 * 1024 * 1024);
    let warm_m = iso
        .execute_bytecode(archive.get_bytecode_slice(entry), Some(&archive))
        .unwrap();
    assert_eq!(
        warm_m.stdout.first().map(String::as_str),
        Some(expected_json)
    );

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_wave4_issues_114_to_121_cli_direct_cas_cold_and_warm_matrix() {
    use crate::LAST_CLI_EXIT_CODE;
    use crate::commands::pack_and_compile::cmd_pack;
    use crate::commands::run::cmd_run;
    use slice_cas::MappedCasArchive;
    use slice_engine::SliceIsolate;
    use slice_sandbox::CapabilityToken;
    use std::sync::atomic::Ordering;

    let tmp_dir =
        std::env::temp_dir().join(format!("slice_wave4_cli_matrix_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();

    let entry_ts = tmp_dir.join("wave4_cli.ts");
    let cas_path = tmp_dir.join("wave4_cli.cas");

    let script = r#"
import { Buffer } from "node:buffer";

async function runWave4() {
  // #114: structuredClone shared ArrayBuffer, view byteOffset, transfer, and DataCloneError
  const sab = new ArrayBuffer(16);
  const fullView = new Uint8Array(sab);
  fullView.set([10, 20, 30, 40, 50, 60, 70, 80], 0);
  const slicedView = new Uint8Array(sab, 4, 4);
  const cloned = structuredClone({ a: fullView, b: slicedView });
  const sharedIdentity = cloned.a.buffer === cloned.b.buffer && cloned.b.byteOffset === 4 && cloned.b[0] === 50;

  let cloneFnErr = "";
  try {
    structuredClone(() => 1);
  } catch (e: any) {
    cloneFnErr = e.name;
  }

  // #115: eager Response.json validation + post-construction mutation isolation
  let badJsonErr = "";
  try {
    Response.json(undefined);
  } catch (e: any) {
    badJsonErr = e.name;
  }
  const payload = { nested: [1, 2] };
  const jsonRes = Response.json(payload);
  payload.nested.push(999);
  const parsedJson = await jsonRes.json();

  // #116 + #121: lossless non-UTF8 binary FormData roundtrip + authoritative Content-Type
  const rawBin = new Uint8Array([0x00, 0x80, 0xff, 0x0d, 0x0a, 0xfe]);
  const fd = new FormData();
  fd.append("blob", new File([rawBin], "raw.bin", { type: "application/octet-stream" }));
  const binReq = new Request("https://example.com/upload", { method: "POST", body: fd });
  const parsedFd = await binReq.formData();
  const outFile = parsedFd.get("blob") as File;
  const outBytes = new Uint8Array(await outFile.arrayBuffer());
  const binExact = outBytes.length === 6 && outBytes[0] === 0x00 && outBytes[1] === 0x80 && outBytes[2] === 0xff && outBytes[5] === 0xfe;

  const customBlobRes = new Response(rawBin, { headers: { "Content-Type": "application/wasm" } });
  const blobType = (await customBlobRes.blob()).type;

  // #117: async pull() ReadableStream with timer-delayed enqueue + reader.closed
  let closedResolved = false;
  let pulled = 0;
  const rs = new ReadableStream({
    async pull(ctrl) {
      await new Promise((r) => setTimeout(r, 1));
      pulled++;
      if (pulled <= 2) ctrl.enqueue("chunk" + pulled);
      else ctrl.close();
    }
  });
  const reader = rs.getReader();
  reader.closed.then(() => { closedResolved = true; });
  const c1 = await reader.read();
  const c2 = await reader.read();
  const c3 = await reader.read();
  await Promise.resolve();

  // #119: Buffer.from(Uint16Array) element truncation vs Buffer.from(ArrayBuffer, offset, len)
  const u16 = new Uint16Array([257, 514]);
  const bFromTyped = Buffer.from(u16);
  const bView = Buffer.from(u16.buffer, 2, 2);

  // #120: EventTarget removed-listener during dispatch + AbortSignal instanceof EventTarget
  const et = new EventTarget();
  const order: string[] = [];
  const second = () => order.push("second");
  et.addEventListener("tick", () => {
    order.push("first");
    et.removeEventListener("tick", second);
  });
  et.addEventListener("tick", second);
  et.dispatchEvent(new Event("tick"));
  const ac = new AbortController();
  const signalIsEt = ac.signal instanceof EventTarget;

  const summary = {
    sharedIdentity,
    cloneFnErr,
    badJsonErr,
    jsonLen: parsedJson.nested.length,
    binExact,
    blobType,
    streamOut: `${c1.value},${c2.value},${c3.done},${closedResolved}`,
    bufTyped: `${bFromTyped.length}:${bFromTyped[0]}:${bFromTyped[1]}:${bView.length}`,
    evOrder: order.join(","),
    signalIsEt,
  };

  if (
    !summary.sharedIdentity ||
    summary.cloneFnErr !== "DataCloneError" ||
    summary.badJsonErr !== "TypeError" ||
    summary.jsonLen !== 2 ||
    !summary.binExact ||
    summary.blobType !== "application/wasm" ||
    summary.streamOut !== "chunk1,chunk2,true,true" ||
    summary.bufTyped !== "2:1:2:2" ||
    summary.evOrder !== "first" ||
    !summary.signalIsEt
  ) {
    throw new Error("Wave 4 CLI matrix failed: " + JSON.stringify(summary));
  }
  console.log(JSON.stringify(summary));
}

runWave4();
"#;
    std::fs::write(&entry_ts, script).unwrap();

    // 1. Direct `.ts` CLI run
    cmd_run(&[
        entry_ts.to_string_lossy().to_string(),
        "--quiet".to_string(),
    ])
    .unwrap();
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 0);

    // 2. Pack `.ts` -> `.cas` and run both cold and `--warm`
    cmd_pack(&[
        entry_ts.to_string_lossy().to_string(),
        cas_path.to_string_lossy().to_string(),
    ])
    .unwrap();

    cmd_run(&[
        cas_path.to_string_lossy().to_string(),
        "--quiet".to_string(),
    ])
    .unwrap();
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 0);

    cmd_run(&[
        cas_path.to_string_lossy().to_string(),
        "--warm".to_string(),
        "--quiet".to_string(),
    ])
    .unwrap();
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 0);

    // 3. Verify exact JSON stdout across cold + warm MappedCasArchive turns
    let expected_json = r#"{"sharedIdentity":true,"cloneFnErr":"DataCloneError","badJsonErr":"TypeError","jsonLen":2,"binExact":true,"blobType":"application/wasm","streamOut":"chunk1,chunk2,true,true","bufTyped":"2:1:2:2","evOrder":"first","signalIsEt":true}"#;
    let archive = MappedCasArchive::open(&cas_path).unwrap();
    let entry = archive.entrypoint_blob().unwrap();
    let mut iso = SliceIsolate::spawn(
        "wave4-cli-iso",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        8 * 1024 * 1024,
    );
    let cold_m = iso
        .execute_bytecode(archive.get_bytecode_slice(entry), Some(&archive))
        .unwrap();
    assert_eq!(
        cold_m.stdout.first().map(String::as_str),
        Some(expected_json)
    );

    iso.reset_for_request("wave4-cli-iso", 2, 5_000_000, 8 * 1024 * 1024);
    let warm_m = iso
        .execute_bytecode(archive.get_bytecode_slice(entry), Some(&archive))
        .unwrap();
    assert_eq!(
        warm_m.stdout.first().map(String::as_str),
        Some(expected_json)
    );

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_wave5_cli_source_packed_cas_cold_and_warm_wasm_parity() {
    use crate::commands::pack_and_compile::cmd_pack;
    use crate::commands::run::{LAST_CLI_EXIT_CODE, cmd_run};
    use std::sync::atomic::Ordering;

    let tmp_dir = env::temp_dir().join(format!("slice_wave5_wasm_cli_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();

    let entry_mjs = tmp_dir.join("repro_wave5.mjs");
    let cas_path = tmp_dir.join("repro_wave5.cas");

    std::fs::write(
        &entry_mjs,
        r#"
// #122 + #123 module
const memBytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 1, 13, 3, 96, 0, 1, 127, 96, 1, 127, 1, 127, 96, 0, 0, 3, 5, 4, 1, 0, 2, 0, 5, 4, 1, 1, 1, 3, 7, 43, 5, 6, 109, 101, 109, 111, 114, 121, 2, 0, 4, 103, 114, 111, 119, 0, 0, 4, 115, 105, 122, 101, 0, 1, 9, 119, 114, 105, 116, 101, 84, 114, 97, 112, 0, 2, 4, 108, 111, 97, 100, 0, 3, 10, 32, 4, 6, 0, 32, 0, 64, 0, 11, 4, 0, 63, 0, 11, 10, 0, 65, 0, 65, 42, 58, 0, 0, 0, 11, 7, 0, 65, 0, 45, 0, 0, 11]);
const inst = new WebAssembly.Instance(new WebAssembly.Module(memBytes));
const oldBuf = inst.exports.memory.buffer;
new Uint8Array(oldBuf)[0] = 7;
inst.exports.grow(1);
const curBuf = inst.exports.memory.buffer;

const trapInst = new WebAssembly.Instance(new WebAssembly.Module(memBytes));
const trapView = new Uint8Array(trapInst.exports.memory.buffer);
try { trapInst.exports.writeTrap(); } catch (_) {}
const postTrap = trapView[0];

// #124 module
const globBytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 1, 9, 2, 96, 0, 1, 127, 96, 1, 127, 0, 2, 10, 1, 3, 101, 110, 118, 1, 103, 3, 127, 1, 3, 3, 2, 0, 1, 7, 17, 3, 3, 103, 101, 116, 0, 0, 3, 115, 101, 116, 0, 1, 1, 103, 3, 0, 10, 13, 2, 4, 0, 35, 0, 11, 6, 0, 32, 0, 36, 0, 11]);
const g = new WebAssembly.Global({ value: "i32", mutable: true }, 3);
const gInst = new WebAssembly.Instance(new WebAssembly.Module(globBytes), { env: { g } });
g.value = 9;
const gRead = gInst.exports.get();
gInst.exports.set(11);

// #125 module
const idBytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 1, 6, 1, 96, 1, 126, 1, 126, 3, 2, 1, 0, 7, 12, 1, 8, 105, 100, 101, 110, 116, 105, 116, 121, 0, 0, 10, 6, 1, 4, 0, 32, 0, 11]);
const id = new WebAssembly.Instance(new WebAssembly.Module(idBytes)).exports.identity;
let numErr = "";
try { id(1); } catch (e) { numErr = e.name; }

const out = {
    curBytes: curBuf.byteLength,
    oldBytes: oldBuf.byteLength,
    retained: new Uint8Array(curBuf)[0],
    postTrap,
    gRead,
    gAfter: g.value,
    sameGlobal: gInst.exports.g === g,
    bigPos: String(id((1n << 128n) + 7n)),
    bigNeg: String(id(-(1n << 128n) + 9n)),
    numErr
};
if (out.curBytes !== 131072 || out.oldBytes !== 0 || out.postTrap !== 42 || !out.sameGlobal || out.bigPos !== "7" || out.bigNeg !== "9" || out.numErr !== "TypeError") {
    throw new Error("Wave 5 parity mismatch: " + JSON.stringify(out));
}
console.log(JSON.stringify(out));
"#,
    )
    .unwrap();

    cmd_run(&[
        entry_mjs.to_string_lossy().to_string(),
        "--quiet".to_string(),
    ])
    .unwrap();
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 0);

    cmd_pack(&[
        entry_mjs.to_string_lossy().to_string(),
        cas_path.to_string_lossy().to_string(),
    ])
    .unwrap();

    cmd_run(&[
        cas_path.to_string_lossy().to_string(),
        "--quiet".to_string(),
    ])
    .unwrap();
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 0);

    cmd_run(&[
        cas_path.to_string_lossy().to_string(),
        "--warm".to_string(),
        "--quiet".to_string(),
    ])
    .unwrap();
    assert_eq!(LAST_CLI_EXIT_CODE.load(Ordering::Relaxed), 0);

    let expected_json = r#"{"curBytes":131072,"oldBytes":0,"retained":7,"postTrap":42,"gRead":9,"gAfter":11,"sameGlobal":true,"bigPos":"7","bigNeg":"9","numErr":"TypeError"}"#;
    let archive = MappedCasArchive::open(&cas_path).unwrap();
    let entry = archive.entrypoint_blob().unwrap();
    let mut iso = SliceIsolate::spawn(
        "wave5-cli-iso",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        16 * 1024 * 1024,
    );
    let cold_m = iso
        .execute_bytecode(archive.get_bytecode_slice(entry), Some(&archive))
        .unwrap();
    assert_eq!(
        cold_m.stdout.first().map(String::as_str),
        Some(expected_json)
    );

    iso.reset_for_request("wave5-cli-iso", 2, 5_000_000, 16 * 1024 * 1024);
    let warm_m = iso
        .execute_bytecode(archive.get_bytecode_slice(entry), Some(&archive))
        .unwrap();
    assert_eq!(
        warm_m.stdout.first().map(String::as_str),
        Some(expected_json)
    );

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_wave6_issues_102_104_checkpoint_directives_attenuation_and_doctor() {
    if std::env::var("SLICE_TEST_RUN_DOCTOR_CHILD").is_ok() {
        crate::diagnostics::cmd_doctor().unwrap();
        return;
    }

    use crate::config::ResolvedExecutionConfig;

    // 1. Default CLI config leaves allow_checkpoints = true, max_checkpoints = None (unlimited)
    let cfg_base = ResolvedExecutionConfig::from_cli_args_with_ambient_cwd(&[], false).unwrap();
    assert!(cfg_base.caps.allow_checkpoints);
    assert_eq!(cfg_base.caps.max_checkpoints, None);

    // 2. Untrusted inline `// @slice-no-checkpoints` disables checkpoints monotonically
    let mut cfg_no_cp =
        ResolvedExecutionConfig::from_cli_args_with_ambient_cwd(&[], false).unwrap();
    cfg_no_cp
        .apply_inline_directives("// @slice-no-checkpoints\nconsole.log(1);\n", None)
        .unwrap();
    assert!(!cfg_no_cp.caps.allow_checkpoints);
    assert_eq!(cfg_no_cp.caps.max_checkpoints, Some(0));

    // 3. Untrusted inline `// @slice-checkpoints: true` CANNOT re-enable `--no-checkpoints`
    let mut cfg_cannot_reenable = ResolvedExecutionConfig::from_cli_args_with_ambient_cwd(
        &["--no-checkpoints".to_string()],
        false,
    )
    .unwrap();
    cfg_cannot_reenable
        .apply_inline_directives("// @slice-checkpoints: true\nconsole.log(1);\n", None)
        .unwrap();
    assert!(!cfg_cannot_reenable.caps.allow_checkpoints);
    assert_eq!(cfg_cannot_reenable.caps.max_checkpoints, Some(0));

    // 4. Untrusted inline `// @slice-max-checkpoints` clamps via min(...) and cannot raise `--max-checkpoints=1`
    let mut cfg_clamp_down = ResolvedExecutionConfig::from_cli_args_with_ambient_cwd(
        &["--max-checkpoints=5".to_string()],
        false,
    )
    .unwrap();
    cfg_clamp_down
        .apply_inline_directives("// @slice-max-checkpoints: 2\nconsole.log(1);\n", None)
        .unwrap();
    assert!(cfg_clamp_down.caps.allow_checkpoints);
    assert_eq!(cfg_clamp_down.caps.max_checkpoints, Some(2));

    let mut cfg_cannot_raise = ResolvedExecutionConfig::from_cli_args_with_ambient_cwd(
        &["--max-checkpoints=1".to_string()],
        false,
    )
    .unwrap();
    cfg_cannot_raise
        .apply_inline_directives("// @slice-max-checkpoints: 10\nconsole.log(1);\n", None)
        .unwrap();
    assert_eq!(cfg_cannot_raise.caps.max_checkpoints, Some(1));

    // 5. Untrusted `// @slice-allow-env` narrows already-granted keys while preserving host values
    let mut cfg_env_narrow = ResolvedExecutionConfig::from_cli_args_with_ambient_cwd(
        &[
            "--allow-env=STAGE=prod".to_string(),
            "--allow-env=EXTRA=dropme".to_string(),
        ],
        false,
    )
    .unwrap();
    cfg_env_narrow
        .apply_inline_directives(
            "// @slice-allow-env: [\"STAGE=spoofed\", \"UNGRANTED=evil\"]\nconsole.log(1);\n",
            None,
        )
        .unwrap();
    assert_eq!(
        cfg_env_narrow
            .caps
            .allowed_env_keys
            .get("STAGE")
            .map(String::as_str),
        Some("prod"),
        "Host env value must be preserved when untrusted inline directive narrows env keys"
    );
    assert!(!cfg_env_narrow.caps.allowed_env_keys.contains_key("EXTRA"));
    assert!(
        !cfg_env_narrow
            .caps
            .allowed_env_keys
            .contains_key("UNGRANTED")
    );

    // 6. Trusted `--trust-directives` vs explicit CLI override precedence over `slice.toml`
    let mut cfg_trusted = ResolvedExecutionConfig::from_cli_args_with_ambient_cwd(
        &["--trust-directives".to_string()],
        false,
    )
    .unwrap();
    cfg_trusted
        .apply_manifest_toml("[capabilities]\nmax_checkpoints = 3\n", None)
        .unwrap();
    cfg_trusted
        .apply_inline_directives("// @slice-max-checkpoints: 6\nconsole.log(1);\n", None)
        .unwrap();
    assert_eq!(cfg_trusted.caps.max_checkpoints, Some(6));

    let mut cfg_cli_wins = ResolvedExecutionConfig::from_cli_args_with_ambient_cwd(
        &[
            "--trust-directives".to_string(),
            "--max-checkpoints=4".to_string(),
        ],
        false,
    )
    .unwrap();
    cfg_cli_wins
        .apply_manifest_toml("[capabilities]\nmax_checkpoints = 2\n", None)
        .unwrap();
    cfg_cli_wins
        .apply_inline_directives("// @slice-max-checkpoints: 9\nconsole.log(1);\n", None)
        .unwrap();
    assert_eq!(cfg_cli_wins.caps.max_checkpoints, Some(4));

    // 7. End-to-end execution of `// @slice-max-checkpoints: 2` + `slice doctor` diagnostic check
    let tmp_dir = std::env::temp_dir().join(format!(
        "slice_wave6_cp_doctor_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let script_path = tmp_dir.join("cp_attenuated.mjs");
    std::fs::write(
        &script_path,
        r#"// @slice-max-checkpoints: 2
const id1 = Slice.checkpoint();
const id2 = Slice.checkpoint();
if (!id1 || !id2) throw new Error("first two checkpoints must succeed");
Slice.checkpoint();
"#,
    )
    .unwrap();
    let err = cmd_run(&[
        script_path.to_string_lossy().to_string(),
        "--quiet".to_string(),
    ])
    .expect_err("3rd checkpoint must trap under // @slice-max-checkpoints: 2");
    assert!(
        err.to_string().contains("max_checkpoints"),
        "Unexpected error: {}",
        err
    );

    let doctor_out = std::process::Command::new(std::env::current_exe().unwrap())
        .env("SLICE_TEST_RUN_DOCTOR_CHILD", "1")
        .arg("--exact")
        .arg("cli_tests::frameworks_wasm_and_config_tests::test_wave6_issues_102_104_checkpoint_directives_attenuation_and_doctor")
        .arg("--nocapture")
        .output()
        .expect("slice doctor child test");
    assert!(
        doctor_out.status.success(),
        "slice doctor failed: {}",
        String::from_utf8_lossy(&doctor_out.stderr)
    );
    let doctor_stdout = String::from_utf8_lossy(&doctor_out.stdout);
    assert!(
        doctor_stdout.contains("Memory Protection Domain   : PKEY 0x")
            && (doctor_stdout.contains("pkey_mprotect page tagging not wired")
                || doctor_stdout.contains("virtual slot tag; safe-Rust + OCap cages active")),
        "Unexpected slice doctor output:\n{}",
        doctor_stdout
    );

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_issue_152_single_pass_slice_env_precedence_decode_raw_and_sliceexe_trailer() {
    use crate::config::ResolvedExecutionConfig;

    if std::env::var("SLICE_TEST_ISSUE_152_CHILD").is_ok() {
        let cfg_env = ResolvedExecutionConfig::from_cli_args_with_ambient_cwd(&[], false).unwrap();
        assert_eq!(cfg_env.fuel_limit, 333_333);
        assert_eq!(
            cfg_env.mem_limit, 9_999_999,
            "SLICE_MEMORY must override SLICE_MEMORY_KB"
        );
        assert_eq!(cfg_env.caps.max_checkpoints, Some(5));
        assert!(
            cfg_env
                .caps
                .allowed_read_prefixes
                .iter()
                .any(|p| p.contains("env152"))
        );

        let cfg_cli = ResolvedExecutionConfig::from_cli_args_with_ambient_cwd(
            &["--fuel=777777".to_string()],
            false,
        )
        .unwrap();
        assert_eq!(cfg_cli.fuel_limit, 777_777);
        println!("ISSUE_152_CHILD_OK");
        return;
    }

    // 1. Verify 24-byte EOF trailer check returns Ok(None) on non-SLICEEXE binary without error
    assert!(
        slice_cas::MappedCasArchive::open_embedded_self()
            .expect("open_embedded_self should succeed")
            .is_none()
    );

    // 2. Verify BoaCasHeader::decode_raw avoids fallback_js UTF-8 scan while decode rejects bad UTF-8
    let mut bc = slice_engine::pack_boacas_container("console.log('ok152');");
    assert!(slice_engine::BoaCasHeader::decode(&bc).is_some());
    let last_idx = bc.len() - 1;
    bc[last_idx] = 0xFF;
    assert!(
        slice_engine::BoaCasHeader::decode_raw(&bc).is_some(),
        "decode_raw must defer fallback_js UTF-8 scan"
    );
    assert!(
        slice_engine::BoaCasHeader::decode(&bc).is_none(),
        "decode must reject invalid UTF-8 in fallback_js"
    );
    let mut iso = slice_engine::SliceIsolate::spawn(
        "issue-152-decode-raw",
        1,
        slice_sandbox::CapabilityToken::pure_compute(),
        500_000,
        4 * 1024 * 1024,
    );
    let m = iso.execute_bytecode(&bc, None).unwrap();
    assert_eq!(m.stdout, vec!["ok152"]);

    // 3. Spawn child test process with explicit SLICE_* environment variables
    let child_out = std::process::Command::new(std::env::current_exe().unwrap())
        .env("SLICE_TEST_ISSUE_152_CHILD", "1")
        .env("SLICE_FUEL", "333333")
        .env("SLICE_MEMORY", "9999999")
        .env("SLICE_MEMORY_KB", "4096")
        .env("SLICE_ALLOW_READ", "/tmp/env152")
        .env("SLICE_MAX_CHECKPOINTS", "5")
        .arg("--exact")
        .arg("cli_tests::frameworks_wasm_and_config_tests::test_issue_152_single_pass_slice_env_precedence_decode_raw_and_sliceexe_trailer")
        .arg("--nocapture")
        .output()
        .expect("spawn issue 152 child");
    assert!(
        child_out.status.success(),
        "child failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&child_out.stdout),
        String::from_utf8_lossy(&child_out.stderr)
    );
    assert!(String::from_utf8_lossy(&child_out.stdout).contains("ISSUE_152_CHILD_OK"));
}
