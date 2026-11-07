//! Native Rust SIMD & Fast-Path Intrinsics (`SHA-NI` / FIPS-180-4 SHA-256, HMAC-SHA256, Base64, SemVer, `ms`, `bytes`).

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
    if s.is_empty() {
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
                (rmaj + 1, 0, 0)
            } else if rmin > 0 {
                (0, rmin + 1, 0)
            } else {
                (0, 0, rpat + 1)
            };
            if v >= upper {
                return Some(false);
            }
        } else if let Some(rest) = token.strip_prefix('~') {
            let (rmaj, rmin, rpat) = semver_parse_strict_xyz(rest)?;
            if v < (rmaj, rmin, rpat) || v >= (rmaj, rmin + 1, 0) {
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

/// Phase 9A Zero-Allocation Streaming FIPS-180-4 SHA-256 State Machine.
/// Processes 64-byte chunks directly from borrowed `&[u8]` slices in place and buffers
/// only the final `<= 64` byte partial block on the stack (0 heap allocations).
pub(crate) struct Sha256State {
    pub(crate) h: [u32; 8],
    pub(crate) buf: [u8; 64],
    pub(crate) buf_len: usize,
    pub(crate) total_len: u64,
}

impl Sha256State {
    #[inline]
    pub(crate) fn new() -> Self {
        Self {
            h: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buf: [0u8; 64],
            buf_len: 0,
            total_len: 0,
        }
    }

    #[inline]
    pub(crate) fn compress_block(h: &mut [u32; 8], chunk: &[u8]) {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        {
            static SHA_NI_STATUS: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
            let status = SHA_NI_STATUS.load(std::sync::atomic::Ordering::Relaxed);
            let use_sha_ni = if status == 1 {
                true
            } else if status == 2 {
                false
            } else {
                let detected = std::arch::is_x86_feature_detected!("sha")
                    && std::arch::is_x86_feature_detected!("sse4.1")
                    && std::arch::is_x86_feature_detected!("ssse3");
                SHA_NI_STATUS.store(
                    if detected { 1 } else { 2 },
                    std::sync::atomic::Ordering::Relaxed,
                );
                detected
            };
            if use_sha_ni {
                unsafe {
                    Self::compress_block_sha_ni(h, chunk);
                }
                return;
            }
        }
        Self::compress_block_soft(h, chunk);
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    #[target_feature(enable = "sha,sse4.1,ssse3")]
    pub(crate) unsafe fn compress_block_sha_ni(h: &mut [u32; 8], chunk: &[u8]) {
        #[cfg(target_arch = "x86")]
        use std::arch::x86::*;
        #[cfg(target_arch = "x86_64")]
        use std::arch::x86_64::*;

        let shuf_mask = _mm_set_epi64x(0x0c0d0e0f08090a0b, 0x0405060700010203);

        let state0_in = _mm_loadu_si128(h.as_ptr() as *const __m128i);
        let state1_in = _mm_loadu_si128(h.as_ptr().add(4) as *const __m128i);

        let tmp = _mm_shuffle_epi32(state0_in, 0xB1);
        let state1_shuf = _mm_shuffle_epi32(state1_in, 0x1B);
        let mut state0 = _mm_alignr_epi8(tmp, state1_shuf, 8);
        let mut state1 = _mm_blend_epi16(state1_shuf, tmp, 0xF0);

        let abef_save = state0;
        let cdgh_save = state1;

        let ptr = chunk.as_ptr() as *const __m128i;
        let mut msg0 = _mm_shuffle_epi8(_mm_loadu_si128(ptr), shuf_mask);
        let mut msg1 = _mm_shuffle_epi8(_mm_loadu_si128(ptr.add(1)), shuf_mask);
        let mut msg2 = _mm_shuffle_epi8(_mm_loadu_si128(ptr.add(2)), shuf_mask);
        let mut msg3 = _mm_shuffle_epi8(_mm_loadu_si128(ptr.add(3)), shuf_mask);

        macro_rules! sha256_4rounds {
            ($msg:expr, $k3:expr, $k2:expr, $k1:expr, $k0:expr) => {{
                let msg_k = _mm_add_epi32(
                    $msg,
                    _mm_set_epi32($k3 as i32, $k2 as i32, $k1 as i32, $k0 as i32),
                );
                state1 = _mm_sha256rnds2_epu32(state1, state0, msg_k);
                state0 = _mm_sha256rnds2_epu32(state0, state1, _mm_srli_si128(msg_k, 8));
            }};
        }

        // Rounds 0..15
        sha256_4rounds!(
            msg0,
            0xe9b5dba5u32,
            0xb5c0fbcfu32,
            0x71374491u32,
            0x428a2f98u32
        );
        sha256_4rounds!(
            msg1,
            0xab1c5ed5u32,
            0x923f82a4u32,
            0x59f111f1u32,
            0x3956c25bu32
        );
        msg0 = _mm_sha256msg1_epu32(msg0, msg1);
        sha256_4rounds!(
            msg2,
            0x550c7dc3u32,
            0x243185beu32,
            0x12835b01u32,
            0xd807aa98u32
        );
        msg1 = _mm_sha256msg1_epu32(msg1, msg2);
        msg0 = _mm_add_epi32(msg0, _mm_alignr_epi8(msg3, msg2, 4));
        msg0 = _mm_sha256msg2_epu32(msg0, msg3);
        sha256_4rounds!(
            msg3,
            0xc19bf174u32,
            0x9bdc06a7u32,
            0x80deb1feu32,
            0x72be5d74u32
        );
        msg2 = _mm_sha256msg1_epu32(msg2, msg3);
        msg1 = _mm_add_epi32(msg1, _mm_alignr_epi8(msg0, msg3, 4));
        msg1 = _mm_sha256msg2_epu32(msg1, msg0);

        // Rounds 16..31
        sha256_4rounds!(
            msg0,
            0x240ca1ccu32,
            0x0fc19dc6u32,
            0xefbe4786u32,
            0xe49b69c1u32
        );
        msg3 = _mm_sha256msg1_epu32(msg3, msg0);
        msg2 = _mm_add_epi32(msg2, _mm_alignr_epi8(msg1, msg0, 4));
        msg2 = _mm_sha256msg2_epu32(msg2, msg1);
        sha256_4rounds!(
            msg1,
            0x76f988dau32,
            0x5cb0a9dcu32,
            0x4a7484aau32,
            0x2de92c6fu32
        );
        msg0 = _mm_sha256msg1_epu32(msg0, msg1);
        msg3 = _mm_add_epi32(msg3, _mm_alignr_epi8(msg2, msg1, 4));
        msg3 = _mm_sha256msg2_epu32(msg3, msg2);
        sha256_4rounds!(
            msg2,
            0xbf597fc7u32,
            0xb00327c8u32,
            0xa831c66du32,
            0x983e5152u32
        );
        msg1 = _mm_sha256msg1_epu32(msg1, msg2);
        msg0 = _mm_add_epi32(msg0, _mm_alignr_epi8(msg3, msg2, 4));
        msg0 = _mm_sha256msg2_epu32(msg0, msg3);
        sha256_4rounds!(
            msg3,
            0x14292967u32,
            0x06ca6351u32,
            0xd5a79147u32,
            0xc6e00bf3u32
        );
        msg2 = _mm_sha256msg1_epu32(msg2, msg3);
        msg1 = _mm_add_epi32(msg1, _mm_alignr_epi8(msg0, msg3, 4));
        msg1 = _mm_sha256msg2_epu32(msg1, msg0);

        // Rounds 32..47
        sha256_4rounds!(
            msg0,
            0x53380d13u32,
            0x4d2c6dfcu32,
            0x2e1b2138u32,
            0x27b70a85u32
        );
        msg3 = _mm_sha256msg1_epu32(msg3, msg0);
        msg2 = _mm_add_epi32(msg2, _mm_alignr_epi8(msg1, msg0, 4));
        msg2 = _mm_sha256msg2_epu32(msg2, msg1);
        sha256_4rounds!(
            msg1,
            0x92722c85u32,
            0x81c2c92eu32,
            0x766a0abbu32,
            0x650a7354u32
        );
        msg0 = _mm_sha256msg1_epu32(msg0, msg1);
        msg3 = _mm_add_epi32(msg3, _mm_alignr_epi8(msg2, msg1, 4));
        msg3 = _mm_sha256msg2_epu32(msg3, msg2);
        sha256_4rounds!(
            msg2,
            0xc76c51a3u32,
            0xc24b8b70u32,
            0xa81a664bu32,
            0xa2bfe8a1u32
        );
        msg1 = _mm_sha256msg1_epu32(msg1, msg2);
        msg0 = _mm_add_epi32(msg0, _mm_alignr_epi8(msg3, msg2, 4));
        msg0 = _mm_sha256msg2_epu32(msg0, msg3);
        sha256_4rounds!(
            msg3,
            0x106aa070u32,
            0xf40e3585u32,
            0xd6990624u32,
            0xd192e819u32
        );
        msg2 = _mm_sha256msg1_epu32(msg2, msg3);
        msg1 = _mm_add_epi32(msg1, _mm_alignr_epi8(msg0, msg3, 4));
        msg1 = _mm_sha256msg2_epu32(msg1, msg0);

        // Rounds 48..63
        sha256_4rounds!(
            msg0,
            0x34b0bcb5u32,
            0x2748774cu32,
            0x1e376c08u32,
            0x19a4c116u32
        );
        msg3 = _mm_sha256msg1_epu32(msg3, msg0);
        msg2 = _mm_add_epi32(msg2, _mm_alignr_epi8(msg1, msg0, 4));
        msg2 = _mm_sha256msg2_epu32(msg2, msg1);
        sha256_4rounds!(
            msg1,
            0x682e6ff3u32,
            0x5b9cca4fu32,
            0x4ed8aa4au32,
            0x391c0cb3u32
        );
        msg3 = _mm_add_epi32(msg3, _mm_alignr_epi8(msg2, msg1, 4));
        msg3 = _mm_sha256msg2_epu32(msg3, msg2);
        sha256_4rounds!(
            msg2,
            0x8cc70208u32,
            0x84c87814u32,
            0x78a5636fu32,
            0x748f82eeu32
        );
        sha256_4rounds!(
            msg3,
            0xc67178f2u32,
            0xbef9a3f7u32,
            0xa4506cebu32,
            0x90befffau32
        );

        state0 = _mm_add_epi32(state0, abef_save);
        state1 = _mm_add_epi32(state1, cdgh_save);

        let out_tmp = _mm_shuffle_epi32(state0, 0x1B);
        let out_s1 = _mm_shuffle_epi32(state1, 0xB1);
        let out0 = _mm_blend_epi16(out_tmp, out_s1, 0xF0);
        let out1 = _mm_alignr_epi8(out_s1, out_tmp, 8);

        _mm_storeu_si128(h.as_mut_ptr() as *mut __m128i, out0);
        _mm_storeu_si128(h.as_mut_ptr().add(4) as *mut __m128i, out1);
    }

    #[inline]
    pub(crate) fn compress_block_soft(h: &mut [u32; 8], chunk: &[u8]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];

        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    #[inline]
    pub(crate) fn update(&mut self, mut data: &[u8]) {
        self.total_len += data.len() as u64;
        if self.buf_len > 0 {
            let need = 64 - self.buf_len;
            if data.len() < need {
                self.buf[self.buf_len..self.buf_len + data.len()].copy_from_slice(data);
                self.buf_len += data.len();
                return;
            }
            self.buf[self.buf_len..64].copy_from_slice(&data[..need]);
            Self::compress_block(&mut self.h, &self.buf);
            self.buf_len = 0;
            data = &data[need..];
        }

        let (chunks, rem) = data.as_chunks::<64>();
        for chunk in chunks {
            Self::compress_block(&mut self.h, chunk);
        }

        if !rem.is_empty() {
            self.buf[..rem.len()].copy_from_slice(rem);
            self.buf_len = rem.len();
        }
    }

    #[inline]
    pub(crate) fn finalize(mut self) -> [u8; 32] {
        let bit_len = self.total_len * 8;
        self.buf[self.buf_len] = 0x80;
        self.buf_len += 1;

        if self.buf_len > 56 {
            self.buf[self.buf_len..64].fill(0);
            Self::compress_block(&mut self.h, &self.buf);
            self.buf.fill(0);
        } else {
            self.buf[self.buf_len..56].fill(0);
        }

        self.buf[56..64].copy_from_slice(&bit_len.to_be_bytes());
        Self::compress_block(&mut self.h, &self.buf);

        let mut out = [0u8; 32];
        for (idx, word) in self.h.iter().enumerate() {
            let bytes = word.to_be_bytes();
            out[idx * 4] = bytes[0];
            out[idx * 4 + 1] = bytes[1];
            out[idx * 4 + 2] = bytes[2];
            out[idx * 4 + 3] = bytes[3];
        }
        out
    }
}

/// Pure-Rust FIPS-180-4 SHA-256 implementation with zero intermediate heap allocations.
#[inline]
pub(crate) fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    let mut state = Sha256State::new();
    state.update(data);
    state.finalize()
}

/// Fast Lookup-Table (`HEX_LUT`) hex encoder avoiding `std::fmt::Write` overhead.
#[inline]
pub(crate) fn digest_to_hex(digest: &[u8; 32]) -> String {
    const HEX_LUT: &[u8; 16] = b"0123456789abcdef";
    let mut hex_bytes = Vec::with_capacity(64);
    for &b in digest {
        hex_bytes.push(HEX_LUT[(b >> 4) as usize]);
        hex_bytes.push(HEX_LUT[(b & 0x0f) as usize]);
    }
    unsafe { String::from_utf8_unchecked(hex_bytes) }
}

#[inline]
pub fn sha256_hex(data: &[u8]) -> String {
    let digest = sha256_bytes(data);
    digest_to_hex(&digest)
}

/// RFC 2104 HMAC-SHA256 implementation streaming directly from borrowed slices with zero heap allocation.
#[inline]
pub fn hmac_sha256_hex(key: &[u8], message: &[u8]) -> String {
    let mut key_block = [0u8; 64];
    if key.len() > 64 {
        let hashed_key = sha256_bytes(key);
        key_block[..32].copy_from_slice(&hashed_key);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }

    let mut inner = Sha256State::new();
    inner.update(&ipad);
    inner.update(message);
    let inner_hash = inner.finalize();

    let mut outer = Sha256State::new();
    outer.update(&opad);
    outer.update(&inner_hash);
    digest_to_hex(&outer.finalize())
}

pub(crate) fn base64_encode(data: &[u8], url_safe: bool) -> String {
    const STD_TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    const URL_TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let table = if url_safe { URL_TABLE } else { STD_TABLE };
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(table[((triple >> 18) & 0x3f) as usize] as char);
        out.push(table[((triple >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(table[((triple >> 6) & 0x3f) as usize] as char);
        } else if !url_safe {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(table[(triple & 0x3f) as usize] as char);
        } else if !url_safe {
            out.push('=');
        }
    }
    out
}

pub(crate) fn base64_decode(input: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity((input.len() * 3) / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for b in input.bytes() {
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\r' | b'\n' | b' ' | b'\t' => continue,
            _ => continue,
        };
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    out
}

/// Fast Lookup-Table (`HEX_LUT`) hex encoder for arbitrary byte slices.
#[inline]
pub(crate) fn bytes_to_hex(bytes: &[u8]) -> String {
    const HEX_LUT: &[u8; 16] = b"0123456789abcdef";
    let mut hex_bytes = Vec::with_capacity(bytes.len() * 2);
    for &b in bytes {
        hex_bytes.push(HEX_LUT[(b >> 4) as usize]);
        hex_bytes.push(HEX_LUT[(b & 0x0f) as usize]);
    }
    unsafe { String::from_utf8_unchecked(hex_bytes) }
}

/// RFC 2104 HMAC-SHA256 raw 32-byte digest with zero heap allocation.
#[inline]
pub(crate) fn hmac_sha256_bytes(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut key_block = [0u8; 64];
    if key.len() > 64 {
        let hashed_key = sha256_bytes(key);
        key_block[..32].copy_from_slice(&hashed_key);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }

    let mut inner = Sha256State::new();
    inner.update(&ipad);
    inner.update(message);
    let inner_hash = inner.finalize();

    let mut outer = Sha256State::new();
    outer.update(&opad);
    outer.update(&inner_hash);
    outer.finalize()
}

/// Decodes a hexadecimal string into raw bytes (stopping at the first non-hex byte pair,
/// matching Node.js `Buffer.from(str, 'hex')` semantics).
#[inline]
pub(crate) fn hex_decode_bytes(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let pairs = bytes.len() / 2;
    let mut out = Vec::with_capacity(pairs);
    for i in 0..pairs {
        let hi = match bytes[i * 2] {
            b'0'..=b'9' => bytes[i * 2] - b'0',
            b'a'..=b'f' => bytes[i * 2] - b'a' + 10,
            b'A'..=b'F' => bytes[i * 2] - b'A' + 10,
            _ => break,
        };
        let lo = match bytes[i * 2 + 1] {
            b'0'..=b'9' => bytes[i * 2 + 1] - b'0',
            b'a'..=b'f' => bytes[i * 2 + 1] - b'a' + 10,
            b'A'..=b'F' => bytes[i * 2 + 1] - b'A' + 10,
            _ => break,
        };
        out.push((hi << 4) | lo);
    }
    out
}

/// Zero-allocation FIPS 180-4 SHA-1 state machine (for WebCrypto `"SHA-1"` and WebSocket handshakes).
pub(crate) struct Sha1State {
    h: [u32; 5],
    buf: [u8; 64],
    buf_len: usize,
    total_len: u64,
}

impl Sha1State {
    #[inline]
    pub(crate) fn new() -> Self {
        Self {
            h: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476, 0xc3d2e1f0],
            buf: [0u8; 64],
            buf_len: 0,
            total_len: 0,
        }
    }

    #[inline]
    fn compress_block(h: &mut [u32; 5], chunk: &[u8]) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = if i < 20 {
                ((b & c) | ((!b) & d), 0x5a827999u32)
            } else if i < 40 {
                (b ^ c ^ d, 0x6ed9eba1u32)
            } else if i < 60 {
                ((b & c) | (b & d) | (c & d), 0x8f1bbcdcu32)
            } else {
                (b ^ c ^ d, 0xca62c1d6u32)
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }

    #[inline]
    pub(crate) fn update(&mut self, mut data: &[u8]) {
        self.total_len += data.len() as u64;
        if self.buf_len > 0 {
            let need = 64 - self.buf_len;
            if data.len() < need {
                self.buf[self.buf_len..self.buf_len + data.len()].copy_from_slice(data);
                self.buf_len += data.len();
                return;
            }
            self.buf[self.buf_len..64].copy_from_slice(&data[..need]);
            Self::compress_block(&mut self.h, &self.buf);
            self.buf_len = 0;
            data = &data[need..];
        }
        let (chunks, rem) = data.as_chunks::<64>();
        for chunk in chunks {
            Self::compress_block(&mut self.h, chunk);
        }
        if !rem.is_empty() {
            self.buf[..rem.len()].copy_from_slice(rem);
            self.buf_len = rem.len();
        }
    }

    #[inline]
    pub(crate) fn finalize(mut self) -> [u8; 20] {
        let bit_len = self.total_len * 8;
        self.buf[self.buf_len] = 0x80;
        self.buf_len += 1;
        if self.buf_len > 56 {
            self.buf[self.buf_len..64].fill(0);
            Self::compress_block(&mut self.h, &self.buf);
            self.buf.fill(0);
        } else {
            self.buf[self.buf_len..56].fill(0);
        }
        self.buf[56..64].copy_from_slice(&bit_len.to_be_bytes());
        Self::compress_block(&mut self.h, &self.buf);
        let mut out = [0u8; 20];
        for (idx, word) in self.h.iter().enumerate() {
            out[idx * 4..idx * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }
}

#[inline]
pub(crate) fn sha1_bytes(data: &[u8]) -> [u8; 20] {
    let mut st = Sha1State::new();
    st.update(data);
    st.finalize()
}

#[inline]
pub(crate) fn hmac_sha1_bytes(key: &[u8], message: &[u8]) -> [u8; 20] {
    let mut key_block = [0u8; 64];
    if key.len() > 64 {
        let hk = sha1_bytes(key);
        key_block[..20].copy_from_slice(&hk);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }
    let mut inner = Sha1State::new();
    inner.update(&ipad);
    inner.update(message);
    let ih = inner.finalize();
    let mut outer = Sha1State::new();
    outer.update(&opad);
    outer.update(&ih);
    outer.finalize()
}

/// Zero-allocation FIPS 180-4 SHA-512 / SHA-384 state machine.
pub(crate) struct Sha512State {
    h: [u64; 8],
    buf: [u8; 128],
    buf_len: usize,
    total_len: u128,
}

impl Sha512State {
    #[inline]
    pub(crate) fn new_512() -> Self {
        Self {
            h: [
                0x6a09e667f3bcc908,
                0xbb67ae8584caa73b,
                0x3c6ef372fe94f82b,
                0xa54ff53a5f1d36f1,
                0x510e527fade682d1,
                0x9b05688c2b3e6c1f,
                0x1f83d9abfb41bd6b,
                0x5be0cd19137e2179,
            ],
            buf: [0u8; 128],
            buf_len: 0,
            total_len: 0,
        }
    }

    #[inline]
    pub(crate) fn new_384() -> Self {
        Self {
            h: [
                0xcbbb9d5dc1059ed8,
                0x629a292a367cd507,
                0x9159015a3070dd17,
                0x152fecd8f70e5939,
                0x67332667ffc00b31,
                0x8eb44a8768581511,
                0xdb0c2e0d64f98fa7,
                0x47b5481dbefa4fa4,
            ],
            buf: [0u8; 128],
            buf_len: 0,
            total_len: 0,
        }
    }

    #[inline]
    fn compress_block(h: &mut [u64; 8], chunk: &[u8]) {
        const K: [u64; 80] = [
            0x428a2f98d728ae22,
            0x7137449123ef65cd,
            0xb5c0fbcfec4d3b2f,
            0xe9b5dba58189dbbc,
            0x3956c25bf348b538,
            0x59f111f1b605d019,
            0x923f82a4af194f9b,
            0xab1c5ed5da6d8118,
            0xd807aa98a3030242,
            0x12835b0145706fbe,
            0x243185be4ee4b28c,
            0x550c7dc3d5ffb4e2,
            0x72be5d74f27b896f,
            0x80deb1fe3b1696b1,
            0x9bdc06a725c71235,
            0xc19bf174cf692694,
            0xe49b69c19ef14ad2,
            0xefbe4786384f25e3,
            0x0fc19dc68b8cd5b5,
            0x240ca1cc77ac9c65,
            0x2de92c6f592b0275,
            0x4a7484aa6ea6e483,
            0x5cb0a9dcbd41fbd4,
            0x76f988da831153b5,
            0x983e5152ee66dfab,
            0xa831c66d2db43210,
            0xb00327c898fb213f,
            0xbf597fc7beef0ee4,
            0xc6e00bf33da88fc2,
            0xd5a79147930aa725,
            0x06ca6351e003826f,
            0x142929670a0e6e70,
            0x27b70a8546d22ffc,
            0x2e1b21385c26c926,
            0x4d2c6dfc5ac42aed,
            0x53380d139d95b3df,
            0x650a73548baf63de,
            0x766a0abb3c77b2a8,
            0x81c2c92e47edaee6,
            0x92722c851482353b,
            0xa2bfe8a14cf10364,
            0xa81a664bbc423001,
            0xc24b8b70d0f89791,
            0xc76c51a30654be30,
            0xd192e819d6ef5218,
            0xd69906245565a910,
            0xf40e35855771202a,
            0x106aa07032bbd1b8,
            0x19a4c116b8d2d0c8,
            0x1e376c085141ab53,
            0x2748774cdf8eeb99,
            0x34b0bcb5e19b48a8,
            0x391c0cb3c5c95a63,
            0x4ed8aa4ae3418acb,
            0x5b9cca4f7763e373,
            0x682e6ff3d6b2b8a3,
            0x748f82ee5defb2fc,
            0x78a5636f43172f60,
            0x84c87814a1f0ab72,
            0x8cc702081a6439ec,
            0x90befffa23631e28,
            0xa4506cebde82bde9,
            0xbef9a3f7b2c67915,
            0xc67178f2e372532b,
            0xca273eceea26619c,
            0xd186b8c721c0c207,
            0xeada7dd6cde0eb1e,
            0xf57d4f7fee6ed178,
            0x06f067aa72176fba,
            0x0a637dc5a2c898a6,
            0x113f9804bef90dae,
            0x1b710b35131c471b,
            0x28db77f523047d84,
            0x32caab7b40c72493,
            0x3c9ebe0a15c9bebc,
            0x431d67c49c100d4c,
            0x4cc5d4becb3e42b6,
            0x597f299cfc657e2a,
            0x5fcb6fab3ad6faec,
            0x6c44198c4a475817,
        ];

        let mut w = [0u64; 80];
        for (i, w_i) in w.iter_mut().enumerate().take(16) {
            let off = i * 8;
            *w_i = u64::from_be_bytes([
                chunk[off],
                chunk[off + 1],
                chunk[off + 2],
                chunk[off + 3],
                chunk[off + 4],
                chunk[off + 5],
                chunk[off + 6],
                chunk[off + 7],
            ]);
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);

        for i in 0..80 {
            let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    #[inline]
    pub(crate) fn update(&mut self, mut data: &[u8]) {
        self.total_len += data.len() as u128;
        if self.buf_len > 0 {
            let need = 128 - self.buf_len;
            if data.len() < need {
                self.buf[self.buf_len..self.buf_len + data.len()].copy_from_slice(data);
                self.buf_len += data.len();
                return;
            }
            self.buf[self.buf_len..128].copy_from_slice(&data[..need]);
            Self::compress_block(&mut self.h, &self.buf);
            self.buf_len = 0;
            data = &data[need..];
        }
        let (chunks, rem) = data.as_chunks::<128>();
        for chunk in chunks {
            Self::compress_block(&mut self.h, chunk);
        }
        if !rem.is_empty() {
            self.buf[..rem.len()].copy_from_slice(rem);
            self.buf_len = rem.len();
        }
    }

    #[inline]
    pub(crate) fn finalize_512(mut self) -> [u8; 64] {
        let bit_len = self.total_len * 8;
        self.buf[self.buf_len] = 0x80;
        self.buf_len += 1;
        if self.buf_len > 112 {
            self.buf[self.buf_len..128].fill(0);
            Self::compress_block(&mut self.h, &self.buf);
            self.buf.fill(0);
        } else {
            self.buf[self.buf_len..112].fill(0);
        }
        self.buf[112..128].copy_from_slice(&bit_len.to_be_bytes());
        Self::compress_block(&mut self.h, &self.buf);
        let mut out = [0u8; 64];
        for (idx, word) in self.h.iter().enumerate() {
            out[idx * 8..idx * 8 + 8].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    #[inline]
    pub(crate) fn finalize_384(self) -> [u8; 48] {
        let full = self.finalize_512();
        let mut out = [0u8; 48];
        out.copy_from_slice(&full[..48]);
        out
    }
}

#[inline]
pub(crate) fn sha512_bytes(data: &[u8]) -> [u8; 64] {
    let mut st = Sha512State::new_512();
    st.update(data);
    st.finalize_512()
}

#[inline]
pub(crate) fn sha384_bytes(data: &[u8]) -> [u8; 48] {
    let mut st = Sha512State::new_384();
    st.update(data);
    st.finalize_384()
}

#[inline]
pub(crate) fn hmac_sha512_bytes(key: &[u8], message: &[u8]) -> [u8; 64] {
    let mut key_block = [0u8; 128];
    if key.len() > 128 {
        let hk = sha512_bytes(key);
        key_block[..64].copy_from_slice(&hk);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 128];
    let mut opad = [0x5cu8; 128];
    for i in 0..128 {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }
    let mut inner = Sha512State::new_512();
    inner.update(&ipad);
    inner.update(message);
    let ih = inner.finalize_512();
    let mut outer = Sha512State::new_512();
    outer.update(&opad);
    outer.update(&ih);
    outer.finalize_512()
}

#[inline]
pub(crate) fn hmac_sha384_bytes(key: &[u8], message: &[u8]) -> [u8; 48] {
    let mut key_block = [0u8; 128];
    if key.len() > 128 {
        let hk = sha384_bytes(key);
        key_block[..48].copy_from_slice(&hk);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 128];
    let mut opad = [0x5cu8; 128];
    for i in 0..128 {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }
    let mut inner = Sha512State::new_384();
    inner.update(&ipad);
    inner.update(message);
    let ih = inner.finalize_384();
    let mut outer = Sha512State::new_384();
    outer.update(&opad);
    outer.update(&ih);
    outer.finalize_384()
}

/// Constant-time slice equality check for `crypto.subtle.verify` and `crypto.subtle.timingSafeEqual`.
#[inline]
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= std::hint::black_box(x ^ y);
    }
    std::hint::black_box(diff) == 0
}

/// Zero-allocation RFC 1321 MD5 digest state machine (16-byte output) for `node:crypto`
/// `createHash("md5")` and `createHmac("md5", ...)` compatibility.
pub(crate) struct Md5State {
    s: [u32; 4],
    buf: [u8; 64],
    buf_len: usize,
    total_len: u64,
}

impl Md5State {
    #[inline]
    pub(crate) fn new() -> Self {
        Self {
            s: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476],
            buf: [0u8; 64],
            buf_len: 0,
            total_len: 0,
        }
    }

    #[inline]
    fn compress_block(s: &mut [u32; 4], chunk: &[u8]) {
        const S_SHIFT: [u32; 64] = [
            7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14,
            20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11,
            16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
        ];
        const K_TAB: [u32; 64] = [
            0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
            0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
            0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
            0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
            0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
            0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
            0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
            0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
            0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
            0xeb86d391,
        ];
        let mut m = [0u32; 16];
        for (i, item) in m.iter_mut().enumerate() {
            let off = i * 4;
            *item = u32::from_le_bytes([chunk[off], chunk[off + 1], chunk[off + 2], chunk[off + 3]]);
        }
        let (mut a, mut b, mut c, mut d) = (s[0], s[1], s[2], s[3]);
        for i in 0..64 {
            let (f, g) = if i < 16 {
                ((b & c) | ((!b) & d), i)
            } else if i < 32 {
                ((d & b) | ((!d) & c), (5 * i + 1) % 16)
            } else if i < 48 {
                (b ^ c ^ d, (3 * i + 5) % 16)
            } else {
                (c ^ (b | (!d)), (7 * i) % 16)
            };
            let temp = d;
            d = c;
            c = b;
            b = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(K_TAB[i])
                    .wrapping_add(m[g])
                    .rotate_left(S_SHIFT[i]),
            );
            a = temp;
        }
        s[0] = s[0].wrapping_add(a);
        s[1] = s[1].wrapping_add(b);
        s[2] = s[2].wrapping_add(c);
        s[3] = s[3].wrapping_add(d);
    }

    #[inline]
    pub(crate) fn update(&mut self, mut data: &[u8]) {
        self.total_len += data.len() as u64;
        if self.buf_len > 0 {
            let need = 64 - self.buf_len;
            if data.len() < need {
                self.buf[self.buf_len..self.buf_len + data.len()].copy_from_slice(data);
                self.buf_len += data.len();
                return;
            }
            self.buf[self.buf_len..64].copy_from_slice(&data[..need]);
            Self::compress_block(&mut self.s, &self.buf);
            self.buf_len = 0;
            data = &data[need..];
        }
        let (chunks, rem) = data.as_chunks::<64>();
        for chunk in chunks {
            Self::compress_block(&mut self.s, chunk);
        }
        if !rem.is_empty() {
            self.buf[..rem.len()].copy_from_slice(rem);
            self.buf_len = rem.len();
        }
    }

    #[inline]
    pub(crate) fn finalize(mut self) -> [u8; 16] {
        let bit_len = self.total_len.wrapping_mul(8);
        self.buf[self.buf_len] = 0x80;
        self.buf_len += 1;
        if self.buf_len > 56 {
            self.buf[self.buf_len..64].fill(0);
            Self::compress_block(&mut self.s, &self.buf);
            self.buf.fill(0);
        } else {
            self.buf[self.buf_len..56].fill(0);
        }
        self.buf[56..64].copy_from_slice(&bit_len.to_le_bytes());
        Self::compress_block(&mut self.s, &self.buf);
        let mut out = [0u8; 16];
        for (i, word) in self.s.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        out
    }
}

#[inline]
pub(crate) fn md5_bytes(data: &[u8]) -> [u8; 16] {
    let mut st = Md5State::new();
    st.update(data);
    st.finalize()
}

#[inline]
pub(crate) fn hmac_md5_bytes(key: &[u8], message: &[u8]) -> [u8; 16] {
    let mut key_block = [0u8; 64];
    if key.len() > 64 {
        let hk = md5_bytes(key);
        key_block[..16].copy_from_slice(&hk);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }
    let mut inner = Md5State::new();
    inner.update(&ipad);
    inner.update(message);
    let ih = inner.finalize();
    let mut outer = Md5State::new();
    outer.update(&opad);
    outer.update(&ih);
    outer.finalize()
}

// ============================================================================
// Phase 2: Native `ring` WebCrypto (`crypto.subtle`) & `node:crypto` Suite
// ============================================================================

const AES_SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab,
    0x76, 0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4,
    0x72, 0xc0, 0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71,
    0xd8, 0x31, 0x15, 0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2,
    0xeb, 0x27, 0xb2, 0x75, 0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6,
    0xb3, 0x29, 0xe3, 0x2f, 0x84, 0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb,
    0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf, 0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45,
    0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8, 0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5,
    0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2, 0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44,
    0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73, 0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a,
    0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb, 0xe0, 0x32, 0x3a, 0x0a, 0x49,
    0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79, 0xe7, 0xc8, 0x37, 0x6d,
    0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08, 0xba, 0x78, 0x25,
    0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a, 0x70, 0x3e,
    0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e, 0xe1,
    0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb,
    0x16,
];

const AES_INV_SBOX: [u8; 256] = [
    0x52, 0x09, 0x6a, 0xd5, 0x30, 0x36, 0xa5, 0x38, 0xbf, 0x40, 0xa3, 0x9e, 0x81, 0xf3, 0xd7,
    0xfb, 0x7c, 0xe3, 0x39, 0x82, 0x9b, 0x2f, 0xff, 0x87, 0x34, 0x8e, 0x43, 0x44, 0xc4, 0xde,
    0xe9, 0xcb, 0x54, 0x7b, 0x94, 0x32, 0xa6, 0xc2, 0x23, 0x3d, 0xee, 0x4c, 0x95, 0x0b, 0x42,
    0xfa, 0xc3, 0x4e, 0x08, 0x2e, 0xa1, 0x66, 0x28, 0xd9, 0x24, 0xb2, 0x76, 0x5b, 0xa2, 0x49,
    0x6d, 0x8b, 0xd1, 0x25, 0x72, 0xf8, 0xf6, 0x64, 0x86, 0x68, 0x98, 0x16, 0xd4, 0xa4, 0x5c,
    0xcc, 0x5d, 0x65, 0xb6, 0x92, 0x6c, 0x70, 0x48, 0x50, 0xfd, 0xed, 0xb9, 0xda, 0x5e, 0x15,
    0x46, 0x57, 0xa7, 0x8d, 0x9d, 0x84, 0x90, 0xd8, 0xab, 0x00, 0x8c, 0xbc, 0xd3, 0x0a, 0xf7,
    0xe4, 0x58, 0x05, 0xb8, 0xb3, 0x45, 0x06, 0xd0, 0x2c, 0x1e, 0x8f, 0xca, 0x3f, 0x0f, 0x02,
    0xc1, 0xaf, 0xbd, 0x03, 0x01, 0x13, 0x8a, 0x6b, 0x3a, 0x91, 0x11, 0x41, 0x4f, 0x67, 0xdc,
    0xea, 0x97, 0xf2, 0xcf, 0xce, 0xf0, 0xb4, 0xe6, 0x73, 0x96, 0xac, 0x74, 0x22, 0xe7, 0xad,
    0x35, 0x85, 0xe2, 0xf9, 0x37, 0xe8, 0x1c, 0x75, 0xdf, 0x6e, 0x47, 0xf1, 0x1a, 0x71, 0x1d,
    0x29, 0xc5, 0x89, 0x6f, 0xb7, 0x62, 0x0e, 0xaa, 0x18, 0xbe, 0x1b, 0xfc, 0x56, 0x3e, 0x4b,
    0xc6, 0xd2, 0x79, 0x20, 0x9a, 0xdb, 0xc0, 0xfe, 0x78, 0xcd, 0x5a, 0xf4, 0x1f, 0xdd, 0xa8,
    0x33, 0x88, 0x07, 0xc7, 0x31, 0xb1, 0x12, 0x10, 0x59, 0x27, 0x80, 0xec, 0x5f, 0x60, 0x51,
    0x7f, 0xa9, 0x19, 0xb5, 0x4a, 0x0d, 0x2d, 0xe5, 0x7a, 0x9f, 0x93, 0xc9, 0x9c, 0xef, 0xa0,
    0xe0, 0x3b, 0x4d, 0xae, 0x2a, 0xf5, 0xb0, 0xc8, 0xeb, 0xbb, 0x3c, 0x83, 0x53, 0x99, 0x61,
    0x17, 0x2b, 0x04, 0x7e, 0xba, 0x77, 0xd6, 0x26, 0xe1, 0x69, 0x14, 0x63, 0x55, 0x21, 0x0c,
    0x7d,
];

#[inline]
fn gf_mul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0u8;
    for _ in 0..8 {
        if (b & 1) != 0 {
            p ^= a;
        }
        let hi = a & 0x80;
        a <<= 1;
        if hi != 0 {
            a ^= 0x1b;
        }
        b >>= 1;
    }
    p
}

/// Pre-expanded AES-128 (`11` round keys) or AES-256 (`15` round keys) key schedule.
#[derive(Clone)]
pub struct AesExpandedKey {
    round_keys: [[u8; 16]; 15],
    rounds: usize,
}

impl AesExpandedKey {
    pub fn new(key: &[u8]) -> Result<Self, &'static str> {
        let (nk, nr) = match key.len() {
            16 => (4usize, 10usize),
            32 => (8usize, 14usize),
            _ => return Err("AES key length must be 16 (128-bit) or 32 (256-bit) bytes"),
        };
        let total_words = 4 * (nr + 1);
        let mut w = [0u32; 60];
        for i in 0..nk {
            w[i] = u32::from_be_bytes([key[4 * i], key[4 * i + 1], key[4 * i + 2], key[4 * i + 3]]);
        }
        const RCON: [u32; 10] = [
            0x01000000, 0x02000000, 0x04000000, 0x08000000, 0x10000000, 0x20000000, 0x40000000,
            0x80000000, 0x1b000000, 0x36000000,
        ];
        let sub_word = |val: u32| -> u32 {
            let b = val.to_be_bytes();
            u32::from_be_bytes([
                AES_SBOX[b[0] as usize],
                AES_SBOX[b[1] as usize],
                AES_SBOX[b[2] as usize],
                AES_SBOX[b[3] as usize],
            ])
        };
        for i in nk..total_words {
            let mut temp = w[i - 1];
            if i % nk == 0 {
                temp = sub_word(temp.rotate_left(8)) ^ RCON[(i / nk) - 1];
            } else if nk == 8 && i % nk == 4 {
                temp = sub_word(temp);
            }
            w[i] = w[i - nk] ^ temp;
        }
        let mut round_keys = [[0u8; 16]; 15];
        for r in 0..=nr {
            for c in 0..4 {
                let bytes = w[r * 4 + c].to_be_bytes();
                round_keys[r][c * 4..c * 4 + 4].copy_from_slice(&bytes);
            }
        }
        Ok(Self {
            round_keys,
            rounds: nr,
        })
    }

    #[inline]
    #[allow(clippy::needless_range_loop)]
    pub fn encrypt_block(&self, block: &mut [u8; 16]) {
        for i in 0..16 {
            block[i] ^= self.round_keys[0][i];
        }
        for r in 1..self.rounds {
            for b in block.iter_mut() {
                *b = AES_SBOX[*b as usize];
            }
            let s = *block;
            block[1] = s[5];
            block[5] = s[9];
            block[9] = s[13];
            block[13] = s[1];
            block[2] = s[10];
            block[6] = s[14];
            block[10] = s[2];
            block[14] = s[6];
            block[3] = s[15];
            block[7] = s[3];
            block[11] = s[7];
            block[15] = s[11];
            for c in 0..4 {
                let i = c * 4;
                let (a0, a1, a2, a3) = (block[i], block[i + 1], block[i + 2], block[i + 3]);
                block[i] = gf_mul(a0, 2) ^ gf_mul(a1, 3) ^ a2 ^ a3;
                block[i + 1] = a0 ^ gf_mul(a1, 2) ^ gf_mul(a2, 3) ^ a3;
                block[i + 2] = a0 ^ a1 ^ gf_mul(a2, 2) ^ gf_mul(a3, 3);
                block[i + 3] = gf_mul(a0, 3) ^ a1 ^ a2 ^ gf_mul(a3, 2);
            }
            for i in 0..16 {
                block[i] ^= self.round_keys[r][i];
            }
        }
        for b in block.iter_mut() {
            *b = AES_SBOX[*b as usize];
        }
        let s = *block;
        block[1] = s[5];
        block[5] = s[9];
        block[9] = s[13];
        block[13] = s[1];
        block[2] = s[10];
        block[6] = s[14];
        block[10] = s[2];
        block[14] = s[6];
        block[3] = s[15];
        block[7] = s[3];
        block[11] = s[7];
        block[15] = s[11];
        for i in 0..16 {
            block[i] ^= self.round_keys[self.rounds][i];
        }
    }

    #[inline]
    #[allow(clippy::needless_range_loop)]
    pub fn decrypt_block(&self, block: &mut [u8; 16]) {
        for i in 0..16 {
            block[i] ^= self.round_keys[self.rounds][i];
        }
        for r in (1..self.rounds).rev() {
            let s = *block;
            block[1] = s[13];
            block[5] = s[1];
            block[9] = s[5];
            block[13] = s[9];
            block[2] = s[10];
            block[6] = s[14];
            block[10] = s[2];
            block[14] = s[6];
            block[3] = s[7];
            block[7] = s[11];
            block[11] = s[15];
            block[15] = s[3];
            for b in block.iter_mut() {
                *b = AES_INV_SBOX[*b as usize];
            }
            for i in 0..16 {
                block[i] ^= self.round_keys[r][i];
            }
            for c in 0..4 {
                let i = c * 4;
                let (a0, a1, a2, a3) = (block[i], block[i + 1], block[i + 2], block[i + 3]);
                block[i] = gf_mul(a0, 14) ^ gf_mul(a1, 11) ^ gf_mul(a2, 13) ^ gf_mul(a3, 9);
                block[i + 1] = gf_mul(a0, 9) ^ gf_mul(a1, 14) ^ gf_mul(a2, 11) ^ gf_mul(a3, 13);
                block[i + 2] = gf_mul(a0, 13) ^ gf_mul(a1, 9) ^ gf_mul(a2, 14) ^ gf_mul(a3, 11);
                block[i + 3] = gf_mul(a0, 11) ^ gf_mul(a1, 13) ^ gf_mul(a2, 9) ^ gf_mul(a3, 14);
            }
        }
        let s = *block;
        block[1] = s[13];
        block[5] = s[1];
        block[9] = s[5];
        block[13] = s[9];
        block[2] = s[10];
        block[6] = s[14];
        block[10] = s[2];
        block[14] = s[6];
        block[3] = s[7];
        block[7] = s[11];
        block[11] = s[15];
        block[15] = s[3];
        for b in block.iter_mut() {
            *b = AES_INV_SBOX[*b as usize];
        }
        for i in 0..16 {
            block[i] ^= self.round_keys[0][i];
        }
    }
}

/// Encrypts `plaintext` with AES-128-CBC or AES-256-CBC and PKCS#7 padding (or unpadded when `auto_pad` is false).
pub fn aes_cbc_encrypt_with_expanded(
    expanded: &AesExpandedKey,
    iv: &[u8],
    plaintext: &[u8],
    auto_pad: bool,
) -> Result<Vec<u8>, &'static str> {
    if iv.len() != 16 {
        return Err("AES-CBC IV must be exactly 16 bytes");
    }
    if !auto_pad && !plaintext.len().is_multiple_of(16) {
        return Err("AES-CBC input length must be a multiple of 16 bytes when padding is disabled");
    }
    let pad_len = if auto_pad {
        16 - (plaintext.len() % 16)
    } else {
        0
    };
    let total_len = plaintext.len() + pad_len;
    let mut out = Vec::with_capacity(total_len);
    out.extend_from_slice(plaintext);
    if auto_pad {
        out.resize(total_len, pad_len as u8);
    }
    let mut prev = [0u8; 16];
    prev.copy_from_slice(iv);
    for chunk in out.chunks_exact_mut(16) {
        let mut block = [0u8; 16];
        for i in 0..16 {
            block[i] = chunk[i] ^ prev[i];
        }
        expanded.encrypt_block(&mut block);
        chunk.copy_from_slice(&block);
        prev = block;
    }
    Ok(out)
}

/// Decrypts `ciphertext` with AES-128-CBC or AES-256-CBC and verifies PKCS#7 padding in constant-style checks.
pub fn aes_cbc_decrypt_with_expanded(
    expanded: &AesExpandedKey,
    iv: &[u8],
    ciphertext: &[u8],
    auto_pad: bool,
) -> Result<Vec<u8>, &'static str> {
    if iv.len() != 16 {
        return Err("AES-CBC IV must be exactly 16 bytes");
    }
    if ciphertext.is_empty() && auto_pad {
        return Err("AES-CBC ciphertext cannot be empty");
    }
    if !ciphertext.len().is_multiple_of(16) {
        return Err("AES-CBC ciphertext length must be a multiple of 16 bytes");
    }
    let mut out = ciphertext.to_vec();
    let mut prev = [0u8; 16];
    prev.copy_from_slice(iv);
    for chunk in out.chunks_exact_mut(16) {
        let mut next_prev = [0u8; 16];
        next_prev.copy_from_slice(chunk);
        let mut block = next_prev;
        expanded.decrypt_block(&mut block);
        for i in 0..16 {
            chunk[i] = block[i] ^ prev[i];
        }
        prev = next_prev;
    }
    if auto_pad {
        let pad_byte = *out.last().ok_or("Empty decrypted buffer")? as usize;
        if pad_byte == 0 || pad_byte > 16 || pad_byte > out.len() {
            return Err("Invalid AES-CBC PKCS#7 padding");
        }
        let start = out.len() - pad_byte;
        let mut bad = 0u8;
        for &b in &out[start..] {
            bad |= b ^ (pad_byte as u8);
        }
        if bad != 0 {
            return Err("Invalid AES-CBC PKCS#7 padding bytes");
        }
        out.truncate(start);
    }
    Ok(out)
}

pub fn aes_cbc_encrypt(
    key: &[u8],
    iv: &[u8],
    plaintext: &[u8],
    auto_pad: bool,
) -> Result<Vec<u8>, &'static str> {
    let expanded = AesExpandedKey::new(key)?;
    aes_cbc_encrypt_with_expanded(&expanded, iv, plaintext, auto_pad)
}

pub fn aes_cbc_decrypt(
    key: &[u8],
    iv: &[u8],
    ciphertext: &[u8],
    auto_pad: bool,
) -> Result<Vec<u8>, &'static str> {
    let expanded = AesExpandedKey::new(key)?;
    aes_cbc_decrypt_with_expanded(&expanded, iv, ciphertext, auto_pad)
}

/// Builds a `ring::aead::LessSafeKey` for 128-bit (`16`-byte) or 256-bit (`32`-byte) AES-GCM.
#[inline]
pub fn make_ring_aead_key(key: &[u8]) -> Result<ring::aead::LessSafeKey, &'static str> {
    let alg = match key.len() {
        16 => &ring::aead::AES_128_GCM,
        32 => &ring::aead::AES_256_GCM,
        _ => return Err("AES-GCM key length must be 16 (128-bit) or 32 (256-bit) bytes"),
    };
    let unbound =
        ring::aead::UnboundKey::new(alg, key).map_err(|_| "Invalid AES-GCM key bytes")?;
    Ok(ring::aead::LessSafeKey::new(unbound))
}

/// Encrypts `plaintext` in-place using a pre-expanded `ring::aead::LessSafeKey`, returning `ciphertext || tag`.
#[inline]
pub fn aes_gcm_seal_with_key(
    less_safe_key: &ring::aead::LessSafeKey,
    iv: &[u8],
    aad: &[u8],
    plaintext: &[u8],
    tag_bits: usize,
) -> Result<Vec<u8>, &'static str> {
    if iv.len() != 12 {
        return Err("AES-GCM IV must be 12 bytes (96 bits)");
    }
    let tag_bytes = match tag_bits {
        96 | 104 | 112 | 120 | 128 => tag_bits / 8,
        _ => return Err("AES-GCM tagLength must be 96, 104, 112, 120, or 128 bits"),
    };
    let nonce =
        ring::aead::Nonce::try_assume_unique_for_key(iv).map_err(|_| "Invalid AES-GCM nonce")?;
    let mut buf = Vec::with_capacity(plaintext.len() + 16);
    buf.extend_from_slice(plaintext);
    let tag = less_safe_key
        .seal_in_place_separate_tag(nonce, ring::aead::Aad::from(aad), &mut buf)
        .map_err(|_| "AES-GCM encryption failed")?;
    buf.extend_from_slice(&tag.as_ref()[..tag_bytes]);
    Ok(buf)
}

/// Decrypts `ciphertext_and_tag` using a pre-expanded `ring::aead::LessSafeKey`, returning `plaintext`.
#[inline]
pub fn aes_gcm_open_with_key(
    less_safe_key: &ring::aead::LessSafeKey,
    iv: &[u8],
    aad: &[u8],
    ciphertext_and_tag: &[u8],
    tag_bits: usize,
) -> Result<Vec<u8>, &'static str> {
    if iv.len() != 12 {
        return Err("AES-GCM IV must be 12 bytes (96 bits)");
    }
    if tag_bits != 128 {
        return Err("AES-GCM decryption requires 128-bit authentication tag");
    }
    if ciphertext_and_tag.len() < 16 {
        return Err("AES-GCM ciphertext too short for authentication tag");
    }
    let nonce =
        ring::aead::Nonce::try_assume_unique_for_key(iv).map_err(|_| "Invalid AES-GCM nonce")?;
    let mut buf = ciphertext_and_tag.to_vec();
    let plain = less_safe_key
        .open_in_place(nonce, ring::aead::Aad::from(aad), &mut buf)
        .map_err(|_| "AES-GCM authentication tag verification failed")?;
    let plain_len = plain.len();
    buf.truncate(plain_len);
    Ok(buf)
}

#[inline]
pub fn aes_gcm_encrypt(
    key: &[u8],
    iv: &[u8],
    aad: &[u8],
    plaintext: &[u8],
    tag_bits: usize,
) -> Result<Vec<u8>, &'static str> {
    let k = make_ring_aead_key(key)?;
    aes_gcm_seal_with_key(&k, iv, aad, plaintext, tag_bits)
}

#[inline]
pub fn aes_gcm_decrypt(
    key: &[u8],
    iv: &[u8],
    aad: &[u8],
    ciphertext_and_tag: &[u8],
    tag_bits: usize,
) -> Result<Vec<u8>, &'static str> {
    let k = make_ring_aead_key(key)?;
    aes_gcm_open_with_key(&k, iv, aad, ciphertext_and_tag, tag_bits)
}

/// Derives `byte_len` key bytes using `ring::pbkdf2::derive` (`SHA-1`, `SHA-256`, `SHA-384`, `SHA-512`).
pub fn pbkdf2_derive_bytes(
    hash_alg: &str,
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    byte_len: usize,
) -> Result<Vec<u8>, &'static str> {
    if byte_len == 0 {
        return Ok(Vec::new());
    }
    let non_zero_iters =
        std::num::NonZeroU32::new(iterations).ok_or("PBKDF2 iterations must be > 0")?;
    let alg = if hash_alg.eq_ignore_ascii_case("SHA-256")
        || hash_alg.eq_ignore_ascii_case("SHA256")
    {
        ring::pbkdf2::PBKDF2_HMAC_SHA256
    } else if hash_alg.eq_ignore_ascii_case("SHA-384") || hash_alg.eq_ignore_ascii_case("SHA384") {
        ring::pbkdf2::PBKDF2_HMAC_SHA384
    } else if hash_alg.eq_ignore_ascii_case("SHA-512") || hash_alg.eq_ignore_ascii_case("SHA512") {
        ring::pbkdf2::PBKDF2_HMAC_SHA512
    } else if hash_alg.eq_ignore_ascii_case("SHA-1") || hash_alg.eq_ignore_ascii_case("SHA1") {
        ring::pbkdf2::PBKDF2_HMAC_SHA1
    } else {
        return Err("Unsupported PBKDF2 digest algorithm");
    };
    let mut out = vec![0u8; byte_len];
    ring::pbkdf2::derive(alg, non_zero_iters, salt, password, &mut out);
    Ok(out)
}

struct HkdfLen(usize);
impl ring::hkdf::KeyType for HkdfLen {
    fn len(&self) -> usize {
        self.0
    }
}

/// Derives `byte_len` key bytes using `ring::hkdf` (`SHA-1`, `SHA-256`, `SHA-384`, `SHA-512`).
pub fn hkdf_derive_bytes(
    hash_alg: &str,
    ikm: &[u8],
    salt: &[u8],
    info: &[u8],
    byte_len: usize,
) -> Result<Vec<u8>, &'static str> {
    if byte_len == 0 {
        return Ok(Vec::new());
    }
    let alg = if hash_alg.eq_ignore_ascii_case("SHA-256")
        || hash_alg.eq_ignore_ascii_case("SHA256")
    {
        ring::hkdf::HKDF_SHA256
    } else if hash_alg.eq_ignore_ascii_case("SHA-384") || hash_alg.eq_ignore_ascii_case("SHA384") {
        ring::hkdf::HKDF_SHA384
    } else if hash_alg.eq_ignore_ascii_case("SHA-512") || hash_alg.eq_ignore_ascii_case("SHA512") {
        ring::hkdf::HKDF_SHA512
    } else if hash_alg.eq_ignore_ascii_case("SHA-1") || hash_alg.eq_ignore_ascii_case("SHA1") {
        ring::hkdf::HKDF_SHA1_FOR_LEGACY_USE_ONLY
    } else {
        return Err("Unsupported HKDF digest algorithm");
    };
    let ring_salt = ring::hkdf::Salt::new(alg, salt);
    let prk = ring_salt.extract(ikm);
    let info_slices = [info];
    let okm = prk
        .expand(&info_slices, HkdfLen(byte_len))
        .map_err(|_| "HKDF output length exceeds maximum (255 * HashLen)")?;
    let mut out = vec![0u8; byte_len];
    okm.fill(&mut out).map_err(|_| "HKDF expand failed")?;
    Ok(out)
}

#[inline]
fn salsa20_8_core(b: &mut [u8; 64]) {
    let mut x = [0u32; 16];
    for (i, item) in x.iter_mut().enumerate() {
        let off = i * 4;
        *item = u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]]);
    }
    let mut w = x;
    for _ in 0..4 {
        w[4] ^= w[0].wrapping_add(w[12]).rotate_left(7);
        w[8] ^= w[4].wrapping_add(w[0]).rotate_left(9);
        w[12] ^= w[8].wrapping_add(w[4]).rotate_left(13);
        w[0] ^= w[12].wrapping_add(w[8]).rotate_left(18);
        w[9] ^= w[5].wrapping_add(w[1]).rotate_left(7);
        w[13] ^= w[9].wrapping_add(w[5]).rotate_left(9);
        w[1] ^= w[13].wrapping_add(w[9]).rotate_left(13);
        w[5] ^= w[1].wrapping_add(w[13]).rotate_left(18);
        w[14] ^= w[10].wrapping_add(w[6]).rotate_left(7);
        w[2] ^= w[14].wrapping_add(w[10]).rotate_left(9);
        w[6] ^= w[2].wrapping_add(w[14]).rotate_left(13);
        w[10] ^= w[6].wrapping_add(w[2]).rotate_left(18);
        w[3] ^= w[15].wrapping_add(w[11]).rotate_left(7);
        w[7] ^= w[3].wrapping_add(w[15]).rotate_left(9);
        w[11] ^= w[7].wrapping_add(w[3]).rotate_left(13);
        w[15] ^= w[11].wrapping_add(w[7]).rotate_left(18);

        w[1] ^= w[0].wrapping_add(w[3]).rotate_left(7);
        w[2] ^= w[1].wrapping_add(w[0]).rotate_left(9);
        w[3] ^= w[2].wrapping_add(w[1]).rotate_left(13);
        w[0] ^= w[3].wrapping_add(w[2]).rotate_left(18);
        w[6] ^= w[5].wrapping_add(w[4]).rotate_left(7);
        w[7] ^= w[6].wrapping_add(w[5]).rotate_left(9);
        w[4] ^= w[7].wrapping_add(w[6]).rotate_left(13);
        w[5] ^= w[4].wrapping_add(w[7]).rotate_left(18);
        w[11] ^= w[10].wrapping_add(w[9]).rotate_left(7);
        w[8] ^= w[11].wrapping_add(w[10]).rotate_left(9);
        w[9] ^= w[8].wrapping_add(w[11]).rotate_left(13);
        w[10] ^= w[9].wrapping_add(w[8]).rotate_left(18);
        w[12] ^= w[15].wrapping_add(w[14]).rotate_left(7);
        w[13] ^= w[12].wrapping_add(w[15]).rotate_left(9);
        w[14] ^= w[13].wrapping_add(w[12]).rotate_left(13);
        w[15] ^= w[14].wrapping_add(w[13]).rotate_left(18);
    }
    for i in 0..16 {
        let bytes = x[i].wrapping_add(w[i]).to_le_bytes();
        b[i * 4..i * 4 + 4].copy_from_slice(&bytes);
    }
}

fn scrypt_block_mix(b: &[u8], y: &mut [u8], r: usize) {
    let mut x = [0u8; 64];
    x.copy_from_slice(&b[(2 * r - 1) * 64..2 * r * 64]);
    for i in 0..(2 * r) {
        let block = &b[i * 64..(i + 1) * 64];
        for j in 0..64 {
            x[j] ^= block[j];
        }
        salsa20_8_core(&mut x);
        let dst_idx = if i % 2 == 0 { i / 2 } else { r + i / 2 };
        y[dst_idx * 64..(dst_idx + 1) * 64].copy_from_slice(&x);
    }
}

fn scrypt_ro_mix(b: &mut [u8], v: &mut [u8], y: &mut [u8], n: usize, r: usize) {
    let block_len = 128 * r;
    for i in 0..n {
        v[i * block_len..(i + 1) * block_len].copy_from_slice(b);
        scrypt_block_mix(b, y, r);
        b.copy_from_slice(y);
    }
    let mask = n - 1;
    let last_off = (2 * r - 1) * 64;
    for _ in 0..n {
        let j = (u32::from_le_bytes([
            b[last_off],
            b[last_off + 1],
            b[last_off + 2],
            b[last_off + 3],
        ]) as usize)
            & mask;
        let v_slice = &v[j * block_len..(j + 1) * block_len];
        for k in 0..block_len {
            b[k] ^= v_slice[k];
        }
        scrypt_block_mix(b, y, r);
        b.copy_from_slice(y);
    }
}

/// RFC 7914 `scrypt` key derivation function (`N`, `r`, `p`, `maxmem`).
pub fn scrypt_derive_bytes(
    password: &[u8],
    salt: &[u8],
    n: usize,
    r: usize,
    p: usize,
    dk_len: usize,
    maxmem: usize,
) -> Result<Vec<u8>, &'static str> {
    if dk_len == 0 {
        return Ok(Vec::new());
    }
    if n < 2 || !n.is_power_of_two() || r == 0 || p == 0 {
        return Err("Invalid scrypt parameters: N must be a power of two > 1, r > 0, p > 0");
    }
    let block_len = 128usize
        .checked_mul(r)
        .ok_or("scrypt block size overflow")?;
    let v_bytes = block_len
        .checked_mul(n)
        .ok_or("scrypt memory limit exceeded")?;
    let total_b = block_len
        .checked_mul(p)
        .ok_or("scrypt parallel buffer overflow")?;
    if v_bytes.saturating_add(total_b) > maxmem {
        return Err("Invalid scrypt parameters: memory limit exceeded (ERR_CRYPTO_SCRYPT_INVALID_PARAMETER)");
    }
    let mut b = pbkdf2_derive_bytes("SHA-256", password, salt, 1, total_b)?;
    let mut v = vec![0u8; v_bytes];
    let mut y = vec![0u8; block_len];
    for i in 0..p {
        scrypt_ro_mix(
            &mut b[i * block_len..(i + 1) * block_len],
            &mut v,
            &mut y,
            n,
            r,
        );
    }
    pbkdf2_derive_bytes("SHA-256", password, &b, 1, dk_len)
}

const ED25519_PKCS8_V1_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04,
    0x20,
];
const ED25519_SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

const ECDSA_P256_SPKI_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08,
    0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];
const ECDSA_P384_SPKI_PREFIX: [u8; 23] = [
    0x30, 0x76, 0x30, 0x10, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x05,
    0x2b, 0x81, 0x04, 0x00, 0x22, 0x03, 0x62, 0x00,
];

/// Wraps a 32-byte Ed25519 private seed and 32-byte public key into PKCS#8 v2 DER (accepted by `ring` and WebCrypto).
pub fn ed25519_seed_and_pub_to_pkcs8_v2(seed: &[u8; 32], pub_bytes: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(85);
    out.extend_from_slice(&[
        0x30, 0x53, 0x02, 0x01, 0x01, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04,
        0x20,
    ]);
    out.extend_from_slice(seed);
    out.extend_from_slice(&[0xa1, 0x23, 0x03, 0x21, 0x00]);
    out.extend_from_slice(pub_bytes);
    out
}

/// Wraps a 32-byte Ed25519 private seed into standard 48-byte PKCS#8 v1 DER for `exportKey("pkcs8")`.
#[inline]
pub fn ed25519_seed_to_pkcs8_v1(seed: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(48);
    out.extend_from_slice(&ED25519_PKCS8_V1_PREFIX);
    out.extend_from_slice(seed);
    out
}

/// Wraps a 32-byte Ed25519 public key into standard 44-byte SPKI DER for `exportKey("spki")`.
#[inline]
pub fn ed25519_pub_to_spki(pub_bytes: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(44);
    out.extend_from_slice(&ED25519_SPKI_PREFIX);
    out.extend_from_slice(pub_bytes);
    out
}

/// Extracts a 32-byte Ed25519 seed from raw 32-byte input, 48-byte PKCS#8 v1, or 83/85-byte PKCS#8 v2.
pub fn extract_ed25519_seed(input: &[u8]) -> Result<[u8; 32], &'static str> {
    if input.len() == 32 {
        let mut s = [0u8; 32];
        s.copy_from_slice(input);
        return Ok(s);
    }
    if input.len() >= 48 && input[..16] == ED25519_PKCS8_V1_PREFIX {
        let mut s = [0u8; 32];
        s.copy_from_slice(&input[16..48]);
        return Ok(s);
    }
    if input.len() >= 48 && input[12..16] == [0x04, 0x22, 0x04, 0x20] {
        let mut s = [0u8; 32];
        s.copy_from_slice(&input[16..48]);
        return Ok(s);
    }
    Err("Invalid Ed25519 private key or PKCS#8 encoding")
}

/// Extracts a 32-byte Ed25519 public key from raw 32-byte input or 44-byte SPKI DER.
pub fn extract_ed25519_pub(input: &[u8]) -> Result<[u8; 32], &'static str> {
    if input.len() == 32 {
        let mut p = [0u8; 32];
        p.copy_from_slice(input);
        return Ok(p);
    }
    if input.len() == 44 && input[..12] == ED25519_SPKI_PREFIX {
        let mut p = [0u8; 32];
        p.copy_from_slice(&input[12..44]);
        return Ok(p);
    }
    Err("Invalid Ed25519 public key or SPKI encoding")
}

/// Derives an Ed25519 `(seed, public_key)` pair from a 32-byte seed using `ring::signature::Ed25519KeyPair`.
pub fn ed25519_keypair_from_seed(seed: [u8; 32]) -> Result<([u8; 32], [u8; 32]), &'static str> {
    use ring::signature::KeyPair;
    let kp = ring::signature::Ed25519KeyPair::from_seed_unchecked(&seed)
        .map_err(|_| "Failed to derive Ed25519 key pair from seed")?;
    let mut pub_bytes = [0u8; 32];
    pub_bytes.copy_from_slice(kp.public_key().as_ref());
    Ok((seed, pub_bytes))
}

/// Signs `msg` with a 32-byte Ed25519 seed, returning the 64-byte signature.
#[inline]
pub fn ed25519_sign_bytes(seed: &[u8; 32], msg: &[u8]) -> Result<[u8; 64], &'static str> {
    let kp = ring::signature::Ed25519KeyPair::from_seed_unchecked(seed)
        .map_err(|_| "Invalid Ed25519 seed")?;
    let sig = kp.sign(msg);
    let mut out = [0u8; 64];
    out.copy_from_slice(sig.as_ref());
    Ok(out)
}

/// Verifies a 64-byte Ed25519 `signature` over `msg` using a 32-byte `pub_bytes`.
#[inline]
pub fn ed25519_verify_bytes(pub_bytes: &[u8], msg: &[u8], signature: &[u8]) -> bool {
    let peer = ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, pub_bytes);
    peer.verify(msg, signature).is_ok()
}

/// Wraps an ECDSA uncompressed public key (`0x04 || X || Y`) and private scalar `d` into PKCS#8 v1 DER
/// accepted by `ring::signature::EcdsaKeyPair::from_pkcs8`.
pub fn ecdsa_components_to_pkcs8(curve: &str, d: &[u8], pub_uncompressed: &[u8]) -> Result<Vec<u8>, &'static str> {
    if curve.eq_ignore_ascii_case("P-256") || curve.eq_ignore_ascii_case("prime256v1") {
        if d.len() != 32 || pub_uncompressed.len() != 65 || pub_uncompressed[0] != 0x04 {
            return Err("Invalid P-256 key component lengths");
        }
        let mut out = Vec::with_capacity(138);
        out.extend_from_slice(&[
            0x30, 0x81, 0x87, 0x02, 0x01, 0x00, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce,
            0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x04,
            0x6d, 0x30, 0x6b, 0x02, 0x01, 0x01, 0x04, 0x20,
        ]);
        out.extend_from_slice(d);
        out.extend_from_slice(&[0xa1, 0x44, 0x03, 0x42, 0x00]);
        out.extend_from_slice(pub_uncompressed);
        Ok(out)
    } else if curve.eq_ignore_ascii_case("P-384") || curve.eq_ignore_ascii_case("secp384r1") {
        if d.len() != 48 || pub_uncompressed.len() != 97 || pub_uncompressed[0] != 0x04 {
            return Err("Invalid P-384 key component lengths");
        }
        let mut out = Vec::with_capacity(185);
        out.extend_from_slice(&[
            0x30, 0x81, 0xb6, 0x02, 0x01, 0x00, 0x30, 0x10, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce,
            0x3d, 0x02, 0x01, 0x06, 0x05, 0x2b, 0x81, 0x04, 0x00, 0x22, 0x04, 0x81, 0x9e, 0x30,
            0x81, 0x9b, 0x02, 0x01, 0x01, 0x04, 0x30,
        ]);
        out.extend_from_slice(d);
        out.extend_from_slice(&[0xa1, 0x64, 0x03, 0x62, 0x00]);
        out.extend_from_slice(pub_uncompressed);
        Ok(out)
    } else {
        Err("Unsupported ECDSA curve (expected P-256 or P-384)")
    }
}

/// Extracts `(d_bytes, pub_uncompressed_bytes)` from a P-256 or P-384 PKCS#8 v1 DER buffer.
pub fn extract_ecdsa_pkcs8_components(
    curve: &str,
    pkcs8: &[u8],
) -> Result<(Vec<u8>, Vec<u8>), &'static str> {
    let (scalar_len, point_len) =
        if curve.eq_ignore_ascii_case("P-256") || curve.eq_ignore_ascii_case("prime256v1") {
            (32usize, 65usize)
        } else if curve.eq_ignore_ascii_case("P-384") || curve.eq_ignore_ascii_case("secp384r1") {
            (48usize, 97usize)
        } else {
            return Err("Unsupported ECDSA curve");
        };
    // Validate with `ring` first and obtain public key directly from `KeyPair::public_key()`
    use ring::signature::KeyPair;
    let rng = ring::rand::SystemRandom::new();
    let alg = if scalar_len == 32 {
        &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING
    } else {
        &ring::signature::ECDSA_P384_SHA384_FIXED_SIGNING
    };
    let kp = ring::signature::EcdsaKeyPair::from_pkcs8(alg, pkcs8, &rng)
        .map_err(|_| "Invalid ECDSA PKCS#8 private key")?;
    let pub_bytes = kp.public_key().as_ref().to_vec();
    if pub_bytes.len() != point_len {
        return Err("Unexpected ECDSA public key length");
    }
    // Locate the `[0x04, scalar_len]` OCTET STRING tag inside PKCS#8
    let marker = [0x02, 0x01, 0x01, 0x04, scalar_len as u8];
    if let Some(pos) = memchr::memmem::find(pkcs8, &marker) {
        let start = pos + marker.len();
        if start + scalar_len <= pkcs8.len() {
            return Ok((pkcs8[start..start + scalar_len].to_vec(), pub_bytes));
        }
    }
    Err("Failed to locate ECDSA private scalar in PKCS#8")
}

/// Wraps an uncompressed ECDSA public point (`0x04 || X || Y`) into SPKI DER.
pub fn ecdsa_pub_to_spki(curve: &str, pub_uncompressed: &[u8]) -> Result<Vec<u8>, &'static str> {
    if curve.eq_ignore_ascii_case("P-256") || curve.eq_ignore_ascii_case("prime256v1") {
        if pub_uncompressed.len() != 65 {
            return Err("Invalid P-256 uncompressed public point length");
        }
        let mut out = Vec::with_capacity(91);
        out.extend_from_slice(&ECDSA_P256_SPKI_PREFIX);
        out.extend_from_slice(pub_uncompressed);
        Ok(out)
    } else if curve.eq_ignore_ascii_case("P-384") || curve.eq_ignore_ascii_case("secp384r1") {
        if pub_uncompressed.len() != 97 {
            return Err("Invalid P-384 uncompressed public point length");
        }
        let mut out = Vec::with_capacity(120);
        out.extend_from_slice(&ECDSA_P384_SPKI_PREFIX);
        out.extend_from_slice(pub_uncompressed);
        Ok(out)
    } else {
        Err("Unsupported ECDSA curve")
    }
}

/// Extracts an uncompressed ECDSA public point (`0x04 || X || Y`) from raw or SPKI DER.
pub fn extract_ecdsa_pub(curve: &str, input: &[u8]) -> Result<Vec<u8>, &'static str> {
    if curve.eq_ignore_ascii_case("P-256") || curve.eq_ignore_ascii_case("prime256v1") {
        if input.len() == 65 && input[0] == 0x04 {
            return Ok(input.to_vec());
        }
        if input.len() == 91 && input[..26] == ECDSA_P256_SPKI_PREFIX {
            return Ok(input[26..91].to_vec());
        }
        Err("Invalid P-256 public key or SPKI DER")
    } else if curve.eq_ignore_ascii_case("P-384") || curve.eq_ignore_ascii_case("secp384r1") {
        if input.len() == 97 && input[0] == 0x04 {
            return Ok(input.to_vec());
        }
        if input.len() == 120 && input[..23] == ECDSA_P384_SPKI_PREFIX {
            return Ok(input[23..120].to_vec());
        }
        Err("Invalid P-384 public key or SPKI DER")
    } else {
        Err("Unsupported ECDSA curve")
    }
}

pub type EcdsaGeneratedKeypair = (Vec<u8>, Vec<u8>, Vec<u8>);

/// Generates a new ECDSA (`P-256` or `P-384`) keypair, returning `(pkcs8_bytes, d_bytes, pub_uncompressed_bytes)`.
pub fn ecdsa_generate_keypair(curve: &str) -> Result<EcdsaGeneratedKeypair, &'static str> {
    let rng = ring::rand::SystemRandom::new();
    let alg = if curve.eq_ignore_ascii_case("P-256") || curve.eq_ignore_ascii_case("prime256v1") {
        &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING
    } else if curve.eq_ignore_ascii_case("P-384") || curve.eq_ignore_ascii_case("secp384r1") {
        &ring::signature::ECDSA_P384_SHA384_FIXED_SIGNING
    } else {
        return Err("Unsupported ECDSA curve (expected P-256 or P-384)");
    };
    let doc = ring::signature::EcdsaKeyPair::generate_pkcs8(alg, &rng)
        .map_err(|_| "Failed to generate ECDSA key pair")?;
    let pkcs8 = doc.as_ref().to_vec();
    let (d, pub_bytes) = extract_ecdsa_pkcs8_components(curve, &pkcs8)?;
    Ok((pkcs8, d, pub_bytes))
}

/// Converts a fixed IEEE P1363 `(r || s)` ECDSA signature into ASN.1 DER `SEQUENCE { INTEGER r, INTEGER s }`.
pub fn ecdsa_fixed_to_der(fixed: &[u8]) -> Result<Vec<u8>, &'static str> {
    if fixed.len() != 64 && fixed.len() != 96 {
        return Err("Invalid fixed ECDSA signature length");
    }
    let half = fixed.len() / 2;
    let encode_int = |slice: &[u8]| -> Vec<u8> {
        let mut s = slice;
        while s.len() > 1 && s[0] == 0 {
            s = &s[1..];
        }
        let mut out = Vec::with_capacity(s.len() + 3);
        out.push(0x02);
        if (s[0] & 0x80) != 0 {
            out.push((s.len() + 1) as u8);
            out.push(0x00);
        } else {
            out.push(s.len() as u8);
        }
        out.extend_from_slice(s);
        out
    };
    let r_der = encode_int(&fixed[..half]);
    let s_der = encode_int(&fixed[half..]);
    let seq_len = r_der.len() + s_der.len();
    let mut out = Vec::with_capacity(seq_len + 3);
    out.push(0x30);
    if seq_len >= 128 {
        out.push(0x81);
    }
    out.push(seq_len as u8);
    out.extend_from_slice(&r_der);
    out.extend_from_slice(&s_der);
    Ok(out)
}

/// Signs `msg` using an ECDSA PKCS#8 key (`P-256` or `P-384`), returning IEEE P1363 (`64`/`96` bytes)
/// when `asn1_der` is `false` or ASN.1 DER when `asn1_der` is `true`.
pub fn ecdsa_sign_bytes(
    curve: &str,
    pkcs8: &[u8],
    msg: &[u8],
    asn1_der: bool,
) -> Result<Vec<u8>, &'static str> {
    let rng = ring::rand::SystemRandom::new();
    let alg = if curve.eq_ignore_ascii_case("P-256") || curve.eq_ignore_ascii_case("prime256v1") {
        &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING
    } else if curve.eq_ignore_ascii_case("P-384") || curve.eq_ignore_ascii_case("secp384r1") {
        &ring::signature::ECDSA_P384_SHA384_FIXED_SIGNING
    } else {
        return Err("Unsupported ECDSA curve");
    };
    let kp = ring::signature::EcdsaKeyPair::from_pkcs8(alg, pkcs8, &rng)
        .map_err(|_| "Invalid ECDSA PKCS#8 key")?;
    let sig = kp
        .sign(&rng, msg)
        .map_err(|_| "ECDSA signing operation failed")?;
    if asn1_der {
        ecdsa_fixed_to_der(sig.as_ref())
    } else {
        Ok(sig.as_ref().to_vec())
    }
}

/// Verifies an ECDSA (`P-256` or `P-384`) signature over `msg` (accepting both IEEE P1363 fixed and ASN.1 DER formats).
pub fn ecdsa_verify_bytes(curve: &str, pub_uncompressed: &[u8], msg: &[u8], signature: &[u8]) -> bool {
    let is_p256 = curve.eq_ignore_ascii_case("P-256") || curve.eq_ignore_ascii_case("prime256v1");
    let is_p384 = curve.eq_ignore_ascii_case("P-384") || curve.eq_ignore_ascii_case("secp384r1");
    if is_p256 {
        if signature.len() == 64 {
            let peer = ring::signature::UnparsedPublicKey::new(
                &ring::signature::ECDSA_P256_SHA256_FIXED,
                pub_uncompressed,
            );
            if peer.verify(msg, signature).is_ok() {
                return true;
            }
        }
        let peer_asn1 = ring::signature::UnparsedPublicKey::new(
            &ring::signature::ECDSA_P256_SHA256_ASN1,
            pub_uncompressed,
        );
        peer_asn1.verify(msg, signature).is_ok()
    } else if is_p384 {
        if signature.len() == 96 {
            let peer = ring::signature::UnparsedPublicKey::new(
                &ring::signature::ECDSA_P384_SHA384_FIXED,
                pub_uncompressed,
            );
            if peer.verify(msg, signature).is_ok() {
                return true;
            }
        }
        let peer_asn1 = ring::signature::UnparsedPublicKey::new(
            &ring::signature::ECDSA_P384_SHA384_ASN1,
            pub_uncompressed,
        );
        peer_asn1.verify(msg, signature).is_ok()
    } else {
        false
    }
}

/// Cached pre-expanded cryptographic key entry stored in the isolate-local / tenant-scoped slot table.
#[allow(dead_code, clippy::large_enum_variant)]
pub(crate) enum CachedCryptoKeyMaterial {
    AesGcm {
        less_safe: ring::aead::LessSafeKey,
        raw: Vec<u8>,
    },
    AesCbc {
        expanded: AesExpandedKey,
        raw: Vec<u8>,
    },
    Hmac {
        ring_key: ring::hmac::Key,
        hash_alg: String,
        raw: Vec<u8>,
    },
    Ed25519Private {
        seed: [u8; 32],
        pub_bytes: [u8; 32],
    },
    Ed25519Public {
        pub_bytes: [u8; 32],
    },
    EcdsaPrivate {
        curve: String,
        pkcs8: Vec<u8>,
        d_bytes: Vec<u8>,
        pub_bytes: Vec<u8>,
    },
    EcdsaPublic {
        curve: String,
        pub_bytes: Vec<u8>,
    },
}

pub(crate) struct TenantCachedCryptoKey {
    pub(crate) tenant_id: String,
    pub(crate) material: CachedCryptoKeyMaterial,
}

thread_local! {
    static NEXT_CRYPTO_KEY_HANDLE: std::cell::Cell<u32> = const { std::cell::Cell::new(1) };
    static CRYPTO_KEY_HANDLE_TABLE: std::cell::RefCell<rustc_hash::FxHashMap<u32, TenantCachedCryptoKey>> =
        std::cell::RefCell::new(rustc_hash::FxHashMap::default());
}

/// Inserts a pre-expanded `CachedCryptoKeyMaterial` into the tenant-scoped handle table, returning its opaque `u32` ID.
pub(crate) fn insert_cached_crypto_key(
    tenant_id: String,
    material: CachedCryptoKeyMaterial,
) -> u32 {
    let id = NEXT_CRYPTO_KEY_HANDLE.with(|c| {
        let cur = c.get();
        c.set(cur.wrapping_add(1).max(1));
        cur
    });
    CRYPTO_KEY_HANDLE_TABLE.with(|table| {
        let mut borrow = table.borrow_mut();
        if borrow.len() >= 1024 {
            borrow.retain(|_, v| v.tenant_id == tenant_id);
            if borrow.len() >= 1024 {
                borrow.clear();
            }
        }
        borrow.insert(
            id,
            TenantCachedCryptoKey {
                tenant_id,
                material,
            },
        );
    });
    id
}

/// Executes `f` with a reference to the cached `CachedCryptoKeyMaterial` for `handle_id` ONLY if its
/// `tenant_id` matches `expected_tenant`, enforcing strict multi-tenant key isolation.
#[inline]
pub(crate) fn with_cached_crypto_key<R>(
    handle_id: u32,
    expected_tenant: &str,
    f: impl FnOnce(&CachedCryptoKeyMaterial) -> R,
) -> Option<R> {
    if handle_id == 0 {
        return None;
    }
    CRYPTO_KEY_HANDLE_TABLE.with(|table| {
        let borrow = table.borrow();
        let entry = borrow.get(&handle_id)?;
        if entry.tenant_id != expected_tenant {
            return None;
        }
        Some(f(&entry.material))
    })
}

/// Clears all cached cryptographic key handles belonging to `tenant_id` (or all handles when `tenant_id` is empty).
#[allow(dead_code)]
pub fn clear_cached_crypto_keys_for_tenant(tenant_id: &str) {
    CRYPTO_KEY_HANDLE_TABLE.with(|table| {
        let mut borrow = table.borrow_mut();
        if tenant_id.is_empty() {
            borrow.clear();
        } else {
            borrow.retain(|_, v| v.tenant_id != tenant_id);
        }
    });
}

/// Strict WhatWG `btoa`: encodes a Latin1 (`0x00..=0xFF`) string to standard Base64,
/// returning `Err` if any code point exceeds `0xFF`.
pub(crate) fn whatwg_btoa(input: &str) -> Result<String, &'static str> {
    if input.is_ascii() {
        return Ok(base64_encode(input.as_bytes(), false));
    }
    let mut bytes = Vec::with_capacity(input.len());
    for ch in input.chars() {
        let code = ch as u32;
        if code > 0xFF {
            return Err("InvalidCharacterError: The string to be encoded contains characters outside of the Latin1 range.");
        }
        bytes.push(code as u8);
    }
    Ok(base64_encode(&bytes, false))
}

/// Strict WhatWG `atob`: strips ASCII whitespace, validates Base64 alphabet and padding,
/// and returns decoded Latin1 bytes (mapped 1:1 to `0x00..=0xFF` code points).
pub(crate) fn whatwg_atob(input: &str) -> Result<String, &'static str> {
    let cleaned: std::borrow::Cow<'_, [u8]> = if input
        .bytes()
        .any(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0c))
    {
        std::borrow::Cow::Owned(
            input
                .bytes()
                .filter(|&b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0c))
                .collect(),
        )
    } else {
        std::borrow::Cow::Borrowed(input.as_bytes())
    };
    if cleaned.len() % 4 == 1 {
        return Err("InvalidCharacterError: The string to be decoded is not correctly encoded.");
    }
    let mut out_chars = String::with_capacity((cleaned.len() * 3) / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    let mut pad_count = 0usize;
    for (idx, &b) in cleaned.iter().enumerate() {
        if b == b'=' {
            pad_count += 1;
            if pad_count > 2 || (idx + 1 < cleaned.len() && cleaned[idx + 1] != b'=') {
                return Err("InvalidCharacterError: Invalid padding in base64 string.");
            }
            continue;
        }
        if pad_count > 0 {
            return Err("InvalidCharacterError: Characters found after base64 padding.");
        }
        let val =
            match b {
                b'A'..=b'Z' => b - b'A',
                b'a'..=b'z' => b - b'a' + 26,
                b'0'..=b'9' => b - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                _ => return Err(
                    "InvalidCharacterError: The string to be decoded contains invalid characters.",
                ),
            };
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            let byte = ((buf >> bits) & 0xff) as u8;
            out_chars.push(byte as char);
        }
    }
    Ok(out_chars)
}

#[inline]
pub(crate) fn splitmix64_next_u64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

thread_local! {
    static HOST_CSPRNG_STATE: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[inline]
fn next_host_entropy_u64() -> u64 {
    HOST_CSPRNG_STATE.with(|cell| {
        let mut s = cell.get();
        if s == 0 {
            use std::hash::{BuildHasher, Hasher};
            let rs = std::collections::hash_map::RandomState::new();
            let mut h = rs.build_hasher();
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0xA5A5_5A5A_F0F0_0F0F);
            h.write_u64(nanos);
            h.write_usize(cell as *const _ as usize);
            s = h.finish() | 1;
        }
        let next = splitmix64_next_u64(&mut s);
        cell.set(s);
        next
    })
}

/// Fills `dest` with random bytes from either the deterministic SplitMix64 seed (`--deterministic`)
/// or the host entropy generator.
pub(crate) fn fill_random_bytes(dest: &mut [u8], mut det_prng: Option<&mut u64>) {
    let mut offset = 0;
    while offset < dest.len() {
        let word = match det_prng.as_deref_mut() {
            Some(state) => splitmix64_next_u64(state),
            None => next_host_entropy_u64(),
        };
        let bytes = word.to_le_bytes();
        let take = (dest.len() - offset).min(8);
        dest[offset..offset + take].copy_from_slice(&bytes[..take]);
        offset += take;
    }
}

/// Generates an RFC 4122 UUIDv4 string (`xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx`),
/// using deterministic PRNG state when active.
pub(crate) fn generate_uuid_v4(det_prng: Option<&mut u64>) -> String {
    let mut b = [0u8; 16];
    fill_random_bytes(&mut b, det_prng);
    // Set version 4 (0b0100_xxxx) and RFC 4122 variant (0b10xx_xxxx)
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;

    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = [0u8; 36];
    let mut j = 0;
    for (i, &byte) in b.iter().enumerate() {
        if i == 4 || i == 6 || i == 8 || i == 10 {
            out[j] = b'-';
            j += 1;
        }
        out[j] = HEX[(byte >> 4) as usize];
        out[j + 1] = HEX[(byte & 0x0f) as usize];
        j += 2;
    }
    unsafe { String::from_utf8_unchecked(out.to_vec()) }
}

// ============================================================================
// Wave J.2: Self-Contained RFC 1951 (DEFLATE), RFC 1950 (Zlib), & RFC 1952 (Gzip)
// ============================================================================

const MAX_DECOMPRESSED_BYTES: usize = 64 * 1024 * 1024;

const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
const CL_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

#[inline]
pub(crate) fn adler32_checksum(data: &[u8]) -> u32 {
    const MOD_ADLER: u32 = 65521;
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for chunk in data.chunks(5552) {
        for &byte in chunk {
            a += byte as u32;
            b += a;
        }
        a %= MOD_ADLER;
        b %= MOD_ADLER;
    }
    (b << 16) | a
}

#[inline]
pub(crate) fn crc32_checksum(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        let mut c = (crc ^ (byte as u32)) & 0xFF;
        for _ in 0..8 {
            c = if (c & 1) != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        crc = (crc >> 8) ^ c;
    }
    !crc
}

struct DeflateBitWriter {
    out: Vec<u8>,
    bit_buf: u32,
    bit_count: u8,
}

impl DeflateBitWriter {
    #[inline]
    fn with_capacity(cap: usize) -> Self {
        Self {
            out: Vec::with_capacity(cap),
            bit_buf: 0,
            bit_count: 0,
        }
    }

    #[inline]
    fn write_bits_lsb(&mut self, val: u32, nbits: u8) {
        self.bit_buf |= (val & ((1u32 << nbits) - 1)) << self.bit_count;
        self.bit_count += nbits;
        while self.bit_count >= 8 {
            self.out.push((self.bit_buf & 0xFF) as u8);
            self.bit_buf >>= 8;
            self.bit_count -= 8;
        }
    }

    #[inline]
    fn write_huffman_msb(&mut self, code: u16, nbits: u8) {
        let mut rev = 0u32;
        for i in 0..nbits {
            if ((code >> (nbits - 1 - i)) & 1) != 0 {
                rev |= 1u32 << i;
            }
        }
        self.write_bits_lsb(rev, nbits);
    }

    #[inline]
    fn write_fixed_sym(&mut self, sym: u16) {
        match sym {
            0..=143 => self.write_huffman_msb(0x30 + sym, 8),
            144..=255 => self.write_huffman_msb(0x190 + (sym - 144), 9),
            256..=279 => self.write_huffman_msb(sym - 256, 7),
            _ => self.write_huffman_msb(0xC0 + (sym - 280), 8),
        }
    }

    #[inline]
    fn write_length_dist(&mut self, length: usize, dist: usize) {
        let mut l_idx = 0usize;
        for (i, &base) in LENGTH_BASE.iter().enumerate() {
            if length >= base as usize {
                l_idx = i;
            } else {
                break;
            }
        }
        self.write_fixed_sym(257 + l_idx as u16);
        let l_extra = LENGTH_EXTRA[l_idx];
        if l_extra > 0 {
            self.write_bits_lsb((length - LENGTH_BASE[l_idx] as usize) as u32, l_extra);
        }

        let mut d_idx = 0usize;
        for (i, &base) in DIST_BASE.iter().enumerate() {
            if dist >= base as usize {
                d_idx = i;
            } else {
                break;
            }
        }
        self.write_huffman_msb(d_idx as u16, 5);
        let d_extra = DIST_EXTRA[d_idx];
        if d_extra > 0 {
            self.write_bits_lsb((dist - DIST_BASE[d_idx] as usize) as u32, d_extra);
        }
    }

    #[inline]
    fn finish(mut self) -> Vec<u8> {
        if self.bit_count > 0 {
            self.out.push((self.bit_buf & 0xFF) as u8);
        }
        self.out
    }
}

#[inline]
fn lz77_hash3(bytes: &[u8], pos: usize) -> usize {
    let v = (bytes[pos] as u32) | ((bytes[pos + 1] as u32) << 8) | ((bytes[pos + 2] as u32) << 16);
    ((v.wrapping_mul(0x1E35_A7BD)) >> 20) as usize & 0xFFF
}

/// Compresses `input` into a raw RFC 1951 DEFLATE bitstream using LZ77 + Fixed Huffman encoding.
pub(crate) fn deflate_raw_compress(input: &[u8]) -> Vec<u8> {
    let mut bw = DeflateBitWriter::with_capacity(input.len() / 2 + 16);
    // BFINAL = 1, BTYPE = 01 (Fixed Huffman)
    bw.write_bits_lsb(1, 1);
    bw.write_bits_lsb(1, 2);

    if input.len() < 3 {
        for &b in input {
            bw.write_fixed_sym(b as u16);
        }
        bw.write_fixed_sym(256);
        return bw.finish();
    }

    let mut head = [usize::MAX; 4096];
    let mut prev = vec![usize::MAX; input.len().min(32768)];
    let mask = prev.len() - 1;
    let mut pos = 0usize;

    while pos < input.len() {
        let mut best_len = 0usize;
        let mut best_dist = 0usize;

        if pos + 3 <= input.len() {
            let h = lz77_hash3(input, pos);
            let mut cand = head[h];
            head[h] = pos;
            if !prev.is_empty() {
                prev[pos & mask] = cand;
            }

            let max_len = (input.len() - pos).min(258);
            let mut chain_steps = 0usize;
            while cand != usize::MAX && pos > cand && (pos - cand) <= 32768 && chain_steps < 24 {
                chain_steps += 1;
                if input[cand + best_len] == input[pos + best_len]
                    && input[cand..cand + 3] == input[pos..pos + 3]
                {
                    let mut mlen = 3usize;
                    while mlen < max_len && input[cand + mlen] == input[pos + mlen] {
                        mlen += 1;
                    }
                    if mlen > best_len {
                        best_len = mlen;
                        best_dist = pos - cand;
                        if best_len == max_len {
                            break;
                        }
                    }
                }
                if prev.is_empty() {
                    break;
                }
                let next_cand = prev[cand & mask];
                if next_cand >= cand {
                    break;
                }
                cand = next_cand;
            }
        }

        if best_len >= 3 {
            bw.write_length_dist(best_len, best_dist);
            let step_end = (pos + best_len).min(input.len());
            pos += 1;
            while pos < step_end {
                if pos + 3 <= input.len() {
                    let h = lz77_hash3(input, pos);
                    let prev_head = head[h];
                    head[h] = pos;
                    if !prev.is_empty() {
                        prev[pos & mask] = prev_head;
                    }
                }
                pos += 1;
            }
        } else {
            bw.write_fixed_sym(input[pos] as u16);
            pos += 1;
        }
    }

    bw.write_fixed_sym(256);
    bw.finish()
}

struct HuffmanTree {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl HuffmanTree {
    fn from_code_lengths(lengths: &[u8]) -> Result<Self, &'static str> {
        let mut counts = [0u16; 16];
        for &len in lengths {
            if len as usize >= 16 {
                return Err("Invalid Huffman code length");
            }
            if len > 0 {
                counts[len as usize] += 1;
            }
        }
        let mut offsets = [0u16; 16];
        let mut total = 0u16;
        for bits in 1..16 {
            offsets[bits] = total;
            total += counts[bits];
        }
        let mut symbols = vec![0u16; total as usize];
        for (sym, &len) in lengths.iter().enumerate() {
            if len > 0 {
                let idx = offsets[len as usize] as usize;
                offsets[len as usize] += 1;
                symbols[idx] = sym as u16;
            }
        }
        Ok(Self { counts, symbols })
    }
}

struct InflateBitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_buf: u32,
    bit_count: u8,
}

impl<'a> InflateBitReader<'a> {
    #[inline]
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bit_buf: 0,
            bit_count: 0,
        }
    }

    #[inline]
    fn ensure_bits(&mut self, n: u8) -> Result<(), &'static str> {
        while self.bit_count < n {
            if self.byte_pos >= self.data.len() {
                return Err("Unexpected end of DEFLATE stream");
            }
            self.bit_buf |= (self.data[self.byte_pos] as u32) << self.bit_count;
            self.byte_pos += 1;
            self.bit_count += 8;
        }
        Ok(())
    }

    #[inline]
    fn read_bits(&mut self, n: u8) -> Result<u32, &'static str> {
        if n == 0 {
            return Ok(0);
        }
        self.ensure_bits(n)?;
        let val = self.bit_buf & ((1u32 << n) - 1);
        self.bit_buf >>= n;
        self.bit_count -= n;
        Ok(val)
    }

    #[inline]
    fn align_byte(&mut self) {
        let drop_bits = self.bit_count & 7;
        self.bit_buf >>= drop_bits;
        self.bit_count -= drop_bits;
    }

    #[inline]
    fn read_u16_aligned(&mut self) -> Result<u16, &'static str> {
        let lo = self.read_bits(8)? as u16;
        let hi = self.read_bits(8)? as u16;
        Ok(lo | (hi << 8))
    }

    #[inline]
    fn decode_huffman(&mut self, tree: &HuffmanTree) -> Result<u16, &'static str> {
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: i32 = 0;
        for len in 1..16 {
            code |= self.read_bits(1)? as i32;
            let count = tree.counts[len] as i32;
            if code - count < first {
                return tree
                    .symbols
                    .get((index + (code - first)) as usize)
                    .copied()
                    .ok_or("Corrupt Huffman symbol table");
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err("Invalid Huffman code in DEFLATE stream")
    }
}

/// Decompresses a raw RFC 1951 DEFLATE stream (`BTYPE = 00`, `01`, or `10`).
pub(crate) fn inflate_raw_decompress(input: &[u8]) -> Result<Vec<u8>, &'static str> {
    let effective_max = crate::boa_realm::BOA_KV_MEM_BUDGET.with(|m| {
        let (used, limit) = m.get();
        if limit > 0 {
            limit.saturating_sub(used).min(MAX_DECOMPRESSED_BYTES)
        } else {
            MAX_DECOMPRESSED_BYTES
        }
    });
    let mut br = InflateBitReader::new(input);
    let mut out = Vec::with_capacity(input.len().saturating_mul(2).min(65536).min(effective_max));

    loop {
        let bfinal = br.read_bits(1)?;
        let btype = br.read_bits(2)?;
        match btype {
            0 => {
                br.align_byte();
                let len = br.read_u16_aligned()?;
                let nlen = br.read_u16_aligned()?;
                if len != !nlen {
                    return Err("Corrupt stored DEFLATE block length complement");
                }
                if out.len().saturating_add(len as usize) > effective_max {
                    return Err("Decompressed payload exceeds sandbox memory limit");
                }
                for _ in 0..len {
                    out.push(br.read_bits(8)? as u8);
                }
            }
            1 | 2 => {
                let (lit_tree, dist_tree) = if btype == 1 {
                    let mut lit_lengths = [0u8; 288];
                    lit_lengths[0..144].fill(8);
                    lit_lengths[144..256].fill(9);
                    lit_lengths[256..280].fill(7);
                    lit_lengths[280..288].fill(8);
                    let dist_lengths = [5u8; 32];
                    (
                        HuffmanTree::from_code_lengths(&lit_lengths)?,
                        HuffmanTree::from_code_lengths(&dist_lengths)?,
                    )
                } else {
                    let hlit = br.read_bits(5)? as usize + 257;
                    let hdist = br.read_bits(5)? as usize + 1;
                    let hclen = br.read_bits(4)? as usize + 4;
                    if hlit > 286 || hdist > 30 {
                        return Err("Invalid dynamic Huffman table header");
                    }
                    let mut cl_lengths = [0u8; 19];
                    for i in 0..hclen {
                        cl_lengths[CL_ORDER[i]] = br.read_bits(3)? as u8;
                    }
                    let cl_tree = HuffmanTree::from_code_lengths(&cl_lengths)?;
                    let total_codes = hlit + hdist;
                    let mut unpacked = Vec::with_capacity(total_codes);
                    while unpacked.len() < total_codes {
                        let sym = br.decode_huffman(&cl_tree)?;
                        match sym {
                            0..=15 => unpacked.push(sym as u8),
                            16 => {
                                let prev = *unpacked
                                    .last()
                                    .ok_or("Invalid repeat code 16 at start of table")?;
                                let repeat = br.read_bits(2)? as usize + 3;
                                for _ in 0..repeat {
                                    unpacked.push(prev);
                                }
                            }
                            17 => {
                                let repeat = br.read_bits(3)? as usize + 3;
                                unpacked.resize(unpacked.len() + repeat, 0);
                            }
                            18 => {
                                let repeat = br.read_bits(7)? as usize + 11;
                                unpacked.resize(unpacked.len() + repeat, 0);
                            }
                            _ => return Err("Invalid code length symbol"),
                        }
                    }
                    if unpacked.len() > total_codes {
                        return Err("Dynamic Huffman code lengths overflow");
                    }
                    (
                        HuffmanTree::from_code_lengths(&unpacked[..hlit])?,
                        HuffmanTree::from_code_lengths(&unpacked[hlit..])?,
                    )
                };

                loop {
                    let sym = br.decode_huffman(&lit_tree)?;
                    if sym < 256 {
                        if out.len() >= effective_max {
                            return Err("Decompressed payload exceeds sandbox memory limit");
                        }
                        out.push(sym as u8);
                    } else if sym == 256 {
                        break;
                    } else {
                        let l_idx = (sym - 257) as usize;
                        if l_idx >= LENGTH_BASE.len() {
                            return Err("Invalid DEFLATE length code");
                        }
                        let length = LENGTH_BASE[l_idx] as usize
                            + br.read_bits(LENGTH_EXTRA[l_idx])? as usize;
                        let d_sym = br.decode_huffman(&dist_tree)? as usize;
                        if d_sym >= DIST_BASE.len() {
                            return Err("Invalid DEFLATE distance code");
                        }
                        let dist =
                            DIST_BASE[d_sym] as usize + br.read_bits(DIST_EXTRA[d_sym])? as usize;
                        if dist == 0 || dist > out.len() {
                            return Err("DEFLATE back-reference distance out of bounds");
                        }
                        if out.len().saturating_add(length) > effective_max {
                            return Err("Decompressed payload exceeds sandbox memory limit");
                        }
                        for _ in 0..length {
                            let b = out[out.len() - dist];
                            out.push(b);
                        }
                    }
                }
            }
            _ => return Err("Invalid DEFLATE block type (BTYPE=11)"),
        }

        if bfinal != 0 {
            break;
        }
    }

    Ok(out)
}

/// Compresses `input` using `"gzip"` (RFC 1952), `"deflate"` (RFC 1950 zlib), or `"deflate-raw"` (RFC 1951).
pub(crate) fn zlib_compress_bytes(input: &[u8], format: &str) -> Result<Vec<u8>, &'static str> {
    let raw = deflate_raw_compress(input);
    match format {
        "deflate-raw" => Ok(raw),
        "deflate" => {
            let mut out = Vec::with_capacity(raw.len() + 6);
            out.extend_from_slice(&[0x78, 0x9C]);
            out.extend_from_slice(&raw);
            let adler = adler32_checksum(input);
            out.extend_from_slice(&adler.to_be_bytes());
            Ok(out)
        }
        "gzip" => {
            let mut out = Vec::with_capacity(raw.len() + 18);
            out.extend_from_slice(&[0x1F, 0x8B, 0x08, 0x00, 0, 0, 0, 0, 0x00, 0xFF]);
            out.extend_from_slice(&raw);
            let crc = crc32_checksum(input);
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(input.len() as u32).to_le_bytes());
            Ok(out)
        }
        _ => Err("Unsupported compression format"),
    }
}

/// Decompresses `input` using `"gzip"` (RFC 1952), `"deflate"` (RFC 1950 zlib), or `"deflate-raw"` (RFC 1951).
pub(crate) fn zlib_decompress_bytes(input: &[u8], format: &str) -> Result<Vec<u8>, &'static str> {
    match format {
        "deflate-raw" => inflate_raw_decompress(input),
        "deflate" => {
            if input.len() < 6 {
                return Err("Truncated zlib stream");
            }
            let cmf = input[0];
            let flg = input[1];
            if (cmf & 0x0F) != 8 || !((cmf as u16) << 8 | (flg as u16)).is_multiple_of(31) {
                return Err("Invalid zlib header");
            }
            let start = if (flg & 0x20) != 0 { 6 } else { 2 };
            if input.len() < start + 4 {
                return Err("Truncated zlib stream");
            }
            let end = input.len() - 4;
            let out = inflate_raw_decompress(&input[start..end])?;
            let expected_adler =
                u32::from_be_bytes([input[end], input[end + 1], input[end + 2], input[end + 3]]);
            if adler32_checksum(&out) != expected_adler {
                return Err("Zlib Adler-32 checksum mismatch");
            }
            Ok(out)
        }
        "gzip" => {
            if input.len() < 18 {
                return Err("Truncated gzip stream");
            }
            if input[0] != 0x1F || input[1] != 0x8B || input[2] != 0x08 {
                return Err("Invalid gzip header magic");
            }
            let flg = input[3];
            let mut pos = 10usize;
            if (flg & 0x04) != 0 {
                if pos + 2 > input.len() {
                    return Err("Truncated gzip FEXTRA header");
                }
                let xlen = u16::from_le_bytes([input[pos], input[pos + 1]]) as usize;
                pos += 2 + xlen;
            }
            if (flg & 0x08) != 0 {
                while pos < input.len() && input[pos] != 0 {
                    pos += 1;
                }
                pos += 1;
            }
            if (flg & 0x10) != 0 {
                while pos < input.len() && input[pos] != 0 {
                    pos += 1;
                }
                pos += 1;
            }
            if (flg & 0x02) != 0 {
                pos += 2;
            }
            if pos + 8 > input.len() {
                return Err("Truncated gzip trailer");
            }
            let end = input.len() - 8;
            let out = inflate_raw_decompress(&input[pos..end])?;
            let expected_crc =
                u32::from_le_bytes([input[end], input[end + 1], input[end + 2], input[end + 3]]);
            let expected_isize = u32::from_le_bytes([
                input[end + 4],
                input[end + 5],
                input[end + 6],
                input[end + 7],
            ]);
            if crc32_checksum(&out) != expected_crc || (out.len() as u32) != expected_isize {
                return Err("Gzip CRC-32 or ISIZE trailer mismatch");
            }
            Ok(out)
        }
        _ => Err("Unsupported decompression format"),
    }
}
