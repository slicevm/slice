//! Attenuated Capability Token (`CapabilityToken`) & Monotonic Lattice Narrowing (`attenuate`).

use super::exec_binary_matches;
use crate::net::{
    canonicalize_net_grant_entry, is_cloud_metadata_clean_host, is_private_or_loopback_clean_host,
    parse_net_grant_parts,
};
use crate::vfs::{is_rejected_vfs_candidate, normalize_vfs_path_lexical, vfs_prefix_matches};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};

fn default_allow_checkpoints() -> bool {
    true
}

/// Attenuated Capability Token granted explicitly to a TC39/SES Compartment.
/// By default, a new Compartment has ZERO ambient authority.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityToken {
    pub allowed_read_prefixes: Vec<String>,
    pub allowed_write_prefixes: Vec<String>,
    #[serde(default)]
    pub virtual_mounts: Vec<String>,
    pub allowed_net_hosts: FxHashSet<String>,
    pub allowed_env_keys: FxHashMap<String, String>,
    #[serde(default)]
    pub allowed_exec_binaries: Vec<String>,
    #[serde(default)]
    pub allowed_extensions: Vec<String>,
    #[serde(default)]
    pub allow_private_networks: bool,
    #[serde(default = "default_allow_checkpoints")]
    pub allow_checkpoints: bool,
    #[serde(default)]
    pub max_checkpoints: Option<u32>,
}

impl Default for CapabilityToken {
    #[inline]
    fn default() -> Self {
        Self {
            allowed_read_prefixes: Vec::new(),
            allowed_write_prefixes: Vec::new(),
            virtual_mounts: Vec::new(),
            allowed_net_hosts: FxHashSet::default(),
            allowed_env_keys: FxHashMap::default(),
            allowed_exec_binaries: Vec::new(),
            allowed_extensions: Vec::new(),
            allow_private_networks: false,
            allow_checkpoints: true,
            max_checkpoints: None,
        }
    }
}

#[inline]
fn is_already_canonical_net_grant(s: &str) -> bool {
    if s == "*" {
        return true;
    }
    if s.is_empty() || s.ends_with('.') || (s.starts_with('[') && s.ends_with(']')) {
        return false;
    }
    let sb = s.as_bytes();
    if memchr::memchr(b'%', sb).is_some() || memchr::memmem::find(sb, b"://").is_some() {
        return false;
    }
    sb.iter()
        .all(|&b| b > 0x20 && b < 0x7F && !b.is_ascii_uppercase())
}

/// Monotonic capability narrowing request passed to [`CapabilityToken::attenuate`].
///
/// Every `Option<&[String]>` field uses `None` to mean "inherit parent grants unchanged"
/// and `Some(slice)` to mean "strictly intersect against `slice`" (`parent & requested`,
/// where `Some(&[])` yields an empty grant set / deny-all).
#[derive(Debug, Clone, Default)]
pub struct CapabilityAttenuationRequest<'a> {
    pub allow_read: Option<&'a [String]>,
    pub allow_write: Option<&'a [String]>,
    pub virtual_mounts: Option<&'a [String]>,
    pub allow_net: Option<&'a [String]>,
    pub allow_private_networks: Option<bool>,
    pub allow_exec: Option<&'a [String]>,
    pub allow_ext: Option<&'a [String]>,
    pub allow_env: Option<&'a [String]>,
    pub allow_checkpoints: Option<bool>,
    pub max_checkpoints: Option<u32>,
    pub consumed_checkpoints: u32,
}

/// Returns `true` if `grant` authorizes extension target `target`.
///
/// Whole-extension grants (e.g. `"store"`) authorize both `"store"` and any method `"store.<method>"`.
/// Method-scoped grants (e.g. `"store.get"`) authorize only that exact method.
/// Wildcards (`"*"`) are never authorized.
#[inline]
pub fn extension_grant_matches(target: &str, grant: &str) -> bool {
    let t = target.trim();
    let g = grant.trim();
    if t.is_empty() || g.is_empty() || g == "*" || t == "*" {
        return false;
    }
    if t == g {
        return true;
    }
    !g.contains('.') && t.len() > g.len() + 1 && t.starts_with(g) && t.as_bytes()[g.len()] == b'.'
}

impl CapabilityToken {
    /// Pure compute compartment with zero I/O or environment authority.
    #[inline]
    pub fn pure_compute() -> Self {
        Self::default()
    }

    #[inline]
    pub fn push_read_prefix(&mut self, prefix: &str) {
        let trimmed = prefix.trim();
        if !trimmed.is_empty() {
            let canon = if trimmed == "*" || trimmed == "/" {
                trimmed.to_string()
            } else {
                normalize_vfs_path_lexical(trimmed)
            };
            if !self.allowed_read_prefixes.contains(&canon) {
                self.allowed_read_prefixes.push(canon);
            }
        }
    }

    #[inline]
    pub fn push_write_prefix(&mut self, prefix: &str) {
        let trimmed = prefix.trim();
        if !trimmed.is_empty() {
            let canon = if trimmed == "*" || trimmed == "/" {
                trimmed.to_string()
            } else {
                normalize_vfs_path_lexical(trimmed)
            };
            if !self.allowed_write_prefixes.contains(&canon) {
                self.allowed_write_prefixes.push(canon);
            }
        }
    }

    /// Registers a virtual-only VFS mount prefix (in-memory readable/writable directory with zero
    /// host-disk fall-through). Wildcard `"*"`, root `"/"`, and rejected traversal/UNC/device
    /// paths are ignored.
    #[inline]
    pub fn push_virtual_mount(&mut self, prefix: &str) {
        let trimmed = prefix.trim();
        if trimmed.is_empty()
            || trimmed == "*"
            || trimmed == "/"
            || is_rejected_vfs_candidate(trimmed)
        {
            return;
        }
        let canon = normalize_vfs_path_lexical(trimmed);
        if canon.is_empty() || canon == "/" {
            return;
        }
        if !self.virtual_mounts.contains(&canon) {
            self.virtual_mounts.push(canon);
        }
    }

    #[inline]
    pub fn insert_net_host(&mut self, host: &str) {
        if let Some(canon) = canonicalize_net_grant_entry(host) {
            self.allowed_net_hosts.insert(canon);
        }
    }

    #[inline]
    pub fn push_exec_binary(&mut self, bin: impl Into<String>) {
        let raw = bin.into();
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let owned = trimmed.to_string();
            if !self.allowed_exec_binaries.contains(&owned) {
                self.allowed_exec_binaries.push(owned);
            }
        }
    }

    /// Grants access to a host extension (`"store"`) or a single method (`"store.get"`).
    /// Wildcards (`"*"`) and invalid empty/dot-bounded names are ignored.
    #[inline]
    pub fn push_extension(&mut self, grant: impl Into<String>) {
        let raw = grant.into();
        let trimmed = raw.trim();
        if trimmed.is_empty()
            || trimmed == "*"
            || trimmed.starts_with('.')
            || trimmed.ends_with('.')
            || trimmed.contains("..")
            || trimmed.bytes().any(|b| b.is_ascii_whitespace() || b == 0)
        {
            return;
        }
        let owned = trimmed.to_string();
        if !self.allowed_extensions.contains(&owned) {
            self.allowed_extensions.push(owned);
        }
    }

    pub fn with_read_prefix(mut self, prefix: impl Into<String>) -> Self {
        let raw = prefix.into();
        self.push_read_prefix(&raw);
        self
    }

    pub fn with_write_prefix(mut self, prefix: impl Into<String>) -> Self {
        let raw = prefix.into();
        self.push_write_prefix(&raw);
        self
    }

    pub fn with_virtual_mount(mut self, prefix: impl Into<String>) -> Self {
        let raw = prefix.into();
        self.push_virtual_mount(&raw);
        self
    }

    pub fn with_net_host(mut self, host: impl Into<String>) -> Self {
        let raw = host.into();
        self.insert_net_host(&raw);
        self
    }

    #[inline]
    pub fn set_allow_private_networks(&mut self, enabled: bool) {
        self.allow_private_networks = enabled;
    }

    #[inline]
    pub fn with_allow_private_networks(mut self, enabled: bool) -> Self {
        self.allow_private_networks = enabled;
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.allowed_env_keys.insert(key.into(), val.into());
        self
    }

    pub fn with_exec_binary(mut self, binary: impl Into<String>) -> Self {
        self.push_exec_binary(binary);
        self
    }

    /// Grants access to a host extension (`"store"`) or a single method (`"store.get"`).
    pub fn with_extension(mut self, grant: impl Into<String>) -> Self {
        self.push_extension(grant);
        self
    }

    #[inline]
    pub fn set_checkpoints(&mut self, enabled: bool) {
        self.allow_checkpoints = enabled;
        if !enabled {
            self.max_checkpoints = Some(0);
        } else if self.max_checkpoints == Some(0) {
            self.max_checkpoints = None;
        }
    }

    #[inline]
    pub fn set_max_checkpoints(&mut self, max: Option<u32>) {
        self.max_checkpoints = max;
        if max == Some(0) {
            self.allow_checkpoints = false;
        }
    }

    #[inline]
    pub fn with_checkpoints(mut self, enabled: bool) -> Self {
        self.set_checkpoints(enabled);
        self
    }

    #[inline]
    pub fn with_max_checkpoints(mut self, max: Option<u32>) -> Self {
        self.set_max_checkpoints(max);
        self
    }

    /// Produces a monotonically attenuated child [`CapabilityToken`] (`child <= self`) across
    /// `FsRead`, `FsWrite`, `NetCap`, `ExecCap`, `EnvCap`, and `Checkpoints`.
    pub fn attenuate(&self, req: &CapabilityAttenuationRequest<'_>) -> Self {
        let mut child = Self::pure_compute();

        match req.allow_read {
            Some(requested) => {
                for r in requested {
                    if !is_rejected_vfs_candidate(r)
                        && self
                            .allowed_read_prefixes
                            .iter()
                            .any(|p| vfs_prefix_matches(r, p))
                    {
                        child.push_read_prefix(r);
                    }
                }
            }
            None => {
                child.allowed_read_prefixes = self.allowed_read_prefixes.clone();
            }
        }

        match req.allow_write {
            Some(requested) => {
                for w in requested {
                    if !is_rejected_vfs_candidate(w)
                        && self
                            .allowed_write_prefixes
                            .iter()
                            .any(|p| vfs_prefix_matches(w, p))
                    {
                        child.push_write_prefix(w);
                    }
                }
            }
            None => {
                child.allowed_write_prefixes = self.allowed_write_prefixes.clone();
            }
        }

        // Virtual-only mounts attenuate strictly against parent's `virtual_mounts` (never widening
        // into host `allowed_read_prefixes` / `allowed_write_prefixes`).
        match req.virtual_mounts {
            Some(requested) => {
                for vm in requested {
                    if !is_rejected_vfs_candidate(vm)
                        && self
                            .virtual_mounts
                            .iter()
                            .any(|p| vfs_prefix_matches(vm, p))
                    {
                        child.push_virtual_mount(vm);
                    }
                }
            }
            None => {
                for pm in &self.virtual_mounts {
                    let read_ok = req
                        .allow_read
                        .is_none_or(|r| r.iter().any(|rp| vfs_prefix_matches(pm, rp)));
                    let write_ok = req
                        .allow_write
                        .is_none_or(|w| w.iter().any(|wp| vfs_prefix_matches(pm, wp)));
                    if read_ok && write_ok {
                        child.push_virtual_mount(pm);
                    }
                }
            }
        }

        let parent_nets_already_canonical = self
            .allowed_net_hosts
            .iter()
            .all(|s| is_already_canonical_net_grant(s));
        let owned_norm_parent_nets: FxHashSet<String>;
        let norm_parent_nets: &FxHashSet<String> = if parent_nets_already_canonical {
            &self.allowed_net_hosts
        } else {
            let mut set = FxHashSet::with_capacity_and_hasher(
                self.allowed_net_hosts.len(),
                Default::default(),
            );
            for raw_parent in &self.allowed_net_hosts {
                if let Some(canon) = canonicalize_net_grant_entry(raw_parent) {
                    set.insert(canon);
                }
            }
            owned_norm_parent_nets = set;
            &owned_norm_parent_nets
        };

        match req.allow_net {
            Some(requested) => {
                let parent_wildcard = norm_parent_nets.contains("*");
                for raw_req in requested {
                    let Some((canon_host, is_ipv6, port_opt)) = parse_net_grant_parts(raw_req)
                    else {
                        continue;
                    };
                    if canon_host == "*" {
                        if parent_wildcard {
                            child.allowed_net_hosts.insert("*".to_string());
                        }
                        continue;
                    }
                    if is_cloud_metadata_clean_host(&canon_host) {
                        continue;
                    }
                    let parent_has_bare = norm_parent_nets.contains(&canon_host);
                    let wildcard_allows_public =
                        parent_wildcard && !is_private_or_loopback_clean_host(&canon_host);
                    match port_opt {
                        None => {
                            if parent_has_bare || wildcard_allows_public {
                                child.allowed_net_hosts.insert(canon_host);
                            }
                        }
                        Some(port) => {
                            let formatted = if is_ipv6 {
                                format!("[{}]:{}", canon_host, port)
                            } else {
                                format!("{}:{}", canon_host, port)
                            };
                            if norm_parent_nets.contains(&formatted)
                                || parent_has_bare
                                || wildcard_allows_public
                            {
                                child.allowed_net_hosts.insert(formatted);
                            }
                        }
                    }
                }
            }
            None => {
                child.allowed_net_hosts = norm_parent_nets.clone();
            }
        }

        match req.allow_exec {
            Some(requested) => {
                let parent_wildcard = self.allowed_exec_binaries.iter().any(|a| a.trim() == "*");
                for req_bin in requested {
                    let trimmed = req_bin.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if trimmed == "*" {
                        if parent_wildcard {
                            child.push_exec_binary("*");
                        }
                    } else if self
                        .allowed_exec_binaries
                        .iter()
                        .any(|allowed| exec_binary_matches(trimmed, allowed))
                    {
                        child.push_exec_binary(trimmed);
                    }
                }
            }
            None => {
                child.allowed_exec_binaries = self.allowed_exec_binaries.clone();
            }
        }

        match req.allow_ext {
            Some(requested) => {
                for req_ext in requested {
                    let trimmed = req_ext.trim();
                    if trimmed.is_empty() || trimmed == "*" {
                        continue;
                    }
                    if self
                        .allowed_extensions
                        .iter()
                        .any(|parent_grant| extension_grant_matches(trimmed, parent_grant))
                    {
                        child.push_extension(trimmed);
                    }
                }
            }
            None => {
                child.allowed_extensions = self.allowed_extensions.clone();
            }
        }

        match req.allow_env {
            Some(requested) => {
                for k in requested {
                    let kt = k.trim();
                    if let Some(val) = self.allowed_env_keys.get(kt) {
                        child.allowed_env_keys.insert(kt.to_string(), val.clone());
                    }
                }
            }
            None => {
                child.allowed_env_keys = self.allowed_env_keys.clone();
            }
        }

        let parent_rem_cp = self
            .max_checkpoints
            .map(|m| m.saturating_sub(req.consumed_checkpoints));
        let child_allow_cp = self.allow_checkpoints
            && req.allow_checkpoints.unwrap_or(true)
            && req.max_checkpoints != Some(0)
            && parent_rem_cp != Some(0);
        let child_max_cp = if !child_allow_cp {
            Some(0)
        } else {
            match (parent_rem_cp, req.max_checkpoints) {
                (Some(p), Some(c)) => Some(p.min(c)),
                (Some(p), None) => Some(p),
                (None, Some(c)) => Some(c),
                (None, None) => None,
            }
        };
        child.allow_private_networks =
            self.allow_private_networks && req.allow_private_networks.unwrap_or(true);
        child.allow_checkpoints = child_allow_cp;
        child.max_checkpoints = child_max_cp;

        child
    }
}
