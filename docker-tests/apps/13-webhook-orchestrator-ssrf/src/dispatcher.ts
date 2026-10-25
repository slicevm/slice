import crypto from "node:crypto";
import qs from "fast-querystring";
import ms from "ms";
import { evaluateOutboundUrlPolicy } from "./ssrf_guard.ts";
import type { WebhookJob, WebhookDispatchSummary } from "./types.ts";

export class WebhookOrchestrator {
    private readonly allowedHosts: Set<string>;
    private readonly signingSecret: string;

    constructor(allowedHosts: string[], signingSecret: string) {
        this.allowedHosts = new Set<string>(allowedHosts);
        this.signingSecret = signingSecret;
    }

    public async processBatch(jobs: WebhookJob[]): Promise<WebhookDispatchSummary> {
        let liveHttpDeliveries = 0;
        let ssrfMetadataBlocked = 0;
        let privateCidrBlocked = 0;
        let untrustedDomainBlocked = 0;
        let retryBackoffMs = 0;
        const deliveryReceipts: string[] = [];

        for (let i = 0; i < jobs.length; i++) {
            const job = jobs[i];
            retryBackoffMs += Number(ms(job.retryWindow));

            const u = new URL(job.targetUrl);
            const parsedQuery = qs.parse(u.search.slice(1));
            const channel = String(parsedQuery.channel || "default");

            const verdict = evaluateOutboundUrlPolicy(job.targetUrl, this.allowedHosts);
            if (!verdict.allowed) {
                if (verdict.reason === "SSRF_METADATA") {
                    ssrfMetadataBlocked++;
                } else if (verdict.reason === "PRIVATE_CIDR") {
                    privateCidrBlocked++;
                } else {
                    untrustedDomainBlocked++;
                }
                deliveryReceipts.push(job.eventId + ":BLOCKED:" + verdict.reason);
                continue;
            }

            const payload = JSON.stringify({
                eventId: job.eventId,
                tenantId: job.tenantId,
                eventType: job.eventType,
                amountCents: job.amountCents,
                channel,
            });
            const signature = crypto
                .createHmac("sha256", this.signingSecret)
                .update(payload)
                .digest("hex")
                .slice(0, 16);

            const res = await fetch(job.targetUrl, {
                method: "POST",
                headers: {
                    "content-type": "application/json",
                    "x-slice-signature": "sha256=" + signature,
                    "x-webhook-event": job.eventType,
                },
                body: payload,
            });

            if (res.status === 200 && res.ok) {
                const bodyJson = (await res.json()) as { ackId: string; status: string };
                liveHttpDeliveries++;
                deliveryReceipts.push(job.eventId + ":" + bodyJson.ackId + ":" + signature);
            }
        }

        const auditSignaturePrefix = crypto
            .createHash("sha256")
            .update(deliveryReceipts.join("|"))
            .digest("hex")
            .slice(0, 16);

        return {
            app: "13-webhook-orchestrator-ssrf",
            libraries: ["fast-querystring@1.1.2", "ms@2.1.3"],
            totalJobsProcessed: jobs.length,
            liveHttpDeliveries,
            ssrfMetadataBlocked,
            privateCidrBlocked,
            untrustedDomainBlocked,
            retryBackoffMs,
            auditSignaturePrefix,
        };
    }
}

