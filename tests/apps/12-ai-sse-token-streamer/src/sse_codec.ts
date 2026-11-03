import type { SseFrame } from "./types.ts";

export function formatSseFrame(frame: SseFrame): string {
    let out = "";
    if (frame.id) {
        out += "id: " + frame.id + "\n";
    }
    if (frame.event) {
        out += "event: " + frame.event + "\n";
    }
    if (frame.retry !== undefined) {
        out += "retry: " + frame.retry + "\n";
    }
    const payload = typeof frame.data === "string" ? frame.data : JSON.stringify(frame.data);
    if (payload.indexOf("\n") === -1) {
        out += "data: " + payload + "\n\n";
    } else {
        const lines = payload.split("\n");
        for (let i = 0; i < lines.length; i++) {
            out += "data: " + lines[i] + "\n";
        }
        out += "\n";
    }
    return out;
}

export function parseSseStream(rawStream: string): Array<{ id: string; event: string; data: string }> {
    const blocks = rawStream.split("\n\n");
    const parsed: Array<{ id: string; event: string; data: string }> = [];
    for (let i = 0; i < blocks.length; i++) {
        const block = blocks[i].trim();
        if (!block) continue;
        const lines = block.split("\n");
        let id = "";
        let event = "message";
        const dataLines: string[] = [];
        for (let j = 0; j < lines.length; j++) {
            const line = lines[j];
            if (line.startsWith("id: ")) {
                id = line.slice(4);
            } else if (line.startsWith("event: ")) {
                event = line.slice(7);
            } else if (line.startsWith("data: ")) {
                dataLines.push(line.slice(6));
            }
        }
        parsed.push({ id, event, data: dataLines.join("\n") });
    }
    return parsed;
}

