import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";
import * as crypto from "node:crypto";
import bytes from "bytes";
import { TraceGraphBuilder } from "./span_graph.ts";

export interface TelemetryPipelineReport {
    app: string;
    libraries: string[];
    tracesProcessed: number;
    spansEvaluated: number;
    p50DurationUs: number;
    p95DurationUs: number;
    totalMemoryFormatted: string;
    walBytesOnDisk: number;
    walSha256: string;
}

export function runTelemetryPipeline(traceCount: number): TelemetryPipelineReport {
    const builder = new TraceGraphBuilder();
    const durations: number[] = [];
    let totalMemoryBytes = 0;
    const walLines: string[] = [];

    for (let i = 0; i < traceCount; i++) {
        const root = builder.buildCircularTrace(i);
        const res = builder.roundtripSerialize(root);
        durations.push(res.totalDurationUs);
        totalMemoryBytes += res.totalMemoryBytes;
        walLines.push(res.wirePayload);
    }

    const walDir = path.join(os.tmpdir(), "slice_telemetry_wal_" + process.pid);
    fs.mkdirSync(walDir, { recursive: true });
    const walFile = path.join(walDir, "traces.flatted.log");
    const fullWalText = walLines.join("\n") + "\n";
    fs.writeFileSync(walFile, fullWalText);

    const walBytesOnDisk = fs.statSync(walFile).size;
    const readBackWal = fs.readFileSync(walFile, "utf8");
    const walSha256 = crypto.createHash("sha256").update(readBackWal).digest("hex").slice(0, 16);

    fs.unlinkSync(walFile);
    fs.rmSync(walDir, { recursive: true, force: true });

    durations.sort((a, b) => a - b);
    const p50DurationUs = durations[Math.floor(durations.length * 0.5)];
    const p95DurationUs = durations[Math.floor(durations.length * 0.95)];

    return {
        app: "06-telemetry-metrics-pipeline",
        libraries: ["bytes@3.1.2", "flatted@3.3.1"],
        tracesProcessed: traceCount,
        spansEvaluated: traceCount * 3,
        p50DurationUs,
        p95DurationUs,
        totalMemoryFormatted: bytes.format(totalMemoryBytes) as string,
        walBytesOnDisk,
        walSha256,
    };
}

