import crypto from "node:crypto";
import { CatalogService } from "./catalog.ts";

const WEBHOOK_SECRET = "wintertc-edge-hmac-secret-2026";

export class EdgeWebRouter {
  private catalog: CatalogService;

  constructor() {
    this.catalog = new CatalogService();
  }

  public signPayload(tenantId: string, body: string): string {
    return crypto.createHmac("sha256", WEBHOOK_SECRET).update(tenantId + ":" + body).digest("hex");
  }

  public handle(req: Request): Response {
    const url = new URL(req.url, "https://edge.slice.dev");
    const tenantId = req.headers.get("x-tenant-id") || "anonymous";
    const apiKey = req.headers.get("x-api-key") || "";

    if (url.pathname === "/legacy/v1/docs") {
      const target = new URL("/api/v2/catalog", url.origin);
      target.searchParams.set("category", "compute");
      target.searchParams.set("from", "legacy");
      return Response.redirect(target.href, 308);
    }

    if (!apiKey.startsWith("sk_edge_")) {
      return Response.json(
        { ok: false, error: "unauthorized", tenantId },
        { status: 401, headers: { "x-edge-tenant": tenantId } }
      );
    }

    if (req.method === "GET" && url.pathname === "/api/v2/catalog") {
      const result = this.catalog.queryCatalog(url.searchParams);
      const resHeaders = new Headers({
        "x-edge-tenant": tenantId,
        "x-canonical-query": result.canonicalQuery,
        "x-item-count": String(result.items.length),
      });
      return Response.json(
        {
          ok: true,
          tenantId,
          count: result.items.length,
          canonicalQuery: result.canonicalQuery,
          skus: result.items.map((it) => it.sku),
        },
        { status: 200, headers: resHeaders }
      );
    }

    if (req.method === "POST" && url.pathname === "/api/v2/orders") {
      const rawPayload = req.headers.get("x-order-payload") || "";
      const signature = req.headers.get("x-signature") || "";
      const expectedSig = this.signPayload(tenantId, rawPayload);
      if (signature !== expectedSig) {
        return Response.json(
          { ok: false, error: "invalid_signature", tenantId },
          { status: 401 }
        );
      }

      const sku = url.searchParams.get("sku") || "";
      const quantity = Number(url.searchParams.get("qty") || "1");
      const customerTier = url.searchParams.get("tier") || "standard";
      const item = this.catalog.findBySku(sku);
      if (!item) {
        return Response.json(
          { ok: false, error: "sku_not_found", sku },
          { status: 404 }
        );
      }

      const discountBps = customerTier === "enterprise" ? 1000 : 0;
      const grossCents = item.priceCents * quantity;
      const totalCents = grossCents - Math.floor((grossCents * discountBps) / 10000);

      return Response.json(
        {
          ok: true,
          tenantId,
          sku: item.sku,
          quantity,
          totalCents,
        },
        {
          status: 201,
          headers: {
            "x-edge-tenant": tenantId,
            "x-order-sku": item.sku,
            "x-order-total-cents": String(totalCents),
          },
        }
      );
    }

    return Response.json(
      { ok: false, error: "route_not_found", path: url.pathname },
      { status: 404 }
    );
  }
}
