const sec = (id: number, payload: number[]): number[] => [id, payload.length, ...payload];
const str = (s: string): number[] => [s.length, ...Array.from(s, (c) => c.charCodeAt(0))];
const body = (code: number[]): number[] => [code.length, ...code];

export function buildCdrSanitizerWasmBinary(): Uint8Array {
    return new Uint8Array([
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
        // Custom section (0): "slice_cdr_engine" -> "cdr-v3.0-zero-trust"
        ...sec(0x00, [
            ...str("slice_cdr_engine"),
            ...Array.from("cdr-v3.0-zero-trust", (c) => c.charCodeAt(0)),
        ]),
        // Type section (1):
        // Type 0: (i32, i32) -> i32
        // Type 1: (i32, i32, i32) -> i32
        ...sec(0x01, [
            0x02,
            0x60, 0x02, 0x7f, 0x7f, 0x01, 0x7f,
            0x60, 0x03, 0x7f, 0x7f, 0x7f, 0x01, 0x7f,
        ]),
        // Import section (2): env.host_byte_fold : Type 0
        ...sec(0x02, [
            0x01,
            ...str("env"),
            ...str("host_byte_fold"),
            0x00, 0x00,
        ]),
        // Function section (3): 1 function of Type 1
        ...sec(0x03, [0x01, 0x01]),
        // Memory section (5): min 1 page (64KB), max 2 pages (128KB)
        ...sec(0x05, [0x01, 0x01, 0x01, 0x02]),
        // Global section (6): 1 mutable i32 global = 0
        ...sec(0x06, [0x01, 0x7f, 0x01, 0x41, 0x00, 0x0b]),
        // Export section (7): memory, docs_scanned, compute_cdr_simhash
        ...sec(0x07, [
            0x03,
            ...str("memory"), 0x02, 0x00,
            ...str("docs_scanned"), 0x03, 0x00,
            ...str("compute_cdr_simhash"), 0x00, 0x01,
        ]),
        // Code section (10):
        ...sec(0x0a, [
            0x01,
            // compute_cdr_simhash(ptr: i32, len: i32, salt: i32) -> i32
            ...body([
                0x01, 0x01, 0x7f, // 1 local (local 3)
                0x20, 0x00, 0x28, 0x02, 0x00, // i32.load(ptr)
                0x20, 0x02, 0x73,             // ^ salt
                0x20, 0x00, 0x20, 0x01, 0x10, 0x00, 0x6a, // + host_byte_fold(ptr, len)
                0x21, 0x03,                   // local.set 3
                0x23, 0x00, 0x41, 0x01, 0x6a, 0x24, 0x00, // docs_scanned += 1
                0x20, 0x03,
                0x0b,
            ]),
        ]),
    ]);
}

