import assert from "node:assert";
import ms from "ms";
import type { FiveRingSecurityCase, WorkerTaskPayload } from "./types.ts";
import { executeWorkerPoolAndRawSocketSession } from "./worker_and_socket_engine.ts";
import { FiveRingSocketWorkerAuditor } from "./worker_socket_security_auditor.ts";

async function main(): Promise<void> {
    const tcpHost = "127.0.0.1";
    const tcpPort = Number(process.env.SLICE_APP22_TCP_PORT || "18096");
    const udpHost = "127.0.0.1";
    const udpPort = Number(process.env.SLICE_APP22_UDP_PORT || "18097");

    const vaultSecret = "sk_live_WAVE_C_RAW_SOCKET_MASTER_KEY_2026";
    const b64Secret = Buffer.from(vaultSecret, "utf8").toString("base64");
    const auditor = new FiveRingSocketWorkerAuditor([vaultSecret]);

    const securityCases: FiveRingSecurityCase[] = [
        {
            caseId: "ring-01",
            ring: "ring1_ocap_wildcard_privileged_port",
            targetHost: "93.184.216.34",
            targetPort: 22,
            allowNetRule: "*",
        },
        {
            caseId: "ring-02",
            ring: "ring1_ocap_wildcard_privileged_port",
            targetHost: "93.184.216.34",
            targetPort: 25,
            allowNetRule: "*",
        },
        {
            caseId: "ring-03",
            ring: "ring2_ssrf_noncanonical_or_metadata",
            targetHost: "169.254.169.254",
            targetPort: 8080,
            allowNetRule: "*",
        },
        {
            caseId: "ring-04",
            ring: "ring2_ssrf_noncanonical_or_metadata",
            targetHost: "0177.0.0.1",
            targetPort: 6379,
            allowNetRule: "*",
        },
        {
            caseId: "ring-05",
            ring: "ring2_ssrf_noncanonical_or_metadata",
            targetHost: "2852039166",
            targetPort: 8080,
            allowNetRule: "*",
        },
        {
            caseId: "ring-06",
            ring: "ring2_ssrf_noncanonical_or_metadata",
            targetHost: "100.64.42.9",
            targetPort: 9000,
            allowNetRule: "*",
        },
        {
            caseId: "ring-07",
            ring: "ring3_egress_dlp_tcp_udp",
            targetHost: "93.184.216.34",
            targetPort: 6379,
            allowNetRule: "93.184.216.34:6379",
            outboundPayload: `AUTH ${vaultSecret}\r\n`,
        },
        {
            caseId: "ring-08",
            ring: "ring3_egress_dlp_tcp_udp",
            targetHost: "93.184.216.34",
            targetPort: 8125,
            allowNetRule: "93.184.216.34:8125",
            outboundPayload: `metric.tag.token:${b64Secret}|c`,
        },
        {
            caseId: "ring-09",
            ring: "ring4_socket_io_budget",
            targetHost: "93.184.216.34",
            targetPort: 6379,
            allowNetRule: "93.184.216.34:6379",
            plannedBytes: 8 * 1024 * 1024,
        },
        {
            caseId: "ring-10",
            ring: "ring5_worker_cap_attenuation",
            targetHost: "93.184.216.34",
            targetPort: 443,
            allowNetRule: "93.184.216.34:443",
            parentCaps: ["read:/app", "net:127.0.0.1:18096"],
            requestedChildCaps: ["read:/app", "read:/etc/shadow"],
        },
        {
            caseId: "ring-11",
            ring: "clean_operation",
            targetHost: "127.0.0.1",
            targetPort: 18096,
            allowNetRule: "127.0.0.1:18096",
            outboundPayload: "PING\r\n",
            plannedBytes: 1024,
            parentCaps: ["read:/app", "net:127.0.0.1:18096"],
            requestedChildCaps: ["net:127.0.0.1:18096"],
        },
        {
            caseId: "ring-12",
            ring: "clean_operation",
            targetHost: "93.184.216.34",
            targetPort: 6379,
            allowNetRule: "93.184.216.34:6379",
            outboundPayload: "GET cluster:health\r\n",
            plannedBytes: 512,
        },
    ];

    const auditSummary = auditor.evaluateSuite(securityCases);
    assert.strictEqual(auditSummary.totalCasesEvaluated, 12);
    assert.strictEqual(auditSummary.ring1PrivilegedPortBlocked, 2);
    assert.strictEqual(auditSummary.ring2SsrfIpBlocked, 4);
    assert.strictEqual(auditSummary.ring3EgressDlpBlocked, 2);
    assert.strictEqual(auditSummary.ring4BudgetOverflowBlocked, 1);
    assert.strictEqual(auditSummary.ring5WorkerEscalationBlocked, 1);
    assert.strictEqual(auditSummary.cleanOperationsAllowed, 2);
    assert.strictEqual(auditSummary.urlCanParseVerified, true);

    const windowMs = Number(ms("30s"));
    const tasks: WorkerTaskPayload[] = [
        {
            taskId: "task-alpha",
            shardIndex: 1,
            tenant: "acme-edge",
            records: [12, 24, 36, 48, 60, 72],
            windowMs,
        },
        {
            taskId: "task-beta",
            shardIndex: 2,
            tenant: "acme-edge",
            records: [15, 30, 45, 60, 75, 90],
            windowMs,
        },
        {
            taskId: "task-gamma",
            shardIndex: 3,
            tenant: "acme-edge",
            records: [10, 20, 30, 40, 50, 60],
            windowMs,
        },
    ];

    const sessionSummary = await executeWorkerPoolAndRawSocketSession(
        tcpHost,
        tcpPort,
        udpHost,
        udpPort,
        tasks,
    );

    assert.strictEqual(sessionSummary.workersSpawned, 3);
    assert.strictEqual(sessionSummary.workerTasksCompleted, 3);
    assert.strictEqual(sessionSummary.totalRecordsProcessed, 18);
    assert.strictEqual(sessionSummary.combinedWeightedSum, 3475);
    assert.strictEqual(sessionSummary.messageChannelSyncVerified, true);
    assert.strictEqual(sessionSummary.tcpCommandsSent, 4);
    assert.strictEqual(sessionSummary.tcpRepliesVerified, 4);
    assert.strictEqual(sessionSummary.tcpStatusValue, "active-wave-c");
    assert.strictEqual(sessionSummary.udpDatagramsSent, 1);
    assert.strictEqual(sessionSummary.udpAckBytesReceived, 30);
    assert.strictEqual(sessionSummary.udpAckVerified, true);

    console.log(
        JSON.stringify({
            suite: "22-worker-threads-and-raw-sockets",
            auditSummary,
            sessionSummary,
        }),
    );
}

main();

