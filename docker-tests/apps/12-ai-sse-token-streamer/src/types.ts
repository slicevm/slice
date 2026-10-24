export interface SseFrame {
    readonly id: string;
    readonly event: "meta" | "token" | "tool_call" | "redacted" | "done";
    readonly data: Record<string, any> | string;
    readonly retry?: number;
}

export interface AgentStreamRequest {
    readonly streamId: string;
    readonly tenantId: string;
    readonly model: string;
    readonly promptTokens: string[];
    readonly auditSecret: string;
}

export interface SseBenchmarkSummary {
    readonly app: string;
    readonly libraries: string[];
    readonly streamsCompleted: number;
    readonly totalFramesEmitted: number;
    readonly totalTokensStreamed: number;
    readonly dlpRedactionsTriggered: number;
    readonly totalSseBytes: number;
    readonly heartbeatIntervalMs: number;
    readonly streamAuditHash: string;
}

