import { describe, test, expect } from "slice:test";
import { buildSlaScorerWasmBytes, scoreTenantsWithWasm } from "../src/wasm_scorer.ts";
import { renderEdgeDashboardSsrHtml } from "../src/ssr_renderer.ts";

describe("App 24 WebAssembly + SSR Edge Pipeline Suite", () => {
    test("validates and instantiates SLA scorer WebAssembly binary", async () => {
        const wasmBytes = buildSlaScorerWasmBytes();
        expect(WebAssembly.validate(wasmBytes)).toBe(true);

        const result = await scoreTenantsWithWasm([
            {
                tenantId: "acme-edge",
                region: "us-east",
                requestsPerSec: 12400,
                p99LatencyUs: 420,
                errorCount: 1,
                memoryBytes: 8388608,
                rawBannerTitle: "Acme <Gateway>"
            },
            {
                tenantId: "legacy-batch",
                region: "us-west",
                requestsPerSec: 1500,
                p99LatencyUs: 1850,
                errorCount: 4,
                memoryBytes: 4194304,
                rawBannerTitle: "Legacy <script>xss</script>"
            }
        ]);

        expect(result.rows.length).toBe(2);
        expect(result.wasmHostHookCalls).toBe(2);
        expect(result.criticalTenantsCount).toBe(1);
        expect(result.wasmTableLength).toBe(3);

        const html = renderEdgeDashboardSsrHtml(
            "admin_01",
            "global",
            "3 minutes",
            result.rows,
            result.totalWasmScoreSum
        );
        expect(html.includes("<!doctype html>")).toBe(true);
        expect(html.includes("<script>")).toBe(false);
    });
});

