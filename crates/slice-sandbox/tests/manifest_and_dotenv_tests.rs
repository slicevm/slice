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
    assert!(tenant
        .verify_fs_write("/tmp/shards/../../etc/passwd")
        .is_err());
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
        vec!["/app".to_string(), "./data".to_string()]
    );
    assert_eq!(
        m.capabilities.allowed_write_prefixes,
        vec!["/tmp".to_string()]
    );
    assert!(m
        .capabilities
        .allowed_net_hosts
        .contains("api.example.com:443"));
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
