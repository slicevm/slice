import bytes from "bytes";
import type { EdgeTenantSample, ScoredTenantRow } from "./types.ts";

const sec = (id: number, payload: number[]): number[] => [id, payload.length, ...payload];
const str = (s: string): number[] => [s.length, ...Array.from(s, (c) => c.charCodeAt(0))];
const body = (code: number[]): number[] => [code.length, ...code];

export function buildSlaScorerWasmBytes(): Uint8Array {
    return new Uint8Array([
        0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,
        // Type section (id 1):
        // Type 0: (i32, i32) -> i32  [on_anomaly(idx, score)]
        // Type 1: (i32, i32, i32, i32) -> i32 [score_tenant(rps, p99Us, errors, idx)]
        // Type 2: (i32) -> i32 [classify_sla(score)]
        ...sec(0x01, [
            0x03,
            0x60, 0x02, 0x7F, 0x7F, 0x01, 0x7F,
            0x60, 0x04, 0x7F, 0x7F, 0x7F, 0x7F, 0x01, 0x7F,
            0x60, 0x01, 0x7F, 0x01, 0x7F
        ]),
        // Import section (id 2): env.on_anomaly : Type 0
        ...sec(0x02, [
            0x01,
            ...str("env"),
            ...str("on_anomaly"),
            0x00, 0x00
        ]),
        // Function section (id 3): 2 functions (type 1, type 2)
        ...sec(0x03, [0x02, 0x01, 0x02]),
        // Memory section (id 5): 1 memory (min=1, max=2)
        ...sec(0x05, [0x01, 0x01, 0x01, 0x02]),
        // Global section (id 6): 1 mutable i32 global = 0
        ...sec(0x06, [0x01, 0x7F, 0x01, 0x41, 0x00, 0x0B]),
        // Export section (id 7): 4 exports
        ...sec(0x07, [
            0x04,
            ...str("memory"), 0x02, 0x00,
            ...str("penalty_acc"), 0x03, 0x00,
            ...str("score_tenant"), 0x00, 0x01,
            ...str("classify_sla"), 0x00, 0x02
        ]),
        // Code section (id 10): 2 function bodies
        ...sec(0x0A, [
            0x02,
            // Func 1: score_tenant(rps: 0, p99Us: 1, errors: 2, idx: 3) -> i32
            // score = 1000 - (p99Us / 10) - (errors * 25) + (rps / 100)
            ...body([
                0x01, 0x01, 0x7F, // 1 local i32 (local 4 = score)
                0x41, 0xE8, 0x07, // i32.const 1000 (signed LEB128: 0xE8, 0x07 = 1000)
                0x20, 0x01, 0x41, 0x0A, 0x6D, // p99Us / 10
                0x6B, // 1000 - (p99Us / 10)
                0x20, 0x02, 0x41, 0x19, 0x6C, // errors * 25
                0x6B, // - (errors * 25)
                0x20, 0x00, 0x41, 0xE4, 0x00, 0x6D, // rps / 100 (signed LEB128 for 100 is 0xE4, 0x00)
                0x6A, // + (rps / 100)
                0x21, 0x04, // local.set 4
                // Store score at linear memory offset (idx * 4)
                0x20, 0x03, 0x41, 0x04, 0x6C, // idx * 4
                0x20, 0x04,
                0x36, 0x02, 0x00, // i32.store
                // penalty_acc += on_anomaly(idx, score)
                0x23, 0x00,
                0x20, 0x03, 0x20, 0x04, 0x10, 0x00,
                0x6A,
                0x24, 0x00,
                0x20, 0x04,
                0x0B
            ]),
            // Func 2: classify_sla(score: 0) -> i32 (score >= 850 ? 1 : 0)
            ...body([
                0x00,
                0x20, 0x00,
                0x41, 0xD2, 0x06, // i32.const 850 (unsigned LEB128: 0xD2, 0x06 = 850)
                0x4E, // i32.ge_s
                0x0B
            ])
        ])
    ]);
}

export function escapeHtml(raw: string): string {
    return raw
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;")
        .replace(/'/g, "&#39;");
}

export async function scoreTenantsWithWasm(samples: EdgeTenantSample[]): Promise<{
    rows: ScoredTenantRow[];
    wasmHostHookCalls: number;
    wasmTableLength: number;
    wasmGlobalPenaltyPoints: number;
    totalWasmScoreSum: number;
    criticalTenantsCount: number;
}> {
    const wasmBytes = buildSlaScorerWasmBytes();
    const mod = await WebAssembly.compile(wasmBytes);

    let wasmHostHookCalls = 0;
    const importObj = {
        env: {
            on_anomaly(idx: number, score: number): number {
                wasmHostHookCalls += 1;
                return score < 850 ? (850 - score) + idx : 0;
            }
        }
    };

    const instance = await WebAssembly.instantiate(mod, importObj);
    const exportsObj = instance.exports as {
        memory: WebAssembly.Memory;
        penalty_acc: WebAssembly.Global;
        score_tenant: (rps: number, p99Us: number, errors: number, idx: number) => number;
        classify_sla: (score: number) => number;
    };

    const dispatchTable = new WebAssembly.Table({
        element: "anyfunc",
        initial: 2,
        maximum: 4
    });
    dispatchTable.grow(1);

    const rows: ScoredTenantRow[] = [];
    let criticalTenantsCount = 0;

    for (let i = 0; i < samples.length; i++) {
        const s = samples[i];
        const score = exportsObj.score_tenant(s.requestsPerSec, s.p99LatencyUs, s.errorCount, i);
        const slaCode = exportsObj.classify_sla(score);
        if (slaCode === 0) {
            criticalTenantsCount += 1;
        }
        rows.push({
            tenantId: s.tenantId,
            region: s.region,
            wasmHealthScore: score,
            wasmSlaTierCode: slaCode,
            slaTierLabel: slaCode === 1 ? "PLATINUM" : "DEGRADED",
            formattedMemory: bytes.format(s.memoryBytes),
            rawBannerTitle: s.rawBannerTitle,
            escapedTitle: escapeHtml(s.rawBannerTitle)
        });
    }

    // Read scores back directly from WebAssembly linear memory Int32Array view
    const scoresInMem = new Int32Array(exportsObj.memory.buffer, 0, samples.length);
    let totalWasmScoreSum = 0;
    for (let i = 0; i < scoresInMem.length; i++) {
        totalWasmScoreSum += scoresInMem[i];
    }

    return {
        rows,
        wasmHostHookCalls,
        wasmTableLength: dispatchTable.length,
        wasmGlobalPenaltyPoints: Number(exportsObj.penalty_acc.value),
        totalWasmScoreSum,
        criticalTenantsCount
    };
}
