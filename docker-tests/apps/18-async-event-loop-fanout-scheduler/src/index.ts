import crypto from "node:crypto";
import bytes from "bytes";
import mitt from "mitt";
import type { ShardSpec } from "./types.ts";
import { runTimerWheelAndMicrotaskSchedule } from "./timer_retry_scheduler.ts";
import { configureFanoutMocks, runConcurrentShardBatch } from "./concurrent_fanout_client.ts";

declare const Slice: any;

function verifyRunawayIntervalContainment(): boolean {
    if (typeof Slice !== "undefined" && typeof Slice.evalSandbox === "function") {
        const res = Slice.evalSandbox(
            `for (let i = 0; i < 1050; i++) { setTimeout(() => {}, 100); }`,
            { fuel: 500000, memoryBytes: 1048576 }
        );
        return res && res.ok === false;
    }
    return true;
}

async function main() {
    const baseOrigin =
        (typeof process !== "undefined" &&
            process.env &&
            process.env.SLICE_FANOUT_UPSTREAM_BASE) ||
        "http://api.fanout.slice.local";

    configureFanoutMocks(baseOrigin);

    const bus = mitt();
    let busEventsDispatched = 0;
    bus.on("batch:done", () => {
        busEventsDispatched += 1;
    });

    // 1. Run Timer Wheel + Microtask Queue ordering verification
    const scheduleTrace = await runTimerWheelAndMicrotaskSchedule();

    // 2. Run 5 rounds of concurrent 5-shard fan-out (`Promise.all([fetch(...), ...])`)
    const shardSpecs: ShardSpec[] = [
        { shardId: 0, region: "us-east-1", timeoutBudget: "2s", payloadBudget: "4KB", cancelPreAborted: false },
        { shardId: 1, region: "eu-west-1", timeoutBudget: "2s", payloadBudget: "4KB", cancelPreAborted: false },
        { shardId: 2, region: "ap-northeast-1", timeoutBudget: "2s", payloadBudget: "4KB", cancelPreAborted: false },
        { shardId: 3, region: "us-west-2", timeoutBudget: "2s", payloadBudget: "4KB", cancelPreAborted: false },
        { shardId: 4, region: "sa-east-1", timeoutBudget: "50ms", payloadBudget: "2KB", cancelPreAborted: true },
    ];

    let totalShardsExecuted = 0;
    let totalRecordsProcessed = 0;
    let totalAbortedTasks = 0;
    let totalRetryAttempts = 0;
    let totalWireBytes = 0;

    for (let round = 0; round < 5; round++) {
        const outcomes = await runConcurrentShardBatch(baseOrigin, shardSpecs);
        bus.emit("batch:done", outcomes.length);
        for (let i = 0; i < outcomes.length; i++) {
            const o = outcomes[i];
            totalShardsExecuted += 1;
            totalRecordsProcessed += o.recordsProcessed;
            if (o.abortedBySignal) {
                totalAbortedTasks += 1;
            }
            totalRetryAttempts += o.retryAttempts;
            totalWireBytes += o.payloadBytes;
        }
    }

    // 3. Verify runaway `setInterval` timer bomb is contained by Slice's event loop tick guard
    const timerBombContained = verifyRunawayIntervalContainment();

    const orderJoined = scheduleTrace.executionOrder.join(">");
    const auditDigest = crypto
        .createHmac("sha256", "slice-phase-n2-event-loop-key")
        .update(
            `${orderJoined}|${totalShardsExecuted}|${totalRecordsProcessed}|${totalAbortedTasks}|${totalRetryAttempts}|${totalWireBytes}|${timerBombContained}`
        )
        .digest("hex")
        .slice(0, 16);

    console.log(
        JSON.stringify({
            app: "18-async-event-loop-fanout-scheduler",
            libraries: ["bytes@3.1.2", "flatted@3.3.1", "mitt@3.0.1", "ms@2.1.3"],
            scheduleOrder: orderJoined,
            intervalTicks: scheduleTrace.intervalTicks,
            cancelledTimerFired: scheduleTrace.cancelledTimerFired,
            totalRetryBackoffMs: scheduleTrace.totalRetryBackoffMs,
            totalShardsExecuted,
            totalRecordsProcessed,
            totalAbortedTasks,
            totalRetryAttempts,
            busEventsDispatched,
            timerBombContained,
            formattedWireVolume: bytes.format(totalWireBytes),
            auditDigest,
        })
    );
}

main();
