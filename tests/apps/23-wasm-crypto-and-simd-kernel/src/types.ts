export interface StreamPacket {
    packetId: string;
    tenant: string;
    payload: string;
    cipherKey: number;
}

export interface WasmKernelTelemetry {
    packetsProcessed: number;
    totalBytesTransformed: number;
    hostCallbackInvocations: number;
    combinedAdlerXorDigest: number;
    finalRoundCounter: number;
    initialMemoryFormatted: string;
    grownMemoryFormatted: string;
    preservedHeaderByte: number;
    fnv64Hex: string;
    warmInstantiationVerified: boolean;
    mutatedPayloadSha256: string;
}

export interface WasmSecurityAuditReport {
    ring1InvalidMagicRejected: boolean;
    ring2RuntimeTrapAndFuelVerified: boolean;
    ring3MemoryMaxPagesEnforced: boolean;
    ring4MissingImportLinkError: boolean;
    ring5ReflectionAndCustomSectionOk: boolean;
    customSectionBanner: string;
    exportedSymbolCount: number;
    importedSymbolCount: number;
    auditDigest: string;
}

export interface App23Summary {
    suite: string;
    windowFormat: string;
    kernelTelemetry: WasmKernelTelemetry;
    securityAudit: WasmSecurityAuditReport;
}

