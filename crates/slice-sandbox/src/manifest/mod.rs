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

fn strip_toml_inline_comment(raw: &str) -> &str {
    let mut in_double = false;
    let mut in_single = false;
    let mut escaped = false;
    for (idx, ch) in raw.char_indices() {
        if in_double {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_double = false;
            }
            continue;
        }
        if in_single {
            if ch == '\'' {
                in_single = false;
            }
            continue;
        }
        match ch {
            '"' => in_double = true,
            '\'' => in_single = true,
            '#' => return raw[..idx].trim(),
            _ => {}
        }
    }
    raw.trim()
}

#[inline]
pub(crate) fn parse_toml_string_array(raw: &str) -> Vec<String> {
    let cleaned = strip_toml_inline_comment(raw);
    let inner = cleaned
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(cleaned);
    let mut items = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut had_quotes = false;

    for ch in inner.chars() {
        if let Some(q) = quote {
            if q == '"' && escaped {
                match ch {
                    'n' => current.push('\n'),
                    'r' => current.push('\r'),
                    't' => current.push('\t'),
                    '\\' => current.push('\\'),
                    '"' => current.push('"'),
                    other => {
                        current.push('\\');
                        current.push(other);
                    }
                }
                escaped = false;
            } else if q == '"' && ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }

        match ch {
            '"' | '\'' => {
                if !had_quotes {
                    current.clear();
                }
                quote = Some(ch);
                had_quotes = true;
            }
            ',' => {
                let val = if had_quotes {
                    current.clone()
                } else {
                    current.trim().to_string()
                };
                if !val.is_empty() {
                    items.push(val);
                }
                current.clear();
                had_quotes = false;
            }
            _ => {
                if !had_quotes {
                    current.push(ch);
                }
            }
        }
    }
    let tail = if had_quotes {
        current
    } else {
        current.trim().to_string()
    };
    if !tail.is_empty() {
        items.push(tail);
    }
    items
}

impl SliceManifest {
    /// Parses a `slice.toml` configuration string without external heavy dependencies.
    pub fn parse_toml(content: &str) -> Self {
        let mut manifest = Self::default();
        let mut current_section = "";

        for line in content.lines() {
            let trimmed = strip_toml_inline_comment(line);
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                current_section = trimmed[1..trimmed.len() - 1].trim();
                continue;
            }
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
                            manifest.capabilities.allow_checkpoints = b;
                            if !b {
                                manifest.capabilities.max_checkpoints = Some(0);
                            }
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

#[cfg(test)]
mod tests {
    use super::SliceManifest;
    use crate::manifest::dotenv::parse_dotenv_content;

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
        assert!(m
            .capabilities
            .allowed_net_hosts
            .contains("api.internal:443"));
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
}
