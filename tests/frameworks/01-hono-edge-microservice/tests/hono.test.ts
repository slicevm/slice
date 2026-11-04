import { describe, test, expect } from "slice:test";
import { createEdgeHonoApp } from "../src/hono_app.ts";
import { signTenantToken } from "../src/schema_and_auth.ts";

describe("Hono v4 Edge Microservice Suite", () => {
    const app = createEdgeHonoApp();

    test("routes parameterized GET order and sets HttpOnly cookie", async () => {
        const token = signTenantToken("acme", "operator");
        const res = await app.fetch(
            new Request("https://edge.slice.local/api/v1/tenants/acme/orders/ORD-777?currency=EUR&discountBps=500", {
                headers: {
                    Authorization: `Bearer ${token}`,
                    Cookie: "slice_sid=sess_test_1",
                    "X-Request-Id": "test-req-1"
                }
            })
        );
        expect(res.status).toBe(200);
        expect(res.headers.get("x-powered-by")).toBe("Slice-Hono-v4");
        const body = await res.json();
        expect(body.tenantId).toBe("acme");
        expect(body.orderId).toBe("ORD-777");
        expect(body.currency).toBe("EUR");
        expect(body.sessionId).toBe("sess_test_1");
    });

    test("blocks cross-tenant bearer token with 401 Unauthorized", async () => {
        const globexToken = signTenantToken("globex", "operator");
        const res = await app.fetch(
            new Request("https://edge.slice.local/api/v1/tenants/acme/orders/ORD-999", {
                headers: {
                    Authorization: `Bearer ${globexToken}`
                }
            })
        );
        expect(res.status).toBe(401);
    });
});

