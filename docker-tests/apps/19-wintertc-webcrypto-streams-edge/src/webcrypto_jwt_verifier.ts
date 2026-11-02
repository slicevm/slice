import type { JwtClaims, WebCryptoAuditResult } from "./types.ts";

function toBase64Url(bytes: Uint8Array): string {
  let binary = "";
  for (let i = 0; i < bytes.byteLength; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function fromBase64Url(b64url: string): Uint8Array {
  let b64 = b64url.replace(/-/g, "+").replace(/_/g, "/");
  while (b64.length % 4 !== 0) {
    b64 += "=";
  }
  const binary = atob(b64);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    out[i] = binary.charCodeAt(i);
  }
  return out;
}

function arrayBufferToHex(ab: ArrayBuffer): string {
  const u8 = new Uint8Array(ab);
  let hex = "";
  for (let i = 0; i < u8.length; i++) {
    const h = u8[i].toString(16);
    hex += h.length === 1 ? "0" + h : h;
  }
  return hex;
}

export async function runWebCryptoJwtAndWebhookAudit(
  secretKeyMaterial: string,
  claims: JwtClaims
): Promise<WebCryptoAuditResult> {
  const encoder = new TextEncoder();
  const decoder = new TextDecoder("utf-8", { fatal: true });

  // 1. Test encodeInto on fixed 16-byte buffer with multi-byte UTF-8 sequence
  const intoTarget = new Uint8Array(10);
  const intoStats = encoder.encodeInto("Edge-\u20ac-Gateway", intoTarget);

  // 2. Import raw HMAC-SHA256 key, export to JWK, and re-import JWK for verification
  const rawKeyBytes = encoder.encode(secretKeyMaterial);
  const signKey = await crypto.subtle.importKey(
    "raw",
    rawKeyBytes,
    { name: "HMAC", hash: "SHA-256" },
    true,
    ["sign", "verify"]
  );
  const exportedJwk = (await crypto.subtle.exportKey("jwk", signKey)) as {
    kty: string;
    k: string;
    alg: string;
  };
  const verifyKey = await crypto.subtle.importKey(
    "jwk",
    exportedJwk,
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["verify"]
  );

  // 3. Sign & verify HS256 JWT
  const headerB64 = toBase64Url(encoder.encode(JSON.stringify({ alg: "HS256", typ: "JWT" })));
  const payloadB64 = toBase64Url(encoder.encode(JSON.stringify(claims)));
  const signingInput = headerB64 + "." + payloadB64;
  const sigAb = await crypto.subtle.sign("HMAC", signKey, encoder.encode(signingInput));
  const sigB64 = toBase64Url(new Uint8Array(sigAb));

  const decodedPayloadJson = decoder.decode(fromBase64Url(payloadB64));
  const parsedClaims = JSON.parse(decodedPayloadJson) as JwtClaims;
  const hs256SignatureOk = await crypto.subtle.verify(
    "HMAC",
    verifyKey,
    fromBase64Url(sigB64),
    encoder.encode(signingInput)
  );
  const hs256TokenValid = hs256SignatureOk && parsedClaims.sub === claims.sub && parsedClaims.tenant === claims.tenant;

  const tamperedInput = headerB64 + "." + toBase64Url(encoder.encode(JSON.stringify({ ...claims, role: "root" })));
  const hs256TamperedOk = await crypto.subtle.verify(
    "HMAC",
    verifyKey,
    fromBase64Url(sigB64),
    encoder.encode(tamperedInput)
  );

  // 4. HMAC-SHA512 Webhook verification + SHA-256 / SHA-512 content digests
  const webhookKey = await crypto.subtle.importKey(
    "raw",
    encoder.encode(secretKeyMaterial + ":webhook-v2"),
    { name: "HMAC", hash: "SHA-512" },
    false,
    ["sign", "verify"]
  );
  const webhookBody = encoder.encode(JSON.stringify({ event: "deployment.ready", tenant: claims.tenant }));
  const webhookMac = await crypto.subtle.sign("HMAC", webhookKey, webhookBody);
  const hs512WebhookValid = await crypto.subtle.verify("HMAC", webhookKey, webhookMac, webhookBody);

  const sha256Ab = await crypto.subtle.digest("SHA-256", webhookBody);
  const sha512Ab = await crypto.subtle.digest("SHA-512", webhookBody);

  return {
    hs256TokenValid,
    hs256TamperedRejected: !hs256TamperedOk,
    hs512WebhookValid,
    jwkAlg: exportedJwk.alg,
    sha256PayloadHex: arrayBufferToHex(sha256Ab),
    sha512PrefixHex: arrayBufferToHex(sha512Ab).slice(0, 24),
    encodeIntoRead: intoStats.read,
    encodeIntoWritten: intoStats.written
  };
}
