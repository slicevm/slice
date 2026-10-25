import { WebhookOrchestrator } from "./dispatcher.ts";
import type { WebhookJob } from "./types.ts";

async function runWebhookBenchmark(): Promise<void> {
    const orchestrator = new WebhookOrchestrator(
        ["127.0.0.1:18095"],
        "whsec_slice_f3_signing_key_2026",
    );

    const eventTypes: Array<WebhookJob["eventType"]> = [
        "invoice.paid",
        "subscription.renewed",
        "payout.settled",
    ];
    const jobs: WebhookJob[] = [];

    for (let i = 0; i < 40; i++) {
        const mode = i % 4;
        let targetUrl: string;
        if (mode === 0) {
            targetUrl = "http://127.0.0.1:18095/upstream/webhook?channel=fintech&seq=" + i;
        } else if (mode === 1) {
            targetUrl =
                i % 8 === 1
                    ? "http://169.254.169.254/latest/meta-data/iam/security-credentials/?channel=aws"
                    : "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/?channel=gcp";
        } else if (mode === 2) {
            targetUrl =
                i % 8 === 2
                    ? "http://10.0.0.15:6379/internal/redis?channel=cache"
                    : "http://192.168.1.50:8080/admin/config?channel=lan";
        } else {
            targetUrl = "https://exfil.evil-collector.org/collect?channel=rogue&id=" + i;
        }

        jobs.push({
            eventId: "evt_" + (1000 + i),
            tenantId: "tenant-" + (i % 5),
            targetUrl,
            eventType: eventTypes[i % eventTypes.length],
            amountCents: (i + 1) * 250,
            retryWindow: "2s",
        });
    }

    const summary = await orchestrator.processBatch(jobs);
    console.log(JSON.stringify(summary));
}

runWebhookBenchmark();
