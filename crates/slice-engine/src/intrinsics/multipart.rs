//! Pure RFC 7578 `multipart/form-data` Parser with Bounded Header & Part Limits.

/// Maximum byte length accepted for a `multipart/form-data` boundary parameter.
pub const MAX_MULTIPART_BOUNDARY_LEN: usize = 256;
/// Maximum number of parts accepted in a single `multipart/form-data` body.
pub const MAX_MULTIPART_PARTS: usize = 2_048;
/// Maximum header block size (`16 KiB`) accepted per multipart part.
pub const MAX_PART_HEADER_BYTES: usize = 16 * 1024;

/// A single decoded part from an RFC 7578 `multipart/form-data` payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedMultipartPart {
    pub name: String,
    pub filename: Option<String>,
    pub content_type: String,
    pub payload: Vec<u8>,
}

#[inline]
fn trim_ascii_bytes(mut s: &[u8]) -> &[u8] {
    while let Some((&first, rest)) = s.split_first() {
        if first == b' ' || first == b'\t' {
            s = rest;
        } else {
            break;
        }
    }
    while let Some((&last, rest)) = s.split_last() {
        if last == b' ' || last == b'\t' {
            s = rest;
        } else {
            break;
        }
    }
    s
}

/// Unescapes backslash-escaped characters and WHATWG `%22` / `%0A` / `%0D` sequences in
/// `Content-Disposition` attribute values.
pub fn unescape_multipart_attr(raw: &[u8]) -> String {
    let s = String::from_utf8_lossy(raw);
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            out.push(bytes[i + 1]);
            i += 2;
            continue;
        }
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let pair = &bytes[i + 1..i + 3];
            if pair.eq_ignore_ascii_case(b"22") {
                out.push(b'"');
                i += 3;
                continue;
            }
            if pair.eq_ignore_ascii_case(b"0a") {
                out.push(b'\n');
                i += 3;
                continue;
            }
            if pair.eq_ignore_ascii_case(b"0d") {
                out.push(b'\r');
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parses an RFC 7578 `Content-Disposition: form-data; name="..."; filename="..."` header value.
pub fn parse_content_disposition_form_data(
    raw: &[u8],
) -> Result<(String, Option<String>), &'static str> {
    let trimmed = trim_ascii_bytes(raw);
    let semi = memchr::memchr(b';', trimmed).unwrap_or(trimmed.len());
    let disp_type = trim_ascii_bytes(&trimmed[..semi]);
    if !disp_type.eq_ignore_ascii_case(b"form-data") {
        return Err("Failed to parse multipart/form-data: Content-Disposition is not form-data");
    }

    let mut name_opt: Option<String> = None;
    let mut filename_opt: Option<String> = None;
    let mut i = if semi < trimmed.len() {
        semi + 1
    } else {
        trimmed.len()
    };

    while i < trimmed.len() {
        while i < trimmed.len() && matches!(trimmed[i], b' ' | b'\t' | b';') {
            i += 1;
        }
        if i >= trimmed.len() {
            break;
        }
        let key_start = i;
        while i < trimmed.len() && trimmed[i] != b'=' && trimmed[i] != b';' {
            i += 1;
        }
        let key = trim_ascii_bytes(&trimmed[key_start..i]);
        if i >= trimmed.len() || trimmed[i] != b'=' {
            continue;
        }
        i += 1; // skip '='
        while i < trimmed.len() && matches!(trimmed[i], b' ' | b'\t') {
            i += 1;
        }
        let val_str = if i < trimmed.len() && trimmed[i] == b'"' {
            i += 1; // skip opening '"'
            let val_start = i;
            while i < trimmed.len() {
                if trimmed[i] == b'\\' && i + 1 < trimmed.len() {
                    i += 2;
                    continue;
                }
                if trimmed[i] == b'"' {
                    break;
                }
                i += 1;
            }
            if i >= trimmed.len() || trimmed[i] != b'"' {
                return Err("Failed to parse multipart/form-data: unterminated quoted parameter");
            }
            let raw_val = &trimmed[val_start..i];
            i += 1; // skip closing '"'
            while i < trimmed.len() && trimmed[i] != b';' {
                i += 1;
            }
            unescape_multipart_attr(raw_val)
        } else {
            let val_start = i;
            while i < trimmed.len() && trimmed[i] != b';' {
                i += 1;
            }
            unescape_multipart_attr(trim_ascii_bytes(&trimmed[val_start..i]))
        };

        if key.eq_ignore_ascii_case(b"name") {
            name_opt = Some(val_str);
        } else if key.eq_ignore_ascii_case(b"filename") {
            filename_opt = Some(val_str);
        }
    }

    let Some(name) = name_opt else {
        return Err("Failed to parse multipart/form-data: missing Content-Disposition name");
    };
    Ok((name, filename_opt))
}

#[inline]
fn classify_multipart_boundary_suffix(bytes: &[u8], after_delim: usize) -> Option<(bool, usize)> {
    if bytes.get(after_delim..after_delim + 2) == Some(b"--") {
        let mut p = after_delim + 2;
        while p < bytes.len() && matches!(bytes[p], b' ' | b'\t') {
            p += 1;
        }
        if p == bytes.len() {
            return Some((true, p));
        }
        if bytes.get(p..p + 2) == Some(b"\r\n") {
            return Some((true, p + 2));
        }
        if bytes[p] == b'\n' {
            return Some((true, p + 1));
        }
        return None;
    }

    let mut p = after_delim;
    while p < bytes.len() && matches!(bytes[p], b' ' | b'\t') {
        p += 1;
    }
    if bytes.get(p..p + 2) == Some(b"\r\n") {
        return Some((false, p + 2));
    }
    if p < bytes.len() && bytes[p] == b'\n' {
        return Some((false, p + 1));
    }
    None
}

/// Parses an RFC 7578 `multipart/form-data` body slice using SIMD `memchr::memmem` boundary scanning.
pub fn parse_rfc7578_multipart(
    bytes: &[u8],
    boundary: &str,
) -> Result<Vec<ParsedMultipartPart>, String> {
    if boundary.is_empty() || boundary.len() > MAX_MULTIPART_BOUNDARY_LEN {
        return Err("Failed to parse multipart/form-data: invalid boundary length".to_string());
    }

    let mut dash_boundary = Vec::with_capacity(2 + boundary.len());
    dash_boundary.extend_from_slice(b"--");
    dash_boundary.extend_from_slice(boundary.as_bytes());
    let finder = memchr::memmem::Finder::new(&dash_boundary);

    let mut search_pos = 0usize;
    let mut part_start_opt: Option<usize> = None;
    while search_pos < bytes.len() {
        let Some(rel) = finder.find(&bytes[search_pos..]) else {
            break;
        };
        let pos = search_pos + rel;
        let valid_line_start = pos == 0 || bytes[pos - 1] == b'\n';
        if valid_line_start
            && let Some((is_closing, next_pos)) =
                classify_multipart_boundary_suffix(bytes, pos + dash_boundary.len())
        {
            if is_closing {
                return Ok(Vec::new());
            }
            part_start_opt = Some(next_pos);
            break;
        }
        search_pos = pos + 1;
    }

    let Some(mut part_start) = part_start_opt else {
        return Err("Failed to parse multipart/form-data: missing boundary".to_string());
    };

    let mut parts = Vec::new();
    loop {
        if parts.len() >= MAX_MULTIPART_PARTS {
            return Err(
                "Failed to parse multipart/form-data: part count exceeds limit".to_string(),
            );
        }

        let header_limit = bytes
            .len()
            .min(part_start.saturating_add(MAX_PART_HEADER_BYTES + 4));
        let mut cursor = part_start;
        let mut disp_raw: Option<&[u8]> = None;
        let mut content_type_opt: Option<String> = None;
        let mut body_start_opt: Option<usize> = None;

        while cursor < header_limit {
            let Some(nl_rel) = memchr::memchr(b'\n', &bytes[cursor..header_limit]) else {
                return Err(
                    "Failed to parse multipart/form-data: unterminated part headers".to_string(),
                );
            };
            let line_end = cursor + nl_rel;
            let line = if line_end > cursor && bytes[line_end - 1] == b'\r' {
                &bytes[cursor..line_end - 1]
            } else {
                &bytes[cursor..line_end]
            };
            cursor = line_end + 1;

            if line.is_empty() {
                body_start_opt = Some(cursor);
                break;
            }

            let Some(colon_idx) = memchr::memchr(b':', line) else {
                return Err(
                    "Failed to parse multipart/form-data: malformed part header".to_string()
                );
            };
            let h_name = trim_ascii_bytes(&line[..colon_idx]);
            let h_val = trim_ascii_bytes(&line[colon_idx + 1..]);
            if h_name.eq_ignore_ascii_case(b"content-disposition") {
                disp_raw = Some(h_val);
            } else if h_name.eq_ignore_ascii_case(b"content-type") {
                content_type_opt = Some(String::from_utf8_lossy(h_val).into_owned());
            }
        }

        let Some(body_start) = body_start_opt else {
            return Err(
                "Failed to parse multipart/form-data: part headers exceed 16 KiB limit".to_string(),
            );
        };
        let Some(disp_bytes) = disp_raw else {
            return Err(
                "Failed to parse multipart/form-data: missing Content-Disposition header"
                    .to_string(),
            );
        };
        let (name, filename) =
            parse_content_disposition_form_data(disp_bytes).map_err(str::to_string)?;

        let mut next_search = body_start;
        let mut found_next: Option<(usize, bool, usize)> = None;
        while next_search < bytes.len() {
            let Some(rel) = finder.find(&bytes[next_search..]) else {
                break;
            };
            let delim_pos = next_search + rel;
            let preceded_by_nl = delim_pos > body_start && bytes[delim_pos - 1] == b'\n';
            if preceded_by_nl
                && let Some((is_closing, after_next)) =
                    classify_multipart_boundary_suffix(bytes, delim_pos + dash_boundary.len())
            {
                let payload_end =
                    if delim_pos >= body_start + 2 && &bytes[delim_pos - 2..delim_pos] == b"\r\n" {
                        delim_pos - 2
                    } else {
                        delim_pos - 1
                    };
                found_next = Some((payload_end, is_closing, after_next));
                break;
            }
            next_search = delim_pos + 1;
        }

        let Some((payload_end, is_closing, after_next)) = found_next else {
            return Err(
                "Failed to parse multipart/form-data: missing closing boundary".to_string(),
            );
        };

        let ct = content_type_opt.unwrap_or_else(|| "text/plain".to_string());
        parts.push(ParsedMultipartPart {
            name,
            filename,
            content_type: ct,
            payload: bytes[body_start..payload_end].to_vec(),
        });

        if is_closing {
            return Ok(parts);
        }
        part_start = after_next;
    }
}
