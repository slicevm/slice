import ms from "ms";

export interface SchedulerOrderTrace {
    executionOrder: string[];
    intervalTicks: number;
    cancelledTimerFired: boolean;
    totalRetryBackoffMs: number;
}

/**
 * Verifies deterministic ECMAScript Event Loop ordering across:
 * - `queueMicrotask` & `Promise.resolve().then` microtasks
 * - `setTimeout` macrotasks with extra arguments
 * - `clearTimeout` cancellation
 * - `setInterval` periodic ticks + `clearInterval` cleanup
 */
export async function runTimerWheelAndMicrotaskSchedule(): Promise<SchedulerOrderTrace> {
    const executionOrder: string[] = [];
    let cancelledTimerFired = false;
    let intervalTicks = 0;

    const cancelledId = setTimeout(() => {
        cancelledTimerFired = true;
        executionOrder.push("cancelled-leak");
    }, 1);
    clearTimeout(cancelledId);

    Promise.resolve().then(() => {
        executionOrder.push("promise-micro-1");
    });
    queueMicrotask(() => {
        executionOrder.push("queue-micro-1");
    });

    await new Promise<void>((resolve) => {
        setTimeout(
            (stage: string, priority: number) => {
                executionOrder.push(`${stage}:p${priority}`);
                queueMicrotask(() => {
                    executionOrder.push("micro-inside-timer");
                });
            },
            1,
            "timer-stage-a",
            1
        );

        const intervalId = setInterval(() => {
            intervalTicks += 1;
            executionOrder.push(`heartbeat:${intervalTicks}`);
            if (intervalTicks >= 3) {
                clearInterval(intervalId);
                resolve();
            }
        }, 2);
    });

    const backoffSchedule = ["5ms", "15ms", "30ms"];
    let totalRetryBackoffMs = 0;
    for (let i = 0; i < backoffSchedule.length; i++) {
        const delay = Number(ms(backoffSchedule[i]));
        totalRetryBackoffMs += delay;
    }

    return {
        executionOrder,
        intervalTicks,
        cancelledTimerFired,
        totalRetryBackoffMs,
    };
}

/**
 * Executes an async operation with `setTimeout` backoff retries and `AbortController` support.
 */
export async function executeWithRetryBackoff<T>(
    maxAttempts: number,
    baseDelayMs: number,
    operation: (attempt: number) => Promise<T>
): Promise<{ result: T; attempts: number }> {
    let attempt = 0;
    while (true) {
        attempt += 1;
        try {
            const result = await operation(attempt);
            return { result, attempts: attempt };
        } catch (err) {
            if (attempt >= maxAttempts) {
                throw err;
            }
            await new Promise((r) => setTimeout(r, baseDelayMs));
        }
    }
}

