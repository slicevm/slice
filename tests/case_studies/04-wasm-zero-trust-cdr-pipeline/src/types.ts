export interface UntrustedDocumentBatch {
    docId: string;
    tenantId: string;
    classification: "HIPAA_CLINICAL" | "PCI_FINANCIAL" | "SOC2_INFRA";
    rawContent: string;
}

export interface SanitizedDocumentRecord {
    docId: string;
    tenantId: string;
    classification: string;
    piiTokensRedacted: number;
    wasmSimHash32: number;
    sanitizedLength: number;
    compressedGzipBytes: number;
    roundtripDecompressedVerified: boolean;
    vaultHmacTag: string;
}

export interface WasmCdrSecurityChecks {
    wasmFuelBombPreempted: boolean;
    wasmMemoryBombCaged: boolean;
    wasmInvalidMagicBlocked: boolean;
    piiZeroLeakVerified: boolean;
}

export interface WasmCdrPipelineReport {
    caseStudy: string;
    retentionWindow: string;
    wasmCustomBanner: string;
    wasmMemoryFormatted: string;
    documentsProcessed: number;
    totalPiiTokensVaulted: number;
    totalRawBytes: number;
    totalCompressedBytes: number;
    records: SanitizedDocumentRecord[];
    securityChecks: WasmCdrSecurityChecks;
    pipelineDigest: string;
}

