//! Centralized Multi-Encoding Outbound Egress Secret DLP Scanner (`AhoCorasickKind::DFA`).
//!
//! Expands registered audit secrets into raw, lowercase/uppercase hexadecimal, percent-encoded
//! (`encodeURIComponent`, `application/x-www-form-urlencoded`, and full `%XX`/`%xx`), and all 3
//! Base64 / Base64URL byte-alignment (`offset = 0, 1, 2`) interior slices, then compiles and caches
//! a thread-local `AhoCorasickKind::DFA` automaton for single-pass `O(candidate)` scanning.

use crate::compartment::SandboxViolation;

#[inline]
fn base64_encode_dlp(data: &[u8], url_safe: bool) -> String {
    const STD_TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    const URL_TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let table = if url_safe { URL_TABLE } else { STD_TABLE };
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    let (chunks, rem) = data.as_chunks::<3>();
    for chunk in chunks {
        let triple = ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | (chunk[2] as u32);
        out.push(table[((triple >> 18) & 0x3f) as usize] as char);
        out.push(table[((triple >> 12) & 0x3f) as usize] as char);
        out.push(table[((triple >> 6) & 0x3f) as usize] as char);
        out.push(table[(triple & 0x3f) as usize] as char);
    }
    if !rem.is_empty() {
        let b0 = rem[0] as u32;
        let b1 = if rem.len() > 1 { rem[1] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8);
        out.push(table[((triple >> 18) & 0x3f) as usize] as char);
        out.push(table[((triple >> 12) & 0x3f) as usize] as char);
        if rem.len() > 1 {
            out.push(table[((triple >> 6) & 0x3f) as usize] as char);
        } else if !url_safe {
            out.push('=');
        }
        if !url_safe {
            out.push('=');
        }
    }
    out
}

#[inline]
fn push_unique_dlp_pattern(out: &mut Vec<String>, pat: String) {
    if !pat.is_empty() && !out.contains(&pat) {
        out.push(pat);
    }
}

#[inline]
fn hex_encode_bytes(bytes: &[u8], uppercase: bool) -> String {
    let lut: &[u8; 16] = if uppercase {
        b"0123456789ABCDEF"
    } else {
        b"0123456789abcdef"
    };
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(lut[(b >> 4) as usize] as char);
        out.push(lut[(b & 0x0f) as usize] as char);
    }
    out
}

#[inline]
fn url_encode_uri_component(bytes: &[u8], uppercase: bool, form_plus: bool) -> String {
    let lut: &[u8; 16] = if uppercase {
        b"0123456789ABCDEF"
    } else {
        b"0123456789abcdef"
    };
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        if form_plus && b == b' ' {
            out.push('+');
            continue;
        }
        let unescaped = if form_plus {
            b.is_ascii_alphanumeric() || matches!(b, b'*' | b'-' | b'.' | b'_')
        } else {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
                )
        };
        if unescaped {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(lut[(b >> 4) as usize] as char);
            out.push(lut[(b & 0x0f) as usize] as char);
        }
    }
    out
}

#[inline]
fn full_percent_encode_bytes(bytes: &[u8], uppercase: bool) -> String {
    let lut: &[u8; 16] = if uppercase {
        b"0123456789ABCDEF"
    } else {
        b"0123456789abcdef"
    };
    let mut out = String::with_capacity(bytes.len() * 3);
    for &b in bytes {
        out.push('%');
        out.push(lut[(b >> 4) as usize] as char);
        out.push(lut[(b & 0x0f) as usize] as char);
    }
    out
}

/// Expands a registered audit `secret` into its deterministic exfiltration encodings:
/// - Raw `secret`
/// - Standard Base64 (`=`-padded and unpadded) and URL-safe Base64 (`-`, `_`) at offset 0
/// - All 3 Base64 / Base64URL triplet byte-alignment (`offset = 0, 1, 2`) interior quantum
///   slices (catching `btoa(prefix + secret + suffix)` for any surrounding prefix/suffix)
/// - Lowercase and uppercase hexadecimal (`hex_lower`, `hex_upper`) and full `%xx`/`%XX`
///   percent-encoding for secrets `>= 3` bytes
/// - `encodeURIComponent` and `URLSearchParams` (`+` form-encoded) percent-encodings
pub fn expand_egress_secret_variants(secret: &str, out: &mut Vec<String>) {
    if secret.is_empty() {
        return;
    }
    let bytes = secret.as_bytes();
    push_unique_dlp_pattern(out, secret.to_string());

    // Offset-0 full Base64 (padded + unpadded) and Base64URL
    let b64_std = base64_encode_dlp(bytes, false);
    let b64_url = base64_encode_dlp(bytes, true);
    let b64_std_unpadded = b64_std.trim_end_matches('=').to_string();
    push_unique_dlp_pattern(out, b64_std);
    push_unique_dlp_pattern(out, b64_std_unpadded);
    push_unique_dlp_pattern(out, b64_url);

    // All 3 byte-alignment (`offset = 0, 1, 2`) interior Base64 & Base64URL slices.
    // For `offset` leading pad bytes (`P = [0u8; offset] + bytes`, `total = offset + len`):
    // - `start_char`: `0` (`offset = 0`), `2` (`offset = 1`), `3` (`offset = 2`)
    // - `end_char`: `(total / 3) * 4` (drops any trailing partial quantum mixed with suffix bytes)
    if bytes.len() >= 5 {
        let mut scratch = Vec::with_capacity(bytes.len() + 2);
        for (offset, start_char) in [(0usize, 0usize), (1usize, 2usize), (2usize, 3usize)] {
            scratch.clear();
            scratch.resize(offset, 0u8);
            scratch.extend_from_slice(bytes);
            let end_char = (scratch.len() / 3) * 4;
            if end_char >= start_char + 5 {
                let enc_std = base64_encode_dlp(&scratch, false);
                if end_char <= enc_std.len() {
                    push_unique_dlp_pattern(out, enc_std[start_char..end_char].to_string());
                }
                let enc_url = base64_encode_dlp(&scratch, true);
                if end_char <= enc_url.len() {
                    push_unique_dlp_pattern(out, enc_url[start_char..end_char].to_string());
                }
            }
        }
    }

    // Hexadecimal and full `%xx`/`%XX` encodings for secrets `>= 3` bytes
    if bytes.len() >= 3 {
        push_unique_dlp_pattern(out, hex_encode_bytes(bytes, false));
        push_unique_dlp_pattern(out, hex_encode_bytes(bytes, true));
        push_unique_dlp_pattern(out, full_percent_encode_bytes(bytes, false));
        push_unique_dlp_pattern(out, full_percent_encode_bytes(bytes, true));
    }

    // `encodeURIComponent` and `application/x-www-form-urlencoded` percent-encodings
    let uri_upper = url_encode_uri_component(bytes, true, false);
    if uri_upper != secret {
        push_unique_dlp_pattern(out, uri_upper);
        push_unique_dlp_pattern(out, url_encode_uri_component(bytes, false, false));
    }
    let form_upper = url_encode_uri_component(bytes, true, true);
    if form_upper != secret {
        push_unique_dlp_pattern(out, form_upper);
        push_unique_dlp_pattern(out, url_encode_uri_component(bytes, false, true));
    }
}

#[inline]
fn hash_dlp_pattern_slice(secrets: &[String]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = rustc_hash::FxHasher::default();
    secrets.len().hash(&mut h);
    for s in secrets {
        s.hash(&mut h);
    }
    h.finish()
}

/// Returns `true` if `secrets` was already produced by [`expand_egress_secret_variants`]
/// (avoiding double-expansion when callers pre-expand at registration time, while automatically
/// expanding raw secret slices passed directly to [`verify_egress_secrets_slice`]).
#[inline]
fn is_already_expanded_dlp_slice(secrets: &[String]) -> bool {
    let Some(first) = secrets.iter().find(|s| !s.is_empty()) else {
        return true;
    };
    let expected_b64 = base64_encode_dlp(first.as_bytes(), false);
    secrets.contains(&expected_b64)
}

type EgressDlpCacheEntry = (u64, usize, std::sync::Arc<aho_corasick::AhoCorasick>);

thread_local! {
    static EGRESS_DLP_AUTOMATON_CACHE: std::cell::RefCell<
        smallvec::SmallVec<[EgressDlpCacheEntry; 8]>,
    > = const { std::cell::RefCell::new(smallvec::SmallVec::new_const()) };
}

#[inline]
fn get_or_build_dlp_automaton(
    secrets: &[String],
) -> Option<std::sync::Arc<aho_corasick::AhoCorasick>> {
    if secrets.is_empty() {
        return None;
    }
    let key_hash = hash_dlp_pattern_slice(secrets);
    let key_len = secrets.len();
    EGRESS_DLP_AUTOMATON_CACHE.with(|cache_cell| {
        {
            let cache = cache_cell.borrow();
            for (h, len, ac) in cache.iter().rev() {
                if *h == key_hash && *len == key_len {
                    return Some(std::sync::Arc::clone(ac));
                }
            }
        }
        let expanded_owned: Vec<String>;
        let pattern_source: &[String] = if is_already_expanded_dlp_slice(secrets) {
            secrets
        } else {
            let mut out = Vec::with_capacity(secrets.len() * 14);
            for s in secrets {
                expand_egress_secret_variants(s, &mut out);
            }
            expanded_owned = out;
            &expanded_owned
        };
        let non_empty: Vec<&[u8]> = pattern_source
            .iter()
            .filter(|s| !s.is_empty())
            .map(String::as_bytes)
            .collect();
        if non_empty.is_empty() {
            return None;
        }
        let ac = aho_corasick::AhoCorasick::builder()
            .kind(Some(aho_corasick::AhoCorasickKind::DFA))
            .build(&non_empty)
            .or_else(|_| aho_corasick::AhoCorasick::new(&non_empty))
            .ok()?;
        let arc = std::sync::Arc::new(ac);
        let mut cache = cache_cell.borrow_mut();
        if cache.len() >= 8 {
            cache.remove(0);
        }
        cache.push((key_hash, key_len, std::sync::Arc::clone(&arc)));
        Some(arc)
    })
}

/// Scans `candidate` in a single `O(candidate)` pass using a thread-local cached
/// `AhoCorasickKind::DFA` automaton built over all raw, hex, percent-encoded, and
/// 0/1/2-byte-shifted Base64 + Base64URL secret variants.
#[inline]
pub fn verify_egress_secrets_slice(
    tenant_id: &str,
    secrets: &[String],
    candidate: &str,
) -> Result<(), SandboxViolation> {
    if secrets.is_empty() || candidate.is_empty() {
        return Ok(());
    }
    let candidate_bytes = candidate.as_bytes();
    if let Some(ac) = get_or_build_dlp_automaton(secrets) {
        if ac.is_match(candidate_bytes) {
            return Err(SandboxViolation::EgressSecretLeakBlocked {
                tenant_id: tenant_id.to_string(),
            });
        }
        return Ok(());
    }
    for pat in secrets {
        if !pat.is_empty() && memchr::memmem::find(candidate_bytes, pat.as_bytes()).is_some() {
            return Err(SandboxViolation::EgressSecretLeakBlocked {
                tenant_id: tenant_id.to_string(),
            });
        }
    }
    Ok(())
}
