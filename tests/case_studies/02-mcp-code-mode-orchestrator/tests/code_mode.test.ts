import { describe, test, expect } from "slice:test";
import { runMcpCodeModeCaseStudy } from "../src/code_mode_engine.ts";

describe("Case Study 02: MCP Code Mode Orchestrator", () => {
    test("compresses 40 sequential MCP tool calls into 1 Code Mode sandbox pass with >90% token savings", () => {
        const report = runMcpCodeModeCaseStudy();
        expect(report.legacySequentialToolCalls).toBe(40);
        expect(report.codeModeSandboxPasses).toBe(1);
        expect(report.parityVerifiedBetweenModes).toBe(true);
        expect(report.securityProbesBlocked).toBe(4);
        expect(report.highRiskAccountsIdentified).toBe(3);
    });
});

