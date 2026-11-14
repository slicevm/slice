//! Attenuated Virtual Filesystem (VFS) Path Normalization & Subprocess `ExecCap` Verification.

use std::borrow::Cow;

#[inline]
fn has_dotdot_segment(path: &str) -> bool {
    if memchr::memmem::find(path.as_bytes(), b"..").is_none() {
        return false;
    }
    let trimmed = path.trim();
    let rest = if trimmed.len() >= 2
        && trimmed.as_bytes()[0].is_ascii_alphabetic()
        && trimmed.as_bytes()[1] == b':'
    {
        &trimmed[2..]
    } else {
        trimmed
    };
    rest.split(['/', '\\']).any(|seg| seg == "..")
}

/// Checks whether a raw VFS candidate path contains forbidden sequences (`SBX-SEC-01`):
/// - `..` parent traversal segments (allowing non-traversal filenames like `[...slug].tsx`)
/// - Null bytes (`\0`)
/// - NT native namespace prefixes (`\??\` or `/??/`)
/// - Win32 device namespace prefixes (`\\.\`, `//./`, or non-drive `\\?\` / `//?/` such as `GLOBALROOT` or `UNC`)
/// - Raw Windows SMB UNC network share paths (`\\server\share`)
/// - Windows drive-relative paths (`C:file.txt` without a root slash after `:`)
#[inline]
pub fn is_rejected_vfs_candidate(candidate_path: &str) -> bool {
    let bytes = candidate_path.as_bytes();
    if memchr::memchr(0, bytes).is_some() || has_dotdot_segment(candidate_path) {
        return true;
    }
    let trimmed = candidate_path.trim_start();
    let tb = trimmed.as_bytes();

    // Reject Windows drive-relative paths (`C:secret.txt` or bare `C:`) across all platforms
    if tb.len() >= 2
        && tb[0].is_ascii_alphabetic()
        && tb[1] == b':'
        && !matches!(tb.get(2), Some(b'/' | b'\\'))
    {
        return true;
    }

    // Reject NT native `\??\` / `/??/` namespace
    if tb.len() >= 3
        && (tb[0] == b'\\' || tb[0] == b'/')
        && tb[1] == b'?'
        && tb[2] == b'?'
        && (tb.len() == 3 || tb[3] == b'\\' || tb[3] == b'/')
    {
        return true;
    }

    if tb.len() >= 3 && (tb[0] == b'\\' || tb[0] == b'/') && (tb[1] == b'\\' || tb[1] == b'/') {
        // Reject `\\.\` or `//./` Win32 device namespace
        if tb[2] == b'.' && (tb.len() == 3 || tb[3] == b'\\' || tb[3] == b'/') {
            return true;
        }
        // Reject `\\?\` or `//?/` extended prefix UNLESS followed by an absolute drive root (`X:\` or `X:/`)
        if tb[2] == b'?' && (tb.len() == 3 || tb[3] == b'\\' || tb[3] == b'/') {
            let is_valid_extended_drive = tb.len() >= 7
                && tb[4].is_ascii_alphabetic()
                && tb[5] == b':'
                && (tb[6] == b'\\' || tb[6] == b'/');
            if !is_valid_extended_drive {
                return true;
            }
        }
    }

    // Reject raw Windows UNC network shares `\\server\share` (two leading backslashes not followed by `?\X:\`)
    if tb.len() >= 2 && tb[0] == b'\\' && tb[1] == b'\\' {
        let is_extended_drive = tb.len() >= 7
            && tb[2] == b'?'
            && (tb[3] == b'\\' || tb[3] == b'/')
            && tb[4].is_ascii_alphabetic()
            && tb[5] == b':'
            && (tb[6] == b'\\' || tb[6] == b'/');
        if !is_extended_drive {
            return true;
        }
    }
    false
}

/// Fast zero-allocation check for paths that are already in canonical lexical VFS form (`SBX-PERF-02`).
#[inline]
fn is_already_canonical_vfs_path(s: &str) -> bool {
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
            if let Some(&last) = stack.last() {
                if last != ".." {
                    stack.pop();
                    continue;
                }
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

struct CachedWorkspaceCwd {
    canonical: String,
    lexical_alias: Option<String>,
}

#[inline]
fn workspace_cwd_info() -> &'static CachedWorkspaceCwd {
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
            if let Some(rem) = norm.strip_prefix(lex.as_str()) {
                if rem.starts_with('/') {
                    let mut mapped = String::with_capacity(cwd_info.canonical.len() + rem.len());
                    mapped.push_str(&cwd_info.canonical);
                    mapped.push_str(rem);
                    return mapped;
                }
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
    if let Some(rem) = norm_candidate.strip_prefix(norm_prefix) {
        return norm_prefix.ends_with('/') || rem.starts_with('/');
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
    if path_starts_with_dir_boundary(norm_cand, &norm_pref) {
        return true;
    }

    // Cross-platform `/tmp` <-> `std::env::temp_dir()` equivalence (cached once per process)
    if norm_pref == "/tmp" || norm_pref.starts_with("/tmp/") {
        static NORMALIZED_OS_TMP: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let os_tmp = NORMALIZED_OS_TMP
            .get_or_init(|| normalize_vfs_path_lexical(&std::env::temp_dir().to_string_lossy()));
        if !os_tmp.is_empty() {
            let mapped_pref: Cow<'_, str> = if norm_pref == "/tmp" {
                Cow::Borrowed(os_tmp.as_str())
            } else {
                Cow::Owned(format!("{}{}", os_tmp, &norm_pref[4..]))
            };
            if path_starts_with_dir_boundary(norm_cand, &mapped_pref) {
                return true;
            }
        }
    }

    // Relative-vs-absolute workspace resolution against cached `current_dir()`
    let cand_is_abs = is_abs_normalized_vfs_path(norm_cand);
    let pref_is_abs = is_abs_normalized_vfs_path(&norm_pref);
    let cwd_info = workspace_cwd_info();
    if cand_is_abs != pref_is_abs {
        let check_against_base = |norm_cwd: &str| -> bool {
            let abs_cand: Cow<'_, str> = if cand_is_abs {
                Cow::Borrowed(norm_cand)
            } else if norm_cand == "." {
                Cow::Borrowed(norm_cwd)
            } else {
                let sep = if norm_cwd.ends_with('/') { "" } else { "/" };
                Cow::Owned(format!("{}{}{}", norm_cwd, sep, norm_cand))
            };
            let abs_pref: Cow<'_, str> = if pref_is_abs {
                Cow::Borrowed(norm_pref.as_ref())
            } else if norm_pref == "." {
                Cow::Borrowed(norm_cwd)
            } else {
                let sep = if norm_cwd.ends_with('/') { "" } else { "/" };
                Cow::Owned(format!("{}{}{}", norm_cwd, sep, norm_pref))
            };
            path_starts_with_dir_boundary(&abs_cand, &abs_pref)
        };
        if check_against_base(&cwd_info.canonical) {
            return true;
        }
        if let Some(ref lex) = cwd_info.lexical_alias {
            if check_against_base(lex) {
                return true;
            }
        }
    } else if cand_is_abs && pref_is_abs {
        if let Some(ref lex) = cwd_info.lexical_alias {
            let remap = |p: &str| -> Option<String> {
                if p == lex {
                    Some(cwd_info.canonical.clone())
                } else if let Some(rem) = p.strip_prefix(lex.as_str()) {
                    if rem.starts_with('/') {
                        Some(format!("{}{}", cwd_info.canonical, rem))
                    } else {
                        None
                    }
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
    }
    false
}

/// Verifies whether `candidate_path` is authorized by `allowed_prefix` under OCap VFS rules:
/// 1. Rejects any raw path containing `..` traversal segments, null bytes, Win32 device paths, or UNC shares (`SBX-SEC-01`).
/// 2. Lexically normalizes both `candidate_path` and `allowed_prefix` via [`normalize_vfs_path_cow`].
/// 3. Enforces strict directory-boundary matching (`/tmp` matches `/tmp` and `/tmp/foo`, but NEVER `/tmp_evil`).
/// 4. Maps `/tmp` <-> `std::env::temp_dir()` seamlessly across Windows, Linux, and macOS.
/// 5. Resolves relative capability prefixes (`"."`, `"./data"`) or relative candidate paths against
///    `std::env::current_dir()` when comparing relative and absolute paths.
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

/// Checks whether a requested subprocess binary (`requested`) matches an allowlisted
/// `ExecCap` entry (`allowed`).
///
/// Security semantics (`SBX-SEC-05`):
/// - Rejects any `requested` string containing `..` path traversal, null bytes, control chars,
///   or shell command-injection metacharacters (`;`, `|`, `&`, `` ` ``, `$`, `>`, `<`, `\n`, `\r`).
/// - If `allowed == "*"`, allows any non-empty binary without forbidden metacharacters or `..`.
/// - If `requested` contains a path separator (`/` or `\`), it must match the normalized path
///   of `allowed` (preventing `/tmp/evil/git` from matching `/usr/bin/git`).
/// - If `requested` is a bare command name (e.g. `"git"` or `"echo"`), it matches if `allowed`
///   is either that bare name or an explicit path whose basename is that name (e.g. `"/usr/bin/git"`).
#[inline]
pub fn exec_binary_matches(requested: &str, allowed: &str) -> bool {
    let req = requested.trim();
    let allow = allowed.trim();
    if req.is_empty()
        || allow.is_empty()
        || has_dotdot_segment(req)
        || req.bytes().any(|b| {
            b < 0x20
                || matches!(
                    b,
                    b';' | b'|'
                        | b'&'
                        | b'`'
                        | b'$'
                        | b'>'
                        | b'<'
                        | b'('
                        | b')'
                        | b'*'
                        | b'?'
                        | b'['
                        | b']'
                        | b'{'
                        | b'}'
                        | b'~'
                        | b'!'
                        | b'#'
                        | b'\''
                        | b'"'
                        | b'\x7f'
                )
        })
    {
        return false;
    }
    if allow == "*" {
        return true;
    }
    let norm_bin = |s: &str| -> String {
        let replaced = s.replace('\\', "/");
        let trimmed = replaced.trim_end_matches('/');
        let lower = trimmed.to_ascii_lowercase();
        lower.strip_suffix(".exe").unwrap_or(&lower).to_string()
    };
    let req_norm = norm_bin(req);
    let allow_norm = norm_bin(allow);
    if req_norm == allow_norm {
        return true;
    }
    if !req_norm.contains('/') {
        if let Some(allow_base) = allow_norm.rsplit('/').next() {
            if req_norm == allow_base {
                return true;
            }
        }
    }
    false
}
