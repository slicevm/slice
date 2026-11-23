//! Lexical VFS Path Normalization & Cached Workspace CWD Resolution.

use std::borrow::Cow;

/// Fast zero-allocation check for paths that are already in canonical lexical VFS form (`SBX-PERF-02`).
#[inline]
pub(super) fn is_already_canonical_vfs_path(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() {
        return false;
    }
    if b == b"." || b == b"/" {
        return true;
    }
    let start_idx = if b[0] == b'/' {
        1
    } else if b.len() >= 3 && b[0].is_ascii_uppercase() && b[1] == b':' && b[2] == b'/' {
        if b.len() == 3 {
            return true;
        }
        3
    } else if b[0] != b'.'
        && b[0] != b'\\'
        && (!b[0].is_ascii_alphabetic() || b.get(1) != Some(&b':'))
    {
        0
    } else {
        return false;
    };

    if b[b.len() - 1] == b'/' || b[b.len() - 1] == b'.' {
        return false;
    }

    let mut prev_slash = start_idx > 0;
    let mut i = start_idx;
    while i < b.len() {
        let c = b[i];
        if c == b'\\' || c == 0 {
            return false;
        }
        if c == b'/' {
            if prev_slash {
                return false;
            }
            prev_slash = true;
        } else {
            if prev_slash && c == b'.' && (i + 1 == b.len() || b[i + 1] == b'/' || b[i + 1] == b'.')
            {
                return false;
            }
            prev_slash = false;
        }
        i += 1;
    }
    true
}

/// Returns a `Cow<'_, str>` containing the lexically normalized VFS path (`SBX-PERF-02`).
/// Borrows `raw` with zero heap allocations whenever `raw` is already in canonical form.
#[inline]
pub fn normalize_vfs_path_cow(raw: &str) -> Cow<'_, str> {
    let trimmed = raw.trim();
    if is_already_canonical_vfs_path(trimmed) {
        return Cow::Borrowed(trimmed);
    }
    Cow::Owned(normalize_vfs_path_lexical(trimmed))
}

/// Lexically normalizes a VFS or host filesystem path into a canonical forward-slash representation:
/// - Converts Windows `\` separators to `/`.
/// - Strips Win32 extended-length prefix (`//?/` or `\\?\`).
/// - Normalizes Windows drive-letter casing to uppercase (`c:/` -> `C:/`).
/// - Resolves `.` and `..` segments lexically on a stack-allocated `SmallVec` so path traversal
///   sequences like `/tmp/../etc/passwd` resolve to `/etc/passwd` **before** any prefix check.
pub fn normalize_vfs_path_lexical(raw: &str) -> String {
    use cow_utils::CowUtils;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return ".".to_string();
    }
    if is_already_canonical_vfs_path(trimmed) {
        return trimmed.to_string();
    }
    let slash_cow = trimmed.cow_replace('\\', "/");
    let mut s = slash_cow.as_ref();
    if let Some(stripped) = s.strip_prefix("//?/") {
        if let Some(unc_rest) = stripped
            .strip_prefix("UNC/")
            .or_else(|| stripped.strip_prefix("unc/"))
        {
            // Keep UNC paths anchored at root (`//UNC/...`) so they never become relative paths
            let mut unc_out = String::with_capacity(unc_rest.len() + 6);
            unc_out.push_str("//UNC/");
            unc_out.push_str(unc_rest.trim_start_matches('/'));
            return unc_out;
        }
        s = stripped;
    }

    let (root_prefix, rest, is_abs) =
        if s.len() >= 2 && s.as_bytes()[0].is_ascii_alphabetic() && s.as_bytes()[1] == b':' {
            let drive = (s.as_bytes()[0].to_ascii_uppercase()) as char;
            let after_colon = &s[2..];
            let after_slash = after_colon.trim_start_matches('/');
            (Some(drive), after_slash, true)
        } else if s.starts_with('/') {
            (None, s.trim_start_matches('/'), true)
        } else {
            (None, s, false)
        };

    let mut stack: smallvec::SmallVec<[&str; 16]> = smallvec::SmallVec::new();
    for seg in rest.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            if let Some(&last) = stack.last()
                && last != ".."
            {
                stack.pop();
                continue;
            }
            if !is_abs {
                stack.push("..");
            }
        } else {
            stack.push(seg);
        }
    }

    let mut out = String::with_capacity(s.len().max(2));
    if let Some(drive) = root_prefix {
        out.push(drive);
        out.push(':');
        out.push('/');
        for (i, seg) in stack.iter().enumerate() {
            if i > 0 {
                out.push('/');
            }
            out.push_str(seg);
        }
    } else if is_abs {
        out.push('/');
        for (i, seg) in stack.iter().enumerate() {
            if i > 0 {
                out.push('/');
            }
            out.push_str(seg);
        }
    } else if stack.is_empty() {
        out.push('.');
    } else {
        for (i, seg) in stack.iter().enumerate() {
            if i > 0 {
                out.push('/');
            }
            out.push_str(seg);
        }
    }
    out
}

#[inline]
pub fn is_abs_normalized_vfs_path(norm: &str) -> bool {
    norm.starts_with('/') || (norm.len() >= 3 && norm.as_bytes()[1] == b':')
}

pub(super) struct CachedWorkspaceCwd {
    pub(super) canonical: String,
    pub(super) lexical_alias: Option<String>,
}

#[inline]
pub(super) fn workspace_cwd_info() -> &'static CachedWorkspaceCwd {
    static CWD_INFO: std::sync::OnceLock<CachedWorkspaceCwd> = std::sync::OnceLock::new();
    CWD_INFO.get_or_init(|| match std::env::current_dir() {
        Ok(raw_cwd) => {
            let lexical = normalize_vfs_path_lexical(&raw_cwd.to_string_lossy());
            let canonical = std::fs::canonicalize(&raw_cwd)
                .map(|c| normalize_vfs_path_lexical(&c.to_string_lossy()))
                .unwrap_or_else(|_| lexical.clone());
            let lexical_alias = if lexical != canonical {
                Some(lexical)
            } else {
                None
            };
            CachedWorkspaceCwd {
                canonical,
                lexical_alias,
            }
        }
        Err(_) => CachedWorkspaceCwd {
            canonical: "/workspace".to_string(),
            lexical_alias: None,
        },
    })
}

/// Returns the process's canonical forward-slash workspace directory, cached once per process.
#[inline]
pub fn cached_normalized_cwd() -> &'static str {
    workspace_cwd_info().canonical.as_str()
}

/// Normalizes `raw` lexically and anchors relative paths (`"data/rel.txt"`, `"./a"`, `"."`)
/// against [`cached_normalized_cwd`] so relative and canonical absolute VFS paths always map
/// to the exact same canonical VFS key without per-call OS syscalls.
#[inline]
pub fn anchor_vfs_path_to_cwd(raw: &str) -> String {
    let norm = normalize_vfs_path_lexical(raw);
    let cwd_info = workspace_cwd_info();
    if is_abs_normalized_vfs_path(&norm) {
        if let Some(ref lex) = cwd_info.lexical_alias {
            if norm == *lex {
                return cwd_info.canonical.clone();
            }
            if let Some(rem) = norm.strip_prefix(lex.as_str())
                && rem.starts_with('/')
            {
                let mut mapped = String::with_capacity(cwd_info.canonical.len() + rem.len());
                mapped.push_str(&cwd_info.canonical);
                mapped.push_str(rem);
                return mapped;
            }
        }
        return norm;
    }
    let base = cwd_info.canonical.as_str();
    if norm == "." {
        return base.to_string();
    }
    if norm == ".." || norm.starts_with("../") || norm.contains("/..") {
        let mut combined = String::with_capacity(base.len() + 1 + norm.len());
        combined.push_str(base);
        if !base.ends_with('/') {
            combined.push('/');
        }
        combined.push_str(&norm);
        return normalize_vfs_path_lexical(&combined);
    }
    let need_sep = !base.ends_with('/');
    let mut out = String::with_capacity(base.len() + usize::from(need_sep) + norm.len());
    out.push_str(base);
    if need_sep {
        out.push('/');
    }
    out.push_str(&norm);
    out
}
