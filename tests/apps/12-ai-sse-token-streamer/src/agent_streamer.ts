import mitt from "mitt";
import { formatSseFrame } from "./sse_codec.ts";
import type { AgentStreamRequest, SseFrame } from "./types.ts";

type StreamEvents = {
    frame: SseFrame;
};

export class AiSseTokenStreamer {
    public dlpRedactionsTriggered: number = 0;
    public totalTokensStreamed: number = 0;
    public totalFramesEmitted: number = 0;

    private containsProtectedSecret(text: string, secret: string): boolean {
        if (!secret) return false;
        const b64 = Buffer.from(secret).toString("base64");
        return text.indexOf(secret) !== -1 || text.indexOf(b64) !== -1;
    }

    public async streamCompletion(req: Request): Promise<Response> {
        const payload = (await req.json()) as AgentStreamRequest;
        const bus = mitt<StreamEvents>();
        const frames: SseFrame[] = [];

        bus.on("frame", (f: SseFrame) => {
            frames.push(f);
            this.totalFramesEmitted += 1;
        });

        bus.emit("frame", {
            id: payload.streamId + ":0",
            event: "meta",
            retry: 2000,
            data: { model: payload.model, tenant: payload.tenantId }
        });

        for (let i = 0; i < payload.promptTokens.length; i++) {
            const rawTok = payload.promptTokens[i];
            if (this.containsProtectedSecret(rawTok, payload.auditSecret)) {
                this.dlpRedactionsTriggered += 1;
                bus.emit("frame", {
                    id: payload.streamId + ":" + (i + 1),
                    event: "redacted",
                    data: { index: i, reason: "EGRESS_SECRET_REDACTED" }
                });
            } else {
                this.totalTokensStreamed += 1;
                bus.emit("frame", {
                    id: payload.streamId + ":" + (i + 1),
                    event: "token",
                    data: { index: i, delta: rawTok }
                });
            }
        }

        bus.emit("frame", {
            id: payload.streamId + ":done",
            event: "done",
            data: "[DONE]"
        });

        const stream = new ReadableStream({
            start(controller: any) {
                for (let j = 0; j < frames.length; j++) {
                    controller.enqueue(formatSseFrame(frames[j]));
                }
                controller.close();
            }
        });

        const reader = stream.getReader();
        let sseBody = "";
        while (true) {
            const step = await reader.read();
            if (step.done) break;
            sseBody += String(step.value);
        }
        reader.releaseLock();

        return new Response(sseBody, {
            status: 200,
            headers: {
                "content-type": "text/event-stream; charset=utf-8",
                "cache-control": "no-cache",
                "x-slice-tenant": payload.tenantId
            }
        });
    }
}

