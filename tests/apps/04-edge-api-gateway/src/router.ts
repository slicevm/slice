import qs from "fast-querystring";
import type { HttpRequest, HttpResponse } from "./auth_middleware.ts";
import { BearerAuthVerifier } from "./auth_middleware.ts";
import { SlidingQuotaRateLimiter } from "./rate_limiter.ts";

export class EdgeGatewayRouter {
    private auth: BearerAuthVerifier;
    private limiter: SlidingQuotaRateLimiter;

    constructor() {
        this.auth = new BearerAuthVerifier();
        this.limiter = new SlidingQuotaRateLimiter("1m");
    }

    public handle(req: HttpRequest): HttpResponse {
        const qIdx = req.url.indexOf("?");
        const pathPart = qIdx >= 0 ? req.url.slice(0, qIdx) : req.url;
        const queryPart = qIdx >= 0 ? req.url.slice(qIdx + 1) : "";
        const queryParams = qs.parse(queryPart);

        if (req.method === "GET" && pathPart === "/v1/health") {
            return {
                status: 200,
                headers: { "content-type": "application/json", "x-edge-cache": "HIT" },
                body: JSON.stringify({ ok: true, region: String(queryParams.region || "iad-01") }),
                latencyPoints: 1
            };
        }

        const tenantId = this.auth.authenticate(req);
        if (!tenantId) {
            return {
                status: 401,
                headers: { "content-type": "application/json" },
                body: JSON.stringify({ error: "unauthorized", requestId: req.id }),
                latencyPoints: 2
            };
        }

        if (!this.limiter.allowRequest(tenantId)) {
            return {
                status: 429,
                headers: {
                    "content-type": "application/json",
                    "retry-after": String(this.limiter.getWindowSeconds())
                },
                body: JSON.stringify({ error: "rate_limit_exceeded", tenantId: tenantId }),
                latencyPoints: 2
            };
        }

        const auditTag = this.auth.computeAuditTag(tenantId, req.id);

        if (req.method === "POST" && pathPart === "/v1/events/ingest") {
            const payloadLen = req.bodyJson ? req.bodyJson.length : 0;
            return {
                status: 200,
                headers: { "content-type": "application/json", "x-audit": auditTag },
                body: JSON.stringify({
                    ingested: true,
                    tenant: tenantId,
                    channel: String(queryParams.channel || "default"),
                    bytes: payloadLen,
                    auditTag: auditTag
                }),
                latencyPoints: 5
            };
        }

        if (req.method === "GET" && pathPart.startsWith("/v1/ledger/")) {
            const accountSlug = pathPart.slice(11);
            const currency = String(queryParams.currency || "USD");
            const balanceCents = (accountSlug.length * 14500) + 9900;
            return {
                status: 200,
                headers: { "content-type": "application/json", "x-audit": auditTag },
                body: JSON.stringify({
                    account: accountSlug,
                    currency: currency,
                    tenant: tenantId,
                    balanceCents: balanceCents
                }),
                latencyPoints: 4
            };
        }

        return {
            status: 404,
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ error: "route_not_found", path: pathPart }),
            latencyPoints: 1
        };
    }
}

