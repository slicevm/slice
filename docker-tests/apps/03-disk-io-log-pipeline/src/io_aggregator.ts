import * as fs from "node:fs";
import * as path from "node:path";
import * as crypto from "node:crypto";
import type { AccessLogRecord } from "./log_generator.ts";

export interface IoPipelineReport {
    readonly shardsScanned: number;
    readonly recordsParsed: number;
    readonly totalDiskBytes: number;
    readonly error5xxCount: number;
    readonly rateLimit429Count: number;
    readonly totalBytesTransferred: number;
    readonly maxLatencyMs: number;
    readonly summarySha256: string;
}

export function scanAndAggregateJsonlShards(workDir: string, totalDiskBytes: number): IoPipelineReport {
    const entries = fs.readdirSync(workDir);
    let shardsScanned = 0;
    let recordsParsed = 0;
    let error5xxCount = 0;
    let rateLimit429Count = 0;
    let totalBytesTransferred = 0;
    let maxLatencyMs = 0;

    for (let i = 0; i < entries.length; i++) {
        const name = String(entries[i]);
        if (!name.endsWith(".jsonl")) {
            continue;
        }
        shardsScanned += 1;
        const fullPath = path.join(workDir, name);
        const content = fs.readFileSync(fullPath, "utf8");
        const lines = content.split("\n");

        for (let j = 0; j < lines.length; j++) {
            const line = lines[j].trim();
            if (!line) {
                continue;
            }
            const rec = JSON.parse(line) as AccessLogRecord;
            recordsParsed += 1;
            if (rec.status >= 500) {
                error5xxCount += 1;
            } else if (rec.status === 429) {
                rateLimit429Count += 1;
            }
            totalBytesTransferred += rec.bytesOut;
            if (rec.latencyMs > maxLatencyMs) {
                maxLatencyMs = rec.latencyMs;
            }
        }
        fs.unlinkSync(fullPath);
    }

    const summaryFile = path.join(workDir, "compacted_summary.json");
    const summaryPayload = JSON.stringify({
        shardsScanned: shardsScanned,
        recordsParsed: recordsParsed,
        error5xxCount: error5xxCount,
        rateLimit429Count: rateLimit429Count,
        totalBytesTransferred: totalBytesTransferred,
        maxLatencyMs: maxLatencyMs
    });

    fs.writeFileSync(summaryFile, summaryPayload);
    const verifiedBack = fs.readFileSync(summaryFile, "utf8");
    const summarySha256 = crypto.createHash("sha256").update(verifiedBack).digest("hex").slice(0, 16);
    fs.unlinkSync(summaryFile);

    return {
        shardsScanned: shardsScanned,
        recordsParsed: recordsParsed,
        totalDiskBytes: totalDiskBytes,
        error5xxCount: error5xxCount,
        rateLimit429Count: rateLimit429Count,
        totalBytesTransferred: totalBytesTransferred,
        maxLatencyMs: maxLatencyMs,
        summarySha256: summarySha256
    };
}

