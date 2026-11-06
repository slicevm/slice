import { describe, test, expect } from "slice:test";
import { runTimeTravelChaosFuzzerStudy } from "../src/chaos_fuzzer_engine.ts";

describe("Case Study 05: Time-Travel Chaos Fuzzer for Distributed Payment Sagas", () => {
    test("executes 48 chaos payment sagas with 18 instant rollbacks, 6 idempotency dedups, and 0-cent drift", () => {
        const report = runTimeTravelChaosFuzzerStudy();
        expect(report.totalSagaScenarios).toBe(48);
        expect(report.cleanSettlementsCommitted).toBe(24);
        expect(report.chaosFaultsRolledBack).toBe(18);
        expect(report.duplicateReplaysDeduplicated).toBe(6);
        expect(report.zeroCentDriftVerified).toBe(true);
        expect(report.initialTotalSupplyCents).toBe(report.finalTotalSupplyCents);
    });
});

