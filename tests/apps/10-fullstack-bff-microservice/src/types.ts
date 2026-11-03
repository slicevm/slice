export interface TenantBffProfile {
  tenantId: string;
  tier: string;
  region: string;
  cacheTtlWindow: string;
  features: {
    aiCopilot: boolean;
    realtimeAnalytics: boolean;
    maxWidgets: number;
  };
}

export interface BffBenchmarkSummary {
  app: string;
  totalRequests: number;
  corsPreflight204Count: number;
  notModified304Count: number;
  dashboard200Count: number;
  preferences201Count: number;
  forbidden403Count: number;
  totalWidgetsRendered: number;
  totalCacheTtlSec: number;
  finalStateHash: string;
}

