import crypto from "node:crypto";
import cookie from "cookie";
import qs from "fast-querystring";
import type { WsRealtimeSessionSummary, WsTelemetryPacket } from "./types.ts";

const RFC6455_WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

export function computeRfc6455AcceptKey(secWebSocketKey: string): string {
    return crypto
        .createHash("sha1")
        .update(secWebSocketKey.trim() + RFC6455_WS_GUID)
        .digest("base64");
}

export function verifyRfc6455MaskingRoundtrip(
    payload: Uint8Array,
    maskKey: [number, number, number, number],
): boolean {
    const buf = new Uint8Array(payload.length);
    for (let i = 0; i < payload.length; i++) {
        buf[i] = payload[i] ^ maskKey[i & 3];
    }
    for (let i = 0; i < buf.length; i++) {
        buf[i] = buf[i] ^ maskKey[i & 3];
    }
    if (buf.length !== payload.length) {
        return false;
    }
    for (let i = 0; i < payload.length; i++) {
        if (buf[i] !== payload[i]) {
            return false;
        }
    }
    return true;
}

export async function runRealtimeWebSocketAndKeepAliveSession(
    baseHttpUrl: string,
    wsUrl: string,
    packets: WsTelemetryPacket[],
    binaryFrames: Uint8Array[],
): Promise<WsRealtimeSessionSummary> {
    // 1. Verify RFC 6455 test vector from Section 1.3 (`dGhlIHNhbXBsZSBub25jZQ==` -> `s3pPLMBiTxaQ9kYGzzhZRbK+xOo=`)
    const rfcVectorAccept = computeRfc6455AcceptKey("dGhlIHNhbXBsZSBub25jZQ==");
    const rfc6455AcceptVerified =
        rfcVectorAccept === "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=";

    // 2. Verify 4-byte XOR client-to-server masking roundtrip
    const sampleBytes = new Uint8Array([0x53, 0x4c, 0x49, 0x43, 0x45, 0x5f, 0x57, 0x53]);
    const maskedFrameRoundtripVerified = verifyRfc6455MaskingRoundtrip(
        sampleBytes,
        [0x37, 0xfa, 0x21, 0x3d],
    );

    // 3. Execute live RFC 6455 WebSocket session against `ws://127.0.0.1:18095/upstream/ws`
    const parsedWsUrl = new URL(wsUrl);
    const queryParams = qs.parse(parsedWsUrl.search.slice(1));
    const sessionCookie = cookie.serialize(
        "slice_ws_tenant",
        String(queryParams.tenant || "acme-realtime"),
        { httpOnly: true, path: "/" },
    );
    const parsedCookie = cookie.parse(sessionCookie);

    const wsOutcome = await new Promise<{
        handshakePath: string;
        textFramesSent: number;
        textAcksReceived: number;
        binaryFramesSent: number;
        binaryXorBytesVerified: number;
        closeCode: number;
        closeReason: string;
        transcript: string[];
    }>((resolve, reject) => {
        const ws = new WebSocket(wsUrl);
        ws.binaryType = "arraybuffer";

        let handshakePath = "";
        let textIdx = 0;
        let binIdx = 0;
        let textFramesSent = 0;
        let textAcksReceived = 0;
        let binaryFramesSent = 0;
        let binaryXorBytesVerified = 0;
        const transcript: string[] = [
            "cookie:" + String(parsedCookie.slice_ws_tenant || "none"),
        ];

        ws.onerror = (err) => {
            reject(new Error("WebSocket error: " + String(err)));
        };

        ws.onmessage = (ev) => {
            if (typeof ev.data === "string") {
                const msg = JSON.parse(ev.data) as {
                    type: string;
                    path?: string;
                    seq?: number;
                    tenant?: string;
                    digest?: string;
                    ok?: boolean;
                };
                if (msg.type === "handshake_ack") {
                    handshakePath = String(msg.path || "");
                    transcript.push("open:" + handshakePath);
                    if (packets.length > 0) {
                        textFramesSent++;
                        ws.send(JSON.stringify(packets[0]));
                    }
                } else if (msg.type === "telemetry_ack") {
                    textAcksReceived++;
                    transcript.push(
                        "ack:" +
                            String(msg.seq) +
                            ":" +
                            String(msg.tenant) +
                            ":" +
                            String(msg.digest).slice(0, 8),
                    );
                    textIdx++;
                    if (textIdx < packets.length) {
                        textFramesSent++;
                        ws.send(JSON.stringify(packets[textIdx]));
                    } else if (binaryFrames.length > 0) {
                        binaryFramesSent++;
                        ws.send(binaryFrames[0]);
                    } else {
                        ws.close(1000, "audit-complete");
                    }
                }
            } else if (ev.data instanceof ArrayBuffer) {
                const received = new Uint8Array(ev.data);
                const expectedSrc = binaryFrames[binIdx];
                let matchesXor = received.length === expectedSrc.length;
                for (let j = 0; j < received.length; j++) {
                    if (received[j] !== ((expectedSrc[j] ^ 0x5a) & 0xff)) {
                        matchesXor = false;
                        break;
                    }
                }
                if (matchesXor) {
                    binaryXorBytesVerified += received.length;
                }
                transcript.push(
                    "bin:" +
                        String(binIdx) +
                        ":" +
                        Array.from(received).join(","),
                );
                binIdx++;
                if (binIdx < binaryFrames.length) {
                    binaryFramesSent++;
                    ws.send(binaryFrames[binIdx]);
                } else {
                    ws.close(1000, "audit-complete");
                }
            }
        };

        ws.onclose = (ev) => {
            const closeCode = ev.code || 1000;
            const closeReason = ev.reason || "audit-complete";
            transcript.push("close:" + String(closeCode));
            resolve({
                handshakePath,
                textFramesSent,
                textAcksReceived,
                binaryFramesSent,
                binaryXorBytesVerified,
                closeCode,
                closeReason,
                transcript,
            });
        };
    });

    // 4. Perform a Keep-Alive HTTP subgraph check on the same upstream server (`127.0.0.1:18095`)
    const httpRes = await fetch(
        baseHttpUrl +
            "/upstream/subgraph/account?tenant=acme-realtime&account=acct-2100",
        {
            method: "GET",
            headers: {
                accept: "application/json",
                cookie: sessionCookie,
                "x-ws-audit-seq": String(wsOutcome.textAcksReceived),
            },
        },
    );
    const httpJson = (await httpRes.json()) as {
        accountId: string;
        tier: string;
        balanceCents: number;
    };
    wsOutcome.transcript.push(
        "http:" +
            String(httpRes.status) +
            ":" +
            httpJson.accountId +
            ":" +
            httpJson.tier,
    );

    const sessionTranscriptDigest = crypto
        .createHash("sha256")
        .update(wsOutcome.transcript.join("|"))
        .digest("hex")
        .slice(0, 32);

    return {
        handshakePath: wsOutcome.handshakePath,
        rfc6455AcceptVerified,
        maskedFrameRoundtripVerified,
        textFramesSent: wsOutcome.textFramesSent,
        textAcksReceived: wsOutcome.textAcksReceived,
        binaryFramesSent: wsOutcome.binaryFramesSent,
        binaryXorBytesVerified: wsOutcome.binaryXorBytesVerified,
        closeCode: wsOutcome.closeCode,
        closeReason: wsOutcome.closeReason,
        upstreamSubgraphStatus: httpRes.status,
        upstreamAccountTier: httpJson.tier,
        sessionTranscriptDigest,
    };
}

