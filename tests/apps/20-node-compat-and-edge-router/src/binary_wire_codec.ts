import * as crypto from "node:crypto";
import assert from "node:assert";
import {
  WIRE_MAGIC_V4,
  DEFAULT_PROTOCOL_VERSION,
  type DecodedWireFrame,
} from "./types.ts";

/**
 * Encodes a binary edge wire frame using real Uint8Array-backed Buffer
 * integer writers (writeUInt32BE, writeUInt16BE, writeBigUInt64BE) and
 * signs the frame with streaming multi-chunk node:crypto createHash/createHmac.
 */
export function encodeAndVerifyWireFrame(
  sequenceId: bigint,
  flags: number,
  payloadText: string,
  hmacSecretHex: string
): DecodedWireFrame {
  const payloadBuf = Buffer.from(payloadText, "utf8");
  assert.strictEqual(Buffer.isBuffer(payloadBuf), true);
  assert.strictEqual(payloadBuf instanceof Uint8Array, true);

  // 18-byte binary header:
  // [0..4]   magic (UInt32BE)
  // [4..6]   version (UInt16BE)
  // [6..8]   flags (UInt16BE)
  // [8..16]  sequenceId (BigUInt64BE)
  // [16..18] payloadLength (UInt16BE)
  const header = Buffer.alloc(18);
  header.writeUInt32BE(WIRE_MAGIC_V4, 0);
  header.writeUInt16BE(DEFAULT_PROTOCOL_VERSION, 4);
  header.writeUInt16BE(flags, 6);
  header.writeBigUInt64BE(sequenceId, 8);
  header.writeUInt16BE(payloadBuf.length, 16);

  const secretKeyBuf = Buffer.from(hmacSecretHex, "hex");
  const hmacTag = crypto
    .createHmac("sha256", secretKeyBuf)
    .update(header)
    .update(payloadBuf)
    .digest();
  assert.strictEqual(Buffer.isBuffer(hmacTag), true);
  assert.strictEqual(hmacTag.length, 32);

  const fullFrame = Buffer.concat([header, payloadBuf, hmacTag]);
  assert.strictEqual(
    fullFrame.length,
    18 + payloadBuf.length + hmacTag.length
  );

  // Decode header from fullFrame using zero-copy subarray + integer readers
  const magic = fullFrame.readUInt32BE(0);
  const version = fullFrame.readUInt16BE(4);
  const decodedFlags = fullFrame.readUInt16BE(6);
  const decodedSeq = fullFrame.readBigUInt64BE(8);
  const decodedLen = fullFrame.readUInt16BE(16);

  assert.strictEqual(magic, WIRE_MAGIC_V4);
  assert.strictEqual(version, DEFAULT_PROTOCOL_VERSION);
  assert.strictEqual(decodedFlags, flags);
  assert.strictEqual(decodedSeq, sequenceId);
  assert.strictEqual(decodedLen, payloadBuf.length);

  const extractedPayload = fullFrame.subarray(18, 18 + decodedLen);
  const extractedTag = fullFrame.subarray(18 + decodedLen);
  assert.strictEqual(extractedPayload.equals(payloadBuf), true);
  assert.strictEqual(extractedTag.equals(hmacTag), true);

  const sha512Digest = crypto
    .createHash("sha512")
    .update(header)
    .update(extractedPayload)
    .digest();
  assert.strictEqual(Buffer.isBuffer(sha512Digest), true);
  assert.strictEqual(sha512Digest.length, 64);

  return {
    magicHex: fullFrame.subarray(0, 4).toString("hex"),
    version,
    flags: decodedFlags,
    sequenceId: decodedSeq.toString(),
    payloadUtf8: extractedPayload.toString("utf8"),
    payloadBase64Url: extractedPayload.toString("base64url"),
    sha512DigestPrefix: sha512Digest.toString("hex").slice(0, 24),
    hmacSha256Hex: extractedTag.toString("hex"),
    isValidBufferInstance:
      Buffer.isBuffer(extractedPayload) &&
      extractedPayload instanceof Uint8Array,
  };
}
