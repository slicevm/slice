//! Subprocess `ExecCap` Binary Matching & Shell Metacharacter Injection Guard.

use crate::vfs::has_dotdot_segment;

#[inline]
fn is_non_backslash_shell_metachar(b: u8) -> bool {
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
}

/// Returns `true` if `s` contains any shell command-injection metacharacter, quote, backslash
/// escape, or ASCII control byte (`< 0x20` or `0x7f`).
#[inline]
pub fn has_exec_shell_metachar(s: &str) -> bool {
    s.bytes()
        .any(|b| b == b'\\' || is_non_backslash_shell_metachar(b))
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
        || req.bytes().any(is_non_backslash_shell_metachar)
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
    !req_norm.contains('/')
        && allow_norm
            .rsplit('/')
            .next()
            .is_some_and(|allow_base| req_norm == allow_base)
}
