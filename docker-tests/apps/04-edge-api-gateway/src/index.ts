import * as crypto from "node:crypto";
import type { HttpMethod, HttpRequest } from "./auth_middleware.ts";
import { EdgeGatewayRouter } from "./router.ts";

function runEdgeApiGatewayApp(): string {
    const router = new EdgeGatewayRouter();
    const tokens = [
        "Bearer tok_live_alpha_99",
        "Bearer tok_live_beta_88",
        "Bearer tok_live_gamma_77",
        "Bearer tok_invalid_000"
    ];
    const urls = [
        "/v1/events/ingest?channel=telemetry&priority=high",
        "/v1/ledger/acc_enterprise_01?currency=EUR&audit=1",
        "/v1/health?region=fra-02",
        "/v1/unknown/route?debug=true"
    ];

    let s200 = 0;
    let s401 = 0;
    let s429 = 0;
    let s404 = 0;
    let totalBytes = 0;
    let latencySum = 0;

    for (let i = 0; i < 100; i++) {
        const url = urls[i % 4];
        const method: HttpMethod = url.startsWith("/v1/events/ingest") ? "POST" : "GET";
        const req: HttpRequest = {
            id: "gw_" + String(5000 + i),
            method: method,
            url: url,
            headers: {
                "authorization": tokens[i % 4],
                "user-agent": "slice-edge-client/2.0"
            },
            bodyJson: method === "POST" ? JSON.stringify({ metric: "cpu_load", value: i * 1.5 }) : null
        };

        const res = router.handle(req);
        if (res.status === 200) s200 += 1;
        else if (res.status === 401) s401 += 1;
        else if (res.status === 429) s429 += 1;
        else if (res.status === 404) s404 += 1;

        totalBytes += res.body.length;
        latencySum += res.latencyPoints;
    }

    const sig = crypto
        .createHash("sha256")
        .update(String(s200) + ":" + String(s401) + ":" + String(totalBytes) + ":" + String(latencySum))
        .digest("hex")
        .slice(0, 16);

    return JSON.stringify({
        app: "04-edge-api-gateway",
        libraries: ["fast-querystring@1.1.2", "ms@2.1.3"],
        totalRequests: 100,
        status200: s200,
        status401: s401,
        status429: s429,
        status404: s404,
        totalResponseBytes: totalBytes,
        latencyPoints: latencySum,
        gatewaySha256: sig
    });
}

console.log(runEdgeApiGatewayApp());

