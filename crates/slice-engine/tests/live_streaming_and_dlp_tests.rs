use crate::{
    HttpRequestContext, LiveHttpStreamGuard, LiveHttpStreamWriter, SliceIsolate,
    compile_source_to_bytecode,
};
use slice_sandbox::{CapabilityToken, SandboxViolation};
use std::sync::{Arc, Mutex};

struct RecordingStreamWriter {
    started: Arc<Mutex<Option<(u16, String)>>>,
    chunks: Arc<Mutex<Vec<Vec<u8>>>>,
    finished: Arc<Mutex<bool>>,
    disconnect_after_chunks: Option<usize>,
}

impl LiveHttpStreamWriter for RecordingStreamWriter {
    fn start_chunked(
        &mut self,
        status: u16,
        content_type: &str,
        _extra_headers: &[(String, String)],
    ) -> Result<(), String> {
        *self.started.lock().unwrap() = Some((status, content_type.to_string()));
        Ok(())
    }

    fn write_chunk(&mut self, data: &[u8]) -> Result<(), String> {
        let mut chunks = self.chunks.lock().unwrap();
        if let Some(limit) = self.disconnect_after_chunks
            && chunks.len() >= limit
        {
            return Err("CLIENT_DISCONNECTED".to_string());
        }
        chunks.push(data.to_vec());
        Ok(())
    }

    fn finish_chunked(&mut self) -> Result<(), String> {
        *self.finished.lock().unwrap() = true;
        Ok(())
    }

    fn is_client_disconnected(&self) -> bool {
        if let Some(limit) = self.disconnect_after_chunks {
            self.chunks.lock().unwrap().len() >= limit
        } else {
            false
        }
    }
}

#[test]
fn test_v0150_live_readable_stream_incremental_chunks_and_cross_chunk_dlp() {
    // 1. Incremental ReadableStream / SSE chunks flush directly to LiveHttpStreamWriter
    let sse_code = r#"
        export default {
            fetch(_req) {
                const stream = new ReadableStream({
                    start(controller) {
                        controller.enqueue("data: token-1\n\n");
                        controller.enqueue("data: token-2\n\n");
                        controller.close();
                    }
                });
                return new Response(stream, {
                    status: 200,
                    headers: { "content-type": "text/event-stream; charset=utf-8" }
                });
            }
        };
    "#;
    let bc = compile_source_to_bytecode(sse_code);
    let mut iso = SliceIsolate::spawn(
        "live-sse-tenant",
        1,
        CapabilityToken::pure_compute(),
        20_000_000,
        16 * 1024 * 1024,
    );

    let started = Arc::new(Mutex::new(None));
    let chunks = Arc::new(Mutex::new(Vec::new()));
    let finished = Arc::new(Mutex::new(false));
    let writer = RecordingStreamWriter {
        started: Arc::clone(&started),
        chunks: Arc::clone(&chunks),
        finished: Arc::clone(&finished),
        disconnect_after_chunks: None,
    };
    let req_ctx = HttpRequestContext {
        method: "GET",
        path: "/sse",
        query: "",
        tenant_id: "live-sse-tenant",
        body: "",
        body_bytes: None,
        headers: &[],
    };

    let guard = LiveHttpStreamGuard::enter(Box::new(writer));
    let outcome = iso.execute_http_request(&bc, None, &req_ctx).unwrap();
    assert!(guard.was_streamed());
    assert!(outcome.streamed_on_wire);
    drop(guard);

    assert_eq!(
        *started.lock().unwrap(),
        Some((200, "text/event-stream; charset=utf-8".to_string()))
    );
    let captured = chunks.lock().unwrap();
    assert_eq!(captured.len(), 2);
    assert_eq!(captured[0], b"data: token-1\n\n");
    assert_eq!(captured[1], b"data: token-2\n\n");
    assert!(*finished.lock().unwrap());

    // 2. Rolling cross-chunk Egress DLP catches secret split across two controller.enqueue() calls
    let dlp_code = r#"
        export default {
            fetch(_req) {
                const stream = new ReadableStream({
                    start(controller) {
                        controller.enqueue("data: part1 sk_live_stream_");
                        controller.enqueue("secret_98765\n\n");
                        controller.close();
                    }
                });
                return new Response(stream, {
                    status: 200,
                    headers: { "content-type": "text/event-stream" }
                });
            }
        };
    "#;
    let dlp_bc = compile_source_to_bytecode(dlp_code);
    let mut iso_dlp = SliceIsolate::spawn(
        "live-sse-dlp",
        2,
        CapabilityToken::pure_compute(),
        20_000_000,
        16 * 1024 * 1024,
    );
    iso_dlp.add_egress_audit_secret("sk_live_stream_secret_98765");

    let dlp_writer = RecordingStreamWriter {
        started: Arc::new(Mutex::new(None)),
        chunks: Arc::new(Mutex::new(Vec::new())),
        finished: Arc::new(Mutex::new(false)),
        disconnect_after_chunks: None,
    };
    let _dlp_guard = LiveHttpStreamGuard::enter(Box::new(dlp_writer));
    let err = iso_dlp
        .execute_http_request(&dlp_bc, None, &req_ctx)
        .unwrap_err();
    assert!(
        matches!(err, SandboxViolation::EgressSecretLeakBlocked { .. }),
        "expected EgressSecretLeakBlocked on split stream secret, got {err:?}"
    );
}
