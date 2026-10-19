// =============================================================================
// REAL-LIFE PRODUCTION MULTI-MODULE GRAPH: MODULE 2 (Telemetry Aggregator & Anomaly Detector)
// =============================================================================

import { ApiEventPayload } from "./validator.ts";

export interface TenantSummary {
    readonly tenantId: string;
    readonly sampleCount: number;
    readonly meanLatencyMs: number;
    readonly maxLatencyMs: number;
    readonly anomalyCount: number;
}

export class TelemetryAggregator {
    private anomalyThresholdMs: number;

    constructor(thresholdMs: number) {
        this.anomalyThresholdMs = thresholdMs;
    }

    public summarizeEvents(events: ApiEventPayload[]): TenantSummary[] {
        const byTenant: Record<string, number[]> = {};
        const tenantOrder: string[] = [];

        for (let i = 0; i < events.length; i++) {
            const ev = events[i];
            if (!byTenant[ev.tenantId]) {
                byTenant[ev.tenantId] = [];
                tenantOrder.push(ev.tenantId);
            }
            const bucket = byTenant[ev.tenantId];
            for (let j = 0; j < ev.metrics.length; j++) {
                bucket.push(ev.metrics[j]);
            }
        }

        const summaries: TenantSummary[] = [];
        for (let k = 0; k < tenantOrder.length; k++) {
            const tid = tenantOrder[k];
            const samples = byTenant[tid];
            let sum = 0;
            let maxVal = 0;
            let anomalies = 0;
            for (let m = 0; m < samples.length; m++) {
                const v = samples[m];
                sum += v;
                if (v > maxVal) {
                    maxVal = v;
                }
                if (v > this.anomalyThresholdMs) {
                    anomalies += 1;
                }
            }
            summaries.push({
                tenantId: tid,
                sampleCount: samples.length,
                meanLatencyMs: Math.round((sum / samples.length) * 100) / 100,
                maxLatencyMs: maxVal,
                anomalyCount: anomalies
            });
        }
        return summaries;
    }
}

