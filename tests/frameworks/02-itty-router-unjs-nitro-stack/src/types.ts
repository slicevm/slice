export interface RadixRoutePolicy {
    policyName: string;
    cacheTtlSec: number;
    rateLimitRpm: number;
    params?: Record<string, string>;
}

export interface CatalogItemPayload {
    category: string;
    itemId: string;
    region: string;
    unitPriceCents: number;
    stockUnits: number;
    cacheTtlSec: number;
    canonicalPath: string;
    etag: string;
}

export interface WebhookEventPayload {
    provider: string;
    eventId: string;
    eventType: string;
    amountCents: number;
    verified: boolean;
    payloadHash: string;
}

export interface UnjsBenchmarkSummary {
    framework: string;
    libraries: string[];
    totalRequestsDispatched: number;
    status200Count: number;
    status202Count: number;
    status304Count: number;
    status404Count: number;
    radixPolicyMatches: number;
    normalizedUrlsCount: number;
    destrPayloadsParsed: number;
    uniqueEtagsGenerated: number;
    totalCatalogValueCents: number;
    totalWebhookAmountCents: number;
    sampleCanonicalSearchUrl: string;
    auditDigest: string;
}

