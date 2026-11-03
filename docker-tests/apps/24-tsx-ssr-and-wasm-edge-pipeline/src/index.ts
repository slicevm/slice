import crypto from "node:crypto";
import cookie from "cookie";
import qs from "fast-querystring";
import ms from "ms";
import { scoreTenantsWithWasm } from "./wasm_scorer.ts";
import { renderEdgeDashboardSsrHtml } from "./ssr_renderer.ts";
import type { EdgeTenantSample, SsrPipelineSummary } from "./types.ts";

const TENANT_SAMPLES: EdgeTenantSample[] = [
    {
        tenantId: "acme-edge",
        region: "us-east",
        requestsPerSec: 12400,
        p99LatencyUs: 420,
        errorCount: 1,
        memoryBytes: 8388608,
        rawBannerTitle: "Acme AI Gateway <prod & live>"
    },
    {
        tenantId: "nexus-quant",
        region: "eu-central",
        requestsPerSec: 28900,
        p99LatencyUs: 310,
        errorCount: 0,
        memoryBytes: 16777216,
        rawBannerTitle: "Nexus Quant Engine \"HFT-01\""
    },
    {
        tenantId: "legacy-batch",
        region: "us-west",
        requestsPerSec: 1500,
        p99LatencyUs: 1850,
        errorCount: 4,
        memoryBytes: 4194304,
        rawBannerTitle: "Legacy Batch <script>alert('xss')</script>"
    },
    {
        tenantId: "cyber-waf",
        region: "ap-northeast",
        requestsPerSec: 9600,
        p99LatencyUs: 540,
        errorCount: 0,
        memoryBytes: 12582912,
        rawBannerTitle: "Cyber Shield WAF Edge"
    }
];

async function main(): Promise<void> {
    const parsedCookies = cookie.parse("slice_session=tenant_admin_09; theme=dark");
    const parsedQuery = qs.parse("region=global-edge&ttlMs=180000&format=ssr");

    const sessionTenant = parsedCookies.slice_session || "anonymous";
    const filterRegion = String(parsedQuery.region || "all");
    const ttlMs = Number(parsedQuery.ttlMs || 60000);
    const cacheTtlWindow = ms(ttlMs, { long: true });

    const {
        rows,
        wasmHostHookCalls,
        wasmTableLength,
        wasmGlobalPenaltyPoints,
        totalWasmScoreSum,
        criticalTenantsCount
    } = await scoreTenantsWithWasm(TENANT_SAMPLES);

    const html = renderEdgeDashboardSsrHtml(
        sessionTenant,
        filterRegion,
        cacheTtlWindow,
        rows,
        totalWasmScoreSum
    );

    const xssEscapedVerified =
        !html.includes("<script>") &&
        html.includes("&lt;script&gt;alert(&#39;xss&#39;)&lt;/script&gt;");

    const setCookieHeader = cookie.serialize("slice_ssr_digest", String(totalWasmScoreSum), {
        path: "/",
        httpOnly: true
    });

    const ssrHtmlSha256 = crypto
        .createHash("sha256")
        .update(html)
        .digest("hex")
        .slice(0, 16);

    const summary: SsrPipelineSummary = {
        suite: "24-tsx-ssr-and-wasm-edge-pipeline",
        sessionTenant,
        filterRegion,
        cacheTtlWindow,
        tenantsEvaluated: rows.length,
        wasmHostHookCalls,
        wasmTableLength,
        wasmGlobalPenaltyPoints,
        totalWasmScoreSum,
        criticalTenantsCount,
        ssrHtmlByteLength: Buffer.byteLength(html, "utf8"),
        xssEscapedVerified,
        setCookieHeader,
        ssrHtmlSha256
    };

    console.log(JSON.stringify(summary));
}

main();

