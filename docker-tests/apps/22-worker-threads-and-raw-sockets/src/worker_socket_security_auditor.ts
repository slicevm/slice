import crypto from "node:crypto";
import bytes from "bytes";
import type { FiveRingAuditSummary, FiveRingSecurityCase } from "./types.ts";

export class FiveRingSocketWorkerAuditor {
    private readonly secrets: string[];
    private readonly maxSocketIoBytes: number;

    constructor(secrets: string[]) {
        this.secrets = secrets;
        this.maxSocketIoBytes = Number(bytes("4MB"));
    }

    private isNonCanonicalOrForbiddenIp(host: string): boolean {
        const clean = host.trim().toLowerCase();
        if (
            clean === "localhost" ||
            clean === "::1" ||
            clean === "[::1]" ||
            clean === "0.0.0.0" ||
            clean === "169.254.169.254"
        ) {
            return true;
        }
        // Single-integer dword IPv4 literal (e.g. 2852039166) or hex integer
        if (/^\d+$/.test(clean) || /^0x[0-9a-f]+$/.test(clean)) {
            return true;
        }
        const parts = clean.split(".");
        if (parts.length === 4) {
            const octets: number[] = [];
            for (let i = 0; i < 4; i++) {
                const p = parts[i];
                if (p.length > 1 && p.startsWith("0")) {
                    // Octal or hex dotted quad (0177.0.0.1 or 0x7f.0.0.1)
                    return true;
                }
                const n = Number(p);
                if (!Number.isInteger(n) || n < 0 || n > 255) {
                    return true;
                }
                octets.push(n);
            }
            const [a, b] = octets;
            if (
                a === 10 ||
                a === 127 ||
                (a === 172 && b >= 16 && b <= 31) ||
                (a === 192 && b === 168) ||
                (a === 169 && b === 254) ||
                (a === 100 && b >= 64 && b <= 127)
            ) {
                return true;
            }
        }
        return false;
    }

    private containsEgressSecret(payload: string): boolean {
        for (let i = 0; i < this.secrets.length; i++) {
            const s = this.secrets[i];
            if (!s) continue;
            const b64 = Buffer.from(s, "utf8").toString("base64");
            const urlEnc = encodeURIComponent(s);
            if (
                payload.indexOf(s) !== -1 ||
                payload.indexOf(b64) !== -1 ||
                payload.indexOf(urlEnc) !== -1
            ) {
                return true;
            }
        }
        return false;
    }

    public evaluateSuite(cases: FiveRingSecurityCase[]): FiveRingAuditSummary {
        let ring1PrivilegedPortBlocked = 0;
        let ring2SsrfIpBlocked = 0;
        let ring3EgressDlpBlocked = 0;
        let ring4BudgetOverflowBlocked = 0;
        let ring5WorkerEscalationBlocked = 0;
        let cleanOperationsAllowed = 0;
        const transcript: string[] = [];

        for (let i = 0; i < cases.length; i++) {
            const c = cases[i];
            let outcome = "allowed";

            if (c.allowNetRule === "*" && c.targetPort > 0 && c.targetPort < 1024) {
                ring1PrivilegedPortBlocked++;
                outcome = "blocked:ring1_privileged_port";
            } else if (
                c.allowNetRule !== `${c.targetHost}:${c.targetPort}` &&
                this.isNonCanonicalOrForbiddenIp(c.targetHost)
            ) {
                ring2SsrfIpBlocked++;
                outcome = "blocked:ring2_ssrf_ip";
            } else if (c.outboundPayload && this.containsEgressSecret(c.outboundPayload)) {
                ring3EgressDlpBlocked++;
                outcome = "blocked:ring3_egress_dlp";
            } else if (
                typeof c.plannedBytes === "number" &&
                c.plannedBytes > this.maxSocketIoBytes
            ) {
                ring4BudgetOverflowBlocked++;
                outcome = "blocked:ring4_socket_budget";
            } else if (c.parentCaps && c.requestedChildCaps) {
                let escalated = false;
                for (let j = 0; j < c.requestedChildCaps.length; j++) {
                    if (c.parentCaps.indexOf(c.requestedChildCaps[j]) === -1) {
                        escalated = true;
                        break;
                    }
                }
                if (escalated) {
                    ring5WorkerEscalationBlocked++;
                    outcome = "blocked:ring5_worker_attenuation";
                } else {
                    cleanOperationsAllowed++;
                }
            } else {
                cleanOperationsAllowed++;
            }

            transcript.push(`${c.caseId}:${c.ring}:${outcome}`);
        }

        const urlCanParseVerified =
            typeof URL.canParse === "function" &&
            URL.canParse("https://edge.acme.example.com:8443/socket") === true &&
            URL.canParse("/relative-without-base") === false &&
            URL.canParse("relative/path", "https://edge.acme.example.com") === true;

        const auditDigest = crypto
            .createHash("sha256")
            .update(transcript.join("|") + "|canParse=" + String(urlCanParseVerified))
            .digest("hex")
            .slice(0, 16);

        return {
            totalCasesEvaluated: cases.length,
            ring1PrivilegedPortBlocked,
            ring2SsrfIpBlocked,
            ring3EgressDlpBlocked,
            ring4BudgetOverflowBlocked,
            ring5WorkerEscalationBlocked,
            cleanOperationsAllowed,
            urlCanParseVerified,
            auditDigest,
        };
    }
}

