import type { StaticAssetRecord } from "./types.ts";

export class ZeroStatCdnEdge {
    private readonly assets: Record<string, StaticAssetRecord>;
    public totalBytesSavedBy304: number = 0;

    constructor(assets: Record<string, StaticAssetRecord>) {
        this.assets = assets;
    }

    public handleFetch(req: Request): Response {
        const url = new URL(req.url);
        if (req.method !== "GET" && req.method !== "HEAD") {
            return Response.json({ error: "method_not_allowed" }, { status: 405 });
        }

        const asset = this.assets[url.pathname];
        if (!asset) {
            return Response.json({ error: "asset_not_found", path: url.pathname }, { status: 404 });
        }

        const ifNoneMatch = req.headers.get("if-none-match");
        if (ifNoneMatch && (ifNoneMatch === "*" || ifNoneMatch === asset.etag)) {
            this.totalBytesSavedBy304 += asset.sizeBytes;
            return new Response(null, {
                status: 304,
                headers: {
                    "etag": asset.etag,
                    "cache-control": asset.cacheControl,
                    "x-slice-static": "HIT-304"
                }
            });
        }

        const body = req.method === "HEAD" ? null : asset.body;
        return new Response(body, {
            status: 200,
            headers: {
                "content-type": asset.contentType,
                "etag": asset.etag,
                "cache-control": asset.cacheControl,
                "x-slice-static": "HIT-200",
                "x-asset-bytes": String(asset.sizeBytes)
            }
        });
    }
}

