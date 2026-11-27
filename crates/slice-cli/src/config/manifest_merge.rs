//! `slice.toml` manifest and inline `// @slice-...` directive merging into `ResolvedExecutionConfig`.

use super::{
    ResolvedExecutionConfig, apply_boacas_override, apply_dotenv_file_to_caps_and_dlp,
    apply_tier_override, maybe_register_sensitive_env_dlp, register_audit_egress_spec,
};
use anyhow::{Context, Result};
use slice_sandbox::{CapabilityToken, SliceManifest};
use std::path::{Path, PathBuf};

impl ResolvedExecutionConfig {
    /// Merges a target project's `slice.toml` or embedded `.cas` manifest TOML into this
    /// configuration, updating settings unless already overridden by `--config`, `SLICE_*` env vars, or CLI flags.
    pub(crate) fn apply_manifest_toml(
        &mut self,
        raw_toml: &str,
        base_dir: Option<&Path>,
    ) -> Result<()> {
        let mut m = SliceManifest::parse_toml(raw_toml);
        if self.has_explicit_config {
            // Explicit `--config=<path>` is authoritative: embedded `.cas` manifests must NEVER
            // widen host-governed capabilities, limits, or `.env` files. Only literal `audit_egress`
            // DLP tokens are registered as monotonic defensive restrictions.
            for sec in m.audit_egress {
                if !sec.trim().eq_ignore_ascii_case("auto") {
                    register_audit_egress_spec(&sec, None, &self.caps, &mut self.audit_secrets)?;
                }
            }
            return Ok(());
        }
        if let Some(cp_override) = self.cli_checkpoints_override {
            m.capabilities.set_checkpoints(cp_override);
        }
        if let Some(max_cp_override) = self.cli_max_checkpoints_override {
            m.capabilities.set_max_checkpoints(Some(max_cp_override));
        }
        if self.cli_fuel_override.is_none() {
            self.fuel_limit = m.fuel_limit;
        }
        if self.cli_mem_override.is_none() {
            self.mem_limit = m.memory_limit_kb.saturating_mul(1024);
        }
        if self.cli_boacas_override.is_none()
            && let Some(b) = m.exper_boacas
        {
            apply_boacas_override(Some(b));
        }
        if self.cli_tier_override.is_none()
            && let Some(t) = m.tier
        {
            apply_tier_override(Some(t));
        }
        if self.cli_warm_override.is_none()
            && let Some(w) = m.warm_pool
        {
            self.warm_pool = w;
        }
        if self.cli_det_override.is_none()
            && let Some(s) = m.deterministic_seed
        {
            self.deterministic_seed = Some(s);
        }
        if self.cli_port_override.is_none()
            && let Some(p) = m.server_port
        {
            self.server_port = Some(p);
        }
        if self.cli_workers_override.is_none()
            && let Some(w) = m.server_workers
        {
            self.server_workers = Some(w);
        }
        if let Some(dir) = base_dir {
            if self.cli_static_override.is_none()
                && let Some(ref sd) = m.static_dir
            {
                self.static_dir = Some(dir.join(sd));
            }
            if self.cli_artifacts_dir_override.is_none()
                && let Some(ref ad) = m.artifacts_dir
            {
                self.artifacts_dir = Some(dir.join(ad));
            }
            if self.cli_tls_cert_override.is_none()
                && let Some(ref tc) = m.tls_cert
            {
                self.tls_cert = Some(dir.join(tc));
            }
            if self.cli_tls_key_override.is_none()
                && let Some(ref tk) = m.tls_key
            {
                self.tls_key = Some(dir.join(tk));
            }
        }
        if self.cli_trust_directives_override.is_none()
            && let Some(td) = m.trust_directives
        {
            self.trust_directives = td;
        }
        merge_parsed_manifest_into_caps_and_dlp(
            m,
            base_dir,
            &mut self.caps,
            &mut self.audit_secrets,
        )
    }

    /// Applies top-of-file inline source directives (`// @slice-...`) from an entrypoint script,
    /// overriding `slice.toml` defaults while respecting `SLICE_*` env vars and CLI flags.
    pub(crate) fn apply_inline_directives(
        &mut self,
        source: &str,
        base_dir: Option<&Path>,
    ) -> Result<()> {
        let trimmed = source.trim_start_matches('\u{feff}').trim_start();
        if !trimmed.starts_with("//")
            && !trimmed.starts_with("/*")
            && !trimmed.starts_with("#!")
            && !trimmed.starts_with("\"use strict\"")
            && !trimmed.starts_with("'use strict'")
        {
            return Ok(());
        }
        let dirs = slice_sandbox::parse_inline_directives(source);
        if self.cli_fuel_override.is_none()
            && let Some(f) = dirs.fuel_limit
        {
            if self.trust_directives {
                self.fuel_limit = f;
            } else {
                self.fuel_limit = self.fuel_limit.min(f);
            }
        }
        if self.cli_mem_override.is_none()
            && let Some(m) = dirs.memory_limit_bytes
        {
            if self.trust_directives {
                self.mem_limit = m;
            } else {
                self.mem_limit = self.mem_limit.min(m);
            }
        }
        if self.cli_boacas_override.is_none()
            && let Some(b) = dirs.boacas
        {
            apply_boacas_override(Some(b));
        }
        if self.cli_tier_override.is_none()
            && let Some(t) = dirs.tier
        {
            apply_tier_override(t);
        }
        if self.cli_warm_override.is_none()
            && let Some(w) = dirs.warm
        {
            self.warm_pool = w;
        }
        if self.cli_det_override.is_none()
            && let Some(s) = dirs.deterministic_seed
        {
            self.deterministic_seed = Some(s);
        }
        if self.cli_port_override.is_none()
            && let Some(p) = dirs.port
        {
            self.server_port = Some(p);
        }
        if self.cli_workers_override.is_none()
            && let Some(w) = dirs.workers
        {
            self.server_workers = Some(w);
        }
        for sec in dirs.audit_egress {
            register_audit_egress_spec(&sec, base_dir, &self.caps, &mut self.audit_secrets)?;
        }

        if self.trust_directives {
            for r in dirs.allow_read {
                self.caps.push_read_prefix(&r);
            }
            for w in dirs.allow_write {
                self.caps.push_write_prefix(&w);
            }
            for nh in dirs.allow_net {
                self.caps.insert_net_host(&nh);
            }
            for eb in dirs.allow_exec {
                self.caps = std::mem::take(&mut self.caps).with_exec_binary(eb);
            }
            if self.cli_checkpoints_override.is_none()
                && let Some(b) = dirs.allow_checkpoints
            {
                self.caps.set_checkpoints(b);
            }
            if self.cli_max_checkpoints_override.is_none()
                && self.cli_checkpoints_override != Some(false)
                && let Some(max_cp) = dirs.max_checkpoints
            {
                self.caps.set_max_checkpoints(Some(max_cp));
            }
            for (k, v) in dirs.allow_env {
                maybe_register_sensitive_env_dlp(&k, &v, &mut self.audit_secrets);
                self.caps.allowed_env_keys.entry(k).or_insert(v);
            }
            for k in dirs.allow_env_inherit {
                let v = std::env::var(&k).unwrap_or_default();
                maybe_register_sensitive_env_dlp(&k, &v, &mut self.audit_secrets);
                self.caps.allowed_env_keys.entry(k).or_insert(v);
            }
            if let Some(ef) = dirs.env_file {
                let env_path = match base_dir {
                    Some(dir) => dir.join(&ef),
                    None => PathBuf::from(&ef),
                };
                apply_dotenv_file_to_caps_and_dlp(
                    &env_path,
                    &mut self.caps,
                    &mut self.audit_secrets,
                )?;
            }
            if self.cli_tls_cert_override.is_none()
                && let Some(tc) = dirs.tls_cert
            {
                let p = match base_dir {
                    Some(dir) => dir.join(&tc),
                    None => PathBuf::from(&tc),
                };
                self.tls_cert = Some(p);
            }
            if self.cli_tls_key_override.is_none()
                && let Some(tk) = dirs.tls_key
            {
                let p = match base_dir {
                    Some(dir) => dir.join(&tk),
                    None => PathBuf::from(&tk),
                };
                self.tls_key = Some(p);
            }
        } else if !dirs.allow_read.is_empty()
            || !dirs.allow_write.is_empty()
            || !dirs.allow_net.is_empty()
            || !dirs.allow_exec.is_empty()
            || !dirs.allow_env.is_empty()
            || !dirs.allow_env_inherit.is_empty()
            || dirs.allow_checkpoints == Some(false)
            || dirs.max_checkpoints.is_some()
        {
            // Untrusted mode (default): canonical monotonic OCap attenuation (`parent & requested`)
            let mut requested_env_keys: Vec<String> = dirs
                .allow_env
                .into_iter()
                .map(|(k, _)| k)
                .chain(dirs.allow_env_inherit)
                .collect();
            requested_env_keys.dedup();

            self.caps = self
                .caps
                .attenuate(&slice_sandbox::CapabilityAttenuationRequest {
                    allow_read: (!dirs.allow_read.is_empty()).then_some(dirs.allow_read.as_slice()),
                    allow_write: (!dirs.allow_write.is_empty())
                        .then_some(dirs.allow_write.as_slice()),
                    virtual_mounts: None,
                    allow_net: (!dirs.allow_net.is_empty()).then_some(dirs.allow_net.as_slice()),
                    allow_private_networks: None,
                    allow_exec: (!dirs.allow_exec.is_empty()).then_some(dirs.allow_exec.as_slice()),
                    allow_env: (!requested_env_keys.is_empty())
                        .then_some(requested_env_keys.as_slice()),
                    allow_checkpoints: (dirs.allow_checkpoints == Some(false)).then_some(false),
                    max_checkpoints: dirs.max_checkpoints,
                    consumed_checkpoints: 0,
                });
        }
        Ok(())
    }

    /// Discovers and applies `slice.toml` for `target_path` (if one exists in `target_path`,
    /// its parent directory, or its grandparent directory), skipping a duplicate disk read and
    /// TOML parse when the discovered manifest is identical to the ambient `./slice.toml` already loaded.
    pub(crate) fn apply_target_path_manifest(&mut self, target_path: &Path) -> Result<()> {
        if self.has_explicit_config {
            return Ok(());
        }
        if let Some(manifest_file) = find_project_manifest_path(target_path) {
            let canon =
                std::fs::canonicalize(&manifest_file).unwrap_or_else(|_| manifest_file.clone());
            if self.loaded_manifest_path.as_deref() == Some(canon.as_path()) {
                return Ok(());
            }
            let raw = std::fs::read_to_string(&manifest_file)
                .with_context(|| format!("Failed to read manifest {}", manifest_file.display()))?;
            self.loaded_manifest_path = Some(canon);
            self.apply_manifest_toml(&raw, manifest_file.parent())?;
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) fn merge_manifest_toml_into_caps(raw_toml: &str, caps: &mut CapabilityToken) {
    let mut dummy_secrets = Vec::new();
    merge_manifest_toml_into_caps_and_dlp(raw_toml, None, caps, &mut dummy_secrets)
        .expect("in-memory manifest TOML merge should not fail");
}

#[cfg(test)]
pub(crate) fn merge_manifest_toml_into_caps_and_dlp(
    raw_toml: &str,
    base_dir: Option<&Path>,
    caps: &mut CapabilityToken,
    audit_secrets: &mut Vec<String>,
) -> Result<()> {
    let m = SliceManifest::parse_toml(raw_toml);
    merge_parsed_manifest_into_caps_and_dlp(m, base_dir, caps, audit_secrets)
}

pub(crate) fn merge_parsed_manifest_into_caps_and_dlp(
    m: SliceManifest,
    base_dir: Option<&Path>,
    caps: &mut CapabilityToken,
    audit_secrets: &mut Vec<String>,
) -> Result<()> {
    for r in m.capabilities.allowed_read_prefixes {
        caps.push_read_prefix(&r);
    }
    for w in m.capabilities.allowed_write_prefixes {
        caps.push_write_prefix(&w);
    }
    for vm in m.capabilities.virtual_mounts {
        caps.push_virtual_mount(&vm);
    }
    for nh in m.capabilities.allowed_net_hosts {
        caps.insert_net_host(&nh);
    }
    if m.capabilities.allow_private_networks {
        caps.set_allow_private_networks(true);
    }
    for eb in m.capabilities.allowed_exec_binaries {
        *caps = std::mem::take(caps).with_exec_binary(eb);
    }
    if !m.capabilities.allow_checkpoints {
        caps.allow_checkpoints = false;
        caps.max_checkpoints = Some(0);
    }
    if let Some(max_cp) = m.capabilities.max_checkpoints {
        caps.max_checkpoints = Some(match caps.max_checkpoints {
            Some(existing) => existing.min(max_cp),
            None => max_cp,
        });
        if max_cp == 0 {
            caps.allow_checkpoints = false;
        }
    }
    for (k, v) in m.capabilities.allowed_env_keys {
        maybe_register_sensitive_env_dlp(&k, &v, audit_secrets);
        caps.allowed_env_keys.entry(k).or_insert(v);
    }
    for sec in m.audit_egress {
        register_audit_egress_spec(&sec, base_dir, caps, audit_secrets)?;
    }
    if let (Some(env_file_rel), Some(dir)) = (m.env_file, base_dir) {
        let env_path = dir.join(&env_file_rel);
        apply_dotenv_file_to_caps_and_dlp(&env_path, caps, audit_secrets)?;
    }
    Ok(())
}

pub(crate) fn find_project_manifest_path(input_path: &Path) -> Option<PathBuf> {
    if input_path.is_dir() {
        let candidate = input_path.join("slice.toml");
        if candidate.exists() {
            return Some(candidate);
        }
    } else if let Some(parent) = input_path.parent() {
        let candidate = parent.join("slice.toml");
        if candidate.exists() {
            return Some(candidate);
        }
        if let Some(grandparent) = parent.parent() {
            let candidate2 = grandparent.join("slice.toml");
            if candidate2.exists() {
                return Some(candidate2);
            }
        }
    }
    None
}
