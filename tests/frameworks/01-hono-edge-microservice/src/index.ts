import crypto from "node:crypto";
import cookie from "cookie";
import { createEdgeHonoApp } from "./hono_app.ts";
import { signTenantToken } from "./schema_and_auth.ts";
import type { HonoBenchmarkSummary, ValidatedOrderReceipt } from "./types.ts";

const TENANTS = ["acme", "globex", "initech", "umbrella"];
const CURRENCIES = ["USD", "EUR", "GBP", "JPY"];

async function runHonoSuite(): Promise<HonoBenchmarkSummary> {
    const app = createEdgeHonoApp();

    let totalRequestsDispatched = 0;
    let status200Count = 0;
    let status201Count = 0;
    let status401Count = 0;
    let status404Count = 0;
    let cookiesIssued = 0;
    let totalRevenueCents = 0;
    let totalTaxCents = 0;
    let middlewareHeadersVerified = true;
    const signatures: string[] = [];

    // 1. Dispatch 6 GET `/api/v1/tenants/:tenantId/orders/:orderId` requests
    for (let i = 0; i < 6; i++) {
        const tenant = TENANTS[i % TENANTS.length];
        const currency = CURRENCIES[i % CURRENCIES.length];
        const orderId = `ORD-${1000 + i}`;
        const discountBps = (i % 5) * 250;
        const token = signTenantToken(tenant, "operator");
        const sidCookie = cookie.serialize("slice_sid", `sid_${tenant}_${i}`);

        const req = new Request(
            `https://edge.slice.local/api/v1/tenants/${tenant}/orders/${orderId}?currency=${currency}&discountBps=${discountBps}&ttl=15m`,
            {
                method: "GET",
                headers: {
                    Authorization: `Bearer ${token}`,
                    Cookie: sidCookie,
                    "X-Request-Id": `req-get-${i}`
                }
            }
        );

        const res = await app.fetch(req);
        totalRequestsDispatched++;
        if (res.status === 200) {
            status200Count++;
        }
        if (
            res.headers.get("x-powered-by") !== "Slice-Hono-v4" ||
            res.headers.get("x-request-id") !== `req-get-${i}`
        ) {
            middlewareHeadersVerified = false;
        }
        const setCookieHeader = res.headers.get("set-cookie") || "";
        if (setCookieHeader.includes(`last_order=${orderId}`)) {
            cookiesIssued++;
        }
        const receipt = (await res.json()) as ValidatedOrderReceipt;
        totalRevenueCents += receipt.totalCents;
        totalTaxCents += receipt.taxCents;
        signatures.push(receipt.signature);
    }

    // 2. Dispatch 3 POST `/api/v1/tenants/:tenantId/checkout` requests
    for (let i = 0; i < 3; i++) {
        const tenant = TENANTS[i % TENANTS.length];
        const currency = CURRENCIES[(i + 1) % CURRENCIES.length];
        const orderId = `CHK-${2000 + i}`;
        const token = signTenantToken(tenant, "checkout");

        const payload = {
            customerId: `enterprise-${tenant}-${i}`,
            currency,
            discountBps: 400,
            items: [
                { sku: "WASM-ISOLATE-PACK", qty: 2 + (i % 4), unitCents: 3200 },
                { sku: "ZERO-STAT-CAS-TIER", qty: 1, unitCents: 1850 }
            ]
        };

        const req = new Request(
            `https://edge.slice.local/api/v1/tenants/${tenant}/checkout`,
            {
                method: "POST",
                headers: {
                    Authorization: `Bearer ${token}`,
                    "Content-Type": "application/json",
                    "X-Idempotency-Key": orderId,
                    "X-Request-Id": `req-post-${i}`
                },
                body: JSON.stringify(payload)
            }
        );

        const res = await app.fetch(req);
        totalRequestsDispatched++;
        if (res.status === 201) {
            status201Count++;
        }
        const setCookieHeader = res.headers.get("set-cookie") || "";
        if (setCookieHeader.includes(`checkout_committed=${orderId}`)) {
            cookiesIssued++;
        }
        const receipt = (await res.json()) as ValidatedOrderReceipt;
        totalRevenueCents += receipt.totalCents;
        totalTaxCents += receipt.taxCents;
        signatures.push(receipt.signature);
    }

    // 3. Dispatch 1 unauthorized cross-tenant / forged token probe (expect 401)
    for (let i = 0; i < 1; i++) {
        const wrongToken = signTenantToken("globex", "guest");
        const req = new Request(
            `https://edge.slice.local/api/v1/tenants/acme/orders/ORD-UNAUTH-${i}`,
            {
                method: "GET",
                headers: {
                    Authorization: `Bearer ${wrongToken}`,
                    "X-Request-Id": `req-unauth-${i}`
                }
            }
        );
        const res = await app.fetch(req);
        totalRequestsDispatched++;
        if (res.status === 401) {
            status401Count++;
        }
    }

    // 4. Dispatch 1 unknown route probe (expect 404)
    const notFoundRes = await app.fetch(
        new Request("https://edge.slice.local/api/v2/non-existent-endpoint")
    );
    totalRequestsDispatched++;
    if (notFoundRes.status === 404) {
        status404Count++;
    }

    // 5. Dispatch 1 `/healthz` probe
    const healthRes = await app.fetch(new Request("https://edge.slice.local/healthz"));
    totalRequestsDispatched++;
    if (healthRes.status === 200) {
        status200Count++;
    }
    const healthBody = (await healthRes.json()) as { status: string; router: string };

    const auditInput = signatures.join(":") + `|${totalRevenueCents}|${totalTaxCents}`;
    const auditDigest = crypto.createHash("sha256").update(auditInput).digest("hex").slice(0, 16);

    return {
        framework: "01-hono-edge-microservice",
        libraries: ["cookie@0.6.0", "hono@4.6.20", "ms@2.1.3"],
        totalRequestsDispatched,
        status200Count,
        status201Count,
        status401Count,
        status404Count,
        cookiesIssued,
        totalRevenueCents,
        totalTaxCents,
        middlewareHeadersVerified,
        healthStatus: `${healthBody.status}:${healthBody.router}`,
        auditDigest
    };
}

runHonoSuite().then((summary) => {
    console.log(JSON.stringify(summary));
});

