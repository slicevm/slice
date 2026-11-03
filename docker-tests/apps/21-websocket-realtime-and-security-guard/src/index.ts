import assert from "node:assert";
import crypto from "node:crypto";
import ms from "ms";
import { NetworkSecurityAuditor } from "./network_security_auditor.ts";
import { runRealtimeWebSocketAndKeepAliveSession } from "./realtime_ws_client.ts";
import type { NetworkSecurityCase, WsTelemetryPacket } from "./types.ts";

async function main(): Promise<void> {
    const baseHttpUrl = "http://127.0.0.1:18095";
    const wsUrl =
        "ws://127.0.0.1:18095/upstream/ws?tenant=acme-realtime&channel=sec-stream";
    const masterSecret = "sk_live_WAVE_B_WS_MASTER_SECRET_2026";
    const b64Secret = Buffer.from(masterSecret, "utf8").toString("base64");

    const auditor = new NetworkSecurityAuditor([masterSecret]);

    const securityCases: NetworkSecurityCase[] = [
        {
            caseId: "sec-01",
            category: "smuggling_cl_te",
            rawTargetUrl: "https://edge.acme.example.com/api/v1/ingest",
            headerName: "x-request-id",
            headerValue: "req-101",
            contentLengthHeader: "12",
            transferEncodingHeader: "chunked",
        },
        {
            caseId: "sec-02",
            category: "smuggling_dup_cl",
            rawTargetUrl: "https://edge.acme.example.com/api/v1/ingest",
            headerName: "x-request-id",
            headerValue: "req-102",
            contentLengthHeader: "16, 32",
        },
        {
            caseId: "sec-03",
            category: "crlf_header_injection",
            rawTargetUrl: "https://edge.acme.example.com/api/v1/ingest",
            headerName: "x-forwarded-for",
            headerValue: "203.0.113.5\r\nX-Injected-Admin: true",
        },
        {
            caseId: "sec-04",
            category: "crlf_header_injection",
            rawTargetUrl: "https://edge.acme.example.com/api/v1/ingest",
            headerName: "invalid header\x00name",
            headerValue: "clean-val",
        },
        {
            caseId: "sec-05",
            category: "chunked_overflow",
            rawTargetUrl: "https://edge.acme.example.com/api/v1/upload",
            headerName: "x-upload-stream",
            headerValue: "chunked-v1",
            chunkSizeHex: "ffffffffffffffff",
        },
        {
            caseId: "sec-06",
            category: "chunked_overflow",
            rawTargetUrl: "https://edge.acme.example.com/api/v1/upload",
            headerName: "x-upload-stream",
            headerValue: "chunked-v1",
            chunkSizeHex: "1a2g3z",
        },
        {
            caseId: "sec-07",
            category: "ssrf_noncanonical_ip",
            rawTargetUrl: "http://0177.0.0.1:8080/admin/keys",
            headerName: "accept",
            headerValue: "application/json",
        },
        {
            caseId: "sec-08",
            category: "ssrf_noncanonical_ip",
            rawTargetUrl: "http://2852039166/latest/meta-data/iam/",
            headerName: "accept",
            headerValue: "application/json",
        },
        {
            caseId: "sec-09",
            category: "ssrf_noncanonical_ip",
            rawTargetUrl: "http://0x7f.0.0.1:9000/internal",
            headerName: "accept",
            headerValue: "application/json",
        },
        {
            caseId: "sec-10",
            category: "ssrf_noncanonical_ip",
            rawTargetUrl: "http://100.64.12.8:8443/cgnat-mesh",
            headerName: "accept",
            headerValue: "application/json",
        },
        {
            caseId: "sec-11",
            category: "ws_frame_dlp_leak",
            rawTargetUrl: "wss://stream.acme.example.com/realtime",
            headerName: "sec-websocket-protocol",
            headerValue: "telemetry.v1",
            wsFramePayload: '{"leak":"' + masterSecret + '"}',
        },
        {
            caseId: "sec-12",
            category: "ws_frame_dlp_leak",
            rawTargetUrl: "wss://stream.acme.example.com/realtime",
            headerName: "sec-websocket-protocol",
            headerValue: "telemetry.v1",
            wsFramePayload: '{"leak_b64":"' + b64Secret + '"}',
        },
        {
            caseId: "sec-13",
            category: "clean_request",
            rawTargetUrl: "https://stream.acme.example.com/realtime",
            headerName: "x-trace-id",
            headerValue: "trace-wave-b-clean-01",
            chunkSizeHex: "400",
            wsFramePayload: '{"event":"heartbeat","ok":true}',
        },
        {
            caseId: "sec-14",
            category: "clean_request",
            rawTargetUrl: "https://api.acme.example.com/v2/metrics",
            headerName: "x-trace-id",
            headerValue: "trace-wave-b-clean-02",
            contentLengthHeader: "128, 128",
        },
    ];

    const auditSummary = auditor.evaluateSuite(securityCases);
    assert.strictEqual(auditSummary.totalCasesEvaluated, 14);
    assert.strictEqual(auditSummary.smugglingClTeBlocked, 1);
    assert.strictEqual(auditSummary.smugglingDupClBlocked, 1);
    assert.strictEqual(auditSummary.crlfHeaderInjectionBlocked, 2);
    assert.strictEqual(auditSummary.chunkedOverflowBlocked, 2);
    assert.strictEqual(auditSummary.ssrfNonCanonicalIpBlocked, 4);
    assert.strictEqual(auditSummary.wsFrameDlpBlocked, 2);
    assert.strictEqual(auditSummary.cleanRequestsAllowed, 2);

    const windowMs = Number(ms("15s"));
    const packets: WsTelemetryPacket[] = [];
    for (let i = 1; i <= 4; i++) {
        const metric = "edge.ws.latency.p" + (90 + i * 2);
        const value = 120 + i * 15;
        const digest = crypto
            .createHash("sha256")
            .update("acme-realtime:" + i + ":" + metric + ":" + value)
            .digest("hex")
            .slice(0, 16);
        packets.push({
            seq: i,
            tenant: "acme-realtime",
            channel: "sec-stream",
            metric,
            value,
            windowMs,
            digest,
        });
    }

    const binaryFrames: Uint8Array[] = [
        new Uint8Array([0x10, 0x20, 0x30, 0x40, 0x50, 0x60]),
        new Uint8Array([0xa1, 0xb2, 0xc3, 0xd4, 0xe5, 0xf6, 0x07, 0x18]),
    ];

    const wsSummary = await runRealtimeWebSocketAndKeepAliveSession(
        baseHttpUrl,
        wsUrl,
        packets,
        binaryFrames,
    );

    assert.strictEqual(wsSummary.handshakePath, "/upstream/ws");
    assert.strictEqual(wsSummary.rfc6455AcceptVerified, true);
    assert.strictEqual(wsSummary.maskedFrameRoundtripVerified, true);
    assert.strictEqual(wsSummary.textFramesSent, 4);
    assert.strictEqual(wsSummary.textAcksReceived, 4);
    assert.strictEqual(wsSummary.binaryFramesSent, 2);
    assert.strictEqual(wsSummary.binaryXorBytesVerified, 14);
    assert.strictEqual(wsSummary.closeCode, 1000);
    assert.strictEqual(wsSummary.upstreamSubgraphStatus, 200);
    assert.strictEqual(wsSummary.upstreamAccountTier, "enterprise");

    console.log(
        JSON.stringify({
            suite: "21-websocket-realtime-and-security-guard",
            auditSummary,
            wsSummary,
        }),
    );
}

main();

