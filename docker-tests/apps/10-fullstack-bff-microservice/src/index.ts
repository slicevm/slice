import crypto from "node:crypto";
import { BffController } from "./bff_controller.ts";
import type { BffBenchmarkSummary } from "./types.ts";

const controller = new BffController();

async function runBffBenchmark(): Promise<void> {
  const tenants = ["tenant-alpha", "tenant-beta", "tenant-gamma"];
  const origins = [
    "https://app.slice.dev",
    "https://console.slice.dev",
    "https://evil.example.org",
  ];

  let corsPreflight204Count = 0;
  let notModified304Count = 0;
  let dashboard200Count = 0;
  let preferences201Count = 0;
  let forbidden403Count = 0;
  let totalWidgetsRendered = 0;
  let totalCacheTtlSec = 0;
  let lastEtag = "";
  let lastDashboardUrl = "";
  let lastDashboardTenant = "";
  let lastDashboardCookie = "";
  let sample200Response: Response | null = null;
  const stateHasher = crypto.createHash("sha256");

  const totalRequests = 60;

  for (let i = 0; i < totalRequests; i++) {
    const tenantId = tenants[i % tenants.length];
    const mode = i % 6;
    let req: Request;

    if (mode === 0) {
      const origin = origins[i % origins.length];
      req = new Request("https://bff.slice.dev/bff/v1/dashboard", {
        method: "OPTIONS",
        headers: { Origin: origin },
      });
    } else if (mode === 1 || mode === 2) {
      const view = i % 2 === 0 ? "expanded" : "compact";
      const u = new URL("/bff/v1/dashboard", "https://bff.slice.dev");
      u.searchParams.set("view", view);
      const cookieStr = `slice_uid=usr_${(i % 5) + 100}; slice_theme=${i % 2 === 0 ? "dark" : "light"}`;
      lastDashboardUrl = u.href;
      lastDashboardTenant = tenantId;
      lastDashboardCookie = cookieStr;

      req = new Request(u.href, {
        method: "GET",
        headers: {
          Origin: "https://app.slice.dev",
          "X-Tenant-Id": tenantId,
          Cookie: cookieStr,
        },
      });
    } else if (mode === 3 && lastEtag) {
      req = new Request(lastDashboardUrl, {
        method: "GET",
        headers: {
          Origin: "https://app.slice.dev",
          "X-Tenant-Id": lastDashboardTenant,
          Cookie: lastDashboardCookie,
          "If-None-Match": lastEtag,
        },
      });
    } else if (mode === 4) {
      req = new Request("https://bff.slice.dev/bff/v1/preferences", {
        method: "POST",
        headers: {
          Origin: "https://console.slice.dev",
          "X-Tenant-Id": tenantId,
          Cookie: `slice_uid=usr_${i}; slice_theme=solarized`,
          "Content-Type": "application/json",
        },
        body: JSON.stringify({ notifications: true }),
      });
    } else {
      req = new Request("https://bff.slice.dev/bff/v1/dashboard?view=grid", {
        method: "GET",
        headers: {
          Origin: "https://evil.example.org",
          "X-Tenant-Id": tenantId,
        },
      });
    }

    const res = controller.dispatch(req);
    stateHasher.update(`${i}:${res.status}:${res.ok}`);

    if (res.status === 204) {
      corsPreflight204Count += 1;
    } else if (res.status === 304) {
      notModified304Count += 1;
      totalCacheTtlSec += Number(res.headers.get("x-cache-ttl") || "0");
    } else if (res.status === 200) {
      dashboard200Count += 1;
      lastEtag = res.headers.get("etag") || "";
      totalCacheTtlSec += Number(res.headers.get("x-cache-ttl") || "0");
      totalWidgetsRendered += Number(res.headers.get("x-widget-count") || "0");
      if (!sample200Response) {
        sample200Response = res;
      }
    } else if (res.status === 201) {
      preferences201Count += 1;
    } else if (res.status === 403) {
      forbidden403Count += 1;
    }
  }

  if (sample200Response) {
    const parsed = (await sample200Response.json()) as { tenantId: string };
    stateHasher.update(parsed.tenantId);
  }

  const summary: BffBenchmarkSummary = {
    app: "10-fullstack-bff-microservice",
    totalRequests,
    corsPreflight204Count,
    notModified304Count,
    dashboard200Count,
    preferences201Count,
    forbidden403Count,
    totalWidgetsRendered,
    totalCacheTtlSec,
    finalStateHash: stateHasher.digest("hex").slice(0, 16),
  };

  console.log(JSON.stringify(summary));
}

runBffBenchmark();

