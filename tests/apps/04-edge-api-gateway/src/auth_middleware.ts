import * as crypto from "node:crypto";

export type HttpMethod = "GET" | "POST" | "DELETE";

export interface HttpRequest {
    readonly id: string;
    readonly method: HttpMethod;
    readonly url: string;
    readonly headers: Record<string, string>;
    readonly bodyJson: string | null;
}

export interface HttpResponse {
    readonly status: number;
    readonly headers: Record<string, string>;
    readonly body: string;
    readonly latencyPoints: number;
}

export class BearerAuthVerifier {
    private validTokens: Record<string, string>;

    constructor() {
        this.validTokens = {
            "tok_live_alpha_99": "tenant_alpha",
            "tok_live_beta_88": "tenant_beta",
            "tok_live_gamma_77": "tenant_gamma"
        };
    }

    public authenticate(req: HttpRequest): string | null {
        const authHeader = req.headers["authorization"] || "";
        if (!authHeader.startsWith("Bearer ")) {
            return null;
        }
        const token = authHeader.slice(7).trim();
        return this.validTokens[token] || null;
    }

    public computeAuditTag(tenantId: string, reqId: string): string {
        return crypto.createHash("sha256").update(tenantId + ":" + reqId).digest("hex").slice(0, 12);
    }
}

