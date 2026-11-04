import { describe, test, expect } from "slice:test";
import { createIttyUnjsGateway, type GatewayTelemetry } from "../src/itty_gateway.ts";

describe("itty-router v5 + UnJS Edge Stack Suite", () => {
    const telemetry: GatewayTelemetry = {
        radixPolicyMatches: 0,
        normalizedUrlsCount: 0,
        destrPayloadsParsed: 0
    };
    const router = createIttyUnjsGateway(telemetry);

    test("matches catalog route via itty-router and radix3 policy and short-circuits 304 ETag", async () => {
        const url = "https://edge.slice.local/v1/catalog/gpu/items/sku-h100?region=us-east-1";
        const res1 = await router.fetch(new Request(url));
        expect(res1.status).toBe(200);
        expect(res1.headers.get("x-radix-policy")).toBe("catalog-item-edge-cache");
        const etag = res1.headers.get("etag") || "";
        expect(etag.startsWith('W/"')).toBe(true);

        const res2 = await router.fetch(
            new Request(url, {
                headers: { "If-None-Match": etag }
            })
        );
        expect(res2.status).toBe(304);
    });

    test("parses webhook payload via destr and hashes with ohash", async () => {
        const res = await router.fetch(
            new Request("https://edge.slice.local/v1/webhooks/stripe", {
                method: "POST",
                body: JSON.stringify({
                    eventId: "evt_test_1",
                    eventType: "invoice.paid",
                    amountCents: 5000,
                    verified: "true"
                })
            })
        );
        expect(res.status).toBe(202);
        const body = await res.json();
        expect(body.verified).toBe(true);
        expect(body.amountCents).toBe(5000);
    });
});

