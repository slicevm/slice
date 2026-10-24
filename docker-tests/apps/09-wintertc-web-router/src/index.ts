import crypto from "node:crypto";
import { EdgeWebRouter } from "./router.ts";
import type { RouterBenchmarkSummary } from "./types.ts";

const router = new EdgeWebRouter();

async function runBenchmarkSuite(): Promise<void> {
  const categories = ["compute", "network", "storage", "all"];
  const skus = ["GPU-H100-80G", "GPU-B200-192G", "NIC-CX7-400G", "NVME-GEN5-15T", "UNKNOWN-SKU"];

  let ok200Count = 0;
  let created201Count = 0;
  let redirect308Count = 0;
  let unauthorized401Count = 0;
  let notFound404Count = 0;
  let totalRevenueCents = 0;
  let matchedCatalogItems = 0;
  let canonicalQuerySample = "";
  let firstOkResponse: Response | null = null;
  const stateHasher = crypto.createHash("sha256");

  const totalRequests = 80;

  for (let i = 0; i < totalRequests; i++) {
    const tenantId = `tenant-${(i % 4) + 1}`;
    const mode = i % 8;
    let req: Request;

    if (mode === 0 || mode === 1 || mode === 2) {
      const cat = categories[i % categories.length];
      const minPrice = (i % 3) * 15000;
      const sort = i % 2 === 0 ? "asc" : "desc";
      const reqUrl = new URL("/api/v2/catalog", "https://edge.slice.dev");
      reqUrl.searchParams.set("category", cat);
      reqUrl.searchParams.set("minPrice", String(minPrice));
      reqUrl.searchParams.set("sort", sort);
      if (i % 4 === 0) {
        reqUrl.searchParams.append("tag", "pcie");
      }
      req = new Request(reqUrl, {
        method: "GET",
        headers: {
          "X-Tenant-Id": tenantId,
          "X-Api-Key": `sk_edge_valid_${tenantId}`,
        },
      });
    } else if (mode === 3 || mode === 4) {
      const sku = skus[i % skus.length];
      const quantity = (i % 3) + 1;
      const customerTier = i % 2 === 0 ? "enterprise" : "standard";
      const bodyStr = `${sku}:${quantity}:${customerTier}`;
      const sig = router.signPayload(tenantId, bodyStr);
      const orderUrl = new URL("/api/v2/orders", "https://edge.slice.dev");
      orderUrl.searchParams.set("sku", sku);
      orderUrl.searchParams.set("qty", String(quantity));
      orderUrl.searchParams.set("tier", customerTier);
      req = new Request(orderUrl, {
        method: "POST",
        headers: new Headers({
          "Content-Type": "application/json",
          "X-Tenant-Id": tenantId,
          "X-Api-Key": `sk_edge_valid_${tenantId}`,
          "X-Order-Payload": bodyStr,
          "X-Signature": sig,
        }),
        body: bodyStr,
      });
    } else if (mode === 5) {
      req = new Request("https://edge.slice.dev/legacy/v1/docs", {
        method: "GET",
        headers: { "X-Tenant-Id": tenantId },
      });
    } else if (mode === 6) {
      req = new Request("https://edge.slice.dev/api/v2/catalog?category=compute", {
        method: "GET",
        headers: {
          "X-Tenant-Id": tenantId,
          "X-Api-Key": "invalid_token",
        },
      });
    } else {
      req = new Request(`https://edge.slice.dev/api/v2/missing-route-${i}`, {
        method: "GET",
        headers: {
          "X-Tenant-Id": tenantId,
          "X-Api-Key": `sk_edge_valid_${tenantId}`,
        },
      });
    }

    const res = router.handle(req);
    stateHasher.update(`${i}:${res.status}:${res.ok}`);

    if (res.status === 200) {
      ok200Count += 1;
      matchedCatalogItems += Number(res.headers.get("x-item-count") || "0");
      if (!firstOkResponse) {
        firstOkResponse = res;
      }
    } else if (res.status === 201) {
      created201Count += 1;
      totalRevenueCents += Number(res.headers.get("x-order-total-cents") || "0");
    } else if (res.status === 308) {
      redirect308Count += 1;
      stateHasher.update(res.headers.get("location") || "");
    } else if (res.status === 401) {
      unauthorized401Count += 1;
    } else if (res.status === 404) {
      notFound404Count += 1;
    }
  }

  if (firstOkResponse) {
    const parsed = (await firstOkResponse.json()) as { canonicalQuery: string };
    canonicalQuerySample = parsed.canonicalQuery;
  }

  const summary: RouterBenchmarkSummary = {
    app: "09-wintertc-web-router",
    totalRequests,
    ok200Count,
    created201Count,
    redirect308Count,
    unauthorized401Count,
    notFound404Count,
    totalRevenueCents,
    matchedCatalogItems,
    canonicalQuerySample,
    finalStateHash: stateHasher.digest("hex").slice(0, 16),
  };

  console.log(JSON.stringify(summary));
}

runBenchmarkSuite();
