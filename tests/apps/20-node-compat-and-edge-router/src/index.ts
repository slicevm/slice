import assert from "node:assert";
import * as util from "node:util";
import { encodeAndVerifyWireFrame } from "./binary_wire_codec.ts";
import { runEdgeRouterPipeline } from "./router_and_events_bus.ts";

async function main() {
  const wireFrame = encodeAndVerifyWireFrame(
    9007199254740993n,
    0x0102,
    '{"rpc":"edge.dispatch","region":"iad-01","currency":"\u20ac"}',
    "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
  );

  assert.strictEqual(wireFrame.magicHex, "534c4334");
  assert.strictEqual(wireFrame.version, 4);
  assert.strictEqual(wireFrame.flags, 258);
  assert.strictEqual(wireFrame.sequenceId, "9007199254740993");
  assert.strictEqual(wireFrame.isValidBufferInstance, true);

  const routeSummary = await runEdgeRouterPipeline(
    "/api/v2/edge/../ tenants/./dispatch//invoke",
    "slice_tenant=acme-fintech-eu; theme=dark",
    "2.6.1"
  );

  assert.strictEqual(
    routeSummary.normalizedRoute,
    "/api/v2/ tenants/dispatch/invoke"
  );
  assert.strictEqual(
    routeSummary.relativeAssetPath,
    "edge/assets/bundles/client.v4.mjs"
  );
  assert.strictEqual(routeSummary.sessionTenant, "acme-fintech-eu");
  assert.strictEqual(routeSummary.ttlMs, 900000);
  assert.strictEqual(routeSummary.maxBodyBytes, 65536);
  assert.strictEqual(routeSummary.engineSemverSatisfied, true);
  assert.deepStrictEqual(routeSummary.eventSequence, [
    "prepend:/api/v2/ tenants/dispatch/invoke",
    "on:/api/v2/ tenants/dispatch/invoke",
    "auth-once:acme-fintech-eu",
    "done:200",
  ]);

  const verifyBuf = Buffer.from(wireFrame.payloadBase64Url, "base64url");
  assert.strictEqual(Buffer.isBuffer(verifyBuf), true);
  assert.strictEqual(util.types.isTypedArray(verifyBuf), true);
  assert.strictEqual(util.types.isUint8Array(verifyBuf), true);
  assert.strictEqual(verifyBuf.toString("utf8"), wireFrame.payloadUtf8);

  console.log(
    JSON.stringify({
      suite: "20-node-compat-and-edge-router",
      wireFrame,
      routeSummary,
    })
  );
}

main();
