export type GatewayRouteMode =
    | "keepalive_subgraph"
    | "redirect_follow_307"
    | "redirect_manual_302"
    | "redirect_ssrf_metadata"
    | "redirect_dns_rebind_private"
    | "redirect_dlp_exfil";

export interface GatewayHopTask {
    taskId: string;
    tenantId: string;
    accountId: string;
    mode: GatewayRouteMode;
    targetUrl: string;
    simulatedRedirectLocation?: string;
    outboundBearerToken: string;
    sessionCookieVal: string;
    auditNote: string;
    timeoutWindow: string;
}

export interface ZeroTrustKeepAliveSummary {
    app: string;
    libraries: string[];
    tasksProcessed: number;
    keepAliveSubgraphHits: number;
    redirectFollow307Completed: number;
    redirectManual302Inspected: number;
    ssrfRedirectMetadataBlocked: number;
    dnsRebindPrivateIpBlocked: number;
    dlpExfiltrationBlocked: number;
    crossOriginAuthStrippedCount: number;
    totalTimeoutBudgetMs: number;
    formattedPayloadVolume: string;
    meshAuditHmac: string;
}

