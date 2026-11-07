//! Inbound HTTP Request/Response Bridge (`__slice_http_*`) and Outbound `fetch` (`Shard 10`) Host Bindings.

use crate::bindings::helpers::active_tenant_id_or_default;
use crate::boa_realm::{
    BOA_ACTIVE_TENANT, BOA_DET_STATE, BOA_EGRESS_SECRETS, BOA_FETCH_MOCKS, BOA_HTTP_REQ,
    BOA_HTTP_RES, BOA_VIOLATION,
};
use crate::event_loop::enqueue_async_fetch;
use crate::outbound_http::{
    check_boa_outbound_fetch_ocap_and_dlp, execute_outbound_fetch_with_redirects,
    parse_outbound_http_url,
};
use crate::state_and_kv::{edge_kv_get, edge_kv_set, verify_egress_secrets_slice};

pub(crate) fn register_http_bridge_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsNativeError, JsValue as BoaVal, NativeFunction};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_http_method"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_headers_all"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let json = BOA_HTTP_REQ.with(|r| {
                let borrow = r.borrow();
                serde_json::to_string(&borrow.headers[..borrow.headers_len])
                    .unwrap_or_else(|_| "[]".to_string())
            });
            Ok(BoaVal::from(js_string!(json)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_method"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let s = BOA_HTTP_REQ.with(|r| r.borrow().method.clone());
            Ok(BoaVal::from(js_string!(s)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_path"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let s = BOA_HTTP_REQ.with(|r| r.borrow().path.clone());
            Ok(BoaVal::from(js_string!(s)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_query"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let s = BOA_HTTP_REQ.with(|r| r.borrow().query.clone());
            Ok(BoaVal::from(js_string!(s)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_tenant"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let s = BOA_HTTP_REQ.with(|r| r.borrow().tenant_id.clone());
            Ok(BoaVal::from(js_string!(s)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_body"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            let s = BOA_HTTP_REQ.with(|r| r.borrow().body.clone());
            Ok(BoaVal::from(js_string!(s)))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_header"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let key = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let found = BOA_HTTP_REQ.with(|r| {
                let borrow = r.borrow();
                borrow.headers[..borrow.headers_len]
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(&key))
                    .map(|(_, v)| v.clone())
            });
            match found {
                Some(v) => Ok(BoaVal::from(js_string!(v))),
                None => Ok(BoaVal::null()),
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_http_respond"),
        4,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let status = args
                .first()
                .and_then(|v| v.as_number())
                .map(|n| n as u16)
                .unwrap_or(200);
            let body = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let content_type = args
                .get(2)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "application/json".to_string());
            let headers_json = args
                .get(3)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();

            // Egress DLP inspection across response body, Content-Type, and serialized response headers.
            let egress_secrets = BOA_EGRESS_SECRETS.with(|s| s.borrow().clone());
            if !egress_secrets.is_empty() {
                let tenant_id = active_tenant_id_or_default();
                if let Err(violation) = verify_egress_secrets_slice(&tenant_id, &egress_secrets, &body)
                    .and_then(|()| {
                        verify_egress_secrets_slice(&tenant_id, &egress_secrets, &content_type)
                    })
                    .and_then(|()| {
                        verify_egress_secrets_slice(&tenant_id, &egress_secrets, &headers_json)
                    })
                {
                    let msg = violation.to_string();
                    BOA_VIOLATION.with(|v| {
                        if v.borrow().is_none() {
                            *v.borrow_mut() = Some(violation);
                        }
                    });
                    return Err(JsNativeError::error().with_message(msg).into());
                }
            }

            let extra_headers: Vec<(String, String)> = if headers_json.is_empty() {
                Vec::new()
            } else {
                serde_json::from_str(&headers_json).unwrap_or_default()
            };
            BOA_HTTP_RES.with(|res| {
                *res.borrow_mut() = Some((status, body, content_type, extra_headers));
            });
            Ok(BoaVal::undefined())
        }),
    );
}

pub(crate) fn register_shard10_outbound_fetch_host_functions(ctx: &mut boa_engine::Context) {
    use boa_engine::{js_string, JsValue as BoaVal, NativeFunction};
    if ctx
        .global_object()
        .has_own_property(js_string!("__slice_fetch_try_fast"), ctx)
        .unwrap_or(false)
    {
        return;
    }

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_mock_fetch"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let key = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let payload = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            if !key.is_empty() && !payload.is_empty() {
                BOA_FETCH_MOCKS.with(|m| {
                    m.borrow_mut().insert(key, payload);
                });
            }
            Ok(BoaVal::undefined())
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fetch_sync"),
        5,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let url = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let method = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped().to_ascii_uppercase())
                .unwrap_or_else(|| "GET".to_string());
            let headers_json = args
                .get(2)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "[]".to_string());
            let body = args
                .get(3)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let redirect_mode = args
                .get(4)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "follow".to_string());

            let Some(parts) = parse_outbound_http_url(&url) else {
                let err_json =
                    serde_json::json!({ "__error": format!("Invalid outbound URL: {}", url) })
                        .to_string();
                return Ok(BoaVal::from(js_string!(err_json)));
            };

            if let Err(err_msg) =
                check_boa_outbound_fetch_ocap_and_dlp(&parts, &url, &headers_json, &body)
            {
                let err_json = serde_json::json!({ "__error": err_msg }).to_string();
                return Ok(BoaVal::from(js_string!(err_json)));
            }

            let method_url_key = format!("{} {}", method, url);
            if let Some(mock_json) = BOA_FETCH_MOCKS.with(|m| {
                let map = m.borrow();
                map.get(&method_url_key).or_else(|| map.get(&url)).cloned()
            }) {
                return Ok(BoaVal::from(js_string!(mock_json)));
            }

            let cassette_key = format!("fetch_cassette:{}:{}", method, url);
            let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
            if let Some(cassette_json) = edge_kv_get(&tenant_id, &cassette_key) {
                return Ok(BoaVal::from(js_string!(cassette_json)));
            }

            let req_headers: Vec<(String, String)> =
                serde_json::from_str(&headers_json).unwrap_or_default();
            match execute_outbound_fetch_with_redirects(
                &url,
                &method,
                &req_headers,
                &body,
                &redirect_mode,
            ) {
                Ok(resp_json) => {
                    if BOA_DET_STATE.with(|s| s.get().is_some()) {
                        edge_kv_set(&tenant_id, &cassette_key, &resp_json, 0);
                    }
                    Ok(BoaVal::from(js_string!(resp_json)))
                }
                Err(err_msg) => {
                    let err_json = serde_json::json!({ "__error": err_msg }).to_string();
                    Ok(BoaVal::from(js_string!(err_json)))
                }
            }
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fetch_try_fast"),
        5,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let url = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let method = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped().to_ascii_uppercase())
                .unwrap_or_else(|| "GET".to_string());
            let headers_json = args
                .get(2)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "[]".to_string());
            let body = args
                .get(3)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let _redirect_mode = args
                .get(4)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "follow".to_string());

            let Some(parts) = parse_outbound_http_url(&url) else {
                let err_json =
                    serde_json::json!({ "__error": format!("Invalid outbound URL: {}", url) })
                        .to_string();
                return Ok(BoaVal::from(js_string!(err_json)));
            };

            if let Err(err_msg) =
                check_boa_outbound_fetch_ocap_and_dlp(&parts, &url, &headers_json, &body)
            {
                let err_json = serde_json::json!({ "__error": err_msg }).to_string();
                return Ok(BoaVal::from(js_string!(err_json)));
            }

            let method_url_key = format!("{} {}", method, url);
            if let Some(mock_json) = BOA_FETCH_MOCKS.with(|m| {
                let map = m.borrow();
                map.get(&method_url_key).or_else(|| map.get(&url)).cloned()
            }) {
                return Ok(BoaVal::from(js_string!(mock_json)));
            }

            let cassette_key = format!("fetch_cassette:{}:{}", method, url);
            let tenant_id = BOA_ACTIVE_TENANT.with(|t| t.borrow().clone());
            if let Some(cassette_json) = edge_kv_get(&tenant_id, &cassette_key) {
                return Ok(BoaVal::from(js_string!(cassette_json)));
            }

            Ok(BoaVal::from(js_string!("")))
        }),
    );

    let _ = ctx.register_global_builtin_callable(
        js_string!("__slice_fetch_enqueue"),
        8,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let url = args
                .first()
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let method = args
                .get(1)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped().to_ascii_uppercase())
                .unwrap_or_else(|| "GET".to_string());
            let headers_json = args
                .get(2)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "[]".to_string());
            let body = args
                .get(3)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            let redirect_mode = args
                .get(4)
                .map(|v| v.to_string(context))
                .transpose()?
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|| "follow".to_string());
            let Some(resolve_fn) = args.get(5).and_then(|v| v.as_object()) else {
                return Ok(BoaVal::from(0));
            };
            let Some(reject_fn) = args.get(6).and_then(|v| v.as_object()) else {
                return Ok(BoaVal::from(0));
            };
            let signal_obj = args.get(7).and_then(|v| v.as_object());

            let req_headers: Vec<(String, String)> =
                serde_json::from_str(&headers_json).unwrap_or_default();
            let id = enqueue_async_fetch(
                url,
                method,
                req_headers,
                body,
                redirect_mode,
                resolve_fn.clone(),
                reject_fn.clone(),
                signal_obj,
            );
            Ok(BoaVal::from(id as f64))
        }),
    );
}
