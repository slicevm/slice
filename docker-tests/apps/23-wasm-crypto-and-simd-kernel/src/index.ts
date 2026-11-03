import ms from "ms";
import { buildEdgeKernelWasmBinary, runWasmKernelPipeline } from "./wasm_kernels.ts";
import { runWasmSecurityAudit } from "./wasm_security_auditor.ts";
import type { App23Summary, StreamPacket } from "./types.ts";

const SAMPLE_PACKETS: StreamPacket[] = [
    {
        packetId: "pkt-001",
        tenant: "acme-edge",
        payload: "model=llama-4-70b;tokens=512;region=iad",
        cipherKey: 0x5A3C96E1
    },
    {
        packetId: "pkt-002",
        tenant: "nexus-quant",
        payload: "symbol=NVDA;bid=142.85;ask=142.89;qty=400",
        cipherKey: 0x1F83D9AB
    },
    {
        packetId: "pkt-003",
        tenant: "globex-iot",
        payload: "sensor=turbine-09;rpm=14820;tempC=78.4",
        cipherKey: 0x6B05688C
    },
    {
        packetId: "pkt-004",
        tenant: "cyber-shield",
        payload: "rule=waf-sql-04;action=block;asn=64512",
        cipherKey: 0x3C6EF372
    }
];

async function main(): Promise<void> {
    const wasmBytes = buildEdgeKernelWasmBinary();
    const { module, telemetry } = await runWasmKernelPipeline(wasmBytes, SAMPLE_PACKETS);
    const securityAudit = runWasmSecurityAudit(module);

    const summary: App23Summary = {
        suite: "23-wasm-crypto-and-simd-kernel",
        windowFormat: ms(900000, { long: true }),
        kernelTelemetry: telemetry,
        securityAudit
    };

    console.log(JSON.stringify(summary));
}

main();

