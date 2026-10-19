import cookie from "cookie";
import ms from "ms";
import { type JwtClaims, JwtSignerVerifier } from "./jwt_codec.ts";

export interface SessionEvalResult {
    authenticated: boolean;
    csrfValid: boolean;
    rbacAllowed: boolean;
    refreshedSetCookieHeader: string;
    tenantId: string;
}

export class OAuthSessionGuard {
    private readonly jwt: JwtSignerVerifier;
    private readonly sessionTtlSec: number;

    constructor(secretKey: string, ttlSpec: string) {
        this.jwt = new JwtSignerVerifier(secretKey, "edge-key-2026-q4");
        const ttlMs = ms(ttlSpec as unknown as number) as unknown as number;
        this.sessionTtlSec = Math.floor(ttlMs / 1000);
    }

    public issueSessionCookieHeader(claims: JwtClaims, csrfToken: string): string {
        const token = this.jwt.sign(claims);
        const sidCookie = cookie.serialize("__Host-slice_session", token, {
            httpOnly: true,
            secure: true,
            sameSite: "strict",
            path: "/",
            maxAge: this.sessionTtlSec,
        });
        const csrfCookie = cookie.serialize("__Host-slice_csrf", csrfToken, {
            httpOnly: false,
            secure: true,
            sameSite: "strict",
            path: "/",
            maxAge: this.sessionTtlSec,
        });
        return sidCookie.split(";")[0] + "; " + csrfCookie.split(";")[0];
    }

    public evaluateRequest(
        rawCookieHeader: string,
        csrfHeader: string,
        requiredScope: string,
        nowSec: number
    ): SessionEvalResult {
        const parsed = cookie.parse(rawCookieHeader);
        const sessionToken = parsed["__Host-slice_session"] || "";
        const csrfCookie = parsed["__Host-slice_csrf"] || "";

        const claims = this.jwt.verify(sessionToken, nowSec);
        if (!claims) {
            return {
                authenticated: false,
                csrfValid: false,
                rbacAllowed: false,
                refreshedSetCookieHeader: "",
                tenantId: "anonymous",
            };
        }

        const csrfValid = csrfCookie.length > 0 && csrfCookie === csrfHeader;
        const rbacAllowed =
            claims.role === "admin" || claims.scopes.indexOf(requiredScope) !== -1;

        const refreshedCookie = cookie.serialize("__Host-slice_session", sessionToken, {
            httpOnly: true,
            secure: true,
            sameSite: "strict",
            path: "/",
            maxAge: this.sessionTtlSec,
        });

        return {
            authenticated: true,
            csrfValid,
            rbacAllowed: csrfValid && rbacAllowed,
            refreshedSetCookieHeader: refreshedCookie,
            tenantId: claims.tenantId,
        };
    }
}

