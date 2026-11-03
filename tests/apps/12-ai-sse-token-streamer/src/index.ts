import crypto from "node:crypto";
import ms from "ms";
import { AiSseTokenStreamer } from "./agent_streamer.ts";
import { parseSseStream } from "./sse_codec.ts";
import type { AgentStreamRequest, SseBenchmarkSummary } from "./types.ts";

async function runSseBenchmark(): Promise<void> {
    const streamer = new AiSseTokenStreamer();
    const auditSecret = "sk_live_nexus_vault_9941";
    const b64Secret = Buffer.from(auditSecret).toString("base64");
    const heartbeatIntervalMs = ms("15s") as number;

    let totalSseBytes = 0;
    const digest = crypto.createHash("sha256");

    for (let i = 0; i < 40; i++) {
        const injectRawSecret = i % 8 === 3;
        const injectB64Secret = i % 8 === 6;
        const tokens = [
            "Analyzing ",
            "module ",
            "graph #" + i + ": ",
            injectRawSecret
                ? "leak=" + auditSecret
                : injectB64Secret
                ? "b64=" + b64Secret
                : "zero-stat ",
            "CAS ",
            "verified."
        ];

        const reqPayload: AgentStreamRequest = {
            streamId: "chatcmpl-" + (1000 + i),
            tenantId: i % 2 === 0 ? "tenant-ai-alpha" : "tenant-ai-beta",
            model: "slice-code-v2",
            promptTokens: tokens,
            auditSecret
        };

        const req = new Request("https://ai.slice.local/v1/chat/completions", {
            method: "POST",
            headers: {
                "content-type": "application/json",
                "accept": "text/event-stream"
            },
            body: JSON.stringify(reqPayload)
        });

        const res = await streamer.streamCompletion(req);
        const rawSse = await res.text();
        totalSseBytes += Buffer.byteLength(rawSse);

        const parsedEvents = parseSseStream(rawSse);
        digest.update(
            res.status +
                ":" +
                (res.headers.get("content-type") || "") +
                ":" +
                parsedEvents.length +
                ":" +
                parsedEvents[parsedEvents.length - 1].data +
                ";"
        );
    }

    const summary: SseBenchmarkSummary = {
        app: "12-ai-sse-token-streamer",
        libraries: ["mitt@3.0.1", "ms@2.1.3"],
        streamsCompleted: 40,
        totalFramesEmitted: streamer.totalFramesEmitted,
        totalTokensStreamed: streamer.totalTokensStreamed,
        dlpRedactionsTriggered: streamer.dlpRedactionsTriggered,
        totalSseBytes,
        heartbeatIntervalMs,
        streamAuditHash: digest.digest("hex").slice(0, 16)
    };

    console.log(JSON.stringify(summary));
}

runSseBenchmark();

