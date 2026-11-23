//! VFS Candidate Path Guards (`..`, NUL, NT/Win32 Device Namespaces, UNC Shares, Drive-Relative Paths).

#[inline]
pub(crate) fn has_dotdot_segment(path: &str) -> bool {
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

#[inline]
fn is_win32_reserved_device_stem(stem: &[u8]) -> bool {
    match stem.len() {
        3 => {
            stem.eq_ignore_ascii_case(b"CON")
                || stem.eq_ignore_ascii_case(b"PRN")
                || stem.eq_ignore_ascii_case(b"AUX")
                || stem.eq_ignore_ascii_case(b"NUL")
        }
        4 => {
            (stem[..3].eq_ignore_ascii_case(b"COM") || stem[..3].eq_ignore_ascii_case(b"LPT"))
                && matches!(stem[3], b'1'..=b'9')
        }
        5 => {
            // Win32 also reserves ISO-8859-1 / UTF-8 superscript digits 1..3 (`\u{00b9}`, `\u{00b2}`, `\u{00b3}`)
            (stem[..3].eq_ignore_ascii_case(b"COM") || stem[..3].eq_ignore_ascii_case(b"LPT"))
                && stem[3] == 0xc2
                && matches!(stem[4], 0xb9 | 0xb2 | 0xb3)
        }
        6 => stem.eq_ignore_ascii_case(b"CONIN$"),
        7 => stem.eq_ignore_ascii_case(b"CONOUT$"),
        _ => false,
    }
}

#[inline]
fn has_win32_reserved_device_segment(trimmed: &str) -> bool {
    let mut b = trimmed.as_bytes();
    if b.len() >= 4
        && (b[0] == b'\\' || b[0] == b'/')
        && (b[1] == b'\\' || b[1] == b'/')
        && b[2] == b'?'
        && (b[3] == b'\\' || b[3] == b'/')
    {
        b = &b[4..];
    }
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        b = &b[2..];
    }
    let mut i = 0usize;
    while i < b.len() {
        let start = i;
        while i < b.len() && b[i] != b'/' && b[i] != b'\\' {
            i += 1;
        }
        let mut seg = &b[start..i];
        i += 1;
        while let Some(&last) = seg.last() {
            if last == b' ' || last == b'.' {
                seg = &seg[..seg.len() - 1];
            } else {
                break;
            }
        }
        if seg.len() < 3 {
            continue;
        }
        let first_upper = seg[0] & !0x20;
        if !matches!(first_upper, b'C' | b'P' | b'A' | b'N' | b'L') {
            continue;
        }
        let mut stem_end = seg.len();
        let mut j = 0usize;
        while j < seg.len() {
            if seg[j] == b'.' || seg[j] == b':' {
                stem_end = j;
                break;
            }
            j += 1;
        }
        let mut stem = &seg[..stem_end];
        while stem.last() == Some(&b' ') {
            stem = &stem[..stem.len() - 1];
        }
        if is_win32_reserved_device_stem(stem) {
            return true;
        }
    }
    false
}

/// Checks whether a raw VFS candidate path contains forbidden sequences (`SBX-SEC-01`):
/// - `..` parent traversal segments (allowing non-traversal filenames like `[...slug].tsx`)
/// - Null bytes (`\0`)
/// - NT native namespace prefixes (`\??\` or `/??/`)
/// - Win32 device namespace prefixes (`\\.\`, `//./`, or non-drive `\\?\` / `//?/` such as `GLOBALROOT` or `UNC`)
/// - Raw or mixed-slash Windows SMB UNC network share paths (`\\server\share`, `\/server\share`, `/\server\share`)
/// - Win32 reserved console/serial/parallel device basenames (`CON`, `PRN`, `AUX`, `NUL`, `CONIN$`, `CONOUT$`, `COM1..9`, `LPT1..9`)
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

    // Reject raw or mixed-slash Windows UNC network shares (`\\server\share`, `\/server\share`, `/\server\share`)
    // unless followed by a valid extended local drive (`?\X:\`)
    if tb.len() >= 2
        && (tb[0] == b'\\' || tb[0] == b'/')
        && (tb[1] == b'\\' || tb[1] == b'/')
        && (tb[0] == b'\\' || tb[1] == b'\\')
    {
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

    has_win32_reserved_device_segment(trimmed)
}
