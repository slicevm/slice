export interface EdgeTenantSample {
    tenantId: string;
    region: string;
    requestsPerSec: number;
    p99LatencyUs: number;
    errorCount: number;
    memoryBytes: number;
    rawBannerTitle: string;
}

export interface ScoredTenantRow {
    tenantId: string;
    region: string;
    wasmHealthScore: number;
    wasmSlaTierCode: number;
    slaTierLabel: string;
    formattedMemory: string;
    rawBannerTitle: string;
    escapedTitle: string;
}

export interface SsrPipelineSummary {
    suite: string;
    sessionTenant: string;
    filterRegion: string;
    cacheTtlWindow: string;
    tenantsEvaluated: number;
    wasmHostHookCalls: number;
    wasmTableLength: number;
    wasmGlobalPenaltyPoints: number;
    totalWasmScoreSum: number;
    criticalTenantsCount: number;
    ssrHtmlByteLength: number;
    xssEscapedVerified: boolean;
    setCookieHeader: string;
    ssrHtmlSha256: string;
}
