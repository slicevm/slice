import crypto from "node:crypto";
import { createIttyUnjsGateway, type GatewayTelemetry } from "./itty_gateway.ts";
import type { CatalogItemPayload, UnjsBenchmarkSummary, WebhookEventPayload } from "./types.ts";

const CATEGORIES = ["gpu", "storage", "network", "security"];
const REGIONS = ["us-east-1", "eu-central-1", "ap-northeast-1"];
const PROVIDERS = ["stripe", "adyen", "github"];

async function runUnjsSuite(): Promise<UnjsBenchmarkSummary> {
    const telemetry: GatewayTelemetry = {
        radixPolicyMatches: 0,
        normalizedUrlsCount: 0,
        destrPayloadsParsed: 0
    };
    const router = createIttyUnjsGateway(telemetry);

    let totalRequestsDispatched = 0;
    let status200Count = 0;
    let status202Count = 0;
    let status304Count = 0;
    let status404Count = 0;
    let totalCatalogValueCents = 0;
    let totalWebhookAmountCents = 0;
    let sampleCanonicalSearchUrl = "";
    const seenEtags = new Set<string>();
    const auditParts: string[] = [];

    // 1. Dispatch 5 Catalog GET requests (200 OK) + 2 Conditional ETag Revalidations (304 Not Modified)
    for (let i = 0; i < 5; i++) {
        const category = CATEGORIES[i % CATEGORIES.length];
        const region = REGIONS[i % REGIONS.length];
        const itemId = `sku-${100 + i}`;
        const url = `https://edge.slice.local/v1/catalog/${category}/items/${itemId}?region=${region}`;

        const res = await router.fetch(new Request(url));
        totalRequestsDispatched++;
        if (res.status === 200) {
            status200Count++;
        }
        const item = (await res.json()) as CatalogItemPayload;
        seenEtags.add(item.etag);
        totalCatalogValueCents += item.unitPriceCents * item.stockUnits;
        auditParts.push(`${item.itemId}:${item.etag}`);

        // For the first 2 items, send a follow-up request with `If-None-Match`
        if (i < 2) {
            const condRes = await router.fetch(
                new Request(url, {
                    headers: { "If-None-Match": item.etag }
                })
            );
            totalRequestsDispatched++;
            if (condRes.status === 304) {
                status304Count++;
            }
        }
    }

    // 2. Dispatch 3 Webhook POST requests parsed via `destr` & hashed via `ohash`
    for (let i = 0; i < 3; i++) {
        const provider = PROVIDERS[i % PROVIDERS.length];
        const rawBody = JSON.stringify({
            eventId: `evt_${3000 + i}`,
            eventType: i % 2 === 0 ? "invoice.paid" : "deployment.ready",
            amountCents: 1250 * (i + 1),
            verified: "true"
        });

        const res = await router.fetch(
            new Request(`https://edge.slice.local/v1/webhooks/${provider}`, {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: rawBody
            })
        );
        totalRequestsDispatched++;
        if (res.status === 202) {
            status202Count++;
        }
        const event = (await res.json()) as WebhookEventPayload;
        totalWebhookAmountCents += event.amountCents;
        auditParts.push(`${event.eventId}:${event.payloadHash}`);
    }

    // 3. Dispatch 1 Search GET request (`ufo` `withQuery` + `joinURL`)
    for (let i = 0; i < 1; i++) {
        const category = CATEGORIES[i % CATEGORIES.length];
        const res = await router.fetch(
            new Request(`https://edge.slice.local/v1/search?category=${category}&page=${i + 1}&limit=25`)
        );
        totalRequestsDispatched++;
        if (res.status === 200) {
            status200Count++;
        }
        const searchBody = (await res.json()) as { nextUrl: string };
        if (i === 0) {
            sampleCanonicalSearchUrl = searchBody.nextUrl;
        }
    }

    // 4. Dispatch 1 Unknown Route Probe (404)
    const missingRes = await router.fetch(new Request("https://edge.slice.local/v2/unknown"));
    totalRequestsDispatched++;
    if (missingRes.status === 404) {
        status404Count++;
    }

    const auditInput = auditParts.join("|") + `|${totalCatalogValueCents}|${totalWebhookAmountCents}`;
    const auditDigest = crypto.createHash("sha256").update(auditInput).digest("hex").slice(0, 16);

    return {
        framework: "02-itty-router-unjs-nitro-stack",
        libraries: [
            "destr@2.0.3",
            "itty-router@5.0.18",
            "ohash@1.1.4",
            "radix3@1.1.2",
            "ufo@1.5.4"
        ],
        totalRequestsDispatched,
        status200Count,
        status202Count,
        status304Count,
        status404Count,
        radixPolicyMatches: telemetry.radixPolicyMatches,
        normalizedUrlsCount: telemetry.normalizedUrlsCount,
        destrPayloadsParsed: telemetry.destrPayloadsParsed,
        uniqueEtagsGenerated: seenEtags.size,
        totalCatalogValueCents,
        totalWebhookAmountCents,
        sampleCanonicalSearchUrl,
        auditDigest
    };
}

runUnjsSuite().then((summary) => {
    console.log(JSON.stringify(summary));
});

