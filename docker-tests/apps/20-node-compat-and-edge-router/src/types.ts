export interface WirePacketHeader {
  magic: number;
  version: number;
  flags: number;
  sequenceId: bigint;
  payloadLength: number;
}

export interface DecodedWireFrame {
  magicHex: string;
  version: number;
  flags: number;
  sequenceId: string;
  payloadUtf8: string;
  payloadBase64Url: string;
  sha512DigestPrefix: string;
  hmacSha256Hex: string;
  isValidBufferInstance: boolean;
}

export interface RouteDispatchSummary {
  normalizedRoute: string;
  relativeAssetPath: string;
  parsedBase: string;
  sessionTenant: string;
  ttlMs: number;
  maxBodyBytes: number;
  engineSemverSatisfied: boolean;
  formattedAuditLine: string;
  eventSequence: string[];
}

export const WIRE_MAGIC_V4 = 0x534c4334; // "SLC4"
export const DEFAULT_PROTOCOL_VERSION = 4;

