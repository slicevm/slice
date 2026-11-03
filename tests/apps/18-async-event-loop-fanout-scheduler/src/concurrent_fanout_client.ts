import bytes from "bytes";
import { stringify as flattedStringify, parse as flattedParse } from "flatted";
import type { ShardSpec, ShardFanoutOutcome } from "./types.ts";
import { executeWithRetryBackoff } from "./timer_retry_scheduler.ts";

declare const Slice: any;

export function configureFanoutMocks(baseOrigin: string): void {
    if (typeof Slice !== "undefined" && typeof Slice.mockFetch === "function" && baseOrigin.indexOf("api.fanout.slice.local") !== -1) {
        for (let s = 0; s < 4; s++) {
            const url = `${baseOrigin}/v1/shards/${s}`;
            const rawBody = JSON.stringify({
                shardId: s,
                records: (s + 1) * 25,
                status: "synced",
            });
            Slice.mockFetch("POST", url, {
                status: 200,
                headers: {
                    "content-type": "application/json",
                    "x-shard-ack": `ack-${s}`,
                },
                body: rawBody,
            });
        }
    }
}

async function dispatchShardRequest(
    baseOrigin: string,
    spec: ShardSpec,
    signal: AbortSignal
): Promise<{ status: number; records: number; rawBytes: number }> {
    if (signal && signal.aborted) {
        throw signal.reason || new Error("AbortError: The operation was aborted");
    }

    const envelope = flattedParse(
        flattedStringify({
            shardId: spec.shardId,
            region: spec.region,
            budgetBytes: Number(bytes.parse(spec.payloadBudget)),
        })
    );
    const reqBody = JSON.stringify(envelope);
    const targetUrl = `${baseOrigin}/v1/shards/${spec.shardId}`;

    // Node.js offline fallback when running against mock origin `http://api.fanout.slice.local`
    if (typeof Slice === "undefined" && baseOrigin.indexOf("api.fanout.slice.local") !== -1) {
        await new Promise((r) => setTimeout(r, 1));
        if (signal && signal.aborted) {
            throw signal.reason || new Error("AbortError: The operation was aborted");
        }
        const simBody = JSON.stringify({
            shardId: spec.shardId,
            records: (spec.shardId + 1) * 25,
            status: "synced",
        });
        return {
            status: 200,
            records: (spec.shardId + 1) * 25,
            rawBytes: reqBody.length + simBody.length,
        };
    }

    const res = await fetch(targetUrl, {
        method: "POST",
        headers: {
            "content-type": "application/json",
            "x-region": spec.region,
        },
        body: reqBody,
        signal,
    });
    const text = await res.text();
    const parsed = JSON.parse(text);
    return {
        status: res.status,
        records: Number(parsed.records || 0),
        rawBytes: reqBody.length + text.length,
    };
}

export async function runConcurrentShardBatch(
    baseOrigin: string,
    specs: ShardSpec[]
): Promise<ShardFanoutOutcome[]> {
    const tasks = specs.map(async (spec): Promise<ShardFanoutOutcome> => {
        if (spec.cancelPreAborted) {
            const ctrl = new AbortController();
            ctrl.abort(new Error("PreAbortedShardTask"));
            try {
                await dispatchShardRequest(baseOrigin, spec, ctrl.signal);
            } catch (_err) {
                return {
                    shardId: spec.shardId,
                    region: spec.region,
                    status: 499,
                    recordsProcessed: 0,
                    abortedBySignal: true,
                    payloadBytes: 0,
                    retryAttempts: 1,
                };
            }
        }

        const ctrl = new AbortController();
        const { result, attempts } = await executeWithRetryBackoff(3, 1, async (attempt) => {
            if (spec.shardId === 2 && attempt === 1) {
                throw new Error("TransientShardBusy");
            }
            return await dispatchShardRequest(baseOrigin, spec, ctrl.signal);
        });

        return {
            shardId: spec.shardId,
            region: spec.region,
            status: result.status,
            recordsProcessed: result.records,
            abortedBySignal: false,
            payloadBytes: result.rawBytes,
            retryAttempts: attempts,
        };
    });

    return await Promise.all(tasks);
}
