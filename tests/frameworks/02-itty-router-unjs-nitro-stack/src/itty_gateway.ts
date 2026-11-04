import { Router } from "itty-router";
import {
    buildCatalogItem,
    buildSearchPaginationUrl,
    canonicalizeEndpointUrl,
    lookupRoutePolicy,
    parseAndHashWebhook
} from "./unjs_pipeline.ts";

export interface GatewayTelemetry {
    radixPolicyMatches: number;
    normalizedUrlsCount: number;
    destrPayloadsParsed: number;
}

export function createIttyUnjsGateway(telemetry: GatewayTelemetry) {
    const router = Router();

    // 1. Upstream UnJS `ufo` + `radix3` Middleware
    router.all("*", (req: any) => {
        const parsedUrl = new URL(req.url);
        const canon = canonicalizeEndpointUrl(parsedUrl.pathname + parsedUrl.search);
        telemetry.normalizedUrlsCount++;
        req.canonQuery = canon.query;

        const policy = lookupRoutePolicy(parsedUrl.pathname);
        if (policy) {
            telemetry.radixPolicyMatches++;
            req.routePolicy = policy;
        }
    });

    // 2. Catalog Item Endpoint with `ohash` ETag Conditional `304 Not Modified`
    router.get("/v1/catalog/:category/items/:itemId", (req: any) => {
        const category = String(req.params.category || "gpu");
        const itemId = String(req.params.itemId || "item-0");
        const region = String(req.canonQuery?.region || req.query?.region || "us-east-1");
        const cacheTtlSec = Number(req.routePolicy?.cacheTtlSec ?? 60);

        const item = buildCatalogItem(category, itemId, region, cacheTtlSec);
        const ifNoneMatch = req.headers.get("if-none-match");

        if (ifNoneMatch && ifNoneMatch === item.etag) {
            return new Response(null, {
                status: 304,
                headers: {
                    ETag: item.etag,
                    "X-Radix-Policy": String(req.routePolicy?.policyName || "none")
                }
            });
        }

        return Response.json(item, {
            status: 200,
            headers: {
                ETag: item.etag,
                "Cache-Control": `public, max-age=${cacheTtlSec}`,
                "X-Radix-Policy": String(req.routePolicy?.policyName || "none")
            }
        });
    });

    // 3. Catalog Search Endpoint with `ufo` `withQuery` + `joinURL`
    router.get("/v1/search", (req: any) => {
        const category = String(req.canonQuery?.category || "gpu");
        const page = Number(req.canonQuery?.page || 1);
        const limit = Number(req.canonQuery?.limit || 20);
        const nextUrl = buildSearchPaginationUrl("search", category, page + 1, limit);

        return Response.json(
            {
                category,
                page,
                limit,
                nextUrl,
                policyName: String(req.routePolicy?.policyName || "catalog-search-dynamic")
            },
            { status: 200 }
        );
    });

    // 4. Webhook Ingestion Endpoint with `destr` + `ohash`
    router.post("/v1/webhooks/:provider", async (req: any) => {
        const provider = String(req.params.provider || "stripe");
        const rawText = await req.text();
        telemetry.destrPayloadsParsed++;
        const event = parseAndHashWebhook(provider, rawText);

        return Response.json(event, {
            status: 202,
            headers: {
                "X-Webhook-Hash": event.payloadHash,
                "X-Radix-Policy": String(req.routePolicy?.policyName || "none")
            }
        });
    });

    // 5. Fallback 404 Route
    router.all("*", () => {
        return Response.json({ error: "NotFound" }, { status: 404 });
    });

    return router;
}

