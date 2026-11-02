import { EventEmitter } from "node:events";
import * as path from "node:path";
import * as util from "node:util";
import assert from "node:assert";
import * as cookie from "cookie";
import ms from "ms";
import bytes from "bytes";
import semver from "semver";
import type { RouteDispatchSummary } from "./types.ts";

export async function runEdgeRouterPipeline(
  rawRoutePath: string,
  cookieHeader: string,
  clientVersion: string
): Promise<RouteDispatchSummary> {
  const bus = new EventEmitter();
  const eventSequence: string[] = [];

  bus.prependListener("route:start", (p: string) => {
    eventSequence.push(`prepend:${p}`);
  });
  bus.on("route:start", (p: string) => {
    eventSequence.push(`on:${p}`);
  });
  bus.once("route:auth", (tenant: string) => {
    eventSequence.push(`auth-once:${tenant}`);
  });
  bus.on("route:complete", (code: number) => {
    eventSequence.push(`done:${code}`);
  });

  // Normalize POSIX route path via node:path
  const normalizedRoute = path.posix.normalize(rawRoutePath);
  const staticRoot = "/var/www/edge/assets";
  const targetAsset = path.posix.join(
    staticRoot,
    "bundles",
    "../bundles/client.v4.mjs"
  );
  const relativeAssetPath = path.posix.relative("/var/www", targetAsset);
  const parsedAsset = path.posix.parse(targetAsset);
  const formattedBack = path.posix.format(parsedAsset);
  assert.strictEqual(formattedBack, targetAsset);

  bus.emit("route:start", normalizedRoute);

  const parsedCookies = cookie.parse(cookieHeader);
  const sessionTenant = parsedCookies["slice_tenant"] || "anonymous";
  bus.emit("route:auth", sessionTenant);
  // Second emit of route:auth must not trigger .once() listener
  bus.emit("route:auth", "ignored-second-auth");

  const ttlMs = Number(ms("15m"));
  const maxBodyBytes = Number(bytes("64kb"));
  const engineSemverSatisfied = Boolean(
    semver.satisfies(clientVersion, ">=2.4.0 <3.0.0")
  );

  // Promisified middleware validator using node:util.promisify
  const validatePolicy = util.promisify(
    (
      tenant: string,
      satisfied: boolean,
      cb: (err: Error | null, status?: number) => void
    ) => {
      if (!tenant || !satisfied) {
        cb(new Error("Policy rejected"));
      } else {
        cb(null, 200);
      }
    }
  );

  const statusCode = await validatePolicy(sessionTenant, engineSemverSatisfied);
  assert.strictEqual(statusCode, 200);
  bus.emit("route:complete", statusCode);

  const formattedAuditLine = util.format(
    "tenant=%s route=%s status=%d asset=%s",
    sessionTenant,
    normalizedRoute,
    statusCode,
    parsedAsset.base
  );

  return {
    normalizedRoute,
    relativeAssetPath,
    parsedBase: parsedAsset.base,
    sessionTenant,
    ttlMs,
    maxBodyBytes,
    engineSemverSatisfied,
    formattedAuditLine,
    eventSequence,
  };
}
