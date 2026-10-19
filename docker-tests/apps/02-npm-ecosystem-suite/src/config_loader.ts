import ms from "ms";
import ini from "ini";

export interface ClusterSlaMetrics {
    readonly totalTimeoutMs: number;
    readonly cacheTtlMs: number;
    readonly heartbeatLabel: string;
    readonly regionCount: number;
    readonly primaryDatacenter: string;
}

export function parseClusterTopologyAndSla(rawIni: string): ClusterSlaMetrics {
    const parsed = ini.parse(rawIni);
    const timeouts = parsed.timeouts || {};
    const regions = parsed.regions || {};

    const readMs = Number(ms(String(timeouts.readTimeout || "5s")));
    const writeMs = Number(ms(String(timeouts.writeTimeout || "15s")));
    const drainMs = Number(ms(String(timeouts.drainWindow || "2m")));
    const cacheTtlMs = Number(ms(String(timeouts.edgeCacheTtl || "4h")));
    const heartbeatLabel = String(ms(Number(timeouts.heartbeatMs || 60000), { long: true }));

    const regionKeys = Object.keys(regions);

    return {
        totalTimeoutMs: readMs + writeMs + drainMs,
        cacheTtlMs: cacheTtlMs,
        heartbeatLabel: heartbeatLabel,
        regionCount: regionKeys.length,
        primaryDatacenter: String(regions.primary || "us-east-1")
    };
}

