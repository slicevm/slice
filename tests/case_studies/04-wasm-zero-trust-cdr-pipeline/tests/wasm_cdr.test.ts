import { describe, test, expect } from "slice:test";
import { runWasmZeroTrustCdrStudy } from "../src/pii_vault_engine.ts";

describe("Case Study 04: Pure-Rust Wasm Content Disarm & Zero-Trust PII Vault", () => {
    test("sanitizes 5 document batches via Wasm CDR kernel, vaults 16 PII tokens, and verifies Wasm security cages", () => {
        const report = runWasmZeroTrustCdrStudy();
        expect(report.documentsProcessed).toBe(5);
        expect(report.totalPiiTokensVaulted).toBe(16);
        expect(report.securityChecks.wasmFuelBombPreempted).toBe(true);
        expect(report.securityChecks.wasmMemoryBombCaged).toBe(true);
        expect(report.securityChecks.piiZeroLeakVerified).toBe(true);
    });
});

