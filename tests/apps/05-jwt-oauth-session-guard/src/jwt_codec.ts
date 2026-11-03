import * as crypto from "node:crypto";

export interface JwtHeader {
    alg: "HS256";
    typ: "JWT";
    kid: string;
}

export interface JwtClaims {
    iss: string;
    sub: string;
    aud: string;
    tenantId: string;
    role: "admin" | "developer" | "viewer";
    scopes: string[];
    iat: number;
    exp: number;
}

export class JwtSignerVerifier {
    private readonly secretKey: string;
    private readonly keyId: string;

    constructor(secretKey: string, keyId: string) {
        this.secretKey = secretKey;
        this.keyId = keyId;
    }

    public toBase64Url(jsonStr: string): string {
        return Buffer.from(jsonStr, "utf8").toString("base64url");
    }

    public fromBase64Url(b64Url: string): string {
        return Buffer.from(b64Url, "base64url").toString("utf8");
    }

    public sign(claims: JwtClaims): string {
        const header: JwtHeader = { alg: "HS256", typ: "JWT", kid: this.keyId };
        const encodedHeader = this.toBase64Url(JSON.stringify(header));
        const encodedPayload = this.toBase64Url(JSON.stringify(claims));
        const signingInput = encodedHeader + "." + encodedPayload;
        const signatureHex = crypto
            .createHmac("sha256", this.secretKey)
            .update(signingInput)
            .digest("hex");
        return signingInput + "." + signatureHex.slice(0, 32);
    }

    public verify(token: string, nowSec: number): JwtClaims | null {
        const parts = token.split(".");
        if (parts.length !== 3) {
            return null;
        }
        const signingInput = parts[0] + "." + parts[1];
        const expectedSig = crypto
            .createHmac("sha256", this.secretKey)
            .update(signingInput)
            .digest("hex")
            .slice(0, 32);
        if (parts[2] !== expectedSig) {
            return null;
        }
        const claims = JSON.parse(this.fromBase64Url(parts[1])) as JwtClaims;
        if (claims.exp <= nowSec) {
            return null;
        }
        return claims;
    }
}

