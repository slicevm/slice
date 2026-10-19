import * as os from "node:os";
import * as path from "node:path";
import * as fs from "node:fs";
import { writeShardedJsonlLogs } from "./log_generator.ts";
import { scanAndAggregateJsonlShards } from "./io_aggregator.ts";

function runDiskIoPipelineApp(): string {
    const workDir = path.join(os.tmpdir(), "slice_io_bench_shards");
    const diskBytes = writeShardedJsonlLogs(workDir, 10, 20);
    const report = scanAndAggregateJsonlShards(workDir, diskBytes);
    if (fs.existsSync(workDir)) {
        fs.rmSync(workDir, { recursive: true, force: true });
    }

    return JSON.stringify({
        app: "03-disk-io-log-pipeline",
        shardsScanned: report.shardsScanned,
        recordsParsed: report.recordsParsed,
        totalDiskBytes: report.totalDiskBytes,
        error5xxCount: report.error5xxCount,
        rateLimit429Count: report.rateLimit429Count,
        totalBytesTransferred: report.totalBytesTransferred,
        maxLatencyMs: report.maxLatencyMs,
        summarySha256: report.summarySha256
    });
}

console.log(runDiskIoPipelineApp());

