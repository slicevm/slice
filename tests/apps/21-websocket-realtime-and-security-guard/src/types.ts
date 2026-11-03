export interface WsTelemetryPacket {
    seq: number;
    tenant: string;
    channel: string;
    metric: string;
    value: number;
    windowMs: number;
    digest: string;
}

export interface NetworkSecurityCase {
    caseId: string;
    category:
        | "smuggling_cl_te"
        | "smuggling_dup_cl"
        | "crlf_header_injection"
        | "chunked_overflow"
        | "ssrf_noncanonical_ip"
        | "ws_frame_dlp_leak"
        | "clean_request";
    rawTargetUrl: string;
    headerName: string;
    headerValue: string;
    contentLengthHeader?: string;
    transferEncodingHeader?: string;
    chunkSizeHex?: string;
    wsFramePayload?: string;
}

export interface SecurityAuditSummary {
    totalCasesEvaluated: number;
    smugglingClTeBlocked: number;
    smugglingDupClBlocked: number;
    crlfHeaderInjectionBlocked: number;
    chunkedOverflowBlocked: number;
    ssrfNonCanonicalIpBlocked: number;
    wsFrameDlpBlocked: number;
    cleanRequestsAllowed: number;
    maxHeaderBudgetBytes: number;
    maxInboundBodyBudgetBytes: number;
    maxWsFrameBudgetBytes: number;
    auditTrailDigest: string;
}

export interface WsRealtimeSessionSummary {
    handshakePath: string;
    rfc6455AcceptVerified: boolean;
    maskedFrameRoundtripVerified: boolean;
    textFramesSent: number;
    textAcksReceived: number;
    binaryFramesSent: number;
    binaryXorBytesVerified: number;
    closeCode: number;
    closeReason: string;
    upstreamSubgraphStatus: number;
    upstreamAccountTier: string;
    sessionTranscriptDigest: string;
}

