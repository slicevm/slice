import crypto from "node:crypto";
import ms from "ms";

const ALLOWED_ORIGINS = [
  "https://app.slice.dev",
  "https://console.slice.dev",
  "https://admin.slice.dev",
];

export class CorsAndEtagGuard {
  public isOriginAllowed(origin: string | null): boolean {
    if (!origin) return true;
    return ALLOWED_ORIGINS.includes(origin);
  }

  public buildPreflightResponse(req: Request): Response {
    const origin = req.headers.get("origin") || "";
    if (!this.isOriginAllowed(origin)) {
      return Response.json({ ok: false, error: "cors_origin_denied" }, { status: 403 });
    }
    const maxAgeSec = Math.floor(ms("2h") / 1000);
    return new Response(null, {
      status: 204,
      headers: {
        "access-control-allow-origin": origin,
        "access-control-allow-methods": "GET, POST, OPTIONS",
        "access-control-allow-headers": "content-type, cookie, if-none-match, x-tenant-id",
        "access-control-max-age": String(maxAgeSec),
      },
    });
  }

  public computeEtag(payloadStr: string): string {
    const digest = crypto.createHash("sha256").update(payloadStr).digest("hex").slice(0, 16);
    return `"W/${digest}"`;
  }

  public buildCacheControlHeader(ttlWindow: string): { headerValue: string; ttlSec: number } {
    const ttlMs = ms(ttlWindow);
    const ttlSec = Math.floor(ttlMs / 1000);
    return {
      headerValue: `private, max-age=${ttlSec}, stale-while-revalidate=30`,
      ttlSec,
    };
  }
}

