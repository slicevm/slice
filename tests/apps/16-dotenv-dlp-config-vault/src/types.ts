export interfaceParsedDotenvVault {
    envMap: Record<string, string>;
    publicKeys: string[];
    autoDlpSecrets: Array<{ key: string; value: string; b64: string; b64Url: string }>;
    maxPayloadBytes: number;
    clusterRegion: string;
}

export interface DlpAuditReport {
    app: string;
    libraries: string[];
    dotenvEntriesLoaded: number;
    publicKeysAllowed: number;
    autoDlpSecretsRegistered: number;
    payloadsAudited: number;
    cleanPayloadsApproved: number;
    plaintextLeaksBlocked: number;
    base64LeaksBlocked: number;
    totalApprovedCookieBytes: string;
    vaultHmacSignature: string;
}

