export interface RedirectSecurityVerdict {
    allowed: boolean;
    reason: "OK" | "SSRF_METADATA" | "DNS_REBIND_PRIVATE_IP" | "DLP_EXFIL";
    stripCrossOriginCredentials: boolean;
}

export class ZeroTrustRedirectAndDlpGuard {
    private readonly secretPatterns: string[];

    constructor(secrets: string[]) {
        this.secretPatterns = [];
        for (let i = 0; i < secrets.length; i++) {
            const raw = secrets[i];
            if (raw.length >= 6) {
                this.secretPatterns.push(raw);
                this.secretPatterns.push(Buffer.from(raw).toString("base64"));
                this.secretPatterns.push(Buffer.from(raw).toString("base64url"));
            }
        }
    }

    public evaluateHopAndRedirect(
        originUrl: string,
        redirectLocation: string | undefined,
        auditPayload: string,
    ): RedirectSecurityVerdict {
        for (let i = 0; i < this.secretPatterns.length; i++) {
            const pat = this.secretPatterns[i];
            if (
                originUrl.indexOf(pat) !== -1 ||
                (redirectLocation && redirectLocation.indexOf(pat) !== -1) ||
                auditPayload.indexOf(pat) !== -1
            ) {
                return {
                    allowed: false,
                    reason: "DLP_EXFIL",
                    stripCrossOriginCredentials: false,
                };
            }
        }

        if (!redirectLocation) {
            return {
                allowed: true,
                reason: "OK",
                stripCrossOriginCredentials: false,
            };
        }

        const originParsed = new URL(originUrl);
        const targetParsed = new URL(redirectLocation, originUrl);
        const host = targetParsed.hostname.toLowerCase();

        if (
            host === "169.254.169.254" ||
            host === "169.254.170.2" ||
            host === "100.100.100.200" ||
            host === "metadata.google.internal" ||
            host.indexOf("169.254.169.254") !== -1 ||
            host.indexOf("fd00:ec2::254") !== -1
        ) {
            return {
                allowed: false,
                reason: "SSRF_METADATA",
                stripCrossOriginCredentials: true,
            };
        }

        if (
            host.startsWith("10.") ||
            host.startsWith("192.168.") ||
            host.startsWith("100.64.") ||
            host.startsWith("172.16.") ||
            host.indexOf("::ffff:127.0.0.1") !== -1 ||
            host.indexOf("rebind-internal.") !== -1
        ) {
            return {
                allowed: false,
                reason: "DNS_REBIND_PRIVATE_IP",
                stripCrossOriginCredentials: true,
            };
        }

        const crossOrigin = originParsed.origin !== targetParsed.origin;
        return {
            allowed: true,
            reason: "OK",
            stripCrossOriginCredentials: crossOrigin,
        };
    }
}

