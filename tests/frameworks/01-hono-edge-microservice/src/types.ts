export interface OrderLineItem {
    sku: string;
    qty: number;
    unitCents: number;
}

export interface CheckoutPayload {
    customerId: string;
    currency: string;
    discountBps: number;
    items: OrderLineItem[];
}

export interface ValidatedOrderReceipt {
    tenantId: string;
    orderId: string;
    customerId: string;
    currency: string;
    sessionId: string;
    subtotalCents: number;
    discountCents: number;
    taxCents: number;
    totalCents: number;
    ttlMs: number;
    signature: string;
}

export interface HonoBenchmarkSummary {
    framework: string;
    libraries: string[];
    totalRequestsDispatched: number;
    status200Count: number;
    status201Count: number;
    status401Count: number;
    status404Count: number;
    cookiesIssued: number;
    totalRevenueCents: number;
    totalTaxCents: number;
    middlewareHeadersVerified: boolean;
    healthStatus: string;
    auditDigest: string;
}

