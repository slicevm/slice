//! WinterTC HTTP Request Execution (`execute_http_request`), Post-Response `Slice.waitUntil`
//! Task Draining (`drain_wait_until_tasks`), and Server WebSocket Frame Dispatch (`execute_websocket_event`).

use crate::boa_realm::{
    acquire_warm_boa_realm_for_tenant, call_realm_ctl, ensure_realm_shards, BOA_HTTP_REQ,
    BOA_HTTP_RES, BOA_WAIT_UNTIL_COUNT, BOA_WS_UPGRADED, REALM_CTL_SET_DET_MODE,
    REALM_CTL_SYNC_ENV,
};
use crate::isolate::{ExecutionMetrics, SliceIsolate};
use slice_cas::MappedCasArchive;
use slice_sandbox::SandboxViolation;
use std::borrow::Cow;

/// Zero-copy HTTP/1.1 request view borrowed directly from the worker thread's 64 KB socket read buffer.
#[derive(Debug, Clone, Copy)]
pub struct HttpRequestContext<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub query: &'a str,
    pub tenant_id: &'a str,
    pub body: &'a str,
    pub headers: &'a [(&'a str, &'a str)],
}

/// Structured outcome of executing a WinterTC HTTP request inside a `SliceIsolate`.
#[derive(Debug, Clone)]
pub struct HttpExecutionOutcome {
    pub metrics: ExecutionMetrics,
    pub status_code: u16,
    pub content_type: Cow<'static, str>,
    pub response_body: String,
    pub response_body_bytes: Option<Vec<u8>>,
    pub response_headers: Vec<(String, String)>,
    pub wait_until_count: usize,
    pub ws_upgraded: bool,
}

impl HttpExecutionOutcome {
    /// Returns the raw wire response bytes (exact binary bytes when present, otherwise UTF-8 `response_body`).
    #[inline]
    pub fn body_bytes(&self) -> &[u8] {
        self.response_body_bytes
            .as_deref()
            .unwrap_or(self.response_body.as_bytes())
    }
}

/// Zero-clone HTTP outcome used on the hot `slice serve` worker path (`execute_http_request_fast`).
#[derive(Debug, Clone)]
pub struct HttpFastOutcome {
    pub total_time_ns: u64,
    pub fuel_consumed: u64,
    pub status_code: u16,
    pub content_type: Cow<'static, str>,
    pub response_body: String,
    pub response_body_bytes: Option<Vec<u8>>,
    pub response_headers: Vec<(String, String)>,
    pub wait_until_count: usize,
    pub ws_upgraded: bool,
}

impl HttpFastOutcome {
    /// Returns the raw wire response bytes without allocating or coercing binary payloads through UTF-8.
    #[inline]
    pub fn body_bytes(&self) -> &[u8] {
        self.response_body_bytes
            .as_deref()
            .unwrap_or(self.response_body.as_bytes())
    }
}

#[inline]
fn populate_boa_http_req(req: &HttpRequestContext<'_>) {
    BOA_HTTP_REQ.with(|r| {
        let mut borrow = r.borrow_mut();
        borrow.active = true;
        borrow.method.clear();
        borrow.method.push_str(req.method);
        borrow.path.clear();
        borrow.path.push_str(req.path);
        borrow.query.clear();
        borrow.query.push_str(req.query);
        borrow.tenant_id.clear();
        borrow.tenant_id.push_str(req.tenant_id);
        borrow.body.clear();
        borrow.body.push_str(req.body);
        borrow.headers_len = req.headers.len();
        for (idx, &(k, v)) in req.headers.iter().enumerate() {
            if idx < borrow.headers.len() {
                borrow.headers[idx].0.clear();
                borrow.headers[idx].0.push_str(k);
                borrow.headers[idx].1.clear();
                borrow.headers[idx].1.push_str(v);
            } else {
                borrow.headers.push((k.to_string(), v.to_string()));
            }
        }
    });
    BOA_HTTP_RES.with(|res| *res.borrow_mut() = None);
    BOA_WS_UPGRADED.with(|w| w.set(false));
    BOA_WAIT_UNTIL_COUNT.with(|c| c.set(0));
}

impl SliceIsolate {
    /// Executes an HTTP request inside this `SliceIsolate` with zero-copy request metadata
    /// (`HttpRequestContext`), custom status/content-type response capture (`Slice.respond`),
    /// and deferred `Slice.waitUntil(fn)` post-response callback support.
    pub fn execute_http_request(
        &mut self,
        bytecode: &[u8],
        cas: Option<&MappedCasArchive>,
        req: &HttpRequestContext<'_>,
    ) -> Result<HttpExecutionOutcome, SandboxViolation> {
        if bytecode.len() >= 8
            && (&bytecode[0..8] == b"SLICEBOA" || &bytecode[0..8] == crate::boacas::BOACAS_MAGIC)
        {
            populate_boa_http_req(req);
            let metrics_res = self.execute_bytecode(bytecode, cas);
            BOA_HTTP_REQ.with(|r| r.borrow_mut().active = false);
            let mut metrics = metrics_res?;

            let custom_res = BOA_HTTP_RES.with(|res| res.borrow_mut().take());
            let ws_upgraded = BOA_WS_UPGRADED.with(|w| w.replace(false));
            let wait_until_count = BOA_WAIT_UNTIL_COUNT.with(|c| c.get());

            let (status_code, content_type, response_body, response_body_bytes, response_headers) =
                if let Some((st, mut body, body_bytes, ct, hdrs)) = custom_res {
                    if body.is_empty() {
                        if let Some(ref raw) = body_bytes {
                            body = String::from_utf8_lossy(raw).into_owned();
                        }
                    }
                    (st, Cow::Owned(ct), body, body_bytes, hdrs)
                } else {
                    let body = if !metrics.stdout.is_empty() {
                        metrics.stdout.join("\n")
                    } else {
                        std::mem::take(&mut metrics.return_value)
                    };
                    (
                        200,
                        Cow::Borrowed("application/json"),
                        body,
                        None,
                        Vec::new(),
                    )
                };

            return Ok(HttpExecutionOutcome {
                metrics,
                status_code,
                content_type,
                response_body,
                response_body_bytes,
                response_headers,
                wait_until_count,
                ws_upgraded,
            });
        }

        let mut metrics = self.execute_bytecode(bytecode, cas)?;
        let body = if !metrics.stdout.is_empty() {
            metrics.stdout.join("\n")
        } else {
            std::mem::take(&mut metrics.return_value)
        };
        Ok(HttpExecutionOutcome {
            metrics,
            status_code: 200,
            content_type: Cow::Borrowed("application/json"),
            response_body: body,
            response_body_bytes: None,
            response_headers: Vec::new(),
            wait_until_count: 0,
            ws_upgraded: false,
        })
    }

    /// Zero-clone HTTP request dispatcher for `slice serve` worker loops.
    /// Avoids cloning `tenant_id`, `stdout`, `"application/json"`, and `"undefined"` on every request.
    pub fn execute_http_request_fast(
        &mut self,
        bytecode: &[u8],
        cas: Option<&MappedCasArchive>,
        req: &HttpRequestContext<'_>,
    ) -> Result<HttpFastOutcome, SandboxViolation> {
        if bytecode.len() >= 8
            && (&bytecode[0..8] == b"SLICEBOA" || &bytecode[0..8] == crate::boacas::BOACAS_MAGIC)
        {
            populate_boa_http_req(req);
            let core_res = self.execute_bytecode_core(bytecode, cas, false);
            BOA_HTTP_REQ.with(|r| r.borrow_mut().active = false);
            let (_instantiate_ns, total_time_ns, ret_str) = core_res?;

            let custom_res = BOA_HTTP_RES.with(|res| res.borrow_mut().take());
            let ws_upgraded = BOA_WS_UPGRADED.with(|w| w.replace(false));
            let wait_until_count = BOA_WAIT_UNTIL_COUNT.with(|c| c.get());

            let (status_code, content_type, response_body, response_body_bytes, response_headers) =
                if let Some((st, body, body_bytes, ct, hdrs)) = custom_res {
                    (st, Cow::Owned(ct), body, body_bytes, hdrs)
                } else {
                    let body = if !self.stdout.is_empty() {
                        self.stdout.join("\n")
                    } else {
                        ret_str
                    };
                    (
                        200,
                        Cow::Borrowed("application/json"),
                        body,
                        None,
                        Vec::new(),
                    )
                };

            return Ok(HttpFastOutcome {
                total_time_ns,
                fuel_consumed: self.compartment.fuel_consumed,
                status_code,
                content_type,
                response_body,
                response_body_bytes,
                response_headers,
                wait_until_count,
                ws_upgraded,
            });
        }

        let (_instantiate_ns, total_time_ns, ret_str) =
            self.execute_bytecode_core(bytecode, cas, false)?;
        let body = if !self.stdout.is_empty() {
            self.stdout.join("\n")
        } else {
            ret_str
        };
        Ok(HttpFastOutcome {
            total_time_ns,
            fuel_consumed: self.compartment.fuel_consumed,
            status_code: 200,
            content_type: Cow::Borrowed("application/json"),
            response_body: body,
            response_body_bytes: None,
            response_headers: Vec::new(),
            wait_until_count: 0,
            ws_upgraded: false,
        })
    }

    /// Executes any deferred `Slice.waitUntil(fn)` callbacks queued during `execute_http_request`
    /// after the HTTP response bytes have already been flushed to the client socket, strictly
    /// bounded by the tenant's remaining CPU fuel, memory ceiling, SES lockdown, Egress DLP, and OCap permissions.
    pub fn drain_wait_until_tasks(&mut self) -> Result<usize, SandboxViolation> {
        let count = BOA_WAIT_UNTIL_COUNT.with(|c| c.replace(0));
        if count == 0 {
            return Ok(0);
        }
        self.compartment.consume_fuel((count as u64) * 5)?;

        let flags = self.setup_boa_compartment_thread_locals();
        let mut realm = acquire_warm_boa_realm_for_tenant(&self.compartment.tenant_id);
        realm.has_pending_wu = false;

        if flags.has_env_keys {
            call_realm_ctl(&mut realm, REALM_CTL_SYNC_ENV, true);
        }
        if self.deterministic_seed.is_some() {
            crate::host_bindings::register_deterministic_host_functions(&mut realm.ctx);
            call_realm_ctl(&mut realm, REALM_CTL_SET_DET_MODE, true);
        }

        self.apply_boa_realm_runtime_limits(&mut realm);
        let prev_stdout_len = self.stdout.len();
        let eval_res = if let Some(ref drain_fn) = realm.drain_wu_fn {
            drain_fn.call(&boa_engine::JsValue::undefined(), &[], &mut realm.ctx)
        } else {
            realm.ctx.eval(boa_engine::Source::from_bytes(
                b"if(typeof globalThis.__slice_drain_wu===\"function\")globalThis.__slice_drain_wu();",
            ))
        };

        let check_ses = self.last_exec_check_ses;
        self.finish_boa_compartment_execution(realm, flags, false, false, check_ses, eval_res)?;

        if !self.egress_audit_secrets.is_empty() {
            for line in &self.stdout[prev_stdout_len..] {
                self.verify_egress_not_leaked(line)?;
            }
        }
        Ok(count)
    }

    /// Dispatches a server-side WebSocket event (`"open"`, `"message"`, or `"close"`) into the
    /// isolate's active `Slice.onWebSocket` / `Slice.upgradeWebSocket` handler and returns any
    /// outgoing server-to-client WebSocket frames (`ServerWsOutFrame`), enforcing full OCap,
    /// CPU fuel, memory ceiling, SES lockdown, and Egress DLP policies.
    pub fn execute_websocket_event(
        &mut self,
        event_type: &str,
        is_binary: bool,
        payload_bytes: &[u8],
        code: u16,
        reason: &str,
    ) -> Result<Vec<crate::websocket::ServerWsOutFrame>, SandboxViolation> {
        self.compartment.consume_fuel(15)?;
        let payload_arg = if is_binary {
            crate::intrinsics::base64_encode(payload_bytes, false)
        } else {
            String::from_utf8_lossy(payload_bytes).into_owned()
        };

        let flags = self.setup_boa_compartment_thread_locals();
        let mut realm = acquire_warm_boa_realm_for_tenant(&self.compartment.tenant_id);
        realm.has_pending_wu = false;
        ensure_realm_shards(&mut realm, (1 << 13) | (1 << 7), false);

        if flags.has_env_keys {
            call_realm_ctl(&mut realm, REALM_CTL_SYNC_ENV, true);
        }
        if self.deterministic_seed.is_some() {
            crate::host_bindings::register_deterministic_host_functions(&mut realm.ctx);
            call_realm_ctl(&mut realm, REALM_CTL_SET_DET_MODE, true);
        }

        self.apply_boa_realm_runtime_limits(&mut realm);

        let prev_stdout_len = self.stdout.len();
        let dispatch_fn = realm
            .ctx
            .global_object()
            .get(
                boa_engine::js_string!("__slice_dispatch_server_ws"),
                &mut realm.ctx,
            )
            .ok()
            .and_then(|v| v.as_object());
        let eval_res = if let Some(fn_obj) = dispatch_fn {
            fn_obj.call(
                &boa_engine::JsValue::undefined(),
                &[
                    boa_engine::JsValue::from(boa_engine::js_string!(event_type)),
                    boa_engine::JsValue::from(is_binary),
                    boa_engine::JsValue::from(boa_engine::js_string!(payload_arg)),
                    boa_engine::JsValue::from(i32::from(code)),
                    boa_engine::JsValue::from(boa_engine::js_string!(reason)),
                ],
                &mut realm.ctx,
            )
        } else {
            Ok(boa_engine::JsValue::undefined())
        };

        // Drive microtasks/timers before draining the server WebSocket outbox (since
        // `finish_boa_compartment_execution` clears thread-local WebSocket state on teardown).
        crate::event_loop::drive_event_loop_to_completion(&mut realm.ctx);
        let frames = crate::websocket::drain_server_ws_outbox();

        self.finish_boa_compartment_execution(realm, flags, false, false, true, eval_res)?;

        if !self.egress_audit_secrets.is_empty() {
            for line in &self.stdout[prev_stdout_len..] {
                self.verify_egress_not_leaked(line)?;
            }
            for frame in &frames {
                match frame {
                    crate::websocket::ServerWsOutFrame::Text(txt) => {
                        self.verify_egress_not_leaked(txt)?;
                    }
                    crate::websocket::ServerWsOutFrame::Binary(bytes) => {
                        let s = String::from_utf8_lossy(bytes);
                        self.verify_egress_not_leaked(&s)?;
                    }
                    crate::websocket::ServerWsOutFrame::Close(_, r) => {
                        self.verify_egress_not_leaked(r)?;
                    }
                }
            }
        }

        Ok(frames)
    }
}
