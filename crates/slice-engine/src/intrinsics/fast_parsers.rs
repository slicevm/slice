//! Fast Zero-Allocation String Parsers & Formatters (`ms`, `bytes`, and Strict/Loose `SemVer`).

#[inline]
pub fn fast_parse_ms(input: &str) -> Option<f64> {
    let s = input.trim();
    if s.is_empty() || s.len() > 100 {
        return None;
    }
    let bytes = s.as_bytes();
    let mut num_end = 0;
    if bytes[0] == b'-' || bytes[0] == b'+' {
        num_end += 1;
    }
    while num_end < bytes.len() && (bytes[num_end].is_ascii_digit() || bytes[num_end] == b'.') {
        num_end += 1;
    }
    if num_end == 0 {
        return None;
    }
    let n: f64 = s[..num_end].parse().ok()?;
    if !n.is_finite() {
        return None;
    }
    let unit = s[num_end..].trim();
    const S: f64 = 1000.0;
    const M: f64 = S * 60.0;
    const H: f64 = M * 60.0;
    const D: f64 = H * 24.0;
    const W: f64 = D * 7.0;
    const Y: f64 = D * 365.25;
    let mult = if unit.is_empty()
        || unit.eq_ignore_ascii_case("ms")
        || unit.eq_ignore_ascii_case("msec")
        || unit.eq_ignore_ascii_case("msecs")
        || unit.eq_ignore_ascii_case("millisecond")
        || unit.eq_ignore_ascii_case("milliseconds")
    {
        1.0
    } else if unit.eq_ignore_ascii_case("s")
        || unit.eq_ignore_ascii_case("sec")
        || unit.eq_ignore_ascii_case("secs")
        || unit.eq_ignore_ascii_case("second")
        || unit.eq_ignore_ascii_case("seconds")
    {
        S
    } else if unit.eq_ignore_ascii_case("m")
        || unit.eq_ignore_ascii_case("min")
        || unit.eq_ignore_ascii_case("mins")
        || unit.eq_ignore_ascii_case("minute")
        || unit.eq_ignore_ascii_case("minutes")
    {
        M
    } else if unit.eq_ignore_ascii_case("h")
        || unit.eq_ignore_ascii_case("hr")
        || unit.eq_ignore_ascii_case("hrs")
        || unit.eq_ignore_ascii_case("hour")
        || unit.eq_ignore_ascii_case("hours")
    {
        H
    } else if unit.eq_ignore_ascii_case("d")
        || unit.eq_ignore_ascii_case("day")
        || unit.eq_ignore_ascii_case("days")
    {
        D
    } else if unit.eq_ignore_ascii_case("w")
        || unit.eq_ignore_ascii_case("week")
        || unit.eq_ignore_ascii_case("weeks")
    {
        W
    } else if unit.eq_ignore_ascii_case("y")
        || unit.eq_ignore_ascii_case("yr")
        || unit.eq_ignore_ascii_case("yrs")
        || unit.eq_ignore_ascii_case("year")
        || unit.eq_ignore_ascii_case("years")
    {
        Y
    } else {
        return None;
    };
    Some(n * mult)
}

#[inline]
pub(crate) fn fast_format_ms(ms: f64, long: bool) -> Option<String> {
    if !ms.is_finite() {
        return None;
    }
    let ms_abs = ms.abs();
    const S: f64 = 1000.0;
    const M: f64 = S * 60.0;
    const H: f64 = M * 60.0;
    const D: f64 = H * 24.0;
    if long {
        let plural = |ms: f64, ms_abs: f64, n: f64, name: &str| -> String {
            let is_plural = ms_abs >= n * 1.5;
            let rounded = (ms / n).round() as i64;
            if is_plural {
                format!("{} {}s", rounded, name)
            } else {
                format!("{} {}", rounded, name)
            }
        };
        if ms_abs >= D {
            return Some(plural(ms, ms_abs, D, "day"));
        }
        if ms_abs >= H {
            return Some(plural(ms, ms_abs, H, "hour"));
        }
        if ms_abs >= M {
            return Some(plural(ms, ms_abs, M, "minute"));
        }
        if ms_abs >= S {
            return Some(plural(ms, ms_abs, S, "second"));
        }
        Some(format!("{} ms", ms as i64))
    } else if ms_abs >= D {
        Some(format!("{}d", (ms / D).round() as i64))
    } else if ms_abs >= H {
        Some(format!("{}h", (ms / H).round() as i64))
    } else if ms_abs >= M {
        Some(format!("{}m", (ms / M).round() as i64))
    } else if ms_abs >= S {
        Some(format!("{}s", (ms / S).round() as i64))
    } else {
        Some(format!("{}ms", ms as i64))
    }
}

#[inline]
pub fn fast_parse_bytes(input: &str) -> Option<f64> {
    let s = input.trim();
    if s.is_empty() || s.len() > 100 {
        return None;
    }
    let bytes = s.as_bytes();
    let mut num_end = 0;
    if bytes[0] == b'-' || bytes[0] == b'+' {
        num_end += 1;
    }
    while num_end < bytes.len() && (bytes[num_end].is_ascii_digit() || bytes[num_end] == b'.') {
        num_end += 1;
    }
    if num_end == 0 {
        return None;
    }
    let n: f64 = s[..num_end].parse().ok()?;
    if !n.is_finite() {
        return None;
    }
    let unit = s[num_end..].trim();
    let mult: f64 = if unit.is_empty() || unit.eq_ignore_ascii_case("b") {
        1.0
    } else if unit.eq_ignore_ascii_case("kb") {
        1024.0
    } else if unit.eq_ignore_ascii_case("mb") {
        1_048_576.0
    } else if unit.eq_ignore_ascii_case("gb") {
        1_073_741_824.0
    } else if unit.eq_ignore_ascii_case("tb") {
        1_099_511_627_776.0
    } else if unit.eq_ignore_ascii_case("pb") {
        1_125_899_906_842_624.0
    } else {
        return None;
    };
    Some((n * mult).floor())
}

#[inline]
pub(crate) fn fast_format_bytes(n: f64) -> Option<String> {
    if !n.is_finite() {
        return None;
    }
    let mag = n.abs();
    let (divisor, unit) = if mag >= 1_125_899_906_842_624.0 {
        (1_125_899_906_842_624.0, "PB")
    } else if mag >= 1_099_511_627_776.0 {
        (1_099_511_627_776.0, "TB")
    } else if mag >= 1_073_741_824.0 {
        (1_073_741_824.0, "GB")
    } else if mag >= 1_048_576.0 {
        (1_048_576.0, "MB")
    } else if mag >= 1024.0 {
        (1024.0, "KB")
    } else {
        (1.0, "B")
    };
    let val = n / divisor;
    let cents = (val * 100.0).round() as i64;
    let whole = cents / 100;
    let frac = (cents % 100).abs();
    let formatted = if frac == 0 {
        format!("{}{}", whole, unit)
    } else if frac % 10 == 0 {
        format!("{}.{}{}", whole, frac / 10, unit)
    } else {
        format!("{}.{:02}{}", whole, frac, unit)
    };
    Some(formatted)
}

pub(crate) fn semver_parse_strict_xyz(input: &str) -> Option<(u64, u64, u64)> {
    let trimmed = input.trim();
    let stripped = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('='))
        .unwrap_or(trimmed)
        .trim_start();
    if stripped.contains('-') || stripped.contains('+') {
        return None;
    }
    let mut parts = stripped.split('.');
    let p1 = parts.next()?;
    let p2 = parts.next()?;
    let p3 = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    for p in [p1, p2, p3] {
        if p.is_empty()
            || (p.len() > 1 && p.starts_with('0'))
            || !p.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
    }
    Some((p1.parse().ok()?, p2.parse().ok()?, p3.parse().ok()?))
}

pub(crate) fn semver_coerce_xyz(input: &str) -> Option<(u64, u64, u64)> {
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() && !bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let start_maj = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    let maj: u64 = input[start_maj..i].parse().ok()?;
    let mut min: u64 = 0;
    let mut pat: u64 = 0;
    if i < bytes.len() && bytes[i] == b'.' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
        i += 1;
        let start_min = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        min = input[start_min..i].parse().ok()?;
        if i < bytes.len()
            && bytes[i] == b'.'
            && i + 1 < bytes.len()
            && bytes[i + 1].is_ascii_digit()
        {
            i += 1;
            let start_pat = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            pat = input[start_pat..i].parse().ok()?;
        }
    }
    Some((maj, min, pat))
}

pub(crate) fn semver_satisfies_xyz(ver_str: &str, range_str: &str) -> Option<bool> {
    let v = semver_parse_strict_xyz(ver_str)?;
    if range_str.contains("||")
        || range_str.contains(" - ")
        || range_str.contains('x')
        || range_str.contains('X')
        || range_str.contains('*')
        || range_str.contains('-')
    {
        return None;
    }
    for token in range_str.split_whitespace() {
        if let Some(rest) = token.strip_prefix('^') {
            let (rmaj, rmin, rpat) = semver_parse_strict_xyz(rest)?;
            if v < (rmaj, rmin, rpat) {
                return Some(false);
            }
            let upper = if rmaj > 0 {
                (rmaj.saturating_add(1), 0, 0)
            } else if rmin > 0 {
                (0, rmin.saturating_add(1), 0)
            } else {
                (0, 0, rpat.saturating_add(1))
            };
            if v >= upper {
                return Some(false);
            }
        } else if let Some(rest) = token.strip_prefix('~') {
            let (rmaj, rmin, rpat) = semver_parse_strict_xyz(rest)?;
            if v < (rmaj, rmin, rpat) || v >= (rmaj, rmin.saturating_add(1), 0) {
                return Some(false);
            }
        } else if let Some(rest) = token.strip_prefix(">=") {
            let r = semver_parse_strict_xyz(rest)?;
            if v < r {
                return Some(false);
            }
        } else if let Some(rest) = token.strip_prefix("<=") {
            let r = semver_parse_strict_xyz(rest)?;
            if v > r {
                return Some(false);
            }
        } else if let Some(rest) = token.strip_prefix('>') {
            let r = semver_parse_strict_xyz(rest)?;
            if v <= r {
                return Some(false);
            }
        } else if let Some(rest) = token.strip_prefix('<') {
            let r = semver_parse_strict_xyz(rest)?;
            if v >= r {
                return Some(false);
            }
        } else {
            let r = semver_parse_strict_xyz(token)?;
            if v != r {
                return Some(false);
            }
        }
    }
    Some(true)
}
