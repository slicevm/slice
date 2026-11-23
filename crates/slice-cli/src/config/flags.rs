//! Shared CLI flag classification, separate-argument consumption, and zero-allocation numeric parsers.

use anyhow::{Result, bail};
use std::path::PathBuf;

/// Zero-allocation parser that extracts ASCII digits from `raw` (ignoring underscores, spaces, or
/// unit suffixes) into a `u64`. Returns `None` if no digits are present or if overflow occurs.
#[cfg(any(test, target_os = "linux"))]
#[inline]
pub(crate) fn parse_u64_digits(raw: &str) -> Option<u64> {
    let mut acc: u64 = 0;
    let mut saw_digit = false;
    for &b in raw.as_bytes() {
        if b.is_ascii_digit() {
            saw_digit = true;
            acc = acc.checked_mul(10)?.checked_add((b - b'0') as u64)?;
        }
    }
    if saw_digit { Some(acc) } else { None }
}

/// Strict unsigned integer parser (`u64`) allowing optional `_` digit separators (`50_000_000`)
/// and surrounding quotes/whitespace, while rejecting any non-digit characters or unit suffixes.
#[inline]
pub(crate) fn parse_strict_u64(raw: &str) -> Option<u64> {
    let s = raw.trim().trim_matches('"').trim_matches('\'').trim();
    if s.is_empty() {
        return None;
    }
    let mut acc: u64 = 0;
    let mut saw_digit = false;
    for &b in s.as_bytes() {
        if b.is_ascii_digit() {
            saw_digit = true;
            acc = acc.checked_mul(10)?.checked_add((b - b'0') as u64)?;
        } else if b == b'_' {
            if !saw_digit {
                return None;
            }
        } else {
            return None;
        }
    }
    if saw_digit { Some(acc) } else { None }
}

/// Unit-aware byte-size parser supporting plain byte counts (`1048576`, `65_536`) as well as
/// case-insensitive `B`, `K`/`KB`/`KiB`, `M`/`MB`/`MiB`, and `G`/`GB`/`GiB` suffixes (`16MB`,
/// `64KB`, `2GiB`). Rejects malformed numbers, unknown suffixes, and `usize` overflow.
#[inline]
pub(crate) fn parse_byte_size_usize(raw: &str) -> Option<usize> {
    let s = raw.trim().trim_matches('"').trim_matches('\'').trim();
    if s.is_empty() {
        return None;
    }
    let bytes = s.as_bytes();
    let mut idx = 0;
    let mut count: u64 = 0;
    let mut saw_digit = false;
    while idx < bytes.len() {
        let b = bytes[idx];
        if b.is_ascii_digit() {
            saw_digit = true;
            count = count.checked_mul(10)?.checked_add((b - b'0') as u64)?;
            idx += 1;
        } else if b == b'_' {
            if !saw_digit {
                return None;
            }
            idx += 1;
        } else {
            break;
        }
    }
    if !saw_digit {
        return None;
    }
    let suffix = s[idx..].trim();
    let multiplier: u64 = if suffix.is_empty()
        || suffix.eq_ignore_ascii_case("b")
        || suffix.eq_ignore_ascii_case("bytes")
    {
        1
    } else if suffix.eq_ignore_ascii_case("k")
        || suffix.eq_ignore_ascii_case("kb")
        || suffix.eq_ignore_ascii_case("kib")
    {
        1024
    } else if suffix.eq_ignore_ascii_case("m")
        || suffix.eq_ignore_ascii_case("mb")
        || suffix.eq_ignore_ascii_case("mib")
    {
        1024 * 1024
    } else if suffix.eq_ignore_ascii_case("g")
        || suffix.eq_ignore_ascii_case("gb")
        || suffix.eq_ignore_ascii_case("gib")
    {
        1024 * 1024 * 1024
    } else {
        return None;
    };
    let total = count.checked_mul(multiplier)?;
    usize::try_from(total).ok()
}

/// Returns `true` when `arg` is a Slice runtime/server flag that always takes a separate value
/// token (`--flag <value>`). Boolean/dual flags (`--checkpoints`, `--deterministic`, `--boacas`)
/// are intentionally excluded so `slice run --checkpoints app.ts` never consumes `app.ts`.
#[inline]
pub(crate) fn runtime_flag_takes_separate_value(arg: &str) -> bool {
    matches!(
        arg,
        "--config"
            | "--env-file"
            | "--audit-egress-file"
            | "--audit-egress"
            | "--trace"
            | "--allow-read"
            | "--allow-write"
            | "--allow-net"
            | "--allow-exec"
            | "--allow-env"
            | "--fuel"
            | "--memory"
            | "--memory-kb"
            | "--memory-mb"
            | "--max-checkpoints"
            | "--tier"
            | "--port"
            | "--host"
            | "--workers"
            | "--static"
            | "--tls-cert"
            | "--tls-key"
    )
}

/// Validates that `arg_idx + 1` exists in `args` for a space-separated runtime flag and advances
/// `arg_idx` past the value token (`+2` total in caller's `while` loop when combined with `continue`).
#[inline]
pub(crate) fn skip_runtime_flag_with_separate_value(
    args: &[String],
    arg_idx: &mut usize,
) -> Result<bool> {
    let arg = &args[*arg_idx];
    if !runtime_flag_takes_separate_value(arg) {
        return Ok(false);
    }
    if *arg_idx + 1 >= args.len() {
        bail!("Flag {} requires a value argument", arg);
    }
    *arg_idx += 2;
    Ok(true)
}

/// Extracts the value for `flag` in either `--flag=<val>` or `--flag <val>` form, advancing
/// `arg_idx` by `+1` extra when the space-separated form is matched. Rejects missing or
/// whitespace-only values fail-closed.
pub(crate) fn match_non_empty_flag_value<'a>(
    args: &'a [String],
    arg_idx: &mut usize,
    flag: &str,
) -> Result<Option<&'a str>> {
    let arg = &args[*arg_idx];
    if arg == flag {
        let Some(next) = args.get(*arg_idx + 1) else {
            bail!("Flag {} requires a non-empty value", flag);
        };
        let trimmed = next.trim();
        if trimmed.is_empty() {
            bail!("Flag {} requires a non-empty value", flag);
        }
        *arg_idx += 1;
        return Ok(Some(trimmed));
    }
    if let Some(rest) = arg.strip_prefix(flag)
        && let Some(val) = rest.strip_prefix('=')
    {
        let trimmed = val.trim();
        if trimmed.is_empty() {
            bail!("Flag {}= requires a non-empty value", flag);
        }
        return Ok(Some(trimmed));
    }
    Ok(None)
}

/// Parses a required non-empty filesystem `PathBuf` for `flag` (`--config`, `--env-file`,
/// `--audit-egress-file`, `--trace`) in either `--flag=<path>` or `--flag <path>` form.
pub(crate) fn match_non_empty_flag_path(
    args: &[String],
    arg_idx: &mut usize,
    flag: &str,
) -> Result<Option<PathBuf>> {
    Ok(match_non_empty_flag_value(args, arg_idx, flag)?.map(PathBuf::from))
}

/// Returns `true` when `arg` is a recognized `slice` runtime / sandbox / server option (used to
/// separate runtime flags from guest `process.argv` script arguments and to reject unknown flags).
#[inline]
pub(crate) fn is_known_slice_runtime_flag(arg: &str) -> bool {
    matches!(
        arg,
        "--quiet"
            | "-q"
            | "--metrics"
            | "--watch"
            | "--watch-once"
            | "--warm"
            | "--no-warm"
            | "--boacas"
            | "--no-boacas"
            | "--exper-boacas"
            | "--no-exper-boacas"
            | "--deterministic"
            | "--no-deterministic"
            | "--checkpoints"
            | "--no-checkpoints"
            | "--trust-directives"
            | "--no-trust-directives"
            | "--no-directives"
            | "--untrusted"
    ) || runtime_flag_takes_separate_value(arg)
        || arg.starts_with("--allow-")
        || arg.starts_with("--config=")
        || arg.starts_with("--trust-directives=")
        || arg.starts_with("--fuel=")
        || arg.starts_with("--memory=")
        || arg.starts_with("--memory-kb=")
        || arg.starts_with("--memory-mb=")
        || arg.starts_with("--tier=")
        || arg.starts_with("--boacas=")
        || arg.starts_with("--exper-boacas=")
        || arg.starts_with("--deterministic=")
        || arg.starts_with("--env-file=")
        || arg.starts_with("--audit-egress=")
        || arg.starts_with("--audit-egress-file=")
        || arg.starts_with("--checkpoints=")
        || arg.starts_with("--max-checkpoints=")
        || arg.starts_with("--trace=")
        || arg.starts_with("--port=")
        || arg.starts_with("--host=")
        || arg.starts_with("--workers=")
        || arg.starts_with("--static=")
        || arg.starts_with("--tls-cert=")
        || arg.starts_with("--tls-key=")
}
