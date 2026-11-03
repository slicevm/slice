import crypto from "node:crypto";
import bytes from "bytes";
import cookie from "cookie";
import qs from "fast-querystring";
import ms from "ms";
import { ZeroTrustRedirectAndDlpGuard } from "./redirect_security_guard.ts";
import type { GatewayHopTask, ZeroTrustKeepAliveSummary } from "./types.ts";

export class ZeroTrustKeepAliveMeshClient {
    private readonly guard: ZeroTrustRedirectAndDlpGuard;
    private readonly hmacSecret: string;

    constructor(secrets: string[], hmacSecret: string) {
        this.guard = new ZeroTrustRedirectAndDlpGuard(secrets);
        this.hmacSecret = hmacSecret;
    }

    public async executeBatch(tasks: GatewayHopTask[]): Promise<ZeroTrustKeepAliveSummary> {
        let keepAliveSubgraphHits = 0;
        let redirectFollow307Completed = 0;
        let redirectManual302Inspected = 0;
        let ssrfRedirectMetadataBlocked = 0;
        let dnsRebindPrivateIpBlocked = 0;
        let dlpExfiltrationBlocked = 0;
        let crossOriginAuthStrippedCount = 0;
        let totalTimeoutBudgetMs = 0;
        let totalPayloadBytes = 0;
        const hopFingerprints: string[] = [];

        for (let i = 0; i < tasks.length; i++) {
            const t = tasks[i];
            totalTimeoutBudgetMs += Number(ms(t.timeoutWindow));

            const serializedCookie = cookie.serialize("slice_sid", t.sessionCookieVal, {
                httpOnly: true,
                path: "/",
            });
            const parsedCookies = cookie.parse(serializedCookie);
            const activeSid = String(parsedCookies.slice_sid || "none");

            const verdict = this.guard.evaluateHopAndRedirect(
                t.targetUrl,
                t.simulatedRedirectLocation,
                t.auditNote,
            );
            if (verdict.stripCrossOriginCredentials) {
                crossOriginAuthStrippedCount++;
            }

            if (!verdict.allowed) {
                if (verdict.reason === "SSRF_METADATA") {
                    ssrfRedirectMetadataBlocked++;
                } else if (verdict.reason === "DNS_REBIND_PRIVATE_IP") {
                    dnsRebindPrivateIpBlocked++;
                } else {
                    dlpExfiltrationBlocked++;
                }
                hopFingerprints.push(t.taskId + ":BLOCKED:" + verdict.reason);
                continue;
            }

            if (t.mode === "keepalive_subgraph") {
                const parsedUrl = new URL(t.targetUrl);
                const q = qs.parse(parsedUrl.search.slice(1));
                const res = await fetch(t.targetUrl, {
                    method: "GET",
                    headers: {
                        accept: "application/json",
                        authorization: "Bearer " + t.outboundBearerToken,
                        cookie: serializedCookie,
                        "x-task-id": t.taskId,
                    },
                });
                if (res.status === 200 && res.ok) {
                    keepAliveSubgraphHits++;
                }
                const textBody = await res.text();
                totalPayloadBytes += textBody.length;
                const data = JSON.parse(textBody) as {
                    accountId: string;
                    tier: string;
                    balanceCents: number;
                };
                hopFingerprints.push(
                    t.taskId +
                        ":SUBGRAPH:" +
                        String(q.tenant || t.tenantId) +
                        ":" +
                        data.accountId +
                        ":" +
                        String(data.balanceCents) +
                        ":" +
                        activeSid,
                );
            } else if (t.mode === "redirect_follow_307") {
                const res = await fetch(t.targetUrl, {
                    method: "GET",
                    headers: {
                        accept: "application/json",
                        authorization: "Bearer " + t.outboundBearerToken,
                    },
                    redirect: "follow",
                });
                const textBody = await res.text();
                totalPayloadBytes += textBody.length;
                const finalPath = new URL(res.url).pathname;
                if (res.status === 200 && res.redirected && finalPath === "/upstream/subgraph/account") {
                    redirectFollow307Completed++;
                }
                hopFingerprints.push(
                    t.taskId + ":REDIR307:" + String(res.redirected) + ":" + finalPath,
                );
            } else if (t.mode === "redirect_manual_302") {
                const res = await fetch(t.targetUrl, {
                    method: "GET",
                    redirect: "manual",
                });
                const loc = res.headers.get("location") || "";
                if (res.status === 302 && loc.indexOf("/upstream/subgraph/account") !== -1) {
                    redirectManual302Inspected++;
                }
                hopFingerprints.push(t.taskId + ":MANUAL302:" + String(res.status) + ":" + loc);
            }
        }

        const meshAuditHmac = crypto
            .createHmac("sha256", this.hmacSecret)
            .update(hopFingerprints.join("|"))
            .digest("hex")
            .slice(0, 16);

        return {
            app: "17-zero-trust-redirect-keepalive-mesh",
            libraries: [
                "bytes@3.1.2",
                "cookie@0.6.0",
                "fast-querystring@1.1.2",
                "ms@2.1.3",
            ],
            tasksProcessed: tasks.length,
            keepAliveSubgraphHits,
            redirectFollow307Completed,
            redirectManual302Inspected,
            ssrfRedirectMetadataBlocked,
            dnsRebindPrivateIpBlocked,
            dlpExfiltrationBlocked,
            crossOriginAuthStrippedCount,
            totalTimeoutBudgetMs,
            formattedPayloadVolume: String(bytes.format(totalPayloadBytes)),
            meshAuditHmac,
        };
    }
}

