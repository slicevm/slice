use crate::*;

#[test]
fn test_slice_manifest_toml_parsing_and_fs_write_ocap() {
    let toml_str = r#"
[package]
name = "edge-gateway"
version = "0.2.0"
entrypoint = "./src/main.ts"

[limits]
fuel = 25_000_000
memory_kb = 8192

[capabilities]
allow_read = ["/data/read", "/config"]
allow_write = ["/tmp/shards"]
allow_net = ["api.stripe.com:443"]

[capabilities.env]
NODE_ENV = "production"
"#;
    let manifest = SliceManifest::parse_toml(toml_str);
    assert_eq!(manifest.name, "edge-gateway");
    assert_eq!(manifest.version, "0.2.0");
    assert_eq!(manifest.entrypoint, "./src/main.ts");
    assert_eq!(manifest.fuel_limit, 25_000_000);
    assert_eq!(manifest.memory_limit_kb, 8192);

    let tenant = TenantCompartment::new(
        "tenant-manifest",
        2,
        manifest.capabilities,
        manifest.fuel_limit,
        manifest.memory_limit_kb * 1024,
    );
    assert!(tenant.verify_fs_read("/data/read/items.json").is_ok());
    assert!(tenant.verify_fs_write("/tmp/shards/shard_00.jsonl").is_ok());
    assert!(tenant.verify_fs_write("/etc/crontab").is_err());
    assert!(
        tenant
            .verify_fs_write("/tmp/shards/../../etc/passwd")
            .is_err()
    );
    assert_eq!(tenant.verify_env_read("NODE_ENV"), Ok("production"));
}

#[test]
fn test_tag_f4_dotenv_parser_and_auto_dlp_secret_classification() {
    let dotenv_raw = r#"
# Production Vault Configuration
SERVICE_REGION=us-east-1
PUBLIC_KEY_ID="pk_live_public_identifier_123"
export STRIPE_SECRET_KEY="sk_live_f4_stripe_vault_998877"
OPENAI_API_TOKEN=tok_openai_f4_secret_665544 # inline comment
DB_PASSWORD='pg_super_secret_pw_f4'
"#;
    let (pairs, secrets) = parse_dotenv_content(dotenv_raw);
    assert_eq!(pairs.len(), 5);
    assert_eq!(
        pairs[0],
        ("SERVICE_REGION".to_string(), "us-east-1".to_string())
    );
    assert_eq!(
        pairs[1],
        (
            "PUBLIC_KEY_ID".to_string(),
            "pk_live_public_identifier_123".to_string()
        )
    );
    assert_eq!(
        pairs[2],
        (
            "STRIPE_SECRET_KEY".to_string(),
            "sk_live_f4_stripe_vault_998877".to_string()
        )
    );
    assert_eq!(
        pairs[3],
        (
            "OPENAI_API_TOKEN".to_string(),
            "tok_openai_f4_secret_665544".to_string()
        )
    );
    assert_eq!(
        pairs[4],
        (
            "DB_PASSWORD".to_string(),
            "pg_super_secret_pw_f4".to_string()
        )
    );

    // Only the 3 sensitive keys (excluding SERVICE_REGION and PUBLIC_KEY_ID) should be auto-registered for DLP
    assert_eq!(
        secrets,
        vec![
            "sk_live_f4_stripe_vault_998877".to_string(),
            "tok_openai_f4_secret_665544".to_string(),
            "pg_super_secret_pw_f4".to_string(),
        ]
    );

    let toml_with_env_file = r#"
[package]
name = "f4-app"
entrypoint = "./src/index.ts"

[capabilities]
env_file = ".env.production"
"#;
    let m = SliceManifest::parse_toml(toml_with_env_file);
    assert_eq!(m.env_file.as_deref(), Some(".env.production"));
}

#[test]
fn test_manifest_engine_server_and_inline_directives_parsing() {
    let toml = r#"
[package]
name = "edge-worker"
version = "1.2.0"

[engine]
tier = "tier1"
boacas = true
warm = true
deterministic_seed = 42

[limits]
fuel = 25_000_000
memory_mb = 32

[capabilities]
allow_read = ["/app", "./data"]
allow_write = ["/tmp"]
allow_net = ["api.example.com:443"]
env_file = ".env.prod"
audit_egress = ["sk_live_secret_12345"]

[capabilities.env]
NODE_ENV = "production"

[server]
port = 8080
workers = 4
static_dir = "./public"
"#;
    let m = SliceManifest::parse_toml(toml);
    assert_eq!(m.name, "edge-worker");
    assert_eq!(m.version, "1.2.0");
    assert_eq!(m.tier, Some(1));
    assert_eq!(m.exper_boacas, Some(true));
    assert_eq!(m.warm_pool, Some(true));
    assert_eq!(m.deterministic_seed, Some(42));
    assert_eq!(m.fuel_limit, 25_000_000);
    assert_eq!(m.memory_limit_kb, 32 * 1024);
    assert_eq!(
        m.capabilities.allowed_read_prefixes,
        vec!["/app".to_string(), "data".to_string()]
    );
    assert_eq!(
        m.capabilities.allowed_write_prefixes,
        vec!["/tmp".to_string()]
    );
    assert!(
        m.capabilities
            .allowed_net_hosts
            .contains("api.example.com:443")
    );
    assert_eq!(
        m.capabilities
            .allowed_env_keys
            .get("NODE_ENV")
            .map(|s| s.as_str()),
        Some("production")
    );
    assert_eq!(m.env_file.as_deref(), Some(".env.prod"));
    assert_eq!(m.audit_egress, vec!["sk_live_secret_12345".to_string()]);
    assert_eq!(m.server_port, Some(8080));
    assert_eq!(m.server_workers, Some(4));
    assert_eq!(m.static_dir.as_deref(), Some("./public"));

    let src = r#"#!/usr/bin/env slice
// @slice-tier: tier1
// @slice-boacas: false
// @slice-warm: true
// @slice-deterministic: 1337
// @slice-fuel: 10_000_000
// @slice-memory-kb: 8192
// @slice-allow-read: ["/tmp", "./fixtures"]
// @slice-allow-write: "/tmp"
// @slice-allow-net: ["127.0.0.1:3000"]
// @slice-allow-env: ["API_MODE=test"]
// @slice-audit-egress: ["top_secret_token"]
// @slice-port: 9090
// @slice-workers: 2
const x = 1 + 2;
"#;
    let dirs = parse_inline_directives(src);
    assert_eq!(dirs.tier, Some(Some(1)));
    assert_eq!(dirs.boacas, Some(false));
    assert_eq!(dirs.warm, Some(true));
    assert_eq!(dirs.deterministic_seed, Some(1337));
    assert_eq!(dirs.fuel_limit, Some(10_000_000));
    assert_eq!(dirs.memory_limit_bytes, Some(8192 * 1024));
    assert_eq!(
        dirs.allow_read,
        vec!["/tmp".to_string(), "./fixtures".to_string()]
    );
    assert_eq!(dirs.allow_write, vec!["/tmp".to_string()]);
    assert_eq!(dirs.allow_net, vec!["127.0.0.1:3000".to_string()]);
    assert_eq!(
        dirs.allow_env,
        vec![("API_MODE".to_string(), "test".to_string())]
    );
    assert_eq!(dirs.audit_egress, vec!["top_secret_token".to_string()]);
    assert_eq!(dirs.port, Some(9090));
    assert_eq!(dirs.workers, Some(2));
}

#[test]
fn test_inline_directives_block_comments_strict_prologue_and_code_stop() {
    // Directives inside single-line or multi-line block comments must be ignored
    let commented_out = r#"/* // @slice-allow-net: * */
/*
// @slice-allow-read: ["/"]
// @slice-allow-write: ["/"]
*/
// @slice-fuel: 777_000
const msg = `
// @slice-allow-net: evil.example.com:443
`;
// @slice-allow-exec: ["sh"]
"#;
    let dirs = parse_inline_directives(commented_out);
    assert!(
        dirs.allow_net.is_empty(),
        "block-commented or template-literal allow-net must be ignored"
    );
    assert!(dirs.allow_read.is_empty());
    assert!(dirs.allow_write.is_empty());
    assert!(dirs.allow_exec.is_empty());
    assert_eq!(dirs.fuel_limit, Some(777_000));

    // Code following a block comment on the same line must terminate directive scanning
    let code_after_comment = "/* license */ const ready = true;\n// @slice-allow-net: *\n";
    let dirs_after = parse_inline_directives(code_after_comment);
    assert!(dirs_after.allow_net.is_empty());

    // UTF-8 BOM + "use strict" prologue before header directives remains supported
    let strict_prologue = "\u{feff}\"use strict\";\n// @slice-fuel: 450_000\n// @slice-allow-net: api.example.com:443\nexport default 1;\n";
    let dirs_strict = parse_inline_directives(strict_prologue);
    assert_eq!(dirs_strict.fuel_limit, Some(450_000));
    assert_eq!(
        dirs_strict.allow_net,
        vec!["api.example.com:443".to_string()]
    );
}

#[test]
fn test_wave6_issue_102_checkpoint_inline_directives_and_unlimited_default() {
    let default_caps = crate::CapabilityToken::default();
    assert!(default_caps.allow_checkpoints);
    assert_eq!(
        default_caps.max_checkpoints, None,
        "Default CapabilityToken must allow unlimited checkpoints while slab FIFO-evicts at 64"
    );

    let bare_disable = parse_inline_directives("// @slice-no-checkpoints\nconsole.log(1);\n");
    assert_eq!(bare_disable.allow_checkpoints, Some(false));
    assert_eq!(bare_disable.max_checkpoints, Some(0));

    let bare_enable = parse_inline_directives("// @slice-checkpoints\nconsole.log(1);\n");
    assert_eq!(bare_enable.allow_checkpoints, Some(true));
    assert_eq!(bare_enable.max_checkpoints, None);

    let kv_disable = parse_inline_directives("// @slice-checkpoints: false\nconsole.log(1);\n");
    assert_eq!(kv_disable.allow_checkpoints, Some(false));
    assert_eq!(kv_disable.max_checkpoints, Some(0));

    let kv_max = parse_inline_directives(
        "// @slice-allow-checkpoints: true\n// @slice-max-checkpoints: 8\nconsole.log(1);\n",
    );
    assert_eq!(kv_max.allow_checkpoints, Some(true));
    assert_eq!(kv_max.max_checkpoints, Some(8));

    let kv_zero = parse_inline_directives("// @slice-max_checkpoints = 0\nconsole.log(1);\n");
    assert_eq!(kv_zero.allow_checkpoints, Some(false));
    assert_eq!(kv_zero.max_checkpoints, Some(0));
}

#[test]
fn test_manifest_and_dotenv_inline_comments_and_quoted_hashes() {
    let toml = r#"
[package]
name = "sveltekit-app" # inline comment on quoted string
version = "0.1.0"
entrypoint = "./build/slice-entry.js" # entry comment

[server]
static_dir = "./build/client" # static assets

[limits]
fuel = 50_000_000 # 50M instructions
memory_mb = 64

[capabilities]
allow_net = ["api.internal:443", "127.0.0.1:8080"] # multi-host array
allow_env = ["NODE_ENV", "HOOK_URL=https://example.com/#frag", "PAIR=a,b"] # trailing array comment
"#;
    let m = SliceManifest::parse_toml(toml);
    assert_eq!(m.name, "sveltekit-app");
    assert_eq!(m.entrypoint, "./build/slice-entry.js");
    assert_eq!(m.static_dir.as_deref(), Some("./build/client"));
    assert_eq!(m.fuel_limit, 50_000_000);
    assert_eq!(m.memory_limit_kb, 64 * 1024);
    assert!(
        m.capabilities
            .allowed_net_hosts
            .contains("api.internal:443")
    );
    assert!(m.capabilities.allowed_net_hosts.contains("127.0.0.1:8080"));
    assert_eq!(
        m.capabilities
            .allowed_env_keys
            .get("HOOK_URL")
            .map(|s| s.as_str()),
        Some("https://example.com/#frag")
    );
    assert_eq!(
        m.capabilities
            .allowed_env_keys
            .get("PAIR")
            .map(|s| s.as_str()),
        Some("a,b")
    );

    let dotenv = r#"
API_KEY="sec#ret_value_123" # trailing comment after double quotes
SINGLE_SECRET='lit#eral_456' # trailing comment after single quotes
PLAIN_VAL=hello_world # trailing comment after unquoted
"#;
    let (pairs, secrets) = parse_dotenv_content(dotenv);
    assert_eq!(
        pairs,
        vec![
            ("API_KEY".to_string(), "sec#ret_value_123".to_string()),
            ("SINGLE_SECRET".to_string(), "lit#eral_456".to_string()),
            ("PLAIN_VAL".to_string(), "hello_world".to_string()),
        ]
    );
    assert!(secrets.contains(&"sec#ret_value_123".to_string()));
    assert!(secrets.contains(&"lit#eral_456".to_string()));
}

#[test]
fn test_shared_parsers_json_error_auto_dlp_net_grant_and_egress_dlp_scanner() {
    // 1. parse_strict_u64 & parse_byte_size_usize
    assert_eq!(parse_strict_u64("50_000_000"), Some(50_000_000));
    assert_eq!(parse_strict_u64(" \"1_024\" "), Some(1_024));
    assert_eq!(parse_strict_u64("_100"), None);
    assert_eq!(parse_strict_u64("64MB"), None);

    assert_eq!(parse_byte_size_usize("65_536"), Some(65_536));
    assert_eq!(parse_byte_size_usize("64KB"), Some(64 * 1024));
    assert_eq!(parse_byte_size_usize("16MiB"), Some(16 * 1024 * 1024));
    assert_eq!(parse_byte_size_usize("1GB"), Some(1024 * 1024 * 1024));
    assert_eq!(parse_byte_size_usize("128B"), Some(128));
    assert_eq!(parse_byte_size_usize("bad"), None);

    let m = SliceManifest::parse_toml("[limits]\nmemory_bytes = \"32MB\"\n");
    assert_eq!(m.memory_limit_kb, 32 * 1024);
    let d = parse_inline_directives("// @slice-memory: 16MiB\nexport default 1;\n");
    assert_eq!(d.memory_limit_bytes, Some(16 * 1024 * 1024));

    // 2. Auto-DLP env entry & JSON escaping helpers
    assert_eq!(MIN_SENSITIVE_SECRET_LEN, 6);
    assert!(is_auto_dlp_env_entry("STRIPE_SECRET_KEY", "sk_live_123"));
    assert!(!is_auto_dlp_env_entry("STRIPE_SECRET_KEY", "short"));
    assert!(!is_auto_dlp_env_entry("NODE_ENV", "production"));
    assert_eq!(
        format_json_error_body("bad \"quote\"\n\\slash\x01"),
        r#"{"error":"bad \"quote\"\n\\slash\u0001"}"#
    );

    // 3. check_net_host_grant, vfs_path_matches_any_prefix, push_exec_binary, has_exec_shell_metachar
    let mut caps = CapabilityToken::pure_compute();
    caps.insert_net_host("localhost:8080");
    caps.insert_net_host("[2001:db8::1]:8443");
    caps.insert_net_host("*");
    caps.push_exec_binary("git");
    caps.push_exec_binary("git");
    assert_eq!(caps.allowed_exec_binaries, vec!["git".to_string()]);
    assert!(has_exec_shell_metachar("echo hi; id"));
    assert!(has_exec_shell_metachar("echo \\$HOME"));
    assert!(!has_exec_shell_metachar("echo hello"));

    assert_eq!(
        check_net_host_grant(&caps.allowed_net_hosts, "127.0.0.1", 8080, false),
        (false, false)
    );
    assert_eq!(
        check_net_host_grant(&caps.allowed_net_hosts, "127.0.0.1", 8080, true),
        (true, false)
    );
    assert_eq!(
        check_net_host_grant(&caps.allowed_net_hosts, "2001:db8::1", 8443, false),
        (true, false)
    );
    assert_eq!(
        check_net_host_grant(&caps.allowed_net_hosts, "api.openai.com", 443, false),
        (false, true)
    );
    assert!(vfs_path_matches_any_prefix(
        "/app/public/index.html",
        &["/app/public".to_string()]
    ));
    assert!(!vfs_path_matches_any_prefix(
        "/app/public/../secret.env",
        &["/app/public".to_string()]
    ));

    // 4. Outbound Egress DLP (raw, hex, percent-encoded, and 0/1/2-shifted Base64 + Base64URL)
    let raw_secret = "sk_live_dlp_central_secret+99/=";
    let mut expanded = Vec::new();
    expand_egress_secret_variants(raw_secret, &mut expanded);
    assert!(verify_egress_secrets_slice("t1", &expanded, "clean body").is_ok());

    for slice_ref in [&expanded[..], &[raw_secret.to_string()][..]] {
        assert!(verify_egress_secrets_slice("t1", slice_ref, raw_secret).is_err());
        assert!(
            verify_egress_secrets_slice(
                "t1",
                slice_ref,
                "736b5f6c6976655f646c705f63656e7472616c5f7365637265742b39392f3d"
            )
            .is_err()
        );
        assert!(
            verify_egress_secrets_slice("t1", slice_ref, "sk_live_dlp_central_secret%2B99%2F%3D")
                .is_err()
        );
        // 1-byte shifted Base64 (`"X" + raw_secret` = `"Xsk_live_dlp_central_secret+99/="`)
        assert!(
            verify_egress_secrets_slice(
                "t1",
                slice_ref,
                "WHNrX2xpdmVfZGxwX2NlbnRyYWxfc2VjcmV0Kzk5Lz0="
            )
            .is_err()
        );
        // 2-byte shifted Base64URL (`"XY" + raw_secret`)
        assert!(
            verify_egress_secrets_slice(
                "t1",
                slice_ref,
                "WFlza19saXZlX2RscF9jZW50cmFsX3NlY3JldCs5OS89"
            )
            .is_err()
        );
    }
}
