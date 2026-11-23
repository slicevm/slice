//! Zero-Trust Hop-by-Hop Redirect Guard, Proxy Selection (`HTTP_PROXY`/`HTTPS_PROXY`/`NO_PROXY`), and Wire Response Serialization.

use super::OutboundUrlParts;
use super::http11_wire::{OutboundHopDialPlan, RawHttpResponse, execute_single_http11_hop};
use super::ocap_and_ssrf::{
    check_boa_outbound_fetch_ocap_and_dlp, parse_outbound_http_url,
    resolve_and_verify_ssrf_socket_addrs,
};
use crate::boa_realm::BOA_ENV_MAP;
use serde::Serialize;

const MAX_REDIRECT_HOPS: usize = 5;

fn resolve_redirect_url(
    base_url: &str,
    base_parts: &OutboundUrlParts,
    location: &str,
) -> Option<String> {
    let loc = location.trim();
    if loc.is_empty() {
        return None;
    }
    if loc.starts_with("http://") || loc.starts_with("https://") {
        return Some(loc.to_string());
    }
    let scheme = if base_parts.is_https { "https" } else { "http" };
    if let Some(scheme_rel) = loc.strip_prefix("//") {
        return Some(format!("{}:{}", scheme, scheme_rel));
    }
    let origin = format!("{}://{}", scheme, base_parts.host_header);
    if loc.starts_with('/') {
        return Some(format!("{}{}", origin, loc));
    }
    let base_path = base_parts.path_and_query.split('?').next().unwrap_or("/");
    let dir = match base_path.rfind('/') {
        Some(idx) => &base_path[..=idx],
        None => "/",
    };
    let _ = base_url;
    Some(format!("{}{}{}", origin, dir, loc))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireFetchResponse {
    status: u16,
    status_text: String,
    headers: Vec<(String, String)>,
    body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    body_base64: Option<String>,
    final_url: String,
    redirected: bool,
}

impl WireFetchResponse {
    fn from_raw(resp: RawHttpResponse, final_url: String, redirected: bool) -> Self {
        match String::from_utf8(resp.body) {
            Ok(utf8_body) => Self {
                status: resp.status,
                status_text: resp.status_text,
                headers: resp.headers,
                body: utf8_body,
                body_base64: None,
                final_url,
                redirected,
            },
            Err(err) => {
                let bytes = err.into_bytes();
                let b64 = crate::intrinsics::base64_encode(&bytes, false);
                let lossy = String::from_utf8_lossy(&bytes).into_owned();
                Self {
                    status: resp.status,
                    status_text: resp.status_text,
                    headers: resp.headers,
                    body: lossy,
                    body_base64: Some(b64),
                    final_url,
                    redirected,
                }
            }
        }
    }
}

fn is_host_bypassed_by_no_proxy(hostname: &str, no_proxy_raw: &str) -> bool {
    let host_lower = hostname.to_ascii_lowercase();
    for item in no_proxy_raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if item == "*" {
            return true;
        }
        let pat = item
            .split(':')
            .next()
            .unwrap_or(item)
            .trim_start_matches('.')
            .to_ascii_lowercase();
        if pat.is_empty() {
            continue;
        }
        if host_lower == pat || host_lower.ends_with(&format!(".{}", pat)) {
            return true;
        }
    }
    false
}

fn resolve_active_proxy_url(is_https: bool, target_hostname: &str) -> Option<String> {
    BOA_ENV_MAP.with(|m| {
        let map = m.borrow();
        if map.is_empty() {
            return None;
        }
        let no_proxy = map
            .get("NO_PROXY")
            .or_else(|| map.get("no_proxy"))
            .map(String::as_str)
            .unwrap_or("");
        if !no_proxy.is_empty() && is_host_bypassed_by_no_proxy(target_hostname, no_proxy) {
            return None;
        }
        let proxy_val = if is_https {
            map.get("HTTPS_PROXY")
                .or_else(|| map.get("https_proxy"))
                .or_else(|| map.get("HTTP_PROXY"))
                .or_else(|| map.get("http_proxy"))
        } else {
            map.get("HTTP_PROXY").or_else(|| map.get("http_proxy"))
        };
        proxy_val
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    })
}

/// Executes an outbound HTTP/1.1 or HTTPS (TLS 1.3/1.2) `fetch()` request with:
/// - Zero-Trust Hop-by-Hop Redirect Guard (`301`/`302`/`303`/`307`/`308`, up to 5 hops)
/// - Per-hop OCap host allowlist + Pre-Connect DNS Resolved-IP SSRF Firewall + Egress DLP
/// - Cross-origin redirect credential stripping (`Authorization`, `Cookie`, `Proxy-Authorization`)
/// - Tenant-scoped Keep-Alive connection pooling (`SHARED_OUTBOUND_CONN_POOL`)
pub(crate) fn execute_outbound_fetch_with_redirects(
    initial_url: &str,
    initial_method: &str,
    initial_headers: &[(String, String)],
    initial_body: &[u8],
    redirect_mode: &str,
) -> Result<String, String> {
    let mut current_url = initial_url.to_string();
    let mut current_method = initial_method.to_string();
    let mut current_headers: Vec<(String, String)> = initial_headers.to_vec();
    let mut current_body = initial_body.to_vec();
    let mut redirected = false;

    for hop in 0..=MAX_REDIRECT_HOPS {
        let parts = parse_outbound_http_url(&current_url)
            .ok_or_else(|| format!("Invalid outbound URL: {}", current_url))?;
        let headers_json =
            serde_json::to_string(&current_headers).unwrap_or_else(|_| "[]".to_string());
        let explicit_host_grant = check_boa_outbound_fetch_ocap_and_dlp(
            &parts,
            &current_url,
            &headers_json,
            &current_body,
        )?;
        let resp = if let Some(proxy_url) =
            resolve_active_proxy_url(parts.is_https, &parts.hostname)
        {
            let p_parts = parse_outbound_http_url(&proxy_url)
                .ok_or_else(|| format!("Invalid proxy URL: {}", proxy_url))?;
            let proxy_grant = check_boa_outbound_fetch_ocap_and_dlp(
                &p_parts,
                &proxy_url,
                &headers_json,
                &current_body,
            )?;
            let proxy_addrs = resolve_and_verify_ssrf_socket_addrs(&p_parts, proxy_grant)?;
            if parts.is_https {
                execute_single_http11_hop(
                    &parts,
                    &parts.path_and_query,
                    OutboundHopDialPlan::HttpsConnectProxy {
                        proxy_parts: &p_parts,
                        proxy_addrs: &proxy_addrs,
                    },
                    &current_method,
                    &current_headers,
                    &current_body,
                )?
            } else {
                let abs_target = format!("http://{}{}", parts.host_header, parts.path_and_query);
                execute_single_http11_hop(
                    &parts,
                    &abs_target,
                    OutboundHopDialPlan::HttpProxy {
                        proxy_parts: &p_parts,
                        proxy_addrs: &proxy_addrs,
                    },
                    &current_method,
                    &current_headers,
                    &current_body,
                )?
            }
        } else {
            let verified_addrs = resolve_and_verify_ssrf_socket_addrs(&parts, explicit_host_grant)?;
            execute_single_http11_hop(
                &parts,
                &parts.path_and_query,
                OutboundHopDialPlan::Direct {
                    verified_addrs: &verified_addrs,
                },
                &current_method,
                &current_headers,
                &current_body,
            )?
        };

        let is_redirect = matches!(resp.status, 301 | 302 | 303 | 307 | 308);
        if is_redirect {
            let location_opt = resp
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("location"))
                .map(|(_, v)| v.clone());

            if let Some(location) = location_opt {
                if redirect_mode == "manual" {
                    return serde_json::to_string(&WireFetchResponse::from_raw(
                        resp,
                        current_url,
                        redirected,
                    ))
                    .map_err(|e| e.to_string());
                }
                if redirect_mode == "error" {
                    return Err(format!(
                        "RedirectError: fetch redirect mode is 'error' (received {} -> {})",
                        resp.status, location
                    ));
                }
                if hop == MAX_REDIRECT_HOPS {
                    return Err(format!(
                        "RedirectError: maximum redirect hops ({}) exceeded",
                        MAX_REDIRECT_HOPS
                    ));
                }

                let next_url = resolve_redirect_url(&current_url, &parts, &location)
                    .ok_or_else(|| format!("Invalid redirect Location header: {}", location))?;
                let next_parts = parse_outbound_http_url(&next_url)
                    .ok_or_else(|| format!("Invalid redirect target URL: {}", next_url))?;

                if next_parts.is_https != parts.is_https || next_parts.host_port != parts.host_port
                {
                    current_headers.retain(|(k, _)| {
                        !k.eq_ignore_ascii_case("authorization")
                            && !k.eq_ignore_ascii_case("cookie")
                            && !k.eq_ignore_ascii_case("proxy-authorization")
                    });
                }

                if resp.status == 303
                    || ((resp.status == 301 || resp.status == 302) && current_method == "POST")
                {
                    current_method = "GET".to_string();
                    current_body.clear();
                    current_headers.retain(|(k, _)| {
                        !k.eq_ignore_ascii_case("content-type")
                            && !k.eq_ignore_ascii_case("content-length")
                    });
                }

                current_url = next_url;
                redirected = true;
                continue;
            }
        }

        return serde_json::to_string(&WireFetchResponse::from_raw(resp, current_url, redirected))
            .map_err(|e| e.to_string());
    }

    Err("RedirectError: redirect loop terminated".to_string())
}
