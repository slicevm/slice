// =============================================================================
// REAL-LIFE PRODUCTION EDGE API GATEWAY, AUTH & TELEMETRY PIPELINE (TypeScript)
// =============================================================================
// Features exercised:
// 1. TypeScript Generics, Discriminated Unions, Interfaces, Type Guards
// 2. HTTP Request Router, Bearer Token HMAC Verification, Rate Limiter
// 3. In-Memory Tenant Quota & Session Store (Object / Map / Array operations)
// 4. Structured Telemetry & Digest Aggregation over 100 API Requests
// =============================================================================

type HttpMethod = "GET" | "POST" | "DELETE";

interface HttpRequest {
    readonly id: string;
    readonly method: HttpMethod;
    readonly path: string;
    readonly headers: Record<string, string>;
    readonly clientIp: string;
    readonly bodyJson: string | null;
}

interface HttpResponse {
    readonly status: number;
    readonly headers: Record<string, string>;
    readonly body: string;
    readonly latencyPoints: number;
}

interface TenantQuota {
    usedRequests: number;
    maxRequests: number;
    tier: "standard" | "burst";
}

class EdgeApiGateway {
    private quotas: Record<string, TenantQuota>;
    private validTokens: Record<string, string>;

    constructor() {
        this.quotas = {};
        this.validTokens = {
            "tok_live_alpha_99": "tenant_alpha",
            "tok_live_beta_88": "tenant_beta",
            "tok_live_gamma_77": "tenant_gamma"
        };
    }

    private authenticate(req: HttpRequest): string | null {
        const authHeader = req.headers["authorization"] || "";
        if (!authHeader.startsWith("Bearer ")) {
            return null;
        }
        const token = authHeader.slice(7).trim();
        return this.validTokens[token] || null;
    }

    private checkRateLimit(tenantId: string): boolean {
        let q = this.quotas[tenantId];
        if (!q) {
            const isBurst = tenantId === "tenant_alpha";
            q = {
                usedRequests: 0,
                maxRequests: isBurst ? 40 : 25,
                tier: isBurst ? "burst" : "standard"
            };
            this.quotas[tenantId] = q;
        }
        q.usedRequests += 1;
        return q.usedRequests <= q.maxRequests;
    }

    public handleRequest(req: HttpRequest): HttpResponse {
        // Public health check endpoint bypasses auth
        if (req.method === "GET" && req.path === "/v1/health") {
            return {
                status: 200,
                headers: { "content-type": "application/json", "x-edge-cache": "HIT" },
                body: JSON.stringify({ ok: true, region: "iad-01" }),
                latencyPoints: 1
            };
        }

        // Authenticate Bearer token
        const tenantId = this.authenticate(req);
        if (!tenantId) {
            return {
                status: 401,
                headers: { "content-type": "application/json" },
                body: JSON.stringify({ error: "unauthorized", requestId: req.id }),
                latencyPoints: 2
            };
        }

        // Enforce per-tenant rate limit
        if (!this.checkRateLimit(tenantId)) {
            return {
                status: 429,
                headers: { "content-type": "application/json", "retry-after": "60" },
                body: JSON.stringify({ error: "rate_limit_exceeded", tenantId: tenantId }),
                latencyPoints: 2
            };
        }

        // Route matching & payload transformation
        if (req.method === "POST" && req.path === "/v1/events/ingest") {
            const payloadLen = req.bodyJson ? req.bodyJson.length : 0;
            return {
                status: 200,
                headers: { "content-type": "application/json", "x-tenant": tenantId },
                body: JSON.stringify({
                    ingested: true,
                    tenant: tenantId,
                    bytes: payloadLen,
                    seq: req.id
                }),
                latencyPoints: 5
            };
        }

        if (req.method === "GET" && req.path.startsWith("/v1/ledger/")) {
            const accountSlug = req.path.slice(11);
            const balanceCents = (accountSlug.length * 14500) + 9900;
            return {
                status: 200,
                headers: { "content-type": "application/json", "x-tenant": tenantId },
                body: JSON.stringify({
                    account: accountSlug,
                    tenant: tenantId,
                    balanceCents: balanceCents
                }),
                latencyPoints: 4
            };
        }

        return {
            status: 404,
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ error: "route_not_found", path: req.path }),
            latencyPoints: 1
        };
    }
}

function runGatewaySimulation(): string {
    const gateway = new EdgeApiGateway();
    const tokens = [
        "Bearer tok_live_alpha_99",
        "Bearer tok_live_beta_88",
        "Bearer tok_live_gamma_77",
        "Bearer tok_invalid_000"
    ];
    const paths = [
        "/v1/events/ingest",
        "/v1/ledger/acc_enterprise_01",
        "/v1/health",
        "/v1/unknown/route"
    ];

    let s200 = 0;
    let s401 = 0;
    let s429 = 0;
    let s404 = 0;
    let totalBytes = 0;
    let latencySum = 0;

    for (let i = 0; i < 100; i++) {
        const path = paths[i % 4];
        const method: HttpMethod = path === "/v1/events/ingest" ? "POST" : "GET";
        const req: HttpRequest = {
            id: "gw_" + String(5000 + i),
            method: method,
            path: path,
            headers: {
                "authorization": tokens[i % 4],
                "user-agent": "slice-edge-client/1.0"
            },
            clientIp: "10.24.0." + String((i % 250) + 1),
            bodyJson: method === "POST" ? JSON.stringify({ metric: "cpu_load", value: i * 1.5 }) : null
        };

        const res = gateway.handleRequest(req);
        if (res.status === 200) s200 += 1;
        else if (res.status === 401) s401 += 1;
        else if (res.status === 429) s429 += 1;
        else if (res.status === 404) s404 += 1;

        totalBytes += res.body.length;
        latencySum += res.latencyPoints;
    }

    return JSON.stringify({
        service: "slice-edge-gateway-v1",
        totalRequests: 100,
        status200: s200,
        status401: s401,
        status429: s429,
        status404: s404,
        totalResponseBytes: totalBytes,
        latencyPoints: latencySum
    });
}

console.log(runGatewaySimulation());

