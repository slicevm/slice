//! Top-of-File Inline Source Directives (`// @slice-<key>: <value>`).

use crate::manifest::{
    parse_bool_setting, parse_tier_setting, parse_toml_string_array, parse_u64_ignoring_underscores,
};

/// Top-of-file inline source directives (`// @slice-<key>: <value>` or `// @slice-<key>=<value>`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InlineDirectives {
    pub tier: Option<Option<u8>>,
    pub boacas: Option<bool>,
    pub warm: Option<bool>,
    pub deterministic_seed: Option<u64>,
    pub fuel_limit: Option<u64>,
    pub memory_limit_bytes: Option<usize>,
    pub allow_read: Vec<String>,
    pub allow_write: Vec<String>,
    pub allow_net: Vec<String>,
    pub allow_env: Vec<(String, String)>,
    pub allow_exec: Vec<String>,
    pub env_file: Option<String>,
    pub audit_egress: Vec<String>,
    pub port: Option<u16>,
    pub workers: Option<usize>,
    pub tls_cert: Option<String>,
    pub tls_key: Option<String>,
}

/// Scans leading comment lines (`// @slice-...`) at the top of a TypeScript/JavaScript source string
/// in `O(header lines)` without allocating on the body (`SBX-PERF-03`).
pub fn parse_inline_directives(source: &str) -> InlineDirectives {
    let mut dirs = InlineDirectives::default();
    for line in source.lines().take(64) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("#!") {
            continue;
        }
        let Some(comment_body) = trimmed.strip_prefix("//") else {
            if trimmed.starts_with("/*") {
                continue;
            }
            break;
        };
        let body = comment_body.trim();
        let Some(rest) = body.strip_prefix("@slice-") else {
            continue;
        };
        if rest == "no-boacas" || rest == "no-exper-boacas" {
            dirs.boacas = Some(false);
            continue;
        } else if rest == "boacas" || rest == "exper-boacas" {
            dirs.boacas = Some(true);
            continue;
        } else if rest == "warm" {
            dirs.warm = Some(true);
            continue;
        } else if rest == "no-warm" {
            dirs.warm = Some(false);
            continue;
        }
        let Some((raw_k, raw_v)) = rest.split_once(':').or_else(|| rest.split_once('=')) else {
            continue;
        };
        let key = raw_k.trim();
        let val = raw_v.trim().trim_matches('"').trim_matches('\'').trim();
        match key {
            "tier" => {
                if let Some(t) = parse_tier_setting(val) {
                    dirs.tier = Some(t);
                }
            }
            "boacas" | "exper-boacas" => {
                if let Some(b) = parse_bool_setting(val) {
                    dirs.boacas = Some(b);
                }
            }
            "warm" | "warm-pool" => {
                if let Some(b) = parse_bool_setting(val) {
                    dirs.warm = Some(b);
                }
            }
            "deterministic" | "deterministic-seed" | "seed" => {
                if let Some(n) = parse_u64_ignoring_underscores(val) {
                    dirs.deterministic_seed = Some(n);
                }
            }
            "fuel" => {
                if let Some(n) = parse_u64_ignoring_underscores(val) {
                    dirs.fuel_limit = Some(n);
                }
            }
            "memory-kb" | "memory_kb" => {
                if let Some(n) = parse_u64_ignoring_underscores(val) {
                    dirs.memory_limit_bytes = Some((n as usize).saturating_mul(1024));
                }
            }
            "memory-mb" | "memory_mb" => {
                if let Some(n) = parse_u64_ignoring_underscores(val) {
                    dirs.memory_limit_bytes = Some((n as usize).saturating_mul(1024 * 1024));
                }
            }
            "memory" | "memory-bytes" => {
                if let Some(n) = parse_u64_ignoring_underscores(val) {
                    dirs.memory_limit_bytes = Some(n as usize);
                }
            }
            "allow-read" | "allow_read" => {
                for item in parse_toml_string_array(val) {
                    dirs.allow_read.push(item);
                }
            }
            "allow-write" | "allow_write" => {
                for item in parse_toml_string_array(val) {
                    dirs.allow_write.push(item);
                }
            }
            "allow-net" | "allow_net" => {
                for item in parse_toml_string_array(val) {
                    dirs.allow_net.push(item);
                }
            }
            "allow-exec" | "allow_exec" => {
                for item in parse_toml_string_array(val) {
                    dirs.allow_exec.push(item);
                }
            }
            "allow-env" | "allow_env" => {
                for item in parse_toml_string_array(val) {
                    if let Some((ek, ev)) = item.split_once('=') {
                        dirs.allow_env
                            .push((ek.trim().to_string(), ev.trim().to_string()));
                    } else {
                        let ek = item.trim();
                        let ev = std::env::var(ek).unwrap_or_default();
                        dirs.allow_env.push((ek.to_string(), ev));
                    }
                }
            }
            "env-file" | "env_file" => {
                if !val.is_empty() {
                    dirs.env_file = Some(val.to_string());
                }
            }
            "audit-egress" | "audit_egress" => {
                for item in parse_toml_string_array(val) {
                    dirs.audit_egress.push(item);
                }
            }
            "port" => {
                if let Some(n) = parse_u64_ignoring_underscores(val) {
                    if let Ok(p) = u16::try_from(n) {
                        dirs.port = Some(p);
                    }
                }
            }
            "workers" => {
                if let Some(n) = parse_u64_ignoring_underscores(val) {
                    dirs.workers = Some((n as usize).max(1));
                }
            }
            "tls-cert" | "tls_cert" if !val.is_empty() => {
                dirs.tls_cert = Some(val.to_string());
            }
            "tls-key" | "tls_key" if !val.is_empty() => {
                dirs.tls_key = Some(val.to_string());
            }
            _ => {}
        }
    }
    dirs
}
