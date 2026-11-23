//! Directory-Boundary VFS Prefix Matching & Cross-Platform `/tmp` Alias Resolution.

use super::guard::is_rejected_vfs_candidate;
use super::normalize::{
    is_abs_normalized_vfs_path, is_already_canonical_vfs_path, normalize_vfs_path_cow,
    normalize_vfs_path_lexical, workspace_cwd_info,
};
use std::borrow::Cow;

#[inline]
fn path_starts_with_dir_boundary(norm_candidate: &str, norm_prefix: &str) -> bool {
    if norm_prefix == "*" || norm_prefix == "/" {
        return true;
    }
    if norm_candidate == norm_prefix {
        return true;
    }
    if norm_prefix == "." {
        return !norm_candidate.is_empty()
            && !is_abs_normalized_vfs_path(norm_candidate)
            && norm_candidate != ".."
            && !norm_candidate.starts_with("../");
    }
    norm_candidate
        .strip_prefix(norm_prefix)
        .is_some_and(|rem| norm_prefix.ends_with('/') || rem.starts_with('/'))
}

#[derive(Debug, Clone)]
pub(crate) struct CachedOsTmpInfo {
    pub(crate) primary: Option<String>,
    pub(crate) lexical_alias: Option<String>,
}

#[inline]
fn is_unsafe_root_tmp_dir(norm: &str) -> bool {
    if norm.is_empty()
        || norm == "/"
        || norm == "."
        || norm == "*"
        || norm == ".."
        || norm.starts_with("../")
    {
        return true;
    }
    let b = norm.as_bytes();
    b.len() == 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'/'
}

#[inline]
pub(crate) fn build_os_tmp_info(raw_tmp: &std::path::Path) -> CachedOsTmpInfo {
    let lex_raw = normalize_vfs_path_lexical(&raw_tmp.to_string_lossy());
    let lex_opt = (!is_unsafe_root_tmp_dir(&lex_raw)).then_some(lex_raw);
    let canon_opt = std::fs::canonicalize(raw_tmp)
        .ok()
        .map(|p| normalize_vfs_path_lexical(&p.to_string_lossy()))
        .filter(|c| !is_unsafe_root_tmp_dir(c));

    match (canon_opt, lex_opt) {
        (Some(canon), Some(lex)) if canon != lex => CachedOsTmpInfo {
            primary: Some(canon),
            lexical_alias: Some(lex),
        },
        (Some(canon), _) => CachedOsTmpInfo {
            primary: Some(canon),
            lexical_alias: None,
        },
        (None, Some(lex)) => CachedOsTmpInfo {
            primary: Some(lex),
            lexical_alias: None,
        },
        (None, None) => CachedOsTmpInfo {
            primary: None,
            lexical_alias: None,
        },
    }
}

#[inline]
fn os_tmp_info() -> &'static CachedOsTmpInfo {
    static OS_TMP_INFO: std::sync::OnceLock<CachedOsTmpInfo> = std::sync::OnceLock::new();
    OS_TMP_INFO.get_or_init(|| build_os_tmp_info(&std::env::temp_dir()))
}

#[inline]
fn matches_bidirectional_tmp_alias(
    norm_cand: &str,
    norm_pref: &str,
    tmp_info: &CachedOsTmpInfo,
) -> bool {
    let aliases = [
        tmp_info.primary.as_deref(),
        tmp_info.lexical_alias.as_deref(),
    ];
    if norm_pref == "/tmp" || norm_pref.starts_with("/tmp/") {
        for os_tmp in aliases.into_iter().flatten() {
            let mapped_pref: Cow<'_, str> = if norm_pref == "/tmp" {
                Cow::Borrowed(os_tmp)
            } else {
                Cow::Owned(format!("{}{}", os_tmp, &norm_pref[4..]))
            };
            if path_starts_with_dir_boundary(norm_cand, &mapped_pref) {
                return true;
            }
        }
    }

    for (idx, os_tmp_opt) in aliases.iter().enumerate() {
        let Some(os_tmp) = *os_tmp_opt else {
            continue;
        };
        let suffix_opt = if norm_pref == os_tmp {
            Some("")
        } else if let Some(rem) = norm_pref.strip_prefix(os_tmp)
            && rem.starts_with('/')
        {
            Some(rem)
        } else {
            None
        };
        if let Some(suffix) = suffix_opt {
            let posix_pref: Cow<'_, str> = if suffix.is_empty() {
                Cow::Borrowed("/tmp")
            } else {
                Cow::Owned(format!("/tmp{}", suffix))
            };
            if path_starts_with_dir_boundary(norm_cand, &posix_pref) {
                return true;
            }
            if let Some(other_alias) = aliases[1 - idx] {
                let cross_pref: Cow<'_, str> = if suffix.is_empty() {
                    Cow::Borrowed(other_alias)
                } else {
                    Cow::Owned(format!("{}{}", other_alias, suffix))
                };
                if path_starts_with_dir_boundary(norm_cand, &cross_pref) {
                    return true;
                }
            }
        }
    }
    false
}

#[inline]
fn anchor_rel_to_base<'a>(norm_path: &'a str, is_abs: bool, norm_cwd: &'a str) -> Cow<'a, str> {
    if is_abs {
        return Cow::Borrowed(norm_path);
    }
    if norm_path == "." {
        return Cow::Borrowed(norm_cwd);
    }
    let sep = if norm_cwd.ends_with('/') { "" } else { "/" };
    let joined = format!("{}{}{}", norm_cwd, sep, norm_path);
    if norm_path == ".." || norm_path.starts_with("../") || norm_path.contains("/..") {
        Cow::Owned(normalize_vfs_path_lexical(&joined))
    } else {
        Cow::Owned(joined)
    }
}

#[inline]
pub(crate) fn vfs_normalized_candidate_matches_prefix_with_tmp(
    raw_norm_cand: &str,
    allowed_prefix: &str,
    tmp_info: &CachedOsTmpInfo,
) -> bool {
    let trimmed_pref = allowed_prefix.trim();
    if trimmed_pref.is_empty() {
        return false;
    }
    if trimmed_pref == "*" || trimmed_pref == "/" {
        return true;
    }
    let norm_cand_cow = normalize_vfs_path_cow(raw_norm_cand);
    let norm_cand = norm_cand_cow.as_ref();
    let norm_pref = normalize_vfs_path_cow(trimmed_pref);
    if path_starts_with_dir_boundary(norm_cand, &norm_pref) {
        return true;
    }

    if matches_bidirectional_tmp_alias(norm_cand, &norm_pref, tmp_info) {
        return true;
    }

    let cand_is_abs = is_abs_normalized_vfs_path(norm_cand);
    let pref_is_abs = is_abs_normalized_vfs_path(&norm_pref);
    let cand_has_dotdot = !cand_is_abs && (norm_cand == ".." || norm_cand.starts_with("../"));
    let pref_has_dotdot = !pref_is_abs && (norm_pref == ".." || norm_pref.starts_with("../"));
    let cwd_info = workspace_cwd_info();
    if cand_is_abs != pref_is_abs || cand_has_dotdot || pref_has_dotdot {
        let check_against_base = |norm_cwd: &str| -> bool {
            let abs_cand = anchor_rel_to_base(norm_cand, cand_is_abs, norm_cwd);
            let abs_pref = anchor_rel_to_base(&norm_pref, pref_is_abs, norm_cwd);
            path_starts_with_dir_boundary(&abs_cand, &abs_pref)
        };
        if check_against_base(&cwd_info.canonical) {
            return true;
        }
        if let Some(ref lex) = cwd_info.lexical_alias
            && check_against_base(lex)
        {
            return true;
        }
    } else if cand_is_abs
        && pref_is_abs
        && let Some(ref lex) = cwd_info.lexical_alias
    {
        let remap = |p: &str| -> Option<String> {
            if p == lex {
                Some(cwd_info.canonical.clone())
            } else if let Some(rem) = p.strip_prefix(lex.as_str()) {
                rem.starts_with('/')
                    .then(|| format!("{}{}", cwd_info.canonical, rem))
            } else {
                None
            }
        };
        let mapped_cand = remap(norm_cand);
        let mapped_pref = remap(&norm_pref);
        if mapped_cand.is_some() || mapped_pref.is_some() {
            let c = mapped_cand.as_deref().unwrap_or(norm_cand);
            let p = mapped_pref.as_deref().unwrap_or(&norm_pref);
            if path_starts_with_dir_boundary(c, p) {
                return true;
            }
        }
    }
    false
}

/// Checks a pre-normalized candidate path (`norm_cand`) against `allowed_prefix`.
#[inline]
pub fn vfs_normalized_candidate_matches_prefix(norm_cand: &str, allowed_prefix: &str) -> bool {
    let trimmed_pref = allowed_prefix.trim();
    if trimmed_pref.is_empty() {
        return false;
    }
    if trimmed_pref == "*" || trimmed_pref == "/" {
        return true;
    }
    let norm_pref = normalize_vfs_path_cow(trimmed_pref);
    if is_already_canonical_vfs_path(norm_cand)
        && path_starts_with_dir_boundary(norm_cand, &norm_pref)
    {
        return true;
    }
    vfs_normalized_candidate_matches_prefix_with_tmp(norm_cand, trimmed_pref, os_tmp_info())
}

/// Verifies whether `candidate_path` is authorized by `allowed_prefix` under OCap VFS rules.
#[inline]
pub fn vfs_prefix_matches(candidate_path: &str, allowed_prefix: &str) -> bool {
    if is_rejected_vfs_candidate(candidate_path) {
        return false;
    }
    let trimmed_pref = allowed_prefix.trim();
    if trimmed_pref.is_empty() {
        return false;
    }
    if trimmed_pref == "*" || trimmed_pref == "/" {
        return true;
    }
    let norm_cand = normalize_vfs_path_cow(candidate_path);
    vfs_normalized_candidate_matches_prefix(&norm_cand, trimmed_pref)
}

/// Verifies whether `path` is authorized by any prefix in `prefixes`, rejecting traversal/device
/// paths and normalizing `path` at most once across all prefixes.
#[inline]
pub fn vfs_path_matches_any_prefix(path: &str, prefixes: &[String]) -> bool {
    if prefixes.is_empty() || is_rejected_vfs_candidate(path) {
        return false;
    }
    let norm_cand = normalize_vfs_path_cow(path);
    prefixes
        .iter()
        .any(|prefix| vfs_normalized_candidate_matches_prefix(&norm_cand, prefix))
}
