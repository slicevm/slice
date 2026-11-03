import crypto from "node:crypto";
import dgram from "node:dgram";
import net from "node:net";
import {
    MessageChannel,
    Worker,
    receiveMessageOnPort,
} from "node:worker_threads";
import type {
    WorkerAndSocketSessionSummary,
    WorkerTaskPayload,
    WorkerTaskResult,
} from "./types.ts";

const WORKER_SCRIPT = `
(async function () {
    const wt = typeof require === "function"
        ? require("node:worker_threads")
        : await (new Function("s", "return import(s)"))("node:worker_threads");
    const parentPort = wt.parentPort || (wt.default && wt.default.parentPort);
    const workerData = wt.workerData || (wt.default && wt.default.workerData) || {};
    parentPort.on("message", function (task) {
        let weightedSum = 0;
        let xorChecksum = Number(workerData.seed || 0) & 0xff;
        const records = task.records || [];
        for (let i = 0; i < records.length; i++) {
            const val = Number(records[i]);
            weightedSum += val * (i + 1) + Number(workerData.multiplier || 1);
            xorChecksum = (xorChecksum ^ ((val + i) & 0xff)) & 0xff;
        }
        parentPort.postMessage({
            taskId: task.taskId,
            shardIndex: task.shardIndex,
            tenant: task.tenant,
            recordCount: records.length,
            weightedSum: weightedSum,
            xorChecksum: xorChecksum,
            workerTag: String(workerData.tag || "worker") + "#" + String(task.shardIndex)
        });
    });
})();
`;

function runSingleWorkerTask(
    task: WorkerTaskPayload,
    seed: number,
    multiplier: number,
): Promise<WorkerTaskResult> {
    return new Promise((resolve, reject) => {
        const worker = new Worker(WORKER_SCRIPT, {
            eval: true,
            workerData: {
                tag: "slice-worker",
                seed,
                multiplier,
            },
        });
        let settled = false;
        worker.on("message", (msg: WorkerTaskResult) => {
            if (!settled) {
                settled = true;
                Promise.resolve(worker.terminate()).then(
                    () => resolve(msg),
                    () => resolve(msg),
                );
            }
        });
        worker.on("error", (err: Error) => {
            if (!settled) {
                settled = true;
                Promise.resolve(worker.terminate()).then(
                    () => reject(err),
                    () => reject(err),
                );
            }
        });
        worker.postMessage(task);
    });
}

function runTcpProtocolSession(
    host: string,
    port: number,
): Promise<{ commandsSent: number; repliesVerified: number; statusValue: string }> {
    return new Promise((resolve, reject) => {
        const socket = net.createConnection({ host, port });
        let rawBuffer = "";
        let settled = false;

        socket.on("connect", () => {
            const wireBatch =
                "PING\r\n" +
                "SET edge:worker:status active-wave-c\r\n" +
                "GET edge:worker:status\r\n" +
                "QUIT\r\n";
            socket.write(wireBatch);
        });

        socket.on("data", (chunk: Buffer | Uint8Array | string) => {
            const text =
                typeof chunk === "string"
                    ? chunk
                    : Buffer.from(chunk).toString("utf8");
            rawBuffer += text;
            if (rawBuffer.indexOf("+BYE") !== -1 && !settled) {
                settled = true;
                socket.destroy();
                const lines = rawBuffer
                    .split("\r\n")
                    .map((s) => s.trim())
                    .filter((s) => s.length > 0);
                let repliesVerified = 0;
                if (lines.indexOf("+PONG") !== -1) repliesVerified++;
                if (lines.indexOf("+OK") !== -1) repliesVerified++;
                if (lines.indexOf("active-wave-c") !== -1) repliesVerified++;
                if (lines.indexOf("+BYE") !== -1) repliesVerified++;
                const statusValue =
                    lines.indexOf("active-wave-c") !== -1
                        ? "active-wave-c"
                        : "missing";
                resolve({
                    commandsSent: 4,
                    repliesVerified,
                    statusValue,
                });
            }
        });

        socket.on("error", (err: Error) => {
            if (!settled) {
                settled = true;
                reject(err);
            }
        });
    });
}

function runUdpTelemetryProbe(
    host: string,
    port: number,
): Promise<{ datagramsSent: number; ackBytes: number; ackVerified: boolean }> {
    return new Promise((resolve, reject) => {
        const udp = dgram.createSocket("udp4");
        let settled = false;
        const payload = Buffer.from(
            "SLICE_UDP_PROBE:acme-realtime:seq=1",
            "utf8",
        );

        udp.on("message", (msg: Buffer | Uint8Array) => {
            if (!settled) {
                settled = true;
                const text = Buffer.from(msg).toString("utf8");
                const byteLen = Buffer.from(msg).length;
                udp.close();
                resolve({
                    datagramsSent: 1,
                    ackBytes: byteLen,
                    ackVerified: text === "UDP_ACK:acme-realtime:seq=1:ok",
                });
            }
        });

        udp.on("error", (err: Error) => {
            if (!settled) {
                settled = true;
                try {
                    udp.close();
                } catch {}
                reject(err);
            }
        });

        udp.send(payload, port, host);
    });
}

export async function executeWorkerPoolAndRawSocketSession(
    tcpHost: string,
    tcpPort: number,
    udpHost: string,
    udpPort: number,
    tasks: WorkerTaskPayload[],
): Promise<WorkerAndSocketSessionSummary> {
    // 1. Synchronous MessageChannel + receiveMessageOnPort verification
    const channel = new MessageChannel();
    channel.port1.postMessage({
        probe: "wave-c-message-channel",
        taskCount: tasks.length,
    });
    const syncReceived = receiveMessageOnPort(channel.port2);
    channel.port1.close();
    channel.port2.close();
    const messageChannelSyncVerified =
        Boolean(syncReceived) &&
        Boolean(syncReceived.message) &&
        syncReceived.message.probe === "wave-c-message-channel" &&
        syncReceived.message.taskCount === tasks.length;

    // 2. Parallel Worker Threads batch execution
    const workerResults: WorkerTaskResult[] = [];
    for (let i = 0; i < tasks.length; i++) {
        const res = await runSingleWorkerTask(
            tasks[i],
            0x42 + i * 7,
            (i + 1) * 3,
        );
        workerResults.push(res);
    }

    let totalRecordsProcessed = 0;
    let combinedWeightedSum = 0;
    let combinedXorChecksum = 0;
    const workerTags: string[] = [];

    for (let i = 0; i < workerResults.length; i++) {
        const r = workerResults[i];
        totalRecordsProcessed += r.recordCount;
        combinedWeightedSum += r.weightedSum;
        combinedXorChecksum = (combinedXorChecksum ^ r.xorChecksum) & 0xff;
        workerTags.push(`${r.workerTag}:${r.weightedSum}:${r.xorChecksum}`);
    }

    // 3. Live TCP & UDP wire-protocol socket session
    const tcpSummary = await runTcpProtocolSession(tcpHost, tcpPort);
    const udpSummary = await runUdpTelemetryProbe(udpHost, udpPort);

    const sessionDigest = crypto
        .createHash("sha256")
        .update(
            workerTags.join("|") +
                `|mc=${messageChannelSyncVerified}` +
                `|tcp=${tcpSummary.repliesVerified}:${tcpSummary.statusValue}` +
                `|udp=${udpSummary.ackVerified}:${udpSummary.ackBytes}`,
        )
        .digest("hex")
        .slice(0, 16);

    return {
        workersSpawned: tasks.length,
        workerTasksCompleted: workerResults.length,
        totalRecordsProcessed,
        combinedWeightedSum,
        combinedXorChecksum,
        messageChannelSyncVerified,
        tcpCommandsSent: tcpSummary.commandsSent,
        tcpRepliesVerified: tcpSummary.repliesVerified,
        tcpStatusValue: tcpSummary.statusValue,
        udpDatagramsSent: udpSummary.datagramsSent,
        udpAckBytesReceived: udpSummary.ackBytes,
        udpAckVerified: udpSummary.ackVerified,
        sessionDigest,
    };
}

