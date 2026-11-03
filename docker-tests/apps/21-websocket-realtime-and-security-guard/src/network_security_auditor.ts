import crypto from "node:crypto";
import bytes from "bytes";
import type { NetworkSecurityCase, SecurityAuditSummary } from "./types.ts";

const RFC9110_TCHAR_REGEX = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/;

export class NetworkSecurityAuditor {
    private readonly registeredSecrets: string[];
    private readonly registeredSecretsBase64: string[];

    constructor(secrets: string[]) {
        this.registeredSecrets = secrets.slice();
        this.registeredSecretsBase64 = secrets.map((s) =>
            Buffer.from(s, "utf8").toString("base64"),
        );
    }

    public isValidHeaderName(name: string): boolean {
        return name.length > 0 && RFC9110_TCHAR_REGEX.test(name);
    }

    public isValidHeaderValue(value: string): boolean {
        for (let i = 0; i < value.length; i++) {
            const c = value.charCodeAt(i);
            if ((c < 0x20 && c !== 0x09) || c === 0x7f) {
                return false;
            }
        }
        return true;
    }

    public isValidChunkHexSize(hexStr: string, maxBodyBytes: number): boolean {
        const trimmed = hexStr.trim();
        if (trimmed.length === 0 || trimmed.length > 16) {
            return false;
        }
        for (let i = 0; i < trimmed.length; i++) {
            const c = trimmed.charCodeAt(i);
            const isDigit = c >= 0x30 && c <= 0x39;
            const isUpperHex = c >= 0x41 && c <= 0x46;
            const isLowerHex = c >= 0x61 && c <= 0x66;
            if (!isDigit && !isUpperHex && !isLowerHex) {
                return false;
            }
        }
        const parsed = BigInt("0x" + trimmed);
        return parsed >= 0n && parsed <= BigInt(maxBodyBytes);
    }

    public isNonCanonicalOrInternalSsrfTarget(rawUrl: string): boolean {
        let host = rawUrl.trim().toLowerCase();
        const schemeIdx = host.indexOf("://");
        if (schemeIdx !== -1) {
            host = host.slice(schemeIdx + 3);
        }
        const slashIdx = host.indexOf("/");
        if (slashIdx !== -1) {
            host = host.slice(0, slashIdx);
        }
        if (host.startsWith("[")) {
            const closeIdx = host.indexOf("]");
            if (closeIdx !== -1) {
                host = host.slice(1, closeIdx);
            }
        } else {
            const colonIdx = host.indexOf(":");
            if (colonIdx !== -1) {
                host = host.slice(0, colonIdx);
            }
        }

        if (
            host === "localhost" ||
            host.endsWith(".localhost") ||
            host.endsWith(".internal") ||
            host.startsWith("fe80:") ||
            host.startsWith("fc00:") ||
            host.startsWith("fd00:") ||
            host === "::1" ||
            host === "::"
        ) {
            return true;
        }

        // Detect octal (0177.0.0.1), hex (0x7f.0.0.1), dword (2852039166 / 2130706433), or shorthand (127.1 / 169.254.43518)
        const parts = host.split(".");
        if (parts.length >= 1 && parts.length <= 4) {
            let allNumericParts = true;
            const vals: number[] = [];
            for (let i = 0; i < parts.length; i++) {
                const p = parts[i];
                if (p.length === 0) {
                    allNumericParts = false;
                    break;
                }
                let n = NaN;
                if (p.startsWith("0x")) {
                    n = Number.parseInt(p.slice(2), 16);
                } else if (p.length > 1 && p.startsWith("0") && /^[0-7]+$/.test(p)) {
                    n = Number.parseInt(p, 8);
                } else if (/^[0-9]+$/.test(p)) {
                    n = Number.parseInt(p, 10);
                }
                if (Number.isNaN(n) || n < 0) {
                    allNumericParts = false;
                    break;
                }
                vals.push(n);
            }

            if (allNumericParts) {
                let ipv4 = 0;
                if (vals.length === 1) {
                    ipv4 = vals[0] >>> 0;
                } else if (vals.length === 2) {
                    ipv4 = ((vals[0] << 24) | (vals[1] & 0x00ffffff)) >>> 0;
                } else if (vals.length === 3) {
                    ipv4 = ((vals[0] << 24) | (vals[1] << 16) | (vals[2] & 0x0000ffff)) >>> 0;
                } else {
                    ipv4 =
                        ((vals[0] << 24) |
                            (vals[1] << 16) |
                            (vals[2] << 8) |
                            vals[3]) >>>
                        0;
                }
                const b0 = (ipv4 >>> 24) & 0xff;
                const b1 = (ipv4 >>> 16) & 0xff;
                if (
                    b0 === 0 ||
                    b0 === 10 ||
                    b0 === 127 ||
                    (b0 === 169 && b1 === 254) ||
                    (b0 === 172 && b1 >= 16 && b1 <= 31) ||
                    (b0 === 192 && b1 === 168) ||
                    (b0 === 100 && b1 >= 64 && b1 <= 127) ||
                    (b0 === 198 && (b1 === 18 || b1 === 19)) ||
                    b0 >= 224
                ) {
                    return true;
                }
            }
        }
        return false;
    }

    public containsEgressSecret(payload: string): boolean {
        for (let i = 0; i < this.registeredSecrets.length; i++) {
            if (
                payload.indexOf(this.registeredSecrets[i]) !== -1 ||
                payload.indexOf(this.registeredSecretsBase64[i]) !== -1
            ) {
                return true;
            }
        }
        return false;
    }

    public evaluateSuite(cases: NetworkSecurityCase[]): SecurityAuditSummary {
        const maxHeaderBudgetBytes = Number(bytes.parse("64kb"));
        const maxInboundBodyBudgetBytes = Number(bytes.parse("4mb"));
        const maxWsFrameBudgetBytes = Number(bytes.parse("4mb"));

        let smugglingClTeBlocked = 0;
        let smugglingDupClBlocked = 0;
        let crlfHeaderInjectionBlocked = 0;
        let chunkedOverflowBlocked = 0;
        let ssrfNonCanonicalIpBlocked = 0;
        let wsFrameDlpBlocked = 0;
        let cleanRequestsAllowed = 0;
        const verdicts: string[] = [];

        for (let i = 0; i < cases.length; i++) {
            const c = cases[i];
            if (
                c.contentLengthHeader !== undefined &&
                c.transferEncodingHeader !== undefined
            ) {
                smugglingClTeBlocked++;
                verdicts.push(c.caseId + ":BLOCK_CL_TE");
                continue;
            }
            if (
                c.contentLengthHeader !== undefined &&
                c.contentLengthHeader.indexOf(",") !== -1
            ) {
                const parts = c.contentLengthHeader.split(",").map((s) => s.trim());
                if (parts[0] !== parts[1]) {
                    smugglingDupClBlocked++;
                    verdicts.push(c.caseId + ":BLOCK_DUP_CL");
                    continue;
                }
            }
            if (
                !this.isValidHeaderName(c.headerName) ||
                !this.isValidHeaderValue(c.headerValue)
            ) {
                crlfHeaderInjectionBlocked++;
                verdicts.push(c.caseId + ":BLOCK_CRLF_HDR");
                continue;
            }
            if (
                c.chunkSizeHex !== undefined &&
                !this.isValidChunkHexSize(c.chunkSizeHex, maxInboundBodyBudgetBytes)
            ) {
                chunkedOverflowBlocked++;
                verdicts.push(c.caseId + ":BLOCK_CHUNK_OVERFLOW");
                continue;
            }
            if (this.isNonCanonicalOrInternalSsrfTarget(c.rawTargetUrl)) {
                ssrfNonCanonicalIpBlocked++;
                verdicts.push(c.caseId + ":BLOCK_SSRF_NONCANONICAL");
                continue;
            }
            if (
                c.wsFramePayload !== undefined &&
                this.containsEgressSecret(c.wsFramePayload)
            ) {
                wsFrameDlpBlocked++;
                verdicts.push(c.caseId + ":BLOCK_WS_DLP");
                continue;
            }

            cleanRequestsAllowed++;
            verdicts.push(c.caseId + ":ALLOW");
        }

        const auditTrailDigest = crypto
            .createHash("sha256")
            .update(verdicts.join("|"))
            .digest("hex")
            .slice(0, 32);

        return {
            totalCasesEvaluated: cases.length,
            smugglingClTeBlocked,
            smugglingDupClBlocked,
            crlfHeaderInjectionBlocked,
            chunkedOverflowBlocked,
            ssrfNonCanonicalIpBlocked,
            wsFrameDlpBlocked,
            cleanRequestsAllowed,
            maxHeaderBudgetBytes,
            maxInboundBodyBudgetBytes,
            maxWsFrameBudgetBytes,
            auditTrailDigest,
        };
    }
}

