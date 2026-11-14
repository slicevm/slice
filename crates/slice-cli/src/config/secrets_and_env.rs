//! Fail-closed `.env` loader, `--audit-egress-file` secret ingestion, and Auto-DLP registration.

use anyhow::{Context, Result};
use slice_sandbox::CapabilityToken;
use std::path::Path;

#[inline]
pub(crate) fn push_unique_audit_secret(audit_secrets: &mut Vec<String>, secret: String) {
    if !secret.is_empty() && !audit_secrets.contains(&secret) {
        audit_secrets.push(secret);
    }
}

#[inline]
pub(crate) fn maybe_register_sensitive_env_dlp(
    key: &str,
    val: &str,
    audit_secrets: &mut Vec<String>,
) {
    if val.len() >= 6 && slice_sandbox::is_sensitive_env_key(key) {
        push_unique_audit_secret(audit_secrets, val.to_string());
    }
}

#[inline]
fn unquote_secret_value(raw: &str) -> &str {
    let trimmed = raw.trim();
    if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    }
}

#[inline]
fn is_dotenv_identifier(key: &str) -> bool {
    !key.is_empty()
        && !key.as_bytes()[0].is_ascii_digit()
        && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Parses and registers secrets from an `--audit-egress-file` / `SLICE_AUDIT_EGRESS_FILE` path.
///
/// Security: Fails closed if `sec_path` is missing, unreadable, a directory, or invalid UTF-8.
/// Supports `.env`-style `KEY=VALUE` / `export KEY="VALUE"` entries (for any `KEY` identifier,
/// regardless of key naming heuristics), raw one-secret-per-line entries, and raw Base64 tokens
/// ending in `=` or `==` padding.
pub(crate) fn load_audit_egress_file(
    sec_path: &Path,
    audit_secrets: &mut Vec<String>,
) -> Result<usize> {
    let raw = std::fs::read_to_string(sec_path)
        .with_context(|| format!("Failed to read --audit-egress-file {}", sec_path.display()))?;
    let before = audit_secrets.len();
    for raw_line in raw.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let stripped = line
            .strip_prefix("export ")
            .map(str::trim_start)
            .unwrap_or(line);
        if let Some((raw_k, raw_v)) = stripped.split_once('=') {
            let k = raw_k.trim();
            let v_trimmed = raw_v.trim();
            // Distinguish `KEY=VALUE` from a bare Base64 token with `=` / `==` padding on its own line.
            let looks_like_b64_padding =
                raw_line.trim_start() == stripped && (v_trimmed.is_empty() || v_trimmed == "=");
            if is_dotenv_identifier(k) && !looks_like_b64_padding {
                let val = unquote_secret_value(v_trimmed);
                push_unique_audit_secret(audit_secrets, val.to_string());
                continue;
            }
        }
        let secret = unquote_secret_value(line);
        push_unique_audit_secret(audit_secrets, secret.to_string());
    }
    Ok(audit_secrets.len().saturating_sub(before))
}

/// Registers a comma-delimited `--audit-egress` / `audit_egress` specification (`"auto"` or literal tokens).
pub(crate) fn register_audit_egress_spec(
    spec: &str,
    base_dir: Option<&Path>,
    caps: &CapabilityToken,
    audit_secrets: &mut Vec<String>,
) -> Result<()> {
    for token in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if token.eq_ignore_ascii_case("auto") {
            if let Some(dir) = base_dir {
                let dotenv_path = dir.join(".env");
                if dotenv_path.exists() {
                    let raw = std::fs::read_to_string(&dotenv_path).with_context(|| {
                        format!(
                            "Failed to read {} for --audit-egress=auto",
                            dotenv_path.display()
                        )
                    })?;
                    let (_, secrets) = slice_sandbox::parse_dotenv_content(&raw);
                    for sec in secrets {
                        push_unique_audit_secret(audit_secrets, sec);
                    }
                }
            }
            for (k, v) in &caps.allowed_env_keys {
                maybe_register_sensitive_env_dlp(k, v, audit_secrets);
            }
        } else {
            push_unique_audit_secret(audit_secrets, token.to_string());
        }
    }
    Ok(())
}

/// Fail-closed `.env` file reader that grants `KEY=VALUE` environment variables in `caps` and
/// auto-registers sensitive credentials (`*_SECRET*`, `*_TOKEN*`, `*_KEY*`, `*_PASSWORD*`, `*_PRIVATE*`)
/// into `audit_secrets`.
pub(crate) fn apply_dotenv_file_to_caps_and_dlp(
    dotenv_path: &Path,
    caps: &mut CapabilityToken,
    audit_secrets: &mut Vec<String>,
) -> Result<usize> {
    let raw = std::fs::read_to_string(dotenv_path)
        .with_context(|| format!("Failed to read --env-file {}", dotenv_path.display()))?;
    let (pairs, secrets) = slice_sandbox::parse_dotenv_content(&raw);
    for (k, v) in pairs {
        caps.allowed_env_keys.insert(k, v);
    }
    let count = secrets.len();
    for sec in secrets {
        push_unique_audit_secret(audit_secrets, sec);
    }
    Ok(count)
}
