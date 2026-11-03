export interface WorkerTaskPayload {
    taskId: string;
    shardIndex: number;
    tenant: string;
    records: number[];
    windowMs: number;
}

export interface WorkerTaskResult {
    taskId: string;
    shardIndex: number;
    tenant: string;
    recordCount: number;
    weightedSum: number;
    xorChecksum: number;
    workerTag: string;
}

export interface FiveRingSecurityCase {
    caseId: string;
    ring:
        | "ring1_ocap_wildcard_privileged_port"
        | "ring2_ssrf_noncanonical_or_metadata"
        | "ring3_egress_dlp_tcp_udp"
        | "ring4_socket_io_budget"
        | "ring5_worker_cap_attenuation"
        | "clean_operation";
    targetHost: string;
    targetPort: number;
    allowNetRule: string;
    outboundPayload?: string;
    plannedBytes?: number;
    parentCaps?: string[];
    requestedChildCaps?: string[];
}

export interface FiveRingAuditSummary {
    totalCasesEvaluated: number;
    ring1PrivilegedPortBlocked: number;
    ring2SsrfIpBlocked: number;
    ring3EgressDlpBlocked: number;
    ring4BudgetOverflowBlocked: number;
    ring5WorkerEscalationBlocked: number;
    cleanOperationsAllowed: number;
    urlCanParseVerified: boolean;
    auditDigest: string;
}

export interface WorkerAndSocketSessionSummary {
    workersSpawned: number;
    workerTasksCompleted: number;
    totalRecordsProcessed: number;
    combinedWeightedSum: number;
    combinedXorChecksum: number;
    messageChannelSyncVerified: boolean;
    tcpCommandsSent: number;
    tcpRepliesVerified: number;
    tcpStatusValue: string;
    udpDatagramsSent: number;
    udpAckBytesReceived: number;
    udpAckVerified: boolean;
    sessionDigest: string;
}

