import ms from "ms";

export interface TenantQuota {
    usedRequests: number;
    maxRequests: number;
    windowMs: number;
}

export class SlidingQuotaRateLimiter {
    private quotas: Record<string, TenantQuota>;
    private defaultWindowMs: number;

    constructor(windowSpec: string) {
        this.quotas = {};
        this.defaultWindowMs = Number(ms(windowSpec));
    }

    public allowRequest(tenantId: string): boolean {
        let q = this.quotas[tenantId];
        if (!q) {
            const isBurst = tenantId === "tenant_alpha";
            q = {
                usedRequests: 0,
                maxRequests: isBurst ? 40 : 25,
                windowMs: this.defaultWindowMs
            };
            this.quotas[tenantId] = q;
        }
        q.usedRequests += 1;
        return q.usedRequests <= q.maxRequests;
    }

    public getWindowSeconds(): number {
        return Math.round(this.defaultWindowMs / 1000);
    }
}

