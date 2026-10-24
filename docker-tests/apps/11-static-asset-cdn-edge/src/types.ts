export interface StaticAssetRecord {
    readonly path: string;
    readonly contentType: string;
    readonly etag: string;
    readonly cacheControl: string;
    readonly maxAgeSec: number;
    readonly sizeBytes: number;
    readonly body: string;
}

export interface CdnMetricsSummary {
    readonly app: string;
    readonly libraries: string[];
    readonly totalRequests: number;
    readonly hit200Count: number;
    readonly notModified304Count: number;
    readonly head200Count: number;
    readonly notFound404Count: number;
    readonly totalBytesServed: number;
    readonly formattedBandwidthSaved: string;
    readonly manifestDigest: string;
}

