import { Hono } from "hono";
import { getCookie, setCookie } from "hono/cookie";
import { validateAndComputeOrder, verifyTenantToken } from "./schema_and_auth.ts";
import type { CheckoutPayload } from "./types.ts";

export function createEdgeHonoApp(): Hono {
    const app = new Hono();

    // 1. Global Edge Security & Telemetry Middleware
    app.use("*", async (c, next) => {
        const reqId = c.req.header("x-request-id") || "req-default";
        await next();
        c.header("X-Request-Id", reqId);
        c.header("X-Powered-By", "Slice-Hono-v4");
        c.header("X-Content-Type-Options", "nosniff");
    });

    // 2. Tenant Bearer HMAC Guard Middleware on `/api/v1/tenants/:tenantId/*`
    app.use("/api/v1/tenants/:tenantId/*", async (c, next) => {
        const tenantId = c.req.param("tenantId") || "";
        const auth = c.req.header("authorization");
        if (!verifyTenantToken(auth, tenantId)) {
            return c.json(
                {
                    error: "UnauthorizedTenantAccess",
                    tenantId
                },
                401
            );
        }
        await next();
    });

    // 3. Healthcheck endpoint
    app.get("/healthz", (c) => {
        return c.json({
            status: "healthy",
            router: "Hono-v4",
            framework: "hono@4.6.20"
        });
    });

    // 4. Parameterized Order Lookup with `hono/cookie` + Query Normalization
    app.get("/api/v1/tenants/:tenantId/orders/:orderId", (c) => {
        const tenantId = c.req.param("tenantId");
        const orderId = c.req.param("orderId");
        const currency = (c.req.query("currency") || "USD").toUpperCase();
        const ttlSpec = c.req.query("ttl") || "15m";
        const tierDiscount = Number(c.req.query("discountBps") || "500");
        const sessionId = getCookie(c, "slice_sid") || `anon-${tenantId}`;

        setCookie(c, "last_order", orderId, {
            path: "/",
            httpOnly: true,
            sameSite: "Lax"
        });

        const syntheticPayload: CheckoutPayload = {
            customerId: `cust-${tenantId}-${orderId.toLowerCase()}`,
            currency,
            discountBps: tierDiscount,
            items: [
                { sku: "EDGE-COMPUTE-UNIT", qty: 3, unitCents: 1450 },
                { sku: "KV-SHARD-OPS", qty: 2, unitCents: 825 }
            ]
        };

        const receipt = validateAndComputeOrder(
            tenantId,
            orderId,
            sessionId,
            syntheticPayload,
            ttlSpec
        );
        return c.json(receipt, 200);
    });

    // 5. POST Checkout Endpoint with JSON Body Validation
    app.post("/api/v1/tenants/:tenantId/checkout", async (c) => {
        const tenantId = c.req.param("tenantId");
        const orderId = c.req.header("x-idempotency-key") || "ORD-POST-DEFAULT";
        const sessionId = getCookie(c, "slice_sid") || `post-${tenantId}`;
        const body = await c.req.json<CheckoutPayload>();

        setCookie(c, "checkout_committed", orderId, {
            path: "/api/v1",
            httpOnly: true,
            secure: true
        });

        const receipt = validateAndComputeOrder(tenantId, orderId, sessionId, body, "30m");
        return c.json(receipt, 201);
    });

    // 6. Structured 404 Handler
    app.notFound((c) => {
        return c.json(
            {
                error: "RouteNotFound",
                path: c.req.path
            },
            404
        );
    });

    return app;
}

