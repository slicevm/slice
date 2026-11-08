//! Declarative Project & OCap Manifest (`slice.toml`), Inline Directives (`// @slice-...`),
//! and `.env` Auto-DLP Credential Classification.

pub mod directives;
pub mod dotenv;

pub use directives::{parse_inline_directives, InlineDirectives};
pub use dotenv::{is_sensitive_env_key, parse_dotenv_content};

use crate::compartment::CapabilityToken;
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
        }
    }
}

/// Parses an unsigned 64-bit integer while ignoring `_` numeric separators on the stack (`SBX-PERF-03`).
#[inline]
pub(crate) fn parse_u64_ignoring_underscores(raw: &str) -> Option<u64> {
    let mut acc = 0u64;
    let mut saw_digit = false;
    for &b in raw.trim().as_bytes() {
        if b.is_ascii_digit() {
            acc = acc.checked_mul(10)?.checked_add((b - b'0') as u64)?;
            saw_digit = true;
        } else if b == b'_' || b == b'"' || b == b'\'' || b.is_ascii_whitespace() {
            continue;
        } else {
            return None;
        }
    }
    saw_digit.then_some(acc)
}

/// Parses a boolean setting string (`true`/`1`/`yes`/`on` vs `false`/`0`/`no`/`off`).
#[inline]
pub fn parse_bool_setting(raw: &str) -> Option<bool> {
    let norm = raw.trim().trim_matches('"').trim_matches('\'').trim();
    if matches!(
        norm,
        "true" | "TRUE" | "1" | "yes" | "YES" | "on" | "ON" | "enabled"
    ) {
        Some(true)
    } else if matches!(
        norm,
        "false" | "FALSE" | "0" | "no" | "NO" | "off" | "OFF" | "disabled"
    ) {
        Some(false)
    } else {
        None
    }
}

/// Parses an execution tier setting (`"1"`/`"tier1"`/`"boa"` -> `Some(1)`,
/// `"0"`/`"tier0"`/`"snp"`/`"slicesnp"` -> `Some(0)`, `"auto"`/`"default"` -> `None`).
#[inline]
pub fn parse_tier_setting(raw: &str) -> Option<Option<u8>> {
    let norm = raw.trim().trim_matches('"').trim_matches('\'').trim();
    if norm.eq_ignore_ascii_case("1")
        || norm.eq_ignore_ascii_case("tier1")
        || norm.eq_ignore_ascii_case("tier-1")
        || norm.eq_ignore_ascii_case("boa")
        || norm.eq_ignore_ascii_case("baseline")
    {
        Some(Some(1))
    } else if norm.eq_ignore_ascii_case("0")
        || norm.eq_ignore_ascii_case("tier0")
        || norm.eq_ignore_ascii_case("tier-0")
        || norm.eq_ignore_ascii_case("snp")
        || norm.eq_ignore_ascii_case("slicesnp")
    {
        Some(Some(0))
    } else if norm.eq_ignore_ascii_case("auto")
        || norm.eq_ignore_ascii_case("default")
        || norm.eq_ignore_ascii_case("optimal")
    {
        Some(None)
    } else {
        None
    }
}

#[inline]
pub(crate) fn parse_toml_string_array(raw: &str) -> Vec<String> {
    let inner = raw.trim().trim_start_matches('[').trim_end_matches(']');
    inner
        .split(',')
        .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

impl SliceManifest {
    /// Parses a `slice.toml` configuration string without external heavy dependencies.
    pub fn parse_toml(content: &str) -> Self {
        let mut manifest = Self::default();
        let mut current_section = "";

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                current_section = &trimmed[1..trimmed.len() - 1];
                continue;
            }
            if let Some((k, v)) = trimmed.split_once('=') {
                let key = k.trim();
                let mut val = v.trim();
                if !val.starts_with('"') && !val.starts_with('\'') && !val.starts_with('[') {
                    if let Some((before_comment, _)) = val.split_once('#') {
                        val = before_comment.trim();
                    }
                }
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
                    ("limits", "fuel") => {
                        if let Some(n) = parse_u64_ignoring_underscores(val) {
                            manifest.fuel_limit = n;
                        }
                    }
                    ("limits", "memory_kb") => {
                        if let Some(n) = parse_u64_ignoring_underscores(val) {
                            manifest.memory_limit_kb = n as usize;
                        }
                    }
                    ("limits", "memory_mb") => {
                        if let Some(n) = parse_u64_ignoring_underscores(val) {
                            manifest.memory_limit_kb = (n as usize).saturating_mul(1024);
                        }
                    }
                    ("limits", "memory_bytes") => {
                        if let Some(n) = parse_u64_ignoring_underscores(val) {
                            manifest.memory_limit_kb = ((n as usize) / 1024).max(1);
                        }
                    }
                    ("capabilities", "allow_read") => {
                        for item in parse_toml_string_array(val) {
                            manifest.capabilities.allowed_read_prefixes.push(item);
                        }
                    }
                    ("capabilities", "allow_write") => {
                        for item in parse_toml_string_array(val) {
                            manifest.capabilities.allowed_write_prefixes.push(item);
                        }
                    }
                    ("capabilities", "allow_net") => {
                        for item in parse_toml_string_array(val) {
                            manifest.capabilities.allowed_net_hosts.insert(item);
                        }
                    }
                    ("capabilities", "allow_exec") | ("capabilities", "allow-exec") => {
                        for item in parse_toml_string_array(val) {
                            manifest.capabilities.allowed_exec_binaries.push(item);
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
                        if let Some(n) = parse_u64_ignoring_underscores(val) {
                            if let Ok(p) = u16::try_from(n) {
                                manifest.server_port = Some(p);
                            }
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
memory_kb = 4096

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
