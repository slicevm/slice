import cookie from "cookie";
import dlv from "dlv";
import { CorsAndEtagGuard } from "./cors_etag.ts";
import type { TenantBffProfile } from "./types.ts";

const TENANT_PROFILES: Record<string, TenantBffProfile> = {
  "tenant-alpha": {
    tenantId: "tenant-alpha",
    tier: "enterprise",
    region: "us-east-1",
    cacheTtlWindow: "15m",
    features: { aiCopilot: true, realtimeAnalytics: true, maxWidgets: 8 },
  },
  "tenant-beta": {
    tenantId: "tenant-beta",
    tier: "growth",
    region: "eu-central-1",
    cacheTtlWindow: "5m",
    features: { aiCopilot: true, realtimeAnalytics: false, maxWidgets: 5 },
  },
  "tenant-gamma": {
    tenantId: "tenant-gamma",
    tier: "starter",
    region: "ap-northeast-1",
    cacheTtlWindow: "1m",
    features: { aiCopilot: false, realtimeAnalytics: false, maxWidgets: 3 },
  },
};

export class BffController {
  public guard: CorsAndEtagGuard;

  constructor() {
    this.guard = new CorsAndEtagGuard();
  }

  public dispatch(req: Request): Response {
    if (req.method === "OPTIONS") {
      return this.guard.buildPreflightResponse(req);
    }

    const origin = req.headers.get("origin");
    if (origin && !this.guard.isOriginAllowed(origin)) {
      return Response.json({ ok: false, error: "origin_forbidden" }, { status: 403 });
    }

    const url = new URL(req.url, "https://bff.slice.dev");
    const cookieHeader = req.headers.get("cookie") || "";
    const parsedCookies = cookie.parse(cookieHeader);
    const sessionUser = parsedCookies["slice_uid"] || "guest";
    const themeCookie = parsedCookies["slice_theme"] || "dark";

    const tenantKey = req.headers.get("x-tenant-id") || "tenant-alpha";
    const profile = TENANT_PROFILES[tenantKey] || TENANT_PROFILES["tenant-gamma"];

    if (req.method === "GET" && url.pathname === "/bff/v1/dashboard") {
      const viewMode = url.searchParams.get("view") || "compact";
      const aiEnabled = Boolean(dlv(profile, "features.aiCopilot", false));
      const maxWidgets = Number(dlv(profile, "features.maxWidgets", 3));
      const region = String(dlv(profile, "region", "us-east-1"));

      const payloadObj = {
        ok: true,
        tenantId: profile.tenantId,
        user: sessionUser,
        theme: themeCookie,
        viewMode,
        region,
        aiEnabled,
        widgets: maxWidgets,
      };
      const serialized = JSON.stringify(payloadObj);
      const etag = this.guard.computeEtag(serialized);
      const cacheInfo = this.guard.buildCacheControlHeader(profile.cacheTtlWindow);

      const ifNoneMatch = req.headers.get("if-none-match");
      if (ifNoneMatch && ifNoneMatch === etag) {
        return new Response(null, {
          status: 304,
          headers: {
            etag,
            "cache-control": cacheInfo.headerValue,
            "x-cache-ttl": String(cacheInfo.ttlSec),
          },
        });
      }

      const setCookieVal = cookie.serialize("slice_last_view", viewMode, {
        path: "/bff",
        httpOnly: true,
        secure: true,
        sameSite: "lax",
        maxAge: cacheInfo.ttlSec,
      });

      return new Response(serialized, {
        status: 200,
        headers: {
          "content-type": "application/json",
          etag,
          "cache-control": cacheInfo.headerValue,
          "x-cache-ttl": String(cacheInfo.ttlSec),
          "x-widget-count": String(maxWidgets),
          "set-cookie": setCookieVal,
        },
      });
    }

    if (req.method === "POST" && url.pathname === "/bff/v1/preferences") {
      return Response.json(
        {
          ok: true,
          tenantId: profile.tenantId,
          user: sessionUser,
          updatedTheme: themeCookie,
        },
        { status: 201 }
      );
    }

    return Response.json({ ok: false, error: "not_found" }, { status: 404 });
  }
}
