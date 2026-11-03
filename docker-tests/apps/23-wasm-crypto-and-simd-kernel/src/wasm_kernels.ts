import crypto from "node:crypto";
import bytes from "bytes";
import type { StreamPacket, WasmKernelTelemetry } from "./types.ts";

const sec = (id: number, payload: number[]): number[] => [id, payload.length, ...payload];
const str = (s: string): number[] => [s.length, ...Array.from(s, (c) => c.charCodeAt(0))];
const body = (code: number[]): number[] => [code.length, ...code];

export function buildEdgeKernelWasmBinary(): Uint8Array {
    return new Uint8Array([
        0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00,
        // Custom section (id 0): "slice_wasm_kernel" -> "v2.4-edge-simd"
        ...sec(0x00, [
            ...str("slice_wasm_kernel"),
            ...Array.from("v2.4-edge-simd", (c) => c.charCodeAt(0))
        ]),
        // Type section (id 1): 3 function signatures
        ...sec(0x01, [
            0x03,
            0x60, 0x02, 0x7F, 0x7F, 0x01, 0x7F,       // Type 0: (i32, i32) -> i32
            0x60, 0x03, 0x7F, 0x7F, 0x7F, 0x01, 0x7F, // Type 1: (i32, i32, i32) -> i32
            0x60, 0x02, 0x7E, 0x7E, 0x01, 0x7E        // Type 2: (i64, i64) -> i64
        ]),
        // Import section (id 2): env.host_audit_mix : Type 0
        ...sec(0x02, [
            0x01,
            ...str("env"),
            ...str("host_audit_mix"),
            0x00, 0x00
        ]),
        // Function section (id 3): 2 functions (type 1, type 2)
        ...sec(0x03, [0x02, 0x01, 0x02]),
        // Memory section (id 5): 1 memory (limits: flags=1, min=1 page, max=4 pages)
        ...sec(0x05, [0x01, 0x01, 0x01, 0x04]),
        // Global section (id 6): 1 mutable i32 global = 10
        ...sec(0x06, [0x01, 0x7F, 0x01, 0x41, 0x0A, 0x0B]),
        // Export section (id 7): 4 exports
        ...sec(0x07, [
            0x04,
            ...str("memory"), 0x02, 0x00,
            ...str("round_counter"), 0x03, 0x00,
            ...str("transform_frame"), 0x00, 0x01,
            ...str("mix64_fnv"), 0x00, 0x02
        ]),
        // Code section (id 10): 2 function bodies
        ...sec(0x0A, [
            0x02,
            // Func 1: transform_frame(ptr: i32, len: i32, key: i32) -> i32
            ...body([
                0x01, 0x01, 0x7F, // 1 local of type i32 (local 3)
                0x20, 0x00, 0x28, 0x02, 0x00, // i32.load(ptr)
                0x20, 0x02, 0x73,             // ^ key
                0x23, 0x00, 0x6A,             // + global.get(round_counter)
                0x20, 0x00, 0x20, 0x01, 0x10, 0x00, 0x6A, // + host_audit_mix(ptr, len)
                0x21, 0x03,                   // local.set 3
                0x20, 0x00, 0x20, 0x03, 0x36, 0x02, 0x00, // i32.store(ptr, local 3)
                0x23, 0x00, 0x20, 0x01, 0x6A, 0x24, 0x00, // round_counter += len
                0x20, 0x03,                   // return local 3
                0x0B
            ]),
            // Func 2: mix64_fnv(seed: i64, delta: i64) -> i64
            ...body([
                0x00, // 0 locals
                0x20, 0x00, 0x20, 0x01, 0x85, // (seed ^ delta)
                0x42, 0x1F, 0x7E,             // * 31n
                0x42, 0x11, 0x7C,             // + 17n
                0x0B
            ])
        ])
    ]);
}

export async function runWasmKernelPipeline(
    wasmBytes: Uint8Array,
    packets: StreamPacket[]
): Promise<{ module: WebAssembly.Module; telemetry: WasmKernelTelemetry }> {
    const encoder = new TextEncoder();
    const mod = new WebAssembly.Module(wasmBytes);

    let hostCallbackInvocations = 0;
    let memoryRef: WebAssembly.Memory | null = null;

    const importObject = {
        env: {
            host_audit_mix(ptr: number, len: number): number {
                hostCallbackInvocations += 1;
                if (!memoryRef) {
                    return 0;
                }
                const view = new Uint8Array(memoryRef.buffer, ptr, len);
                let acc = 0;
                for (let i = 0; i < view.length; i++) {
                    acc = (acc + view[i] * (i + 1)) | 0;
                }
                return acc;
            }
        }
    };

    const instance = new WebAssembly.Instance(mod, importObject);
    const exportsObj = instance.exports as {
        memory: WebAssembly.Memory;
        round_counter: WebAssembly.Global;
        transform_frame: (ptr: number, len: number, key: number) => number;
        mix64_fnv: (seed: bigint, delta: bigint) => bigint;
    };
    memoryRef = exportsObj.memory;

    const initialMemoryFormatted = bytes.format(memoryRef.buffer.byteLength);
    let writeOffset = 64;
    let totalBytesTransformed = 0;
    let combinedAdlerXorDigest = 0;
    let fnvAcc = 1469598103934665603n;

    for (let i = 0; i < packets.length; i++) {
        const pkt = packets[i];
        const encoded = encoder.encode(`${pkt.tenant}:${pkt.packetId}:${pkt.payload}`);
        const alignedLen = Math.max(4, Math.ceil(encoded.byteLength / 4) * 4);
        const u8 = new Uint8Array(memoryRef.buffer);
        u8.fill(0, writeOffset, writeOffset + alignedLen);
        u8.set(encoded, writeOffset);

        const frameDigest = exportsObj.transform_frame(writeOffset, encoded.byteLength, pkt.cipherKey);
        combinedAdlerXorDigest = (combinedAdlerXorDigest ^ frameDigest) >>> 0;
        fnvAcc = exportsObj.mix64_fnv(fnvAcc, BigInt( Math.abs(frameDigest) ));

        totalBytesTransformed += encoded.byteLength;
        writeOffset += alignedLen;
    }

    const headerByteBeforeGrow = new Uint8Array(memoryRef.buffer)[64];
    memoryRef.grow(1);
    const grownMemoryFormatted = bytes.format(memoryRef.buffer.byteLength);
    const preservedHeaderByte = new Uint8Array(memoryRef.buffer)[64];

    if (preservedHeaderByte !== headerByteBeforeGrow) {
        throw new Error("Linear memory byte at offset 64 was not preserved across memory.grow(1)");
    }

    const mutatedSlice = new Uint8Array(memoryRef.buffer, 64, writeOffset - 64);
    const mutatedPayloadSha256 = crypto
        .createHash("sha256")
        .update(mutatedSlice)
        .digest("hex")
        .slice(0, 16);

    // Exercise warm module instantiation via WebAssembly.instantiate(wasmBytes, importObject)
    const warmResult = await WebAssembly.instantiate(wasmBytes, importObject);
    const warmExports = warmResult.instance.exports as {
        mix64_fnv: (seed: bigint, delta: bigint) => bigint;
    };
    const warmCheck = warmExports.mix64_fnv(100n, 23n);
    const warmInstantiationVerified = warmCheck === (100n ^ 23n) * 31n + 17n;

    return {
        module: mod,
        telemetry: {
            packetsProcessed: packets.length,
            totalBytesTransformed,
            hostCallbackInvocations,
            combinedAdlerXorDigest,
            finalRoundCounter: Number(exportsObj.round_counter.value),
            initialMemoryFormatted,
            grownMemoryFormatted,
            preservedHeaderByte,
            fnv64Hex: fnvAcc.toString(16),
            warmInstantiationVerified,
            mutatedPayloadSha256
        }
    };
}

