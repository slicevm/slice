export interface WebhookJob {
    eventId: string;
    tenantId: string;
    targetUrl: string;
    eventType: "invoice.paid" | "subscription.renewed" | "payout.settled";
    amountCents: number;
    retryWindow: string;
}

export interface WebhookDispatchSummary {
    app: string;
    libraries: string[];
    totalJobsProcessed: number;
    liveHttpDeliveries: number;
    ssrfMetadataBlocked: number;
    privateCidrBlocked: number;
    untrustedDomainBlocked: number;
    retryBackoffMs: number;
    auditSignaturePrefix: string;
}

