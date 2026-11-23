//! Top-of-File Inline Source Directives (`// @slice-<key>: <value>`).

use crate::manifest::{
    parse_bool_setting, parse_tier_setting, parse_toml_string_array,
    parse_u64_ignoring_underscores, strip_toml_inline_comment,
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
    pub allow_env_inherit: Vec<String>,
    pub allow_exec: Vec<String>,
    pub allow_checkpoints: Option<bool>,
    pub max_checkpoints: Option<u32>,
    pub env_file: Option<String>,
    pub audit_egress: Vec<String>,
    pub port: Option<u16>,
    pub workers: Option<usize>,
    pub tls_cert: Option<String>,
    pub tls_key: Option<String>,
}

#[inline]
fn unquote_scalar_directive(raw: &str) -> &str {
    let cleaned = strip_toml_inline_comment(raw);
    if cleaned.len() >= 2
        && ((cleaned.starts_with('"') && cleaned.ends_with('"'))
            || (cleaned.starts_with('\'') && cleaned.ends_with('\'')))
    {
        cleaned[1..cleaned.len() - 1].trim()
    } else {
        cleaned
    }
}

fn apply_directive_body(dirs: &mut InlineDirectives, comment_body: &str) {
    let body = comment_body.trim();
    let Some(rest) = body.strip_prefix("@slice-") else {
        return;
    };
    let Some((raw_k, raw_v)) = rest.split_once(':').or_else(|| rest.split_once('=')) else {
        let flag = strip_toml_inline_comment(rest);
        match flag {
            "no-boacas" | "no-exper-boacas" => dirs.boacas = Some(false),
            "boacas" | "exper-boacas" => dirs.boacas = Some(true),
            "warm" => dirs.warm = Some(true),
            "no-warm" => dirs.warm = Some(false),
            "checkpoints" | "allow-checkpoints" => {
                dirs.allow_checkpoints = Some(true);
                if dirs.max_checkpoints == Some(0) {
                    dirs.max_checkpoints = None;
                }
            }
            "no-checkpoints" => {
                dirs.allow_checkpoints = Some(false);
                dirs.max_checkpoints = Some(0);
            }
            _ => {}
        }
        return;
    };
    let key = raw_k.trim();
    let raw_v_trimmed = raw_v.trim();
    let val = unquote_scalar_directive(raw_v_trimmed);
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
                let bytes = usize::try_from(n)
                    .unwrap_or(usize::MAX)
                    .saturating_mul(1024);
                dirs.memory_limit_bytes = Some(bytes);
            }
        }
        "memory-mb" | "memory_mb" => {
            if let Some(n) = parse_u64_ignoring_underscores(val) {
                let bytes = usize::try_from(n)
                    .unwrap_or(usize::MAX)
                    .saturating_mul(1024 * 1024);
                dirs.memory_limit_bytes = Some(bytes);
            }
        }
        "memory" | "memory-bytes" => {
            if let Some(n) = parse_u64_ignoring_underscores(val) {
                dirs.memory_limit_bytes = Some(usize::try_from(n).unwrap_or(usize::MAX));
            }
        }
        "allow-read" | "allow_read" => {
            for item in parse_toml_string_array(raw_v_trimmed) {
                dirs.allow_read.push(item);
            }
        }
        "allow-write" | "allow_write" => {
            for item in parse_toml_string_array(raw_v_trimmed) {
                dirs.allow_write.push(item);
            }
        }
        "allow-net" | "allow_net" => {
            for item in parse_toml_string_array(raw_v_trimmed) {
                dirs.allow_net.push(item);
            }
        }
        "allow-exec" | "allow_exec" => {
            for item in parse_toml_string_array(raw_v_trimmed) {
                dirs.allow_exec.push(item);
            }
        }
        "checkpoints" | "allow-checkpoints" | "allow_checkpoints" => {
            if let Some(b) = parse_bool_setting(val) {
                dirs.allow_checkpoints = Some(b);
                if !b {
                    dirs.max_checkpoints = Some(0);
                } else if dirs.max_checkpoints == Some(0) {
                    dirs.max_checkpoints = None;
                }
            }
        }
        "max-checkpoints" | "max_checkpoints" => {
            if let Some(n) = parse_u64_ignoring_underscores(val) {
                let max_cp = u32::try_from(n).unwrap_or(u32::MAX);
                dirs.max_checkpoints = Some(max_cp);
                if max_cp == 0 {
                    dirs.allow_checkpoints = Some(false);
                }
            }
        }
        "allow-env" | "allow_env" => {
            for item in parse_toml_string_array(raw_v_trimmed) {
                if let Some((ek, ev)) = item.split_once('=') {
                    let k = ek.trim();
                    if !k.is_empty() {
                        dirs.allow_env.push((k.to_string(), ev.trim().to_string()));
                    }
                } else {
                    let ek = item.trim();
                    if !ek.is_empty() {
                        dirs.allow_env_inherit.push(ek.to_string());
                    }
                }
            }
        }
        "env-file" | "env_file" => {
            if !val.is_empty() {
                dirs.env_file = Some(val.to_string());
            }
        }
        "audit-egress" | "audit_egress" => {
            for item in parse_toml_string_array(raw_v_trimmed) {
                dirs.audit_egress.push(item);
            }
        }
        "port" => {
            if let Some(n) = parse_u64_ignoring_underscores(val)
                && let Ok(p) = u16::try_from(n)
            {
                dirs.port = Some(p);
            }
        }
        "workers" => {
            if let Some(n) = parse_u64_ignoring_underscores(val) {
                dirs.workers = Some(usize::try_from(n).unwrap_or(usize::MAX).max(1));
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

/// Scans leading comment lines (`// @slice-...`) at the top of a TypeScript/JavaScript source string
/// in `O(header lines)` without allocating on the body (`SBX-PERF-03`).
///
/// Tracks `/* ... */` block-comment state across lines so commented-out directives are ignored,
/// and stops scanning immediately at the first non-comment, non-shebang, non-`"use strict"` token.
pub fn parse_inline_directives(source: &str) -> InlineDirectives {
    let mut dirs = InlineDirectives::default();
    let mut in_block_comment = false;

    for (line_idx, raw_line) in source.lines().take(64).enumerate() {
        let mut s = if line_idx == 0 {
            raw_line.trim_start_matches('\u{feff}').trim()
        } else {
            raw_line.trim()
        };
        if line_idx == 0 && !in_block_comment && s.starts_with("#!") {
            continue;
        }
        while !s.is_empty() {
            if in_block_comment {
                if let Some(end_idx) = s.find("*/") {
                    in_block_comment = false;
                    s = s[end_idx + 2..].trim_start();
                    continue;
                }
                break;
            }
            if let Some(comment_body) = s.strip_prefix("//") {
                apply_directive_body(&mut dirs, comment_body);
                break;
            }
            if let Some(after_open) = s.strip_prefix("/*") {
                in_block_comment = true;
                s = after_open;
                continue;
            }
            if let Some(rem) = s
                .strip_prefix("\"use strict\"")
                .or_else(|| s.strip_prefix("'use strict'"))
            {
                let rem = rem.trim_start();
                let rem = rem.strip_prefix(';').unwrap_or(rem).trim_start();
                if rem.is_empty() || rem.starts_with("//") || rem.starts_with("/*") {
                    s = rem;
                    continue;
                }
            }
            return dirs;
        }
    }
    dirs
}
