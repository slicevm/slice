import { destr } from "destr";
import { hash } from "ohash";
import { createRouter } from "radix3";
import { joinURL, normalizeURL, parseQuery, withQuery } from "ufo";
import type { CatalogItemPayload, RadixRoutePolicy, WebhookEventPayload } from "./types.ts";

const policyRouter = createRouter<RadixRoutePolicy>({
    routes: {
        "/v1/catalog/:category/items/:itemId": {
            policyName: "catalog-item-edge-cache",
            cacheTtlSec: 120,
            rateLimitRpm: 600
        },
        "/v1/search": {
            policyName: "catalog-search-dynamic",
            cacheTtlSec: 30,
            rateLimitRpm: 300
        },
        "/v1/webhooks/**:provider": {
            policyName: "webhook-ingest-strict",
            cacheTtlSec: 0,
            rateLimitRpm: 120
        }
    }
});

const CATEGORY_BASE_PRICES: Record<string, number> = {
    gpu: 245000,
    storage: 18900,
    network: 42500,
    security: 12000
};

export function lookupRoutePolicy(pathname: string): RadixRoutePolicy | null {
    return policyRouter.lookup(pathname);
}

export function canonicalizeEndpointUrl(rawUrl: string): {
    normalized: string;
    query: Record<string, any>;
} {
    const normalized = normalizeURL(rawUrl);
    const qIdx = normalized.indexOf("?");
    const search = qIdx !== -1 ? normalized.slice(qIdx + 1) : "";
    const query = parseQuery(search);
    return { normalized, query };
}

export function buildSearchPaginationUrl(basePath: string, category: string, page: number, limit: number): string {
    const joined = joinURL("/v1", basePath);
    return withQuery(joined, {
        category,
        page,
        limit,
        sort: "price_asc"
    });
}

export function buildCatalogItem(
    category: string,
    itemId: string,
    region: string,
    cacheTtlSec: number
): CatalogItemPayload {
    const basePrice = CATEGORY_BASE_PRICES[category] ?? 10000;
    let itemCodeSum = 0;
    for (let i = 0; i < itemId.length; i++) {
        itemCodeSum += itemId.charCodeAt(i) * (i + 1);
    }
    const unitPriceCents = basePrice + (itemCodeSum % 7500);
    const stockUnits = 10 + (itemCodeSum % 90);
    const canonicalPath = joinURL("/v1/catalog", category, "items", itemId);
    const etag = `W/"${hash({ category, itemId, region, unitPriceCents, stockUnits })}"`;

    return {
        category,
        itemId,
        region,
        unitPriceCents,
        stockUnits,
        cacheTtlSec,
        canonicalPath,
        etag
    };
}

export function parseAndHashWebhook(provider: string, rawBody: string): WebhookEventPayload {
    const parsed = destr<{
        eventId?: string;
        eventType?: string;
        amountCents?: number;
        verified?: string | boolean;
    }>(rawBody);

    const verifiedBool = destr<boolean>(String(parsed?.verified ?? "true"));
    const eventId = String(parsed?.eventId || "evt_unknown");
    const eventType = String(parsed?.eventType || "payment.settled");
    const amountCents = Number(parsed?.amountCents || 0);
    const payloadHash = hash({ provider, eventId, eventType, amountCents, verified: verifiedBool });

    return {
        provider,
        eventId,
        eventType,
        amountCents,
        verified: Boolean(verifiedBool),
        payloadHash
    };
}

