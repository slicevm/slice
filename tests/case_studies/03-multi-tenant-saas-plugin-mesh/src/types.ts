export type PluginHealthState = "HEALTHY" | "HOT_SWAPPED_V2" | "CIRCUIT_BROKEN_NOISY_NEIGHBOR";

export interface TenantPluginManifest {
    tenantId: string;
    pluginName: string;
    version: string;
    fuelLimit: number;
    memoryLimitBytes: number;
    taxRateBps: number;
    loyaltyMultiplier: number;
    faultMode: "NONE" | "CPU_RUNAWAY_LOOP" | "UNAUTHORIZED_FS_READ" | "SES_PROTOTYPE_TAMPER";
    hotSwapTargetTaxRateBps?: number;
}

export interface TenantExecutionSummary {
    tenantId: string;
    pluginName: string;
    activeVersion: string;
    healthState: PluginHealthState;
    eventsProcessed: number;
    eventsSucceeded: number;
    eventsTrapped: number;
    trapReason: string;
    totalNetSettledCents: number;
    totalLoyaltyPoints: number;
    kvCounterFinal: number;
}

export interface SaasPluginMeshReport {
    caseStudy: string;
    totalTenants: number;
    totalCheckoutEvents: number;
    healthyTenants: number;
    hotSwappedTenants: number;
    noisyNeighborsIsolated: number;
    crossTenantKvIsolationVerified: boolean;
    totalPlatformVolumeCents: number;
    tenantSummaries: TenantExecutionSummary[];
    meshDigest: string;
}

