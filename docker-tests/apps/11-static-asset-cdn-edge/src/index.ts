import crypto from "node:crypto";
import bytes from "bytes";
import { buildStaticAssetTable } from "./asset_manifest.ts";
import { ZeroStatCdnEdge } from "./cdn_server.ts";
import type { CdnMetricsSummary } from "./types.ts";

async function runCdnBenchmark(): Promise<void> {
    const table = buildStaticAssetTable();
    const cdn = new ZeroStatCdnEdge(table);

    const paths = [
        "/",
        "/index.html",
        "/docs",
        "/docs/",
        "/assets/app.css",
        "/assets/bundle.js",
        "/assets/logo.svg",
        "/assets/manifest.json",
        "/assets/missing.wasm"
    ];

    let hit200Count = 0;
    let notModified304Count = 0;
    let head200Count = 0;
    let notFound404Count = 0;
    let totalBytesServed = 0;
    const digest = crypto.createHash("sha256");

    for (let i = 0; i < 80; i++) {
        const targetPath = paths[i % paths.length];
        const isHead = i % 10 === 7 && targetPath !== "/assets/missing.wasm";
        const useCachedEtag = !isHead && i % 3 === 1 && table[targetPath] !== undefined;

        const headers = new Headers({
            "Accept": "*/*",
            "X-Slice-Tenant": i % 2 === 0 ? "tenant-cdn-eu" : "tenant-cdn-us"
        });
        if (useCachedEtag) {
            headers.set("If-None-Match", table[targetPath].etag);
        }

        const req = new Request("https://cdn.slice.local" + targetPath, {
            method: isHead ? "HEAD" : "GET",
            headers
        });

        const res = cdn.handleFetch(req);
        const text = await res.text();

        if (res.status === 304) {
            notModified304Count += 1;
        } else if (res.status === 200) {
            if (isHead) {
                head200Count += 1;
            } else {
                hit200Count += 1;
                totalBytesServed += Buffer.byteLength(text);
            }
        } else if (res.status === 404) {
            notFound404Count += 1;
        }

        digest.update(
            res.status + ":" + (res.headers.get("etag") || "none") + ":" + text.length + ";"
        );
    }

    const summary: CdnMetricsSummary = {
        app: "11-static-asset-cdn-edge",
        libraries: ["bytes@3.1.2", "ms@2.1.3"],
        totalRequests: 80,
        hit200Count,
        notModified304Count,
        head200Count,
        notFound404Count,
        totalBytesServed,
        formattedBandwidthSaved: bytes.format(cdn.totalBytesSavedBy304),
        manifestDigest: digest.digest("hex").slice(0, 16)
    };

    console.log(JSON.stringify(summary));
}

runCdnBenchmark();

