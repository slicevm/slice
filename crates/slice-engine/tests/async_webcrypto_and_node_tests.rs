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
        wall_elapsed < Duration::from_millis(250),
        "66-second deterministic timer suite should finish in < 250ms, took {:?}",
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
    assert!(
        fanout_elapsed < Duration::from_millis(220),
        "Expected 4x60ms parallel fan-out + 35ms timeout to complete in < 220ms, took {:?}",
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
    let mut iso1 = SliceIsolate::spawn(
        "phase-n4-tenant-1",
        1,
        CapabilityToken::pure_compute(),
        25_000_000,
        8 * 1024 * 1024,
    );
    iso1.set_deterministic_seed(Some(20261104));
    let m1 = iso1.execute_bytecode(&bc, None).unwrap();

    let mut iso2 = SliceIsolate::spawn(
        "phase-n4-tenant-2",
        2,
        CapabilityToken::pure_compute(),
        25_000_000,
        8 * 1024 * 1024,
    );
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
});
let exitCaught = false;
try {
    proc.exit(7);
} catch (e: any) {
    exitCaught = (e && e.code === "ERR_PROCESS_EXIT" && e.exitCode === 7);
}

proc.stdout.write("stdout-line-1\n");
proc.stderr.write("stderr-line-2\n");
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
    assert_eq!(out["exitCaught"], true);
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
    assert_eq!(
        out["words"],
        serde_json::json!(["Hello", "Slice", "2026"])
    );
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


