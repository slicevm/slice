export interface TenantAccountState {
    tenantId: string;
    planVersion: string;
    creditsCents: number;
    retentionWindowMs: number;
    quotas: {
        dailyRequests: number;
        usedRequests: number;
    };
}

export interface SpecExecutionRecord {
    suite: string;
    name: string;
    passed: boolean;
    rolledBackCleanly: boolean;
    stateDigest: string;
}

export interface TimeTravelSuiteReport {
    app: string;
    libraries: string[];
    suitesExecuted: number;
    specsExecuted: number;
    specsPassed: number;
    vfsRollbacksVerified: number;
    kvRollbacksVerified: number;
    totalRetentionMs: number;
    suiteAuditHash: string;
}

