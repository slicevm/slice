import * as fs from "node:fs";
import * as path from "node:path";

export interface AccessLogRecord {
    readonly seq: number;
    readonly service: string;
    readonly status: number;
    readonly latencyMs: number;
    readonly bytesOut: number;
}

export function writeShardedJsonlLogs(workDir: string, shardCount: number, recordsPerShard: number): number {
    if (!fs.existsSync(workDir)) {
        fs.mkdirSync(workDir, { recursive: true });
    }

    const services = ["auth-edge", "checkout-api", "cas-router", "ledger-sync"];
    const statuses = [200, 200, 200, 201, 404, 429, 500];
    let totalBytesWritten = 0;

    for (let s = 0; s < shardCount; s++) {
        const shardName = "shard_" + (s < 10 ? "0" + String(s) : String(s)) + ".jsonl";
        const shardPath = path.join(workDir, shardName);
        const lines: string[] = [];

        for (let r = 0; r < recordsPerShard; r++) {
            const seq = s * recordsPerShard + r;
            const rec: AccessLogRecord = {
                seq: seq,
                service: services[seq % services.length],
                status: statuses[seq % statuses.length],
                latencyMs: ((seq * 17) % 120) + 2,
                bytesOut: ((seq * 251) % 8192) + 256
            };
            lines.push(JSON.stringify(rec));
        }

        const payload = lines.join("\n") + "\n";
        fs.writeFileSync(shardPath, payload);
        const stat = fs.statSync(shardPath);
        totalBytesWritten += stat.size;
    }

    return totalBytesWritten;
}

