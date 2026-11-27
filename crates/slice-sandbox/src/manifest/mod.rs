//! Declarative Project & OCap Manifest (`slice.toml`), Inline Directives (`// @slice-...`),
//! and `.env` Auto-DLP Credential Classification.

pub mod directives;
pub mod dotenv;
pub mod parsers;

pub use directives::{InlineDirectives, parse_inline_directives};
pub use dotenv::{
    MIN_SENSITIVE_SECRET_LEN, is_auto_dlp_env_entry, is_sensitive_env_key, parse_dotenv_content,
};
pub use parsers::{
    escape_json_string_into, format_json_error_body, parse_bool_setting, parse_byte_size_usize,
    parse_strict_u64, parse_tier_setting,
};

use crate::compartment::CapabilityToken;
use parsers::{
    bracket_delta_outside_quotes, parse_toml_string_array, parse_u64_ignoring_underscores,
    strip_toml_inline_comment,
};
use serde::{Deserialize, Serialize};

/// Declarative Project & OCap Manifest (`slice.toml`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliceManifest {
    pub name: String,
    pub version: String,
    pub entrypoint: String,
    pub fuel_limit: u64,
    pub memory_limit_kb: usize,
    pub env_file: Option<String>,
    pub capabilities: CapabilityToken,
    #[serde(default)]
    pub exper_boacas: Option<bool>,
    #[serde(default)]
    pub tier: Option<u8>,
    #[serde(default)]
    pub warm_pool: Option<bool>,
    #[serde(default)]
    pub deterministic_seed: Option<u64>,
    #[serde(default)]
    pub audit_egress: Vec<String>,
    #[serde(default)]
    pub server_port: Option<u16>,
    #[serde(default)]
    pub server_workers: Option<usize>,
    #[serde(default)]
    pub static_dir: Option<String>,
    #[serde(default)]
    pub tls_cert: Option<String>,
    #[serde(default)]
    pub tls_key: Option<String>,
    #[serde(default)]
    pub trust_directives: Option<bool>,
    #[serde(default)]
    pub artifacts_dir: Option<String>,
}

impl Default for SliceManifest {
    fn default() -> Self {
        Self {
            name: "slice-edge-service".to_string(),
            version: "0.1.0".to_string(),
            entrypoint: "./src/index.ts".to_string(),
            fuel_limit: 50_000_000,
            memory_limit_kb: 4096,
            env_file: None,
            capabilities: CapabilityToken::pure_compute(),
            exper_boacas: None,
            tier: None,
            warm_pool: None,
            deterministic_seed: None,
            audit_egress: Vec::new(),
            server_port: None,
            server_workers: None,
            static_dir: None,
            tls_cert: None,
            tls_key: None,
            trust_directives: None,
            artifacts_dir: None,
        }
    }
}

pub(crate) const MAX_SAFE_MEMORY_KB: usize = usize::MAX / 1024;

impl SliceManifest {
    /// Parses a `slice.toml` configuration string without external heavy dependencies,
    /// supporting single-line and multi-line `[...]` array values with `#` comments.
    pub fn parse_toml(content: &str) -> Self {
        let mut manifest = Self::default();
        let mut current_section = "";
        let mut pending_buf = String::new();
        let mut pending_depth = 0i32;

        for line in content.lines() {
            let cleaned = strip_toml_inline_comment(line);
            if cleaned.is_empty() {
                continue;
            }

            let owned_joined;
            let trimmed = if pending_depth > 0 {
                pending_depth += bracket_delta_outside_quotes(cleaned);
                if !pending_buf.is_empty() {
                    pending_buf.push(' ');
                }
                pending_buf.push_str(cleaned);
                if pending_depth > 0 {
                    continue;
                }
                pending_depth = 0;
                owned_joined = std::mem::take(&mut pending_buf);
                owned_joined.as_str()
            } else {
                if cleaned.starts_with('[') && cleaned.ends_with(']') && !cleaned.contains('=') {
                    current_section = cleaned[1..cleaned.len() - 1].trim();
                    continue;
                }
                if cleaned.contains('=') {
                    let delta = bracket_delta_outside_quotes(cleaned);
                    if delta > 0 {
                        pending_buf.clear();
                        pending_buf.push_str(cleaned);
                        pending_depth = delta;
                        continue;
                    }
                }
                cleaned
            };

            if let Some((k, v)) = trimmed.split_once('=') {
                let key = k.trim();
                let val = v.trim();
                match (current_section, key) {
                    ("package", "name") => {
                        manifest.name = val.trim_matches('"').trim_matches('\'').to_string();
                    }
                    ("package", "version") => {
                        manifest.version = val.trim_matches('"').trim_matches('\'').to_string();
                    }
                    ("package", "entrypoint") => {
                        manifest.entrypoint = val.trim_matches('"').trim_matches('\'').to_string();
                    }
                    ("limits", "fuel") | ("runtime", "fuel") | ("runtime", "fuel_limit") => {
                        if let Some(n) = parse_u64_ignoring_underscores(val) {
                            manifest.fuel_limit = n;
                        }
                    }
                    ("limits", "memory_kb")
                    | ("runtime", "memory_kb")
                    | ("runtime", "memory_limit_kb") => {
                        if let Some(n) = parse_u64_ignoring_underscores(val) {
                            manifest.memory_limit_kb = usize::try_from(n)
                                .unwrap_or(MAX_SAFE_MEMORY_KB)
                                .min(MAX_SAFE_MEMORY_KB);
                        }
                    }
                    ("limits", "memory_mb")
                    | ("runtime", "memory_mb")
                    | ("runtime", "memory_limit_mb") => {
                        if let Some(n) = parse_u64_ignoring_underscores(val) {
                            manifest.memory_limit_kb = usize::try_from(n)
                                .unwrap_or(MAX_SAFE_MEMORY_KB)
                                .saturating_mul(1024)
                                .min(MAX_SAFE_MEMORY_KB);
                        }
                    }
                    ("limits", "memory_bytes")
                    | ("limits", "memory")
                    | ("runtime", "memory_bytes")
                    | ("runtime", "memory_limit_bytes")
                    | ("runtime", "memory") => {
                        if let Some(bytes) = parse_byte_size_usize(val) {
                            manifest.memory_limit_kb = (bytes / 1024).clamp(1, MAX_SAFE_MEMORY_KB);
                        }
                    }
                    ("limits", "max_checkpoints") | ("capabilities", "max_checkpoints") => {
                        if let Some(n) = parse_u64_ignoring_underscores(val) {
                            let max = n.min(u32::MAX as u64) as u32;
                            manifest.capabilities.max_checkpoints = Some(max);
                            if max == 0 {
                                manifest.capabilities.allow_checkpoints = false;
                            }
                        }
                    }
                    ("capabilities", "checkpoints")
                    | ("capabilities", "allow_checkpoints")
                    | ("limits", "checkpoints") => {
                        if let Some(b) = parse_bool_setting(val) {
                            manifest.capabilities.set_checkpoints(b);
                        }
                    }
                    ("capabilities", "allow_read") => {
                        for item in parse_toml_string_array(val) {
                            manifest.capabilities.push_read_prefix(&item);
                        }
                    }
                    ("capabilities", "allow_write") => {
                        for item in parse_toml_string_array(val) {
                            manifest.capabilities.push_write_prefix(&item);
                        }
                    }
                    ("capabilities", "virtual_mount")
                    | ("capabilities", "virtual_mounts")
                    | ("capabilities", "virtual-mount") => {
                        for item in parse_toml_string_array(val) {
                            manifest.capabilities.push_virtual_mount(&item);
                        }
                    }
                    ("run", "artifacts_dir")
                    | ("run", "artifacts-dir")
                    | ("capabilities", "artifacts_dir") => {
                        let d = val.trim_matches('"').trim_matches('\'').trim().to_string();
                        if !d.is_empty() {
                            manifest.artifacts_dir = Some(d);
                        }
                    }
                    ("capabilities", "allow_net") => {
                        for item in parse_toml_string_array(val) {
                            manifest.capabilities.insert_net_host(&item);
                        }
                    }
                    ("capabilities", "allow_private_networks")
                    | ("capabilities", "allow-private-networks") => {
                        if let Some(b) = parse_bool_setting(val) {
                            manifest.capabilities.set_allow_private_networks(b);
                        }
                    }
                    ("capabilities", "allow_exec") | ("capabilities", "allow-exec") => {
                        for item in parse_toml_string_array(val) {
                            manifest.capabilities.push_exec_binary(item);
                        }
                    }
                    ("capabilities", "allow_ext")
                    | ("capabilities", "allow-ext")
                    | ("capabilities", "allow_extensions") => {
                        for item in parse_toml_string_array(val) {
                            manifest.capabilities.push_extension(item);
                        }
                    }
                    ("capabilities", "allow_env") => {
                        for item in parse_toml_string_array(val) {
                            if let Some((k, v)) = item.split_once('=') {
                                manifest
                                    .capabilities
                                    .allowed_env_keys
                                    .insert(k.trim().to_string(), v.trim().to_string());
                            } else {
                                let k = item.trim();
                                let v = std::env::var(k).unwrap_or_default();
                                manifest
                                    .capabilities
                                    .allowed_env_keys
                                    .insert(k.to_string(), v);
                            }
                        }
                    }
                    ("capabilities", "env_file") => {
                        let ef = val.trim_matches('"').trim_matches('\'').trim().to_string();
                        if !ef.is_empty() {
                            manifest.env_file = Some(ef);
                        }
                    }
                    ("capabilities", "audit_egress") | ("security", "audit_egress") => {
                        for item in parse_toml_string_array(val) {
                            manifest.audit_egress.push(item);
                        }
                    }
                    ("capabilities", "trust_directives")
                    | ("security", "trust_directives")
                    | ("engine", "trust_directives")
                    | ("runtime", "trust_directives") => {
                        if let Some(b) = parse_bool_setting(val) {
                            manifest.trust_directives = Some(b);
                        }
                    }
                    ("capabilities.env", _) => {
                        let env_val = val.trim_matches('"').trim_matches('\'').to_string();
                        manifest
                            .capabilities
                            .allowed_env_keys
                            .insert(key.to_string(), env_val);
                    }
                    ("engine", "boacas") | ("runtime", "boacas") | ("experiments", "boacas") => {
                        if let Some(b) = parse_bool_setting(val) {
                            manifest.exper_boacas = Some(b);
                        }
                    }
                    ("engine", "tier") | ("runtime", "tier") => {
                        if let Some(t) = parse_tier_setting(val) {
                            manifest.tier = t;
                        }
                    }
                    ("engine", "warm")
                    | ("engine", "warm_pool")
                    | ("runtime", "warm")
                    | ("runtime", "warm_pool") => {
                        if let Some(b) = parse_bool_setting(val) {
                            manifest.warm_pool = Some(b);
                        }
                    }
                    ("engine", "deterministic_seed")
                    | ("engine", "deterministic")
                    | ("runtime", "deterministic_seed") => {
                        if let Some(n) = parse_u64_ignoring_underscores(val) {
                            manifest.deterministic_seed = Some(n);
                        }
                    }
                    ("server", "port") => {
                        if let Some(n) = parse_u64_ignoring_underscores(val)
                            && let Ok(p) = u16::try_from(n)
                        {
                            manifest.server_port = Some(p);
                        }
                    }
                    ("server", "workers") => {
                        if let Some(n) = parse_u64_ignoring_underscores(val) {
                            manifest.server_workers = Some((n as usize).max(1));
                        }
                    }
                    ("server", "static_dir") | ("server", "static") => {
                        let sd = val.trim_matches('"').trim_matches('\'').trim().to_string();
                        if !sd.is_empty() {
                            manifest.static_dir = Some(sd);
                        }
                    }
                    ("server", "tls_cert") | ("server", "tls-cert") | ("server", "cert") => {
                        let tc = val.trim_matches('"').trim_matches('\'').trim().to_string();
                        if !tc.is_empty() {
                            manifest.tls_cert = Some(tc);
                        }
                    }
                    ("server", "tls_key") | ("server", "tls-key") | ("server", "key") => {
                        let tk = val.trim_matches('"').trim_matches('\'').trim().to_string();
                        if !tk.is_empty() {
                            manifest.tls_key = Some(tk);
                        }
                    }
                    _ => {}
                }
            }
        }
        manifest
    }

    /// Generates a documented default `slice.toml` file for `slice init`.
    pub fn default_template(project_name: &str) -> String {
        format!(
            r#"# Slice (Nexus-JS) Declarative MicroVM & Object Capability Manifest

[package]
name = "{}"
version = "0.1.0"
entrypoint = "./src/index.ts"

[engine]
# Default optimal execution tier ("tier1" = Precompiled Boa ES2024+ with Tier-2 SIMD intrinsics)
tier = "tier1"
# Precompiled Boa CodeBlock CAS container (`SLICEBC\x01`, enabled by default)
boacas = true

[limits]
# Deterministic CPU instruction fuel ceiling per tenant invocation
fuel = 50_000_000
# Hard per-compartment memory allocation ceiling in KB
memory_kb = 16384

[capabilities]
# Attenuated Virtual Filesystem read prefixes (Zero ambient filesystem access)
allow_read = ["/app/public", "/app/config"]
# Attenuated outbound network host:port allowlist
allow_net = ["api.internal:443", "edge.slice.local:80"]

[capabilities.env]
SERVICE_STAGE = "production"
RUNTIME_ENGINE = "Slice-NexusJS"
"#,
            project_name
        )
    }
}
