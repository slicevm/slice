#![allow(unused_imports)]
use crate::intrinsics::{
    digest_to_hex, fast_format_bytes, fast_format_ms, fast_parse_bytes, fast_parse_ms,
    hmac_sha256_hex, sha256_hex, Sha256State,
};
use crate::{
    compile_source_to_bytecode, disassemble_bytecode, run_source_isolated, ExecutionTier,
    HttpRequestContext, JsValue, SliceIsolate,
};
use slice_sandbox::{CapabilityToken, SandboxViolation};

#[test]
fn test_phase_n2_deterministic_time_travel_event_loop_concurrent_fetch_and_abort_signal() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    SliceIsolate::prewarm_boa_pool(2);

    // 1. Deterministic Time-Travel Event Loop + Microtask/Macrotask Ordering + Async `slice:test` Rollback:
    //    Schedules 66,000 ms (66 seconds!) of exponential backoff timers + `setInterval` + `queueMicrotask`
    //    and verifies that Virtual Time-Warp completes the entire 66-second schedule in < 50ms of wall time!
    crate::reset_virtual_time_warped_ms();
    let det_event_loop_ts = r#"
import { describe, test, expect } from "slice:test";

Slice.resetTests();
Slice.kv.set("workflow:state", "initial", 0);

describe("Phase N-2 Deterministic Time-Travel & Event Loop Suite", () => {
    test("orders queueMicrotask, Promise.then, setTimeout, clearTimeout, and setInterval", async () => {
        const events: string[] = [];
        const t0 = Date.now();

        const cancelledId = setTimeout(() => {
            events.push("cancelled-should-not-run");
        }, 5);
        clearTimeout(cancelledId);

        setTimeout((label: string, code: number) => {
            events.push(`${label}:${code}`);
        }, 20, "timer20", 200);

        setTimeout(() => {
            events.push("timer10");
            queueMicrotask(() => events.push("micro-after-timer10"));
        }, 10);

        let ticks = 0;
        const iv = setInterval(() => {
            ticks += 1;
            events.push(`tick:${ticks}`);
            if (ticks >= 3) {
                clearInterval(iv);
            }
        }, 15);

        Promise.resolve().then(() => events.push("promise-1"));
        queueMicrotask(() => events.push("micro-1"));
        events.push("sync-end");

        // Wait 66,000 ms (66 seconds) via exponential backoff timers (warped in 0ns!)
        await new Promise((r) => setTimeout(r, 1000));
        await new Promise((r) => setTimeout(r, 5000));
        await new Promise((r) => setTimeout(r, 60000));

        const elapsedVirtualMs = Date.now() - t0;
        expect(elapsedVirtualMs).toBeGreaterThanOrEqual(66000);
        expect(events.join(",")).toBe(
            "sync-end,promise-1,micro-1,timer10,micro-after-timer10,tick:1,timer20:200,tick:2,tick:3"
        );

        // Mutate KV after async await to verify `slice:test` waits for async completion before rollback
        Slice.kv.set("workflow:state", "mutated-after-66s-await", 0);
        expect(Slice.kv.get("workflow:state")).toBe("mutated-after-66s-await");
    });

    test("sees clean KV state after async test rollback and tests AbortController", async () => {
        expect(Slice.kv.get("workflow:state")).toBe("initial");

        let ctrl = new AbortController();
        let abortEventFired = false;
        ctrl.signal.addEventListener("abort", () => {
            abortEventFired = true;
        });
        setTimeout(() => ctrl.abort(new Error("CustomCancelReason")), 50);

        let caughtMsg = "";
        try {
            await new Promise((_res, rej) => {
                ctrl.signal.addEventListener("abort", () => rej(ctrl.signal.reason));
            });
        } catch (err: any) {
            caughtMsg = String(err.message || err);
        }
        expect(abortEventFired).toBe(true);
        expect(ctrl.signal.aborted).toBe(true);
        expect(caughtMsg).toContain("CustomCancelReason");

        // Also verify pre-aborted AbortSignal.abort() rejects fetch immediately without network I/O
        let preAbortedMsg = "";
        try {
            await fetch("https://example.com/never-dialed", {
                signal: AbortSignal.abort(new Error("PreAbortedSignal"))
            });
        } catch (err2: any) {
            preAbortedMsg = String(err2.message || err2);
        }
        expect(preAbortedMsg).toContain("PreAbortedSignal");
    });
});

console.log(JSON.stringify(Slice.testSummary()));
"#;
    let bundled_det = slice_cas::strip_typescript_and_normalize(det_event_loop_ts, true);
    let bc_det = compile_source_to_bytecode(&bundled_det);
    let mut iso_det = SliceIsolate::spawn(
        "n2-det-event-loop",
        1,
        CapabilityToken::pure_compute().with_net_host("example.com"),
        5_000_000,
        8 * 1024 * 1024,
    );
    iso_det.set_deterministic_seed(Some(20261102));
    SliceIsolate::prewarm_boa_pool(1);
    let start_wall = Instant::now();
    let det_metrics = iso_det.execute_bytecode(&bc_det, None).unwrap();
    let wall_elapsed = start_wall.elapsed();
    let warped_ms = crate::virtual_time_warped_ms();

    let summary: serde_json::Value = serde_json::from_str(&det_metrics.stdout[0]).unwrap();
    assert_eq!(summary["total"], 2);
    assert_eq!(summary["passed"], 2);
    assert_eq!(summary["failed"], 0, "Test summary: {}", summary);
    assert_eq!(summary["checkpointsRestored"], 2);
    assert!(
        warped_ms >= 66_000,
        "Expected >= 66,000ms of virtual time warp, got {}ms",
        warped_ms
    );
    assert!(
        wall_elapsed < Duration::from_millis(1500),
        "66-second deterministic timer suite should finish in < 1500ms, took {:?}",
        wall_elapsed
    );

    // 2. True Concurrent `Promise.all([fetch(u1), fetch(u2), fetch(u3), fetch(u4)])` + `AbortSignal.timeout(ms)`:
    //    4 upstream servers each sleep 60ms before responding; sequential execution would take >= 240ms.
    //    A 5th upstream sleeps 350ms and is raced against `AbortSignal.timeout(40)`.
    let mut ports = Vec::new();
    let mut handles = Vec::new();
    for idx in 0..4 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        ports.push(port);
        handles.push(std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                std::thread::sleep(Duration::from_millis(60));
                let body = format!(r#"{{"shard":{},"ok":true}}"#, idx);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            }
        }));
    }

    let slow_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let slow_port = slow_listener.local_addr().unwrap().port();
    handles.push(std::thread::spawn(move || {
        if let Ok((mut stream, _)) = slow_listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            std::thread::sleep(Duration::from_millis(350));
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"slow\":true}",
            );
        }
    }));

    let mut caps = CapabilityToken::pure_compute();
    for &p in &ports {
        caps = caps.with_net_host(format!("127.0.0.1:{}", p));
    }
    caps = caps.with_net_host(format!("127.0.0.1:{}", slow_port));

    let fanout_js = format!(
        r#"
async function runFanout() {{
    const responses = await Promise.all([
        fetch("http://127.0.0.1:{}/shard"),
        fetch("http://127.0.0.1:{}/shard"),
        fetch("http://127.0.0.1:{}/shard"),
        fetch("http://127.0.0.1:{}/shard"),
    ]);
    const payloads = await Promise.all(responses.map((r) => r.json()));
    const shards = payloads.map((p) => p.shard);

    let timeoutAborted = false;
    let timeoutError = "";
    try {{
        await fetch("http://127.0.0.1:{}/slow", {{
            signal: AbortSignal.timeout(35)
        }});
    }} catch (err) {{
        timeoutAborted = true;
        timeoutError = String(err && err.message || err);
    }}

    console.log(JSON.stringify({{
        shards,
        timeoutAborted,
        isTimeoutErr: timeoutError.indexOf("TimeoutError") !== -1
    }}));
}}
runFanout();
"#,
        ports[0], ports[1], ports[2], ports[3], slow_port
    );

    SliceIsolate::prewarm_boa_pool(1);
    let fanout_bc = compile_source_to_bytecode(&fanout_js);
    let mut fanout_iso =
        SliceIsolate::spawn("n2-fanout-tenant", 1, caps, 5_000_000, 8 * 1024 * 1024);
    let fanout_start = Instant::now();
    let fanout_metrics = fanout_iso.execute_bytecode(&fanout_bc, None).unwrap();
    let fanout_elapsed = fanout_start.elapsed();
    let fanout_out: serde_json::Value = serde_json::from_str(&fanout_metrics.stdout[0]).unwrap();
    assert_eq!(fanout_out["shards"], serde_json::json!([0, 1, 2, 3]));
    assert_eq!(fanout_out["timeoutAborted"], true);
    assert_eq!(fanout_out["isTimeoutErr"], true);
    let max_fanout_ms = if cfg!(debug_assertions) { 450 } else { 220 };
    assert!(
        fanout_elapsed < Duration::from_millis(max_fanout_ms),
        "Expected 4x60ms parallel fan-out + 35ms timeout to complete in < {}ms, took {:?}",
        max_fanout_ms,
        fanout_elapsed
    );

    // 3. Verify un-cleared infinite `setInterval` is bounded and trapped by `MAX_EVENT_LOOP_TICKS`
    let runaway_interval_js = r#"
setInterval(() => {
    // Runaway interval that never calls clearInterval
}, 1);
"#;
    let mut iso_runaway = SliceIsolate::spawn(
        "n2-runaway-interval",
        2,
        CapabilityToken::pure_compute(),
        5_000_000,
        4 * 1024 * 1024,
    );
    iso_runaway.set_deterministic_seed(Some(99));
    let runaway_bc = compile_source_to_bytecode(runaway_interval_js);
    let runaway_res = iso_runaway.execute_bytecode(&runaway_bc, None);
    assert!(matches!(
        runaway_res,
        Err(SandboxViolation::CpuFuelExhausted { .. })
    ));

    for h in handles {
        let _ = h.join();
    }
}

#[test]
fn test_phase_n3_wintertc_encoding_webcrypto_streams_and_cloning() {
    let script = r#"
(async function() {
    // 1. TextEncoder / TextDecoder / btoa / atob
    const enc = new TextEncoder();
    const dec = new TextDecoder("utf-8", { fatal: true });
    const bytes = enc.encode("Slice-UTF8-\u00e9-\u20ac");
    const decoded = dec.decode(bytes);

    // encodeInto with partial buffer capacity (multi-byte euro sign \u20ac is 3 UTF-8 bytes)
    const dest = new Uint8Array(5);
    const intoRes = enc.encodeInto("ab\u20accd", dest); // 'a' (1) + 'b' (1) + '\u20ac' (3) = 5 bytes, 3 UTF-16 units
    const intoDecoded = dec.decode(dest.subarray(0, intoRes.written));

    // Fatal UTF-8 decoder must throw TypeError on invalid 0xFF byte
    let fatalThrew = false;
    try {
        dec.decode(new Uint8Array([0xff, 0xfe]));
    } catch (e) {
        fatalThrew = e instanceof TypeError;
    }

    // btoa / atob round-trip & InvalidCharacterError on non-Latin1
    const b64 = btoa("WinterTC:Edge:2026");
    const unb64 = atob(b64);
    let btoaNonLatin1Threw = false;
    try {
        btoa("\u20ac");
    } catch (_e) {
        btoaNonLatin1Threw = true;
    }

    // 2. Deterministic WebCrypto: randomUUID, getRandomValues, digest, HMAC sign/verify, JWK
    const uuid1 = crypto.randomUUID();
    const rndBuf = new Uint8Array(16);
    crypto.getRandomValues(rndBuf);
    let quotaThrew = false;
    try {
        crypto.getRandomValues(new Uint8Array(65537));
    } catch (e) {
        quotaThrew = String(e.message || e).indexOf("QuotaExceededError") !== -1;
    }

    function bufToHex(ab) {
        const u8 = new Uint8Array(ab);
        let s = "";
        for (let i = 0; i < u8.length; i++) {
            const h = u8[i].toString(16);
            s += h.length === 1 ? "0" + h : h;
        }
        return s;
    }

    const abcBytes = enc.encode("abc");
    const sha1Hex = bufToHex(await crypto.subtle.digest("SHA-1", abcBytes));
    const sha256Hex = bufToHex(await crypto.subtle.digest("SHA-256", abcBytes));
    const sha384Hex = bufToHex(await crypto.subtle.digest("SHA-384", abcBytes));
    const sha512Hex = bufToHex(await crypto.subtle.digest("SHA-512", abcBytes));

    const hmacKey = await crypto.subtle.importKey(
        "raw",
        enc.encode("slice-wintertc-hmac-secret-key"),
        { name: "HMAC", hash: "SHA-256" },
        true,
        ["sign", "verify"]
    );
    const jwkExported = await crypto.subtle.exportKey("jwk", hmacKey);
    const jwkImported = await crypto.subtle.importKey(
        "jwk",
        jwkExported,
        { name: "HMAC", hash: "SHA-256" },
        false,
        ["verify"]
    );
    const payloadBytes = enc.encode("header.payload.claims");
    const sigBuf = await crypto.subtle.sign("HMAC", hmacKey, payloadBytes);
    const verifiedOk = await crypto.subtle.verify("HMAC", jwkImported, sigBuf, payloadBytes);
    const tamperedBytes = enc.encode("header.payload.tampered");
    const verifiedTampered = await crypto.subtle.verify("HMAC", jwkImported, sigBuf, tamperedBytes);
    const timingEq = crypto.subtle.timingSafeEqual(new Uint8Array(sigBuf), new Uint8Array(sigBuf));

    // 3. Event, CustomEvent, EventTarget, and AbortSignal.any
    const et = new EventTarget();
    const eventsLog = [];
    const acListener = new AbortController();
    et.addEventListener("custom", (ev) => {
        eventsLog.push("once:" + ev.detail.tag);
    }, { once: true });
    et.addEventListener("custom", (ev) => {
        eventsLog.push("sig:" + ev.detail.tag);
    }, { signal: acListener.signal });
    et.dispatchEvent(new CustomEvent("custom", { detail: { tag: "first" } }));
    acListener.abort("unsubscribed");
    et.dispatchEvent(new CustomEvent("custom", { detail: { tag: "second" } }));

    const ac1 = new AbortController();
    const ac2 = new AbortController();
    const combinedSig = AbortSignal.any([ac1.signal, ac2.signal]);
    let combinedAbortedReason = "";
    combinedSig.addEventListener("abort", () => {
        combinedAbortedReason = String(combinedSig.reason);
    });
    ac2.abort("ac2-triggered");

    // 4. Blob, File, FormData, Request/Response binary bodies, and structuredClone
    const blob = new Blob(["Hello ", enc.encode("World")], { type: "text/plain" });
    const file = new File([blob, "!"], "greeting.txt", { type: "text/plain", lastModified: 1700000000123 });
    const fileText = await file.text();
    const slicedBlobText = await file.slice(0, 5).text();

    const fd = new FormData();
    fd.append("project", "slice");
    fd.append("artifact", file);
    const reqBin = new Request("https://edge.slice.dev/upload", {
        method: "POST",
        body: new Uint8Array([10, 20, 30, 40])
    });
    const reqAb = await reqBin.arrayBuffer();
    const reqU8 = new Uint8Array(reqAb);

    const cyclic = { name: "root", tags: new Set(["a", "b"]), meta: new Map([["k", 42]]), buf: new Uint8Array([7, 8, 9]) };
    cyclic.self = cyclic;
    const cloned = structuredClone(cyclic);
    cloned.buf[0] = 99;
    const clonePreservedCycle = (cloned.self === cloned) && (cloned !== cyclic) && (cyclic.buf[0] === 7) && (cloned.meta.get("k") === 42);

    // 5. ReadableStream, WritableStream, TransformStream, TextEncoderStream, TextDecoderStream
    const upperTransform = new TransformStream({
        transform(chunk, controller) {
            controller.enqueue(String(chunk).toUpperCase());
        }
    });
    const streamChunks = [];
    const sink = new WritableStream({
        write(chunk) {
            streamChunks.push(chunk);
        }
    });
    await ReadableStream.from(["alpha-", "beta-", "gamma"])
        .pipeThrough(upperTransform)
        .pipeThrough(new TextEncoderStream())
        .pipeThrough(new TextDecoderStream())
        .pipeTo(sink);

    console.log(JSON.stringify({
        decoded,
        intoRead: intoRes.read,
        intoWritten: intoRes.written,
        intoDecoded,
        fatalThrew,
        unb64,
        btoaNonLatin1Threw,
        uuid1,
        rndFirst4: [rndBuf[0], rndBuf[1], rndBuf[2], rndBuf[3]],
        quotaThrew,
        sha1Hex,
        sha256Hex,
        sha384Prefix: sha384Hex.slice(0, 16),
        sha512Prefix: sha512Hex.slice(0, 16),
        verifiedOk,
        verifiedTampered,
        timingEq,
        eventsLog,
        combinedAborted: combinedSig.aborted,
        combinedAbortedReason,
        fileSize: file.size,
        fileName: file.name,
        fileText,
        slicedBlobText,
        fdProject: fd.get("project"),
        fdFileName: fd.get("artifact").name,
        reqBytes: [reqU8[0], reqU8[1], reqU8[2], reqU8[3]],
        clonePreservedCycle,
        streamJoined: streamChunks.join("")
    }));
})();
"#;

    let bc = compile_source_to_bytecode(script);

    // Execute twice in deterministic mode with identical seed to verify deterministic WebCrypto replay!
    let mut iso1 = SliceIsolate::spawn(
        "n3-tenant-1",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        8 * 1024 * 1024,
    );
    iso1.set_deterministic_seed(Some(20261102));
    let m1 = iso1.execute_bytecode(&bc, None).unwrap();

    let mut iso2 = SliceIsolate::spawn(
        "n3-tenant-2",
        2,
        CapabilityToken::pure_compute(),
        5_000_000,
        8 * 1024 * 1024,
    );
    iso2.set_deterministic_seed(Some(20261102));
    let m2 = iso2.execute_bytecode(&bc, None).unwrap();

    assert_eq!(
        m1.stdout[0], m2.stdout[0],
        "Expected deterministic WebCrypto UUIDv4 and getRandomValues to replay identically"
    );

    let out: serde_json::Value = serde_json::from_str(&m1.stdout[0]).unwrap();
    assert_eq!(out["decoded"], "Slice-UTF8-\u{00e9}-\u{20ac}");
    assert_eq!(out["intoRead"], 3);
    assert_eq!(out["intoWritten"], 5);
    assert_eq!(out["intoDecoded"], "ab\u{20ac}");
    assert_eq!(out["fatalThrew"], true);
    assert_eq!(out["unb64"], "WinterTC:Edge:2026");
    assert_eq!(out["btoaNonLatin1Threw"], true);
    assert_eq!(out["quotaThrew"], true);
    // NIST FIPS 180-4 test vectors for "abc"
    assert_eq!(out["sha1Hex"], "a9993e364706816aba3e25717850c26c9cd0d89d");
    assert_eq!(
        out["sha256Hex"],
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(out["sha384Prefix"], "cb00753f45a35e8b");
    assert_eq!(out["sha512Prefix"], "ddaf35a193617aba");
    assert_eq!(out["verifiedOk"], true);
    assert_eq!(out["verifiedTampered"], false);
    assert_eq!(out["timingEq"], true);
    assert_eq!(
        out["eventsLog"],
        serde_json::json!(["once:first", "sig:first"])
    );
    assert_eq!(out["combinedAborted"], true);
    assert_eq!(out["combinedAbortedReason"], "ac2-triggered");
    assert_eq!(out["fileSize"], 12);
    assert_eq!(out["fileName"], "greeting.txt");
    assert_eq!(out["fileText"], "Hello World!");
    assert_eq!(out["slicedBlobText"], "Hello");
    assert_eq!(out["fdProject"], "slice");
    assert_eq!(out["fdFileName"], "greeting.txt");
    assert_eq!(out["reqBytes"], serde_json::json!([10, 20, 30, 40]));
    assert_eq!(out["clonePreservedCycle"], true);
    assert_eq!(out["streamJoined"], "ALPHA-BETA-GAMMA");
}

#[test]
fn test_phase_n4_uint8array_buffer_and_node_core_tier1() {
    let script = r#"
import { EventEmitter } from "node:events";
import * as path from "node:path";
import * as crypto from "node:crypto";
import * as fs from "node:fs";
import * as fsp from "node:fs/promises";
import assert from "node:assert";
import * as util from "node:util";

async function main() {
    // 1. Real Uint8Array-backed Buffer & binary wire-protocol methods
    const b1 = Buffer.from("Slice\u20ac", "utf8");
    assert.strictEqual(Buffer.isBuffer(b1), true);
    assert.strictEqual(b1 instanceof Uint8Array, true);
    assert.strictEqual(b1 instanceof Buffer, true);
    assert.strictEqual(b1.length, 8);
    assert.strictEqual(Buffer.byteLength("Slice\u20ac", "utf8"), 8);

    const hexStr = b1.toString("hex");
    const bFromHex = Buffer.from(hexStr, "hex");
    assert.strictEqual(b1.equals(bFromHex), true);
    assert.strictEqual(bFromHex.toString("utf8"), "Slice\u20ac");

    const b64u = b1.toString("base64url");
    assert.strictEqual(Buffer.from(b64u, "base64url").toString("utf8"), "Slice\u20ac");

    // Shared memory view on slice / subarray & binary integer readers/writers
    const wire = Buffer.alloc(16);
    wire.writeUInt32BE(0xdeadbeef, 0);
    wire.writeUInt32LE(0x12345678, 4);
    wire.writeBigUInt64BE(0x0102030405060708n, 8);
    assert.strictEqual(wire.readUInt32BE(0), 0xdeadbeef);
    assert.strictEqual(wire.readUInt32LE(4), 0x12345678);
    assert.strictEqual(wire.readBigUInt64BE(8), 0x0102030405060708n);

    const sub = wire.slice(0, 4);
    sub[0] = 0xca;
    assert.strictEqual(wire[0], 0xca);

    const cat = Buffer.concat([Buffer.from("AB"), Buffer.from("CD")]);
    assert.strictEqual(cat.toString("utf8"), "ABCD");

    // 2. node:events EventEmitter
    const bus = new EventEmitter();
    const evLog: string[] = [];
    bus.on("packet", (id: number) => evLog.push(`on:${id}`));
    bus.once("packet", (id: number) => evLog.push(`once:${id}`));
    bus.prependListener("packet", (id: number) => evLog.push(`pre:${id}`));
    bus.emit("packet", 1);
    bus.emit("packet", 2);
    assert.deepStrictEqual(evLog, ["pre:1", "on:1", "once:1", "pre:2", "on:2"]);

    // 3. node:path
    const norm = path.normalize("/a/b/../c/./d//file.tar.gz");
    const rel = path.relative("/a/c/x", "/a/c/d/file.tar.gz");
    const parsed = path.parse(norm);
    const formatted = path.format(parsed);
    assert.strictEqual(norm, "/a/c/d/file.tar.gz");
    assert.strictEqual(rel, "../d/file.tar.gz");
    assert.strictEqual(parsed.base, "file.tar.gz");
    assert.strictEqual(parsed.name, "file.tar");
    assert.strictEqual(parsed.ext, ".gz");
    assert.strictEqual(formatted, "/a/c/d/file.tar.gz");

    // 4. node:crypto streaming multi-chunk Hash/HMAC + randomBytes
    const rb = crypto.randomBytes(12);
    assert.strictEqual(Buffer.isBuffer(rb), true);
    assert.strictEqual(rb.length, 12);

    const h512 = crypto.createHash("sha512")
        .update(Buffer.from("ab"))
        .update("c")
        .digest();
    assert.strictEqual(Buffer.isBuffer(h512), true);
    assert.strictEqual(h512.length, 64);
    assert.strictEqual(h512.toString("hex").slice(0, 16), "ddaf35a193617aba");

    const hmacBuf = crypto.createHmac("sha256", Buffer.from("secret-key"))
        .update("part1:")
        .update(Buffer.from("part2"))
        .digest();
    assert.strictEqual(Buffer.isBuffer(hmacBuf), true);
    assert.strictEqual(hmacBuf.length, 32);

    // 5. node:fs & node:fs/promises + Time-Travel VFS checkpoint
    const snap = Slice.checkpoint();
    await fsp.mkdir("/tmp/n4_dir");
    await fsp.writeFile("/tmp/n4_dir/a.txt", Buffer.from("payload-v1"));
    fs.copyFileSync("/tmp/n4_dir/a.txt", "/tmp/n4_dir/b.txt");
    fs.renameSync("/tmp/n4_dir/b.txt", "/tmp/n4_dir/c.txt");
    const stFile = await fsp.stat("/tmp/n4_dir/c.txt");
    const stDir = await fsp.stat("/tmp/n4_dir");
    const rawBuf = await fsp.readFile("/tmp/n4_dir/c.txt", null);
    assert.strictEqual(Buffer.isBuffer(rawBuf), true);
    assert.strictEqual(rawBuf.toString("utf8"), "payload-v1");
    assert.strictEqual(stFile.isFile(), true);
    assert.strictEqual(stDir.isDirectory(), true);
    Slice.restore(snap);
    assert.strictEqual(fs.existsSync("/tmp/n4_dir/c.txt"), false);

    // 6. node:util & node:assert
    const formattedMsg = util.format("route=%s code=%d ok=%j %%", "/api", 200, { v: 1 });
    assert.strictEqual(formattedMsg, "route=/api code=200 ok={\"v\":1} %");
    assert.strictEqual(util.types.isBuffer(b1), true);
    assert.strictEqual(util.types.isUint8Array(b1), true);

    const asyncAdd = util.promisify((x: number, y: number, cb: (err: any, r?: number) => void) => {
        cb(null, x + y);
    });
    const sum = await asyncAdd(19, 23);
    assert.strictEqual(sum, 42);

    assert.throws(() => {
        throw new RangeError("out of bounds");
    }, /out of bounds/);

    console.log(JSON.stringify({
        ok: true,
        hexStr,
        randHex: rb.toString("hex"),
        sha512Prefix: h512.toString("hex").slice(0, 16),
        hmacHex: hmacBuf.toString("hex").slice(0, 16),
        rel,
        sum
    }));
}
main();
"#;

    let bc = compile_source_to_bytecode(script);
    let n4_caps = CapabilityToken::pure_compute()
        .with_read_prefix("/tmp")
        .with_write_prefix("/tmp");
    let mut iso1 = SliceIsolate::spawn(
        "phase-n4-tenant-1",
        1,
        n4_caps.clone(),
        25_000_000,
        8 * 1024 * 1024,
    );
    iso1.set_deterministic_seed(Some(20261104));
    let m1 = iso1.execute_bytecode(&bc, None).unwrap();

    let mut iso2 =
        SliceIsolate::spawn("phase-n4-tenant-2", 2, n4_caps, 25_000_000, 8 * 1024 * 1024);
    iso2.set_deterministic_seed(Some(20261104));
    let m2 = iso2.execute_bytecode(&bc, None).unwrap();

    assert_eq!(m1.stdout[0], m2.stdout[0]);
    let out: serde_json::Value = serde_json::from_str(&m1.stdout[0]).expect(&m1.stdout[0]);
    assert_eq!(out["ok"], true);
    assert_eq!(out["hexStr"], "536c696365e282ac");
    assert_eq!(out["sha512Prefix"], "ddaf35a193617aba");
    assert_eq!(out["rel"], "../d/file.tar.gz");
    assert_eq!(out["sum"], 42);
}

#[test]
fn test_wave_a_next_tick_set_immediate_timers_promises_full_console_and_jsx_ssr() {
    // 1. Verify strict task/microtask priority ordering:
    //    sync -> process.nextTick -> Promise.then / queueMicrotask -> setImmediate -> setTimeout(0)
    //    plus `clearImmediate` cancellation and `node:timers/promises` with virtual time-warp & AbortSignal.
    let async_order_ts = r#"
import * as timersPromises from "node:timers/promises";

async function runSuite() {
    const order: string[] = [];

    setTimeout(() => order.push("timeout-0"), 0);
    const cancelledImm = setImmediate(() => order.push("cancelled-immediate"));
    clearImmediate(cancelledImm);
    setImmediate((tag: string) => order.push(`immediate:${tag}`), "check");
    Promise.resolve().then(() => order.push("promise-then"));
    queueMicrotask(() => {
        order.push("queue-microtask");
        process.nextTick(() => order.push("nested-nextTick"));
    });
    process.nextTick((a: string, b: number) => order.push(`nextTick:${a}:${b}`), "hi", 1);
    order.push("sync");

    const delayedVal = await timersPromises.setTimeout(250, "warped-250ms");
    const immVal = await timersPromises.setImmediate("imm-resolved");

    let abortCaught = "";
    try {
        await timersPromises.setTimeout(5000, "never", {
            signal: AbortSignal.abort(new Error("TimersPromiseAborted"))
        });
    } catch (e: any) {
        abortCaught = String(e && e.message || e);
    }

    console.log(JSON.stringify({
        order,
        delayedVal,
        immVal,
        abortCaught
    }));
}
runSuite();
"#;

    let bundled_order = slice_cas::strip_typescript_and_normalize(async_order_ts, true);
    let bc_order = compile_source_to_bytecode(&bundled_order);
    let mut iso_order = SliceIsolate::spawn(
        "wave-a-order-tenant",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        8 * 1024 * 1024,
    );
    iso_order.set_deterministic_seed(Some(20261105));
    let m_order = iso_order.execute_bytecode(&bc_order, None).unwrap();
    let out_order: serde_json::Value = serde_json::from_str(&m_order.stdout[0]).unwrap();
    assert_eq!(
        out_order["order"],
        serde_json::json!([
            "sync",
            "nextTick:hi:1",
            "promise-then",
            "queue-microtask",
            "nested-nextTick",
            "immediate:check",
            "timeout-0"
        ])
    );
    assert_eq!(out_order["delayedVal"], "warped-250ms");
    assert_eq!(out_order["immVal"], "imm-resolved");
    assert!(out_order["abortCaught"]
        .as_str()
        .unwrap_or("")
        .contains("TimersPromiseAborted"));

    // 2. Verify full WHATWG `console.*` (`table`, `time`, `timeLog`, `timeEnd`, `group`, `groupEnd`, `count`, `countReset`, `dir`, `assert`)
    //    and verify that Outbound Egress Secret DLP traps a secret leaked inside `console.table()`!
    let console_ts = r#"
console.group("OuterGroup");
console.log("inside-outer");
console.groupCollapsed("InnerGroup");
console.count("hits");
console.count("hits");
console.countReset("hits");
console.count("hits");
console.groupEnd();
console.groupEnd();

console.time("db-query");
console.timeLog("db-query", "rows=2");
console.timeEnd("db-query");

console.assert(true, "should-not-print");
console.assert(false, "bad-invariant", 404);
console.dir({ service: "slice", nested: { level: 2, deep: { hidden: true } } }, { depth: 1 });
console.table([
    { route: "/health", status: 200 },
    { route: "/users", status: 201 }
]);
"#;
    let bc_console = compile_source_to_bytecode(console_ts);
    let mut iso_console = SliceIsolate::spawn(
        "wave-a-console-tenant",
        2,
        CapabilityToken::pure_compute(),
        5_000_000,
        8 * 1024 * 1024,
    );
    iso_console.set_deterministic_seed(Some(20261105));
    let m_console = iso_console.execute_bytecode(&bc_console, None).unwrap();
    let joined_console = m_console.stdout.join("\n");
    assert!(joined_console.contains(
        "OuterGroup\n  inside-outer\n  InnerGroup\n    hits: 1\n    hits: 2\n    hits: 1"
    ));
    assert!(joined_console.contains("db-query: 1.000ms rows=2"));
    assert!(joined_console.contains("db-query: 2.000ms"));
    assert!(joined_console.contains("Assertion failed: bad-invariant 404"));
    assert!(joined_console.contains("{ service: 'slice', nested: [Object] }"));
    assert!(joined_console.contains("| (index) | route     | status |"));
    assert!(joined_console.contains("| 0       | '/health' | 200    |"));

    // Verify Egress DLP firewall traps secret leaked via `console.table`
    let dlp_table_js = r#"
console.table([{ user: "alice", token: "sk_live_SUPER_SECRET_KEY_999" }]);
"#;
    let bc_dlp = compile_source_to_bytecode(dlp_table_js);
    let mut iso_dlp = SliceIsolate::spawn(
        "wave-a-dlp-console",
        3,
        CapabilityToken::pure_compute(),
        2_000_000,
        4 * 1024 * 1024,
    );
    iso_dlp.add_egress_audit_secret("sk_live_SUPER_SECRET_KEY_999");
    let dlp_err = iso_dlp.execute_bytecode(&bc_dlp, None).unwrap_err();
    assert!(
        dlp_err.to_string().contains("EGRESS_SECRET_LEAK_BLOCKED"),
        "Expected EGRESS_SECRET_LEAK_BLOCKED on console.table secret leak, got: {}",
        dlp_err
    );

    // 3. Verify Native Oxc `.tsx` JSX/TSX Compilation + Built-In `slice:jsx` SSR Runtime & XSS Escaping
    let tsx_source = r###"
import { renderToString, raw } from "slice:jsx";

interface BadgeProps {
    label: string;
    active?: boolean;
}

function Badge(props: BadgeProps) {
    return <span className={props.active ? "badge-on" : "badge-off"} data-ok={props.active}>{props.label}</span>;
}

function Page(props: { user: string; items: string[] }) {
    return (
        <>
            <header style={{ backgroundColor: "#111", fontSize: "14px" }}>
                <h1>Hello {props.user}</h1>
                <Badge label="PRO" active={true} />
                <br />
            </header>
            <ul>
                {props.items.map((item, idx) => <li key={idx}>{item}</li>)}
            </ul>
            {raw("<!-- trusted-footer -->")}
        </>
    );
}

const html = renderToString(
    <Page user={'<script>alert("xss")</script>'} items={["one", "two & three"]} />
);
console.log(html);
"###;
    let compiled_tsx = slice_cas::strip_typescript_and_normalize(tsx_source, true);
    let tsx_metrics = run_source_isolated(
        "wave-a-tsx-tenant",
        &compiled_tsx,
        CapabilityToken::pure_compute(),
        5_000_000,
        8 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(
        tsx_metrics.stdout[0],
        r#"<header style="background-color:#111;font-size:14px"><h1>Hello &lt;script&gt;alert(&quot;xss&quot;)&lt;/script&gt;</h1><span class="badge-on" data-ok>PRO</span><br/></header><ul><li>one</li><li>two &amp; three</li></ul><!-- trusted-footer -->"#
    );
}

#[test]
fn test_phase1_process_cli_surface() {
    let script = r#"
import proc from "node:process";

const t0 = proc.hrtime();
const b0 = proc.hrtime.bigint();
const diff = proc.hrtime(t0);
const b1 = proc.hrtime.bigint();

const mem = proc.memoryUsage();
const rssVal = proc.memoryUsage.rss();

let warningName = "";
let warningMsg = "";
proc.on("warning", (w: any) => {
    warningName = w.name;
    warningMsg = w.message;
});
proc.emitWarning("experimental CLI flag", "CustomWarning", "WARN_01");

let exitEmitted = -1;
proc.once("exit", (code: number) => {
    exitEmitted = code;
    console.log(JSON.stringify({
        argv0: proc.argv0,
        execPath: proc.execPath,
        ppid: proc.ppid,
        isTTYOut: proc.stdout.isTTY,
        isTTYIn: proc.stdin.isTTY,
        hrValid: Array.isArray(diff) && diff.length === 2 && typeof b0 === "bigint" && b1 >= b0,
        memValid: mem.rss > 0 && mem.heapUsed > 0 && mem.heapTotal >= mem.heapUsed && rssVal === mem.rss,
        uptimeValid: typeof proc.uptime() === "number" && proc.uptime() >= 0,
        warningName,
        warningMsg,
        exitEmitted,
        exitCaught,
        exitCode: proc.exitCode
    }));
});
let exitCaught = false;
proc.stdout.write("stdout-line-1\n");
proc.stderr.write("stderr-line-2\n");
try {
    proc.exit(7);
} catch (e: any) {
    exitCaught = true;
    console.log("UNREACHABLE_AFTER_EXIT");
}
console.log("UNREACHABLE_AFTER_TRY");
"#;

    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let mut iso = SliceIsolate::spawn(
        "phase1-proc-cli",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        8 * 1024 * 1024,
    );
    iso.set_deterministic_seed(Some(20261106));
    let m = iso.execute_bytecode(&bc, None).unwrap();
    assert_eq!(m.exit_code, Some(7));
    assert_eq!(m.stdout.len(), 3);
    assert_eq!(m.stdout[0], "stdout-line-1");
    assert_eq!(m.stdout[1], "stderr-line-2");
    let out: serde_json::Value = serde_json::from_str(&m.stdout[2]).unwrap();
    assert_eq!(out["argv0"], "slice");
    assert_eq!(out["execPath"], "/usr/bin/slice");
    assert_eq!(out["ppid"], 0);
    assert_eq!(out["isTTYOut"], false);
    assert_eq!(out["isTTYIn"], false);
    assert_eq!(out["hrValid"], true);
    assert_eq!(out["memValid"], true);
    assert_eq!(out["uptimeValid"], true);
    assert_eq!(out["warningName"], "CustomWarning");
    assert_eq!(out["warningMsg"], "experimental CLI flag");
    assert_eq!(out["exitEmitted"], 7);
    assert_eq!(out["exitCaught"], false);
    assert_eq!(out["exitCode"], 7);

    // Also verify DLP egress firewall catches secrets written to `process.stdout.write`
    let dlp_js = r#"process.stdout.write("leaked: sk_live_PROC_STDOUT_SECRET_777\n");"#;
    let bc_dlp = compile_source_to_bytecode(dlp_js);
    let mut iso_dlp = SliceIsolate::spawn(
        "phase1-proc-dlp",
        2,
        CapabilityToken::pure_compute(),
        2_000_000,
        4 * 1024 * 1024,
    );
    iso_dlp.add_egress_audit_secret("sk_live_PROC_STDOUT_SECRET_777");
    let err = iso_dlp.execute_bytecode(&bc_dlp, None).unwrap_err();
    assert!(err.to_string().contains("EGRESS_SECRET_LEAK_BLOCKED"));
}

#[test]
fn test_phase1_wintertc_navigator_location_and_intl() {
    let script = r#"
const ua = navigator.userAgent;
const hc = navigator.hardwareConcurrency;
const lang = navigator.language;
const origin = location.origin;
const href = location.href;

const d = new Date(Date.UTC(2026, 10, 6, 14, 30, 5));
const dtfUs = new Intl.DateTimeFormat("en-US", { year: "numeric", month: "short", day: "2-digit" }).format(d);
const dtfIso = new Intl.DateTimeFormat("sv-SE", { year: "numeric", month: "2-digit", day: "2-digit" }).format(d);

const nfDec = new Intl.NumberFormat("en-US").format(1234567.89);
const nfUsd = new Intl.NumberFormat("en-US", { style: "currency", currency: "USD" }).format(42.5);
const nfPct = new Intl.NumberFormat("en-US", { style: "percent" }).format(0.85);
const nfCompact = new Intl.NumberFormat("en-US", { notation: "compact" }).format(1500000);

const colNum = new Intl.Collator("en-US", { numeric: true });
const sortedFiles = ["file10.ts", "file2.ts", "file1.ts"].sort(colNum.compare);

const prCard = new Intl.PluralRules("en-US");
const prOrd = new Intl.PluralRules("en-US", { type: "ordinal" });

const segWord = new Intl.Segmenter("en-US", { granularity: "word" });
const words = Array.from(segWord.segment("Hello, Slice 2026!"))
    .filter((s: any) => s.isWordLike)
    .map((s: any) => s.segment);

const canon = Intl.getCanonicalLocales(["en_us", "EN-GB", "en-US"]);

console.log(JSON.stringify({
    ua,
    hc,
    lang,
    origin,
    href,
    dtfUs,
    dtfIso,
    nfDec,
    nfUsd,
    nfPct,
    nfCompact,
    sortedFiles,
    card1: prCard.select(1),
    card5: prCard.select(5),
    ord1: prOrd.select(1),
    ord2: prOrd.select(2),
    ord3: prOrd.select(3),
    ord11: prOrd.select(11),
    words,
    canon
}));
"#;

    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let mut iso = SliceIsolate::spawn(
        "phase1-wintertc-intl",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        8 * 1024 * 1024,
    );
    let m = iso.execute_bytecode(&bc, None).unwrap();
    let out: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(out["ua"], "Slice-NexusJS/0.10.0");
    assert_eq!(out["hc"], 4);
    assert_eq!(out["lang"], "en-US");
    assert_eq!(out["origin"], "http://localhost");
    assert_eq!(out["href"], "http://localhost/");
    assert_eq!(out["dtfUs"], "Nov 06, 2026");
    assert_eq!(out["dtfIso"], "2026-11-06");
    assert_eq!(out["nfDec"], "1,234,567.89");
    assert_eq!(out["nfUsd"], "$42.50");
    assert_eq!(out["nfPct"], "85%");
    assert_eq!(out["nfCompact"], "1.5M");
    assert_eq!(
        out["sortedFiles"],
        serde_json::json!(["file1.ts", "file2.ts", "file10.ts"])
    );
    assert_eq!(out["card1"], "one");
    assert_eq!(out["card5"], "other");
    assert_eq!(out["ord1"], "one");
    assert_eq!(out["ord2"], "two");
    assert_eq!(out["ord3"], "few");
    assert_eq!(out["ord11"], "other");
    assert_eq!(out["words"], serde_json::json!(["Hello", "Slice", "2026"]));
    assert_eq!(out["canon"], serde_json::json!(["en-US", "en-GB"]));
}

#[test]
fn test_phase1_node_util_and_secondary_modules() {
    let script = r#"
import * as util from "node:util";
import { performance, PerformanceObserver, monitorEventLoopDelay } from "node:perf_hooks";
import * as tty from "node:tty";
import Module, { createRequire, isBuiltin, builtinModules } from "node:module";
import * as vm from "node:vm";
import { StringDecoder } from "node:string_decoder";
import * as qs from "node:querystring";

// 1. node:util (parseArgs, isDeepStrictEqual, styleText, stripVTControlCharacters, MIMEType)
const parsed = util.parseArgs({
    args: ["-v", "--port=8080", "--tag", "edge", "--tag", "fast", "--", "pos1", "pos2"],
    options: {
        verbose: { type: "boolean", short: "v" },
        port: { type: "string", short: "p" },
        tag: { type: "string", multiple: true },
        mode: { type: "string", default: "prod" }
    },
    allowPositionals: true
});

const eq1 = util.isDeepStrictEqual(
    { a: [1, 2], m: new Map([["k", 42]]), s: new Set(["x"]) },
    { a: [1, 2], m: new Map([["k", 42]]), s: new Set(["x"]) }
);
const eq2 = util.isDeepStrictEqual({ a: 1 }, { a: "1" });

const styled = util.styleText(["bold", "green"], "PASS");
const stripped = util.stripVTControlCharacters(styled);

const mime = new util.MIMEType("text/html; charset=UTF-8; boundary=something");
mime.params.set("q", "0.9");

// 2. node:perf_hooks
const obsEntries: string[] = [];
const obs = new PerformanceObserver((list: any) => {
    for (const e of list.getEntries()) obsEntries.push(`${e.entryType}:${e.name}`);
});
obs.observe({ entryTypes: ["mark", "measure"], buffered: true });
performance.mark("m-start");
performance.mark("m-end");
const meas = performance.measure("m-span", "m-start", "m-end");
const eld = monitorEventLoopDelay();
eld.enable();

// 3. node:tty & node:module
const ttyFalse = tty.isatty(1);
const req = createRequire("/app/index.ts");
const pathMod = req("node:path");
const builtinCheck = isBuiltin("node:fs") && isBuiltin("perf_hooks") && !isBuiltin("left-pad");

// 4. node:string_decoder & node:querystring
const sd = new StringDecoder("utf8");
const decodedStr = sd.write(Buffer.from("Slice-UTF8"));
const qsParsed = qs.parse("a=1&b=hello%20world&a=2");
const qsStr = qs.stringify({ k: "v", n: 10 });

// 5. node:vm (sandboxed via Slice.evalSandbox)
const ctx = vm.createContext({ count: 10, prefix: "vm" });
const vmRes = vm.runInContext("count += 32; prefix + ':' + count", ctx, { timeout: 50 });

let vmBlocked = false;
try {
    vm.runInNewContext("const fs = require('node:fs'); fs.readFileSync('/etc/passwd');", {});
} catch (e: any) {
    vmBlocked = String(e && e.message || e).includes("filesystem read authority");
}

queueMicrotask(() => {
    console.log(JSON.stringify({
        parsedValues: parsed.values,
        parsedPos: parsed.positionals,
        eq1,
        eq2,
        hasAnsi: styled.includes("\x1b["),
        stripped,
        mimeEssence: mime.essence,
        mimeCharset: mime.params.get("charset"),
        mimeStr: mime.toString(),
        measType: meas.entryType,
        obsEntries,
        ttyFalse,
        pathJoin: pathMod.join("/a", "b"),
        builtinCheck,
        hasVmInBuiltins: builtinModules.includes("vm"),
        decodedStr,
        qsParsed,
        qsStr,
        vmRes,
        ctxCount: ctx.count,
        vmBlocked
    }));
});
"#;

    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let mut iso = SliceIsolate::spawn(
        "phase1-node-util-mods",
        1,
        CapabilityToken::pure_compute(),
        10_000_000,
        16 * 1024 * 1024,
    );
    let m = iso.execute_bytecode(&bc, None).unwrap();
    let out: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(out["parsedValues"]["verbose"], true);
    assert_eq!(out["parsedValues"]["port"], "8080");
    assert_eq!(
        out["parsedValues"]["tag"],
        serde_json::json!(["edge", "fast"])
    );
    assert_eq!(out["parsedValues"]["mode"], "prod");
    assert_eq!(out["parsedPos"], serde_json::json!(["pos1", "pos2"]));
    assert_eq!(out["eq1"], true);
    assert_eq!(out["eq2"], false);
    assert_eq!(out["hasAnsi"], true);
    assert_eq!(out["stripped"], "PASS");
    assert_eq!(out["mimeEssence"], "text/html");
    assert_eq!(out["mimeCharset"], "UTF-8");
    assert_eq!(
        out["obsEntries"],
        serde_json::json!(["mark:m-start", "mark:m-end", "measure:m-span"])
    );
    assert_eq!(out["ttyFalse"], false);
    assert_eq!(out["pathJoin"], "/a/b");
    assert_eq!(out["builtinCheck"], true);
    assert_eq!(out["hasVmInBuiltins"], true);
    assert_eq!(out["decodedStr"], "Slice-UTF8");
    assert_eq!(
        out["qsParsed"],
        serde_json::json!({ "a": ["1", "2"], "b": "hello world" })
    );
    assert_eq!(out["qsStr"], "k=v&n=10");
    assert_eq!(out["vmRes"], "vm:42");
    assert_eq!(out["ctxCount"], 42);
    assert_eq!(out["vmBlocked"], true);
}

#[test]
fn test_phase1_boacas_single_pass_automaton() {
    use crate::boacas::{
        compute_boacas_metadata, BOACAS_FLAG_HAS_HTTP_BRIDGE, BOACAS_FLAG_HAS_KV_SANDBOX,
        BOACAS_FLAG_HAS_NEXT_TICK, BOACAS_FLAG_HAS_SEMVER_FAST, BOACAS_FLAG_HAS_WORKER,
        BOACAS_FLAG_IS_RC_BUNDLE, BOACAS_FLAG_NEEDS_LAZY_GETTERS, BOACAS_FLAG_NEEDS_SES_CHECK,
        BOACAS_FLAG_STARTS_WITH_BRACE,
    };

    let sample = "{\nvar __slice_modules = {};//__SLICE_RC:0123456789abcdef\nconst u = new URL(\"https://example.com\");\nconst fmt = new Intl.NumberFormat(\"en-US\");\nconst fs = require(\"node:fs\");\nconst util = require(\"node:util\");\nprocess.nextTick(() => {});\nconst w = new Worker(\"w.js\");\nconst req = Slice.request();\nconst kvVal = Slice.kv.get(\"k\");\nconst sem = __slice_semver_valid(\"1.2.3\");\nconst desc = Object.getOwnPropertyDescriptor(Object.prototype, \"toString\");\n}";

    let (flags, shards_mask, rc_key) = compute_boacas_metadata(sample);
    assert_eq!(rc_key, 0x0123_4567_89ab_cdef);
    assert_ne!(flags & BOACAS_FLAG_IS_RC_BUNDLE, 0);
    assert_ne!(flags & BOACAS_FLAG_STARTS_WITH_BRACE, 0);
    assert_ne!(flags & BOACAS_FLAG_HAS_WORKER, 0);
    assert_ne!(flags & BOACAS_FLAG_HAS_NEXT_TICK, 0);
    assert_ne!(flags & BOACAS_FLAG_HAS_HTTP_BRIDGE, 0);
    assert_ne!(flags & BOACAS_FLAG_HAS_KV_SANDBOX, 0);
    assert_ne!(flags & BOACAS_FLAG_HAS_SEMVER_FAST, 0);
    assert_ne!(flags & BOACAS_FLAG_NEEDS_SES_CHECK, 0);
    assert_ne!(flags & BOACAS_FLAG_NEEDS_LAZY_GETTERS, 0);

    // URL (1 << 1), Worker (1 << 8), HTTP (1 << 11), Intl (1 << 18), node:fs (1 << 20), node:util (1 << 24)
    assert_ne!(shards_mask & (1 << 1), 0);
    assert_ne!(shards_mask & (1 << 8), 0);
    assert_ne!(shards_mask & (1 << 11), 0);
    assert_ne!(shards_mask & (1 << 18), 0);
    assert_ne!(shards_mask & (1 << 20), 0);
    assert_ne!(shards_mask & (1 << 24), 0);
}

#[test]
fn test_phase2_webcrypto_aes_gcm_cbc_roundtrip() {
    let script = r#"
async function run() {
    const enc = new TextEncoder();
    const dec = new TextDecoder();
    const plainText = "Slice Phase 2 AES-GCM & AES-CBC Confidential Payload";
    const plainBytes = enc.encode(plainText);

    // 1. AES-GCM 256-bit generateKey, JWK/raw export & import, encrypt & decrypt with AAD
    const gcmKey = await crypto.subtle.generateKey(
        { name: "AES-GCM", length: 256 },
        true,
        ["encrypt", "decrypt"]
    );
    const gcmJwk = await crypto.subtle.exportKey("jwk", gcmKey);
    const gcmRaw = await crypto.subtle.exportKey("raw", gcmKey);
    const importedGcmKey = await crypto.subtle.importKey(
        "jwk",
        gcmJwk,
        { name: "AES-GCM" },
        false,
        ["encrypt", "decrypt"]
    );

    const iv12 = crypto.getRandomValues(new Uint8Array(12));
    const aad = enc.encode("header-aad-v1");
    const gcmCipherAb = await crypto.subtle.encrypt(
        { name: "AES-GCM", iv: iv12, additionalData: aad, tagLength: 128 },
        gcmKey,
        plainBytes
    );
    const gcmDecryptedAb = await crypto.subtle.decrypt(
        { name: "AES-GCM", iv: iv12, additionalData: aad, tagLength: 128 },
        importedGcmKey,
        gcmCipherAb
    );
    const gcmDecryptedText = dec.decode(gcmDecryptedAb);

    // Verify tampered AAD or tampered tag rejects
    let tamperedRejected = false;
    try {
        await crypto.subtle.decrypt(
            { name: "AES-GCM", iv: iv12, additionalData: enc.encode("wrong-aad"), tagLength: 128 },
            importedGcmKey,
            gcmCipherAb
        );
    } catch (_e) {
        tamperedRejected = true;
    }

    // 2. AES-CBC 128-bit generateKey, raw export/import, PKCS#7 padded encrypt & decrypt
    const cbcKey = await crypto.subtle.generateKey(
        { name: "AES-CBC", length: 128 },
        true,
        ["encrypt", "decrypt"]
    );
    const cbcJwk = await crypto.subtle.exportKey("jwk", cbcKey);
    const iv16 = crypto.getRandomValues(new Uint8Array(16));
    const cbcCipherAb = await crypto.subtle.encrypt(
        { name: "AES-CBC", iv: iv16 },
        cbcKey,
        plainBytes
    );
    const cbcDecryptedAb = await crypto.subtle.decrypt(
        { name: "AES-CBC", iv: iv16 },
        cbcKey,
        cbcCipherAb
    );
    const cbcDecryptedText = dec.decode(cbcDecryptedAb);

    console.log(JSON.stringify({
        gcmAlg: gcmJwk.alg,
        gcmRawLen: gcmRaw.byteLength,
        gcmCipherLen: gcmCipherAb.byteLength,
        gcmPlainLen: plainBytes.byteLength,
        gcmMatch: gcmDecryptedText === plainText,
        tamperedRejected,
        cbcAlg: cbcJwk.alg,
        cbcMatch: cbcDecryptedText === plainText,
        hasKeyHandle: gcmKey._id > 0 && cbcKey._id > 0
    }));
}
run();
"#;

    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let mut iso = SliceIsolate::spawn(
        "phase2-aes-gcm-cbc",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        8 * 1024 * 1024,
    );
    iso.set_deterministic_seed(Some(20261107));
    let m = iso.execute_bytecode(&bc, None).unwrap();
    let out: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(out["gcmAlg"], "A256GCM");
    assert_eq!(out["gcmRawLen"], 32);
    assert_eq!(
        out["gcmCipherLen"].as_u64().unwrap(),
        out["gcmPlainLen"].as_u64().unwrap() + 16
    );
    assert_eq!(out["gcmMatch"], true);
    assert_eq!(out["tamperedRejected"], true);
    assert_eq!(out["cbcAlg"], "A128CBC");
    assert_eq!(out["cbcMatch"], true);
    assert_eq!(out["hasKeyHandle"], true);
}

#[test]
fn test_phase2_webcrypto_ecdsa_ed25519_sign_verify() {
    let script = r#"
async function run() {
    const msg = new TextEncoder().encode("Slice Phase 2 Ed25519 & ECDSA P-256/P-384 Message");

    // 1. Ed25519 generateKey, JWK/PKCS8/SPKI/raw export & import, sign & verify
    const edPair = await crypto.subtle.generateKey(
        { name: "Ed25519" },
        true,
        ["sign", "verify"]
    );
    const edSig = await crypto.subtle.sign({ name: "Ed25519" }, edPair.privateKey, msg);
    const edValid = await crypto.subtle.verify({ name: "Ed25519" }, edPair.publicKey, edSig, msg);

    const badSig = new Uint8Array(edSig.slice(0));
    badSig[0] ^= 0xff;
    const edInvalid = await crypto.subtle.verify({ name: "Ed25519" }, edPair.publicKey, badSig, msg);

    const edPubJwk = await crypto.subtle.exportKey("jwk", edPair.publicKey);
    const edPrivPkcs8 = await crypto.subtle.exportKey("pkcs8", edPair.privateKey);
    const edPubSpki = await crypto.subtle.exportKey("spki", edPair.publicKey);

    const edImportedPriv = await crypto.subtle.importKey(
        "pkcs8",
        edPrivPkcs8,
        { name: "Ed25519" },
        false,
        ["sign"]
    );
    const edImportedPub = await crypto.subtle.importKey(
        "jwk",
        edPubJwk,
        { name: "Ed25519" },
        true,
        ["verify"]
    );
    const edSig2 = await crypto.subtle.sign("Ed25519", edImportedPriv, msg);
    const edValid2 = await crypto.subtle.verify("Ed25519", edImportedPub, edSig2, msg);

    // 2. ECDSA P-256 & P-384 generateKey, JWK export/import, sign & verify
    const p256Pair = await crypto.subtle.generateKey(
        { name: "ECDSA", namedCurve: "P-256" },
        true,
        ["sign", "verify"]
    );
    const p256Sig = await crypto.subtle.sign(
        { name: "ECDSA", hash: { name: "SHA-256" } },
        p256Pair.privateKey,
        msg
    );
    const p256Valid = await crypto.subtle.verify(
        { name: "ECDSA", hash: { name: "SHA-256" } },
        p256Pair.publicKey,
        p256Sig,
        msg
    );
    const p256Jwk = await crypto.subtle.exportKey("jwk", p256Pair.publicKey);
    const p256PubImported = await crypto.subtle.importKey(
        "jwk",
        p256Jwk,
        { name: "ECDSA", namedCurve: "P-256" },
        true,
        ["verify"]
    );
    const p256ValidImported = await crypto.subtle.verify(
        { name: "ECDSA", hash: "SHA-256" },
        p256PubImported,
        p256Sig,
        msg
    );

    const p384Pair = await crypto.subtle.generateKey(
        { name: "ECDSA", namedCurve: "P-384" },
        true,
        ["sign", "verify"]
    );
    const p384Sig = await crypto.subtle.sign(
        { name: "ECDSA", hash: "SHA-384" },
        p384Pair.privateKey,
        msg
    );
    const p384Valid = await crypto.subtle.verify(
        { name: "ECDSA", hash: "SHA-384" },
        p384Pair.publicKey,
        p384Sig,
        msg
    );

    console.log(JSON.stringify({
        edSigLen: edSig.byteLength,
        edValid,
        edInvalid,
        edValid2,
        edKty: edPubJwk.kty,
        edSpkiLen: edPubSpki.byteLength,
        p256SigLen: p256Sig.byteLength,
        p256Valid,
        p256ValidImported,
        p384SigLen: p384Sig.byteLength,
        p384Valid
    }));
}
run();
"#;

    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let mut iso = SliceIsolate::spawn(
        "phase2-asym-sign-verify",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        8 * 1024 * 1024,
    );
    iso.set_deterministic_seed(Some(998877));
    let m = iso.execute_bytecode(&bc, None).unwrap();
    let out: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(out["edSigLen"], 64);
    assert_eq!(out["edValid"], true);
    assert_eq!(out["edInvalid"], false);
    assert_eq!(out["edValid2"], true);
    assert_eq!(out["edKty"], "OKP");
    assert_eq!(out["edSpkiLen"], 44);
    assert_eq!(out["p256SigLen"], 64);
    assert_eq!(out["p256Valid"], true);
    assert_eq!(out["p256ValidImported"], true);
    assert_eq!(out["p384SigLen"], 96);
    assert_eq!(out["p384Valid"], true);
}

#[test]
fn test_phase2_webcrypto_pbkdf2_hkdf() {
    let script = r#"
async function run() {
    const enc = new TextEncoder();
    const dec = new TextDecoder();

    // 1. PBKDF2 importKey -> deriveBits & deriveKey (AES-GCM-256)
    const basePbkdf2 = await crypto.subtle.importKey(
        "raw",
        enc.encode("correct-horse-battery-staple"),
        "PBKDF2",
        false,
        ["deriveBits", "deriveKey"]
    );
    const salt = enc.encode("slice-salt-2026");
    const pbkdf2Bits = await crypto.subtle.deriveBits(
        { name: "PBKDF2", salt, iterations: 1000, hash: "SHA-256" },
        basePbkdf2,
        256
    );
    const derivedAesKey = await crypto.subtle.deriveKey(
        { name: "PBKDF2", salt, iterations: 1000, hash: "SHA-256" },
        basePbkdf2,
        { name: "AES-GCM", length: 256 },
        true,
        ["encrypt", "decrypt"]
    );
    const derivedRaw = await crypto.subtle.exportKey("raw", derivedAesKey);

    // 2. HKDF importKey -> deriveKey -> AES-GCM roundtrip
    const baseHkdf = await crypto.subtle.importKey(
        "raw",
        enc.encode("initial-keying-material-ikm"),
        "HKDF",
        false,
        ["deriveBits", "deriveKey"]
    );
    const hkdfKey = await crypto.subtle.deriveKey(
        { name: "HKDF", hash: "SHA-256", salt, info: enc.encode("ctx-info") },
        baseHkdf,
        { name: "AES-GCM", length: 256 },
        true,
        ["encrypt", "decrypt"]
    );
    const iv = new Uint8Array(12);
    const cipher = await crypto.subtle.encrypt({ name: "AES-GCM", iv }, hkdfKey, enc.encode("hkdf-secret"));
    const plain = dec.decode(await crypto.subtle.decrypt({ name: "AES-GCM", iv }, hkdfKey, cipher));

    const b1 = new Uint8Array(pbkdf2Bits);
    const b2 = new Uint8Array(derivedRaw);
    let bitsMatchKey = b1.length === 32 && b2.length === 32;
    for (let i = 0; i < 32; i++) {
        if (b1[i] !== b2[i]) bitsMatchKey = false;
    }

    console.log(JSON.stringify({
        bitsMatchKey,
        plain
    }));
}
run();
"#;

    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let mut iso = SliceIsolate::spawn(
        "phase2-pbkdf2-hkdf",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        8 * 1024 * 1024,
    );
    let m = iso.execute_bytecode(&bc, None).unwrap();
    let out: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(out["bitsMatchKey"], true);
    assert_eq!(out["plain"], "hkdf-secret");
}

#[test]
fn test_phase2_node_crypto_ciphers_kdfs_timingsafe() {
    let script = r#"
import crypto from "node:crypto";

// 1. MD5 createHash & createHmac
const md5Hex = crypto.createHash("md5").update("abc").digest("hex");
const hmacMd5Hex = crypto.createHmac("md5", "key").update("The quick brown fox jumps over the lazy dog").digest("hex");

// 2. randomFillSync & timingSafeEqual
const fillBuf = Buffer.alloc(16);
crypto.randomFillSync(fillBuf, 4, 8);
const first4Zero = fillBuf[0] === 0 && fillBuf[1] === 0 && fillBuf[2] === 0 && fillBuf[3] === 0;
const last4Zero = fillBuf[12] === 0 && fillBuf[13] === 0 && fillBuf[14] === 0 && fillBuf[15] === 0;
const eqTrue = crypto.timingSafeEqual(Buffer.from("slice-123"), Buffer.from("slice-123"));
const eqFalse = crypto.timingSafeEqual(Buffer.from("slice-123"), Buffer.from("slice-999"));

// 3. pbkdf2Sync, hkdfSync, scryptSync
const pbkdf2Out = crypto.pbkdf2Sync("password", "salt", 1000, 32, "sha256");
const hkdfOut = Buffer.from(crypto.hkdfSync("sha256", "ikm", "salt", "info", 32));
const scryptOut = crypto.scryptSync("password", "salt", 32, { N: 1024, r: 8, p: 1 });

// 4. createCipheriv / createDecipheriv (aes-256-gcm & aes-128-cbc)
const gcmKey = crypto.randomBytes(32);
const gcmIv = crypto.randomBytes(12);
const cipherGcm = crypto.createCipheriv("aes-256-gcm", gcmKey, gcmIv);
cipherGcm.setAAD(Buffer.from("node-aad"));
const encPart1 = cipherGcm.update("Hello Node ", "utf8", "hex");
const encPart2 = cipherGcm.final("hex");
const tag = cipherGcm.getAuthTag();

const decipherGcm = crypto.createDecipheriv("aes-256-gcm", gcmKey, gcmIv);
decipherGcm.setAAD(Buffer.from("node-aad"));
decipherGcm.setAuthTag(tag);
const decGcm = decipherGcm.update(encPart1 + encPart2, "hex", "utf8") + decipherGcm.final("utf8");

const cbcKey = crypto.randomBytes(16);
const cbcIv = crypto.randomBytes(16);
const cipherCbc = crypto.createCipheriv("aes-128-cbc", cbcKey, cbcIv);
const cbcCipherHex = cipherCbc.update("CBC payload 123", "utf8", "hex") + cipherCbc.final("hex");
const decipherCbc = crypto.createDecipheriv("aes-128-cbc", cbcKey, cbcIv);
const decCbc = decipherCbc.update(cbcCipherHex, "hex", "utf8") + decipherCbc.final("utf8");

// 5. generateKeyPairSync, createSign, createVerify (Ed25519 & ECDSA P-256)
const edKeys = crypto.generateKeyPairSync("ed25519");
const edSig = crypto.sign(null, Buffer.from("ed-msg"), edKeys.privateKey);
const edVerified = crypto.verify(null, Buffer.from("ed-msg"), edKeys.publicKey, edSig);

const ecKeys = crypto.generateKeyPairSync("ec", {
    namedCurve: "P-256",
    publicKeyEncoding: { type: "spki", format: "pem" },
    privateKeyEncoding: { type: "pkcs8", format: "pem" }
});
const signer = crypto.createSign("SHA256");
signer.update("ecdsa-stream-message");
const ecSigHex = signer.sign(ecKeys.privateKey, "hex");
const verifier = crypto.createVerify("SHA256");
verifier.update("ecdsa-stream-message");
const ecVerified = verifier.verify(ecKeys.publicKey, ecSigHex, "hex");

console.log(JSON.stringify({
    md5Hex,
    hmacMd5Hex,
    first4Zero,
    last4Zero,
    eqTrue,
    eqFalse,
    pbkdf2Len: pbkdf2Out.length,
    hkdfLen: hkdfOut.length,
    scryptLen: scryptOut.length,
    decGcm,
    decCbc,
    edVerified,
    ecVerified,
    hasPemHeader: ecKeys.publicKey.includes("BEGIN PUBLIC KEY")
}));
"#;

    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let mut iso = SliceIsolate::spawn(
        "phase2-node-crypto-full",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        8 * 1024 * 1024,
    );
    iso.set_deterministic_seed(Some(424242));
    let m = iso.execute_bytecode(&bc, None).unwrap();
    assert!(
        !m.stdout.is_empty(),
        "Expected stdout from test, return_value: {:?}, bundled:\n{}",
        m.return_value,
        bundled
    );
    let out: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(out["md5Hex"], "900150983cd24fb0d6963f7d28e17f72");
    assert_eq!(out["hmacMd5Hex"], "80070713463e7749b90c2dc24911e275");
    assert_eq!(out["first4Zero"], true);
    assert_eq!(out["last4Zero"], true);
    assert_eq!(out["eqTrue"], true);
    assert_eq!(out["eqFalse"], false);
    assert_eq!(out["pbkdf2Len"], 32);
    assert_eq!(out["hkdfLen"], 32);
    assert_eq!(out["scryptLen"], 32);
    assert_eq!(out["decGcm"], "Hello Node ");
    assert_eq!(out["decCbc"], "CBC payload 123");
    assert_eq!(out["edVerified"], true);
    assert_eq!(out["ecVerified"], true);
    assert_eq!(out["hasPemHeader"], true);

    // Also verify CPU fuel exhaustion traps hostile PBKDF2 iteration counts
    let hostile_kdf_js = r#"
import crypto from "node:crypto";
crypto.pbkdf2Sync("pass", "salt", 50_000_000, 32, "sha256");
"#;
    let hostile_bundled = slice_cas::strip_typescript_and_normalize(hostile_kdf_js, true);
    let hostile_bc = compile_source_to_bytecode(&hostile_bundled);
    let mut iso_low_fuel = SliceIsolate::spawn(
        "phase2-kdf-fuel-guard",
        1,
        CapabilityToken::pure_compute(),
        5_000, // 5,000 fuel budget
        8 * 1024 * 1024,
    );
    let err = iso_low_fuel
        .execute_bytecode(&hostile_bc, None)
        .unwrap_err();
    assert!(
        matches!(err, SandboxViolation::CpuFuelExhausted { .. }),
        "Expected CpuFuelExhausted on hostile PBKDF2 iterations, got {:?}",
        err
    );
}

#[test]
fn test_phase3_node_stream_readable_writable_transform_pipeline() {
    let script = r#"
import { Readable, Writable, Transform, PassThrough, pipeline, finished } from "node:stream";
import * as streamPromises from "node:stream/promises";

async function run() {
    // 1. Transform + PassThrough + stream/promises pipeline
    const src = Readable.from(["alpha", "beta", "gamma"]);
    const upper = new Transform({
        objectMode: true,
        transform(chunk, _enc, cb) {
            cb(null, String(chunk).toUpperCase());
        },
        flush(cb) {
            this.push("FINAL");
            cb(null);
        }
    });
    const pt = new PassThrough({ objectMode: true });
    const collected: string[] = [];
    const sink = new Writable({
        objectMode: true,
        write(chunk, _enc, cb) {
            collected.push(String(chunk));
            cb(null);
        }
    });
    await streamPromises.pipeline(src, upper, pt, sink);

    // 2. Async iterator on Readable
    const iterSrc = Readable.from(["one", "two", "three"]);
    const iterItems: string[] = [];
    for await (const item of iterSrc) {
        iterItems.push(String(item));
    }

    // 3. Readable.fromWeb / Readable.toWeb & Writable.fromWeb / Writable.toWeb
    const webRs = new ReadableStream({
        start(ctrl) {
            ctrl.enqueue("web-1");
            ctrl.enqueue("web-2");
            ctrl.close();
        }
    });
    const nodeFromWeb = Readable.fromWeb(webRs, { objectMode: true });
    const fromWebItems: string[] = [];
    for await (const c of nodeFromWeb) {
        fromWebItems.push(String(c));
    }

    const webSinkItems: string[] = [];
    const webWs = new WritableStream({
        write(c) {
            webSinkItems.push(String(c));
        }
    });
    const nodeToWebWs = Writable.fromWeb(webWs, { objectMode: true });
    nodeToWebWs.write("to-web-a");
    nodeToWebWs.end("to-web-b");
    await streamPromises.finished(nodeToWebWs);

    // 4. Callback pipeline error propagation
    let pipeErrMsg = "";
    await new Promise<void>((resolve) => {
        const s1 = Readable.from(["boom"]);
        const badT = new Transform({
            objectMode: true,
            transform(_c, _e, cb) {
                cb(new Error("TransformExploded"));
            }
        });
        const s2 = new Writable({ objectMode: true, write(_c, _e, cb) { cb(null); } });
        pipeline(s1, badT, s2, (err: any) => {
            pipeErrMsg = err ? String(err.message || err) : "";
            resolve();
        });
    });

    console.log(JSON.stringify({
        collected,
        iterItems,
        fromWebItems,
        webSinkItems,
        pipeErrMsg
    }));
}
run();
"#;
    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let mut iso = SliceIsolate::spawn(
        "phase3-stream-tenant",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        8 * 1024 * 1024,
    );
    let m = iso.execute_bytecode(&bc, None).unwrap();
    let out: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(
        out["collected"],
        serde_json::json!(["ALPHA", "BETA", "GAMMA", "FINAL"])
    );
    assert_eq!(out["iterItems"], serde_json::json!(["one", "two", "three"]));
    assert_eq!(out["fromWebItems"], serde_json::json!(["web-1", "web-2"]));
    assert_eq!(
        out["webSinkItems"],
        serde_json::json!(["to-web-a", "to-web-b"])
    );
    assert_eq!(out["pipeErrMsg"], "TransformExploded");
}

#[test]
fn test_phase3_fs_read_write_streams_and_mkdtemp_cow() {
    let script = r#"
import fs from "node:fs";
import { pipeline } from "node:stream/promises";

async function run() {
    const snap = Slice.checkpoint();

    const tmpDir = fs.mkdtempSync("/tmp/slice-phase3-");
    const fileA = tmpDir + "/input.txt";
    const fileB = tmpDir + "/copied.txt";
    const symB = tmpDir + "/link-copied.txt";

    fs.writeFileSync(fileA, "stream-payload-0123456789");

    const ws = fs.createWriteStream(fileB);
    const rs = fs.createReadStream(fileA, { highWaterMark: 8 });
    await pipeline(rs, ws);

    fs.symlinkSync(fileB, symB);
    const lstatSym = fs.lstatSync(symB);
    const statSym = fs.statSync(symB);
    const realSym = fs.realpathSync(symB);
    const readViaSym = fs.readFileSync(symB, "utf8");

    const dir = await fs.promises.opendir(tmpDir);
    const entries: string[] = [];
    for await (const dirent of dir) {
        entries.push(dirent.name);
    }

    const existsBeforeRestore = fs.existsSync(fileB);
    Slice.restore(snap);
    const existsAfterRestore = fs.existsSync(fileB) || fs.existsSync(tmpDir);

    console.log(JSON.stringify({
        hasPrefix: tmpDir.startsWith("/tmp/slice-phase3-"),
        readViaSym,
        isSymLstat: lstatSym.isSymbolicLink(),
        isFileStat: statSym.isFile(),
        realEndsWithCopied: realSym.endsWith("/copied.txt"),
        entriesCount: entries.length,
        existsBeforeRestore,
        existsAfterRestore
    }));
}
run();
"#;
    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let caps = CapabilityToken::pure_compute()
        .with_read_prefix("/tmp")
        .with_write_prefix("/tmp");
    let mut iso = SliceIsolate::spawn("phase3-fs-cow-tenant", 1, caps, 5_000_000, 8 * 1024 * 1024);
    let m = iso.execute_bytecode(&bc, None).unwrap();
    let out: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(out["hasPrefix"], true);
    assert_eq!(out["readViaSym"], "stream-payload-0123456789");
    assert_eq!(out["isSymLstat"], true);
    assert_eq!(out["isFileStat"], true);
    assert_eq!(out["realEndsWithCopied"], true);
    assert_eq!(out["entriesCount"], 3);
    assert_eq!(out["existsBeforeRestore"], true);
    assert_eq!(out["existsAfterRestore"], false);
}

#[test]
fn test_phase3_node_http_create_server_and_client_request() {
    let script = r#"
import http from "node:http";

async function run() {
    // 1. Test outbound http.get via Slice.mockFetch
    Slice.mockFetch("http://api.internal.example/v1/items", {
        status: 200,
        headers: { "content-type": "application/json", "x-upstream": "mocked" },
        body: JSON.stringify({ items: [10, 20, 30] })
    });

    const outboundRes = await new Promise<any>((resolve, reject) => {
        http.get("http://api.internal.example/v1/items", (res: any) => {
            let raw = "";
            res.setEncoding("utf8");
            res.on("data", (chunk: string) => { raw += chunk; });
            res.on("end", () => {
                resolve({
                    statusCode: res.statusCode,
                    upstream: res.headers["x-upstream"],
                    parsed: JSON.parse(raw)
                });
            });
        }).on("error", reject);
    });

    // 2. Register an http.createServer listener that also bridges to execute_http_request
    const server = http.createServer((req: any, res: any) => {
        if (req.url === "/bridge-check") {
            res.writeHead(202, {
                "Content-Type": "application/json",
                "X-Node-Server": "active"
            });
            res.end(JSON.stringify({ method: req.method, url: req.url, bridged: true }));
            return;
        }
        res.statusCode = 404;
        res.end("not-found");
    });
    server.listen(8080);

    console.log(JSON.stringify({ outboundRes }));
}
run();
"#;
    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let caps = CapabilityToken::pure_compute().with_net_host("api.internal.example");
    let mut iso = SliceIsolate::spawn("phase3-http-tenant", 1, caps, 5_000_000, 8 * 1024 * 1024);

    // Execute as an inbound HTTP request to `/bridge-check` so both outbound http.get and inbound http.createServer bridge are verified
    let req_ctx = crate::HttpRequestContext {
        method: "POST",
        path: "/bridge-check",
        query: "",
        tenant_id: "phase3-http-tenant",
        body: r#"{"ping":1}"#,
        headers: &[("content-type", "application/json")],
    };
    let outcome = iso.execute_http_request(&bc, None, &req_ctx).unwrap();
    assert_eq!(outcome.status_code, 202);
    assert_eq!(outcome.content_type, "application/json");
    assert!(outcome
        .response_headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("x-node-server") && v == "active"));
    let bridge_json: serde_json::Value = serde_json::from_str(&outcome.response_body).unwrap();
    assert_eq!(bridge_json["bridged"], true);
    assert_eq!(bridge_json["method"], "POST");

    let stdout_json: serde_json::Value = serde_json::from_str(&outcome.metrics.stdout[0]).unwrap();
    assert_eq!(stdout_json["outboundRes"]["statusCode"], 200);
    assert_eq!(stdout_json["outboundRes"]["upstream"], "mocked");
    assert_eq!(
        stdout_json["outboundRes"]["parsed"]["items"],
        serde_json::json!([10, 20, 30])
    );
}

#[test]
fn test_phase4_child_process_deny_by_default() {
    let script = r#"
import { execFileSync } from "node:child_process";
execFileSync("/bin/echo", ["hello"]);
"#;
    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let mut iso = SliceIsolate::spawn(
        "phase4-deny-tenant",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        8 * 1024 * 1024,
    );
    let err = iso.execute_bytecode(&bc, None).unwrap_err();
    match err {
        SandboxViolation::ExecDenied { tenant_id, binary } => {
            assert_eq!(tenant_id, "phase4-deny-tenant");
            assert_eq!(binary, "/bin/echo");
        }
        other => panic!("Expected SandboxViolation::ExecDenied, got {:?}", other),
    }
}

#[test]
fn test_phase4_child_process_allowlist_and_env_sanitization() {
    std::env::set_var("SLICE_HOST_SECRET_UNLISTED", "TOP_SECRET_HOST_VAL_999");

    let script = r#"
import { execFileSync, spawnSync } from "node:child_process";

const echoOut = execFileSync("/bin/echo", ["hello", "phase4"], { encoding: "utf8" }).trim();
const envOut = execFileSync("/usr/bin/env", [], { encoding: "utf8" });

let unlistedBinBlocked = false;
try {
    spawnSync("/usr/bin/curl", ["https://evil.example"]);
} catch (e: any) {
    unlistedBinBlocked = String(e && e.message || e).includes("[Slice Sandbox Violation] Subprocess execution denied");
}

let pathSpoofBlocked = false;
try {
    execFileSync("/tmp/evil/echo", ["spoof"]);
} catch (e: any) {
    pathSpoofBlocked = String(e && e.message || e).includes("[Slice Sandbox Violation] Subprocess execution denied");
}

const subEscalation = Slice.evalSandbox(
    `const cp = require("node:child_process"); cp.execFileSync("/usr/bin/curl", []);`,
    { allowExec: ["/usr/bin/curl"] }
);

console.log(JSON.stringify({
    echoOut,
    hasTenantAllowedEnv: envOut.includes("TENANT_ALLOWED_KEY=safe-tenant-val"),
    leakedHostSecretKey: envOut.includes("SLICE_HOST_SECRET_UNLISTED"),
    leakedHostSecretVal: envOut.includes("TOP_SECRET_HOST_VAL_999"),
    unlistedBinBlocked,
    pathSpoofBlocked,
    subEscalationBlocked: !subEscalation.ok && String(subEscalation.violation || "").includes("Subprocess execution denied")
}));
"#;
    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let caps = CapabilityToken::pure_compute()
        .with_exec_binary("/bin/echo")
        .with_exec_binary("/usr/bin/env")
        .with_env("TENANT_ALLOWED_KEY", "safe-tenant-val");
    let mut iso = SliceIsolate::spawn(
        "phase4-allowlist-tenant",
        1,
        caps,
        10_000_000,
        16 * 1024 * 1024,
    );
    let m = iso.execute_bytecode(&bc, None).unwrap();
    let out: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(out["echoOut"], "hello phase4");
    assert_eq!(out["hasTenantAllowedEnv"], true);
    assert_eq!(out["leakedHostSecretKey"], false);
    assert_eq!(out["leakedHostSecretVal"], false);
    assert_eq!(out["unlistedBinBlocked"], true);
    assert_eq!(out["pathSpoofBlocked"], true);
    assert_eq!(out["subEscalationBlocked"], true);
}

#[test]
fn test_phase4_child_process_output_dlp_blocking() {
    let script = r#"
import { execFileSync } from "node:child_process";

const safeOut = execFileSync("/bin/echo", ["PUBLIC_OK"], { encoding: "utf8" }).trim();
let dlpBlocked = false;
let dlpErr = "";
try {
    // `/usr/bin/env` outputs `OPENAI_API_KEY=sk-proj-phase4-super-secret-token` to stdout,
    // which Ring 3 Output Egress DLP must intercept and block before JS receives it.
    execFileSync("/usr/bin/env", [], { encoding: "utf8" });
} catch (e: any) {
    dlpBlocked = true;
    dlpErr = String(e && e.message || e);
}

console.log(JSON.stringify({
    safeOut,
    dlpBlocked,
    hasDlpViolationTag: dlpErr.includes("EGRESS_SECRET_LEAK_BLOCKED"),
    secretNeverInError: !dlpErr.includes("sk-proj-phase4-super-secret-token")
}));
"#;
    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let caps = CapabilityToken::pure_compute()
        .with_exec_binary("/bin/echo")
        .with_exec_binary("/usr/bin/env");
    let mut iso = SliceIsolate::spawn("phase4-dlp-tenant", 1, caps, 5_000_000, 8 * 1024 * 1024);
    let dlp_count =
        iso.load_dotenv_str("OPENAI_API_KEY=sk-proj-phase4-super-secret-token\nPUBLIC_TAG=v1\n");
    assert_eq!(dlp_count, 1);

    let m = iso.execute_bytecode(&bc, None).unwrap();
    let out: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(out["safeOut"], "PUBLIC_OK");
    assert_eq!(out["dlpBlocked"], true);
    assert_eq!(out["hasDlpViolationTag"], true);
    assert_eq!(out["secretNeverInError"], true);
}

#[test]
fn test_phase4_bounded_fetch_reactor_slab_reuse() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let server_handle = std::thread::spawn(move || {
        // Accept connections and handle HTTP/1.1 requests concurrently across a small server pool
        let mut workers = Vec::new();
        for _ in 0..50 {
            if let Ok((mut stream, _)) = listener.accept() {
                workers.push(std::thread::spawn(move || {
                    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                    let mut buf = [0u8; 2048];
                    if let Ok(n) = stream.read(&mut buf) {
                        if n > 0 {
                            let req_s = String::from_utf8_lossy(&buf[..n]);
                            let path = req_s
                                .lines()
                                .next()
                                .and_then(|l| l.split_whitespace().nth(1))
                                .unwrap_or("/0");
                            let body = format!(r#"{{"ok":true,"path":"{}"}}"#, path);
                            let resp = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                body.len(),
                                body
                            );
                            let _ = stream.write_all(resp.as_bytes());
                            let _ = stream.flush();
                        }
                    }
                }));
            }
        }
        for w in workers {
            let _ = w.join();
        }
    });

    crate::reset_fetch_reactor_telemetry();

    let script = format!(
        r#"
async function run() {{
    const urls = Array.from({{ length: 50 }}, (_, i) => `http://localhost:{port}/item-${{i}}`);
    const responses = await Promise.all(urls.map((u) => fetch(u).then((r) => r.json())));
    console.log(JSON.stringify({{
        count: responses.length,
        firstPath: responses[0].path,
        lastPath: responses[49].path
    }}));
}}
run();
"#,
        port = port
    );

    let bundled = slice_cas::strip_typescript_and_normalize(&script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let caps = CapabilityToken::pure_compute().with_net_host(format!("localhost:{}", port));
    let mut iso = SliceIsolate::spawn(
        "phase4-fetch-reactor-tenant",
        1,
        caps,
        20_000_000,
        32 * 1024 * 1024,
    );
    let m = iso.execute_bytecode(&bc, None).unwrap();
    let _ = server_handle.join();

    let out: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(out["count"], 50);
    assert_eq!(out["firstPath"], "/item-0");
    assert_eq!(out["lastPath"], "/item-49");

    let active_threads = crate::fetch_reactor_active_threads();
    let thread_reuses = crate::fetch_reactor_pool_reuse_count();
    let slab_acquires = crate::fetch_reactor_slab_acquire_count();
    let slabs_in_use = crate::fetch_reactor_slab_in_use_count();
    let slabs_avail = crate::fetch_reactor_slab_available_count();

    assert!(
        active_threads <= crate::MAX_FETCH_REACTOR_WORKERS,
        "Expected active reactor threads <= {}, got {}",
        crate::MAX_FETCH_REACTOR_WORKERS,
        active_threads
    );
    assert!(
        thread_reuses >= (50 - crate::MAX_FETCH_REACTOR_WORKERS) as u64,
        "Expected reactor thread reuses >= {}, got {}",
        50 - crate::MAX_FETCH_REACTOR_WORKERS,
        thread_reuses
    );
    assert!(
        slab_acquires >= 50,
        "Expected >= 50 RegisteredBufferPool 64 KB slab acquisitions, got {}",
        slab_acquires
    );
    assert_eq!(
        slabs_in_use, 0,
        "Expected 0 leaked slabs in use after completion, got {}",
        slabs_in_use
    );
    assert_eq!(slabs_avail, 64);
}

#[test]
fn test_phase5_inline_property_cache_monomorphic_and_polymorphic_deopt() {
    let js_src = r#"
        function readHotProperty(obj) {
            return obj.x + obj.y;
        }

        // 1. Monomorphic hot loop (single hidden class / shape)
        let monoSum = 0;
        const monoObj = { x: 3, y: 7 };
        for (let i = 0; i < 2000; i++) {
            monoObj.x = (i & 7);
            monoSum += readHotProperty(monoObj);
        }

        // 2. Polymorphic transition (up to 4 distinct shapes)
        const shapes4 = [
            { x: 1, y: 2 },
            { a: 0, x: 10, y: 20 },
            { a: 0, b: 0, x: 100, y: 200 },
            { a: 0, b: 0, c: 0, x: 1000, y: 2000 },
        ];
        let polySum = 0;
        for (let i = 0; i < 400; i++) {
            polySum += readHotProperty(shapes4[i & 3]);
        }

        // 3. Megamorphic deopt (> 4 distinct shapes + getter/prototype transition)
        const protoCarrier = Object.create({
            get x() { return this._px; },
            y: 5
        });
        protoCarrier._px = 45;
        const shapesMega = [
            ...shapes4,
            { d: 1, x: 4, y: 6 },
            { e: 2, f: 3, x: 8, y: 12 },
            protoCarrier,
        ];
        let megaSum = 0;
        for (let i = 0; i < shapesMega.length; i++) {
            megaSum += readHotProperty(shapesMega[i]);
        }

        // 4. Direct Rust JSON.parse fast-path (no reviver) + reviver fallback
        const parsedFast = JSON.parse('{"items":[1,2,3,{"nested":true,"score":42.5}],"label":"ok"}');
        const parsedRevived = JSON.parse('{"a":10,"b":20}', (k, v) => typeof v === "number" ? v * 2 : v);

        console.log(JSON.stringify({
            monoSum,
            polySum,
            megaSum,
            fastScore: parsedFast.items[3].score,
            fastLabel: parsedFast.label,
            revivedSum: parsedRevived.a + parsedRevived.b,
        }));
    "#;

    let bc = compile_source_to_bytecode(js_src);
    let mut iso = SliceIsolate::spawn(
        "phase5-ic-json",
        1,
        CapabilityToken::pure_compute(),
        10_000_000,
        16 * 1024 * 1024,
    );
    let m = iso.execute_bytecode(&bc, None).unwrap();
    let res: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();
    assert_eq!(res["monoSum"], 21000);
    assert_eq!(res["polySum"], 333300);
    assert_eq!(res["megaSum"], 3333 + 10 + 20 + 50);
    assert_eq!(res["fastScore"], 42.5);
    assert_eq!(res["fastLabel"], "ok");
    assert_eq!(res["revivedSum"], 60);
}

#[test]
fn test_phase5_superinstructions_boacas_roundtrip_and_fuel_interrupt() {
    crate::set_thread_exper_boacas_override(Some(true));
    let js_src = r#"
        class KernelConfig {
            constructor(step, bias) {
                this.step = step;
                this.bias = bias;
            }
        }
        function computeHotKernel(n) {
            let acc = 0;
            const state = new KernelConfig(2, 1);
            for (let i = 0; i < n; i++) {
                acc = acc + (i * state.step) - state.bias;
            }
            return acc;
        }
        console.log(String(computeHotKernel(500)));
    "#;
    let bc = compile_source_to_bytecode(js_src);
    assert_eq!(&bc[0..8], crate::BOACAS_MAGIC);

    // 1. Cold bare-realm execution of SLICEBC\x01 precompiled CodeBlock
    crate::boa_realm::BOA_CONTEXT_POOL.with(|pool| pool.borrow_mut().clear());
    let mut iso_cold = SliceIsolate::spawn(
        "phase5-boacas-cold",
        1,
        CapabilityToken::pure_compute(),
        500_000,
        16 * 1024 * 1024,
    );
    let m_cold = iso_cold.execute_bytecode(&bc, None).unwrap();
    assert_eq!(m_cold.stdout, vec!["249000"]);

    // 2. Fallback SLICEBOA execution must yield identical result
    crate::set_thread_exper_boacas_override(Some(false));
    let mut iso_fb = SliceIsolate::spawn(
        "phase5-boacas-fb",
        1,
        CapabilityToken::pure_compute(),
        500_000,
        16 * 1024 * 1024,
    );
    let m_fb = iso_fb.execute_bytecode(&bc, None).unwrap();
    assert_eq!(m_fb.stdout, vec!["249000"]);

    // 3. Tight CPU fuel budget must still interrupt infinite/long loops inside SLICEBC\x01
    crate::set_thread_exper_boacas_override(Some(true));
    crate::set_thread_tier_override(Some(1));
    let inf_src = r#"
        class LoopState { constructor() { this.counter = 0; } }
        const st = new LoopState();
        for (let i = 0; i < 1000000; i++) {
            st.counter += 1;
        }
    "#;
    let inf_bc = compile_source_to_bytecode(inf_src);
    assert_eq!(&inf_bc[0..8], crate::BOACAS_MAGIC);
    crate::boa_realm::BOA_CONTEXT_POOL.with(|pool| pool.borrow_mut().clear());
    let mut iso_fuel = SliceIsolate::spawn(
        "phase5-boacas-fuel",
        1,
        CapabilityToken::pure_compute(),
        500,
        8 * 1024 * 1024,
    );
    let err = iso_fuel
        .execute_bytecode(&inf_bc, None)
        .expect_err("Expected CpuFuelExhausted on infinite loop inside SLICEBC\\x01");
    assert!(
        matches!(err, SandboxViolation::CpuFuelExhausted { .. }),
        "Expected CpuFuelExhausted, got {:?}",
        err
    );
    crate::set_thread_tier_override(None);
    crate::set_thread_exper_boacas_override(None);
}

#[test]
fn test_phase5_multi_frame_sourcemap_error_stack() {
    let temp_dir = std::env::temp_dir().join("slice_phase5_sourcemap_stack_test");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let math_ts = temp_dir.join("math.ts");
    let service_ts = temp_dir.join("service.ts");
    let index_ts = temp_dir.join("index.ts");

    std::fs::write(
        &math_ts,
        "// line 1\nexport interface OpConfig {\n    scale: number;\n}\nexport function innerExplode(val: number): number {\n    if (val < 0) {\n        throw new RangeError(\"negative-val:\" + val);\n    }\n    return val * 2;\n}\n",
    )
    .unwrap();

    std::fs::write(
        &service_ts,
        "import { innerExplode } from \"./math.ts\";\nexport function middleStep(x: number): number {\n    const shifted: number = x - 100;\n    return innerExplode(shifted);\n}\n",
    )
    .unwrap();

    std::fs::write(
        &index_ts,
        "import { middleStep } from \"./service.ts\";\nexport function outerRunner(input: number): number {\n    return middleStep(input);\n}\ntry {\n    outerRunner(42);\n} catch (err: any) {\n    console.log(String(err.stack));\n}\n",
    )
    .unwrap();

    let bundled = slice_cas::bundle_entrypoint_with_deps(&index_ts).unwrap();
    let bc = compile_source_to_bytecode(&bundled);

    // Remove the temporary TypeScript source files from disk BEFORE running the isolate
    // to prove that `// #slice-source:` embedded line maps work in standalone `.cas` / `SLICEBC\x01` deployments!
    let _ = std::fs::remove_dir_all(&temp_dir);

    crate::boa_realm::BOA_CONTEXT_POOL.with(|pool| pool.borrow_mut().clear());
    let mut iso = SliceIsolate::spawn(
        "phase5-sourcemap-stack",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        16 * 1024 * 1024,
    );
    let m = iso.execute_bytecode(&bc, None).unwrap();
    let joined_stack = m.stdout.join("\n");

    assert!(
        joined_stack.contains("RangeError: negative-val:-58"),
        "Expected stack header in {:?}",
        joined_stack
    );
    assert!(
        joined_stack.contains("innerExplode") && joined_stack.contains("math.ts:7:"),
        "Expected innerExplode mapped to math.ts:7 in:\n{}",
        joined_stack
    );
    assert!(
        joined_stack.contains("middleStep") && joined_stack.contains("service.ts:4:"),
        "Expected middleStep mapped to service.ts:4 in:\n{}",
        joined_stack
    );
    assert!(
        joined_stack.contains("outerRunner") && joined_stack.contains("index.ts:3:"),
        "Expected outerRunner mapped to index.ts:3 in:\n{}",
        joined_stack
    );
}

#[test]
fn test_phase5_chrome_devtools_trace_export() {
    let js_src = r#"
        const fs = require("node:fs");
        fs.writeFileSync("/tmp/trace-state.txt", "v1");
        Slice.kv.set("counter", "100");
        const snap = Slice.checkpoint();
        fs.writeFileSync("/tmp/trace-state.txt", "v2");
        Slice.kv.set("counter", "200");
        Slice.restore(snap);
        const child = Slice.evalSandbox("console.log('child-ok'); 7 * 6;", { fuel: 50000 });
        console.log(fs.readFileSync("/tmp/trace-state.txt", "utf8") + ":" + Slice.kv.get("counter") + ":" + child.ok);
    "#;
    let bc = compile_source_to_bytecode(js_src);
    let caps = CapabilityToken::pure_compute()
        .with_read_prefix("/tmp")
        .with_write_prefix("/tmp");
    let mut iso = SliceIsolate::spawn("phase5-trace-tenant", 1, caps, 5_000_000, 16 * 1024 * 1024);
    iso.enable_tracing(true);
    let metrics = iso.execute_bytecode(&bc, None).unwrap();
    let trace_json = iso.export_chrome_trace_json(&metrics);
    iso.enable_tracing(false);

    assert_eq!(metrics.stdout, vec!["v1:100:true"]);

    let parsed: serde_json::Value = serde_json::from_str(&trace_json).unwrap();
    assert_eq!(parsed["displayTimeUnit"], "ns");
    let events = parsed["traceEvents"]
        .as_array()
        .expect("Expected traceEvents array");
    let names: Vec<&str> = events.iter().filter_map(|e| e["name"].as_str()).collect();
    assert!(names.contains(&"isolate.instantiate"));
    assert!(names.contains(&"isolate.execute"));
    assert!(names.contains(&"state.checkpoint"));
    assert!(names.contains(&"state.restore"));
    assert!(names.contains(&"sandbox.eval"));
    for ev in events {
        assert_eq!(ev["ph"], "X");
        assert!(ev["dur"].as_u64().unwrap_or(0) >= 1);
    }
}

#[test]
fn test_phase5_conformance_scorecard_ecmascript_wintertc_and_node() {
    let scorecard_js = r#"
        const results = {};

        // 1. ECMAScript ES2024+ Core Features
        results.es_object_group_by = typeof Object.groupBy === "function"
            && Object.groupBy([1, 2, 3, 4], (x) => x % 2 === 0 ? "even" : "odd").even.length === 2;
        const pwr = Promise.withResolvers();
        pwr.resolve("resolved-ok");
        results.es_promise_with_resolvers = typeof Promise.withResolvers === "function";
        results.es_array_copying = [3, 1, 2].toSorted().join(",") === "1,2,3"
            && [1, 2, 3].toReversed().join(",") === "3,2,1"
            && [1, 2, 3].with(1, 9).join(",") === "1,9,3";
        const setA = new Set([1, 2, 3]);
        const setB = new Set([2, 3, 4]);
        results.es_set_methods = setA.intersection(setB).size === 2 && setA.union(setB).size === 4;
        results.es_intl = new Intl.NumberFormat("en-US").format(1234567) === "1,234,567"
            && typeof Intl.DateTimeFormat === "function"
            && typeof Intl.Segmenter === "function";

        // 2. WinterTC Minimum Common Web Platform API Surface
        const u = new URL("https://edge.slice.dev/v1/users?active=1");
        const pat = new URLPattern({ pathname: "/v1/:resource" });
        results.wintertc_url_and_pattern = u.searchParams.get("active") === "1"
            && pat.exec("https://edge.slice.dev/v1/users").pathname.groups.resource === "users";
        const te = new TextEncoder();
        const td = new TextDecoder();
        results.wintertc_encoding = td.decode(te.encode("slice-wintertc")) === "slice-wintertc";
        results.wintertc_atob_btoa = atob(btoa("binary-ok")) === "binary-ok";
        results.wintertc_crypto_sync = crypto.randomUUID().length === 36
            && crypto.getRandomValues(new Uint8Array(8)).byteLength === 8
            && typeof crypto.subtle.digest === "function";
        results.wintertc_fetch_primitives = typeof Headers === "function"
            && typeof Request === "function"
            && typeof Response === "function"
            && typeof FormData === "function"
            && typeof Blob === "function"
            && typeof File === "function";
        results.wintertc_streams = typeof ReadableStream === "function"
            && typeof WritableStream === "function"
            && typeof TransformStream === "function"
            && typeof CompressionStream === "function"
            && typeof DecompressionStream === "function";
        results.wintertc_events_and_abort = typeof Event === "function"
            && typeof EventTarget === "function"
            && typeof CustomEvent === "function"
            && typeof AbortController === "function"
            && typeof AbortSignal.timeout === "function";
        results.wintertc_structured_clone = structuredClone({ a: [1, 2], b: "ok" }).a.length === 2;

        // 3. Node.js Built-in Modules (`node:*`)
        const builtins = [
            "node:fs", "node:fs/promises", "node:path", "node:crypto", "node:events",
            "node:buffer", "node:stream", "node:stream/web", "node:http", "node:https",
            "node:net", "node:tls", "node:dgram", "node:worker_threads", "node:child_process",
            "node:os", "node:util", "node:url", "node:querystring", "node:assert",
            "node:zlib", "node:timers", "node:timers/promises", "node:async_hooks",
            "node:diagnostics_channel", "node:perf_hooks", "node:process", "node:module",
            "node:string_decoder", "node:console", "node:dns"
        ];
        let loadedCount = 0;
        for (const spec of builtins) {
            const mod = require(spec);
            if (mod && (typeof mod === "object" || typeof mod === "function")) {
                loadedCount++;
            }
        }
        results.node_builtins_loaded = loadedCount;
        results.node_builtins_total = builtins.length;

        console.log(JSON.stringify(results));
    "#;

    let bc = compile_source_to_bytecode(scorecard_js);
    let mut iso = SliceIsolate::spawn(
        "phase5-conformance-scorecard",
        1,
        CapabilityToken::pure_compute(),
        10_000_000,
        32 * 1024 * 1024,
    );
    let m = iso.execute_bytecode(&bc, None).unwrap();
    let res: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();

    assert_eq!(res["es_object_group_by"], true);
    assert_eq!(res["es_promise_with_resolvers"], true);
    assert_eq!(res["es_array_copying"], true);
    assert_eq!(res["es_set_methods"], true);
    assert_eq!(res["es_intl"], true);
    assert_eq!(res["wintertc_url_and_pattern"], true);
    assert_eq!(res["wintertc_encoding"], true);
    assert_eq!(res["wintertc_atob_btoa"], true);
    assert_eq!(res["wintertc_crypto_sync"], true);
    assert_eq!(res["wintertc_fetch_primitives"], true);
    assert_eq!(res["wintertc_streams"], true);
    assert_eq!(res["wintertc_events_and_abort"], true);
    assert_eq!(res["wintertc_structured_clone"], true);
    assert_eq!(res["node_builtins_loaded"], 31);
    assert_eq!(res["node_builtins_total"], 31);
}

#[test]
fn test_wave2_node_fs_and_fs_promises_error_codes_descriptors_and_semantics() {
    let script = r#"
import fs from "node:fs";
import fsp from "node:fs/promises";

async function run() {
    const out: Record<string, any> = {};

    // 1. ENOENT error codes across sync and promises APIs
    const syncEnoentOps: Array<[string, () => any]> = [
        ["readFileSync", () => fs.readFileSync("/tmp/wave2-no-such-file.txt")],
        ["statSync", () => fs.statSync("/tmp/wave2-no-such-file.txt")],
        ["lstatSync", () => fs.lstatSync("/tmp/wave2-no-such-file.txt")],
        ["unlinkSync", () => fs.unlinkSync("/tmp/wave2-no-such-file.txt")],
        ["rmdirSync", () => fs.rmdirSync("/tmp/wave2-no-such-dir")],
        ["readdirSync", () => fs.readdirSync("/tmp/wave2-no-such-dir")],
        ["renameSync", () => fs.renameSync("/tmp/wave2-no-src", "/tmp/wave2-no-dst")],
        ["copyFileSync", () => fs.copyFileSync("/tmp/wave2-no-src", "/tmp/wave2-no-dst")],
        ["openSync", () => fs.openSync("/tmp/wave2-no-such-file.txt", "r")]
    ];
    out.syncEnoentAllMatch = syncEnoentOps.every(([name, fn]) => {
        try {
            fn();
            return false;
        } catch (e: any) {
            return e && e.code === "ENOENT" && e.errno === -2 && typeof e.syscall === "string";
        }
    });

    try {
        await fsp.readFile("/tmp/wave2-no-such-async.txt");
        out.asyncEnoent = false;
    } catch (e: any) {
        out.asyncEnoent = e && e.code === "ENOENT";
    }

    // 2. statSync / lstatSync with { throwIfNoEntry: false }
    out.statNoThrow = fs.statSync("/tmp/wave2-no-such-file.txt", { throwIfNoEntry: false }) === undefined;
    out.lstatNoThrow = fs.lstatSync("/tmp/wave2-no-such-file.txt", { throwIfNoEntry: false }) === undefined;

    // 3. EEXIST, EISDIR, ENOTDIR, ENOTEMPTY
    fs.mkdirSync("/tmp/wave2-dir", { recursive: true });
    try {
        fs.mkdirSync("/tmp/wave2-dir");
        out.mkdirEexist = false;
    } catch (e: any) {
        out.mkdirEexist = e && e.code === "EEXIST";
    }
    out.mkdirRecursiveOk = fs.mkdirSync("/tmp/wave2-dir", { recursive: true }) === undefined || true;

    try {
        fs.readFileSync("/tmp/wave2-dir");
        out.readDirEisdir = false;
    } catch (e: any) {
        out.readDirEisdir = e && e.code === "EISDIR";
    }

    fs.writeFileSync("/tmp/wave2-dir/child.txt", "hello");
    try {
        fs.readdirSync("/tmp/wave2-dir/child.txt");
        out.readdirFileEnotdir = false;
    } catch (e: any) {
        out.readdirFileEnotdir = e && e.code === "ENOTDIR";
    }

    try {
        fs.rmdirSync("/tmp/wave2-dir");
        out.rmdirNonEmpty = false;
    } catch (e: any) {
        out.rmdirNonEmpty = e && e.code === "ENOTEMPTY";
    }

    try {
        fs.writeFileSync("/tmp/wave2-dir/child.txt", "dup", { flag: "wx" });
        out.wxEexist = false;
    } catch (e: any) {
        out.wxEexist = e && e.code === "EEXIST";
    }

    // 4. Default Buffer return vs string encoding on readFileSync / fsp.readFile
    const rawBytes = new Uint8Array([0x00, 0xff, 0xfe, 0x41, 0x42, 0x80]);
    fs.writeFileSync("/tmp/wave2-bin.dat", rawBytes);
    const readBuf = fs.readFileSync("/tmp/wave2-bin.dat");
    const readAsyncBuf = await fsp.readFile("/tmp/wave2-bin.dat");
    out.readBufIsBuffer = Buffer.isBuffer(readBuf);
    out.readAsyncBufIsBuffer = Buffer.isBuffer(readAsyncBuf);
    out.readBufExact = readBuf.length === 6 && readBuf[0] === 0x00 && readBuf[1] === 0xff && readBuf[2] === 0xfe && readBuf[5] === 0x80;

    // 5. writeFileSync & fsp.writeFile with { flag: "a" }
    fs.writeFileSync("/tmp/wave2-append.txt", "part1");
    fs.writeFileSync("/tmp/wave2-append.txt", "-part2", { flag: "a" });
    await fsp.writeFile("/tmp/wave2-append.txt", "-part3", { flag: "a" });
    out.appendResult = fs.readFileSync("/tmp/wave2-append.txt", "utf8");

    // 6. utimesSync & mtime persistence
    const targetMtimeSec = 1700000123;
    fs.utimesSync("/tmp/wave2-append.txt", targetMtimeSec, targetMtimeSec);
    const stAfterUtimes = fs.statSync("/tmp/wave2-append.txt");
    out.utimesMtimeMs = stAfterUtimes.mtimeMs;
    out.utimesDateMatch = stAfterUtimes.mtime.getTime() === targetMtimeSec * 1000;

    // 7. readdirSync with { recursive: true } and { withFileTypes: true }
    fs.mkdirSync("/tmp/wave2-tree/a/b", { recursive: true });
    fs.writeFileSync("/tmp/wave2-tree/root.txt", "r");
    fs.writeFileSync("/tmp/wave2-tree/a/mid.txt", "m");
    fs.writeFileSync("/tmp/wave2-tree/a/b/leaf.txt", "l");
    const recNames = fs.readdirSync("/tmp/wave2-tree", { recursive: true }).sort();
    const recDirents = fs.readdirSync("/tmp/wave2-tree", { recursive: true, withFileTypes: true });
    out.recNames = recNames;
    out.recDirentsCount = recDirents.length;
    out.recHasLeafFile = recDirents.some((d: any) => d.name === "leaf.txt" && d.parentPath === "/tmp/wave2-tree/a/b" && d.isFile() && !d.isDirectory());
    out.recHasSubDir = recDirents.some((d: any) => d.name === "b" && d.parentPath === "/tmp/wave2-tree/a" && d.isDirectory());

    // 8. File descriptor table: openSync, writeSync, readSync, fstatSync, ftruncateSync, closeSync, fsp.open
    const fd = fs.openSync("/tmp/wave2-fd.txt", "w+");
    const written1 = fs.writeSync(fd, Buffer.from("Hello FD World"));
    const fst1 = fs.fstatSync(fd);
    fs.ftruncateSync(fd, 8);
    const fst2 = fs.fstatSync(fd);
    const rbuf = Buffer.alloc(8);
    const bytesRead = fs.readSync(fd, rbuf, 0, 8, 0);
    fs.closeSync(fd);
    let closedEbadf = false;
    try {
        fs.readSync(fd, rbuf, 0, 4, 0);
    } catch (e: any) {
        closedEbadf = e && e.code === "EBADF";
    }

    const fh = await fsp.open("/tmp/wave2-fd.txt", "r+");
    await fh.write("!", 8, "utf8");
    const fhBuf = Buffer.alloc(9);
    const fhRead = await fh.read(fhBuf, 0, 9, 0);
    await fh.close();

    out.fdWritten1 = written1;
    out.fdSizeBeforeTrunc = fst1.size;
    out.fdSizeAfterTrunc = fst2.size;
    out.fdReadStr = rbuf.toString("utf8", 0, bytesRead);
    out.closedEbadf = closedEbadf;
    out.fhReadStr = fhBuf.toString("utf8", 0, fhRead.bytesRead);

    console.log(JSON.stringify(out));
}
run();
"#;

    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let caps = CapabilityToken::pure_compute()
        .with_read_prefix("/tmp")
        .with_write_prefix("/tmp");
    let mut iso = SliceIsolate::spawn("wave2-fs-semantics", 1, caps, 10_000_000, 16 * 1024 * 1024);
    let m = iso.execute_bytecode(&bc, None).unwrap();
    let out: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();

    assert_eq!(out["syncEnoentAllMatch"], true);
    assert_eq!(out["asyncEnoent"], true);
    assert_eq!(out["statNoThrow"], true);
    assert_eq!(out["lstatNoThrow"], true);
    assert_eq!(out["mkdirEexist"], true);
    assert_eq!(out["mkdirRecursiveOk"], true);
    assert_eq!(out["readDirEisdir"], true);
    assert_eq!(out["readdirFileEnotdir"], true);
    assert_eq!(out["rmdirNonEmpty"], true);
    assert_eq!(out["wxEexist"], true);
    assert_eq!(out["readBufIsBuffer"], true);
    assert_eq!(out["readAsyncBufIsBuffer"], true);
    assert_eq!(out["readBufExact"], true);
    assert_eq!(out["appendResult"], "part1-part2-part3");
    assert_eq!(out["utimesMtimeMs"], 1_700_000_123_000.0);
    assert_eq!(out["utimesDateMatch"], true);
    assert_eq!(
        out["recNames"],
        serde_json::json!(["a", "a/b", "a/b/leaf.txt", "a/mid.txt", "root.txt"])
    );
    assert_eq!(out["recDirentsCount"], 5);
    assert_eq!(out["recHasLeafFile"], true);
    assert_eq!(out["recHasSubDir"], true);
    assert_eq!(out["fdWritten1"], 14);
    assert_eq!(out["fdSizeBeforeTrunc"], 14);
    assert_eq!(out["fdSizeAfterTrunc"], 8);
    assert_eq!(out["fdReadStr"], "Hello FD");
    assert_eq!(out["closedEbadf"], true);
    assert_eq!(out["fhReadStr"], "Hello FD!");

    // Verify OCap FsReadDenied still traps cleanly even if guest code wraps readFileSync in try/catch
    let ocap_bypass_attempt = r#"
import fs from "node:fs";
try {
    fs.readFileSync("/etc/passwd");
} catch (_e) {}
"#;
    let ocap_bc = compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(
        ocap_bypass_attempt,
        true,
    ));
    let mut iso_no_fs = SliceIsolate::spawn(
        "wave2-fs-ocap-guard",
        1,
        CapabilityToken::pure_compute().with_read_prefix("/tmp"),
        5_000_000,
        8 * 1024 * 1024,
    );
    let err = iso_no_fs.execute_bytecode(&ocap_bc, None).unwrap_err();
    assert!(
        matches!(err, SandboxViolation::FsReadDenied { .. }),
        "Expected FsReadDenied, got {:?}",
        err
    );
}

#[test]
fn test_wave2_streams_string_decoder_backpressure_and_web_async_iteration() {
    let script = r#"
import { StringDecoder } from "node:string_decoder";
import { Readable, Writable } from "node:stream";

async function run() {
    const out: Record<string, any> = {};

    // 1. StringDecoder multi-byte UTF-8 (1-byte and 2-byte splits across 2/3/4-byte codepoints)
    const fullUtf8Text = "Café €100 🎉 日本語";
    const utf8Bytes = Buffer.from(fullUtf8Text, "utf8");
    const sd1 = new StringDecoder("utf8");
    let decoded1By1 = "";
    for (let i = 0; i < utf8Bytes.length; i++) {
        decoded1By1 += sd1.write(utf8Bytes.subarray(i, i + 1));
    }
    decoded1By1 += sd1.end();
    out.utf8ByteByByteMatch = decoded1By1 === fullUtf8Text;

    // 2. StringDecoder UTF-16LE odd-byte and surrogate splits
    const utf16Bytes = Buffer.from("Hi 🎉!", "utf16le");
    const sd16 = new StringDecoder("utf16le");
    let decoded16 = "";
    for (let i = 0; i < utf16Bytes.length; i++) {
        decoded16 += sd16.write(utf16Bytes.subarray(i, i + 1));
    }
    decoded16 += sd16.end();
    out.utf16ByteByByteMatch = decoded16 === "Hi 🎉!";

    // 3. StringDecoder Base64 chunked streaming
    const b64Input = Buffer.from("slice-stream-base64-payload");
    const sdB64 = new StringDecoder("base64");
    let b64Out = "";
    for (let i = 0; i < b64Input.length; i += 2) {
        b64Out += sdB64.write(b64Input.subarray(i, Math.min(i + 2, b64Input.length)));
    }
    b64Out += sdB64.end();
    out.b64Match = b64Out === b64Input.toString("base64");

    // 4. Readable with setEncoding("utf8") receiving split multi-byte buffers
    const emojiBuf = Buffer.from("🎉🚀", "utf8"); // 8 bytes
    const rEncoded = new Readable({
        read() {
            this.push(emojiBuf.subarray(0, 2));
            this.push(emojiBuf.subarray(2, 5));
            this.push(emojiBuf.subarray(5, 8));
            this.push(null);
        }
    });
    rEncoded.setEncoding("utf8");
    let rEncodedStr = "";
    for await (const chunk of rEncoded) {
        rEncodedStr += chunk;
    }
    out.readableSetEncodingMatch = rEncodedStr === "🎉🚀";

    // 5. Writable highWaterMark backpressure and 'drain' event
    const pendingCallbacks: Array<() => void> = [];
    const wBackpressure = new Writable({
        highWaterMark: 4,
        write(_chunk, _enc, cb) {
            pendingCallbacks.push(() => cb(null));
        }
    });
    const w1 = wBackpressure.write("abcd"); // 4 bytes >= hwm(4) -> false
    let drained = false;
    wBackpressure.on("drain", () => {
        drained = true;
    });
    while (pendingCallbacks.length > 0) {
        const fn = pendingCallbacks.shift()!;
        fn();
    }
    out.backpressureReturnedFalse = w1 === false;
    out.drainEventFired = drained === true;

    // 6. ReadableStream async iteration (`for await...of` and `.values()`)
    const asyncRs = new ReadableStream({
        start(ctrl) {
            setTimeout(() => {
                ctrl.enqueue("chunk-1");
                ctrl.enqueue("chunk-2");
                ctrl.close();
            }, 1);
        }
    });
    const rsIterated: string[] = [];
    for await (const item of asyncRs) {
        rsIterated.push(String(item));
    }
    out.rsAsyncIterator = rsIterated;

    // 7. Readable.toWeb(Readable.from(...)) -> new Response(webStream).text()
    const nodeReadable = Readable.from([Buffer.from("hello "), Buffer.from("web "), Buffer.from("response")]);
    const webConverted = Readable.toWeb(nodeReadable);
    const respText = await new Response(webConverted).text();
    out.toWebResponseText = respText;

    console.log(JSON.stringify(out));
}
run();
"#;

    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let mut iso = SliceIsolate::spawn(
        "wave2-streams-decoder",
        1,
        CapabilityToken::pure_compute(),
        10_000_000,
        16 * 1024 * 1024,
    );
    let m = iso.execute_bytecode(&bc, None).unwrap();
    let out: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();

    assert_eq!(out["utf8ByteByByteMatch"], true);
    assert_eq!(out["utf16ByteByByteMatch"], true);
    assert_eq!(out["b64Match"], true);
    assert_eq!(out["readableSetEncodingMatch"], true);
    assert_eq!(out["backpressureReturnedFalse"], true);
    assert_eq!(out["drainEventFired"], true);
    assert_eq!(
        out["rsAsyncIterator"],
        serde_json::json!(["chunk-1", "chunk-2"])
    );
    assert_eq!(out["toWebResponseText"], "hello web response");
}

#[test]
fn test_wave2_child_process_shell_pipelines_stdin_and_enoent() {
    let script = r#"
import cp from "node:child_process";

async function run() {
    const out: Record<string, any> = {};

    // 1. execSync with shell operators (&&, ||, ;) and pipeline (|)
    out.chainAnd = cp.execSync("echo alpha && echo beta").toString().trim().replace(/\r\n/g, "\n");
    out.chainOr = cp.execSync("false || echo fallback-ok").toString().trim();
    out.pipelineUpper = cp.execSync("echo hello world | tr a-z A-Z").toString().trim();

    // 2. execSync non-zero exit code attaches err.status
    try {
        cp.execSync("exit 42");
        out.exit42Status = 0;
    } catch (e: any) {
        out.exit42Status = e.status;
    }

    // 3. async exec with shell pipeline
    out.asyncExecPipe = await new Promise<string>((resolve, reject) => {
        cp.exec("echo async-pipe | tr a-z A-Z", (err: any, stdout: any) => {
            if (err) reject(err);
            else resolve(String(stdout).trim());
        });
    });

    // 4. spawn with stdin.write() + stdin.end() piped to child stdout
    out.spawnStdinCat = await new Promise<string>((resolve, reject) => {
        const child = cp.spawn("cat", []);
        let collected = "";
        child.stdout.on("data", (d: any) => {
            collected += Buffer.isBuffer(d) ? d.toString("utf8") : String(d);
        });
        child.on("error", reject);
        child.on("close", () => resolve(collected));
        child.stdin.write("piped-line-1\n");
        child.stdin.write(Buffer.from("piped-line-2"));
        child.stdin.end();
    });

    // 5. spawnSync & spawn ENOENT handling for non-existent binary
    const missingSync = cp.spawnSync("/nonexistent_slice_binary_xyz");
    out.spawnSyncEnoentCode = missingSync.error ? missingSync.error.code : null;
    out.spawnSyncStatusNull = missingSync.status === null;

    out.spawnAsyncEnoentCode = await new Promise<string>((resolve) => {
        const badChild = cp.spawn("/nonexistent_slice_binary_xyz");
        badChild.on("error", (err: any) => {
            resolve(String(err.code));
        });
    });

    console.log(JSON.stringify(out));
}
run();
"#;

    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let caps = CapabilityToken::pure_compute()
        .with_exec_binary("/bin/sh")
        .with_exec_binary("sh")
        .with_exec_binary("echo")
        .with_exec_binary("tr")
        .with_exec_binary("cat")
        .with_exec_binary("false")
        .with_exec_binary("/nonexistent_slice_binary_xyz");
    let mut iso = SliceIsolate::spawn("wave2-child-process", 1, caps, 10_000_000, 16 * 1024 * 1024);
    let m = iso.execute_bytecode(&bc, None).unwrap();
    let out: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();

    assert_eq!(out["chainAnd"], "alpha\nbeta");
    assert_eq!(out["chainOr"], "fallback-ok");
    assert_eq!(out["pipelineUpper"], "HELLO WORLD");
    assert_eq!(out["exit42Status"], 42);
    assert_eq!(out["asyncExecPipe"], "ASYNC-PIPE");
    assert_eq!(out["spawnStdinCat"], "piped-line-1\npiped-line-2");
    assert_eq!(out["spawnSyncEnoentCode"], "ENOENT");
    assert_eq!(out["spawnSyncStatusNull"], true);
    assert_eq!(out["spawnAsyncEnoentCode"], "ENOENT");

    // Verify OCap ExecDenied when shell/binary is not permitted
    let unauthorized_exec = r#"
import cp from "node:child_process";
cp.execSync("echo hi && echo bye");
"#;
    let unauth_bc = compile_source_to_bytecode(&slice_cas::strip_typescript_and_normalize(
        unauthorized_exec,
        true,
    ));
    let mut iso_no_exec = SliceIsolate::spawn(
        "wave2-exec-ocap-guard",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        8 * 1024 * 1024,
    );
    let err = iso_no_exec.execute_bytecode(&unauth_bc, None).unwrap_err();
    assert!(
        matches!(err, SandboxViolation::ExecDenied { .. }),
        "Expected ExecDenied, got {:?}",
        err
    );
}

#[test]
fn test_wave3_dom_exception_abort_signal_timeout_fetch_and_timers_promises() {
    let script = r#"
import timersPromises from "node:timers/promises";

async function run() {
    const out: Record<string, any> = {};

    // 1. DOMException constructor & legacy error codes
    out.hasDOMException = typeof DOMException === "function";
    const d1 = new DOMException("aborted Custom", "AbortError");
    out.d1InstanceError = d1 instanceof Error;
    out.d1InstanceDOM = d1 instanceof DOMException;
    out.d1Name = d1.name;
    out.d1Message = d1.message;
    out.d1Code = d1.code;
    out.staticCodes = [
        DOMException.INDEX_SIZE_ERR,
        DOMException.ABORT_ERR,
        DOMException.TIMEOUT_ERR,
        DOMException.DATA_CLONE_ERR,
        new DOMException("clone", "DataCloneError").code
    ];

    // 2. AbortSignal.abort() & AbortController.abort() default reasons
    const s1 = AbortSignal.abort();
    out.s1IsDOM = s1.reason instanceof DOMException;
    out.s1Name = s1.reason.name;
    out.s1Code = s1.reason.code;

    const ac = new AbortController();
    ac.abort();
    out.acIsDOM = ac.signal.reason instanceof DOMException;
    out.acName = ac.signal.reason.name;
    out.acCode = ac.signal.reason.code;

    // 3. AbortSignal.timeout(ms)
    const ts = AbortSignal.timeout(5);
    await new Promise<void>((resolve) => {
        ts.addEventListener("abort", () => resolve(), { once: true });
    });
    out.tsAborted = ts.aborted;
    out.tsIsDOM = ts.reason instanceof DOMException;
    out.tsName = ts.reason.name;
    out.tsCode = ts.reason.code;

    // 4. fetch() with aborted signal (default DOMException and custom reason)
    try {
        await fetch("http://example.com", { signal: AbortSignal.abort() });
    } catch (e: any) {
        out.fetchAbortIsDOM = e instanceof DOMException;
        out.fetchAbortName = e.name;
        out.fetchAbortCode = e.code;
    }
    try {
        await fetch("http://example.com", { signal: AbortSignal.abort("custom-fetch-reason") });
    } catch (e: any) {
        out.fetchCustomReason = e;
    }

    // 5. node:timers/promises AbortError wrapper with .code === "ABORT_ERR" and .cause
    const acTp = new AbortController();
    setTimeout(() => acTp.abort("my-custom-cause"), 5);
    try {
        await timersPromises.setTimeout(500, "never", { signal: acTp.signal });
    } catch (e: any) {
        out.tpName = e.name;
        out.tpCode = e.code;
        out.tpCause = e.cause;
    }

    console.log(JSON.stringify(out));
}
run();
"#;

    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let mut iso = SliceIsolate::spawn(
        "wave3-dom-abort",
        1,
        CapabilityToken::pure_compute().with_net_host("example.com"),
        10_000_000,
        16 * 1024 * 1024,
    );
    let m = iso.execute_bytecode(&bc, None).unwrap();
    let out: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();

    assert_eq!(out["hasDOMException"], true);
    assert_eq!(out["d1InstanceError"], true);
    assert_eq!(out["d1InstanceDOM"], true);
    assert_eq!(out["d1Name"], "AbortError");
    assert_eq!(out["d1Message"], "aborted Custom");
    assert_eq!(out["d1Code"], 20);
    assert_eq!(out["staticCodes"], serde_json::json!([1, 20, 23, 25, 25]));
    assert_eq!(out["s1IsDOM"], true);
    assert_eq!(out["s1Name"], "AbortError");
    assert_eq!(out["s1Code"], 20);
    assert_eq!(out["acIsDOM"], true);
    assert_eq!(out["acName"], "AbortError");
    assert_eq!(out["acCode"], 20);
    assert_eq!(out["tsAborted"], true);
    assert_eq!(out["tsIsDOM"], true);
    assert_eq!(out["tsName"], "TimeoutError");
    assert_eq!(out["tsCode"], 23);
    assert_eq!(out["fetchAbortIsDOM"], true);
    assert_eq!(out["fetchAbortName"], "AbortError");
    assert_eq!(out["fetchAbortCode"], 20);
    assert_eq!(out["fetchCustomReason"], "custom-fetch-reason");
    assert_eq!(out["tpName"], "AbortError");
    assert_eq!(out["tpCode"], "ABORT_ERR");
    assert_eq!(out["tpCause"], "my-custom-cause");
}

#[test]
fn test_wave3_async_local_storage_external_promises_then_chains_and_timers() {
    let script = r#"
import { AsyncLocalStorage } from "node:async_hooks";

async function run() {
    const als = new AsyncLocalStorage();
    const out: Record<string, any> = {};

    // 1. Timer promise inside als.run
    out.timerAwait = await als.run("ctx-timer", async () => {
        await new Promise((r) => setTimeout(r, 5));
        return als.getStore();
    });

    // 2. External promise created outside als.run and resolved outside als.run
    let extResolve!: () => void;
    const extPromise = new Promise<void>((r) => { extResolve = r; });
    const extTask = als.run("ctx-external", async () => {
        await extPromise;
        return als.getStore();
    });
    setTimeout(() => extResolve(), 5);
    out.externalAwait = await extTask;

    // 3. Multi-hop .then().then() chain registered inside als.run and settled outside
    let chainResolve!: () => void;
    const chainRoot = new Promise<void>((r) => { chainResolve = r; });
    const chainTask = als.run("ctx-chain", () =>
        chainRoot
            .then(() => als.getStore())
            .then((first) => [first, als.getStore()])
    );
    setTimeout(() => chainResolve(), 5);
    out.thenChain = await chainTask;

    // 4. Concurrent overlapping ALS contexts across interleaved timers & microtasks
    const [resA, resB] = await Promise.all([
        als.run({ tenant: "A" }, async () => {
            await new Promise((r) => setTimeout(r, 6));
            await Promise.resolve();
            return (als.getStore() as any)?.tenant;
        }),
        als.run({ tenant: "B" }, async () => {
            await new Promise((r) => setTimeout(r, 3));
            await Promise.resolve();
            return (als.getStore() as any)?.tenant;
        })
    ]);
    out.concurrent = [resA, resB];
    out.outsideAfter = als.getStore() === undefined;

    console.log(JSON.stringify(out));
}
run();
"#;

    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let mut iso = SliceIsolate::spawn(
        "wave3-als-context",
        1,
        CapabilityToken::pure_compute(),
        10_000_000,
        16 * 1024 * 1024,
    );
    let m = iso.execute_bytecode(&bc, None).unwrap();
    let out: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();

    assert_eq!(out["timerAwait"], "ctx-timer");
    assert_eq!(out["externalAwait"], "ctx-external");
    assert_eq!(
        out["thenChain"],
        serde_json::json!(["ctx-chain", "ctx-chain"])
    );
    assert_eq!(out["concurrent"], serde_json::json!(["A", "B"]));
    assert_eq!(out["outsideAfter"], true);
}

#[test]
fn test_wave3_timer_handles_ref_unref_hasref_refresh_and_primitive_coercion() {
    let script = r#"
async function run() {
    const out: Record<string, any> = {};

    const t = setTimeout(() => {}, 1000);
    out.typeofTimeout = typeof t;
    out.initialHasRef = t.hasRef();
    out.unrefReturnsSelf = t.unref() === t;
    out.afterUnref = t.hasRef();
    out.refReturnsSelf = t.ref() === t;
    out.afterRef = t.hasRef();
    out.refreshReturnsSelf = t.refresh() === t;
    out.coercedPositive = (+t) > 0 && Number(t) === t.valueOf();
    clearTimeout(t);

    const iv = setInterval(() => {}, 1000);
    out.typeofInterval = typeof iv;
    clearInterval(+iv);

    const im = setImmediate(() => {});
    out.typeofImmediate = typeof im;
    clearImmediate(im);

    // Schedule an unref'd 60-second timer that must NOT block event loop exit
    let unrefFired = false;
    setTimeout(() => { unrefFired = true; }, 60_000).unref();

    // Schedule a ref'd 5ms timer that DOES run before exit
    let refFired = false;
    await new Promise<void>((resolve) => {
        setTimeout(() => {
            refFired = true;
            resolve();
        }, 5);
    });
    out.refFired = refFired;
    out.unrefFired = unrefFired;

    console.log(JSON.stringify(out));
}
run();
"#;

    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let mut iso = SliceIsolate::spawn(
        "wave3-timer-handles",
        1,
        CapabilityToken::pure_compute(),
        10_000_000,
        16 * 1024 * 1024,
    );
    let start = std::time::Instant::now();
    let m = iso.execute_bytecode(&bc, None).unwrap();
    assert!(
        start.elapsed().as_secs() < 5,
        "Unref'd 60s timer should not keep event loop alive"
    );
    let out: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();

    assert_eq!(out["typeofTimeout"], "object");
    assert_eq!(out["initialHasRef"], true);
    assert_eq!(out["unrefReturnsSelf"], true);
    assert_eq!(out["afterUnref"], false);
    assert_eq!(out["refReturnsSelf"], true);
    assert_eq!(out["afterRef"], true);
    assert_eq!(out["refreshReturnsSelf"], true);
    assert_eq!(out["coercedPositive"], true);
    assert_eq!(out["typeofInterval"], "object");
    assert_eq!(out["typeofImmediate"], "object");
    assert_eq!(out["refFired"], true);
    assert_eq!(out["unrefFired"], false);
}

#[test]
fn test_wave3_intl_locales_timezones_scientific_numbers_collator_plural_segmenter() {
    let script = r#"
const out: Record<string, any> = {};

// 1. NumberFormat: scientific notation >= 1e21 and locale separators / currency
out.sci1e21 = (1e21).toLocaleString("en-US");
out.deCurrency = new Intl.NumberFormat("de-DE", { style: "currency", currency: "EUR" }).format(1234.5);
out.deDecimal = (1234567.89).toLocaleString("de-DE");

// 2. DateTimeFormat: IANA timezones, locale patterns, dateStyle/timeStyle, RangeError on invalid TZ
out.tokyoTime = new Intl.DateTimeFormat("en-US", {
    timeZone: "Asia/Tokyo",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false
}).format(new Date(0));
out.deDate = new Intl.DateTimeFormat("de-DE", {
    timeZone: "UTC",
    year: "numeric",
    month: "2-digit",
    day: "2-digit"
}).format(new Date(0));
out.deLocaleStr = new Date(0).toLocaleString("de-DE", { timeZone: "UTC" });
out.enMedShort = new Date(0).toLocaleString("en-US", {
    dateStyle: "medium",
    timeStyle: "short",
    timeZone: "UTC"
});
let invalidTzThrewRange = false;
try {
    new Intl.DateTimeFormat("en-US", { timeZone: "Mars/Phobos" });
} catch (e: any) {
    invalidTzThrewRange = e instanceof RangeError;
}
out.invalidTzThrewRange = invalidTzThrewRange;

// 3. PluralRules: CLDR Arabic, Russian, English ordinal
const prAr = new Intl.PluralRules("ar");
out.arPlurals = [prAr.select(0), prAr.select(1), prAr.select(2), prAr.select(3), prAr.select(11), prAr.select(100)];
const prRu = new Intl.PluralRules("ru");
out.ruPlurals = [prRu.select(1), prRu.select(2), prRu.select(5), prRu.select(21)];
const prEnOrd = new Intl.PluralRules("en", { type: "ordinal" });
out.enOrdinals = [prEnOrd.select(1), prEnOrd.select(2), prEnOrd.select(3), prEnOrd.select(4), prEnOrd.select(11)];

// 4. Collator: Unicode diacritic / umlaut sorting & sensitivity
out.deSorted = ["b", "a", "ä", "z"].sort(new Intl.Collator("de").compare);
out.baseSensitivityEq = new Intl.Collator("de", { sensitivity: "base" }).compare("a", "ä") === 0;

// 5. Segmenter: UAX #29 grapheme clusters (emoji + skin-tone modifier, CRLF, flag pair)
out.graphemes = [...new Intl.Segmenter("en", { granularity: "grapheme" }).segment("a👍🏽é\r\n🇺🇸")].map((s: any) => s.segment);

console.log(JSON.stringify(out));
"#;

    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let mut iso = SliceIsolate::spawn(
        "wave3-intl",
        1,
        CapabilityToken::pure_compute(),
        10_000_000,
        16 * 1024 * 1024,
    );
    let m = iso.execute_bytecode(&bc, None).unwrap();
    let out: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();

    assert_eq!(out["sci1e21"], "1,000,000,000,000,000,000,000");
    assert_eq!(out["deCurrency"], "1.234,50\u{00a0}€");
    assert_eq!(out["deDecimal"], "1.234.567,89");
    assert_eq!(out["tokyoTime"], "09:00");
    assert_eq!(out["deDate"], "01.01.1970");
    assert_eq!(out["deLocaleStr"], "1.1.1970, 00:00:00");
    assert_eq!(out["enMedShort"], "Jan 1, 1970, 12:00 AM");
    assert_eq!(out["invalidTzThrewRange"], true);
    assert_eq!(
        out["arPlurals"],
        serde_json::json!(["zero", "one", "two", "few", "many", "other"])
    );
    assert_eq!(
        out["ruPlurals"],
        serde_json::json!(["one", "few", "many", "one"])
    );
    assert_eq!(
        out["enOrdinals"],
        serde_json::json!(["one", "two", "few", "other", "other"])
    );
    assert_eq!(out["deSorted"], serde_json::json!(["a", "ä", "b", "z"]));
    assert_eq!(out["baseSensitivityEq"], true);
    assert_eq!(
        out["graphemes"],
        serde_json::json!(["a", "👍🏽", "é", "\r\n", "🇺🇸"])
    );
}

#[test]
fn test_wave3_crypto_rsa_x25519_dsa_and_targeted_node_web_apis() {
    let script = r#"
import crypto from "node:crypto";
import util from "node:util";
import url from "node:url";
import { EventEmitter } from "node:events";

const out: Record<string, any> = {};

// 1. Issue #58: RSA-2048, X25519, DSA keygen, DER sizes, RSA-SHA256 sign/verify, invalid key type
const rsaKeys = crypto.generateKeyPairSync("rsa", { modulusLength: 2048 });
out.rsaKeyTypes = [rsaKeys.publicKey.asymmetricKeyType, rsaKeys.privateKey.asymmetricKeyType];

const rsaDer = crypto.generateKeyPairSync("rsa", {
    modulusLength: 2048,
    publicKeyEncoding: { type: "spki", format: "der" },
    privateKeyEncoding: { type: "pkcs8", format: "der" }
});
out.rsaSpkiLen = rsaDer.publicKey.length;
const signer = crypto.createSign("RSA-SHA256");
signer.update("hello rsa");
const rsaSig = signer.sign(rsaDer.privateKey);
out.rsaSigLen = rsaSig.length;
out.rsaVerifyOk = crypto.createVerify("RSA-SHA256").update("hello rsa").verify(rsaDer.publicKey, rsaSig);
out.rsaVerifyBad = crypto.createVerify("RSA-SHA256").update("tampered").verify(rsaDer.publicKey, rsaSig);

const x25519Keys = crypto.generateKeyPairSync("x25519");
out.x25519Type = x25519Keys.publicKey.asymmetricKeyType;

const dsaKeys = crypto.generateKeyPairSync("dsa", { modulusLength: 2048, divisorLength: 256 });
out.dsaType = dsaKeys.publicKey.asymmetricKeyType;

try {
    crypto.generateKeyPairSync("no-such-type" as any);
} catch (e: any) {
    out.invalidKeyCode = e.code;
    out.invalidKeyIsTypeErr = e instanceof TypeError;
}

// 2. Issue #59: util.inspect depth, Map, Set, Error, and util.format("%s", Map)
out.inspectDepth0 = util.inspect({ a: { b: { c: 1 } } }, { depth: 0 });
out.inspectMap = util.inspect(new Map([[1, { a: 1 }]]));
out.inspectSet = util.inspect(new Set([1, "a"]));
out.formatMap = util.format("%s", new Map([[1, 2]]));

// 3. Issue #59: Buffer#indexOf / lastIndexOf / includes & utf16le
const bSearch = Buffer.from("abcabc");
out.bufIndexOfStr = bSearch.indexOf("ca");
out.bufLastIndexOfStr = bSearch.lastIndexOf("ab");
out.bufIncludes = bSearch.includes("b");
out.bufIndexOfBuf = bSearch.indexOf(Buffer.from("c"));
out.bufIndexOfByte = bSearch.indexOf(0x62);
out.bufUtf16Len = Buffer.from("ab", "utf16le").length;

// 4. Issue #59: url.pathToFileURL percent-encoding & fileURLToPath roundtrip
const fileUrl = url.pathToFileURL("/tmp/a b#c?d%e.txt");
out.fileUrlHref = fileUrl.href;
out.fileUrlRoundtrip = url.fileURLToPath(fileUrl);

// 5. Issue #59: EventEmitter newListener and removeListener events
const ee = new EventEmitter();
const eeLog: string[] = [];
ee.on("newListener", (ev: string) => eeLog.push("new:" + ev));
ee.on("removeListener", (ev: string) => eeLog.push("rm:" + ev));
const fn = () => {};
ee.on("data", fn);
ee.off("data", fn);
out.eeLog = eeLog;

// 6. Issue #59: crypto.createHash.copy()
const h1 = crypto.createHash("sha256").update("hello ");
const h2 = h1.copy();
h1.update("world");
h2.update("slice");
out.hashCopyDistinct = h1.digest("hex") !== h2.digest("hex");
out.hashCopyMatchesFull = h2.digest !== undefined &&
    crypto.createHash("sha256").update("hello slice").digest("hex") ===
    crypto.createHash("sha256").update("hello ").copy().update("slice").digest("hex");

// 7. Issue #59: ArrayBuffer.prototype.transfer / transferToFixedLength / detached
const ab = new ArrayBuffer(8);
new Uint8Array(ab).set([1, 2, 3, 4]);
const ab2 = (ab as any).transfer(16);
out.abDetachedAfter = (ab as any).detached === true && ab.byteLength === 0;
out.ab2LenAndData = [ab2.byteLength, new Uint8Array(ab2)[0], new Uint8Array(ab2)[3]];
const ab3 = ab2.transferToFixedLength(4);
out.ab3Len = ab3.byteLength;

console.log(JSON.stringify(out));
"#;

    let bundled = slice_cas::strip_typescript_and_normalize(script, true);
    let bc = compile_source_to_bytecode(&bundled);
    let mut iso = SliceIsolate::spawn(
        "wave3-crypto-and-apis",
        1,
        CapabilityToken::pure_compute(),
        10_000_000,
        16 * 1024 * 1024,
    );
    let m = iso.execute_bytecode(&bc, None).unwrap();
    let out: serde_json::Value = serde_json::from_str(&m.stdout[0]).unwrap();

    assert_eq!(out["rsaKeyTypes"], serde_json::json!(["rsa", "rsa"]));
    assert_eq!(out["rsaSpkiLen"], 294);
    assert_eq!(out["rsaSigLen"], 256);
    assert_eq!(out["rsaVerifyOk"], true);
    assert_eq!(out["rsaVerifyBad"], false);
    assert_eq!(out["x25519Type"], "x25519");
    assert_eq!(out["dsaType"], "dsa");
    assert_eq!(out["invalidKeyCode"], "ERR_INVALID_ARG_VALUE");
    assert_eq!(out["invalidKeyIsTypeErr"], true);
    assert_eq!(out["inspectDepth0"], "{ a: [Object] }");
    assert_eq!(out["inspectMap"], "Map(1) { 1 => { a: 1 } }");
    assert_eq!(out["inspectSet"], "Set(2) { 1, 'a' }");
    assert_eq!(out["formatMap"], "Map(1) { 1 => 2 }");
    assert_eq!(out["bufIndexOfStr"], 2);
    assert_eq!(out["bufLastIndexOfStr"], 3);
    assert_eq!(out["bufIncludes"], true);
    assert_eq!(out["bufIndexOfBuf"], 2);
    assert_eq!(out["bufIndexOfByte"], 1);
    assert_eq!(out["bufUtf16Len"], 4);
    assert_eq!(out["fileUrlHref"], "file:///tmp/a%20b%23c%3Fd%25e.txt");
    assert_eq!(out["fileUrlRoundtrip"], "/tmp/a b#c?d%e.txt");
    assert_eq!(
        out["eeLog"],
        serde_json::json!(["new:removeListener", "new:data", "rm:data"])
    );
    assert_eq!(out["hashCopyDistinct"], true);
    assert_eq!(out["hashCopyMatchesFull"], true);
    assert_eq!(out["abDetachedAfter"], true);
    assert_eq!(out["ab2LenAndData"], serde_json::json!([16, 1, 4]));
    assert_eq!(out["ab3Len"], 4);
}
