import crypto from "node:crypto";
import ms from "ms";
import type { StaticAssetRecord } from "./types.ts";

const RAW_ASSETS: Array<{ path: string; ext: string; ttl: string; body: string }> = [
    {
        path: "/index.html",
        ext: "html",
        ttl: "1h",
        body: "<!doctype html><html><head><link rel=\"stylesheet\" href=\"/assets/app.css\"></head><body><div id=\"app\">Slice Edge UI</div><script src=\"/assets/bundle.js\"></script></body></html>"
    },
    {
        path: "/docs/index.html",
        ext: "html",
        ttl: "30m",
        body: "<!doctype html><html><head><title>Slice Docs</title></head><body><h1>Zero-Stat CAS Manual</h1></body></html>"
    },
    {
        path: "/assets/app.css",
        ext: "css",
        ttl: "7d",
        body: ":root{--bg:#090d16;--fg:#f8fafc}body{margin:0;background:var(--bg);color:var(--fg);font-family:system-ui,sans-serif}"
    },
    {
        path: "/assets/bundle.js",
        ext: "js",
        ttl: "7d",
        body: "export function bootEdgeApp(){return{ready:true,version:'0.2.0',runtime:'slice-nexusjs'};}"
    },
    {
        path: "/assets/logo.svg",
        ext: "svg",
        ttl: "14d",
        body: "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 64 64\"><circle cx=\"32\" cy=\"32\" r=\"28\" fill=\"#38bdf8\"/></svg>"
    },
    {
        path: "/assets/manifest.json",
        ext: "json",
        ttl: "15m",
        body: "{\"name\":\"Slice Fullstack App\",\"short_name\":\"Slice\",\"start_url\":\"/\",\"display\":\"standalone\"}"
    }
];

export function detectMimeType(ext: string): string {
    switch (ext) {
        case "html":
            return "text/html; charset=utf-8";
        case "css":
            return "text/css; charset=utf-8";
        case "js":
            return "application/javascript; charset=utf-8";
        case "svg":
            return "image/svg+xml";
        case "json":
            return "application/json";
        default:
            return "application/octet-stream";
    }
}

export function buildStaticAssetTable(): Record<string, StaticAssetRecord> {
    const table: Record<string, StaticAssetRecord> = Object.create(null);
    for (let i = 0; i < RAW_ASSETS.length; i++) {
        const item = RAW_ASSETS[i];
        const hash = crypto.createHash("sha256").update(item.body).digest("hex").slice(0, 16);
        const etag = "\"" + hash + "\"";
        const ttlMs = ms(item.ttl) as number;
        const maxAgeSec = Math.floor(ttlMs / 1000);
        const rec: StaticAssetRecord = {
            path: item.path,
            contentType: detectMimeType(item.ext),
            etag,
            cacheControl: "public, max-age=" + maxAgeSec,
            maxAgeSec,
            sizeBytes: Buffer.byteLength(item.body),
            body: item.body
        };
        table[item.path] = rec;
        if (item.path === "/index.html") {
            table["/"] = rec;
        } else if (item.path.endsWith("/index.html")) {
            const dir = item.path.slice(0, item.path.length - "/index.html".length);
            table[dir] = rec;
            table[dir + "/"] = rec;
        }
    }
    return table;
}

