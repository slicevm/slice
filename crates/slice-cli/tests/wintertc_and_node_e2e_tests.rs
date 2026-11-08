#![allow(unused_imports)]
use crate::config_and_watch::{
    build_cas_archive, build_cas_archive_with_static, merge_manifest_toml_into_caps,
};
use crate::http_server::{resolve_tenant_bytecode_hotswap, serve_connection_keep_alive};
use crate::mcp_and_repl::{
    handle_mcp_jsonrpc_line, measure_current_process_rss_kb, run_security_containment_checks,
};
use crate::test_runner::{cmd_test, discover_test_files};
use crate::{cmd_init, cmd_run};
use slice_cas::MappedCasArchive;
use slice_engine::{compile_source_to_bytecode, SliceIsolate};
use slice_sandbox::CapabilityToken;
use std::env;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;

#[test]
fn test_phase_n1_wintertc_redirect_keepalive_and_ssrf_mesh_roundtrip() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let bound_port = listener.local_addr().unwrap().port();
    let host_port = format!("127.0.0.1:{}", bound_port);

    let upstream_ts = r#"
export default {
    fetch(req: Request): Response {
        const url = new URL(req.url);
        if (url.pathname === "/upstream/subgraph/account") {
            const accountId = url.searchParams.get("account") || "acct-0";
            return Response.json({
                accountId,
                tier: "enterprise",
                balanceCents: 14500,
                region: "us-east-1"
            });
        }
        if (url.pathname === "/upstream/redirect-307") {
            return Response.redirect("/upstream/subgraph/account?" + url.searchParams.toString(), 307);
        }
        if (url.pathname === "/upstream/redirect-302") {
            const accountId = url.searchParams.get("account") || "acct-0";
            return Response.redirect("https://partner-gateway.example.com/upstream/subgraph/account?account=" + accountId, 302);
        }
        return Response.json({ ok: true });
    }
};
"#;
    let upstream_bundled = slice_cas::strip_typescript_and_normalize(upstream_ts, true);
    let upstream_bc = std::sync::Arc::new(compile_source_to_bytecode(&upstream_bundled));
    let bc_clone = std::sync::Arc::clone(&upstream_bc);

    // Serve ALL 20 requests over a SINGLE accepted Keep-Alive TCP stream!
    let srv_handle = std::thread::spawn(move || {
        SliceIsolate::prewarm_boa_bundle(&bc_clone);
        let mut read_buf = vec![0u8; 65536];
        let mut write_buf = Vec::with_capacity(4096);
        if let Ok((stream, _)) = listener.accept() {
            let _ = serve_connection_keep_alive(
                stream,
                1,
                &bc_clone,
                None,
                None,
                None,
                &CapabilityToken::pure_compute(),
                10_000_000,
                8 * 1024 * 1024,
                &mut read_buf,
                &mut write_buf,
                None,
            );
        }
    });

    // Copy App 17 (`tests/apps/17-zero-trust-redirect-keepalive-mesh`) into a temp dir bound to `bound_port`
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let app17_src = repo_root
        .join("tests")
        .join("apps")
        .join("17-zero-trust-redirect-keepalive-mesh")
        .join("src");
    let tmp_dir = std::env::temp_dir().join("slice_n1_app17_roundtrip");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let dst_src = tmp_dir.join("src");
    std::fs::create_dir_all(&dst_src).unwrap();

    for file_name in [
        "types.ts",
        "redirect_security_guard.ts",
        "keepalive_mesh_client.ts",
        "index.ts",
    ] {
        let content = std::fs::read_to_string(app17_src.join(file_name)).unwrap();
        let patched = content.replace("127.0.0.1:18095", &host_port);
        std::fs::write(dst_src.join(file_name), patched).unwrap();
    }
    std::fs::write(
        tmp_dir.join("package.json"),
        r#"{"name":"17-zero-trust-redirect-keepalive-mesh","version":"1.0.0","main":"./src/index.ts"}"#,
    )
    .unwrap();
    std::fs::write(
        tmp_dir.join("slice.toml"),
        format!(
            "[package]\nname = \"17-zero-trust-redirect-keepalive-mesh\"\nentrypoint = \"./src/index.ts\"\n[capabilities]\nallow_net = [\"{}\"]\n",
            host_port
        ),
    )
    .unwrap();

    // Populate node_modules in `tmp_dir` so CI works without external `bun install`
    let nm = tmp_dir.join("node_modules");
    for (pkg, code) in [
        (
            "bytes",
            "module.exports = { format: function(n) { return n < 1024 ? n + 'B' : (n / 1024).toFixed(2) + 'KB'; } };",
        ),
        (
            "cookie",
            "module.exports = { serialize: function(k, v) { return k + '=' + v + '; Path=/; HttpOnly'; }, parse: function(s) { var o = {}; String(s).split(';').forEach(function(p) { var kv = p.trim().split('='); if (kv.length === 2) o[kv[0]] = kv[1]; }); return o; } };",
        ),
        (
            "fast-querystring",
            "module.exports = { parse: function(q) { var o = {}; String(q).split('&').forEach(function(p) { var kv = p.split('='); if (kv[0]) o[decodeURIComponent(kv[0])] = decodeURIComponent(kv[1] || ''); }); return o; } };",
        ),
        (
            "ms",
            "module.exports = function(s) { var str = String(s); if (str.endsWith('ms')) return Number(str.slice(0, -2)); if (str.endsWith('s')) return Number(str.slice(0, -1)) * 1000; return Number(str); };",
        ),
    ] {
        let pkg_dir = nm.join(pkg);
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("index.js"), code).unwrap();
    }

    let cas_out = tmp_dir.join("app17.cas");
    build_cas_archive_with_static(&tmp_dir, &cas_out, "./src/index.ts", None).unwrap();

    let archive = MappedCasArchive::open(&cas_out).unwrap();
    let mut caps = CapabilityToken::pure_compute();
    if let Some(manifest_toml) = archive.get_manifest_toml() {
        merge_manifest_toml_into_caps(manifest_toml, &mut caps);
    }
    let entry = archive.entrypoint_blob().unwrap();
    slice_engine::clear_outbound_conn_pool();
    let reuse_before = slice_engine::outbound_pool_reuse_count();
    let mut iso = SliceIsolate::spawn("app17-tenant", 1, caps, 50_000_000, 16 * 1024 * 1024);
    let metrics = iso
        .execute_bytecode(archive.get_bytecode_slice(entry), Some(&archive))
        .unwrap();
    let reuse_after = slice_engine::outbound_pool_reuse_count();

    // Drop pooled connection so `srv_handle` sees EOF and exits cleanly
    slice_engine::clear_outbound_conn_pool();
    let _ = srv_handle.join();
    let _ = std::fs::remove_dir_all(&tmp_dir);

    assert_eq!(
        metrics.stdout[0],
        r#"{"app":"17-zero-trust-redirect-keepalive-mesh","libraries":["bytes@3.1.2","cookie@0.6.0","fast-querystring@1.1.2","ms@2.1.3"],"tasksProcessed":30,"keepAliveSubgraphHits":5,"redirectFollow307Completed":5,"redirectManual302Inspected":5,"ssrfRedirectMetadataBlocked":5,"dnsRebindPrivateIpBlocked":5,"dlpExfiltrationBlocked":5,"crossOriginAuthStrippedCount":15,"totalTimeoutBudgetMs":50000,"formattedPayloadVolume":"860B","meshAuditHmac":"90f5a8253a5d07de"}"#
    );
    assert!(
        reuse_after >= reuse_before + 19,
        "Expected at least 19 Keep-Alive socket reuses across 20 HTTP hops, got {}",
        reuse_after - reuse_before
    );
}

#[test]
fn test_phase_n2_wintertc_async_event_loop_fanout_and_time_travel_roundtrip() {
    // 1. Spawn a real WinterTC shard upstream server (`serve_connection_keep_alive`)
    //    that handles concurrent `POST /v1/shards/0..3` fan-out requests.
    let shard_gateway_ts = r#"
export default {
    fetch(req: Request): Response {
        const url = new URL(req.url);
        if (url.pathname.startsWith("/v1/shards/")) {
            const shardId = Number(url.pathname.split("/").pop() || "0");
            return Response.json(
                { shardId, records: (shardId + 1) * 25, status: "synced" },
                { status: 200, headers: { "x-shard-ack": "ack-" + shardId } }
            );
        }
        return new Response("Not Found", { status: 404 });
    }
};
"#;
    let gw_bundled = slice_cas::strip_typescript_and_normalize(shard_gateway_ts, true);
    let gw_bc: std::sync::Arc<[u8]> =
        std::sync::Arc::from(compile_source_to_bytecode(&gw_bundled).into_boxed_slice());

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let host_port = format!("127.0.0.1:{}", port);

    let stop_srv = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_clone = std::sync::Arc::clone(&stop_srv);
    let gw_bc_clone = std::sync::Arc::clone(&gw_bc);
    let srv_handle = std::thread::spawn(move || {
        let _ = listener.set_nonblocking(true);
        let mut conn_threads = Vec::new();
        let mut slot = 1u32;
        while !stop_clone.load(std::sync::atomic::Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    let bc = std::sync::Arc::clone(&gw_bc_clone);
                    let s = slot;
                    slot += 1;
                    conn_threads.push(std::thread::spawn(move || {
                        let mut read_buf = vec![0u8; 65536];
                        let mut write_buf = Vec::with_capacity(65536);
                        let _ = serve_connection_keep_alive(
                            stream,
                            s,
                            &bc,
                            None,
                            None,
                            None,
                            &CapabilityToken::pure_compute(),
                            15_000_000,
                            16 * 1024 * 1024,
                            &mut read_buf,
                            &mut write_buf,
                            None,
                        );
                    }));
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(_) => break,
            }
        }
        for ct in conn_threads {
            let _ = ct.join();
        }
    });

    // 2. Bundle App 18 (`18-async-event-loop-fanout-scheduler`) into a CAS archive
    let app18_src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/apps/18-async-event-loop-fanout-scheduler");
    let tmp_dir = std::env::temp_dir().join(format!("slice_n2_app18_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(tmp_dir.join("src")).unwrap();

    for file in [
        "types.ts",
        "timer_retry_scheduler.ts",
        "concurrent_fanout_client.ts",
        "index.ts",
    ] {
        let content = std::fs::read_to_string(app18_src.join("src").join(file)).unwrap();
        std::fs::write(tmp_dir.join("src").join(file), content).unwrap();
    }
    std::fs::copy(app18_src.join("package.json"), tmp_dir.join("package.json")).unwrap();
    std::fs::write(
        tmp_dir.join("slice.toml"),
        format!(
            "[package]\nname = \"18-async-event-loop-fanout-scheduler\"\nentrypoint = \"./src/index.ts\"\n[capabilities]\nallow_net = [\"api.fanout.slice.local\", \"{}\"]\nallow_env = [\"NODE_ENV\", \"SLICE_FANOUT_UPSTREAM_BASE\"]\n",
            host_port
        ),
    )
    .unwrap();

    let nm = tmp_dir.join("node_modules");
    for (pkg, code) in [
        (
            "bytes",
            "module.exports = { parse: function(s) { var str = String(s).toUpperCase(); if (str.endsWith('KB')) return Number(str.slice(0, -2)) * 1024; return Number(str); }, format: function(n) { return n < 1024 ? n + 'B' : (n / 1024).toFixed(2) + 'KB'; } };",
        ),
        (
            "flatted",
            "module.exports = { stringify: function(v) { return JSON.stringify([v]); }, parse: function(s) { return JSON.parse(s)[0]; } };",
        ),
        (
            "mitt",
            "module.exports = function() { var m = {}; return { on: function(t, f) { (m[t] = m[t] || []).push(f); }, emit: function(t, e) { (m[t] || []).forEach(function(f) { f(e); }); } }; };",
        ),
        (
            "ms",
            "module.exports = function(s) { var str = String(s); if (str.endsWith('ms')) return Number(str.slice(0, -2)); if (str.endsWith('s')) return Number(str.slice(0, -1)) * 1000; return Number(str); };",
        ),
    ] {
        let pkg_dir = nm.join(pkg);
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("index.js"), code).unwrap();
    }

    let cas_out = tmp_dir.join("app18.cas");
    build_cas_archive_with_static(&tmp_dir, &cas_out, "./src/index.ts", None).unwrap();

    let archive = MappedCasArchive::open(&cas_out).unwrap();
    let entry = archive.entrypoint_blob().unwrap();

    // 3A. Run in Deterministic Virtual Time-Warp mode (`mockFetch` + 0ns timer warp)
    let mut caps_det = CapabilityToken::pure_compute();
    if let Some(manifest_toml) = archive.get_manifest_toml() {
        merge_manifest_toml_into_caps(manifest_toml, &mut caps_det);
    }
    slice_engine::reset_virtual_time_warped_ms();
    let mut iso_det = SliceIsolate::spawn(
        "app18-det-tenant",
        1,
        caps_det,
        50_000_000,
        16 * 1024 * 1024,
    );
    iso_det.set_deterministic_seed(Some(20261102));
    let metrics_det = iso_det
        .execute_bytecode(archive.get_bytecode_slice(entry), Some(&archive))
        .unwrap();
    let warped_ms = slice_engine::virtual_time_warped_ms();
    assert!(
        warped_ms >= 10,
        "Expected deterministic virtual time warp >= 10ms, got {}ms",
        warped_ms
    );

    let expected_output = r#"{"app":"18-async-event-loop-fanout-scheduler","libraries":["bytes@3.1.2","flatted@3.3.1","mitt@3.0.1","ms@2.1.3"],"scheduleOrder":"promise-micro-1>queue-micro-1>timer-stage-a:p1>micro-inside-timer>heartbeat:1>heartbeat:2>heartbeat:3","intervalTicks":3,"cancelledTimerFired":false,"totalRetryBackoffMs":50,"totalShardsExecuted":25,"totalRecordsProcessed":1250,"totalAbortedTasks":5,"totalRetryAttempts":30,"busEventsDispatched":5,"timerBombContained":true,"formattedWireVolume":"1.92KB","auditDigest":"0b76cd366dc934f2"}"#;
    assert_eq!(metrics_det.stdout[0], expected_output);

    // 3B. Run in Live Concurrent Socket Fan-Out mode against the 4-worker WinterTC server
    let reuse_before = slice_engine::outbound_pool_reuse_count();
    let mut caps_live = CapabilityToken::pure_compute().with_env(
        "SLICE_FANOUT_UPSTREAM_BASE",
        format!("http://{}", host_port),
    );
    if let Some(manifest_toml) = archive.get_manifest_toml() {
        merge_manifest_toml_into_caps(manifest_toml, &mut caps_live);
    }
    let mut iso_live = SliceIsolate::spawn(
        "app18-live-tenant",
        2,
        caps_live,
        50_000_000,
        16 * 1024 * 1024,
    );
    let metrics_live = iso_live
        .execute_bytecode(archive.get_bytecode_slice(entry), Some(&archive))
        .unwrap();
    let reuse_after = slice_engine::outbound_pool_reuse_count();

    assert_eq!(metrics_live.stdout[0], expected_output);
    assert!(
        reuse_after >= reuse_before + 15,
        "Expected >= 15 Keep-Alive socket reuses across 5 rounds of 4-shard concurrent fan-out, got {}",
        reuse_after - reuse_before
    );

    stop_srv.store(true, std::sync::atomic::Ordering::Relaxed);
    slice_engine::clear_outbound_conn_pool();
    let _ = srv_handle.join();
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_phase_n3_wintertc_webcrypto_streams_cloning_and_http_roundtrip() {
    let tmp_dir = std::env::temp_dir().join("slice_phase_n3_wintertc_test");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let src_dir = tmp_dir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();

    let manifest_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/apps/19-wintertc-webcrypto-streams-edge");
    std::fs::copy(
        manifest_src.join("package.json"),
        tmp_dir.join("package.json"),
    )
    .unwrap();
    std::fs::copy(manifest_src.join("slice.toml"), tmp_dir.join("slice.toml")).unwrap();
    for f in [
        "types.ts",
        "webcrypto_jwt_verifier.ts",
        "streams_and_formdata_pipeline.ts",
        "index.ts",
    ] {
        std::fs::copy(manifest_src.join("src").join(f), src_dir.join(f)).unwrap();
    }

    // Stub minimal CJS modules for cookie, ms, bytes, and flatted in node_modules
    let nm = tmp_dir.join("node_modules");
    std::fs::create_dir_all(nm.join("ms")).unwrap();
    std::fs::write(
        nm.join("ms/package.json"),
        r#"{"name":"ms","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        nm.join("ms/index.js"),
        r#"module.exports = function(v, o) { return __slice_fast_ms(v, Boolean(o && o.long)); };"#,
    )
    .unwrap();

    std::fs::create_dir_all(nm.join("bytes")).unwrap();
    std::fs::write(
        nm.join("bytes/package.json"),
        r#"{"name":"bytes","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        nm.join("bytes/index.js"),
        r#"module.exports = { parse: function(v){ return __slice_fast_bytes_parse(v); }, format: function(v){ return __slice_fast_bytes_format(v); } };"#,
    )
    .unwrap();

    std::fs::create_dir_all(nm.join("cookie")).unwrap();
    std::fs::write(
        nm.join("cookie/package.json"),
        r#"{"name":"cookie","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        nm.join("cookie/index.js"),
        r#"exports.parse = function(str) {
  var out = {};
  var parts = String(str || "").split(";");
  for (var i = 0; i < parts.length; i++) {
    var p = parts[i].trim();
    var eq = p.indexOf("=");
    if (eq !== -1) out[p.slice(0, eq).trim()] = p.slice(eq + 1).trim();
  }
  return out;
};"#,
    )
    .unwrap();

    std::fs::create_dir_all(nm.join("flatted")).unwrap();
    std::fs::write(
        nm.join("flatted/package.json"),
        r#"{"name":"flatted","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        nm.join("flatted/index.js"),
        r#"exports.stringify = function(obj) {
  var copy = Object.assign({}, obj);
  delete copy.root;
  return JSON.stringify(copy);
};
exports.parse = function(str) {
  var obj = JSON.parse(str);
  obj.root = obj;
  return obj;
};"#,
    )
    .unwrap();

    let cas_path = tmp_dir.join("app19.cas");
    build_cas_archive_with_static(&tmp_dir, &cas_path, "./src/index.ts", None).unwrap();
    let archive = MappedCasArchive::open(&cas_path).unwrap();
    let entry = archive.entrypoint_blob().unwrap();

    let mut caps = CapabilityToken::pure_compute();
    if let Some(manifest_toml) = archive.get_manifest_toml() {
        merge_manifest_toml_into_caps(manifest_toml, &mut caps);
    }
    let mut iso = SliceIsolate::spawn("app19-tenant", 1, caps, 50_000_000, 16 * 1024 * 1024);
    let metrics = iso
        .execute_bytecode(archive.get_bytecode_slice(entry), Some(&archive))
        .unwrap();

    let expected_output = r#"{"app":"19-wintertc-webcrypto-streams-edge","libraries":["bytes@3.1.2","cookie@1.0.2","flatted@3.3.1","ms@2.1.3"],"hs256TokenValid":true,"hs256TamperedRejected":true,"hs512WebhookValid":true,"jwkAlg":"HS256","encodeIntoRead":8,"encodeIntoWritten":10,"transformedPayload":"ROUTE:API  |AUTH:JWT  |STREAM:OK::EOF","totalTransformedBytes":37,"formFieldsCount":3,"uploadedFileName":"bundle-manifest.bin","uploadedFileSize":38,"slicedHeaderText":"SLICE-EDGE","cyclicCloneVerified":true,"dispatchedEvents":["once:init","live:init","live:transform"],"anySignalAbortedBy":"secondary-deadline-exceeded","formattedWireVolume":"75B","auditDigest":"7319225b3dc10146"}"#;
    assert_eq!(metrics.stdout[0], expected_output);

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_phase_n4_buffer_node_core_and_ast_bundler_roundtrip() {
    let tmp_dir = std::env::temp_dir().join("slice_phase_n4_compat_test");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let src_dir = tmp_dir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();

    let manifest_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/apps/20-node-compat-and-edge-router");
    std::fs::copy(
        manifest_src.join("package.json"),
        tmp_dir.join("package.json"),
    )
    .unwrap();
    std::fs::copy(manifest_src.join("slice.toml"), tmp_dir.join("slice.toml")).unwrap();
    for f in [
        "types.ts",
        "binary_wire_codec.ts",
        "router_and_events_bus.ts",
        "index.ts",
    ] {
        std::fs::copy(manifest_src.join("src").join(f), src_dir.join(f)).unwrap();
    }

    // Stub minimal CJS modules for cookie, ms, bytes, and semver in node_modules
    let nm = tmp_dir.join("node_modules");
    std::fs::create_dir_all(nm.join("ms")).unwrap();
    std::fs::write(
        nm.join("ms/package.json"),
        r#"{"name":"ms","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        nm.join("ms/index.js"),
        r#"module.exports = function(v, o) { return __slice_fast_ms(v, Boolean(o && o.long)); };"#,
    )
    .unwrap();

    std::fs::create_dir_all(nm.join("bytes")).unwrap();
    std::fs::write(
        nm.join("bytes/package.json"),
        r#"{"name":"bytes","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        nm.join("bytes/index.js"),
        r#"var fn = function(v){ return typeof v === "number" ? __slice_fast_bytes_format(v) : __slice_fast_bytes_parse(v); };
fn.parse = function(v){ return __slice_fast_bytes_parse(v); };
fn.format = function(v){ return __slice_fast_bytes_format(v); };
module.exports = fn;"#,
    )
    .unwrap();

    std::fs::create_dir_all(nm.join("cookie")).unwrap();
    std::fs::write(
        nm.join("cookie/package.json"),
        r#"{"name":"cookie","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        nm.join("cookie/index.js"),
        r#"exports.parse = function(str) {
  var out = {};
  var parts = String(str || "").split(";");
  for (var i = 0; i < parts.length; i++) {
    var p = parts[i].trim();
    var eq = p.indexOf("=");
    if (eq !== -1) out[p.slice(0, eq).trim()] = p.slice(eq + 1).trim();
  }
  return out;
};"#,
    )
    .unwrap();

    std::fs::create_dir_all(nm.join("semver")).unwrap();
    std::fs::write(
        nm.join("semver/package.json"),
        r#"{"name":"semver","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        nm.join("semver/index.js"),
        r#"module.exports = { satisfies: function(v, r){ return __slice_semver_satisfies(String(v), String(r)); } };"#,
    )
    .unwrap();

    let cas_path = tmp_dir.join("app20.cas");
    build_cas_archive_with_static(&tmp_dir, &cas_path, "./src/index.ts", None).unwrap();
    let archive = MappedCasArchive::open(&cas_path).unwrap();
    let entry = archive.entrypoint_blob().unwrap();

    let mut caps = CapabilityToken::pure_compute();
    if let Some(manifest_toml) = archive.get_manifest_toml() {
        merge_manifest_toml_into_caps(manifest_toml, &mut caps);
    }
    let mut iso = SliceIsolate::spawn("app20-tenant", 1, caps, 50_000_000, 16 * 1024 * 1024);
    let metrics = iso
        .execute_bytecode(archive.get_bytecode_slice(entry), Some(&archive))
        .unwrap();

    let expected_output = r#"{"suite":"20-node-compat-and-edge-router","wireFrame":{"magicHex":"534c4334","version":4,"flags":258,"sequenceId":"9007199254740993","payloadUtf8":"{\"rpc\":\"edge.dispatch\",\"region\":\"iad-01\",\"currency\":\"€\"}","payloadBase64Url":"eyJycGMiOiJlZGdlLmRpc3BhdGNoIiwicmVnaW9uIjoiaWFkLTAxIiwiY3VycmVuY3kiOiLigqwifQ","sha512DigestPrefix":"06c23b3b4132558f8dd7b430","hmacSha256Hex":"fda684d490d2bd86c7023653983dab877506c69edf9bb254675a7305e9d91c6c","isValidBufferInstance":true},"routeSummary":{"normalizedRoute":"/api/v2/ tenants/dispatch/invoke","relativeAssetPath":"edge/assets/bundles/client.v4.mjs","parsedBase":"client.v4.mjs","sessionTenant":"acme-fintech-eu","ttlMs":900000,"maxBodyBytes":65536,"engineSemverSatisfied":true,"formattedAuditLine":"tenant=acme-fintech-eu route=/api/v2/ tenants/dispatch/invoke status=200 asset=client.v4.mjs","eventSequence":["prepend:/api/v2/ tenants/dispatch/invoke","on:/api/v2/ tenants/dispatch/invoke","auth-once:acme-fintech-eu","done:200"]}}"#;
    assert_eq!(metrics.stdout[0], expected_output);

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_wave_a_cli_tsx_ssr_and_test_runner() {
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    let tmp_dir = env::temp_dir().join("slice_wave_a_tsx_cli_test");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(tmp_dir.join("src")).unwrap();
    std::fs::create_dir_all(tmp_dir.join("tests")).unwrap();

    // 1. Write a TSX component (`src/Card.tsx`) and TSX server entrypoint (`src/index.tsx`)
    std::fs::write(
        tmp_dir.join("src/Card.tsx"),
        r#"
export interface CardProps {
    title: string;
    items: string[];
}

export function Card(props: CardProps) {
    return (
        <section className="edge-card" data-count={props.items.length}>
            <h2>{props.title}</h2>
            <ul>
                {props.items.map((item) => (
                    <li key={item}>{item}</li>
                ))}
            </ul>
        </section>
    );
}
"#,
    )
    .unwrap();

    std::fs::write(
        tmp_dir.join("src/index.tsx"),
        r#"
import { renderToString } from "slice:jsx";
import { Card } from "./Card";

export default {
    fetch(req: Request): Response {
        const url = new URL(req.url);
        const title = url.searchParams.get("title") ?? "Default <Title>";
        const html = renderToString(
            <Card title={title} items={["Oxc TSX", "Slice SSR & <Safe>"]} />
        );
        return new Response("<!doctype html>" + html, {
            status: 200,
            headers: { "content-type": "text/html; charset=utf-8" }
        });
    }
};
"#,
    )
    .unwrap();

    // 2. Write a `.test.tsx` test file verifying JSX SSR, `setImmediate`/`nextTick`, and `console.table`
    std::fs::write(
        tmp_dir.join("tests/ssr.test.tsx"),
        r#"
import { describe, test, expect } from "slice:test";
import { renderToString } from "slice:jsx";
import { Card } from "../src/Card";

describe("TSX SSR & Event Loop Suite", () => {
    test("renders TSX component with XSS escaping", () => {
        const html = renderToString(<Card title="Hello <XSS>" items={["A", "B"]} />);
        expect(html).toContain('<section class="edge-card" data-count="2">');
        expect(html).toContain("<h2>Hello &lt;XSS&gt;</h2>");
        expect(html).toContain("<li>A</li><li>B</li>");
    });

    test("console.table and setImmediate execute cleanly", () => {
        setImmediate(() => {});
        console.table([{ feature: "TSX", ok: true }]);
        expect(typeof setImmediate).toBe("function");
    });
});
"#,
    )
    .unwrap();

    // 3. Verify `discover_test_files` and `cmd_test` execute `tests/ssr.test.tsx`
    let discovered = discover_test_files(&tmp_dir).unwrap();
    assert_eq!(discovered.len(), 1);
    assert!(discovered[0].to_string_lossy().ends_with("ssr.test.tsx"));
    let test_args = vec![tmp_dir.to_string_lossy().to_string()];
    cmd_test(&test_args).unwrap();

    // 4. Verify `.tsx` HTTP Server SSR over TCP
    let bundled = slice_cas::bundle_entrypoint_with_deps(&tmp_dir.join("src/index.tsx")).unwrap();
    let bytecode = Arc::new(compile_source_to_bytecode(&bundled));

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let counter = Arc::new(AtomicUsize::new(0));
    let bc_srv = Arc::clone(&bytecode);
    let counter_srv = Arc::clone(&counter);

    let srv = std::thread::spawn(move || {
        SliceIsolate::prewarm_boa_pool(1);
        SliceIsolate::prewarm_boa_bundle(&bc_srv);
        let mut read_buf = vec![0u8; 65536];
        let mut write_buf = Vec::with_capacity(4096);
        if let Ok((stream, _)) = listener.accept() {
            let _ = serve_connection_keep_alive(
                stream,
                0,
                &bc_srv,
                None,
                None,
                None,
                &CapabilityToken::pure_compute(),
                2_000_000,
                4 * 1024 * 1024,
                &mut read_buf,
                &mut write_buf,
                Some((&counter_srv, 0)),
            );
        }
    });

    let mut client = TcpStream::connect(addr).unwrap();
    client
        .write_all(
            b"GET /?title=Wave%20A%20%3CRust%3E HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .unwrap();
    let mut resp = String::new();
    client.read_to_string(&mut resp).unwrap();
    srv.join().unwrap();
    let _ = std::fs::remove_dir_all(&tmp_dir);

    assert!(resp.contains("HTTP/1.1 200 OK"));
    assert!(resp.contains("Content-Type: text/html; charset=utf-8"));
    assert!(resp.contains(
        "<!doctype html><section class=\"edge-card\" data-count=\"2\"><h2>Wave A &lt;Rust&gt;</h2><ul><li>Oxc TSX</li><li>Slice SSR &amp; &lt;Safe&gt;</li></ul></section>"
    ));
}
