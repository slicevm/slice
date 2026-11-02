export interface JwtClaims {
  sub: string;
  tenant: string;
  role: string;
  expMs: number;
}

export interface WebCryptoAuditResult {
  hs256TokenValid: boolean;
  hs256TamperedRejected: boolean;
  hs512WebhookValid: boolean;
  jwkAlg: string;
  sha256PayloadHex: string;
  sha512PrefixHex: string;
  encodeIntoRead: number;
  encodeIntoWritten: number;
}

export interface StreamsAndCloneResult {
  transformedPayload: string;
  totalTransformedBytes: number;
  formFieldsCount: number;
  uploadedFileName: string;
  uploadedFileSize: number;
  slicedHeaderText: string;
  cyclicCloneVerified: boolean;
  dispatchedEvents: string[];
  anySignalAbortedBy: string;
}

