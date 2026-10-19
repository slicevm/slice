import * as crypto from "node:crypto";
import type { JwtClaims } from "./jwt_codec.ts";
import { OAuthSessionGuard } from "./session_manager.ts";

const guard = new OAuthSessionGuard("slice-zero-trust-hmac-secret-2026", "2h");
const baseNowSec = 1760000000;

let authenticatedCount = 0;
let csrfVerifiedCount = 0;
let rbacGrantedCount = 0;
let deniedCount = 0;
let totalCookieBytes = 0;
const auditHasher = crypto.createHash("sha256");

const roles: Array<"admin" | "developer" | "viewer"> = ["admin", "developer", "viewer"];
const requiredScopes = ["deployments:write", "metrics:read", "logs:read"];

for (let i = 0; i < 50; i++) {
    const role = roles[i % roles.length];
    const scopes =
        role === "admin"
            ? ["deployments:write", "metrics:read", "logs:read"]
            : role === "developer"
              ? ["deployments:write", "logs:read"]
              : ["metrics:read"];

    const isExpired = i % 10 === 9;
    const claims: JwtClaims = {
        iss: "https://auth.slice.edge",
        sub: "usr_" + (1000 + i),
        aud: "slice-control-plane",
        tenantId: "org_" + (i % 8),
        role,
        scopes,
        iat: baseNowSec - 600,
        exp: isExpired ? baseNowSec - 10 : baseNowSec + 7200,
    };

    const csrfToken = "csrf_tok_" + (i * 17 + 31);
    const cookieHeader = guard.issueSessionCookieHeader(claims, csrfToken);
    // Simulate tampered CSRF on every 7th request
    const sentCsrfHeader = i % 7 === 6 ? "csrf_tampered" : csrfToken;
    const reqScope = requiredScopes[i % requiredScopes.length];

    const res = guard.evaluateRequest(cookieHeader, sentCsrfHeader, reqScope, baseNowSec);
    if (res.authenticated) {
        authenticatedCount += 1;
    }
    if (res.csrfValid) {
        csrfVerifiedCount += 1;
    }
    if (res.rbacAllowed) {
        rbacGrantedCount += 1;
        totalCookieBytes += res.refreshedSetCookieHeader.length;
    } else {
        deniedCount += 1;
    }
    auditHasher.update(
        res.tenantId + ":" + res.authenticated + ":" + res.csrfValid + ":" + res.rbacAllowed
    );
}

const summary = {
    app: "05-jwt-oauth-session-guard",
    libraries: ["cookie@0.6.0", "ms@2.1.3"],
    sessionsEvaluated: 50,
    authenticatedCount,
    csrfVerifiedCount,
    rbacGrantedCount,
    deniedCount,
    totalCookieBytes,
    auditSha256: auditHasher.digest("hex").slice(0, 16),
};

console.log(JSON.stringify(summary));

