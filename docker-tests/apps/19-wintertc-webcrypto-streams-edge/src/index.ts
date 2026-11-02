import bytes from "bytes";
import * as cookie from "cookie";
import { parse as flattedParse, stringify as flattedStringify } from "flatted";
import ms from "ms";
import { runWebCryptoJwtAndWebhookAudit } from "./webcrypto_jwt_verifier.ts";
import { runWinterTcStreamsAndCloningPipeline } from "./streams_and_formdata_pipeline.ts";

async function main() {
  const ttlMs = ms("15m") as number;
  const parsedCookies = cookie.parse("slice_session=wintertc-edge-session-01; slice_tier=microvm");
  const sessionCookie = parsedCookies.slice_session || "missing";

  const cryptoAudit = await runWebCryptoJwtAndWebhookAudit("slice-phase-n3-wintertc-secret-key", {
    sub: sessionCookie,
    tenant: "tenant-wintertc-19",
    role: "edge-gateway",
    expMs: 1700000000000 + ttlMs
  });

  const streamsAudit = await runWinterTcStreamsAndCloningPipeline();

  // Verify flatted cyclic serialization alongside structuredClone
  const cyclicEnvelope: any = {
    session: sessionCookie,
    crypto: cryptoAudit,
    streams: streamsAudit
  };
  cyclicEnvelope.root = cyclicEnvelope;
  const roundTripped = flattedParse(flattedStringify(cyclicEnvelope));

  const formattedBytes = bytes.format(
    roundTripped.streams.uploadedFileSize + roundTripped.streams.totalTransformedBytes
  );

  // Compute final WebCrypto SHA-256 audit digest
  const enc = new TextEncoder();
  const summaryBytes = enc.encode(
    [
      roundTripped.crypto.sha256PayloadHex,
      roundTripped.streams.transformedPayload,
      roundTripped.streams.uploadedFileName,
      String(roundTripped.streams.cyclicCloneVerified)
    ].join("|")
  );
  const digestAb = await crypto.subtle.digest("SHA-256", summaryBytes);
  const digestU8 = new Uint8Array(digestAb);
  let auditDigest = "";
  for (let i = 0; i < 8; i++) {
    const h = digestU8[i].toString(16);
    auditDigest += h.length === 1 ? "0" + h : h;
  }

  console.log(
    JSON.stringify({
      app: "19-wintertc-webcrypto-streams-edge",
      libraries: ["bytes@3.1.2", "cookie@1.0.2", "flatted@3.3.1", "ms@2.1.3"],
      hs256TokenValid: cryptoAudit.hs256TokenValid,
      hs256TamperedRejected: cryptoAudit.hs256TamperedRejected,
      hs512WebhookValid: cryptoAudit.hs512WebhookValid,
      jwkAlg: cryptoAudit.jwkAlg,
      encodeIntoRead: cryptoAudit.encodeIntoRead,
      encodeIntoWritten: cryptoAudit.encodeIntoWritten,
      transformedPayload: streamsAudit.transformedPayload,
      totalTransformedBytes: streamsAudit.totalTransformedBytes,
      formFieldsCount: streamsAudit.formFieldsCount,
      uploadedFileName: streamsAudit.uploadedFileName,
      uploadedFileSize: streamsAudit.uploadedFileSize,
      slicedHeaderText: streamsAudit.slicedHeaderText,
      cyclicCloneVerified: streamsAudit.cyclicCloneVerified,
      dispatchedEvents: streamsAudit.dispatchedEvents,
      anySignalAbortedBy: streamsAudit.anySignalAbortedBy,
      formattedWireVolume: formattedBytes,
      auditDigest
    })
  );
}

main();
