import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";
import * as crypto from "node:crypto";
import type { SpecExecutionRecord, TimeTravelSuiteReport } from "./types.ts";
import { applyTenantMutation, createBaselineState, digestState } from "./state_machine.ts";

declare const Slice: any;

interface SnapshotHandle {
    sliceSnapId: number;
    backupVfsContent: string;
    backupKvBalance: string;
    backupKvVersion: string;
}

export class TimeTravelHarness {
    private readonly vfsFile: string;
    private readonly kvStore: Record<string, string> = Object.create(null);

    constructor() {
        this.vfsFile = path.join(os.tmpdir(), "slice_app15_state_snapshot.json");
    }

    public initBaseline(jsonState: string, credits: string, version: string): void {
        fs.writeFileSync(this.vfsFile, jsonState);
        this.kvSet("tenant:credits", credits);
        this.kvSet("tenant:version", version);
    }

    public kvGet(key: string): string {
        if (typeof Slice !== "undefined" && Slice && Slice.kv) {
            return Slice.kv.get(key) || "";
        }
        return this.kvStore[key] || "";
    }

    public kvSet(key: string, val: string): void {
        if (typeof Slice !== "undefined" && Slice && Slice.kv) {
            Slice.kv.set(key, val, 0);
        }
        this.kvStore[key] = val;
    }

    public readVfs(): string {
        return fs.readFileSync(this.vfsFile, "utf8");
    }

    public writeVfs(data: string): void {
        fs.writeFileSync(this.vfsFile, data);
    }

    public checkpoint(): SnapshotHandle {
        const sliceSnapId =
            typeof Slice !== "undefined" && Slice && typeof Slice.checkpoint === "function"
                ? Number(Slice.checkpoint())
                : 0;
        return {
            sliceSnapId,
            backupVfsContent: this.readVfs(),
            backupKvBalance: this.kvGet("tenant:credits"),
            backupKvVersion: this.kvGet("tenant:version"),
        };
    }

    public restore(handle: SnapshotHandle): void {
        if (
            handle.sliceSnapId > 0 &&
            typeof Slice !== "undefined" &&
            Slice &&
            typeof Slice.restore === "function"
        ) {
            Slice.restore(handle.sliceSnapId);
            return;
        }
        this.writeVfs(handle.backupVfsContent);
        this.kvSet("tenant:credits", handle.backupKvBalance);
        this.kvSet("tenant:version", handle.backupKvVersion);
    }

    public cleanup(): void {
        try {
            if (fs.existsSync(this.vfsFile)) {
                fs.unlinkSync(this.vfsFile);
            }
        } catch (_e) {
            // ignore cleanup errors
        }
    }
}

export function runTimeTravelVerificationSuite(): TimeTravelSuiteReport {
    const harness = new TimeTravelHarness();
    const baseline = createBaselineState("tenant-f4-prime");
    const baselineJson = JSON.stringify(baseline);
    const baselineDigest = digestState(baseline);
    harness.initBaseline(baselineJson, String(baseline.creditsCents), baseline.planVersion);

    const records: SpecExecutionRecord[] = [];
    let vfsRollbacksVerified = 0;
    let kvRollbacksVerified = 0;
    let totalRetentionMs = 0;

    const suites = [
        "Suite-01-Semver-Upgrades",
        "Suite-02-Credit-Ledger",
        "Suite-03-Quota-Enforcement",
        "Suite-04-Rollback-Invariants",
    ];

    for (let sIdx = 0; sIdx < suites.length; sIdx++) {
        const suiteName = suites[sIdx];
        for (let i = 0; i < 6; i++) {
            const specIndex = sIdx * 6 + i;
            const snap = harness.checkpoint();

            // Verify pre-spec state is 100% identical to baseline (proving previous spec rolled back!)
            const currentVfs = harness.readVfs();
            const currentCredits = harness.kvGet("tenant:credits");
            const currentVersion = harness.kvGet("tenant:version");
            const preClean =
                currentVfs === baselineJson &&
                currentCredits === "25000" &&
                currentVersion === "2.4.0";

            const parsed = JSON.parse(currentVfs);
            let specPassed = false;
            let stateDigest = baselineDigest;

            if (specIndex % 4 === 3) {
                // Negative invariant spec: dirty VFS & KV then trigger an expected business rule rejection
                harness.writeVfs("{\"corrupted\":true}");
                harness.kvSet("tenant:credits", "0");
                harness.kvSet("tenant:version", "99.0.0");
                let threwExpected = false;
                try {
                    if (sIdx === 0) {
                        applyTenantMutation(parsed, "6.0.0", 500, "1h", 10);
                    } else if (sIdx === 1) {
                        applyTenantMutation(parsed, "3.1.0", 99999, "1h", 10);
                    } else {
                        applyTenantMutation(parsed, "3.1.0", 500, "1h", 5000);
                    }
                } catch (_err) {
                    threwExpected = true;
                }
                specPassed = preClean && threwExpected;
                totalRetentionMs += parsed.retentionWindowMs;
            } else {
                // Positive mutation spec: mutate state, persist to VFS & KV, verify digest
                const targetVer = `3.${sIdx}.${i}`;
                const debit = (i + 1) * 750;
                const extraWin = `${(i + 1) * 2}h`;
                const addReqs = (i + 1) * 25;
                const mutated = applyTenantMutation(parsed, targetVer, debit, extraWin, addReqs);
                harness.writeVfs(JSON.stringify(mutated));
                harness.kvSet("tenant:credits", String(mutated.creditsCents));
                harness.kvSet("tenant:version", mutated.planVersion);

                stateDigest = digestState(mutated);
                totalRetentionMs += mutated.retentionWindowMs;
                specPassed =
                    preClean &&
                    harness.kvGet("tenant:credits") === String(25000 - debit) &&
                    harness.kvGet("tenant:version") === targetVer;
            }

            // Immediately time-travel rollback VFS and KV state to `snap`
            harness.restore(snap);

            const postVfsClean = harness.readVfs() === baselineJson;
            const postKvClean =
                harness.kvGet("tenant:credits") === "25000" &&
                harness.kvGet("tenant:version") === "2.4.0";

            if (postVfsClean) vfsRollbacksVerified++;
            if (postKvClean) kvRollbacksVerified++;

            records.push({
                suite: suiteName,
                name: `spec-${specIndex + 1}`,
                passed: specPassed && postVfsClean && postKvClean,
                rolledBackCleanly: postVfsClean && postKvClean,
                stateDigest,
            });
        }
    }

    harness.cleanup();

    const auditSummary = records
        .map((r) => `${r.suite}/${r.name}:${r.passed}:${r.stateDigest}`)
        .join("|");
    const suiteAuditHash = crypto
        .createHash("sha256")
        .update(auditSummary)
        .digest("hex")
        .slice(0, 16);

    return {
        app: "15-time-travel-test-suite",
        libraries: ["dlv@1.1.3", "dset@3.1.3", "ms@2.1.3", "semver@7.6.3"],
        suitesExecuted: suites.length,
        specsExecuted: records.length,
        specsPassed: records.filter((r) => r.passed).length,
        vfsRollbacksVerified,
        kvRollbacksVerified,
        totalRetentionMs,
        suiteAuditHash,
    };
}

