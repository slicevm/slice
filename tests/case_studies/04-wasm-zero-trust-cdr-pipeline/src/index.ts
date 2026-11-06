import { runWasmZeroTrustCdrStudy } from "./pii_vault_engine.ts";
import { renderWasmCdrVisualReport } from "./visual_renderer.ts";

const isVisual = typeof process !== "undefined" && process.env && process.env.SLICE_VISUAL === "1";
if (isVisual) {
    const scaleRaw = typeof process !== "undefined" && process.env ? Number(process.env.SLICE_LOAD_SCALE || "1") : 1;
    const scale = Number.isFinite(scaleRaw) && scaleRaw > 0 ? Math.min(Math.floor(scaleRaw), 50) : 1;
    const t0 = typeof performance !== "undefined" ? performance.now() : Date.now();
    let report = runWasmZeroTrustCdrStudy();
    for (let s = 1; s < scale; s++) {
        report = runWasmZeroTrustCdrStudy();
    }
    const elapsedMs = Math.max(0.05, (typeof performance !== "undefined" ? performance.now() : Date.now()) - t0);
    console.log(renderWasmCdrVisualReport(report, { scale, elapsedMs }));
} else {
    const report = runWasmZeroTrustCdrStudy();
    console.log(JSON.stringify(report));
}
