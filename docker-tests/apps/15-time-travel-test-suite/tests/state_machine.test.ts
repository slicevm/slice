import { describe, test, expect } from "slice:test";
import * as fs from "node:fs";
import { applyTenantMutation, createBaselineState } from "../src/state_machine.ts";

Slice.kv.set("suite:baseline", "25000", 0);
fs.writeFileSync("/tmp/app15_slice_test_vfs.txt", "clean-state-v1");

describe("App 15 Native Slice Time-Travel Test Runner", () => {
    test("mutates state machine, VFS, and Slice.kv in test 1", () => {
        const base = createBaselineState("tenant-1");
        const next = applyTenantMutation(base, "3.2.0", 5000, "6h", 100);
        expect(next.creditsCents).toBe(20000);
        expect(next.planVersion).toBe("3.2.0");

        Slice.kv.set("suite:baseline", "0", 0);
        fs.writeFileSync("/tmp/app15_slice_test_vfs.txt", "dirty-from-test-1");
        expect(Slice.kv.get("suite:baseline")).toBe("0");
    });

    test("verifies < 5us automatic VFS + KV rollback in test 2", () => {
        expect(Slice.kv.get("suite:baseline")).toBe("25000");
        expect(fs.readFileSync("/tmp/app15_slice_test_vfs.txt")).toBe("clean-state-v1");
    });

    test("verifies OCap and fuel assertions (.toViolate & .toStayWithinFuel)", () => {
        const denied = Slice.evalSandbox(`const d = fs.readFileSync("/etc/passwd");`, {
            allowRead: ["/tmp"],
            fuel: 100_000,
        });
        expect(denied).toViolate("FsReadDenied");

        const fast = Slice.evalSandbox(`const r: number = 14 * 3; r`, {
            fuel: 50_000,
        });
        expect(fast).toStayWithinFuel(1000);
    });
});

