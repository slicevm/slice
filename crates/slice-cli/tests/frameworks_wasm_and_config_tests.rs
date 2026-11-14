#![allow(unused_imports)]
use crate::config_and_watch::{
    build_cas_archive, build_cas_archive_with_static, merge_manifest_toml_into_caps,
};
use crate::http_server::{resolve_tenant_bytecode_hotswap, serve_connection_keep_alive};
use crate::mcp_and_repl::{
    handle_mcp_jsonrpc_line, measure_current_process_rss_kb, run_security_containment_checks,
};
use crate::test_runner::{cmd_test, discover_test_files};
use crate::{cmd_init, cmd_run};
use slice_cas::MappedCasArchive;
use slice_engine::{compile_source_to_bytecode, SliceIsolate};
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
    use crate::config_and_watch::ResolvedExecutionConfig;

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
    assert!(cfg
        .audit_secrets
        .contains(&"sk_live_subdir_manifest_998877".to_string()));

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
    assert!(cfg_override
        .caps
        .allowed_read_prefixes
        .contains(&"/custom".to_string()));
    assert!(cfg_override
        .caps
        .allowed_read_prefixes
        .contains(&"/tmp".to_string()));

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
    assert!(cfg_trusted
        .caps
        .allowed_read_prefixes
        .contains(&"/inline_read".to_string()));

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
    use crate::config_and_watch::ResolvedExecutionConfig;

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
    assert!(cfg_default
        .audit_secrets
        .contains(&"AGENT_CANARY_TOKEN".to_string()));

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
    assert!(cfg_attenuate
        .caps
        .allowed_net_hosts
        .contains("api.stripe.com"));
    assert!(!cfg_attenuate
        .caps
        .allowed_net_hosts
        .contains("hooks.slack.com"));
    assert!(!cfg_attenuate
        .caps
        .allowed_net_hosts
        .contains("evil.example.com"));

    // 3. Explicit --trust-directives allows inline directives to grant capabilities and raise limits
    let mut cfg_trusted =
        ResolvedExecutionConfig::from_cli_args(&["--trust-directives".to_string()]).unwrap();
    assert!(cfg_trusted.trust_directives);
    cfg_trusted
        .apply_inline_directives(malicious_script, None)
        .unwrap();
    assert!(cfg_trusted
        .caps
        .allowed_read_prefixes
        .contains(&"/".to_string()));
    assert!(cfg_trusted
        .caps
        .allowed_write_prefixes
        .contains(&"/tmp".to_string()));
    assert!(cfg_trusted
        .caps
        .allowed_net_hosts
        .contains("evil.example.com"));
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
    use crate::config_and_watch::ResolvedExecutionConfig;
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
        headers: &[],
    };
    let outcome = iso.execute_http_request(&bc, None, &req_ctx).unwrap();
    assert_eq!(outcome.status_code, 200);
    assert!(outcome
        .response_body
        .contains(r#""servedBy":"Slice.serve""#));
    assert!(outcome.response_body.contains("/hello-serve?x=1"));
}
