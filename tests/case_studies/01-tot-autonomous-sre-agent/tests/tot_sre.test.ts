import { describe, test, expect } from "slice:test";
import { runTotAutonomousSreStudy } from "../src/tot_agent_engine.ts";

describe("Case Study 01: Depth-4 Tree-of-Thought Autonomous SRE Agent", () => {
    test("evaluates 20 branches across 4 depths with 16 instant VFS+KV rollbacks and 4 commits", () => {
        const report = runTotAutonomousSreStudy();
        expect(report.totalBranchesEvaluated).toBe(20);
        expect(report.committedPath.length).toBe(4);
        expect(report.rollbacksExecuted).toBe(16);
        expect(report.dlpExfiltrationsBlocked).toBe(3);
        expect(report.sesPollutionsBlocked).toBe(2);
        expect(report.fuelDosPreempted).toBe(3);
        expect(report.fsEscapesBlocked).toBe(2);
        expect(report.finalP99Ms).toBe(69);
    });
});

